use cockpit_protocol::orchestration::{
    ActorRef, IntentState, MessageKind, OperatorOrigin, OrchestrationActionResult, RunKind,
    RunStage, Task, TaskAssignmentIntent,
};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use super::{
    error, messages,
    store::{AssignmentIntent, LockedStore, OrchestrationState},
    tasks_md::{validate_authoring_creation, validate_uuid},
};
use crate::InspectionError;

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct AssignmentPointer {
    event: String,
    root_id: String,
    task_id: String,
    task_revision: String,
    request_hash: String,
    origin: OperatorOrigin,
}

fn conflict() -> InspectionError {
    error(
        "task_assignment_conflict",
        "Task is not assigned: inspect the canonical task and resolve this assignment explicitly",
    )
}

fn validate_root(
    state: &OrchestrationState,
    session_id: &str,
    root_id: &str,
    allow_closed: bool,
) -> Result<(), InspectionError> {
    let root = state
        .runs
        .iter()
        .find(|run| run.run_id == root_id)
        .ok_or_else(|| error("run_not_found", "Assignment root does not exist"))?;
    if root.session_id != session_id
        || root.root_id != root.run_id
        || root.parent_run_id.is_some()
        || !matches!(root.kind, RunKind::Supervisor | RunKind::Adopted)
        || (!allow_closed && root.stage == RunStage::Closed)
    {
        return Err(error(
            "task_assignment_root",
            "Assignment requires a same-session top-level root; only abandonment permits closed tracking",
        ));
    }
    Ok(())
}

fn request_hash(root_id: &str, title: &str, body: &str) -> String {
    let mut digest = Sha256::new();
    for value in [root_id, title, body] {
        digest.update((value.len() as u64).to_le_bytes());
        digest.update(value.as_bytes());
    }
    format!("{:x}", digest.finalize())
}

/// The actor is checked by core; origin is the actual operator transport.
/// Both journal and final pointer saves are owned here, never by the caller.
#[allow(clippy::too_many_arguments)]
pub(super) fn assign(
    locked: &LockedStore<'_>,
    state: &mut OrchestrationState,
    session_id: &str,
    origin: OperatorOrigin,
    root_id: &str,
    task_id: &str,
    title: &str,
    description: &str,
) -> Result<OrchestrationActionResult, InspectionError> {
    let root_id = validate_uuid(root_id)?.to_string();
    let task_id = validate_uuid(task_id)?.to_string();
    validate_root(state, session_id, &root_id, false)?;
    validate_authoring_creation(title, description)?;
    if title.len() > 256 {
        return Err(error(
            "invalid_task",
            "Assignment title must not exceed 256 bytes",
        ));
    }
    let hash = request_hash(&root_id, title, description);
    let message_id = format!("assign-{task_id}");
    if let Some(message) = state.messages.iter().find(|message| {
        message.message_id == message_id && matches!(message.from, ActorRef::Operator)
    }) {
        let pointer: AssignmentPointer =
            serde_json::from_str(&message.text).map_err(|_| conflict())?;
        if message.to_run_id != root_id
            || message.kind != MessageKind::Instruction
            || pointer.event != "task_assigned"
            || pointer.root_id != root_id
            || pointer.task_id != task_id
            || pointer.request_hash != hash
        {
            return Err(conflict());
        }
        return Ok(OrchestrationActionResult::TaskAssigned {
            task: locked.tasks(&root_id)?.task(&task_id)?.clone(),
            to_run_id: root_id,
            seq: message.seq,
            duplicate: true,
        });
    }
    let (index, duplicate) = match state
        .assignment_intents
        .iter()
        .position(|intent| intent.task_id == task_id)
    {
        Some(index) => {
            let intent = &state.assignment_intents[index];
            if intent.root_id != root_id
                || intent.request_hash != hash
                || intent.state == IntentState::Conflict
            {
                return Err(conflict());
            }
            (index, true)
        }
        None => {
            let index = state.assignment_intents.len();
            state.assignment_intents.push(AssignmentIntent {
                root_id,
                task_id,
                request_hash: hash,
                title: title.to_owned(),
                body: description.to_owned(),
                origin,
                state: IntentState::Pending,
            });
            // This is the only persistent body copy, written before Markdown.
            locked.save(state)?;
            (index, false)
        }
    };
    resume(locked, state, index, duplicate)?.ok_or_else(conflict)
}

fn finalize(
    locked: &LockedStore<'_>,
    state: &mut OrchestrationState,
    index: usize,
    task: Task,
    duplicate: bool,
) -> Result<OrchestrationActionResult, InspectionError> {
    let intent = &state.assignment_intents[index];
    let pointer = AssignmentPointer {
        event: "task_assigned".into(),
        root_id: intent.root_id.clone(),
        task_id: intent.task_id.clone(),
        task_revision: task.task_revision.clone(),
        request_hash: intent.request_hash.clone(),
        origin: intent.origin,
    };
    let text = serde_json::to_string(&pointer)
        .map_err(|failure| error("task_assignment_write", failure.to_string()))?;
    let result = messages::append(state, messages::AppendMessage { from: ActorRef::Operator, to_run_id: &pointer.root_id, message_id: &format!("assign-{}", pointer.task_id), kind: MessageKind::Instruction, text: &text, in_reply_to: None, report: None, stale: false, from_subagent_id: None, escalated_from: None })?;
    let OrchestrationActionResult::Message {
        seq,
        duplicate: message_duplicate,
        ..
    } = result
    else {
        unreachable!("append returns the stored message receipt")
    };
    state.assignment_intents.remove(index);
    // Pointer and journal removal must be durable together before success.
    locked.save(state)?;
    Ok(OrchestrationActionResult::TaskAssigned {
        task,
        to_run_id: pointer.root_id,
        seq,
        duplicate: duplicate || message_duplicate,
    })
}

/// Read afresh at each boundary. An unrelated Markdown edit is preserved;
/// changed/ambiguous identified tasks never acquire assignment mail.
fn resume(
    locked: &LockedStore<'_>,
    state: &mut OrchestrationState,
    index: usize,
    duplicate: bool,
) -> Result<Option<OrchestrationActionResult>, InspectionError> {
    let intent = &state.assignment_intents[index];
    let root_open = state
        .runs
        .iter()
        .find(|run| run.run_id == intent.root_id)
        .is_some_and(|run| validate_root(state, &run.session_id, &intent.root_id, false).is_ok());
    if !root_open {
        state.assignment_intents[index].state = IntentState::Conflict;
        locked.save(state)?;
        return Ok(None);
    }
    let document = locked.tasks(&intent.root_id)?;
    match document.task(&intent.task_id) {
        Err(failure) if failure.code == "task_not_found" => {
            document.create_with_id(&intent.task_id, &intent.title, &intent.body)?;
        }
        Err(failure) if failure.code == "task_id_duplicate" => {}
        Err(failure) => return Err(failure),
        Ok(_) => {}
    }
    // This reread also catches edits after the canonical append but before mail.
    let document = locked.tasks(&intent.root_id)?;
    let task = match document.task(&intent.task_id) {
        Ok(task) if document.matches_creation(task, &intent.title, &intent.body) => {
            Some(task.clone())
        }
        Ok(_) => None,
        Err(failure)
            if matches!(
                failure.code.as_str(),
                "task_not_found" | "task_id_duplicate"
            ) =>
        {
            None
        }
        Err(failure) => return Err(failure),
    };
    match task {
        Some(task) => finalize(locked, state, index, task, duplicate).map(Some),
        None => {
            state.assignment_intents[index].state = IntentState::Conflict;
            locked.save(state)?;
            Ok(None)
        }
    }
}

/// Called with the same lock/state as acceptance recovery. Each effect saves
/// independently so a later I/O failure cannot lose an earlier recovery.
pub(super) fn recover(
    locked: &LockedStore<'_>,
    state: &mut OrchestrationState,
) -> Result<bool, InspectionError> {
    let mut changed = false;
    let mut index = 0;
    while index < state.assignment_intents.len() {
        if state.assignment_intents[index].state == IntentState::Conflict {
            index += 1;
            continue;
        }
        let finalized = resume(locked, state, index, true)?.is_some();
        changed = true;
        if !finalized {
            index += 1;
        }
    }
    Ok(changed)
}

#[allow(clippy::too_many_arguments)]
pub(super) fn resolve(
    locked: &LockedStore<'_>,
    state: &mut OrchestrationState,
    session_id: &str,
    root_id: &str,
    task_id: &str,
    expected_task_revision: Option<&str>,
    assign: bool,
) -> Result<OrchestrationActionResult, InspectionError> {
    let root_id = validate_uuid(root_id)?.to_string();
    let task_id = validate_uuid(task_id)?.to_string();
    validate_root(state, session_id, &root_id, !assign)?;
    let index = state
        .assignment_intents
        .iter()
        .position(|intent| {
            intent.root_id == root_id
                && intent.task_id == task_id
                && intent.state == IntentState::Conflict
        })
        .ok_or_else(|| {
            error(
                "task_assignment_not_conflicted",
                "No conflicted assignment exists",
            )
        })?;
    if !assign {
        state.assignment_intents.remove(index);
        locked.save(state)?;
        return Ok(OrchestrationActionResult::Done);
    }
    let document = locked.tasks(&root_id)?;
    let task = document.task(&task_id)?;
    if expected_task_revision != Some(task.task_revision.as_str()) {
        return Err(error(
            "task_revision_conflict",
            "Inspect the exact current task revision before assigning",
        ));
    }
    finalize(locked, state, index, task.clone(), false)
}

pub(super) fn snapshot_intents(
    state: &OrchestrationState,
    session_id: &str,
) -> Vec<TaskAssignmentIntent> {
    state
        .assignment_intents
        .iter()
        .filter(|intent| {
            state
                .runs
                .iter()
                .any(|run| run.run_id == intent.root_id && run.session_id == session_id)
        })
        .map(|intent| TaskAssignmentIntent {
            root_id: intent.root_id.clone(),
            task_id: intent.task_id.clone(),
            state: intent.state,
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use cockpit_protocol::orchestration::Run;
    use std::path::PathBuf;
    use uuid::Uuid;

    use super::super::{
        store::{MAX_STATE_BYTES, OrchestrationStore},
        tasks_md::root_filename,
    };

    const SESSION: &str = "assignment-session";
    const TITLE: &str = "Canonical assignment";
    const BODY: &str = "Task detail\nwith another line";

    struct Fixture {
        path: PathBuf,
        store: OrchestrationStore,
        root_id: String,
        task_id: String,
    }

    impl Fixture {
        fn new() -> Self {
            let path = std::env::temp_dir().join(format!("cockpit-assignment-{}", Uuid::new_v4()));
            let store = OrchestrationStore::open(&path).unwrap();
            let root_id = Uuid::new_v4().to_string();
            let task_id = Uuid::new_v4().to_string();
            let locked = store.lock().unwrap();
            let mut state = locked.read().unwrap();
            state.runs.push(Run {
                session_id: SESSION.into(),
                prepare_brief: String::new(),
                run_id: root_id.clone(),
                kind: RunKind::Supervisor,
                label: "Supervisor".into(),
                root_id: root_id.clone(),
                parent_run_id: None,
                task_id: None,
                attempt: 1,
                task_revision_at_propose: None,
                stage: RunStage::Active,
                close_reason: None,
                dispatch: None,
                target: None,
                setup: None,
                prepare_plan: None,
                init_receipt: None,
                work_plan: None,
                grants: Vec::new(),
                last_report: None,
                result: None,
                annotations: Vec::new(),
                location: None,
                bound_omp_session: Some("actual-main".into()),
                bound_omp_process: None,
                launch_shell_identity: None,
                retirement: None,
                supersedes_run_id: None,
                created_at: super::super::now(),
                updated_at: super::super::now(),
            });
            locked.save(&mut state).unwrap();
            drop(locked);
            Self {
                path,
                store,
                root_id,
                task_id,
            }
        }

        fn journal(&self, locked: &LockedStore<'_>, state: &mut OrchestrationState) {
            state.assignment_intents.push(AssignmentIntent {
                root_id: self.root_id.clone(),
                task_id: self.task_id.clone(),
                request_hash: request_hash(&self.root_id, TITLE, BODY),
                title: TITLE.into(),
                body: BODY.into(),
                origin: OperatorOrigin::Native,
                state: IntentState::Pending,
            });
            locked.save(state).unwrap();
        }

        fn assign(
            &self,
            locked: &LockedStore<'_>,
            state: &mut OrchestrationState,
        ) -> Result<OrchestrationActionResult, InspectionError> {
            assign(
                locked,
                state,
                SESSION,
                OperatorOrigin::Native,
                &self.root_id,
                &self.task_id,
                TITLE,
                BODY,
            )
        }

        fn write(&self, bytes: &[u8]) {
            self.store
                .tasks_dir()
                .write(root_filename(&self.root_id).unwrap(), bytes)
                .unwrap();
        }

        fn bytes(&self) -> Vec<u8> {
            self.store
                .tasks_dir()
                .read(root_filename(&self.root_id).unwrap())
                .unwrap()
        }

        fn assert_final(&self, locked: &LockedStore<'_>) {
            let state = locked.read().unwrap();
            assert!(state.assignment_intents.is_empty());
            assert_eq!(state.messages.len(), 1);
            let message = &state.messages[0];
            assert_eq!(message.message_id, format!("assign-{}", self.task_id));
            assert_eq!(message.kind, MessageKind::Instruction);
            assert!(matches!(message.from, ActorRef::Operator));
            let document = locked.tasks(&self.root_id).unwrap();
            assert_eq!(document.tasks.len(), 1);
            let task = document.task(&self.task_id).unwrap();
            let pointer: serde_json::Value = serde_json::from_str(&message.text).unwrap();
            assert_eq!(
                pointer,
                serde_json::json!({
                    "event": "task_assigned", "root_id": self.root_id, "task_id": self.task_id,
                    "task_revision": task.task_revision,
                    "request_hash": request_hash(&self.root_id, TITLE, BODY), "origin": "native"
                })
            );
        }
    }

    impl Drop for Fixture {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.path);
        }
    }

    #[test]
    fn new_assignment_refuses_managed_source_injection_before_journaling() {
        for description in [
            "- [ ] An unmanaged step",
            "<!-- cockpit-relations: depends_on=11111111-1111-4111-8111-111111111111 -->",
            "<!-- cockpit-checklist: begin -->",
        ] {
            let fixture = Fixture::new();
            fixture.write(b"Existing introduction\n");
            let locked = fixture.store.lock().unwrap();
            let mut state = locked.read().unwrap();
            let before = fixture.bytes();
            let revision = state.revision;
            assert_eq!(
                assign(
                    &locked,
                    &mut state,
                    SESSION,
                    OperatorOrigin::Native,
                    &fixture.root_id,
                    &fixture.task_id,
                    TITLE,
                    description,
                ).unwrap_err().code,
                "invalid_task"
            );
            assert_eq!(fixture.bytes(), before);
            assert_eq!(state.revision, revision);
            assert!(state.assignment_intents.is_empty());
            assert!(state.messages.is_empty());
        }
    }

    #[test]
    fn legacy_assignment_recovery_preserves_original_nested_checklist_bytes() {
        let fixture = Fixture::new();
        let locked = fixture.store.lock().unwrap();
        let mut state = locked.read().unwrap();
        fixture.journal(&locked, &mut state);
        let legacy = "Legacy prose\n- [x] Existing nested step";
        state.assignment_intents[0].body = legacy.into();
        state.assignment_intents[0].request_hash = request_hash(&fixture.root_id, TITLE, legacy);
        locked.save(&mut state).unwrap();
        recover(&locked, &mut state).unwrap();
        assert_eq!(
            locked.tasks(&fixture.root_id).unwrap().task(&fixture.task_id).unwrap().body,
            legacy
        );
        assert!(fixture.bytes().ends_with(b"  Legacy prose\n  - [x] Existing nested step\n"));
        assert!(state.assignment_intents.is_empty());
        assert_eq!(state.messages[0].message_id, format!("assign-{}", fixture.task_id));
    }

    #[test]
    fn successful_assignment_deduplicates_and_retains_only_canonical_pointer() {
        let fixture = Fixture::new();
        let locked = fixture.store.lock().unwrap();
        let mut state = locked.read().unwrap();
        let first = fixture.assign(&locked, &mut state).unwrap();
        assert!(matches!(
            first,
            OrchestrationActionResult::TaskAssigned {
                seq: 1,
                duplicate: false,
                ..
            }
        ));
        let revision = state.revision;
        assert!(matches!(
            fixture.assign(&locked, &mut state).unwrap(),
            OrchestrationActionResult::TaskAssigned {
                seq: 1,
                duplicate: true,
                ..
            }
        ));
        assert_eq!(state.revision, revision);
        fixture.assert_final(&locked);
        assert_eq!(
            assign(
                &locked,
                &mut state,
                SESSION,
                OperatorOrigin::Browser,
                &fixture.root_id,
                &fixture.task_id,
                TITLE,
                "Changed request"
            )
            .unwrap_err()
            .code,
            "task_assignment_conflict"
        );
        fixture.assert_final(&locked);
    }

    #[test]
    fn completed_repeat_returns_current_canonical_task_without_rewriting_mail() {
        let fixture = Fixture::new();
        let locked = fixture.store.lock().unwrap();
        let mut state = locked.read().unwrap();
        fixture.assign(&locked, &mut state).unwrap();
        let mail = state.messages[0].text.clone();
        let original = locked
            .tasks(&fixture.root_id)
            .unwrap()
            .task(&fixture.task_id)
            .unwrap()
            .clone();
        let current = locked
            .tasks(&fixture.root_id)
            .unwrap()
            .update(
                &fixture.task_id,
                &original.task_revision,
                Some("Current edited task"),
                Some("Current body"),
            )
            .unwrap();
        let revision = state.revision;
        let result = fixture.assign(&locked, &mut state).unwrap();
        let OrchestrationActionResult::TaskAssigned {
            task,
            seq,
            duplicate,
            ..
        } = result
        else {
            panic!("expected canonical assignment receipt");
        };
        assert_eq!(task.title, current.title);
        assert_eq!(task.body, current.body);
        assert_eq!(task.task_revision, current.task_revision);
        assert_eq!(seq, 1);
        assert!(duplicate);
        assert_eq!(state.revision, revision);
        assert_eq!(locked.read().unwrap().messages.len(), 1);
        assert_eq!(locked.read().unwrap().messages[0].text, mail);
    }

    #[test]
    fn recovery_after_pending_save_preserves_fresh_unrelated_markdown() {
        let fixture = Fixture::new();
        {
            let locked = fixture.store.lock().unwrap();
            let mut state = locked.read().unwrap();
            fixture.journal(&locked, &mut state);
        }
        fixture.write(b"# User notes\n\nExternal prose, added after the journal.\n");
        let reopened = OrchestrationStore::open(&fixture.path).unwrap();
        let locked = reopened.lock().unwrap();
        let mut state = locked.read().unwrap();
        assert!(recover(&locked, &mut state).unwrap());
        assert!(
            fixture
                .bytes()
                .starts_with(b"# User notes\n\nExternal prose, added after the journal.\n")
        );
        fixture.assert_final(&locked);
        assert!(!recover(&locked, &mut state).unwrap());
        fixture.assert_final(&locked);
    }

    #[test]
    fn recovery_after_canonical_append_creates_exactly_one_message_and_task() {
        let fixture = Fixture::new();
        {
            let locked = fixture.store.lock().unwrap();
            let mut state = locked.read().unwrap();
            fixture.journal(&locked, &mut state);
            locked
                .tasks(&fixture.root_id)
                .unwrap()
                .create_with_id(&fixture.task_id, TITLE, BODY)
                .unwrap();
        }
        let before = fixture.bytes();
        let reopened = OrchestrationStore::open(&fixture.path).unwrap();
        let locked = reopened.lock().unwrap();
        let mut state = locked.read().unwrap();
        assert!(recover(&locked, &mut state).unwrap());
        assert_eq!(fixture.bytes(), before);
        fixture.assert_final(&locked);
        assert!(matches!(
            fixture.assign(&locked, &mut state).unwrap(),
            OrchestrationActionResult::TaskAssigned {
                seq: 1,
                duplicate: true,
                ..
            }
        ));
    }

    #[test]
    fn unrelated_newline_convention_edit_does_not_change_identified_task_recovery() {
        let fixture = Fixture::new();
        let locked = fixture.store.lock().unwrap();
        let mut state = locked.read().unwrap();
        fixture.journal(&locked, &mut state);
        locked
            .tasks(&fixture.root_id)
            .unwrap()
            .create_with_id(&fixture.task_id, TITLE, BODY)
            .unwrap();
        let mut bytes = b"External CRLF prose\r\n".to_vec();
        bytes.extend_from_slice(&fixture.bytes());
        fixture.write(&bytes);
        recover(&locked, &mut state).unwrap();
        assert_eq!(fixture.bytes(), bytes);
        fixture.assert_final(&locked);
    }

    #[test]
    fn changed_task_conflicts_without_mail_and_resolution_requires_current_cas() {
        let fixture = Fixture::new();
        let locked = fixture.store.lock().unwrap();
        let mut state = locked.read().unwrap();
        fixture.journal(&locked, &mut state);
        let task = locked
            .tasks(&fixture.root_id)
            .unwrap()
            .create_with_id(&fixture.task_id, TITLE, BODY)
            .unwrap();
        let current = locked
            .tasks(&fixture.root_id)
            .unwrap()
            .update(
                &fixture.task_id,
                &task.task_revision,
                Some("User's authoritative title"),
                None,
            )
            .unwrap();
        let before = fixture.bytes();
        recover(&locked, &mut state).unwrap();
        assert!(state.messages.is_empty());
        assert_eq!(state.assignment_intents[0].state, IntentState::Conflict);
        assert_eq!(
            fixture.assign(&locked, &mut state).unwrap_err().code,
            "task_assignment_conflict"
        );
        assert_eq!(
            resolve(
                &locked,
                &mut state,
                SESSION,
                &fixture.root_id,
                &fixture.task_id,
                Some(&task.task_revision),
                true
            )
            .unwrap_err()
            .code,
            "task_revision_conflict"
        );
        assert_eq!(
            resolve(
                &locked,
                &mut state,
                SESSION,
                &fixture.root_id,
                &fixture.task_id,
                None,
                true
            )
            .unwrap_err()
            .code,
            "task_revision_conflict"
        );
        assert!(state.messages.is_empty());
        let result = resolve(
            &locked,
            &mut state,
            SESSION,
            &fixture.root_id,
            &fixture.task_id,
            Some(&current.task_revision),
            true,
        )
        .unwrap();
        assert!(matches!(
            result,
            OrchestrationActionResult::TaskAssigned {
                duplicate: false,
                ..
            }
        ));
        assert_eq!(fixture.bytes(), before);
        fixture.assert_final(&locked);
    }

    #[test]
    fn checkbox_and_duplicate_identity_edits_conflict_without_mail() {
        for duplicate in [false, true] {
            let fixture = Fixture::new();
            let locked = fixture.store.lock().unwrap();
            let mut state = locked.read().unwrap();
            fixture.journal(&locked, &mut state);
            let task = locked
                .tasks(&fixture.root_id)
                .unwrap()
                .create_with_id(&fixture.task_id, TITLE, BODY)
                .unwrap();
            if duplicate {
                let mut bytes = fixture.bytes();
                bytes.extend_from_slice(
                    format!("- [ ] Another <!-- cockpit-task: {} -->\n", fixture.task_id)
                        .as_bytes(),
                );
                fixture.write(&bytes);
            } else {
                locked
                    .tasks(&fixture.root_id)
                    .unwrap()
                    .check(&fixture.task_id, &task.task_revision, true)
                    .unwrap();
            }
            let before = fixture.bytes();
            recover(&locked, &mut state).unwrap();
            assert!(state.messages.is_empty());
            assert_eq!(state.assignment_intents[0].state, IntentState::Conflict);
            assert!(!recover(&locked, &mut state).unwrap());
            assert_eq!(fixture.bytes(), before);
            assert!(matches!(
                resolve(
                    &locked,
                    &mut state,
                    SESSION,
                    &fixture.root_id,
                    &fixture.task_id,
                    None,
                    false
                )
                .unwrap(),
                OrchestrationActionResult::Done
            ));
            assert_eq!(fixture.bytes(), before);
            assert!(locked.read().unwrap().assignment_intents.is_empty());
            assert!(locked.read().unwrap().messages.is_empty());
        }
    }

    #[test]
    fn changed_request_during_pending_does_not_change_original_journal_or_markdown() {
        let fixture = Fixture::new();
        let locked = fixture.store.lock().unwrap();
        let mut state = locked.read().unwrap();
        fixture.journal(&locked, &mut state);
        let revision = state.revision;
        assert_eq!(
            assign(
                &locked,
                &mut state,
                SESSION,
                OperatorOrigin::Native,
                &fixture.root_id,
                &fixture.task_id,
                "Different",
                BODY
            )
            .unwrap_err()
            .code,
            "task_assignment_conflict"
        );
        assert_eq!(state.revision, revision);
        assert_eq!(state.assignment_intents[0].title, TITLE);
        assert!(locked.tasks(&fixture.root_id).unwrap().tasks.is_empty());
        assert!(matches!(
            fixture.assign(&locked, &mut state).unwrap(),
            OrchestrationActionResult::TaskAssigned {
                seq: 1,
                duplicate: true,
                ..
            }
        ));
        fixture.assert_final(&locked);
    }

    #[test]
    fn pending_save_failure_cannot_create_task_or_message() {
        let fixture = Fixture::new();
        let locked = fixture.store.lock().unwrap();
        let mut state = locked.read().unwrap();
        state.runs[0].label = "x".repeat(MAX_STATE_BYTES);
        assert_eq!(
            fixture.assign(&locked, &mut state).unwrap_err().code,
            "orchestration_state_full"
        );
        assert!(locked.tasks(&fixture.root_id).unwrap().tasks.is_empty());
        let durable = locked.read().unwrap();
        assert!(durable.assignment_intents.is_empty());
        assert!(durable.messages.is_empty());
    }

    #[test]
    fn final_save_failure_retains_durable_pending_and_recovers_without_second_task() {
        let fixture = Fixture::new();
        let locked = fixture.store.lock().unwrap();
        let mut state = locked.read().unwrap();
        fixture.journal(&locked, &mut state);
        let encoded_len = serde_json::to_vec(&state).unwrap().len();
        let padding = MAX_STATE_BYTES - encoded_len - 128;
        state.runs[0].label.push_str(&"x".repeat(padding));
        locked.save(&mut state).unwrap();
        assert_eq!(
            fixture.assign(&locked, &mut state).unwrap_err().code,
            "orchestration_state_full"
        );
        let before = fixture.bytes();
        let mut durable = locked.read().unwrap();
        assert_eq!(durable.assignment_intents.len(), 1);
        assert_eq!(durable.assignment_intents[0].state, IntentState::Pending);
        assert!(durable.messages.is_empty());
        assert_eq!(locked.tasks(&fixture.root_id).unwrap().tasks.len(), 1);
        durable.runs[0].label = "Supervisor".into();
        locked.save(&mut durable).unwrap();
        recover(&locked, &mut durable).unwrap();
        assert_eq!(fixture.bytes(), before);
        fixture.assert_final(&locked);
    }

    #[test]
    fn canonical_write_failure_keeps_durable_pending_for_recovery() {
        let fixture = Fixture::new();
        fixture.write(&vec![b'x'; super::super::tasks_md::MAX_TASK_DOCUMENT_BYTES]);
        let locked = fixture.store.lock().unwrap();
        let mut state = locked.read().unwrap();
        assert_eq!(
            fixture.assign(&locked, &mut state).unwrap_err().code,
            "tasks_full"
        );
        let durable = locked.read().unwrap();
        assert_eq!(durable.assignment_intents.len(), 1);
        assert!(durable.messages.is_empty());
        fixture.write(b"# User restored valid document\n");
        let mut durable = locked.read().unwrap();
        recover(&locked, &mut durable).unwrap();
        assert!(
            fixture
                .bytes()
                .starts_with(b"# User restored valid document\n")
        );
        fixture.assert_final(&locked);
    }

    #[test]
    fn assignment_rejects_wrong_session_child_closed_and_worker_roots_without_journal() {
        let fixture = Fixture::new();
        let locked = fixture.store.lock().unwrap();
        let mut state = locked.read().unwrap();
        assert_eq!(
            assign(
                &locked,
                &mut state,
                "another-session",
                OperatorOrigin::Native,
                &fixture.root_id,
                &fixture.task_id,
                TITLE,
                BODY
            )
            .unwrap_err()
            .code,
            "task_assignment_root"
        );
        state.runs[0].parent_run_id = Some(Uuid::new_v4().to_string());
        assert_eq!(
            fixture.assign(&locked, &mut state).unwrap_err().code,
            "task_assignment_root"
        );
        state.runs[0].parent_run_id = None;
        state.runs[0].kind = RunKind::Worker;
        assert_eq!(
            fixture.assign(&locked, &mut state).unwrap_err().code,
            "task_assignment_root"
        );
        state.runs[0].kind = RunKind::Supervisor;
        state.runs[0].stage = RunStage::Closed;
        assert_eq!(
            fixture.assign(&locked, &mut state).unwrap_err().code,
            "task_assignment_root"
        );
        assert!(state.assignment_intents.is_empty());
        assert!(state.messages.is_empty());
        assert!(locked.tasks(&fixture.root_id).unwrap().tasks.is_empty());
    }

    #[test]
    fn closed_root_recovery_is_conflict_and_snapshot_never_publishes_payload() {
        let fixture = Fixture::new();
        let locked = fixture.store.lock().unwrap();
        let mut state = locked.read().unwrap();
        fixture.journal(&locked, &mut state);
        state.runs[0].stage = RunStage::Closed;
        locked.save(&mut state).unwrap();
        recover(&locked, &mut state).unwrap();
        assert_eq!(state.assignment_intents[0].state, IntentState::Conflict);
        assert!(state.messages.is_empty());
        assert!(locked.tasks(&fixture.root_id).unwrap().tasks.is_empty());
        assert_eq!(
            serde_json::to_value(snapshot_intents(&state, SESSION)).unwrap(),
            serde_json::json!([{
                "root_id": fixture.root_id, "task_id": fixture.task_id, "state": "conflict"
            }])
        );
        assert!(snapshot_intents(&state, "another-session").is_empty());
        assert_eq!(
            resolve(
                &locked,
                &mut state,
                SESSION,
                &fixture.root_id,
                &fixture.task_id,
                Some("any"),
                true
            )
            .unwrap_err()
            .code,
            "task_assignment_root"
        );
        assert_eq!(
            resolve(
                &locked,
                &mut state,
                "another-session",
                &fixture.root_id,
                &fixture.task_id,
                None,
                false
            )
            .unwrap_err()
            .code,
            "task_assignment_root"
        );
        assert!(matches!(
            resolve(
                &locked,
                &mut state,
                SESSION,
                &fixture.root_id,
                &fixture.task_id,
                None,
                false
            )
            .unwrap(),
            OrchestrationActionResult::Done
        ));
        assert!(locked.read().unwrap().assignment_intents.is_empty());
        assert!(locked.read().unwrap().messages.is_empty());
        assert!(locked.tasks(&fixture.root_id).unwrap().tasks.is_empty());
    }
}
