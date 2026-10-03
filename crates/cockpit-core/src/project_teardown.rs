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
