use std::{
    cell::Cell,
    io::{self, Write},
    path::{Path, PathBuf},
};

use cap_fs_ext::DirExt;
use cap_std::fs::Dir;
use cockpit_protocol::orchestration::{
    IntentState, Message, OperatorOrigin, Run, TaskIntent, TaskOwnership,
};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use super::tasks_md::{TaskDocument, read_locked_document};
use crate::{
    InspectionError,
    project_store::{
        ExecutionLease, LockGuard, ProjectStore, atomic_write_bytes,
    },
};

pub(crate) const MAX_STATE_BYTES: usize = 8 * 1024 * 1024;
pub(crate) const SCHEMA: u32 = 4;

/// Canonical machine records and temporary transaction journals. Task content
/// lives in tasks/*.md; assignment payloads exist only until durable delivery.
/// Runtime observations are never persisted here.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct OrchestrationState {
    pub schema: u32,
    pub revision: u64,
    pub runs: Vec<Run>,
    pub messages: Vec<Message>,
    pub task_ownership: Vec<TaskOwnership>,
    pub inbox_hints: Vec<InboxHint>,
    pub task_intents: Vec<TaskIntent>,
    pub assignment_intents: Vec<AssignmentIntent>,
    pub scope_intents: Vec<super::scope_intents::ScopeIntent>,
    pub credentials: Vec<super::credentials::JobCredentialRecord>,
    pub checkout_claims: Vec<super::claims::CheckoutClaim>,
    pub operations: Vec<DurableOperationRecord>,
    #[serde(default)]
    pub resource_requests: Vec<cockpit_protocol::orchestration::ResourceRequestRecord>,
    #[serde(default)]
    pub resource_sequence: u64,
    #[serde(default)]
    pub owned_resources: Vec<cockpit_protocol::orchestration::OwnedResourceRecord>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum HintState { Pending, Submitting, Sent, Unknown, Deferred }

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct InboxHint {
    pub run_id: String,
    pub attachment_id: String,
    pub attachment_epoch: u64,
    pub through_seq: u64,
    pub attempt_id: String,
    pub bootstrap: bool,
    pub state: HintState,
    pub cause: Option<String>,
    pub updated_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct AssignmentIntent {
    pub root_id: String,
    pub task_id: String,
    pub request_hash: String,
    pub title: String,
    pub body: String,
    pub origin: OperatorOrigin,
    pub state: IntentState,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum DurableOperationKind { SupervisorStart, RunResume, RunAttach, RunRegister, TaskUnassign, WorkerStart, TaskScopeUpdate, IntegrationRecord, Report, MessageSend, ResourceRequest, ResourceRegister, CleanupFinish }
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct DurableOperationRecord {
    pub operation_id: String,
    pub actor_run_id: Option<String>,
    pub principal_key: String,
    pub root_id: String,
    pub kind: DurableOperationKind,
    pub request_digest: String,
    pub run_id: Option<String>,
    pub resume_operation_id: Option<String>,
    pub result: Option<cockpit_protocol::orchestration::OrchestrationActionResult>,
    pub effect_receipts: Vec<DurableEffectReceipt>,
    pub created_at: String,
    pub updated_at: String,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "effect", rename_all = "snake_case", deny_unknown_fields)]
pub(crate) enum DurableEffectReceipt {
    Launch { attachment_id: String, attachment_epoch: u64, dispatch: cockpit_protocol::orchestration::DispatchState },
    Setup { setup: cockpit_protocol::orchestration::SetupSummary },
    Stop { retirement: cockpit_protocol::orchestration::RunRetirement },
}

impl Default for OrchestrationState {
    fn default() -> Self {
        Self {
            schema: SCHEMA,
            revision: 0,
            runs: Vec::new(),
            messages: Vec::new(),
            task_ownership: Vec::new(),
            inbox_hints: Vec::new(),
            task_intents: Vec::new(),
            assignment_intents: Vec::new(),
            scope_intents: Vec::new(),
            credentials: Vec::new(),
            checkout_claims: Vec::new(),
            operations: Vec::new(),
            resource_requests: Vec::new(),
            resource_sequence: 0,
            owned_resources: Vec::new(),
        }
    }
}

#[derive(Debug)]
pub(crate) struct OrchestrationStore {
    base: PathBuf,
    identity: String,
    store: ProjectStore,
    tasks: Dir,
}

impl OrchestrationStore {
    pub(crate) fn open(state_root: &Path) -> Result<Self, InspectionError> {
        let base = if state_root.is_absolute() {
            state_root.join("orchestration")
        } else {
            std::env::current_dir()
                .map_err(|e| io_error("orchestration_state_open", e))?
                .join(state_root)
                .join("orchestration")
        };
        let store = ProjectStore::new(&base)?;
        let base = std::fs::canonicalize(&base)
            .map_err(|error| io_error("orchestration_state_open", error))?;
        let mut hash = Sha256::new();
        hash.update(b"cockpit-state-store-v1\0");
        hash.update(base.as_os_str().as_encoded_bytes());
        let identity = format!("{:x}", hash.finalize());
        match store.state_dir().create_dir("tasks") {
            Ok(()) => {}
            Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {}
            Err(error) => return Err(io_error("orchestration_state_open", error)),
        }
        let tasks = store
            .state_dir()
            .open_dir_nofollow("tasks")
            .map_err(|error| io_error("unsafe_path", error))?;
        Ok(Self { base, identity, store, tasks })
    }

    pub(crate) fn base(&self) -> &Path {
        &self.base
    }
    pub(crate) fn identity(&self) -> &str {
        &self.identity
    }
    #[cfg(test)]
    pub(crate) fn tasks_dir(&self) -> &Dir {
        &self.tasks
    }

    pub(crate) fn lock(&self) -> Result<LockedStore<'_>, InspectionError> {
        Ok(LockedStore {
            store: self,
            _guard: self
                .store
                .acquire_named_lock(".state.lock", "orchestration_state_lock")?,
            revision: Cell::new(None),
        })
    }

    pub(crate) fn try_acquire_execution_lease(
        &self,
        run_id: &str,
    ) -> Result<Option<ExecutionLease>, InspectionError> {
        let id = super::tasks_md::validate_uuid(run_id)?;
        self.store
            .try_acquire_named_execution_lease(&format!(".run-{id}.exec.lock"))
    }

    /// Metadata invalidation, not a CAS token. Exact content CAS is performed
    /// by TaskDocument against the complete document bytes.
    pub(crate) fn tasks_token(&self) -> Result<String, InspectionError> {
        let mut records = Vec::new();
        for entry in self
            .tasks
            .entries()
            .map_err(|e| io_error("tasks_read", e))?
        {
            let name = entry.map_err(|e| io_error("tasks_read", e))?.file_name();
            let Some(name) = name.to_str().filter(|name| name.ends_with(".md")) else {
                continue;
            };
            let metadata = match self.tasks.symlink_metadata(name) {
                Ok(metadata) => metadata,
                Err(error) if error.kind() == io::ErrorKind::NotFound => continue,
                Err(error) => return Err(io_error("tasks_read", error)),
            };
            if metadata.file_type().is_symlink() || !metadata.is_file() {
                return Err(InspectionError::new(
                    "unsafe_path",
                    "task document is not a regular file",
                ));
            }
            #[cfg(unix)]
            let inode = cap_fs_ext::MetadataExt::ino(&metadata);
            #[cfg(not(unix))]
            let inode = 0u64;
            records.push((
                name.to_owned(),
                metadata.len(),
                metadata.modified().map_err(|e| io_error("tasks_read", e))?,
                inode,
            ));
        }
        records.sort_by(|a, b| a.0.cmp(&b.0));
        let mut hash = Sha256::new();
        for (name, len, modified, inode) in records {
            hash.update((name.len() as u64).to_le_bytes());
            hash.update(name.as_bytes());
            hash.update(len.to_le_bytes());
            hash.update(format!("{modified:?}").as_bytes());
            hash.update(inode.to_le_bytes());
        }
        Ok(format!("{:x}", hash.finalize()))
    }

    pub(crate) fn tasks_token_for_root(&self, root_id: &str) -> Result<String, InspectionError> {
        let root_id = super::tasks_md::validate_uuid(root_id)?;
        let name = format!("{root_id}.md");
        let mut hash = Sha256::new();
        match self.tasks.symlink_metadata(&name) {
            Ok(metadata) => {
                if metadata.file_type().is_symlink() || !metadata.is_file() {
                    return Err(InspectionError::new("unsafe_path", "task document is not a regular file"));
                }
                #[cfg(unix)]
                let inode = cap_fs_ext::MetadataExt::ino(&metadata);
                #[cfg(not(unix))]
                let inode = 0u64;
                hash.update(name.as_bytes());
                hash.update(metadata.len().to_le_bytes());
                hash.update(format!("{:?}", metadata.modified().map_err(|e| io_error("tasks_read", e))?).as_bytes());
                hash.update(inode.to_le_bytes());
            }
            Err(error) if error.kind() == io::ErrorKind::NotFound => {}
            Err(error) => return Err(io_error("tasks_read", error)),
        }
        Ok(format!("{:x}", hash.finalize()))
    }
}

/// One named lock covers both the state document and canonical task mutations.
/// Borrowed TaskDocument values cannot outlive this guard.
pub(crate) struct LockedStore<'a> {
    store: &'a OrchestrationStore,
    _guard: LockGuard,
    revision: Cell<Option<(u32, u64)>>,
}

#[derive(Deserialize)]
struct StateHeader { schema: u32, revision: u64 }

pub(crate) fn encode_state(state: &OrchestrationState) -> Result<Vec<u8>, InspectionError> {
    validate_counters(state)?;
    let mut encoded = BoundedStateBytes { bytes: Vec::new(), full: false };
    serde_json::to_writer(&mut encoded, state).map_err(|error| {
        if encoded.full {
            InspectionError::new("orchestration_state_full", "orchestration state exceeds 8 MiB; durable records cannot be discarded")
        } else {
            InspectionError::new("orchestration_state_write", error.to_string())
        }
    })?;
    Ok(encoded.bytes)
}

impl LockedStore<'_> {
    pub(crate) fn state_dir(&self) -> &Dir { self.store.store.state_dir() }
    pub(crate) fn exact_bytes(&self) -> Result<Vec<u8>, InspectionError> {
        crate::project_store::read_bytes_bounded(self.state_dir(), "state.json", MAX_STATE_BYTES as u64)
    }
    pub(crate) fn read(&self) -> Result<OrchestrationState, InspectionError> {
        let mut source_schema = SCHEMA;
        let state = match self.state_dir().symlink_metadata("state.json") {
            Ok(_) => {
                let bytes = self.exact_bytes()?;
                let header: StateHeader = serde_json::from_slice(&bytes)
                    .map_err(|_| InspectionError::new("state_corrupt", "invalid orchestration state header"))?;
                source_schema = header.schema;
                match header.schema {
                    SCHEMA => serde_json::from_slice::<OrchestrationState>(&bytes)
                        .map_err(|e| InspectionError::new("state_corrupt", e.to_string()))?,
                    _ => return Err(InspectionError::new("orchestration_state_incompatible", "Orchestration state uses an incompatible schema; select a fresh state root. No migration or reset was performed")),
                }
            }
            Err(error) if error.kind() == io::ErrorKind::NotFound => OrchestrationState::default(),
            Err(error) => return Err(io_error("orchestration_state_read", error)),
        };
        if state.schema != SCHEMA {
            return Err(InspectionError::new(
                "state_corrupt",
                "unsupported orchestration state schema",
            ));
        }
        validate_counters(&state)?;
        self.revision.set(Some((source_schema, state.revision)));
        Ok(state)
    }

    pub(crate) fn save(&self, state: &mut OrchestrationState) -> Result<u64, InspectionError> {
        let (source_schema, current) = match self.revision.get() {
            Some(revision) => revision,
            None => {
                self.read()?;
                self.revision.get().expect("successful read sets the baseline")
            }
        };
        // Recheck the exact schema/revision read under this lock before ordinary publication.
        if self.state_dir().symlink_metadata("state.json").is_ok() {
            let header: StateHeader = serde_json::from_slice(&self.exact_bytes()?)
                .map_err(|_| InspectionError::new("state_corrupt", "invalid orchestration state header"))?;
            if header.schema != source_schema {
                return Err(InspectionError::new("orchestration_revision_conflict", "on-disk state schema changed"));
            }
            if header.revision != current {
                return Err(InspectionError::new("orchestration_revision_conflict", "on-disk state revision changed"));
            }
        }
        if state.schema != SCHEMA {
            return Err(InspectionError::new(
                "state_corrupt",
                "unsupported orchestration state schema",
            ));
        }
        validate_counters(state)?;
        if state.revision != current {
            return Err(InspectionError::new(
                "orchestration_revision_conflict",
                "state revision changed",
            ));
        }
        let next = current
            .checked_add(1)
            .filter(|next| *next <= super::MAX_SAFE_COUNTER)
            .ok_or_else(|| {
                InspectionError::new(
                    "orchestration_state_full",
                    "JSON-safe state revision exhausted",
                )
            })?;
        state.revision = next;
        let result = encode_state(state).and_then(|encoded| {
            atomic_write_bytes(self.state_dir(), "state.json", &encoded)
                .map_err(|error| io_error("orchestration_state_write", error))
        });
        if let Err(error) = result {
            state.revision = current;
            return Err(error);
        }
        self.revision.set(Some((SCHEMA, next)));
        Ok(next)
    }

    pub(crate) fn tasks(&self, root_id: &str) -> Result<TaskDocument<'_>, InspectionError> {
        read_locked_document(&self.store.tasks, root_id, &self._guard)
    }
}

struct BoundedStateBytes {
    bytes: Vec<u8>,
    full: bool,
}
impl Write for BoundedStateBytes {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        if bytes.len() > MAX_STATE_BYTES.saturating_sub(self.bytes.len()) {
            self.full = true;
            return Err(io::Error::new(
                io::ErrorKind::FileTooLarge,
                "state capacity exceeded",
            ));
        }
        self.bytes.extend_from_slice(bytes);
        Ok(bytes.len())
    }
    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

fn io_error(code: &str, error: io::Error) -> InspectionError {
    InspectionError::new(code, error.to_string())
}

fn validate_counters(state: &OrchestrationState) -> Result<(), InspectionError> {
    if state.revision > super::MAX_SAFE_COUNTER
        || state.resource_sequence > super::MAX_SAFE_COUNTER
        || state.resource_requests.iter().any(|r| r.sequence > super::MAX_SAFE_COUNTER
            || r.holder.attachment_epoch > super::MAX_SAFE_COUNTER
            || r.build_processes.iter().any(|p| p.process.start_ticks > super::MAX_SAFE_COUNTER))
        || state.runs.iter().any(|run| run.attachment.as_ref().is_some_and(|a| a.epoch > super::MAX_SAFE_COUNTER)
            || run.resume.as_ref().is_some_and(|r| r.previous_attachment_epoch.is_some_and(|e| e > super::MAX_SAFE_COUNTER)
                || r.replacement_attachment_epoch.is_some_and(|e| e > super::MAX_SAFE_COUNTER)))
        || state.runs.iter().any(|run| run.assignment_generation.is_some_and(|value| value == 0 || value > super::MAX_SAFE_COUNTER))
        || state.task_ownership.iter().any(|ownership| ownership.generation > super::MAX_SAFE_COUNTER)
        || state.inbox_hints.iter().any(|hint| hint.through_seq > super::MAX_SAFE_COUNTER || hint.attachment_epoch > super::MAX_SAFE_COUNTER)
        || state.credentials.iter().any(|c| c.attachment_epoch > super::MAX_SAFE_COUNTER)
        || state.checkout_claims.iter().any(|c| c.revision > super::MAX_SAFE_COUNTER
            || c.holder.as_ref().is_some_and(|h| h.attachment_epoch > super::MAX_SAFE_COUNTER))
        || state
            .messages
            .iter()
            .any(|message| message.seq > super::MAX_SAFE_COUNTER)
        || state.credentials.iter().any(|credential| credential.assignment_generation.is_some_and(|value| value == 0 || value > super::MAX_SAFE_COUNTER))
    {
        return Err(InspectionError::new(
            "state_corrupt",
            "orchestration counters exceed JSON-safe integer precision",
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use cockpit_protocol::orchestration::{ActorRef, DeliveryStage, IntentState, MessageKind};
    use uuid::Uuid;

    struct Fixture(PathBuf);
    impl Fixture {
        fn new() -> Self {
            Self(
                std::env::temp_dir()
                    .join(format!("cockpit-orchestration-store-{}", Uuid::new_v4())),
            )
        }
        fn open(&self) -> OrchestrationStore {
            OrchestrationStore::open(&self.0).unwrap()
        }
    }
    impl Drop for Fixture {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn store_identity_is_stable_before_roots_and_distinct_between_directories() {
        let fixture = Fixture::new();
        let store = fixture.open();
        assert_eq!(store.identity().len(), 64);
        assert_eq!(store.identity(), fixture.open().identity());
        assert_eq!(store.identity(), OrchestrationStore::open(&fixture.0.join(".")).unwrap().identity());
        assert_ne!(store.identity(), Fixture::new().open().identity());
        assert!(store.lock().unwrap().read().unwrap().runs.is_empty());
        assert!(!store.identity().contains(fixture.0.to_str().unwrap()));
    }

    fn message(text: String) -> Message {
        Message {
            message_id: Uuid::new_v4().to_string(),
            to_run_id: Uuid::new_v4().to_string(),
            seq: 1,
            from: ActorRef::Operator,
            kind: MessageKind::Instruction,
            text,
            in_reply_to: None,
            report: None,
            stale: false,
            escalated_from: None,
            stage: DeliveryStage::Acked,
            created_at: "2000-01-01T00:00:00Z".into(),
            acked_at: Some("2000-01-01T00:00:00Z".into()),
        }
    }

    #[test]
    fn reopening_retains_acked_messages_and_pending_accept_intents() {
        let fixture = Fixture::new();
        let store = fixture.open();
        let lock = store.lock().unwrap();
        let mut state = lock.read().unwrap();
        state
            .messages
            .push(message("never prune this receipt".into()));
        state.task_intents.push(TaskIntent {
            intent_id: Uuid::new_v4().to_string(),
            root_id: Uuid::new_v4().to_string(),
            task_id: Uuid::new_v4().to_string(),
            run_id: Uuid::new_v4().to_string(),
            expected_task_revision: "expected".into(),
            state: IntentState::Pending,
            origin: None,
            supervisor_run_id: None,
            conversation: None,
            result_message_id: None,
        });
        assert_eq!(lock.save(&mut state).unwrap(), 1);
        drop(lock);
        drop(store);
        let reopened = fixture.open();
        let lock = reopened.lock().unwrap();
        let state = lock.read().unwrap();
        assert_eq!(state.revision, 1);
        assert_eq!(state.messages.len(), 1);
        assert_eq!(state.task_intents.len(), 1);
        assert_eq!(state.messages[0].text, "never prune this receipt");
    }

    #[test]
    fn capacity_failure_keeps_prior_document_and_revision() {
        let fixture = Fixture::new();
        let store = fixture.open();
        let lock = store.lock().unwrap();
        let mut state = lock.read().unwrap();
        lock.save(&mut state).unwrap();
        let before = store.store.state_dir().read("state.json").unwrap();
        state.messages.push(message("x".repeat(MAX_STATE_BYTES)));
        assert_eq!(
            lock.save(&mut state).unwrap_err().code,
            "orchestration_state_full"
        );
        assert_eq!(state.revision, 1);
        assert_eq!(store.store.state_dir().read("state.json").unwrap(), before);
    }

    #[test]
    fn stale_state_cannot_overwrite_a_committed_revision() {
        let fixture = Fixture::new();
        let store = fixture.open();
        let lock = store.lock().unwrap();
        let mut state = lock.read().unwrap();
        let mut stale = state.clone();
        lock.save(&mut state).unwrap();
        assert_eq!(
            lock.save(&mut stale).unwrap_err().code,
            "orchestration_revision_conflict"
        );
    }

    #[test]
    fn obsolete_schema_is_rejected_without_discarding_durable_bytes() {
        let fixture = Fixture::new();
        let store = fixture.open();
        let lock = store.lock().unwrap();
        let mut state = lock.read().unwrap();
        lock.save(&mut state).unwrap();
        let source = serde_json::to_vec(&serde_json::json!({
            "schema": 1, "revision": state.revision, "runs": [], "messages": [],
            "subagents": [], "task_intents": [], "assignment_intents": [],
        })).unwrap();
        lock.state_dir().write("state.json", &source).unwrap();
        assert_eq!(lock.save(&mut state).unwrap_err().code, "orchestration_revision_conflict");
        assert_eq!(lock.exact_bytes().unwrap(), source);
        assert_eq!(lock.read().unwrap_err().code, "orchestration_state_incompatible");
        assert_eq!(lock.exact_bytes().unwrap(), source);
        drop(lock);
        drop(store);
        let reopened = fixture.open();
        let lock = reopened.lock().unwrap();
        assert_eq!(lock.read().unwrap_err().code, "orchestration_state_incompatible");
        assert_eq!(lock.exact_bytes().unwrap(), source);
        let mut fresh = OrchestrationState::default();
        assert_eq!(lock.save(&mut fresh).unwrap_err().code, "orchestration_state_incompatible");
        assert_eq!(fresh.revision, 0);
        assert_eq!(lock.exact_bytes().unwrap(), source);
    }

    #[test]
    fn json_safe_revision_exhaustion_preserves_the_document() {
        let fixture = Fixture::new();
        let store = fixture.open();
        let mut state = OrchestrationState::default();
        state.revision = super::super::MAX_SAFE_COUNTER;
        crate::project_store::atomic_write_json(store.store.state_dir(), "state.json", &state)
            .unwrap();
        let before = store.store.state_dir().read("state.json").unwrap();
        let lock = store.lock().unwrap();
        let mut current = lock.read().unwrap();
        assert_eq!(
            lock.save(&mut current).unwrap_err().code,
            "orchestration_state_full"
        );
        assert_eq!(current.revision, super::super::MAX_SAFE_COUNTER);
        assert_eq!(store.store.state_dir().read("state.json").unwrap(), before);
    }

    #[test]
    fn run_execution_lease_excludes_recovery_and_releases_on_drop() {
        let fixture = Fixture::new();
        let first = fixture.open();
        let second = fixture.open();
        let id = Uuid::new_v4().to_string();
        let lease = first.try_acquire_execution_lease(&id).unwrap().unwrap();
        assert!(second.try_acquire_execution_lease(&id).unwrap().is_none());
        assert!(
            first
                .store
                .state_dir()
                .symlink_metadata(format!(".run-{id}.exec.lock"))
                .unwrap()
                .is_file()
        );
        drop(lease);
        assert!(second.try_acquire_execution_lease(&id).unwrap().is_some());
    }

    #[test]
    fn oversized_state_reads_fail_before_decoding() {
        let fixture = Fixture::new();
        let store = fixture.open();
        store
            .store
            .state_dir()
            .write("state.json", vec![b' '; MAX_STATE_BYTES + 1])
            .unwrap();
        assert_eq!(
            store.lock().unwrap().read().unwrap_err().code,
            "unsafe_path"
        );
    }

    #[cfg(unix)]
    #[test]
    fn state_symlink_and_task_directory_symlink_fail_closed() {
        use std::os::unix::fs::symlink;
        let fixture = Fixture::new();
        let store = fixture.open();
        let outside = fixture.0.join("outside");
        std::fs::write(&outside, b"{}").unwrap();
        symlink(&outside, store.base().join("state.json")).unwrap();
        assert_eq!(
            store.lock().unwrap().read().unwrap_err().code,
            "unsafe_path"
        );
        std::fs::remove_dir(store.base().join("tasks")).unwrap();
        symlink(&fixture.0, store.base().join("tasks")).unwrap();
        assert!(OrchestrationStore::open(&fixture.0).is_err());
    }
}
