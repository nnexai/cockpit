use std::fs::File;
use std::io::{self, Read, Write};
use std::path::{Component, Path, PathBuf};
use std::sync::Arc;
use std::thread;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use cap_fs_ext::{DirExt, OpenOptionsFollowExt, OpenOptionsSyncExt};
use cap_std::fs::{Dir, OpenOptions};
use cockpit_protocol::projects::{ProjectArtifact, WorkspaceOperation, WorkspaceSetupPlan};
use fs2::FileExt;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

#[cfg(all(target_os = "linux", target_env = "gnu"))]
use nix::fcntl::{RenameFlags, renameat2};

use crate::InspectionError;

const LOCK_WAIT: Duration = Duration::from_secs(5);
const MAX_RECORD_BYTES: u64 = 2 * 1024 * 1024;
const MAX_COMPANION_ENTRIES: usize = 1024;
const MAX_OPERATION_ENTRIES: usize = 4096;

/// The durable association written next to a Cockpit-owned companion.
///
/// This is deliberately evidence of an association, rather than a registry of
/// Herdr workspaces. Every use must validate it against a fresh Herdr snapshot.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct CompanionManifest {
    pub schema_version: u32,
    pub cockpit_operation_id: String,
    pub herdr_session_identity: String,
    pub herdr_workspace_id: String,
    pub repository_key: String,
    pub repository_root: String,
    pub checkout_path: String,
    pub artifact: Option<ProjectArtifact>,
    pub created_at: String,
    pub updated_at: String,
    pub ownership: String,
}

/// Durable evidence for a teardown removal that may need explicit recovery.
/// The receipt is separate from setup progress so an indeterminate teardown
/// never reopens or rewrites the original workspace lifecycle.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub(crate) struct TeardownReceipt {
    pub operation_id: String,
    pub workspace_id: String,
    pub endpoint_identity: String,
    pub checkout_path: String,
    pub companion: CompanionManifest,
    pub state: TeardownReceiptState,
    pub updated_at: String,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub(crate) enum TeardownReceiptState {
    Pending,
    OutcomeUnknown,
    OrphanedCompanion,
    Completed,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct StoredOperation {
    operation: WorkspaceOperation,
}

/// A short journal compare-and-swap lock. The lock file is retained forever;
/// ownership is the kernel lock on its open descriptor, not its age.
#[derive(Debug)]
pub(crate) struct LockGuard {
    _file: File,
}

impl Drop for LockGuard {
    fn drop(&mut self) {
        // Explicitly release even if another thread briefly forked a child
        // before its close-on-exec descriptors have been closed.
        let _ = fs2::FileExt::unlock(&self._file);
    }
}

/// A per-operation execution lease held across the complete side-effect
/// sequence. It is intentionally separate from journal CAS locks.
#[derive(Debug)]
pub struct ExecutionLease {
    _file: File,
}

/// Small file-backed journal for setup operations and companion associations.
///
/// The state directory is opened once and all record access is descriptor
/// relative. This prevents a path check followed by a path open from crossing a
/// symlink or replacement directory.
#[derive(Debug, Clone)]
pub struct ProjectStore {
    root_dir: Arc<Dir>,
}

impl ProjectStore {
    pub fn new(root: impl AsRef<Path>) -> Result<Self, InspectionError> {
        let (_, root_dir) = prepare_root(root.as_ref(), "state")?;
        Ok(Self {
            root_dir: Arc::new(root_dir),
        })
    }
    pub(crate) fn state_dir(&self) -> &Dir {
        self.root_dir.as_ref()
    }

    /// Acquire a lock in this store for a sibling persistence module.
    pub(crate) fn acquire_named_lock(
        &self,
        name: &str,
        code: &'static str,
    ) -> Result<LockGuard, InspectionError> {
        if name.is_empty() || name.contains('/') || name.contains('\\') || name.contains('\0') {
            return Err(InspectionError::new("unsafe_path", "invalid lock name"));
        }
        self.acquire_file_lock(name, code)
            .map(|file| LockGuard { _file: file })
    }

    pub(crate) fn try_acquire_named_lock(
        &self,
        name: &str,
        code: &'static str,
    ) -> Result<Option<LockGuard>, InspectionError> {
        if name.is_empty() || name.contains(['/', '\\', '\0']) {
            return Err(InspectionError::new("unsafe_path", "invalid lock name"));
        }
        self.try_acquire_file_lock(name, code)
            .map(|file| file.map(|file| LockGuard { _file: file }))
    }

    pub(crate) fn acquire_record_lock(&self, id: &str) -> Result<LockGuard, InspectionError> {
        validate_operation_id(id)?;
        self.acquire_named_lock(&format!(".{id}.lock"), "state_lock")
    }

    pub fn persist_plan(
        &self,
        plan: WorkspaceSetupPlan,
    ) -> Result<WorkspaceOperation, InspectionError> {
        validate_operation_id(&plan.operation_id)?;
        let now = timestamp();
        let operation = WorkspaceOperation {
            operation_id: plan.operation_id.clone(),
            generation: plan.generation,
            sequence: 0,
            session_id: plan.session_id.clone(),
            plan,
            state: cockpit_protocol::projects::WorkspaceOperationState::Planned,
            step: cockpit_protocol::projects::WorkspaceOperationStep::Planned,
            workspace_id: None,
            tab_id: None,
            pane_id: None,
            companion_id: None,
            owned_resources: Vec::new(),
            error: None,
            resume_allowed: false,
            cancel_requested: false,
            updated_at: now,
        };
        self.persist_initial(&operation)?;
        Ok(operation)
    }

    pub fn load(&self, operation_id: &str) -> Result<WorkspaceOperation, InspectionError> {
        validate_operation_id(operation_id)?;
        let _lock = self.acquire_lock(operation_id)?;
        read_json(&self.root_dir, &record_name(operation_id))
            .map(|stored: StoredOperation| stored.operation)
    }

    /// Apply one serialized change to the latest operation.
    ///
    /// The lock, generation, journal sequence, timestamp, and atomic write are
    /// all owned by the store. Callers only mutate operation state in `change`.
    pub fn update<F>(
        &self,
        operation_id: &str,
        expected_generation: Option<u32>,
        change: F,
    ) -> Result<WorkspaceOperation, InspectionError>
    where
        F: FnOnce(&mut WorkspaceOperation) -> Result<(), InspectionError>,
    {
        validate_operation_id(operation_id)?;
        let _lock = self.acquire_lock(operation_id)?;
        let name = record_name(operation_id);
        let stored: StoredOperation = read_json(&self.root_dir, &name)?;
        let mut operation = stored.operation;
        if let Some(expected) = expected_generation {
            if operation.generation != expected {
                return Err(InspectionError::new(
                    "stale_generation",
                    "operation generation is no longer current",
                ));
            }
        }
        let generation = operation.generation.checked_add(1).ok_or_else(|| {
            InspectionError::new("invalid_generation", "operation generation overflow")
        })?;
        let sequence = operation.sequence.checked_add(1).ok_or_else(|| {
            InspectionError::new("invalid_sequence", "operation sequence overflow")
        })?;
        change(&mut operation)?;
        if operation.operation_id != operation_id {
            return Err(InspectionError::new(
                "invalid_operation_id",
                "operation identity cannot change",
            ));
        }
        operation.generation = generation;
        operation.sequence = sequence;
        operation.updated_at = timestamp();
        atomic_write_json(
            &self.root_dir,
            &name,
            &StoredOperation {
                operation: operation.clone(),
            },
        )
        .map_err(|error| {
            if error.kind() == io::ErrorKind::InvalidInput {
                InspectionError::new("unsafe_path", "state destination is not a regular file")
            } else {
                map_io(error, "state_write")
            }
        })?;
        Ok(operation)
    }

    pub fn list(&self) -> Result<Vec<WorkspaceOperation>, InspectionError> {
        let mut result = Vec::new();
        let entries = self.root_dir.entries().map_err(io_error("state_read"))?;
        let mut entries_seen = 0usize;
        for entry in entries {
            entries_seen = entries_seen.saturating_add(1);
            if entries_seen > MAX_OPERATION_ENTRIES {
                return Err(InspectionError::new(
                    "state_lookup_bounded",
                    "operation lookup exceeded its bounded entry limit",
                ));
            }
            let entry = entry.map_err(io_error("state_read"))?;
            let name = entry.file_name();
            let Some(name) = name.to_str() else { continue };
            let Some(stem) = name.strip_suffix(".json") else {
                continue;
            };
            if Uuid::parse_str(stem).is_err() {
                continue;
            }
            let file_type = entry.file_type().map_err(io_error("state_read"))?;
            if file_type.is_symlink() || !file_type.is_file() {
                return Err(InspectionError::new(
                    "unsafe_path",
                    "state record is not a regular file",
                ));
            }
            let _lock = self.acquire_lock(stem)?;
            let stored = read_json::<StoredOperation>(&self.root_dir, name)?;
            result.push(stored.operation);
        }
        result.sort_by(|a, b| {
            a.updated_at
                .cmp(&b.updated_at)
                .then(a.operation_id.cmp(&b.operation_id))
        });
        Ok(result)
    }

    /// Acquire the durable lease which excludes another host from executing
    /// or recovering this operation. The lease is released when its owner
    /// drops (including a process crash).
    pub fn acquire_execution_lease(
        &self,
        operation_id: &str,
    ) -> Result<ExecutionLease, InspectionError> {
        validate_operation_id(operation_id)?;
        let file = self.acquire_file_lock(&lease_name(operation_id), "execution_lease")?;
        Ok(ExecutionLease { _file: file })
    }

    /// Attempt to acquire the execution lease without waiting.
    pub fn try_acquire_execution_lease(
        &self,
        operation_id: &str,
    ) -> Result<Option<ExecutionLease>, InspectionError> {
        validate_operation_id(operation_id)?;
        self.try_acquire_file_lock(&lease_name(operation_id), "execution_lease")
            .map(|file| file.map(|file| ExecutionLease { _file: file }))
    }

    pub fn write_companion(
        &self,
        companion_root: impl AsRef<Path>,
        manifest: &CompanionManifest,
    ) -> Result<PathBuf, InspectionError> {
        validate_operation_id(&manifest.cockpit_operation_id)?;
        validate_resource_id(&manifest.herdr_workspace_id, "workspace")?;
        if manifest.ownership != "cockpit" {
            return Err(InspectionError::new(
                "invalid_ownership",
                "companion ownership must be cockpit",
            ));
        }

        let (root_path, root) = prepare_root(companion_root.as_ref(), "companion")?;
        let id = manifest.cockpit_operation_id.as_str();
        let (temporary_name, temporary_dir) = create_companion_staging_dir(&root)?;
        if let Err(error) = atomic_write_json(&temporary_dir, "manifest.json", manifest) {
            let _ = temporary_dir.remove_open_dir_all();
            return Err(if error.kind() == io::ErrorKind::InvalidInput {
                InspectionError::new(
                    "unsafe_path",
                    "companion manifest destination is not a regular file",
                )
            } else {
                map_io(error, "companion_manifest")
            });
        }

        let publish_result = publish_companion_no_replace(&root, &temporary_name, id);
        if let Err(error) = publish_result {
            let _ = temporary_dir.remove_open_dir_all();
            if error.kind() == io::ErrorKind::AlreadyExists {
                return Err(InspectionError::new(
                    "association_conflict",
                    "companion directory already exists",
                ));
            }
            return Err(map_io(error, "companion_publish"));
        }
        drop(temporary_dir);
        Ok(root_path.join(id))
    }

    pub fn read_companion(
        &self,
        companion_root: impl AsRef<Path>,
        companion_id: &str,
    ) -> Result<CompanionManifest, InspectionError> {
        validate_operation_id(companion_id)?;
        let (_, root) = prepare_root(companion_root.as_ref(), "companion")?;
        let child = root.open_dir_nofollow(companion_id).map_err(|error| {
            if error.kind() == io::ErrorKind::NotFound {
                InspectionError::new("companion_missing", "companion directory does not exist")
            } else {
                map_io(error, "companion_read")
            }
        })?;
        match child.symlink_metadata("manifest.json") {
            Ok(_) => read_json(&child, "manifest.json"),
            Err(error) if error.kind() == io::ErrorKind::NotFound => Err(InspectionError::new(
                "companion_missing",
                "companion manifest does not exist",
            )),
            Err(error) => Err(map_io(error, "companion_read")),
        }
    }
    /// Enumerate bounded companion manifests without following any path
    /// component. Non-companion entries (including staging files) are ignored;
    /// a malformed manifest is an explicit association failure.
    pub fn list_companions(
        &self,
        companion_root: impl AsRef<Path>,
    ) -> Result<Vec<(String, CompanionManifest)>, InspectionError> {
        let (_, root) = prepare_root(companion_root.as_ref(), "companion")?;
        let mut companions = Vec::new();
        let entries = root.entries().map_err(io_error("companion_read"))?;
        let mut entries_seen = 0usize;
        for entry in entries {
            entries_seen = entries_seen.saturating_add(1);
            if entries_seen > MAX_COMPANION_ENTRIES {
                return Err(InspectionError::new(
                    "companion_lookup_bounded",
                    "companion lookup exceeded its bounded entry limit",
                ));
            }
            let entry = entry.map_err(io_error("companion_read"))?;
            let file_type = entry.file_type().map_err(io_error("companion_read"))?;
            if file_type.is_symlink() {
                return Err(InspectionError::new(
                    "unsafe_path",
                    "companion directory is a symlink",
                ));
            }
            if !file_type.is_dir() {
                continue;
            }
            let name = entry.file_name();
            let Some(id) = name.to_str() else {
                continue;
            };
            if Uuid::parse_str(id).is_err() {
                continue;
            }
            let child = root
                .open_dir_nofollow(id)
                .map_err(io_error("companion_read"))?;
            let Ok(metadata) = child.symlink_metadata("manifest.json") else {
                continue;
            };
            if metadata.file_type().is_symlink() || !metadata.is_file() {
                return Err(InspectionError::new(
                    "unsafe_path",
                    "companion manifest is not a regular file",
                ));
            }
            let manifest: CompanionManifest = read_json(&child, "manifest.json")?;
            if manifest.cockpit_operation_id != id {
                return Err(InspectionError::new(
                    "association_conflict",
                    "companion directory and manifest identities differ",
                ));
            }
            companions.push((id.to_owned(), manifest));
        }
        companions.sort_by(|left, right| left.0.cmp(&right.0));
        Ok(companions)
    }

    /// Replace only the association metadata for an existing companion.
    /// Generated and user files remain untouched. The creator operation identity
    /// and repository provenance are immutable; endpoint/workspace changes are
    /// allowed only through an explicit reattachment flow.
    pub fn reattach_companion(
        &self,
        companion_root: impl AsRef<Path>,
        manifest: &CompanionManifest,
    ) -> Result<PathBuf, InspectionError> {
        validate_operation_id(&manifest.cockpit_operation_id)?;
        validate_resource_id(&manifest.herdr_workspace_id, "workspace")?;
        if manifest.ownership != "cockpit" {
            return Err(InspectionError::new(
                "invalid_ownership",
                "companion ownership must be cockpit",
            ));
        }
        let _lock = self.acquire_lock(&manifest.cockpit_operation_id)?;
        let (_, root) = prepare_root(companion_root.as_ref(), "companion")?;
        let id = manifest.cockpit_operation_id.as_str();
        let child = root.open_dir_nofollow(id).map_err(|error| {
            if error.kind() == io::ErrorKind::NotFound {
                InspectionError::new("companion_missing", "companion directory does not exist")
            } else {
                map_io(error, "companion_read")
            }
        })?;
        let existing: CompanionManifest = read_json(&child, "manifest.json")?;
        if existing.cockpit_operation_id != manifest.cockpit_operation_id
            || existing.repository_key != manifest.repository_key
            || existing.repository_root != manifest.repository_root
            || existing.checkout_path != manifest.checkout_path
            || existing.artifact != manifest.artifact
            || existing.ownership != "cockpit"
        {
            return Err(InspectionError::new(
                "association_conflict",
                "companion provenance cannot be replaced",
            ));
        }
        atomic_write_json(&child, "manifest.json", manifest).map_err(|error| {
            if error.kind() == io::ErrorKind::InvalidInput {
                InspectionError::new(
                    "unsafe_path",
                    "companion manifest destination is not a regular file",
                )
            } else {
                map_io(error, "companion_manifest")
            }
        })?;
        Ok(companion_root.as_ref().to_path_buf().join(id))
    }

    /// Remove one exactly reviewed Cockpit-owned companion directory. The
    /// descriptor-relative operation cannot traverse outside `companion_root`;
    /// the manifest must still exactly match the reviewed association.
    pub fn remove_owned_companion(
        &self,
        companion_root: impl AsRef<Path>,
        manifest: &CompanionManifest,
    ) -> Result<(), InspectionError> {
        validate_operation_id(&manifest.cockpit_operation_id)?;
        if manifest.ownership != "cockpit" {
            return Err(InspectionError::new(
                "invalid_ownership",
                "companion ownership must be cockpit",
            ));
        }
        let _lock = self.acquire_lock(&manifest.cockpit_operation_id)?;
        let (_, root) = prepare_root(companion_root.as_ref(), "companion")?;
        let id = manifest.cockpit_operation_id.as_str();
        let child = root.open_dir_nofollow(id).map_err(|error| {
            if error.kind() == io::ErrorKind::NotFound {
                InspectionError::new("companion_missing", "companion directory does not exist")
            } else {
                map_io(error, "companion_read")
            }
        })?;
        let existing: CompanionManifest = read_json(&child, "manifest.json")?;
        if existing != *manifest {
            return Err(InspectionError::new(
                "association_conflict",
                "companion association changed after teardown review",
            ));
        }
        root.remove_dir_all(id).map_err(|error| {
            if error.kind() == io::ErrorKind::InvalidInput {
                InspectionError::new("unsafe_path", "companion destination is unsafe")
            } else {
                map_io(error, "companion_remove")
            }
        })
    }

    pub(crate) fn read_teardown_receipt(
        &self,
        operation_id: &str,
    ) -> Result<Option<TeardownReceipt>, InspectionError> {
        validate_operation_id(operation_id)?;
        let _lock = self.acquire_named_lock(&teardown_lock_name(operation_id), "teardown_lock")?;
        let name = teardown_record_name(operation_id);
        match self.root_dir.symlink_metadata(&name) {
            Ok(metadata) if metadata.file_type().is_symlink() || !metadata.is_file() => Err(
                InspectionError::new("unsafe_path", "teardown receipt is not a regular file"),
            ),
            Ok(_) => read_json(&self.root_dir, &name).map(Some),
            Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(None),
            Err(error) => Err(map_io(error, "teardown_read")),
        }
    }

    pub(crate) fn write_teardown_receipt(
        &self,
        receipt: &TeardownReceipt,
    ) -> Result<(), InspectionError> {
        validate_operation_id(&receipt.operation_id)?;
        validate_resource_id(&receipt.workspace_id, "workspace")?;
        if receipt.companion.cockpit_operation_id != receipt.operation_id
            || receipt.companion.ownership != "cockpit"
            || receipt.checkout_path != receipt.companion.checkout_path
            || receipt.endpoint_identity != receipt.companion.herdr_session_identity
            || receipt.workspace_id != receipt.companion.herdr_workspace_id
        {
            return Err(InspectionError::new(
                "association_conflict",
                "teardown receipt does not match its reviewed companion",
            ));
        }
        let _lock =
            self.acquire_named_lock(&teardown_lock_name(&receipt.operation_id), "teardown_lock")?;
        atomic_write_json(
            &self.root_dir,
            &teardown_record_name(&receipt.operation_id),
            receipt,
        )
        .map_err(|error| {
            if error.kind() == io::ErrorKind::InvalidInput {
                InspectionError::new("unsafe_path", "teardown receipt destination is unsafe")
            } else {
                map_io(error, "teardown_write")
            }
        })
    }

    fn acquire_lock(&self, id: &str) -> Result<LockGuard, InspectionError> {
        let file = self.acquire_file_lock(&format!(".{id}.lock"), "state_lock")?;
        Ok(LockGuard { _file: file })
    }

    fn open_lock_file(&self, name: &str, code: &'static str) -> Result<File, InspectionError> {
        match self.root_dir.symlink_metadata(name) {
            Ok(metadata) if metadata.file_type().is_symlink() || !metadata.is_file() => {
                return Err(InspectionError::new(
                    "unsafe_path",
                    "lock path is not a regular file",
                ));
            }
            Ok(_) => {}
            Err(error) if error.kind() == io::ErrorKind::NotFound => {}
            Err(error) => return Err(map_io(error, code)),
        }
        let mut options = OpenOptions::new();
        options
            .read(true)
            .write(true)
            .create(true)
            .follow(cap_fs_ext::FollowSymlinks::No)
            .nonblock(true);
        let file = self
            .root_dir
            .open_with(name, &options)
            .map_err(|error| {
                if error.kind() == io::ErrorKind::InvalidInput
                    || error.kind() == io::ErrorKind::PermissionDenied
                    || error.kind() == io::ErrorKind::IsADirectory
                {
                    InspectionError::new("unsafe_path", "lock path is not a safe regular file")
                } else {
                    map_io(error, code)
                }
            })?
            .into_std();
        if !file.metadata().map_err(io_error(code))?.is_file() {
            return Err(InspectionError::new(
                "unsafe_path",
                "lock path is not a regular file",
            ));
        }
        Ok(file)
    }

    fn acquire_file_lock(&self, name: &str, code: &'static str) -> Result<File, InspectionError> {
        let file = self.open_lock_file(name, code)?;
        let deadline = Instant::now() + LOCK_WAIT;
        loop {
            match file.try_lock_exclusive() {
                Ok(()) => return Ok(file),
                Err(error) if error.kind() == io::ErrorKind::WouldBlock => {
                    if Instant::now() >= deadline {
                        return Err(InspectionError::new(
                            "store_busy",
                            "operation lock acquisition timed out",
                        ));
                    }
                    thread::sleep(Duration::from_millis(10));
                }
                Err(error) => return Err(map_io(error, code)),
            }
        }
    }

    fn try_acquire_file_lock(
        &self,
        name: &str,
        code: &'static str,
    ) -> Result<Option<File>, InspectionError> {
        let file = self.open_lock_file(name, code)?;
        match file.try_lock_exclusive() {
            Ok(()) => Ok(Some(file)),
            Err(error) if error.kind() == io::ErrorKind::WouldBlock => Ok(None),
            Err(error) => Err(map_io(error, code)),
        }
    }

    fn persist_initial(&self, operation: &WorkspaceOperation) -> Result<(), InspectionError> {
        let _lock = self.acquire_lock(&operation.operation_id)?;
        let name = record_name(&operation.operation_id);
        match self.root_dir.symlink_metadata(&name) {
            Ok(_) => {
                return Err(InspectionError::new(
                    "operation_exists",
                    "operation journal already exists",
                ));
            }
            Err(error) if error.kind() == io::ErrorKind::NotFound => {}
            Err(error) => return Err(map_io(error, "state_read")),
        }
        atomic_write_json(
            &self.root_dir,
            &name,
            &StoredOperation {
                operation: operation.clone(),
            },
        )
        .map_err(|error| {
            if error.kind() == io::ErrorKind::InvalidInput {
                InspectionError::new("unsafe_path", "state destination is not a regular file")
            } else {
                map_io(error, "state_write")
            }
        })
    }
}

fn absolute_root(path: &Path) -> io::Result<PathBuf> {
    let raw = if path.is_absolute() {
        path.to_path_buf()
    } else {
        std::env::current_dir()?.join(path)
    };
    let mut normalized = PathBuf::new();
    for component in raw.components() {
        match component {
            Component::RootDir => normalized.push(Path::new("/")),
            Component::CurDir => {}
            Component::Normal(name) => normalized.push(name),
            Component::ParentDir | Component::Prefix(_) => {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidInput,
                    "unsafe root path",
                ));
            }
        }
    }
    Ok(normalized)
}
pub(crate) fn prepare_root(path: &Path, kind: &str) -> Result<(PathBuf, Dir), InspectionError> {
    let absolute = absolute_root(path).map_err(|error| {
        if error.kind() == io::ErrorKind::InvalidInput {
            InspectionError::new("unsafe_path", format!("unsafe {kind} root path"))
        } else {
            map_io(error, "root_open")
        }
    })?;
    let mut dir = Dir::open_ambient_dir(Path::new("/"), cap_std::ambient_authority())
        .map_err(io_error("root_open"))?;
    for component in absolute.components() {
        match component {
            Component::RootDir | Component::CurDir => {}
            Component::Normal(name) => {
                match dir.symlink_metadata(name) {
                    Ok(metadata) if metadata.file_type().is_symlink() || !metadata.is_dir() => {
                        return Err(InspectionError::new(
                            "unsafe_path",
                            format!("{kind} root is not a real directory"),
                        ));
                    }
                    Ok(_) => {}
                    Err(error) if error.kind() == io::ErrorKind::NotFound => {
                        if let Err(error) = dir.create_dir(name) {
                            if error.kind() != io::ErrorKind::AlreadyExists {
                                return Err(map_io(error, "root_create"));
                            }
                        }
                    }
                    Err(error) => return Err(map_io(error, "root_stat")),
                }
                dir = dir.open_dir_nofollow(Path::new(name)).map_err(|error| {
                    if error.kind() == io::ErrorKind::InvalidInput
                        || error.kind() == io::ErrorKind::NotFound
                    {
                        InspectionError::new(
                            "unsafe_path",
                            format!("{kind} root is not a real directory"),
                        )
                    } else {
                        map_io(error, "root_open")
                    }
                })?;
            }
            Component::ParentDir | Component::Prefix(_) => {
                return Err(InspectionError::new(
                    "unsafe_path",
                    format!("unsafe {kind} root path"),
                ));
            }
        }
    }
    if !dir.dir_metadata().map_err(io_error("root_stat"))?.is_dir() {
        return Err(InspectionError::new(
            "unsafe_path",
            format!("{kind} root is not a real directory"),
        ));
    }
    Ok((absolute, dir))
}

/// Safely create a project root and return its absolute, descriptor-checked path.
pub(crate) fn prepare_project_root(path: &Path) -> Result<PathBuf, InspectionError> {
    prepare_root(path, "project").map(|(path, _)| path)
}

/// Validate an existing project root without creating any component.
pub(crate) fn validate_project_root(path: &Path) -> Result<(), InspectionError> {
    let absolute = absolute_root(path).map_err(|error| {
        if error.kind() == io::ErrorKind::InvalidInput {
            InspectionError::new("unsafe_path", "unsafe project root path")
        } else {
            map_io(error, "root_open")
        }
    })?;
    let dir = open_dir_nofollow_absolute(&absolute).map_err(|error| {
        if error.kind() == io::ErrorKind::InvalidInput || error.kind() == io::ErrorKind::NotFound {
            InspectionError::new("unsafe_path", "project root is not a real directory")
        } else {
            map_io(error, "root_open")
        }
    })?;
    if !dir.dir_metadata().map_err(io_error("root_stat"))?.is_dir() {
        return Err(InspectionError::new(
            "unsafe_path",
            "project root is not a real directory",
        ));
    }
    Ok(())
}

pub(crate) fn open_dir_nofollow_absolute(path: &Path) -> io::Result<Dir> {
    let absolute = absolute_root(path)?;
    let mut dir = Dir::open_ambient_dir(Path::new("/"), cap_std::ambient_authority())?;
    for component in absolute.components() {
        match component {
            Component::RootDir | Component::CurDir => {}
            Component::Normal(name) => {
                dir = dir.open_dir_nofollow(Path::new(name))?;
            }
            Component::ParentDir | Component::Prefix(_) => {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidInput,
                    "unsafe root path",
                ));
            }
        }
    }
    Ok(dir)
}

fn create_companion_staging_dir(root: &Dir) -> Result<(String, Dir), InspectionError> {
    for _ in 0..16 {
        let name = format!(".companion-{}.tmp", Uuid::new_v4());
        match root.create_dir(&name) {
            Ok(()) => {
                let dir = match root.open_dir_nofollow(&name) {
                    Ok(dir) => dir,
                    Err(error) => {
                        let _ = root.remove_dir_all(&name);
                        return Err(map_io(error, "companion_stage"));
                    }
                };
                return Ok((name, dir));
            }
            Err(error) if error.kind() == io::ErrorKind::AlreadyExists => continue,
            Err(error) => return Err(map_io(error, "companion_stage")),
        }
    }
    Err(InspectionError::new(
        "companion_stage",
        "could not allocate a unique staging directory",
    ))
}

#[cfg(all(target_os = "linux", target_env = "gnu"))]
fn publish_companion_no_replace(
    root: &Dir,
    temporary_name: &str,
    operation_id: &str,
) -> io::Result<()> {
    renameat2(
        root,
        temporary_name,
        root,
        operation_id,
        RenameFlags::RENAME_NOREPLACE,
    )
    .map_err(|error| io::Error::from_raw_os_error(error as i32))
}

#[cfg(not(all(target_os = "linux", target_env = "gnu")))]
fn publish_companion_no_replace(
    _root: &Dir,
    _temporary_name: &str,
    _operation_id: &str,
) -> io::Result<()> {
    Err(io::Error::new(
        io::ErrorKind::Unsupported,
        "atomic no-replace companion publication is supported only on GNU/Linux",
    ))
}

fn record_name(id: &str) -> String {
    format!("{id}.json")
}

fn lease_name(id: &str) -> String {
    format!(".{id}.exec.lock")
}

fn teardown_record_name(id: &str) -> String {
    format!("{id}.teardown.json")
}

fn teardown_lock_name(id: &str) -> String {
    format!(".{id}.teardown.lock")
}

pub(crate) fn atomic_write_json<T: Serialize>(dir: &Dir, name: &str, value: &T) -> io::Result<()> {
    let tmp = format!(".{}.tmp", Uuid::new_v4());
    let bytes = serde_json::to_vec_pretty(value)
        .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))?;
    let mut options = OpenOptions::new();
    options
        .write(true)
        .create_new(true)
        .follow(cap_fs_ext::FollowSymlinks::No);
    let mut file = dir.open_with(&tmp, &options)?;
    file.write_all(&bytes)?;
    match dir.symlink_metadata(name) {
        Ok(metadata) if metadata.file_type().is_symlink() || !metadata.is_file() => {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "destination is not a regular file",
            ));
        }
        Ok(_) => {}
        Err(error) if error.kind() == io::ErrorKind::NotFound => {}
        Err(error) => return Err(error),
    }
    dir.rename(&tmp, dir, name)?;
    Ok(())
}

fn read_json<T: for<'de> Deserialize<'de>>(dir: &Dir, name: &str) -> Result<T, InspectionError> {
    read_json_bounded(dir, name, MAX_RECORD_BYTES)
}

pub(crate) fn read_json_bounded<T: for<'de> Deserialize<'de>>(
    dir: &Dir,
    name: &str,
    max_bytes: u64,
) -> Result<T, InspectionError> {
    let metadata = dir.symlink_metadata(name).map_err(io_error("state_read"))?;
    if metadata.file_type().is_symlink() || !metadata.is_file() || metadata.len() > max_bytes {
        return Err(InspectionError::new(
            "unsafe_path",
            "state record is not a bounded regular file",
        ));
    }
    let mut options = OpenOptions::new();
    options
        .read(true)
        .follow(cap_fs_ext::FollowSymlinks::No)
        .nonblock(true);
    let mut file = dir
        .open_with(name, &options)
        .map_err(io_error("state_read"))?;
    let mut bytes = Vec::with_capacity(metadata.len() as usize);
    file.read_to_end(&mut bytes)
        .map_err(io_error("state_read"))?;
    if bytes.len() as u64 > max_bytes {
        return Err(InspectionError::new(
            "unsafe_path",
            "state record exceeded its bounded size while reading",
        ));
    }
    serde_json::from_slice(&bytes).map_err(|e| InspectionError::new("state_corrupt", e.to_string()))
}
fn validate_operation_id(value: &str) -> Result<(), InspectionError> {
    if Uuid::parse_str(value).is_err() {
        return Err(InspectionError::new(
            "invalid_operation_id",
            "operation identity must be a UUID",
        ));
    }
    Ok(())
}

fn validate_resource_id(value: &str, kind: &str) -> Result<(), InspectionError> {
    if value.is_empty()
        || value.len() > 256
        || value.contains('/')
        || value.contains('\\')
        || value.contains('\0')
    {
        return Err(InspectionError::new(
            "invalid_identity",
            format!("invalid {kind} identity"),
        ));
    }
    Ok(())
}

pub(crate) fn timestamp() -> String {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis()
        .to_string()
}

fn io_error(code: &'static str) -> impl Fn(io::Error) -> InspectionError {
    move |error| map_io(error, code)
}

fn map_io(error: io::Error, code: &str) -> InspectionError {
    InspectionError::new(code, error.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use cockpit_protocol::projects::WorkspaceSetupMode;
    use std::fs;
    use std::sync::Arc;

    fn temp_root(label: &str) -> PathBuf {
        let path =
            std::env::temp_dir().join(format!("cockpit-project-store-{label}-{}", Uuid::new_v4()));
        fs::create_dir_all(&path).expect("temporary root");
        path
    }

    fn plan(id: &str) -> WorkspaceSetupPlan {
        WorkspaceSetupPlan {
            operation_id: id.to_owned(),
            generation: 1,
            endpoint_identity: "endpoint-a".to_owned(),
            session_id: "setup-fixture".to_owned(),
            repository: cockpit_protocol::projects::RepositoryCandidate {
                repository_id: "repo-a".to_owned(),
                name: "repo".to_owned(),
                root: "/repo".to_owned(),
                checkout_path: "/repo".to_owned(),
                common_dir: "/repo/.git".to_owned(),
                branch: Some("main".to_owned()),
                is_linked_worktree: false,
                is_detached: false,
                provenance: "p".to_owned(),
            },
            mode: WorkspaceSetupMode::Create,
            branch: Some("task".to_owned()),
            base: Some("main".to_owned()),
            checkout_path: "/work/task".to_owned(),
            companion_path: "/companion/id".to_owned(),
            companion_id: id.to_owned(),
            companion_created_by_operation: true,
            label: "Task".to_owned(),
            focus: false,
            trust_repository: false,
            artifact: None,
            effects: vec!["create".to_owned()],
            warnings: vec![],
        }
    }

    fn manifest(id: &str) -> CompanionManifest {
        CompanionManifest {
            schema_version: 1,
            cockpit_operation_id: id.to_owned(),
            herdr_session_identity: "session-a".to_owned(),
            herdr_workspace_id: "workspace-a".to_owned(),
            repository_key: "repo-a".to_owned(),
            repository_root: "/repo".to_owned(),
            checkout_path: "/repo".to_owned(),
            artifact: None,
            created_at: "1".to_owned(),
            updated_at: "1".to_owned(),
            ownership: "cockpit".to_owned(),
        }
    }

    fn teardown_receipt(id: &str, state: TeardownReceiptState) -> TeardownReceipt {
        let mut companion = manifest(id);
        companion.herdr_session_identity = "endpoint-a".to_owned();
        companion.herdr_workspace_id = "workspace-a".to_owned();
        TeardownReceipt {
            operation_id: id.to_owned(),
            workspace_id: "workspace-a".to_owned(),
            endpoint_identity: "endpoint-a".to_owned(),
            checkout_path: companion.checkout_path.clone(),
            companion,
            state,
            updated_at: "1".to_owned(),
        }
    }

    #[test]
    fn teardown_receipt_survives_reopen_and_preserves_unknown_state() {
        let root = temp_root("teardown-receipt-restart");
        let id = Uuid::new_v4().to_string();
        let mut receipt = teardown_receipt(&id, TeardownReceiptState::Pending);
        ProjectStore::new(&root)
            .expect("first store")
            .write_teardown_receipt(&receipt)
            .expect("write pending receipt");

        let reopened = ProjectStore::new(&root).expect("reopened store");
        assert_eq!(
            reopened
                .read_teardown_receipt(&id)
                .expect("read pending receipt")
                .expect("receipt"),
            receipt,
        );
        receipt.state = TeardownReceiptState::OutcomeUnknown;
        receipt.updated_at = "2".to_owned();
        reopened
            .write_teardown_receipt(&receipt)
            .expect("persist unknown receipt");
        drop(reopened);

        assert_eq!(
            ProjectStore::new(&root)
                .expect("second reopened store")
                .read_teardown_receipt(&id)
                .expect("read unknown receipt")
                .expect("receipt")
                .state,
            TeardownReceiptState::OutcomeUnknown,
        );
        fs::remove_dir_all(root).expect("cleanup");
    }

    #[test]
    fn concurrent_updates_serialize_latest_state_and_journal() {
        let root = temp_root("update");
        let store = Arc::new(ProjectStore::new(&root).expect("store"));
        let id = Uuid::new_v4().to_string();
        store.persist_plan(plan(&id)).expect("persist");
        let left = Arc::clone(&store);
        let right = Arc::clone(&store);
        let left_id = id.clone();
        let right_id = id.clone();
        let t1 = std::thread::spawn(move || {
            left.update(&left_id, None, |operation| {
                operation.cancel_requested = true;
                Ok(())
            })
        });
        let t2 = std::thread::spawn(move || {
            right.update(&right_id, None, |operation| {
                operation.resume_allowed = true;
                Ok(())
            })
        });
        let first = t1.join().expect("thread").expect("first update");
        let second = t2.join().expect("thread").expect("second update");
        assert_eq!(first.generation.min(second.generation), 2);
        assert_eq!(first.generation.max(second.generation), 3);
        assert_eq!(store.load(&id).expect("load").sequence, 2);
        fs::remove_dir_all(root).expect("cleanup");
    }

    #[test]
    fn expected_generation_update_allows_one_writer() {
        let root = temp_root("update-cas");
        let store = Arc::new(ProjectStore::new(&root).expect("store"));
        let id = Uuid::new_v4().to_string();
        store.persist_plan(plan(&id)).expect("persist");
        let left = Arc::clone(&store);
        let right = Arc::clone(&store);
        let left_id = id.clone();
        let right_id = id.clone();
        let t1 = std::thread::spawn(move || left.update(&left_id, Some(1), |_| Ok(())));
        let t2 = std::thread::spawn(move || right.update(&right_id, Some(1), |_| Ok(())));
        let outcomes = [t1.join().expect("thread"), t2.join().expect("thread")];
        assert_eq!(outcomes.iter().filter(|result| result.is_ok()).count(), 1);
        assert!(outcomes.iter().any(|result| {
            result
                .as_ref()
                .err()
                .is_some_and(|error| error.code == "stale_generation")
        }));
        fs::remove_dir_all(root).expect("cleanup");
    }

    #[test]
    fn try_execution_lease_is_nonblocking_and_exclusive() {
        let root = temp_root("try-execution-lease");
        let store = ProjectStore::new(&root).expect("store");
        let id = Uuid::new_v4().to_string();
        let first = store.acquire_execution_lease(&id).expect("first lease");
        assert!(
            store
                .try_acquire_execution_lease(&id)
                .expect("try lease")
                .is_none()
        );
        drop(first);
        assert!(
            store
                .try_acquire_execution_lease(&id)
                .expect("released try lease")
                .is_some()
        );
        fs::remove_dir_all(root).expect("cleanup");
    }

    #[cfg(all(target_os = "linux", target_env = "gnu"))]
    #[test]
    fn foreign_empty_companion_destination_is_preserved_and_retry_publishes() {
        let state = temp_root("foreign-destination-state");
        let companions = temp_root("foreign-destination-companions");
        let store = ProjectStore::new(&state).expect("store");
        let id = Uuid::new_v4().to_string();
        let destination = companions.join(&id);
        fs::create_dir(&destination).expect("foreign destination");
        let error = store
            .write_companion(&companions, &manifest(&id))
            .expect_err("foreign destination must win");
        assert_eq!(error.code, "association_conflict");
        assert!(destination.is_dir());
        assert!(
            fs::read_dir(&destination)
                .expect("destination entries")
                .next()
                .is_none()
        );
        fs::remove_dir(&destination).expect("remove foreign destination");
        let published = store
            .write_companion(&companions, &manifest(&id))
            .expect("retry publication");
        assert_eq!(published, destination);
        assert_eq!(
            store
                .read_companion(&companions, &id)
                .expect("read manifest"),
            manifest(&id)
        );
        fs::remove_dir_all(state).expect("cleanup state");
        fs::remove_dir_all(companions).expect("cleanup companions");
    }

    #[cfg(unix)]
    #[test]
    fn project_root_rejects_intermediate_symlink_without_creation() {
        use std::os::unix::fs::symlink;
        let root = temp_root("root-symlink");
        let target = temp_root("root-symlink-target");
        let link = root.join("linked");
        symlink(&target, &link).expect("intermediate symlink");
        let nested = link.join("new-root");
        let error = prepare_project_root(&nested).expect_err("symlink root must fail");
        assert_eq!(error.code, "unsafe_path");
        assert!(!target.join("new-root").exists());
        fs::remove_dir_all(root).expect("cleanup root");
        fs::remove_dir_all(target).expect("cleanup target");
    }

    #[cfg(unix)]
    #[test]
    fn symlinked_state_record_is_rejected() {
        use std::os::unix::fs::symlink;
        let root = temp_root("symlink");
        let store = ProjectStore::new(&root).expect("store");
        let id = Uuid::new_v4().to_string();

        store.persist_plan(plan(&id)).expect("persist");
        let record = root.join(format!("{id}.json"));
        let target = root.join("target.json");
        fs::rename(&record, &target).expect("move record");
        symlink(&target, &record).expect("symlink record");
        let error = store.load(&id).expect_err("symlink must fail");
        assert_eq!(error.code, "unsafe_path");
        fs::remove_dir_all(root).expect("cleanup");
    }
    #[test]
    fn execution_lease_file_is_retained_after_release() {
        let root = temp_root("execution-lease");
        let store = ProjectStore::new(&root).expect("store");
        let id = Uuid::new_v4().to_string();
        let lease = store.acquire_execution_lease(&id).expect("lease");
        let lock_path = root.join(format!(".{id}.exec.lock"));
        assert!(lock_path.is_file());
        drop(lease);
        assert!(lock_path.is_file(), "lock inode must not be age-unlinked");
        fs::remove_dir_all(root).expect("cleanup");
    }

    #[test]
    fn execution_lease_excludes_concurrent_host_until_release() {
        let root = temp_root("execution-exclusion");
        let store = Arc::new(ProjectStore::new(&root).expect("store"));
        let id = Uuid::new_v4().to_string();
        let first = store.acquire_execution_lease(&id).expect("first lease");
        let other = Arc::clone(&store);
        let (tx, rx) = std::sync::mpsc::channel();
        let id_for_thread = id.clone();
        let thread = std::thread::spawn(move || {
            let result = other.acquire_execution_lease(&id_for_thread);
            tx.send(result.is_ok()).expect("result");
        });
        assert!(
            rx.recv_timeout(std::time::Duration::from_millis(100))
                .is_err(),
            "live lease must exclude another host"
        );
        drop(first);
        assert!(
            rx.recv_timeout(std::time::Duration::from_secs(1))
                .expect("released lease result")
        );
        thread.join().expect("thread");
        fs::remove_dir_all(root).expect("cleanup");
    }

    #[cfg(unix)]
    #[test]
    fn companion_symlink_substitution_is_rejected() {
        use std::os::unix::fs::symlink;
        let root = temp_root("companion-link");
        let outside = temp_root("companion-outside");
        let store = ProjectStore::new(&root).expect("store");
        let id = Uuid::new_v4().to_string();
        symlink(&outside, root.join(&id)).expect("link");
        let error = store
            .read_companion(&root, &id)
            .expect_err("symlink must fail");
        assert_ne!(error.code, "state_corrupt");
        fs::remove_dir_all(root).expect("cleanup root");
        fs::remove_dir_all(outside).expect("cleanup outside");
    }
}
