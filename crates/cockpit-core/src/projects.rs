mod defaults;

use std::collections::{BTreeMap, HashSet};
use std::path::Path;
use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};

use cockpit_protocol::context::{ContextRoot, ContextRootKind};
use cockpit_protocol::project_teardown::{
    WorkspaceTeardownExecuteRequest, WorkspaceTeardownOutcome, WorkspaceTeardownPreview,
    WorkspaceTeardownPreviewRequest, WorkspaceTeardownRecovery, WorkspaceTeardownRecoveryList,
    WorkspaceTeardownRecoveryState, WorkspaceTeardownResult,
};
use cockpit_protocol::projects::{
    ProjectArtifact, ProjectConfiguration, RepositoryCandidate, RepositoryListResponse,
    WorkspaceCheckoutOwnership, WorkspaceOperation, WorkspaceOperationRequest,
    WorkspaceOperationState, WorkspaceOperationStep, WorkspaceOwnedResource,
    WorkspaceReconcileRequest, WorkspaceRecoveryAction, WorkspaceSetupMode, WorkspaceSetupPlan,
    WorkspaceSetupRequest,
};
use cockpit_protocol::v1::ErrorResponse;
use sha2::{Digest, Sha256};
use tokio::sync::Mutex;
use uuid::Uuid;

use crate::project_adapter::{
    ProjectInventory, ProjectTerminalRequest, ProjectWorktreeRemoveRequest, ProjectWorktreeRequest,
    ProjectWorktreeResult,
};
use crate::project_store::{
    CompanionManifest, ProjectStore, TeardownReceipt, TeardownReceiptState, prepare_project_root,
    prepare_root, timestamp, validate_project_root,
};
use crate::project_teardown::{
    self, WorkspaceTeardownCommand, WorkspaceTeardownEvidence, WorkspaceTeardownWorktree,
};
use crate::repositories::{self, RepositoryCatalog};
use crate::sources::{
    SourceFetchRequest, SourceMetadata, SourceService, source_authority_for_checkout,
};
use crate::{InspectionError, ProjectHerdrAdapter};
/// sessions, workspaces, tabs, panes, and worktrees; this service only journals
/// its own effects and the companion association.
pub struct ProjectService {
    configuration: ProjectConfiguration,
    adapter: Arc<dyn ProjectHerdrAdapter>,
    store: ProjectStore,
    sources: Option<Arc<SourceService>>,
    shutting_down: AtomicBool,
    cancelled: Mutex<HashSet<String>>,
    workers: Mutex<HashSet<String>>,
}

struct FreshTeardownEvidence {
    operation: WorkspaceOperation,
    companion: Option<CompanionManifest>,
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
            companion: self.companion.as_ref(),
            endpoint_identity: &self.endpoint_identity,
            repository_key: &self.repository_key,
            repository_root: &self.repository_root,
            worktrees: &self.worktrees,
            receipt: self.receipt.as_ref(),
        }
    }
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
            sources: None,
            shutting_down: AtomicBool::new(false),
            cancelled: Mutex::new(HashSet::new()),
            workers: Mutex::new(HashSet::new()),
        })
    }

    pub fn configuration(&self) -> ProjectConfiguration {
        self.configuration.clone()
    }

    pub fn with_sources(mut self, sources: Arc<SourceService>) -> Self {
        self.sources = Some(sources);
        self
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
        let catalog = RepositoryCatalog::new(self.configuration.clone());
        let operation_id = Uuid::new_v4().to_string();
        let (
            repository,
            mode,
            ownership,
            branch,
            base,
            checkout_path,
            companion_id,
            companion_created_by_operation,
            label,
            focus,
            artifact,
            effects,
        ) = match request {
            WorkspaceSetupRequest::Create {
                repository_id,
                branch,
                base_ref,
                checkout_path,
                label,
                task_name,
                artifact_url,
                focus,
            } => {
                let repository = catalog.resolve(repository_id).await?;
                let inventory = self
                    .adapter
                    .project_inventory(session, &repository.checkout_path)
                    .await?;
                verify_inventory(&inventory, &repository)?;
                let mut artifact = match artifact_url.as_deref() {
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
                let mut source_metadata = None;
                if let Some(artifact) = artifact.as_mut() {
                    let sources = self.sources.as_ref().ok_or_else(|| {
                        InspectionError::new(
                            "source_provider_unsupported",
                            "source validation is not configured in this host",
                        )
                    })?;
                    let authority = source_authority_for_checkout(
                        &self.configuration,
                        Path::new(&repository.checkout_path),
                        &artifact.provider_id,
                    )
                    .await?;
                    let validated = sources
                        .validate_artifact(SourceFetchRequest {
                            provider_id: artifact.provider_id.clone(),
                            artifact_url: artifact.original_url.clone(),
                            authority: authority.clone(),
                        })
                        .await?;
                    artifact.canonical_url =
                        reviewed_artifact_url(artifact, &validated)?.to_owned();
                    if artifact.kind == "review" {
                        source_metadata = Some(
                            sources
                                .metadata(SourceFetchRequest {
                                    provider_id: artifact.provider_id.clone(),
                                    artifact_url: artifact.canonical_url.clone(),
                                    authority,
                                })
                                .await?,
                        );
                    }
                }
                let source_base = if artifact
                    .as_ref()
                    .is_some_and(|artifact| artifact.kind == "review")
                {
                    source_metadata
                        .as_ref()
                        .and_then(|metadata| metadata.source_commit.clone())
                } else {
                    None
                };
                if let (Some(artifact), Some(metadata)) =
                    (artifact.as_ref(), source_metadata.as_ref())
                {
                    validate_review_metadata(&self.configuration, artifact, metadata)?;
                    if artifact.kind == "review"
                        && let (Some(source_branch), Some(commit)) = (
                            metadata.source_branch.as_deref(),
                            metadata.source_commit.as_deref(),
                        )
                    {
                        verify_source_branch(&catalog, &repository, source_branch, commit).await?;
                    }
                }
                let branch = match branch.as_deref() {
                    Some(branch) => branch.to_owned(),
                    None if artifact
                        .as_ref()
                        .is_some_and(|artifact| artifact.kind == "review") =>
                    {
                        let metadata = source_metadata.as_ref().ok_or_else(|| {
                            InspectionError::new(
                                "source_provider_contract",
                                "review setup metadata is unavailable",
                            )
                        })?;
                        let source_branch = metadata.source_branch.as_deref().ok_or_else(|| {
                            InspectionError::new(
                                "source_branch_unavailable",
                                "GitLab merge request has no usable source branch",
                            )
                        })?;
                        source_branch.to_owned()
                    }
                    None => expand_template(
                        &self.configuration.branch_template,
                        &repository,
                        task_name.as_deref(),
                        artifact.as_ref(),
                    )?,
                };
                catalog.validate_branch(&repository, &branch).await?;
                let path = match checkout_path.as_deref() {
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
                                task_name
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
                let base = match base_ref.as_deref() {
                    Some(base) => {
                        let resolved = catalog.resolve_base(&repository, base).await?;
                        if source_base
                            .as_deref()
                            .is_some_and(|source_commit| source_commit != resolved)
                        {
                            return Err(InspectionError::new(
                                "source_base_mismatch",
                                "explicit base does not match the reviewed merge-request head commit",
                            ));
                        }
                        Some(resolved)
                    }
                    None => source_base,
                };
                let label = label.clone().unwrap_or_else(|| {
                    task_name.clone().unwrap_or_else(|| repository.name.clone())
                });
                validate_text(&label, "label", 256)?;
                (
                    Some(repository),
                    WorkspaceSetupMode::Create,
                    WorkspaceCheckoutOwnership::OwnedWorktree,
                    Some(branch),
                    base,
                    path,
                    operation_id.clone(),
                    true,
                    label,
                    *focus,
                    artifact,
                    vec!["Create configured Herdr worktree".to_owned()],
                )
            }
            WorkspaceSetupRequest::Open {
                path,
                label,
                task_name,
                focus,
            } => {
                let checkout_path = accessible_directory(path)?;
                let repository = catalog
                    .discover_checkout(Path::new(&checkout_path))
                    .await
                    .ok();
                let label = label.clone().unwrap_or_else(|| {
                    task_name
                        .clone()
                        .unwrap_or_else(|| directory_label(&checkout_path))
                });
                validate_text(&label, "label", 256)?;
                (
                    repository,
                    WorkspaceSetupMode::Open,
                    WorkspaceCheckoutOwnership::BorrowedDirectory,
                    None,
                    None,
                    checkout_path,
                    operation_id.clone(),
                    true,
                    label,
                    *focus,
                    None,
                    vec![
                        "Create Herdr Space for borrowed directory; Cockpit will never delete it"
                            .to_owned(),
                    ],
                )
            }
        };
        let companion_path = bounded_path(
            &companion_id,
            Path::new(&self.configuration.companion_root),
            "companion_path",
        )?;
        let endpoint_identity = if mode == WorkspaceSetupMode::Create {
            let repository = repository.as_ref().expect("Create plan has repository");
            let inventory = self
                .adapter
                .project_inventory(session, &repository.checkout_path)
                .await?;
            verify_inventory(&inventory, repository)?;
            inventory.endpoint_identity
        } else {
            self.adapter.project_endpoint_identity(session).await?
        };
        let mut effects = effects;
        effects.push(format!("Associate Cockpit companion at {}", companion_path));
        effects.push(
            "Create a new context-aware terminal with allowlisted COCKPIT_* environment".to_owned(),
        );
        if mode == WorkspaceSetupMode::Create {
            effects.push(
                "Run configured repository actions automatically (trust_repository remains unset)"
                    .to_owned(),
            );
        }
        if artifact.is_some() {
            effects.push("Hydrate the selected source into the companion Context".to_owned());
            effects.push("Record the selected artifact in the companion manifest".to_owned());
        }
        let plan = WorkspaceSetupPlan {
            operation_id,
            generation: 1,
            endpoint_identity,
            session_id: session.to_owned(),
            repository,
            mode,
            ownership,
            branch,
            base,
            checkout_path,
            companion_path,
            companion_id,
            companion_created_by_operation,
            label,
            focus,
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
        // Re-check the reviewed authority and effects before taking the
        // execution lease. A changed repository, endpoint, path, or source
        // authority must force a fresh plan rather than silently executing a
        // different operation.
        if let Err(error) = self.preflight_plan(session, &operation.plan).await {
            if error.code.starts_with("source_") {
                return Err(error);
            }
            return Err(InspectionError::new(
                "stale_plan",
                format!("reviewed setup is no longer valid: {}", error.message),
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
                TeardownReceiptState::OrphanedCompanion => {
                    WorkspaceTeardownRecoveryState::OrphanedCompanion
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

    /// Execute one explicitly reviewed teardown action. A worktree removal is
    /// always non-force and removes the companion only after Herdr confirms the
    /// exact requested checkout was removed.
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
                    message: "workspace closed; the checkout and companion were retained"
                        .to_owned(),
                })
            }
            WorkspaceTeardownCommand::ForgetAssociation { operation_id } => {
                self.store.update(&operation_id, None, |operation| {
                    if operation.session_id != session {
                        return Err(InspectionError::new(
                            "stale_identity",
                            "operation belongs to another session",
                        ));
                    }
                    operation.companion_id = None;
                    operation.owned_resources.retain(|resource| {
                        resource.kind != "companion" || resource.created_by_operation
                    });
                    Ok(())
                })?;
                Ok(WorkspaceTeardownResult {
                    operation_id,
                    workspace_id: request.workspace_id.clone(),
                    action: request.action,
                    outcome: WorkspaceTeardownOutcome::Retained,
                    message: "borrowed checkout association forgotten; no files were removed"
                        .to_owned(),
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
                            message: "Herdr did not confirm removal; reconcile before retrying and the companion remains retained".to_owned(),
                        });
                    }
                }
                match self
                    .store
                    .remove_owned_companion(&self.configuration.companion_root, &receipt.companion)
                {
                    Ok(()) => {
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
                    Err(error) if error.code == "companion_missing" => {
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
                    Err(error) => {
                        receipt.state = TeardownReceiptState::OrphanedCompanion;
                        receipt.updated_at = timestamp();
                        self.store.write_teardown_receipt(&receipt)?;
                        Ok(WorkspaceTeardownResult {
                            operation_id: request.operation_id.clone(),
                            workspace_id,
                            action: request.action,
                            outcome: WorkspaceTeardownOutcome::OrphanedCompanion,
                            message: format!(
                                "Herdr removed the worktree; the reviewed companion was retained: {}",
                                error.message
                            ),
                        })
                    }
                }
            }
            WorkspaceTeardownCommand::ReconcileRemoveOutcome { operation_id } => {
                self.reconcile_teardown_removal(session, &operation_id, request, &evidence)
                    .await
            }
            WorkspaceTeardownCommand::RemoveOrphanedCompanion { operation_id } => {
                self.recover_orphaned_companion(&operation_id, request, &evidence)
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
        let companion = evidence.companion.clone().ok_or_else(|| {
            InspectionError::new(
                "teardown_not_allowed",
                "the reviewed companion is no longer available",
            )
        })?;
        Ok(TeardownReceipt {
            operation_id: evidence.operation.operation_id.clone(),
            workspace_id: evidence.operation.workspace_id.clone().ok_or_else(|| {
                InspectionError::new("stale_identity", "operation no longer has a workspace")
            })?,
            endpoint_identity: evidence.endpoint_identity.clone(),
            checkout_path: evidence.operation.plan.checkout_path.clone(),
            companion,
            state: TeardownReceiptState::Pending,
            updated_at: timestamp(),
        })
    }

    fn recovery_receipt(
        &self,
        evidence: &FreshTeardownEvidence,
        expected: TeardownReceiptState,
        require_companion: bool,
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
            || (require_companion && evidence.companion.as_ref() != Some(&receipt.companion))
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
            .recovery_receipt(evidence, TeardownReceiptState::OutcomeUnknown, false)
            .or_else(|error| {
                if error.code == "stale_identity" {
                    self.recovery_receipt(evidence, TeardownReceiptState::Pending, false)
                } else {
                    Err(error)
                }
            })?;
        match evidence.worktrees.as_slice() {
            [] => {
                let companion_missing = evidence.companion.is_none();
                receipt.state = if companion_missing {
                    TeardownReceiptState::Completed
                } else {
                    TeardownReceiptState::OrphanedCompanion
                };
                receipt.updated_at = timestamp();
                self.store.write_teardown_receipt(&receipt)?;
                Ok(WorkspaceTeardownResult {
                    operation_id: operation_id.to_owned(),
                    workspace_id: request.workspace_id.clone(),
                    action: request.action,
                    outcome: if companion_missing {
                        WorkspaceTeardownOutcome::Completed
                    } else {
                        WorkspaceTeardownOutcome::OrphanedCompanion
                    },
                    message: if companion_missing {
                        "fresh Herdr inventory confirms the worktree and reviewed companion are gone".to_owned()
                    } else {
                        "fresh Herdr inventory confirms the worktree is gone; explicitly review companion cleanup".to_owned()
                    },
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

    fn recover_orphaned_companion(
        &self,
        operation_id: &str,
        request: &WorkspaceTeardownExecuteRequest,
        evidence: &FreshTeardownEvidence,
    ) -> Result<WorkspaceTeardownResult, InspectionError> {
        let mut receipt =
            self.recovery_receipt(evidence, TeardownReceiptState::OrphanedCompanion, true)?;
        match self
            .store
            .remove_owned_companion(&self.configuration.companion_root, &receipt.companion)
        {
            Ok(()) => {
                receipt.state = TeardownReceiptState::Completed;
                receipt.updated_at = timestamp();
                self.store.write_teardown_receipt(&receipt)?;
                Ok(WorkspaceTeardownResult {
                    operation_id: operation_id.to_owned(),
                    workspace_id: request.workspace_id.clone(),
                    action: request.action,
                    outcome: WorkspaceTeardownOutcome::Completed,
                    message: "the reviewed orphaned companion was removed".to_owned(),
                })
            }
            Err(error) if error.code == "companion_missing" => {
                receipt.state = TeardownReceiptState::Completed;
                receipt.updated_at = timestamp();
                self.store.write_teardown_receipt(&receipt)?;
                Ok(WorkspaceTeardownResult {
                    operation_id: operation_id.to_owned(),
                    workspace_id: request.workspace_id.clone(),
                    action: request.action,
                    outcome: WorkspaceTeardownOutcome::Completed,
                    message: "the reviewed orphaned companion was removed".to_owned(),
                })
            }
            Err(error) => {
                receipt.updated_at = timestamp();
                self.store.write_teardown_receipt(&receipt)?;
                Ok(WorkspaceTeardownResult {
                    operation_id: operation_id.to_owned(),
                    workspace_id: request.workspace_id.clone(),
                    action: request.action,
                    outcome: WorkspaceTeardownOutcome::OrphanedCompanion,
                    message: format!("the reviewed companion was retained: {}", error.message),
                })
            }
        }
    }

    async fn fresh_teardown_evidence(
        &self,
        session: &str,
        operation: WorkspaceOperation,
    ) -> Result<FreshTeardownEvidence, InspectionError> {
        if operation.plan.mode == WorkspaceSetupMode::Open || operation.plan.repository.is_none() {
            let endpoint_identity = self.adapter.project_endpoint_identity(session).await?;
            let companion = match self.store.read_companion(
                &self.configuration.companion_root,
                &operation.plan.companion_id,
            ) {
                Ok(manifest) => Some(manifest),
                Err(error) if error.code == "companion_missing" => None,
                Err(error) => return Err(error),
            };
            return Ok(FreshTeardownEvidence {
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
                companion,
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
        let companion = match self.store.read_companion(
            &self.configuration.companion_root,
            &operation.plan.companion_id,
        ) {
            Ok(manifest) => Some(manifest),
            Err(error) if error.code == "companion_missing" => None,
            Err(error) => return Err(error),
        };
        let receipt = self.store.read_teardown_receipt(&operation.operation_id)?;
        Ok(FreshTeardownEvidence {
            endpoint_identity: inventory.endpoint_identity,
            repository_key: inventory.repository_key,
            repository_root: inventory.repository_root,
            operation,
            companion,
            worktrees,
            receipt,
        })
    }
    /// Return companions whose durable metadata is still authorized by the
    /// current Herdr endpoint and, for owned worktrees, a fresh inventory.
    /// Paths supplied by callers are never used as authorization.
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
                || operation
                    .plan
                    .repository
                    .as_ref()
                    .map(|repository| repository.common_dir.as_str())
                    .unwrap_or("")
                    != manifest.repository_key
                || operation
                    .plan
                    .repository
                    .as_ref()
                    .map(|repository| repository.root.as_str())
                    .unwrap_or("")
                    != manifest.repository_root
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
            if operation.plan.mode == WorkspaceSetupMode::Create {
                let repository = operation
                    .plan
                    .repository
                    .as_ref()
                    .expect("Create operation has repository");
                let inventory = self
                    .adapter
                    .project_inventory(session_id, &repository.checkout_path)
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
            } else if self.adapter.project_endpoint_identity(session_id).await? != endpoint_identity
            {
                return Err(InspectionError::new(
                    "stale_identity",
                    "companion endpoint is no longer current",
                ));
            }
            let companion_path = Path::new(&self.configuration.companion_root).join(&companion_id);
            let root_id =
                stable_companion_root_id(&companion_path.to_string_lossy(), &companion_id)?;
            roots.push(ContextRoot {
                root_id,
                kind: ContextRootKind::Companion,
                label: format!("{} context", operation.plan.label),
                path: Path::new(&self.configuration.companion_root)
                    .join(&companion_id)
                    .to_string_lossy()
                    .into_owned(),
                repository_id: operation
                    .plan
                    .repository
                    .as_ref()
                    .map(|repository| repository.repository_id.clone())
                    .unwrap_or_default(),
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
                verify_inventory(&inventory, &repository)?;
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
                    verify_inventory(&reopened, &repository)?;
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
                    .project_inventory(session, &repository.checkout_path)
                    .await?;
                verify_inventory(&inventory, &repository)?;
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
    async fn preflight_plan(
        &self,
        session: &str,
        plan: &WorkspaceSetupPlan,
    ) -> Result<Option<ProjectInventory>, InspectionError> {
        validate_project_root(Path::new(&self.configuration.worktree_root))?;
        self.store
            .preflight_companion_publication(&self.configuration.companion_root)?;
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
        if let Some(artifact) = plan.artifact.as_ref() {
            let repository = plan.repository.as_ref().ok_or_else(|| {
                InspectionError::new(
                    "source_repository_missing",
                    "source setup requires a reviewed repository",
                )
            })?;
            let sources = self.sources.as_ref().ok_or_else(|| {
                InspectionError::new(
                    "source_provider_unsupported",
                    "source validation is not configured in this host",
                )
            })?;
            let authority = source_authority_for_checkout(
                &self.configuration,
                Path::new(&repository.checkout_path),
                &artifact.provider_id,
            )
            .await?;
            let validated = sources
                .validate_artifact(SourceFetchRequest {
                    provider_id: artifact.provider_id.clone(),
                    artifact_url: artifact.original_url.clone(),
                    authority: authority.clone(),
                })
                .await?;
            if reviewed_artifact_url(artifact, &validated)? != artifact.canonical_url {
                return Err(InspectionError::new(
                    "stale_plan",
                    "source provenance changed; review a fresh setup plan before starting",
                ));
            }
            if artifact.kind == "review" {
                let metadata = sources
                    .metadata(SourceFetchRequest {
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
        }
        Ok(before)
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
        let repository = plan.repository.as_ref();
        let before = self.preflight_plan(session, &plan).await?;
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
        if plan.mode == WorkspaceSetupMode::Create {
            let before = before.as_ref().expect("Create plan has inventory");
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
        let companion_id = plan.companion_id.as_str();
        let companion = match self
            .store
            .read_companion(&self.configuration.companion_root, companion_id)
        {
            Ok(existing) => {
                verify_manifest_identity(&existing, &plan, companion_id)?;
                let existing = if existing.herdr_session_identity == plan.endpoint_identity
                    && existing.herdr_workspace_id == result.workspace_id
                {
                    existing
                } else if plan.mode == WorkspaceSetupMode::Open
                    && !plan.companion_created_by_operation
                {
                    let reattached = CompanionManifest {
                        herdr_session_identity: plan.endpoint_identity.clone(),
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
                    &plan.endpoint_identity,
                    companion_id,
                )?;
                self.store
                    .sync_companion_publication(&self.configuration.companion_root, &existing)?;
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
                    herdr_session_identity: plan.endpoint_identity.clone(),
                    herdr_workspace_id: result.workspace_id.clone(),
                    repository_key: repository
                        .map(|repository| repository.common_dir.clone())
                        .unwrap_or_default(),
                    repository_root: repository
                        .map(|repository| repository.root.clone())
                        .unwrap_or_default(),
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
        let mut context_error = None;
        if !step_at_least(operation.step, WorkspaceOperationStep::ContextReady) {
            self.store.update(id, None, |operation| {
                operation.step = WorkspaceOperationStep::ContextPreparing;
                operation.state = WorkspaceOperationState::Running;
                operation.resume_allowed = false;
                operation.error = None;
                Ok(())
            })?;
            let materialization = async {
                let Some(artifact) = plan.artifact.as_ref() else {
                    return Ok::<(), InspectionError>(());
                };
                let sources = self.sources.as_ref().ok_or_else(|| {
                    InspectionError::new(
                        "source_provider_unsupported",
                        "source materialization is not configured in this host",
                    )
                })?;
                let authority = source_authority_for_checkout(
                    &self.configuration,
                    Path::new(
                        &repository
                            .expect("artifact plans require a repository")
                            .checkout_path,
                    ),
                    &artifact.provider_id,
                )
                .await?;
                let (_, companion_root) =
                    prepare_root(Path::new(&plan.companion_path), "companion")?;
                let response = sources
                    .fetch_to_companion(
                        SourceFetchRequest {
                            provider_id: artifact.provider_id.clone(),
                            artifact_url: artifact.original_url.clone(),
                            authority,
                        },
                        Some((&companion_root, companion_id)),
                    )
                    .await?;
                if response.entries.iter().any(|entry| {
                    matches!(
                        entry.status,
                        cockpit_protocol::sources::SourceMaterializationStatus::Conflict
                            | cockpit_protocol::sources::SourceMaterializationStatus::Failed
                    )
                }) {
                    return Err(InspectionError::new(
                        "source_sync_conflict",
                        "source materialization preserved companion edits and needs retry",
                    ));
                }
                if response.entries.is_empty() {
                    return Err(InspectionError::new(
                        "source_provider_contract",
                        "source provider returned no primary artifact",
                    ));
                }
                Ok::<(), InspectionError>(())
            }
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
        ContextPreparing => 9,
        ContextReady => 10,
        Completed => 11,
    };
    rank(current) >= rank(wanted)
}

fn reviewed_artifact_url<'a>(
    artifact: &'a ProjectArtifact,
    response: &'a cockpit_protocol::sources::SourceImportResponse,
) -> Result<&'a str, InspectionError> {
    let primary = response
        .entries
        .iter()
        .find(|entry| {
            entry.provider_id == artifact.provider_id
                && entry.resource_type == artifact.kind
                && entry.canonical_id == artifact.canonical_id
        })
        .ok_or_else(|| {
            InspectionError::new(
                "source_identity_mismatch",
                "source provider did not return the reviewed artifact identity",
            )
        })?;
    Ok(primary
        .source_url
        .as_deref()
        .unwrap_or(&artifact.canonical_url))
}
async fn verify_source_branch(
    catalog: &RepositoryCatalog,
    repository: &RepositoryCandidate,
    branch: &str,
    expected_commit: &str,
) -> Result<(), InspectionError> {
    catalog.validate_branch(repository, branch).await?;
    if let Some(commit) = catalog.local_branch_commit(repository, branch).await? {
        if commit != expected_commit {
            return Err(InspectionError::new(
                "source_ref_mismatch",
                "the existing local source branch is not at the reviewed merge-request commit",
            ));
        }
        return Ok(());
    }
    if let Some(commit) = catalog.origin_tracking_commit(repository, branch).await? {
        if commit == expected_commit {
            return Ok(());
        }
        return Err(InspectionError::new(
            "source_ref_mismatch",
            "the origin tracking source ref is not at the reviewed merge-request commit",
        ));
    }
    Err(InspectionError::new(
        "source_ref_unavailable",
        "the reviewed merge-request source ref is unavailable locally; fetch it before setup",
    ))
}
fn validate_review_metadata(
    configuration: &ProjectConfiguration,
    artifact: &ProjectArtifact,
    metadata: &SourceMetadata,
) -> Result<(), InspectionError> {
    if artifact.kind != "review" {
        return Ok(());
    }
    if metadata.source_branch.is_none() {
        return Err(InspectionError::new(
            "source_branch_unavailable",
            "merge request metadata has no usable source branch",
        ));
    }
    let is_gitlab = configuration
        .providers
        .iter()
        .find(|provider| provider.id == artifact.provider_id)
        .is_some_and(|provider| {
            Path::new(&provider.executable)
                .file_name()
                .is_some_and(|name| name == "glab")
        });
    if is_gitlab && metadata.source_commit.is_none() {
        return Err(InspectionError::new(
            "source_commit_unavailable",
            "merge request metadata has no complete source commit",
        ));
    }
    Ok(())
}

async fn validate_review_plan_source(
    catalog: &RepositoryCatalog,
    repository: &RepositoryCandidate,
    plan: &WorkspaceSetupPlan,
    metadata: &SourceMetadata,
) -> Result<(), InspectionError> {
    let source_branch = metadata.source_branch.as_deref().ok_or_else(|| {
        InspectionError::new(
            "source_branch_unavailable",
            "merge request metadata has no usable source branch",
        )
    })?;
    if let Some(commit) = metadata.source_commit.as_deref() {
        if plan.base.as_deref() != Some(commit) {
            return Err(InspectionError::new(
                "stale_plan",
                "merge request head commit changed; review a fresh setup plan",
            ));
        }
        verify_source_branch(catalog, repository, source_branch, commit).await?;
    }
    Ok(())
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
        || manifest.repository_key
            != plan
                .repository
                .as_ref()
                .map(|repository| repository.common_dir.as_str())
                .unwrap_or("")
        || manifest.repository_root
            != plan
                .repository
                .as_ref()
                .map(|repository| repository.root.as_str())
                .unwrap_or("")
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
    match request {
        WorkspaceSetupRequest::Create {
            repository_id,
            branch,
            base_ref,
            checkout_path,
            label,
            task_name,
            artifact_url,
            ..
        } => {
            validate_text(repository_id, "repository_id", 256)?;
            for (value, field, max) in [
                (branch.as_deref(), "branch", 256),
                (base_ref.as_deref(), "base_ref", 256),
                (checkout_path.as_deref(), "checkout_path", 4096),
                (label.as_deref(), "label", 256),
                (task_name.as_deref(), "task_name", 256),
                (artifact_url.as_deref(), "artifact_url", 2048),
            ] {
                if let Some(value) = value {
                    validate_text(value, field, max)?;
                }
            }
        }
        WorkspaceSetupRequest::Open {
            path,
            label,
            task_name,
            ..
        } => {
            validate_text(path, "path", 4096)?;
            for value in [label.as_deref(), task_name.as_deref()]
                .into_iter()
                .flatten()
            {
                validate_text(value, "label", 256)?;
            }
        }
    }
    Ok(())
}

fn accessible_directory(value: &str) -> Result<String, InspectionError> {
    let path = Path::new(value);
    if !path.is_absolute() {
        return Err(InspectionError::new(
            "invalid_path",
            "directory path must be absolute",
        ));
    }
    let metadata = std::fs::metadata(path).map_err(|error| {
        InspectionError::new(
            "directory_unavailable",
            format!("cannot access directory: {error}"),
        )
    })?;
    if !metadata.is_dir() {
        return Err(InspectionError::new(
            "invalid_path",
            "directory path is not a directory",
        ));
    }
    std::fs::read_dir(path).map_err(|error| {
        InspectionError::new(
            "directory_unavailable",
            format!("cannot read directory: {error}"),
        )
    })?;
    Ok(value.to_owned())
}

fn directory_label(path: &str) -> String {
    Path::new(path)
        .file_name()
        .and_then(|name| name.to_str())
        .filter(|name| !name.is_empty())
        .unwrap_or(path)
        .to_owned()
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
    task_name: Option<&str>,
    artifact: Option<&ProjectArtifact>,
) -> Result<String, InspectionError> {
    let task = slug(
        task_name
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
    use std::collections::BTreeMap;
    use std::sync::{
        Arc, Mutex as StdMutex,
        atomic::{AtomicUsize, Ordering},
    };

    use cockpit_protocol::{
        project_teardown::{
            WorkspaceTeardownAction, WorkspaceTeardownExecuteRequest,
            WorkspaceTeardownPreviewRequest,
        },
        projects::{ProjectConfiguration, ProjectLimits, WorkspaceSetupRequest},
        v1::{
            FocusRequest, FocusResponse, HerdrCompatibility, ResourceMutationRequest,
            ResourceMutationResponse, SessionListResponse, SessionSnapshotResponse,
            TerminalOpenRequest,
        },
    };

    use crate::{
        HerdrAdapter, ProjectHerdrAdapter, SessionSubscription, TerminalSession,
        project_adapter::{
            ProjectInventory, ProjectTerminalRequest, ProjectTerminalResult,
            ProjectWorktreeRemoveRequest, ProjectWorktreeRequest, ProjectWorktreeResult,
        },
    };

    #[derive(Default)]
    struct NestedDirectoryAdapter {
        inventory_calls: AtomicUsize,
        worktree_requests: StdMutex<Vec<ProjectWorktreeRequest>>,
        terminal_requests: StdMutex<Vec<ProjectTerminalRequest>>,
        closed_workspaces: StdMutex<Vec<String>>,
    }

    fn unused<T>() -> Result<T, InspectionError> {
        Err(InspectionError::new(
            "test_adapter_unused",
            "test adapter method is not expected",
        ))
    }

    #[async_trait::async_trait]
    impl HerdrAdapter for NestedDirectoryAdapter {
        async fn inspect(&self) -> Result<HerdrCompatibility, InspectionError> {
            unused()
        }

        async fn inspect_session(&self, _: &str) -> Result<HerdrCompatibility, InspectionError> {
            unused()
        }

        async fn sessions(&self) -> Result<SessionListResponse, InspectionError> {
            unused()
        }

        async fn session_snapshot(
            &self,
            _: &str,
        ) -> Result<SessionSnapshotResponse, InspectionError> {
            unused()
        }

        async fn focus(&self, _: &str, _: &FocusRequest) -> Result<FocusResponse, InspectionError> {
            unused()
        }

        async fn mutate(
            &self,
            _: &str,
            _: &ResourceMutationRequest,
        ) -> Result<ResourceMutationResponse, InspectionError> {
            unused()
        }

        async fn subscribe_session(
            &self,
            _: &str,
            _: &SessionSnapshotResponse,
        ) -> Result<SessionSubscription, InspectionError> {
            unused()
        }

        async fn open_terminal(
            &self,
            _: &TerminalOpenRequest,
        ) -> Result<TerminalSession, InspectionError> {
            unused()
        }
    }

    #[async_trait::async_trait]
    impl ProjectHerdrAdapter for NestedDirectoryAdapter {
        async fn project_endpoint_identity(&self, _: &str) -> Result<String, InspectionError> {
            Ok("endpoint".to_owned())
        }

        async fn project_inventory(
            &self,
            _: &str,
            _: &str,
        ) -> Result<ProjectInventory, InspectionError> {
            self.inventory_calls.fetch_add(1, Ordering::Relaxed);
            unused()
        }

        async fn project_worktree(
            &self,
            _: &str,
            request: &ProjectWorktreeRequest,
        ) -> Result<ProjectWorktreeResult, InspectionError> {
            self.worktree_requests
                .lock()
                .expect("worktree requests")
                .push(request.clone());
            Ok(ProjectWorktreeResult {
                workspace_id: "workspace".to_owned(),
                tab_id: Some("workspace:root".to_owned()),
                pane_id: Some("workspace:pane".to_owned()),
                checkout_path: request.checkout_path.clone(),
                branch: None,
                already_open: false,
            })
        }

        async fn project_terminal(
            &self,
            _: &str,
            request: &ProjectTerminalRequest,
        ) -> Result<ProjectTerminalResult, InspectionError> {
            self.terminal_requests
                .lock()
                .expect("terminal requests")
                .push(request.clone());
            Ok(ProjectTerminalResult {
                workspace_id: request.workspace_id.clone(),
                tab_id: "workspace:context".to_owned(),
                pane_id: "workspace:context-pane".to_owned(),
            })
        }

        async fn project_worktree_dirty(
            &self,
            _: &str,
            _: u32,
            _: u32,
        ) -> Result<bool, InspectionError> {
            unused()
        }

        async fn project_close_workspace(
            &self,
            _: &str,
            _: &str,
            workspace_id: &str,
        ) -> Result<(), InspectionError> {
            self.closed_workspaces
                .lock()
                .expect("closed workspaces")
                .push(workspace_id.to_owned());
            Ok(())
        }

        async fn project_remove_worktree(
            &self,
            _: &str,
            _: &ProjectWorktreeRemoveRequest,
        ) -> Result<(), InspectionError> {
            unused()
        }
    }

    fn configuration(root: &Path) -> ProjectConfiguration {
        ProjectConfiguration {
            version: 1,
            repository_roots: vec![root.to_string_lossy().into_owned()],
            worktree_root: root.join("worktrees").to_string_lossy().into_owned(),
            companion_root: root.join("companions").to_string_lossy().into_owned(),
            state_root: root.join("state").to_string_lossy().into_owned(),
            branch_template: "{repo}/{task_id}".to_owned(),
            checkout_template: "{repo}-{task_id}".to_owned(),
            providers: Vec::new(),
            limits: ProjectLimits {
                catalog_depth: 4,
                catalog_entries: 256,
                git_timeout_ms: 2_000,
                git_output_bytes: 2 * 1024 * 1024,
                operation_timeout_ms: 2_000,
                context_preview_bytes: 1024 * 1024,
                context_preview_lines: 2_000,
                context_directory_entries: 64,
                context_tree_depth: 8,
            },
            origins: BTreeMap::new(),
        }
    }

    fn git(root: &Path, args: &[&str]) {
        let output = std::process::Command::new("git")
            .current_dir(root)
            .args(args)
            .output()
            .expect("git starts");
        assert!(
            output.status.success(),
            "git {args:?}: {}",
            String::from_utf8_lossy(&output.stderr)
        );
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
        std::fs::create_dir_all(&configuration.companion_root).expect("companion root");
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
            .execute_inner("session", &plan.operation_id)
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
