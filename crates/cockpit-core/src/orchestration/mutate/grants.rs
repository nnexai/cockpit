use super::{Applied, MutationCtx};
use super::super::*;
use super::tasks::task_transition_target;

pub(in crate::orchestration) fn prepare(
    ctx: &mut MutationCtx<'_>,
    run_id: String,
    plan_revision: String,
) -> Result<Applied, InspectionError> {
    let locked = ctx.locked;
    let actor = ctx.actor;
    let caller = ctx.caller;
    let session_id = ctx.session_id;
    let state = &mut *ctx.state;
    let (index, provenance) =
        management_target(state, actor, caller, session_id, &run_id)?;
    require_stage(&state.runs[index], RunStage::AwaitingPrepare)?;
    match_plan(state.runs[index].prepare_plan.as_ref(), &plan_revision)?;
    let document = locked.tasks(&state.runs[index].root_id)?;
    task_transition_target(state, &state.runs[index], &document, true)?;
    if let Some(old) = state.runs[index].supersedes_run_id.clone() {
        let old_index = run_index(state, &old)?;
        if state.runs[old_index].stage == RunStage::Closed {
            return Err(error(
                "task_has_active_run",
                "The replacement target changed; propose again",
            ));
        }
        state.runs[old_index].stage = RunStage::Closed;
        state.runs[old_index].close_reason = Some(CloseReason::Superseded);
        state.runs[old_index].updated_at = now();
        brief(
            state,
            &old,
            MessageKind::CancelRequest,
            &format!(
                "Run superseded by {run_id}; stop work and report any outstanding effects."
            ),
            "superseded",
        )?;
    }
    let run = &mut state.runs[index];
    run.grants
        .push(grant(provenance, GrantScope::Prepare, plan_revision));
    run.stage = RunStage::Preparing;
    run.dispatch = Some(dispatch(DispatchStep::SetupPending));
    Ok(Applied::Machine(OrchestrationActionResult::Done))
}

pub(in crate::orchestration) fn execute(
    ctx: &mut MutationCtx<'_>,
    run_id: String,
    plan_revision: String,
    note: Option<String>,
) -> Result<Applied, InspectionError> {
    let locked = ctx.locked;
    let actor = ctx.actor;
    let caller = ctx.caller;
    let session_id = ctx.session_id;
    let state = &mut *ctx.state;
    let (index, provenance) =
        management_target(state, actor, caller, session_id, &run_id)?;
    require_stage(&state.runs[index], RunStage::Ready)?;
    match_plan(state.runs[index].work_plan.as_ref(), &plan_revision)?;
    if state.runs[index].init_receipt.as_ref().is_none_or(|receipt| {
        receipt.kind != ReportKind::Ready || receipt.plan.as_deref()
            != state.runs[index].work_plan.as_ref().map(|plan| plan.text.as_str())
    }) {
        return Err(error("invalid_stage", "Matching initialization receipt is missing"));
    }
    let document = locked.tasks(&state.runs[index].root_id)?;
    task_transition_target(state, &state.runs[index], &document, false)?;
    let mut text = state.runs[index]
        .work_plan
        .as_ref()
        .expect("matched plan")
        .text
        .clone();
    if let Some(note) = note {
        bounded(&note, 16 * 1024)?;
        text.push_str("\n\nManagement note:\n");
        text.push_str(&note);
    }
    bounded(&text, 16 * 1024)?;
    state.runs[index].grants.push(grant(
        provenance,
        GrantScope::Execute,
        plan_revision,
    ));
    state.runs[index].stage = RunStage::Working;
    brief(
        state,
        &run_id,
        MessageKind::WorkBrief,
        &text,
        "execute",
    )?;
    Ok(Applied::Machine(OrchestrationActionResult::Done))
}

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
    task_transition_target(state, &state.runs[index], &document, false)?;
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
    let from = actor_ref(actor, caller, state);
    messages::append(state, messages::AppendMessage { from: from, to_run_id: &run_id, message_id: &format!("sendback-{}", id()), kind: MessageKind::Answer, text: &text, in_reply_to: None, report: None, stale: false, from_subagent_id: None, escalated_from: None })?;
    state.runs[index].annotations.push(annotation);
    Ok(Applied::Machine(OrchestrationActionResult::Done))
}

fn match_plan(plan: Option<&PlanRecord>, revision: &str) -> Result<(), InspectionError> {
    if plan.is_some_and(|p| p.plan_revision == revision) {
        Ok(())
    } else {
        Err(error(
            "plan_changed",
            "The exact plan changed; refresh and review it again",
        ))
    }
}
fn grant(provenance: DecisionProvenance, scope: GrantScope, plan_revision: String) -> Grant {
    Grant {
        grant_id: id(),
        scope,
        plan_revision,
        origin: provenance.origin,
        supervisor_run_id: provenance.supervisor_run_id,
        omp_session_id: provenance.omp_session_id,
        granted_at: now(),
    }
}
