use std::collections::BTreeMap;
use std::io::{ErrorKind, Read};
use std::path::{Component, Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use cap_fs_ext::{DirExt, OpenOptionsFollowExt, OpenOptionsSyncExt};
use cap_std::fs::{Dir, Metadata, OpenOptions};
use cockpit_protocol::context::{
    ContextDirectory, ContextDirectoryRequest, ContextDocument, ContextDocumentRequest,
    ContextEntry, ContextEntryKind, ContextLaunchRequest, ContextRoot, ContextRootKind,
    DetectionConfidence, ExtensionKind, PanePresentation, ReviewLaunchRequest,
};
use cockpit_protocol::context_assets::{ContextSnapshotRequest, ContextSnapshotResponse};
use cockpit_protocol::projects::{ProjectConfiguration, ProjectDiagnostic};
use cockpit_protocol::sources::{
    SourceImportRequest, SourceImportResponse, SourceListRequest, SourceRefreshRequest,
};
use sha2::{Digest, Sha256};
use tokio::process::Command;
use tokio::sync::Semaphore;

use crate::InspectionError;
use crate::extension_adapter::{ExtensionHerdrAdapter, ExtensionLaunch, ExtensionPaneEvidence};
use crate::projects::ProjectService;
use crate::repositories::RepositoryCatalog;
use crate::sources::{SourceAuthority, SourceFetchRequest, SourceService};

#[derive(Clone)]
pub struct ContextService {
    pub(crate) configuration: ProjectConfiguration,
    adapter: Arc<dyn ExtensionHerdrAdapter>,
    projects: Arc<ProjectService>,
    sources: Option<Arc<SourceService>>,
    /// Shared across transient host handlers so bounded searches cannot exhaust blocking workers.
    pub(crate) search_permits: Arc<Semaphore>,
}

/// Fresh, internal proof used by the durable reference-comment service.
///
/// The proof intentionally contains only identity and the authorized companion
/// root. Callers cannot provide any of these values as authority.
#[derive(Debug, Clone)]
pub(crate) struct ContextCommentEvidence {
    pub binding_id: String,
    pub terminal_id: String,
    pub workspace_id: String,
    pub tab_id: String,
    pub root_id: String,
    pub companion_id: String,
    pub companion_path: String,
}

pub(crate) struct AuthorizedRoot {
    root: ContextRoot,
    canonical: PathBuf,
    dir: Dir,
    max_depth: u32,
}

impl ContextService {
    pub fn new(
        configuration: ProjectConfiguration,
        adapter: Arc<dyn ExtensionHerdrAdapter>,
        projects: Arc<ProjectService>,
    ) -> Self {
        Self {
            configuration,
            adapter,
            projects,
            sources: None,
            search_permits: Arc::new(Semaphore::new(2)),
        }
    }
    pub fn with_sources(mut self, sources: Arc<SourceService>) -> Self {
        self.sources = Some(sources);
        self
    }

    pub async fn import_source(
        &self,
        session_id: &str,
        pane_id: &str,
        request: &SourceImportRequest,
    ) -> Result<SourceImportResponse, InspectionError> {
        let authorized = self
            .authorize_companion_root(session_id, pane_id, &request.binding_id, &request.root_id)
            .await?;
        let companion_id = authorized.root.companion_id.as_deref().ok_or_else(|| {
            InspectionError::new(
                "source_companion_unavailable",
                "the authorized companion has no durable identity",
            )
        })?;
        let service = self.sources.as_ref().ok_or_else(|| {
            InspectionError::new(
                "source_provider_unsupported",
                "source import is not configured",
            )
        })?;
        let authority = verified_origin(
            &self.configuration,
            Path::new(&authorized.root.checkout_path),
            &request.provider_id,
        )
        .await?;
        let mut response = service
            .fetch_to_companion_hydrated(
                SourceFetchRequest {
                    provider_id: request.provider_id.clone(),
                    artifact_url: request.artifact_url.clone(),
                    authority,
                },
                Some((&authorized.dir, companion_id)),
                request.hydrate_references,
            )
            .await?;
        response.binding_id = request.binding_id.clone();
        response.root_id = authorized.root.root_id;
        Ok(response)
    }
    pub async fn refresh_source(
        &self,
        session_id: &str,
        pane_id: &str,
        request: &SourceRefreshRequest,
    ) -> Result<SourceImportResponse, InspectionError> {
        let authorized = self
            .authorize_companion_root(session_id, pane_id, &request.binding_id, &request.root_id)
            .await?;
        let companion_id = authorized.root.companion_id.as_deref().ok_or_else(|| {
            InspectionError::new(
                "source_companion_unavailable",
                "the authorized companion has no durable identity",
            )
        })?;
        let service = self.sources.as_ref().ok_or_else(|| {
            InspectionError::new(
                "source_provider_unsupported",
                "source import is not configured",
            )
        })?;
        let cached = service.find_provider_id(&request.source_id)?;
        let authority = verified_origin(
            &self.configuration,
            Path::new(&authorized.root.checkout_path),
            &cached,
        )
        .await?;
        let mut response = service
            .refresh_cached_hydrated(
                &request.source_id,
                authority,
                Some((&authorized.dir, companion_id)),
                request.hydrate_references,
            )
            .await?;
        response.binding_id = request.binding_id.clone();
        response.root_id = authorized.root.root_id;
        Ok(response)
    }
    pub async fn list_sources(
        &self,
        session_id: &str,
        pane_id: &str,
        request: &SourceListRequest,
    ) -> Result<SourceImportResponse, InspectionError> {
        let authorized = self
            .authorize_companion_root(session_id, pane_id, &request.binding_id, &request.root_id)
            .await?;
        let companion_id = authorized.root.companion_id.as_deref().ok_or_else(|| {
            InspectionError::new(
                "source_companion_unavailable",
                "the authorized companion has no durable identity",
            )
        })?;
        let service = self.sources.as_ref().ok_or_else(|| {
            InspectionError::new(
                "source_provider_unsupported",
                "source import is not configured",
            )
        })?;
        Ok(SourceImportResponse {
            binding_id: request.binding_id.clone(),
            root_id: authorized.root.root_id,
            entries: service.list_for_companion(&authorized.dir, companion_id)?,
            diagnostics: Vec::new(),
        })
    }
    /// Resolve the current Context pane and its companion-only source
    /// association. This is deliberately crate-visible: comments must use
    /// fresh Herdr/process evidence rather than persisted or caller-supplied
    /// attachment fields.
    pub(crate) async fn comment_evidence(
        &self,
        session_id: &str,
        pane_id: &str,
        binding_id: &str,
    ) -> Result<ContextCommentEvidence, InspectionError> {
        let evidence = self
            .adapter
            .inspect_extension_pane(session_id, pane_id)
            .await?;
        if evidence.pane_id != pane_id {
            return Err(InspectionError::new(
                "pane_identity_mismatch",
                "Herdr returned evidence for a different pane",
            ));
        }
        let presentation = self.presentation(session_id, &evidence).await?;
        require_binding(&presentation, binding_id)?;
        if presentation.extension != Some(ExtensionKind::Context)
            || presentation.renderer != Some(ExtensionKind::Context)
            || !verified_confidence(presentation.confidence)
        {
            return Err(InspectionError::new(
                "comments_detached",
                "comments require a verified Context companion pane",
            ));
        }
        let root_id = presentation.default_root_id.ok_or_else(|| {
            InspectionError::new(
                "comments_detached",
                "the Context pane has no verified companion root",
            )
        })?;
        let root = presentation
            .roots
            .iter()
            .find(|root| root.root_id == root_id)
            .ok_or_else(|| {
                InspectionError::new(
                    "comments_detached",
                    "the Context companion root is no longer authorized",
                )
            })?;
        if root.kind != ContextRootKind::Companion {
            return Err(InspectionError::new(
                "comments_detached",
                "comments can only attach to a Context companion",
            ));
        }
        let companion_id = root.companion_id.clone().ok_or_else(|| {
            InspectionError::new(
                "comments_detached",
                "the Context companion has no verified identity",
            )
        })?;
        Ok(ContextCommentEvidence {
            binding_id: presentation.binding_id,
            terminal_id: presentation.terminal_id,
            workspace_id: evidence.workspace_id,
            tab_id: evidence.tab_id,
            root_id,
            companion_id,
            companion_path: root.path.clone(),
        })
    }

    /// Freshly authorize a companion root for a bounded, read-only operation.
    /// The returned descriptor capability cannot be constructed by a caller and
    /// retains the same root identity and no-follow policy as Context reads.
    pub(crate) async fn authorize_companion_root(
        &self,
        session_id: &str,
        pane_id: &str,
        binding_id: &str,
        root_id: &str,
    ) -> Result<AuthorizedRoot, InspectionError> {
        let presentation = self.inspect_pane(session_id, pane_id).await?;
        require_binding(&presentation, binding_id)?;
        let mut authorized = find_root(&presentation.roots, root_id)?;
        if authorized.root.kind != ContextRootKind::Companion {
            return Err(InspectionError::new(
                "context_root_not_companion",
                "bounded Context search is available only for companion roots",
            ));
        }
        authorized.max_depth = self.configuration.limits.context_tree_depth;
        Ok(authorized)
    }

    /// Materialize a freshly resolved local repository under this pane's
    /// currently authorized companion. The request never carries a filesystem
    /// path, so it cannot widen the Context root capability.
    pub async fn snapshot_local_repository(
        &self,
        session_id: &str,
        pane_id: &str,
        request: &ContextSnapshotRequest,
    ) -> Result<ContextSnapshotResponse, InspectionError> {
        let authorized = self
            .authorize_companion_root(session_id, pane_id, &request.binding_id, &request.root_id)
            .await?;
        let companion_id = authorized.root.companion_id.as_deref().ok_or_else(|| {
            InspectionError::new(
                "context_snapshot_companion_unavailable",
                "the authorized companion has no durable identity",
            )
        })?;
        let repository = RepositoryCatalog::new(self.configuration.clone())
            .resolve(&request.repository_id)
            .await?;
        let mut response = crate::context_assets::snapshot_working_tree(
            &self.configuration,
            companion_id,
            &authorized.dir,
            &authorized.canonical,
            &repository,
        )
        .await?;
        response.binding_id = request.binding_id.clone();
        response.root_id = authorized.root.root_id;
        Ok(response)
    }

    pub async fn inspect_pane(
        &self,
        session_id: &str,
        pane_id: &str,
    ) -> Result<PanePresentation, InspectionError> {
        let evidence = self
            .adapter
            .inspect_extension_pane(session_id, pane_id)
            .await?;
        if evidence.pane_id != pane_id {
            return Err(InspectionError::new(
                "pane_identity_mismatch",
                "Herdr returned evidence for a different pane",
            ));
        }
        self.presentation(session_id, &evidence).await
    }

    pub async fn directory(
        &self,
        session_id: &str,
        pane_id: &str,
        request: &ContextDirectoryRequest,
    ) -> Result<ContextDirectory, InspectionError> {
        let presentation = self.inspect_pane(session_id, pane_id).await?;
        require_binding(&presentation, &request.binding_id)?;
        let authorized = find_root(&presentation.roots, &request.root_id)?;
        let relative = relative_path(&request.path)?;
        check_depth(&relative, self.configuration.limits.context_tree_depth)?;
        if let Some(message) = reserved_context_path(authorized.root.kind, &relative) {
            return Err(InspectionError::new("context_reserved_path", message));
        }
        let directory = resolve_directory(&authorized.dir, &relative)?;
        let metadata = directory.dir_metadata().map_err(|error| {
            InspectionError::new("context_directory_unavailable", error.to_string())
        })?;
        if !metadata.is_dir() {
            return Err(InspectionError::new(
                "context_not_directory",
                "requested context path is not a directory",
            ));
        }
        revalidate_root(&authorized.dir, &authorized.root.root_id)?;
        let limit = self.configuration.limits.context_directory_entries as usize;
        let mut entries = Vec::new();
        let mut truncated = false;
        let read_dir = directory.entries().map_err(|error| {
            InspectionError::new("context_directory_unavailable", error.to_string())
        })?;
        for item in read_dir {
            if entries.len() >= limit {
                truncated = true;
                break;
            }
            let item = match item {
                Ok(item) => item,
                Err(error) => {
                    entries.push(ContextEntry {
                        entry_id: stable_id(&authorized.root.root_id, "<unavailable>"),
                        name: "<unavailable>".to_owned(),
                        path: None,
                        kind: ContextEntryKind::Other,
                        bytes: None,
                        revision: String::new(),
                        refusal: Some(format!("entry became unavailable: {error}")),
                    });
                    continue;
                }
            };
            let name = item.file_name().to_string_lossy().into_owned();
            if name.contains('\0') {
                continue;
            }
            let child_relative = if relative.as_os_str().is_empty() {
                PathBuf::from(&name)
            } else {
                relative.join(&name)
            };
            if reserved_context_path(authorized.root.kind, &child_relative).is_some() {
                continue;
            }
            let entry_id = stable_id(&authorized.root.root_id, &child_relative.to_string_lossy());
            let metadata = match directory.symlink_metadata(Path::new(&name)) {
                Ok(metadata) => metadata,
                Err(error) => {
                    entries.push(ContextEntry {
                        entry_id,
                        name,
                        path: None,
                        kind: ContextEntryKind::Other,
                        bytes: None,
                        revision: String::new(),
                        refusal: Some(format!("entry became unavailable: {error}")),
                    });
                    continue;
                }
            };
            let (kind, path, bytes, refusal) =
                match reserved_context_path(authorized.root.kind, &child_relative) {
                    Some(message) => (
                        ContextEntryKind::Other,
                        None,
                        None,
                        Some(message.to_owned()),
                    ),
                    None => classify_entry(&metadata, &child_relative),
                };
            entries.push(ContextEntry {
                entry_id,
                name,
                path,
                kind,
                bytes,
                revision: metadata_revision(&metadata),
                refusal,
            });
        }
        entries.sort_by(|left, right| {
            let left_dir = left.kind == ContextEntryKind::Directory;
            let right_dir = right.kind == ContextEntryKind::Directory;
            right_dir.cmp(&left_dir).then(left.name.cmp(&right.name))
        });
        Ok(ContextDirectory {
            binding_id: presentation.binding_id,
            root_id: authorized.root.root_id,
            path: relative.to_string_lossy().into_owned(),
            entries,
            truncated,
            diagnostics: presentation.diagnostics,
        })
    }

    pub async fn document(
        &self,
        session_id: &str,
        pane_id: &str,
        request: &ContextDocumentRequest,
    ) -> Result<ContextDocument, InspectionError> {
        let presentation = self.inspect_pane(session_id, pane_id).await?;
        require_binding(&presentation, &request.binding_id)?;
        let authorized = find_root(&presentation.roots, &request.root_id)?;
        let relative = relative_path(&request.path)?;
        if let Some(message) = reserved_context_path(authorized.root.kind, &relative) {
            return Err(InspectionError::new("context_reserved_path", message));
        }
        check_depth(&relative, self.configuration.limits.context_tree_depth)?;
        let (parent, leaf) = resolve_parent(&authorized.dir, &relative)?;
        let before = parent.symlink_metadata(&leaf).map_err(|error| {
            InspectionError::new(
                if error.kind() == ErrorKind::NotFound {
                    "context_file_missing"
                } else {
                    "context_file_unavailable"
                },
                format!("cannot inspect context file: {error}"),
            )
        })?;
        let revision = metadata_revision(&before);
        if let Some(expected) = request.expected_revision.as_deref() {
            if expected != revision {
                return Err(InspectionError::new(
                    "context_stale_revision",
                    "context file changed since it was listed",
                ));
            }
        }
        let mut result = ContextDocument {
            binding_id: presentation.binding_id,
            root_id: authorized.root.root_id.clone(),
            path: relative.to_string_lossy().into_owned(),
            revision: revision.clone(),
            content_hash: None,
            bytes: before.len(),
            media_type: media_type(&relative),
            text: None,
            truncated: false,
            diagnostics: Vec::new(),
        };
        if before.file_type().is_symlink() {
            refuse_document(
                &mut result,
                "context_symlink_refused",
                "symbolic links are not followed",
            );
            return Ok(result);
        }
        if !before.is_file() {
            refuse_document(
                &mut result,
                "context_special_file_refused",
                "special files cannot be opened",
            );
            return Ok(result);
        }
        let max_bytes = self.configuration.limits.context_preview_bytes as usize;
        if before.len() > max_bytes as u64 {
            result.truncated = true;
            result.diagnostics.push(diagnostic(
                "context_preview_bytes",
                "file exceeds the configured preview byte limit",
                Some(&result.path),
            ));
            return Ok(result);
        }
        revalidate_root(&authorized.dir, &authorized.root.root_id)?;
        let mut options = OpenOptions::new();
        options
            .read(true)
            .follow(cap_fs_ext::FollowSymlinks::No)
            .nonblock(true);
        let file = parent.open_with(&leaf, &options).map_err(|error| {
            let code = if error.kind() == ErrorKind::WouldBlock || is_symlink_open_error(&error) {
                "context_special_file_refused"
            } else if error.kind() == ErrorKind::NotFound {
                "context_file_missing"
            } else {
                "context_file_unavailable"
            };
            InspectionError::new(code, format!("cannot open context file: {error}"))
        })?;
        let opened = file.metadata().map_err(|error| {
            InspectionError::new(
                "context_file_unavailable",
                format!("cannot stat context file: {error}"),
            )
        })?;
        if !opened.is_file() || opened.file_type().is_symlink() {
            refuse_document(
                &mut result,
                "context_special_file_refused",
                "file type changed before opening",
            );
            return Ok(result);
        }
        if metadata_revision(&opened) != revision {
            return Err(InspectionError::new(
                "context_changed_during_read",
                "context file changed while opening",
            ));
        }
        let mut content = Vec::with_capacity(before.len() as usize);
        file.take(max_bytes.saturating_add(1) as u64)
            .read_to_end(&mut content)
            .map_err(|error| {
                InspectionError::new(
                    "context_file_unavailable",
                    format!("cannot read context file: {error}"),
                )
            })?;
        let after = parent.symlink_metadata(&leaf).map_err(|error| {
            InspectionError::new(
                "context_changed_during_read",
                format!("cannot recheck context file: {error}"),
            )
        })?;
        if metadata_revision(&after) != revision {
            return Err(InspectionError::new(
                "context_changed_during_read",
                "context file changed while reading",
            ));
        }
        if content.len() > max_bytes {
            result.truncated = true;
            result.diagnostics.push(diagnostic(
                "context_preview_bytes",
                "file grew beyond the configured preview byte limit",
                Some(&result.path),
            ));
            return Ok(result);
        }
        if content.contains(&0) || std::str::from_utf8(&content).is_err() {
            result.content_hash = Some(hash_bytes(&content));
            refuse_document(
                &mut result,
                "context_binary",
                "binary or non-UTF-8 content is not rendered as source",
            );
            return Ok(result);
        }
        let max_lines = self.configuration.limits.context_preview_lines as usize;
        let (text_bytes, line_truncated) = bounded_lines(&content, max_lines);
        result.truncated = line_truncated;
        if line_truncated {
            result.diagnostics.push(diagnostic(
                "context_preview_lines",
                "file exceeds the configured preview line limit",
                Some(&result.path),
            ));
        }
        result.content_hash = Some(hash_bytes(&content));
        result.text = Some(String::from_utf8(text_bytes.to_vec()).expect("validated UTF-8"));
        Ok(result)
    }
    pub async fn open(
        &self,
        session_id: &str,
        request: &ContextLaunchRequest,
    ) -> Result<PanePresentation, InspectionError> {
        let source = self
            .adapter
            .inspect_extension_pane(session_id, &request.pane_id)
            .await?;
        if source.pane_id != request.pane_id {
            return Err(InspectionError::new(
                "pane_identity_mismatch",
                "Herdr returned evidence for a different pane",
            ));
        }
        let presentation = self.presentation(session_id, &source).await?;
        require_binding(&presentation, &request.binding_id)?;
        if !presentation.can_open_context {
            return Err(InspectionError::new(
                "context_open_unavailable",
                "this pane is not at an authorized companion context",
            ));
        }
        let actual_cwd = evidence_cwd(&source).ok_or_else(|| {
            InspectionError::new(
                "context_open_unavailable",
                "the source pane cwd is unavailable or unsafe",
            )
        })?;
        let root = find_root(&presentation.roots, &request.root_id)?;
        if root.root.kind != ContextRootKind::Companion || root.canonical != actual_cwd {
            return Err(InspectionError::new(
                "context_open_unavailable",
                "Context must open from the source pane's exact companion root",
            ));
        }
        revalidate_root(&root.dir, &root.root.root_id)?;
        let launched = self
            .adapter
            .launch_context_pane(
                session_id,
                &ExtensionLaunch {
                    endpoint_identity: source.endpoint_identity,
                    pane_id: source.pane_id,
                    terminal_id: source.terminal_id,
                    workspace_id: source.workspace_id,
                    cwd: root.canonical.to_string_lossy().into_owned(),
                    direction: request.direction,
                },
            )
            .await?;
        self.presentation(session_id, &launched).await
    }

    /// Open the installed Reviewr pane from an authorized repository checkout.
    /// The requested repository is an opaque root identity, never a
    /// caller-controlled filesystem path.
    pub async fn open_review(
        &self,
        session_id: &str,
        request: &ReviewLaunchRequest,
    ) -> Result<PanePresentation, InspectionError> {
        if request.pane_id.is_empty()
            || request.binding_id.is_empty()
            || request.repository_id.is_empty()
            || request.repository_id.len() > 256
            || request.repository_id.chars().any(char::is_control)
        {
            return Err(InspectionError::new(
                "review_open_invalid_request",
                "review launch request is incomplete",
            ));
        }
        let source = self
            .adapter
            .inspect_extension_pane(session_id, &request.pane_id)
            .await?;
        if source.pane_id != request.pane_id {
            return Err(InspectionError::new(
                "pane_identity_mismatch",
                "Herdr returned evidence for a different pane",
            ));
        }
        let presentation = self.presentation(session_id, &source).await?;
        require_binding(&presentation, &request.binding_id)?;
        if !presentation.can_open_review {
            return Err(InspectionError::new(
                "review_open_unavailable",
                "Reviewr is unavailable for the current configured repository checkout",
            ));
        }
        let checkout =
            self.review_checkout_for_presentation(&presentation, &request.repository_id)?;
        let launched = self
            .adapter
            .launch_review_pane(
                session_id,
                &ExtensionLaunch {
                    endpoint_identity: source.endpoint_identity,
                    pane_id: source.pane_id,
                    terminal_id: source.terminal_id,
                    workspace_id: source.workspace_id,
                    cwd: checkout.to_string_lossy().into_owned(),
                    direction: request.direction,
                },
            )
            .await?;
        self.presentation(session_id, &launched).await
    }

    fn review_checkout_for_presentation(
        &self,
        presentation: &PanePresentation,
        repository_id: &str,
    ) -> Result<PathBuf, InspectionError> {
        let root = presentation
            .roots
            .iter()
            .find(|root| {
                matches!(
                    root.kind,
                    ContextRootKind::Repository | ContextRootKind::Companion
                ) && root.repository_id == repository_id
            })
            .ok_or_else(|| {
                InspectionError::new(
                    "context_root_not_authorized",
                    "selected repository is not authorized for this source pane",
                )
            })?;
        let (checkout, _, _) =
            canonical_directory(Path::new(&root.checkout_path)).map_err(|_| {
                InspectionError::new(
                    "review_open_unavailable",
                    "selected repository checkout is unavailable",
                )
            })?;
        Ok(checkout)
    }

    async fn presentation(
        &self,
        session_id: &str,
        evidence: &ExtensionPaneEvidence,
    ) -> Result<PanePresentation, InspectionError> {
        let (roots, diagnostics, viewer_companion_id) =
            self.authorized_roots(session_id, evidence).await?;
        let mut roots: Vec<_> = roots.into_values().collect();
        roots.sort_by_key(|root| {
            (
                if root.root.kind == ContextRootKind::Repository {
                    0u8
                } else {
                    1u8
                },
                root.root.root_id.clone(),
            )
        });
        let actual_cwd = evidence_cwd(evidence);
        let can_open_context = evidence.can_open_context
            && actual_cwd.as_ref().is_some_and(|cwd| {
                roots.iter().any(|root| {
                    root.root.kind == ContextRootKind::Companion
                        && Path::new(&root.root.path) == cwd.as_path()
                })
            });
        let can_open_review = evidence.can_open_review
            && roots.iter().any(|root| {
                matches!(
                    root.root.kind,
                    ContextRootKind::Repository | ContextRootKind::Companion
                )
            });
        let renderer = match (evidence.extension, evidence.confidence) {
            (Some(ExtensionKind::Review), confidence) if verified_confidence(confidence) => {
                Some(ExtensionKind::Review)
            }
            (Some(ExtensionKind::Context), confidence)
                if verified_confidence(confidence) && viewer_companion_id.is_some() =>
            {
                Some(ExtensionKind::Context)
            }
            _ => None,
        };
        let default_root_id = if renderer == Some(ExtensionKind::Context) {
            viewer_companion_id.clone()
        } else {
            actual_cwd
                .as_deref()
                .and_then(|cwd| {
                    roots
                        .iter()
                        .filter(|root| is_within(cwd, Path::new(&root.root.path)))
                        .max_by_key(|root| Path::new(&root.root.path).components().count())
                })
                .map(|root| root.root.root_id.clone())
                .or_else(|| roots.first().map(|root| root.root.root_id.clone()))
        };
        let mut reason = evidence.reason.clone();
        if evidence.extension == Some(ExtensionKind::Context)
            && renderer != Some(ExtensionKind::Context)
        {
            reason.push_str("; the viewer browsing root is not its verified companion directory");
        }
        if evidence.can_open_context && !can_open_context {
            reason.push_str("; Open Context requires a source pane at its companion directory");
        }
        if evidence.can_open_review && !can_open_review {
            reason.push_str(
                "; Open Review requires an authorized repository or companion association",
            );
        }
        Ok(PanePresentation {
            session_id: session_id.to_owned(),
            pane_id: evidence.pane_id.clone(),
            terminal_id: evidence.terminal_id.clone(),
            binding_id: binding_id(session_id, evidence),
            extension: evidence.extension,
            renderer,
            confidence: evidence.confidence,
            reason,
            roots: roots.into_iter().map(|root| root.root).collect(),
            default_root_id,
            can_open_context,
            can_open_review,
            diagnostics,
        })
    }

    async fn authorized_roots(
        &self,
        session_id: &str,
        evidence: &ExtensionPaneEvidence,
    ) -> Result<
        (
            BTreeMap<String, AuthorizedRoot>,
            Vec<ProjectDiagnostic>,
            Option<String>,
        ),
        InspectionError,
    > {
        let listed = RepositoryCatalog::new(self.configuration.clone())
            .list()
            .await?;
        let mut diagnostics = listed.diagnostics;
        let actual_cwd = evidence_cwd(evidence);
        let mut companion_roots = Vec::new();
        if !evidence.workspace_id.is_empty() && !evidence.endpoint_identity.is_empty() {
            match self
                .projects
                .context_companions(
                    session_id,
                    &evidence.workspace_id,
                    &evidence.endpoint_identity,
                )
                .await
            {
                Ok(companions) => {
                    for companion in companions {
                        let (path, dir, metadata) =
                            match canonical_directory(Path::new(&companion.path)) {
                                Ok(opened) => opened,
                                Err(_) => {
                                    diagnostics.push(diagnostic(
                                        "context_companion_unavailable",
                                        "authorized companion is no longer a directory",
                                        Some(&companion.path),
                                    ));
                                    continue;
                                }
                            };
                        let root_id = format!(
                            "companion:{}:{}",
                            companion
                                .companion_id
                                .as_deref()
                                .unwrap_or(&companion.repository_id),
                            filesystem_identity(&metadata)
                        );
                        let mut root = companion;
                        root.root_id = root_id.clone();
                        root.path = path.to_string_lossy().into_owned();
                        companion_roots.push(AuthorizedRoot {
                            root,
                            canonical: path,
                            dir,
                            max_depth: self.configuration.limits.context_tree_depth,
                        });
                    }
                }
                Err(error) => diagnostics.push(diagnostic(&error.code, &error.message, None)),
            }
        }
        let viewer_companion =
            resolve_viewer_companion(&self.configuration, evidence, &companion_roots).await;
        let association_cwd = viewer_companion
            .as_ref()
            .map(|(_, path)| path.clone())
            .or(actual_cwd);
        let viewer_companion_id = viewer_companion
            .as_ref()
            .map(|(root_id, _)| root_id.clone());
        let mut roots = BTreeMap::new();
        for candidate in listed.repositories {
            let (checkout, dir, metadata) =
                match canonical_directory(Path::new(&candidate.checkout_path)) {
                    Ok(opened) => opened,
                    Err(_) => continue,
                };
            let cwd_in_checkout = association_cwd
                .as_ref()
                .is_some_and(|cwd| is_within(cwd, &checkout));
            let cwd_in_companion = companion_roots.iter().any(|companion| {
                let associated_checkout =
                    absolute_context_path(Path::new(&companion.root.checkout_path)).ok();
                association_cwd.as_ref().is_some_and(|cwd| {
                    is_within(cwd, &companion.canonical)
                        && companion.root.repository_id == candidate.repository_id
                        && associated_checkout.as_deref() == Some(checkout.as_path())
                })
            });
            if !cwd_in_checkout && !cwd_in_companion {
                continue;
            }
            let root_id = format!(
                "repository:{}:{}",
                candidate.repository_id,
                filesystem_identity(&metadata)
            );
            let root = ContextRoot {
                root_id: root_id.clone(),
                kind: ContextRootKind::Repository,
                label: candidate.name,
                path: checkout.to_string_lossy().into_owned(),
                repository_id: candidate.repository_id,
                checkout_path: checkout.to_string_lossy().into_owned(),
                companion_id: None,
            };
            roots.insert(
                root_id,
                AuthorizedRoot {
                    root,
                    canonical: checkout,
                    dir,
                    max_depth: self.configuration.limits.context_tree_depth,
                },
            );
        }
        for companion in companion_roots {
            let root_id = companion.root.root_id.clone();
            roots.insert(root_id, companion);
        }
        Ok((roots, diagnostics, viewer_companion_id))
    }
}

async fn verified_origin(
    configuration: &ProjectConfiguration,
    checkout: &Path,
    provider_id: &str,
) -> Result<SourceAuthority, InspectionError> {
    let mut command = Command::new("git");
    command
        .current_dir(checkout)
        .args([
            "-c",
            "core.hooksPath=/dev/null",
            "remote",
            "get-url",
            "origin",
        ])
        .env("GIT_TERMINAL_PROMPT", "0")
        .env_remove("GIT_DIR")
        .env_remove("GIT_WORK_TREE");
    let output = crate::process::run_bounded_command(
        command,
        configuration.limits.git_output_bytes as usize,
        configuration.limits.git_output_bytes as usize,
        Duration::from_millis(configuration.limits.git_timeout_ms as u64),
        "source origin",
    )
    .await?;
    if !output.status.success() {
        return Err(InspectionError::new(
            "source_primary_origin_unavailable",
            "the companion primary checkout has no readable origin remote",
        ));
    }
    let provider = configuration
        .providers
        .iter()
        .find(|provider| provider.id == provider_id)
        .ok_or_else(|| {
            InspectionError::new(
                "source_provider_unsupported",
                "selected source provider is not configured",
            )
        })?;
    let instance = normalized_provider_instance(&provider.base_url)?;
    let value = std::str::from_utf8(&output.stdout)
        .map_err(|_| {
            InspectionError::new(
                "source_primary_origin_invalid",
                "origin remote is not UTF-8",
            )
        })?
        .trim();
    let (owner, repository) = origin_repository(value, &instance)?;
    Ok(SourceAuthority {
        provider_instance: instance.render(),
        origin_host: instance.host,
        origin_port: instance.port,
        origin_base_path: instance.base_path,
        owner,
        repository,
    })
}

#[derive(Debug)]
struct ProviderInstance {
    scheme: String,
    host: String,
    port: Option<u16>,
    base_path: String,
}
impl ProviderInstance {
    fn render(&self) -> String {
        format!(
            "{}://{}{}{}",
            self.scheme,
            self.host,
            self.port.map(|port| format!(":{port}")).unwrap_or_default(),
            self.base_path
        )
    }
}
fn normalized_provider_instance(value: &str) -> Result<ProviderInstance, InspectionError> {
    let url = url::Url::parse(value).map_err(|_| {
        InspectionError::new(
            "source_provider_invalid",
            "configured provider URL is invalid",
        )
    })?;
    if !matches!(url.scheme(), "http" | "https")
        || url.username() != ""
        || url.password().is_some()
        || url.query().is_some()
        || url.fragment().is_some()
    {
        return Err(InspectionError::new(
            "source_provider_invalid",
            "configured provider URL must be credential-free HTTP(S)",
        ));
    }
    let host = url
        .host_str()
        .ok_or_else(|| {
            InspectionError::new(
                "source_provider_invalid",
                "configured provider URL lacks a host",
            )
        })?
        .to_ascii_lowercase();
    let default_port = match url.scheme() {
        "http" => 80,
        "https" => 443,
        _ => unreachable!(),
    };
    let port = url.port().filter(|port| *port != default_port);
    let path = url.path().trim_end_matches('/');
    let base_path = if path.is_empty() {
        String::new()
    } else {
        path.to_owned()
    };
    Ok(ProviderInstance {
        scheme: url.scheme().to_ascii_lowercase(),
        host,
        port,
        base_path,
    })
}
fn origin_repository(
    value: &str,
    instance: &ProviderInstance,
) -> Result<(String, String), InspectionError> {
    let (host, port, path, scheme) = if value.contains("://") {
        let url = url::Url::parse(value).map_err(|_| {
            InspectionError::new("source_primary_origin_invalid", "origin remote is invalid")
        })?;
        let host = url
            .host_str()
            .ok_or_else(|| {
                InspectionError::new(
                    "source_primary_origin_invalid",
                    "origin remote lacks a host",
                )
            })?
            .to_ascii_lowercase();
        (
            host,
            url.port(),
            url.path().to_owned(),
            Some(url.scheme().to_ascii_lowercase()),
        )
    } else {
        let (_, tail) = value.rsplit_once('@').unwrap_or(("", value));
        let (host, path) = tail.split_once(':').ok_or_else(|| {
            InspectionError::new(
                "source_primary_origin_invalid",
                "origin remote does not identify owner/repository",
            )
        })?;
        (host.to_ascii_lowercase(), None, format!("/{path}"), None)
    };
    if host != instance.host
        || scheme
            .as_deref()
            .is_some_and(|scheme| scheme != instance.scheme)
        || (scheme.is_some()
            && port.filter(|port| *port != if instance.scheme == "https" { 443 } else { 80 })
                != instance.port)
    {
        return Err(InspectionError::new(
            "source_primary_origin_mismatch",
            "primary origin does not match the selected configured provider instance",
        ));
    }
    let prefix = if instance.base_path.is_empty() {
        "/".to_owned()
    } else {
        format!("{}/", instance.base_path)
    };
    let remainder = path
        .strip_prefix(&prefix)
        .ok_or_else(|| {
            InspectionError::new(
                "source_primary_origin_mismatch",
                "primary origin does not match the configured provider base path",
            )
        })?
        .trim_end_matches(".git");
    let mut parts = remainder.split('/');
    let owner = parts.next().unwrap_or("");
    let repository = parts.next().unwrap_or("");
    if owner.is_empty()
        || repository.is_empty()
        || parts.next().is_some()
        || owner.chars().any(char::is_control)
        || repository.chars().any(char::is_control)
    {
        return Err(InspectionError::new(
            "source_primary_origin_invalid",
            "origin remote does not identify owner/repository",
        ));
    }
    Ok((owner.to_owned(), repository.to_owned()))
}

fn require_binding(presentation: &PanePresentation, binding: &str) -> Result<(), InspectionError> {
    if binding != presentation.binding_id {
        return Err(InspectionError::new(
            "context_stale_binding",
            "context pane identity or process evidence changed",
        ));
    }
    Ok(())
}

fn find_root(roots: &[ContextRoot], root_id: &str) -> Result<AuthorizedRoot, InspectionError> {
    let root = roots
        .iter()
        .find(|root| root.root_id == root_id)
        .ok_or_else(|| {
            InspectionError::new(
                "context_root_not_authorized",
                "context root is not authorized for this pane",
            )
        })?;
    let path = absolute_context_path(Path::new(&root.path)).map_err(|error| {
        InspectionError::new(
            "context_root_unavailable",
            format!("cannot resolve context root: {error}"),
        )
    })?;
    let dir = open_dir_nofollow_absolute(&path).map_err(|error| {
        InspectionError::new(
            "context_root_unavailable",
            format!("cannot open context root: {error}"),
        )
    })?;

    let metadata = dir
        .dir_metadata()
        .map_err(|error| InspectionError::new("context_root_unavailable", error.to_string()))?;
    if !metadata.is_dir() || root_id_identity_mismatch(root_id, &metadata) {
        return Err(InspectionError::new(
            "context_stale_root",
            "context root filesystem identity changed",
        ));
    }
    Ok(AuthorizedRoot {
        root: root.clone(),
        canonical: path,
        dir,
        max_depth: 0,
    })
}
impl AuthorizedRoot {
    pub(crate) fn root_id(&self) -> &str {
        &self.root.root_id
    }

    pub(crate) fn relative_path(&self, value: &str) -> Result<PathBuf, InspectionError> {
        let relative = relative_path(value)?;
        check_depth(&relative, self.max_depth)?;
        if let Some(message) = reserved_context_path(self.root.kind, &relative) {
            return Err(InspectionError::new("context_reserved_path", message));
        }
        Ok(relative)
    }

    pub(crate) fn resolve_parent(
        &self,
        relative: &Path,
    ) -> Result<(Dir, PathBuf), InspectionError> {
        resolve_parent(&self.dir, relative)
    }

    pub(crate) fn resolve_directory(&self, relative: &Path) -> Result<Dir, InspectionError> {
        resolve_directory(&self.dir, relative)
    }

    pub(crate) fn revalidate(&self) -> Result<(), InspectionError> {
        revalidate_root(&self.dir, &self.root.root_id)
    }
}

fn reserved_context_path(kind: ContextRootKind, relative: &Path) -> Option<&'static str> {
    let components = relative.components().filter_map(|component| {
        let Component::Normal(name) = component else {
            return None;
        };
        Some(name)
    });
    let mut first = true;
    for name in components {
        if name == ".git" {
            return Some("Git metadata is not exposed");
        }
        if kind == ContextRootKind::Companion {
            if first
                && (name == "manifest.json"
                    || name == "context-manifest.json"
                    || name == ".context-assets.lock")
            {
                return Some("Cockpit companion metadata is not exposed");
            }
            if name.to_str().is_some_and(|name| {
                (name.starts_with(".companion-") || name.starts_with(".context-snapshot-"))
                    && name.ends_with(".tmp")
            }) {
                return Some("Cockpit staging internals are not exposed");
            }
        }
        first = false;
    }
    None
}

fn relative_path(value: &str) -> Result<PathBuf, InspectionError> {
    if value.len() > 4096 || value.contains('\0') {
        return Err(InspectionError::new(
            "context_invalid_path",
            "context path is invalid",
        ));
    }
    let candidate = Path::new(value);
    if candidate.is_absolute() {
        return Err(InspectionError::new(
            "context_path_escape",
            "absolute context paths are refused",
        ));
    }
    let mut result = PathBuf::new();
    for component in candidate.components() {
        match component {
            Component::Normal(part) => result.push(part),
            Component::CurDir => {}
            Component::ParentDir | Component::RootDir | Component::Prefix(_) => {
                return Err(InspectionError::new(
                    "context_path_escape",
                    "context path escapes its root",
                ));
            }
        }
    }
    Ok(result)
}

fn resolve_directory(root: &Dir, relative: &Path) -> Result<Dir, InspectionError> {
    let mut current = root
        .try_clone()
        .map_err(|error| InspectionError::new("context_path_unavailable", error.to_string()))?;
    for component in relative.components() {
        let Component::Normal(name) = component else {
            return Err(InspectionError::new(
                "context_path_escape",
                "context path escapes its root",
            ));
        };
        let metadata = current.symlink_metadata(name).map_err(|error| {
            InspectionError::new(
                if error.kind() == ErrorKind::NotFound {
                    "context_file_missing"
                } else {
                    "context_path_unavailable"
                },
                format!("cannot access context path: {error}"),
            )
        })?;
        if metadata.file_type().is_symlink() {
            return Err(InspectionError::new(
                "context_symlink_refused",
                "symbolic links are not followed",
            ));
        }
        if !metadata.is_dir() {
            return Err(InspectionError::new(
                "context_not_directory",
                "requested context path is not a directory",
            ));
        }
        current = current
            .open_dir_nofollow(Path::new(name))
            .map_err(|error| {
                InspectionError::new(
                    if error.kind() == ErrorKind::NotFound {
                        "context_file_missing"
                    } else {
                        "context_path_unavailable"
                    },
                    format!("cannot open context directory: {error}"),
                )
            })?;
    }
    Ok(current)
}

fn resolve_parent(root: &Dir, relative: &Path) -> Result<(Dir, PathBuf), InspectionError> {
    let leaf = relative.file_name().ok_or_else(|| {
        InspectionError::new("context_not_file", "requested context path is not a file")
    })?;
    let parent_path = relative.parent().unwrap_or_else(|| Path::new(""));
    let parent = resolve_directory(root, parent_path)?;
    Ok((parent, leaf.into()))
}

fn revalidate_root(root: &Dir, root_id: &str) -> Result<(), InspectionError> {
    let metadata = root
        .dir_metadata()
        .map_err(|error| InspectionError::new("context_stale_root", error.to_string()))?;
    if !metadata.is_dir() || root_id_identity_mismatch(root_id, &metadata) {
        return Err(InspectionError::new(
            "context_stale_root",
            "context root filesystem identity changed",
        ));
    }
    Ok(())
}

fn absolute_context_path(path: &Path) -> std::io::Result<PathBuf> {
    let raw = if path.is_absolute() {
        path.to_path_buf()
    } else {
        std::env::current_dir()?.join(path)
    };
    let mut absolute = PathBuf::new();
    for component in raw.components() {
        match component {
            Component::RootDir => absolute.push(Path::new("/")),
            Component::CurDir => {}
            Component::Normal(name) => absolute.push(name),
            Component::ParentDir | Component::Prefix(_) => {
                return Err(std::io::Error::new(
                    ErrorKind::InvalidInput,
                    "unsafe context root path",
                ));
            }
        }
    }
    if absolute.is_absolute() {
        Ok(absolute)
    } else {
        Err(std::io::Error::new(
            ErrorKind::InvalidInput,
            "context root path is not absolute",
        ))
    }
}

fn verified_confidence(confidence: DetectionConfidence) -> bool {
    matches!(
        confidence,
        DetectionConfidence::VerifiedLaunch | DetectionConfidence::VerifiedProcess
    )
}

fn evidence_path(path: &str) -> Option<PathBuf> {
    let path = Path::new(path);
    path.is_absolute()
        .then(|| absolute_context_path(path).ok())
        .flatten()
}

fn evidence_cwd(evidence: &ExtensionPaneEvidence) -> Option<PathBuf> {
    let absolute = evidence_path(evidence.cwd.as_deref()?)?;
    let dir = open_dir_nofollow_absolute(&absolute).ok()?;
    let metadata = dir.dir_metadata().ok()?;
    metadata.is_dir().then_some(absolute)
}

async fn resolve_viewer_companion(
    configuration: &ProjectConfiguration,
    evidence: &ExtensionPaneEvidence,
    companions: &[AuthorizedRoot],
) -> Option<(String, PathBuf)> {
    if evidence.extension != Some(ExtensionKind::Context)
        || !verified_confidence(evidence.confidence)
    {
        return None;
    }
    let viewer_cwd = evidence.viewer_cwd.as_deref().and_then(evidence_path)?;
    for companion in companions {
        if !is_within(&viewer_cwd, &companion.canonical) {
            continue;
        }
        let resolved = resolve_viewer_root(configuration, &viewer_cwd).await?;
        if resolved == companion.canonical {
            return Some((companion.root.root_id.clone(), companion.canonical.clone()));
        }
        return None;
    }
    None
}

async fn resolve_viewer_root(configuration: &ProjectConfiguration, cwd: &Path) -> Option<PathBuf> {
    let dir = open_dir_nofollow_absolute(cwd).ok()?;
    if !dir.dir_metadata().ok()?.is_dir() {
        return None;
    }
    let mut command = Command::new("git");
    command
        .current_dir(cwd)
        .arg("--attr-source=4b825dc642cb6eb9a060e54bf8d69288fbee4904")
        .arg("-c")
        .arg("core.hooksPath=/dev/null")
        .arg("-c")
        .arg("core.fsmonitor=false")
        .arg("rev-parse")
        .arg("--show-toplevel")
        .env("GIT_OPTIONAL_LOCKS", "0")
        .env_remove("GIT_DIR")
        .env_remove("GIT_WORK_TREE")
        .env_remove("GIT_COMMON_DIR")
        .env_remove("GIT_INDEX_FILE")
        .env_remove("GIT_OBJECT_DIRECTORY")
        .env("GIT_TERMINAL_PROMPT", "0")
        .env("GIT_ASKPASS", "");
    let output = crate::process::run_bounded_command(
        command,
        configuration.limits.git_output_bytes as usize,
        configuration.limits.git_output_bytes as usize,
        Duration::from_millis(configuration.limits.git_timeout_ms as u64),
        "file_viewer_root_resolution",
    )
    .await
    .ok()?;
    if !output.status.success() {
        return Some(cwd.to_owned());
    }
    let text = std::str::from_utf8(&output.stdout).ok()?.trim();
    if text.is_empty() || text.lines().count() != 1 || text.chars().any(char::is_control) {
        return None;
    }
    let root = absolute_context_path(Path::new(text)).ok()?;
    let root_dir = open_dir_nofollow_absolute(&root).ok()?;
    root_dir.dir_metadata().ok()?.is_dir().then_some(root)
}
fn canonical_directory(path: &Path) -> Result<(PathBuf, Dir, Metadata), InspectionError> {
    let absolute = absolute_context_path(path).map_err(|error| {
        InspectionError::new(
            "context_root_unavailable",
            format!("cannot resolve context root: {error}"),
        )
    })?;
    let dir = open_dir_nofollow_absolute(&absolute).map_err(|error| {
        InspectionError::new(
            "context_root_unavailable",
            format!("cannot open context root: {error}"),
        )
    })?;
    let metadata = dir
        .dir_metadata()
        .map_err(|error| InspectionError::new("context_root_unavailable", error.to_string()))?;
    if !metadata.is_dir() {
        return Err(InspectionError::new(
            "context_root_unavailable",
            "context root is not a real directory",
        ));
    }
    Ok((absolute, dir, metadata))
}

fn check_depth(path: &Path, max_depth: u32) -> Result<(), InspectionError> {
    let depth = path.components().count() as u32;
    if depth > max_depth {
        return Err(InspectionError::new(
            "context_tree_depth",
            "context path exceeds the configured tree depth limit",
        ));
    }
    Ok(())
}

fn classify_entry(
    metadata: &Metadata,
    path: &Path,
) -> (
    ContextEntryKind,
    Option<String>,
    Option<u64>,
    Option<String>,
) {
    if metadata.file_type().is_symlink() {
        return (
            ContextEntryKind::Symlink,
            None,
            None,
            Some("symbolic links are not followed".to_owned()),
        );
    }
    if metadata.is_dir() {
        return (
            ContextEntryKind::Directory,
            Some(path.to_string_lossy().into_owned()),
            None,
            None,
        );
    }
    if metadata.is_file() {
        return (
            ContextEntryKind::File,
            Some(path.to_string_lossy().into_owned()),
            Some(metadata.len()),
            None,
        );
    }
    (
        ContextEntryKind::Other,
        None,
        None,
        Some("special files cannot be opened".to_owned()),
    )
}

fn bounded_lines(content: &[u8], max_lines: usize) -> (&[u8], bool) {
    if max_lines == 0 {
        return (&content[..0], !content.is_empty());
    }
    let mut lines = 0usize;
    for (index, byte) in content.iter().enumerate() {
        if *byte == b'\n' {
            lines += 1;
            if lines == max_lines {
                let end = index + 1;
                return if end < content.len() {
                    (&content[..end], true)
                } else {
                    (content, false)
                };
            }
        }
    }
    (content, false)
}

fn open_dir_nofollow_absolute(path: &Path) -> std::io::Result<Dir> {
    let mut dir = Dir::open_ambient_dir(Path::new("/"), cap_std::ambient_authority())?;
    for component in path.components() {
        match component {
            Component::RootDir | Component::CurDir => {}
            Component::Normal(name) => {
                dir = dir.open_dir_nofollow(Path::new(name))?;
            }
            Component::ParentDir | Component::Prefix(_) => {
                return Err(std::io::Error::new(
                    ErrorKind::InvalidInput,
                    "unsafe context root path",
                ));
            }
        }
    }
    Ok(dir)
}

fn is_symlink_open_error(error: &std::io::Error) -> bool {
    #[cfg(unix)]
    {
        return error.raw_os_error() == Some(nix::libc::ELOOP);
    }
    #[cfg(not(unix))]
    {
        let _ = error;
        false
    }
}
pub(crate) fn metadata_revision(metadata: &Metadata) -> String {
    format!(
        "{}:{}:{}",
        filesystem_identity(metadata),
        metadata.len(),
        modified_nanos(metadata)
    )
}

fn filesystem_identity(metadata: &Metadata) -> String {
    #[cfg(unix)]
    {
        use cap_std::fs::MetadataExt;
        return format!("{}:{}", metadata.dev(), metadata.ino());
    }
    #[cfg(not(unix))]
    {
        format!("{}", metadata.len())
    }
}

fn root_id_identity_mismatch(root_id: &str, metadata: &Metadata) -> bool {
    !root_id.ends_with(&format!(":{}", filesystem_identity(metadata)))
}

fn modified_nanos(metadata: &Metadata) -> u128 {
    metadata
        .modified()
        .ok()
        .and_then(|time| {
            time.duration_since(cap_std::time::SystemTime::from_std(std::time::UNIX_EPOCH))
                .ok()
        })
        .map_or(0, |duration| duration.as_nanos())
}

fn hash_bytes(bytes: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(bytes);
    format!("sha256:{:x}", hasher.finalize())
}

fn stable_id(root_id: &str, path: &str) -> String {
    hash_bytes(format!("{root_id}\0{path}").as_bytes())
}

fn binding_id(session_id: &str, evidence: &ExtensionPaneEvidence) -> String {
    stable_id(
        session_id,
        &format!(
            "{}\0{}\0{}\0{}\0{}\0{}\0{}\0{}\0{:?}\0{:?}",
            evidence.endpoint_identity,
            evidence.pane_id,
            evidence.terminal_id,
            evidence.workspace_id,
            evidence.process_identity,
            evidence.cwd.as_deref().unwrap_or(""),
            evidence.foreground_cwd.as_deref().unwrap_or(""),
            evidence.viewer_cwd.as_deref().unwrap_or(""),
            evidence.extension,
            evidence.confidence
        ),
    )
}

fn media_type(path: &Path) -> String {
    let extension = path
        .extension()
        .and_then(|extension| extension.to_str())
        .unwrap_or("")
        .to_ascii_lowercase();
    match extension.as_str() {
        "md" | "markdown" => "text/markdown",
        "json" => "application/json",
        "toml" => "application/toml",
        "yaml" | "yml" => "application/yaml",
        "rs" => "text/x-rust",
        "ts" | "tsx" => "text/typescript",
        "js" | "jsx" => "text/javascript",
        "css" => "text/css",
        "html" | "htm" => "text/html",
        "xml" => "application/xml",
        _ => "text/plain",
    }
    .to_owned()
}

fn refuse_document(result: &mut ContextDocument, code: &str, message: &str) {
    result.media_type = "application/octet-stream".to_owned();
    result
        .diagnostics
        .push(diagnostic(code, message, Some(&result.path)));
}

fn diagnostic(code: &str, message: &str, path: Option<&str>) -> ProjectDiagnostic {
    ProjectDiagnostic {
        code: code.to_owned(),
        message: message.to_owned(),
        path: path.map(str::to_owned),
    }
}

fn is_within(path: &Path, root: &Path) -> bool {
    path == root || path.strip_prefix(root).is_ok()
}

#[cfg(test)]
mod source_authority_tests {
    use super::*;

    #[test]
    fn provider_origin_requires_exact_host_port_and_base_path_but_maps_ssh() {
        let instance =
            normalized_provider_instance("https://forge.test:8443/gitea/").expect("instance");
        assert_eq!(instance.render(), "https://forge.test:8443/gitea");
        assert_eq!(
            origin_repository("git@forge.test:gitea/acme/repo.git", &instance).expect("ssh"),
            ("acme".to_owned(), "repo".to_owned())
        );
        assert_eq!(
            origin_repository("https://forge.test:8443/other/acme/repo.git", &instance)
                .expect_err("base mismatch")
                .code,
            "source_primary_origin_mismatch"
        );
        assert_eq!(
            origin_repository("https://other.test:8443/gitea/acme/repo.git", &instance)
                .expect_err("host mismatch")
                .code,
            "source_primary_origin_mismatch"
        );
    }
}
