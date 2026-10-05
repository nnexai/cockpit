use std::io::{ErrorKind, Read, Seek, SeekFrom};
use std::path::{Component, Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use cap_fs_ext::{DirExt, OpenOptionsFollowExt, OpenOptionsSyncExt};
use cap_std::fs::{Dir, Metadata, OpenOptions};
use cockpit_protocol::context::{
    ContextDirectory, ContextDirectoryRequest, ContextDocument, ContextDocumentRequest,
    ContextEntry, ContextEntryKind, ContextFileIndex, ContextFileIndexMode, ContextFileIndexRequest,
    ContextFileIndexSource, ContextFileIndexState, ContextIndexedFile,
    ContextRoot, ContextRootKind,
};
use cockpit_protocol::projects::{ProjectConfiguration, ProjectDiagnostic};
use sha2::{Digest, Sha256};
use tokio::process::Command;
use tokio::sync::Semaphore;

use crate::InspectionError;
use crate::extension_adapter::{SourcePaneAdapter, SourcePaneEvidence};
use crate::projects::ProjectService;
const MAX_DIRECTORY_SCAN: usize = 100_000;

#[derive(Clone)]
pub struct ContextService {
    pub(crate) configuration: ProjectConfiguration,
    pub(crate) adapter: Arc<dyn SourcePaneAdapter>,
    projects: Arc<ProjectService>,
    library: Option<Arc<crate::library::LibraryService>>,
    viewers: Option<Arc<crate::viewer::ViewerService>>,
    /// Shared across transient host handlers so bounded searches cannot exhaust blocking workers.
    pub(crate) search_permits: Arc<Semaphore>,
}

/// Fresh, internal proof used by the durable reference-comment service.
///
/// The proof intentionally contains only identity and the authorized browsing
/// root. Callers cannot provide any of these values as authority.
#[derive(Debug, Clone)]
pub(crate) struct ContextCommentEvidence {
    pub binding_id: String,
    pub server_instance: String,
    pub terminal_id: String,
    pub workspace_id: String,
    pub tab_id: String,
    pub root_id: String,
    pub root_path: String,
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
        adapter: Arc<dyn SourcePaneAdapter>,
        projects: Arc<ProjectService>,
    ) -> Self {
        Self {
            configuration,
            adapter,
            projects,
            library: None,
            viewers: None,
            search_permits: Arc::new(Semaphore::new(2)),
        }
    }
    pub fn with_viewers(mut self, viewers: Arc<crate::viewer::ViewerService>) -> Self {
        self.viewers = Some(viewers);
        self
    }

    pub fn with_library(mut self, library: Arc<crate::library::LibraryService>) -> Self {
        self.library = Some(library);
        self
    }

    pub(crate) async fn authorize_viewer(
        &self, session_id: &str, viewer_id: &str, binding_id: &str,
    ) -> Result<crate::viewer::ViewerAuthorization, InspectionError> {
        self.viewers.as_ref().ok_or_else(|| InspectionError::new(
            "viewer_not_found", "viewer service is unavailable",
        ))?.authorize(session_id, viewer_id, binding_id).await
    }

    pub(crate) async fn comment_evidence(
        &self, session_id: &str, viewer_id: &str, binding_id: &str,
    ) -> Result<ContextCommentEvidence, InspectionError> {
        let authorization = self.authorize_viewer(session_id, viewer_id, binding_id).await?;
        let context = &authorization.context;
        if context.kind != cockpit_protocol::viewer::ViewerKind::Files {
            return Err(InspectionError::new("comments_detached", "comments require a Files viewer"));
        }
        let root_id = context.default_root_id.as_deref().ok_or_else(||
            InspectionError::new("comments_detached", "viewer has no browsing root"))?;
        let root = context.roots.iter().find(|root| root.root_id == root_id)
            .ok_or_else(|| InspectionError::new("comments_detached", "viewer browsing root is unavailable"))?;
        Ok(ContextCommentEvidence {
            binding_id: context.binding_id.clone(),
            server_instance: authorization.server_instance,
            terminal_id: authorization.source.terminal_id,
            workspace_id: context.space_id.clone(),
            tab_id: context.tab_id.clone(),
            root_id: root.root_id.clone(),
            root_path: root.path.clone(),
        })
    }

    pub(crate) async fn authorize_files_root(
        &self, session_id: &str, viewer_id: &str, binding_id: &str, root_id: &str,
    ) -> Result<AuthorizedRoot, InspectionError> {
        let authorization = self.authorize_viewer(session_id, viewer_id, binding_id).await?;
        let mut authorized = find_root(&authorization.context.roots, root_id)?;
        if authorization.context.kind != cockpit_protocol::viewer::ViewerKind::Files {
            return Err(InspectionError::new("context_root_not_files", "bounded reads require a Files viewer"));
        }
        authorized.max_depth = self.configuration.limits.context_tree_depth;
        Ok(authorized)
    }

    pub(crate) async fn authorize_media_root(
        &self, session_id: &str, viewer_id: &str, binding_id: &str, root_id: &str,
    ) -> Result<AuthorizedRoot, InspectionError> {
        self.authorize_files_root(session_id, viewer_id, binding_id, root_id).await
    }


    pub async fn directory(
        &self,
        session_id: &str,
        pane_id: &str,
        request: &ContextDirectoryRequest,
    ) -> Result<ContextDirectory, InspectionError> {
        let presentation = self.authorize_viewer(session_id, pane_id, &request.binding_id).await?.context;
        let authorized = find_root(&presentation.roots, &request.root_id)?;
        let mut result = read_directory(&authorized, request, &self.configuration.limits)?;
        result.diagnostics = presentation.diagnostics;
        Ok(result)
    }

    pub async fn document(
        &self,
        session_id: &str,
        pane_id: &str,
        request: &ContextDocumentRequest,
    ) -> Result<ContextDocument, InspectionError> {
        let presentation = self.authorize_viewer(session_id, pane_id, &request.binding_id).await?.context;
        let authorized = find_root(&presentation.roots, &request.root_id)?;
        read_document(&authorized, request, &self.configuration.limits)
    }

    pub async fn file_index(
        &self,
        session_id: &str,
        pane_id: &str,
        request: &ContextFileIndexRequest,
    ) -> Result<ContextFileIndex, InspectionError> {
        let presentation = self.authorize_viewer(session_id, pane_id, &request.binding_id).await?.context;
        let authorized = find_root(&presentation.roots, &request.root_id)?;
        if request.mode == ContextFileIndexMode::Cached {
            let canonical = authorized.canonical.clone();
            let kind = authorized.root.kind;
            let binding_id = request.binding_id.clone();
            let root_id = request.root_id.clone();
            let cache_root = PathBuf::from(&self.configuration.cache_root);
            let diagnostics = presentation.diagnostics;
            return tokio::task::spawn_blocking(move || {
                let cached = crate::file_index_cache::load(&canonical, kind, &cache_root);
                Ok::<_, InspectionError>(match cached {
                    Some(cached) => ContextFileIndex {
                        binding_id,
                        root_id,
                        files: cached.files,
                        truncated: cached.truncated,
                        source: cached.source,
                        state: ContextFileIndexState::Cached,
                        diagnostics,
                    },
                    None => ContextFileIndex {
                        binding_id,
                        root_id,
                        files: Vec::new(),
                        truncated: false,
                        source: ContextFileIndexSource::Walk,
                        state: ContextFileIndexState::Miss,
                        diagnostics,
                    },
                })
            })
            .await
            .map_err(|error| InspectionError::new("context_file_index", error.to_string()))?;
        }
        let git_root = git_root_matches(&self.configuration, &authorized.canonical, authorized.root.kind).await?;
        let git_output = if git_root {
            Some(git_file_list(&self.configuration, &authorized.canonical).await?)
        } else {
            None
        };
        let source = if git_root { ContextFileIndexSource::Git } else { ContextFileIndexSource::Walk };
        let root_path = authorized.canonical.clone();
        let kind = authorized.root.kind;
        let cache_root = PathBuf::from(&self.configuration.cache_root);
        let files = tokio::task::spawn_blocking(move || {
            authorized.revalidate()?;
            let result = enumerate_file_index(&authorized, git_root, git_output)?;
            crate::file_index_cache::store(&root_path, kind, source, result.1, result.0.clone(), &cache_root);
            Ok::<_, InspectionError>(result)
        })
        .await
        .map_err(|error| InspectionError::new("context_file_index", error.to_string()))??;
        Ok(ContextFileIndex {
            binding_id: request.binding_id.clone(),
            root_id: request.root_id.clone(),
            files: files.0,
            truncated: files.1,
            source,
            state: ContextFileIndexState::Fresh,
            diagnostics: presentation.diagnostics,
        })
    }
    /// Recompute roots from the source evidence pinned when a viewer opened.
    /// Plugin installation and process identity never grant authority.
    pub(crate) async fn source_options(
        &self, session_id: &str, evidence: &SourcePaneEvidence,
    ) -> Result<cockpit_protocol::viewer::ViewerSourceOptions, InspectionError> {
        let cwd = evidence_cwd(evidence);
        let mut roots = Vec::new();
        let mut diagnostics = Vec::new();
        let mut files_context_root_id = None;
        let mut selected_repositories = Vec::new();
        if let Some(library) = &self.library {
            let target = cockpit_protocol::library::SpaceTarget {
                session_id: session_id.to_owned(),
                space_id: evidence.workspace_id.clone(),
            };
            match library.space_listing(&target).await {
                Ok(listing) => {
                    diagnostics.extend(listing.diagnostics);
                    match canonical_directory(Path::new(library.root_path())) {
                        Ok((path, dir, _)) => {
                            let root = AuthorizedRoot::library(path, dir, self.configuration.limits.context_tree_depth)?.summary();
                            files_context_root_id = Some(root.root_id.clone());
                            roots.push(root);
                        }
                        Err(error) => diagnostics.push(diagnostic(&error.code, &error.message, None)),
                    }
                    selected_repositories = listing.repository_paths;
                }
                Err(error) => diagnostics.push(diagnostic(&error.code, &error.message, None)),
            }
        }
        let mut review_repository_ids = Vec::new();
        if let Some(cwd) = &cwd {
            if let Ok(repository) = self.projects.cached_discover_checkout(cwd).await {
                if let Ok(root) = verified_repository_root(
                    &self.configuration, Path::new(&repository.checkout_path), repository.repository_id, repository.name,
                ).await {
                    review_repository_ids.push(root.repository_id.clone());
                    roots.push(root);
                }
            }
        }
        let selected_catalog = if selected_repositories.is_empty() {
            Vec::new()
        } else {
            match self.projects.repositories().await {
                Ok(listing) => {
                    diagnostics.extend(listing.diagnostics);
                    listing.repositories
                }
                Err(error) => {
                    diagnostics.push(diagnostic(&error.code, &error.message, None));
                    Vec::new()
                }
            }
        };
        for repository_path in selected_repositories {
            if roots.iter().any(|root| root.kind == ContextRootKind::Repository && root.path == repository_path) {
                continue;
            }
            let Some(repository) = selected_catalog.iter().find(|repository| repository.checkout_path == repository_path) else {
                diagnostics.push(diagnostic("context_root_not_authorized", "selected repository is no longer in the configured catalog", Some(&repository_path)));
                continue;
            };
            match verified_repository_root(&self.configuration, Path::new(&repository_path), repository.repository_id.clone(), repository.name.clone()).await {
                Ok(root) => roots.push(root),
                Err(error) => diagnostics.push(diagnostic(&error.code, &error.message, Some(&repository_path))),
            }
        }
        let folder = match &cwd {
            Some(cwd) => resolve_viewer_root(&self.configuration, cwd).await,
            None => None,
        };
        let files_folder_root_id = if let Some(folder) = folder {
            if let Ok((folder, _, metadata)) = canonical_directory(&folder) {
                let root_id = format!("folder:{}", filesystem_identity(&metadata));
                roots.push(ContextRoot {
                    root_id: root_id.clone(), kind: ContextRootKind::Folder,
                    label: folder.file_name().and_then(|name| name.to_str()).unwrap_or("Folder").to_owned(),
                    path: folder.to_string_lossy().into_owned(), repository_id: root_id.clone(),
                    checkout_path: folder.to_string_lossy().into_owned(),
                });
                Some(root_id)
            } else { None }
        } else { None };
        roots.sort_by(|a,b| a.root_id.cmp(&b.root_id));
        let confirmed = self.adapter.tab_evidence(session_id, &evidence.tab_id).await?;
        if !confirmed.present || confirmed.endpoint_identity != evidence.endpoint_identity || confirmed.workspace_id != evidence.workspace_id {
            return Err(InspectionError::new("viewer_tab_absent", "viewer tab or server identity is no longer current"));
        }
        Ok(cockpit_protocol::viewer::ViewerSourceOptions {
            session_id: session_id.to_owned(), pane_id: evidence.pane_id.clone(),
            tab_id: evidence.tab_id.clone(), space_id: evidence.workspace_id.clone(),
            files_context_root_id, files_folder_root_id, review_repository_ids, roots,
            reason: "Files use the full Library, selected repositories, or this terminal's verified folder".to_owned(), diagnostics,
        })
    }

    pub(crate) async fn cached_discover_checkout(
        &self, cwd: &Path,
    ) -> Result<cockpit_protocol::projects::RepositoryCandidate, InspectionError> {
        self.projects.cached_discover_checkout(cwd).await
    }
}

async fn verified_repository_root(
    configuration: &ProjectConfiguration, path: &Path, repository_id: String, label: String,
) -> Result<ContextRoot, InspectionError> {
    if git_metadata_path(path) {
        return Err(InspectionError::new("context_root_unavailable", "Git metadata cannot be a repository root"));
    }
    let (path, dir, metadata) = canonical_directory(path)?;
    let git_metadata = dir.symlink_metadata(".git")
        .map_err(|error| InspectionError::new("context_root_unavailable", error.to_string()))?;
    if git_metadata.file_type().is_symlink() || (!git_metadata.is_dir() && !git_metadata.is_file()) {
        return Err(InspectionError::new("context_root_unavailable", "Git metadata must not be a symbolic link"));
    }
    git_root_matches(configuration, &path, ContextRootKind::Repository).await?;
    let (_, _, current) = canonical_directory(&path)?;
    if filesystem_identity(&metadata) != filesystem_identity(&current) {
        return Err(InspectionError::new("context_stale_root", "repository filesystem identity changed"));
    }
    Ok(ContextRoot {
        root_id: format!("repository:{}:{}", repository_id, filesystem_identity(&metadata)),
        kind: ContextRootKind::Repository, label,
        path: path.to_string_lossy().into_owned(), repository_id,
        checkout_path: path.to_string_lossy().into_owned(),
    })
}

const MAX_FILE_INDEX_FILES: usize = 50_000;
const MAX_FILE_INDEX_GIT_BYTES: usize = 16 * 1024 * 1024;

async fn git_root_matches(
    configuration: &ProjectConfiguration,
    root: &Path,
    kind: ContextRootKind,
) -> Result<bool, InspectionError> {
    let mut command = tokio::process::Command::new("git");
    command
        .current_dir(root)
        .args(["-c", "core.fsmonitor=false", "rev-parse", "--show-toplevel"])
        .env("GIT_OPTIONAL_LOCKS", "0")
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .env("GIT_CONFIG_GLOBAL", "/dev/null")
        .env("GIT_CONFIG_SYSTEM", "/dev/null")
        .env("GIT_TERMINAL_PROMPT", "0")
        .env("GIT_ASKPASS", "")
        .env_remove("GIT_DIR")
        .env_remove("GIT_WORK_TREE")
        .env_remove("GIT_INDEX_FILE")
        .env_remove("GIT_COMMON_DIR");
    let result = crate::process::run_bounded_command(
        command,
        4096,
        4096,
        Duration::from_millis(configuration.limits.git_timeout_ms as u64),
        "context git root",
    )
    .await;
    let root_has_git_metadata = std::fs::symlink_metadata(root.join(".git")).is_ok();
    let Ok(output) = result else {
        if kind == ContextRootKind::Repository || root_has_git_metadata {
            return Err(InspectionError::new("context_file_index", "Git checkout could not be verified"));
        }
        return Ok(false);
    };
    if output.status.success()
        && Path::new(String::from_utf8_lossy(&output.stdout).trim()).canonicalize().ok().as_deref() == Some(root)
    {
        return Ok(true);
    }
    if kind == ContextRootKind::Repository || root_has_git_metadata {
        return Err(InspectionError::new("context_file_index", "Git checkout could not be verified"));
    }
    Ok(false)
}
async fn git_file_list(configuration: &ProjectConfiguration, root: &Path) -> Result<Vec<u8>, InspectionError> {
    let mut command = tokio::process::Command::new("git");
    command.current_dir(root)
        .args(["-c", "core.fsmonitor=false", "ls-files", "-z", "--cached", "--others", "--exclude-standard"])
        .env("GIT_OPTIONAL_LOCKS", "0")
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .env("GIT_CONFIG_GLOBAL", "/dev/null")
        .env("GIT_CONFIG_SYSTEM", "/dev/null")
        .env("GIT_TERMINAL_PROMPT", "0")
        .env("GIT_ASKPASS", "")
        .env_remove("GIT_DIR")
        .env_remove("GIT_WORK_TREE")
        .env_remove("GIT_INDEX_FILE")
        .env_remove("GIT_COMMON_DIR");
    let output = crate::process::run_bounded_command(
        command, MAX_FILE_INDEX_GIT_BYTES, 4096,
        Duration::from_millis(configuration.limits.git_timeout_ms as u64), "context git file list",
    ).await?;
    if !output.status.success() {
        return Err(InspectionError::new("context_file_index", "Git could not enumerate checkout files"));
    }
    Ok(output.stdout)
}

pub(crate) fn enumerate_file_index(
    authorized: &AuthorizedRoot,
    git_root: bool,
    git_output: Option<Vec<u8>>,
) -> Result<(Vec<ContextIndexedFile>, bool), InspectionError> {
    if git_root {
        let bytes = git_output.ok_or_else(|| InspectionError::new("context_file_index", "Git file list is unavailable"))?;
        let mut sorted: Vec<&[u8]> = bytes.split(|byte| *byte == 0).filter(|raw| !raw.is_empty()).collect();
        sorted.sort_unstable();
        let mut paths = Vec::new();
        let mut truncated = false;
        let mut previous: Option<&[u8]> = None;
        for raw in sorted {
            if previous == Some(raw) { continue; }
            previous = Some(raw);
            let Ok(path) = std::str::from_utf8(raw) else { continue };
            let Ok(relative) = relative_path(path) else { continue };
            if reserved_context_path(authorized.root.kind, &relative).is_some()
                || path.contains('\\') {
                continue;
            }
            let Some(metadata) = regular_file_metadata(&authorized.dir, &relative) else { continue };
            if paths.len() == MAX_FILE_INDEX_FILES { truncated = true; break; }
            paths.push(ContextIndexedFile { path: path.to_owned(), bytes: Some(metadata.len()) });
        }
        return Ok((paths, truncated));
    }
    let mut stack = vec![PathBuf::new()];
    let mut files = Vec::new();
    let mut truncated = false;
    let mut scanned = 0usize;
    while let Some(prefix) = stack.pop() {
        if prefix.components().count() >= 64 {
            truncated = true;
            continue;
        }
        let directory = authorized.resolve_directory(&prefix)?;
        let entries = directory.entries().map_err(|error| InspectionError::new("context_file_index", error.to_string()))?;
        for entry in entries {
            scanned += 1;
            if scanned > 200_000 { truncated = true; break; }
            let entry = entry.map_err(|error| InspectionError::new("context_file_index", error.to_string()))?;
            let name = entry.file_name();
            let relative = prefix.join(&name);
            if crate::library::folder_io::excluded_source_path(&relative)
                || reserved_context_path(authorized.root.kind, &relative).is_some()
                || relative.to_str().is_none()
                || relative.as_os_str().as_encoded_bytes().contains(&b'\\') {
                continue;
            }
            let metadata = directory.symlink_metadata(Path::new(&name)).map_err(|error| InspectionError::new("context_file_index", error.to_string()))?;
            if metadata.file_type().is_symlink() { continue; }
            if metadata.is_dir() {
                stack.push(relative);
            } else if metadata.is_file() {
                files.push(ContextIndexedFile { path: relative.to_string_lossy().into_owned(), bytes: Some(metadata.len()) });
            }
        }
        if scanned > 200_000 { break; }
    }
    files.sort_unstable_by(|left, right| left.path.as_bytes().cmp(right.path.as_bytes()));
    if files.len() > MAX_FILE_INDEX_FILES {
        files.truncate(MAX_FILE_INDEX_FILES);
        truncated = true;
    }
    Ok((files, truncated))
}

fn regular_file_metadata(root: &Dir, relative: &Path) -> Option<Metadata> {
    let mut components = relative.components().peekable();
    let mut directory = root.try_clone().ok()?;
    while let Some(component) = components.next() {
        let Component::Normal(name) = component else { return None };
        if components.peek().is_some() {
            directory = directory.open_dir_nofollow(name).ok()?;
        } else {
            let metadata = directory.symlink_metadata(Path::new(name)).ok()?;
            return (!metadata.file_type().is_symlink() && metadata.is_file()).then_some(metadata);
        }
    }
    None
}
pub(crate) fn read_directory(
    authorized: &AuthorizedRoot,
    request: &ContextDirectoryRequest,
    limits: &cockpit_protocol::projects::ProjectLimits,
) -> Result<ContextDirectory, InspectionError> {
    let relative = relative_path(&request.path)?;
    check_depth(&relative, limits.context_tree_depth)?;
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
    let limit = limits.context_directory_entries as usize;
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
        binding_id: request.binding_id.clone(),
        root_id: authorized.root.root_id.clone(),
        path: relative.to_string_lossy().into_owned(),
        entries: page,
        truncated,
        revision: Some(directory_revision),
        next_offset,
        total_entries,
        diagnostics: Vec::new(),
    })
}

pub(crate) fn read_document(
    authorized: &AuthorizedRoot,
    request: &ContextDocumentRequest,
    limits: &cockpit_protocol::projects::ProjectLimits,
) -> Result<ContextDocument, InspectionError> {
    let relative = relative_path(&request.path)?;
    if let Some(message) = reserved_context_path(authorized.root.kind, &relative) {
        return Err(InspectionError::new("context_reserved_path", message));
    }
    check_depth(&relative, limits.context_tree_depth)?;
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
        binding_id: request.binding_id.clone(),
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
    let max_bytes = limits.context_preview_bytes as usize;
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
    let max_lines = limits.context_preview_lines as usize;
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

pub(crate) fn find_root(roots: &[ContextRoot], root_id: &str) -> Result<AuthorizedRoot, InspectionError> {
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
    pub(crate) fn library(
        path: PathBuf,
        dir: Dir,
        max_depth: u32,
    ) -> Result<Self, InspectionError> {
        let metadata = dir
            .dir_metadata()
            .map_err(|error| InspectionError::new("library_unavailable", error.to_string()))?;
        let identity = format!("library:{}", filesystem_identity(&metadata));
        Ok(Self {
            root: ContextRoot {
                root_id: identity.clone(),
                kind: ContextRootKind::Library,
                label: "Library".into(),
                path: path.to_string_lossy().into_owned(),
                repository_id: identity,
                checkout_path: path.to_string_lossy().into_owned(),
            },
            canonical: path,
            dir,
            max_depth,
        })
    }

    pub(crate) fn summary(&self) -> ContextRoot {
        self.root.clone()
    }

    pub(crate) fn root_id(&self) -> &str {
        &self.root.root_id
    }
    pub(crate) fn canonical_path(&self) -> &Path {
        &self.canonical
    }

    pub(crate) fn root_kind(&self) -> ContextRootKind {
        self.root.kind
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

pub(crate) fn reserved_context_path(kind: ContextRootKind, relative: &Path) -> Option<&'static str> {
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
        if kind == ContextRootKind::Library && first && name == ".cockpit" {
            return Some("Cockpit Library metadata is not exposed");
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

fn evidence_path(path: &str) -> Option<PathBuf> {
    let path = Path::new(path);
    path.is_absolute()
        .then(|| absolute_context_path(path).ok())
        .flatten()
}

fn evidence_cwd(evidence: &SourcePaneEvidence) -> Option<PathBuf> {
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


#[cfg(test)]
mod review_checkout_tests {
    use super::*;
    use std::collections::BTreeMap;
    use std::sync::Arc;

    use cockpit_protocol::{
        viewer::{ViewerContext, ViewerKind, ViewerOpenRequest, ViewerSourceSelector},
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
        sources::SourceService,
        extension_adapter::TabEvidence,
        viewer::ViewerService,
        repositories::RepositoryCatalog,
        HerdrAdapter, ProjectHerdrAdapter, SessionSubscription, TerminalSession,
        project_adapter::{
            ProjectInventory, ProjectTerminalRequest, ProjectTerminalResult,
            ProjectWorktreeRemoveRequest, ProjectWorktreeRequest, ProjectWorktreeResult,
        },
    };

    struct TestAdapter {
        evidence: Mutex<SourcePaneEvidence>,
        tab: Mutex<TabEvidence>,
        source_present: std::sync::atomic::AtomicBool,
    }

    impl TestAdapter {
        fn new(cwd: &Path, foreground_cwd: &Path) -> Self {
            Self {
                evidence: Mutex::new(SourcePaneEvidence {
                    endpoint_identity: "endpoint".to_owned(),
                    pane_id: "pane".to_owned(),
                    terminal_id: "terminal".to_owned(),
                    workspace_id: "space".to_owned(),
                    tab_id: "tab".to_owned(),
                    cwd: Some(cwd.to_string_lossy().into_owned()),
                    foreground_cwd: Some(foreground_cwd.to_string_lossy().into_owned()),
                }),
                tab: Mutex::new(TabEvidence {
                    endpoint_identity: "endpoint".to_owned(),
                    server_instance: "server".to_owned(),
                    workspace_id: "space".to_owned(),
                    present: true,
                }),
                source_present: std::sync::atomic::AtomicBool::new(true),
            }
        }
    }

    fn services(
        configuration: ProjectConfiguration,
        adapter: Arc<TestAdapter>,
    ) -> (Arc<ContextService>, Arc<ViewerService>) {
        let projects = Arc::new(
            ProjectService::new(configuration.clone(), adapter.clone()).expect("project service"),
        );
        let context = Arc::new(ContextService::new(configuration, adapter, projects));
        let viewers = Arc::new(ViewerService::new(context.clone()));
        let context = Arc::new(context.as_ref().clone().with_viewers(viewers.clone()));
        (context, viewers)
    }

    fn library_services(
        configuration: ProjectConfiguration, adapter: Arc<TestAdapter>,
    ) -> (Arc<crate::library::LibraryService>, Arc<ContextService>, Arc<ViewerService>) {
        let sources = Arc::new(SourceService::new(&configuration, vec![]).expect("source service"));
        let library = Arc::new(crate::library::LibraryService::new(configuration.clone(), sources).with_herdr(adapter.clone()));
        let projects = Arc::new(ProjectService::new(configuration.clone(), adapter.clone()).expect("project service"));
        let context = Arc::new(ContextService::new(configuration, adapter, projects).with_library(library.clone()));
        let viewers = Arc::new(ViewerService::new(context.clone()));
        let context = Arc::new(context.as_ref().clone().with_viewers(viewers.clone()));
        (library, context, viewers)
    }

    async fn assert_files_features(
        configuration: &ProjectConfiguration, context: &Arc<ContextService>,
        viewer: &ViewerContext, root_path: &Path,
    ) {
        let root_id = viewer.default_root_id.clone().expect("Files root");
        let document = context.document("session", &viewer.viewer_id, &ContextDocumentRequest {
            binding_id: viewer.binding_id.clone(), root_id: root_id.clone(),
            path: "notes.md".into(), expected_revision: None, offset: None,
        }).await.expect("read live source");
        assert_eq!(document.text.as_deref(), Some("direct source needle\n"));
        let search = crate::context_search::ContextSearchService::new(context.clone())
            .search("session", &viewer.viewer_id, &cockpit_protocol::context_search::ContextSearchRequest {
                binding_id: viewer.binding_id.clone(), root_id: root_id.clone(),
                query: "needle".into(), request_generation: 1, offset: None, revision: None,
            }).await.expect("bounded Files search");
        assert_eq!(search.results.iter().map(|row| row.path.as_str()).collect::<Vec<_>>(), vec!["notes.md"]);
        let media = context.media("session", &viewer.viewer_id, &cockpit_protocol::context_media::ContextMediaRequest {
            binding_id: viewer.binding_id.clone(), root_id: root_id.clone(),
            path: "image.jpg".into(), expected_revision: None,
        }).await.expect("bounded Files raster");
        assert_eq!((media.width, media.height, media.mime_type.as_str()), (1, 1, "image/jpeg"));
        let index = context.file_index("session", &viewer.viewer_id, &ContextFileIndexRequest {
            binding_id: viewer.binding_id.clone(), root_id: root_id.clone(), mode: ContextFileIndexMode::Fresh,
        }).await.expect("Files index");
        assert!(index.files.iter().any(|entry| entry.path == "notes.md"));
        assert!(!index.files.iter().any(|entry| entry.path.starts_with(".git/")));
        let comments = crate::comments::CommentsService::new(configuration.clone(), context.clone()).expect("comments");
        let scope = cockpit_protocol::comments::CommentRequestScope {
            binding_id: viewer.binding_id.clone(), client_id: "client".into(),
        };
        let batch = comments.batch("session", &viewer.viewer_id, &cockpit_protocol::comments::CommentBatchRequest {
            scope: scope.clone(), batch_id: None,
        }).await.expect("same-tab comments");
        let captured = comments.upsert("session", &viewer.viewer_id, &cockpit_protocol::comments::CommentUpsertRequest {
            batch: cockpit_protocol::comments::CommentBatchMutation {
                scope: scope.clone(), batch_id: batch.batch_id, expected_generation: batch.generation,
            },
            draft_id: None,
            capture: Some(cockpit_protocol::comments::CommentCapture {
                root_id: root_id.clone(), path: "notes.md".into(), expected_revision: document.revision,
                start_line: Some(1), end_line: Some(1), review: None,
            }),
            comment_text: "Direct comment".into(),
        }).await.expect("capture direct source comment");
        assert_eq!(captured.drafts[0].file_ref.absolute_path, root_path.join("notes.md").to_string_lossy());
        let preview = comments.preview("session", &viewer.viewer_id, &cockpit_protocol::comments::CommentPreviewRequest {
            batch: cockpit_protocol::comments::CommentBatchMutation {
                scope, batch_id: captured.batch_id.clone(), expected_generation: captured.generation,
            },
            retain_stale_excerpts: false,
        }).await.expect("authorized direct-source payload");
        assert!(preview.payload.starts_with(&format!("{}:1\n", root_path.join("notes.md").to_string_lossy())));
        assert!(preview.payload.contains("Direct comment"));
        assert!(matches!(&captured.owner, cockpit_protocol::comments::CommentOwner::Viewer {
            tab_id, source_id, ..
        } if tab_id == "tab" && source_id == &root_id));
    }

    fn direct_source_files(root: &Path) {
        std::fs::create_dir_all(root).expect("source root");
        std::fs::write(root.join("notes.md"), "direct source needle\n").expect("source text");
        // A complete 1x1, 8-bit, three-component JPEG header accepted by the
        // bounded media parser (the viewer receives bytes, not host paths).
        std::fs::write(root.join("image.jpg"), [
            0xff, 0xd8, 0xff, 0xc0, 0, 17, 8, 0, 1, 0, 1, 3,
            1, 0x11, 0, 2, 0x11, 0, 3, 0x11, 0, 0xff, 0xd9,
        ]).expect("source raster");
    }

    #[tokio::test]
    async fn files_context_exposes_full_library_without_cwd_or_selected_items() {
        let workspace = std::env::temp_dir().join(format!("cockpit-direct-library-{}", Uuid::new_v4()));
        let source = workspace.join("unrelated");
        std::fs::create_dir_all(&source).expect("source");
        let configuration = configuration(&workspace);
        let library_path = PathBuf::from(&configuration.library_root);
        direct_source_files(&library_path);
        std::fs::create_dir(library_path.join("unselected-folder")).expect("unselected content");
        std::fs::write(library_path.join("unselected-folder/document.md"), "unselected live data").expect("unselected document");
        let adapter = Arc::new(TestAdapter::new(&source, &source));
        {
            let mut evidence = adapter.evidence.lock().await;
            evidence.cwd = None;
            evidence.foreground_cwd = None;
        }
        let (library, context, viewers) = library_services(configuration.clone(), adapter.clone());
        let target = cockpit_protocol::library::SpaceTarget { session_id: "session".into(), space_id: "space".into() };
        assert!(library.space_listing(&target).await.expect("live Space").items.is_empty());
        std::fs::write(library_path.join(".cockpit/private.txt"), "private needle").expect("private metadata");
        let options = viewers.sources("session", "pane").await.expect("Library without cwd");
        assert!(options.files_folder_root_id.is_none());
        let viewer = viewers.open("session", &open_request(ViewerSourceSelector::FilesContext {})).await.expect("full Library viewer");
        let root_id = viewer.default_root_id.clone().expect("Library root");
        assert_eq!(Some(root_id.clone()), options.files_context_root_id);
        assert_eq!(viewer.roots[0].kind, ContextRootKind::Library);
        assert_files_features(&configuration, &context, &viewer, &library_path).await;
        let directory = context.directory("session", &viewer.viewer_id, &ContextDirectoryRequest {
            binding_id: viewer.binding_id.clone(), root_id: root_id.clone(), path: "".into(), offset: None, revision: None,
        }).await.expect("full Library directory");
        assert!(directory.entries.iter().any(|entry| entry.name == "unselected-folder"));
        assert!(!directory.entries.iter().any(|entry| entry.name == ".cockpit"));
        let unselected = context.document("session", &viewer.viewer_id, &ContextDocumentRequest {
            binding_id: viewer.binding_id.clone(), root_id: root_id.clone(),
            path: "unselected-folder/document.md".into(), expected_revision: None, offset: None,
        }).await.expect("unselected Library content remains readable");
        assert_eq!(unselected.text.as_deref(), Some("unselected live data"));
        let mut request = ContextDocumentRequest {
            binding_id: viewer.binding_id.clone(), root_id, path: ".cockpit/private.txt".into(), expected_revision: None, offset: None,
        };
        assert_eq!(context.document("session", &viewer.viewer_id, &request).await.expect_err("metadata hidden").code, "context_reserved_path");
        request.path = "notes.md".into();
        std::fs::write(library_path.join("notes.md"), "refreshed live data").expect("refresh");
        assert_eq!(context.document("session", &viewer.viewer_id, &request).await.expect("live refresh").text.as_deref(), Some("refreshed live data"));
        std::fs::rename(&library_path, workspace.join("old-library")).expect("retain original Library");
        std::fs::create_dir(&library_path).expect("replacement Library");
        assert_eq!(context.document("session", &viewer.viewer_id, &request).await.expect_err("stale Library root").code, "context_root_not_authorized");
        std::fs::remove_dir_all(workspace).expect("cleanup");
    }

    #[tokio::test]
    async fn files_repository_uses_fresh_selection_and_rejects_paths_and_retired_roots() {
        let workspace = std::env::temp_dir().join(format!("cockpit-direct-repository-{}", Uuid::new_v4()));
        let repository = workspace.join("repository");
        git_fixture(&repository);
        direct_source_files(&repository);
        let source = workspace.join("unrelated");
        std::fs::create_dir(&source).expect("unrelated source");
        let adapter = Arc::new(TestAdapter::new(&source, &source));
        let configuration = configuration(&workspace);
        let (library, context, viewers) = library_services(configuration.clone(), adapter.clone());
        let target = cockpit_protocol::library::SpaceTarget { session_id: "session".into(), space_id: "space".into() };
        library.space_repositories(cockpit_protocol::library::SpaceRepositoriesRequest {
            target: target.clone(), repository_paths: vec![repository.to_string_lossy().into_owned()],
        }).await.expect("select repository");
        let options = viewers.sources("session", "pane").await.expect("fresh selected repository");
        assert!(options.review_repository_ids.is_empty(), "selection does not grant Review authority");
        let root = options.roots.iter().find(|root| root.kind == ContextRootKind::Repository).expect("selected root");
        assert_eq!(root.path, repository.to_string_lossy());
        assert_eq!(viewers.open("session", &open_request(ViewerSourceSelector::FilesRepository {
            root_id: repository.to_string_lossy().into_owned(),
        })).await.expect_err("client path grants no authority").code, "viewer_source_unavailable");
        assert_eq!(viewers.open("session", &open_request(ViewerSourceSelector::Review {
            repository_id: root.repository_id.clone(),
        })).await.expect_err("extra repository is Files-only").code, "viewer_source_unavailable");
        let viewer = viewers.open("session", &open_request(ViewerSourceSelector::FilesRepository {
            root_id: root.root_id.clone(),
        })).await.expect("selected repository Files viewer");
        assert_files_features(&configuration, &context, &viewer, &repository).await;
        let request = ContextDocumentRequest {
            binding_id: viewer.binding_id.clone(), root_id: root.root_id.clone(),
            path: "notes.md".into(), expected_revision: None, offset: None,
        };
        #[cfg(unix)]
        {
            let metadata = workspace.join("original-git");
            std::fs::rename(repository.join(".git"), &metadata).expect("retain Git metadata");
            std::os::unix::fs::symlink(&metadata, repository.join(".git")).expect("unsafe Git metadata");
            assert_eq!(context.document("session", &viewer.viewer_id, &request).await.expect_err("symlink Git root revoked").code, "context_root_not_authorized");
            std::fs::remove_file(repository.join(".git")).expect("remove fixture symlink");
            std::fs::rename(metadata, repository.join(".git")).expect("restore Git metadata");
        }
        library.space_repositories(cockpit_protocol::library::SpaceRepositoriesRequest {
            target: target.clone(), repository_paths: vec![],
        }).await.expect("remove selection");
        assert_eq!(context.document("session", &viewer.viewer_id, &request).await.expect_err("unselected root revoked").code, "context_root_not_authorized");
        library.space_repositories(cockpit_protocol::library::SpaceRepositoriesRequest {
            target, repository_paths: vec![repository.to_string_lossy().into_owned()],
        }).await.expect("restore selection");
        std::fs::rename(&repository, workspace.join("old-repository")).expect("retain selected checkout");
        git_fixture(&repository);
        direct_source_files(&repository);
        assert_eq!(context.document("session", &viewer.viewer_id, &request).await.expect_err("replacement checkout").code, "context_root_not_authorized");
        adapter.tab.lock().await.endpoint_identity = "new-endpoint".into();
        assert_eq!(context.document("session", &viewer.viewer_id, &request).await.expect_err("retired endpoint").code, "viewer_tab_absent");
        std::fs::remove_dir_all(workspace).expect("cleanup");
    }

    #[tokio::test]
    async fn persisted_repository_selection_loses_authority_when_removed_from_catalog() {
        let workspace = std::env::temp_dir().join(format!("cockpit-direct-catalog-revocation-{}", Uuid::new_v4()));
        let repository = workspace.join("repository");
        git_fixture(&repository);
        direct_source_files(&repository);
        let source = workspace.join("source");
        std::fs::create_dir(&source).expect("source");
        let adapter = Arc::new(TestAdapter::new(&source, &source));
        let initial = configuration(&workspace);
        let target = cockpit_protocol::library::SpaceTarget { session_id: "session".into(), space_id: "space".into() };
        let (library, context, viewers) = library_services(initial.clone(), adapter.clone());
        library.space_repositories(cockpit_protocol::library::SpaceRepositoriesRequest {
            target: target.clone(), repository_paths: vec![repository.to_string_lossy().into_owned()],
        }).await.expect("select configured repository");
        let issued = viewers.sources("session", "pane").await.expect("initial roots");
        let root = issued.roots.iter().find(|root| root.path == repository.to_string_lossy()).expect("issued repository");
        let selector = ViewerSourceSelector::FilesRepository { root_id: root.root_id.clone() };
        viewers.open("session", &open_request(selector.clone())).await.expect("initial authorized viewer");
        drop(viewers);
        drop(context);
        drop(library);
        let configured = workspace.join("remaining-configured-root");
        std::fs::create_dir(&configured).expect("remaining root");
        let mut revised = initial;
        revised.repository_roots = vec![configured.to_string_lossy().into_owned()];
        let (library, _context, viewers) = library_services(revised, adapter.clone());
        let listing = library.space_listing(&target).await.expect("retained selection");
        assert_eq!(listing.repository_paths, vec![repository.to_string_lossy().into_owned()]);
        assert!(!listing.diagnostics.is_empty(), "selection remains recoverable but diagnosed");
        let options = viewers.sources("session", "pane").await.expect("revised roots");
        assert!(!options.roots.iter().any(|root| root.path == repository.to_string_lossy()), "retired catalog selection grants no root");
        assert_eq!(viewers.open("session", &open_request(selector)).await.expect_err("stale selector").code, "viewer_source_unavailable");
        {
            let mut evidence = adapter.evidence.lock().await;
            evidence.cwd = Some(repository.to_string_lossy().into_owned());
            evidence.foreground_cwd = evidence.cwd.clone();
        }
        let options = viewers.sources("session", "pane").await.expect("terminal-owned checkout roots");
        assert!(options.roots.iter().any(|root| root.kind == ContextRootKind::Repository && root.path == repository.to_string_lossy()),
            "independent live terminal checkout authority is not catalog-restricted");
        std::fs::remove_dir_all(workspace).expect("cleanup");
    }

    fn open_request(source: ViewerSourceSelector) -> ViewerOpenRequest {
        ViewerOpenRequest {
            tab_id: "tab".to_owned(),
            kind: match &source {
                ViewerSourceSelector::Review { .. } => ViewerKind::Review,
                _ => ViewerKind::Files,
            },
            source_pane_id: "pane".to_owned(),
            source,
            client_id: "client".to_owned(),
        }
    }

    async fn open_folder(viewers: &ViewerService) -> ViewerContext {
        viewers.open("session", &open_request(ViewerSourceSelector::FilesFolder {}))
            .await.expect("open Files folder viewer")
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
            session_id: &str,
        ) -> Result<SessionSnapshotResponse, InspectionError> {
            let tab = self.tab.lock().await.clone();
            Ok(SessionSnapshotResponse {
                session_id: session_id.to_owned(),
                server_instance: tab.server_instance,
                version: "test".into(),
                protocol: 1,
                focused_space_id: None,
                focused_tab_id: None,
                focused_pane_id: None,
                herdr_shell: None,
                spaces: if tab.present {
                    vec![cockpit_protocol::v1::SpaceSummary {
                        id: tab.workspace_id, label: "Test Space".into(), number: 1,
                        tab_count: 1, pane_count: 1, focused: false, agent_status: "none".into(), git: None,
                    }]
                } else { vec![] },
                tabs: vec![], panes: vec![], agents: vec![],
            })
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
            Ok(self.tab.lock().await.endpoint_identity.clone())
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
    impl SourcePaneAdapter for TestAdapter {
        async fn source_pane_evidence(
            &self,
            _: &str,
            pane_id: &str,
        ) -> Result<SourcePaneEvidence, InspectionError> {
            if !self.source_present.load(std::sync::atomic::Ordering::Relaxed) {
                return Err(InspectionError::new("pane_not_found", "source pane closed"));
            }
            let evidence = self.evidence.lock().await.clone();
            if pane_id != evidence.pane_id {
                return Err(InspectionError::new("pane_not_found", "unknown source pane"));
            }
            Ok(evidence)
        }

        async fn tab_evidence(
            &self,
            _: &str,
            _: &str,
        ) -> Result<TabEvidence, InspectionError> {
            Ok(self.tab.lock().await.clone())
        }
    }

    fn configuration(root: &Path) -> ProjectConfiguration {
        ProjectConfiguration { version: 1, orchestration: Default::default(), repository_roots: vec![root.to_string_lossy().into_owned()],
        worktree_root: root.join("worktrees").to_string_lossy().into_owned(),
        companion_root: root.join("companions").to_string_lossy().into_owned(),
        state_root: root.join("state").to_string_lossy().into_owned(),
        cache_root: root.join("cache").to_string_lossy().into_owned(),
        library_root: root.join("library").to_string_lossy().into_owned(),
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
            library_folder_files: 512,
            library_folder_bytes: 32 * 1024 * 1024,
            library_file_bytes: 4 * 1024 * 1024,
            library_space_pages: 200,
            library_attachment_bytes: 25 * 1024 * 1024,
            library_item_attachment_bytes: 100 * 1024 * 1024,
            library_max_items: 20_000,
        },
        origins: BTreeMap::new(), }
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
    async fn opens_files_from_a_fresh_source_folder_root_while_library_unavailable() {
        let workspace =
            std::env::temp_dir().join(format!("cockpit-context-files-{}", Uuid::new_v4()));
        let repository = workspace.join("repository");
        git_fixture(&repository);
        let source = repository.join("nested");
        std::fs::create_dir(&source).expect("source directory");
        let adapter = Arc::new(TestAdapter::new(&source, &source));
        let configuration = configuration(&workspace);
        std::fs::write(&configuration.library_root, b"not a directory").expect("unusable Library");
        let sources = Arc::new(SourceService::new(&configuration, vec![]).expect("source service"));
        let library = crate::library::LibraryService::new(configuration.clone(), sources);
        assert_eq!(
            library
                .listing(None)
                .await
                .expect_err("Library unavailable")
                .code,
            "library_unavailable"
        );
        let (context, viewers) = services(configuration, adapter);
        let options = viewers.sources("session", "pane").await.expect("source options");
        assert!(options.files_context_root_id.is_none());
        let files_root_id = options.files_folder_root_id.clone().expect("source folder root");
        let presentation = open_folder(&viewers).await;
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
        assert_eq!(presentation.default_root_id.as_deref(), Some(files_root_id.as_str()));

        let media_root = context
            .authorize_media_root("session", &presentation.viewer_id, &presentation.binding_id, &files_root_id)
            .await
            .expect("Folder root is authorized for bounded media reads");
        assert_eq!(media_root.root.kind, ContextRootKind::Folder);
        let repository_root_id = options
            .roots
            .iter()
            .find(|root| root.kind == ContextRootKind::Repository)
            .expect("repository root")
            .root_id
            .clone();
        let media_error = match context
            .authorize_media_root(
                "session",
                &presentation.viewer_id,
                &presentation.binding_id,
                &repository_root_id,
            )
            .await
        {
            Ok(_) => panic!("Repository root cannot authorize media"),
            Err(error) => error,
        };
        assert_eq!(media_error.code, "context_root_not_authorized");

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
        let adapter = Arc::new(TestAdapter::new(&non_git_cwd, &child));
        let configuration = configuration(&workspace);
        let (_, viewers) = services(configuration.clone(), adapter);
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

        let options = viewers.sources("session", "pane").await.expect("source options");
        assert_eq!(options.review_repository_ids, vec![child_id.clone()]);
        let presentation = viewers.open("session", &open_request(
            ViewerSourceSelector::Review { repository_id: child_id.clone() },
        )).await.expect("foreground checkout viewer");
        let default = presentation.default_root_id.as_deref().expect("checkout root");
        assert_eq!(
            presentation
                .roots
                .iter()
                .find(|root| root.root_id == default)
                .expect("default root")
                .repository_id,
            child_id
        );

        assert_eq!(
            viewers.open("session", &open_request(
                ViewerSourceSelector::Review { repository_id: parent_id },
            )).await.expect_err("parent selection is refused").code,
            "viewer_source_unavailable"
        );

        std::fs::remove_dir_all(workspace).expect("cleanup");
    }

    #[tokio::test]
    async fn files_viewer_reads_and_comments_on_an_ordinary_folder() {
        let workspace =
            std::env::temp_dir().join(format!("cockpit-context-folder-{}", Uuid::new_v4()));
        let catalog = workspace.join("catalog");
        git_fixture(&catalog);
        let folder = workspace.join("ordinary-folder");
        std::fs::create_dir_all(folder.join(".git")).expect("ordinary folder");
        std::fs::write(folder.join("notes.md"), "ordinary context\n").expect("write context");
        let adapter = Arc::new(TestAdapter::new(&folder, &folder));
        let configuration = configuration(&catalog);
        let (context, viewers) = services(configuration.clone(), adapter);
        let presentation = open_folder(&viewers).await;
        let root_id = presentation.default_root_id.clone().expect("folder root");
        let root = presentation
            .roots
            .iter()
            .find(|root| root.root_id == root_id)
            .expect("selected root");
        assert_eq!(root.kind, ContextRootKind::Folder);
        assert!(presentation.roots.iter().all(|root| root.path == folder.to_string_lossy()));
        assert_eq!(root.path, folder.to_string_lossy().as_ref());

        let directory = context
            .directory(
                "session",
                &presentation.viewer_id,
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
                &presentation.viewer_id,
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
                &presentation.viewer_id,
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
                &presentation.viewer_id,
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
                &presentation.viewer_id,
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
                &presentation.viewer_id,
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
        assert_eq!(
            second_directory_page.entries.iter().map(|entry| entry.name.as_str()).collect::<Vec<_>>(),
            vec!["entry-9.txt", "large.txt", "notes.md"],
        );
        let comments = crate::comments::CommentsService::new(configuration, context.clone())
            .expect("comments");
        let scope = cockpit_protocol::comments::CommentRequestScope {
            binding_id: presentation.binding_id.clone(),
            client_id: "client".to_owned(),
        };
        let batch = comments
            .batch(
                "session",
                &presentation.viewer_id,
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
            .upsert("session", &presentation.viewer_id, &request)
            .await
            .expect("capture ordinary folder source");
        assert_eq!(
            saved.drafts[0].file_ref.absolute_path,
            folder.join("notes.md").to_string_lossy()
        );
        assert!(matches!(
            &saved.owner,
            cockpit_protocol::comments::CommentOwner::Viewer { source_id, .. } if source_id == &root_id
        ));
        std::fs::write(folder.join("notes.md"), "updated context\n").expect("change source");
        let mut edit = request.clone();
        edit.batch.expected_generation = saved.generation;
        edit.draft_id = Some(saved.drafts[0].draft_id.clone());
        edit.capture = None;
        edit.comment_text = "Edited prose on old source".to_owned();
        let edited = comments
            .upsert("session", &presentation.viewer_id, &edit)
            .await
            .expect("edit retained comment prose");
        assert_eq!(
            edited.drafts[0].source_state,
            cockpit_protocol::comments::CommentSourceState::Changed,
            "an edit response cannot relabel a retained old-source comment as current"
        );
        assert_eq!(edited.drafts[0].file_ref.revision, document.revision);
        assert_eq!(edited.drafts[0].comment_text, edit.comment_text);
        let mut outside = request.clone();
        outside.batch.expected_generation = edited.generation;
        outside.capture.as_mut().unwrap().path = "../outside.md".to_owned();
        assert!(
            comments.upsert("session", &presentation.viewer_id, &outside).await.is_err(),
            "comments preserve bounded reads"
        );
        std::fs::rename(&folder, workspace.join("old-folder")).expect("replace root");
        std::fs::create_dir(&folder).expect("new root");
        std::fs::write(folder.join("notes.md"), "ordinary context\n").expect("same text new root");
        outside.capture = request.capture.clone();
        assert!(
            comments.upsert("session", &presentation.viewer_id, &outside).await.is_err(),
            "old batch cannot attach to replacement root"
        );
        std::fs::remove_dir_all(workspace).expect("cleanup");
    }

    #[tokio::test]
    #[cfg(unix)]
    async fn files_viewer_refuses_symlink_sources_and_replaced_roots() {
        let workspace =
            std::env::temp_dir().join(format!("cockpit-context-folder-{}", Uuid::new_v4()));
        let folder = workspace.join("folder");
        std::fs::create_dir_all(&folder).expect("folder");
        std::fs::write(folder.join("notes.md"), "original\n").expect("source");
        let linked = workspace.join("linked-folder");
        std::os::unix::fs::symlink(&folder, &linked).expect("folder symlink");
        let adapter = Arc::new(TestAdapter::new(&linked, &linked));
        let (context, viewers) = services(configuration(&workspace), adapter.clone());
        assert_eq!(
            viewers.open("session", &open_request(ViewerSourceSelector::FilesFolder {}))
                .await.expect_err("symlink source is unsafe").code,
            "viewer_source_unavailable",
        );
        {
            let mut evidence = adapter.evidence.lock().await;
            evidence.cwd = Some(folder.to_string_lossy().into_owned());
            evidence.foreground_cwd = evidence.cwd.clone();
        }
        let viewer = open_folder(&viewers).await;
        let request = ContextDocumentRequest {
            binding_id: viewer.binding_id.clone(),
            root_id: viewer.default_root_id.clone().expect("folder root"),
            path: "notes.md".to_owned(),
            expected_revision: None,
            offset: None,
        };
        assert_eq!(
            context.document("session", &viewer.viewer_id, &request).await.unwrap().text.as_deref(),
            Some("original\n"),
        );
        std::fs::rename(&folder, workspace.join("old-folder")).expect("retain original inode");
        std::fs::create_dir(&folder).expect("replacement folder");
        std::fs::write(folder.join("notes.md"), "original\n").expect("same text different root");
        assert_eq!(
            context.document("session", &viewer.viewer_id, &request)
                .await.expect_err("replacement root cannot be authorized").code,
            "context_root_not_authorized",
        );
        std::fs::remove_dir_all(workspace).expect("cleanup");
    }

    #[tokio::test]
    async fn viewer_source_switch_rebinds_then_release_revokes_reads() {
        let workspace =
            std::env::temp_dir().join(format!("cockpit-context-rebind-{}", Uuid::new_v4()));
        let first = workspace.join("first");
        let second = workspace.join("second");
        std::fs::create_dir_all(&first).expect("first folder");
        std::fs::create_dir_all(&second).expect("second folder");
        std::fs::write(first.join("notes.md"), "first\n").expect("first source");
        std::fs::write(second.join("notes.md"), "second\n").expect("second source");
        let adapter = Arc::new(TestAdapter::new(&first, &first));
        let (context, viewers) = services(configuration(&workspace), adapter.clone());
        let first_viewer = open_folder(&viewers).await;
        let old_request = ContextDocumentRequest {
            binding_id: first_viewer.binding_id.clone(),
            root_id: first_viewer.default_root_id.clone().expect("first root"),
            path: "notes.md".to_owned(),
            expected_revision: None,
            offset: None,
        };
        {
            let mut evidence = adapter.evidence.lock().await;
            evidence.foreground_cwd = Some(second.to_string_lossy().into_owned());
        }
        assert_eq!(
            context.document("session", &first_viewer.viewer_id, &old_request)
                .await.unwrap().text.as_deref(),
            Some("first\n"),
            "cd does not change the pinned browsing source",
        );
        let second_viewer = open_folder(&viewers).await;
        assert_eq!(first_viewer.viewer_id, second_viewer.viewer_id);
        assert_ne!(first_viewer.binding_id, second_viewer.binding_id);
        assert_eq!(
            context.document("session", &first_viewer.viewer_id, &old_request)
                .await.expect_err("old binding is retired").code,
            "context_stale_binding",
        );
        let request = ContextDocumentRequest {
            binding_id: second_viewer.binding_id.clone(),
            root_id: second_viewer.default_root_id.clone().expect("second root"),
            ..old_request
        };
        adapter.source_present.store(false, std::sync::atomic::Ordering::Relaxed);
        assert_eq!(
            context.document("session", &second_viewer.viewer_id, &request)
                .await.unwrap().text.as_deref(),
            Some("second\n"),
            "closing the source terminal does not revoke a live tab viewer",
        );
        viewers.release("session", &second_viewer.viewer_id).await.expect("release");
        assert_eq!(
            context.document("session", &second_viewer.viewer_id, &request)
                .await.expect_err("released viewer cannot read").code,
            "viewer_not_found",
        );
        std::fs::remove_dir_all(workspace).expect("cleanup");
    }

    #[tokio::test]
    async fn viewer_rejects_cross_tab_sources_and_changed_server_identity() {
        let workspace =
            std::env::temp_dir().join(format!("cockpit-context-identity-{}", Uuid::new_v4()));
        std::fs::create_dir_all(&workspace).expect("folder");
        std::fs::write(workspace.join("notes.md"), "source\n").expect("source");
        let adapter = Arc::new(TestAdapter::new(&workspace, &workspace));
        let (context, viewers) = services(configuration(&workspace), adapter.clone());
        let mut wrong_tab = open_request(ViewerSourceSelector::FilesFolder {});
        wrong_tab.tab_id = "other-tab".to_owned();
        assert_eq!(
            viewers.open("session", &wrong_tab).await.expect_err("source belongs to another tab").code,
            "viewer_source_not_in_tab",
        );
        let viewer = open_folder(&viewers).await;
        let request = ContextDocumentRequest {
            binding_id: viewer.binding_id.clone(),
            root_id: viewer.default_root_id.clone().expect("folder root"),
            path: "notes.md".to_owned(),
            expected_revision: None,
            offset: None,
        };
        adapter.tab.lock().await.endpoint_identity = "new-endpoint".to_owned();
        assert_eq!(
            context.document("session", &viewer.viewer_id, &request)
                .await.expect_err("new server cannot inherit old viewer authority").code,
            "viewer_tab_absent",
        );
        assert_eq!(
            context.document("session", &viewer.viewer_id, &request)
                .await.expect_err("old viewer was pruned").code,
            "viewer_not_found",
        );
        std::fs::remove_dir_all(workspace).expect("cleanup");
    }
}
#[cfg(test)]
mod file_index_tests {
    use super::*;
    use std::process::Command;
    use uuid::Uuid;

    fn git(root: &Path, args: &[&str]) -> Vec<u8> {
        let output = Command::new("git")
            .current_dir(root)
            .args(args)
            .env("GIT_CONFIG_NOSYSTEM", "1")
            .env("GIT_CONFIG_GLOBAL", "/dev/null")
            .env_remove("GIT_DIR")
            .env_remove("GIT_WORK_TREE")
            .env_remove("GIT_INDEX_FILE")
            .output()
            .expect("git starts");
        assert!(output.status.success(), "git {args:?}: {}", String::from_utf8_lossy(&output.stderr));
        output.stdout
    }

    #[test]
    fn git_index_excludes_ignored_files_and_walk_never_follows_symlinks() {
        let root = std::env::temp_dir().join(format!("cockpit-context-file-index-{}", Uuid::new_v4()));
        std::fs::create_dir_all(&root).unwrap();
        git(&root, &["init", "-q"]);
        git(&root, &["config", "user.email", "test@example.invalid"]);
        git(&root, &["config", "user.name", "Test"]);
        std::fs::write(root.join(".gitignore"), "ignored/\n").unwrap();
        std::fs::write(root.join("tracked.txt"), "tracked").unwrap();
        std::fs::create_dir_all(root.join("ignored")).unwrap();
        std::fs::write(root.join("ignored/secret.txt"), "ignored").unwrap();
        std::fs::write(root.join("untracked.txt"), "untracked").unwrap();
        git(&root, &["add", ".gitignore", "tracked.txt"]);
        git(&root, &["commit", "-qm", "fixture"]);
        #[cfg(unix)]
        std::os::unix::fs::symlink(root.join("tracked.txt"), root.join("link.txt")).unwrap();

        let dir = open_dir_nofollow_absolute(&root).unwrap();
        let repository = AuthorizedRoot {
            root: ContextRoot {
                root_id: "repository:test".into(),
                kind: ContextRootKind::Repository,
                label: "Test".into(),
                path: root.to_string_lossy().into_owned(),
                repository_id: "test".into(),
                checkout_path: root.to_string_lossy().into_owned(),
            },
            canonical: root.to_path_buf(),
            dir,
            max_depth: 64,
        };
        let output = git(&root, &["ls-files", "-z", "--cached", "--others", "--exclude-standard"]);
        let (files, truncated) = enumerate_file_index(&repository, true, Some(output)).unwrap();
        assert!(!truncated);
        assert_eq!(files.iter().map(|file| file.path.as_str()).collect::<Vec<_>>(), vec![".gitignore", "tracked.txt", "untracked.txt"]);

        let plain = std::env::temp_dir().join(format!("cockpit-context-file-index-plain-{}", Uuid::new_v4()));
        std::fs::create_dir_all(&plain).unwrap();
        std::fs::write(plain.join("visible.txt"), "visible").unwrap();
        std::fs::create_dir_all(plain.join(".cockpit")).unwrap();
        std::fs::write(plain.join(".cockpit/private"), "private").unwrap();
        std::fs::create_dir_all(plain.join("node_modules/pkg")).unwrap();
        std::fs::write(plain.join("node_modules/pkg/private.js"), "private").unwrap();
        #[cfg(unix)]
        std::os::unix::fs::symlink(root.join("tracked.txt"), plain.join("linked.txt")).unwrap();
        let plain_dir = cap_std::fs::Dir::open_ambient_dir(&plain, cap_std::ambient_authority()).unwrap();
        let library = AuthorizedRoot::library(plain.clone(), plain_dir, 64).unwrap();
        let (files, truncated) = enumerate_file_index(&library, false, None).unwrap();
        assert!(!truncated);
        assert_eq!(files.iter().map(|file| file.path.as_str()).collect::<Vec<_>>(), vec!["visible.txt"]);
        std::fs::remove_dir_all(root).unwrap();
        std::fs::remove_dir_all(plain).unwrap();
    }
}
