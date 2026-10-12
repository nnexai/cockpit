use super::{Applied, MutationCtx};
use super::super::*;
use super::tasks::task_transition_target;

pub(in crate::orchestration) fn send_back(
    ctx: &mut MutationCtx<'_>,
    run_id: String,
    text: String,
) -> Result<Applied, InspectionError> {
    let locked = ctx.locked;
    let actor = ctx.actor;
    let caller = ctx.caller;
    let session_id = ctx.session_id;
    let state = &mut *ctx.state;
    let (index, provenance) =
        management_target(state, actor, caller, session_id, &run_id)?;
    bounded(&text, 16 * 1024)?;
    require_stage(&state.runs[index], RunStage::Reported)?;
    let document = locked.tasks(&state.runs[index].root_id)?;
    task_transition_target(state, &state.runs[index], &document)?;
    // Reviewed scope guidance resumes work without approving or freezing an exact plan.
    let result_message_id = &state.runs[index]
        .result
        .as_ref()
        .ok_or_else(|| error("invalid_stage", "Explicit Result is missing"))?
        .message_id;
    let annotation = decision_annotation(
        actor_ref(actor, caller, state),
        &provenance,
        &format!("Result {result_message_id} sent back"),
    );
    state.runs[index].stage = RunStage::Working;
    state.runs[index].result = None;
    state.runs[index].source_output = None;
    state.runs[index].integration = Integration::Pending;
    let from = actor_ref(actor, caller, state);
    messages::append(state, messages::AppendMessage { from: from, to_run_id: &run_id, message_id: &format!("sendback-{}", id()), kind: MessageKind::Answer, text: &text, in_reply_to: None, report: None, stale: false, escalated_from: None })?;
    state.runs[index].annotations.push(annotation);
    Ok(Applied::Machine(OrchestrationActionResult::Done))
}

