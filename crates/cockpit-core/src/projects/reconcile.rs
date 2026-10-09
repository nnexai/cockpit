use std::collections::BTreeMap;
use std::sync::Arc;

use cockpit_protocol::projects::{
    RepositoryCandidate, WorkspaceOperation, WorkspaceOperationState, WorkspaceOperationStep,
    WorkspaceOwnedResource, WorkspaceReconcileRequest, WorkspaceRecoveryAction, WorkspaceSetupMode,
};

use crate::InspectionError;
use crate::project_adapter::ProjectWorktreeRequest;
use crate::project_store::ProjectStore;
use crate::repositories::RepositoryCatalog;

use super::execute::pending_unknown;
use super::{
    ProjectService, error_response, plan_expired, unstarted, validate_session, verify_inventory,
    verify_worktree_result,
};

impl ProjectService {
    pub async fn reconcile(
        self: &Arc<Self>,
        session: &str,
        request: &WorkspaceReconcileRequest,
    ) -> Result<WorkspaceOperation, InspectionError> {
        validate_session(session)?;
        validate_reconcile_request(session, request)?;
        let operation = self.store.load(&request.operation_id)?;
        if operation.session_id != session || operation.generation != request.expected_generation {
            return Err(InspectionError::new(
                "stale_identity",
                "operation session or generation is stale",
            ));
        }
        let lease = self
            .store
            .acquire_execution_lease(&operation.operation_id)?;
        let repository = operation.plan.repository.clone().ok_or_else(|| {
            InspectionError::new(
                "reconciliation_requires_inspection",
                "a borrowed directory has no Git inventory; inspect its unknown workspace outcome before retrying",
            )
        })?;
        let fresh_repository = RepositoryCatalog::new(self.configuration.clone())
            .resolve(&repository.repository_id)
            .await?;
        if fresh_repository.root != repository.root
            || fresh_repository.common_dir != repository.common_dir
            || fresh_repository.checkout_path != repository.checkout_path
        {
            return Err(InspectionError::new(
                "repository_identity_stale",
                "repository changed since operation planning",
            ));
        }
        let result = match request.action {
            WorkspaceRecoveryAction::AcceptExistingWorktree => {
                self.accept_existing_worktree(session, operation, &repository)
                    .await?
            }
            WorkspaceRecoveryAction::RetryEnvironment => {
                self.retry_environment(session, &operation, &repository)
                    .await?
            }
        };
        drop(lease);
        Ok(result)
    }

    async fn accept_existing_worktree(
        &self,
        session: &str,
        mut operation: WorkspaceOperation,
        repository: &RepositoryCandidate,
    ) -> Result<WorkspaceOperation, InspectionError> {
        if operation.step != WorkspaceOperationStep::HerdrRequested
            || operation.workspace_id.is_some()
        {
            return Err(InspectionError::new(
                "invalid_reconcile",
                "existing worktree recovery requires an unknown Herdr worktree outcome",
            ));
        }
        let inventory = self
            .adapter
            .project_inventory(session, &repository.checkout_path)
            .await?;
        verify_inventory(&inventory, repository)?;
        if inventory.endpoint_identity != operation.plan.endpoint_identity {
            return Err(InspectionError::new(
                "stale_identity",
                "Herdr endpoint identity changed while reconciling",
            ));
        }
        let entries = inventory
            .worktrees
            .iter()
            .filter(|entry| entry.checkout_path == operation.plan.checkout_path)
            .collect::<Vec<_>>();
        if entries.len() != 1 {
            return Err(InspectionError::new(
                "workspace_conflict",
                "fresh inventory did not prove exactly one existing checkout",
            ));
        }
        let entry = entries[0];
        if operation.plan.branch.is_some() && entry.branch != operation.plan.branch {
            return Err(InspectionError::new(
                "workspace_conflict",
                "existing checkout branch differs from reviewed plan",
            ));
        }
        let workspace_id = if let Some(workspace_id) = entry.open_workspace_id.clone() {
            workspace_id
        } else {
            // The checkout is known, but no workspace is open. Recovery
            // is an explicit Open dispatch; never redispatch Create.
            operation = self.mark_dispatch(
                &operation.operation_id,
                WorkspaceOperationStep::HerdrRequested,
            )?;
            if operation.state == WorkspaceOperationState::Cancelled {
                return Err(InspectionError::new("cancelled", "operation was cancelled"));
            }
            self.open_existing_checkout(session, &operation, repository)
                .await?
        };
        self.store.update(
            &operation.operation_id,
            Some(operation.generation),
            |operation| {
                operation.workspace_id = Some(workspace_id.clone());
                operation.owned_resources.push(WorkspaceOwnedResource {
                    kind: "worktree".to_owned(),
                    path: entry.checkout_path.clone(),
                    created_by_operation: false,
                });
                // An uncertain checkout without a workspace is opened
                // explicitly above. Its returned workspace is already
                // proven by fresh inventory, so resume must continue
                // from worktree readiness instead of dispatching Open
                // a second time.
                operation.step = WorkspaceOperationStep::WorktreeReady;
                if operation.cancel_requested {
                    operation.state = WorkspaceOperationState::Cancelled;
                    operation.resume_allowed = false;
                } else {
                    operation.state = WorkspaceOperationState::Partial;
                    operation.resume_allowed = true;
                }
                operation.error = None;
                Ok(())
            },
        )
    }

    async fn open_existing_checkout(
        &self,
        session: &str,
        operation: &WorkspaceOperation,
        repository: &RepositoryCandidate,
    ) -> Result<String, InspectionError> {
        let opened = self
            .adapter
            .project_worktree(
                session,
                &ProjectWorktreeRequest {
                    endpoint_identity: operation.plan.endpoint_identity.clone(),
                    mode: WorkspaceSetupMode::Open,
                    source_cwd: repository.checkout_path.clone(),
                    branch: None,
                    base: None,
                    checkout_path: operation.plan.checkout_path.clone(),
                    label: operation.plan.label.clone(),
                    focus: operation.plan.focus,
                    env: BTreeMap::new(),
                    open_existing_worktree: true,
                },
            )
            .await?;
        verify_worktree_result(&opened, &operation.plan)?;
        let reopened = self
            .adapter
            .project_inventory(session, &repository.checkout_path)
            .await?;
        verify_inventory(&reopened, repository)?;
        if reopened.endpoint_identity != operation.plan.endpoint_identity
            || !reopened.worktrees.iter().any(|candidate| {
                candidate.checkout_path == operation.plan.checkout_path
                    && candidate.open_workspace_id.as_deref() == Some(opened.workspace_id.as_str())
            })
        {
            return Err(InspectionError::new(
                "workspace_conflict",
                "explicit Open did not prove the recovered workspace",
            ));
        }
        Ok(opened.workspace_id)
    }

    async fn retry_environment(
        &self,
        session: &str,
        operation: &WorkspaceOperation,
        repository: &RepositoryCandidate,
    ) -> Result<WorkspaceOperation, InspectionError> {
        if operation.workspace_id.is_none()
            || operation.pane_id.is_some()
            || !matches!(
                operation.step,
                WorkspaceOperationStep::WorkspaceVerified
                    | WorkspaceOperationStep::EnvironmentRequested
            )
        {
            return Err(InspectionError::new(
                "invalid_reconcile",
                "environment retry requires a verified workspace",
            ));
        }
        let inventory = self
            .adapter
            .project_inventory(session, &repository.checkout_path)
            .await?;
        verify_inventory(&inventory, repository)?;
        if inventory.endpoint_identity != operation.plan.endpoint_identity
            || !inventory.worktrees.iter().any(|entry| {
                entry.checkout_path == operation.plan.checkout_path
                    && entry.open_workspace_id.as_deref() == operation.workspace_id.as_deref()
            })
        {
            return Err(InspectionError::new(
                "stale_identity",
                "workspace provenance changed while reconciling environment",
            ));
        }
        self.store.update(
            &operation.operation_id,
            Some(operation.generation),
            |operation| {
                operation.step = WorkspaceOperationStep::WorkspaceVerified;
                if operation.cancel_requested {
                    operation.state = WorkspaceOperationState::Cancelled;
                    operation.resume_allowed = false;
                } else {
                    operation.state = WorkspaceOperationState::Partial;
                    operation.resume_allowed = true;
                }
                operation.error = Some(error_response(
                    "environment_retry_acknowledged",
                    "retry may create a new environment tab; prior uncertain panes are untouched",
                ));
                Ok(())
            },
        )
    }
}

pub(super) fn recover_startup(store: &ProjectStore) -> Result<(), InspectionError> {
    for operation in store.list()? {
        // A plan that never started owns nothing and was not abandoned: keep
        // a recent one for its dialog and delete an expired one.
        if unstarted(&operation) {
            if plan_expired(&operation) {
                store.discard_unstarted_plan(&operation.operation_id)?;
            }
            continue;
        }
        if matches!(
            operation.state,
            WorkspaceOperationState::Completed
                | WorkspaceOperationState::Cancelled
                | WorkspaceOperationState::NeedsReview
                | WorkspaceOperationState::OutcomeUnknown
                | WorkspaceOperationState::Partial
        ) {
            continue;
        }
        let Some(_lease) = store.try_acquire_execution_lease(&operation.operation_id)? else {
            continue;
        };
        store.update(
            &operation.operation_id,
            Some(operation.generation),
            |operation| {
                if pending_unknown(operation) {
                    operation.state = WorkspaceOperationState::OutcomeUnknown;
                    operation.resume_allowed = false;
                } else {
                    operation.state = WorkspaceOperationState::Partial;
                    operation.resume_allowed = true;
                }
                operation.error = Some(error_response(
                    "abandoned",
                    "operation was abandoned by a prior service instance",
                ));
                Ok(())
            },
        )?;
    }
    Ok(())
}

fn validate_reconcile_request(
    session: &str,
    request: &WorkspaceReconcileRequest,
) -> Result<(), InspectionError> {
    if session.is_empty() || request.operation_id.is_empty() || request.expected_generation == 0 {
        return Err(InspectionError::new(
            "invalid_request",
            "reconcile identity is invalid",
        ));
    }
    Ok(())
}
