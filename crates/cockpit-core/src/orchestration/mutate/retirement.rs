use super::{Applied, MutationCtx};
use super::super::*;
use super::super::retirement;

pub(in crate::orchestration) fn native_receipt(
    ctx: &mut MutationCtx<'_>,
    retirement_id: String,
    outcome: NativeStopReceipt,
) -> Result<Applied, InspectionError> {
    let actor = ctx.actor;
    let caller = ctx.caller;
    let state = &mut *ctx.state;
    let index = required_caller(caller)?;
    let Actor::Agent(agent) = actor else {
        return Err(error("actor_forbidden", "Retirement receipts are agent-only"));
    };
    if matches!(outcome, NativeStopReceipt::ShutdownRequested)
        && !retirement::blockers(state, &state.runs[index]).is_empty() {
        return Err(error("retirement_state_changed", "Worker has open descendants or running subagents"));
    }
    retirement::apply_native_receipt(&mut state.runs[index], agent, &retirement_id, outcome)?;
    Ok(Applied::Machine(OrchestrationActionResult::Done))
}
