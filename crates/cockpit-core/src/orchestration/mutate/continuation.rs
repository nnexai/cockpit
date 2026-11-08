use super::{Applied, MutationCtx};
use super::super::*;
use super::super::store::{DurableOperationKind, LockedStore};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ContinueIntent {
    pub operation_id: String,
    pub principal_key: String,
    pub root_id: String,
    pub task_id: String,
    pub run_id: String,
    pub expected_task_revision: String,
    pub expected_doc_revision: String,
    pub intended_task_revision: String,
    pub description: String,
    pub text: String,
    pub from: ActorRef,
    pub route: ContinueRoute,
    pub state: IntentState,
}

pub(super) fn apply(ctx: &mut MutationCtx<'_>, action: OrchestrationAction) -> Result<Applied, InspectionError> {
    let OrchestrationAction::TaskContinue { operation_id, root_id, task_id, expected_task_revision, text } = &action else { unreachable!() };
    super::tasks::task_root_authority(ctx.state, ctx.actor, ctx.caller, ctx.session_id, root_id)?;
    let principal = super::resume::principal_key(ctx)?;
    if let Some(mut result) = super::resume::replay(ctx, operation_id, DurableOperationKind::TaskContinue, &action)? {
        if ctx.state.continue_intents.iter().any(|i| i.operation_id == *operation_id && i.principal_key == principal) {
            result = resume(ctx.locked, ctx.state, operation_id, &principal, true)?;
            ctx.service.revision.send_replace(ctx.state.revision);
        } else if let OrchestrationActionResult::TaskContinued { duplicate, .. } = &mut result { *duplicate = true; }
        return Ok(Applied::Published(OrchestrationMutationResponse { revision: ctx.state.revision, result }));
    }
    bounded(text, 16 * 1024)?;
    if text.trim().is_empty() { return Err(error("invalid_task", "Follow-up text is required")); }
    if ctx.state.continue_intents.iter().any(|i| i.root_id == *root_id && i.task_id == *task_id)
        || ctx.state.task_intents.iter().any(|i| i.root_id == *root_id && i.task_id == *task_id)
        || ctx.state.scope_intents.iter().any(|i| i.root_id == *root_id && i.task_id == *task_id && i.state == IntentState::Pending) {
        return Err(error("intent_conflict", "The task has an unsettled intent"));
    }
    let document = ctx.locked.tasks(root_id)?;
    let task = document.task(task_id)?;
    if task.task_revision != *expected_task_revision { return Err(error("task_revision_conflict", "Task changed")); }
    let run = projection::current_task_run(ctx.state, root_id, task_id)
        .ok_or_else(|| error("invalid_stage", "Continuation requires the current reported or accepted attempt"))?;
    let route = if !task.checked && run.stage == RunStage::Reported {
        ContinueRoute::SameWorker { run_id: run.run_id.clone() }
    } else if task.checked && run.stage == RunStage::Closed && run.close_reason == Some(CloseReason::Accepted) {
        ContinueRoute::Successor
    } else { return Err(error("invalid_stage", "Continuation requires a reported or accepted task")); };
    if run.result.is_none() { return Err(error("invalid_stage", "Explicit Result is missing")); }
    if matches!(route, ContinueRoute::Successor) {
        bounded(&successor_brief(run, text), 16 * 1024)?;
    }
    let run_id = run.run_id.clone();
    let date = now();
    let description = format!("{}\n\n**Follow-up ({}):** {}", task.description, &date[..10], text);
    let prepared = document.prepare_continue(task_id, expected_task_revision, &description)?;
    let intended = prepared.task.task_revision.clone();
    let result = OrchestrationActionResult::TaskContinued { operation_id: operation_id.clone(), task_id: task_id.clone(), task_revision: intended.clone(), route: route.clone(), duplicate: false };
    let from = actor_ref(ctx.actor, ctx.caller, ctx.state);
    super::resume::record(ctx, operation_id.clone(), DurableOperationKind::TaskContinue, &action, root_id.clone(), result, Vec::new())?;
    ctx.state.continue_intents.push(ContinueIntent { operation_id: operation_id.clone(), principal_key: principal.clone(), root_id: root_id.clone(), task_id: task_id.clone(), run_id,
        expected_task_revision: expected_task_revision.clone(), expected_doc_revision: document.doc_revision.clone(), intended_task_revision: intended, description, text: text.clone(), from, route, state: IntentState::Pending });
    ctx.locked.save(ctx.state)?;
    match prepared.publish() { Ok(_) => {}, Err(failure) if failure.code == "task_revision_conflict" => {}, Err(failure) => return Err(failure) }
    let result = resume(ctx.locked, ctx.state, operation_id, &principal, false);
    ctx.service.revision.send_replace(ctx.state.revision);
    Ok(Applied::Published(OrchestrationMutationResponse { revision: ctx.state.revision, result: result? }))
}

fn successor_brief(run: &Run, text: &str) -> String {
    let summary = &run.result.as_ref().expect("validated result").summary;
    let mut end = summary.len().min(4096);
    while !summary.is_char_boundary(end) { end -= 1; }
    format!("Continuation of accepted attempt {} ({}). Prior Result: {}. Follow-up: {}", run.attempt, run.run_id, &summary[..end], text)
}

fn continuation_target(run: &Run) -> Option<DispatchTarget> {
    let head = match &run.source_output { Some(SourceOutput::Code { head_commit, .. }) => Some(head_commit.clone()), _ => None };
    match &run.target {
        Some(DispatchTarget::SpaceWorktree { workspace_id, .. }) => Some(DispatchTarget::SpaceWorktree { workspace_id: workspace_id.clone(), branch: None, base_ref: head }),
        Some(DispatchTarget::Setup { request: WorkspaceSetupRequest::Create { .. } }) => {
            let mut target = run.target.clone();
            if let Some(DispatchTarget::Setup { request: WorkspaceSetupRequest::Create { branch, base_ref, checkout_path, focus, .. } }) = &mut target {
                *branch = None; *base_ref = head; *checkout_path = None; *focus = false;
            }
            target
        }
        target => target.clone(),
    }
}

fn resume(locked: &LockedStore<'_>, state: &mut OrchestrationState, operation_id: &str, principal: &str, duplicate: bool) -> Result<OrchestrationActionResult, InspectionError> {
    let index = state.continue_intents.iter().position(|i| i.operation_id == operation_id && i.principal_key == principal)
        .ok_or_else(|| error("orchestration_state_invalid", "Continuation journal is missing"))?;
    let intent = state.continue_intents[index].clone();
    if intent.state == IntentState::Conflict { return Err(error("intent_conflict", "Continuation has a retained conflict")); }
    let document = locked.tasks(&intent.root_id)?;
    let task = document.task(&intent.task_id);
    let current = projection::current_task_run(state, &intent.root_id, &intent.task_id);
    let valid_run = current.is_some_and(|run| run.run_id == intent.run_id && run.result.is_some() && match &intent.route {
        ContinueRoute::SameWorker { .. } => run.stage == RunStage::Reported,
        ContinueRoute::Successor => run.stage == RunStage::Closed && run.close_reason == Some(CloseReason::Accepted),
    });
    let written = task.as_ref().is_ok_and(|task| task.task_revision == intent.intended_task_revision);
    let fenced = task.as_ref().is_ok_and(|task| task.task_revision == intent.expected_task_revision) && document.doc_revision == intent.expected_doc_revision;
    if !valid_run || (!written && !fenced) {
        state.continue_intents[index].state = IntentState::Conflict;
        locked.save(state)?;
        return Err(error("intent_conflict", "Continuation no longer matches the task or attempt"));
    }
    if !written {
        if let Err(failure) = document.prepare_continue(&intent.task_id, &intent.expected_task_revision, &intent.description)?.publish() {
            if failure.code != "task_revision_conflict" { return Err(failure); }
            state.continue_intents[index].state = IntentState::Conflict;
            locked.save(state)?;
            return Err(error("intent_conflict", "Continuation conflicted with a task edit"));
        }
    }
    let document = locked.tasks(&intent.root_id)?;
    if !document.task(&intent.task_id).is_ok_and(|task| task.task_revision == intent.intended_task_revision) {
        state.continue_intents[index].state = IntentState::Conflict;
        locked.save(state)?;
        return Err(error("intent_conflict", "Continuation conflicted with a task edit"));
    }
    let run_index = run_index(state, &intent.run_id)?;
    match &intent.route {
        ContinueRoute::SameWorker { .. } => {
            let annotation = Annotation { at: now(), by: intent.from.clone(), text: "Result sent back for continuation".into() };
            super::grants::reopen_reported(state, run_index, &intent.text, intent.from.clone(), annotation)?;
        }
        ContinueRoute::Successor => {
            let run = &state.runs[run_index];
            let brief = successor_brief(run, &intent.text);
            let schedule = TaskSchedule { operation_id: intent.operation_id.clone(), principal_key: intent.principal_key.clone(), target: continuation_target(run), brief,
                cleanup_policy: run.cleanup_policy.unwrap_or(WorkerCleanupPolicy::OwnedWhenSafe), label: Some(run.label.clone()), scheduled_at: now(), hold: None };
            let attempt = run.attempt;
            let owner = state.task_ownership.iter_mut().find(|o| o.root_id == intent.root_id && o.task_id == intent.task_id)
                .ok_or_else(|| error("orchestration_state_invalid", "Accepted task ownership is missing"))?;
            owner.owner = None;
            owner.reopened_after_attempt = Some(attempt);
            owner.schedule = Some(schedule);
        }
    }
    let result = OrchestrationActionResult::TaskContinued { operation_id: intent.operation_id.clone(), task_id: intent.task_id.clone(), task_revision: intent.intended_task_revision, route: intent.route, duplicate };
    let operation = state.operations.iter_mut().find(|o| o.operation_id == intent.operation_id && o.principal_key == intent.principal_key)
        .ok_or_else(|| error("orchestration_state_invalid", "Continuation operation is missing"))?;
    operation.result = Some(result.clone());
    operation.updated_at = now();
    state.continue_intents.remove(index);
    locked.save(state)?;
    Ok(result)
}

pub(in crate::orchestration) fn recover(locked: &LockedStore<'_>, state: &mut OrchestrationState) -> Result<(), InspectionError> {
    let pending: Vec<_> = state.continue_intents.iter().filter(|i| i.state == IntentState::Pending)
        .map(|i| (i.operation_id.clone(), i.principal_key.clone())).collect();
    for (operation, principal) in pending {
        if let Err(failure) = resume(locked, state, &operation, &principal, true) {
            if failure.code != "intent_conflict" { return Err(failure); }
        }
    }
    Ok(())
}
