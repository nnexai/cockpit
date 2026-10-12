use cockpit_protocol::orchestration::{
    ActorRef, CloseReason, IntentState, MessageKind, OrchestrationActionResult, RunKind, RunStage,
    TaskScopeUpdateResult, TaskScopeUpdateState,
};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use super::{
    dependencies, error, messages, now, projection,
    store::{LockedStore, OrchestrationState},
};
use crate::InspectionError;

/// Only unsettled scope payloads live here. A settled operation retains its
/// actor/digest and compact receipt in the existing operation journal.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ScopeIntent {
    pub operation_id: String,
    pub principal_key: String,
    pub root_id: String,
    pub task_id: String,
    pub run_id: String,
    pub expected_task_revision: String,
    pub expected_doc_revision: String,
    pub intended_task_revision: String,
    pub intended_doc_revision: String,
    pub title: Option<String>,
    pub description: Option<String>,
    pub from: ActorRef,
    pub notify_worker: bool,
    pub state: IntentState,
}

#[derive(Serialize)]
struct ScopePointer<'a> {
    event: &'static str,
    operation_id: &'a str,
    root_id: &'a str,
    task_id: &'a str,
    task_revision: &'a str,
    doc_revision: &'a str,
}

fn settle(
    locked: &LockedStore<'_>, state: &mut OrchestrationState, index: usize,
    result: OrchestrationActionResult,
) -> Result<OrchestrationActionResult, InspectionError> {
    let operation_id = &state.scope_intents[index].operation_id;
    let principal_key = &state.scope_intents[index].principal_key;
    let record = state.operations.iter_mut().find(|record|
        &record.operation_id == operation_id && &record.principal_key == principal_key)
        .ok_or_else(|| error("orchestration_state_invalid", "Scope journal has no durable operation"))?;
    record.result = Some(result.clone());
    record.updated_at = now();
    if matches!(&result, OrchestrationActionResult::TaskScopeUpdated { update }
        if update.state == TaskScopeUpdateState::Applied) {
        state.scope_intents.remove(index);
    } else {
        // Keep the rejected draft available; never write it over fresh user content.
        state.scope_intents[index].state = IntentState::Conflict;
    }
    locked.save(state)?;
    Ok(result)
}

pub(super) fn resume(
    locked: &LockedStore<'_>, state: &mut OrchestrationState,
    operation_id: &str, principal_key: &str, duplicate: bool,
) -> Result<OrchestrationActionResult, InspectionError> {
    let index = state.scope_intents.iter().position(|intent|
        intent.operation_id == operation_id && intent.principal_key == principal_key)
        .ok_or_else(|| error("orchestration_state_invalid", "Pending scope journal is absent"))?;
    let intent = state.scope_intents[index].clone();
    let document = locked.tasks(&intent.root_id)?;
    let task = document.task(&intent.task_id);
    let active = state.runs.iter().any(|root| root.run_id == intent.root_id
        && root.root_id == root.run_id && root.parent_run_id.is_none()
        && matches!(root.kind, RunKind::Supervisor | RunKind::Adopted) && root.stage == RunStage::Active)
        && projection::current_task_run(state, &intent.root_id, &intent.task_id)
            .is_some_and(|run| run.run_id == intent.run_id && run.kind == RunKind::Worker
                && run.stage == RunStage::Working)
        && !state.task_intents.iter().any(|acceptance| acceptance.root_id == intent.root_id
            && acceptance.task_id == intent.task_id)
        && !state.runs.iter().any(|run| run.root_id == intent.root_id
            && run.task_id.as_deref() == Some(intent.task_id.as_str())
            && run.close_reason == Some(CloseReason::Accepted));
    let valid_task = match &task {
        Ok(task) => !task.checked && dependencies::DependencyGraph::new(&document.tasks).require(task).is_ok(),
        Err(failure) if matches!(failure.code.as_str(), "task_not_found" | "task_id_duplicate") => false,
        Err(failure) => return Err(error(&failure.code, &failure.message)),
    };
    let written = task.as_ref().is_ok_and(|task| task.task_revision == intent.intended_task_revision)
        && (intent.intended_task_revision != intent.expected_task_revision
            || document.doc_revision == intent.intended_doc_revision);
    let fenced = task.as_ref().is_ok_and(|task| task.task_revision == intent.expected_task_revision)
        && document.doc_revision == intent.expected_doc_revision;
    if intent.state == IntentState::Conflict || !active || !valid_task || (!written && !fenced) {
        let result = OrchestrationActionResult::TaskScopeUpdated { update: TaskScopeUpdateResult {
            operation_id: intent.operation_id, state: TaskScopeUpdateState::Conflict,
            task_revision: task.ok().map(|task| task.task_revision.clone()),
            doc_revision: Some(document.doc_revision.clone()), instruction_message_id: None, duplicate,
        }};
        return settle(locked, state, index, result);
    }
    if !written {
        match document.update(&intent.task_id, &intent.expected_task_revision,
            intent.title.as_deref(), intent.description.as_deref()) {
            Ok(_) => {}
            Err(failure) if failure.code == "task_revision_conflict" => {
                let current = locked.tasks(&intent.root_id)?;
                let task_revision = match current.task(&intent.task_id) {
                    Ok(task) => Some(task.task_revision.clone()),
                    Err(failure) if matches!(failure.code.as_str(), "task_not_found" | "task_id_duplicate") => None,
                    Err(failure) => return Err(failure),
                };
                let result = OrchestrationActionResult::TaskScopeUpdated { update: TaskScopeUpdateResult {
                    operation_id: intent.operation_id, state: TaskScopeUpdateState::Conflict,
                    task_revision, doc_revision: Some(current.doc_revision),
                    instruction_message_id: None, duplicate,
                }};
                return settle(locked, state, index, result);
            }
            Err(failure) => return Err(failure),
        }
    }
    // A user may edit Markdown independently of the state lock. Never finalize
    // against a stale snapshot or overwrite a different post-write task.
    let document = locked.tasks(&intent.root_id)?;
    let task = document.task(&intent.task_id)?;
    if task.task_revision != intent.intended_task_revision {
        let result = OrchestrationActionResult::TaskScopeUpdated { update: TaskScopeUpdateResult {
            operation_id: intent.operation_id, state: TaskScopeUpdateState::Conflict,
            task_revision: Some(task.task_revision.clone()), doc_revision: Some(document.doc_revision.clone()),
            instruction_message_id: None, duplicate,
        }};
        return settle(locked, state, index, result);
    }
    let instruction_message_id = if intent.notify_worker {
        let message_id = format!("scope-{:x}-{}", Sha256::digest(intent.principal_key.as_bytes()), intent.operation_id);
        let text = serde_json::to_string(&ScopePointer {
            event: "task_scope_updated", operation_id: &intent.operation_id,
            root_id: &intent.root_id, task_id: &intent.task_id,
            task_revision: &task.task_revision, doc_revision: &document.doc_revision,
        }).map_err(|failure| error("task_scope_write", failure.to_string()))?;
        messages::append(state, messages::AppendMessage {
            from: intent.from, to_run_id: &intent.run_id, message_id: &message_id,
            kind: MessageKind::Instruction, text: &text, in_reply_to: None,
            report: None, stale: false, escalated_from: None,
        })?;
        Some(message_id)
    } else { None };
    let result = OrchestrationActionResult::TaskScopeUpdated { update: TaskScopeUpdateResult {
        operation_id: intent.operation_id, state: TaskScopeUpdateState::Applied,
        task_revision: Some(task.task_revision.clone()), doc_revision: Some(document.doc_revision.clone()),
        instruction_message_id, duplicate,
    }};
    // The instruction, operation receipt and journal removal are one state save.
    settle(locked, state, index, result)
}

pub(super) fn recover(
    locked: &LockedStore<'_>, state: &mut OrchestrationState,
) -> Result<bool, InspectionError> {
    let mut changed = false;
    let mut index = 0;
    while index < state.scope_intents.len() {
        if state.scope_intents[index].state == IntentState::Conflict { index += 1; continue }
        let operation_id = state.scope_intents[index].operation_id.clone();
        let principal_key = state.scope_intents[index].principal_key.clone();
        let result = resume(locked, state, &operation_id, &principal_key, true)?;
        if matches!(result, OrchestrationActionResult::TaskScopeUpdated { update }
            if update.state == TaskScopeUpdateState::Conflict) { index += 1; }
        changed = true;
    }
    Ok(changed)
}
