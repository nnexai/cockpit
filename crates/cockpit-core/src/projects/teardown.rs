use std::sync::atomic::Ordering;

use cockpit_protocol::project_teardown::{
    WorkspaceTeardownExecuteRequest, WorkspaceTeardownOutcome, WorkspaceTeardownPreview,
    WorkspaceTeardownPreviewRequest, WorkspaceTeardownRecovery, WorkspaceTeardownRecoveryList,
    WorkspaceTeardownRecoveryState, WorkspaceTeardownResult,
};
use cockpit_protocol::projects::{WorkspaceOperation, WorkspaceSetupMode};
use cockpit_protocol::orchestration::{
    CleanupEffectState, CleanupReason, CleanupReasonClass, DispatchStep, OwnedResourceIdentity,
    NativeProcessIdentity, OwnedResourceRecord, OwnedSpaceIdentity, Run,
};
use crate::orchestration::cleanup::{CleanupEffectOutcome, CleanupEvaluation};


use crate::InspectionError;
use crate::project_adapter::{ProjectSnapshot, ProjectWorktreeRemoveRequest};
use crate::project_store::{TeardownReceipt, TeardownReceiptState, timestamp};
use crate::project_teardown::{
    self, WorkspaceTeardownCommand, WorkspaceTeardownEvidence, WorkspaceTeardownWorktree,
};

use super::{ProjectService, validate_session};

struct FreshTeardownEvidence {
    operation_id: String,
    workspace_id: String,
    checkout_path: String,
    session_id: String,
    endpoint_identity: String,
    repository_key: String,
    repository_root: String,
    worktrees: Vec<WorkspaceTeardownWorktree>,
    receipt: Option<TeardownReceipt>,
}

impl FreshTeardownEvidence {
    fn evidence(&self) -> WorkspaceTeardownEvidence<'_> {
        WorkspaceTeardownEvidence {
            operation: None,
            operation_id: &self.operation_id,
            checkout_path: &self.checkout_path,
            endpoint_identity: &self.endpoint_identity,
            repository_key: &self.repository_key,
            repository_root: &self.repository_root,
            worktrees: &self.worktrees,
            receipt: self.receipt.as_ref(),
        }
    }
}

fn git_terminal_overlaps(target: &std::path::Path, cwd: Option<&str>, canonical: Option<&std::path::Path>) -> Result<bool, InspectionError> {
    use std::path::{Component, Path, PathBuf};
    let cwd = cwd.map(Path::new).filter(|cwd| cwd.is_absolute()).ok_or_else(||
        InspectionError::new("terminal_cwd_unknown", "a current terminal has no absolute classified working directory"))?;
    // Keep lexical overlap even when a path is absent or traverses out of the
    // target. A resolvable alias into the target is also a consumer.
    if cwd.starts_with(target) || canonical.is_some_and(|path| path.starts_with(target)) { return Ok(true) }
    let mut normalized = PathBuf::new();
    for component in cwd.components() {
        match component {
            Component::ParentDir => { normalized.pop(); }
            Component::CurDir => {}
            other => normalized.push(other.as_os_str()),
        }
    }
    Ok(normalized.starts_with(target))
}

#[cfg(test)]
mod git_terminal_tests {
    use super::git_terminal_overlaps;
    use std::path::Path;

    #[test]
    fn missing_unrelated_absolute_cwd_does_not_block_owned_git_cleanup() {
        let target = Path::new("/work/owned");
        assert!(!git_terminal_overlaps(target, Some("/deleted/unrelated"), None).unwrap());
        assert!(!git_terminal_overlaps(target, Some("/work/owned-other"), None).unwrap());
        for cwd in ["/work/owned", "/work/owned/deleted", "/work/owned/../else", "/work/other/../owned/deleted"] {
            assert!(git_terminal_overlaps(target, Some(cwd), None).unwrap(), "{cwd}");
        }
        assert!(git_terminal_overlaps(target, Some("/alias"), Some(Path::new("/work/owned/subdir"))).unwrap());
        assert!(git_terminal_overlaps(target, None, None).is_err());
        assert!(git_terminal_overlaps(target, Some("relative"), None).is_err());
    }
}

impl ProjectService {
    pub(super) async fn capture_owned_creation(
        &self,
        operation_id: &str,
        created: &crate::project_adapter::ProjectWorktreeResult,
    ) -> Result<(), InspectionError> {
        let operation = self.store.load(operation_id)?;
        let mut resources = vec![OwnedResourceIdentity::Space {
            identity: OwnedSpaceIdentity {
                creation_operation_id: operation_id.into(),
                endpoint_identity: operation.plan.endpoint_identity.clone(),
                session_id: operation.session_id.clone(),
                workspace_id: created.workspace_id.clone(),
                checkout_path: created.checkout_path.clone(),
            },
        }];
        let mut git_capture_error = None;
        if operation.plan.mode == WorkspaceSetupMode::Create {
            let repository = operation.plan.repository.as_ref().ok_or_else(|| {
                InspectionError::new("repository_missing", "owned creation has no retained repository")
            })?;
            let branch = created.branch.as_ref().map(|b| format!("refs/heads/{b}"));
            // Missing physical proof retains Git resources. Do not acquire new
            // ownership later during a recovery or registration inspection.
            match crate::project_adapter::capture_owned_git_identity(
                operation_id, &repository.root, &created.checkout_path, branch.as_deref(),
                self.configuration.limits.git_timeout_ms as u64,
                self.configuration.limits.git_output_bytes as usize,
            ).await {
                Ok(identity) => {
                    resources.push(OwnedResourceIdentity::Worktree { identity: identity.clone() });
                    if identity.branch_ref.is_some() {
                        resources.push(OwnedResourceIdentity::Branch { identity });
                    }
                }
                Err(error) => git_capture_error = Some(format!("{}: {}", error.code, error.message)),
            }
        }
        self.store.write_owned_creation(&crate::project_store::OwnedCreationReceipt {
            operation_id: operation_id.into(), resources, git_capture_error, moved_space: None,
        })
    }

    pub async fn owned_resources_for_run(&self, run: &Run) -> Result<Vec<OwnedResourceIdentity>, InspectionError> {
        let creator = run.setup.as_ref().and_then(|setup| setup.operation_id.as_deref());
        let mut resources = match creator {
            Some(id) => {
                let operation = self.load_teardown_operation(&run.session_id, id)?;
                if self.store.owned_checkout(id)?.is_some_and(|journal| journal.resume_binding.is_some()) {
                    return Err(InspectionError::new("association_conflict", "a Space-only resumed binding cannot become the original checkout creator"));
                }
                let Some(receipt) = self.store.read_owned_creation(id)? else { return Ok(Vec::new()); };
                project_teardown::validate_creation_receipt(&receipt, &operation)?;
                receipt.resources
            }
            None => Vec::new(),
        };
        if run.dispatch.as_ref().is_some_and(|dispatch| dispatch.move_operation_id.is_some()
            && matches!(dispatch.step, DispatchStep::MoveIntent | DispatchStep::MoveUnknown))
        {
            // The move intent precedes the committed placement. Its setup can
            // still name the predecessor Space while the new binding is in flight.
            // Original filesystem receipts remain valid, but neither Space is
            // granted by this uncommitted placement.
            resources.retain(|resource| !matches!(resource, OwnedResourceIdentity::Space { .. }));
            return Ok(resources);
        }
        let current_workspace = run.setup.as_ref().and_then(|setup| setup.workspace_id.as_deref());
        let current_move = run.dispatch.as_ref().and_then(|dispatch| dispatch.move_operation_id.as_deref());
        if resources.iter().any(|resource| matches!(resource, OwnedResourceIdentity::Space { identity }
            if Some(identity.workspace_id.as_str()) == current_workspace
                && current_move.is_none_or(|move_id| identity.creation_operation_id == move_id)))
        {
            return Ok(resources);
        }
        let Some(workspace) = current_workspace else { return Ok(resources); };
        let Some(move_id) = current_move else {
            if creator.is_some() { return Err(InspectionError::new("association_conflict", "changed owned Space has no exact current move receipt")); }
            return Ok(resources);
        };
        let operation = self.load_teardown_operation(&run.session_id, move_id)?;
        self.validate_resume_binding_for_run(&operation, run)?;
        let receipt = self.store.read_owned_creation(move_id)?.ok_or_else(|| InspectionError::new("ownership_unproven", "resumed Space has no actual creation receipt"))?;
        project_teardown::validate_creation_receipt(&receipt, &operation)?;
        if receipt.resources.len() != 1 || !matches!(&receipt.resources[0], OwnedResourceIdentity::Space { identity }
            if identity.workspace_id == workspace && identity.creation_operation_id == move_id)
        { return Err(InspectionError::new("association_conflict", "resumed binding did not create only the exact current Space")); }
        resources.retain(|resource| !matches!(resource, OwnedResourceIdentity::Space { .. }));
        resources.extend(receipt.resources);
        Ok(resources)
    }
    pub async fn owned_creation_limitation_for_run(&self, run: &Run) -> Result<Option<String>, InspectionError> {
        let Some(operation_id) = run.setup.as_ref().and_then(|s| s.operation_id.as_deref()) else {
            return Ok(None);
        };
        self.load_teardown_operation(&run.session_id, operation_id)?;
        Ok(self.store.read_owned_creation(operation_id)?.and_then(|r| r.git_capture_error))
    }

    fn validate_resume_binding_for_run(&self, operation: &WorkspaceOperation, run: &Run) -> Result<(), InspectionError> {
        let journal = self.store.owned_checkout(&operation.operation_id)?.ok_or_else(|| InspectionError::new("ownership_unproven", "resumed Space journal is missing"))?;
        let lineage = journal.resume_binding.as_ref().ok_or_else(|| InspectionError::new("association_conflict", "current move is not a resumed Space binding"))?;
        let binding = journal.bind.as_ref().ok_or_else(|| InspectionError::new("association_conflict", "resumed Space has no retained move receipt"))?;
        let setup = run.setup.as_ref().ok_or_else(|| InspectionError::new("association_conflict", "resumed run has no setup identity"))?;
        if lineage.previous_operation_id.as_deref() != setup.operation_id.as_deref()
            || run.checkout.as_ref() != Some(&lineage.expected_checkout)
            || setup.checkout_path != lineage.expected_checkout.path || operation.plan.checkout_path != setup.checkout_path
            || operation.session_id != run.session_id || binding.operation_id != operation.operation_id
            || binding.move_operation_id != operation.operation_id
            || run.dispatch.as_ref().and_then(|dispatch| dispatch.move_operation_id.as_deref()) != Some(operation.operation_id.as_str())
            || setup.workspace_id.as_deref() != Some(binding.workspace_id.as_str())
            || operation.workspace_id.as_deref() != Some(binding.workspace_id.as_str())
        { return Err(InspectionError::new("association_conflict", "resumed Space lineage differs from the current run checkout/move/placement")); }
        Ok(())
    }


    pub async fn validate_owned_resource(&self, identity: &OwnedResourceIdentity) -> Result<(), InspectionError> {
        let setup_operation_id = self.resource_setup_operation(identity)?;
        let operation = self.store.load(&setup_operation_id)?;
        let receipt = self.store.read_owned_creation(&setup_operation_id)?.ok_or_else(|| {
            InspectionError::new("ownership_unproven", "no actual owned creation receipt")
        })?;
        project_teardown::validate_creation_receipt(&receipt, &operation)?;
        if receipt.resources.iter().filter(|r| *r == identity).count() != 1 {
            return Err(InspectionError::new("association_conflict", "resource differs from its exact creation receipt"));
        }
        Ok(())
    }

    fn resource_setup_operation(&self, identity: &OwnedResourceIdentity) -> Result<String, InspectionError> {
        let creation_operation_id = match identity {
            OwnedResourceIdentity::Worktree { identity } | OwnedResourceIdentity::Branch { identity } => return Ok(identity.creation_operation_id.clone()),
            OwnedResourceIdentity::AttemptDirectory { identity } => return Ok(identity.creation_operation_id.clone()),
            OwnedResourceIdentity::Space { identity } => &identity.creation_operation_id,
            _ => return Err(InspectionError::new("unsupported_resource", "not a project resource")),
        };
        let mut matched = None;
        for operation in self.store.list()? {
            if self.store.owned_checkout(&operation.operation_id)?.and_then(|j| j.bind)
                .is_some_and(|bind| bind.move_operation_id == *creation_operation_id)
            {
                if matched.replace(operation.operation_id).is_some() {
                    return Err(InspectionError::new("association_conflict", "move operation belongs to multiple setup journals"));
                }
            }
        }
        Ok(matched.unwrap_or_else(|| creation_operation_id.clone()))
    }

    pub(crate) async fn inspect_owned_resource(
        &self, resource: &OwnedResourceRecord, run: &Run,
        proof: Option<&crate::orchestration::cleanup::OwnedConsumerProof<'_>>,
        merged: Option<&crate::orchestration::cleanup::merged::MergedProof>,
    ) -> Result<CleanupEvaluation, InspectionError> {
        self.validate_owned_resource(&resource.identity).await?;
        let binding_operation_id = self.resource_setup_operation(&resource.identity)?;
        let binding_operation = self.store.load(&binding_operation_id)?;
        let journal = self.store.owned_checkout(&binding_operation_id)?;
        let creator = if let Some(lineage) = journal.as_ref().and_then(|journal| journal.resume_binding.as_ref()) {
            self.validate_resume_binding_for_run(&binding_operation, run)?;
            lineage.previous_operation_id.as_deref()
        } else {
            Some(binding_operation_id.as_str())
        };
        project_teardown::owned_run_identity(resource, run, creator)?;
        match &resource.identity {
            OwnedResourceIdentity::Worktree { identity } | OwnedResourceIdentity::Branch { identity } => {
                let head = match project_teardown::cleanup_owned_head(identity, run, merged) {
                    Ok(head) => head,
                    Err(error) => return Ok(CleanupEvaluation::Waiting(cleanup_reason(CleanupReasonClass::Reconcilable, error))),
                };
                if let Some(merged) = merged {
                    if let Err(error) = merged.reprove().await {
                        return Ok(CleanupEvaluation::Waiting(cleanup_reason(CleanupReasonClass::Reconcilable, error)));
                    }
                }
                let present = crate::project_adapter::inspect_owned_git(
                    identity, head, matches!(resource.identity, OwnedResourceIdentity::Worktree { .. }),
                    self.configuration.limits.git_timeout_ms as u64,
                    self.configuration.limits.git_output_bytes as usize,
                ).await;
                match present {
                    Ok(false) => Ok(CleanupEvaluation::Absent),
                    Err(error) => Ok(CleanupEvaluation::Kept(cleanup_reason(CleanupReasonClass::UserWork, error))),
                    Ok(true) => {
                        if let Err(error) = project_teardown::owned_consumers_quiescent(resource, run, proof).await {
                            return Ok(CleanupEvaluation::Unknown(cleanup_reason(CleanupReasonClass::Authority, error)));
                        }
                        if let Err(error) = self.validate_cleanup_incarnation(run).await {
                            return Ok(CleanupEvaluation::Unknown(cleanup_reason(CleanupReasonClass::Authority, error)));
                        }
                        if let Err(error) = self.current_git_terminals_quiescent(identity).await {
                            return Ok(CleanupEvaluation::Unknown(cleanup_reason(CleanupReasonClass::Authority, error)));
                        }
                        Ok(CleanupEvaluation::Ready)
                    }
                }
            }
            OwnedResourceIdentity::Space { identity } => {
                let captured = if let Some(retirement) = &run.retirement {
                    retirement.identity.as_ref().ok_or_else(|| InspectionError::new(
                        "space_incarnation_unproven", "retired Space has no captured placement identity",
                    )).and_then(|location| captured_space_incarnation(identity, &location.session_id,
                        &location.endpoint_identity, &location.workspace_id, location.herdr_boot_id.as_deref(),
                        location.herdr_peer.as_ref()))
                } else {
                    run.attachment.as_ref().and_then(|attachment| attachment.location.as_ref())
                        .ok_or_else(|| InspectionError::new("space_incarnation_unproven", "Space has no captured placement identity"))
                        .and_then(|location| captured_space_incarnation(identity, &location.session_id,
                            &location.endpoint_identity, &location.workspace_id, location.boot_id.as_deref(),
                            location.peer_process.as_ref()))
                };
                let captured = match captured {
                    Ok(captured) => captured,
                    Err(error) => return Ok(CleanupEvaluation::Unknown(cleanup_reason(CleanupReasonClass::Authority, error))),
                };
                let observed = match self.adapter.project_snapshot_at_endpoint(
                    &identity.session_id, &identity.endpoint_identity,
                ).await {
                    Ok(observed) => observed,
                    Err(error) => return Ok(CleanupEvaluation::Unknown(cleanup_reason(CleanupReasonClass::Authority, error))),
                };
                match fresh_owned_space_absence(identity, captured, &observed) {
                    Ok(true) => return Ok(CleanupEvaluation::Absent),
                    Ok(false) => {}
                    Err(error) => return Ok(CleanupEvaluation::Unknown(cleanup_reason(CleanupReasonClass::Authority, error))),
                }
                let snapshot = &observed.snapshot;
                let notes = crate::notes::space_has_binding(
                    std::path::Path::new(&self.configuration.notes_root),
                    &identity.endpoint_identity, &identity.session_id, &identity.workspace_id,
                );
                let browser = crate::browser::space_has_saved_state(
                    std::path::Path::new(&self.configuration.state_root),
                    &identity.endpoint_identity, &identity.session_id, &identity.workspace_id,
                );
                match notes.and_then(|notes| browser.map(|browser| notes || browser)) {
                    Err(error) => return Ok(CleanupEvaluation::Unknown(cleanup_reason(CleanupReasonClass::Authority, error))),
                    Ok(true) => return Ok(CleanupEvaluation::Kept(CleanupReason { class: CleanupReasonClass::UserWork,
                        code: "space_saved_state".into(), message: "Notes binding or browser saved state protects this Space".into() })),
                    Ok(false) => {}
                }
                if snapshot.panes.iter().any(|p| p.space_id == identity.workspace_id)
                    || snapshot.tabs.iter().any(|t| t.space_id == identity.workspace_id)
                    || snapshot.agents.iter().any(|a| a.space_id == identity.workspace_id)
                {
                    return Ok(CleanupEvaluation::Kept(CleanupReason { class: CleanupReasonClass::UserWork,
                        code: "space_consumers".into(), message: "current terminal, tab or agent consumers protect this Space".into() }));
                }
                if let Err(error) = project_teardown::owned_consumers_quiescent(resource, run, proof).await {
                    return Ok(CleanupEvaluation::Unknown(cleanup_reason(CleanupReasonClass::Authority, error)));
                }
                Ok(CleanupEvaluation::Unknown(CleanupReason { class: CleanupReasonClass::Authority,
                    code: "space_ui_consumers_unproven".into(),
                    message: "current binding absence and sealed worker drain do not prove pinned Notes editors or local UI drafts quiescent".into() }))
            }
            _ => Err(InspectionError::new("unsupported_resource", "not a project resource")),
        }
    }

    /// Fence a cleanup effect against its immutable retired Herdr incarnation.
    pub async fn validate_cleanup_incarnation(&self, run: &Run) -> Result<(), InspectionError> {
        let retirement = run.retirement.as_ref().filter(|retirement| matches!(
            retirement.state, cockpit_protocol::orchestration::RetirementState::Retired { .. },
        )).ok_or_else(|| InspectionError::new("space_incarnation_unproven", "cleanup has no retired placement receipt"))?;
        let identity = retirement.identity.as_ref().ok_or_else(|| InspectionError::new(
            "space_incarnation_unproven", "cleanup has no immutable retirement identity",
        ))?;
        if identity.session_id != run.session_id || identity.endpoint_identity.is_empty()
            || identity.workspace_id.is_empty() || identity.tab_id.is_empty() || identity.pane_id.is_empty()
            || identity.terminal_id.is_empty()
        {
            return Err(InspectionError::new("space_placement_changed", "cleanup retirement placement is incomplete or belongs to another session"));
        }
        let captured = recorded_incarnation(identity.herdr_boot_id.as_deref(), identity.herdr_peer.as_ref())?;
        let observed = self.adapter.project_snapshot_at_endpoint(
            &identity.session_id, &identity.endpoint_identity,
        ).await?;
        response_bound_snapshot(&observed, &identity.session_id, &identity.endpoint_identity)?;
        validate_server_incarnation(captured, &observed)
    }

    pub(super) fn validate_source_incarnation(
        source: &cockpit_protocol::orchestration::RunLocation, observed: &ProjectSnapshot,
    ) -> Result<(), InspectionError> {
        response_bound_snapshot(observed, &source.session_id, &source.endpoint_identity)?;
        validate_server_incarnation(recorded_incarnation(source.boot_id.as_deref(), source.peer_process.as_ref())?, observed)
    }

    pub(crate) async fn apply_owned_resource(
        &self, resource: &OwnedResourceRecord, run: &Run,
        proof: Option<&crate::orchestration::cleanup::OwnedConsumerProof<'_>>,
        merged: Option<&crate::orchestration::cleanup::merged::MergedProof>,
    ) -> Result<CleanupEffectOutcome, InspectionError> {
        let evaluation = self.inspect_owned_resource(resource, run, proof, merged).await?;
        match evaluation {
            CleanupEvaluation::Absent => return Ok(CleanupEffectOutcome { state: CleanupEffectState::Absent, evidence: Some("fresh exact resource inspection confirms absence".into()), reason: None, effects: Vec::new() }),
            CleanupEvaluation::Ready => {}
            CleanupEvaluation::Waiting(reason) | CleanupEvaluation::Kept(reason) | CleanupEvaluation::Unknown(reason) =>
                return Ok(CleanupEffectOutcome { state: CleanupEffectState::Refused, evidence: None, reason: Some(reason), effects: Vec::new() }),
        }
        if let OwnedResourceIdentity::Worktree { identity } | OwnedResourceIdentity::Branch { identity } = &resource.identity {
            self.validate_cleanup_incarnation(run).await?;
            self.current_git_terminals_quiescent(identity).await?;
            // Re-observe the exact sealed scope and private release CAS after
            // other async inspections, adjacent to the non-force Git effect.
            project_teardown::owned_consumers_quiescent(resource, run, proof).await?;
            if let Some(merged) = merged {
                if let Err(error) = merged.reprove().await {
                    return Ok(CleanupEffectOutcome { state: CleanupEffectState::Refused, evidence: None,
                        reason: Some(cleanup_reason(CleanupReasonClass::Reconcilable, error)), effects: Vec::new() });
                }
            }
        }
        let result = match &resource.identity {
            OwnedResourceIdentity::Worktree { identity } => crate::project_adapter::remove_owned_worktree(
                identity, project_teardown::cleanup_owned_head(identity, run, merged)?,
                self.configuration.limits.git_timeout_ms as u64, self.configuration.limits.git_output_bytes as usize,
            ).await,
            OwnedResourceIdentity::Branch { identity } => crate::project_adapter::remove_owned_branch(
                identity, project_teardown::cleanup_owned_head(identity, run, merged)?,
                self.configuration.limits.git_timeout_ms as u64, self.configuration.limits.git_output_bytes as usize,
            ).await,
            _ => return Err(InspectionError::new("unsupported_resource", "no proven project effect is eligible")),
        };
        Ok(match result {
            Ok(applied) => CleanupEffectOutcome { state: if applied { CleanupEffectState::Applied } else { CleanupEffectState::Absent },
                evidence: Some(merged.map_or_else(|| "guarded non-force effect followed by exact outcome inspection".into(),
                    |proof| proof.evidence())), reason: None, effects: Vec::new() },
            Err(error) => CleanupEffectOutcome { state: CleanupEffectState::Unknown, evidence: None,
                reason: Some(cleanup_reason(CleanupReasonClass::Reconcilable, error)), effects: Vec::new() },
        })
    }
    async fn current_git_terminals_quiescent(
        &self, identity: &cockpit_protocol::orchestration::OwnedGitIdentity,
    ) -> Result<(), InspectionError> {
        let sessions = self.adapter.sessions().await?;
        if sessions.sessions.len() > 4096 {
            return Err(InspectionError::new("consumer_inventory_bounded", "current session inventory exceeds its bound"));
        }
        let target = std::path::Path::new(&identity.checkout_path);
        for session in sessions.sessions.iter().filter(|s| s.running) {
            let endpoint = self.adapter.project_endpoint_identity(&session.id).await?;
            let observed = self.adapter.project_snapshot_at_endpoint(&session.id, &endpoint).await?;
            let snapshot = current_physical_snapshot(&observed, &session.id, &endpoint)?;
            for pane in &snapshot.panes {
                let canonical = pane.cwd.as_deref().and_then(|cwd| std::fs::canonicalize(cwd).ok());
                if git_terminal_overlaps(target, pane.cwd.as_deref(), canonical.as_deref())? {
                    return Err(InspectionError::new("foreign_terminal_consumer", "a current terminal retains this owned checkout"));
                }
            }
        }
        Ok(())
    }


    /// Obtain a fresh, non-mutating teardown preview. Git status failures are
    /// represented as `unknown` evidence, which safely blocks removal.
    pub async fn teardown_preview(
        &self,
        session: &str,
        request: &WorkspaceTeardownPreviewRequest,
    ) -> Result<WorkspaceTeardownPreview, InspectionError> {
        validate_session(session)?;
        let evidence = self.fresh_teardown_evidence(session, &request.workspace_id, None).await?;
        project_teardown::preview(request, evidence.evidence())
    }

    /// List durable cleanup records for this session. Entries are journal
    /// pointers only; opening one must still obtain a fresh teardown preview.
    pub fn teardown_recoveries(
        &self,
        session: &str,
    ) -> Result<WorkspaceTeardownRecoveryList, InspectionError> {
        validate_session(session)?;
        let mut recoveries = Vec::new();
        for receipt in self.store.list_teardown_receipts()? {
            if receipt.session_id != session { continue; }
            let state = match receipt.state {
                TeardownReceiptState::Pending => WorkspaceTeardownRecoveryState::Pending,
                TeardownReceiptState::OutcomeUnknown => WorkspaceTeardownRecoveryState::OutcomeUnknown,
                TeardownReceiptState::Completed => continue,
            };
            recoveries.push(WorkspaceTeardownRecovery {
                operation_id: receipt.operation_id, workspace_id: receipt.workspace_id,
                checkout_path: receipt.checkout_path, state,
            });
        }
        recoveries.sort_by(|left, right| {
            left.checkout_path
                .cmp(&right.checkout_path)
                .then(left.operation_id.cmp(&right.operation_id))
        });
        Ok(WorkspaceTeardownRecoveryList { recoveries })
    }

    /// Execute an explicitly reviewed removal, forcing only confirmed dirty
    /// paths. A durable pending journal always precedes the external effect.
    pub async fn teardown_execute(
        &self,
        session: &str,
        request: &WorkspaceTeardownExecuteRequest,
    ) -> Result<WorkspaceTeardownResult, InspectionError> {
        validate_session(session)?;
        if self.shutting_down.load(Ordering::SeqCst) {
            return Err(InspectionError::new(
                "shutdown",
                "project service is shutting down",
            ));
        }
        // Different previews may name distinct operations for the same Space.
        use sha2::Digest;
        let mut workspace_key = sha2::Sha256::new();
        workspace_key.update(session.as_bytes());
        workspace_key.update([0]);
        workspace_key.update(request.workspace_id.as_bytes());
        let lease_name = format!("manual-teardown-{:x}.lock", workspace_key.finalize());
        let _workspace_lease = self.store.try_acquire_named_execution_lease(&lease_name)?
            .ok_or_else(|| InspectionError::new("operation_busy", "another removal of this Space is in progress"))?;
        let _lease = self.store.acquire_execution_lease(&request.operation_id)?;
        let evidence = self.fresh_teardown_evidence(
            session, &request.workspace_id, Some(&request.operation_id),
        ).await?;
        if request.action == cockpit_protocol::project_teardown::WorkspaceTeardownAction::RemoveOwnedWorktree
            && evidence.receipt.as_ref().is_some_and(|receipt| receipt.state == TeardownReceiptState::Completed)
        {
            return Err(InspectionError::new("stale_identity", "this removal operation already finished; request a fresh review"));
        }
        let command = project_teardown::command(request, evidence.evidence())?;
        match command {
            WorkspaceTeardownCommand::CloseSpace {
                workspace_id,
                endpoint_identity,
            } => {
                self.adapter
                    .project_close_workspace(session, &endpoint_identity, &workspace_id)
                    .await?;
                Ok(WorkspaceTeardownResult {
                    operation_id: request.operation_id.clone(),
                    workspace_id,
                    action: request.action,
                    outcome: WorkspaceTeardownOutcome::Completed,
                    message: "workspace closed; the checkout was retained".to_owned(),
                })
            }
            WorkspaceTeardownCommand::RemoveOwnedWorktree {
                workspace_id,
                endpoint_identity,
                checkout_path,
                force,
                ..
            } => {
                let mut receipt = self.reviewed_teardown_receipt(&evidence)?;
                receipt.state = TeardownReceiptState::Pending;
                receipt.updated_at = timestamp();
                self.store.write_teardown_receipt(&receipt)?;
                let remove = ProjectWorktreeRemoveRequest {
                    endpoint_identity,
                    workspace_id: workspace_id.clone(),
                    checkout_path,
                    force,
                };
                match self.adapter.project_remove_worktree(session, &remove).await {
                    Ok(()) => {}
                    Err(_) => {
                        receipt.state = TeardownReceiptState::OutcomeUnknown;
                        receipt.updated_at = timestamp();
                        // If this write fails, the prior pending receipt remains
                        // durable and still prohibits a blind redispatch.
                        self.store.write_teardown_receipt(&receipt)?;
                        return Ok(WorkspaceTeardownResult {
                            operation_id: request.operation_id.clone(),
                            workspace_id,
                            action: request.action,
                            outcome: WorkspaceTeardownOutcome::OutcomeUnknown,
                            message: "Herdr did not confirm removal; reconcile before retrying"
                                .to_owned(),
                        });
                    }
                }
                receipt.state = TeardownReceiptState::Completed;
                receipt.updated_at = timestamp();
                self.store.write_teardown_receipt(&receipt)?;
                Ok(WorkspaceTeardownResult {
                    operation_id: request.operation_id.clone(),
                    workspace_id,
                    action: request.action,
                    outcome: WorkspaceTeardownOutcome::Completed,
                    message: if force { "Herdr discarded the reviewed changes and removed the worktree" }
                        else { "Herdr removed the worktree without force" }.to_owned(),
                })
            }
            WorkspaceTeardownCommand::ReconcileRemoveOutcome { operation_id } => {
                self.reconcile_teardown_removal(session, &operation_id, request, &evidence)
                    .await
            }
        }
    }

    fn load_teardown_operation(
        &self,
        session: &str,
        operation_id: &str,
    ) -> Result<WorkspaceOperation, InspectionError> {
        let operation = self.store.load(operation_id)?;
        if operation.session_id != session {
            return Err(InspectionError::new(
                "stale_identity",
                "operation belongs to another session",
            ));
        }
        Ok(operation)
    }

    fn reviewed_teardown_receipt(
        &self,
        evidence: &FreshTeardownEvidence,
    ) -> Result<TeardownReceipt, InspectionError> {
        Ok(TeardownReceipt {
            operation_id: evidence.operation_id.clone(), manual: true,
            workspace_id: evidence.workspace_id.clone(),
            endpoint_identity: evidence.endpoint_identity.clone(),
            session_id: evidence.session_id.clone(),
            repository_key: evidence.repository_key.clone(),
            repository_root: evidence.repository_root.clone(),
            checkout_path: evidence.checkout_path.clone(),
            state: TeardownReceiptState::Pending, updated_at: timestamp(),
        })
    }

    fn recovery_receipt(
        &self,
        evidence: &FreshTeardownEvidence,
        expected: TeardownReceiptState,
    ) -> Result<TeardownReceipt, InspectionError> {
        let receipt = evidence.receipt.clone().ok_or_else(|| {
            InspectionError::new(
                "teardown_receipt_missing",
                "teardown recovery receipt is missing",
            )
        })?;
        if receipt.state != expected
            || receipt.operation_id != evidence.operation_id
            || receipt.workspace_id != evidence.workspace_id
            || receipt.checkout_path != evidence.checkout_path
            || receipt.repository_key != evidence.repository_key
            || receipt.repository_root != evidence.repository_root
        {
            return Err(InspectionError::new(
                "stale_identity",
                "teardown recovery receipt no longer matches fresh provenance",
            ));
        }
        Ok(receipt)
    }

    async fn reconcile_teardown_removal(
        &self,
        _session: &str,
        operation_id: &str,
        request: &WorkspaceTeardownExecuteRequest,
        evidence: &FreshTeardownEvidence,
    ) -> Result<WorkspaceTeardownResult, InspectionError> {
        let mut receipt = self
            .recovery_receipt(evidence, TeardownReceiptState::OutcomeUnknown)
            .or_else(|error| {
                if error.code == "stale_identity" {
                    self.recovery_receipt(evidence, TeardownReceiptState::Pending)
                } else {
                    Err(error)
                }
            })?;
        // Presence alone cannot prove a timed-out effect stopped running.
        if evidence.worktrees.is_empty()
            && std::fs::symlink_metadata(&receipt.checkout_path)
                .is_err_and(|error| error.kind() == std::io::ErrorKind::NotFound)
        {
            receipt.state = TeardownReceiptState::Completed;
            receipt.updated_at = timestamp();
            self.store.write_teardown_receipt(&receipt)?;
            Ok(WorkspaceTeardownResult {
                operation_id: operation_id.to_owned(), workspace_id: request.workspace_id.clone(),
                action: request.action, outcome: WorkspaceTeardownOutcome::Completed,
                message: "fresh inventory and filesystem evidence confirm removal".to_owned(),
            })
        } else {
            Ok(WorkspaceTeardownResult {
                operation_id: operation_id.to_owned(), workspace_id: request.workspace_id.clone(),
                action: request.action, outcome: WorkspaceTeardownOutcome::OutcomeUnknown,
                message: "removal is not conclusively finished; the durable receipt prevents redispatch".to_owned(),
            })
        }
    }

    async fn fresh_teardown_evidence(
        &self, session: &str, workspace_id: &str, requested_operation_id: Option<&str>,
    ) -> Result<FreshTeardownEvidence, InspectionError> {
        let endpoint_identity = self.adapter.project_endpoint_identity(session).await?;
        let observed = self.adapter.project_snapshot_at_endpoint(session, &endpoint_identity).await?;
        let snapshot = response_bound_snapshot(&observed, session, &endpoint_identity)?;
        let mut spaces = snapshot.spaces.iter().filter(|space| space.id == workspace_id);
        let space = match (spaces.next(), spaces.next()) {
            (Some(space), None) => Some(space), (None, _) => None,
            _ => return Err(InspectionError::new("association_conflict", "selected Space is ambiguous")),
        };
        if space.is_none() && (snapshot.tabs.iter().any(|tab| tab.space_id == workspace_id)
            || snapshot.panes.iter().any(|pane| pane.space_id == workspace_id))
        {
            return Err(InspectionError::new("association_conflict", "selected Space still has dangling live consumers"));
        }
        let live_checkout = space.and_then(|space| space.git.as_ref()).map(|git| git.checkout_path.as_str());
        let mut pending = self.store.list_teardown_receipts()?.into_iter().filter(|receipt|
            matches!(receipt.state, TeardownReceiptState::Pending | TeardownReceiptState::OutcomeUnknown)
                && live_checkout.map_or(receipt.session_id == session && receipt.workspace_id == workspace_id,
                    |path| receipt.checkout_path == path));
        let pending = match (pending.next(), pending.next()) {
            (Some(receipt), None) => Some(receipt), (None, _) => None,
            _ => return Err(InspectionError::new("association_conflict", "multiple uncertain removals name this Space")),
        };
        if let (Some(receipt), Some(requested)) = (&pending, requested_operation_id)
            && receipt.operation_id != requested
        {
            return Err(InspectionError::new("stale_identity", "an earlier removal must be reconciled before dispatch"));
        }
        let operation_id = pending.as_ref().map(|receipt| receipt.operation_id.clone())
            .or_else(|| requested_operation_id.map(str::to_owned))
            .unwrap_or_else(|| uuid::Uuid::new_v4().to_string());
        let receipt = match pending {
            Some(receipt) => Some(receipt), None => self.store.read_teardown_receipt(&operation_id)?,
        };
        if receipt.as_ref().is_some_and(|receipt|
            receipt.workspace_id != workspace_id
                || (receipt.state == TeardownReceiptState::Completed && receipt.session_id != session))
        {
            return Err(InspectionError::new("stale_identity", "teardown journal belongs to another selected target"));
        }
        let source = live_checkout
            .or_else(|| receipt.as_ref().map(|receipt| receipt.repository_root.as_str()).filter(|path| !path.is_empty()));
        let mut evidence = FreshTeardownEvidence {
            operation_id, workspace_id: workspace_id.to_owned(),
            checkout_path: receipt.as_ref().map(|receipt| receipt.checkout_path.clone()).unwrap_or_default(),
            session_id: session.to_owned(), endpoint_identity: endpoint_identity.clone(),
            repository_key: String::new(), repository_root: String::new(),
            worktrees: Vec::new(), receipt: None,
        };
        if let Some(source) = source {
            let inventory = self.adapter.project_inventory(session, source).await?;
            if inventory.endpoint_identity != endpoint_identity {
                return Err(InspectionError::new("stale_identity", "Herdr endpoint changed during target inspection"));
            }
            let relevant = inventory.worktrees.into_iter().filter(|entry|
                entry.open_workspace_id.as_deref() == Some(workspace_id)
                    || receipt.as_ref().is_some_and(|receipt| entry.checkout_path == receipt.checkout_path))
                .collect::<Vec<_>>();
            if relevant.len() == 1 { evidence.checkout_path = relevant[0].checkout_path.clone(); }
            evidence.repository_key = inventory.repository_key;
            evidence.repository_root = inventory.repository_root;
            for entry in relevant {
                let status = if entry.is_linked_worktree && !entry.is_primary {
                    self.adapter.project_worktree_dirty(
                        &entry.checkout_path, self.configuration.limits.git_timeout_ms,
                        self.configuration.limits.git_output_bytes,
                    ).await.ok()
                } else { None };
                evidence.worktrees.push(WorkspaceTeardownWorktree {
                    checkout_path: entry.checkout_path, open_workspace_id: entry.open_workspace_id,
                    is_linked_worktree: entry.is_linked_worktree, is_primary: entry.is_primary, status,
                });
            }
            if space.is_some() && evidence.worktrees.is_empty() {
                evidence.worktrees.push(WorkspaceTeardownWorktree {
                    checkout_path: evidence.checkout_path.clone(), open_workspace_id: Some(workspace_id.to_owned()),
                    is_linked_worktree: false, is_primary: false, status: None,
                });
            }
        } else if let Some(space) = space {
            evidence.checkout_path = snapshot.panes.iter()
                .find(|pane| pane.space_id == space.id)
                .and_then(|pane| pane.cwd.clone()).unwrap_or_default();
            evidence.worktrees.push(WorkspaceTeardownWorktree {
                checkout_path: evidence.checkout_path.clone(), open_workspace_id: Some(space.id.clone()),
                is_linked_worktree: false, is_primary: false, status: None,
            });
        } else if receipt.is_none() {
            return Err(InspectionError::new("workspace_not_found", "selected Space is no longer live"));
        }
        evidence.receipt = receipt;
        Ok(evidence)
    }
}

fn cleanup_reason(class: CleanupReasonClass, error: InspectionError) -> CleanupReason {
    CleanupReason { class, code: error.code, message: error.message }
}

#[derive(Debug, Clone, Copy)]
struct SpaceIncarnation<'a> {
    boot: Option<&'a str>,
    peer: Option<&'a NativeProcessIdentity>,
}

fn known_peer(peer: &NativeProcessIdentity) -> bool {
    peer.pid > 0 && peer.start_ticks > 0
        && peer.kernel_boot_id.as_deref().is_some_and(|boot| uuid::Uuid::parse_str(boot).is_ok())
}

fn captured_space_incarnation<'a>(
    identity: &OwnedSpaceIdentity, session: &str, endpoint: &str, workspace: &str,
    boot: Option<&'a str>, peer: Option<&'a NativeProcessIdentity>,
) -> Result<SpaceIncarnation<'a>, InspectionError> {
    if identity.session_id != session || identity.endpoint_identity != endpoint || identity.workspace_id != workspace {
        return Err(InspectionError::new("space_placement_changed", "Space creation differs from its retained placement incarnation"));
    }
    recorded_incarnation(boot, peer)
}

fn recorded_incarnation<'a>(
    boot: Option<&'a str>, peer: Option<&'a NativeProcessIdentity>,
) -> Result<SpaceIncarnation<'a>, InspectionError> {
    let captured = SpaceIncarnation { boot: boot.filter(|boot| !boot.is_empty()), peer: peer.filter(|peer| known_peer(peer)) };
    if captured.boot.is_none() && captured.peer.is_none() {
        return Err(InspectionError::new("space_incarnation_unproven", "cleanup has no recorded actual daemon boot or complete physical peer"));
    }
    Ok(captured)
}

fn fresh_owned_space_absence(
    identity: &OwnedSpaceIdentity, captured: SpaceIncarnation<'_>, observed: &ProjectSnapshot,
) -> Result<bool, InspectionError> {
    response_bound_snapshot(observed, &identity.session_id, &identity.endpoint_identity)?;
    validate_server_incarnation(captured, observed)?;
    if observed.snapshot.spaces.iter().any(|space| space.id == identity.workspace_id) {
        return Ok(false);
    }
    if observed.snapshot.panes.iter().any(|pane| pane.space_id == identity.workspace_id)
        || observed.snapshot.tabs.iter().any(|tab| tab.space_id == identity.workspace_id)
        || observed.snapshot.agents.iter().any(|agent| agent.space_id == identity.workspace_id)
    {
        return Err(InspectionError::new("space_snapshot_conflict", "missing Space still has terminal, tab or agent consumers in the same snapshot"));
    }
    Ok(true)
}

fn validate_server_incarnation(captured: SpaceIncarnation<'_>, observed: &ProjectSnapshot) -> Result<(), InspectionError> {
    let fresh_boot = observed.boot_id.as_deref().filter(|boot| !boot.is_empty());
    let fresh_peer = observed.peer_process.as_ref().filter(|peer| known_peer(peer));
    if let (Some(recorded), Some(fresh)) = (captured.boot, fresh_boot)
        && recorded != fresh
    {
        return Err(InspectionError::new("space_boot_changed", "response-bound daemon boot conflicts with the recorded cleanup placement"));
    }
    if let (Some(recorded), Some(fresh)) = (captured.peer, fresh_peer)
        && recorded != fresh
    {
        return Err(InspectionError::new("space_peer_changed", "response-bound physical peer conflicts with the recorded cleanup placement"));
    }
    let boot_matches = captured.boot.is_some() && captured.boot == fresh_boot;
    let peer_matches = captured.peer.is_some() && captured.peer == fresh_peer;
    if !boot_matches && !peer_matches {
        return Err(InspectionError::new("space_incarnation_unproven", "cleanup has no matching recorded daemon boot or proved physical peer"));
    }
    Ok(())
}

fn response_bound_snapshot<'a>(
    observed: &'a ProjectSnapshot, session: &str, endpoint: &str,
) -> Result<&'a cockpit_protocol::v1::SessionSnapshotResponse, InspectionError> {
    if observed.endpoint_identity != endpoint || observed.snapshot.session_id != session {
        return Err(InspectionError::new("endpoint_changed", "response-bound terminal inventory belongs to another peer or session"));
    }
    Ok(&observed.snapshot)
}

fn current_physical_snapshot<'a>(
    observed: &'a ProjectSnapshot, session: &str, endpoint: &str,
) -> Result<&'a cockpit_protocol::v1::SessionSnapshotResponse, InspectionError> {
    let snapshot = response_bound_snapshot(observed, session, endpoint)?;
    if !observed.peer_process.as_ref().is_some_and(known_peer) {
        return Err(InspectionError::new("terminal_peer_unproven", "current terminal census requires a complete proved physical peer"));
    }
    Ok(snapshot)
}

#[cfg(test)]
mod tests;
