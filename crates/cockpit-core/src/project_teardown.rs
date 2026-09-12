use cockpit_protocol::project_teardown::{
    WorkspaceTeardownAction, WorkspaceTeardownCompanionState, WorkspaceTeardownDirtyState,
    WorkspaceTeardownExecuteRequest, WorkspaceTeardownOwnership, WorkspaceTeardownPreview,
    WorkspaceTeardownPreviewRequest, WorkspaceTeardownWorkspaceState,
};
use cockpit_protocol::projects::{
    WorkspaceCheckoutOwnership, WorkspaceOperation, WorkspaceOperationState,
};

use crate::InspectionError;
use crate::project_store::{CompanionManifest, TeardownReceipt, TeardownReceiptState};

pub const REMOVE_WORKTREE_CONFIRMATION: &str = "REMOVE WORKTREE";
pub const REMOVE_COMPANION_CONFIRMATION: &str = "REMOVE COMPANION";

/// Fresh adapter evidence needed to preview a teardown. This deliberately
/// carries only provenance and Git status: the integration owner must obtain
/// it from Herdr and a bounded read-only Git inspection immediately before
/// both preview and execution.
#[derive(Debug, Clone)]
pub struct WorkspaceTeardownEvidence<'a> {
    pub operation: &'a WorkspaceOperation,
    pub companion: Option<&'a CompanionManifest>,
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
        companion_path: String,
    },
    ForgetAssociation {
        operation_id: String,
    },
    ReconcileRemoveOutcome {
        operation_id: String,
    },
    RemoveOrphanedCompanion {
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
    let matching_worktrees = evidence
        .worktrees
        .iter()
        .filter(|worktree| worktree.checkout_path == checkout_path)
        .collect::<Vec<_>>();
    let (workspace_state, worktree) = if operation.workspace_id.as_deref()
        != Some(request.workspace_id.as_str())
    {
        (WorkspaceTeardownWorkspaceState::Ambiguous, None)
    } else {
        match matching_worktrees.as_slice() {
            [] => (WorkspaceTeardownWorkspaceState::Missing, None),
            [worktree]
                if worktree.open_workspace_id.as_deref() == Some(request.workspace_id.as_str()) =>
            {
                (WorkspaceTeardownWorkspaceState::Live, Some(*worktree))
            }
            [_] | [_, _, ..] => (WorkspaceTeardownWorkspaceState::Ambiguous, None),
        }
    };
    let companion_state = companion_state(
        operation,
        evidence.companion,
        evidence.endpoint_identity,
        &request.workspace_id,
    );
    let ownership = ownership(operation, companion_state);
    let is_linked_worktree = worktree.is_some_and(|value| value.is_linked_worktree);
    let dirty_state = match worktree.and_then(|value| value.dirty) {
        Some(false) => WorkspaceTeardownDirtyState::Clean,
        Some(true) => WorkspaceTeardownDirtyState::Dirty,
        None => WorkspaceTeardownDirtyState::Unknown,
    };
    let companion_path = (companion_state == WorkspaceTeardownCompanionState::Owned)
        .then(|| operation.plan.companion_path.clone());

    let mut blockers = Vec::new();
    let mut warnings = Vec::new();
    let provenance_confirmed = evidence.endpoint_identity == operation.plan.endpoint_identity
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
        WorkspaceTeardownOwnership::Foreign => {
            blockers.push("the companion association belongs to another operation".to_owned())
        }
        WorkspaceTeardownOwnership::Unknown => {
            blockers.push("owned worktree and companion provenance is incomplete".to_owned())
        }
    }
    if companion_state == WorkspaceTeardownCompanionState::Missing {
        warnings
            .push("the companion is missing; no companion cleanup will be attempted".to_owned());
    }
    if workspace_state == WorkspaceTeardownWorkspaceState::Missing
        && companion_state == WorkspaceTeardownCompanionState::Owned
    {
        warnings.push(
            "the owned companion is orphaned; retain it or explicitly reattach it".to_owned(),
        );
    }

    let receipt = evidence.receipt.filter(|receipt| {
        receipt.state != TeardownReceiptState::Completed
            && receipt_matches(receipt, operation, &evidence, request)
    });
    let removal_pending = receipt.is_some_and(|receipt| {
        matches!(
            receipt.state,
            TeardownReceiptState::Pending | TeardownReceiptState::OutcomeUnknown
        )
    });
    let orphaned_companion =
        receipt.is_some_and(|receipt| receipt.state == TeardownReceiptState::OrphanedCompanion);
    if removal_pending {
        blockers.push(
            "a prior worktree removal has an unknown outcome; reconcile it before retrying"
                .to_owned(),
        );
        warnings
            .push("the companion was retained because Herdr did not confirm removal".to_owned());
    }
    if orphaned_companion {
        warnings.push(
            "Herdr removal was confirmed but the reviewed companion still needs explicit cleanup"
                .to_owned(),
        );
    }

    let mut allowed_actions = Vec::new();
    if workspace_state == WorkspaceTeardownWorkspaceState::Live && provenance_confirmed {
        allowed_actions.push(WorkspaceTeardownAction::CloseSpace);
    }
    if ownership == WorkspaceTeardownOwnership::BorrowedOpened
        && companion_state != WorkspaceTeardownCompanionState::Ambiguous
        && provenance_confirmed
    {
        allowed_actions.push(WorkspaceTeardownAction::ForgetAssociation);
    }
    if removal_pending && provenance_confirmed {
        allowed_actions.push(WorkspaceTeardownAction::ReconcileRemoveOutcome);
    }
    let orphan_cleanup_allowed = orphaned_companion
        && provenance_confirmed
        && workspace_state == WorkspaceTeardownWorkspaceState::Missing
        && companion_state == WorkspaceTeardownCompanionState::Owned;
    if orphan_cleanup_allowed {
        allowed_actions.push(WorkspaceTeardownAction::RemoveOrphanedCompanion);
    }
    let removal_allowed = blockers.is_empty() && companion_path.is_some() && receipt.is_none();
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
        companion_state,
        is_linked_worktree,
        dirty_state,
        companion_path,
        allowed_actions,
        blockers,
        warnings,
        required_confirmation: if removal_allowed {
            Some(REMOVE_WORKTREE_CONFIRMATION.to_owned())
        } else if orphan_cleanup_allowed {
            Some(REMOVE_COMPANION_CONFIRMATION.to_owned())
        } else {
            None
        },
    })
}

/// Recompute a fresh preview before producing an adapter command. This module
/// never removes files; the integration owner must execute the returned Herdr
/// command, confirm the authoritative removal, then delete only the reviewed
/// owned companion directory through the descriptor-relative store.
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
    if request.expected_endpoint_identity != preview.endpoint_identity
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
        WorkspaceTeardownAction::ForgetAssociation => {
            Ok(WorkspaceTeardownCommand::ForgetAssociation {
                operation_id: preview.operation_id,
            })
        }
        WorkspaceTeardownAction::ReconcileRemoveOutcome => {
            Ok(WorkspaceTeardownCommand::ReconcileRemoveOutcome {
                operation_id: preview.operation_id,
            })
        }
        WorkspaceTeardownAction::RemoveOrphanedCompanion => {
            if request.confirmation != REMOVE_COMPANION_CONFIRMATION {
                return Err(InspectionError::new(
                    "confirmation_required",
                    "type the required confirmation before removing this companion",
                ));
            }
            Ok(WorkspaceTeardownCommand::RemoveOrphanedCompanion {
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
            let companion_path = preview.companion_path.ok_or_else(|| {
                InspectionError::new(
                    "teardown_not_allowed",
                    "the reviewed companion is no longer owned",
                )
            })?;
            Ok(WorkspaceTeardownCommand::RemoveOwnedWorktree {
                workspace_id: preview.workspace_id,
                endpoint_identity: preview.endpoint_identity,
                force: false,
                checkout_path: preview.checkout_path,
                companion_path,
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
        && receipt.workspace_id == request.workspace_id
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

fn companion_state(
    operation: &WorkspaceOperation,
    companion: Option<&CompanionManifest>,
    endpoint_identity: &str,
    workspace_id: &str,
) -> WorkspaceTeardownCompanionState {
    let Some(companion) = companion else {
        return WorkspaceTeardownCompanionState::Missing;
    };
    if companion.cockpit_operation_id != operation.operation_id {
        return WorkspaceTeardownCompanionState::Foreign;
    }
    let companion_owned = operation.companion_id.as_deref()
        == Some(companion.cockpit_operation_id.as_str())
        && operation.owned_resources.iter().any(|resource| {
            resource.kind == "companion"
                && resource.path == operation.plan.companion_path
                && resource.created_by_operation
        });
    if !companion_owned
        || companion.ownership != "cockpit"
        || companion.herdr_session_identity != endpoint_identity
        || companion.herdr_workspace_id != workspace_id
        || companion.repository_key
            != operation
                .plan
                .repository
                .as_ref()
                .map(|repository| repository.common_dir.as_str())
                .unwrap_or("")
        || companion.repository_root
            != operation
                .plan
                .repository
                .as_ref()
                .map(|repository| repository.root.as_str())
                .unwrap_or("")
        || companion.checkout_path != operation.plan.checkout_path
    {
        return WorkspaceTeardownCompanionState::Ambiguous;
    }
    WorkspaceTeardownCompanionState::Owned
}

fn ownership(
    operation: &WorkspaceOperation,
    companion_state: WorkspaceTeardownCompanionState,
) -> WorkspaceTeardownOwnership {
    let worktree = operation.owned_resources.iter().find(|resource| {
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
    match worktree {
        Some(resource) if !resource.created_by_operation => {
            WorkspaceTeardownOwnership::BorrowedOpened
        }
        Some(resource)
            if resource.created_by_operation
                && companion_state == WorkspaceTeardownCompanionState::Owned =>
        {
            WorkspaceTeardownOwnership::OwnedCreated
        }
        Some(_) if companion_state == WorkspaceTeardownCompanionState::Foreign => {
            WorkspaceTeardownOwnership::Foreign
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
                companion_path: "/companions/operation".to_owned(),
                companion_id: "operation".to_owned(),
                companion_created_by_operation: created,
                label: "Task".to_owned(),
                focus: true,
                artifact: None,
                effects: Vec::new(),
                warnings: Vec::new(),
            },
            state: WorkspaceOperationState::Completed,
            step: WorkspaceOperationStep::Completed,
            workspace_id: Some("workspace".to_owned()),
            tab_id: None,
            pane_id: None,
            companion_id: Some("operation".to_owned()),
            owned_resources: vec![
                WorkspaceOwnedResource {
                    kind: "worktree".to_owned(),
                    path: "/worktrees/task".to_owned(),
                    created_by_operation: created,
                },
                WorkspaceOwnedResource {
                    kind: "companion".to_owned(),
                    path: "/companions/operation".to_owned(),
                    created_by_operation: true,
                },
            ],
            error: None,
            resume_allowed: false,
            cancel_requested: false,
            updated_at: "0".to_owned(),
        }
    }

    fn companion() -> CompanionManifest {
        CompanionManifest {
            schema_version: 1,
            cockpit_operation_id: "operation".to_owned(),
            herdr_session_identity: "endpoint".to_owned(),
            herdr_workspace_id: "workspace".to_owned(),
            repository_key: "/repository/.git".to_owned(),
            repository_root: "/repository".to_owned(),
            checkout_path: "/worktrees/task".to_owned(),
            artifact: None,
            created_at: "0".to_owned(),
            updated_at: "0".to_owned(),
            ownership: "cockpit".to_owned(),
        }
    }

    fn evidence<'a>(
        operation: &'a WorkspaceOperation,
        companion: &'a CompanionManifest,
        worktrees: &'a [WorkspaceTeardownWorktree],
    ) -> WorkspaceTeardownEvidence<'a> {
        WorkspaceTeardownEvidence {
            operation,
            companion: Some(companion),
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
            companion: companion(),
            state,
            updated_at: "0".to_owned(),
        }
    }

    #[test]
    fn owned_clean_linked_worktree_requires_confirmation() {
        let operation = operation(true);
        let companion = companion();
        let worktrees = worktrees(Some(false));
        let preview = preview(
            &WorkspaceTeardownPreviewRequest {
                workspace_id: "workspace".to_owned(),
            },
            evidence(&operation, &companion, &worktrees),
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
        let companion = companion();
        let clean_worktrees = worktrees(Some(false));
        let borrowed_preview = preview(
            &WorkspaceTeardownPreviewRequest {
                workspace_id: "workspace".to_owned(),
            },
            evidence(&borrowed, &companion, &clean_worktrees),
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
            evidence(&owned, &companion, &dirty_worktrees),
        )
        .expect("preview");
        assert!(
            !dirty_preview
                .allowed_actions
                .contains(&WorkspaceTeardownAction::RemoveOwnedWorktree)
        );
    }

    #[test]
    fn borrowed_directory_can_close_or_forget_but_never_remove_files() {
        let mut operation = operation(false);
        operation.plan.repository = None;
        operation.plan.ownership =
            cockpit_protocol::projects::WorkspaceCheckoutOwnership::BorrowedDirectory;
        operation.owned_resources[0].kind = "directory".to_owned();
        let mut companion = companion();
        companion.repository_key.clear();
        companion.repository_root.clear();
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
                companion: Some(&companion),
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
            preview
                .allowed_actions
                .contains(&WorkspaceTeardownAction::ForgetAssociation)
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
        let companion = companion();
        let worktrees = worktrees(Some(false));
        let preview = preview(
            &WorkspaceTeardownPreviewRequest {
                workspace_id: "workspace".to_owned(),
            },
            evidence(&operation, &companion, &worktrees),
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
        let companion = companion();
        let worktrees = worktrees(Some(false));
        let receipt = receipt(TeardownReceiptState::OutcomeUnknown);
        let preview = preview(
            &WorkspaceTeardownPreviewRequest {
                workspace_id: "workspace".to_owned(),
            },
            WorkspaceTeardownEvidence {
                operation: &operation,
                companion: Some(&companion),
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
    fn orphan_receipt_requires_explicit_companion_confirmation() {
        let operation = operation(true);
        let companion = companion();
        let receipt = receipt(TeardownReceiptState::OrphanedCompanion);
        let preview = preview(
            &WorkspaceTeardownPreviewRequest {
                workspace_id: "workspace".to_owned(),
            },
            WorkspaceTeardownEvidence {
                operation: &operation,
                companion: Some(&companion),
                endpoint_identity: "endpoint",
                repository_key: "/repository/.git",
                repository_root: "/repository",
                worktrees: &[],
                receipt: Some(&receipt),
            },
        )
        .expect("preview");
        assert!(
            preview
                .allowed_actions
                .contains(&WorkspaceTeardownAction::RemoveOrphanedCompanion)
        );
        assert_eq!(
            preview.required_confirmation.as_deref(),
            Some(REMOVE_COMPANION_CONFIRMATION)
        );
    }
}
