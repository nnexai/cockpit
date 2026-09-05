use std::collections::{BTreeMap, HashSet};
use std::path::Path;
use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};

use cockpit_protocol::context::{ContextRoot, ContextRootKind};
use cockpit_protocol::projects::{
    ProjectArtifact, ProjectConfiguration, RepositoryCandidate, RepositoryListResponse,
    WorkspaceOperation, WorkspaceOperationRequest, WorkspaceOperationState, WorkspaceOperationStep,
    WorkspaceOwnedResource, WorkspaceReconcileRequest, WorkspaceRecoveryAction, WorkspaceSetupMode,
    WorkspaceSetupPlan, WorkspaceSetupRequest,
};
use cockpit_protocol::v1::ErrorResponse;
use sha2::{Digest, Sha256};
use tokio::sync::Mutex;
use uuid::Uuid;

use crate::project_adapter::{
    ProjectTerminalRequest, ProjectWorktreeRequest, ProjectWorktreeResult,
};
use crate::project_store::{
    CompanionManifest, ProjectStore, prepare_project_root, validate_project_root,
};
use crate::repositories::{self, RepositoryCatalog};
use crate::{InspectionError, ProjectHerdrAdapter};
/// sessions, workspaces, tabs, panes, and worktrees; this service only journals
/// its own effects and the companion association.
pub struct ProjectService {
    configuration: ProjectConfiguration,
    adapter: Arc<dyn ProjectHerdrAdapter>,
    store: ProjectStore,
    shutting_down: AtomicBool,
    cancelled: Mutex<HashSet<String>>,
    workers: Mutex<HashSet<String>>,
}

impl ProjectService {
    pub fn new(
        configuration: ProjectConfiguration,
        adapter: Arc<dyn ProjectHerdrAdapter>,
    ) -> Result<Self, InspectionError> {
        let store = ProjectStore::new(&configuration.state_root)?;
        prepare_project_root(Path::new(&configuration.worktree_root))?;
        prepare_project_root(Path::new(&configuration.companion_root))?;
        validate_project_root(Path::new(&configuration.worktree_root))?;
        validate_project_root(Path::new(&configuration.companion_root))?;
        recover_startup(&store)?;
        Ok(Self {
            configuration,
            adapter,
            store,
            shutting_down: AtomicBool::new(false),
            cancelled: Mutex::new(HashSet::new()),
            workers: Mutex::new(HashSet::new()),
        })
    }

    pub fn configuration(&self) -> ProjectConfiguration {
        self.configuration.clone()
    }

    pub async fn repositories(&self) -> Result<RepositoryListResponse, InspectionError> {
        RepositoryCatalog::new(self.configuration.clone())
            .list()
            .await
    }

    pub async fn plan(
        &self,
        session: &str,
        request: &WorkspaceSetupRequest,
    ) -> Result<WorkspaceSetupPlan, InspectionError> {
        validate_session(session)?;
        validate_setup_request(request)?;
        if !request.trust_repository {
            return Err(InspectionError::new(
                "consent_required",
                "repository actions require explicit consent before review",
            ));
        }
        if request.mode == WorkspaceSetupMode::Open
            && (request.branch.is_some() == request.checkout_path.is_some())
        {
            return Err(InspectionError::new(
                "invalid_selector",
                "Open requires exactly one branch or checkout path selector",
            ));
        }
        let catalog = RepositoryCatalog::new(self.configuration.clone());
        let repository = catalog.resolve(&request.repository_id).await?;
        let inventory = self
            .adapter
            .project_inventory(session, &repository.checkout_path)
            .await?;
        verify_inventory(&inventory, &repository)?;
        let mut artifact = match request.artifact_url.as_deref() {
            Some(url) => {
                let artifact = repositories::resolve_artifact(&self.configuration, url)?;
                if artifact.canonical_url.contains('@') {
                    return Err(InspectionError::new(
                        "secret_input",
                        "artifact URL userinfo is not allowed",
                    ));
                }
                Some(artifact)
            }
            None => None,
        };
        let operation_id = Uuid::new_v4().to_string();
        let (checkout_path, branch) = if request.mode == WorkspaceSetupMode::Open {
            let found = if let Some(branch) = request.branch.as_deref() {
                inventory
                    .worktrees
                    .iter()
                    .filter(|entry| entry.branch.as_deref() == Some(branch))
                    .collect::<Vec<_>>()
            } else {
                let path = request.checkout_path.as_deref().expect("selector checked");
                inventory
                    .worktrees
                    .iter()
                    .filter(|entry| entry.checkout_path == path)
                    .collect::<Vec<_>>()
            };
            let entry = match found.as_slice() {
                [entry] => *entry,
                [] => {
                    return Err(InspectionError::new(
                        "worktree_not_found",
                        "Open selector did not match a fresh Herdr worktree listing",
                    ));
                }
                _ => {
                    return Err(InspectionError::new(
                        "worktree_conflict",
                        "Open selector matched multiple worktrees",
                    ));
                }
            };
            (entry.checkout_path.clone(), entry.branch.clone())
        } else {
            let branch = match request.branch.as_deref() {
                Some(branch) => branch.to_owned(),
                None => expand_template(
                    &self.configuration.branch_template,
                    &repository,
                    request,
                    artifact.as_ref(),
                )?,
            };
            catalog.validate_branch(&repository, &branch).await?;
            let path = match request.checkout_path.as_deref() {
                Some(path) => bounded_path(
                    path,
                    Path::new(&self.configuration.worktree_root),
                    "checkout_path",
                )?,
                None => {
                    let path = expand_path_template(
                        &self.configuration.checkout_template,
                        &repository,
                        &operation_id,
                        &slug(
                            request
                                .task_name
                                .as_deref()
                                .or_else(|| artifact.as_ref().map(|a| a.canonical_id.as_str()))
                                .unwrap_or("task"),
                        ),
                    )?;
                    bounded_path(
                        &path,
                        Path::new(&self.configuration.worktree_root),
                        "checkout_path",
                    )?
                }
            };
            (path, Some(branch))
        };
        let base = if request.mode == WorkspaceSetupMode::Open {
            None
        } else {
            match request.base.as_deref() {
                Some(base) => Some(catalog.resolve_base(&repository, base).await?),
                None => None,
            }
        };
        let (companion_id, companion_created_by_operation) =
            if request.mode == WorkspaceSetupMode::Open {
                let matches = self
                    .store
                    .list_companions(&self.configuration.companion_root)?
                    .into_iter()
                    .filter(|(_, manifest)| {
                        manifest.repository_key == repository.common_dir
                            && manifest.repository_root == repository.root
                            && manifest.checkout_path == checkout_path
                    })
                    .collect::<Vec<_>>();
                match matches.as_slice() {
                    [] => (operation_id.clone(), true),
                    [(id, manifest)]
                        if manifest.ownership == "cockpit"
                            && artifact.as_ref().map_or(true, |requested| {
                                Some(requested) == manifest.artifact.as_ref()
                            }) =>
                    {
                        if artifact.is_none() {
                            artifact = manifest.artifact.clone();
                        }
                        (id.clone(), false)
                    }
                    [(_, _)] => {
                        return Err(InspectionError::new(
                            "association_conflict",
                            "existing checkout companion metadata differs from the reviewed plan",
                        ));
                    }
                    _ => {
                        return Err(InspectionError::new(
                            "association_conflict",
                            "multiple companions are associated with the exact checkout",
                        ));
                    }
                }
            } else {
                (operation_id.clone(), true)
            };
        let companion_path = bounded_path(
            &companion_id,
            Path::new(&self.configuration.companion_root),
            "companion_path",
        )?;
        let label = request.label.clone().unwrap_or_else(|| {
            request
                .task_name
                .clone()
                .unwrap_or_else(|| repository.name.clone())
        });
        validate_text(&label, "label", 256)?;
        let mut effects = vec![
            format!(
                "{} Herdr worktree at {}",
                if request.mode == WorkspaceSetupMode::Create {
                    "Create"
                } else {
                    "Open"
                },
                checkout_path
            ),
            format!("Associate Cockpit companion at {}", companion_path),
            "Create a new context-aware terminal with allowlisted COCKPIT_* environment".to_owned(),
        ];
        if request.mode == WorkspaceSetupMode::Open {
            effects
                .push("Borrow existing checkout; Cockpit will not claim or delete it".to_owned());
            if !companion_created_by_operation {
                effects.push(
                    "Borrow existing companion; preserve its creation ownership and user files"
                        .to_owned(),
                );
            }
        }
        if artifact.is_some() {
            effects.push("Record the selected artifact in the companion manifest".to_owned());
        }
        let plan = WorkspaceSetupPlan {
            operation_id,
            generation: 1,
            endpoint_identity: inventory.endpoint_identity.clone(),
            session_id: session.to_owned(),
            repository,
            mode: request.mode,
            branch,
            base,
            checkout_path,
            companion_path,
            companion_id,
            companion_created_by_operation,
            label,
            focus: request.focus,
            trust_repository: request.trust_repository,
            artifact,
            effects,
            warnings: Vec::new(),
        };
        self.store.persist_plan(plan.clone())?;
        Ok(plan)
    }

    pub async fn start(
        self: &Arc<Self>,
        session: &str,
        request: &WorkspaceOperationRequest,
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
            service.execute(&sid, &id, lease).await;
        });
        Ok(started)
    }

    pub async fn get(
        &self,
        session: &str,
        id: &str,
    ) -> Result<WorkspaceOperation, InspectionError> {
        validate_session(session)?;
        let operation = self.store.load(id)?;
        if operation.session_id != session {
            return Err(InspectionError::new(
                "stale_identity",
                "operation belongs to another session",
            ));
        }
        Ok(operation)
    }
    /// Return companions whose durable metadata is still authorized by the
    /// current Herdr endpoint and a fresh worktree inventory. Paths supplied by
    /// callers are never used as authorization.
    pub async fn context_companions(
        &self,
        session_id: &str,
        workspace_id: &str,
        endpoint_identity: &str,
    ) -> Result<Vec<ContextRoot>, InspectionError> {
        validate_identity(workspace_id, "workspace_id")?;
        validate_session(session_id)?;
        validate_text(endpoint_identity, "endpoint_identity", 4096)?;

        let operations = self.store.list()?;
        let manifests = self
            .store
            .list_companions(&self.configuration.companion_root)?;
        let mut roots = Vec::new();
        let mut seen_checkouts = HashSet::new();
        for (companion_id, manifest) in manifests {
            if manifest.herdr_session_identity != endpoint_identity
                || manifest.herdr_workspace_id != workspace_id
            {
                continue;
            }
            if manifest.ownership != "cockpit" {
                return Err(InspectionError::new(
                    "association_conflict",
                    "matching companion is not Cockpit-owned",
                ));
            }
            let operation = operations
                .iter()
                .find(|operation| operation.operation_id == manifest.cockpit_operation_id)
                .ok_or_else(|| {
                    InspectionError::new(
                        "association_conflict",
                        "companion creator operation is missing",
                    )
                })?;
            if operation.companion_id.as_deref() != Some(companion_id.as_str())
                || !operation
                    .owned_resources
                    .iter()
                    .any(|resource| resource.kind == "companion" && resource.created_by_operation)
                || operation.plan.companion_id != companion_id
                || operation.plan.repository.common_dir != manifest.repository_key
                || operation.plan.repository.root != manifest.repository_root
                || operation.plan.checkout_path != manifest.checkout_path
            {
                return Err(InspectionError::new(
                    "association_conflict",
                    "companion creator provenance differs",
                ));
            }
            if !seen_checkouts.insert(manifest.checkout_path.clone()) {
                return Err(InspectionError::new(
                    "association_conflict",
                    "multiple companions are associated with the exact checkout",
                ));
            }
            let inventory = self
                .adapter
                .project_inventory(session_id, &manifest.checkout_path)
                .await?;
            if inventory.endpoint_identity != endpoint_identity
                || inventory.repository_key != manifest.repository_key
                || inventory.repository_root != manifest.repository_root
            {
                return Err(InspectionError::new(
                    "stale_identity",
                    "companion provenance is not confirmed by the current Herdr endpoint",
                ));
            }
            let matches = inventory
                .worktrees
                .iter()
                .filter(|entry| {
                    entry.checkout_path == manifest.checkout_path
                        && entry.open_workspace_id.as_deref() == Some(workspace_id)
                })
                .count();
            if matches != 1 {
                return Err(InspectionError::new(
                    "association_conflict",
                    "fresh inventory did not prove exactly one companion worktree",
                ));
            }
            let companion_path = Path::new(&self.configuration.companion_root).join(&companion_id);
            let root_id =
                stable_companion_root_id(&companion_path.to_string_lossy(), &companion_id)?;
            roots.push(ContextRoot {
                root_id,
                kind: ContextRootKind::Companion,
                label: format!("{} context", operation.plan.repository.name),
                path: Path::new(&self.configuration.companion_root)
                    .join(&companion_id)
                    .to_string_lossy()
                    .into_owned(),
                repository_id: operation.plan.repository.repository_id.clone(),
                checkout_path: manifest.checkout_path.clone(),
                companion_id: Some(companion_id),
            });
        }
        roots.sort_by(|left, right| left.root_id.cmp(&right.root_id));
        Ok(roots)
    }

    pub async fn resume(
        self: &Arc<Self>,
        session: &str,
        request: &WorkspaceOperationRequest,
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
            service.execute(&sid, &id, lease).await;
        });
        Ok(resumed)
    }
    pub async fn reconcile(
        self: &Arc<Self>,
        session: &str,
        request: &WorkspaceReconcileRequest,
    ) -> Result<WorkspaceOperation, InspectionError> {
        validate_session(session)?;
        validate_reconcile_request(session, request)?;
        let mut operation = self.store.load(&request.operation_id)?;
        if operation.session_id != session || operation.generation != request.expected_generation {
            return Err(InspectionError::new(
                "stale_identity",
                "operation session or generation is stale",
            ));
        }
        let lease = self
            .store
            .acquire_execution_lease(&operation.operation_id)?;
        let fresh_repository = RepositoryCatalog::new(self.configuration.clone())
            .resolve(&operation.plan.repository.repository_id)
            .await?;
        if fresh_repository.root != operation.plan.repository.root
            || fresh_repository.common_dir != operation.plan.repository.common_dir
            || fresh_repository.checkout_path != operation.plan.repository.checkout_path
        {
            return Err(InspectionError::new(
                "repository_identity_stale",
                "repository changed since operation planning",
            ));
        }
        let result = match request.action {
            WorkspaceRecoveryAction::AcceptExistingWorktree => {
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
                    .project_inventory(session, &operation.plan.repository.checkout_path)
                    .await?;
                verify_inventory(&inventory, &operation.plan.repository)?;
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
                    let opened = self
                        .adapter
                        .project_worktree(
                            session,
                            &ProjectWorktreeRequest {
                                endpoint_identity: operation.plan.endpoint_identity.clone(),
                                mode: WorkspaceSetupMode::Open,
                                source_cwd: operation.plan.repository.checkout_path.clone(),
                                branch: None,
                                base: None,
                                checkout_path: operation.plan.checkout_path.clone(),
                                label: operation.plan.label.clone(),
                                focus: operation.plan.focus,
                                trust_repository: operation.plan.trust_repository,
                            },
                        )
                        .await?;
                    verify_worktree_result(&opened, &operation.plan)?;
                    let reopened = self
                        .adapter
                        .project_inventory(session, &operation.plan.repository.checkout_path)
                        .await?;
                    verify_inventory(&reopened, &operation.plan.repository)?;
                    if reopened.endpoint_identity != operation.plan.endpoint_identity
                        || !reopened.worktrees.iter().any(|candidate| {
                            candidate.checkout_path == operation.plan.checkout_path
                                && candidate.open_workspace_id.as_deref()
                                    == Some(opened.workspace_id.as_str())
                        })
                    {
                        return Err(InspectionError::new(
                            "workspace_conflict",
                            "explicit Open did not prove the recovered workspace",
                        ));
                    }
                    opened.workspace_id
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
                        operation.step = WorkspaceOperationStep::HerdrObserved;
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
                )?
            }
            WorkspaceRecoveryAction::RetryEnvironment => {
                if operation.workspace_id.is_none()
                    || operation.companion_id.is_none()
                    || operation.pane_id.is_some()
                    || !matches!(
                        operation.step,
                        WorkspaceOperationStep::CompanionReady
                            | WorkspaceOperationStep::EnvironmentRequested
                    )
                {
                    return Err(InspectionError::new(
                        "invalid_reconcile",
                        "environment retry requires a verified workspace and companion",
                    ));
                }
                let inventory = self
                    .adapter
                    .project_inventory(session, &operation.plan.repository.checkout_path)
                    .await?;
                verify_inventory(&inventory, &operation.plan.repository)?;
                if inventory.endpoint_identity != operation.plan.endpoint_identity
                    || !inventory.worktrees.iter().any(|entry| {
                        entry.checkout_path == operation.plan.checkout_path
                            && entry.open_workspace_id.as_deref()
                                == operation.workspace_id.as_deref()
                    })
                {
                    return Err(InspectionError::new(
                        "stale_identity",
                        "workspace provenance changed while reconciling environment",
                    ));
                }
                self.store.update(&operation.operation_id, Some(operation.generation), |operation| {
                    operation.step = WorkspaceOperationStep::CompanionReady;
                    if operation.cancel_requested {
                        operation.state = WorkspaceOperationState::Cancelled;
                        operation.resume_allowed = false;
                    } else {
                        operation.state = WorkspaceOperationState::Partial;
                        operation.resume_allowed = true;
                    }
                    operation.error = Some(error_response("environment_retry_acknowledged", "retry may create a new environment tab; prior uncertain panes are untouched"));
                    Ok(())
                })?
            }
        };
        drop(lease);
        Ok(result)
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

    async fn execute(&self, session: &str, id: &str, _lease: crate::project_store::ExecutionLease) {
        let result = self.execute_inner(session, id).await;
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

    async fn execute_inner(&self, session: &str, id: &str) -> Result<(), InspectionError> {
        self.boundary(id).await?;
        let mut operation = self.store.load(id)?;
        let plan = operation.plan.clone();
        validate_project_root(Path::new(&self.configuration.worktree_root))?;
        validate_project_root(Path::new(&self.configuration.companion_root))?;
        let fresh_repository = RepositoryCatalog::new(self.configuration.clone())
            .resolve(&plan.repository.repository_id)
            .await?;
        if fresh_repository.root != plan.repository.root
            || fresh_repository.common_dir != plan.repository.common_dir
            || fresh_repository.checkout_path != plan.repository.checkout_path
        {
            return Err(InspectionError::new(
                "repository_identity_stale",
                "repository changed since operation planning",
            ));
        }
        let before = self
            .adapter
            .project_inventory(session, &plan.repository.checkout_path)
            .await?;
        verify_inventory(&before, &plan.repository)?;
        if before.endpoint_identity != plan.endpoint_identity {
            return Err(InspectionError::new(
                "stale_identity",
                "Herdr endpoint identity differs from reviewed plan",
            ));
        }
        let borrowed = operation
            .owned_resources
            .iter()
            .any(|resource| resource.kind == "worktree" && !resource.created_by_operation);
        let result = if let Some(workspace_id) = operation.workspace_id.clone() {
            if borrowed && operation.step == WorkspaceOperationStep::HerdrObserved {
                operation = self.mark_dispatch(id, WorkspaceOperationStep::HerdrRequested)?;
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
                            source_cwd: plan.repository.checkout_path.clone(),
                            branch: None,
                            base: None,
                            checkout_path: plan.checkout_path.clone(),
                            label: plan.label.clone(),
                            focus: plan.focus,
                            trust_repository: plan.trust_repository,
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
            operation = self.mark_dispatch(id, WorkspaceOperationStep::HerdrRequested)?;
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
                        source_cwd: plan.repository.checkout_path.clone(),
                        branch: if plan.mode == WorkspaceSetupMode::Open {
                            None
                        } else {
                            plan.branch.clone()
                        },
                        base: plan.base.clone(),
                        checkout_path: plan.checkout_path.clone(),
                        label: plan.label.clone(),
                        focus: plan.focus,
                        trust_repository: plan.trust_repository,
                    },
                )
                .await?;
            verify_worktree_result(&created, &plan)?;
            self.merge_worktree_receipt(
                id,
                &created,
                plan.mode == WorkspaceSetupMode::Create && !created.already_open,
            )?
        };
        verify_worktree_result(&result, &plan)?;
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
        let after = self
            .adapter
            .project_inventory(session, &plan.repository.checkout_path)
            .await?;
        verify_inventory(&after, &plan.repository)?;
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
        let companion_id = plan.companion_id.as_str();
        let companion = match self
            .store
            .read_companion(&self.configuration.companion_root, companion_id)
        {
            Ok(existing) => {
                verify_manifest_identity(&existing, &plan, companion_id)?;
                let existing = if existing.herdr_session_identity == before.endpoint_identity
                    && existing.herdr_workspace_id == result.workspace_id
                {
                    existing
                } else if plan.mode == WorkspaceSetupMode::Open
                    && !plan.companion_created_by_operation
                {
                    let reattached = CompanionManifest {
                        herdr_session_identity: before.endpoint_identity.clone(),
                        herdr_workspace_id: result.workspace_id.clone(),
                        updated_at: now(),
                        ..existing
                    };
                    self.store
                        .reattach_companion(&self.configuration.companion_root, &reattached)?;
                    reattached
                } else {
                    return Err(InspectionError::new(
                        "association_conflict",
                        "existing companion is attached to a different Herdr workspace",
                    ));
                };
                verify_manifest(
                    &existing,
                    &plan,
                    &result.workspace_id,
                    &before.endpoint_identity,
                    companion_id,
                )?;
                Path::new(&self.configuration.companion_root).join(companion_id)
            }
            Err(error) if error.code == "companion_missing" => {
                if !plan.companion_created_by_operation {
                    return Err(InspectionError::new(
                        "association_conflict",
                        "borrowed companion disappeared during operation",
                    ));
                }
                let manifest = CompanionManifest {
                    schema_version: 1,
                    cockpit_operation_id: companion_id.to_owned(),
                    herdr_session_identity: before.endpoint_identity.clone(),
                    herdr_workspace_id: result.workspace_id.clone(),
                    repository_key: plan.repository.common_dir.clone(),
                    repository_root: plan.repository.root.clone(),
                    checkout_path: result.checkout_path.clone(),
                    artifact: plan.artifact.clone(),
                    created_at: now(),
                    updated_at: now(),
                    ownership: "cockpit".to_owned(),
                };
                self.store
                    .write_companion(&self.configuration.companion_root, &manifest)?
            }
            Err(error) => return Err(error),
        };
        operation = self.store.update(id, None, |operation| {
            operation.companion_id = Some(companion_id.to_owned());
            if !operation
                .owned_resources
                .iter()
                .any(|resource| resource.kind == "companion")
            {
                operation.owned_resources.push(WorkspaceOwnedResource {
                    kind: "companion".to_owned(),
                    path: companion.to_string_lossy().into_owned(),
                    created_by_operation: plan.companion_created_by_operation,
                });
            }
            if !step_at_least(operation.step, WorkspaceOperationStep::CompanionReady) {
                operation.step = WorkspaceOperationStep::CompanionReady;
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
        let env_inventory = self
            .adapter
            .project_inventory(session, &plan.repository.checkout_path)
            .await?;
        verify_inventory(&env_inventory, &plan.repository)?;
        if env_inventory.endpoint_identity != plan.endpoint_identity
            || !env_inventory.worktrees.iter().any(|worktree| {
                worktree.checkout_path == plan.checkout_path
                    && worktree.open_workspace_id.as_deref() == Some(result.workspace_id.as_str())
            })
        {
            return Err(InspectionError::new(
                "stale_identity",
                "workspace provenance changed before environment dispatch",
            ));
        }
        if operation.pane_id.is_none() {
            let mut env = BTreeMap::new();
            env.insert(
                "COCKPIT_CONTEXT_PATH".to_owned(),
                companion.to_string_lossy().into_owned(),
            );
            env.insert(
                "COCKPIT_WORKSPACE_ID".to_owned(),
                result.workspace_id.clone(),
            );
            env.insert(
                "COCKPIT_REPOSITORY_KEY".to_owned(),
                plan.repository.repository_id.clone(),
            );
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
        self.boundary(id).await?;
        let completed = self.store.update(id, None, |operation| {
            if operation.cancel_requested {
                operation.state = WorkspaceOperationState::Cancelled;
                operation.resume_allowed = false;
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

    fn mark_dispatch(
        &self,
        id: &str,
        step: WorkspaceOperationStep,
    ) -> Result<WorkspaceOperation, InspectionError> {
        let pending = match step {
            WorkspaceOperationStep::HerdrRequested => {
                error_response("herdr_dispatch_pending", "awaiting Herdr worktree outcome")
            }
            WorkspaceOperationStep::EnvironmentRequested => error_response(
                "environment_dispatch_pending",
                "awaiting terminal dispatch outcome",
            ),
            _ => {
                return Err(InspectionError::new(
                    "invalid_operation_state",
                    "invalid dispatch checkpoint",
                ));
            }
        };
        self.store.update(id, None, |operation| {
            if operation.cancel_requested {
                operation.state = WorkspaceOperationState::Cancelled;
                operation.resume_allowed = false;
                return Ok(());
            }
            operation.state = WorkspaceOperationState::Running;
            operation.step = step;
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
            if !operation
                .owned_resources
                .iter()
                .any(|resource| resource.kind == "worktree")
            {
                operation.owned_resources.push(WorkspaceOwnedResource {
                    kind: "worktree".to_owned(),
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
                        | "consent_required"
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
fn pending_unknown(operation: &WorkspaceOperation) -> bool {
    (operation.step == WorkspaceOperationStep::HerdrRequested && operation.workspace_id.is_none())
        || (operation.step == WorkspaceOperationStep::EnvironmentRequested
            && operation.pane_id.is_none())
}

fn recover_startup(store: &ProjectStore) -> Result<(), InspectionError> {
    for operation in store.list()? {
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

fn step_at_least(current: WorkspaceOperationStep, wanted: WorkspaceOperationStep) -> bool {
    use WorkspaceOperationStep::*;
    let rank = |step| match step {
        Planned => 0,
        Validated => 1,
        HerdrRequested => 2,
        HerdrObserved => 3,
        WorktreeReady => 4,
        WorkspaceVerified => 5,
        CompanionReady => 6,
        EnvironmentRequested => 7,
        EnvironmentReady => 8,
        Completed => 9,
    };
    rank(current) >= rank(wanted)
}

fn verify_inventory(
    inventory: &crate::project_adapter::ProjectInventory,
    repository: &RepositoryCandidate,
) -> Result<(), InspectionError> {
    if inventory.repository_key != repository.common_dir
        || inventory.repository_root != repository.root
    {
        return Err(InspectionError::new(
            "repository_conflict",
            "authoritative repository provenance differs",
        ));
    }
    if inventory.endpoint_identity.is_empty() {
        return Err(InspectionError::new(
            "stale_identity",
            "Herdr endpoint identity is missing",
        ));
    }
    Ok(())
}

fn verify_worktree_result(
    result: &crate::project_adapter::ProjectWorktreeResult,
    plan: &WorkspaceSetupPlan,
) -> Result<(), InspectionError> {
    if result.workspace_id.is_empty()
        || result.checkout_path != plan.checkout_path
        || (plan.branch.is_some() && result.branch != plan.branch)
    {
        return Err(InspectionError::new(
            "workspace_conflict",
            "Herdr result differs from reviewed worktree",
        ));
    }
    Ok(())
}
fn verify_manifest_identity(
    manifest: &CompanionManifest,
    plan: &WorkspaceSetupPlan,
    companion_id: &str,
) -> Result<(), InspectionError> {
    if manifest.schema_version != 1
        || manifest.cockpit_operation_id != companion_id
        || manifest.repository_key != plan.repository.common_dir
        || manifest.repository_root != plan.repository.root
        || manifest.checkout_path != plan.checkout_path
        || manifest.artifact != plan.artifact
        || manifest.ownership != "cockpit"
    {
        return Err(InspectionError::new(
            "association_conflict",
            "existing companion provenance differs",
        ));
    }
    Ok(())
}

fn verify_manifest(
    manifest: &CompanionManifest,
    plan: &WorkspaceSetupPlan,
    workspace_id: &str,
    endpoint_identity: &str,
    companion_id: &str,
) -> Result<(), InspectionError> {
    verify_manifest_identity(manifest, plan, companion_id)?;
    if manifest.herdr_session_identity != endpoint_identity
        || manifest.herdr_workspace_id != workspace_id
    {
        return Err(InspectionError::new(
            "association_conflict",
            "existing companion endpoint/workspace differs",
        ));
    }
    Ok(())
}

fn stable_companion_root_id(
    root_path: &str,
    companion_id: &str,
) -> Result<String, InspectionError> {
    let path = Path::new(root_path);
    let metadata = std::fs::symlink_metadata(path).map_err(|error| {
        InspectionError::new(
            "companion_unavailable",
            format!("cannot inspect companion: {error}"),
        )
    })?;
    if !metadata.is_dir() || metadata.file_type().is_symlink() {
        return Err(InspectionError::new(
            "unsafe_path",
            "companion root identity is not a real directory",
        ));
    }
    let mut hasher = Sha256::new();
    hasher.update(companion_id.as_bytes());
    hasher.update([0]);
    hasher.update(root_path.as_bytes());
    hasher.update([0]);
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        hasher.update(metadata.dev().to_le_bytes());
        hasher.update(metadata.ino().to_le_bytes());
    }
    #[cfg(not(unix))]
    {
        hasher.update(metadata.len().to_le_bytes());
        hasher.update(
            metadata
                .modified()
                .ok()
                .and_then(|time| time.duration_since(std::time::UNIX_EPOCH).ok())
                .map(|duration| duration.as_nanos().to_le_bytes().to_vec())
                .unwrap_or_default(),
        );
    }
    let digest = hasher.finalize();
    let mut encoded = String::with_capacity(digest.len() * 2);
    for byte in digest {
        use std::fmt::Write as _;
        let _ = write!(encoded, "{byte:02x}");
    }
    Ok(format!("companion-{encoded}"))
}

fn validate_setup_request(request: &WorkspaceSetupRequest) -> Result<(), InspectionError> {
    if request.repository_id.is_empty() {
        return Err(InspectionError::new(
            "invalid_repository",
            "repository selection is required",
        ));
    }
    if let Some(branch) = &request.branch {
        validate_text(branch, "branch", 256)?;
    }
    if let Some(base) = &request.base {
        validate_text(base, "base", 256)?;
    }
    if let Some(path) = &request.checkout_path {
        validate_text(path, "checkout_path", 4096)?;
    }
    if let Some(url) = &request.artifact_url {
        validate_text(url, "artifact_url", 2048)?;
    }
    Ok(())
}

fn validate_operation_request(
    session: &str,
    request: &WorkspaceOperationRequest,
) -> Result<(), InspectionError> {
    if request.operation_id.is_empty() || request.expected_generation == 0 || session.is_empty() {
        return Err(InspectionError::new(
            "invalid_request",
            "operation identity is invalid",
        ));
    }
    Ok(())
}

fn validate_session(session: &str) -> Result<(), InspectionError> {
    if session.is_empty() || session.len() > 256 || session.contains('/') || session.contains('\\')
    {
        return Err(InspectionError::new(
            "invalid_session",
            "invalid session identity",
        ));
    }
    Ok(())
}

fn validate_identity(value: &str, field: &str) -> Result<(), InspectionError> {
    validate_text(value, field, 256)?;
    if value.contains('/') || value.contains('\\') {
        return Err(InspectionError::new(
            "invalid_identity",
            format!("{field} contains a path separator"),
        ));
    }
    Ok(())
}

fn validate_text(value: &str, field: &str, max: usize) -> Result<(), InspectionError> {
    if value.is_empty()
        || value.len() > max
        || value.contains('\0')
        || value.chars().any(|c| c.is_control())
    {
        return Err(InspectionError::new(
            "invalid_input",
            format!("{field} is empty, too long, or contains control characters"),
        ));
    }
    Ok(())
}

fn bounded_path(value: &str, root: &Path, field: &str) -> Result<String, InspectionError> {
    validate_text(value, field, 4096)?;
    let root = std::fs::canonicalize(root).unwrap_or_else(|_| root.to_path_buf());
    let path = Path::new(value);
    let joined = if path.is_absolute() {
        path.to_path_buf()
    } else {
        if path
            .components()
            .any(|c| matches!(c, std::path::Component::ParentDir))
        {
            return Err(InspectionError::new(
                "path_outside_root",
                format!("{field} contains parent traversal"),
            ));
        }
        root.join(path)
    };
    if joined == root {
        return Err(InspectionError::new(
            "path_collision",
            format!("{field} cannot be configured root"),
        ));
    }
    let mut existing = joined.clone();
    loop {
        match std::fs::symlink_metadata(&existing) {
            Ok(metadata) => {
                if metadata.file_type().is_symlink() {
                    return Err(InspectionError::new(
                        "unsafe_path",
                        format!("{field} contains a symlink"),
                    ));
                }
                break;
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                if !existing.pop() {
                    return Err(InspectionError::new(
                        "path_unavailable",
                        format!("{field} has no existing root"),
                    ));
                }
            }
            Err(error) => return Err(InspectionError::new("path_unavailable", error.to_string())),
        }
    }
    let canonical_existing = std::fs::canonicalize(&existing)
        .map_err(|e| InspectionError::new("path_unavailable", e.to_string()))?;
    if !canonical_existing.starts_with(&root) || (path.is_absolute() && !joined.starts_with(&root))
    {
        return Err(InspectionError::new(
            "path_outside_root",
            format!("{field} is outside configured root"),
        ));
    }
    Ok(joined.to_string_lossy().into_owned())
}

fn expand_template(
    template: &str,
    repository: &RepositoryCandidate,
    request: &WorkspaceSetupRequest,
    artifact: Option<&ProjectArtifact>,
) -> Result<String, InspectionError> {
    let task = slug(
        request
            .task_name
            .as_deref()
            .or_else(|| artifact.map(|a| a.canonical_id.as_str()))
            .unwrap_or("task"),
    );
    let id = artifact.map(|a| a.canonical_id.as_str()).unwrap_or("task");
    let value = template
        .replace("{repo}", &repository.name)
        .replace("{task_id}", id)
        .replace("{slug}", &task);
    validate_text(&value, "branch", 256)?;
    Ok(value)
}

fn expand_path_template(
    template: &str,
    repository: &RepositoryCandidate,
    operation_id: &str,
    slug: &str,
) -> Result<String, InspectionError> {
    let value = template
        .replace("{repo}", &repository.name)
        .replace("{task_id}", operation_id)
        .replace("{slug}", slug);
    validate_text(&value, "checkout_path", 4096)?;
    Ok(value)
}

fn slug(value: &str) -> String {
    let mut out = String::new();
    for c in value.chars() {
        if c.is_ascii_alphanumeric() {
            out.push(c.to_ascii_lowercase());
        } else if !out.ends_with('-') {
            out.push('-');
        }
    }
    out.trim_matches('-').chars().take(64).collect::<String>()
}

fn error_response(code: &str, message: impl Into<String>) -> ErrorResponse {
    ErrorResponse {
        code: code.to_owned(),
        message: message.into(),
    }
}
fn now() -> String {
    format!(
        "{}",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis()
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn relative_and_absolute_destinations_are_contained() {
        let root = std::env::temp_dir().join(format!("cockpit-project-path-{}", Uuid::new_v4()));
        std::fs::create_dir_all(&root).expect("root");
        let relative = bounded_path("nested/checkout", &root, "checkout").expect("relative path");
        assert!(Path::new(&relative).starts_with(&root));
        let absolute = bounded_path(&relative, &root, "checkout").expect("absolute path");
        assert_eq!(absolute, relative);
        assert!(bounded_path("../escape", &root, "checkout").is_err());
        std::fs::remove_dir_all(root).expect("cleanup");
    }

    #[cfg(unix)]
    #[test]
    fn destination_symlink_substitution_is_rejected() {
        use std::os::unix::fs::symlink;
        let root =
            std::env::temp_dir().join(format!("cockpit-project-path-link-{}", Uuid::new_v4()));
        let outside =
            std::env::temp_dir().join(format!("cockpit-project-path-out-{}", Uuid::new_v4()));
        std::fs::create_dir_all(&root).expect("root");
        std::fs::create_dir_all(&outside).expect("outside");
        symlink(&outside, root.join("nested")).expect("link");
        let error = bounded_path("nested/checkout", &root, "checkout").expect_err("symlink escape");
        assert_eq!(error.code, "unsafe_path");
        std::fs::remove_dir_all(root).expect("cleanup root");
        std::fs::remove_dir_all(outside).expect("cleanup outside");
    }
}
