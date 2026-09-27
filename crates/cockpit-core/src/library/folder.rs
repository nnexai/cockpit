use super::*;
use crate::context_assets::{create_parent, excluded_source_path, git_inventory, git_output,
    open_absolute_dir_nofollow, read_stable_source_bounded, source_root_revalidate};
use crate::project_store::atomic_write_bytes;
use cap_fs_ext::DirExt;
use cap_std::fs::Dir;
use std::path::{Component, PathBuf};
use store::MarkerFile;

pub(super) fn recognizes(input: &str) -> bool {
    input.starts_with('/') || input == "~" || input.starts_with("~/")
}
fn unavailable(e: impl std::fmt::Display) -> InspectionError {
    error("library_folder_unavailable", e.to_string())
}
fn folder_id(path: &Path) -> String {
    format!("folder:{:x}", Sha256::digest(path.as_os_str().as_encoded_bytes()))
}
fn canonical_managed(path: &Path) -> Result<PathBuf, InspectionError> {
    match std::fs::canonicalize(path) {
        Ok(path) => Ok(path),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            let parent = path.parent().ok_or_else(|| unavailable(e))?;
            Ok(canonical_managed(parent)?.join(path.file_name().ok_or_else(|| unavailable("Invalid managed root"))?))
        }
        Err(e) => Err(unavailable(e)),
    }
}
fn open_source(configuration: &ProjectConfiguration, input: &str) -> Result<(PathBuf, Dir), InspectionError> {
    let path = if input == "~" || input.starts_with("~/") {
        let home = std::env::var_os("HOME").ok_or_else(|| unavailable("HOME is not set"))?;
        PathBuf::from(home).join(input.strip_prefix("~/").unwrap_or(""))
    } else { PathBuf::from(input) };
    if !path.is_absolute() || path.components().any(|c| matches!(c, Component::ParentDir | Component::Prefix(_))) {
        return Err(unavailable("Folder path must be absolute and contain no parent traversal"));
    }
    let root = open_absolute_dir_nofollow(&path).map_err(unavailable)?;
    let path = std::fs::canonicalize(path).map_err(unavailable)?;
    source_root_revalidate(&root, &path)?;
    for managed in [&configuration.library_root, &configuration.companion_root,
        &configuration.state_root, &configuration.worktree_root] {
        let managed = canonical_managed(Path::new(managed))?;
        if path.starts_with(&managed) || managed.starts_with(&path) {
            return Err(error("library_folder_refused", "Folder overlaps a Cockpit-owned root"));
        }
    }
    Ok((path, root))
}
struct Inventory {
    path: PathBuf,
    root: Dir,
    paths: Vec<PathBuf>,
    info: LibraryFolderInfo,
    diagnostics: Vec<ProjectDiagnostic>,
}
fn walk(root: &Dir, prefix: &Path, paths: &mut Vec<PathBuf>, info: &mut LibraryFolderInfo, visited: &mut usize, depth: usize) -> Result<(), InspectionError> {
    if depth > 64 { return Err(unavailable("Folder nesting exceeds the safe inventory depth")); }
    for entry in root.entries().map_err(unavailable)? {
        let entry = entry.map_err(unavailable)?;
        *visited += 1;
        if *visited > 1_000_000 { return Err(unavailable("Folder inventory exceeds the safe entry bound")); }
        let name = entry.file_name();
        let path = prefix.join(&name);
        if excluded_source_path(&path) { info.skipped_ignored += 1; continue; }
        if path.to_str().is_none() || path.as_os_str().as_encoded_bytes().contains(&b'\\') || path == Path::new(".cockpit-item.json") {
            info.skipped_other += 1; continue;
        }
        let metadata = root.symlink_metadata(&name).map_err(unavailable)?;
        if metadata.file_type().is_symlink() { info.skipped_symlinks += 1; }
        else if metadata.is_dir() {
            let child = root.open_dir_nofollow(&name).map_err(unavailable)?;
            walk(&child, &path, paths, info, visited, depth + 1)?;
        } else if metadata.is_file() { paths.push(path); }
        else { info.skipped_special += 1; }
    }
    Ok(())
}
async fn inventory(configuration: &ProjectConfiguration, input: &str) -> Result<Inventory, InspectionError> {
    let (path, root) = open_source(configuration, input)?;
    let git = git_output(configuration, &path, &["rev-parse", "--show-toplevel"]).await?;
    let is_git = git.status.success() &&
        std::str::from_utf8(&git.stdout).is_ok_and(|s| Path::new(s.trim_end()) == path);
    let mut info = LibraryFolderInfo { origin_path: path.to_string_lossy().into_owned(), git_working_tree: is_git,
        files: 0, bytes: 0, skipped_symlinks: 0, skipped_special: 0, skipped_ignored: 0, skipped_other: 0 };
    let mut diagnostics = vec![];
    let mut paths = if is_git {
        let (paths, gitlinks, excluded, _, _) = git_inventory(configuration, &path).await?;
        info.skipped_ignored = excluded.len() as u32;
        info.skipped_other = gitlinks.len() as u32;
        for link in gitlinks {
            diagnostics.push(ProjectDiagnostic { code: "library_folder_gitlink".into(),
                message: format!("Gitlink {} was not traversed", link.commit), path: Some(link.path.to_string_lossy().into_owned()) });
        }
        let ignored = git_output(configuration, &path, &["ls-files", "-z", "--others", "--ignored", "--exclude-standard"]).await?;
        if !ignored.status.success() { return Err(unavailable("Git could not count ignored files")); }
        info.skipped_ignored += ignored.stdout.split(|b| *b == 0).filter(|p| !p.is_empty()).count() as u32;
        paths
    } else {
        let mut paths = vec![];
        walk(&root, Path::new(""), &mut paths, &mut info, &mut 0, 0)?;
        paths
    };
    paths.sort_by(|a, b| a.as_os_str().as_encoded_bytes().cmp(b.as_os_str().as_encoded_bytes()));
    source_root_revalidate(&root, &path)?;
    Ok(Inventory { path, root, paths, info, diagnostics })
}
impl LibraryService {
    pub(super) async fn resolve_folder(&self, input: &str) -> Result<LibraryResolution, InspectionError> {
        let inventory = inventory(&self.configuration, input).await?;
        let store = self.open()?;
        let existing = self.entry(&store, &folder_id(&inventory.path))?;
        Ok(LibraryResolution { kind: LibraryInputKind::Folder, provider_id: None, provider_instance: None,
            title: inventory.path.file_name().unwrap_or_default().to_string_lossy().into_owned(),
            canonical_id: Some(inventory.info.origin_path), container_label: None,
            existing_item_id: existing.map(|e| e.summary.item_id), existing_follow_id: None, page_count: None,
            git_working_tree: Some(inventory.info.git_working_tree), file_count: Some(inventory.paths.len() as u64), diagnostics: inventory.diagnostics })
    }
    pub(super) async fn start_folder_add(&self, request: LibraryAddRequest) -> Result<LibraryOperation, InspectionError> {
        let handle = operations::runtime()?;
        let (path, _) = open_source(&self.configuration, &request.input)?;
        let store = self.open()?;
        let item_id = folder_id(&path);
        let lease = store.lease(&item_id)?;
        let (record, operation_lease) = operations::create(&store, LibraryOperationKind::Add, Some(1))?;
        let record = if let Some(target) = &request.target {
            operations::set_target(&store, &record.operation_id, target.clone())?
        } else { record };
        let service = self.clone();
        let id = record.operation_id.clone();
        let worker_store = store.clone();
        operations::spawn(handle, store, id.clone(), operation_lease, async move {
            let _lease = lease;
            if operations::cancelled(&worker_store, &id)? { return Ok(()); }
            let old = service.entry(&worker_store, &item_id)?;
            if old.is_some() && !request.refresh_existing {
                let old = old.as_ref().unwrap();
                if let Some(target) = &request.target { space::prepare_saved_item(&worker_store, &id, target, &old.summary)?; }
                operations::row(&worker_store, &id, Some(&old.summary), LibraryReportOutcome::Unchanged, Some("Already saved in Library".into()))?;
            } else {
                service.save_folder(&worker_store, &id, &path.to_string_lossy(), request.label.as_deref(), old, None, request.target.as_ref()).await?;
            }
            if let Some(target) = &request.target {
                let saved = operations::get(&worker_store, &id)?.item_ids;
                if !saved.is_empty() { service.copy_saved_items(&worker_store, &id, target, &saved).await?; }
            }
            Ok(())
        });
        Ok(record)
    }
    pub(super) async fn save_folder(&self, store: &Arc<Store>, operation: &str, input: &str,
        label: Option<&str>, old: Option<LibraryIndexEntry>, confirmed: Option<&[LibraryConflictFile]>, target: Option<&SpaceTarget>) -> Result<(), InspectionError> {
        if let Some(old) = &old {
            let check = { let _lock = store.shared()?; store.check_confirmation(old, confirmed) };
            if let Err(e) = check {
                if e.code == "library_conflict" { return self.record_conflict(store, operation, old.clone(), e.message); }
                return Err(e);
            }
        }
        let mut captured = inventory(&self.configuration, input).await?;
        let id = folder_id(&captured.path);
        let title = old.as_ref().map(|e| e.summary.title.clone()).unwrap_or_else(|| label.filter(|s| !s.trim().is_empty()).unwrap_or_else(|| captured.path.file_name().and_then(|s| s.to_str()).unwrap_or("folder")).to_owned());
        let slug: String = title.chars().map(|c| if c.is_ascii_alphanumeric() { c.to_ascii_lowercase() } else { '-' }).take(60).collect();
        let slug = slug.trim_matches('-');
        let path = old.as_ref().map(|e| e.summary.item_path.clone()).unwrap_or_else(|| format!("folders/{}-{}", if slug.is_empty() { "folder" } else { slug }, &id[7..15]));
        let stage = store.stage()?;
        let mut files = vec![];
        let mut total = 0;
        let mut limited = false;
        for relative in &captured.paths {
            if operations::cancelled(store, operation)? { return Ok(()); }
            if relative == Path::new(".cockpit-item.json") || relative.as_os_str().as_encoded_bytes().contains(&b'\\') {
                captured.info.skipped_other += 1; continue;
            }
            let source = match read_stable_source_bounded(&captured.root, relative, self.configuration.limits.library_file_bytes) {
                Ok(source) => source,
                Err(e) => {
                    match e.code.as_str() {
                        "context_snapshot_symlink" => captured.info.skipped_symlinks += 1,
                        "context_snapshot_special_file" => captured.info.skipped_special += 1,
                        "context_snapshot_hardlink" | "context_snapshot_native_binary" | "context_snapshot_file_missing" => captured.info.skipped_other += 1,
                        "context_snapshot_file_bytes" => { total += 1; limited = true; },
                        _ => return Err(e),
                    }
                    continue;
                }
            };
            total += 1;
            if limited || files.len() >= self.configuration.limits.library_folder_files as usize || captured.info.bytes + source.bytes.len() as u64 > self.configuration.limits.library_folder_bytes {
                limited = true; continue;
            }
            let (parent, leaf) = create_parent(&stage.dir, relative)?;
            atomic_write_bytes(&parent, &leaf.to_string_lossy(), &source.bytes).map_err(unavailable)?;
            captured.info.bytes += source.bytes.len() as u64;
            files.push(MarkerFile { path: relative.to_string_lossy().into_owned(), hash: source.hash, bytes: source.bytes.len() as u64 });
        }
        source_root_revalidate(&captured.root, &captured.path)?;
        captured.info.files = files.len() as u64;
        let revision = store::hash(&serde_json::to_vec(&files).map_err(unavailable)?);
        let now = timestamp();
        let equal = old.as_ref().is_some_and(|e| e.summary.revision == revision);
        let partial = limited.then(|| LibraryPartial { unit: "files".into(), have: files.len() as u64, total: Some(total), reason: "Folder capture limits reached".into() });
        let mut entry = LibraryIndexEntry { canonical_url: None, marker_hash: None, inventory: vec![], summary: LibraryItemSummary {
            item_id: id.clone(), logical_id: id, kind: LibraryItemKind::FolderCopy, provider_id: None, provider_instance: None,
            resource_type: None, canonical_id: None, container: None, parent_item_id: None, ancestors: vec![], order: None,
            title, document_path: files.first().map(|file| format!("{path}/{}", file.path)), item_path: path, source_url: None, original_url: None, source_revision: None, revision,
            state: if limited { LibraryItemState::Partial } else if old.is_some() && !equal { LibraryItemState::Changed } else { LibraryItemState::Fresh },
            partial, conflict: vec![], fetched_at: Some(now.clone()), checked_at: Some(now), follow_id: None, attachments: vec![],
            folder: Some(captured.info), diagnostics: captured.diagnostics,
        }};
        if equal && confirmed.is_none() {
            let old = old.as_ref().unwrap();
            entry.marker_hash = old.marker_hash.clone();
            entry.inventory = old.inventory.clone();
            entry.summary.fetched_at = old.summary.fetched_at.clone();
            if let Some(target) = target { space::prepare_saved_item(store, operation, target, &entry.summary)?; }
            store.update(entry.clone())?;
        } else {
            store.seal(&stage, &mut entry, files)?;
            if operations::cancelled(store, operation)? { return Ok(()); }
            if let Some(target) = target { space::prepare_saved_item(store, operation, target, &entry.summary)?; }
            if let Err(e) = store.publish(stage, entry.clone(), old.as_ref().map(|e| e.summary.revision.as_str()), confirmed) {
                if e.code == "library_conflict" {
                    if let Some(old) = old { return self.record_conflict(store, operation, old, e.message); }
                }
                return Err(e);
            }
        }
        operations::row(store, operation, Some(&entry.summary), if limited { LibraryReportOutcome::Partial } else if old.is_none() { LibraryReportOutcome::New } else if equal { LibraryReportOutcome::Unchanged } else { LibraryReportOutcome::Updated }, None)
    }
}

#[cfg(test)]
mod tests;
