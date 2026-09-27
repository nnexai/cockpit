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
    io::{self, Read},
    path::{Component, Path, PathBuf},
    sync::Arc,
};
use uuid::Uuid;

const MAX_INDEX: u64 = 64 * 1024 * 1024;
const MAX_RECORD: u64 = 64 * 1024 * 1024;
const MAX_FILE: u64 = 1024 * 1024 * 1024;
const MARKER: &str = ".cockpit-item.json";
const MAX_TREE_ENTRIES: usize = 1_000_000;
const MAX_TREE_BYTES: u64 = 4 * 1024 * 1024 * 1024 + MAX_RECORD;
const MAX_TREE_DEPTH: usize = 64;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct LibraryIndexEntry {
    pub summary: LibraryItemSummary,
    /// Validated provider URL used for future refreshes, never the user's input.
    pub canonical_url: Option<String>,
    pub marker_hash: Option<String>,
    /// Trusted complete tree, including directories and marker; never rebuilt from a changed marker.
    pub inventory: Vec<MarkerFile>,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Index {
    schema: u32,
    pub generation: String,
    pub items: Vec<LibraryIndexEntry>,
    pub follows: Vec<LibraryFollowSummary>,
}
impl Default for Index {
    fn default() -> Self {
        Self {
            schema: 1,
            generation: Uuid::new_v4().to_string(),
            items: vec![],
            follows: vec![],
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
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Marker {
    pub schema: u32,
    pub item_id: String,
    pub revision: String,
    pub files: Vec<MarkerFile>,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
enum Method {
    Exchange,
    TwoRename,
    NewTarget,
    Remove,
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
fn open_child(parent: &Dir, name: &str) -> Result<Dir, InspectionError> {
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
    let leaf = value.strip_prefix("folders/").unwrap_or(value);
    component(leaf) && !leaf.starts_with('.')
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
            #[cfg(test)]
            fault: std::sync::Mutex::new(None),
            #[cfg(test)]
            force_two_rename: std::sync::atomic::AtomicBool::new(false),
        });
        let _lock = store.exclusive()?;
        if !exists(&store.meta, "index.json")? {
            store.commit(&mut Index::default())?;
        }
        store.recover()?;
        Ok(store)
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
        if !exists(&self.meta, "index.json")? {
            return Ok(Index::default());
        }
        let index: Index = read_json_bounded(&self.meta, "index.json", MAX_INDEX)
            .map_err(|e| corrupt(e.message))?;
        if index.schema != 1 || index.items.len() > 1_000_000 {
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
        Ok(index)
    }
    fn commit(&self, index: &mut Index) -> Result<(), InspectionError> {
        index.generation = Uuid::new_v4().to_string();
        bounded_write(&self.meta, "index.json", index, MAX_INDEX)
    }
    pub fn update(&self, entry: LibraryIndexEntry) -> Result<(), InspectionError> {
        let _lock = self.exclusive()?;
        let mut index = self.index()?;
        let old = index
            .items
            .iter_mut()
            .find(|e| e.summary.item_id == entry.summary.item_id)
            .ok_or_else(|| error("library_item_not_found", "Library item no longer exists"))?;
        if old.summary.revision != entry.summary.revision {
            return Err(error("library_conflict", "Library revision changed"));
        }
        *old = entry;
        self.commit(&mut index)
    }
    /// Commit presentation and follow-record changes that never touch item files.
    /// The closure must preserve every item's revision, path and inventory.
    pub fn mutate_index<R>(
        &self,
        change: impl FnOnce(&mut Index) -> Result<R, InspectionError>,
    ) -> Result<R, InspectionError> {
        let _lock = self.exclusive()?;
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
        if let Some(leaf) = path.strip_prefix("folders/") {
            Ok((self.root.open_dir_nofollow("folders").map_err(io_error)?, leaf))
        } else {
            Ok((self.root.try_clone().map_err(io_error)?, path))
        }
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
        let stage = self.stage()?;
        let markdown = crate::sources::library_markdown(asset, &entry.summary.revision);
        let keys = format!(
            "---\nlibrary_item_id: {}\nlibrary_revision: {}\n",
            serde_json::to_string(&entry.summary.item_id).unwrap(),
            serde_json::to_string(&entry.summary.revision).unwrap()
        );
        let markdown = keys
            + markdown
                .strip_prefix("---\n")
                .ok_or_else(|| corrupt("source document has no frontmatter"))?;
        atomic_write_bytes(&stage.dir, "document.md", markdown.as_bytes()).map_err(io_error)?;
        self.seal(
            &stage,
            entry,
            vec![MarkerFile {
                path: "document.md".into(),
                hash: hash(markdown.as_bytes()),
                bytes: markdown.len() as u64,
            }],
        )?;
        Ok(stage)
    }
    pub fn seal(
        &self,
        stage: &Stage,
        entry: &mut LibraryIndexEntry,
        files: Vec<MarkerFile>,
    ) -> Result<(), InspectionError> {
        let marker = Marker {
            schema: 1,
            item_id: entry.summary.item_id.clone(),
            revision: entry.summary.revision.clone(),
            files,
        };
        bounded_write(&stage.dir, MARKER, &marker, MAX_RECORD)?;
        entry.marker_hash = Some(file_hash(&stage.dir, MARKER)?.0);
        entry.inventory = inventory(&stage.dir)?;
        // Newly created intermediate directories must be durable as well as the
        // item root: a file's fsync alone does not persist its ancestors.
        for directory in entry.inventory.iter().rev().filter(|file| file.hash == "directory") {
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
        Ok(conflicts(entry, &inventory(&dir)?))
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
        mut stage: Stage,
        entry: LibraryIndexEntry,
        previous: Option<&str>,
        confirmed: Option<&[LibraryConflictFile]>,
    ) -> Result<(), InspectionError> {
        validate_entry(&entry)?;
        verify_entry(&stage.dir, &entry)?;
        let _lock = self.exclusive()?;
        self.recover()?;
        let mut index = self.index()?;
        if entry.summary.item_path.starts_with("folders/") {
            open_child(&self.root, "folders")?;
            sync(&self.root)?;
        }
        let (target_root, target_name) = self.item_parent(&entry.summary.item_path)?;
        let target_name = target_name.to_owned();
        let old = index
            .items
            .iter()
            .find(|e| e.summary.item_id == entry.summary.item_id);
        if old.map(|e| e.summary.revision.as_str()) != previous {
            return Err(error(
                "library_conflict",
                "Library revision changed before publish",
            ));
        }
        let predecessor = if let Some(old) = old {
            if old.summary.item_path != entry.summary.item_path {
                return Err(corrupt("item path changed"));
            }
            let snapshot = inventory(&self.item_dir(&old.summary.item_path)?)?;
            check_confirmation(&conflicts(old, &snapshot), confirmed)?;
            Some(snapshot)
        } else {
            None
        };
        if old.is_none() && index.items.len() >= self.max_items {
            return Err(error(
                "library_full",
                "Library item limit reached; remove an item before adding another",
            ));
        }
        let mut method = if old.is_some() {
            Method::Exchange
        } else {
            Method::NewTarget
        };
        #[cfg(test)]
        if old.is_some()
            && self
                .force_two_rename
                .load(std::sync::atomic::Ordering::SeqCst)
        {
            method = Method::TwoRename;
        }
        let id = Uuid::new_v4().to_string();
        let mut intent = Intent {
            schema: 1,
            intent_id: id.clone(),
            op: "publish".into(),
            method,
            item_id: entry.summary.item_id.clone(),
            target: entry.summary.item_path.clone(),
            staging: Some(stage.name.clone()),
            backup: id,
            previous_revision: previous.map(str::to_owned),
            old_entry: old.cloned(),
            predecessor,
            new_entry: Some(entry),
        };
        self.write_intent(&intent)?;
        // Once journaled, only recovery owns this stage, including after an I/O failure.
        stage.owned = false;
        let result = (|| {
            self.fault("journal")?;
            if method == Method::Exchange {
                match rename_special(&self.staging, &stage.name, &target_root, &target_name, true) {
                    Ok(()) => {}
                    Err(e) if matches!(e.raw_os_error(), Some(22 | 38 | 95)) => {
                        method = Method::TwoRename;
                        intent.method = method;
                        self.write_intent(&intent)?;
                    }
                    Err(e) => return Err(io_error(e)),
                }
            }
            if method == Method::TwoRename {
                rename_special(
                    &target_root,
                    &target_name,
                    &self.trash,
                    &intent.backup,
                    false,
                )
                .map_err(io_error)?;
                sync(&target_root)?;
                sync(&self.trash)?;
                self.fault("old_to_backup")?;
            }
            if matches!(method, Method::TwoRename | Method::NewTarget) {
                rename_special(
                    &self.staging,
                    &stage.name,
                    &target_root,
                    &target_name,
                    false,
                )
                .map_err(io_error)?;
            }
            self.fault("rename_unsynced")?;
            sync(&target_root)?;
            sync(&self.staging)?;
            self.fault("new_to_target")?;
            upsert(&mut index, intent.new_entry.as_ref().unwrap().clone());
            self.commit(&mut index)?;
            self.fault("index_commit")?;
            self.finish(&intent)
        })();
        if let Err(e) = &result {
            if e.code != "library_test_crash" {
                self.recover()?;
            }
        }
        result
    }
    pub fn remove(&self, id: &str, revision: &str) -> Result<(), InspectionError> {
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
        let (target_root, target_name) = self.item_parent(&entry.summary.item_path)?;
        let predecessor = inventory(&self.item_dir(&entry.summary.item_path)?)?;
        check_confirmation(&conflicts(entry, &predecessor), None)?;
        let intent_id = Uuid::new_v4().to_string();
        let intent = Intent {
            schema: 1,
            intent_id: intent_id.clone(),
            op: "remove".into(),
            method: Method::Remove,
            item_id: id.into(),
            target: entry.summary.item_path.clone(),
            staging: None,
            backup: intent_id,
            previous_revision: Some(revision.into()),
            old_entry: Some(entry.clone()),
            predecessor: Some(predecessor),
            new_entry: None,
        };
        self.write_intent(&intent)?;
        rename_special(
            &target_root,
            target_name,
            &self.trash,
            &intent.backup,
            false,
        )
        .map_err(io_error)?;
        self.fault("rename_unsynced")?;
        sync(&target_root)?;
        sync(&self.trash)?;
        self.fault("remove_to_backup")?;
        index.items.retain(|e| e.summary.item_id != id);
        self.commit(&mut index)?;
        self.fault("index_commit")?;
        self.finish(&intent)
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
        let mut index = self.index()?;
        for file in self.journal.entries().map_err(io_error)? {
            let file = file.map_err(io_error)?;
            let name = file.file_name().to_string_lossy().into_owned();
            if !name.ends_with(".json") {
                continue;
            }
            let intent: Intent = read_json_bounded(&self.journal, &name, MAX_RECORD)
                .map_err(|e| corrupt(e.message))?;
            validate_intent(&intent, &name)?;
            let (target_root, target_name) = self.item_parent(&intent.target)?;
            let target = exists(&target_root, target_name)?;
            let backup = exists(&self.trash, &intent.backup)?;
            let target_new = intent.new_entry.as_ref().is_some_and(|entry| {
                target_root
                    .open_dir_nofollow(target_name)
                    .ok()
                    .is_some_and(|dir| verify_entry(&dir, entry).is_ok())
            });
            let is_old = |parent: &Dir, name: &str| {
                intent.predecessor.as_ref().is_some_and(|snapshot| {
                    parent
                        .open_dir_nofollow(name)
                        .ok()
                        .is_some_and(|dir| inventory(&dir).is_ok_and(|actual| actual == *snapshot))
                })
            };
            let target_old = is_old(&target_root, target_name);
            let mut forward = false;
            let mut rollback = false;
            match intent.method {
                Method::Remove if target && !backup => rollback = target_old,
                Method::Remove if !target && backup && is_old(&self.trash, &intent.backup) => {
                    self.sync_transition(&intent)?;
                    index.items.retain(|e| e.summary.item_id != intent.item_id);
                    self.commit(&mut index)?;
                    self.finish(&intent)?;
                    continue;
                }
                Method::NewTarget if !target => rollback = true,
                Method::NewTarget if target_new => forward = true,
                Method::Exchange if target_new => {
                    forward = intent
                        .staging
                        .as_ref()
                        .is_some_and(|stage| is_old(&self.staging, stage))
                }
                Method::TwoRename if target_new => forward = is_old(&self.trash, &intent.backup),
                Method::Exchange | Method::TwoRename if target_old && !backup => rollback = true,
                Method::TwoRename if !target && backup && is_old(&self.trash, &intent.backup) => {
                    let entry = intent.new_entry.as_ref().unwrap();
                    let stage = intent.staging.as_ref().unwrap();
                    let valid = self
                        .staging
                        .open_dir_nofollow(stage)
                        .ok()
                        .is_some_and(|dir| verify_entry(&dir, entry).is_ok());
                    if valid {
                        rename_special(&self.staging, stage, &target_root, target_name, false)
                            .map_err(io_error)?;
                        sync(&self.staging)?;
                        forward = true;
                    } else {
                        rename_special(
                            &self.trash,
                            &intent.backup,
                            &target_root,
                            target_name,
                            false,
                        )
                        .map_err(io_error)?;
                        sync(&self.trash)?;
                        rollback = true;
                    }
                    sync(&target_root)?;
                }
                _ => {}
            }
            if forward || rollback {
                // A prior process may have died immediately after rename, before its fsync.
                // Sync all rename parents even when recovery only observed the transition.
                self.sync_transition(&intent)?;
                if forward {
                    upsert(&mut index, intent.new_entry.as_ref().unwrap().clone());
                    self.commit(&mut index)?;
                } else if let Some(old) = &intent.old_entry {
                    upsert(&mut index, old.clone());
                    self.commit(&mut index)?;
                }
                self.finish(&intent)?;
            } else {
                if let Some(entry) = index
                    .items
                    .iter_mut()
                    .find(|e| e.summary.item_id == intent.item_id)
                {
                    entry.summary.state = LibraryItemState::Failed;
                    entry.summary.diagnostics = vec![ProjectDiagnostic {
                        code: "library_corrupt".into(),
                        message:
                            "Journal does not match a recoverable filesystem state; files retained"
                                .into(),
                        path: Some(intent.target.clone()),
                    }];
                    self.commit(&mut index)?;
                } else {
                    return Err(corrupt(
                        "unindexed journal target is not recoverable; files retained",
                    ));
                }
            }
        }
        Ok(())
    }
    fn sync_transition(&self, intent: &Intent) -> Result<(), InspectionError> {
        self.fault("recovery_sync")?;
        sync(&self.item_parent(&intent.target)?.0)?;
        if intent.staging.is_some() {
            sync(&self.staging)?;
        }
        if matches!(intent.method, Method::TwoRename | Method::Remove) {
            sync(&self.trash)?;
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
    Ok(())
}
fn validate_intent(i: &Intent, name: &str) -> Result<(), InspectionError> {
    if i.schema != 1
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
    if i.method == Method::Remove {
        if i.op != "remove" || i.new_entry.is_some() || i.staging.is_some() {
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
fn upsert(index: &mut Index, entry: LibraryIndexEntry) {
    if let Some(old) = index
        .items
        .iter_mut()
        .find(|e| e.summary.item_id == entry.summary.item_id)
    {
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
fn read_marker(dir: &Dir) -> Result<Marker, InspectionError> {
    let marker: Marker =
        read_json_bounded(dir, MARKER, MAX_RECORD).map_err(|e| corrupt(e.message))?;
    if marker.schema != 1 || marker.files.len() > 100_000 {
        return Err(corrupt("unsupported marker"));
    }
    let mut paths = std::collections::HashSet::new();
    for file in &marker.files {
        if !paths.insert(&file.path) || file.path == MARKER || file.bytes > MAX_FILE {
            return Err(corrupt("invalid marker file"));
        }
        safe_file_path(&file.path)?;
    }
    Ok(marker)
}
fn safe_file_path(path: &str) -> Result<(), InspectionError> {
    if path.is_empty()
        || path.len() > 4096
        || path.contains('\\')
        || Path::new(path)
            .components()
            .any(|c| !matches!(c, Component::Normal(_)))
    {
        return Err(corrupt("invalid item file path"));
    }
    Ok(())
}
fn file_hash(dir: &Dir, path: &str) -> Result<(String, u64), InspectionError> {
    safe_file_path(path)?;
    let path = Path::new(path);
    let mut parent = dir.try_clone().map_err(io_error)?;
    if let Some(p) = path.parent() {
        for c in p.components() {
            parent = parent.open_dir_nofollow(c.as_os_str()).map_err(io_error)?;
        }
    }
    let mut options = OpenOptions::new();
    options
        .read(true)
        .follow(cap_fs_ext::FollowSymlinks::No)
        .nonblock(true);
    let mut file = match parent.open_with(path.file_name().unwrap(), &options) {
        Ok(f) => f,
        Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(("missing".into(), 0)),
        Err(e) => return Err(io_error(e)),
    };
    let metadata = file.metadata().map_err(io_error)?;
    if !metadata.is_file() || metadata.len() > MAX_FILE {
        return Err(corrupt("item file is not a bounded regular file"));
    }
    let mut hash = Sha256::new();
    let mut bytes = 0;
    let mut buffer = [0u8; 32768];
    loop {
        let n = file.read(&mut buffer).map_err(io_error)?;
        if n == 0 {
            break;
        }
        bytes += n as u64;
        if bytes > MAX_FILE {
            return Err(corrupt("item file grew beyond limit"));
        }
        hash.update(&buffer[..n]);
    }
    Ok((format!("sha256:{:x}", hash.finalize()), bytes))
}
/// Inventory is bounded in depth, entries, per-file bytes and total bytes. Directory
/// handles and regular files are opened no-follow; unsupported nodes fail closed.
fn inventory(dir: &Dir) -> Result<Vec<MarkerFile>, InspectionError> {
    fn walk(
        dir: &Dir,
        prefix: &str,
        depth: usize,
        files: &mut Vec<MarkerFile>,
        bytes: &mut u64,
    ) -> Result<(), InspectionError> {
        if depth > MAX_TREE_DEPTH {
            return Err(corrupt("item tree exceeds depth limit"));
        }
        for entry in dir.entries().map_err(io_error)? {
            let entry = entry.map_err(io_error)?;
            let name = entry
                .file_name()
                .into_string()
                .map_err(|_| corrupt("item path is not UTF-8"))?;
            let path = if prefix.is_empty() {
                name.clone()
            } else {
                format!("{prefix}/{name}")
            };
            safe_file_path(&path)?;
            if files.len() >= MAX_TREE_ENTRIES {
                return Err(corrupt("item tree exceeds entry limit"));
            }
            let metadata = dir.symlink_metadata(&name).map_err(io_error)?;
            if metadata.is_dir() {
                files.push(MarkerFile {
                    path: path.clone(),
                    hash: "directory".into(),
                    bytes: 0,
                });
                walk(
                    &dir.open_dir_nofollow(&name).map_err(io_error)?,
                    &path,
                    depth + 1,
                    files,
                    bytes,
                )?;
            } else if metadata.is_file() {
                if metadata.len() > MAX_TREE_BYTES.saturating_sub(*bytes) {
                    return Err(corrupt("item tree exceeds byte limit"));
                }
                let (hash, size) = file_hash(dir, &name)?;
                if hash == "missing" {
                    return Err(error("library_conflict", "Item changed during inventory"));
                }
                *bytes += size;
                if *bytes > MAX_TREE_BYTES {
                    return Err(corrupt("item tree exceeds byte limit"));
                }
                files.push(MarkerFile {
                    path,
                    hash,
                    bytes: size,
                });
            } else {
                return Err(error(
                    "library_conflict",
                    format!("Item contains a symlink or special file: {path}"),
                ));
            }
        }
        Ok(())
    }
    let mut files = Vec::new();
    walk(dir, "", 0, &mut files, &mut 0)?;
    files.sort_by(|a, b| a.path.cmp(&b.path));
    Ok(files)
}
fn conflicts(entry: &LibraryIndexEntry, actual: &[MarkerFile]) -> Vec<LibraryConflictFile> {
    let mut expected = entry.inventory.iter().peekable();
    let mut current = actual.iter().peekable();
    let mut changed = Vec::new();
    while expected.peek().is_some() || current.peek().is_some() {
        let order = match (expected.peek(), current.peek()) {
            (Some(old), Some(now)) => old.path.cmp(&now.path),
            (Some(_), None) => std::cmp::Ordering::Less,
            _ => std::cmp::Ordering::Greater,
        };
        match order {
            std::cmp::Ordering::Less => changed.push(LibraryConflictFile {
                path: expected.next().unwrap().path.clone(),
                current_hash: "missing".into(),
            }),
            std::cmp::Ordering::Greater => {
                let now = current.next().unwrap();
                changed.push(LibraryConflictFile {
                    path: now.path.clone(),
                    current_hash: now.hash.clone(),
                });
            }
            std::cmp::Ordering::Equal => {
                let old = expected.next().unwrap();
                let now = current.next().unwrap();
                if old != now {
                    changed.push(LibraryConflictFile {
                        path: now.path.clone(),
                        current_hash: now.hash.clone(),
                    });
                }
            }
        }
    }
    changed
}
fn check_confirmation(
    actual: &[LibraryConflictFile],
    confirmed: Option<&[LibraryConflictFile]>,
) -> Result<(), InspectionError> {
    let mut confirmed: Vec<_> = confirmed
        .unwrap_or_default()
        .iter()
        .map(|f| (&f.path, &f.current_hash))
        .collect();
    confirmed.sort();
    if !confirmed
        .into_iter()
        .eq(actual.iter().map(|f| (&f.path, &f.current_hash)))
    {
        return Err(error(
            "library_conflict",
            "Library files changed; confirm their current hashes before replacing",
        ));
    }
    Ok(())
}
fn verify(dir: &Dir, id: &str, revision: &str) -> Result<(), InspectionError> {
    let marker = read_marker(dir)?;
    if marker.item_id != id || marker.revision != revision {
        return Err(corrupt("marker identity mismatch"));
    }
    for file in marker.files {
        let (hash, bytes) = file_hash(dir, &file.path)?;
        if hash != file.hash || bytes != file.bytes {
            return Err(corrupt("marker file hash mismatch"));
        }
    }
    Ok(())
}

fn verify_entry(dir: &Dir, entry: &LibraryIndexEntry) -> Result<(), InspectionError> {
    if inventory(dir)? != entry.inventory {
        return Err(corrupt(
            "published inventory differs from the journaled entry",
        ));
    }
    if entry.marker_hash.as_deref() != Some(&file_hash(dir, MARKER)?.0) {
        return Err(corrupt("published marker differs from the journaled entry"));
    }
    verify(dir, &entry.summary.item_id, &entry.summary.revision)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::library::{
        asset_entry,
        tests::{assert_store_valid, asset, fixture, reopen},
    };
    use std::sync::{atomic::Ordering, mpsc};

    fn folder(
        store: &Arc<Store>,
        origin: &Path,
        previous: Option<&LibraryIndexEntry>,
        bytes: &[u8],
    ) -> (Stage, LibraryIndexEntry) {
        let mut entry = previous
            .cloned()
            .unwrap_or_else(|| asset_entry(&asset(1, "folder"), None));
        if previous.is_none() {
            entry.summary.item_id = format!("folder:{}", Uuid::new_v4());
            entry.summary.kind = LibraryItemKind::FolderCopy;
            entry.summary.logical_id = entry.summary.item_id.clone();
            entry.summary.item_path = "copied-folder".into();
            entry.summary.document_path = None;
            entry.canonical_url = None;
        }
        entry.summary.revision = hash(bytes);
        entry.summary.folder = Some(LibraryFolderInfo {
            origin_path: origin.to_string_lossy().into_owned(),
            git_working_tree: false,
            files: 2,
            bytes: (bytes.len() * 2) as u64,
            skipped_symlinks: 3,
            skipped_special: 1,
            skipped_ignored: 2,
            skipped_other: 4,
        });
        let stage = store.stage().unwrap();
        // Two files make reader consistency and complete marker verification observable.
        atomic_write_bytes(&stage.dir, "first.txt", bytes).unwrap();
        atomic_write_bytes(&stage.dir, "second.txt", bytes).unwrap();
        store
            .seal(
                &stage,
                &mut entry,
                vec![
                    MarkerFile {
                        path: "first.txt".into(),
                        hash: hash(bytes),
                        bytes: bytes.len() as u64,
                    },
                    MarkerFile {
                        path: "second.txt".into(),
                        hash: hash(bytes),
                        bytes: bytes.len() as u64,
                    },
                ],
            )
            .unwrap();
        (stage, entry)
    }
    fn fault(store: &Store, point: &'static str) {
        *store.fault.lock().unwrap_or_else(|e| e.into_inner()) = Some(point);
    }
    fn assert_entry(store: &Store, expected: &LibraryIndexEntry, bytes: &[u8]) {
        assert_store_valid(store);
        let _lock = store.shared().unwrap();
        let index = store.index().unwrap();
        assert_eq!(
            serde_json::to_value(&index.items[0]).unwrap(),
            serde_json::to_value(expected).unwrap()
        );
        let dir = store
            .root
            .open_dir_nofollow(&expected.summary.item_path)
            .unwrap();
        assert_eq!(dir.read("first.txt").unwrap(), bytes);
        assert_eq!(dir.read("second.txt").unwrap(), bytes);
        assert_eq!(store.journal.entries().unwrap().count(), 0);
    }
    fn refresh_folder_from_recovered_origin(store: &Arc<Store>, expected: &LibraryIndexEntry) {
        // S4's filesystem refresh consumer can reconstruct its input from the
        // complete recovered entry; no transient original request is required.
        let entry = store.index().unwrap().items.remove(0);
        assert_eq!(
            entry.summary.folder.as_ref().unwrap().origin_path,
            expected.summary.folder.as_ref().unwrap().origin_path
        );
        let origin = Path::new(&entry.summary.folder.as_ref().unwrap().origin_path);
        std::fs::write(origin, b"later folder revision").unwrap();
        let bytes = std::fs::read(origin).unwrap();
        let (stage, refreshed) = folder(store, origin, Some(&entry), &bytes);
        store
            .publish(
                stage,
                refreshed.clone(),
                Some(&entry.summary.revision),
                None,
            )
            .unwrap();
        assert_entry(store, &refreshed, &bytes);
    }
    #[tokio::test]
    async fn first_folder_publish_crash_recovers_complete_origin_and_can_refresh() {
        let f = fixture();
        let store = f.service.open().unwrap();
        let origin = f.root.join("origin.txt");
        std::fs::write(&origin, b"first").unwrap();
        let (stage, entry) = folder(&store, &origin, None, b"first");
        let stage_name = stage.name.clone();
        fault(&store, "new_to_target");
        assert_eq!(
            store
                .publish(stage, entry.clone(), None, None)
                .unwrap_err()
                .code,
            "library_test_crash"
        );
        let reopened = reopen(&f);
        let listing = reopened.listing(None).await.unwrap();
        assert_eq!(
            listing.items[0].folder.as_ref().unwrap().origin_path,
            origin.to_string_lossy()
        );
        let recovered = reopened.open().unwrap();
        assert_entry(&recovered, &entry, b"first");
        assert!(!exists(&recovered.staging, &stage_name).unwrap());
        refresh_folder_from_recovered_origin(&recovered, &entry);
    }
    #[test]
    fn two_rename_r1_crash_with_invalid_stage_rolls_back_old_content_and_index() {
        let f = fixture();
        let store = f.service.open().unwrap();
        let origin = f.root.join("origin.txt");
        let (stage, old) = folder(&store, &origin, None, b"old");
        store.publish(stage, old.clone(), None, None).unwrap();
        let (stage, new) = folder(&store, &origin, Some(&old), b"new");
        let stage_name = stage.name.clone();
        store.force_two_rename.store(true, Ordering::SeqCst);
        fault(&store, "old_to_backup");
        assert_eq!(
            store
                .publish(stage, new, Some(&old.summary.revision), None)
                .unwrap_err()
                .code,
            "library_test_crash"
        );
        assert!(!exists(&store.root, &old.summary.item_path).unwrap());
        // D2 rolls back at R1 only when the staged replacement fails verification.
        // This deliberately supplies the rollback precondition for acceptance (2).
        let staged = store.staging.open_dir_nofollow(&stage_name).unwrap();
        atomic_write_bytes(&staged, "first.txt", b"torn stage").unwrap();
        let recovered = reopen(&f).open().unwrap();
        assert_entry(&recovered, &old, b"old");
        assert!(!exists(&recovered.staging, &stage_name).unwrap());
        assert_eq!(recovered.trash.entries().unwrap().count(), 0);
    }
    #[test]
    fn two_rename_r1_crash_with_valid_stage_rolls_forward_per_d2() {
        let f = fixture();
        let store = f.service.open().unwrap();
        let origin = f.root.join("origin.txt");
        let (stage, old) = folder(&store, &origin, None, b"old");
        store.publish(stage, old.clone(), None, None).unwrap();
        let (stage, new) = folder(&store, &origin, Some(&old), b"new");
        store.force_two_rename.store(true, Ordering::SeqCst);
        fault(&store, "old_to_backup");
        assert_eq!(
            store
                .publish(stage, new.clone(), Some(&old.summary.revision), None)
                .unwrap_err()
                .code,
            "library_test_crash"
        );
        let recovered = reopen(&f).open().unwrap();
        assert_entry(&recovered, &new, b"new");
        assert_eq!(recovered.staging.entries().unwrap().count(), 0);
        assert_eq!(recovered.trash.entries().unwrap().count(), 0);
    }
    #[test]
    fn replacement_crash_after_target_rename_recovers_complete_entry_and_origin() {
        let f = fixture();
        let store = f.service.open().unwrap();
        let origin = f.root.join("origin.txt");
        let (stage, old) = folder(&store, &origin, None, b"old");
        store.publish(stage, old.clone(), None, None).unwrap();
        let moved_origin = f.root.join("moved-origin.txt");
        let (stage, new) = folder(&store, &moved_origin, Some(&old), b"new");
        fault(&store, "new_to_target");
        assert_eq!(
            store
                .publish(stage, new.clone(), Some(&old.summary.revision), None)
                .unwrap_err()
                .code,
            "library_test_crash"
        );
        let recovered = reopen(&f).open().unwrap();
        assert_entry(&recovered, &new, b"new");
        refresh_folder_from_recovered_origin(&recovered, &new);
    }
    #[test]
    fn crash_after_index_commit_is_idempotent_on_repeated_reopen() {
        let f = fixture();
        let store = f.service.open().unwrap();
        let origin = f.root.join("origin.txt");
        let (stage, entry) = folder(&store, &origin, None, b"committed");
        fault(&store, "index_commit");
        assert_eq!(
            store
                .publish(stage, entry.clone(), None, None)
                .unwrap_err()
                .code,
            "library_test_crash"
        );
        let recovered = reopen(&f).open().unwrap();
        assert_entry(&recovered, &entry, b"committed");
        let generation = recovered.index().unwrap().generation;
        let again = reopen(&f).open().unwrap();
        assert_entry(&again, &entry, b"committed");
        assert_eq!(again.index().unwrap().generation, generation);
    }
    #[test]
    fn host_b_open_retains_host_a_active_download_and_crash_orphan() {
        let f = fixture();
        let a = f.service.open().unwrap();
        let origin = f.root.join("origin.txt");
        let (stage, entry) = folder(&a, &origin, None, b"active download");
        let stage_name = stage.name.clone();
        let mut orphan = a.stage().unwrap();
        atomic_write_bytes(&orphan.dir, "orphan.bin", b"crash-left bytes").unwrap();
        let orphan_name = orphan.name.clone();
        orphan.owned = false;
        drop(orphan);
        let b = reopen(&f).open().unwrap();
        assert_eq!(
            b.staging
                .open_dir_nofollow(&stage_name)
                .unwrap()
                .read("first.txt")
                .unwrap(),
            b"active download"
        );
        assert_eq!(
            b.staging
                .open_dir_nofollow(&orphan_name)
                .unwrap()
                .read("orphan.bin")
                .unwrap(),
            b"crash-left bytes"
        );
        a.publish(stage, entry.clone(), None, None).unwrap();
        assert_entry(&b, &entry, b"active download");
        assert!(!exists(&a.staging, &stage_name).unwrap());
        let failed = a.stage().unwrap();
        let failed_name = failed.name.clone();
        drop(failed);
        assert!(!exists(&a.staging, &failed_name).unwrap());
        let reopened = reopen(&f).open().unwrap();
        assert_eq!(
            reopened
                .staging
                .open_dir_nofollow(&orphan_name)
                .unwrap()
                .read("orphan.bin")
                .unwrap(),
            b"crash-left bytes"
        );
    }
    #[test]
    fn shared_reader_blocks_two_rename_and_never_observes_missing_or_mixed_target() {
        let f = fixture();
        let store = f.service.open().unwrap();
        let origin = f.root.join("origin.txt");
        let (stage, old) = folder(&store, &origin, None, b"old");
        store.publish(stage, old.clone(), None, None).unwrap();
        let (stage, new) = folder(&store, &origin, Some(&old), b"new");
        store.force_two_rename.store(true, Ordering::SeqCst);
        let (held_tx, held_rx) = mpsc::channel();
        let (release_tx, release_rx) = mpsc::channel();
        let reader_store = store.clone();
        let old_for_reader = old.clone();
        let reader = std::thread::spawn(move || {
            let lock = reader_store.shared().unwrap();
            held_tx.send(()).unwrap();
            release_rx.recv().unwrap();
            let dir = reader_store
                .root
                .open_dir_nofollow(&old_for_reader.summary.item_path)
                .unwrap();
            assert_eq!(
                read_marker(&dir).unwrap().revision,
                old_for_reader.summary.revision
            );
            assert_eq!(dir.read("first.txt").unwrap(), b"old");
            assert_eq!(dir.read("second.txt").unwrap(), b"old");
            drop(lock);
        });
        held_rx.recv().unwrap();
        let (done_tx, done_rx) = mpsc::channel();
        let writer_store = store.clone();
        let new_for_writer = new.clone();
        let writer = std::thread::spawn(move || {
            writer_store
                .publish(stage, new_for_writer, Some(&old.summary.revision), None)
                .unwrap();
            done_tx.send(()).unwrap();
        });
        assert!(
            done_rx
                .recv_timeout(std::time::Duration::from_millis(30))
                .is_err(),
            "writer must wait for shared reader"
        );
        release_tx.send(()).unwrap();
        reader.join().unwrap();
        writer.join().unwrap();
        done_rx.recv().unwrap();
        assert_entry(&store, &new, b"new");
    }
    #[test]
    fn publish_rechecks_edits_and_cleans_rejected_operation_stage() {
        let f = fixture();
        let store = f.service.open().unwrap();
        let origin = f.root.join("origin.txt");
        let (stage, old) = folder(&store, &origin, None, b"old");
        store.publish(stage, old.clone(), None, None).unwrap();
        let (stage, new) = folder(&store, &origin, Some(&old), b"new");
        let name = stage.name.clone();
        let target = store
            .root
            .open_dir_nofollow(&old.summary.item_path)
            .unwrap();
        atomic_write_bytes(&target, "second.txt", b"user edited during fetch").unwrap();
        assert_eq!(
            store
                .publish(stage, new, Some(&old.summary.revision), None)
                .unwrap_err()
                .code,
            "library_conflict"
        );
        assert_eq!(
            target.read("second.txt").unwrap(),
            b"user edited during fetch"
        );
        assert!(!exists(&store.staging, &name).unwrap());
        assert_eq!(
            store.index().unwrap().items[0].summary.revision,
            old.summary.revision
        );
    }
    #[test]
    fn item_limit_refuses_add_without_eviction_and_remove_crash_finishes() {
        let f = fixture();
        let store = Store::open(Path::new(&f.service.configuration.library_root), 1).unwrap();
        let mut first = asset_entry(&asset(1, "one"), None);
        let stage = store.stage_asset(&mut first, &asset(1, "one")).unwrap();
        store.publish(stage, first.clone(), None, None).unwrap();
        let mut second = asset_entry(&asset(2, "two"), None);
        let stage = store.stage_asset(&mut second, &asset(2, "two")).unwrap();
        let stage_name = stage.name.clone();
        assert_eq!(
            store.publish(stage, second, None, None).unwrap_err().code,
            "library_full"
        );
        assert_eq!(
            store.index().unwrap().items[0].summary.item_id,
            first.summary.item_id
        );
        assert!(!exists(&store.staging, &stage_name).unwrap());
        assert_store_valid(&store);
        fault(&store, "remove_to_backup");
        assert_eq!(
            store
                .remove(&first.summary.item_id, &first.summary.revision)
                .unwrap_err()
                .code,
            "library_test_crash"
        );
        let recovered = reopen(&f).open().unwrap();
        assert!(recovered.index().unwrap().items.is_empty());
        assert!(!exists(&recovered.root, &first.summary.item_path).unwrap());
        assert_eq!(recovered.trash.entries().unwrap().count(), 0);
        assert_eq!(recovered.journal.entries().unwrap().count(), 0);
    }
    #[test]
    fn added_files_and_directories_refuse_replacement_and_removal_without_confirmation() {
        let f = fixture();
        let store = f.service.open().unwrap();
        let mut old = asset_entry(&asset(1, "old"), None);
        let stage = store.stage_asset(&mut old, &asset(1, "old")).unwrap();
        store.publish(stage, old.clone(), None, None).unwrap();
        let target = store
            .root
            .open_dir_nofollow(&old.summary.item_path)
            .unwrap();
        atomic_write_bytes(&target, "notes.md", b"user notes").unwrap();
        target.create_dir("empty").unwrap();
        target.create_dir("nested").unwrap();
        atomic_write_bytes(
            &target.open_dir_nofollow("nested").unwrap(),
            "draft.md",
            b"user draft",
        )
        .unwrap();
        let confirmed = store.conflicts(&old).unwrap();
        assert_eq!(
            confirmed
                .iter()
                .map(|f| f.path.as_str())
                .collect::<Vec<_>>(),
            vec!["empty", "nested", "nested/draft.md", "notes.md"]
        );
        let mut new = asset_entry(&asset(1, "new"), Some(&old));
        let stage = store.stage_asset(&mut new, &asset(1, "new")).unwrap();
        assert_eq!(
            store
                .publish(stage, new, Some(&old.summary.revision), None)
                .unwrap_err()
                .code,
            "library_conflict"
        );
        assert_eq!(
            store
                .remove(&old.summary.item_id, &old.summary.revision)
                .unwrap_err()
                .code,
            "library_conflict"
        );
        assert_eq!(target.read("notes.md").unwrap(), b"user notes");
        assert_eq!(
            target
                .open_dir_nofollow("nested")
                .unwrap()
                .read("draft.md")
                .unwrap(),
            b"user draft"
        );
        assert!(target.open_dir_nofollow("empty").is_ok());
        atomic_write_bytes(&target, "notes.md", b"later notes").unwrap();
        let mut new = asset_entry(&asset(1, "new"), Some(&old));
        let stage = store.stage_asset(&mut new, &asset(1, "new")).unwrap();
        assert_eq!(
            store
                .publish(stage, new, Some(&old.summary.revision), Some(&confirmed))
                .unwrap_err()
                .code,
            "library_conflict"
        );
        assert_eq!(target.read("notes.md").unwrap(), b"later notes");
    }

    #[test]
    fn tampered_or_missing_marker_never_bypasses_document_publish_cas() {
        for missing in [false, true] {
            let f = fixture();
            let store = f.service.open().unwrap();
            let mut old = asset_entry(&asset(1, "old"), None);
            let stage = store.stage_asset(&mut old, &asset(1, "old")).unwrap();
            store.publish(stage, old.clone(), None, None).unwrap();
            let target = store
                .root
                .open_dir_nofollow(&old.summary.item_path)
                .unwrap();
            if missing {
                target.remove_file(MARKER).unwrap();
            } else {
                atomic_write_bytes(
                    &target,
                    MARKER,
                    br#"{"schema":1,"item_id":"forged","revision":"forged","files":[]}"#,
                )
                .unwrap();
            }
            atomic_write_bytes(&target, "document.md", b"confirmed edit").unwrap();
            atomic_write_bytes(&target, "notes.md", b"added notes").unwrap();
            let confirmed = store.conflicts(&old).unwrap();
            assert_eq!(
                confirmed
                    .iter()
                    .map(|f| f.path.as_str())
                    .collect::<Vec<_>>(),
                vec![MARKER, "document.md", "notes.md"]
            );
            store.check_confirmation(&old, Some(&confirmed)).unwrap();
            let mut new = asset_entry(&asset(1, "new"), Some(&old));
            let stage = store.stage_asset(&mut new, &asset(1, "new")).unwrap();
            atomic_write_bytes(&target, "document.md", b"edited after confirmation").unwrap();
            assert_eq!(
                store
                    .publish(stage, new, Some(&old.summary.revision), Some(&confirmed))
                    .unwrap_err()
                    .code,
                "library_conflict"
            );
            assert_eq!(
                store
                    .remove(&old.summary.item_id, &old.summary.revision)
                    .unwrap_err()
                    .code,
                "library_conflict"
            );
            assert_eq!(
                target.read("document.md").unwrap(),
                b"edited after confirmation"
            );
            assert_eq!(target.read("notes.md").unwrap(), b"added notes");
        }
    }

    #[test]
    fn confirmed_edited_predecessor_recovers_at_journal_and_two_rename_r1() {
        for (point, valid_stage, forward) in [
            ("journal", true, false),
            ("old_to_backup", false, false),
            ("old_to_backup", true, true),
        ] {
            let f = fixture();
            let store = f.service.open().unwrap();
            let origin = f.root.join("origin.txt");
            let (stage, old) = folder(&store, &origin, None, b"old");
            store.publish(stage, old.clone(), None, None).unwrap();
            let target = store
                .root
                .open_dir_nofollow(&old.summary.item_path)
                .unwrap();
            atomic_write_bytes(&target, "first.txt", b"confirmed user edit").unwrap();
            atomic_write_bytes(&target, "notes.md", b"confirmed addition").unwrap();
            target.remove_file(MARKER).unwrap();
            let confirmed = store.conflicts(&old).unwrap();
            let snapshot = inventory(&target).unwrap();
            let (stage, new) = folder(&store, &origin, Some(&old), b"new");
            let stage_name = stage.name.clone();
            store
                .force_two_rename
                .store(point == "old_to_backup", Ordering::SeqCst);
            fault(&store, point);
            assert_eq!(
                store
                    .publish(
                        stage,
                        new.clone(),
                        Some(&old.summary.revision),
                        Some(&confirmed)
                    )
                    .unwrap_err()
                    .code,
                "library_test_crash"
            );
            if !valid_stage {
                atomic_write_bytes(
                    &store.staging.open_dir_nofollow(&stage_name).unwrap(),
                    "first.txt",
                    b"invalid stage",
                )
                .unwrap();
            }
            let recovered = reopen(&f).open().unwrap();
            if forward {
                assert_entry(&recovered, &new, b"new");
            } else {
                assert_eq!(
                    serde_json::to_value(&recovered.index().unwrap().items[0]).unwrap(),
                    serde_json::to_value(&old).unwrap()
                );
                assert_eq!(
                    inventory(
                        &recovered
                            .root
                            .open_dir_nofollow(&old.summary.item_path)
                            .unwrap()
                    )
                    .unwrap(),
                    snapshot
                );
            }
            assert_eq!(recovered.journal.entries().unwrap().count(), 0);
            assert_eq!(recovered.trash.entries().unwrap().count(), 0);
            assert!(!exists(&recovered.staging, &stage_name).unwrap());
        }
    }

    #[test]
    fn recovery_sync_failure_keeps_old_index_and_journal_until_durable_retry() {
        for method in [
            Method::NewTarget,
            Method::Exchange,
            Method::TwoRename,
            Method::Remove,
        ] {
            let f = fixture();
            let store = f.service.open().unwrap();
            let origin = f.root.join("origin.txt");
            let (stage, old) = folder(&store, &origin, None, b"old");
            if method != Method::NewTarget {
                store.publish(stage, old.clone(), None, None).unwrap();
            } else {
                drop(stage);
            }
            let before = serde_json::to_value(store.index().unwrap()).unwrap();
            let new = if method == Method::Remove {
                fault(&store, "rename_unsynced");
                assert_eq!(
                    store
                        .remove(&old.summary.item_id, &old.summary.revision)
                        .unwrap_err()
                        .code,
                    "library_test_crash"
                );
                None
            } else {
                let previous = (method != Method::NewTarget).then_some(&old);
                let (stage, new) = folder(&store, &origin, previous, b"new");
                store
                    .force_two_rename
                    .store(method == Method::TwoRename, Ordering::SeqCst);
                fault(&store, "rename_unsynced");
                assert_eq!(
                    store
                        .publish(
                            stage,
                            new.clone(),
                            previous.map(|e| e.summary.revision.as_str()),
                            None
                        )
                        .unwrap_err()
                        .code,
                    "library_test_crash"
                );
                Some(new)
            };
            // The rename happened, but its durable boundary failed. Recovery must
            // retain both the old index and replay intent if that boundary fails again.
            fault(&store, "recovery_sync");
            assert_eq!(
                store.recover_pending().unwrap_err().code,
                "library_test_crash"
            );
            assert_eq!(
                serde_json::to_value(store.index().unwrap()).unwrap(),
                before
            );
            assert_eq!(store.journal.entries().unwrap().count(), 1);
            let recovered = reopen(&f).open().unwrap();
            if let Some(new) = new {
                assert_entry(&recovered, &new, b"new");
            } else {
                assert!(recovered.index().unwrap().items.is_empty());
                assert!(!exists(&recovered.root, &old.summary.item_path).unwrap());
            }
            assert_eq!(recovered.journal.entries().unwrap().count(), 0);
        }
    }
    #[test]
    fn missing_tracked_file_and_added_symlink_cannot_be_removed_silently() {
        let f = fixture();
        let store = f.service.open().unwrap();
        let origin = f.root.join("origin.txt");
        let (stage, old) = folder(&store, &origin, None, b"old");
        store.publish(stage, old.clone(), None, None).unwrap();
        let target = store
            .root
            .open_dir_nofollow(&old.summary.item_path)
            .unwrap();
        target.remove_file("first.txt").unwrap();
        let conflicts = store.conflicts(&old).unwrap();
        assert_eq!(conflicts.len(), 1);
        assert_eq!(conflicts[0].path, "first.txt");
        assert_eq!(conflicts[0].current_hash, "missing");
        assert_eq!(
            store
                .remove(&old.summary.item_id, &old.summary.revision)
                .unwrap_err()
                .code,
            "library_conflict"
        );
        #[cfg(unix)]
        {
            std::fs::write(&origin, b"outside content").unwrap();
            std::os::unix::fs::symlink(
                &origin,
                store.path.join(&old.summary.item_path).join("outside"),
            )
            .unwrap();
            assert_eq!(store.conflicts(&old).unwrap_err().code, "library_conflict");
            assert_eq!(
                store
                    .remove(&old.summary.item_id, &old.summary.revision)
                    .unwrap_err()
                    .code,
                "library_conflict"
            );
            assert_eq!(std::fs::read(&origin).unwrap(), b"outside content");
        }
        assert_eq!(target.read("second.txt").unwrap(), b"old");
    }
}
