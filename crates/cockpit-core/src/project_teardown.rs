use cockpit_protocol::project_teardown::{
    WorkspaceTeardownAction, WorkspaceTeardownDirtyState,
    WorkspaceTeardownExecuteRequest, WorkspaceTeardownOwnership, WorkspaceTeardownPreview,
    WorkspaceTeardownPreviewRequest, WorkspaceTeardownWorkspaceState,
};
use cockpit_protocol::projects::{
    WorkspaceCheckoutOwnership, WorkspaceOperation, WorkspaceOperationState,
};

use crate::InspectionError;
use crate::project_store::{TeardownReceipt, TeardownReceiptState};

pub const REMOVE_WORKTREE_CONFIRMATION: &str = "REMOVE WORKTREE";
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

pub(crate) fn integrated_owned_head<'a>(
    identity: &'a cockpit_protocol::orchestration::OwnedGitIdentity,
    run: &'a cockpit_protocol::orchestration::Run,
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
    let Integration::Integrated { accepted_result_message_id, accepted_task_revision, source,
        target_repository_key, method, mappings, verified_at, .. } = &run.integration else {
        return Err(InspectionError::new("waiting_integration", "accepted Git output has not been validated as integrated"));
    };
    let SourceOutput::Code { repository_key, checkout_key, head_commit, commits,
        result_message_id, task_revision, .. } = source else {
        return Err(InspectionError::new("integration_unverified", "Git cleanup needs a validated code integration record"));
    };
    // Source Results use the canonical project key; the owned creation journal
    // uses Herdr's raw repository/common-directory key. Join both contracts
    // through the recorded Git project, never compare a hash to a pathname.
    let repository_matches = run.project.as_ref().is_some_and(|project|
        repository_key == &project.key
            && matches!(&project.source, ProjectSource::Git { common_dir }
                if common_dir == &identity.common_dir && common_dir == &identity.repository_key));
    if run.stage != RunStage::Closed || run.close_reason != Some(CloseReason::Accepted)
        || verified_at.is_empty()
        || !repository_matches || target_repository_key != repository_key
        || checkout_key != &identity.checkout_key
        || run.source_output.as_ref() != Some(source)
        || run.result.as_ref().map(|r| r.message_id.as_str()) != Some(result_message_id.as_str())
        || accepted_result_message_id != result_message_id || accepted_task_revision != task_revision
    {
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



/// Fresh adapter evidence needed to preview a teardown. This deliberately
/// carries only provenance and Git status: the integration owner must obtain
/// it from Herdr and a bounded read-only Git inspection immediately before
/// both preview and execution.
#[derive(Debug, Clone)]
pub struct WorkspaceTeardownEvidence<'a> {
    pub operation: &'a WorkspaceOperation,
    pub session_id: &'a str,
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
    /// `None` means the bounded status inspection failed or was unavailable;
    /// that state blocks removal.
    pub dirty: Option<bool>,
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
    let operation = evidence.operation;
    let checkout_path = operation.plan.checkout_path.clone();
    let mut matching_worktrees = evidence
        .worktrees
        .iter()
        .filter(|worktree| worktree.checkout_path == checkout_path);
    let (workspace_state, worktree) = if operation.workspace_id.as_deref()
        != Some(request.workspace_id.as_str())
    {
        (WorkspaceTeardownWorkspaceState::Ambiguous, None)
    } else {
        match (matching_worktrees.next(), matching_worktrees.next()) {
            (None, _) => (WorkspaceTeardownWorkspaceState::Missing, None),
            (Some(worktree), None)
                if worktree.open_workspace_id.as_deref() == Some(request.workspace_id.as_str()) =>
            {
                (WorkspaceTeardownWorkspaceState::Live, Some(worktree))
            }
            _ => (WorkspaceTeardownWorkspaceState::Ambiguous, None),
        }
    };
    let ownership = ownership(operation);
    let is_linked_worktree = worktree.is_some_and(|value| value.is_linked_worktree);
    let dirty_state = match worktree.and_then(|value| value.dirty) {
        Some(false) => WorkspaceTeardownDirtyState::Clean,
        Some(true) => WorkspaceTeardownDirtyState::Dirty,
        None => WorkspaceTeardownDirtyState::Unknown,
    };

    let mut blockers = Vec::new();
    let warnings = Vec::new();
    let provenance_confirmed = operation.operation_id == operation.plan.operation_id
        && evidence.session_id == operation.session_id
        && evidence.session_id == operation.plan.session_id
        && evidence.endpoint_identity == operation.plan.endpoint_identity
        && if operation.plan.mode == cockpit_protocol::projects::WorkspaceSetupMode::Open {
            true
        } else {
            operation.plan.repository.as_ref().map_or(
                evidence.repository_key.is_empty() && evidence.repository_root.is_empty(),
                |repository| {
                    evidence.repository_key == repository.common_dir
                        && evidence.repository_root == repository.root
                },
            )
        };
    if !provenance_confirmed {
        blockers.push("fresh Herdr provenance differs from the reviewed operation".to_owned());
    }
    if operation.plan.ownership
        == cockpit_protocol::projects::WorkspaceCheckoutOwnership::OwnedWorktree
    {
        if workspace_state != WorkspaceTeardownWorkspaceState::Live {
            blockers.push("the exact worktree is not live in the requested workspace".to_owned());
        }
        if !is_linked_worktree {
            blockers.push("the target is not a linked worktree".to_owned());
        }
        match dirty_state {
            WorkspaceTeardownDirtyState::Dirty => {
                blockers.push("the worktree has tracked or untracked changes".to_owned())
            }
            WorkspaceTeardownDirtyState::Unknown => {
                blockers.push("worktree status could not be confirmed".to_owned())
            }
            WorkspaceTeardownDirtyState::Clean => {}
        }
    }
    match ownership {
        WorkspaceTeardownOwnership::OwnedCreated => {}
        WorkspaceTeardownOwnership::BorrowedOpened => {
            blockers.push("the checkout was opened as borrowed and cannot be removed".to_owned())
        }
        WorkspaceTeardownOwnership::Unknown => {
            blockers.push("owned worktree provenance is incomplete".to_owned())
        }
    }

    let receipt = evidence.receipt;
    let removal_pending = receipt.is_some_and(|receipt| {
        matches!(receipt.state, TeardownReceiptState::Pending | TeardownReceiptState::OutcomeUnknown)
    });
    let receipt_confirmed = receipt.is_some_and(|receipt| receipt_matches(receipt, operation, &evidence, request));
    if receipt.is_some() && !receipt_confirmed {
        blockers.push("the prior teardown receipt belongs to a different target identity".to_owned());
    }
    if removal_pending {
        blockers.push(
            "a prior worktree removal has an unknown outcome; reconcile it before retrying"
                .to_owned(),
        );
    }

    let mut allowed_actions = Vec::new();
    if workspace_state == WorkspaceTeardownWorkspaceState::Live && provenance_confirmed {
        allowed_actions.push(WorkspaceTeardownAction::CloseSpace);
    }
    if removal_pending && provenance_confirmed && receipt_confirmed {
        allowed_actions.push(WorkspaceTeardownAction::ReconcileRemoveOutcome);
    }
    let removal_allowed = blockers.is_empty() && !removal_pending;
    if removal_allowed {
        allowed_actions.push(WorkspaceTeardownAction::RemoveOwnedWorktree);
    }

    Ok(WorkspaceTeardownPreview {
        operation_id: operation.operation_id.clone(),
        workspace_id: request.workspace_id.clone(),
        endpoint_identity: evidence.endpoint_identity.to_owned(),
        repository_key: (!evidence.repository_key.is_empty())
            .then(|| evidence.repository_key.to_owned()),
        repository_root: (!evidence.repository_root.is_empty())
            .then(|| evidence.repository_root.to_owned()),
        checkout_path,
        ownership,
        workspace_state,
        is_linked_worktree,
        dirty_state,
        allowed_actions,
        blockers,
        warnings,
        required_confirmation: if removal_allowed {
            Some(REMOVE_WORKTREE_CONFIRMATION.to_owned())
        } else {
            None
        },
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
            if request.confirmation != REMOVE_WORKTREE_CONFIRMATION {
                return Err(InspectionError::new(
                    "confirmation_required",
                    "type the required confirmation before removing this worktree",
                ));
            }
            Ok(WorkspaceTeardownCommand::RemoveOwnedWorktree {
                workspace_id: preview.workspace_id,
                endpoint_identity: preview.endpoint_identity,
                force: false,
                checkout_path: preview.checkout_path,
            })
        }
    }
}

fn receipt_matches(
    receipt: &TeardownReceipt,
    operation: &WorkspaceOperation,
    evidence: &WorkspaceTeardownEvidence<'_>,
    request: &WorkspaceTeardownPreviewRequest,
) -> bool {
    receipt.operation_id == operation.operation_id
        && receipt.operation_id == operation.plan.operation_id
        && receipt.workspace_id == request.workspace_id
        && operation.workspace_id.as_deref() == Some(receipt.workspace_id.as_str())
        && receipt.session_id == operation.session_id
        && receipt.session_id == operation.plan.session_id
        && evidence.session_id == receipt.session_id
        && receipt.repository_key == evidence.repository_key
        && receipt.repository_root == evidence.repository_root
        && receipt.endpoint_identity == operation.plan.endpoint_identity
        && receipt.checkout_path == operation.plan.checkout_path
        && evidence.endpoint_identity == receipt.endpoint_identity
        && if operation.plan.mode == cockpit_protocol::projects::WorkspaceSetupMode::Open {
            true
        } else {
            operation.plan.repository.as_ref().map_or(
                evidence.repository_key.is_empty() && evidence.repository_root.is_empty(),
                |repository| {
                    evidence.repository_key == repository.common_dir
                        && evidence.repository_root == repository.root
                },
            )
        }
}


fn ownership(operation: &WorkspaceOperation) -> WorkspaceTeardownOwnership {
    let worktree = operation.owned_resources.iter().filter(|resource| {
        resource.kind == "worktree" && resource.path == operation.plan.checkout_path
    });
    if operation.state != WorkspaceOperationState::Completed {
        return WorkspaceTeardownOwnership::Unknown;
    }
    if operation.plan.mode == cockpit_protocol::projects::WorkspaceSetupMode::Open
        || operation.plan.ownership != WorkspaceCheckoutOwnership::OwnedWorktree
    {
        return WorkspaceTeardownOwnership::BorrowedOpened;
    }
    let mut worktree = worktree;
    match (worktree.next(), worktree.next()) {
        (Some(resource), None) if !resource.created_by_operation => {
            WorkspaceTeardownOwnership::BorrowedOpened
        }
        (Some(resource), None) if resource.created_by_operation => {
            WorkspaceTeardownOwnership::OwnedCreated
        }
        _ => WorkspaceTeardownOwnership::Unknown,
    }
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
        WorkspaceSetupPlan,
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
        operation: &'a WorkspaceOperation,
        worktrees: &'a [WorkspaceTeardownWorktree],
    ) -> WorkspaceTeardownEvidence<'a> {
        WorkspaceTeardownEvidence {
            operation,
            session_id: "session",
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
            dirty,
        }]
    }

    fn receipt(state: TeardownReceiptState) -> TeardownReceipt {
        TeardownReceipt {
            operation_id: "operation".to_owned(),
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
        assert!(integrated_owned_head(&identity, &run).is_err());
        run.integration = Integration::Integrated {
            operation_id:"integration".into(), accepted_result_message_id:"result".into(),
            accepted_task_revision:"task-revision".into(), source:source.clone(),
            target_repository_key:canonical_project_key.clone(), target_ref:"refs/heads/main".into(),
            target_commit:"c".repeat(40), method:IntegrationMethod::RecordedAdaptation,
            mappings:vec![], checks:vec![], integrator_run_id:"integrator".into(), verified_at:"verified".into(),
        };
        assert!(integrated_owned_head(&identity, &run).is_err());
        if let Integration::Integrated { mappings, .. } = &mut run.integration {
            mappings.push(IntegrationMapping {
                source_commits:vec!["b".repeat(40)], target_commits:vec!["c".repeat(40)],
                source_paths:vec!["source.rs".into()], target_paths:vec!["target.rs".into()], explanation:Some("validated adaptation".into()),
            });
        }
        assert_eq!(integrated_owned_head(&identity, &run).unwrap(), "b".repeat(40));
        assert_ne!(canonical_project_key, identity.repository_key, "canonical project hash is not the raw Git key");
        let mut wrong_source_key = run.clone();
        if let Integration::Integrated { source: SourceOutput::Code { repository_key, .. }, .. } = &mut wrong_source_key.integration {
            *repository_key = identity.repository_key.clone();
        }
        if let Integration::Integrated { target_repository_key, source, .. } = &mut wrong_source_key.integration {
            *target_repository_key = identity.repository_key.clone();
            wrong_source_key.source_output = Some(source.clone());
            wrong_source_key.result.as_mut().unwrap().source_output = Some(source.clone());
        }
        assert!(integrated_owned_head(&identity, &wrong_source_key).is_err());
        let mut wrong_target_key = run.clone();
        if let Integration::Integrated { target_repository_key, .. } = &mut wrong_target_key.integration {
            *target_repository_key = identity.repository_key.clone();
        }
        assert!(integrated_owned_head(&identity, &wrong_target_key).is_err());
        let mut wrong_project_key = run.clone();
        wrong_project_key.project.as_mut().unwrap().key = "e".repeat(64);
        assert!(integrated_owned_head(&identity, &wrong_project_key).is_err());
        let mut wrong_common_dir = run.clone();
        wrong_common_dir.project.as_mut().unwrap().source = cockpit_protocol::orchestration::ProjectSource::Git {
            common_dir: "/foreign/.git".into() };
        assert!(integrated_owned_head(&identity, &wrong_common_dir).is_err());
        let mut wrong_raw_key = identity.clone();
        wrong_raw_key.repository_key = "/foreign/.git".into();
        assert!(integrated_owned_head(&wrong_raw_key, &run).is_err());
        if let Integration::Integrated { verified_at, .. } = &mut run.integration { verified_at.clear(); }
        assert!(integrated_owned_head(&identity, &run).is_err());
        // The exemption does not apply to code output.
        run.integration = Integration::NotApplicable { reason: "non-code deliverable".into(), result_message_id: "result".into() };
        assert!(integrated_owned_head(&identity, &run).is_err());
        let noncode = SourceOutput::NonCode { result_message_id: "result".into(), task_revision: "task-revision".into(),
            artifacts: vec!["untracked-deliverable.md".into()], focused_checks: vec![] };
        run.source_output = Some(noncode.clone());
        run.result.as_mut().unwrap().source_output = Some(noncode);
        assert_eq!(integrated_owned_head(&identity, &run).unwrap(), identity.creation_tip);
        if let Integration::NotApplicable { result_message_id, .. } = &mut run.integration {
            *result_message_id = "different-result".into();
        }
        assert!(integrated_owned_head(&identity, &run).is_err());
    }


    #[test]
    fn owned_clean_linked_worktree_requires_confirmation() {
        let operation = operation(true);
        let worktrees = worktrees(Some(false));
        let preview = preview(
            &WorkspaceTeardownPreviewRequest {
                workspace_id: "workspace".to_owned(),
            },
            evidence(&operation, &worktrees),
        )
        .expect("preview");
        assert_eq!(preview.ownership, WorkspaceTeardownOwnership::OwnedCreated);
        assert_eq!(
            preview.required_confirmation.as_deref(),
            Some(REMOVE_WORKTREE_CONFIRMATION)
        );
    }

    #[test]
    fn borrowed_or_dirty_worktrees_are_not_removable() {
        let borrowed = operation(false);
        let clean_worktrees = worktrees(Some(false));
        let borrowed_preview = preview(
            &WorkspaceTeardownPreviewRequest {
                workspace_id: "workspace".to_owned(),
            },
            evidence(&borrowed, &clean_worktrees),
        )
        .expect("preview");
        assert!(
            !borrowed_preview
                .allowed_actions
                .contains(&WorkspaceTeardownAction::RemoveOwnedWorktree)
        );

        let owned = operation(true);
        let dirty_worktrees = worktrees(Some(true));
        let dirty_preview = preview(
            &WorkspaceTeardownPreviewRequest {
                workspace_id: "workspace".to_owned(),
            },
            evidence(&owned, &dirty_worktrees),
        )
        .expect("preview");
        assert!(
            !dirty_preview
                .allowed_actions
                .contains(&WorkspaceTeardownAction::RemoveOwnedWorktree)
        );
    }

    #[test]
    fn borrowed_directory_can_close_but_never_remove_files() {
        let mut operation = operation(false);
        operation.plan.repository = None;
        operation.plan.ownership =
            cockpit_protocol::projects::WorkspaceCheckoutOwnership::BorrowedDirectory;
        operation.owned_resources[0].kind = "directory".to_owned();
        let directories = [WorkspaceTeardownWorktree {
            checkout_path: "/worktrees/task".to_owned(),
            open_workspace_id: Some("workspace".to_owned()),
            is_linked_worktree: false,
            dirty: None,
        }];
        let preview = preview(
            &WorkspaceTeardownPreviewRequest {
                workspace_id: "workspace".to_owned(),
            },
            WorkspaceTeardownEvidence {
                operation: &operation,
                session_id: "session",
                endpoint_identity: "endpoint",
                repository_key: "",
                repository_root: "",
                worktrees: &directories,
                receipt: None,
            },
        )
        .expect("preview");
        assert!(
            preview
                .allowed_actions
                .contains(&WorkspaceTeardownAction::CloseSpace)
        );
        assert!(
            !preview
                .allowed_actions
                .contains(&WorkspaceTeardownAction::RemoveOwnedWorktree)
        );
    }

    #[test]
    fn created_directory_receipt_never_authorizes_worktree_removal() {
        let mut operation = operation(true);
        operation.plan.ownership = WorkspaceCheckoutOwnership::BorrowedDirectory;
        operation.owned_resources[0].kind = "directory".to_owned();
        let worktrees = worktrees(Some(false));
        let preview = preview(
            &WorkspaceTeardownPreviewRequest {
                workspace_id: "workspace".to_owned(),
            },
            evidence(&operation, &worktrees),
        )
        .expect("preview");

        assert_eq!(
            preview.ownership,
            WorkspaceTeardownOwnership::BorrowedOpened
        );
        assert!(
            !preview
                .allowed_actions
                .contains(&WorkspaceTeardownAction::RemoveOwnedWorktree)
        );
    }

    #[test]
    fn pending_removal_receipt_blocks_redispatch_and_requires_reconciliation() {
        let operation = operation(true);
        let worktrees = worktrees(Some(false));
        let receipt = receipt(TeardownReceiptState::OutcomeUnknown);
        let preview = preview(
            &WorkspaceTeardownPreviewRequest {
                workspace_id: "workspace".to_owned(),
            },
            WorkspaceTeardownEvidence {
                operation: &operation,
                session_id: "session",
                endpoint_identity: "endpoint",
                repository_key: "/repository/.git",
                repository_root: "/repository",
                worktrees: &worktrees,
                receipt: Some(&receipt),
            },
        )
        .expect("preview");
        assert!(
            !preview
                .allowed_actions
                .contains(&WorkspaceTeardownAction::RemoveOwnedWorktree)
        );
        assert!(
            preview
                .allowed_actions
                .contains(&WorkspaceTeardownAction::ReconcileRemoveOutcome)
        );
    }

    #[test]
    fn reconciled_retained_worktree_requires_new_review_before_removal() {
        let operation = operation(true);
        let worktrees = worktrees(Some(false));
        let receipt = receipt(TeardownReceiptState::Completed);
        let mut fresh = evidence(&operation, &worktrees);
        fresh.receipt = Some(&receipt);
        let preview = preview(&WorkspaceTeardownPreviewRequest {
            workspace_id: "workspace".to_owned(),
        }, fresh).expect("preview");
        assert!(preview.allowed_actions.contains(&WorkspaceTeardownAction::RemoveOwnedWorktree));
        assert!(!preview.allowed_actions.contains(&WorkspaceTeardownAction::ReconcileRemoveOutcome));
    }

    #[test]
    fn stale_completed_receipt_blocks_fresh_removal() {
        let operation = operation(true);
        let worktrees = worktrees(Some(false));
        let mut receipt = receipt(TeardownReceiptState::Completed);
        receipt.session_id = "other".to_owned();
        let mut fresh = evidence(&operation, &worktrees);
        fresh.receipt = Some(&receipt);
        let preview = preview(&WorkspaceTeardownPreviewRequest {
            workspace_id: "workspace".to_owned(),
        }, fresh).expect("preview");
        assert!(!preview.allowed_actions.contains(&WorkspaceTeardownAction::RemoveOwnedWorktree));
    }

    #[test]
    fn identity_changes_and_incomplete_worktree_receipts_block_removal() {
        let worktrees = worktrees(Some(false));
        for mutation in 0..8 {
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
                _ => unreachable!(),
            }
            let preview = preview(&WorkspaceTeardownPreviewRequest {
                workspace_id: "workspace".to_owned(),
            }, evidence(&operation, &worktrees)).expect("preview");
            assert!(!preview.allowed_actions.contains(&WorkspaceTeardownAction::RemoveOwnedWorktree), "mutation {mutation}");
        }
    }

    #[test]
    fn mismatched_unknown_receipt_blocks_both_redispatch_and_reconciliation() {
        let operation = operation(true);
        let worktrees = worktrees(Some(false));
        let mut receipt = receipt(TeardownReceiptState::OutcomeUnknown);
        receipt.session_id = "other".to_owned();
        let mut fresh = evidence(&operation, &worktrees);
        fresh.receipt = Some(&receipt);
        let preview = preview(&WorkspaceTeardownPreviewRequest {
            workspace_id: "workspace".to_owned(),
        }, fresh).expect("preview");
        assert!(!preview.allowed_actions.contains(&WorkspaceTeardownAction::RemoveOwnedWorktree));
        assert!(!preview.allowed_actions.contains(&WorkspaceTeardownAction::ReconcileRemoveOutcome));
    }

    #[test]
    fn unknown_status_nonlinked_and_ambiguous_worktree_block_removal() {
        let operation = operation(true);
        for mutation in 0..3 {
            let mut worktrees = worktrees(Some(false)).to_vec();
            match mutation {
                0 => worktrees[0].dirty = None,
                1 => worktrees[0].is_linked_worktree = false,
                2 => worktrees.push(worktrees[0].clone()),
                _ => unreachable!(),
            }
            let preview = preview(&WorkspaceTeardownPreviewRequest {
                workspace_id: "workspace".to_owned(),
            }, evidence(&operation, &worktrees)).expect("preview");
            assert!(!preview.allowed_actions.contains(&WorkspaceTeardownAction::RemoveOwnedWorktree));
        }
    }

    #[test]
    fn removal_requires_matching_full_socket_endpoint_identity() {
        let endpoint = "unix-socket:/tmp/cockpit-fixture/config/herdr/sessions/fixture/herdr.sock:pid=939432:uid=1000:gid=1000:start=5501986";
        let mut operation = operation(true);
        operation.plan.endpoint_identity = endpoint.to_owned();
        let worktrees = worktrees(Some(false));
        let fresh = || {
            let mut fresh = evidence(&operation, &worktrees);
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
        };
        assert!(matches!(command(&request, fresh()).expect("full endpoint command"),
            WorkspaceTeardownCommand::RemoveOwnedWorktree { endpoint_identity, force: false, .. } if endpoint_identity == endpoint));
        request.expected_endpoint_identity = endpoint.replace("pid=939432", "pid=939433");
        assert_eq!(command(&request, fresh()).expect_err("foreign endpoint").code, "stale_identity");
    }

    #[test]
    fn command_checks_confirmation_and_exact_reviewed_identity() {
        let operation = operation(true);
        let worktrees = worktrees(Some(false));
        let mut request = WorkspaceTeardownExecuteRequest {
            operation_id: "operation".to_owned(),
            workspace_id: "workspace".to_owned(),
            expected_endpoint_identity: "endpoint".to_owned(),
            expected_checkout_path: "/worktrees/task".to_owned(),
            action: WorkspaceTeardownAction::RemoveOwnedWorktree,
            confirmation: String::new(),
        };
        assert_eq!(command(&request, evidence(&operation, &worktrees)).unwrap_err().code, "confirmation_required");
        request.confirmation = REMOVE_WORKTREE_CONFIRMATION.to_owned();
        assert!(matches!(command(&request, evidence(&operation, &worktrees)).expect("command"),
            WorkspaceTeardownCommand::RemoveOwnedWorktree { force: false, .. }));
        request.operation_id = "other".to_owned();
        assert_eq!(command(&request, evidence(&operation, &worktrees)).unwrap_err().code, "stale_identity");
    }
}
