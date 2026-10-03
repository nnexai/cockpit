use std::fs::File;
use std::io::{self, Read, Write};
use std::path::{Component, Path, PathBuf};
use std::sync::{Arc, atomic::{AtomicU64, Ordering}};
use std::thread;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use cap_fs_ext::{DirExt, OpenOptionsFollowExt, OpenOptionsSyncExt};
use cap_std::fs::{Dir, OpenOptions};
use cockpit_protocol::projects::{WorkspaceOperation, WorkspaceSetupPlan};
use fs2::FileExt;
use serde::{Deserialize, Serialize};
use uuid::Uuid;


use crate::InspectionError;

const LOCK_WAIT: Duration = Duration::from_secs(5);
const MAX_RECORD_BYTES: u64 = 2 * 1024 * 1024;
const MAX_OPERATION_ENTRIES: usize = 4096;

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
    pub session_id: String,
    pub repository_key: String,
    pub repository_root: String,
    pub state: TeardownReceiptState,
    pub updated_at: String,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub(crate) enum TeardownReceiptState {
    Pending,
    OutcomeUnknown,
    Completed,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct StoredOperation {
    operation: WorkspaceOperation,
}

/// Remove the retired lifecycle before serde sees the current, strict DTOs.
/// This only rewrites the journal; companion/user paths are never opened.
fn migrate_legacy_operation_json(value: &mut serde_json::Value) -> bool {
    let Some(operation) = value.get_mut("operation").and_then(serde_json::Value::as_object_mut) else {
        return false;
    };
    let mut changed = operation.remove("companion_id").is_some();
    if let Some(plan) = operation.get_mut("plan").and_then(serde_json::Value::as_object_mut) {
        for field in ["companion_path", "companion_id", "companion_created_by_operation"] {
            changed |= plan.remove(field).is_some();
        }
        if let Some(effects) = plan.get_mut("effects").and_then(serde_json::Value::as_array_mut) {
            let before = effects.len();
            effects.retain(|effect| !effect.as_str().is_some_and(|effect| effect.contains("companion")));
            changed |= effects.len() != before;
        }
    }
    if let Some(resources) = operation.get_mut("owned_resources").and_then(serde_json::Value::as_array_mut) {
        let before = resources.len();
        resources.retain(|resource| resource.get("kind").and_then(serde_json::Value::as_str) != Some("companion"));
        changed |= resources.len() != before;
    }
    if operation.get("step").and_then(serde_json::Value::as_str)
        .is_some_and(|step| matches!(step, "companion_ready" | "associate_companion"))
    {
        operation.insert("step".to_owned(), serde_json::json!("workspace_verified"));
        changed = true;
    }
    changed
}

fn receipt_matches_operation(receipt: &TeardownReceipt, operation: &WorkspaceOperation) -> bool {
    let repository = operation.plan.repository.as_ref();
    receipt.operation_id == operation.operation_id
        && receipt.operation_id == operation.plan.operation_id
        && operation.workspace_id.as_deref() == Some(receipt.workspace_id.as_str())
        && receipt.endpoint_identity == operation.plan.endpoint_identity
        && receipt.session_id == operation.session_id
        && receipt.session_id == operation.plan.session_id
        && receipt.checkout_path == operation.plan.checkout_path
        && receipt.repository_key == repository.map(|repository| repository.common_dir.as_str()).unwrap_or("")
        && receipt.repository_root == repository.map(|repository| repository.root.as_str()).unwrap_or("")
}

fn migrate_legacy_teardown_receipt(
    value: &mut serde_json::Value,
    load: impl FnOnce() -> Result<WorkspaceOperation, InspectionError>,
) -> Result<bool, InspectionError> {
    let Some(receipt) = value.as_object_mut() else {
        return Ok(false);
    };
    let orphaned = receipt.get("state").and_then(serde_json::Value::as_str) == Some("orphaned_companion");
    let companion = receipt.remove("companion");
    let mut changed = companion.is_some() || orphaned;
    // Herdr removal was already confirmed. Retire only the obsolete cleanup
    // obligation; every old directory and note is retained.
    if orphaned {
        receipt.insert("state".to_owned(), serde_json::json!("completed"));
    }
    if let Some(companion) = companion {
        let operation = if orphaned { load().ok() } else { Some(load()?) };
        let exact = operation.as_ref().is_some_and(|operation| {
            let repository = operation.plan.repository.as_ref();
            companion.get("cockpit_operation_id") == receipt.get("operation_id")
            && companion.get("herdr_workspace_id") == receipt.get("workspace_id")
            && companion.get("herdr_session_identity") == receipt.get("endpoint_identity")
            && companion.get("checkout_path") == receipt.get("checkout_path")
            && companion.get("ownership").and_then(serde_json::Value::as_str) == Some("cockpit")
            && receipt.get("operation_id").and_then(serde_json::Value::as_str) == Some(operation.operation_id.as_str())
            && receipt.get("workspace_id").and_then(serde_json::Value::as_str) == operation.workspace_id.as_deref()
            && receipt.get("endpoint_identity").and_then(serde_json::Value::as_str) == Some(operation.plan.endpoint_identity.as_str())
            && receipt.get("checkout_path").and_then(serde_json::Value::as_str) == Some(operation.plan.checkout_path.as_str())
            && companion.get("repository_key").and_then(serde_json::Value::as_str) == Some(repository.map(|repository| repository.common_dir.as_str()).unwrap_or(""))
            && companion.get("repository_root").and_then(serde_json::Value::as_str) == Some(repository.map(|repository| repository.root.as_str()).unwrap_or(""))
        });
        // Foreign evidence cannot acquire reconciliation authority. An
        // identity mismatch keeps the obligation but prohibits redispatch.
        receipt.insert("session_id".to_owned(), serde_json::json!(
            operation.as_ref().filter(|_| exact).map(|operation| operation.session_id.as_str()).unwrap_or("")
        ));
        for field in ["repository_key", "repository_root"] {
            receipt.insert(field.to_owned(), companion.get(field).cloned().unwrap_or(serde_json::json!("")));
        }
    }
    for field in ["companion_path", "companion_id", "companion_created_by_operation"] {
        changed |= receipt.remove(field).is_some();
    }
    Ok(changed)
}

fn migrate_legacy_operation(operation: &mut WorkspaceOperation) -> bool {
    use cockpit_protocol::{
        projects::{WorkspaceCheckoutOwnership, WorkspaceOperationState, WorkspaceSetupMode},
        v1::ErrorResponse,
    };

    // `ownership` was absent before path-only setup. Serde defaults it to
    // borrowed so incomplete history can never gain delete authority. An
    // exact created-worktree receipt is the only proof that can promote a
    // legacy Create. Open has always borrowed its checkout.
    if operation.plan.mode == WorkspaceSetupMode::Open {
        if operation.plan.ownership != WorkspaceCheckoutOwnership::BorrowedDirectory {
            operation.plan.ownership = WorkspaceCheckoutOwnership::BorrowedDirectory;
            return true;
        }
        return false;
    }
    if operation.plan.ownership != WorkspaceCheckoutOwnership::BorrowedDirectory {
        return false;
    }
    if operation.owned_resources.iter().any(|resource| {
        resource.kind == "worktree"
            && resource.path == operation.plan.checkout_path
            && resource.created_by_operation
    }) {
        operation.plan.ownership = WorkspaceCheckoutOwnership::OwnedWorktree;
        return true;
    }
    let error = ErrorResponse {
        code: "needs_review".to_owned(),
        message: "legacy Create operation lacks a recorded created-worktree receipt".to_owned(),
    };
    if operation.state == WorkspaceOperationState::NeedsReview
        && !operation.resume_allowed
        && operation.error.as_ref() == Some(&error)
    {
        return false;
    }
    operation.state = WorkspaceOperationState::NeedsReview;
    operation.resume_allowed = false;
    operation.error = Some(error);
    true
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

/// Small file-backed journal for setup operations and teardown receipts.
///
/// The state directory is opened once and all record access is descriptor
/// relative. This prevents a path check followed by a path open from crossing a
/// symlink or replacement directory.
#[derive(Debug, Clone)]
pub struct ProjectStore {
    root_dir: Arc<Dir>,
    mutation_generation: Arc<AtomicU64>,
}

impl ProjectStore {
    pub fn new(root: impl AsRef<Path>) -> Result<Self, InspectionError> {
        let (_, root_dir) = prepare_root(root.as_ref(), "state")?;
        Ok(Self {
            root_dir: Arc::new(root_dir),
            mutation_generation: Arc::new(AtomicU64::new(0)),
        })
    }
    pub(crate) fn state_dir(&self) -> &Dir {
        self.root_dir.as_ref()
    }
    pub(crate) fn mutation_generation(&self) -> u64 {
        self.mutation_generation.load(Ordering::Acquire)
    }

    fn bump_mutation_generation(&self) {
        self.mutation_generation.fetch_add(1, Ordering::AcqRel);
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
            owned_resources: Vec::new(),
            error: None,
            resume_allowed: false,
            cancel_requested: false,
            updated_at: now,
        };
        self.persist_initial(&operation)?;
        Ok(operation)
    }

    /// Delete a plan that was never started. Such a record owns no Herdr or
    /// filesystem resource. Returns false, and keeps the record, when it has
    /// started, changed, or is being executed by another host.
    pub fn discard_unstarted_plan(&self, operation_id: &str) -> Result<bool, InspectionError> {
        use cockpit_protocol::projects::{WorkspaceOperationState, WorkspaceOperationStep};
        validate_operation_id(operation_id)?;
        let Some(_lease) = self.try_acquire_execution_lease(operation_id)? else {
            return Ok(false);
        };
        let lock = self.acquire_lock(operation_id)?;
        let stored = self.read_stored_operation(&record_name(operation_id))?;
        let operation = stored.operation;
        if operation.state != WorkspaceOperationState::Planned
            || operation.step != WorkspaceOperationStep::Planned
            || operation.sequence != 0
        {
            return Ok(false);
        }
        self.root_dir
            .remove_file(record_name(operation_id))
            .map_err(io_error("state_write"))?;
        drop(lock);
        for name in [lease_name(operation_id), format!(".{operation_id}.lock")] {
            match self.root_dir.remove_file(&name) {
                Ok(()) => {}
                Err(error) if error.kind() == io::ErrorKind::NotFound => {}
                Err(error) => return Err(map_io(error, "state_write")),
            }
        }
        Ok(true)
    }

    pub fn load(&self, operation_id: &str) -> Result<WorkspaceOperation, InspectionError> {
        validate_operation_id(operation_id)?;
        let _lock = self.acquire_lock(operation_id)?;
        let name = record_name(operation_id);
        let stored = self.read_stored_operation(&name)?;
        Ok(stored.operation)
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
        let stored = self.read_stored_operation(&name)?;
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
        self.bump_mutation_generation();
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
            let stored = self.read_stored_operation(name)?;
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
            Ok(_) => {
                let mut value: serde_json::Value = read_json(&self.root_dir, &name)?;
                let migrated = migrate_legacy_teardown_receipt(&mut value, || self.load(operation_id))?;
                let receipt: TeardownReceipt = serde_json::from_value(value)
                    .map_err(|error| InspectionError::new("state_corrupt", error.to_string()))?;
                if migrated {
                    atomic_write_json(&self.root_dir, &name, &receipt)
                        .map_err(io_error("teardown_write"))?;
                }
                Ok(Some(receipt))
            }
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
        validate_resource_id(&receipt.session_id, "session")?;
        validate_endpoint_identity(&receipt.endpoint_identity)?;
        let operation = self.load(&receipt.operation_id)?;
        if !receipt_matches_operation(receipt, &operation) {
            return Err(InspectionError::new(
                "stale_identity",
                "teardown receipt does not match the exact workspace operation",
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

    /// Call only while holding the operation's journal lock.
    fn read_stored_operation(&self, name: &str) -> Result<StoredOperation, InspectionError> {
        let mut value: serde_json::Value = read_json(&self.root_dir, name)?;
        let raw_migrated = migrate_legacy_operation_json(&mut value);
        let mut stored: StoredOperation = serde_json::from_value(value)
            .map_err(|error| InspectionError::new("state_corrupt", error.to_string()))?;
        let ownership_migrated = migrate_legacy_operation(&mut stored.operation);
        if raw_migrated || ownership_migrated {
            atomic_write_json(&self.root_dir, name, &stored).map_err(io_error("state_write"))?;
            self.bump_mutation_generation();
        }
        Ok(stored)
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
    let bytes = serde_json::to_vec_pretty(value)
        .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))?;
    atomic_write_bytes(dir, name, &bytes)
}

pub(crate) fn atomic_write_bytes(dir: &Dir, name: &str, bytes: &[u8]) -> io::Result<()> {
    let tmp = format!(".{}.tmp", Uuid::new_v4());
    let mut options = OpenOptions::new();
    options
        .write(true)
        .create_new(true)
        .follow(cap_fs_ext::FollowSymlinks::No);
    let mut file = dir.open_with(&tmp, &options)?;
    #[cfg(unix)]
    rustix::fs::fchmod(&file, rustix::fs::Mode::from_raw_mode(0o600))
        .map_err(io::Error::from)?;
    file.write_all(bytes)?;
    file.sync_all()?;
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
    dir.open(".")?.sync_all()
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
    let opened_metadata = file.metadata().map_err(io_error("state_read"))?;
    if !opened_metadata.is_file() || opened_metadata.len() > max_bytes {
        return Err(InspectionError::new("unsafe_path", "opened state record is not a bounded regular file"));
    }
    let mut bytes = Vec::with_capacity(opened_metadata.len() as usize);
    (&mut file).take(max_bytes.saturating_add(1)).read_to_end(&mut bytes)
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

fn validate_endpoint_identity(value: &str) -> Result<(), InspectionError> {
    if value.is_empty() || value.len() > 4096 || value.chars().any(char::is_control) {
        return Err(InspectionError::new(
            "invalid_identity",
            "endpoint identity must be nonempty, bounded and contain no control characters",
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
    use cockpit_protocol::projects::{
        WorkspaceCheckoutOwnership, WorkspaceOperationState, WorkspaceOwnedResource,
        WorkspaceSetupMode,
    };
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
            repository: Some(cockpit_protocol::projects::RepositoryCandidate {
                repository_id: "repo-a".to_owned(),
                name: "repo".to_owned(),
                root: "/repo".to_owned(),
                checkout_path: "/repo".to_owned(),
                common_dir: "/repo/.git".to_owned(),
                branch: Some("main".to_owned()),
                is_linked_worktree: false,
                is_detached: false,
                provenance: "p".to_owned(),
            }),
            mode: WorkspaceSetupMode::Create,
            ownership: cockpit_protocol::projects::WorkspaceCheckoutOwnership::OwnedWorktree,
            branch: Some("task".to_owned()),
            base: Some("main".to_owned()),
            checkout_path: "/work/task".to_owned(),
            label: "Task".to_owned(),
            focus: false,
            artifact: None,
            linked_artifacts: Vec::new(),
            effects: vec!["create".to_owned()],
            warnings: vec![],
        }
    }

    fn teardown_receipt(id: &str, state: TeardownReceiptState) -> TeardownReceipt {
        TeardownReceipt {
            operation_id: id.to_owned(),
            workspace_id: "workspace-a".to_owned(),
            endpoint_identity: "endpoint-a".to_owned(),
            session_id: "setup-fixture".to_owned(),
            repository_key: "/repo/.git".to_owned(),
            repository_root: "/repo".to_owned(),
            checkout_path: "/work/task".to_owned(),
            state,
            updated_at: "1".to_owned(),
        }
    }

    fn omit_legacy_ownership(root: &Path, id: &str) {
        let record = root.join(format!("{id}.json"));
        let mut value: serde_json::Value =
            serde_json::from_slice(&fs::read(&record).expect("read operation record"))
                .expect("parse operation record");
        value["operation"]["plan"]
            .as_object_mut()
            .expect("plan object")
            .remove("ownership");
        fs::write(
            record,
            serde_json::to_vec(&value).expect("serialize legacy operation"),
        )
        .expect("write legacy operation");
    }

    #[test]
    fn only_a_never_started_plan_is_discarded_with_its_lock_files() {
        let root = temp_root("discard-unstarted");
        let store = ProjectStore::new(&root).expect("store");
        let unstarted = Uuid::new_v4().to_string();
        let started = Uuid::new_v4().to_string();
        store.persist_plan(plan(&unstarted)).expect("persist plan");
        store.persist_plan(plan(&started)).expect("persist plan");
        store
            .update(&started, None, |operation| {
                operation.step = cockpit_protocol::projects::WorkspaceOperationStep::Validated;
                operation.state = WorkspaceOperationState::Running;
                Ok(())
            })
            .expect("start");

        assert!(store.discard_unstarted_plan(&unstarted).expect("discard"));
        assert!(
            !store
                .discard_unstarted_plan(&started)
                .expect("keep started")
        );
        let remaining: Vec<String> = fs::read_dir(&root)
            .expect("read root")
            .map(|entry| {
                entry
                    .expect("entry")
                    .file_name()
                    .into_string()
                    .expect("utf-8")
            })
            .filter(|name| name.contains(&unstarted))
            .collect();
        assert!(remaining.is_empty(), "{remaining:?}");
        assert_eq!(
            store
                .list()
                .expect("list")
                .iter()
                .map(|operation| operation.operation_id.clone())
                .collect::<Vec<_>>(),
            vec![started]
        );
        fs::remove_dir_all(root).expect("cleanup");
    }

    #[test]
    fn legacy_create_with_exact_worktree_receipt_migrates_to_owned_worktree() {
        let root = temp_root("legacy-created-worktree");
        let store = ProjectStore::new(&root).expect("store");
        let id = Uuid::new_v4().to_string();
        store.persist_plan(plan(&id)).expect("persist plan");
        store
            .update(&id, None, |operation| {
                operation.owned_resources.push(WorkspaceOwnedResource {
                    kind: "worktree".to_owned(),
                    path: operation.plan.checkout_path.clone(),
                    created_by_operation: true,
                });
                Ok(())
            })
            .expect("record created worktree");
        omit_legacy_ownership(&root, &id);

        let migrated = store.load(&id).expect("migrate legacy Create");
        assert_eq!(
            migrated.plan.ownership,
            WorkspaceCheckoutOwnership::OwnedWorktree
        );
        assert_eq!(migrated.state, WorkspaceOperationState::Planned);
        fs::remove_dir_all(root).expect("cleanup");
    }

    #[test]
    fn legacy_create_without_worktree_receipt_requires_review() {
        let root = temp_root("legacy-unproven-create");
        let store = ProjectStore::new(&root).expect("store");
        let id = Uuid::new_v4().to_string();
        store.persist_plan(plan(&id)).expect("persist plan");
        omit_legacy_ownership(&root, &id);

        let migrated = store.load(&id).expect("read legacy Create");
        assert_eq!(migrated.state, WorkspaceOperationState::NeedsReview);
        assert!(!migrated.resume_allowed);
        assert_eq!(
            migrated.error.as_ref().map(|error| error.code.as_str()),
            Some("needs_review")
        );
        fs::remove_dir_all(root).expect("cleanup");
    }

    #[test]
    fn open_operations_always_migrate_to_borrowed_directory() {
        let root = temp_root("legacy-open");
        let store = ProjectStore::new(&root).expect("store");
        let id = Uuid::new_v4().to_string();
        store.persist_plan(plan(&id)).expect("persist plan");
        store
            .update(&id, None, |operation| {
                operation.plan.mode = WorkspaceSetupMode::Open;
                operation.plan.ownership = WorkspaceCheckoutOwnership::OwnedWorktree;
                operation.owned_resources.push(WorkspaceOwnedResource {
                    kind: "worktree".to_owned(),
                    path: operation.plan.checkout_path.clone(),
                    created_by_operation: true,
                });
                Ok(())
            })
            .expect("record legacy Open");

        let migrated = store.load(&id).expect("migrate Open");
        assert_eq!(
            migrated.plan.ownership,
            WorkspaceCheckoutOwnership::BorrowedDirectory
        );
        fs::remove_dir_all(root).expect("cleanup");
    }

    #[test]
    fn teardown_receipt_accepts_full_endpoint_identity_only_when_exactly_matching() {
        let endpoint = "unix-socket:/tmp/cockpit-fixture/config/herdr/sessions/fixture/herdr.sock:pid=939432:uid=1000:gid=1000:start=5501986";
        let root = temp_root("teardown-endpoint");
        let id = Uuid::new_v4().to_string();
        let store = ProjectStore::new(&root).expect("store");
        let mut setup = plan(&id);
        setup.endpoint_identity = endpoint.to_owned();
        store.persist_plan(setup).expect("plan");
        store.update(&id, None, |operation| {
            operation.workspace_id = Some("workspace-a".to_owned());
            Ok(())
        }).expect("workspace receipt");
        let mut receipt = teardown_receipt(&id, TeardownReceiptState::Pending);
        receipt.endpoint_identity = endpoint.to_owned();
        store.write_teardown_receipt(&receipt).expect("full matching endpoint");
        assert_eq!(store.read_teardown_receipt(&id).expect("read").expect("receipt").endpoint_identity, endpoint);
        receipt.endpoint_identity = endpoint.replace("pid=939432", "pid=939433");
        assert_eq!(store.write_teardown_receipt(&receipt).expect_err("different endpoint").code, "stale_identity");
        for invalid in ["".to_owned(), format!("{endpoint}\n"), "x".repeat(4097)] {
            receipt.endpoint_identity = invalid;
            assert_eq!(store.write_teardown_receipt(&receipt).expect_err("invalid endpoint").code, "invalid_identity");
        }
        assert_eq!(store.read_teardown_receipt(&id).expect("retained read").expect("retained receipt").endpoint_identity, endpoint);
        fs::remove_dir_all(root).expect("cleanup");
    }

    #[test]
    fn teardown_receipt_survives_reopen_and_preserves_unknown_state() {
        let root = temp_root("teardown-receipt-restart");
        let id = Uuid::new_v4().to_string();
        let mut receipt = teardown_receipt(&id, TeardownReceiptState::Pending);
        let store = ProjectStore::new(&root).expect("store");
        store.persist_plan(plan(&id)).expect("plan");
        store.update(&id, None, |operation| {
            operation.workspace_id = Some("workspace-a".to_owned());
            Ok(())
        }).expect("workspace receipt");
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

    fn legacy_companion(id: &str) -> serde_json::Value {
        serde_json::json!({
            "schema_version": 1,
            "cockpit_operation_id": id,
            "herdr_session_identity": "endpoint-a",
            "herdr_workspace_id": "workspace-a",
            "repository_key": "/repo/.git",
            "repository_root": "/repo",
            "checkout_path": "/work/task",
            "artifact": null,
            "created_at": "1",
            "updated_at": "1",
            "ownership": "cockpit"
        })
    }

    fn legacy_operation(root: &Path, id: &str, step: &str) {
        let path = root.join(record_name(id));
        let mut value: serde_json::Value =
            serde_json::from_slice(&fs::read(&path).expect("record")).expect("json");
        value["operation"]["companion_id"] = serde_json::json!(id);
        value["operation"]["step"] = serde_json::json!(step);
        let plan = &mut value["operation"]["plan"];
        plan["companion_path"] = serde_json::json!(root.join("old-companion").to_str().unwrap());
        plan["companion_id"] = serde_json::json!(id);
        plan["companion_created_by_operation"] = serde_json::json!(true);
        plan["effects"] = serde_json::json!(["create worktree", "create companion context"]);
        value["operation"]["owned_resources"].as_array_mut().unwrap().push(serde_json::json!({
            "kind": "companion",
            "path": root.join("old-companion").to_str().unwrap(),
            "created_by_operation": true
        }));
        fs::write(path, serde_json::to_vec(&value).unwrap()).expect("legacy record");
    }

    #[test]
    fn legacy_companion_journals_migrate_before_load_list_and_update() {
        for access in 0..3 {
            let root = temp_root("legacy-companion-operation");
            let store = ProjectStore::new(&root).expect("store");
            let id = Uuid::new_v4().to_string();
            store.persist_plan(plan(&id)).expect("plan");
            fs::create_dir(root.join("old-companion")).expect("old directory");
            fs::write(root.join("old-companion/notes.md"), "user notes").expect("notes");
            legacy_operation(&root, &id, if access == 1 { "associate_companion" } else { "companion_ready" });
            let record = root.join(record_name(&id));
            let mut raw: serde_json::Value = serde_json::from_slice(&fs::read(&record).unwrap()).unwrap();
            raw["operation"]["owned_resources"].as_array_mut().unwrap().push(serde_json::json!({
                "kind": "worktree", "path": "/work/task", "created_by_operation": true
            }));
            fs::write(record, serde_json::to_vec(&raw).unwrap()).unwrap();
            let operation = match access {
                0 => store.load(&id).expect("load"),
                1 => store.list().expect("list").remove(0),
                2 => store.update(&id, Some(1), |_| Ok(())).expect("update"),
                _ => unreachable!(),
            };
            assert_eq!(operation.step, cockpit_protocol::projects::WorkspaceOperationStep::WorkspaceVerified);
            assert_eq!(operation.owned_resources.len(), 1);
            assert_eq!(operation.owned_resources[0].kind, "worktree");
            assert_eq!(operation.owned_resources[0].path, "/work/task");
            assert!(operation.owned_resources[0].created_by_operation);
            assert_eq!(operation.plan.effects, ["create worktree"]);
            assert_eq!(operation.session_id, "setup-fixture");
            let persisted: serde_json::Value = serde_json::from_slice(&fs::read(root.join(record_name(&id))).unwrap()).unwrap();
            assert!(persisted["operation"].get("companion_id").is_none());
            assert!(persisted["operation"]["plan"].get("companion_path").is_none());
            assert!(persisted["operation"]["plan"].get("companion_id").is_none());
            assert!(persisted["operation"]["plan"].get("companion_created_by_operation").is_none());
            assert_eq!(fs::read_to_string(root.join("old-companion/notes.md")).unwrap(), "user notes");
            fs::remove_dir_all(root).expect("cleanup");
        }
    }

    #[test]
    fn discard_legacy_unstarted_plan_keeps_old_companion_files() {
        let root = temp_root("discard-legacy-companion");
        let store = ProjectStore::new(&root).expect("store");
        let id = Uuid::new_v4().to_string();
        store.persist_plan(plan(&id)).expect("plan");
        fs::create_dir(root.join("old-companion")).expect("old directory");
        fs::write(root.join("old-companion/notes.md"), "notes").expect("notes");
        legacy_operation(&root, &id, "planned");
        assert!(store.discard_unstarted_plan(&id).expect("discard"));
        assert_eq!(fs::read_to_string(root.join("old-companion/notes.md")).unwrap(), "notes");
        fs::remove_dir_all(root).expect("cleanup");
    }

    #[test]
    fn legacy_teardown_receipts_preserve_unknown_outcomes_and_retire_cleanup() {
        for state in ["pending", "outcome_unknown", "orphaned_companion", "completed"] {
            let root = temp_root("legacy-teardown-receipt");
            let store = ProjectStore::new(&root).expect("store");
            let id = Uuid::new_v4().to_string();
            store.persist_plan(plan(&id)).expect("plan");
            store.update(&id, None, |operation| {
                operation.workspace_id = Some("workspace-a".to_owned());
                Ok(())
            }).expect("workspace");
            fs::create_dir(root.join("old-companion")).expect("old directory");
            fs::write(root.join("old-companion/notes.md"), "user notes").expect("notes");
            let value = serde_json::json!({
                "operation_id": id,
                "workspace_id": "workspace-a",
                "endpoint_identity": "endpoint-a",
                "checkout_path": "/work/task",
                "companion": legacy_companion(&id),
                "state": state,
                "updated_at": "1"
            });
            fs::write(root.join(teardown_record_name(&id)), serde_json::to_vec(&value).unwrap()).expect("legacy receipt");
            let receipt = store.read_teardown_receipt(&id).expect("migration").unwrap();
            assert_eq!(receipt.session_id, "setup-fixture");
            assert_eq!(receipt.repository_key, "/repo/.git");
            assert_eq!(receipt.repository_root, "/repo");
            assert_eq!(receipt.state, match state {
                "pending" => TeardownReceiptState::Pending,
                "outcome_unknown" => TeardownReceiptState::OutcomeUnknown,
                _ => TeardownReceiptState::Completed,
            });
            let upgraded: serde_json::Value = serde_json::from_slice(&fs::read(root.join(teardown_record_name(&id))).unwrap()).unwrap();
            assert!(upgraded.get("companion").is_none());
            assert_eq!(fs::read_to_string(root.join("old-companion/notes.md")).unwrap(), "user notes");
            assert_eq!(ProjectStore::new(&root).unwrap().read_teardown_receipt(&id).unwrap().unwrap(), receipt);
            fs::remove_dir_all(root).expect("cleanup");
        }
    }

    #[test]
    fn orphaned_cleanup_receipt_without_operation_retires_without_filesystem_access() {
        let root = temp_root("orphaned-legacy-receipt");
        let store = ProjectStore::new(&root).expect("store");
        let id = Uuid::new_v4().to_string();
        let value = serde_json::json!({
            "operation_id": id, "workspace_id": "workspace-a",
            "endpoint_identity": "endpoint-a", "checkout_path": "/work/task",
            "companion": legacy_companion(&id),
            "state": "orphaned_companion", "updated_at": "1"
        });
        fs::write(root.join(teardown_record_name(&id)), serde_json::to_vec(&value).unwrap()).unwrap();
        let receipt = store.read_teardown_receipt(&id).expect("safe retirement").unwrap();
        assert_eq!(receipt.state, TeardownReceiptState::Completed);
        assert!(receipt.session_id.is_empty());
        fs::remove_dir_all(root).expect("cleanup");
    }

    #[test]
    fn foreign_legacy_receipt_cannot_gain_reconciliation_identity() {
        let root = temp_root("foreign-legacy-receipt");
        let store = ProjectStore::new(&root).expect("store");
        let id = Uuid::new_v4().to_string();
        store.persist_plan(plan(&id)).unwrap();
        store.update(&id, None, |operation| {
            operation.workspace_id = Some("workspace-a".to_owned());
            Ok(())
        }).unwrap();
        let mut companion = legacy_companion(&id);
        companion["herdr_session_identity"] = serde_json::json!("other-endpoint");
        let value = serde_json::json!({
            "operation_id": id, "workspace_id": "workspace-a",
            "endpoint_identity": "endpoint-a", "checkout_path": "/work/task",
            "session_id": "setup-fixture",
            "companion": companion, "state": "outcome_unknown", "updated_at": "1"
        });
        fs::write(root.join(teardown_record_name(&id)), serde_json::to_vec(&value).unwrap()).unwrap();
        let receipt = store.read_teardown_receipt(&id).expect("migration").unwrap();
        assert_eq!(receipt.state, TeardownReceiptState::OutcomeUnknown);
        assert!(receipt.session_id.is_empty());
        assert!(!receipt_matches_operation(&receipt, &store.load(&id).unwrap()));
        fs::remove_dir_all(root).expect("cleanup");
    }

    #[test]
    fn teardown_receipt_requires_exact_operation_provenance() {
        let root = temp_root("teardown-provenance");
        let store = ProjectStore::new(&root).expect("store");
        let id = Uuid::new_v4().to_string();
        store.persist_plan(plan(&id)).unwrap();
        store.update(&id, None, |operation| {
            operation.workspace_id = Some("workspace-a".to_owned());
            Ok(())
        }).unwrap();
        let valid = teardown_receipt(&id, TeardownReceiptState::Pending);
        for mutation in 0..6 {
            let mut receipt = valid.clone();
            match mutation {
                0 => receipt.session_id = "other".to_owned(),
                1 => receipt.workspace_id = "other".to_owned(),
                2 => receipt.endpoint_identity = "other".to_owned(),
                3 => receipt.checkout_path = "/other".to_owned(),
                4 => receipt.repository_key = "/other/.git".to_owned(),
                5 => receipt.repository_root = "/other".to_owned(),
                _ => unreachable!(),
            }
            assert_eq!(store.write_teardown_receipt(&receipt).unwrap_err().code, "stale_identity");
        }
        store.write_teardown_receipt(&valid).expect("valid receipt");
        store.update(&id, None, |operation| {
            operation.plan.session_id = "different-plan-session".to_owned();
            Ok(())
        }).unwrap();
        assert_eq!(store.write_teardown_receipt(&valid).unwrap_err().code, "stale_identity");
        store.update(&id, None, |operation| {
            operation.plan.session_id = valid.session_id.clone();
            operation.session_id = "different-operation-session".to_owned();
            Ok(())
        }).unwrap();
        assert_eq!(store.write_teardown_receipt(&valid).unwrap_err().code, "stale_identity");
        fs::remove_dir_all(root).expect("cleanup");
    }

    #[cfg(unix)]
    #[test]
    fn symlinked_teardown_receipt_is_rejected_without_touching_target() {
        use std::os::unix::fs::symlink;
        let root = temp_root("teardown-symlink");
        let store = ProjectStore::new(&root).expect("store");
        let id = Uuid::new_v4().to_string();
        let target = root.join("user-notes");
        fs::write(&target, "notes").unwrap();
        symlink(&target, root.join(teardown_record_name(&id))).unwrap();
        assert_eq!(store.read_teardown_receipt(&id).unwrap_err().code, "unsafe_path");
        assert_eq!(fs::read_to_string(target).unwrap(), "notes");
        fs::remove_dir_all(root).expect("cleanup");
    }

    #[test]
    fn oversized_journal_is_rejected_before_migration() {
        let root = temp_root("oversized-journal");
        let store = ProjectStore::new(&root).expect("store");
        let id = Uuid::new_v4().to_string();
        fs::write(root.join(record_name(&id)), vec![b' '; MAX_RECORD_BYTES as usize + 1]).unwrap();
        assert_eq!(store.load(&id).unwrap_err().code, "unsafe_path");
        fs::remove_dir_all(root).expect("cleanup");
    }

}
