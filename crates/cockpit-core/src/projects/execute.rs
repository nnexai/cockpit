use std::collections::BTreeMap;
use std::path::Path;
use std::sync::{Arc, atomic::Ordering};

use cockpit_protocol::library::{
    LibraryPhaseName, LibraryPhaseState, SpaceAddRequest, SpaceTarget,
};
use cockpit_protocol::projects::{
    ProjectArtifact, WorkspaceCheckoutOwnership, WorkspaceOperation, WorkspaceOperationRequest,
    WorkspaceOperationState, WorkspaceOperationStep, WorkspaceOwnedResource, WorkspaceSetupMode,
    WorkspaceSetupPlan,
};
use cockpit_protocol::v1::ErrorResponse;

use crate::InspectionError;
use crate::library::LibraryService;
use crate::project_adapter::{
    ProjectInventory, ProjectTerminalRequest, ProjectWorktreeRequest, ProjectWorktreeResult,
};
use crate::project_store::validate_project_root;
use crate::repositories::RepositoryCatalog;
use crate::sources::{FetchedAssets, SourceFetchRequest, instance_authority};

use super::plan::{
    accessible_directory, reviewed_artifact_url, validate_review_metadata,
    validate_review_plan_source,
};
use super::{
    ProjectService, error_response, plan_expired, validate_operation_request, validate_session,
    verify_inventory, verify_worktree_result,
};

#[derive(Clone, Copy)]
enum StepAction {
    Checkpoint,
    Dispatch {
        code: &'static str,
        message: &'static str,
    },
}

#[derive(Clone, Copy)]
enum StepReceipt {
    None,
    WorkspaceId,
    PaneId,
}

struct SetupStep {
    step: WorkspaceOperationStep,
    action: StepAction,
    receipt: StepReceipt,
}

// Execution order, not protocol declaration order: Environment precedes Context.
static SETUP_STEPS: [SetupStep; 11] = [
    SetupStep {
        step: WorkspaceOperationStep::Planned,
        action: StepAction::Checkpoint,
        receipt: StepReceipt::None,
    },
    SetupStep {
        step: WorkspaceOperationStep::Validated,
        action: StepAction::Checkpoint,
        receipt: StepReceipt::None,
    },
    SetupStep {
        step: WorkspaceOperationStep::HerdrRequested,
        action: StepAction::Dispatch {
            code: "herdr_dispatch_pending",
            message: "awaiting Herdr worktree outcome",
        },
        receipt: StepReceipt::WorkspaceId,
    },
    SetupStep {
        step: WorkspaceOperationStep::HerdrObserved,
        action: StepAction::Checkpoint,
        receipt: StepReceipt::None,
    },
    SetupStep {
        step: WorkspaceOperationStep::WorktreeReady,
        action: StepAction::Checkpoint,
        receipt: StepReceipt::None,
    },
    SetupStep {
        step: WorkspaceOperationStep::WorkspaceVerified,
        action: StepAction::Checkpoint,
        receipt: StepReceipt::None,
    },
    SetupStep {
        step: WorkspaceOperationStep::EnvironmentRequested,
        action: StepAction::Dispatch {
            code: "environment_dispatch_pending",
            message: "awaiting terminal dispatch outcome",
        },
        receipt: StepReceipt::PaneId,
    },
    SetupStep {
        step: WorkspaceOperationStep::EnvironmentReady,
        action: StepAction::Checkpoint,
        receipt: StepReceipt::None,
    },
    SetupStep {
        step: WorkspaceOperationStep::ContextPreparing,
        action: StepAction::Checkpoint,
        receipt: StepReceipt::None,
    },
    SetupStep {
        step: WorkspaceOperationStep::ContextReady,
        action: StepAction::Checkpoint,
        receipt: StepReceipt::None,
    },
    SetupStep {
        step: WorkspaceOperationStep::Completed,
        action: StepAction::Checkpoint,
        receipt: StepReceipt::None,
    },
];

// The ordered table is the sole source of step descriptors and relative order.
fn setup_step(step: WorkspaceOperationStep) -> (usize, &'static SetupStep) {
    SETUP_STEPS
        .iter()
        .enumerate()
        .find(|(_, entry)| entry.step == step)
        .expect("setup step is present in execution table")
}

fn step_at_least(current: WorkspaceOperationStep, wanted: WorkspaceOperationStep) -> bool {
    setup_step(current).0 >= setup_step(wanted).0
}

pub(super) fn pending_unknown(operation: &WorkspaceOperation) -> bool {
    match setup_step(operation.step).1.receipt {
        StepReceipt::WorkspaceId => operation.workspace_id.is_none(),
        StepReceipt::PaneId => operation.pane_id.is_none(),
        StepReceipt::None => false,
    }
}

impl ProjectService {
    pub async fn start(
        self: &Arc<Self>,
        session: &str,
        request: &WorkspaceOperationRequest,
        library: Arc<LibraryService>,
    ) -> Result<WorkspaceOperation, InspectionError> {
        validate_session(session)?;
        validate_operation_request(session, request)?;
        if self.shutting_down.load(Ordering::SeqCst) {
            return Err(InspectionError::new(
                "shutdown",
                "project service is shutting down",
            ));
        }
        let operation = self.store.load(&request.operation_id)?;
        if operation.session_id != session || operation.generation != request.expected_generation {
            return Err(InspectionError::new(
                "stale_identity",
                "operation session or generation is stale",
            ));
        }
        if operation.state != WorkspaceOperationState::Planned
            || operation.step != WorkspaceOperationStep::Planned
        {
            return Err(InspectionError::new(
                "invalid_operation_state",
                "only a planned operation can start",
            ));
        }
        if plan_expired(&operation) {
            return Err(InspectionError::new(
                "stale_plan",
                "this setup plan expired; review a fresh plan before starting",
            ));
        }
        // Re-check the reviewed authority and effects before taking the
        // execution lease. A changed repository, endpoint, path, or source
        // authority must force a fresh plan rather than silently executing a
        // different operation.
        let (_, fetched) = self
            .preflight_plan(session, &operation.plan, true)
            .await
            .map_err(|error| {
                if error.code.starts_with("source_") {
                    error
                } else {
                    InspectionError::new(
                        "stale_plan",
                        format!("reviewed setup is no longer valid: {}", error.message),
                    )
                }
            })?;
        let lease = self
            .store
            .acquire_execution_lease(&operation.operation_id)?;
        if self.shutting_down.load(Ordering::SeqCst) {
            return Err(InspectionError::new(
                "shutdown",
                "project service is shutting down",
            ));
        }
        let started = self.store.update(
            &operation.operation_id,
            Some(operation.generation),
            |operation| {
                operation.state = WorkspaceOperationState::Running;
                operation.step = WorkspaceOperationStep::Validated;
                Ok(())
            },
        )?;
        self.workers
            .lock()
            .await
            .insert(started.operation_id.clone());
        let service = Arc::clone(self);
        let id = started.operation_id.clone();
        let sid = session.to_owned();
        tokio::spawn(async move {
            service
                .execute(&sid, &id, lease, library, Some(fetched))
                .await;
        });
        Ok(started)
    }

    pub async fn resume(
        self: &Arc<Self>,
        session: &str,
        request: &WorkspaceOperationRequest,
        library: Arc<LibraryService>,
    ) -> Result<WorkspaceOperation, InspectionError> {
        validate_session(session)?;
        validate_operation_request(session, request)?;
        if self.shutting_down.load(Ordering::SeqCst) {
            return Err(InspectionError::new(
                "shutdown",
                "project service is shutting down",
            ));
        }
        let operation = self.store.load(&request.operation_id)?;
        if operation.session_id != session || operation.generation != request.expected_generation {
            return Err(InspectionError::new(
                "stale_identity",
                "operation session or generation is stale",
            ));
        }
        if !operation.resume_allowed
            || matches!(
                operation.state,
                WorkspaceOperationState::Completed | WorkspaceOperationState::Cancelled
            )
        {
            return Err(InspectionError::new(
                "resume_not_allowed",
                "operation has no proven resumable step",
            ));
        }
        if pending_unknown(&operation) {
            return Err(InspectionError::new(
                "needs_review",
                "dispatch outcome is unknown; reconcile before retry",
            ));
        }
        let lease = self
            .store
            .acquire_execution_lease(&operation.operation_id)?;
        if self.shutting_down.load(Ordering::SeqCst) {
            return Err(InspectionError::new(
                "shutdown",
                "project service is shutting down",
            ));
        }
        let resumed = self.store.update(
            &operation.operation_id,
            Some(operation.generation),
            |operation| {
                if operation.cancel_requested {
                    operation.state = WorkspaceOperationState::Cancelled;
                    operation.resume_allowed = false;
                    return Ok(());
                }
                operation.state = WorkspaceOperationState::Running;
                operation.cancel_requested = false;
                Ok(())
            },
        )?;
        if resumed.state == WorkspaceOperationState::Cancelled {
            return Err(InspectionError::new("cancelled", "operation was cancelled"));
        }
        self.cancelled.lock().await.remove(&resumed.operation_id);
        self.workers
            .lock()
            .await
            .insert(resumed.operation_id.clone());
        let service = Arc::clone(self);
        let id = resumed.operation_id.clone();
        let sid = session.to_owned();
        tokio::spawn(async move {
            service.execute(&sid, &id, lease, library, None).await;
        });
        Ok(resumed)
    }

    pub async fn cancel(
        &self,
        session: &str,
        request: &WorkspaceOperationRequest,
    ) -> Result<WorkspaceOperation, InspectionError> {
        validate_session(session)?;
        validate_operation_request(session, request)?;
        let operation = self.store.update(
            &request.operation_id,
            Some(request.expected_generation),
            |operation| {
                if operation.session_id != session {
                    return Err(InspectionError::new(
                        "stale_identity",
                        "operation belongs to another session",
                    ));
                }
                if operation.state == WorkspaceOperationState::Completed {
                    return Ok(());
                }
                operation.cancel_requested = true;
                if operation.state != WorkspaceOperationState::Running {
                    operation.state = WorkspaceOperationState::Cancelled;
                    operation.resume_allowed = false;
                }
                Ok(())
            },
        )?;
        if operation.state == WorkspaceOperationState::Running {
            self.cancelled
                .lock()
                .await
                .insert(operation.operation_id.clone());
        }
        Ok(operation)
    }
    async fn fetch_setup_artifact(
        &self,
        artifact: &ProjectArtifact,
    ) -> Result<FetchedAssets, InspectionError> {
        let sources = self.sources.as_ref().ok_or_else(|| {
            InspectionError::new(
                "source_provider_unsupported",
                "source validation is not configured in this host",
            )
        })?;
        let authority = instance_authority(
            &self.configuration,
            &artifact.provider_id,
            &artifact.original_url,
        )?;
        let mut fetched = sources
            .validate_artifact_for_setup(SourceFetchRequest {
                provider_id: artifact.provider_id.clone(),
                artifact_url: artifact.original_url.clone(),
                authority,
            })
            .await?;
        if reviewed_artifact_url(artifact, &fetched)? != artifact.canonical_url {
            return Err(InspectionError::new(
                "stale_plan",
                "source provenance changed; review a fresh setup plan before starting",
            ));
        }
        for asset in &mut fetched.assets {
            if asset.source.provider_id == artifact.provider_id
                && asset.source.resource_type == artifact.kind
                && asset.source.canonical_id == artifact.canonical_id
            {
                asset.original_url = Some(artifact.original_url.clone());
            }
        }
        Ok(fetched)
    }

    async fn preflight_plan(
        &self,
        session: &str,
        plan: &WorkspaceSetupPlan,
        validate_sources: bool,
    ) -> Result<(Option<ProjectInventory>, Vec<FetchedAssets>), InspectionError> {
        validate_project_root(Path::new(&self.configuration.worktree_root))?;
        let before = if plan.mode == WorkspaceSetupMode::Create {
            let repository = plan.repository.as_ref().ok_or_else(|| {
                InspectionError::new("repository_missing", "create plan has no repository")
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
                    "repository changed since the setup plan was reviewed",
                ));
            }
            let inventory = self
                .adapter
                .project_inventory(session, &repository.checkout_path)
                .await?;
            verify_inventory(&inventory, repository)?;
            if inventory.endpoint_identity != plan.endpoint_identity {
                return Err(InspectionError::new(
                    "stale_identity",
                    "Herdr endpoint identity differs from the reviewed setup plan",
                ));
            }
            Some(inventory)
        } else {
            accessible_directory(&plan.checkout_path)?;
            if self.adapter.project_endpoint_identity(session).await? != plan.endpoint_identity {
                return Err(InspectionError::new(
                    "stale_identity",
                    "Herdr endpoint identity differs from the reviewed setup plan",
                ));
            }
            None
        };
        let mut fetched = Vec::new();
        if validate_sources && let Some(artifact) = plan.artifact.as_ref() {
            let repository = plan.repository.as_ref().ok_or_else(|| {
                InspectionError::new(
                    "source_repository_missing",
                    "source setup requires a reviewed repository",
                )
            })?;
            let validated = self.fetch_setup_artifact(artifact).await?;
            if artifact.kind == "review" {
                let sources = self.sources.as_ref().expect("source validation succeeded");
                let authority = instance_authority(
                    &self.configuration,
                    &artifact.provider_id,
                    &artifact.original_url,
                )?;
                let metadata = sources
                    .metadata_for_setup(SourceFetchRequest {
                        provider_id: artifact.provider_id.clone(),
                        artifact_url: artifact.canonical_url.clone(),
                        authority,
                    })
                    .await?;
                validate_review_metadata(&self.configuration, artifact, &metadata)?;
                validate_review_plan_source(
                    &RepositoryCatalog::new(self.configuration.clone()),
                    repository,
                    plan,
                    &metadata,
                )
                .await
                .map_err(|error| {
                    if error.code.starts_with("source_ref_") {
                        InspectionError::new(
                            "stale_plan",
                            format!("reviewed source ref is no longer valid: {}", error.message),
                        )
                    } else {
                        error
                    }
                })?;
            }
            fetched.push(validated);
            for linked in &plan.linked_artifacts {
                fetched.push(self.fetch_setup_artifact(linked).await?);
            }
        }
        Ok((before, fetched))
    }

    async fn execute(
        &self,
        session: &str,
        id: &str,
        _lease: crate::project_store::ExecutionLease,
        library: Arc<LibraryService>,
        fetched: Option<Vec<FetchedAssets>>,
    ) {
        let result = self.execute_inner(session, id, &library, fetched).await;
        match result {
            Ok(()) => {}
            Err(error) if error.code == "shutdown" => {
                let _ = self.settle_shutdown(id).await;
            }
            Err(error) => {
                let _ = self.fail(id, error).await;
            }
        }
        self.workers.lock().await.remove(id);
    }

    async fn execute_inner(
        &self,
        session: &str,
        id: &str,
        library: &LibraryService,
        fetched: Option<Vec<FetchedAssets>>,
    ) -> Result<(), InspectionError> {
        self.boundary(id).await?;
        let mut operation = self.store.load(id)?;
        let plan = operation.plan.clone();
        let (before, preflight_assets) = self
            .preflight_plan(
                session,
                &plan,
                fetched.is_none() && operation.workspace_id.is_none(),
            )
            .await?;
        let fetched = fetched.unwrap_or(preflight_assets);
        let borrowed = operation
            .owned_resources
            .iter()
            .any(|resource| resource.kind == "worktree" && !resource.created_by_operation);
        let result = self
            .ensure_worktree(session, id, &mut operation, &plan, borrowed)
            .await?;
        operation = self
            .verify_workspace(session, id, &plan, before.as_ref(), &result)
            .await?;
        self.verify_environment_target(session, &plan, &result)
            .await?;
        operation = self
            .ensure_environment(session, id, operation, &plan, &result, library)
            .await?;
        let context_error = self
            .prepare_context(
                session,
                id,
                operation.step,
                &plan,
                &result,
                library,
                fetched,
            )
            .await?;
        self.boundary(id).await?;
        let completed = self.store.update(id, None, |operation| {
            if operation.cancel_requested {
                operation.state = WorkspaceOperationState::Cancelled;
                operation.resume_allowed = false;
            } else if let Some(error) = context_error.clone() {
                operation.state = WorkspaceOperationState::Partial;
                operation.resume_allowed = true;
                operation.error = Some(error);
            } else {
                operation.state = WorkspaceOperationState::Completed;
                operation.step = WorkspaceOperationStep::Completed;
                operation.resume_allowed = false;
                operation.error = None;
            }
            Ok(())
        })?;
        if completed.state == WorkspaceOperationState::Cancelled {
            return Err(InspectionError::new("cancelled", "operation was cancelled"));
        }
        Ok(())
    }

    async fn ensure_worktree(
        &self,
        session: &str,
        id: &str,
        operation: &mut WorkspaceOperation,
        plan: &WorkspaceSetupPlan,
        borrowed: bool,
    ) -> Result<ProjectWorktreeResult, InspectionError> {
        let repository = plan.repository.as_ref();
        let result = if let Some(workspace_id) = operation.workspace_id.clone() {
            if borrowed && operation.step == WorkspaceOperationStep::HerdrObserved {
                *operation = self.mark_dispatch(id, WorkspaceOperationStep::HerdrRequested)?;
                if operation.state == WorkspaceOperationState::Cancelled {
                    return Err(InspectionError::new("cancelled", "operation was cancelled"));
                }
                let opened = self
                    .adapter
                    .project_worktree(
                        session,
                        &ProjectWorktreeRequest {
                            endpoint_identity: plan.endpoint_identity.clone(),
                            mode: WorkspaceSetupMode::Open,
                            source_cwd: repository
                                .map(|repository| repository.checkout_path.clone())
                                .unwrap_or_else(|| plan.checkout_path.clone()),
                            branch: None,
                            base: None,
                            checkout_path: plan.checkout_path.clone(),
                            label: plan.label.clone(),
                            focus: plan.focus,
                            env: BTreeMap::new(),
                            open_existing_worktree: false,
                        },
                    )
                    .await?;
                self.merge_worktree_receipt(id, &opened, false)?
            } else {
                ProjectWorktreeResult {
                    workspace_id,
                    tab_id: operation.tab_id.clone(),
                    pane_id: operation.pane_id.clone(),
                    checkout_path: plan.checkout_path.clone(),
                    branch: plan.branch.clone(),
                    already_open: !operation
                        .owned_resources
                        .iter()
                        .any(|r| r.kind == "worktree" && r.created_by_operation),
                }
            }
        } else {
            *operation = self.mark_dispatch(id, WorkspaceOperationStep::HerdrRequested)?;
            if operation.state == WorkspaceOperationState::Cancelled {
                return Err(InspectionError::new("cancelled", "operation was cancelled"));
            }
            let created = self
                .adapter
                .project_worktree(
                    session,
                    &ProjectWorktreeRequest {
                        endpoint_identity: plan.endpoint_identity.clone(),
                        mode: plan.mode,
                        source_cwd: if plan.mode == WorkspaceSetupMode::Open {
                            plan.checkout_path.clone()
                        } else {
                            repository
                                .expect("Create plan has repository")
                                .checkout_path
                                .clone()
                        },
                        branch: if plan.mode == WorkspaceSetupMode::Open {
                            None
                        } else {
                            plan.branch.clone()
                        },
                        base: plan.base.clone(),
                        checkout_path: plan.checkout_path.clone(),
                        label: plan.label.clone(),
                        focus: plan.focus,
                        env: BTreeMap::new(),
                        open_existing_worktree: false,
                    },
                )
                .await?;
            verify_worktree_result(&created, plan)?;
            self.merge_worktree_receipt(
                id,
                &created,
                plan.mode == WorkspaceSetupMode::Create && !created.already_open,
            )?
        };
        Ok(result)
    }

    async fn verify_workspace(
        &self,
        session: &str,
        id: &str,
        plan: &WorkspaceSetupPlan,
        before: Option<&ProjectInventory>,
        result: &ProjectWorktreeResult,
    ) -> Result<WorkspaceOperation, InspectionError> {
        let repository = plan.repository.as_ref();
        verify_worktree_result(result, plan)?;
        self.store.update(id, None, |operation| {
            if operation.cancel_requested {
                operation.state = WorkspaceOperationState::Cancelled;
                operation.resume_allowed = false;
            }
            if !step_at_least(operation.step, WorkspaceOperationStep::WorktreeReady) {
                operation.step = WorkspaceOperationStep::WorktreeReady;
            }
            operation.error = None;
            Ok(())
        })?;
        if plan.mode == WorkspaceSetupMode::Create {
            let before = before.expect("Create plan has inventory");
            let repository = repository.expect("Create plan has repository");
            let after = self
                .adapter
                .project_inventory(session, &repository.checkout_path)
                .await?;
            verify_inventory(&after, repository)?;
            if after.endpoint_identity != before.endpoint_identity {
                return Err(InspectionError::new(
                    "stale_identity",
                    "Herdr endpoint identity changed during operation",
                ));
            }
            let entry = after
                .worktrees
                .iter()
                .find(|w| {
                    w.checkout_path == plan.checkout_path
                        && w.open_workspace_id.as_deref() == Some(result.workspace_id.as_str())
                })
                .ok_or_else(|| {
                    InspectionError::new(
                        "workspace_conflict",
                        "fresh inventory did not prove the exact worktree",
                    )
                })?;
            if plan.mode == WorkspaceSetupMode::Create
                && plan.branch.is_some()
                && entry.branch != plan.branch
            {
                return Err(InspectionError::new(
                    "workspace_conflict",
                    "authoritative branch differs from reviewed plan",
                ));
            }
        }
        let operation = self.store.update(id, None, |operation| {
            if !step_at_least(operation.step, WorkspaceOperationStep::WorkspaceVerified) {
                operation.step = WorkspaceOperationStep::WorkspaceVerified;
            }
            operation.error = None;
            if operation.cancel_requested {
                operation.state = WorkspaceOperationState::Cancelled;
                operation.resume_allowed = false;
            }
            Ok(())
        })?;
        if operation.state == WorkspaceOperationState::Cancelled {
            return Err(InspectionError::new("cancelled", "operation was cancelled"));
        }
        Ok(operation)
    }

    async fn verify_environment_target(
        &self,
        session: &str,
        plan: &WorkspaceSetupPlan,
        result: &ProjectWorktreeResult,
    ) -> Result<(), InspectionError> {
        let repository = plan.repository.as_ref();
        if plan.mode == WorkspaceSetupMode::Create {
            let repository = repository.expect("Create plan has repository");
            let env_inventory = self
                .adapter
                .project_inventory(session, &repository.checkout_path)
                .await?;
            verify_inventory(&env_inventory, repository)?;
            if env_inventory.endpoint_identity != plan.endpoint_identity
                || !env_inventory.worktrees.iter().any(|worktree| {
                    worktree.checkout_path == plan.checkout_path
                        && worktree.open_workspace_id.as_deref()
                            == Some(result.workspace_id.as_str())
                })
            {
                return Err(InspectionError::new(
                    "stale_identity",
                    "workspace provenance changed before environment dispatch",
                ));
            }
        } else if self.adapter.project_endpoint_identity(session).await? != plan.endpoint_identity {
            return Err(InspectionError::new(
                "stale_identity",
                "Herdr endpoint changed before environment dispatch",
            ));
        }
        Ok(())
    }

    async fn ensure_environment(
        &self,
        session: &str,
        id: &str,
        mut operation: WorkspaceOperation,
        plan: &WorkspaceSetupPlan,
        result: &ProjectWorktreeResult,
        library: &LibraryService,
    ) -> Result<WorkspaceOperation, InspectionError> {
        let repository = plan.repository.as_ref();
        if operation.pane_id.is_none() {
            let mut env = BTreeMap::new();
            env.insert(
                "COCKPIT_LIBRARY_ROOT".to_owned(),
                library.root_path().to_owned(),
            );
            env.insert(
                "COCKPIT_WORKSPACE_ID".to_owned(),
                result.workspace_id.clone(),
            );
            if let Some(repository) = repository {
                env.insert(
                    "COCKPIT_REPOSITORY_KEY".to_owned(),
                    repository.repository_id.clone(),
                );
            }
            if let Some(artifact) = &plan.artifact {
                env.insert(
                    "COCKPIT_ARTIFACT_URL".to_owned(),
                    artifact.canonical_url.clone(),
                );
            }
            operation = self.mark_dispatch(id, WorkspaceOperationStep::EnvironmentRequested)?;
            if operation.state == WorkspaceOperationState::Cancelled {
                return Err(InspectionError::new("cancelled", "operation was cancelled"));
            }
            let terminal = self
                .adapter
                .project_terminal(
                    session,
                    &ProjectTerminalRequest {
                        endpoint_identity: plan.endpoint_identity.clone(),
                        workspace_id: result.workspace_id.clone(),
                        cwd: result.checkout_path.clone(),
                        label: format!("{} context", plan.label),
                        focus: plan.focus,
                        env,
                    },
                )
                .await?;
            if terminal.workspace_id != result.workspace_id {
                return Err(InspectionError::new(
                    "workspace_conflict",
                    "terminal returned a different workspace",
                ));
            }
            operation = self.store.update(id, None, |operation| {
                operation.tab_id = Some(terminal.tab_id.clone());
                operation.pane_id = Some(terminal.pane_id.clone());
                operation.step = WorkspaceOperationStep::EnvironmentReady;
                operation.error = None;
                if operation.cancel_requested {
                    operation.state = WorkspaceOperationState::Cancelled;
                    operation.resume_allowed = false;
                }
                Ok(())
            })?;
            if operation.state == WorkspaceOperationState::Cancelled {
                return Err(InspectionError::new("cancelled", "operation was cancelled"));
            }
        }
        Ok(operation)
    }

    async fn prepare_context(
        &self,
        session: &str,
        id: &str,
        step: WorkspaceOperationStep,
        plan: &WorkspaceSetupPlan,
        result: &ProjectWorktreeResult,
        library: &LibraryService,
        fetched: Vec<FetchedAssets>,
    ) -> Result<Option<ErrorResponse>, InspectionError> {
        let mut context_error = None;
        if !step_at_least(step, WorkspaceOperationStep::ContextReady) {
            self.store.update(id, None, |operation| {
                operation.step = WorkspaceOperationStep::ContextPreparing;
                operation.state = WorkspaceOperationState::Running;
                operation.resume_allowed = false;
                operation.error = None;
                Ok(())
            })?;
            let materialization = self
                .materialize_context(session, plan, &result.workspace_id, library, fetched)
                .await;
            match materialization {
                Ok(()) => {
                    self.store.update(id, None, |operation| {
                        operation.step = WorkspaceOperationStep::ContextReady;
                        operation.error = None;
                        Ok(())
                    })?;
                }
                Err(error) => {
                    let response = error_response(&error.code, &error.message);
                    context_error = Some(response.clone());
                    self.store.update(id, None, |operation| {
                        operation.step = WorkspaceOperationStep::ContextPreparing;
                        operation.state = WorkspaceOperationState::Partial;
                        operation.resume_allowed = true;
                        operation.error = Some(response.clone());
                        Ok(())
                    })?;
                }
            }
        }
        Ok(context_error)
    }

    async fn materialize_context(
        &self,
        session: &str,
        plan: &WorkspaceSetupPlan,
        workspace_id: &str,
        library: &LibraryService,
        mut fetched: Vec<FetchedAssets>,
    ) -> Result<(), InspectionError> {
        let Some(artifact) = plan.artifact.as_ref() else {
            return Ok::<(), InspectionError>(());
        };
        let target = SpaceTarget {
            session_id: session.to_owned(),
            space_id: workspace_id.to_owned(),
        };
        let mut expected_ids = Vec::new();
        let mut saved_ids = Vec::new();
        for artifact in std::iter::once(artifact).chain(plan.linked_artifacts.iter()) {
            let authority = instance_authority(
                &self.configuration,
                &artifact.provider_id,
                &artifact.canonical_url,
            )?;
            let item_id = crate::sources::source_id(&crate::sources::SourceRef {
                provider_id: artifact.provider_id.clone(),
                provider_instance: authority.provider_instance,
                resource_type: artifact.kind.clone(),
                canonical_id: artifact.canonical_id.clone(),
            });
            let prevalidated = fetched.iter().any(|batch| {
                batch.assets.iter().any(|asset| {
                    asset.source.provider_id == artifact.provider_id
                        && asset.source.resource_type == artifact.kind
                        && asset.source.canonical_id == artifact.canonical_id
                })
            });
            if !prevalidated {
                // Resume selects saved items without another provider read.
                // Only an item never saved in the Library needs fetching.
                let mut offset = None;
                let saved = loop {
                    let listing = library.listing(offset).await?;
                    if listing.items.iter().any(|item| item.item_id == item_id) {
                        break true;
                    }
                    match listing.next_offset {
                        Some(next) => offset = Some(next),
                        None => break false,
                    }
                };
                if saved {
                    saved_ids.push(item_id.clone());
                } else {
                    fetched.push(self.fetch_setup_artifact(artifact).await?);
                }
            }
            expected_ids.push(item_id);
        }
        let response = if fetched.is_empty() {
            let operation = library
                .start_space_add(SpaceAddRequest {
                    target,
                    item_ids: saved_ids,
                })
                .await?;
            loop {
                let response = library.operation(&operation.operation_id).await?;
                if response.finished {
                    break response;
                }
                tokio::time::sleep(std::time::Duration::from_millis(25)).await;
            }
        } else {
            library
                .add_fetched_and_select(target, fetched, saved_ids)
                .await?
        };
        if let Some(phase) = response
            .phases
            .iter()
            .find(|phase| phase.state != LibraryPhaseState::Done)
        {
            return Err(InspectionError::new(
                    if phase.phase == LibraryPhaseName::Space { "source_sync_conflict" }
                    else { phase.error.as_ref().map(|error| error.code.as_str()).unwrap_or("source_provider_contract") },
                    phase.error.as_ref().map(|error| error.message.clone()).unwrap_or_else(||
                        "Setup artifacts were saved in the Library but not selected for the Space; retry the saved Library items".into()),
                ));
        }
        let selected = response
            .space
            .as_ref()
            .filter(|space| space.space_id == workspace_id)
            .ok_or_else(|| {
                InspectionError::new(
                    "source_provider_contract",
                    "Library operation returned no matching Space selection",
                )
            })?;
        if expected_ids
            .iter()
            .any(|id| !selected.item_ids.contains(id))
        {
            return Err(InspectionError::new(
                "source_provider_contract",
                "Library operation did not select every reviewed artifact",
            ));
        }
        Ok::<(), InspectionError>(())
    }

    pub(super) fn mark_dispatch(
        &self,
        id: &str,
        step: WorkspaceOperationStep,
    ) -> Result<WorkspaceOperation, InspectionError> {
        let descriptor = setup_step(step).1;
        let StepAction::Dispatch { code, message } = descriptor.action else {
            return Err(InspectionError::new(
                "invalid_operation_state",
                "invalid dispatch checkpoint",
            ));
        };
        let pending = error_response(code, message);
        self.store.update(id, None, |operation| {
            if operation.cancel_requested {
                operation.state = WorkspaceOperationState::Cancelled;
                operation.resume_allowed = false;
                return Ok(());
            }
            operation.state = WorkspaceOperationState::Running;
            operation.step = descriptor.step;
            operation.error = Some(pending.clone());
            Ok(())
        })
    }

    fn merge_worktree_receipt(
        &self,
        id: &str,
        result: &ProjectWorktreeResult,
        created: bool,
    ) -> Result<ProjectWorktreeResult, InspectionError> {
        self.store.update(id, None, |operation| {
            operation.workspace_id = Some(result.workspace_id.clone());
            // The worktree's root pane is not the context-bearing terminal.
            // Only the environment dispatch may populate tab_id and pane_id.
            let resource_kind =
                if operation.plan.ownership == WorkspaceCheckoutOwnership::OwnedWorktree {
                    "worktree"
                } else {
                    "directory"
                };
            if !operation
                .owned_resources
                .iter()
                .any(|resource| resource.kind == resource_kind)
            {
                operation.owned_resources.push(WorkspaceOwnedResource {
                    kind: resource_kind.to_owned(),
                    path: result.checkout_path.clone(),
                    created_by_operation: created,
                });
            }
            operation.step = WorkspaceOperationStep::HerdrObserved;
            operation.error = None;
            if operation.cancel_requested {
                operation.state = WorkspaceOperationState::Cancelled;
                operation.resume_allowed = false;
            }
            Ok(())
        })?;
        Ok(result.clone())
    }

    async fn boundary(&self, id: &str) -> Result<(), InspectionError> {
        if self.shutting_down.load(Ordering::SeqCst) {
            return Err(InspectionError::new("shutdown", "service is shutting down"));
        }
        if self.is_cancelled(id).await {
            return Err(InspectionError::new("cancelled", "operation was cancelled"));
        }
        Ok(())
    }

    async fn is_cancelled(&self, id: &str) -> bool {
        self.cancelled.lock().await.contains(id)
    }

    async fn fail(&self, id: &str, error: InspectionError) -> Result<(), InspectionError> {
        self.store
            .update(id, None, |operation| {
                if operation.cancel_requested || error.code == "cancelled" {
                    operation.state = WorkspaceOperationState::Cancelled;
                    operation.resume_allowed = false;
                } else if pending_unknown(operation) {
                    operation.state = WorkspaceOperationState::OutcomeUnknown;
                    operation.resume_allowed = false;
                } else if matches!(
                    error.code.as_str(),
                    "workspace_conflict"
                        | "repository_conflict"
                        | "stale_identity"
                        | "association_conflict"
                        | "unsupported_capability"
                        | "invalid_project_request"
                        | "invalid_environment"
                ) {
                    operation.state = WorkspaceOperationState::NeedsReview;
                    operation.resume_allowed = false;
                } else {
                    operation.state = WorkspaceOperationState::Partial;
                    operation.resume_allowed = true;
                }
                operation.error = Some(error_response(&error.code, error.message.clone()));
                Ok(())
            })
            .map(|_| ())
    }

    async fn settle_shutdown(&self, id: &str) -> Result<(), InspectionError> {
        self.store
            .update(id, None, |operation| {
                if operation.cancel_requested {
                    operation.state = WorkspaceOperationState::Cancelled;
                    operation.resume_allowed = false;
                } else if pending_unknown(operation) {
                    operation.state = WorkspaceOperationState::OutcomeUnknown;
                    operation.resume_allowed = false;
                } else if operation.state == WorkspaceOperationState::Running {
                    operation.state = WorkspaceOperationState::Partial;
                    operation.resume_allowed = true;
                }
                Ok(())
            })
            .map(|_| ())
    }

    pub async fn shutdown(&self) {
        self.shutting_down.store(true, Ordering::SeqCst);
        loop {
            if self.workers.lock().await.is_empty() {
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::super::test_support::*;
    use super::*;
    use crate::sources::SourceService;
    use cockpit_protocol::project_teardown::{
        WorkspaceTeardownAction, WorkspaceTeardownExecuteRequest, WorkspaceTeardownPreviewRequest,
    };
    use cockpit_protocol::projects::{ProviderKind, WorkspaceSetupRequest};
    use std::sync::atomic::AtomicUsize;
    use uuid::Uuid;

    #[tokio::test]
    async fn setup_saves_primary_and_linked_artifacts_and_retries_selection_without_fetch() {
        for fail_selection in [false, true] {
            let root =
                std::env::temp_dir().join(format!("cockpit-setup-library-{}", Uuid::new_v4()));
            let repository = root.join("repository");
            std::fs::create_dir_all(&repository).unwrap();
            git(&repository, &["init"]);
            git(
                &repository,
                &[
                    "-c",
                    "user.name=Fixture",
                    "-c",
                    "user.email=fixture@example.test",
                    "commit",
                    "--allow-empty",
                    "-m",
                    "fixture",
                ],
            );
            git(
                &repository,
                &[
                    "remote",
                    "add",
                    "origin",
                    "https://unrelated.test/local/repo.git",
                ],
            );
            let mut configuration = configuration(&root);
            configuration.providers = vec![
                cockpit_protocol::projects::ProjectProvider {
                    id: "tea".into(),
                    kind: ProviderKind::Gitea,
                    base_url: "https://forge.test".into(),
                    executable: Some("tea".into()),
                    login: None,
                    deployment: None,
                },
                cockpit_protocol::projects::ProjectProvider {
                    id: "jira".into(),
                    kind: ProviderKind::Jira,
                    base_url: "https://jira.test".into(),
                    executable: None,
                    login: None,
                    deployment: Some(cockpit_protocol::projects::ProviderDeployment::DataCenter),
                },
            ];
            let calls = Arc::new(AtomicUsize::new(0));
            let providers = configuration
                .providers
                .iter()
                .map(|provider| {
                    Arc::new(SetupProvider {
                        configuration: configuration.clone(),
                        provider_id: provider.id.clone(),
                        calls: calls.clone(),
                    }) as Arc<dyn crate::sources::SourceProvider>
                })
                .collect();
            let sources = Arc::new(SourceService::new(&configuration, providers).unwrap());
            let adapter = Arc::new(NestedDirectoryAdapter::default());
            adapter
                .selection_available
                .store(!fail_selection, Ordering::SeqCst);
            let service = Arc::new(
                ProjectService::new(configuration.clone(), adapter.clone())
                    .unwrap()
                    .with_sources(sources.clone()),
            );
            let candidate = service
                .repositories()
                .await
                .unwrap()
                .repositories
                .into_iter()
                .find(|candidate| candidate.checkout_path == repository.to_string_lossy())
                .unwrap();
            adapter.repository.set(candidate.clone()).unwrap();
            let library = Arc::new(
                LibraryService::new(configuration.clone(), sources).with_herdr(adapter.clone()),
            );
            let plan = service
                .plan(
                    "session",
                    &WorkspaceSetupRequest::Create {
                        repository_id: candidate.repository_id,
                        artifact_url: Some("https://forge.test/other/service/issues/7".into()),
                        linked_artifact_urls: vec!["https://jira.test/browse/OPS-3".into()],
                        branch: Some("setup-library".into()),
                        base_ref: None,
                        checkout_path: None,
                        label: Some("Setup".into()),
                        task_name: None,
                        focus: false,
                    },
                )
                .await
                .unwrap();
            assert!(
                !Path::new(&configuration.library_root).exists(),
                "planning must not persist provider content"
            );
            assert_eq!(
                calls.load(Ordering::SeqCst),
                2,
                "planning validates both artifacts"
            );
            service
                .start(
                    "session",
                    &WorkspaceOperationRequest {
                        operation_id: plan.operation_id.clone(),
                        expected_generation: plan.generation,
                    },
                    library.clone(),
                )
                .await
                .unwrap();
            let mut operation = settled_setup(&service, &plan.operation_id).await;
            assert_eq!(
                calls.load(Ordering::SeqCst),
                2,
                "Library must save the validated assets without another fetch"
            );
            let target = SpaceTarget {
                session_id: "session".into(),
                space_id: "workspace".into(),
            };
            if fail_selection {
                assert_eq!(operation.state, WorkspaceOperationState::Partial);
                assert_eq!(
                    operation.error.as_ref().unwrap().code,
                    "source_sync_conflict"
                );
                assert!(operation.resume_allowed);
                let saved = library.listing(None).await.unwrap();
                let mut identities = saved
                    .items
                    .iter()
                    .filter_map(|item| item.canonical_id.as_deref())
                    .collect::<Vec<_>>();
                identities.sort();
                assert_eq!(
                    identities,
                    vec!["OPS-3", "other/service#7"],
                    "all linked assets must survive a selection failure"
                );
                let fetched = calls.load(Ordering::SeqCst);
                adapter.selection_available.store(true, Ordering::SeqCst);
                service
                    .resume(
                        "session",
                        &WorkspaceOperationRequest {
                            operation_id: operation.operation_id.clone(),
                            expected_generation: operation.generation,
                        },
                        library.clone(),
                    )
                    .await
                    .unwrap();
                operation = settled_setup(&service, &plan.operation_id).await;
                assert_eq!(
                    calls.load(Ordering::SeqCst),
                    fetched,
                    "selection retry must never ask the provider"
                );
            }
            assert_eq!(
                operation.state,
                WorkspaceOperationState::Completed,
                "{operation:?}"
            );
            assert_eq!(
                adapter.terminal_requests.lock().unwrap()[0]
                    .env
                    .get("COCKPIT_LIBRARY_ROOT"),
                Some(&configuration.library_root),
            );
            let items = library.listing(None).await.unwrap().items;
            let mut identities = items
                .iter()
                .filter_map(|item| item.canonical_id.as_deref())
                .collect::<Vec<_>>();
            identities.sort();
            assert_eq!(identities, vec!["OPS-3", "other/service#7"]);
            let listing = library.space_listing(&target).await.unwrap();
            assert_eq!(listing.items.len(), items.len());
            for item in &items {
                assert!(
                    listing
                        .items
                        .iter()
                        .any(|selected| selected.item_id == item.item_id)
                );
                let bytes = std::fs::read_to_string(
                    Path::new(&configuration.library_root)
                        .join(item.document_path.as_ref().unwrap()),
                )
                .unwrap();
                let expected_read = if item.canonical_id.as_deref() == Some("other/service#7") {
                    1
                } else {
                    2
                };
                assert!(
                    bytes.contains(&format!("Setup context body: fetch {expected_read}")),
                    "Library must contain the validated provider result"
                );
            }
            {
                let requests = adapter.terminal_requests.lock().expect("terminal requests");
                let env = &requests[0].env;
                assert_eq!(
                    env.get("COCKPIT_WORKSPACE_ID").map(String::as_str),
                    Some("workspace")
                );
                assert!(!env.contains_key("COCKPIT_CONTEXT_PATH"));
            }
            std::fs::remove_dir_all(root).unwrap();
        }
    }

    #[tokio::test]
    async fn service_opens_and_closes_nested_git_directory_without_worktree_inventory() {
        let root = std::env::temp_dir().join(format!("cockpit-project-open-{}", Uuid::new_v4()));
        let repository = root.join("repository");
        let nested = repository.join("nested");
        let nested_path = nested.to_string_lossy().into_owned();
        std::fs::create_dir_all(&nested).expect("nested directory");
        git(&repository, &["init"]);
        std::fs::write(nested.join("keep.txt"), "keep\n").expect("borrowed file");
        let configuration = configuration(&root);
        std::fs::create_dir_all(&configuration.worktree_root).expect("worktree root");
        let adapter = Arc::new(NestedDirectoryAdapter::default());
        let service = ProjectService::new(configuration, adapter.clone()).expect("project service");

        let plan = service
            .plan(
                "session",
                &WorkspaceSetupRequest::Open {
                    path: nested_path.clone(),
                    label: None,
                    task_name: None,
                    focus: false,
                },
            )
            .await
            .expect("plan nested directory");
        service
            .execute_inner(
                "session",
                &plan.operation_id,
                &LibraryService::new(
                    service.configuration.clone(),
                    Arc::new(SourceService::new(&service.configuration, vec![]).unwrap()),
                ),
                None,
            )
            .await
            .expect("open nested directory");

        let operation = service.store.load(&plan.operation_id).expect("operation");
        assert_eq!(operation.plan.checkout_path, nested_path);
        assert!(operation.owned_resources.iter().any(|resource| {
            resource.kind == "directory"
                && resource.path == nested_path
                && !resource.created_by_operation
        }));
        assert_eq!(adapter.inventory_calls.load(Ordering::Relaxed), 0);
        let worktree_requests = adapter.worktree_requests.lock().expect("worktree requests");
        assert_eq!(worktree_requests.len(), 1);
        assert_eq!(worktree_requests[0].mode, WorkspaceSetupMode::Open);
        assert_eq!(worktree_requests[0].source_cwd, nested_path);
        assert_eq!(worktree_requests[0].checkout_path, nested_path);
        drop(worktree_requests);
        assert_eq!(
            adapter.terminal_requests.lock().expect("terminal requests")[0].cwd,
            nested_path
        );

        let preview = service
            .teardown_preview(
                "session",
                &WorkspaceTeardownPreviewRequest {
                    workspace_id: "workspace".to_owned(),
                },
            )
            .await
            .expect("borrowed directory preview");
        assert_eq!(preview.checkout_path, nested_path);
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
        service
            .teardown_execute(
                "session",
                &WorkspaceTeardownExecuteRequest {
                    operation_id: plan.operation_id,
                    workspace_id: "workspace".to_owned(),
                    expected_endpoint_identity: preview.endpoint_identity,
                    expected_checkout_path: preview.checkout_path,
                    action: WorkspaceTeardownAction::CloseSpace,
                    confirmation: String::new(),
                },
            )
            .await
            .expect("close borrowed directory");
        assert_eq!(adapter.inventory_calls.load(Ordering::Relaxed), 0);
        assert_eq!(
            adapter
                .closed_workspaces
                .lock()
                .expect("closed workspaces")
                .as_slice(),
            ["workspace"]
        );
        assert_eq!(
            std::fs::read_to_string(nested.join("keep.txt")).expect("borrowed file"),
            "keep\n"
        );
        std::fs::remove_dir_all(root).expect("cleanup");
    }
}
