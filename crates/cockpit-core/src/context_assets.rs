use std::collections::BTreeSet;
use std::io::{ErrorKind, Read, Write};
use std::path::{Component, Path, PathBuf};
use std::time::Duration;

use cap_fs_ext::{DirExt, OpenOptionsFollowExt, OpenOptionsSyncExt};
use cap_std::fs::{Dir, Metadata, OpenOptions};
use cockpit_protocol::projects::ProjectConfiguration;
use cockpit_protocol::library::{
    LibraryConflictFile, LibraryFollowSummary, LibraryItemKind, LibraryItemState,
    LibraryItemSummary, SpaceCopyMode, SpaceCopyRow, SpaceCopyState, SpaceFollowSummary,
};
use fs2::FileExt;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use tokio::process::Command;
use uuid::Uuid;

use crate::InspectionError;
use crate::process::run_bounded_command;
use crate::project_store::{CompanionManifest, read_json_bounded, timestamp};
#[cfg(test)]
use crate::project_store::atomic_write_json;

const MANIFEST_NAME: &str = "context-manifest.json";
const MANIFEST_SCHEMA_VERSION: u32 = 2;
const PENDING_SOURCE_INTENT_SCHEMA_VERSION: u32 = 1;
const MAX_MANIFEST_BYTES: u64 = 2 * 1024 * 1024;
const MAX_SNAPSHOT_FILE_BYTES: usize = 4 * 1024 * 1024;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct ContextManifest {
    schema_version: u32,
    companion_id: String,
    owner_workspace_id: String,
    owner_worktree_path: String,
    primary_repository_identity: String,
    entries: Vec<ContextManifestEntry>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    library_follows: Vec<SpaceFollowRecord>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    library_copies: Vec<LibraryCopyRecord>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pending_source_intent: Option<PendingSourceIntent>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pending_library_remove: Option<ContextManifestEntry>,
    updated_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct SpaceFollowRecord {
    follow_id: String,
    known_page_item_ids: Vec<String>,
    added_at: String,
    updated_at: String,
}

/// The complete item inventory is persisted with the first file's write-ahead
/// intent, so an interrupted multi-file copy cannot masquerade as complete.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct LibraryCopyRecord {
    item_id: String,
    files: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    logical_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    revision: Option<String>,
}

#[cfg(test)]
thread_local! {
    static LIBRARY_COPY_FAIL_AFTER_FILES: std::cell::Cell<Option<usize>> = const { std::cell::Cell::new(None) };
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct ContextManifestEntry {
    logical_id: String,
    relative_path: String,
    kind: String,
    source: String,
    generated: bool,
    revision: String,
    content_hash: String,
    bytes: u64,
    copy_mode: String,
    status: String,
    updated_at: String,
    source_repository_id: String,
    source_checkout_path: String,
    source_identity: String,
    source_hash_before: String,
    source_hash_after: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    library_item_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    library_revision: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    library_file: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    library_follow_id: Option<String>,
}

/// A durable, in-manifest write-ahead record for one generated source
/// replacement. The companion lock allows only one pending source publish.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct PendingSourceIntent {
    schema_version: u32,
    relative_path: String,
    previous_written_hash: Option<String>,
    previous_entry: Option<ContextManifestEntry>,
    new_written_hash: String,
    intended_entry: ContextManifestEntry,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    expected_previous_hash: Option<String>,
}

/// A verified Library revision, held under the caller's shared Library lock.
pub(crate) struct LibraryItemView<'a> {
    pub root: &'a Dir,
    pub summary: &'a LibraryItemSummary,
    pub files: &'a [crate::library::store::MarkerFile],
}

impl LibraryItemView<'_> {
    fn files(&self) -> impl Iterator<Item = &crate::library::store::MarkerFile> {
        self.files.iter().filter(|file| file.path != ".cockpit-item.json" && file.hash != "directory")
    }
}

pub(crate) enum LibraryCopyMode<'a> {
    NewOnly,
    Update,
    Replace { confirmed: &'a [LibraryConflictFile] },
}

pub(crate) struct LibraryCopyResult {
    pub written: Vec<String>,
    pub skipped_edited: Vec<String>,
    pub copy_mode: Option<SpaceCopyMode>,
}

fn aggregate_copy_mode(modes: impl Iterator<Item = SpaceCopyMode>) -> Option<SpaceCopyMode> {
    modes.reduce(|left, right| if left == right { left } else { SpaceCopyMode::Mixed })
}

fn library_destination(item: &LibraryItemView<'_>, file: &str, file_count: usize) -> String {
    let summary = item.summary;
    let canonical = readable_name(summary.canonical_id.as_deref().unwrap_or(&summary.item_id));
    if summary.kind == LibraryItemKind::FolderCopy {
        return format!("{}/{file}", summary.item_path);
    }
    let base = format!("sources/{}/{}",
        readable_name(summary.provider_id.as_deref().unwrap_or("unknown")),
        readable_name(summary.resource_type.as_deref().unwrap_or("document")));
    if file_count == 1 && summary.resource_type.as_deref() != Some("page") {
        format!("{base}/{canonical}.md")
    } else {
        let container = summary.container.as_ref()
            .map(|c| format!("{}/", readable_name(&c.container_id))).unwrap_or_default();
        format!("{base}/{container}{canonical}-{}/{file}", readable_name(&summary.title))
    }
}

/// NewOnly never updates an existing linked revision. Explicit re-add can link
/// a legacy source or restore a missing file. Copies use the durable intent path.
/// `follow_id` names the Space follow a new copy belongs to; an existing linked
/// entry keeps its own follow membership, so item and follow updates never move it.
pub(crate) fn materialize_library_item(
    root: &Dir, companion_id: &str, item: &LibraryItemView<'_>, mode: LibraryCopyMode<'_>,
    follow_id: Option<&str>,
) -> Result<LibraryCopyResult, InspectionError> {
    let _lock = acquire_companion_lock(root)?;
    let association = read_companion_association(root)?;
    let mut manifest = read_manifest(root, companion_id, &association)?;
    recover_pending_source_intent(root, &mut manifest)?;
    let updating = !matches!(mode, LibraryCopyMode::NewOnly);
    let file_count = item.files().count();
    let linked = manifest.entries.iter().filter(|entry|
        entry.library_item_id.as_deref() == Some(item.summary.item_id.as_str())).collect::<Vec<_>>();
    if updating && (item.summary.state == LibraryItemState::RemovedAtSource
        || (linked.is_empty() && !manifest.library_copies.iter().any(|copy| copy.item_id == item.summary.item_id))) {
        return Ok(LibraryCopyResult { written: vec![], skipped_edited: vec![], copy_mode: None });
    }
    let confirmed = match &mode {
        LibraryCopyMode::Replace { confirmed } => *confirmed,
        _ => &[],
    };
    // Validate every confirmation before changing any selected file. A confirmation
    // authorizes one exact path and hash, never all files belonging to the item.
    for expected in confirmed {
        if !linked.iter().find(|entry| entry.relative_path == expected.path)
            .is_some_and(|entry| read_space_file(root, entry)
                .is_ok_and(|file| file.hash == expected.current_hash))
        {
            return Err(space_copy_conflict());
        }
    }
    let mut written = Vec::new();
    let mut skipped_edited = Vec::new();
    let mut modes = Vec::new();
    let inventory_changed = !manifest.library_copies.iter().any(|copy| copy.item_id == item.summary.item_id
        && copy.files.iter().map(String::as_str).eq(item.files().map(|file| file.path.as_str()))
        && copy.revision.as_deref() == Some(item.summary.revision.as_str()));
    if inventory_changed {
        manifest.library_copies.retain(|copy| copy.item_id != item.summary.item_id);
        manifest.library_copies.push(LibraryCopyRecord {
            item_id: item.summary.item_id.clone(),
            files: item.files().map(|file| file.path.clone()).collect(),
            logical_id: Some(item.summary.logical_id.clone()),
            revision: Some(item.summary.revision.clone()),
        });
    }
    for file in item.files() {
        let previous = manifest.entries.iter().find(|entry|
            entry.library_item_id.as_deref() == Some(item.summary.item_id.as_str())
                && entry.library_file.as_deref() == Some(file.path.as_str())).cloned();
        let logical_id = previous.as_ref().map(|entry| entry.logical_id.clone()).unwrap_or_else(|| {
            if item.summary.kind != LibraryItemKind::FolderCopy && (file.path == "document.md" || file_count == 1) {
                item.summary.logical_id.clone()
            } else { format!("{}#{}", item.summary.logical_id, file.path) }
        });
        let previous = previous.or_else(|| manifest.entries.iter()
            .find(|entry| entry.logical_id == logical_id && entry.library_item_id.is_none()).cloned());
        let mut missing = false;
        let mut expected_previous_hash = None;
        if let Some(entry) = &previous {
            match read_space_file(root, entry) {
                Ok(current) => {
                    if let Some(expected_hash) = confirmed.iter().find(|file| file.path == entry.relative_path) {
                        if expected_hash.path != entry.relative_path || expected_hash.current_hash != current.hash {
                            return Err(space_copy_conflict());
                        }
                        expected_previous_hash = Some(current.hash.clone());
                    } else if current.hash != entry.content_hash {
                        if updating {
                            skipped_edited.push(entry.relative_path.clone());
                            continue;
                        }
                        return Err(InspectionError::new("source_sync_conflict", "An edited Space copy will not be overwritten"));
                    }
                    if entry.library_item_id.is_some() {
                        if entry.library_item_id.as_deref() != Some(item.summary.item_id.as_str())
                            || entry.library_revision.as_deref() != Some(item.summary.revision.as_str())
                            || entry.content_hash != file.hash
                        {
                            if !updating {
                                return Err(InspectionError::new("source_sync_conflict", "An existing Space revision requires an explicit update"));
                            }
                        }
                        if current.hash == file.hash {
                            modes.extend(match entry.copy_mode.as_str() {
                                "reflink" => Some(SpaceCopyMode::Reflink),
                                "copy" => Some(SpaceCopyMode::Copy), _ => None,
                            });
                            if updating && (entry.library_revision.as_deref() != Some(item.summary.revision.as_str())
                                || entry.content_hash != file.hash)
                            {
                                let mut adopted = entry.clone();
                                adopted.library_revision = Some(item.summary.revision.clone());
                                adopted.revision = item.summary.revision.clone();
                                adopted.content_hash = file.hash.clone();
                                adopted.bytes = file.bytes;
                                adopted.source_hash_before = file.hash.clone();
                                adopted.source_hash_after = file.hash.clone();
                                replace_source_manifest_entry(&mut manifest, adopted);
                                write_manifest_durable(root, &manifest)?;
                            }
                            continue;
                        }
                    }
                }
                Err(error) if error.code == "context_snapshot_file_missing" => {
                    if confirmed.iter().any(|file| file.path == entry.relative_path) { return Err(space_copy_conflict()); }
                    missing = true;
                }
                Err(error) => return Err(error),
            }
        }
        let source_path = safe_source_relative(&file.path)?;
        let source = read_stable_source_bounded(item.root, &source_path, file.bytes)?;
        if source.hash != file.hash || source.bytes.len() as u64 != file.bytes {
            return Err(InspectionError::new("library_conflict", "Library file differs from its recorded revision"));
        }
        let relative = if let Some(entry) = &previous { entry.relative_path.clone() } else {
            let candidate = library_destination(item, &file.path, file_count);
            if manifest.entries.iter().any(|entry| entry.relative_path == candidate)
                || root.symlink_metadata(&candidate).is_ok()
            {
                if file_count != 1 || item.summary.kind == LibraryItemKind::FolderCopy { return Err(source_publish_conflict()); }
                format!("{}-{}.md", candidate.trim_end_matches(".md"), short_hash(logical_id.as_bytes()))
            } else { candidate }
        };
        let path = safe_companion_relative(&relative)?;
        let (parent, leaf) = create_parent(root, &path)?;
        let temporary = format!(".source-{}.tmp", Uuid::new_v4());
        let mut options = OpenOptions::new();
        options.write(true).create_new(true).follow(cap_fs_ext::FollowSymlinks::No);
        let mut destination = parent.open_with(&temporary, &options)
            .map_err(io_error("source_materialize_failed"))?;
        let cloned = (|| {
            let source_file = open_source_for_clone(item.root, &source_path, &source.identity)?;
            let mode = match reflink(&destination, &source_file) {
                Ok(()) => SpaceCopyMode::Reflink,
                Err(error) if reflink_fallback(&error) => {
                    destination.write_all(&source.bytes).map_err(io_error("source_materialize_failed"))?;
                    SpaceCopyMode::Copy
                }
                Err(error) => return Err(io_error("source_materialize_failed")(error)),
            };
            destination.sync_all().map_err(io_error("source_materialize_failed"))?;
            if read_regular_bounded(&parent, Path::new(&temporary), file.bytes)?.hash != file.hash {
                return Err(InspectionError::new("library_conflict", "Library source changed during copy"));
            }
            Ok(mode)
        })();
        let copy_mode = match cloned {
            Ok(mode) => mode,
            Err(error) => { let _ = parent.remove_file(&temporary); return Err(error); }
        };
        let intended = ContextManifestEntry {
            logical_id, relative_path: relative.clone(),
            kind: item.summary.resource_type.clone().unwrap_or_else(|| "folder".into()),
            source: item.summary.provider_id.clone().unwrap_or_else(|| "local".into()),
            generated: true, revision: item.summary.revision.clone(),
            content_hash: file.hash.clone(), bytes: file.bytes,
            copy_mode: match copy_mode { SpaceCopyMode::Reflink => "reflink", _ => "copy" }.into(),
            status: "complete".into(), updated_at: timestamp(),
            source_repository_id: item.summary.provider_instance.clone().unwrap_or_default(),
            source_checkout_path: String::new(),
            source_identity: item.summary.canonical_id.clone().unwrap_or_default(),
            source_hash_before: file.hash.clone(), source_hash_after: file.hash.clone(),
            library_item_id: Some(item.summary.item_id.clone()),
            library_revision: Some(item.summary.revision.clone()),
            library_file: Some(file.path.clone()),
            library_follow_id: previous.as_ref().filter(|entry| entry.library_item_id.is_some())
                .map_or_else(|| follow_id.map(str::to_owned), |entry| entry.library_follow_id.clone()),
        };
        manifest.pending_source_intent = Some(PendingSourceIntent {
            schema_version: PENDING_SOURCE_INTENT_SCHEMA_VERSION, relative_path: relative.clone(),
            previous_written_hash: previous.as_ref().map(|entry| entry.content_hash.clone()),
            previous_entry: previous.clone(), new_written_hash: file.hash.clone(),
            intended_entry: intended.clone(),
            expected_previous_hash: expected_previous_hash.clone(),
        });
        write_manifest_durable(root, &manifest)?;
        let mut expected_entry = previous.clone();
        if let (Some(entry), Some(expected)) = (&mut expected_entry, expected_previous_hash) {
            entry.content_hash = expected;
        }
        let recheck = if missing {
            recheck_source_destination(root, &path, &parent, &leaf, &None)
        } else { recheck_source_destination(root, &path, &parent, &leaf, &expected_entry) };
        if let Err(error) = recheck {
            discard_pending_source_intent(root, &mut manifest, &parent, &temporary)?;
            return Err(if updating && error.code == "source_sync_conflict" { space_copy_conflict() } else { error });
        }
        parent.rename(&temporary, &parent, &leaf).map_err(io_error("source_materialize_failed"))?;
        sync_directory(&parent).map_err(io_error("source_materialize_failed"))?;
        replace_source_manifest_entry(&mut manifest, intended);
        manifest.pending_source_intent = None;
        manifest.updated_at = timestamp();
        write_manifest_durable(root, &manifest)?;
        written.push(relative);
        modes.push(copy_mode);
        #[cfg(test)]
        if LIBRARY_COPY_FAIL_AFTER_FILES.with(|fault| {
            if fault.get() == Some(written.len()) { fault.set(None); true } else { false }
        }) {
            return Err(InspectionError::new("library_test_crash", "Space file published before remaining files"));
        }
    }
    if updating {
        let obsolete = manifest.entries.iter().filter(|entry|
            entry.library_item_id.as_deref() == Some(item.summary.item_id.as_str())
                && !item.files().any(|file| entry.library_file.as_deref() == Some(file.path.as_str())))
            .cloned().collect::<Vec<_>>();
        for entry in obsolete {
            // Update never discards an edited Space-only file, even when the
            // replacement dialog confirmed edits elsewhere in this item.
            match library_remove_expected(root, &entry, &[]) {
                Ok(expected) => {
                    remove_library_entry(root, &mut manifest, &entry, expected.as_deref())?;
                    written.push(entry.relative_path);
                }
                Err(error) if error.code == "space_copy_conflict" => skipped_edited.push(entry.relative_path),
                Err(error) => return Err(error),
            }
        }
    }
    if file_count == 0 && inventory_changed {
        write_manifest_durable(root, &manifest)?;
    }
    Ok(LibraryCopyResult { written, skipped_edited, copy_mode: aggregate_copy_mode(modes.into_iter()) })
}

fn space_copy_conflict() -> InspectionError {
    InspectionError::new("space_copy_conflict", "The Space copy changed; reload it before confirming")
}

/// Remove manifest-owned files; all edited bytes require exact confirmations.
pub(crate) fn remove_library_copy(
    root: &Dir, companion_id: &str, logical_id: &str, confirmed: &[LibraryConflictFile],
) -> Result<(), InspectionError> {
    let _lock = acquire_companion_lock(root)?;
    let association = read_companion_association(root)?;
    let mut manifest = read_manifest(root, companion_id, &association)?;
    recover_pending_source_intent(root, &mut manifest)?;
    let entry = manifest.entries.iter().find(|entry| entry.logical_id == logical_id
        || entry.library_file.as_deref().is_some_and(|file| entry.logical_id.strip_suffix(file)
            .and_then(|prefix| prefix.strip_suffix('#')) == Some(logical_id))).cloned();
    let Some(entry) = entry else {
        let position = manifest.library_copies.iter().position(|copy|
            copy.logical_id.as_deref().unwrap_or(&copy.item_id) == logical_id)
            .ok_or_else(|| InspectionError::new("library_item_not_found", "Space copy does not exist"))?;
        if !confirmed.is_empty() { return Err(space_copy_conflict()); }
        manifest.library_copies.remove(position);
        return write_manifest_durable(root, &manifest);
    };
    let entries = manifest.entries.iter().filter(|candidate|
        if let Some(id) = &entry.library_item_id { candidate.library_item_id.as_ref() == Some(id) }
        else { candidate.logical_id == entry.logical_id }).cloned().collect::<Vec<_>>();
    if confirmed.iter().any(|file| !entries.iter().any(|entry| entry.relative_path == file.path)) {
        return Err(space_copy_conflict());
    }
    let expected = entries.iter().map(|entry| library_remove_expected(root, entry, confirmed))
        .collect::<Result<Vec<_>, _>>()?;
    for (entry, expected) in entries.iter().zip(expected) {
        remove_library_entry(root, &mut manifest, entry, expected.as_deref())?;
    }
    if manifest.library_copies.iter().any(|copy| Some(&copy.item_id) == entry.library_item_id.as_ref()) {
        manifest.library_copies.retain(|copy| Some(&copy.item_id) != entry.library_item_id.as_ref());
        write_manifest_durable(root, &manifest)?;
    }
    Ok(())
}

fn library_remove_expected(
    root: &Dir, entry: &ContextManifestEntry, confirmed: &[LibraryConflictFile],
) -> Result<Option<String>, InspectionError> {
    let current = match read_space_file(root, entry) {
        Ok(file) => Some(file),
        Err(error) if error.code == "context_snapshot_file_missing" => None,
        Err(error) => return Err(error),
    };
    let confirmation = confirmed.iter().find(|file| file.path == entry.relative_path);
    let expected = confirmation.map(|file| file.current_hash.as_str()).unwrap_or(&entry.content_hash);
    if current.as_ref().is_some_and(|file| file.hash != expected)
        || (current.is_none() && confirmation.is_some())
    {
        return Err(space_copy_conflict());
    }
    Ok(current.map(|_| expected.to_owned()))
}

fn remove_library_entry(
    root: &Dir, manifest: &mut ContextManifest, entry: &ContextManifestEntry, expected: Option<&str>,
) -> Result<(), InspectionError> {
    let path = safe_companion_relative(&entry.relative_path)?;
    manifest.pending_library_remove = Some(entry.clone());
    write_manifest_durable(root, manifest)?;
    if let Some(expected) = expected {
        let (parent, leaf) = resolve_parent(root, &path).map_err(|_| space_copy_conflict())?;
        let mut expected_entry = entry.clone();
        expected_entry.content_hash = expected.into();
        if recheck_source_destination(root, &path, &parent, &leaf, &Some(expected_entry)).is_err() {
            manifest.pending_library_remove = None;
            write_manifest_durable(root, manifest)?;
            return Err(space_copy_conflict());
        }
        parent.remove_file(&leaf).map_err(io_error("source_materialize_failed"))?;
        sync_directory(&parent).map_err(io_error("source_materialize_failed"))?;
    } else if !matches!(read_space_file(root, entry), Err(error) if error.code == "context_snapshot_file_missing") {
        manifest.pending_library_remove = None;
        write_manifest_durable(root, manifest)?;
        return Err(space_copy_conflict());
    }
    finish_library_remove(manifest, entry);
    write_manifest_durable(root, manifest)
}

fn finish_library_remove(manifest: &mut ContextManifest, entry: &ContextManifestEntry) {
    manifest.entries.retain(|candidate| candidate.logical_id != entry.logical_id);
    if let Some(id) = &entry.library_item_id {
        if manifest.entries.iter().any(|candidate| candidate.library_item_id.as_ref() == Some(id)) {
            if let Some(copy) = manifest.library_copies.iter_mut().find(|copy| &copy.item_id == id) {
                copy.files.retain(|file| Some(file) != entry.library_file.as_ref());
            }
        } else {
            manifest.library_copies.retain(|copy| &copy.item_id != id || copy.files.is_empty());
        }
    }
    manifest.pending_library_remove = None;
    manifest.updated_at = timestamp();
}
/// D23 precedence applies identically to a file and the multi-file aggregate.
fn space_copy_state(
    missing: bool, edited: bool, linked: bool,
    library: Option<&LibraryItemSummary>, library_newer: bool,
) -> SpaceCopyState {
    if missing { SpaceCopyState::MissingInSpace }
    else if edited { SpaceCopyState::EditedInSpace }
    else if !linked { SpaceCopyState::NotLinked }
    else if library.is_none() { SpaceCopyState::NotInLibrary }
    else if library.is_some_and(|item| item.state == LibraryItemState::RemovedAtSource) {
        SpaceCopyState::RemovedAtSource
    } else if library_newer { SpaceCopyState::LibraryNewer }
    else { SpaceCopyState::UpToDate }
}

/// One row per unfollowed item, plus one aggregate row per Space follow whose
/// pages are counted per D8 (`known_page_item_ids`) rather than listed.
pub(crate) fn library_space_rows(
    root: &Dir, companion_id: &str, library: &[LibraryItemSummary], follows: &[LibraryFollowSummary],
) -> Result<Vec<SpaceCopyRow>, InspectionError> {
    let association = read_companion_association(root)?;
    // Listing never writes, including v1 manifests and interrupted intents.
    let manifest = read_manifest(root, companion_id, &association)?;
    let mut groups = std::collections::BTreeMap::<String, Vec<&ContextManifestEntry>>::new();
    for entry in &manifest.entries {
        if entry.status == "skipped_gitlink" { continue; }
        let key = entry.library_item_id.clone().unwrap_or_else(|| entry.logical_id.clone());
        groups.entry(key).or_default().push(entry);
    }
    let mut rows = Vec::new();
    let mut follow_pages = std::collections::BTreeMap::<String, Vec<SpaceCopyRow>>::new();
    for entries in groups.values() {
        let first = entries.iter().copied().find(|entry| entry.library_file.as_deref() == Some("document.md"))
            .unwrap_or(entries[0]);
        let current = first.library_item_id.as_ref()
            .and_then(|id| library.iter().find(|item| &item.item_id == id));
        let newer = current.is_some_and(|item| entries.iter()
            .any(|entry| entry.library_revision.as_deref() != Some(item.revision.as_str())));
        let mut missing = manifest.library_copies.iter()
            .find(|copy| Some(&copy.item_id) == first.library_item_id.as_ref())
            .is_some_and(|copy| copy.files.iter().any(|file| !entries.iter()
                .any(|entry| entry.library_file.as_ref() == Some(file))));
        let mut edited = Vec::new();
        for entry in entries {
            match read_space_file(root, entry) {
                Ok(file) if file.hash != entry.content_hash => edited.push(LibraryConflictFile {
                    path: entry.relative_path.clone(), current_hash: file.hash,
                }),
                Ok(_) => {}
                Err(error) if error.code == "context_snapshot_file_missing" => missing = true,
                Err(error) => return Err(error),
            }
        }
        let row = SpaceCopyRow {
            item_id: first.library_item_id.clone(),
            logical_id: current.map(|item| item.logical_id.clone()).unwrap_or_else(|| {
                first.library_file.as_ref()
                    .and_then(|file| first.logical_id.strip_suffix(file.as_str()).and_then(|prefix| prefix.strip_suffix('#')))
                    .unwrap_or(&first.logical_id).into()
            }),
            title: current.map(|item| item.title.clone()).unwrap_or_else(|| first.source_identity.clone()),
            provider_id: Some(first.source.clone()), resource_type: Some(first.kind.clone()),
            kind: current.map(|item| item.kind).unwrap_or(LibraryItemKind::ProviderSnapshot),
            state: space_copy_state(missing, !edited.is_empty(), first.library_item_id.is_some(), current, newer),
            library_newer: newer,
            paths: entries.iter().map(|entry| entry.relative_path.clone()).collect(), edited,
            copy_mode: aggregate_copy_mode(entries.iter().filter_map(|entry| match entry.copy_mode.as_str() {
                "reflink" => Some(SpaceCopyMode::Reflink), "copy" => Some(SpaceCopyMode::Copy), _ => None,
            })),
            library_revision_copied: first.library_revision.clone(),
            current_library_revision: current.map(|item| item.revision.clone()), follow: None,
        };
        match &first.library_follow_id {
            Some(follow_id) => follow_pages.entry(follow_id.clone()).or_default().push(row),
            None => rows.push(row),
        }
    }
    // Empty folders and an interrupted first-file publication still have a
    // durable item inventory, even though no per-file row exists yet.
    for copy in manifest.library_copies.iter().filter(|copy| !groups.contains_key(&copy.item_id)) {
        let current = library.iter().find(|item| item.item_id == copy.item_id);
        let newer = current.is_some_and(|item| copy.revision.as_deref() != Some(item.revision.as_str()));
        let row = SpaceCopyRow {
            item_id: Some(copy.item_id.clone()),
            logical_id: current.map(|item| &item.logical_id).or(copy.logical_id.as_ref()).unwrap_or(&copy.item_id).clone(),
            title: current.map(|item| &item.title).unwrap_or(&copy.item_id).clone(),
            provider_id: current.and_then(|item| item.provider_id.clone()),
            resource_type: current.and_then(|item| item.resource_type.clone()),
            kind: current.map(|item| item.kind).unwrap_or(LibraryItemKind::FolderCopy),
            state: space_copy_state(!copy.files.is_empty(), false, true, current, newer),
            library_newer: newer, paths: vec![], edited: vec![], copy_mode: None,
            library_revision_copied: copy.revision.clone(),
            current_library_revision: current.map(|item| item.revision.clone()), follow: None,
        };
        let follow = manifest.library_follows.iter()
            .find(|record| record.known_page_item_ids.contains(&copy.item_id));
        match follow {
            Some(record) => follow_pages.entry(record.follow_id.clone()).or_default().push(row),
            None => rows.push(row),
        }
    }
    let mut follow_ids = manifest.library_follows.iter().map(|record| record.follow_id.clone())
        .collect::<BTreeSet<_>>();
    follow_ids.extend(follow_pages.keys().cloned());
    for follow_id in follow_ids {
        let pages = follow_pages.remove(&follow_id).unwrap_or_default();
        let known = manifest.library_follows.iter().find(|record| record.follow_id == follow_id)
            .map(|record| record.known_page_item_ids.as_slice()).unwrap_or_default();
        rows.push(follow_space_row(&follow_id, pages, known, library,
            follows.iter().find(|follow| follow.follow_id == follow_id)));
    }
    rows.sort_by(|a, b| {
        let rank = |state| match state { SpaceCopyState::UpToDate => 1, SpaceCopyState::NotLinked => 2, _ => 0 };
        (rank(a.state), &a.title, &a.logical_id).cmp(&(rank(b.state), &b.title, &b.logical_id))
    });
    Ok(rows)
}

/// D8 aggregate: `new` pages are the follow's current Library pages this Space
/// was never offered; `changed` pages are unedited copies behind or missing.
fn follow_space_row(
    follow_id: &str, pages: Vec<SpaceCopyRow>, known: &[String], library: &[LibraryItemSummary],
    follow: Option<&LibraryFollowSummary>,
) -> SpaceCopyRow {
    let new_pages = library.iter().filter(|item| item.follow_id.as_deref() == Some(follow_id)
        && item.state != LibraryItemState::RemovedAtSource
        && !known.contains(&item.item_id)
        && !pages.iter().any(|page| page.item_id.as_ref() == Some(&item.item_id))).count() as u32;
    let count = |state| pages.iter().filter(|page| page.state == state).count() as u32;
    let changed_pages = count(SpaceCopyState::LibraryNewer) + count(SpaceCopyState::MissingInSpace);
    let edited_pages = count(SpaceCopyState::EditedInSpace);
    let library_newer = new_pages + changed_pages > 0;
    let state = if follow.is_none() { SpaceCopyState::NotInLibrary }
        else if edited_pages > 0 { SpaceCopyState::EditedInSpace }
        else if library_newer { SpaceCopyState::LibraryNewer }
        else { SpaceCopyState::UpToDate };
    SpaceCopyRow {
        item_id: None,
        logical_id: follow_id.to_owned(),
        title: follow.map(|follow| format!("{} · {}", follow.space_key, follow.space_name))
            .unwrap_or_else(|| follow_id.to_owned()),
        provider_id: follow.map(|follow| follow.provider_id.clone())
            .or_else(|| pages.iter().find_map(|page| page.provider_id.clone())),
        resource_type: Some("space".into()),
        kind: LibraryItemKind::ProviderSnapshot,
        state,
        library_newer,
        paths: pages.iter().flat_map(|page| page.paths.iter().cloned()).collect(),
        edited: pages.iter().flat_map(|page| page.edited.iter().cloned()).collect(),
        copy_mode: aggregate_copy_mode(pages.iter().filter_map(|page| page.copy_mode)),
        library_revision_copied: None,
        current_library_revision: None,
        follow: Some(SpaceFollowSummary {
            follow_id: follow_id.to_owned(),
            space_key: follow.map(|follow| follow.space_key.clone()).unwrap_or_default(),
            page_count: pages.len() as u32,
            new_pages,
            changed_pages,
            edited_pages,
            removed_at_source_pages: count(SpaceCopyState::RemovedAtSource),
        }),
    }
}

/// One follow's copies in a Space, read without writing.
pub(crate) struct SpaceFollowCopies {
    pub known: Vec<String>,
    /// Library item ids copied as this follow's pages, with their Space paths.
    pub pages: Vec<(String, Vec<String>)>,
    /// Every Library item id with a copy in this Space, followed or not.
    pub linked: BTreeSet<String>,
}

pub(crate) fn library_follow_copies(
    root: &Dir, companion_id: &str, follow_id: &str,
) -> Result<SpaceFollowCopies, InspectionError> {
    let association = read_companion_association(root)?;
    let manifest = read_manifest(root, companion_id, &association)?;
    let known = manifest.library_follows.iter().find(|record| record.follow_id == follow_id)
        .map(|record| record.known_page_item_ids.clone());
    let mut pages = std::collections::BTreeMap::<String, Vec<String>>::new();
    for entry in &manifest.entries {
        if let (Some(id), Some(follow)) = (&entry.library_item_id, &entry.library_follow_id) {
            if follow == follow_id {
                pages.entry(id.clone()).or_default().push(entry.relative_path.clone());
            }
        }
    }
    let mut linked = manifest.entries.iter().filter_map(|entry| entry.library_item_id.clone())
        .collect::<BTreeSet<_>>();
    for copy in &manifest.library_copies {
        linked.insert(copy.item_id.clone());
        // An interrupted first write of a known page is still this follow's page.
        if known.as_ref().is_some_and(|known| known.contains(&copy.item_id)) {
            pages.entry(copy.item_id.clone()).or_default();
        }
    }
    Ok(SpaceFollowCopies {
        known: known.unwrap_or_default(),
        pages: pages.into_iter().collect(),
        linked,
    })
}

/// Record that the Space was offered these follow pages (written or already
/// present). Creates the follow record; never removes known ids.
pub(crate) fn record_follow_pages(
    root: &Dir, companion_id: &str, follow_id: &str, item_ids: &[String],
) -> Result<(), InspectionError> {
    let _lock = acquire_companion_lock(root)?;
    let association = read_companion_association(root)?;
    let mut manifest = read_manifest(root, companion_id, &association)?;
    recover_pending_source_intent(root, &mut manifest)?;
    let now = timestamp();
    let mut created = false;
    let position = match manifest.library_follows.iter().position(|record| record.follow_id == follow_id) {
        Some(position) => position,
        None => {
            created = true;
            manifest.library_follows.push(SpaceFollowRecord {
                follow_id: follow_id.to_owned(), known_page_item_ids: vec![],
                added_at: now.clone(), updated_at: now.clone(),
            });
            manifest.library_follows.len() - 1
        }
    };
    let record = &mut manifest.library_follows[position];
    let before = record.known_page_item_ids.len();
    for id in item_ids {
        if !record.known_page_item_ids.contains(id) {
            record.known_page_item_ids.push(id.clone());
        }
    }
    if record.known_page_item_ids.len() == before && !created {
        return Ok(());
    }
    record.updated_at = now.clone();
    manifest.updated_at = now;
    write_manifest_durable(root, &manifest)
}

/// Remove every copy of a follow's pages (edited files only with an exact
/// confirmation) and the Space's follow record. Other entries are untouched.
pub(crate) fn remove_follow_copy(
    root: &Dir, companion_id: &str, follow_id: &str, confirmed: &[LibraryConflictFile],
) -> Result<(), InspectionError> {
    let _lock = acquire_companion_lock(root)?;
    let association = read_companion_association(root)?;
    let mut manifest = read_manifest(root, companion_id, &association)?;
    recover_pending_source_intent(root, &mut manifest)?;
    let entries = manifest.entries.iter()
        .filter(|entry| entry.library_follow_id.as_deref() == Some(follow_id)).cloned().collect::<Vec<_>>();
    let record = manifest.library_follows.iter().position(|record| record.follow_id == follow_id);
    if entries.is_empty() && record.is_none() {
        return Err(InspectionError::new("library_item_not_found", "Space copy does not exist"));
    }
    if confirmed.iter().any(|file| !entries.iter().any(|entry| entry.relative_path == file.path)) {
        return Err(space_copy_conflict());
    }
    let expected = entries.iter().map(|entry| library_remove_expected(root, entry, confirmed))
        .collect::<Result<Vec<_>, _>>()?;
    for (entry, expected) in entries.iter().zip(expected) {
        remove_library_entry(root, &mut manifest, entry, expected.as_deref())?;
    }
    let known = record.map(|position| manifest.library_follows.remove(position).known_page_item_ids)
        .unwrap_or_default();
    manifest.library_copies.retain(|copy| {
        let page = known.contains(&copy.item_id)
            || entries.iter().any(|entry| entry.library_item_id.as_ref() == Some(&copy.item_id));
        !page || manifest.entries.iter().any(|entry| entry.library_item_id.as_ref() == Some(&copy.item_id))
    });
    manifest.updated_at = timestamp();
    write_manifest_durable(root, &manifest)
}

/// Materialize one immutable provider payload through the companion lock,
/// manifest, and no-follow descriptor policy.
#[cfg(test)]
pub(crate) fn materialize_source_markdown(
    root: &Dir,
    companion_id: &str,
    provider_id: &str,
    provider_instance: &str,
    resource_type: &str,
    canonical_id: &str,
    revision: Option<&str>,
    content_hash: &str,
    markdown: &[u8],
) -> Result<(String, bool), InspectionError> {
    let _lock = acquire_companion_lock(root)?;
    let association = read_companion_association(root)?;
    let mut manifest = read_manifest(root, companion_id, &association)?;
    recover_pending_source_intent(root, &mut manifest)?;
    let logical_id =
        format!("source:{provider_id}:{provider_instance}:{resource_type}:{canonical_id}");
    // `content_hash` identifies the immutable provider record. The manifest's
    // file hash must instead describe exactly what was written so a later user
    // edit can be distinguished from a legitimate provider refresh.
    let materialized_hash = hash(markdown);
    let previous_entry = manifest
        .entries
        .iter()
        .find(|entry| entry.logical_id == logical_id)
        .cloned();
    let mut replace_owned = false;
    if let Some(entry) = &previous_entry {
        let current = read_stable_source(root, &safe_companion_relative(&entry.relative_path)?)?;
        if current.hash != entry.content_hash {
            return Err(InspectionError::new(
                "source_sync_conflict",
                "a user-modified generated source will not be overwritten",
            ));
        }
        if entry.source_hash_before == content_hash {
            return Ok((entry.relative_path.clone(), false));
        }
        replace_owned = true;
    }
    // A refresh keeps the file where it is; a new source is named after the
    // provider and the item's own identity, for example `sources/jira/issue/PROJ-12.md`.
    let relative = match &previous_entry {
        Some(entry) => entry.relative_path.clone(),
        None => {
            let directory = format!(
                "sources/{}/{}",
                readable_name(provider_id),
                readable_name(resource_type)
            );
            let name = readable_name(canonical_id);
            let taken = |candidate: &str| {
                manifest
                    .entries
                    .iter()
                    .any(|entry| entry.relative_path == candidate)
                    || root.symlink_metadata(candidate).is_ok()
            };
            let readable = format!("{directory}/{name}.md");
            if taken(&readable) {
                format!(
                    "{directory}/{name}-{}.md",
                    short_hash(logical_id.as_bytes())
                )
            } else {
                readable
            }
        }
    };
    let path = safe_companion_relative(&relative)?;
    let (parent, leaf) = create_parent(root, &path)?;
    if parent.symlink_metadata(&leaf).is_ok() && !replace_owned {
        return Err(InspectionError::new(
            "source_sync_conflict",
            "an existing companion file is not a matching generated source",
        ));
    }
    let temporary = format!(".source-{}.tmp", Uuid::new_v4());
    let mut options = OpenOptions::new();
    options
        .write(true)
        .create_new(true)
        .follow(cap_fs_ext::FollowSymlinks::No);
    let mut file = parent
        .open_with(&temporary, &options)
        .map_err(io_error("source_materialize_failed"))?;
    file.write_all(markdown)
        .map_err(io_error("source_materialize_failed"))?;
    file.sync_all()
        .map_err(io_error("source_materialize_failed"))?;
    let intended_entry = ContextManifestEntry {
        logical_id,
        relative_path: relative.clone(),
        kind: resource_type.to_owned(),
        source: provider_id.to_owned(),
        generated: true,
        revision: revision.unwrap_or("unknown").to_owned(),
        content_hash: materialized_hash.clone(),
        bytes: markdown.len() as u64,
        copy_mode: "write".to_owned(),
        status: "complete".to_owned(),
        updated_at: timestamp(),
        source_repository_id: provider_instance.to_owned(),
        source_checkout_path: String::new(),
        source_identity: canonical_id.to_owned(),
        source_hash_before: content_hash.to_owned(),
        source_hash_after: materialized_hash,
        library_item_id: None,
        library_revision: None,
        library_file: None,
        library_follow_id: None,
    };
    manifest.pending_source_intent = Some(PendingSourceIntent {
        schema_version: PENDING_SOURCE_INTENT_SCHEMA_VERSION,
        relative_path: relative.clone(),
        previous_written_hash: previous_entry
            .as_ref()
            .map(|entry| entry.content_hash.clone()),
        previous_entry: previous_entry.clone(),
        new_written_hash: intended_entry.content_hash.clone(),
        intended_entry: intended_entry.clone(),
        expected_previous_hash: None,
    });
    write_manifest_durable(root, &manifest)?;
    // Recheck after the intent is durable and immediately before replacement.
    // A late user edit abandons this publish; it must never be overwritten.
    if let Err(error) = recheck_source_destination(root, &path, &parent, &leaf, &previous_entry) {
        discard_pending_source_intent(root, &mut manifest, &parent, &temporary)?;
        return Err(error);
    }
    parent
        .rename(&temporary, &parent, &leaf)
        .map_err(io_error("source_materialize_failed"))?;
    sync_directory(&parent).map_err(io_error("source_materialize_failed"))?;
    replace_source_manifest_entry(&mut manifest, intended_entry);
    manifest.pending_source_intent = None;
    manifest.updated_at = timestamp();
    write_manifest_durable(root, &manifest)?;
    Ok((relative, true))
}


#[derive(Debug)]
pub(crate) struct SourceBytes {
    pub(crate) bytes: Vec<u8>,
    pub(crate) hash: String,
    pub(crate) identity: String,
}

#[derive(Debug)]
pub(crate) struct Gitlink {
    pub(crate) path: PathBuf,
    pub(crate) commit: String,
}

struct CompanionLock {
    _file: std::fs::File,
}




pub(crate) async fn git_inventory(
    configuration: &ProjectConfiguration,
    source: &Path,
) -> Result<
    (
        Vec<PathBuf>,
        Vec<Gitlink>,
        Vec<PathBuf>,
        Option<String>,
        bool,
    ),
    InspectionError,
> {
    let tracked = git_output(
        configuration,
        source,
        &["ls-files", "--stage", "-z", "--cached"],
    )
    .await?;
    let untracked = git_output(
        configuration,
        source,
        &["ls-files", "-z", "--others", "--exclude-standard"],
    )
    .await?;
    let status = git_output(
        configuration,
        source,
        &["status", "--porcelain=v1", "--untracked-files=all"],
    )
    .await?;
    for output in [&tracked, &untracked, &status] {
        if !output.status.success() {
            return Err(InspectionError::new(
                "context_snapshot_git_failed",
                "Git could not inventory the selected repository without mutation",
            ));
        }
    }
    let head = git_output(configuration, source, &["rev-parse", "--verify", "HEAD"]).await?;
    let source_head = if head.status.success() {
        Some(single_line_utf8(
            &head.stdout,
            "context_snapshot_git_output",
        )?)
    } else {
        None
    };
    let mut paths = BTreeSet::new();
    let mut gitlinks = Vec::new();
    let mut excluded = BTreeSet::new();
    for raw in tracked
        .stdout
        .split(|byte| *byte == 0)
        .filter(|path| !path.is_empty())
    {
        let separator = raw.iter().position(|byte| *byte == b'\t').ok_or_else(|| {
            InspectionError::new(
                "context_snapshot_git_output",
                "Git returned an invalid index record",
            )
        })?;
        let (stage, raw_path) = raw.split_at(separator);
        let raw_path = &raw_path[1..];
        let stage = std::str::from_utf8(stage).map_err(|_| {
            InspectionError::new(
                "context_snapshot_git_output",
                "Git returned non-UTF-8 index metadata",
            )
        })?;
        let mut fields = stage.split_whitespace();
        let mode = fields.next().ok_or_else(|| {
            InspectionError::new("context_snapshot_git_output", "Git index mode is missing")
        })?;
        let object = fields.next().ok_or_else(|| {
            InspectionError::new("context_snapshot_git_output", "Git index object is missing")
        })?;
        let text = std::str::from_utf8(raw_path).map_err(|_| {
            InspectionError::new(
                "context_snapshot_non_utf8_path",
                "working-tree snapshot paths must be valid UTF-8",
            )
        })?;
        let path = safe_source_relative(text)?;
        if excluded_source_path(&path) {
            excluded.insert(path);
            continue;
        }
        if mode == "160000" {
            gitlinks.push(Gitlink {
                path,
                commit: object.to_owned(),
            });
        } else {
            paths.insert(path);
        }
    }
    for raw in untracked
        .stdout
        .split(|byte| *byte == 0)
        .filter(|path| !path.is_empty())
    {
        let text = std::str::from_utf8(raw).map_err(|_| {
            InspectionError::new(
                "context_snapshot_non_utf8_path",
                "working-tree snapshot paths must be valid UTF-8",
            )
        })?;
        let path = safe_source_relative(text)?;
        if excluded_source_path(&path) {
            excluded.insert(path);
            continue;
        }
        paths.insert(path);
    }
    Ok((
        paths.into_iter().collect(),
        gitlinks,
        excluded.into_iter().collect(),
        source_head,
        !status.stdout.is_empty(),
    ))
}

pub(crate) async fn git_output(
    configuration: &ProjectConfiguration,
    source: &Path,
    args: &[&str],
) -> Result<std::process::Output, InspectionError> {
    let mut command = Command::new("git");
    command
        .current_dir(source)
        .arg("-c")
        .arg("core.hooksPath=/dev/null")
        .arg("-c")
        .arg("core.fsmonitor=false")
        .args(args)
        .env("GIT_OPTIONAL_LOCKS", "0")
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .env("GIT_CONFIG_GLOBAL", "/dev/null")
        .env("GIT_CONFIG_SYSTEM", "/dev/null")
        .env("GIT_TERMINAL_PROMPT", "0")
        .env("GIT_ASKPASS", "")
        .env_remove("GIT_DIR")
        .env_remove("GIT_WORK_TREE")
        .env_remove("GIT_COMMON_DIR")
        .env_remove("GIT_INDEX_FILE")
        .env_remove("GIT_OBJECT_DIRECTORY");
    run_bounded_command(
        command,
        configuration.limits.git_output_bytes as usize,
        configuration.limits.git_output_bytes as usize,
        Duration::from_millis(configuration.limits.git_timeout_ms as u64),
        "context_snapshot_git",
    )
    .await
}

fn read_companion_association(root: &Dir) -> Result<CompanionManifest, InspectionError> {
    let association: CompanionManifest =
        read_json_bounded(root, "manifest.json", MAX_MANIFEST_BYTES).map_err(|_| {
            InspectionError::new(
                "context_snapshot_companion_unavailable",
                "the authorized companion has no readable durable association",
            )
        })?;
    if association.ownership != "cockpit" {
        return Err(InspectionError::new(
            "context_snapshot_companion_unavailable",
            "the companion association is not Cockpit-owned",
        ));
    }
    Ok(association)
}

fn read_manifest(
    root: &Dir,
    companion_id: &str,
    association: &CompanionManifest,
) -> Result<ContextManifest, InspectionError> {
    match root.symlink_metadata(MANIFEST_NAME) {
        Ok(_) => {
            let mut manifest: ContextManifest =
                read_json_bounded(root, MANIFEST_NAME, MAX_MANIFEST_BYTES)?;
            if !matches!(manifest.schema_version, 1 | MANIFEST_SCHEMA_VERSION)
                || manifest.companion_id != companion_id
                || manifest.owner_workspace_id != association.herdr_workspace_id
                || manifest.owner_worktree_path != association.checkout_path
                || manifest.primary_repository_identity != association.repository_key
            {
                return Err(InspectionError::new(
                    "context_manifest_owner_mismatch",
                    "the context manifest is not owned by this authorized companion",
                ));
            }
            // Upgrade only in memory; reads never rewrite legacy manifests.
            manifest.schema_version = MANIFEST_SCHEMA_VERSION;
            Ok(manifest)
        }
        Err(error) if error.kind() == ErrorKind::NotFound => Ok(ContextManifest {
            schema_version: MANIFEST_SCHEMA_VERSION,
            companion_id: companion_id.to_owned(),
            owner_workspace_id: association.herdr_workspace_id.clone(),
            owner_worktree_path: association.checkout_path.clone(),
            primary_repository_identity: association.repository_key.clone(),
            entries: Vec::new(),
            library_follows: Vec::new(),
            library_copies: Vec::new(),
            pending_source_intent: None,
            pending_library_remove: None,
            updated_at: timestamp(),
        }),
        Err(error) => Err(InspectionError::new(
            "context_manifest_unavailable",
            error.to_string(),
        )),
    }
}

fn recover_pending_source_intent(
    root: &Dir,
    manifest: &mut ContextManifest,
) -> Result<(), InspectionError> {
    if let Some(entry) = manifest.pending_library_remove.clone() {
        match read_space_file(root, &entry) {
            Err(error) if error.code == "context_snapshot_file_missing" => finish_library_remove(manifest, &entry),
            _ => manifest.pending_library_remove = None,
        }
        write_manifest_durable(root, manifest)?;
    }
    let Some(intent) = manifest.pending_source_intent.clone() else {
        return Ok(());
    };
    validate_pending_source_intent(&intent)?;
    let path = safe_companion_relative(&intent.relative_path)?;
    let max_bytes = intent.previous_entry.as_ref().map_or(0, |entry| entry.bytes)
        .max(intent.intended_entry.bytes).max(MAX_SNAPSHOT_FILE_BYTES as u64);
    let current = match read_stable_source_bounded(root, &path, max_bytes) {
        Ok(current) => current,
        Err(error)
            if error.code == "context_snapshot_file_missing" && intent.previous_entry.is_none() =>
        {
            manifest.pending_source_intent = None;
            return write_manifest_durable(root, manifest);
        }
        Err(_) => return clear_pending_source_conflict(root, manifest),
    };
    let current_entry = manifest
        .entries
        .iter()
        .find(|entry| entry.logical_id == intent.intended_entry.logical_id);
    if current_entry == Some(&intent.intended_entry) {
        if current.hash != intent.new_written_hash {
            return clear_pending_source_conflict(root, manifest);
        }
        manifest.pending_source_intent = None;
        return write_manifest_durable(root, manifest);
    }
    let previous_matches = match (&intent.previous_entry, current_entry) {
        (None, None) => true,
        (Some(previous), Some(current_entry)) => previous == current_entry,
        _ => false,
    };
    if !previous_matches {
        return clear_pending_source_conflict(root, manifest);
    }
    if current.hash == intent.new_written_hash {
        replace_source_manifest_entry(manifest, intent.intended_entry);
        manifest.pending_source_intent = None;
        manifest.updated_at = timestamp();
        return write_manifest_durable(root, manifest);
    }
    if intent
        .expected_previous_hash
        .as_deref()
        .or(intent.previous_written_hash.as_deref())
        .is_some_and(|expected| current.hash == expected)
    {
        // The intent persisted but the rename did not. It is safe to discard
        // it because the prior manifest and bytes still agree exactly.
        manifest.pending_source_intent = None;
        return write_manifest_durable(root, manifest);
    }
    clear_pending_source_conflict(root, manifest)
}

fn recheck_source_destination(
    root: &Dir,
    path: &Path,
    parent: &Dir,
    leaf: &Path,
    previous_entry: &Option<ContextManifestEntry>,
) -> Result<(), InspectionError> {
    let Some(previous) = previous_entry else {
        return match parent.symlink_metadata(leaf) {
            Err(error) if error.kind() == ErrorKind::NotFound => Ok(()),
            Ok(_) => Err(InspectionError::new(
                "source_sync_conflict",
                "an existing companion file is not a matching generated source",
            )),
            Err(error) => Err(InspectionError::new(
                "source_materialize_failed",
                error.to_string(),
            )),
        };
    };
    let current = read_stable_source_bounded(root, path, previous.bytes.max(MAX_SNAPSHOT_FILE_BYTES as u64)).map_err(|_| {
        InspectionError::new(
            "source_sync_conflict",
            "a generated source changed while Cockpit was refreshing it",
        )
    })?;
    if current.hash != previous.content_hash {
        return Err(InspectionError::new(
            "source_sync_conflict",
            "a user-modified generated source will not be overwritten",
        ));
    }
    Ok(())
}

fn discard_pending_source_intent(
    root: &Dir,
    manifest: &mut ContextManifest,
    parent: &Dir,
    temporary: &str,
) -> Result<(), InspectionError> {
    manifest.pending_source_intent = None;
    write_manifest_durable(root, manifest)?;
    match parent.remove_file(temporary) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == ErrorKind::NotFound => Ok(()),
        Err(error) => Err(InspectionError::new(
            "source_materialize_failed",
            error.to_string(),
        )),
    }
}

fn clear_pending_source_conflict(
    root: &Dir,
    manifest: &mut ContextManifest,
) -> Result<(), InspectionError> {
    manifest.pending_source_intent = None;
    write_manifest_durable(root, manifest)?;
    Err(source_publish_conflict())
}

fn validate_pending_source_intent(intent: &PendingSourceIntent) -> Result<(), InspectionError> {
    if intent.schema_version != PENDING_SOURCE_INTENT_SCHEMA_VERSION
        || intent.relative_path != intent.intended_entry.relative_path
        || intent.new_written_hash != intent.intended_entry.content_hash
        || intent
            .previous_entry
            .as_ref()
            .map(|entry| entry.content_hash.as_str())
            != intent.previous_written_hash.as_deref()
    {
        return Err(InspectionError::new(
            "source_publish_intent_invalid",
            "the pending source publish intent is not internally consistent",
        ));
    }
    if let Some(previous) = &intent.previous_entry {
        if previous.logical_id != intent.intended_entry.logical_id
            || previous.relative_path != intent.relative_path
        {
            return Err(InspectionError::new(
                "source_publish_intent_invalid",
                "the pending source publish intent does not describe one owned path",
            ));
        }
    }
    safe_companion_relative(&intent.relative_path)?;
    Ok(())
}

fn replace_source_manifest_entry(manifest: &mut ContextManifest, intended: ContextManifestEntry) {
    manifest
        .entries
        .retain(|entry| entry.logical_id != intended.logical_id);
    manifest.entries.push(intended);
}

fn source_publish_conflict() -> InspectionError {
    InspectionError::new(
        "source_sync_conflict",
        "a pending generated source publish does not match its recorded bytes",
    )
}

fn write_manifest_durable(root: &Dir, manifest: &ContextManifest) -> Result<(), InspectionError> {
    let temporary = format!(".context-manifest-{}.tmp", Uuid::new_v4());
    let bytes = serde_json::to_vec_pretty(manifest)
        .map_err(|error| InspectionError::new("source_manifest_failed", error.to_string()))?;
    let mut options = OpenOptions::new();
    options
        .write(true)
        .create_new(true)
        .follow(cap_fs_ext::FollowSymlinks::No);
    let mut file = root
        .open_with(&temporary, &options)
        .map_err(io_error("source_manifest_failed"))?;
    file.write_all(&bytes)
        .map_err(io_error("source_manifest_failed"))?;
    file.sync_all()
        .map_err(io_error("source_manifest_failed"))?;
    match root.symlink_metadata(MANIFEST_NAME) {
        Ok(metadata) if metadata.file_type().is_symlink() || !metadata.is_file() => {
            return Err(InspectionError::new(
                "source_manifest_failed",
                "the context manifest destination is not a regular file",
            ));
        }
        Ok(_) => {}
        Err(error) if error.kind() == ErrorKind::NotFound => {}
        Err(error) => {
            return Err(InspectionError::new(
                "source_manifest_failed",
                error.to_string(),
            ));
        }
    }
    root.rename(&temporary, root, MANIFEST_NAME)
        .map_err(io_error("source_manifest_failed"))?;
    sync_directory(root).map_err(io_error("source_manifest_failed"))?;
    Ok(())
}

fn sync_directory(dir: &Dir) -> std::io::Result<()> {
    dir.open(Path::new("."))?.sync_all()
}


pub(crate) fn read_stable_source(root: &Dir, relative: &Path) -> Result<SourceBytes, InspectionError> {
    read_stable_source_bounded(root, relative, MAX_SNAPSHOT_FILE_BYTES as u64)
}

pub(crate) fn read_stable_source_bounded(
    root: &Dir, relative: &Path, max_bytes: u64,
) -> Result<SourceBytes, InspectionError> {
    let (parent, leaf) = resolve_parent(root, relative)?;
    let before = read_regular_bounded(&parent, &leaf, max_bytes)?;
    let after = read_regular_bounded(&parent, &leaf, max_bytes)?;
    if before.identity != after.identity || before.hash != after.hash {
        return Err(InspectionError::new(
            "context_snapshot_source_changed",
            "a source file changed during snapshot; retry to capture a complete generation",
        ));
    }
    Ok(before)
}

fn read_space_file(root: &Dir, entry: &ContextManifestEntry) -> Result<SourceBytes, InspectionError> {
    let path = safe_companion_relative(&entry.relative_path)?;
    if entry.bytes <= MAX_SNAPSHOT_FILE_BYTES as u64 {
        read_stable_source(root, &path)
    } else {
        read_stable_source_bounded(root, &path, entry.bytes)
    }
}

fn read_regular_bounded(parent: &Dir, leaf: &Path, max_bytes: u64) -> Result<SourceBytes, InspectionError> {
    let metadata = parent.symlink_metadata(leaf).map_err(|error| {
        InspectionError::new(
            if error.kind() == ErrorKind::NotFound {
                "context_snapshot_file_missing"
            } else {
                "context_snapshot_file_unavailable"
            },
            error.to_string(),
        )
    })?;
    if metadata.file_type().is_symlink() {
        return Err(InspectionError::new(
            "context_snapshot_symlink",
            "symbolic links are recorded as skipped",
        ));
    }
    if !metadata.is_file() {
        return Err(InspectionError::new(
            "context_snapshot_special_file",
            "only regular files are eligible for snapshots",
        ));
    }
    if metadata.len() > max_bytes {
        return Err(InspectionError::new(
            "context_snapshot_file_bytes",
            "a source file exceeds Cockpit's snapshot file limit",
        ));
    }
    if hardlinked(&metadata) {
        return Err(InspectionError::new(
            "context_snapshot_hardlink",
            "hardlinked source files are not copied into snapshots",
        ));
    }
    let mut options = OpenOptions::new();
    options
        .read(true)
        .follow(cap_fs_ext::FollowSymlinks::No)
        .nonblock(true);
    let file = parent.open_with(leaf, &options).map_err(|error| {
        InspectionError::new("context_snapshot_file_unavailable", error.to_string())
    })?;
    let opened = file.metadata().map_err(|error| {
        InspectionError::new("context_snapshot_file_unavailable", error.to_string())
    })?;
    if opened.file_type().is_symlink()
        || !opened.is_file()
        || identity(&opened) != identity(&metadata)
    {
        return Err(InspectionError::new(
            "context_snapshot_source_changed",
            "a source file changed while opening",
        ));
    }
    let mut bytes = Vec::with_capacity(metadata.len() as usize);
    file.take(max_bytes.saturating_add(1))
        .read_to_end(&mut bytes)
        .map_err(|error| {
            InspectionError::new("context_snapshot_file_unavailable", error.to_string())
        })?;
    if bytes.len() as u64 > max_bytes {
        return Err(InspectionError::new(
            "context_snapshot_file_bytes",
            "a source file grew beyond Cockpit's snapshot file limit",
        ));
    }
    if native_executable(&opened, &bytes) {
        return Err(InspectionError::new(
            "context_snapshot_native_binary",
            "native executable binaries are not copied into snapshots",
        ));
    }
    Ok(SourceBytes {
        hash: hash(&bytes),
        bytes,
        identity: identity(&opened),
    })
}


fn open_source_for_clone(
    root: &Dir,
    relative: &Path,
    expected_identity: &str,
) -> Result<cap_std::fs::File, InspectionError> {
    let (parent, leaf) = resolve_parent(root, relative)?;
    let metadata = parent
        .symlink_metadata(&leaf)
        .map_err(io_error("context_snapshot_source_changed"))?;
    if metadata.file_type().is_symlink()
        || !metadata.is_file()
        || identity(&metadata) != expected_identity
    {
        return Err(InspectionError::new(
            "context_snapshot_source_changed",
            "a source file changed before reflink",
        ));
    }
    let mut options = OpenOptions::new();
    options
        .read(true)
        .follow(cap_fs_ext::FollowSymlinks::No)
        .nonblock(true);
    let file = parent
        .open_with(&leaf, &options)
        .map_err(io_error("context_snapshot_source_changed"))?;
    let opened = file
        .metadata()
        .map_err(io_error("context_snapshot_source_changed"))?;
    if opened.file_type().is_symlink()
        || !opened.is_file()
        || identity(&opened) != expected_identity
    {
        return Err(InspectionError::new(
            "context_snapshot_source_changed",
            "a source file changed while opening for reflink",
        ));
    }
    Ok(file)
}

#[cfg(target_os = "linux")]
fn reflink(destination: &cap_std::fs::File, source: &cap_std::fs::File) -> std::io::Result<()> {
    rustix::fs::ioctl_ficlone(destination, source).map_err(std::io::Error::from)
}

#[cfg(not(target_os = "linux"))]
fn reflink(_destination: &cap_std::fs::File, _source: &cap_std::fs::File) -> std::io::Result<()> {
    Err(std::io::Error::from(ErrorKind::Unsupported))
}

fn reflink_fallback(error: &std::io::Error) -> bool {
    matches!(
        error.kind(),
        ErrorKind::Unsupported | ErrorKind::InvalidInput
    ) || error.raw_os_error() == Some(nix::libc::EXDEV)
        || error.raw_os_error() == Some(nix::libc::ENOTTY)
        || error.raw_os_error() == Some(nix::libc::EOPNOTSUPP)
}

pub(crate) fn create_parent(root: &Dir, relative: &Path) -> Result<(Dir, PathBuf), InspectionError> {
    let leaf = relative.file_name().ok_or_else(|| {
        InspectionError::new("context_snapshot_path", "snapshot path has no file name")
    })?;
    let mut current = root
        .try_clone()
        .map_err(io_error("context_snapshot_destination_unavailable"))?;
    for component in relative
        .parent()
        .unwrap_or_else(|| Path::new(""))
        .components()
    {
        let Component::Normal(name) = component else {
            return Err(InspectionError::new(
                "context_snapshot_path",
                "snapshot path escapes its root",
            ));
        };
        current = ensure_directory(
            &current,
            name.to_str().ok_or_else(|| {
                InspectionError::new("context_snapshot_path", "snapshot paths must be UTF-8")
            })?,
        )?;
    }
    Ok((current, leaf.into()))
}

fn ensure_directory(root: &Dir, name: &str) -> Result<Dir, InspectionError> {
    match root.create_dir(name) {
        Ok(()) => {}
        Err(error) if error.kind() == ErrorKind::AlreadyExists => {}
        Err(error) => {
            return Err(InspectionError::new(
                "context_snapshot_destination_unavailable",
                error.to_string(),
            ));
        }
    }
    let metadata = root
        .symlink_metadata(name)
        .map_err(io_error("context_snapshot_destination_unavailable"))?;
    if metadata.file_type().is_symlink() || !metadata.is_dir() {
        return Err(InspectionError::new(
            "context_snapshot_destination_unavailable",
            "snapshot directory is not a real directory",
        ));
    }
    root.open_dir_nofollow(Path::new(name))
        .map_err(io_error("context_snapshot_destination_unavailable"))
}

fn open_directory(root: &Dir, path: &Path) -> Result<Dir, InspectionError> {
    let mut current = root
        .try_clone()
        .map_err(io_error("context_snapshot_destination_unavailable"))?;
    for component in path.components() {
        let Component::Normal(name) = component else {
            return Err(InspectionError::new(
                "context_snapshot_path",
                "snapshot path escapes its root",
            ));
        };
        current = current
            .open_dir_nofollow(Path::new(name))
            .map_err(|error| {
                if error.kind() == ErrorKind::NotFound {
                    InspectionError::new("context_snapshot_file_missing", error.to_string())
                } else {
                    io_error("context_snapshot_destination_unavailable")(error)
                }
            })?;
    }
    Ok(current)
}

fn resolve_parent(root: &Dir, relative: &Path) -> Result<(Dir, PathBuf), InspectionError> {
    let leaf = relative.file_name().ok_or_else(|| {
        InspectionError::new("context_snapshot_path", "snapshot path has no file name")
    })?;
    Ok((
        open_directory(root, relative.parent().unwrap_or_else(|| Path::new("")))?,
        leaf.into(),
    ))
}

fn safe_source_relative(value: &str) -> Result<PathBuf, InspectionError> {
    safe_relative(value)
}

fn safe_companion_relative(value: &str) -> Result<PathBuf, InspectionError> {
    safe_relative(value)
}

fn safe_relative(value: &str) -> Result<PathBuf, InspectionError> {
    let candidate = Path::new(value);
    if value.is_empty() || candidate.is_absolute() || value.contains('\0') {
        return Err(InspectionError::new(
            "context_snapshot_path",
            "snapshot paths must be bounded relative paths",
        ));
    }
    let mut result = PathBuf::new();
    for component in candidate.components() {
        match component {
            Component::Normal(part) => result.push(part),
            _ => {
                return Err(InspectionError::new(
                    "context_snapshot_path",
                    "snapshot path escapes its root",
                ));
            }
        }
    }
    Ok(result)
}

pub(crate) fn excluded_source_path(path: &Path) -> bool {
    path.components().any(|component| {
        let Component::Normal(name) = component else {
            return true;
        };
        matches!(
            name.to_str(),
            Some(".git" | "node_modules" | "target" | "build" | "dist" | ".next")
        )
    })
}


pub(crate) fn source_root_revalidate(source_root: &Dir, source: &Path) -> Result<(), InspectionError> {
    let opened = source_root.dir_metadata().map_err(|error| {
        InspectionError::new("context_snapshot_repository_changed", error.to_string())
    })?;
    let reopened = open_absolute_dir_nofollow(source).map_err(|error| {
        InspectionError::new("context_snapshot_repository_changed", error.to_string())
    })?;
    let current = reopened.dir_metadata().map_err(|error| {
        InspectionError::new("context_snapshot_repository_changed", error.to_string())
    })?;
    if !opened.is_dir()
        || !current.is_dir()
        || object_identity(&opened) != object_identity(&current)
    {
        return Err(InspectionError::new(
            "context_snapshot_repository_changed",
            "the selected checkout changed after catalog resolution",
        ));
    }
    Ok(())
}

fn acquire_companion_lock(root: &Dir) -> Result<CompanionLock, InspectionError> {
    const LOCK_NAME: &str = ".context-assets.lock";
    match root.symlink_metadata(LOCK_NAME) {
        Ok(metadata) if metadata.file_type().is_symlink() || !metadata.is_file() => {
            return Err(InspectionError::new(
                "context_snapshot_lock_unavailable",
                "the companion snapshot lock path is not a regular file",
            ));
        }
        Ok(_) | Err(_) => {}
    }
    let mut options = OpenOptions::new();
    options
        .write(true)
        .create(true)
        .follow(cap_fs_ext::FollowSymlinks::No);
    let file = root
        .open_with(LOCK_NAME, &options)
        .map_err(io_error("context_snapshot_lock_unavailable"))?
        .into_std();
    file.lock_exclusive()
        .map_err(io_error("context_snapshot_lock_unavailable"))?;
    Ok(CompanionLock { _file: file })
}

pub(crate) fn open_absolute_dir_nofollow(path: &Path) -> std::io::Result<Dir> {
    let mut dir = Dir::open_ambient_dir(Path::new("/"), cap_std::ambient_authority())?;
    for component in path.components() {
        match component {
            Component::RootDir | Component::CurDir => {}
            Component::Normal(name) => dir = dir.open_dir_nofollow(Path::new(name))?,
            Component::ParentDir | Component::Prefix(_) => {
                return Err(std::io::Error::new(
                    ErrorKind::InvalidInput,
                    "unsafe absolute path",
                ));
            }
        }
    }
    Ok(dir)
}


/// A file or directory name a person can read: letters, digits, `.`, `_` and
/// `-`, with other characters replaced by `-`. Never empty or hidden.
fn readable_name(value: &str) -> String {
    let mut name = String::new();
    for character in value.chars() {
        let character = if character.is_ascii_alphanumeric() || matches!(character, '.' | '_' | '-')
        {
            character
        } else {
            '-'
        };
        if character == '-' && name.ends_with('-') {
            continue;
        }
        name.push(character);
        if name.len() >= 80 {
            break;
        }
    }
    let name = name.trim_matches(|character| character == '-' || character == '.');
    if name.is_empty() {
        "item".to_owned()
    } else {
        name.to_owned()
    }
}

fn short_hash(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))[..8].to_owned()
}


fn hash(bytes: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(bytes);
    format!("sha256:{:x}", hasher.finalize())
}

fn identity(metadata: &Metadata) -> String {
    #[cfg(unix)]
    {
        use cap_std::fs::MetadataExt;
        return format!(
            "{}:{}:{}:{}",
            metadata.dev(),
            metadata.ino(),
            metadata.len(),
            metadata.mtime_nsec()
        );
    }
    #[cfg(not(unix))]
    {
        format!("{}:{:?}", metadata.len(), metadata.modified().ok())
    }
}

fn object_identity(metadata: &Metadata) -> String {
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

fn hardlinked(metadata: &Metadata) -> bool {
    #[cfg(unix)]
    {
        use cap_std::fs::MetadataExt;
        return metadata.nlink() > 1;
    }
    #[cfg(not(unix))]
    {
        let _ = metadata;
        false
    }
}

fn native_executable(metadata: &Metadata, bytes: &[u8]) -> bool {
    #[cfg(unix)]
    {
        use cap_std::fs::MetadataExt;
        if metadata.mode() & 0o111 == 0 {
            return false;
        }
        return bytes.starts_with(b"\x7fELF")
            || matches!(
                bytes.get(..4),
                Some(
                    [0xfe, 0xed, 0xfa, 0xce]
                        | [0xfe, 0xed, 0xfa, 0xcf]
                        | [0xcf, 0xfa, 0xed, 0xfe]
                        | [0xce, 0xfa, 0xed, 0xfe]
                )
            );
    }
    #[cfg(not(unix))]
    {
        let _ = (metadata, bytes);
        false
    }
}

fn single_line_utf8(bytes: &[u8], code: &str) -> Result<String, InspectionError> {
    let text = std::str::from_utf8(bytes)
        .map_err(|_| InspectionError::new(code, "Git returned non-UTF-8 output"))?
        .trim();
    if text.is_empty() || text.lines().count() != 1 || text.chars().any(char::is_control) {
        return Err(InspectionError::new(
            code,
            "Git returned an invalid revision",
        ));
    }
    Ok(text.to_owned())
}


fn io_error(code: &'static str) -> impl FnOnce(std::io::Error) -> InspectionError {
    move |error| InspectionError::new(code, error.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use cap_std::fs::Dir;
    use std::fs;

    fn temp_dir(name: &str) -> PathBuf {
        let path =
            std::env::temp_dir().join(format!("cockpit-context-assets-{name}-{}", Uuid::new_v4()));
        fs::create_dir_all(&path).expect("create temporary directory");
        path
    }


    fn association(workspace: &str, checkout: &Path) -> CompanionManifest {
        CompanionManifest {
            schema_version: 1,
            cockpit_operation_id: "op-test".to_owned(),
            herdr_session_identity: "session-test".to_owned(),
            herdr_workspace_id: workspace.to_owned(),
            repository_key: "primary-repository".to_owned(),
            repository_root: checkout.to_string_lossy().into_owned(),
            checkout_path: checkout.to_string_lossy().into_owned(),
            artifact: None,
            created_at: timestamp(),
            updated_at: timestamp(),
            ownership: "cockpit".to_owned(),
        }
    }

    fn stage_interrupted_source_refresh(
        dir: &Dir,
        manifest: &mut ContextManifest,
        previous: ContextManifestEntry,
        markdown: &[u8],
    ) -> ContextManifestEntry {
        let mut intended = previous.clone();
        intended.revision = "2".to_owned();
        intended.content_hash = hash(markdown);
        intended.bytes = markdown.len() as u64;
        intended.updated_at = timestamp();
        intended.source_hash_before = "two".to_owned();
        intended.source_hash_after = intended.content_hash.clone();
        manifest.pending_source_intent = Some(PendingSourceIntent {
            schema_version: PENDING_SOURCE_INTENT_SCHEMA_VERSION,
            relative_path: intended.relative_path.clone(),
            previous_written_hash: Some(previous.content_hash.clone()),
            previous_entry: Some(previous),
            new_written_hash: intended.content_hash.clone(),
            intended_entry: intended.clone(),
            expected_previous_hash: None,
        });
        write_manifest_durable(dir, manifest).expect("persist source publish intent");
        intended
    }

    fn rename_source_for_interrupted_publish(dir: &Dir, relative: &str, markdown: &[u8]) {
        let path = safe_companion_relative(relative).expect("safe source path");
        let (parent, leaf) = create_parent(dir, &path).expect("source parent");
        let temporary = format!(".interrupted-source-{}.tmp", Uuid::new_v4());
        let mut options = OpenOptions::new();
        options
            .write(true)
            .create_new(true)
            .follow(cap_fs_ext::FollowSymlinks::No);
        let mut file = parent
            .open_with(&temporary, &options)
            .expect("temporary source");
        file.write_all(markdown).expect("write temporary source");
        file.sync_all().expect("sync temporary source");
        parent
            .rename(&temporary, &parent, &leaf)
            .expect("replace source atomically");
    }


    #[test]
    fn generated_names_are_readable_and_safe() {
        assert_eq!(super::readable_name("PROJ-123"), "PROJ-123");
        assert_eq!(super::readable_name("group/project!12"), "group-project-12");
        assert_eq!(super::readable_name("../..//"), "item");
        assert_eq!(super::readable_name(".hidden"), "hidden");
        assert!(super::readable_name(&"x".repeat(500)).len() <= 80);
    }

    #[test]
    fn source_policy_rejects_escape_and_excluded_paths() {
        assert!(safe_source_relative("../escape").is_err());
        assert!(excluded_source_path(
            &safe_source_relative(".git/config").expect("relative")
        ));
        assert!(excluded_source_path(
            &safe_source_relative("node_modules/pkg/index.js").expect("relative")
        ));
        assert_eq!(
            safe_source_relative("src/lib.rs")
                .expect("safe")
                .to_string_lossy(),
            "src/lib.rs"
        );
    }


    #[test]
    fn changed_source_rejects_the_pending_copy() {
        let root = temp_dir("changed");
        fs::write(root.join("source.rs"), b"before").expect("source");
        let root_dir =
            Dir::open_ambient_dir(&root, cap_std::ambient_authority()).expect("open root");
        let source = read_stable_source(&root_dir, Path::new("source.rs")).expect("read source");
        fs::write(root.join("source.rs"), b"after").expect("mutate source");
        assert_eq!(
            open_source_for_clone(&root_dir, Path::new("source.rs"), &source.identity)
                .expect_err("changed source must fail")
                .code,
            "context_snapshot_source_changed"
        );
        fs::remove_dir_all(root).expect("cleanup");
    }

    #[test]
    fn manifest_owner_mismatch_is_refused() {
        let root = temp_dir("manifest");
        let root_dir =
            Dir::open_ambient_dir(&root, cap_std::ambient_authority()).expect("open root");
        let first = association("workspace-a", Path::new("/worktree-a"));
        atomic_write_json(&root_dir, "manifest.json", &first).expect("association");
        let manifest = ContextManifest {
            schema_version: MANIFEST_SCHEMA_VERSION,
            companion_id: "companion-a".to_owned(),
            owner_workspace_id: "workspace-b".to_owned(),
            owner_worktree_path: "/worktree-a".to_owned(),
            primary_repository_identity: "primary-repository".to_owned(),
            entries: Vec::new(),
            library_follows: Vec::new(),
            library_copies: Vec::new(),
            pending_source_intent: None,
            pending_library_remove: None,
            updated_at: timestamp(),
        };
        atomic_write_json(&root_dir, MANIFEST_NAME, &manifest).expect("content manifest");
        assert_eq!(
            read_manifest(&root_dir, "companion-a", &first)
                .expect_err("wrong owner")
                .code,
            "context_manifest_owner_mismatch"
        );

        fs::remove_dir_all(root).expect("cleanup");
    }


    #[cfg(unix)]
    #[test]
    fn symlink_and_hardlink_are_not_eligible_sources() {
        use std::os::unix::fs::symlink;
        let root = temp_dir("links");
        fs::write(root.join("source"), b"bytes").expect("source");
        symlink("source", root.join("link")).expect("symlink");
        fs::hard_link(root.join("source"), root.join("alias")).expect("hardlink");
        let root_dir =
            Dir::open_ambient_dir(&root, cap_std::ambient_authority()).expect("open root");
        assert_eq!(
            read_stable_source(&root_dir, Path::new("link"))
                .expect_err("symlink")
                .code,
            "context_snapshot_symlink"
        );
        assert_eq!(
            read_stable_source(&root_dir, Path::new("alias"))
                .expect_err("hardlink")
                .code,
            "context_snapshot_hardlink"
        );
        fs::remove_dir_all(root).expect("cleanup");
    }

    #[test]
    fn generated_source_refresh_preserves_user_edits_and_replaces_owned_bytes() {
        let root = temp_dir("source-refresh");
        let dir = Dir::open_ambient_dir(&root, cap_std::ambient_authority()).expect("open root");
        atomic_write_json(
            &dir,
            "manifest.json",
            &association("workspace-a", Path::new("/worktree-a")),
        )
        .expect("association");
        let first = materialize_source_markdown(
            &dir,
            "companion-a",
            "tea",
            "https://forge.test",
            "issue",
            "acme/repo#1",
            Some("1"),
            "one",
            b"first\n",
        )
        .expect("first");
        assert!(first.1);
        let second = materialize_source_markdown(
            &dir,
            "companion-a",
            "tea",
            "https://forge.test",
            "issue",
            "acme/repo#1",
            Some("2"),
            "two",
            b"second\n",
        )
        .expect("replace owned");
        assert!(second.1);
        assert_eq!(
            fs::read(root.join(&second.0)).expect("replaced"),
            b"second\n"
        );
        fs::write(root.join(&second.0), b"user edit\n").expect("edit");
        assert_eq!(
            materialize_source_markdown(
                &dir,
                "companion-a",
                "tea",
                "https://forge.test",
                "issue",
                "acme/repo#1",
                Some("3"),
                "three",
                b"third\n"
            )
            .expect_err("conflict")
            .code,
            "source_sync_conflict"
        );
        assert_eq!(
            fs::read(root.join(&second.0)).expect("preserved"),
            b"user edit\n"
        );
        fs::remove_dir_all(root).expect("cleanup");
    }

    #[test]
    fn interrupted_source_refresh_commits_the_matching_pending_publish() {
        let root = temp_dir("source-publish-recovery");
        let dir = Dir::open_ambient_dir(&root, cap_std::ambient_authority()).expect("open root");
        let association = association("workspace-a", Path::new("/worktree-a"));
        atomic_write_json(&dir, "manifest.json", &association).expect("association");
        let first = materialize_source_markdown(
            &dir,
            "companion-a",
            "tea",
            "https://forge.test",
            "issue",
            "acme/repo#1",
            Some("1"),
            "one",
            b"first\n",
        )
        .expect("first source");
        let mut manifest = read_manifest(&dir, "companion-a", &association).expect("manifest");
        let previous = manifest.entries[0].clone();
        let intended = stage_interrupted_source_refresh(&dir, &mut manifest, previous, b"second\n");
        // This models a process crash after the source rename but before the
        // final manifest replacement. The pending intent was written first.
        rename_source_for_interrupted_publish(&dir, &first.0, b"second\n");

        let recovered = materialize_source_markdown(
            &dir,
            "companion-a",
            "tea",
            "https://forge.test",
            "issue",
            "acme/repo#1",
            Some("2"),
            "two",
            b"second\n",
        )
        .expect("recover publish");
        assert_eq!(recovered, (first.0.clone(), false));
        let recovered_manifest =
            read_manifest(&dir, "companion-a", &association).expect("recovered manifest");
        assert_eq!(recovered_manifest.entries, vec![intended]);
        assert!(recovered_manifest.pending_source_intent.is_none());
        fs::remove_dir_all(root).expect("cleanup");
    }

    #[test]
    fn interrupted_source_refresh_does_not_adopt_user_bytes() {
        let root = temp_dir("source-publish-conflict");
        let dir = Dir::open_ambient_dir(&root, cap_std::ambient_authority()).expect("open root");
        let association = association("workspace-a", Path::new("/worktree-a"));
        atomic_write_json(&dir, "manifest.json", &association).expect("association");
        let first = materialize_source_markdown(
            &dir,
            "companion-a",
            "tea",
            "https://forge.test",
            "issue",
            "acme/repo#1",
            Some("1"),
            "one",
            b"first\n",
        )
        .expect("first source");
        let mut manifest = read_manifest(&dir, "companion-a", &association).expect("manifest");
        let previous = manifest.entries[0].clone();
        stage_interrupted_source_refresh(&dir, &mut manifest, previous.clone(), b"second\n");
        fs::write(root.join(&first.0), b"user edit\n").expect("user edit");

        assert_eq!(
            materialize_source_markdown(
                &dir,
                "companion-a",
                "tea",
                "https://forge.test",
                "issue",
                "acme/repo#1",
                Some("2"),
                "two",
                b"second\n",
            )
            .expect_err("conflict")
            .code,
            "source_sync_conflict"
        );
        assert_eq!(
            fs::read(root.join(&first.0)).expect("preserved"),
            b"user edit\n"
        );
        let preserved = read_manifest(&dir, "companion-a", &association).expect("manifest");
        assert_eq!(preserved.entries, vec![previous]);
        assert!(preserved.pending_source_intent.is_none());
        assert!(
            materialize_source_markdown(
                &dir,
                "companion-a",
                "tea",
                "https://forge.test",
                "issue",
                "acme/repo#2",
                Some("1"),
                "other",
                b"unrelated\n",
            )
            .expect("unrelated source remains importable")
            .1
        );
        fs::remove_dir_all(root).expect("cleanup");
    }
}

#[cfg(test)]
mod library_copy_tests {
    use super::*;

    fn item() -> LibraryItemSummary {
        LibraryItemSummary {
            item_id: "source:item".into(), logical_id: "source:tea:https://forge.test:issue:acme/repo#1".into(),
            kind: LibraryItemKind::ProviderSnapshot, provider_id: Some("tea".into()),
            provider_instance: Some("https://forge.test".into()), resource_type: Some("issue".into()),
            canonical_id: Some("acme/repo#1".into()), container: None, parent_item_id: None,
            ancestors: vec![], order: None, title: "Issue".into(), document_path: Some("item/document.md".into()),
            item_path: "item".into(), source_url: None, original_url: None, source_revision: Some("1".into()),
            revision: "revision-one".into(), state: LibraryItemState::Fresh, partial: None,
            conflict: vec![], fetched_at: None, checked_at: None, follow_id: None, attachments: vec![],
            folder: None, diagnostics: vec![],
        }
    }

    struct Fixture { path: PathBuf, library: Dir, first: Dir, second: Dir }
    impl Fixture {
        fn new() -> Self {
            let path = std::env::temp_dir().join(format!("cockpit-space-copy-{}", Uuid::new_v4()));
            for name in ["library", "first", "second"] { std::fs::create_dir_all(path.join(name)).unwrap(); }
            let open = |name| Dir::open_ambient_dir(path.join(name), cap_std::ambient_authority()).unwrap();
            let library = open("library");
            let first = open("first");
            let second = open("second");
            for dir in [&first, &second] {
                atomic_write_json(dir, "manifest.json", &CompanionManifest {
                    schema_version: 1, cockpit_operation_id: "companion".into(),
                    herdr_session_identity: "endpoint".into(), herdr_workspace_id: "space".into(),
                    repository_key: String::new(), repository_root: String::new(),
                    checkout_path: "/checkout".into(), artifact: None, created_at: timestamp(),
                    updated_at: timestamp(), ownership: "cockpit".into(),
                }).unwrap();
            }
            Self { path, library, first, second }
        }
        fn file(&self, path: &str, bytes: &[u8]) -> crate::library::store::MarkerFile {
            if let Some(parent) = Path::new(path).parent() { self.library.create_dir_all(parent).unwrap(); }
            self.library.write(path, bytes).unwrap();
            crate::library::store::MarkerFile { path: path.into(), hash: hash(bytes), bytes: bytes.len() as u64 }
        }
    }
    impl Drop for Fixture {
        fn drop(&mut self) { let _ = std::fs::remove_dir_all(&self.path); }
    }

    #[test]
    fn independent_inodes_current_no_write_and_edit_protection() {
        let f = Fixture::new();
        let summary = item();
        let files = [f.file("document.md", b"saved bytes")];
        let view = LibraryItemView { root: &f.library, summary: &summary, files: &files };
        let copy = materialize_library_item(&f.first, "companion", &view, LibraryCopyMode::NewOnly, None).unwrap();
        let path = &copy.written[0];
        materialize_library_item(&f.second, "companion", &view, LibraryCopyMode::NewOnly, None).unwrap();
        #[cfg(unix)] {
            use cap_std::fs::MetadataExt;
            let original = f.library.metadata("document.md").unwrap().ino();
            let first = f.first.metadata(path).unwrap().ino();
            let second = f.second.metadata(path).unwrap().ino();
            assert_ne!(original, first); assert_ne!(original, second); assert_ne!(first, second);
        }
        let before = f.first.read(MANIFEST_NAME).unwrap();
        let metadata = f.first.metadata(path).unwrap();
        let current = materialize_library_item(&f.first, "companion", &view, LibraryCopyMode::NewOnly, None).unwrap();
        assert!(current.written.is_empty());
        assert_eq!(f.first.read(MANIFEST_NAME).unwrap(), before);
        assert_eq!(identity(&f.first.metadata(path).unwrap()), identity(&metadata));
        f.first.write(path, b"user edit").unwrap();
        assert_eq!(materialize_library_item(&f.first, "companion", &view, LibraryCopyMode::NewOnly, None).err().unwrap().code, "source_sync_conflict");
        assert_eq!(f.second.read(path).unwrap(), b"saved bytes");
        assert_eq!(f.library.read("document.md").unwrap(), b"saved bytes");
    }

    #[test]
    fn multi_file_precedence_and_missing_parent_restore() {
        let f = Fixture::new();
        let mut summary = item();
        let files = [f.file("document.md", b"body"), f.file("attachments/image.png", b"image bytes")];
        let view = LibraryItemView { root: &f.library, summary: &summary, files: &files };
        let copied = materialize_library_item(&f.first, "companion", &view, LibraryCopyMode::NewOnly, None).unwrap();
        assert!(copied.written[0].ends_with("/document.md"));
        assert!(copied.written[1].ends_with("/attachments/image.png"));
        let rows = |item: &LibraryItemSummary| library_space_rows(&f.first, "companion", std::slice::from_ref(item), &[]).unwrap();
        assert_eq!(rows(&summary)[0].state, SpaceCopyState::UpToDate);
        summary.revision = "revision-two".into();
        assert_eq!(rows(&summary)[0].state, SpaceCopyState::LibraryNewer);
        summary.state = LibraryItemState::RemovedAtSource;
        assert_eq!(rows(&summary)[0].state, SpaceCopyState::RemovedAtSource);
        f.first.write(&copied.written[0], b"edited").unwrap();
        assert_eq!(rows(&summary)[0].state, SpaceCopyState::EditedInSpace);
        assert!(rows(&summary)[0].library_newer);
        assert_eq!(library_space_rows(&f.first, "companion", &[], &[]).unwrap()[0].state, SpaceCopyState::EditedInSpace);
        let parent = Path::new(&copied.written[1]).parent().unwrap();
        f.first.remove_dir_all(parent).unwrap();
        assert_eq!(rows(&summary)[0].state, SpaceCopyState::MissingInSpace);
        f.first.write(&copied.written[0], b"body").unwrap();
        summary.revision = "revision-one".into();
        summary.state = LibraryItemState::Fresh;
        let view = LibraryItemView { root: &f.library, summary: &summary, files: &files };
        let restored = materialize_library_item(&f.first, "companion", &view, LibraryCopyMode::NewOnly, None).unwrap();
        assert_eq!(restored.written, vec![copied.written[1].clone()]);
        assert_eq!(rows(&summary)[0].state, SpaceCopyState::UpToDate);
        assert_eq!(library_space_rows(&f.first, "companion", &[], &[]).unwrap()[0].state, SpaceCopyState::NotInLibrary);
    }

    #[test]
    fn legacy_manifest_is_inert_until_explicit_readd() {
        let f = Fixture::new();
        let summary = item();
        let (legacy_path, _) = materialize_source_markdown(&f.first, "companion",
            "tea", "https://forge.test", "issue", "acme/repo#1", Some("old"), "old-hash", b"legacy bytes").unwrap();
        let mut legacy: serde_json::Value = serde_json::from_slice(&f.first.read(MANIFEST_NAME).unwrap()).unwrap();
        legacy["schema_version"] = 1.into();
        let bytes = serde_json::to_vec_pretty(&legacy).unwrap();
        f.first.write(MANIFEST_NAME, &bytes).unwrap();
        let old_entry = legacy["entries"][0].clone();
        assert_eq!(library_space_rows(&f.first, "companion", std::slice::from_ref(&summary), &[]).unwrap()[0].state, SpaceCopyState::NotLinked);
        assert_eq!(f.first.read(MANIFEST_NAME).unwrap(), bytes);
        assert_eq!(f.first.read(&legacy_path).unwrap(), b"legacy bytes");
        // Another item's v2 write preserves every field of the unlinked entry.
        materialize_source_markdown(&f.first, "companion", "tea", "https://forge.test",
            "issue", "acme/repo#2", None, "two", b"another").unwrap();
        let upgraded: serde_json::Value = serde_json::from_slice(&f.first.read(MANIFEST_NAME).unwrap()).unwrap();
        assert_eq!(upgraded["schema_version"], 2);
        assert_eq!(upgraded["entries"][0], old_entry);
        let files = [f.file("document.md", b"Library bytes")];
        let view = LibraryItemView { root: &f.library, summary: &summary, files: &files };
        let linked = materialize_library_item(&f.first, "companion", &view, LibraryCopyMode::NewOnly, None).unwrap();
        assert_eq!(linked.written, vec![legacy_path.clone()]);
        let rows = library_space_rows(&f.first, "companion", std::slice::from_ref(&summary), &[]).unwrap();
        assert_eq!(rows.iter().find(|row| row.item_id.is_some()).unwrap().state, SpaceCopyState::UpToDate);
        let untouched = rows.iter().find(|row| row.item_id.is_none()).unwrap();
        assert_eq!(untouched.state, SpaceCopyState::NotLinked);
        assert_eq!(f.first.read(&untouched.paths[0]).unwrap(), b"another");
        assert_eq!(f.first.read(legacy_path).unwrap(), b"Library bytes");
    }

    #[test]
    fn interrupted_multi_file_copy_stays_missing_without_any_attempt_record() {
        let f = Fixture::new();
        let summary = item();
        let files = [f.file("document.md", b"body"), f.file("attachments/image.png", b"image")];
        let view = LibraryItemView { root: &f.library, summary: &summary, files: &files };
        LIBRARY_COPY_FAIL_AFTER_FILES.with(|fault| fault.set(Some(1)));
        let failure = materialize_library_item(&f.first, "companion", &view, LibraryCopyMode::NewOnly, None).err().unwrap();
        assert_eq!(failure.code, "library_test_crash");
        // This listing uses a reopened companion and no Library-side attempt
        // storage at all: dismissing an attempt cannot turn the partial copy green.
        let reopened = Dir::open_ambient_dir(f.path.join("first"), cap_std::ambient_authority()).unwrap();
        let before = reopened.read(MANIFEST_NAME).unwrap();
        let rows = library_space_rows(&reopened, "companion", std::slice::from_ref(&summary), &[]).unwrap();
        assert_eq!(rows[0].paths.len(), 1);
        assert_eq!(rows[0].state, SpaceCopyState::MissingInSpace);
        assert_eq!(library_space_rows(&reopened, "companion", &[], &[]).unwrap()[0].state, SpaceCopyState::MissingInSpace);
        assert_eq!(reopened.read(MANIFEST_NAME).unwrap(), before);
        let completed = materialize_library_item(&reopened, "companion", &view, LibraryCopyMode::NewOnly, None).unwrap();
        assert_eq!(completed.written.len(), 1);
        assert!(completed.written[0].ends_with("/attachments/image.png"));
        let rows = library_space_rows(&reopened, "companion", std::slice::from_ref(&summary), &[]).unwrap();
        assert_eq!(rows[0].state, SpaceCopyState::UpToDate);
        assert_eq!(rows[0].paths.len(), 2);
    }

    #[test]
    fn unsafe_library_source_cannot_escape_or_be_hardlinked() {
        let f = Fixture::new();
        let summary = item();
        let mut files = [f.file("document.md", b"bytes")];
        files[0].path = "../outside".into();
        let view = LibraryItemView { root: &f.library, summary: &summary, files: &files };
        assert_eq!(materialize_library_item(&f.first, "companion", &view, LibraryCopyMode::NewOnly, None).err().unwrap().code, "context_snapshot_path");
        #[cfg(unix)] {
            files[0].path = "document.md".into();
            std::fs::hard_link(f.path.join("library/document.md"), f.path.join("alias")).unwrap();
            let view = LibraryItemView { root: &f.library, summary: &summary, files: &files };
            assert_eq!(materialize_library_item(&f.first, "companion", &view, LibraryCopyMode::NewOnly, None).err().unwrap().code, "context_snapshot_hardlink");
        }
        assert!(!f.first.exists(MANIFEST_NAME));
    }

    fn folder_item() -> LibraryItemSummary {
        let mut summary = item();
        summary.kind = LibraryItemKind::FolderCopy;
        summary.item_id = "folder:01234567".into();
        summary.logical_id = "folder:01234567".into();
        summary.item_path = "folders/notes-01234567".into();
        summary.document_path = None;
        summary
    }

    #[test]
    fn empty_folder_remains_linked_through_add_update_and_remove() {
        let f = Fixture::new();
        let mut summary = folder_item();
        materialize_library_item(&f.first, "companion",
            &LibraryItemView { root: &f.library, summary: &summary, files: &[] }, LibraryCopyMode::NewOnly, None).unwrap();
        assert_eq!(library_space_rows(&f.first, "companion", std::slice::from_ref(&summary), &[]).unwrap()[0].state, SpaceCopyState::UpToDate);
        summary.revision = "revision-two".into();
        let files = [f.file("only.txt", b"only")];
        materialize_library_item(&f.first, "companion",
            &LibraryItemView { root: &f.library, summary: &summary, files: &files }, LibraryCopyMode::Update, None).unwrap();
        assert_eq!(f.first.read("folders/notes-01234567/only.txt").unwrap(), b"only");
        summary.revision = "revision-three".into();
        materialize_library_item(&f.first, "companion",
            &LibraryItemView { root: &f.library, summary: &summary, files: &[] }, LibraryCopyMode::Update, None).unwrap();
        assert!(!f.first.exists("folders/notes-01234567/only.txt"));
        let row = library_space_rows(&f.first, "companion", std::slice::from_ref(&summary), &[]).unwrap().remove(0);
        assert_eq!(row.state, SpaceCopyState::UpToDate);
        assert!(row.paths.is_empty());
        assert_eq!(library_space_rows(&f.first, "companion", &[], &[]).unwrap()[0].state, SpaceCopyState::NotInLibrary);
        remove_library_copy(&f.first, "companion", &summary.logical_id, &[]).unwrap();
        assert!(library_space_rows(&f.first, "companion", &[summary], &[]).unwrap().is_empty());
    }

    #[test]
    fn folder_copy_uses_recorded_size_above_the_default_capture_limit() {
        let f = Fixture::new();
        let mut summary = folder_item();
        let mut bytes = vec![b'a'; MAX_SNAPSHOT_FILE_BYTES + 1];
        let files = [f.file("large.txt", &bytes)];
        materialize_library_item(&f.first, "companion",
            &LibraryItemView { root: &f.library, summary: &summary, files: &files }, LibraryCopyMode::NewOnly, None).unwrap();
        let path = "folders/notes-01234567/large.txt";
        assert_eq!(f.first.read(path).unwrap(), bytes);
        assert_eq!(library_space_rows(&f.first, "companion", std::slice::from_ref(&summary), &[]).unwrap()[0].state, SpaceCopyState::UpToDate);
        bytes[0] = b'b';
        summary.revision = "revision-two".into();
        let files = [f.file("large.txt", &bytes)];
        materialize_library_item(&f.first, "companion",
            &LibraryItemView { root: &f.library, summary: &summary, files: &files }, LibraryCopyMode::Update, None).unwrap();
        assert_eq!(f.first.read(path).unwrap(), bytes);
        remove_library_copy(&f.first, "companion", &summary.logical_id, &[]).unwrap();
        assert!(!f.first.exists(path));
    }

    #[test]
    fn interrupted_folder_remove_retains_other_files_and_unmanaged_notes() {
        let f = Fixture::new();
        let summary = folder_item();
        let files = [f.file("a.txt", b"a"), f.file("nested/b.txt", b"b")];
        materialize_library_item(&f.first, "companion",
            &LibraryItemView { root: &f.library, summary: &summary, files: &files }, LibraryCopyMode::NewOnly, None).unwrap();
        let association = read_companion_association(&f.first).unwrap();
        let mut manifest = read_manifest(&f.first, "companion", &association).unwrap();
        let entry = manifest.entries[0].clone();
        manifest.pending_library_remove = Some(entry.clone());
        write_manifest_durable(&f.first, &manifest).unwrap();
        f.first.remove_file(&entry.relative_path).unwrap();
        f.first.write("folders/notes-01234567/personal.txt", b"unmanaged notes").unwrap();
        recover_pending_source_intent(&f.first, &mut manifest).unwrap();
        assert_eq!(manifest.library_copies[0].files, ["nested/b.txt"]);
        assert_eq!(f.first.read("folders/notes-01234567/nested/b.txt").unwrap(), b"b");
        remove_library_copy(&f.first, "companion", &summary.logical_id, &[]).unwrap();
        assert!(!f.first.exists("folders/notes-01234567/nested/b.txt"));
        assert_eq!(f.first.read("folders/notes-01234567/personal.txt").unwrap(), b"unmanaged notes");
    }

    #[test]
    fn folder_add_mirrors_relative_layout_with_per_file_hashes() {
        let f = Fixture::new();
        let summary = folder_item();
        let files = [f.file("README.md", b"readme"), f.file("src/nested/code.rs", b"code")];
        let copied = materialize_library_item(&f.first, "companion",
            &LibraryItemView { root: &f.library, summary: &summary, files: &files }, LibraryCopyMode::NewOnly, None).unwrap();
        assert_eq!(copied.written, ["folders/notes-01234567/README.md", "folders/notes-01234567/src/nested/code.rs"]);
        let manifest = read_manifest(&f.first, "companion", &read_companion_association(&f.first).unwrap()).unwrap();
        for file in &files {
            let entry = manifest.entries.iter().find(|entry| entry.library_file.as_ref() == Some(&file.path)).unwrap();
            assert_eq!(entry.content_hash, file.hash);
            assert_eq!(entry.library_revision.as_ref(), Some(&summary.revision));
            assert_eq!(f.first.read(&entry.relative_path).unwrap(), f.library.read(&file.path).unwrap());
        }
        assert_eq!(library_space_rows(&f.first, "companion", &[summary], &[]).unwrap()[0].state, SpaceCopyState::UpToDate);
    }

    #[test]
    fn folder_update_copies_changes_adds_files_restores_missing_and_deletes_unedited_space_only() {
        let f = Fixture::new();
        let mut summary = folder_item();
        let files = [f.file("keep.txt", b"keep"), f.file("changed.txt", b"old"),
            f.file("nested/missing.txt", b"restore"), f.file("gone.txt", b"gone")];
        materialize_library_item(&f.first, "companion",
            &LibraryItemView { root: &f.library, summary: &summary, files: &files }, LibraryCopyMode::NewOnly, None).unwrap();
        let unchanged = identity(&f.first.metadata("folders/notes-01234567/keep.txt").unwrap());
        f.first.remove_dir_all("folders/notes-01234567/nested").unwrap();
        summary.revision = "revision-two".into();
        let files = [f.file("keep.txt", b"keep"), f.file("changed.txt", b"new"),
            f.file("nested/missing.txt", b"restore"), f.file("added/deep.txt", b"added")];
        let view = LibraryItemView { root: &f.library, summary: &summary, files: &files };
        let copied = materialize_library_item(&f.first, "companion", &view, LibraryCopyMode::Update, None).unwrap();
        assert!(copied.skipped_edited.is_empty());
        assert_eq!(f.first.read("folders/notes-01234567/changed.txt").unwrap(), b"new");
        assert_eq!(f.first.read("folders/notes-01234567/nested/missing.txt").unwrap(), b"restore");
        assert_eq!(f.first.read("folders/notes-01234567/added/deep.txt").unwrap(), b"added");
        assert!(!f.first.exists("folders/notes-01234567/gone.txt"));
        assert_eq!(identity(&f.first.metadata("folders/notes-01234567/keep.txt").unwrap()), unchanged);
        assert_eq!(library_space_rows(&f.first, "companion", std::slice::from_ref(&summary), &[]).unwrap()[0].state, SpaceCopyState::UpToDate);
        let before = f.first.read(MANIFEST_NAME).unwrap();
        assert!(materialize_library_item(&f.first, "companion", &view, LibraryCopyMode::Update, None).unwrap().written.is_empty());
        assert_eq!(f.first.read(MANIFEST_NAME).unwrap(), before);
    }

    #[test]
    fn folder_update_preserves_and_reports_edited_space_only_files() {
        let f = Fixture::new();
        let mut summary = folder_item();
        let files = [f.file("edited.txt", b"baseline"), f.file("gone.txt", b"gone"), f.file("live.txt", b"old")];
        materialize_library_item(&f.first, "companion",
            &LibraryItemView { root: &f.library, summary: &summary, files: &files }, LibraryCopyMode::NewOnly, None).unwrap();
        let edited = "folders/notes-01234567/edited.txt";
        f.first.write(edited, b"my notes").unwrap();
        summary.revision = "revision-two".into();
        let files = [f.file("live.txt", b"new")];
        let copied = materialize_library_item(&f.first, "companion",
            &LibraryItemView { root: &f.library, summary: &summary, files: &files }, LibraryCopyMode::Update, None).unwrap();
        assert_eq!(copied.skipped_edited, [edited]);
        assert_eq!(f.first.read(edited).unwrap(), b"my notes");
        assert_eq!(f.first.read("folders/notes-01234567/live.txt").unwrap(), b"new");
        assert!(!f.first.exists("folders/notes-01234567/gone.txt"));
        let row = library_space_rows(&f.first, "companion", std::slice::from_ref(&summary), &[]).unwrap().remove(0);
        assert_eq!(row.state, SpaceCopyState::EditedInSpace);
        assert_eq!(row.edited.iter().map(|file| (file.path.as_str(), file.current_hash.as_str())).collect::<Vec<_>>(),
            vec![(edited, hash(b"my notes").as_str())]);
        let before = f.first.read(MANIFEST_NAME).unwrap();
        assert_eq!(remove_library_copy(&f.first, "companion", &summary.logical_id, &[]).unwrap_err().code, "space_copy_conflict");
        assert_eq!(f.first.read(MANIFEST_NAME).unwrap(), before);
        remove_library_copy(&f.first, "companion", &summary.logical_id, &row.edited).unwrap();
        assert!(!f.first.exists(edited));
        assert!(!f.first.exists("folders/notes-01234567/live.txt"));
        assert!(library_space_rows(&f.first, "companion", &[summary], &[]).unwrap().is_empty());
    }

    #[test]
    fn folder_confirmation_is_per_path_and_stale_cas_preflights_all_files() {
        let f = Fixture::new();
        let mut summary = folder_item();
        let files = [f.file("a.txt", b"a"), f.file("b.txt", b"b"), f.file("c.txt", b"c")];
        materialize_library_item(&f.first, "companion",
            &LibraryItemView { root: &f.library, summary: &summary, files: &files }, LibraryCopyMode::NewOnly, None).unwrap();
        let a = "folders/notes-01234567/a.txt";
        let b = "folders/notes-01234567/b.txt";
        let c = "folders/notes-01234567/c.txt";
        f.first.write(a, b"edit a").unwrap();
        f.first.write(b, b"edit b").unwrap();
        summary.revision = "revision-two".into();
        let files = [f.file("a.txt", b"new a"), f.file("b.txt", b"new b"), f.file("c.txt", b"new c")];
        let view = LibraryItemView { root: &f.library, summary: &summary, files: &files };
        let stale = [LibraryConflictFile { path: a.into(), current_hash: hash(b"edit a") },
            LibraryConflictFile { path: b.into(), current_hash: hash(b"stale") }];
        let before = f.first.read(MANIFEST_NAME).unwrap();
        assert_eq!(materialize_library_item(&f.first, "companion", &view, LibraryCopyMode::Replace { confirmed: &stale }, None).err().unwrap().code, "space_copy_conflict");
        assert_eq!(f.first.read(a).unwrap(), b"edit a");
        assert_eq!(f.first.read(c).unwrap(), b"c");
        assert_eq!(f.first.read(MANIFEST_NAME).unwrap(), before);
        let copied = materialize_library_item(&f.first, "companion", &view, LibraryCopyMode::Replace { confirmed: &stale[..1] }, None).unwrap();
        assert_eq!(copied.skipped_edited, [b]);
        assert_eq!(f.first.read(a).unwrap(), b"new a");
        assert_eq!(f.first.read(b).unwrap(), b"edit b");
        assert_eq!(f.first.read(c).unwrap(), b"new c");
        let before = f.first.read(MANIFEST_NAME).unwrap();
        assert_eq!(remove_library_copy(&f.first, "companion", &summary.logical_id, &stale[1..]).unwrap_err().code, "space_copy_conflict");
        assert_eq!(f.first.read(a).unwrap(), b"new a");
        assert_eq!(f.first.read(MANIFEST_NAME).unwrap(), before);
        let confirmed = [LibraryConflictFile { path: b.into(), current_hash: hash(b"edit b") }];
        materialize_library_item(&f.first, "companion", &view, LibraryCopyMode::Replace { confirmed: &confirmed }, None).unwrap();
        assert_eq!(f.first.read(b).unwrap(), b"new b");
        assert_eq!(library_space_rows(&f.first, "companion", std::slice::from_ref(&summary), &[]).unwrap()[0].state, SpaceCopyState::UpToDate);
        f.first.remove_file(c).unwrap();
        remove_library_copy(&f.first, "companion", &summary.logical_id, &[]).unwrap();
        assert!(library_space_rows(&f.first, "companion", &[summary], &[]).unwrap().is_empty());
    }

    #[test]
    fn confirmed_replace_recovery_preserves_baseline_and_adopts_only_published_bytes() {
        let f = Fixture::new();
        let mut summary = item();
        let files = [f.file("document.md", b"body")];
        let copied = materialize_library_item(&f.first, "companion",
            &LibraryItemView { root: &f.library, summary: &summary, files: &files }, LibraryCopyMode::NewOnly, None).unwrap();
        let path = &copied.written[0];
        f.first.write(path, b"my edit").unwrap();
        summary.revision = "revision-two".into();
        let files = [f.file("document.md", b"new body")];
        let view = LibraryItemView { root: &f.library, summary: &summary, files: &files };
        let wrong = LibraryConflictFile { path: path.clone(), current_hash: hash(b"stale edit") };
        assert_eq!(materialize_library_item(&f.first, "companion", &view,
            LibraryCopyMode::Replace { confirmed: std::slice::from_ref(&wrong) }, None).err().unwrap().code, "space_copy_conflict");
        assert_eq!(f.first.read(path).unwrap(), b"my edit");

        let association = read_companion_association(&f.first).unwrap();
        let mut manifest = read_manifest(&f.first, "companion", &association).unwrap();
        let previous = manifest.entries[0].clone();
        let mut intended = previous.clone();
        intended.content_hash = files[0].hash.clone();
        intended.library_revision = Some(summary.revision.clone());
        manifest.pending_source_intent = Some(PendingSourceIntent {
            schema_version: PENDING_SOURCE_INTENT_SCHEMA_VERSION, relative_path: path.clone(),
            previous_written_hash: Some(previous.content_hash.clone()), previous_entry: Some(previous.clone()),
            new_written_hash: intended.content_hash.clone(), intended_entry: intended.clone(),
            expected_previous_hash: Some(hash(b"my edit")),
        });
        write_manifest_durable(&f.first, &manifest).unwrap();
        recover_pending_source_intent(&f.first, &mut manifest).unwrap();
        assert_eq!(manifest.entries[0], previous);
        assert_eq!(f.first.read(path).unwrap(), b"my edit");
        let confirmed = LibraryConflictFile { path: path.clone(), current_hash: hash(b"my edit") };
        materialize_library_item(&f.first, "companion", &view,
            LibraryCopyMode::Replace { confirmed: std::slice::from_ref(&confirmed) }, None).unwrap();
        assert_eq!(f.first.read(path).unwrap(), b"new body");
        assert_eq!(library_space_rows(&f.first, "companion", &[summary], &[]).unwrap()[0].state, SpaceCopyState::UpToDate);
    }

    #[test]
    fn removal_intent_recovery_never_deletes_new_user_bytes() {
        let f = Fixture::new();
        let summary = item();
        let files = [f.file("document.md", b"body")];
        let copied = materialize_library_item(&f.first, "companion",
            &LibraryItemView { root: &f.library, summary: &summary, files: &files }, LibraryCopyMode::NewOnly, None).unwrap();
        let path = &copied.written[0];
        let association = read_companion_association(&f.first).unwrap();
        let mut manifest = read_manifest(&f.first, "companion", &association).unwrap();
        let entry = manifest.entries[0].clone();
        manifest.pending_library_remove = Some(entry.clone());
        write_manifest_durable(&f.first, &manifest).unwrap();
        f.first.write(path, b"edit after intent").unwrap();
        recover_pending_source_intent(&f.first, &mut manifest).unwrap();
        assert_eq!(f.first.read(path).unwrap(), b"edit after intent");
        assert_eq!(manifest.entries, vec![entry.clone()]);
        manifest.pending_library_remove = Some(entry);
        write_manifest_durable(&f.first, &manifest).unwrap();
        f.first.remove_file(path).unwrap();
        recover_pending_source_intent(&f.first, &mut manifest).unwrap();
        assert!(manifest.entries.is_empty());
        assert!(manifest.library_copies.is_empty());
        assert!(library_space_rows(&f.first, "companion", &[summary], &[]).unwrap().is_empty());
    }
}
