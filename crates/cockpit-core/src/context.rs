use std::collections::BTreeMap;
use std::io::{ErrorKind, Read, Seek, SeekFrom};
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
use crate::sources::{SourceFetchRequest, SourceService, source_authority_for_checkout};
const MAX_DIRECTORY_SCAN: usize = 100_000;

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
        let authority = source_authority_for_checkout(
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
        let authority = source_authority_for_checkout(
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
        let (entries, diagnostics) = service.list_for_companion(&authorized.dir, companion_id)?;
        Ok(SourceImportResponse {
            binding_id: request.binding_id.clone(),
            root_id: authorized.root.root_id,
            entries,
            diagnostics,
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
        self.comment_evidence_for_presentation(&presentation, &evidence, binding_id)
    }

    pub(crate) fn comment_evidence_for_presentation(
        &self,
        presentation: &PanePresentation,
        evidence: &ExtensionPaneEvidence,
        binding_id: &str,
    ) -> Result<ContextCommentEvidence, InspectionError> {
        require_binding(presentation, binding_id)?;
        if presentation.extension != Some(ExtensionKind::Context)
            || presentation.renderer != Some(ExtensionKind::Context)
            || !verified_confidence(presentation.confidence)
        {
            return Err(InspectionError::new(
                "comments_detached",
                "comments require a verified file-viewer pane",
            ));
        }
        let root_id = presentation.default_root_id.clone().ok_or_else(|| {
            InspectionError::new(
                "comments_detached",
                "the file-viewer pane has no verified browsing root",
            )
        })?;
        let root = presentation
            .roots
            .iter()
            .find(|root| root.root_id == root_id)
            .ok_or_else(|| {
                InspectionError::new(
                    "comments_detached",
                    "the file-viewer root is no longer authorized",
                )
            })?;
        let companion_id = match root.kind {
            ContextRootKind::Companion => root.companion_id.clone().ok_or_else(|| {
                InspectionError::new(
                    "comments_detached",
                    "the companion has no verified identity",
                )
            })?,
            ContextRootKind::Folder => root.root_id.clone(),
            ContextRootKind::Repository => {
                return Err(InspectionError::new(
                    "comments_detached",
                    "comments require the file-viewer's verified browsing root",
                ));
            }
        };
        Ok(ContextCommentEvidence {
            binding_id: presentation.binding_id.clone(),
            terminal_id: presentation.terminal_id.clone(),
            workspace_id: evidence.workspace_id.clone(),
            tab_id: evidence.tab_id.clone(),
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

    /// Freshly authorize a file-viewer root for bounded media reads. Folder
    /// roots are sourced only from verified viewer or source-pane evidence;
    /// repository roots never grant this capability.
    pub(crate) async fn authorize_media_root(
        &self,
        session_id: &str,
        pane_id: &str,
        binding_id: &str,
        root_id: &str,
    ) -> Result<AuthorizedRoot, InspectionError> {
        let presentation = self.inspect_pane(session_id, pane_id).await?;
        require_binding(&presentation, binding_id)?;
        let mut authorized = find_root(&presentation.roots, root_id)?;
        if !matches!(
            authorized.root.kind,
            ContextRootKind::Companion | ContextRootKind::Folder
        ) {
            return Err(InspectionError::new(
                "context_root_not_media",
                "bounded media reads are available only for companion or folder roots",
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
        let (presentation, _) = self.inspect_pane_with_evidence(session_id, pane_id).await?;
        Ok(presentation)
    }

    /// Return a presentation with the exact adapter evidence that produced it.
    /// Crate-local callers may carry this proof through one operation; callers
    /// must obtain a fresh pair for every request.
    pub(crate) async fn inspect_pane_with_evidence(
        &self,
        session_id: &str,
        pane_id: &str,
    ) -> Result<(PanePresentation, ExtensionPaneEvidence), InspectionError> {
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
        Ok((presentation, evidence))
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
        let offset = request.offset.unwrap_or(0) as usize;
        if offset > MAX_DIRECTORY_SCAN {
            return Err(InspectionError::new(
                "context_directory_bounded",
                "directory continuation offset exceeds its bounded scan limit",
            ));
        }
        let directory_revision = metadata_revision(&metadata);
        if let Some(expected) = request.revision.as_deref() {
            if expected != directory_revision {
                return Err(InspectionError::new(
                    "context_stale_revision",
                    "directory changed since it was listed",
                ));
            }
        }
        let mut entries = Vec::new();
        let mut truncated = false;
        let mut scanned_entries = 0usize;
        let read_dir = directory.entries().map_err(|error| {
            InspectionError::new("context_directory_unavailable", error.to_string())
        })?;
        for item in read_dir {
            if scanned_entries >= MAX_DIRECTORY_SCAN {
                truncated = true;
                break;
            }
            scanned_entries += 1;
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
        let after = directory.dir_metadata().map_err(|error| {
            InspectionError::new("context_directory_unavailable", error.to_string())
        })?;
        if metadata_revision(&after) != directory_revision {
            return Err(InspectionError::new(
                "context_changed_during_read",
                "directory changed while it was being listed",
            ));
        }
        entries.sort_by(|left, right| {
            let left_dir = left.kind == ContextEntryKind::Directory;
            let right_dir = right.kind == ContextEntryKind::Directory;
            right_dir.cmp(&left_dir).then(left.name.cmp(&right.name))
        });
        let total_entries = if truncated {
            None
        } else {
            Some(u32::try_from(entries.len()).unwrap_or(u32::MAX))
        };
        let page_end = offset.saturating_add(limit).min(entries.len());
        let page = if offset < entries.len() {
            entries[offset..page_end].to_vec()
        } else {
            Vec::new()
        };
        let next_offset = if page_end < entries.len() {
            Some(page_end as u32)
        } else {
            None
        };
        Ok(ContextDirectory {
            binding_id: presentation.binding_id,
            root_id: authorized.root.root_id,
            path: relative.to_string_lossy().into_owned(),
            entries: page,
            truncated,
            revision: Some(directory_revision),
            next_offset,
            total_entries,
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
            offset: Some(request.offset.unwrap_or(0)),
            next_offset: None,
            total_bytes: Some(before.len()),
            line_offset: (request.offset.unwrap_or(0) == 0).then_some(0),
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
        let offset = request.offset.unwrap_or(0) as u64;
        if offset > before.len() {
            return Err(InspectionError::new(
                "context_invalid_request",
                "document continuation offset exceeds the file",
            ));
        }
        revalidate_root(&authorized.dir, &authorized.root.root_id)?;
        let mut options = OpenOptions::new();
        options
            .read(true)
            .follow(cap_fs_ext::FollowSymlinks::No)
            .nonblock(true);
        let mut file = parent.open_with(&leaf, &options).map_err(|error| {
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
        file.seek(SeekFrom::Start(offset)).map_err(|error| {
            InspectionError::new(
                "context_file_unavailable",
                format!("cannot seek context file: {error}"),
            )
        })?;
        let mut content = Vec::with_capacity(max_bytes.min((before.len() - offset) as usize));
        (&mut file)
            .take(max_bytes as u64)
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
        let valid_bytes = match std::str::from_utf8(&content) {
            Ok(_) => content.len(),
            Err(error) if error.valid_up_to() > 0 && !content.contains(&0) => error.valid_up_to(),
            Err(_) => 0,
        };
        if valid_bytes == 0 && (!content.is_empty() || before.len() > offset) {
            refuse_document(
                &mut result,
                "context_binary",
                "binary or non-UTF-8 content is not rendered as source",
            );
            return Ok(result);
        }
        let mut consumed = valid_bytes;
        let max_lines = self.configuration.limits.context_preview_lines as usize;
        let (text_bytes, line_truncated) = bounded_lines(&content[..valid_bytes], max_lines);
        if line_truncated {
            consumed = text_bytes.len();
        }
        let end = offset.saturating_add(consumed as u64);
        result.truncated = end < before.len();
        result.next_offset = (end < before.len()).then_some(end as u32);
        if result.truncated {
            result.diagnostics.push(diagnostic(
                if line_truncated {
                    "context_preview_lines"
                } else {
                    "context_preview_bytes"
                },
                if line_truncated {
                    "file exceeds the configured preview line limit; continue reading for more"
                } else {
                    "file exceeds the configured preview byte limit; continue reading for more"
                },
                Some(&result.path),
            ));
        }
        result.truncated = result.truncated || line_truncated;
        if offset == 0 && !result.truncated {
            result.content_hash = Some(hash_bytes(&content));
        }
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
        let actual_cwd = evidence_cwd(&source).ok_or_else(|| {
            InspectionError::new(
                "context_open_unavailable",
                "the source pane cwd is unavailable or unsafe",
            )
        })?;
        let root = find_root(&presentation.roots, &request.root_id)?;
        let launch_cwd = match root.root.kind {
            ContextRootKind::Companion
                if presentation.can_open_context
                    && companion_matches_checkout(&root, &actual_cwd) =>
            {
                actual_cwd.clone()
            }
            ContextRootKind::Folder
                if presentation.can_open_files
                    && presentation.files_root_id.as_deref() == Some(request.root_id.as_str()) =>
            {
                let resolved = resolve_viewer_root(&self.configuration, &actual_cwd)
                    .await
                    .ok_or_else(|| {
                        InspectionError::new(
                            "context_open_unavailable",
                            "the source pane cwd is unavailable or unsafe",
                        )
                    })?;
                if root.canonical != resolved {
                    return Err(InspectionError::new(
                        "context_open_unavailable",
                        "the source pane folder changed before files could open",
                    ));
                }
                root.canonical.clone()
            }
            _ => {
                return Err(InspectionError::new(
                    "context_open_unavailable",
                    "the requested root is not available from the current source pane",
                ));
            }
        };
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
                    cwd: launch_cwd.to_string_lossy().into_owned(),
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
                "Reviewr is unavailable for the current Git checkout",
            ));
        }
        let checkout = self
            .review_checkout_for_evidence(&presentation, &source, &request.repository_id)
            .await?;
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

    async fn review_checkout_for_evidence(
        &self,
        presentation: &PanePresentation,
        evidence: &ExtensionPaneEvidence,
        repository_id: &str,
    ) -> Result<PathBuf, InspectionError> {
        if presentation.extension == Some(ExtensionKind::Context)
            && presentation.renderer == Some(ExtensionKind::Context)
            && verified_confidence(presentation.confidence)
        {
            let root_id = presentation.default_root_id.as_deref().ok_or_else(|| {
                InspectionError::new(
                    "context_root_not_authorized",
                    "the file-viewer pane has no verified browsing root",
                )
            })?;
            let root = presentation
                .roots
                .iter()
                .find(|root| root.root_id == root_id)
                .filter(|root| {
                    root.kind == ContextRootKind::Companion
                        && root.repository_id == repository_id
                        && evidence
                            .viewer_cwd
                            .as_deref()
                            .and_then(evidence_path)
                            .as_deref()
                            == Some(Path::new(&root.path))
                })
                .ok_or_else(|| {
                    InspectionError::new(
                        "context_root_not_authorized",
                        "Review from Context requires its verified companion checkout",
                    )
                })?;
            let (checkout, _, _) =
                canonical_directory(Path::new(&root.checkout_path)).map_err(|_| {
                    InspectionError::new(
                        "review_open_unavailable",
                        "the Context companion checkout is unavailable",
                    )
                })?;
            return Ok(checkout);
        }
        let cwd = evidence_cwd(evidence).ok_or_else(|| {
            InspectionError::new(
                "review_open_unavailable",
                "the source pane cwd is unavailable or unsafe",
            )
        })?;
        let repository = RepositoryCatalog::new(self.configuration.clone())
            .discover_checkout(&cwd)
            .await
            .map_err(|_| {
                InspectionError::new(
                    "review_open_unavailable",
                    "the source pane cwd is not an available Git checkout",
                )
            })?;
        if repository.repository_id != repository_id {
            return Err(InspectionError::new(
                "context_root_not_authorized",
                "selected repository is not the source pane's current checkout",
            ));
        }
        Ok(PathBuf::from(repository.checkout_path))
    }

    async fn presentation(
        &self,
        session_id: &str,
        evidence: &ExtensionPaneEvidence,
    ) -> Result<PanePresentation, InspectionError> {
        let (
            roots,
            diagnostics,
            viewer_companion_id,
            viewer_folder_id,
            files_root_id,
            current_repository_id,
        ) = self.authorized_roots(session_id, evidence).await?;
        let mut roots: Vec<_> = roots.into_values().collect();
        roots.sort_by_key(|root| {
            (
                match root.root.kind {
                    ContextRootKind::Repository => 0u8,
                    ContextRootKind::Companion => 1u8,
                    ContextRootKind::Folder => 2u8,
                },
                root.root.root_id.clone(),
            )
        });
        let actual_cwd = evidence_cwd(evidence);
        let can_open_context = evidence.can_open_context
            && actual_cwd
                .as_ref()
                .and_then(|cwd| resolve_companion_for_checkout(cwd, &roots))
                .is_some();
        let can_open_files = evidence.can_open_context && files_root_id.is_some();
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
                if verified_confidence(confidence)
                    && (viewer_companion_id.is_some() || viewer_folder_id.is_some()) =>
            {
                Some(ExtensionKind::Context)
            }
            _ => None,
        };
        let source_companion_id = actual_cwd
            .as_deref()
            .and_then(|cwd| resolve_companion_for_checkout(cwd, &roots))
            .map(|root| root.root.root_id.clone());
        let source_repository_id = actual_cwd.as_deref().and_then(|cwd| {
            roots
                .iter()
                .filter(|root| {
                    root.root.kind == ContextRootKind::Repository && is_within(cwd, &root.canonical)
                })
                .max_by_key(|root| root.canonical.components().count())
                .map(|root| root.root.root_id.clone())
        });
        let default_root_id = if renderer == Some(ExtensionKind::Context) {
            viewer_companion_id.clone().or(viewer_folder_id)
        } else if can_open_context && renderer != Some(ExtensionKind::Review) {
            source_companion_id
                .or_else(|| {
                    current_repository_id.as_deref().and_then(|repository_id| {
                        roots
                            .iter()
                            .find(|root| {
                                root.root.kind == ContextRootKind::Repository
                                    && root.root.repository_id == repository_id
                            })
                            .map(|root| root.root.root_id.clone())
                    })
                })
                .or_else(|| roots.first().map(|root| root.root.root_id.clone()))
        } else {
            source_repository_id
                .or_else(|| {
                    current_repository_id.as_deref().and_then(|repository_id| {
                        roots
                            .iter()
                            .find(|root| {
                                root.root.kind == ContextRootKind::Repository
                                    && root.root.repository_id == repository_id
                            })
                            .map(|root| root.root.root_id.clone())
                    })
                })
                .or_else(|| roots.first().map(|root| root.root.root_id.clone()))
        };
        let mut reason = evidence.reason.clone();
        if evidence.extension == Some(ExtensionKind::Context)
            && renderer != Some(ExtensionKind::Context)
        {
            reason.push_str("; the viewer browsing root is unavailable or unsafe");
        }
        if evidence.can_open_context && !can_open_context {
            reason
                .push_str("; Open Context requires a source pane at a reviewed companion checkout");
        }
        if evidence.can_open_context && !can_open_files {
            reason.push_str("; Open files requires a safe source pane directory");
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
            can_open_files,
            files_root_id,
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
            Option<String>,
            Option<String>,
            Option<String>,
        ),
        InspectionError,
    > {
        let catalog = RepositoryCatalog::new(self.configuration.clone());
        let listed = catalog.list().await?;
        let mut diagnostics = listed.diagnostics;
        let actual_cwd = evidence_cwd(evidence);
        let mut repositories = listed.repositories;
        let mut current_repository_id = None;
        if let Some(cwd) = actual_cwd.as_deref() {
            if let Ok(candidate) = catalog.discover_checkout(cwd).await {
                current_repository_id = Some(candidate.repository_id.clone());
                repositories.push(candidate);
            }
        }
        let mut companion_roots = Vec::new();
        let mut companion_diagnostic = None;
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
                Err(error) => {
                    companion_diagnostic = Some(diagnostic(&error.code, &error.message, None))
                }
            }
        }
        let viewer_root = resolve_verified_viewer_root(&self.configuration, evidence).await;
        let viewer_companion = resolve_viewer_companion(viewer_root.as_deref(), &companion_roots);
        let verified_viewer = evidence.extension == Some(ExtensionKind::Context)
            && verified_confidence(evidence.confidence)
            && viewer_root.is_some();
        let viewer_folder = viewer_root
            .as_ref()
            .filter(|_| viewer_companion.is_none())
            .cloned();
        let source_folder = if !evidence.can_open_context || verified_viewer {
            None
        } else if let Some(cwd) = actual_cwd.as_deref() {
            resolve_viewer_root(&self.configuration, cwd).await
        } else {
            None
        };
        // Ordinary file browsing does not require task/companion discovery.
        // A non-Git folder is normal, not a failed Context setup.
        if let Some(folder) = viewer_folder.as_ref() {
            // Catalog discovery can encounter this ordinary browsing folder too.
            // Its lack of Git metadata is not a file-browser failure.
            diagnostics.retain(|item| {
                !(item.code == "repository_unavailable"
                    && item
                        .path
                        .as_deref()
                        .is_some_and(|path| Path::new(path) == folder))
            });
        } else {
            diagnostics.extend(companion_diagnostic);
        }
        let association_cwd = viewer_companion
            .as_ref()
            .map(|(_, path)| path.clone())
            .or_else(|| viewer_folder.clone())
            .or_else(|| if verified_viewer { None } else { actual_cwd });
        let viewer_companion_id = viewer_companion
            .as_ref()
            .map(|(root_id, _)| root_id.clone());
        let mut viewer_folder_id = None;
        let mut files_root_id = None;
        let mut roots = BTreeMap::new();
        for candidate in repositories {
            if viewer_folder.is_some() {
                continue;
            }
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
        if let Some((path, is_viewer_folder)) = viewer_folder
            .map(|path| (path, true))
            .or_else(|| source_folder.map(|path| (path, false)))
        {
            if let Ok((path, dir, metadata)) = canonical_directory(&path) {
                let identity = filesystem_identity(&metadata);
                let root_id = format!("folder:{identity}");
                let label = path
                    .file_name()
                    .and_then(|name| name.to_str())
                    .filter(|name| !name.is_empty())
                    .unwrap_or("Folder")
                    .to_owned();
                let root = ContextRoot {
                    root_id: root_id.clone(),
                    kind: ContextRootKind::Folder,
                    label,
                    path: path.to_string_lossy().into_owned(),
                    repository_id: format!("folder:{identity}"),
                    checkout_path: path.to_string_lossy().into_owned(),
                    companion_id: None,
                };
                roots.insert(
                    root_id.clone(),
                    AuthorizedRoot {
                        root,
                        canonical: path,
                        dir,
                        max_depth: self.configuration.limits.context_tree_depth,
                    },
                );
                if is_viewer_folder {
                    viewer_folder_id = Some(root_id);
                } else {
                    files_root_id = Some(root_id);
                }
            }
        }
        if viewer_folder_id.is_none() {
            for companion in companion_roots {
                let root_id = companion.root.root_id.clone();
                roots.insert(root_id, companion);
            }
        }
        Ok((
            roots,
            diagnostics,
            viewer_companion_id,
            viewer_folder_id,
            files_root_id,
            current_repository_id,
        ))
    }
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

    pub(crate) fn directory_revision(&self) -> Result<String, InspectionError> {
        self.dir
            .dir_metadata()
            .map(|metadata| metadata_revision(&metadata))
            .map_err(|error| InspectionError::new("context_root_unavailable", error.to_string()))
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
    let absolute = evidence_path(
        evidence
            .foreground_cwd
            .as_deref()
            .or(evidence.cwd.as_deref())?,
    )?;
    let dir = open_dir_nofollow_absolute(&absolute).ok()?;
    let metadata = dir.dir_metadata().ok()?;
    metadata.is_dir().then_some(absolute)
}

fn resolve_viewer_companion(
    viewer_root: Option<&Path>,
    companions: &[AuthorizedRoot],
) -> Option<(String, PathBuf)> {
    let viewer_root = viewer_root?;
    let mut matches = companions.iter().filter(|companion| {
        companion.canonical == viewer_root
            || companion_checkout(companion).as_deref() == Some(viewer_root)
    });
    let companion = matches.next()?;
    if matches.next().is_some() {
        return None;
    }
    Some((companion.root.root_id.clone(), companion.canonical.clone()))
}

fn companion_checkout(companion: &AuthorizedRoot) -> Option<PathBuf> {
    absolute_context_path(Path::new(&companion.root.checkout_path)).ok()
}

fn companion_matches_checkout(companion: &AuthorizedRoot, checkout: &Path) -> bool {
    companion.root.kind == ContextRootKind::Companion
        && (companion.canonical == checkout
            || companion_checkout(companion).as_deref() == Some(checkout))
}

fn resolve_companion_for_checkout<'a>(
    checkout: &Path,
    roots: &'a [AuthorizedRoot],
) -> Option<&'a AuthorizedRoot> {
    let mut matches = roots
        .iter()
        .filter(|companion| companion_matches_checkout(companion, checkout));
    let companion = matches.next()?;
    if matches.next().is_some() {
        return None;
    }
    Some(companion)
}

async fn resolve_verified_viewer_root(
    configuration: &ProjectConfiguration,
    evidence: &ExtensionPaneEvidence,
) -> Option<PathBuf> {
    if evidence.extension != Some(ExtensionKind::Context)
        || !verified_confidence(evidence.confidence)
    {
        return None;
    }
    let viewer_cwd = evidence.viewer_cwd.as_deref().and_then(evidence_path)?;
    resolve_viewer_root(configuration, &viewer_cwd).await
}

async fn resolve_viewer_root(configuration: &ProjectConfiguration, cwd: &Path) -> Option<PathBuf> {
    if git_metadata_path(cwd) {
        return None;
    }
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
    if git_metadata_path(&root) {
        return None;
    }
    let root_dir = open_dir_nofollow_absolute(&root).ok()?;
    root_dir.dir_metadata().ok()?.is_dir().then_some(root)
}

fn git_metadata_path(path: &Path) -> bool {
    path.components().any(|component| match component {
        Component::Normal(name) => name == ".git",
        _ => false,
    })
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
mod review_checkout_tests {
    use super::*;
    use std::collections::BTreeMap;
    use std::sync::Arc;

    use cockpit_protocol::{
        context::{ContextLaunchRequest, ContextSplitDirection},
        projects::{ProjectLimits, ProjectProvider},
        v1::{
            FocusRequest, FocusResponse, HerdrCompatibility, ResourceMutationRequest,
            ResourceMutationResponse, SessionListResponse, SessionSnapshotResponse,
            TerminalOpenRequest,
        },
    };
    use tokio::sync::Mutex;
    use uuid::Uuid;

    use crate::{
        HerdrAdapter, ProjectHerdrAdapter, SessionSubscription, TerminalSession,
        project_adapter::{
            ProjectInventory, ProjectTerminalRequest, ProjectTerminalResult,
            ProjectWorktreeRemoveRequest, ProjectWorktreeRequest, ProjectWorktreeResult,
        },
    };

    struct TestAdapter {
        evidence: ExtensionPaneEvidence,
        context_launches: Mutex<Vec<ExtensionLaunch>>,
        review_launches: Mutex<Vec<ExtensionLaunch>>,
    }

    fn unavailable<T>() -> Result<T, InspectionError> {
        Err(InspectionError::new(
            "test_adapter_unused",
            "test adapter method is not expected",
        ))
    }

    #[async_trait::async_trait]
    impl HerdrAdapter for TestAdapter {
        async fn inspect(&self) -> Result<HerdrCompatibility, InspectionError> {
            unavailable()
        }
        async fn inspect_session(&self, _: &str) -> Result<HerdrCompatibility, InspectionError> {
            unavailable()
        }
        async fn sessions(&self) -> Result<SessionListResponse, InspectionError> {
            unavailable()
        }
        async fn session_snapshot(
            &self,
            _: &str,
        ) -> Result<SessionSnapshotResponse, InspectionError> {
            unavailable()
        }
        async fn focus(&self, _: &str, _: &FocusRequest) -> Result<FocusResponse, InspectionError> {
            unavailable()
        }
        async fn mutate(
            &self,
            _: &str,
            _: &ResourceMutationRequest,
        ) -> Result<ResourceMutationResponse, InspectionError> {
            unavailable()
        }
        async fn subscribe_session(
            &self,
            _: &str,
            _: &SessionSnapshotResponse,
        ) -> Result<SessionSubscription, InspectionError> {
            unavailable()
        }
        async fn open_terminal(
            &self,
            _: &TerminalOpenRequest,
        ) -> Result<TerminalSession, InspectionError> {
            unavailable()
        }
    }

    #[async_trait::async_trait]
    impl ProjectHerdrAdapter for TestAdapter {
        async fn project_endpoint_identity(&self, _: &str) -> Result<String, InspectionError> {
            unavailable()
        }

        async fn project_inventory(
            &self,
            _: &str,
            _: &str,
        ) -> Result<ProjectInventory, InspectionError> {
            unavailable()
        }
        async fn project_worktree(
            &self,
            _: &str,
            _: &ProjectWorktreeRequest,
        ) -> Result<ProjectWorktreeResult, InspectionError> {
            unavailable()
        }
        async fn project_terminal(
            &self,
            _: &str,
            _: &ProjectTerminalRequest,
        ) -> Result<ProjectTerminalResult, InspectionError> {
            unavailable()
        }
        async fn project_worktree_dirty(
            &self,
            _: &str,
            _: u32,
            _: u32,
        ) -> Result<bool, InspectionError> {
            unavailable()
        }
        async fn project_close_workspace(
            &self,
            _: &str,
            _: &str,
            _: &str,
        ) -> Result<(), InspectionError> {
            unavailable()
        }
        async fn project_remove_worktree(
            &self,
            _: &str,
            _: &ProjectWorktreeRemoveRequest,
        ) -> Result<(), InspectionError> {
            unavailable()
        }
    }

    #[async_trait::async_trait]
    impl ExtensionHerdrAdapter for TestAdapter {
        async fn inspect_extension_pane(
            &self,
            _: &str,
            _: &str,
        ) -> Result<ExtensionPaneEvidence, InspectionError> {
            Ok(self.evidence.clone())
        }
        async fn launch_context_pane(
            &self,
            _: &str,
            request: &ExtensionLaunch,
        ) -> Result<ExtensionPaneEvidence, InspectionError> {
            self.context_launches.lock().await.push(request.clone());
            Ok(self.evidence.clone())
        }
        async fn launch_review_pane(
            &self,
            _: &str,
            request: &ExtensionLaunch,
        ) -> Result<ExtensionPaneEvidence, InspectionError> {
            self.review_launches.lock().await.push(request.clone());
            Ok(self.evidence.clone())
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
            providers: vec![ProjectProvider {
                id: "test".to_owned(),
                base_url: "https://example.test/".to_owned(),
                executable: "false".to_owned(),
                login: None,
            }],
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

    fn git_fixture(root: &Path) {
        std::fs::create_dir_all(root).expect("fixture directory");
        git(root, &["init"]);
        git(root, &["config", "user.email", "fixture@example.test"]);
        git(root, &["config", "user.name", "Fixture"]);
        std::fs::write(root.join("base.txt"), "base\n").expect("write base");
        git(root, &["add", "--all"]);
        git(root, &["commit", "-m", "base"]);
    }

    #[tokio::test]
    async fn opens_files_from_a_fresh_source_folder_root() {
        let workspace =
            std::env::temp_dir().join(format!("cockpit-context-files-{}", Uuid::new_v4()));
        let repository = workspace.join("repository");
        git_fixture(&repository);
        let source = repository.join("nested");
        std::fs::create_dir(&source).expect("source directory");
        let adapter = Arc::new(TestAdapter {
            evidence: ExtensionPaneEvidence {
                endpoint_identity: "endpoint".to_owned(),
                pane_id: "pane".to_owned(),
                terminal_id: "terminal".to_owned(),
                workspace_id: String::new(),
                tab_id: "tab".to_owned(),
                cwd: Some(source.to_string_lossy().into_owned()),
                foreground_cwd: Some(source.to_string_lossy().into_owned()),
                viewer_cwd: None,
                process_identity: "process".to_owned(),
                extension: None,
                confidence: DetectionConfidence::None,
                reason: "fixture".to_owned(),
                can_open_context: true,
                can_open_review: false,
            },
            context_launches: Mutex::new(Vec::new()),
            review_launches: Mutex::new(Vec::new()),
        });
        let configuration = configuration(&workspace);
        let projects = Arc::new(
            ProjectService::new(configuration.clone(), adapter.clone()).expect("project service"),
        );
        let context = ContextService::new(configuration, adapter.clone(), projects);

        let presentation = context
            .inspect_pane("session", "pane")
            .await
            .expect("source presentation");
        assert!(!presentation.can_open_context);
        assert!(presentation.can_open_files);
        let files_root_id = presentation
            .files_root_id
            .clone()
            .expect("source folder root");
        let files_root = presentation
            .roots
            .iter()
            .find(|root| root.root_id == files_root_id)
            .expect("files root");
        assert_eq!(files_root.kind, ContextRootKind::Folder);
        assert_eq!(files_root.path, repository.to_string_lossy());
        assert!(
            resolve_viewer_root(&context.configuration, &repository.join(".git"))
                .await
                .is_none(),
            "Git metadata cannot become a Folder root"
        );
        assert_eq!(
            presentation
                .roots
                .iter()
                .find(|root| root.root_id == presentation.default_root_id.as_deref().unwrap())
                .expect("default root")
                .kind,
            ContextRootKind::Repository,
            "the original Context default root remains the repository"
        );

        let media_root = context
            .authorize_media_root("session", "pane", &presentation.binding_id, &files_root_id)
            .await
            .expect("Folder root is authorized for bounded media reads");
        assert_eq!(media_root.root.kind, ContextRootKind::Folder);
        let repository_root_id = presentation
            .roots
            .iter()
            .find(|root| root.kind == ContextRootKind::Repository)
            .expect("repository root")
            .root_id
            .clone();
        let media_error = match context
            .authorize_media_root(
                "session",
                "pane",
                &presentation.binding_id,
                &repository_root_id,
            )
            .await
        {
            Ok(_) => panic!("Repository root cannot authorize media"),
            Err(error) => error,
        };
        assert_eq!(media_error.code, "context_root_not_media");

        context
            .open(
                "session",
                &ContextLaunchRequest {
                    pane_id: "pane".to_owned(),
                    binding_id: presentation.binding_id.clone(),
                    root_id: files_root_id.clone(),
                    direction: ContextSplitDirection::Right,
                },
            )
            .await
            .expect("open files");
        let launches = adapter.context_launches.lock().await;
        assert_eq!(launches.len(), 1);
        assert_eq!(launches[0].endpoint_identity, "endpoint");
        assert_eq!(launches[0].pane_id, "pane");
        assert_eq!(launches[0].terminal_id, "terminal");
        assert_eq!(launches[0].cwd, repository.to_string_lossy());
        drop(launches);

        assert_eq!(
            context
                .open(
                    "session",
                    &ContextLaunchRequest {
                        pane_id: "pane".to_owned(),
                        binding_id: presentation.binding_id,
                        root_id: repository_root_id,
                        direction: ContextSplitDirection::Down,
                    },
                )
                .await
                .expect_err("repository root is not an Open files target")
                .code,
            "context_open_unavailable"
        );
        assert_eq!(adapter.context_launches.lock().await.len(), 1);
        std::fs::remove_dir_all(workspace).expect("cleanup");
    }

    #[tokio::test]
    async fn review_uses_the_foreground_nested_checkout_and_refuses_its_parent() {
        let workspace =
            std::env::temp_dir().join(format!("cockpit-context-review-{}", Uuid::new_v4()));
        let parent = workspace.join("parent");
        git_fixture(&parent);
        let child = parent.join("child");
        git_fixture(&child);
        let non_git_cwd = workspace.join("ordinary-folder");
        std::fs::create_dir(&non_git_cwd).expect("ordinary directory");
        let adapter = Arc::new(TestAdapter {
            evidence: ExtensionPaneEvidence {
                endpoint_identity: String::new(),
                pane_id: "pane".to_owned(),
                terminal_id: "terminal".to_owned(),
                workspace_id: String::new(),
                tab_id: "tab".to_owned(),
                cwd: Some(non_git_cwd.to_string_lossy().into_owned()),
                foreground_cwd: Some(child.to_string_lossy().into_owned()),
                viewer_cwd: None,
                process_identity: "process".to_owned(),
                extension: None,
                confidence: DetectionConfidence::None,
                reason: "fixture".to_owned(),
                can_open_context: false,
                can_open_review: true,
            },
            context_launches: Mutex::new(Vec::new()),
            review_launches: Mutex::new(Vec::new()),
        });
        let configuration = configuration(&workspace);
        let projects = Arc::new(
            ProjectService::new(configuration.clone(), adapter.clone()).expect("project service"),
        );
        let context = ContextService::new(configuration.clone(), adapter.clone(), projects);
        let catalog = RepositoryCatalog::new(configuration);
        let listed = catalog.list().await.expect("catalog listing");
        let parent_id = listed
            .repositories
            .iter()
            .find(|candidate| candidate.checkout_path == parent.to_string_lossy())
            .expect("parent checkout")
            .repository_id
            .clone();
        let child_id = listed
            .repositories
            .iter()
            .find(|candidate| candidate.checkout_path == child.to_string_lossy())
            .expect("child checkout")
            .repository_id
            .clone();

        let presentation = context
            .inspect_pane("session", "pane")
            .await
            .expect("presentation");
        assert!(presentation.can_open_review);
        let default = presentation
            .default_root_id
            .as_deref()
            .expect("current checkout root");
        assert_eq!(
            presentation
                .roots
                .iter()
                .find(|root| root.root_id == default)
                .expect("default root")
                .repository_id,
            child_id
        );

        let child_request = ReviewLaunchRequest {
            pane_id: "pane".to_owned(),
            binding_id: presentation.binding_id.clone(),
            repository_id: child_id,
            direction: ContextSplitDirection::Right,
        };
        context
            .open_review("session", &child_request)
            .await
            .expect("foreground checkout launch");
        assert_eq!(
            adapter.review_launches.lock().await[0].cwd,
            child.to_string_lossy()
        );

        let parent_request = ReviewLaunchRequest {
            repository_id: parent_id,
            ..child_request
        };
        assert_eq!(
            context
                .open_review("session", &parent_request)
                .await
                .expect_err("parent selection is refused")
                .code,
            "context_root_not_authorized"
        );

        // A verified companion remains a valid task-context launch source,
        // even though its own directory is intentionally outside Git.
        let companion = workspace.join("companion");
        std::fs::create_dir(&companion).unwrap();
        let mut companion_evidence = adapter.evidence.clone();
        companion_evidence.cwd = Some(companion.to_string_lossy().into_owned());
        companion_evidence.foreground_cwd = companion_evidence.cwd.clone();
        companion_evidence.viewer_cwd = companion_evidence.cwd.clone();
        let mut companion_presentation = presentation.clone();
        companion_presentation.extension = Some(ExtensionKind::Context);
        companion_presentation.renderer = Some(ExtensionKind::Context);
        companion_presentation.confidence = DetectionConfidence::VerifiedProcess;
        companion_presentation.default_root_id = Some("companion".into());
        let mut associated = presentation
            .roots
            .iter()
            .find(|root| root.root_id == default)
            .unwrap()
            .clone();
        associated.root_id = "companion".into();
        associated.kind = ContextRootKind::Companion;
        associated.path = companion.to_string_lossy().into_owned();
        let associated_id = associated.repository_id.clone();
        companion_presentation.roots = vec![associated];
        assert_eq!(
            context
                .review_checkout_for_evidence(
                    &companion_presentation,
                    &companion_evidence,
                    &associated_id
                )
                .await
                .unwrap(),
            child
        );
        assert!(
            context
                .review_checkout_for_evidence(
                    &companion_presentation,
                    &companion_evidence,
                    "unrelated"
                )
                .await
                .is_err()
        );
        std::fs::remove_dir_all(workspace).expect("cleanup");
    }

    #[tokio::test]
    async fn verified_viewer_reads_an_ordinary_folder_from_its_plugin_context() {
        let workspace =
            std::env::temp_dir().join(format!("cockpit-context-folder-{}", Uuid::new_v4()));
        let catalog = workspace.join("catalog");
        git_fixture(&catalog);
        let plugin_install = workspace.join("plugin-install");
        std::fs::create_dir_all(&plugin_install).expect("plugin install directory");
        let folder = workspace.join("ordinary-folder");
        std::fs::create_dir_all(folder.join(".git")).expect("ordinary folder");
        std::fs::write(folder.join("notes.md"), "ordinary context\n").expect("write context");
        let adapter = Arc::new(TestAdapter {
            evidence: ExtensionPaneEvidence {
                endpoint_identity: String::new(),
                pane_id: "pane".to_owned(),
                terminal_id: "terminal".to_owned(),
                workspace_id: String::new(),
                tab_id: "tab".to_owned(),
                cwd: Some(plugin_install.to_string_lossy().into_owned()),
                foreground_cwd: Some(plugin_install.to_string_lossy().into_owned()),
                viewer_cwd: Some(folder.to_string_lossy().into_owned()),
                process_identity: "process".to_owned(),
                extension: Some(ExtensionKind::Context),
                confidence: DetectionConfidence::VerifiedProcess,
                reason: "verified viewer".to_owned(),
                can_open_context: false,
                can_open_review: false,
            },
            context_launches: Mutex::new(Vec::new()),
            review_launches: Mutex::new(Vec::new()),
        });
        let configuration = configuration(&catalog);
        let projects = Arc::new(
            ProjectService::new(configuration.clone(), adapter.clone()).expect("project service"),
        );
        let context = Arc::new(ContextService::new(
            configuration.clone(),
            adapter,
            projects,
        ));

        let presentation = context
            .inspect_pane("session", "pane")
            .await
            .expect("folder presentation");
        assert_eq!(presentation.renderer, Some(ExtensionKind::Context));
        let root_id = presentation.default_root_id.clone().expect("folder root");
        let root = presentation
            .roots
            .iter()
            .find(|root| root.root_id == root_id)
            .expect("selected root");
        assert_eq!(root.kind, ContextRootKind::Folder);
        assert_eq!(
            presentation.roots.len(),
            1,
            "ordinary viewers expose only their browsing folder"
        );
        assert_eq!(root.path, folder.to_string_lossy().as_ref());
        assert_ne!(root.path, plugin_install.to_string_lossy().as_ref());

        let directory = context
            .directory(
                "session",
                "pane",
                &ContextDirectoryRequest {
                    binding_id: presentation.binding_id.clone(),
                    root_id: root.root_id.clone(),
                    path: String::new(),
                    offset: None,
                    revision: None,
                },
            )
            .await
            .expect("ordinary folder directory");
        assert_eq!(
            directory
                .entries
                .iter()
                .map(|entry| entry.name.as_str())
                .collect::<Vec<_>>(),
            vec!["notes.md"]
        );
        let document = context
            .document(
                "session",
                "pane",
                &ContextDocumentRequest {
                    binding_id: presentation.binding_id.clone(),
                    root_id: root.root_id.clone(),
                    path: "notes.md".to_owned(),
                    expected_revision: None,
                    offset: None,
                },
            )
            .await
            .expect("ordinary folder document");
        assert_eq!(document.text.as_deref(), Some("ordinary context\n"));
        std::fs::write(
            folder.join("large.txt"),
            vec![b'x'; configuration.limits.context_preview_bytes as usize + 17],
        )
        .expect("write large context source");
        let first_page = context
            .document(
                "session",
                "pane",
                &ContextDocumentRequest {
                    binding_id: presentation.binding_id.clone(),
                    root_id: root.root_id.clone(),
                    path: "large.txt".to_owned(),
                    expected_revision: None,
                    offset: None,
                },
            )
            .await
            .expect("large first page");
        assert!(first_page.truncated);
        assert_eq!(
            first_page.text.as_deref().map(str::len),
            Some(configuration.limits.context_preview_bytes as usize)
        );
        let second_page = context
            .document(
                "session",
                "pane",
                &ContextDocumentRequest {
                    binding_id: presentation.binding_id.clone(),
                    root_id: root.root_id.clone(),
                    path: "large.txt".to_owned(),
                    expected_revision: Some(first_page.revision.clone()),
                    offset: first_page.next_offset,
                },
            )
            .await
            .expect("large continuation page");
        assert!(!second_page.truncated);
        assert_eq!(second_page.text.as_deref(), Some("xxxxxxxxxxxxxxxxx"));
        for index in 0..65 {
            std::fs::write(folder.join(format!("entry-{index}.txt")), "entry")
                .expect("write directory entry");
        }
        let first_directory_page = context
            .directory(
                "session",
                "pane",
                &ContextDirectoryRequest {
                    binding_id: presentation.binding_id.clone(),
                    root_id: root.root_id.clone(),
                    path: String::new(),
                    offset: None,
                    revision: None,
                },
            )
            .await
            .expect("first directory page");
        assert_eq!(first_directory_page.entries.len(), 64);
        let second_directory_page = context
            .directory(
                "session",
                "pane",
                &ContextDirectoryRequest {
                    binding_id: presentation.binding_id.clone(),
                    root_id: root.root_id.clone(),
                    path: String::new(),
                    offset: first_directory_page.next_offset,
                    revision: first_directory_page.revision.clone(),
                },
            )
            .await
            .expect("second directory page");
        assert!(!second_directory_page.entries.is_empty());
        assert!(second_directory_page.entries.len() < 64);
        let comments = crate::comments::CommentsService::new(configuration, context.clone())
            .expect("comments");
        let scope = cockpit_protocol::comments::CommentRequestScope {
            binding_id: presentation.binding_id.clone(),
            client_id: "client".to_owned(),
        };
        let batch = comments
            .batch(
                "session",
                "pane",
                &cockpit_protocol::comments::CommentBatchRequest {
                    scope: scope.clone(),
                    batch_id: None,
                },
            )
            .await
            .expect("ordinary folder comments");
        let request = cockpit_protocol::comments::CommentUpsertRequest {
            batch: cockpit_protocol::comments::CommentBatchMutation {
                scope: scope.clone(),
                batch_id: batch.batch_id.clone(),
                expected_generation: batch.generation,
            },
            draft_id: None,
            capture: Some(cockpit_protocol::comments::CommentCapture {
                root_id: root_id.clone(),
                path: "notes.md".to_owned(),
                expected_revision: document.revision.clone(),
                start_line: Some(1),
                end_line: Some(1),
                review: None,
            }),
            comment_text: "Folder comment".to_owned(),
        };
        let saved = comments
            .upsert("session", "pane", &request)
            .await
            .expect("capture ordinary folder source");
        assert_eq!(
            saved.drafts[0].file_ref.absolute_path,
            folder.join("notes.md").to_string_lossy()
        );
        assert_eq!(saved.owner.source_id, root_id);
        let mut outside = request.clone();
        outside.batch.expected_generation = saved.generation;
        outside.capture.as_mut().unwrap().path = "../plugin-install/outside.md".to_owned();
        assert!(
            comments.upsert("session", "pane", &outside).await.is_err(),
            "comments preserve bounded reads"
        );
        std::fs::rename(&folder, workspace.join("old-folder")).expect("replace root");
        std::fs::create_dir(&folder).expect("new root");
        std::fs::write(folder.join("notes.md"), "ordinary context\n").expect("same text new root");
        outside.capture = request.capture.clone();
        assert!(
            comments.upsert("session", "pane", &outside).await.is_err(),
            "old batch cannot attach to replacement root"
        );
        std::fs::remove_dir_all(workspace).expect("cleanup");
    }

    #[tokio::test]
    async fn verified_viewer_refuses_an_unsafe_or_replaced_folder_root() {
        let workspace =
            std::env::temp_dir().join(format!("cockpit-context-folder-{}", Uuid::new_v4()));
        let plugin_install = workspace.join("plugin-install");
        let target = workspace.join("target");
        std::fs::create_dir_all(&plugin_install).expect("plugin install directory");
        std::fs::create_dir_all(&target).expect("target directory");
        let linked = workspace.join("linked-folder");
        #[cfg(unix)]
        std::os::unix::fs::symlink(&target, &linked).expect("folder symlink");
        let adapter = Arc::new(TestAdapter {
            evidence: ExtensionPaneEvidence {
                endpoint_identity: String::new(),
                pane_id: "pane".to_owned(),
                terminal_id: "terminal".to_owned(),
                workspace_id: String::new(),
                tab_id: "tab".to_owned(),
                cwd: Some(plugin_install.to_string_lossy().into_owned()),
                foreground_cwd: Some(plugin_install.to_string_lossy().into_owned()),
                viewer_cwd: Some(linked.to_string_lossy().into_owned()),
                process_identity: "process".to_owned(),
                extension: Some(ExtensionKind::Context),
                confidence: DetectionConfidence::VerifiedProcess,
                reason: "verified viewer".to_owned(),
                can_open_context: false,
                can_open_review: false,
            },
            context_launches: Mutex::new(Vec::new()),
            review_launches: Mutex::new(Vec::new()),
        });
        let configuration = configuration(&workspace);
        let projects = Arc::new(
            ProjectService::new(configuration.clone(), adapter.clone()).expect("project service"),
        );
        let context = ContextService::new(configuration, adapter, projects);

        let presentation = context
            .inspect_pane("session", "pane")
            .await
            .expect("unsafe presentation");
        assert_ne!(presentation.renderer, Some(ExtensionKind::Context));
        assert!(
            presentation
                .roots
                .iter()
                .all(|root| root.path != plugin_install.to_string_lossy())
        );

        let original = workspace.join("original-folder");
        std::fs::create_dir(&original).expect("original folder");
        let (_, _, original_metadata) = canonical_directory(&original).expect("original root");
        let stale_root = ContextRoot {
            root_id: format!("folder:{}", filesystem_identity(&original_metadata)),
            kind: ContextRootKind::Folder,
            label: "original-folder".to_owned(),
            path: original.to_string_lossy().into_owned(),
            repository_id: String::new(),
            checkout_path: original.to_string_lossy().into_owned(),
            companion_id: None,
        };
        let stale_root_id = stale_root.root_id.clone();
        std::fs::rename(&original, workspace.join("renamed-folder")).expect("rename folder");
        std::fs::create_dir(&original).expect("replacement folder");
        let error = match find_root(&[stale_root], &stale_root_id) {
            Ok(_) => panic!("replacement root is refused"),
            Err(error) => error,
        };
        assert_eq!(error.code, "context_stale_root");
        std::fs::remove_dir_all(workspace).expect("cleanup");
    }

    #[test]
    fn companion_checkout_match_is_exact_and_unambiguous() {
        let workspace =
            std::env::temp_dir().join(format!("cockpit-context-association-{}", Uuid::new_v4()));
        let checkout = workspace.join("checkout");
        let companion_path = workspace.join("companion");
        std::fs::create_dir_all(&checkout).expect("checkout directory");
        std::fs::create_dir_all(&companion_path).expect("companion directory");
        let (canonical, dir, _) = canonical_directory(&companion_path).expect("companion root");
        let companion = AuthorizedRoot {
            root: ContextRoot {
                root_id: "companion:one".to_owned(),
                kind: ContextRootKind::Companion,
                label: "Context".to_owned(),
                path: canonical.to_string_lossy().into_owned(),
                repository_id: "repository".to_owned(),
                checkout_path: checkout.to_string_lossy().into_owned(),
                companion_id: Some("one".to_owned()),
            },
            canonical,
            dir,
            max_depth: 8,
        };
        assert!(companion_matches_checkout(&companion, &checkout));
        assert_eq!(
            resolve_companion_for_checkout(&checkout, std::slice::from_ref(&companion))
                .map(|root| root.root.root_id.as_str()),
            Some("companion:one")
        );
        assert_eq!(
            resolve_viewer_companion(Some(&checkout), std::slice::from_ref(&companion))
                .map(|(root_id, _)| root_id),
            Some("companion:one".to_owned())
        );

        let second_path = workspace.join("second-companion");
        std::fs::create_dir_all(&second_path).expect("second companion directory");
        let (second_canonical, second_dir, _) =
            canonical_directory(&second_path).expect("second companion root");
        let second = AuthorizedRoot {
            root: ContextRoot {
                root_id: "companion:two".to_owned(),
                kind: ContextRootKind::Companion,
                label: "Context".to_owned(),
                path: second_canonical.to_string_lossy().into_owned(),
                repository_id: "repository".to_owned(),
                checkout_path: checkout.to_string_lossy().into_owned(),
                companion_id: Some("two".to_owned()),
            },
            canonical: second_canonical,
            dir: second_dir,
            max_depth: 8,
        };
        assert!(
            resolve_companion_for_checkout(&checkout, &[companion, second]).is_none(),
            "ambiguous checkout associations must not authorize Context"
        );
        std::fs::remove_dir_all(workspace).expect("cleanup");
    }
}
