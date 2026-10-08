use std::{
    cell::Cell,
    io::{self, Write},
    path::{Path, PathBuf},
};

use cap_fs_ext::DirExt;
use cap_std::fs::Dir;
use cockpit_protocol::orchestration::{
    IntentState, Message, OperatorOrigin, Run, Subagent, TaskIntent,
};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use super::tasks_md::{TaskDocument, read_locked_document};
use crate::{
    InspectionError,
    project_store::{
        ExecutionLease, LockGuard, ProjectStore, atomic_write_bytes, read_json_bounded,
    },
};

pub(crate) const MAX_STATE_BYTES: usize = 8 * 1024 * 1024;
const SCHEMA: u32 = 1;

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
    pub subagents: Vec<Subagent>,
    pub task_intents: Vec<TaskIntent>,
    #[serde(default)]
    pub assignment_intents: Vec<AssignmentIntent>,
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

impl Default for OrchestrationState {
    fn default() -> Self {
        Self {
            schema: SCHEMA,
            revision: 0,
            runs: Vec::new(),
            messages: Vec::new(),
            subagents: Vec::new(),
            task_intents: Vec::new(),
            assignment_intents: Vec::new(),
        }
    }
}

#[derive(Debug)]
pub(crate) struct OrchestrationStore {
    base: PathBuf,
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
        match store.state_dir().create_dir("tasks") {
            Ok(()) => {}
            Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {}
            Err(error) => return Err(io_error("orchestration_state_open", error)),
        }
        let tasks = store
            .state_dir()
            .open_dir_nofollow("tasks")
            .map_err(|error| io_error("unsafe_path", error))?;
        Ok(Self { base, store, tasks })
    }

    pub(crate) fn base(&self) -> &Path {
        &self.base
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
}

/// One named lock covers both the state document and canonical task mutations.
/// Borrowed TaskDocument values cannot outlive this guard.
pub(crate) struct LockedStore<'a> {
    store: &'a OrchestrationStore,
    _guard: LockGuard,
    revision: Cell<Option<u64>>,
}

impl LockedStore<'_> {
    pub(crate) fn read(&self) -> Result<OrchestrationState, InspectionError> {
        let state = match self.store.store.state_dir().symlink_metadata("state.json") {
            Ok(_) => read_json_bounded::<OrchestrationState>(
                self.store.store.state_dir(),
                "state.json",
                MAX_STATE_BYTES as u64,
            )?,
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
        self.revision.set(Some(state.revision));
        Ok(state)
    }

    pub(crate) fn save(&self, state: &mut OrchestrationState) -> Result<u64, InspectionError> {
        let current = match self.revision.get() {
            Some(revision) => revision,
            None => self.read()?.revision,
        };
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
        let mut encoded = BoundedStateBytes {
            bytes: Vec::new(),
            full: false,
        };
        let result = serde_json::to_writer(&mut encoded, &*state)
            .map_err(|error| {
                if encoded.full {
                    InspectionError::new(
                        "orchestration_state_full",
                        "orchestration state exceeds 8 MiB; durable records cannot be discarded",
                    )
                } else {
                    InspectionError::new("orchestration_state_write", error.to_string())
                }
            })
            .and_then(|()| {
                atomic_write_bytes(self.store.store.state_dir(), "state.json", &encoded.bytes)
                    .map_err(|error| io_error("orchestration_state_write", error))
            });
        if let Err(error) = result {
            state.revision = current;
            return Err(error);
        }
        self.revision.set(Some(next));
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
        || state
            .messages
            .iter()
            .any(|message| message.seq > super::MAX_SAFE_COUNTER)
        || state.subagents.iter().any(|agent| {
            agent
                .last_control
                .as_ref()
                .is_some_and(|control| control.seq > super::MAX_SAFE_COUNTER)
        })
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
            from_subagent_id: None,
            stage: DeliveryStage::Acked,
            woken_omp_session: None,
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
            omp_session_id: None,
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
    fn schema_one_without_assignment_journal_keeps_legacy_acceptance() {
        let fixture = Fixture::new();
        let store = fixture.open();
        let root_id = Uuid::new_v4().to_string();
        let task_id = Uuid::new_v4().to_string();
        let run_id = Uuid::new_v4().to_string();
        let legacy = serde_json::json!({
            "schema": 1, "revision": 3, "runs": [], "messages": [], "subagents": [],
            "task_intents": [{
                "intent_id": "legacy-intent", "root_id": root_id, "task_id": task_id,
                "run_id": run_id, "expected_task_revision": "legacy-revision", "state": "pending"
            }]
        });
        store
            .store
            .state_dir()
            .write("state.json", serde_json::to_vec(&legacy).unwrap())
            .unwrap();
        let locked = store.lock().unwrap();
        let mut state = locked.read().unwrap();
        assert!(state.assignment_intents.is_empty());
        assert!(state.task_intents[0].origin.is_none());
        assert!(state.task_intents[0].supervisor_run_id.is_none());
        assert!(state.task_intents[0].omp_session_id.is_none());
        assert!(state.task_intents[0].result_message_id.is_none());
        locked.save(&mut state).unwrap();
        let saved = locked.read().unwrap();
        assert_eq!(saved.schema, 1);
        assert_eq!(saved.task_intents[0].intent_id, "legacy-intent");
        assert_eq!(saved.task_intents[0].root_id, root_id);
        assert_eq!(saved.task_intents[0].task_id, task_id);
        assert_eq!(saved.task_intents[0].run_id, run_id);
        assert_eq!(
            saved.task_intents[0].expected_task_revision,
            "legacy-revision"
        );
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
