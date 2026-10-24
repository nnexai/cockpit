use cockpit_protocol::project_teardown::{
    WorkspaceTeardownAction, WorkspaceTeardownDirtyState,
    WorkspaceTeardownExecuteRequest, WorkspaceTeardownOwnership, WorkspaceTeardownPreview,
    WorkspaceTeardownPreviewRequest, WorkspaceTeardownWorkspaceState,
};
use cockpit_protocol::projects::{
    WorkspaceCheckoutOwnership, WorkspaceOperation,
};

use crate::InspectionError;
use crate::project_store::{TeardownReceipt, TeardownReceiptState};

pub const REMOVE_WORKTREE_CONFIRMATION: &str = "REMOVE WORKTREE";
pub const DISCARD_WORKTREE_CONFIRMATION: &str = "DISCARD AND REMOVE";
pub(crate) fn validate_creation_receipt(
    receipt: &crate::project_store::OwnedCreationReceipt,
    operation: &WorkspaceOperation,
) -> Result<(), InspectionError> {
    use cockpit_protocol::orchestration::OwnedResourceIdentity;
    let conflict = || InspectionError::new("association_conflict", "owned creation receipt differs from its creating operation");
    if receipt.operation_id != operation.operation_id
        || receipt.operation_id != operation.plan.operation_id
        || receipt.resources.is_empty()
    {
        return Err(conflict());
    }
    let mut kinds = 0u8;
    for resource in &receipt.resources {
        let kind = match resource {
            OwnedResourceIdentity::Worktree { identity } | OwnedResourceIdentity::Branch { identity } => {
                let repository = operation.plan.repository.as_ref().ok_or_else(conflict)?;
                let mut owned = operation.owned_resources.iter().filter(|r| r.kind == "worktree");
                let owned = match (owned.next(), owned.next()) {
                    (Some(owned), None) if owned.created_by_operation => owned,
                    _ => return Err(conflict()),
                };
                if operation.plan.ownership != WorkspaceCheckoutOwnership::OwnedWorktree
                    || owned.path != identity.checkout_path
                    || identity.creation_operation_id != receipt.operation_id
                    || identity.checkout_path != operation.plan.checkout_path
                    || identity.repository_root != repository.root
                    || identity.common_dir != repository.common_dir
                    || identity.repository_key != repository.common_dir
                    || identity.branch_ref.as_deref().is_some_and(|b| !b.starts_with("refs/heads/"))
                    || identity.branch_ref.as_deref().and_then(|b| b.strip_prefix("refs/heads/")) != operation.plan.branch.as_deref()
                {
                    return Err(conflict());
                }
                if matches!(resource, OwnedResourceIdentity::Worktree { .. }) { 1 } else {
                    if identity.branch_ref.is_none() { return Err(conflict()); }
                    2
                }
            }
            OwnedResourceIdentity::Space { identity } => {
                let creation_operation = match receipt.moved_space.as_ref() {
                    Some(binding) if binding.operation_id == receipt.operation_id
                        && binding.workspace_id == identity.workspace_id
                        && binding.endpoint_identity == identity.endpoint_identity
                        && operation.tab_id.as_deref() == Some(binding.tab_id.as_str())
                        && operation.pane_id.as_deref() == Some(binding.pane_id.as_str())
                        && !binding.terminal_id.is_empty()
                        && !binding.move_operation_id.is_empty() => &binding.move_operation_id,
                    Some(_) => return Err(conflict()),
                    None => &receipt.operation_id,
                };
                if identity.creation_operation_id != *creation_operation
                    || operation.workspace_id.as_deref() != Some(identity.workspace_id.as_str())
                    || identity.session_id != operation.session_id
                    || identity.session_id != operation.plan.session_id
                    || identity.endpoint_identity != operation.plan.endpoint_identity
                    || identity.checkout_path != operation.plan.checkout_path
                {
                    return Err(conflict());
                }
                4
            }
            OwnedResourceIdentity::AttemptDirectory { identity } => {
                let mut owned = operation.owned_resources.iter().filter(|r| r.kind == "attempt_directory");
                let owned = match (owned.next(), owned.next()) {
                    (Some(owned), None) if owned.created_by_operation => owned,
                    _ => return Err(conflict()),
                };
                if operation.plan.ownership != WorkspaceCheckoutOwnership::OwnedAttemptDirectory
                    || operation.plan.mode != cockpit_protocol::projects::WorkspaceSetupMode::Open
                    || identity.creation_operation_id != receipt.operation_id
                    || identity.root != operation.plan.checkout_path || owned.path != identity.root
                    || identity.device.is_empty() || identity.inode.is_empty()
                    || operation.plan.repository.is_some() || operation.plan.branch.is_some() || operation.plan.base.is_some()
                { return Err(conflict()); }
                8
            }
            _ => return Err(conflict()),
        };
        if kinds & kind != 0 { return Err(conflict()); }
        kinds |= kind;
    }
    let worktree = receipt.resources.iter().find_map(|r| match r {
        OwnedResourceIdentity::Worktree { identity } => Some(identity), _ => None,
    });
    let branch = receipt.resources.iter().find_map(|r| match r {
        OwnedResourceIdentity::Branch { identity } => Some(identity), _ => None,
    });
    if branch.is_some() && branch != worktree { return Err(conflict()); }
    if receipt.moved_space.is_some() && kinds & 4 == 0 { return Err(conflict()); }
    if operation.plan.ownership == WorkspaceCheckoutOwnership::OwnedAttemptDirectory && kinds & 8 == 0 {
        return Err(conflict());
    }
    Ok(())
}
pub(crate) fn owned_run_identity(
    resource: &cockpit_protocol::orchestration::OwnedResourceRecord,
    run: &cockpit_protocol::orchestration::Run,
    setup_operation_id: Option<&str>,
) -> Result<(), InspectionError> {
    use cockpit_protocol::orchestration::{CleanupAuthority, OwnedResourceIdentity, ResourceOwnership};
    let operation_id = match &resource.identity {
        OwnedResourceIdentity::Worktree { identity } | OwnedResourceIdentity::Branch { identity } => &identity.creation_operation_id,
        OwnedResourceIdentity::Space { identity } => &identity.creation_operation_id,
        OwnedResourceIdentity::AttemptDirectory { identity } => &identity.creation_operation_id,
        _ => return Err(InspectionError::new("unsupported_resource", "not a project resource")),
    };
    if resource.run_id != run.run_id || resource.root_id != run.root_id
        || resource.ownership != ResourceOwnership::Owned
        || resource.authority != CleanupAuthority::OwnedWhenSafe
        || resource.creation_operation_id.as_deref() != Some(operation_id.as_str())
        || run.setup.as_ref().and_then(|s| s.operation_id.as_deref()) != setup_operation_id
    {
        return Err(InspectionError::new("ownership_unproven", "cleanup requires the exact owned creating run and standing authority"));
    }
    Ok(())
}

pub(crate) fn cleanup_owned_head<'a>(
    identity: &'a cockpit_protocol::orchestration::OwnedGitIdentity,
    run: &'a cockpit_protocol::orchestration::Run,
    merged: Option<&crate::orchestration::cleanup::merged::MergedProof>,
) -> Result<&'a str, InspectionError> {
    use cockpit_protocol::orchestration::{CloseReason, Integration, IntegrationMethod, ProjectSource, RunStage, SourceOutput};
    if let (Integration::NotApplicable { result_message_id, .. }, Some(source @ SourceOutput::NonCode {
        result_message_id: source_result, ..
    })) = (&run.integration, &run.source_output) {
        if run.stage != RunStage::Closed || run.close_reason != Some(CloseReason::Accepted)
            || result_message_id != source_result
            || run.result.as_ref().is_none_or(|result| result.message_id != *source_result
                || result.outcome != Some(cockpit_protocol::orchestration::ReportOutcome::Succeeded)
                || result.source_output.as_ref() != Some(source))
        {
            return Err(InspectionError::new("integration_unverified", "non-code exemption does not cover this exact accepted Result"));
        }
        // Non-code has no Git output to integrate. Only the unchanged creation
        // tip is eligible; status checks still retain untracked artifacts.
        return Ok(&identity.creation_tip);
    }
    let source = run.source_output.as_ref().ok_or_else(||
        InspectionError::new("waiting_integration", "accepted Git output has no exact source Result"))?;
    let SourceOutput::Code { repository_key, checkout_key, head_commit, commits,
        result_message_id, task_revision, .. } = source else {
        return Err(InspectionError::new("integration_unverified", "Git cleanup needs exact accepted code output"));
    };
    // Source Results use the canonical project key; the owned creation journal
    // uses Herdr's raw repository/common-directory key. Join both contracts
    // through the recorded Git project, never compare a hash to a pathname.
    let repository_matches = run.project.as_ref().is_some_and(|project|
        repository_key == &project.key
            && matches!(&project.source, ProjectSource::Git { common_dir }
                if common_dir == &identity.common_dir && common_dir == &identity.repository_key));
    if run.stage != RunStage::Closed || run.close_reason != Some(CloseReason::Accepted)
        || !repository_matches || checkout_key != &identity.checkout_key
        || run.result.as_ref().is_none_or(|result| result.message_id != *result_message_id
            || result.outcome != Some(cockpit_protocol::orchestration::ReportOutcome::Succeeded)
            || result.source_output.as_ref() != Some(source))
    {
        return Err(InspectionError::new("integration_unverified", "validated integration does not cover this exact accepted checkout output"));
    }
    if merged.is_some_and(|proof| proof.matches_run(run) && proof.accepted_head == *head_commit) {
        return Ok(head_commit);
    }
    let Integration::Integrated { accepted_result_message_id, accepted_task_revision, source: integrated,
        target_repository_key, method, mappings, verified_at, .. } = &run.integration else {
        return Err(InspectionError::new("waiting_integration", "accepted output requires integration or fresh containment in the root target"));
    };
    if verified_at.is_empty() || integrated.as_ref() != source || target_repository_key != repository_key
        || accepted_result_message_id != result_message_id || accepted_task_revision != task_revision {
        return Err(InspectionError::new("integration_unverified", "validated integration does not cover this exact accepted checkout output"));
    }
    if matches!(method, IntegrationMethod::PatchEquivalent | IntegrationMethod::RecordedAdaptation)
        && (mappings.is_empty() || mappings.iter().flat_map(|m| &m.source_commits).ne(commits.iter()))
    {
        return Err(InspectionError::new("integration_unverified", "adapted integration requires the validated complete source mapping"));
    }
    Ok(head_commit)
}

pub(crate) async fn owned_consumers_quiescent(
    resource: &cockpit_protocol::orchestration::OwnedResourceRecord,
    run: &cockpit_protocol::orchestration::Run,
    proof: Option<&crate::orchestration::cleanup::OwnedConsumerProof<'_>>,
) -> Result<(), InspectionError> {
    if !resource.consumers.is_empty() {
        return Err(InspectionError::new("resource_consumers", "registered consumers still retain this resource"));
    }
    let proof = proof.ok_or_else(|| InspectionError::new(
        "consumer_proof_missing",
        "exact service-issued released claim and owned scope authority is required; empty consumer lists and PID absence are not proof",
    ))?;
    proof.validate(resource, run).await
}



/// Fresh response-bound live inventory and bounded Git status. Historical
/// setup records supply an optional journal key, never removal authority.
#[derive(Debug, Clone)]
pub struct WorkspaceTeardownEvidence<'a> {
    pub operation: Option<&'a WorkspaceOperation>,
    pub operation_id: &'a str,
    pub checkout_path: &'a str,
    pub endpoint_identity: &'a str,
    pub repository_key: &'a str,
    pub repository_root: &'a str,
    pub worktrees: &'a [WorkspaceTeardownWorktree],
    pub(crate) receipt: Option<&'a TeardownReceipt>,
}

#[derive(Debug, Clone)]
pub struct WorkspaceTeardownWorktree {
    pub checkout_path: String,
    pub open_workspace_id: Option<String>,
    pub is_linked_worktree: bool,
    pub is_primary: bool,
    /// Failed or unavailable bounded status blocks removal.
    pub status: Option<crate::project_adapter::WorktreeStatus>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WorkspaceTeardownCommand {
    CloseSpace {
        workspace_id: String,
        endpoint_identity: String,
    },
    RemoveOwnedWorktree {
        workspace_id: String,
        endpoint_identity: String,
        force: bool,
        checkout_path: String,
    },
    ReconcileRemoveOutcome {
        operation_id: String,
    },
}

pub fn preview(
    request: &WorkspaceTeardownPreviewRequest,
    evidence: WorkspaceTeardownEvidence<'_>,
) -> Result<WorkspaceTeardownPreview, InspectionError> {
    validate_identity(&request.workspace_id, "workspace_id")?;
    let mut matching = evidence.worktrees.iter().filter(|worktree|
        worktree.open_workspace_id.as_deref() == Some(request.workspace_id.as_str()));
    let (workspace_state, worktree) = match (matching.next(), matching.next()) {
        (Some(worktree), None) => (WorkspaceTeardownWorkspaceState::Live, Some(worktree)),
        (None, _) => (WorkspaceTeardownWorkspaceState::Missing, None),
        _ => (WorkspaceTeardownWorkspaceState::Ambiguous, None),
    };
    let checkout_path = worktree.map(|entry| entry.checkout_path.as_str())
        .unwrap_or(evidence.checkout_path);
    let is_linked_worktree = worktree.is_some_and(|entry| entry.is_linked_worktree);
    let status = worktree.and_then(|entry| entry.status.as_ref());
    let dirty_state = match status {
        Some(status) if status.count == 0 => WorkspaceTeardownDirtyState::Clean,
        Some(_) => WorkspaceTeardownDirtyState::Dirty,
        None => WorkspaceTeardownDirtyState::Unknown,
    };
    let mut blockers = Vec::new();
    if workspace_state != WorkspaceTeardownWorkspaceState::Live {
        blockers.push("the exact worktree is not live in the requested Space".to_owned());
    }
    if !is_linked_worktree {
        blockers.push("the target is not a linked worktree".to_owned());
    }
    let checkout = std::path::Path::new(checkout_path);
    let root = std::path::Path::new(evidence.repository_root);
    if evidence.repository_key.is_empty() || !checkout.is_absolute() || !root.is_absolute()
        || root.starts_with(checkout) || worktree.is_some_and(|entry| entry.is_primary)
    {
        blockers.push("the primary or unverified repository checkout cannot be removed".to_owned());
    }
    if dirty_state == WorkspaceTeardownDirtyState::Unknown {
        blockers.push("Git status could not be confirmed; refresh the review before removing".to_owned());
    }
    let receipt = evidence.receipt;
    let removal_pending = receipt.is_some_and(|receipt|
        matches!(receipt.state, TeardownReceiptState::Pending | TeardownReceiptState::OutcomeUnknown));
    let receipt_confirmed = receipt.is_some_and(|receipt|
        receipt_matches(receipt, &evidence, request, checkout_path));
    if receipt.is_some() && !receipt_confirmed {
        blockers.push("the prior teardown receipt belongs to a different target identity".to_owned());
    }
    if removal_pending {
        blockers.push("a prior removal has an unknown outcome; reconcile it before retrying".to_owned());
    }
    let mut allowed_actions = Vec::new();
    // Closing while an uncertain removal is in flight would destroy the
    // remaining authoritative evidence needed to reconcile that effect.
    if workspace_state == WorkspaceTeardownWorkspaceState::Live && !removal_pending {
        allowed_actions.push(WorkspaceTeardownAction::CloseSpace);
    }
    if removal_pending && receipt_confirmed {
        allowed_actions.push(WorkspaceTeardownAction::ReconcileRemoveOutcome);
    }
    let removal_allowed = blockers.is_empty();
    if removal_allowed {
        allowed_actions.push(WorkspaceTeardownAction::RemoveOwnedWorktree);
    }
    Ok(WorkspaceTeardownPreview {
        operation_id: evidence.operation.map(|operation| operation.operation_id.as_str())
            .unwrap_or(evidence.operation_id).to_owned(),
        workspace_id: request.workspace_id.clone(),
        endpoint_identity: evidence.endpoint_identity.to_owned(),
        repository_key: (!evidence.repository_key.is_empty()).then(|| evidence.repository_key.to_owned()),
        repository_root: (!evidence.repository_root.is_empty()).then(|| evidence.repository_root.to_owned()),
        checkout_path: checkout_path.to_owned(),
        ownership: if is_linked_worktree { WorkspaceTeardownOwnership::SelectedLinked }
            else { WorkspaceTeardownOwnership::BorrowedOpened },
        workspace_state,
        is_linked_worktree,
        dirty_state,
        discard_count: status.map_or(0, |status| status.count),
        discard_paths: status.map_or_else(Vec::new, |status| status.entries.clone()),
        discard_fingerprint: status.filter(|status| status.count > 0)
            .map(|status| status.porcelain_sha256.clone()),
        allowed_actions,
        blockers,
        warnings: Vec::new(),
        required_confirmation: removal_allowed.then(|| match dirty_state {
            WorkspaceTeardownDirtyState::Dirty => DISCARD_WORKTREE_CONFIRMATION.to_owned(),
            _ => REMOVE_WORKTREE_CONFIRMATION.to_owned(),
        }),
    })
}

/// Recompute a fresh preview before producing an adapter command. This module
/// never removes files; the integration owner executes the returned Herdr
/// command and confirms authoritative removal before completing its receipt.
pub fn command(
    request: &WorkspaceTeardownExecuteRequest,
    evidence: WorkspaceTeardownEvidence<'_>,
) -> Result<WorkspaceTeardownCommand, InspectionError> {
    let preview = preview(
        &WorkspaceTeardownPreviewRequest {
            workspace_id: request.workspace_id.clone(),
        },
        evidence,
    )?;
    if request.operation_id != preview.operation_id
        || request.expected_endpoint_identity != preview.endpoint_identity
        || request.expected_checkout_path != preview.checkout_path
    {
        return Err(InspectionError::new(
            "stale_identity",
            "the reviewed teardown target changed; request a fresh preview",
        ));
    }
    if !preview.allowed_actions.contains(&request.action) {
        return Err(InspectionError::new(
            "teardown_not_allowed",
            "the requested teardown action is not allowed by fresh provenance",
        ));
    }
    match request.action {
        WorkspaceTeardownAction::CloseSpace => Ok(WorkspaceTeardownCommand::CloseSpace {
            workspace_id: preview.workspace_id,
            endpoint_identity: preview.endpoint_identity,
        }),
        WorkspaceTeardownAction::ReconcileRemoveOutcome => {
            Ok(WorkspaceTeardownCommand::ReconcileRemoveOutcome {
                operation_id: preview.operation_id,
            })
        }
        WorkspaceTeardownAction::RemoveOwnedWorktree => {
            if Some(request.confirmation.as_str()) != preview.required_confirmation.as_deref() {
                return Err(InspectionError::new(
                    "confirmation_required",
                    "type the required confirmation before removing this worktree",
                ));
            }
            if request.discard_fingerprint != preview.discard_fingerprint {
                return Err(InspectionError::new(
                    "stale_identity",
                    "the reviewed dirty paths changed; request a fresh preview",
                ));
            }
            Ok(WorkspaceTeardownCommand::RemoveOwnedWorktree {
                workspace_id: preview.workspace_id,
                endpoint_identity: preview.endpoint_identity,
                force: preview.dirty_state == WorkspaceTeardownDirtyState::Dirty,
                checkout_path: preview.checkout_path,
            })
        }
    }
}

fn receipt_matches(
    receipt: &TeardownReceipt,
    evidence: &WorkspaceTeardownEvidence<'_>,
    request: &WorkspaceTeardownPreviewRequest,
    checkout_path: &str,
) -> bool {
    receipt.operation_id == evidence.operation_id
        && receipt.workspace_id == request.workspace_id
        && receipt.checkout_path == checkout_path
        && receipt.repository_key == evidence.repository_key
        && receipt.repository_root == evidence.repository_root
}

fn validate_identity(value: &str, field: &str) -> Result<(), InspectionError> {
    if value.is_empty()
        || value.len() > 256
        || value.contains('/')
        || value.contains('\\')
        || value.chars().any(char::is_control)
    {
        return Err(InspectionError::new(
            "invalid_teardown_request",
            format!("{field} is invalid"),
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use cockpit_protocol::projects::{
        RepositoryCandidate, WorkspaceOperationStep, WorkspaceOwnedResource, WorkspaceSetupMode,
        WorkspaceOperationState, WorkspaceSetupPlan,
    };

    fn operation(created: bool) -> WorkspaceOperation {
        WorkspaceOperation {
            operation_id: "operation".to_owned(),
            generation: 1,
            sequence: 1,
            session_id: "session".to_owned(),
            plan: WorkspaceSetupPlan {
                operation_id: "operation".to_owned(),
                generation: 1,
                endpoint_identity: "endpoint".to_owned(),
                session_id: "session".to_owned(),
                repository: Some(RepositoryCandidate {
                    repository_id: "repository".to_owned(),
                    name: "repository".to_owned(),
                    root: "/repository".to_owned(),
                    checkout_path: "/repository".to_owned(),
                    common_dir: "/repository/.git".to_owned(),
                    branch: Some("main".to_owned()),
                    is_linked_worktree: false,
                    is_detached: false,
                    provenance: "catalog".to_owned(),
                }),
                mode: if created {
                    WorkspaceSetupMode::Create
                } else {
                    WorkspaceSetupMode::Open
                },
                ownership: if created {
                    cockpit_protocol::projects::WorkspaceCheckoutOwnership::OwnedWorktree
                } else {
                    cockpit_protocol::projects::WorkspaceCheckoutOwnership::BorrowedDirectory
                },
                branch: Some("task".to_owned()),
                base: None,
                checkout_path: "/worktrees/task".to_owned(),
                label: "Task".to_owned(),
                focus: true,
                artifact: None,
                linked_artifacts: Vec::new(),
                effects: Vec::new(),
                warnings: Vec::new(),
            },
            state: WorkspaceOperationState::Completed,
            step: WorkspaceOperationStep::Completed,
            workspace_id: Some("workspace".to_owned()),
            tab_id: None,
            pane_id: None,
            owned_resources: vec![
                WorkspaceOwnedResource {
                    kind: "worktree".to_owned(),
                    path: "/worktrees/task".to_owned(),
                    created_by_operation: created,
                },
            ],
            error: None,
            resume_allowed: false,
            cancel_requested: false,
            updated_at: "0".to_owned(),
        }
    }

    fn evidence<'a>(
        operation: Option<&'a WorkspaceOperation>,
        worktrees: &'a [WorkspaceTeardownWorktree],
    ) -> WorkspaceTeardownEvidence<'a> {
        WorkspaceTeardownEvidence {
            operation,
            operation_id: operation.map_or("a9b3e282-e372-4c49-bbd7-1586b6773754", |operation| operation.operation_id.as_str()),
            checkout_path: "/worktrees/task",
            endpoint_identity: "endpoint",
            repository_key: "/repository/.git",
            repository_root: "/repository",
            worktrees,
            receipt: None,
        }
    }

    fn worktrees(dirty: Option<bool>) -> [WorkspaceTeardownWorktree; 1] {
        [WorkspaceTeardownWorktree {
            checkout_path: "/worktrees/task".to_owned(),
            open_workspace_id: Some("workspace".to_owned()),
            is_linked_worktree: true,
            is_primary: false,
            status: dirty.map(|dirty| crate::project_adapter::WorktreeStatus {
                porcelain_sha256: if dirty { "a".repeat(64) } else { "b".repeat(64) },
                entries: if dirty { vec!["changed.txt".to_owned()] } else { Vec::new() },
                count: u32::from(dirty),
            }),
        }]
    }

    fn receipt(state: TeardownReceiptState) -> TeardownReceipt {
        TeardownReceipt {
            operation_id: "operation".to_owned(),
            manual: false,
            workspace_id: "workspace".to_owned(),
            endpoint_identity: "endpoint".to_owned(),
            checkout_path: "/worktrees/task".to_owned(),
            session_id: "session".to_owned(),
            repository_key: "/repository/.git".to_owned(),
            repository_root: "/repository".to_owned(),
            state,
            updated_at: "0".to_owned(),
        }
    }
    fn git_identity() -> cockpit_protocol::orchestration::OwnedGitIdentity {
        cockpit_protocol::orchestration::OwnedGitIdentity {
            creation_operation_id: "operation".into(), machine_id: "machine".into(),
            repository_key: "/repository/.git".into(), repository_root: "/repository".into(),
            repository_device: "1".into(), repository_inode: "2".into(),
            common_dir: "/repository/.git".into(), common_device: "1".into(), common_inode: "3".into(),
            checkout_path: "/worktrees/task".into(), checkout_device: "1".into(), checkout_inode: "4".into(),
            checkout_key: "checkout".into(), branch_ref: Some("refs/heads/task".into()),
            creation_tip: "a".repeat(40),
        }
    }

    #[test]
    fn owned_receipt_rejects_same_path_conflicting_provenance_and_borrowed_creation() {
        use cockpit_protocol::orchestration::OwnedResourceIdentity;
        let identity = git_identity();
        let mut branch = identity.clone();
        branch.checkout_inode = "999".into();
        let mut receipt = crate::project_store::OwnedCreationReceipt {
            operation_id: "operation".into(), resources: vec![
                OwnedResourceIdentity::Worktree { identity: identity.clone() },
                OwnedResourceIdentity::Branch { identity: branch },
            ], git_capture_error: None, moved_space: None,
        };
        assert!(validate_creation_receipt(&receipt, &operation(true)).is_err());
        receipt.resources = vec![OwnedResourceIdentity::Worktree { identity }];
        assert!(validate_creation_receipt(&receipt, &operation(false)).is_err());
        assert!(validate_creation_receipt(&receipt, &operation(true)).is_ok());
        receipt.resources.push(receipt.resources[0].clone());
        assert!(validate_creation_receipt(&receipt, &operation(true)).is_err());
    }

    #[test]
    fn accepted_git_cleanup_checks_code_mapping_and_noncode_exemption() {
        use cockpit_protocol::orchestration::{Integration, IntegrationMethod, IntegrationMapping, Run, SourceOutput};
        let identity = git_identity();
        // Canonical cockpit:project:git:v1 key for the fixture common directory.
        let canonical_project_key = "f8e39957618599aa4b40a126448ac682c466262c530b1a22c8400c22464990bf".to_owned();
        let source = SourceOutput::Code {
            repository_key: canonical_project_key.clone(), checkout_key: identity.checkout_key.clone(),
            base_commit: identity.creation_tip.clone(), head_commit: "b".repeat(40), commits: vec!["b".repeat(40)],
            task_revision: "task-revision".into(), result_message_id: "result".into(), focused_checks: vec![],
        };
        let mut run: Run = serde_json::from_value(serde_json::json!({
            "session_id":"session","brief":"work","run_id":"run","kind":"worker","label":"worker",
            "root_id":"root","parent_run_id":null,"task_id":null,"attempt":1,"task_revision_at_start":null,
            "launch_config":cockpit_protocol::projects::OrchestrationConfiguration::default(),
            "assignment_generation":null,
            "stage":"closed","close_reason":"accepted","dispatch":null,"target":null,"setup":null,
            "cleanup_policy":null,"last_report":null,"result":{"message_id":"result","kind":"result","outcome":"succeeded",
                "summary":"accepted","plan":null,"source_output":source,"at":"0"},
            "source_output":source,"integration":{"state":"pending"},"annotations":[],
            "project":{"key":canonical_project_key,"source":{"kind":"git","common_dir":identity.common_dir},
                "repository_id":null,"source_workspace_id":null},
            "checkout":null,"attachment":null,"checkout_claim_id":null,"resume":null,"retirement":null,
            "created_at":"0","updated_at":"0"
        })).expect("closed accepted run");
        assert!(cleanup_owned_head(&identity, &run, None).is_err());
        run.integration = Integration::Integrated {
            operation_id:"integration".into(), accepted_result_message_id:"result".into(),
            accepted_task_revision:"task-revision".into(), source:Box::new(source.clone()),
            target_repository_key:canonical_project_key.clone(), target_ref:"refs/heads/main".into(),
            target_commit:"c".repeat(40), method:IntegrationMethod::RecordedAdaptation,
            mappings:vec![].into(), checks:vec![], integrator_run_id:"integrator".into(), verified_at:"verified".into(),
        };
        assert!(cleanup_owned_head(&identity, &run, None).is_err());
        if let Integration::Integrated { mappings, .. } = &mut run.integration {
            *mappings = vec![IntegrationMapping {
                source_commits:vec!["b".repeat(40)], target_commits:vec!["c".repeat(40)],
                source_paths:vec!["source.rs".into()], target_paths:vec!["target.rs".into()], explanation:Some("validated adaptation".into()),
            }].into();
        }
        assert_eq!(cleanup_owned_head(&identity, &run, None).unwrap(), "b".repeat(40));
        assert_ne!(canonical_project_key, identity.repository_key, "canonical project hash is not the raw Git key");
        let mut wrong_source_key = run.clone();
        if let Integration::Integrated { source, .. } = &mut wrong_source_key.integration {
            if let SourceOutput::Code { repository_key, .. } = &mut **source {
                *repository_key = identity.repository_key.clone();
            }
        }
        if let Integration::Integrated { target_repository_key, source, .. } = &mut wrong_source_key.integration {
            *target_repository_key = identity.repository_key.clone();
            wrong_source_key.source_output = Some(*source.clone());
            wrong_source_key.result.as_mut().unwrap().source_output = Some(*source.clone());
        }
        assert!(cleanup_owned_head(&identity, &wrong_source_key, None).is_err());
        let mut wrong_target_key = run.clone();
        if let Integration::Integrated { target_repository_key, .. } = &mut wrong_target_key.integration {
            *target_repository_key = identity.repository_key.clone();
        }
        assert!(cleanup_owned_head(&identity, &wrong_target_key, None).is_err());
        let mut wrong_project_key = run.clone();
        wrong_project_key.project.as_mut().unwrap().key = "e".repeat(64);
        assert!(cleanup_owned_head(&identity, &wrong_project_key, None).is_err());
        let mut wrong_common_dir = run.clone();
        wrong_common_dir.project.as_mut().unwrap().source = cockpit_protocol::orchestration::ProjectSource::Git {
            common_dir: "/foreign/.git".into() };
        assert!(cleanup_owned_head(&identity, &wrong_common_dir, None).is_err());
        let mut wrong_raw_key = identity.clone();
        wrong_raw_key.repository_key = "/foreign/.git".into();
        assert!(cleanup_owned_head(&wrong_raw_key, &run, None).is_err());
        if let Integration::Integrated { verified_at, .. } = &mut run.integration { verified_at.clear(); }
        assert!(cleanup_owned_head(&identity, &run, None).is_err());
        // The exemption does not apply to code output.
        run.integration = Integration::NotApplicable { reason: "non-code deliverable".into(), result_message_id: "result".into() };
        assert!(cleanup_owned_head(&identity, &run, None).is_err());
        let noncode = SourceOutput::NonCode { result_message_id: "result".into(), task_revision: "task-revision".into(),
            artifacts: vec!["untracked-deliverable.md".into()], focused_checks: vec![] };
        run.source_output = Some(noncode.clone());
        run.result.as_mut().unwrap().source_output = Some(noncode);
        assert_eq!(cleanup_owned_head(&identity, &run, None).unwrap(), identity.creation_tip);
        if let Integration::NotApplicable { result_message_id, .. } = &mut run.integration {
            *result_message_id = "different-result".into();
        }
        assert!(cleanup_owned_head(&identity, &run, None).is_err());
    }


    #[test]
    fn selected_clean_linked_worktree_requires_confirmation() {
        let operation = operation(true);
        let worktrees = worktrees(Some(false));
        let preview = preview(
            &WorkspaceTeardownPreviewRequest { workspace_id: "workspace".to_owned() },
            evidence(Some(&operation), &worktrees),
        ).expect("preview");
        assert_eq!(preview.operation_id, operation.operation_id);
        assert_eq!(preview.ownership, WorkspaceTeardownOwnership::SelectedLinked);
        assert_eq!(preview.dirty_state, WorkspaceTeardownDirtyState::Clean);
        assert_eq!(preview.required_confirmation.as_deref(), Some(REMOVE_WORKTREE_CONFIRMATION));
        assert_eq!(preview.discard_count, 0);
        assert!(preview.discard_paths.is_empty());
        assert_eq!(preview.discard_fingerprint, None);
        assert!(preview.blockers.is_empty());
        assert!(preview.allowed_actions.contains(&WorkspaceTeardownAction::RemoveOwnedWorktree));
    }

    #[test]
    fn borrowed_and_created_linked_worktrees_allow_reviewed_dirty_removal() {
        for created in [false, true] {
            for dirty in [false, true] {
                let operation = operation(created);
                let worktrees = worktrees(Some(dirty));
                let preview = preview(
                    &WorkspaceTeardownPreviewRequest { workspace_id: "workspace".to_owned() },
                    evidence(Some(&operation), &worktrees),
                ).expect("preview");
                assert_eq!(preview.ownership, WorkspaceTeardownOwnership::SelectedLinked);
                assert!(preview.allowed_actions.contains(&WorkspaceTeardownAction::RemoveOwnedWorktree));
                assert!(preview.blockers.is_empty());
                assert_eq!(preview.required_confirmation.as_deref(), Some(if dirty {
                    DISCARD_WORKTREE_CONFIRMATION
                } else {
                    REMOVE_WORKTREE_CONFIRMATION
                }));
                assert_eq!(preview.discard_count, u32::from(dirty));
                if dirty {
                    assert_eq!(preview.dirty_state, WorkspaceTeardownDirtyState::Dirty);
                    assert_eq!(preview.discard_paths, ["changed.txt"]);
                    assert_eq!(preview.discard_fingerprint, Some("a".repeat(64)));
                }
            }
        }
    }

    #[test]
    fn selected_linked_worktree_without_setup_uses_manual_operation_id() {
        let worktrees = worktrees(Some(false));
        let mut fresh = evidence(None, &worktrees);
        fresh.operation_id = "b21536c7-c65d-4f62-9c02-9ba239217147";
        let preview = preview(
            &WorkspaceTeardownPreviewRequest { workspace_id: "workspace".to_owned() },
            fresh.clone(),
        ).expect("preview without setup");
        assert_eq!(preview.operation_id, fresh.operation_id);
        assert_eq!(preview.ownership, WorkspaceTeardownOwnership::SelectedLinked);
        let request = WorkspaceTeardownExecuteRequest {
            operation_id: preview.operation_id,
            workspace_id: preview.workspace_id,
            expected_endpoint_identity: preview.endpoint_identity,
            expected_checkout_path: preview.checkout_path,
            action: WorkspaceTeardownAction::RemoveOwnedWorktree,
            confirmation: REMOVE_WORKTREE_CONFIRMATION.to_owned(),
            discard_fingerprint: None,
        };
        assert_eq!(command(&request, fresh).expect("manual removal command"),
            WorkspaceTeardownCommand::RemoveOwnedWorktree {
                workspace_id: "workspace".to_owned(),
                endpoint_identity: "endpoint".to_owned(),
                checkout_path: "/worktrees/task".to_owned(),
                force: false,
            });
    }

    #[test]
    fn borrowed_directory_can_close_but_never_remove_files() {
        let mut operation = operation(false);
        operation.plan.repository = None;
        operation.owned_resources[0].kind = "directory".to_owned();
        let mut directories = worktrees(None);
        directories[0].is_linked_worktree = false;
        let mut fresh = evidence(Some(&operation), &directories);
        fresh.repository_key = "";
        fresh.repository_root = "";
        let preview = preview(
            &WorkspaceTeardownPreviewRequest { workspace_id: "workspace".to_owned() },
            fresh,
        ).expect("preview");
        assert_eq!(preview.ownership, WorkspaceTeardownOwnership::BorrowedOpened);
        assert!(preview.allowed_actions.contains(&WorkspaceTeardownAction::CloseSpace));
        assert!(!preview.allowed_actions.contains(&WorkspaceTeardownAction::RemoveOwnedWorktree));
    }

    #[test]
    fn historical_directory_creation_does_not_override_selected_linked_worktree() {
        let mut operation = operation(true);
        operation.plan.ownership = WorkspaceCheckoutOwnership::BorrowedDirectory;
        operation.owned_resources[0].kind = "directory".to_owned();
        let worktrees = worktrees(Some(false));
        let preview = preview(
            &WorkspaceTeardownPreviewRequest { workspace_id: "workspace".to_owned() },
            evidence(Some(&operation), &worktrees),
        ).expect("preview");
        assert_eq!(preview.ownership, WorkspaceTeardownOwnership::SelectedLinked);
        assert!(preview.allowed_actions.contains(&WorkspaceTeardownAction::RemoveOwnedWorktree));
    }

    #[test]
    fn foreign_and_incomplete_setup_records_do_not_block_selected_linked_removal() {
        let worktrees = worktrees(Some(false));
        for mutation in 0..11 {
            let mut operation = operation(true);
            match mutation {
                0 => operation.session_id = "other".to_owned(),
                1 => operation.plan.session_id = "other".to_owned(),
                2 => operation.plan.endpoint_identity = "other".to_owned(),
                3 => operation.plan.repository.as_mut().unwrap().common_dir = "/other/.git".to_owned(),
                4 => operation.workspace_id = Some("other".to_owned()),
                5 => operation.owned_resources.clear(),
                6 => operation.owned_resources[0].path = "/other".to_owned(),
                7 => operation.owned_resources.push(operation.owned_resources[0].clone()),
                8 => operation.plan.repository = None,
                9 => operation.plan.checkout_path = "/other".to_owned(),
                10 => operation.state = WorkspaceOperationState::Planned,
                _ => unreachable!(),
            }
            let mut fresh = evidence(Some(&operation), &worktrees);
            fresh.operation_id = "a9b3e282-e372-4c49-bbd7-1586b6773754";
            let preview = preview(
                &WorkspaceTeardownPreviewRequest { workspace_id: "workspace".to_owned() },
                fresh,
            ).expect("preview");
            assert_eq!(preview.operation_id, operation.operation_id);
            assert_eq!(preview.ownership, WorkspaceTeardownOwnership::SelectedLinked);
            assert!(preview.allowed_actions.contains(&WorkspaceTeardownAction::RemoveOwnedWorktree), "mutation {mutation}");
            assert!(preview.blockers.is_empty(), "mutation {mutation}");
        }
    }

    #[test]
    fn pending_removal_receipt_blocks_close_and_redispatch_but_allows_reconciliation() {
        let operation = operation(true);
        let worktrees = worktrees(Some(false));
        for state in [TeardownReceiptState::Pending, TeardownReceiptState::OutcomeUnknown] {
            let mut receipt = receipt(state);
            receipt.session_id = "historical-session".to_owned();
            receipt.endpoint_identity = "historical-endpoint".to_owned();
            let mut fresh = evidence(Some(&operation), &worktrees);
            fresh.receipt = Some(&receipt);
            let preview = preview(
                &WorkspaceTeardownPreviewRequest { workspace_id: "workspace".to_owned() },
                fresh.clone(),
            ).expect("preview");
            assert_eq!(preview.allowed_actions, [WorkspaceTeardownAction::ReconcileRemoveOutcome]);
            let mut request = WorkspaceTeardownExecuteRequest {
                operation_id: preview.operation_id,
                workspace_id: preview.workspace_id,
                expected_endpoint_identity: preview.endpoint_identity,
                expected_checkout_path: preview.checkout_path,
                action: WorkspaceTeardownAction::RemoveOwnedWorktree,
                confirmation: REMOVE_WORKTREE_CONFIRMATION.to_owned(),
                discard_fingerprint: None,
            };
            assert_eq!(command(&request, fresh.clone()).expect_err("no replay").code, "teardown_not_allowed");
            request.action = WorkspaceTeardownAction::CloseSpace;
            assert_eq!(command(&request, fresh.clone()).expect_err("no close").code, "teardown_not_allowed");
            request.action = WorkspaceTeardownAction::ReconcileRemoveOutcome;
            assert_eq!(command(&request, fresh).expect("reconcile"),
                WorkspaceTeardownCommand::ReconcileRemoveOutcome { operation_id: operation.operation_id.clone() });
        }
    }

    #[test]
    fn missing_target_with_matching_manual_receipt_can_reconcile_without_setup() {
        for state in [TeardownReceiptState::Pending, TeardownReceiptState::OutcomeUnknown] {
            let mut fresh = evidence(None, &[]);
            let mut receipt = receipt(state);
            receipt.operation_id = fresh.operation_id.to_owned();
            receipt.manual = true;
            receipt.session_id = "old-session".to_owned();
            receipt.endpoint_identity = "old-endpoint".to_owned();
            fresh.receipt = Some(&receipt);
            let preview = preview(
                &WorkspaceTeardownPreviewRequest { workspace_id: "workspace".to_owned() },
                fresh.clone(),
            ).expect("missing-target preview");
            assert_eq!(preview.workspace_state, WorkspaceTeardownWorkspaceState::Missing);
            assert_eq!(preview.checkout_path, receipt.checkout_path);
            assert_eq!(preview.allowed_actions, [WorkspaceTeardownAction::ReconcileRemoveOutcome]);
            let request = WorkspaceTeardownExecuteRequest {
                operation_id: preview.operation_id,
                workspace_id: preview.workspace_id,
                expected_endpoint_identity: preview.endpoint_identity,
                expected_checkout_path: preview.checkout_path,
                action: WorkspaceTeardownAction::ReconcileRemoveOutcome,
                confirmation: String::new(),
                discard_fingerprint: None,
            };
            assert_eq!(command(&request, fresh).expect("missing-target reconcile"),
                WorkspaceTeardownCommand::ReconcileRemoveOutcome { operation_id: receipt.operation_id.clone() });
        }
    }

    #[test]
    fn completed_receipt_with_foreign_session_and_endpoint_allows_fresh_removal() {
        let operation = operation(true);
        let worktrees = worktrees(Some(false));
        let mut receipt = receipt(TeardownReceiptState::Completed);
        receipt.session_id = "other-session".to_owned();
        receipt.endpoint_identity = "other-endpoint".to_owned();
        let mut fresh = evidence(Some(&operation), &worktrees);
        fresh.receipt = Some(&receipt);
        let preview = preview(
            &WorkspaceTeardownPreviewRequest { workspace_id: "workspace".to_owned() },
            fresh,
        ).expect("preview");
        assert!(preview.allowed_actions.contains(&WorkspaceTeardownAction::RemoveOwnedWorktree));
        assert!(!preview.allowed_actions.contains(&WorkspaceTeardownAction::ReconcileRemoveOutcome));
        assert!(preview.blockers.is_empty());
    }

    #[test]
    fn mismatched_receipt_target_blocks_removal_and_pending_reconciliation() {
        let operation = operation(true);
        let worktrees = worktrees(Some(false));
        for state in [TeardownReceiptState::Pending, TeardownReceiptState::OutcomeUnknown, TeardownReceiptState::Completed] {
            for mutation in 0..5 {
                let mut receipt = receipt(state);
                match mutation {
                    0 => receipt.operation_id = "other".to_owned(),
                    1 => receipt.workspace_id = "other".to_owned(),
                    2 => receipt.checkout_path = "/worktrees/other".to_owned(),
                    3 => receipt.repository_key = "/other/.git".to_owned(),
                    4 => receipt.repository_root = "/other".to_owned(),
                    _ => unreachable!(),
                }
                let mut fresh = evidence(Some(&operation), &worktrees);
                fresh.receipt = Some(&receipt);
                let preview = preview(
                    &WorkspaceTeardownPreviewRequest { workspace_id: "workspace".to_owned() },
                    fresh,
                ).expect("preview");
                assert!(!preview.allowed_actions.contains(&WorkspaceTeardownAction::RemoveOwnedWorktree), "mutation {mutation}");
                assert!(!preview.allowed_actions.contains(&WorkspaceTeardownAction::ReconcileRemoveOutcome), "mutation {mutation}");
                if state != TeardownReceiptState::Completed {
                    assert!(!preview.allowed_actions.contains(&WorkspaceTeardownAction::CloseSpace));
                }
            }
        }
    }

    #[test]
    fn unknown_status_nonlinked_primary_missing_and_ambiguous_targets_block_removal() {
        for mutation in 0..8 {
            let mut worktrees = worktrees(Some(false)).to_vec();
            match mutation {
                0 => worktrees[0].status = None,
                1 => worktrees[0].is_linked_worktree = false,
                2 => worktrees.push(worktrees[0].clone()),
                3 => worktrees[0].is_primary = true,
                4 => worktrees[0].checkout_path = "/repository".to_owned(),
                5 => worktrees.clear(),
                6 => worktrees[0].checkout_path = "/".to_owned(),
                7 => worktrees[0].checkout_path = "relative/task".to_owned(),
                _ => unreachable!(),
            }
            let fresh = evidence(None, &worktrees);
            let preview = preview(
                &WorkspaceTeardownPreviewRequest { workspace_id: "workspace".to_owned() },
                fresh.clone(),
            ).expect("preview");
            assert!(!preview.allowed_actions.contains(&WorkspaceTeardownAction::RemoveOwnedWorktree), "mutation {mutation}");
            assert_eq!(preview.required_confirmation, None);
            let request = WorkspaceTeardownExecuteRequest {
                operation_id: preview.operation_id,
                workspace_id: preview.workspace_id,
                expected_endpoint_identity: preview.endpoint_identity,
                expected_checkout_path: preview.checkout_path,
                action: WorkspaceTeardownAction::RemoveOwnedWorktree,
                confirmation: REMOVE_WORKTREE_CONFIRMATION.to_owned(),
                discard_fingerprint: None,
            };
            assert_eq!(command(&request, fresh).expect_err("unsafe target").code, "teardown_not_allowed", "mutation {mutation}");
        }
    }

    #[test]
    fn unverified_repository_identity_blocks_selected_linked_removal() {
        let worktrees = worktrees(Some(false));
        for mutation in 0..3 {
            let mut fresh = evidence(None, &worktrees);
            match mutation {
                0 => fresh.repository_key = "",
                1 => fresh.repository_root = "",
                2 => fresh.repository_root = "relative/repository",
                _ => unreachable!(),
            }
            let preview = preview(
                &WorkspaceTeardownPreviewRequest { workspace_id: "workspace".to_owned() },
                fresh,
            ).expect("preview");
            assert!(!preview.allowed_actions.contains(&WorkspaceTeardownAction::RemoveOwnedWorktree));
        }
    }

    #[test]
    fn removal_requires_matching_full_socket_endpoint_identity() {
        let endpoint = "unix-socket:/tmp/cockpit-fixture/config/herdr/sessions/fixture/herdr.sock:pid=939432:uid=1000:gid=1000:start=5501986";
        let operation = operation(true);
        let worktrees = worktrees(Some(false));
        let fresh = || {
            let mut fresh = evidence(Some(&operation), &worktrees);
            fresh.endpoint_identity = endpoint;
            fresh
        };
        let mut request = WorkspaceTeardownExecuteRequest {
            operation_id: operation.operation_id.clone(),
            workspace_id: "workspace".to_owned(),
            expected_endpoint_identity: endpoint.to_owned(),
            expected_checkout_path: "/worktrees/task".to_owned(),
            action: WorkspaceTeardownAction::RemoveOwnedWorktree,
            confirmation: REMOVE_WORKTREE_CONFIRMATION.to_owned(),
            discard_fingerprint: None,
        };
        assert!(matches!(command(&request, fresh()).expect("full endpoint command"),
            WorkspaceTeardownCommand::RemoveOwnedWorktree { endpoint_identity, force: false, .. } if endpoint_identity == endpoint));
        for changed in [
            endpoint.replace("pid=939432", "pid=939433"),
            endpoint.replace("uid=1000", "uid=1001"),
            endpoint.replace("gid=1000", "gid=1001"),
            endpoint.replace("start=5501986", "start=5501987"),
            endpoint.replace("fixture/herdr.sock", "other/herdr.sock"),
        ] {
            request.expected_endpoint_identity = changed;
            assert_eq!(command(&request, fresh()).expect_err("foreign endpoint").code, "stale_identity");
        }
    }

    #[test]
    fn clean_command_checks_confirmation_and_exact_reviewed_identity() {
        let operation = operation(true);
        let worktrees = worktrees(Some(false));
        let mut request = WorkspaceTeardownExecuteRequest {
            operation_id: "operation".to_owned(),
            workspace_id: "workspace".to_owned(),
            expected_endpoint_identity: "endpoint".to_owned(),
            expected_checkout_path: "/worktrees/task".to_owned(),
            action: WorkspaceTeardownAction::RemoveOwnedWorktree,
            confirmation: String::new(),
            discard_fingerprint: None,
        };
        assert_eq!(command(&request, evidence(Some(&operation), &worktrees)).expect_err("confirmation").code, "confirmation_required");
        request.confirmation = REMOVE_WORKTREE_CONFIRMATION.to_owned();
        assert!(matches!(command(&request, evidence(Some(&operation), &worktrees)).expect("command"),
            WorkspaceTeardownCommand::RemoveOwnedWorktree { force: false, .. }));
        request.operation_id = "other".to_owned();
        assert_eq!(command(&request, evidence(Some(&operation), &worktrees)).expect_err("operation identity").code, "stale_identity");
        request.operation_id = "operation".to_owned();
        request.expected_checkout_path = "/worktrees/other".to_owned();
        assert_eq!(command(&request, evidence(Some(&operation), &worktrees)).expect_err("checkout identity").code, "stale_identity");
        request.expected_checkout_path = "/worktrees/task/".to_owned();
        assert_eq!(command(&request, evidence(Some(&operation), &worktrees)).expect_err("exact checkout spelling").code, "stale_identity");
        request.expected_checkout_path = "/worktrees/task".to_owned();
        request.discard_fingerprint = Some("a".repeat(64));
        assert_eq!(command(&request, evidence(Some(&operation), &worktrees)).expect_err("dirty review became clean").code, "stale_identity");
    }

    #[test]
    fn dirty_command_requires_exact_discard_phrase_and_current_raw_porcelain_fingerprint() {
        let mut worktrees = worktrees(Some(true));
        let mut request = WorkspaceTeardownExecuteRequest {
            operation_id: evidence(None, &worktrees).operation_id.to_owned(),
            workspace_id: "workspace".to_owned(),
            expected_endpoint_identity: "endpoint".to_owned(),
            expected_checkout_path: "/worktrees/task".to_owned(),
            action: WorkspaceTeardownAction::RemoveOwnedWorktree,
            confirmation: String::new(),
            discard_fingerprint: Some("a".repeat(64)),
        };
        for phrase in ["", REMOVE_WORKTREE_CONFIRMATION, "DISCARD AND REMOVE ", "discard and remove"] {
            request.confirmation = phrase.to_owned();
            assert_eq!(command(&request, evidence(None, &worktrees)).expect_err("exact dirty confirmation").code, "confirmation_required");
        }
        request.confirmation = DISCARD_WORKTREE_CONFIRMATION.to_owned();
        request.discard_fingerprint = None;
        assert_eq!(command(&request, evidence(None, &worktrees)).expect_err("missing fingerprint").code, "stale_identity");
        request.discard_fingerprint = Some("c".repeat(64));
        assert_eq!(command(&request, evidence(None, &worktrees)).expect_err("wrong fingerprint").code, "stale_identity");
        request.discard_fingerprint = Some("a".repeat(64));
        assert!(matches!(command(&request, evidence(None, &worktrees)).expect("reviewed discard"),
            WorkspaceTeardownCommand::RemoveOwnedWorktree { force: true, .. }));
        // Identical displayed paths and count still require a new review when
        // the raw porcelain bytes (for example the status columns) change.
        worktrees[0].status.as_mut().unwrap().porcelain_sha256 = "d".repeat(64);
        assert_eq!(command(&request, evidence(None, &worktrees)).expect_err("changed raw status").code, "stale_identity");
        request.discard_fingerprint = Some("d".repeat(64));
        assert!(matches!(command(&request, evidence(None, &worktrees)).expect("refreshed discard"),
            WorkspaceTeardownCommand::RemoveOwnedWorktree { force: true, .. }));
    }
}
