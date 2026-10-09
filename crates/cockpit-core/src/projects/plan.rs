use std::path::Path;
use std::sync::Arc;

use cockpit_protocol::projects::{
    ProjectArtifact, ProjectConfiguration, ProviderKind, RepositoryCandidate,
    WorkspaceCheckoutOwnership, WorkspaceSetupMode, WorkspaceSetupPlan, WorkspaceSetupRequest,
};
use uuid::Uuid;

use crate::InspectionError;
use crate::repositories::{self, RepositoryCatalog};
use crate::sources::{FetchedAssets, SourceFetchRequest, SourceMetadata, instance_authority};

use super::{
    PLAN_PRUNE_INTERVAL, ProjectService, plan_expired, unstarted, validate_session, validate_text,
    verify_inventory,
};

struct PlannedCheckout {
    repository: Option<RepositoryCandidate>,
    mode: WorkspaceSetupMode,
    ownership: WorkspaceCheckoutOwnership,
    branch: Option<String>,
    base: Option<String>,
    checkout_path: String,
    label: String,
    focus: bool,
    artifact: Option<ProjectArtifact>,
    linked_artifacts: Vec<ProjectArtifact>,
    effects: Vec<String>,
}

struct CreateFields<'a> {
    repository_id: &'a str,
    branch: Option<&'a str>,
    base_ref: Option<&'a str>,
    checkout_path: Option<&'a str>,
    label: Option<&'a str>,
    task_name: Option<&'a str>,
    artifact_url: Option<&'a str>,
    linked_artifact_urls: &'a [String],
    focus: bool,
}

impl ProjectService {
    /// The setup form plans as the user types, so expired unstarted plans
    /// are removed now and then instead of accumulating.
    fn prune_expired_plans(&self) {
        {
            let mut last = self
                .last_plan_prune
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            if last.is_some_and(|last| last.elapsed() < PLAN_PRUNE_INTERVAL) {
                return;
            }
            *last = Some(std::time::Instant::now());
        }
        let Ok(operations) = self.store.list() else {
            return;
        };
        for operation in operations {
            if unstarted(&operation) && plan_expired(&operation) {
                let _ = self.store.discard_unstarted_plan(&operation.operation_id);
            }
        }
    }

    pub async fn plan(
        &self,
        session: &str,
        request: &WorkspaceSetupRequest,
    ) -> Result<WorkspaceSetupPlan, InspectionError> {
        validate_session(session)?;
        validate_setup_request(request)?;
        self.prune_expired_plans();
        let catalog = RepositoryCatalog::new(self.configuration.clone());
        let operation_id = Uuid::new_v4().to_string();
        let mut checkout = match request {
            WorkspaceSetupRequest::Create {
                repository_id,
                branch,
                base_ref,
                checkout_path,
                label,
                task_name,
                artifact_url,
                linked_artifact_urls,
                focus,
            } => {
                self.plan_create(
                    session,
                    &catalog,
                    &operation_id,
                    CreateFields {
                        repository_id,
                        branch: branch.as_deref(),
                        base_ref: base_ref.as_deref(),
                        checkout_path: checkout_path.as_deref(),
                        label: label.as_deref(),
                        task_name: task_name.as_deref(),
                        artifact_url: artifact_url.as_deref(),
                        linked_artifact_urls,
                        focus: *focus,
                    },
                )
                .await?
            }
            WorkspaceSetupRequest::Open {
                path,
                label,
                task_name,
                focus,
            } => {
                plan_open(
                    &catalog,
                    path,
                    label.as_deref(),
                    task_name.as_deref(),
                    *focus,
                )
                .await?
            }
        };
        let endpoint_identity = self.plan_endpoint_identity(session, &checkout).await?;
        plan_effects(&mut checkout);
        let plan = WorkspaceSetupPlan {
            operation_id,
            generation: 1,
            endpoint_identity,
            session_id: session.to_owned(),
            repository: checkout.repository,
            mode: checkout.mode,
            ownership: checkout.ownership,
            branch: checkout.branch,
            base: checkout.base,
            checkout_path: checkout.checkout_path,
            label: checkout.label,
            focus: checkout.focus,
            artifact: checkout.artifact,
            linked_artifacts: checkout.linked_artifacts,
            effects: checkout.effects,
            warnings: Vec::new(),
        };
        self.store.persist_plan(plan.clone())?;
        Ok(plan)
    }

    async fn plan_create(
        &self,
        session: &str,
        catalog: &RepositoryCatalog,
        operation_id: &str,
        fields: CreateFields<'_>,
    ) -> Result<PlannedCheckout, InspectionError> {
        let repository = catalog.resolve(fields.repository_id).await?;
        let inventory = self
            .adapter
            .project_inventory(session, &repository.checkout_path)
            .await?;
        verify_inventory(&inventory, &repository)?;
        let (artifact, source_metadata) =
            self.resolve_primary_artifact(fields.artifact_url).await?;
        let linked_artifacts = if let Some(primary) = artifact.as_ref() {
            self.validate_linked_artifacts(primary, fields.linked_artifact_urls)
                .await?
        } else {
            Vec::new()
        };
        let source_base = self
            .review_source_base(
                catalog,
                &repository,
                artifact.as_ref(),
                source_metadata.as_ref(),
            )
            .await?;
        let branch = self.resolve_branch(
            &repository,
            fields.branch,
            fields.task_name,
            artifact.as_ref(),
            source_metadata.as_ref(),
        )?;
        catalog.validate_branch(&repository, &branch).await?;
        let checkout_path = self.resolve_checkout_path(
            &repository,
            operation_id,
            fields.checkout_path,
            fields.task_name,
            artifact.as_ref(),
        )?;
        let base = resolve_plan_base(catalog, &repository, fields.base_ref, source_base).await?;
        // A Space named after its branch stays recognizable next to
        // other task Spaces of the same repository.
        let label = fields
            .label
            .or(fields.task_name)
            .map(str::to_owned)
            .unwrap_or_else(|| branch.clone());
        validate_text(&label, "label", 256)?;
        Ok(PlannedCheckout {
            repository: Some(repository),
            mode: WorkspaceSetupMode::Create,
            ownership: WorkspaceCheckoutOwnership::OwnedWorktree,
            branch: Some(branch),
            base,
            checkout_path,
            label,
            focus: fields.focus,
            artifact,
            linked_artifacts,
            effects: vec!["Create configured Herdr worktree".to_owned()],
        })
    }

    async fn resolve_primary_artifact(
        &self,
        artifact_url: Option<&str>,
    ) -> Result<(Option<ProjectArtifact>, Option<SourceMetadata>), InspectionError> {
        let mut artifact = match artifact_url {
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
            let authority = instance_authority(
                &self.configuration,
                &artifact.provider_id,
                &artifact.original_url,
            )?;
            let request = SourceFetchRequest {
                provider_id: artifact.provider_id.clone(),
                artifact_url: artifact.original_url.clone(),
                authority: authority.clone(),
            };
            // The small metadata read (usually reused from the
            // defaults lookup) proves the artifact exists; the full
            // read runs in the background and the start preflight
            // waits for it before anything is created.
            let metadata = match sources.metadata_for_setup(request.clone()).await {
                Ok(metadata) => Some(metadata),
                Err(error) if error.code == "source_metadata_unsupported" => None,
                Err(error) => return Err(error),
            };
            match metadata
                .as_ref()
                .and_then(|metadata| metadata.source_url.clone())
            {
                // The provider named the canonical URL the full read
                // will report, so the full read can wait.
                Some(url) => {
                    artifact.canonical_url = url;
                    let prefetch = Arc::clone(sources);
                    tokio::spawn(async move { prefetch.prefetch_for_setup(request).await });
                }
                None => {
                    let validated = sources.validate_artifact_for_setup(request).await?;
                    artifact.canonical_url =
                        reviewed_artifact_url(artifact, &validated)?.to_owned();
                }
            }
            if artifact.kind == "review" {
                source_metadata = metadata;
            }
        }
        Ok((artifact, source_metadata))
    }

    async fn review_source_base(
        &self,
        catalog: &RepositoryCatalog,
        repository: &RepositoryCandidate,
        artifact: Option<&ProjectArtifact>,
        source_metadata: Option<&SourceMetadata>,
    ) -> Result<Option<String>, InspectionError> {
        let source_base = if artifact.is_some_and(|artifact| artifact.kind == "review") {
            source_metadata.and_then(|metadata| metadata.source_commit.clone())
        } else {
            None
        };
        if let (Some(artifact), Some(metadata)) = (artifact, source_metadata) {
            validate_review_metadata(&self.configuration, artifact, metadata)?;
            if artifact.kind == "review"
                && let (Some(source_branch), Some(commit)) = (
                    metadata.source_branch.as_deref(),
                    metadata.source_commit.as_deref(),
                )
            {
                verify_source_branch(catalog, repository, source_branch, commit).await?;
            }
        }
        Ok(source_base)
    }

    fn resolve_branch(
        &self,
        repository: &RepositoryCandidate,
        branch: Option<&str>,
        task_name: Option<&str>,
        artifact: Option<&ProjectArtifact>,
        source_metadata: Option<&SourceMetadata>,
    ) -> Result<String, InspectionError> {
        let branch = match branch {
            Some(branch) => branch.to_owned(),
            None if artifact.is_some_and(|artifact| artifact.kind == "review") => {
                let metadata = source_metadata.ok_or_else(|| {
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
                repository,
                task_name,
                artifact,
            )?,
        };
        Ok(branch)
    }

    fn resolve_checkout_path(
        &self,
        repository: &RepositoryCandidate,
        operation_id: &str,
        checkout_path: Option<&str>,
        task_name: Option<&str>,
        artifact: Option<&ProjectArtifact>,
    ) -> Result<String, InspectionError> {
        let path = match checkout_path {
            Some(path) => bounded_path(
                path,
                Path::new(&self.configuration.worktree_root),
                "checkout_path",
            )?,
            None => {
                let path = expand_path_template(
                    &self.configuration.checkout_template,
                    repository,
                    operation_id,
                    &slug(
                        task_name
                            .or_else(|| artifact.map(|a| a.canonical_id.as_str()))
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
        Ok(path)
    }

    /// Resolve and read each linked work item so a missing or unreadable one
    /// is reported before setup starts, not after the worktree exists.
    async fn validate_linked_artifacts(
        &self,
        primary: &ProjectArtifact,
        urls: &[String],
    ) -> Result<Vec<ProjectArtifact>, InspectionError> {
        let mut linked: Vec<ProjectArtifact> = Vec::new();
        for url in urls {
            let mut artifact = repositories::resolve_artifact(&self.configuration, url)?;
            if (artifact.provider_id == primary.provider_id
                && artifact.canonical_id == primary.canonical_id)
                || linked.iter().any(|known| {
                    known.provider_id == artifact.provider_id
                        && known.canonical_id == artifact.canonical_id
                })
            {
                continue;
            }
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
            let validated = sources
                .validate_artifact_for_setup(SourceFetchRequest {
                    provider_id: artifact.provider_id.clone(),
                    artifact_url: artifact.original_url.clone(),
                    authority,
                })
                .await
                .map_err(|error| {
                    InspectionError::new(
                        error.code,
                        format!("linked {}: {}", artifact.canonical_id, error.message),
                    )
                })?;
            artifact.canonical_url = reviewed_artifact_url(&artifact, &validated)?.to_owned();
            linked.push(artifact);
        }
        Ok(linked)
    }

    async fn plan_endpoint_identity(
        &self,
        session: &str,
        checkout: &PlannedCheckout,
    ) -> Result<String, InspectionError> {
        let endpoint_identity = if checkout.mode == WorkspaceSetupMode::Create {
            let repository = checkout
                .repository
                .as_ref()
                .expect("Create plan has repository");
            let inventory = self
                .adapter
                .project_inventory(session, &repository.checkout_path)
                .await?;
            verify_inventory(&inventory, repository)?;
            inventory.endpoint_identity
        } else {
            self.adapter.project_endpoint_identity(session).await?
        };
        Ok(endpoint_identity)
    }
}

async fn resolve_plan_base(
    catalog: &RepositoryCatalog,
    repository: &RepositoryCandidate,
    base_ref: Option<&str>,
    source_base: Option<String>,
) -> Result<Option<String>, InspectionError> {
    let base = match base_ref {
        Some(base) => {
            let resolved = catalog.resolve_base(repository, base).await?;
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
    Ok(base)
}

async fn plan_open(
    catalog: &RepositoryCatalog,
    path: &str,
    label: Option<&str>,
    task_name: Option<&str>,
    focus: bool,
) -> Result<PlannedCheckout, InspectionError> {
    let checkout_path = accessible_directory(path)?;
    let repository = catalog
        .discover_checkout(Path::new(&checkout_path))
        .await
        .ok();
    let label = label.map(str::to_owned).unwrap_or_else(|| {
        task_name
            .map(str::to_owned)
            .unwrap_or_else(|| directory_label(&checkout_path))
    });
    validate_text(&label, "label", 256)?;
    Ok(PlannedCheckout {
        repository,
        mode: WorkspaceSetupMode::Open,
        ownership: WorkspaceCheckoutOwnership::BorrowedDirectory,
        branch: None,
        base: None,
        checkout_path,
        label,
        focus,
        artifact: None,
        linked_artifacts: Vec::new(),
        effects: vec![
            "Create Herdr Space for borrowed directory; Cockpit will never delete it".to_owned(),
        ],
    })
}

fn plan_effects(checkout: &mut PlannedCheckout) {
    checkout.effects.push(
        "Create a new context-aware terminal with allowlisted COCKPIT_* environment".to_owned(),
    );
    if checkout.mode == WorkspaceSetupMode::Create {
        checkout.effects.push(
            "Run configured repository actions automatically (trust_repository remains unset)"
                .to_owned(),
        );
    }
    if checkout.artifact.is_some() {
        checkout.effects.push(
            "Save the selected source in the Library and select it for this Space".to_owned(),
        );
    }
    for linked in &checkout.linked_artifacts {
        checkout.effects.push(format!(
            "Save and select linked {} for this Space",
            linked.canonical_id
        ));
    }
}

pub(super) fn reviewed_artifact_url<'a>(
    artifact: &'a ProjectArtifact,
    response: &'a FetchedAssets,
) -> Result<&'a str, InspectionError> {
    let primary = response
        .assets
        .iter()
        .find(|entry| {
            entry.source.provider_id == artifact.provider_id
                && entry.source.resource_type == artifact.kind
                && entry.source.canonical_id == artifact.canonical_id
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
pub(super) fn validate_review_metadata(
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
        .is_some_and(|provider| provider.kind == ProviderKind::Gitlab);
    if is_gitlab && metadata.source_commit.is_none() {
        return Err(InspectionError::new(
            "source_commit_unavailable",
            "merge request metadata has no complete source commit",
        ));
    }
    Ok(())
}

pub(super) async fn validate_review_plan_source(
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

const MAX_LINKED_ARTIFACT_URLS: usize = 4;

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
            linked_artifact_urls,
            ..
        } => {
            validate_text(repository_id, "repository_id", 256)?;
            if linked_artifact_urls.len() > MAX_LINKED_ARTIFACT_URLS
                || (!linked_artifact_urls.is_empty() && artifact_url.is_none())
            {
                return Err(InspectionError::new(
                    "invalid_request",
                    "linked work items need a primary artifact and are limited to four",
                ));
            }
            for url in linked_artifact_urls {
                validate_text(url, "linked_artifact_urls", 2048)?;
            }
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

pub(super) fn accessible_directory(value: &str) -> Result<String, InspectionError> {
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

pub(super) fn expand_template(
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

#[cfg(test)]
mod tests {
    use super::super::test_support::*;
    use super::*;
    use crate::project_adapter::ProjectInventory;
    use std::sync::atomic::Ordering;

    #[test]
    fn review_commit_requirement_uses_gitlab_kind_not_executable() {
        let mut configuration = configuration(Path::new("."));
        configuration.providers = vec![cockpit_protocol::projects::ProjectProvider {
            id: "forge".into(),
            kind: ProviderKind::Gitlab,
            base_url: "https://forge.test".into(),
            executable: Some("custom-gitlab-client".into()),
            login: None,
            deployment: None,
        }];
        let artifact = ProjectArtifact {
            provider_id: "forge".into(),
            kind: "review".into(),
            canonical_id: "acme/repo!42".into(),
            original_url: "https://forge.test/acme/repo/-/merge_requests/42".into(),
            canonical_url: "https://forge.test/acme/repo/-/merge_requests/42".into(),
        };
        let mut metadata = SourceMetadata {
            title: "Review".into(),
            source_branch: Some("feature".into()),
            source_url: Some(artifact.canonical_url.clone()),
            source_commit: None,
            description: None,
        };
        assert_eq!(
            validate_review_metadata(&configuration, &artifact, &metadata)
                .unwrap_err()
                .code,
            "source_commit_unavailable"
        );
        metadata.source_commit = Some("a".repeat(40));
        validate_review_metadata(&configuration, &artifact, &metadata).unwrap();
        metadata.source_commit = None;
        configuration.providers[0].kind = ProviderKind::Gitea;
        configuration.providers[0].executable = Some("/usr/local/bin/glab".into());
        validate_review_metadata(&configuration, &artifact, &metadata).unwrap();
    }

    #[tokio::test]
    async fn project_space_repository_requires_fresh_git_and_unique_inventory_proof() {
        use crate::project_adapter::ProjectWorktreeEntry;
        let root = std::env::temp_dir().join(format!("cockpit-project-space-{}", Uuid::new_v4()));
        let configured = root.join("configured");
        let primary = configured.join("repository");
        std::fs::create_dir_all(&primary).unwrap();
        git(&primary, &["init"]);
        git(
            &primary,
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
        let linked = root.join("linked");
        git(
            &primary,
            &[
                "worktree",
                "add",
                "-b",
                "linked-fixture",
                linked.to_str().unwrap(),
            ],
        );
        git(
            &linked,
            &[
                "-c",
                "user.name=Fixture",
                "-c",
                "user.email=fixture@example.test",
                "commit",
                "--allow-empty",
                "-m",
                "linked source HEAD differs from primary",
            ],
        );
        let mut config = configuration(&root);
        config.repository_roots = vec![configured.to_string_lossy().into_owned()];
        let adapter = Arc::new(NestedDirectoryAdapter::default());
        let service = ProjectService::new(config, adapter.clone()).unwrap();
        let repository = service
            .repositories()
            .await
            .unwrap()
            .repositories
            .into_iter()
            .next()
            .unwrap();
        let inventory = ProjectInventory {
            endpoint_identity: "endpoint".into(),
            repository_key: repository.common_dir.clone(),
            repository_root: repository.root.clone(),
            supported_methods: vec![],
            worktrees: vec![
                ProjectWorktreeEntry {
                    checkout_path: primary.to_string_lossy().into_owned(),
                    branch: repository.branch.clone(),
                    open_workspace_id: Some("primary-space".into()),
                    is_primary: true,
                    is_linked_worktree: false,
                    dirty: Some(false),
                },
                ProjectWorktreeEntry {
                    checkout_path: linked.to_string_lossy().into_owned(),
                    branch: Some("linked-fixture".into()),
                    open_workspace_id: Some("linked-space".into()),
                    is_primary: false,
                    is_linked_worktree: true,
                    dirty: Some(false),
                },
            ],
        };
        *adapter.inventory.lock() = Some(inventory.clone());
        let primary_head = RepositoryCatalog::new(service.configuration())
            .resolve_base(&repository, "HEAD")
            .await
            .unwrap();
        for (space, cwd) in [("primary-space", &primary), ("linked-space", &linked)] {
            let source = service
                .space_repository("session", space, cwd.to_str().unwrap())
                .await
                .unwrap();
            assert_eq!(source.repository.repository_id, repository.repository_id);
            let checkout = RepositoryCatalog::new(service.configuration())
                .discover_checkout(cwd)
                .await
                .unwrap();
            let expected_head = RepositoryCatalog::new(service.configuration())
                .resolve_checkout_head(&checkout)
                .await
                .unwrap();
            assert_eq!(source.source_head, expected_head);
            if space == "linked-space" {
                assert_ne!(
                    source.source_head, primary_head,
                    "source linked checkout has its own HEAD"
                );
            }
            for (suffix, explicit, expected) in [
                ("default", None, source.source_head.as_str()),
                (
                    "explicit",
                    Some(primary_head.clone()),
                    primary_head.as_str(),
                ),
            ] {
                let request = source.worktree_request(
                    Some(format!("{space}-{suffix}")),
                    explicit,
                    "source-base-test".into(),
                );
                let plan = service.plan("session", &request).await.unwrap();
                assert_eq!(
                    plan.base.as_deref(),
                    Some(expected),
                    "explicit base wins; otherwise use actual source HEAD"
                );
            }
        }
        assert_eq!(
            service
                .repository_open_spaces("session", &repository.repository_id)
                .await
                .unwrap(),
            vec!["linked-space", "primary-space"]
        );
        assert_eq!(
            service
                .space_repository("session", "missing", primary.to_str().unwrap())
                .await
                .unwrap_err()
                .code,
            "space_repository_mismatch"
        );
        // A valid repository key alone does not prove this Space's checkout.
        assert_eq!(
            service
                .space_repository("session", "primary-space", linked.to_str().unwrap())
                .await
                .unwrap_err()
                .code,
            "space_repository_mismatch"
        );
        let mut duplicate = inventory.clone();
        duplicate.worktrees.push(duplicate.worktrees[0].clone());
        *adapter.inventory.lock() = Some(duplicate);
        assert_eq!(
            service
                .space_repository("session", "primary-space", primary.to_str().unwrap())
                .await
                .unwrap_err()
                .code,
            "space_repository_mismatch"
        );
        let mut contradictory = inventory.clone();
        contradictory.repository_key = "unrelated-common-directory".into();
        *adapter.inventory.lock() = Some(contradictory);
        assert_eq!(
            service
                .space_repository("session", "primary-space", primary.to_str().unwrap())
                .await
                .unwrap_err()
                .code,
            "repository_conflict"
        );
        assert_eq!(
            service
                .repository_open_spaces("session", &repository.repository_id)
                .await
                .unwrap_err()
                .code,
            "repository_conflict"
        );
        *adapter.inventory.lock() = Some(inventory);
        let outside = root.join("outside");
        std::fs::create_dir_all(&outside).unwrap();
        git(&outside, &["init"]);
        assert_eq!(
            service
                .space_repository("session", "outside-space", outside.to_str().unwrap())
                .await
                .unwrap_err()
                .code,
            "project_repository_not_configured"
        );
        assert!(
            adapter.worktree_requests.lock().unwrap().is_empty(),
            "proof must not create a checkout"
        );
        assert!(
            adapter.terminal_requests.lock().unwrap().is_empty(),
            "proof must not create a terminal"
        );
        assert_eq!(
            adapter.inventory_calls.load(Ordering::SeqCst),
            16,
            "each proof and each Create plan uses fresh inventory"
        );
        std::fs::remove_dir_all(&root).unwrap();
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
