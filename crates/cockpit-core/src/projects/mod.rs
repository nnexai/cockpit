mod defaults;
mod execute;
mod plan;
mod reconcile;
mod teardown;

use std::collections::HashSet;
use std::path::Path;
use std::sync::{Arc, atomic::AtomicBool};

use cockpit_protocol::projects::{
    ProjectConfiguration, RepositoryCandidate, RepositoryListResponse, WorkspaceOperation,
    WorkspaceOperationRequest, WorkspaceOperationState, WorkspaceOperationStep, WorkspaceSetupPlan,
    WorkspaceSetupRequest,
};
use cockpit_protocol::v1::ErrorResponse;
use tokio::sync::Mutex;

use crate::project_store::{ProjectStore, prepare_project_root, validate_project_root};
use crate::repositories::RepositoryCatalog;
use crate::sources::SourceService;
use crate::{InspectionError, ProjectHerdrAdapter};

pub struct ProjectService {
    configuration: ProjectConfiguration,
    adapter: Arc<dyn ProjectHerdrAdapter>,
    store: ProjectStore,
    repository_cache: Arc<crate::repository_cache::RepositoryDiscoveryCache>,
    sources: Option<Arc<SourceService>>,
    shutting_down: AtomicBool,
    cancelled: Mutex<HashSet<String>>,
    workers: Mutex<HashSet<String>>,
    last_plan_prune: std::sync::Mutex<Option<std::time::Instant>>,
}

/// Configured repository identity and the immutable HEAD of the actual source Space.
#[derive(Debug)]
pub struct SpaceRepository {
    pub repository: RepositoryCandidate,
    pub source_head: String,
}

impl SpaceRepository {
    pub(crate) fn worktree_request(
        &self,
        branch: Option<String>,
        base_ref: Option<String>,
        task_name: String,
    ) -> WorkspaceSetupRequest {
        WorkspaceSetupRequest::Create {
            repository_id: self.repository.repository_id.clone(),
            branch,
            base_ref: Some(base_ref.unwrap_or_else(|| self.source_head.clone())),
            checkout_path: None,
            label: None,
            task_name: Some(task_name),
            artifact_url: None,
            linked_artifact_urls: vec![],
            focus: false,
        }
    }
}

/// A plan the user did not start within this time is stale: starting it
/// asks for a fresh plan, and the unstarted record is deleted.
const PLAN_TTL_MS: u128 = 60 * 60 * 1000;
const PLAN_PRUNE_INTERVAL: std::time::Duration = std::time::Duration::from_secs(10 * 60);

fn plan_expired(operation: &WorkspaceOperation) -> bool {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis();
    operation
        .updated_at
        .parse::<u128>()
        .is_ok_and(|updated| now.saturating_sub(updated) > PLAN_TTL_MS)
}

fn unstarted(operation: &WorkspaceOperation) -> bool {
    operation.state == WorkspaceOperationState::Planned
        && operation.step == WorkspaceOperationStep::Planned
        && operation.sequence == 0
}

impl ProjectService {
    pub fn new(
        configuration: ProjectConfiguration,
        adapter: Arc<dyn ProjectHerdrAdapter>,
    ) -> Result<Self, InspectionError> {
        let store = ProjectStore::new(&configuration.state_root)?;
        prepare_project_root(Path::new(&configuration.worktree_root))?;
        validate_project_root(Path::new(&configuration.worktree_root))?;
        reconcile::recover_startup(&store)?;
        Ok(Self {
            configuration,
            adapter,
            store,
            repository_cache: Arc::new(crate::repository_cache::RepositoryDiscoveryCache::new(
                std::time::Duration::from_secs(30),
                std::time::Duration::from_secs(300),
            )),
            sources: None,
            shutting_down: AtomicBool::new(false),
            cancelled: Mutex::new(HashSet::new()),
            workers: Mutex::new(HashSet::new()),
            last_plan_prune: std::sync::Mutex::new(None),
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
        let generation = self.store.mutation_generation();
        let listed = RepositoryCatalog::new(self.configuration.clone())
            .list()
            .await?;
        if self.store.mutation_generation() == generation {
            self.repository_cache.publish(listed.clone(), generation);
        }
        Ok(listed)
    }

    pub(crate) async fn cached_repositories(
        &self,
    ) -> Result<RepositoryListResponse, InspectionError> {
        self.repository_cache
            .list(
                &RepositoryCatalog::new(self.configuration.clone()),
                self.store.mutation_generation(),
            )
            .await
    }

    pub(crate) async fn cached_discover_checkout(
        &self,
        cwd: &Path,
    ) -> Result<RepositoryCandidate, InspectionError> {
        self.repository_cache
            .discover(
                &RepositoryCatalog::new(self.configuration.clone()),
                cwd,
                self.store.mutation_generation(),
            )
            .await
    }

    pub fn prewarm_repositories(self: &Arc<Self>) {
        let service = Arc::clone(self);
        tokio::spawn(async move {
            let _ = service.cached_repositories().await;
        });
    }

    /// Prove an open Space belongs to one configured primary repository, using
    /// fresh Git provenance and Herdr's authoritative open-worktree inventory.
    pub async fn space_repository(
        &self,
        session: &str,
        workspace_id: &str,
        space_cwd: &str,
    ) -> Result<SpaceRepository, InspectionError> {
        validate_session(session)?;
        let catalog = RepositoryCatalog::new(self.configuration.clone());
        let checkout = catalog.discover_checkout(Path::new(space_cwd)).await?;
        let mut candidates = catalog
            .list()
            .await?
            .repositories
            .into_iter()
            .filter(|candidate| {
                !candidate.is_linked_worktree
                    && candidate.checkout_path == checkout.root
                    && candidate.common_dir == checkout.common_dir
            });
        let repository = candidates.next().ok_or_else(|| {
            InspectionError::new(
                "project_repository_not_configured",
                format!(
                    "Space {workspace_id} checkout {space_cwd} has no configured primary repository"
                ),
            )
        })?;
        if candidates.next().is_some() {
            return Err(InspectionError::new(
                "repository_identity_conflict",
                "Project Space matched multiple configured primary repositories",
            ));
        }
        let inventory = self
            .adapter
            .project_inventory(session, &repository.checkout_path)
            .await?;
        verify_inventory(&inventory, &repository)?;
        let mut matching = inventory
            .worktrees
            .iter()
            .filter(|entry| entry.open_workspace_id.as_deref() == Some(workspace_id));
        let entry = matching.next().ok_or_else(|| {
            InspectionError::new(
                "space_repository_mismatch",
                format!(
                    "Herdr does not identify Space {workspace_id} as an open checkout of {}",
                    repository.repository_id
                ),
            )
        })?;
        if matching.next().is_some()
            || std::fs::canonicalize(&entry.checkout_path).ok().as_deref()
                != Some(Path::new(&checkout.checkout_path))
            || entry.is_linked_worktree != checkout.is_linked_worktree
        {
            return Err(InspectionError::new(
                "space_repository_mismatch",
                format!(
                    "Space {workspace_id} inventory does not uniquely prove its actual checkout {space_cwd}"
                ),
            ));
        }
        let source_head = catalog.resolve_checkout_head(&checkout).await?;
        Ok(SpaceRepository {
            repository,
            source_head,
        })
    }

    /// Return real open Space IDs after freshly verifying repository provenance.
    pub async fn repository_open_spaces(
        &self,
        session: &str,
        repository_id: &str,
    ) -> Result<Vec<String>, InspectionError> {
        validate_session(session)?;
        let repository = RepositoryCatalog::new(self.configuration.clone())
            .resolve(repository_id)
            .await?;
        let inventory = self
            .adapter
            .project_inventory(session, &repository.checkout_path)
            .await?;
        verify_inventory(&inventory, &repository)?;
        let mut spaces: Vec<_> = inventory
            .worktrees
            .into_iter()
            .filter_map(|entry| entry.open_workspace_id)
            .collect();
        spaces.sort();
        spaces.dedup();
        Ok(spaces)
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

fn error_response(code: &str, message: impl Into<String>) -> ErrorResponse {
    ErrorResponse {
        code: code.to_owned(),
        message: message.into(),
    }
}

#[cfg(test)]
mod test_support;
