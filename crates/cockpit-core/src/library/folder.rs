use super::*;
use super::folder_io::{create_parent, excluded_source_path, git_inventory, git_output,
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
    for managed in [&configuration.library_root,
        &configuration.state_root, &configuration.worktree_root, &configuration.cache_root,
        &configuration.notes_root] {
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
struct CapturedFolder {
    inventory: Inventory,
    stage: store::Stage,
    files: Vec<MarkerFile>,
    total: u64,
    limited: bool,
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
    let configuration_for_open = configuration.clone();
    let input_for_open = input.to_owned();
    let (path, root) = tokio::task::spawn_blocking(move || {
        open_source(&configuration_for_open, &input_for_open)
    }).await.map_err(unavailable)??;
    let git = git_output(configuration, &path, &["rev-parse", "--show-toplevel"]).await?;
    let is_git = git.status.success() &&
        std::str::from_utf8(&git.stdout).is_ok_and(|s| Path::new(s.trim_end()) == path);
    let mut info = LibraryFolderInfo { origin_path: path.to_string_lossy().into_owned(), git_working_tree: is_git,
        files: 0, bytes: 0, skipped_symlinks: 0, skipped_special: 0, skipped_ignored: 0, skipped_other: 0 };
    let mut diagnostics = vec![];
    let mut paths = if is_git {
        let (paths, gitlinks, excluded) = git_inventory(configuration, &path).await?;
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
    } else { vec![] };
    let (path, root, paths, info) = tokio::task::spawn_blocking(move || {
        if !is_git {
            walk(&root, Path::new(""), &mut paths, &mut info, &mut 0, 0)?;
        }
        paths.sort_by(|a, b| a.as_os_str().as_encoded_bytes().cmp(b.as_os_str().as_encoded_bytes()));
        source_root_revalidate(&root, &path)?;
        Ok::<_, InspectionError>((path, root, paths, info))
    }).await.map_err(unavailable)??;
    Ok(Inventory { path, root, paths, info, diagnostics })
}

fn capture_folder_files(
    store: &Arc<Store>,
    operation: &str,
    mut captured: Inventory,
    file_byte_limit: u64,
    file_count_limit: usize,
    folder_byte_limit: u64,
) -> Result<Option<CapturedFolder>, InspectionError> {
    let stage = store.stage()?;
    let mut files = vec![];
    let mut total = 0;
    let mut limited = false;
    for relative in &captured.paths {
        if operations::cancelled(store, operation)? { return Ok(None); }
        if relative == Path::new(".cockpit-item.json") || relative.as_os_str().as_encoded_bytes().contains(&b'\\') {
            captured.info.skipped_other += 1;
            continue;
        }
        let source = match read_stable_source_bounded(&captured.root, relative, file_byte_limit) {
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
        if limited || files.len() >= file_count_limit || captured.info.bytes + source.bytes.len() as u64 > folder_byte_limit {
            limited = true;
            continue;
        }
        let (parent, leaf) = create_parent(&stage.dir, relative)?;
        atomic_write_bytes(&parent, &leaf.to_string_lossy(), &source.bytes).map_err(unavailable)?;
        captured.info.bytes += source.bytes.len() as u64;
        files.push(MarkerFile { path: relative.to_string_lossy().into_owned(), hash: source.hash, bytes: source.bytes.len() as u64 });
    }
    source_root_revalidate(&captured.root, &captured.path)?;
    Ok(Some(CapturedFolder { inventory: captured, stage, files, total, limited }))
}
impl LibraryService {
    pub(super) async fn resolve_folder(&self, input: &str) -> Result<LibraryResolution, InspectionError> {
        let inventory = inventory(&self.configuration, input).await?;
        let store = self.open()?;
        let existing = self.entry(&store, &folder_id(&inventory.path))?;
        Ok(LibraryResolution { kind: LibraryInputKind::Folder, provider_id: None, provider_instance: None,
            title: inventory.path.file_name().unwrap_or_default().to_string_lossy().into_owned(),
            canonical_id: Some(inventory.info.origin_path), container_label: None,
            existing_item_id: existing.map(|e| e.summary.item_id), existing_follow_id: None, item_count: None, item_count_exact: true, follow_mode: None,
            git_working_tree: Some(inventory.info.git_working_tree), file_count: Some(inventory.paths.len() as u64), diagnostics: inventory.diagnostics, reference_depth: None })
    }
    pub(super) async fn start_folder_add(&self, request: LibraryAddRequest) -> Result<LibraryOperation, InspectionError> {
        let handle = operations::runtime()?;
        let configuration_for_open = self.configuration.clone();
        let input_for_open = request.input.clone();
        let (path, _) = tokio::task::spawn_blocking(move || {
            open_source(&configuration_for_open, &input_for_open)
        }).await.map_err(unavailable)??;
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
                operations::row(&worker_store, &id, Some(&old.summary), LibraryReportOutcome::Unchanged, Some("Already saved in Library".into()))?;
            } else {
                service.save_folder(&worker_store, &id, &path.to_string_lossy(), request.label.as_deref(), old, None).await?;
            }
            if let Some(target) = &request.target {
                let saved = operations::get(&worker_store, &id)?.item_ids;
                service.select_saved_items(&worker_store, &id, target, &saved).await?;
            }
            Ok(())
        });
        Ok(record)
    }
    pub(super) async fn save_folder(&self, store: &Arc<Store>, operation: &str, input: &str,
        label: Option<&str>, old: Option<LibraryIndexEntry>, confirmed: Option<&[LibraryConflictFile]>) -> Result<(), InspectionError> {
        if let Some(old) = &old {
            let check = { let _lock = store.shared()?; store.check_confirmation(old, confirmed) };
            if let Err(e) = check {
                if e.code == "library_conflict" { return self.record_conflict(store, operation, old.clone(), e.message); }
                return Err(e);
            }
        }
        let captured = inventory(&self.configuration, input).await?;
        let id = folder_id(&captured.path);
        let title = old.as_ref().map(|e| e.summary.title.clone()).unwrap_or_else(|| label.filter(|s| !s.trim().is_empty()).unwrap_or_else(|| captured.path.file_name().and_then(|s| s.to_str()).unwrap_or("folder")).to_owned());
        let leaf = super::layout::folder_leaf(&title);
        let base_path = format!("folders/{leaf}");
        let index_items = {
            let _lock = store.shared()?;
            store.index()?.items
        };
        let path = if super::layout::is_reserved(&leaf, false)
            || index_items.iter().any(|item| {
                item.summary.item_id != id && item.summary.item_path == base_path
            })
        {
            format!("folders/{}", super::layout::tagged(&leaf, id.strip_prefix("folder:").unwrap_or(&id)))
        } else {
            base_path
        };
        let capture_store = store.clone();
        let capture_operation = operation.to_owned();
        let file_byte_limit = self.configuration.limits.library_file_bytes;
        let file_count_limit = self.configuration.limits.library_folder_files as usize;
        let folder_byte_limit = self.configuration.limits.library_folder_bytes;
        let Some(capture) = tokio::task::spawn_blocking(move || capture_folder_files(
            &capture_store, &capture_operation, captured,
            file_byte_limit, file_count_limit, folder_byte_limit,
        )).await.map_err(unavailable)?? else {
            return Ok(());
        };
        let mut captured = capture.inventory;
        let stage = capture.stage;
        let files = capture.files;
        let total = capture.total;
        let limited = capture.limited;
        captured.info.files = files.len() as u64;
        let revision = store::hash(&serde_json::to_vec(&files).map_err(unavailable)?);
        let now = timestamp();
        let equal = old.as_ref().is_some_and(|e| e.summary.revision == revision);
        let partial = limited.then(|| LibraryPartial { unit: "files".into(), have: files.len() as u64, total: Some(total), reason: "Folder capture limits reached".into() });
        let mut entry = LibraryIndexEntry { canonical_url: None, inventory: vec![], references: None, relations_captured: false, summary: LibraryItemSummary {
            item_id: id.clone(), logical_id: id, kind: LibraryItemKind::FolderCopy, provider_id: None, provider_instance: None,
            resource_type: None, canonical_id: None, container: None, parent_item_id: None, ancestors: vec![], order: None,
            title, document_path: files.first().map(|file| format!("{path}/{}", file.path)), item_path: path, source_url: None, original_url: None, source_revision: None, revision,
            state: if limited { LibraryItemState::Partial } else if old.is_some() && !equal { LibraryItemState::Changed } else { LibraryItemState::Fresh },
            partial, conflict: vec![], fetched_at: Some(now.clone()), checked_at: Some(now), refs: vec![LibraryItemRef::Manual], purge_after: None, issue: None, attachments: vec![],
            folder: Some(captured.info), diagnostics: captured.diagnostics, reference_depth: None, included_by: None,
        }};
        if equal && confirmed.is_none() {
            let old = old.as_ref().unwrap();
            entry.inventory = old.inventory.clone();
            entry.summary.fetched_at = old.summary.fetched_at.clone();
            store.update(entry.clone())?;
        } else {
            store.seal(&stage, &mut entry, files)?;
            if operations::cancelled(store, operation)? { return Ok(()); }
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

#[cfg(test)]
mod notes_root_tests {
    use super::*;

    #[test]
    fn imports_refuse_notes_root_ancestors_and_descendants() {
        let fixture = crate::library::tests::fixture();
        let source = fixture.root.join("source");
        let notes = source.join("notes");
        std::fs::create_dir_all(notes.join("child")).expect("Notes test directories");
        let mut configuration = fixture.service.configuration.clone();
        configuration.notes_root = notes.to_string_lossy().into_owned();
        for path in [&source, &notes, &notes.join("child")] {
            let error = open_source(&configuration, &path.to_string_lossy())
                .expect_err("Notes overlap must be refused");
            assert_eq!(error.code, "library_folder_refused");
        }
        let sibling = source.join("notes-sibling");
        std::fs::create_dir(&sibling).expect("independent source");
        assert!(open_source(&configuration, &sibling.to_string_lossy()).is_ok());
    }
}
