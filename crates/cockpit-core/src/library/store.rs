use crate::{
    InspectionError,
    project_store::{atomic_write_bytes, prepare_root, read_json_bounded},
};
use cap_fs_ext::{DirExt, OpenOptionsFollowExt, OpenOptionsSyncExt};
use cap_std::fs::{Dir, OpenOptions};
use cockpit_protocol::{library::*, projects::ProjectDiagnostic};
use fs2::FileExt;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    fs::File,
    io,
    path::{Component, Path, PathBuf},
    sync::{Arc, Mutex},
};
use uuid::Uuid;

mod inventory;
mod publication;
mod recovery;
use inventory::{
    check_confirmation, conflicts, inventory, owned_inventory, owned_roots, safe_file_path,
    verify_entry,
};

const MAX_INDEX: u64 = 64 * 1024 * 1024;
const MAX_RECORD: u64 = 64 * 1024 * 1024;
const MAX_FILE: u64 = 1024 * 1024 * 1024;
const MAX_TREE_ENTRIES: usize = 1_000_000;
const MAX_TREE_BYTES: u64 = 4 * 1024 * 1024 * 1024 + MAX_RECORD;
const MAX_TREE_DEPTH: usize = 64;

/// Current index format with durable Space selections.
const SCHEMA: u32 = 4;
const INTENT_SCHEMA: u32 = 2;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct LibraryIndexEntry {
    pub summary: LibraryItemSummary,
    /// Validated provider URL used for future refreshes, never the user's input.
    pub canonical_url: Option<String>,
    /// Trusted inventory of this item's owned entries; not reconstructed from disk.
    pub inventory: Vec<MarkerFile>,
    /// Outgoing references extracted when the content was saved, when applicable.
    pub references: Option<Vec<crate::sources::SourceReference>>,
    /// Whether the saved Jira document carries structured `parent`/`subtasks`/`links`.
    pub relations_captured: bool,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct SpaceContextRecord {
    pub space_context_id: String,
    pub session_id: String,
    pub space_id: String,
    pub item_ids: Vec<String>,
    pub repository_paths: Vec<String>,
    pub legacy_migrated: bool,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Index {
    schema: u32,
    pub generation: String,
    pub items: Vec<LibraryIndexEntry>,
    pub follows: Vec<LibraryFollowSummary>,
    pub space_contexts: Vec<SpaceContextRecord>,
}
impl Default for Index {
    fn default() -> Self {
        Self {
            schema: SCHEMA,
            generation: Uuid::new_v4().to_string(),
            items: vec![],
            follows: vec![],
            space_contexts: vec![],
        }
    }
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct MarkerFile {
    pub path: String,
    pub hash: String,
    pub bytes: u64,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
enum Method {
    Exchange,
    TwoRename,
    NewTarget,
    Remove,
    RemoveOwned,
    Merge,
    Move,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Intent {
    schema: u32,
    intent_id: String,
    op: String,
    method: Method,
    item_id: String,
    target: String,
    staging: Option<String>,
    backup: String,
    previous_revision: Option<String>,
    new_entry: Option<LibraryIndexEntry>,
    old_entry: Option<LibraryIndexEntry>,
    predecessor: Option<Vec<MarkerFile>>,
    source: Option<String>,
    moved_entries: Option<Vec<LibraryIndexEntry>>,
    move_inventory: Option<Vec<MarkerFile>>,
}

pub(crate) struct Store {
    pub path: PathBuf,
    pub root: Dir,
    pub meta: Dir,
    pub operations: Dir,
    staging: Dir,
    trash: Dir,
    journal: Dir,
    locks: Dir,
    max_items: usize,
    index_cache: Mutex<Option<((u64, u64, u64, Option<cap_std::time::SystemTime>), Arc<Index>)>>,
    #[cfg(test)]
    pub fault: std::sync::Mutex<Option<&'static str>>,
    #[cfg(test)]
    pub force_two_rename: std::sync::atomic::AtomicBool,
}
pub(crate) struct Stage {
    store: Arc<Store>,
    pub name: String,
    pub dir: Dir,
    owned: bool,
}
impl Drop for Stage {
    fn drop(&mut self) {
        if self.owned {
            let _ = self.store.staging.remove_dir_all(&self.name);
        }
    }
}
pub(crate) struct Lease {
    _file: File,
}
impl Drop for Lease {
    fn drop(&mut self) {
        // A concurrent fork can retain the open file description until exec.
        // Release ownership now rather than waiting for every descriptor to close.
        let _ = FileExt::unlock(&self._file);
    }
}

pub(crate) fn error(code: &str, message: impl Into<String>) -> InspectionError {
    InspectionError::new(code, message)
}
fn io_error(e: io::Error) -> InspectionError {
    error("library_unavailable", e.to_string())
}
fn corrupt(message: impl Into<String>) -> InspectionError {
    error("library_corrupt", message)
}
pub(crate) fn hash(bytes: &[u8]) -> String {
    format!("sha256:{:x}", Sha256::digest(bytes))
}
fn component(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 255
        && Path::new(value).components().count() == 1
        && matches!(
            Path::new(value).components().next(),
            Some(Component::Normal(_))
        )
        && !value.contains('\\')
}
pub(super) fn open_child(parent: &Dir, name: &str) -> Result<Dir, InspectionError> {
    match parent.create_dir(name) {
        Ok(()) => sync(parent)?,
        Err(e) if e.kind() == io::ErrorKind::AlreadyExists => {}
        Err(e) => return Err(io_error(e)),
    }
    parent.open_dir_nofollow(name).map_err(io_error)
}
fn sync(dir: &Dir) -> Result<(), InspectionError> {
    #[cfg(any(all(target_os = "linux", target_env = "gnu"), target_os = "macos"))]
    {
        let fd = rustix::fs::openat(
            dir,
            ".",
            rustix::fs::OFlags::RDONLY
                | rustix::fs::OFlags::DIRECTORY
                | rustix::fs::OFlags::NOFOLLOW,
            rustix::fs::Mode::empty(),
        )
        .map_err(|e| io_error(e.into()))?;
        #[cfg(target_os = "macos")]
        return rustix::fs::fcntl_fullfsync(fd).map_err(|e| io_error(e.into()));
        #[cfg(not(target_os = "macos"))]
        return rustix::fs::fsync(fd).map_err(|e| io_error(e.into()));
    }
    #[cfg(not(any(all(target_os = "linux", target_env = "gnu"), target_os = "macos")))]
    Err(error(
        "library_unavailable",
        "durable Library publication is unsupported on this platform",
    ))
}
fn rename_special(
    from: &Dir,
    source: &str,
    to: &Dir,
    target: &str,
    exchange: bool,
) -> io::Result<()> {
    #[cfg(all(target_os = "linux", target_env = "gnu"))]
    {
        use nix::fcntl::{RenameFlags, renameat2};
        renameat2(
            from,
            source,
            to,
            target,
            if exchange {
                RenameFlags::RENAME_EXCHANGE
            } else {
                RenameFlags::RENAME_NOREPLACE
            },
        )
        .map_err(|e| io::Error::from_raw_os_error(e as i32))
    }
    #[cfg(target_os = "macos")]
    {
        use rustix::fs::{RenameFlags, renameat_with};
        renameat_with(
            from,
            source,
            to,
            target,
            if exchange {
                RenameFlags::EXCHANGE
            } else {
                RenameFlags::NOREPLACE
            },
        )
        .map_err(Into::into)
    }
    #[cfg(not(any(all(target_os = "linux", target_env = "gnu"), target_os = "macos")))]
    {
        Err(io::Error::new(
            io::ErrorKind::Unsupported,
            "Library atomic rename unsupported",
        ))
    }
}
fn lock_file(dir: &Dir, name: &str) -> Result<File, InspectionError> {
    let mut options = OpenOptions::new();
    options
        .read(true)
        .write(true)
        .create(true)
        .follow(cap_fs_ext::FollowSymlinks::No)
        .nonblock(true);
    let file = dir.open_with(name, &options).map_err(io_error)?.into_std();
    if !file.metadata().map_err(io_error)?.is_file() {
        return Err(corrupt("lock is not a regular file"));
    }
    Ok(file)
}
fn item_path(value: &str) -> bool {
    let mut components = value.split('/');
    let Some(first) = components.next() else {
        return false;
    };
    component(first)
        && first != ".cockpit"
        && components
            .all(|part| component(part) && part != "." && part != "..")
        && value.split('/').count() <= MAX_TREE_DEPTH
}

impl Store {
    pub fn open(path: &Path, max_items: usize) -> Result<Arc<Self>, InspectionError> {
        let (path, root) =
            prepare_root(path, "Library").map_err(|e| error("library_unavailable", e.message))?;
        let meta = open_child(&root, ".cockpit")?;
        let store = Arc::new(Self {
            path,
            root,
            operations: open_child(&meta, "operations")?,
            staging: open_child(&meta, "staging")?,
            trash: open_child(&meta, "trash")?,
            journal: open_child(&meta, "journal")?,
            locks: open_child(&meta, "locks")?,
            meta,
            max_items,
            index_cache: Mutex::new(None),
            #[cfg(test)]
            fault: std::sync::Mutex::new(None),
            #[cfg(test)]
            force_two_rename: std::sync::atomic::AtomicBool::new(false),
        });
        let _lock = store.exclusive()?;
        if !exists(&store.meta, "index.json")? {
            store.commit(&mut Index::default())?;
        }
        store.ensure_readme()?;
        store.recover()?;
        Ok(store)
    }
    fn ensure_readme(&self) -> Result<(), InspectionError> {
        const README: &str = "# Library\n\nThis directory is a human-readable mirror of saved provider content.\nProvider items are nested under provider, host, and source hierarchy; each Markdown document is named after its title. Attachments are stored in `_files/` beside the document. `.cockpit/` contains private Library index, journal, staging, and lock state and must not be edited.\n";
        let current = self.root.read("README.md").ok();
        if current.as_deref() != Some(README.as_bytes()) {
            atomic_write_bytes(&self.root, "README.md", README.as_bytes()).map_err(io_error)?;
        }
        Ok(())
    }
    pub fn recover_pending(&self) -> Result<(), InspectionError> {
        let _lock = self.exclusive()?;
        self.recover()
    }
    pub fn shared(&self) -> Result<Lease, InspectionError> {
        let file = lock_file(&self.meta, "library.lock")?;
        FileExt::lock_shared(&file).map_err(io_error)?;
        Ok(Lease { _file: file })
    }
    pub fn exclusive(&self) -> Result<Lease, InspectionError> {
        let file = lock_file(&self.meta, "library.lock")?;
        FileExt::lock_exclusive(&file).map_err(io_error)?;
        Ok(Lease { _file: file })
    }
    pub fn try_exclusive(&self) -> Result<Option<Lease>, InspectionError> {
        let file = lock_file(&self.meta, "library.lock")?;
        match FileExt::try_lock_exclusive(&file) {
            Ok(()) => Ok(Some(Lease { _file: file })),
            Err(e) if e.kind() == io::ErrorKind::WouldBlock => Ok(None),
            Err(e) => Err(io_error(e)),
        }
    }
    pub fn lease(&self, id: &str) -> Result<Lease, InspectionError> {
        let file = lock_file(
            &self.locks,
            &format!("{:x}.lock", Sha256::digest(id.as_bytes())),
        )?;
        file.try_lock_exclusive().map_err(|e| {
            if e.kind() == io::ErrorKind::WouldBlock {
                error(
                    "library_item_busy",
                    "Library item is already being modified",
                )
            } else {
                io_error(e)
            }
        })?;
        Ok(Lease { _file: file })
    }
    /// Caller holds shared or exclusive library.lock.
    pub fn index(&self) -> Result<Index, InspectionError> {
        Ok(self.index_shared()?.as_ref().clone())
    }
    pub fn index_shared(&self) -> Result<Arc<Index>, InspectionError> {
        if !exists(&self.meta, "index.json")? {
            return Ok(Arc::new(Index::default()));
        }
        let metadata = self.meta.symlink_metadata("index.json").map_err(io_error)?;
        if metadata.file_type().is_symlink() || !metadata.is_file() || metadata.len() > MAX_INDEX {
            return Err(corrupt("Library index is not a bounded regular file"));
        }
        let identity = (
            cap_fs_ext::MetadataExt::dev(&metadata),
            cap_fs_ext::MetadataExt::ino(&metadata),
            metadata.len(),
            metadata.modified().ok(),
        );
        if let Some((cached_identity, index)) = self.index_cache.lock().unwrap_or_else(|p| p.into_inner()).as_ref()
            && *cached_identity == identity {
            return Ok(Arc::clone(index));
        }
        let index: Index = read_json_bounded(&self.meta, "index.json", MAX_INDEX)
            .map_err(|e| corrupt(e.message))?;
        if index.schema != SCHEMA || index.items.len() > 1_000_000 {
            return Err(corrupt("unsupported or oversized Library index"));
        }
        let mut ids = std::collections::HashSet::new();
        let mut paths = std::collections::HashSet::new();
        for entry in &index.items {
            validate_entry(entry)?;
            if !ids.insert(&entry.summary.item_id) || !paths.insert(&entry.summary.item_path) {
                return Err(corrupt("duplicate Library identity or path"));
            }
        }
        let mut contexts = std::collections::HashSet::new();
        for context in &index.space_contexts {
            if context.space_context_id.is_empty() || context.session_id.is_empty()
                || context.space_id.is_empty() || context.item_ids.len() > 1_000_000
                || context.repository_paths.len() > 64
                || !contexts.insert(&context.space_context_id)
            {
                return Err(corrupt("invalid or duplicate Space selection identity"));
            }
            if context.item_ids.windows(2).any(|pair| pair[0] >= pair[1])
                || context.item_ids.iter().any(String::is_empty) {
                return Err(corrupt("invalid Space selected item IDs"));
            }
        }
        let index = Arc::new(index);
        *self.index_cache.lock().unwrap_or_else(|p| p.into_inner()) = Some((identity, Arc::clone(&index)));
        Ok(index)
    }
    fn commit(&self, index: &mut Index) -> Result<(), InspectionError> {
        index.generation = Uuid::new_v4().to_string();
        bounded_write(&self.meta, "index.json", index, MAX_INDEX)
    }
    /// Replaces an item's entry at an unchanged revision. The caller's `refs`,
    /// `purge_after`, `reference_depth` and `included_by` are ignored: they change
    /// only through `mutate_index` and `add_ref`.
    pub fn update(&self, entry: LibraryIndexEntry) -> Result<(), InspectionError> {
        self.update_for_reference(entry, None)
    }
    /// Revalidate the holder in the same critical section as the metadata save.
    pub(super) fn update_for_reference(
        &self,
        entry: LibraryIndexEntry,
        reference: Option<&LibraryItemRef>,
    ) -> Result<(), InspectionError> {
        let _lock = self.exclusive()?;
        let mut index = self.index()?;
        Self::check_reference(&index, &entry.summary, reference)?;
        let old = index
            .items
            .iter_mut()
            .find(|e| e.summary.item_id == entry.summary.item_id)
            .ok_or_else(|| error("library_item_not_found", "Library item no longer exists"))?;
        if old.summary.revision != entry.summary.revision {
            return Err(error("library_conflict", "Library revision changed"));
        }
        upsert(&mut index, entry);
        self.commit(&mut index)
    }
    /// Adds a reference to an existing item, clearing its purge mark.
    pub fn add_ref(&self, item_id: &str, reference: LibraryItemRef) -> Result<(), InspectionError> {
        self.mutate_index_if(|index| {
            let entry = index
                .items
                .iter()
                .find(|e| e.summary.item_id == item_id)
                .ok_or_else(|| error("library_item_not_found", "Library item no longer exists"))?;
            Self::check_reference(index, &entry.summary, Some(&reference))?;
            let entry = index.items.iter_mut().find(|e| e.summary.item_id == item_id).unwrap();
            let changed = !entry.summary.refs.contains(&reference) || entry.summary.purge_after.is_some();
            crate::library::refs::insert_ref(&mut entry.summary, reference);
            Ok(((), changed))
        })
    }
    /// Called with the current index under library.lock. Canonical exclusions
    /// apply only to the follow's own site; Library-id exclusions are global.
    pub(super) fn reference_allowed(
        index: &Index,
        summary: &LibraryItemSummary,
        reference: &LibraryItemRef,
    ) -> bool {
        let LibraryItemRef::Follow { follow_id } = reference else { return true; };
        index.follows.iter().find(|follow| &follow.follow_id == follow_id).is_some_and(|follow| {
            !follow.excluded_ids.contains(&summary.item_id)
                && !(summary.provider_id.as_deref() == Some(follow.provider_id.as_str())
                    && summary.provider_instance.as_deref() == Some(follow.provider_instance.as_str())
                    && summary.canonical_id.as_ref().is_some_and(|id| follow.excluded_ids.contains(id)))
        })
    }
    fn check_reference(
        index: &Index,
        summary: &LibraryItemSummary,
        reference: Option<&LibraryItemRef>,
    ) -> Result<(), InspectionError> {
        if reference.is_some_and(|reference| !Self::reference_allowed(index, summary, reference)) {
            return Err(error("library_follow_excluded", "Follow was stopped or this item was excluded"));
        }
        Ok(())
    }
    /// Commit presentation and follow-record changes that never touch item files.
    /// The closure must preserve every item's revision, path and inventory.
    pub fn mutate_index<R>(
        &self,
        change: impl FnOnce(&mut Index) -> Result<R, InspectionError>,
    ) -> Result<R, InspectionError> {
        let _lock = self.exclusive()?;
        self.mutate_index_locked(change)
    }
    /// Like `mutate_index`, but a metadata-only no-op does not publish a generation.
    pub(super) fn mutate_index_if<R>(
        &self,
        change: impl FnOnce(&mut Index) -> Result<(R, bool), InspectionError>,
    ) -> Result<R, InspectionError> {
        let _lock = self.exclusive()?;
        let mut index = self.index()?;
        let before = index.items.iter().map(|e|
            (e.summary.item_id.clone(), e.summary.revision.clone(), e.summary.item_path.clone()))
            .collect::<Vec<_>>();
        let (result, changed) = change(&mut index)?;
        if !changed { return Ok(result); }
        let after = index.items.iter().map(|e|
            (e.summary.item_id.clone(), e.summary.revision.clone(), e.summary.item_path.clone()))
            .collect::<Vec<_>>();
        if before != after {
            return Err(error("library_conflict", "Index mutation changed item identity"));
        }
        self.commit(&mut index)?;
        Ok(result)
    }
    /// Caller must already hold the exclusive Library lock.
    pub(super) fn mutate_index_locked<R>(
        &self,
        change: impl FnOnce(&mut Index) -> Result<R, InspectionError>,
    ) -> Result<R, InspectionError> {
        let mut index = self.index()?;
        let before = index
            .items
            .iter()
            .map(|e| (e.summary.item_id.clone(), e.summary.revision.clone(), e.summary.item_path.clone()))
            .collect::<Vec<_>>();
        let result = change(&mut index)?;
        let after = index
            .items
            .iter()
            .map(|e| (e.summary.item_id.clone(), e.summary.revision.clone(), e.summary.item_path.clone()))
            .collect::<Vec<_>>();
        if before != after {
            return Err(error("library_conflict", "Index mutation changed item identity"));
        }
        self.commit(&mut index)?;
        Ok(result)
    }
    fn item_parent<'a>(&self, path: &'a str) -> Result<(Dir, &'a str), InspectionError> {
        if !item_path(path) {
            return Err(corrupt("invalid Library item path"));
        }
        let (parent, leaf) = path.rsplit_once('/').unwrap_or(("", path));
        let mut dir = self.root.try_clone().map_err(io_error)?;
        if !parent.is_empty() {
            for component in parent.split('/') {
                dir = dir.open_dir_nofollow(component).map_err(io_error)?;
            }
        }
        Ok((dir, leaf))
    }
    fn create_item_parent<'a>(&self, path: &'a str) -> Result<(Dir, &'a str), InspectionError> {
        if !item_path(path) {
            return Err(corrupt("invalid Library item path"));
        }
        let (parent, leaf) = path.rsplit_once('/').unwrap_or(("", path));
        let mut dir = self.root.try_clone().map_err(io_error)?;
        if !parent.is_empty() {
            for component in parent.split('/') {
                dir = open_child(&dir, component)?;
            }
        }
        Ok((dir, leaf))
    }
    pub(crate) fn item_dir(&self, path: &str) -> Result<Dir, InspectionError> {
        let (parent, leaf) = self.item_parent(path)?;
        parent.open_dir_nofollow(leaf).map_err(io_error)
    }
    pub fn stage(self: &Arc<Self>) -> Result<Stage, InspectionError> {
        let name = Uuid::new_v4().to_string();
        self.staging.create_dir(&name).map_err(io_error)?;
        let dir = self.staging.open_dir_nofollow(&name).map_err(io_error)?;
        Ok(Stage {
            store: self.clone(),
            name,
            dir,
            owned: true,
        })
    }
    /// Only the store materializes provider documents.
    pub fn stage_asset(
        self: &Arc<Self>,
        entry: &mut LibraryIndexEntry,
        asset: &crate::sources::SourceAsset,
    ) -> Result<Stage, InspectionError> {
        self.stage_asset_into(self.stage()?, entry, asset, vec![])
    }
    pub fn stage_asset_into(
        &self,
        stage: Stage,
        entry: &mut LibraryIndexEntry,
        asset: &crate::sources::SourceAsset,
        mut files: Vec<MarkerFile>,
    ) -> Result<Stage, InspectionError> {
        let markdown = crate::sources::library_markdown(asset, &crate::sources::content_revision(asset));
        let keys = format!(
            "---\nlibrary_item_id: {}\nlibrary_revision: {}\n",
            serde_json::to_string(&entry.summary.item_id).unwrap(),
            serde_json::to_string(&entry.summary.revision).unwrap()
        );
        let markdown = keys
            + markdown
                .strip_prefix("---\n")
                .ok_or_else(|| corrupt("source document has no frontmatter"))?;
        let document = entry
            .summary
            .document_path
            .as_deref()
            .and_then(|path| path.strip_prefix(&format!("{}/", entry.summary.item_path)))
            .filter(|name| component(name))
            .ok_or_else(|| corrupt("provider item has invalid document path"))?;
        atomic_write_bytes(&stage.dir, document, markdown.as_bytes()).map_err(io_error)?;
        files.push(MarkerFile {
            path: document.into(),
            hash: hash(markdown.as_bytes()),
            bytes: markdown.len() as u64,
        });
        self.seal(&stage, entry, files)?;
        Ok(stage)
    }
    pub fn seal(
        &self,
        stage: &Stage,
        entry: &mut LibraryIndexEntry,
        _files: Vec<MarkerFile>,
    ) -> Result<(), InspectionError> {
        entry.inventory = owned_inventory(&stage.dir, entry)?;
        // Newly created intermediate directories must be durable as well as the
        // item root: a file's fsync alone does not persist its ancestors.
        for directory in inventory(&stage.dir)?.iter().rev().filter(|file| file.hash == "directory") {
            let mut dir = stage.dir.try_clone().map_err(io_error)?;
            for component in Path::new(&directory.path).components() {
                dir = dir.open_dir_nofollow(component.as_os_str()).map_err(io_error)?;
            }
            sync(&dir)?;
        }
        sync(&stage.dir)?;
        sync(&self.staging)
    }
    pub fn conflicts(
        &self,
        entry: &LibraryIndexEntry,
    ) -> Result<Vec<LibraryConflictFile>, InspectionError> {
        let dir = self.item_dir(&entry.summary.item_path)?;
        Ok(conflicts(entry, &owned_inventory(&dir, entry)?))
    }
    pub fn check_confirmation(
        &self,
        entry: &LibraryIndexEntry,
        confirmed: Option<&[LibraryConflictFile]>,
    ) -> Result<(), InspectionError> {
        check_confirmation(&self.conflicts(entry)?, confirmed)
    }
    pub fn publish(
        &self,
        stage: Stage,
        entry: LibraryIndexEntry,
        previous: Option<&str>,
        confirmed: Option<&[LibraryConflictFile]>,
    ) -> Result<(), InspectionError> {
        self.publish_for_reference(stage, entry, previous, confirmed, None)
    }
    /// The caller holds the item lease; check the current holder under the
    /// publication lock, after all remote attachment work has finished.
    pub(super) fn publish_for_reference(
        &self,
        mut stage: Stage,
        entry: LibraryIndexEntry,
        previous: Option<&str>,
        confirmed: Option<&[LibraryConflictFile]>,
        reference: Option<&LibraryItemRef>,
    ) -> Result<(), InspectionError> {
        validate_entry(&entry)?;
        verify_entry(&stage.dir, &entry)?;
        let _lock = self.exclusive()?;
        self.recover()?;
        let mut index = self.index()?;
        Self::check_reference(&index, &entry.summary, reference)?;
        let (target_root, target_name) = self.create_item_parent(&entry.summary.item_path)?;
        sync(&target_root)?;
        let target_name = target_name.to_owned();
        let (old, predecessor) = self.prepare_publication(
            &mut index, &entry, previous, confirmed, &target_root, &target_name,
        )?;
        upsert(&mut index, entry.clone());
        index.generation = Uuid::new_v4().to_string();
        let method = if let Some(old) = &old {
            if old.summary.kind == LibraryItemKind::FolderCopy {
                Method::Exchange
            } else {
                Method::Merge
            }
        } else {
            Method::NewTarget
        };
        #[cfg(test)]
        let method = if old
            .as_ref()
            .is_some_and(|entry| entry.summary.kind == LibraryItemKind::FolderCopy)
            && self
                .force_two_rename
                .load(std::sync::atomic::Ordering::SeqCst)
        {
            Method::TwoRename
        } else {
            method
        };
        if let Some(old) = &old {
            if old.summary.kind != LibraryItemKind::FolderCopy {
                let owned = owned_roots(old);
                for name in owned_roots(&entry) {
                    if exists(&target_root, &target_name)?
                        && !owned.iter().any(|old_name| old_name == &name)
                        && exists(
                            &target_root.open_dir_nofollow(&target_name).map_err(io_error)?,
                            &name,
                        )?
                    {
                        return Err(error(
                            "library_conflict",
                            format!("Library destination entry is already occupied: {name}"),
                        ));
                    }
                }
            }
        }
        let id = Uuid::new_v4().to_string();
        let mut intent = Intent {
            schema: INTENT_SCHEMA,
            intent_id: id.clone(),
            op: "publish".into(),
            method,
            item_id: entry.summary.item_id.clone(),
            target: entry.summary.item_path.clone(),
            staging: Some(stage.name.clone()),
            backup: id,
            previous_revision: previous.map(str::to_owned),
            old_entry: old.clone(),
            predecessor,
            source: None,
            moved_entries: None,
            move_inventory: None,
            new_entry: Some(entry),
        };
        self.write_intent(&intent)?;
        // Once journaled, only recovery owns this stage, including after an I/O failure.
        stage.owned = false;
        let result = self.apply_publication(
            &stage, &target_root, &target_name, &mut intent, &mut index,
        );
        if let Err(e) = &result {
            if e.code != "library_test_crash" {
                self.recover()?;
            }
        }
        result
    }
    pub fn remove(&self, id: &str, revision: &str) -> Result<(), InspectionError> {
        self.remove_where(id, revision, |_| Ok(()))
    }
    /// Removes an item once `guard` accepts its current entry. The guard runs
    /// under the item lease and the exclusive library lock, so a check on
    /// `refs` cannot race a concurrent `add_ref`.
    pub fn remove_where(
        &self,
        id: &str,
        revision: &str,
        guard: impl FnOnce(&LibraryIndexEntry) -> Result<(), InspectionError>,
    ) -> Result<(), InspectionError> {
        let _lease = self.lease(id)?;
        let _lock = self.exclusive()?;
        let mut index = self.index()?;
        let entry = index
            .items
            .iter()
            .find(|e| e.summary.item_id == id)
            .ok_or_else(|| error("library_item_not_found", "Library item does not exist"))?;
        if entry.summary.revision != revision {
            return Err(error("library_conflict", "Library revision changed"));
        }
        guard(entry)?;
        let (target_root, target_name) = self.item_parent(&entry.summary.item_path)?;
        let predecessor = owned_inventory(&self.item_dir(&entry.summary.item_path)?, entry)?;
        check_confirmation(&conflicts(entry, &predecessor), None)?;
        let intent_id = Uuid::new_v4().to_string();
        let intent = Intent {
            schema: INTENT_SCHEMA,
            intent_id: intent_id.clone(),
            op: "remove".into(),
            method: if entry.summary.kind == LibraryItemKind::FolderCopy {
                Method::Remove
            } else {
                Method::RemoveOwned
            },
            item_id: id.into(),
            target: entry.summary.item_path.clone(),
            staging: None,
            backup: intent_id,
            previous_revision: Some(revision.into()),
            old_entry: Some(entry.clone()),
            predecessor: Some(predecessor),
            source: None,
            moved_entries: None,
            move_inventory: None,
            new_entry: None,
        };
        self.write_intent(&intent)?;
        if intent.method == Method::Remove {
            rename_special(
                &target_root,
                target_name,
                &self.trash,
                &intent.backup,
                false,
            ).map_err(io_error)?;
        } else {
            let target = self.item_dir(&entry.summary.item_path)?;
            let backup = open_child(&self.trash, &intent.backup)?;
            for name in owned_roots(entry) {
                if exists(&target, &name)? {
                    rename_special(&target, &name, &backup, &name, false).map_err(io_error)?;
                }
            }
        }
        self.fault("rename_unsynced")?;
        sync(&target_root)?;
        sync(&self.trash)?;
        self.fault("remove_to_backup")?;
        index.items.retain(|e| e.summary.item_id != id);
        self.commit(&mut index)?;
        self.fault("index_commit")?;
        self.finish(&intent)?;
        if intent.method == Method::RemoveOwned {
            self.prune_empty(&intent.target)?;
        }
        Ok(())
    }
    fn prune_empty(&self, path: &str) -> Result<(), InspectionError> {
        let parts = path.split('/').collect::<Vec<_>>();
        for count in (2..=parts.len()).rev() {
            let prefix = parts[..count].join("/");
            let (parent, leaf) = self.item_parent(&prefix)?;
            let Ok(dir) = parent.open_dir_nofollow(leaf) else {
                continue;
            };
            if dir.entries().map_err(io_error)?.next().is_none() {
                parent.remove_dir(leaf).map_err(io_error)?;
                sync(&parent)?;
            } else {
                break;
            }
        }
        Ok(())
    }
    fn write_intent(&self, intent: &Intent) -> Result<(), InspectionError> {
        bounded_write(
            &self.journal,
            &format!("{}.json", intent.intent_id),
            intent,
            MAX_RECORD,
        )
    }
    fn finish(&self, intent: &Intent) -> Result<(), InspectionError> {
        self.journal
            .remove_file(format!("{}.json", intent.intent_id))
            .map_err(io_error)?;
        sync(&self.journal)?;
        if let Some(stage) = &intent.staging {
            remove_tree(&self.staging, stage)?;
        }
        remove_tree(&self.trash, &intent.backup)?;
        Ok(())
    }
    /// Never sweep staging or trash: another host may own unjournaled work.
    fn recover(&self) -> Result<(), InspectionError> {
        let mut pending = Vec::new();
        for entry in self.journal.entries().map_err(io_error)? {
            let name = entry.map_err(io_error)?.file_name().to_string_lossy().into_owned();
            if name.ends_with(".json") {
                pending.push(name);
            }
        }
        if pending.is_empty() {
            return Ok(());
        }
        let mut index = self.index()?;
        for name in pending {
            let intent: Intent = read_json_bounded(&self.journal, &name, MAX_RECORD)
                .map_err(|e| corrupt(e.message))?;
            validate_intent(&intent, &name)?;
            self.recover_intent(&intent, &mut index)?;
        }
        Ok(())
    }
    fn move_item(
        &self,
        index: &mut Index,
        old: &LibraryIndexEntry,
        new_path: &str,
    ) -> Result<LibraryIndexEntry, InspectionError> {
        let source_dir = self.item_dir(&old.summary.item_path)?;
        let move_inventory = inventory(&source_dir)?;
        let old_prefix = format!("{}/", old.summary.item_path);
        let mut moved_entries = Vec::with_capacity(index.items.len());
        for item in &index.items {
            let mut moved = if item.summary.item_id == old.summary.item_id {
                old.clone()
            } else {
                item.clone()
            };
            if moved.summary.item_path == old.summary.item_path
                || moved.summary.item_path.starts_with(&old_prefix)
            {
                moved.summary.item_path = format!(
                    "{new_path}{}",
                    moved.summary.item_path.strip_prefix(&old.summary.item_path).unwrap()
                );
                if let Some(document) = moved.summary.document_path.as_mut()
                    && (document == &old.summary.item_path || document.starts_with(&old_prefix))
                {
                    *document = format!(
                        "{new_path}{}",
                        document.strip_prefix(&old.summary.item_path).unwrap()
                    );
                }
            }
            moved_entries.push(moved);
        }
        let moved_ids = moved_entries
            .iter()
            .map(|entry| entry.summary.item_id.as_str())
            .collect::<std::collections::HashSet<_>>();
        if moved_entries.iter().any(|entry| {
            index.items.iter().any(|existing| {
                !moved_ids.contains(existing.summary.item_id.as_str())
                    && existing.summary.item_path == entry.summary.item_path
            })
        }) {
            return Err(error("library_conflict", "Moved Library item collides with an existing path"));
        }
        let moved_entry = moved_entries
            .iter()
            .find(|entry| entry.summary.item_id == old.summary.item_id)
            .cloned()
            .ok_or_else(|| corrupt("moved item disappeared from index"))?;
        let mut prospective = index.clone();
        for moved in &moved_entries {
            upsert(&mut prospective, moved.clone());
        }
        prospective.generation = Uuid::new_v4().to_string();
        if serde_json::to_vec_pretty(&prospective)
            .map_err(|e| corrupt(e.to_string()))?
            .len() as u64 > MAX_INDEX
        {
            return Err(error(
                "library_full",
                "Library index capacity reached; remove an item before moving it",
            ));
        }
        let (source_root, source_name) = self.item_parent(&old.summary.item_path)?;
        let (target_root, target_name) = self.create_item_parent(new_path)?;
        if exists(&target_root, target_name)? {
            return Err(error("library_conflict", "Library destination is already occupied"));
        }
        let id = Uuid::new_v4().to_string();
        let intent = Intent {
            schema: INTENT_SCHEMA,
            intent_id: id.clone(),
            op: "move".into(),
            method: Method::Move,
            item_id: old.summary.item_id.clone(),
            target: new_path.to_owned(),
            staging: None,
            backup: id,
            previous_revision: Some(old.summary.revision.clone()),
            new_entry: Some(moved_entry.clone()),
            old_entry: Some(old.clone()),
            predecessor: Some(owned_inventory(&source_dir, old)?),
            source: Some(old.summary.item_path.clone()),
            moved_entries: Some(moved_entries.clone()),
            move_inventory: Some(move_inventory),
        };
        self.write_intent(&intent)?;
        let result = (|| {
            self.fault("move_journal")?;
            rename_special(&source_root, source_name, &target_root, target_name, false)
                .map_err(io_error)?;
            sync(&source_root)?;
            sync(&target_root)?;
            self.fault("move_renamed")?;
            for moved in moved_entries {
                upsert(index, moved);
            }
            self.commit(index)?;
            self.fault("move_index_commit")?;
            self.finish(&intent)
        })();
        if let Err(error) = &result {
            if error.code != "library_test_crash" {
                self.recover()?;
            }
        }
        result?;
        self.prune_empty(intent.source.as_deref().unwrap())?;
        Ok(moved_entry)
    }
    fn publish_owned_entries(
        &self,
        target: &Dir,
        stage: &Dir,
        intent: &Intent,
    ) -> Result<(), InspectionError> {
        let old_roots = intent.old_entry.as_ref().map(owned_roots).unwrap_or_default();
        let new_roots = owned_roots(intent.new_entry.as_ref().ok_or_else(|| corrupt("merge has no new entry"))?);
        let backup = open_child(&self.trash, &intent.backup)?;
        for name in &old_roots {
            if exists(target, name)? && !exists(&backup, name)? {
                rename_special(target, name, &backup, name, false).map_err(io_error)?;
            }
        }
        sync(target)?;
        sync(&backup)?;
        self.fault("entries_backed_up")?;
        for name in &new_roots {
            if exists(stage, name)? {
                if exists(target, name)? {
                    return Err(error(
                        "library_conflict",
                        format!("Library destination entry is already occupied: {name}"),
                    ));
                }
                rename_special(stage, name, target, name, false).map_err(io_error)?;
            }
        }
        sync(target)?;
        sync(stage)?;
        sync(&self.trash)?;
        verify_entry(target, intent.new_entry.as_ref().unwrap())
    }
    fn sync_transition(&self, intent: &Intent) -> Result<(), InspectionError> {
        self.fault("recovery_sync")?;
        sync(&self.item_parent(&intent.target)?.0)?;
        if intent.staging.is_some() {
            sync(&self.staging)?;
        }
        if matches!(
            intent.method,
            Method::TwoRename | Method::Remove | Method::RemoveOwned | Method::Merge
        ) {
            sync(&self.trash)?;
        }
        if let Some(source) = &intent.source {
            sync(&self.item_parent(source)?.0)?;
        }
        Ok(())
    }
    fn fault(&self, point: &'static str) -> Result<(), InspectionError> {
        #[cfg(test)]
        {
            let mut fault = self.fault.lock().unwrap_or_else(|e| e.into_inner());
            if *fault == Some(point) {
                *fault = None;
                return Err(error("library_test_crash", point));
            }
        }
        let _ = point;
        Ok(())
    }
}
fn validate_entry(entry: &LibraryIndexEntry) -> Result<(), InspectionError> {
    if !item_path(&entry.summary.item_path)
        || entry.summary.item_id.is_empty()
        || entry.summary.revision.is_empty()
    {
        return Err(corrupt("invalid Library index entry"));
    }
    if entry.inventory.len() > MAX_TREE_ENTRIES
        || entry
            .inventory
            .windows(2)
            .any(|pair| pair[0].path >= pair[1].path)
    {
        return Err(corrupt("invalid trusted item inventory"));
    }
    if entry.summary.kind == LibraryItemKind::ProviderSnapshot {
        let document = entry.summary.document_path.as_deref()
            .and_then(|path| path.strip_prefix(&format!("{}/", entry.summary.item_path)))
            .filter(|path| component(path) && path.ends_with(".md"))
            .ok_or_else(|| corrupt("provider entry has invalid document path"))?;
        if entry.inventory.iter().any(|file| {
            file.path != document && file.path != crate::library::layout::FILES_DIR
                && !file.path.starts_with(&format!("{}/", crate::library::layout::FILES_DIR))
        }) {
            return Err(corrupt("provider inventory contains an unowned path"));
        }
    }
    for file in &entry.inventory {
        safe_file_path(&file.path)?;
    }
    Ok(())
}
fn validate_intent(i: &Intent, name: &str) -> Result<(), InspectionError> {
    if i.schema != INTENT_SCHEMA
        || Uuid::parse_str(&i.intent_id).is_err()
        || name != format!("{}.json", i.intent_id)
        || i.backup != i.intent_id
        || !item_path(&i.target)
        || i.staging
            .as_ref()
            .is_some_and(|s| Uuid::parse_str(s).is_err())
    {
        return Err(corrupt("invalid journal paths"));
    }
    if i.method == Method::Move {
        let old = i.old_entry.as_ref().ok_or_else(|| corrupt("move has no old entry"))?;
        let new = i.new_entry.as_ref().ok_or_else(|| corrupt("move has no new entry"))?;
        let source = i.source.as_deref().ok_or_else(|| corrupt("move has no source"))?;
        validate_entry(old)?;
        validate_entry(new)?;
        let moved = i.moved_entries.as_ref().ok_or_else(|| corrupt("move has no entries"))?;
        if i.op != "move"
            || i.staging.is_some()
            || i.target == source
            || old.summary.item_id != i.item_id
            || old.summary.item_path != source
            || new.summary.item_id != i.item_id
            || new.summary.item_path != i.target
            || old.summary.revision != new.summary.revision
            || i.previous_revision.as_deref() != Some(old.summary.revision.as_str())
            || i.predecessor.is_none()
            || i.move_inventory.is_none()
            || !moved.iter().any(|entry| entry.summary.item_id == i.item_id && entry.summary.item_path == i.target)
        {
            return Err(corrupt("invalid move intent"));
        }
        for entry in moved {
            validate_entry(entry)?;
        }
        return Ok(());
    }
    if matches!(i.method, Method::Remove | Method::RemoveOwned) {
        if i.op != "remove" || i.new_entry.is_some() || i.staging.is_some() || i.old_entry.is_none() {
            return Err(corrupt("invalid remove intent"));
        }
    } else {
        let e = i
            .new_entry
            .as_ref()
            .ok_or_else(|| corrupt("publish intent has no complete entry"))?;
        validate_entry(e)?;
        if i.op != "publish"
            || i.staging.is_none()
            || e.summary.item_id != i.item_id
            || e.summary.item_path != i.target
        {
            return Err(corrupt("invalid publish intent"));
        }
    }
    if let Some(old) = &i.old_entry {
        validate_entry(old)?;
        if old.summary.item_id != i.item_id
            || old.summary.item_path != i.target
            || Some(&old.summary.revision) != i.previous_revision.as_ref()
            || i.predecessor.is_none()
        {
            return Err(corrupt("invalid journal predecessor"));
        }
    } else if i.previous_revision.is_some()
        || i.predecessor.is_some()
        || i.method != Method::NewTarget
    {
        return Err(corrupt("missing journal predecessor"));
    }
    Ok(())
}
/// Inserts a new entry, or replaces an existing one while keeping the index's
/// current `refs`, `purge_after`, `reference_depth` and `included_by` (D3: only
/// `mutate_index`/`add_ref` change them, so a stale caller copy or a journal
/// replay never rewinds them).
fn upsert(index: &mut Index, mut entry: LibraryIndexEntry) {
    if let Some(old) = index
        .items
        .iter_mut()
        .find(|e| e.summary.item_id == entry.summary.item_id)
    {
        entry.summary.refs = std::mem::take(&mut old.summary.refs);
        entry.summary.purge_after = old.summary.purge_after.take();
        entry.summary.reference_depth = old.summary.reference_depth.take();
        entry.summary.included_by = old.summary.included_by.take();
        *old = entry;
    } else {
        index.items.push(entry);
    }
}
pub(crate) fn bounded_write<T: Serialize>(
    dir: &Dir,
    name: &str,
    value: &T,
    max: u64,
) -> Result<(), InspectionError> {
    let bytes = serde_json::to_vec_pretty(value).map_err(|e| corrupt(e.to_string()))?;
    if bytes.len() as u64 > max {
        return Err(corrupt("Library record exceeds its byte limit"));
    }
    atomic_write_bytes(dir, name, &bytes).map_err(io_error)
}
fn exists(dir: &Dir, name: &str) -> Result<bool, InspectionError> {
    match dir.symlink_metadata(name) {
        Ok(_) => Ok(true),
        Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(false),
        Err(e) => Err(io_error(e)),
    }
}
fn remove_tree(dir: &Dir, name: &str) -> Result<(), InspectionError> {
    if exists(dir, name)? {
        dir.remove_dir_all(name).map_err(io_error)?;
        sync(dir)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests;
