use std::sync::atomic::Ordering;

use cockpit_protocol::project_teardown::{
    WorkspaceTeardownExecuteRequest, WorkspaceTeardownOutcome, WorkspaceTeardownPreview,
    WorkspaceTeardownPreviewRequest, WorkspaceTeardownRecovery, WorkspaceTeardownRecoveryList,
    WorkspaceTeardownRecoveryState, WorkspaceTeardownResult,
};
use cockpit_protocol::projects::{WorkspaceOperation, WorkspaceSetupMode};
use cockpit_protocol::orchestration::{
    CleanupEffectState, CleanupReason, CleanupReasonClass, OwnedResourceIdentity,
    OwnedResourceRecord, OwnedSpaceIdentity, Run,
};
use crate::orchestration::cleanup::{CleanupEffectOutcome, CleanupEvaluation};


use crate::InspectionError;
use crate::project_adapter::ProjectWorktreeRemoveRequest;
use crate::project_store::{TeardownReceipt, TeardownReceiptState, timestamp};
use crate::project_teardown::{
    self, WorkspaceTeardownCommand, WorkspaceTeardownEvidence, WorkspaceTeardownWorktree,
};

use super::{ProjectService, validate_session, verify_inventory};

struct FreshTeardownEvidence {
    operation: WorkspaceOperation,
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
            operation: &self.operation,
            session_id: &self.session_id,
            endpoint_identity: &self.endpoint_identity,
            repository_key: &self.repository_key,
            repository_root: &self.repository_root,
            worktrees: &self.worktrees,
            receipt: self.receipt.as_ref(),
        }
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
            operation_id: operation_id.into(), resources, git_capture_error,
        })
    }

    pub async fn owned_resources_for_run(&self, run: &Run) -> Result<Vec<OwnedResourceIdentity>, InspectionError> {
        let Some(operation_id) = run.setup.as_ref().and_then(|s| s.operation_id.as_deref()) else {
            return Ok(Vec::new());
        };
        let operation = self.load_teardown_operation(&run.session_id, operation_id)?;
        let Some(receipt) = self.store.read_owned_creation(operation_id)? else { return Ok(Vec::new()); };
        project_teardown::validate_creation_receipt(&receipt, &operation)?;
        Ok(receipt.resources)
    }
    pub async fn owned_creation_limitation_for_run(&self, run: &Run) -> Result<Option<String>, InspectionError> {
        let Some(operation_id) = run.setup.as_ref().and_then(|s| s.operation_id.as_deref()) else {
            return Ok(None);
        };
        self.load_teardown_operation(&run.session_id, operation_id)?;
        Ok(self.store.read_owned_creation(operation_id)?.and_then(|r| r.git_capture_error))
    }


    pub async fn validate_owned_resource(&self, identity: &OwnedResourceIdentity) -> Result<(), InspectionError> {
        let operation_id = match identity {
            OwnedResourceIdentity::Worktree { identity } | OwnedResourceIdentity::Branch { identity } => &identity.creation_operation_id,
            OwnedResourceIdentity::Space { identity } => &identity.creation_operation_id,
            _ => return Err(InspectionError::new("unsupported_resource", "not a project resource")),
        };
        let operation = self.store.load(operation_id)?;
        let receipt = self.store.read_owned_creation(operation_id)?.ok_or_else(|| {
            InspectionError::new("ownership_unproven", "no actual owned creation receipt")
        })?;
        project_teardown::validate_creation_receipt(&receipt, &operation)?;
        if receipt.resources.iter().filter(|r| *r == identity).count() != 1 {
            return Err(InspectionError::new("association_conflict", "resource differs from its exact creation receipt"));
        }
        Ok(())
    }

    pub async fn inspect_owned_resource(
        &self, resource: &OwnedResourceRecord, run: &Run,
    ) -> Result<CleanupEvaluation, InspectionError> {
        self.validate_owned_resource(&resource.identity).await?;
        project_teardown::owned_run_identity(resource, run)?;
        match &resource.identity {
            OwnedResourceIdentity::Worktree { identity } | OwnedResourceIdentity::Branch { identity } => {
                let head = match project_teardown::integrated_owned_head(identity, run) {
                    Ok(head) => head,
                    Err(error) => return Ok(CleanupEvaluation::Waiting(cleanup_reason(CleanupReasonClass::Reconcilable, error))),
                };
                let present = crate::project_adapter::inspect_owned_git(
                    identity, head, matches!(resource.identity, OwnedResourceIdentity::Worktree { .. }),
                    self.configuration.limits.git_timeout_ms as u64,
                    self.configuration.limits.git_output_bytes as usize,
                ).await;
                match present {
                    Ok(false) => Ok(CleanupEvaluation::Absent),
                    Err(error) => Ok(CleanupEvaluation::Kept(cleanup_reason(CleanupReasonClass::UserWork, error))),
                    Ok(true) => {
                        if let Err(error) = project_teardown::owned_consumers_quiescent(resource) {
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
                let snapshot = self.adapter.session_snapshot(&identity.session_id).await?;
                if snapshot.session_id != identity.session_id {
                    return Err(InspectionError::new("stale_identity", "Space snapshot belongs to another session"));
                }
                if self.adapter.project_endpoint_identity(&identity.session_id).await? != identity.endpoint_identity {
                    return Ok(CleanupEvaluation::Unknown(CleanupReason { class: CleanupReasonClass::Authority,
                        code: "endpoint_changed".into(), message: "Space endpoint no longer matches its creation receipt".into() }));
                }
                if !snapshot.spaces.iter().any(|s| s.id == identity.workspace_id) {
                    return Ok(CleanupEvaluation::Absent);
                }
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
                if let Err(error) = project_teardown::owned_consumers_quiescent(resource) {
                    return Ok(CleanupEvaluation::Unknown(cleanup_reason(CleanupReasonClass::Authority, error)));
                }
                Ok(CleanupEvaluation::Unknown(CleanupReason { class: CleanupReasonClass::Authority,
                    code: "space_ui_consumers_unproven".into(),
                    message: "current binding absence and sealed worker drain do not prove pinned Notes editors or local UI drafts quiescent".into() }))
            }
            _ => Err(InspectionError::new("unsupported_resource", "not a project resource")),
        }
    }

    pub async fn apply_owned_resource(
        &self, resource: &OwnedResourceRecord, run: &Run,
    ) -> Result<CleanupEffectOutcome, InspectionError> {
        let evaluation = self.inspect_owned_resource(resource, run).await?;
        match evaluation {
            CleanupEvaluation::Absent => return Ok(CleanupEffectOutcome { state: CleanupEffectState::Absent, evidence: Some("fresh exact resource inspection confirms absence".into()), reason: None, effects: Vec::new() }),
            CleanupEvaluation::Ready => {}
            CleanupEvaluation::Waiting(reason) | CleanupEvaluation::Kept(reason) | CleanupEvaluation::Unknown(reason) =>
                return Ok(CleanupEffectOutcome { state: CleanupEffectState::Refused, evidence: None, reason: Some(reason), effects: Vec::new() }),
        }
        let result = match &resource.identity {
            OwnedResourceIdentity::Worktree { identity } => crate::project_adapter::remove_owned_worktree(
                identity, project_teardown::integrated_owned_head(identity, run)?,
                self.configuration.limits.git_timeout_ms as u64, self.configuration.limits.git_output_bytes as usize,
            ).await,
            OwnedResourceIdentity::Branch { identity } => crate::project_adapter::remove_owned_branch(
                identity, project_teardown::integrated_owned_head(identity, run)?,
                self.configuration.limits.git_timeout_ms as u64, self.configuration.limits.git_output_bytes as usize,
            ).await,
            _ => return Err(InspectionError::new("unsupported_resource", "no proven project effect is eligible")),
        };
        Ok(match result {
            Ok(applied) => CleanupEffectOutcome { state: if applied { CleanupEffectState::Applied } else { CleanupEffectState::Absent },
                evidence: Some("guarded non-force effect followed by exact outcome inspection".into()), reason: None, effects: Vec::new() },
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
            let snapshot = self.adapter.session_snapshot(&session.id).await?;
            if snapshot.session_id != session.id {
                return Err(InspectionError::new("consumer_identity_changed", "terminal inventory belongs to a different session"));
            }
            for pane in &snapshot.panes {
                let cwd = pane.cwd.as_deref().ok_or_else(|| {
                    InspectionError::new("terminal_cwd_unknown", "a current terminal has no classified working directory")
                })?;
                let cwd = std::fs::canonicalize(cwd).map_err(|_| {
                    InspectionError::new("terminal_cwd_unknown", "a current terminal working directory cannot be classified")
                })?;
                if cwd.starts_with(target) {
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
        let operation =
            self.load_teardown_operation_for_workspace(session, &request.workspace_id)?;
        let evidence = self.fresh_teardown_evidence(session, operation).await?;
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
        for operation in self.store.list()? {
            if operation.session_id != session {
                continue;
            }
            let Some(receipt) = self.store.read_teardown_receipt(&operation.operation_id)? else {
                continue;
            };
            let Some(workspace_id) = operation.workspace_id.as_deref() else {
                continue;
            };
            if receipt.operation_id != operation.operation_id
                || receipt.workspace_id != workspace_id
                || receipt.endpoint_identity != operation.plan.endpoint_identity
                || receipt.checkout_path != operation.plan.checkout_path
                || receipt.session_id != operation.session_id
                || receipt.session_id != operation.plan.session_id
                || receipt.repository_key
                    != operation
                        .plan
                        .repository
                        .as_ref()
                        .map(|repository| repository.common_dir.as_str())
                        .unwrap_or("")
                || receipt.repository_root
                    != operation
                        .plan
                        .repository
                        .as_ref()
                        .map(|repository| repository.root.as_str())
                        .unwrap_or("")
            {
                return Err(InspectionError::new(
                    "association_conflict",
                    "teardown receipt differs from its operation journal",
                ));
            }
            let state = match receipt.state {
                TeardownReceiptState::Pending => WorkspaceTeardownRecoveryState::Pending,
                TeardownReceiptState::OutcomeUnknown => {
                    WorkspaceTeardownRecoveryState::OutcomeUnknown
                }
                TeardownReceiptState::Completed => continue,
            };
            recoveries.push(WorkspaceTeardownRecovery {
                operation_id: operation.operation_id,
                workspace_id: workspace_id.to_owned(),
                checkout_path: operation.plan.checkout_path,
                state,
            });
        }
        recoveries.sort_by(|left, right| {
            left.checkout_path
                .cmp(&right.checkout_path)
                .then(left.operation_id.cmp(&right.operation_id))
        });
        Ok(WorkspaceTeardownRecoveryList { recoveries })
    }

    /// Execute one explicitly reviewed teardown action. Worktree removal is
    /// always non-force; unrelated directories and existing notes are retained.
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
        let operation = self.load_teardown_operation(session, &request.operation_id)?;
        let _lease = self
            .store
            .acquire_execution_lease(&operation.operation_id)?;
        // Reload after acquiring the cross-host lease so a prior executor's
        // durable receipt is always part of the command decision.
        let operation = self.load_teardown_operation(session, &request.operation_id)?;
        let evidence = self.fresh_teardown_evidence(session, operation).await?;
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
                    message: "Herdr removed the owned worktree without force".to_owned(),
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

    fn load_teardown_operation_for_workspace(
        &self,
        session: &str,
        workspace_id: &str,
    ) -> Result<WorkspaceOperation, InspectionError> {
        let matches = self
            .store
            .list()?
            .into_iter()
            .filter(|operation| {
                operation.session_id == session
                    && operation.workspace_id.as_deref() == Some(workspace_id)
            })
            .collect::<Vec<_>>();
        match matches.as_slice() {
            [operation] => Ok(operation.clone()),
            [] => Err(InspectionError::new(
                "workspace_operation_not_found",
                "no Cockpit operation is associated with this workspace",
            )),
            _ => Err(InspectionError::new(
                "association_conflict",
                "multiple Cockpit operations are associated with this workspace",
            )),
        }
    }

    fn reviewed_teardown_receipt(
        &self,
        evidence: &FreshTeardownEvidence,
    ) -> Result<TeardownReceipt, InspectionError> {
        Ok(TeardownReceipt {
            operation_id: evidence.operation.operation_id.clone(),
            workspace_id: evidence.operation.workspace_id.clone().ok_or_else(|| {
                InspectionError::new("stale_identity", "operation no longer has a workspace")
            })?,
            endpoint_identity: evidence.endpoint_identity.clone(),
            session_id: evidence.session_id.clone(),
            repository_key: evidence.repository_key.clone(),
            repository_root: evidence.repository_root.clone(),
            checkout_path: evidence.operation.plan.checkout_path.clone(),
            state: TeardownReceiptState::Pending,
            updated_at: timestamp(),
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
            || receipt.operation_id != evidence.operation.operation_id
            || receipt.workspace_id
                != evidence
                    .operation
                    .workspace_id
                    .as_deref()
                    .unwrap_or_default()
            || receipt.endpoint_identity != evidence.endpoint_identity
            || receipt.checkout_path != evidence.operation.plan.checkout_path
            || receipt.session_id != evidence.session_id
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
        match evidence.worktrees.as_slice() {
            [] => {
                receipt.state = TeardownReceiptState::Completed;
                receipt.updated_at = timestamp();
                self.store.write_teardown_receipt(&receipt)?;
                Ok(WorkspaceTeardownResult {
                    operation_id: operation_id.to_owned(),
                    workspace_id: request.workspace_id.clone(),
                    action: request.action,
                    outcome: WorkspaceTeardownOutcome::Completed,
                    message: "fresh Herdr inventory confirms the worktree is gone".to_owned(),
                })
            }
            [worktree]
                if worktree.open_workspace_id.as_deref() == Some(request.workspace_id.as_str()) =>
            {
                receipt.state = TeardownReceiptState::Completed;
                receipt.updated_at = timestamp();
                self.store.write_teardown_receipt(&receipt)?;
                Ok(WorkspaceTeardownResult {
                    operation_id: operation_id.to_owned(),
                    workspace_id: request.workspace_id.clone(),
                    action: request.action,
                    outcome: WorkspaceTeardownOutcome::Retained,
                    message: "fresh Herdr inventory confirms the worktree remains; request a new removal review before dispatching again".to_owned(),
                })
            }
            _ => Ok(WorkspaceTeardownResult {
                operation_id: operation_id.to_owned(),
                workspace_id: request.workspace_id.clone(),
                action: request.action,
                outcome: WorkspaceTeardownOutcome::OutcomeUnknown,
                message: "fresh Herdr inventory is ambiguous; removal remains blocked".to_owned(),
            }),
        }
    }

    async fn fresh_teardown_evidence(
        &self,
        session: &str,
        operation: WorkspaceOperation,
    ) -> Result<FreshTeardownEvidence, InspectionError> {
        if operation.plan.mode == WorkspaceSetupMode::Open || operation.plan.repository.is_none() {
            let endpoint_identity = self.adapter.project_endpoint_identity(session).await?;
            return Ok(FreshTeardownEvidence {
                session_id: session.to_owned(),
                receipt: self.store.read_teardown_receipt(&operation.operation_id)?,
                endpoint_identity,
                repository_key: String::new(),
                repository_root: String::new(),
                worktrees: vec![WorkspaceTeardownWorktree {
                    checkout_path: operation.plan.checkout_path.clone(),
                    open_workspace_id: operation.workspace_id.clone(),
                    is_linked_worktree: false,
                    dirty: None,
                }],
                operation,
            });
        }
        let repository = operation.plan.repository.as_ref().expect("checked above");
        let inventory = self
            .adapter
            .project_inventory(session, &repository.checkout_path)
            .await?;
        verify_inventory(&inventory, repository)?;
        let matching_count = inventory
            .worktrees
            .iter()
            .filter(|entry| entry.checkout_path == operation.plan.checkout_path)
            .count();
        let dirty = if matching_count == 1 {
            self.adapter
                .project_worktree_dirty(
                    &operation.plan.checkout_path,
                    self.configuration.limits.git_timeout_ms,
                    self.configuration.limits.git_output_bytes,
                )
                .await
                .ok()
        } else {
            None
        };
        let worktrees = inventory
            .worktrees
            .into_iter()
            .filter(|entry| entry.checkout_path == operation.plan.checkout_path)
            .map(|entry| WorkspaceTeardownWorktree {
                checkout_path: entry.checkout_path,
                open_workspace_id: entry.open_workspace_id,
                is_linked_worktree: entry.is_linked_worktree,
                dirty,
            })
            .collect();
        let receipt = self.store.read_teardown_receipt(&operation.operation_id)?;
        Ok(FreshTeardownEvidence {
            session_id: session.to_owned(),
            endpoint_identity: inventory.endpoint_identity,
            repository_key: inventory.repository_key,
            repository_root: inventory.repository_root,
            operation,
            worktrees,
            receipt,
        })
    }
}

fn cleanup_reason(class: CleanupReasonClass, error: InspectionError) -> CleanupReason {
    CleanupReason { class, code: error.code, message: error.message }
}
