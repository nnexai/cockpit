use std::collections::BTreeMap;
use std::sync::Arc;

use cockpit_protocol::projects::{
    RepositoryCandidate, WorkspaceOperation, WorkspaceOperationState, WorkspaceOperationStep,
    WorkspaceOwnedResource, WorkspaceReconcileRequest, WorkspaceRecoveryAction, WorkspaceSetupMode,
};

use crate::InspectionError;
use crate::project_adapter::{ProjectWorktreeRequest, ProjectWorktreeResult};
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
        let (workspace_id, tab_id, pane_id) = if let Some(workspace_id) = entry.open_workspace_id.clone() {
            // Inventory proves the Space, not the identity of its original root pane.
            (workspace_id, None, None)
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
            let opened = self.open_existing_checkout(session, &operation, repository)
                .await?;
            (opened.workspace_id, opened.tab_id, opened.pane_id)
        };
        self.store.update(
            &operation.operation_id,
            Some(operation.generation),
            |operation| {
                operation.workspace_id = Some(workspace_id.clone());
                if operation.tab_id.is_none() {
                    operation.tab_id = tab_id.clone();
                }
                if operation.pane_id.is_none() {
                    operation.pane_id = pane_id.clone();
                }
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
    ) -> Result<ProjectWorktreeResult, InspectionError> {
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
        Ok(opened)
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
        let actor_placement = store.owned_checkout(&operation.operation_id)?.is_some();
        store.update(
            &operation.operation_id,
            Some(operation.generation),
            |operation| {
                if pending_unknown(operation) {
                    operation.state = WorkspaceOperationState::OutcomeUnknown;
                    operation.resume_allowed = false;
                } else {
                    operation.state = WorkspaceOperationState::Partial;
                    operation.resume_allowed = !actor_placement;
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

#[cfg(test)]
mod tests {
    use super::*;
    use super::super::test_support::*;
    use crate::library::LibraryService;
    use crate::project_adapter::{ProjectInventory, ProjectWorktreeEntry};
    use crate::sources::SourceService;
    use cockpit_protocol::projects::{WorkspaceOperationRequest, WorkspaceSetupRequest};
    use uuid::Uuid;

    #[tokio::test]
    async fn recovery_preserves_proven_root_receipts_without_replaying_create() {
        for already_open in [false, true] {
            let root = std::env::temp_dir().join(format!("cockpit-setup-recovery-{}", Uuid::new_v4()));
            let repository = root.join("repository");
            std::fs::create_dir_all(&repository).unwrap();
            git(&repository, &["init"]);
            git(&repository, &["-c", "user.name=Fixture", "-c", "user.email=fixture@example.test",
                "commit", "--allow-empty", "-m", "fixture"]);
            let configuration = configuration(&root);
            std::fs::create_dir_all(&configuration.worktree_root).unwrap();
            let adapter = Arc::new(NestedDirectoryAdapter::default());
            let service = Arc::new(ProjectService::new(configuration.clone(), adapter.clone()).unwrap());
            let candidate = service.repositories().await.unwrap().repositories.into_iter()
                .find(|candidate| candidate.checkout_path == repository.to_string_lossy()).unwrap();
            adapter.repository.set(candidate.clone()).unwrap();
            let plan = service.plan("session", &WorkspaceSetupRequest::Create {
                repository_id: candidate.repository_id.clone(),
                branch: Some("recovered".to_owned()), base_ref: None, checkout_path: None,
                label: None, task_name: None, artifact_url: None, linked_artifact_urls: vec![],
                focus: false,
            }).await.unwrap();
            *adapter.inventory.lock() = Some(ProjectInventory {
                endpoint_identity: plan.endpoint_identity.clone(),
                repository_key: candidate.common_dir.clone(),
                repository_root: candidate.root.clone(),
                supported_methods: vec![],
                worktrees: vec![ProjectWorktreeEntry {
                    checkout_path: plan.checkout_path.clone(), branch: plan.branch.clone(),
                    open_workspace_id: already_open.then(|| "workspace".to_owned()),
                    is_primary: false, is_linked_worktree: true, dirty: Some(false),
                }],
            });
            let pending = service.mark_dispatch(&plan.operation_id, WorkspaceOperationStep::HerdrRequested).unwrap();
            let library = Arc::new(LibraryService::new(configuration.clone(),
                Arc::new(SourceService::new(&configuration, vec![]).unwrap())));
            service.resume("session", &WorkspaceOperationRequest {
                operation_id: plan.operation_id.clone(), expected_generation: pending.generation,
            }, library.clone()).await.unwrap_err();
            let retained = service.store.load(&plan.operation_id).unwrap();
            assert_eq!(retained.generation, pending.generation);
            assert_eq!(retained.state, pending.state);
            assert_eq!(retained.step, pending.step);
            assert!(pending_unknown(&retained));
            assert!(retained.workspace_id.is_none());
            assert!(retained.tab_id.is_none());
            assert!(retained.pane_id.is_none());
            assert!(adapter.worktree_requests.lock().unwrap().is_empty());

            let recovered = service.reconcile("session", &WorkspaceReconcileRequest {
                operation_id: plan.operation_id.clone(), expected_generation: pending.generation,
                action: WorkspaceRecoveryAction::AcceptExistingWorktree,
            }).await.unwrap();
            assert_eq!(recovered.step, WorkspaceOperationStep::WorktreeReady);
            assert_eq!(recovered.state, WorkspaceOperationState::Partial);
            assert!(recovered.resume_allowed);
            assert_eq!(recovered.workspace_id.as_deref(), Some("workspace"));
            assert_eq!(recovered.tab_id.as_deref(), (!already_open).then_some("workspace:root"));
            assert_eq!(recovered.pane_id.as_deref(), (!already_open).then_some("workspace:root-pane"));
            assert!(recovered.owned_resources.iter().all(|resource| !resource.created_by_operation));
            let dispatches = usize::from(!already_open);
            {
                let requests = adapter.worktree_requests.lock().unwrap();
                assert_eq!(requests.len(), dispatches);
                assert!(requests.iter().all(|request|
                    request.mode == WorkspaceSetupMode::Open && request.open_existing_worktree));
            }

            service.resume("session", &WorkspaceOperationRequest {
                operation_id: plan.operation_id.clone(), expected_generation: recovered.generation,
            }, library).await.unwrap();
            let completed = settled_setup(&service, &plan.operation_id).await;
            assert_eq!(completed.state, WorkspaceOperationState::Completed, "{completed:?}");
            assert_eq!(completed.tab_id, recovered.tab_id);
            assert_eq!(completed.pane_id, recovered.pane_id);
            assert_eq!(adapter.worktree_requests.lock().unwrap().len(), dispatches);
            std::fs::remove_dir_all(root).unwrap();
        }
    }
}
