use super::{Applied, MutationCtx};
use super::super::*;
use super::super::store::DurableOperationKind;

pub(super) fn register(ctx: &mut MutationCtx<'_>, action: OrchestrationAction) -> Result<Applied, InspectionError> {
    let OrchestrationAction::RunRegister { operation_id } = &action else { unreachable!() };
    tasks_md::validate_uuid(operation_id)?;
    let Actor::Agent(caller) = ctx.actor else { return Err(error("actor_forbidden", "Registration requires an issued run credential")); };
    let job = credentials::validate_registration(ctx.state, caller)?;
    let index = job.run_index;
    let run = &ctx.state.runs[index];
    let root_id = run.root_id.clone();
    if let Some(task_id) = &run.task_id {
        if ctx.locked.tasks(&root_id)?.task(task_id)?.checked {
            return Err(error("task_accepted", "Accepted tasks cannot be claimed"));
        }
    }
    if let Some(result) = super::resume::replay(ctx, operation_id, DurableOperationKind::RunRegister, &action)? {
        return Ok(Applied::Document(result));
    }
    let activated = run.attachment.as_ref().is_some_and(|attachment|
        attachment.state == AttachmentState::Connected)
        && matches!(run.stage, RunStage::Active | RunStage::Working | RunStage::Reported);
    let duplicate = activated
        && (caller.conversation.is_none() || ctx.state.credentials.iter()
            .find(|record| record.credential_id == caller.auth.credential_id)
            .and_then(|record| record.conversation.as_ref()) == caller.conversation.as_ref());
    let run_id = run.run_id.clone();
    if duplicate {
        return Ok(Applied::Document(OrchestrationActionResult::RunRegistered {
            run_id: run.run_id.clone(), conversation: caller.conversation.clone(),
            assignment_generation: job.assignment_generation, duplicate: true,
        }));
    }
    if job.role == JobRole::Worker {
        let task_id = run.task_id.as_ref().expect("worker task").clone();
        let ownership = ctx.state.task_ownership.iter_mut().find(|record|
            record.root_id == root_id && record.task_id == task_id).expect("validated vacancy");
        if ownership.owner.is_none() {
            ownership.generation = job.assignment_generation.expect("worker generation");
            ownership.owner = Some(TaskOwner { run_id: run_id.clone(), conversation: caller.conversation.clone(), claim_operation_id: operation_id.clone() });
        }
        if caller.conversation.is_some() {
            ownership.owner.as_mut().expect("claimed owner").conversation = caller.conversation.clone();
        }
    }
    if let Some(claim_id) = &job.checkout_claim_id {
        let revision = ctx.state.checkout_claims.iter().find(|claim| &claim.claim_id == claim_id).expect("validated claim").revision;
        let holder = claims::AttachmentReference { run_id: run_id.clone(), attachment_id: job.attachment_id.clone(), attachment_epoch: job.attachment_epoch };
        claims::hold_claim(ctx.state, claim_id, &holder, revision)?;
    }
    if caller.conversation.is_some() {
        ctx.state.credentials.iter_mut().find(|record| record.credential_id == caller.auth.credential_id)
            .expect("validated capability").conversation = caller.conversation.clone();
    }
    let run = &mut ctx.state.runs[index];
    let attachment = run.attachment.as_mut().expect("validated attachment");
    if caller.conversation.is_some() { attachment.conversation = caller.conversation.clone(); }
    attachment.state = AttachmentState::Connected;
    attachment.updated_at = now();
    if !activated {
        run.stage = if job.role == JobRole::Root { RunStage::Active } else { RunStage::Working };
        run.updated_at = now();
        if let Some(dispatch) = &mut run.dispatch {
            if dispatch.latch.is_none() || matches!(dispatch.latch_state, Some(AgentLatchState::ReleaseIntent | AgentLatchState::Released)) {
                dispatch.step = DispatchStep::Launched;
                dispatch.error = None;
            }
            dispatch.updated_at = now();
        }
        if let Some(resume) = &mut run.resume {
            resume.phase = ResumePhase::Connected;
            resume.cause = None;
            resume.updated_at = now();
        }
        let kind = if job.role == JobRole::Worker { MessageKind::WorkBrief } else { MessageKind::SupervisorBrief };
        let text = run.brief.clone();
        let phase = format!("registration-{}", attachment.epoch);
        brief(ctx.state, &run_id, kind, &text, &phase)?;
    }
    let result = OrchestrationActionResult::RunRegistered { run_id, conversation: caller.conversation.clone(), assignment_generation: job.assignment_generation, duplicate };
    super::resume::record(ctx, operation_id.clone(), DurableOperationKind::RunRegister, &action, root_id, result.clone(), Vec::new())?;
    Ok(Applied::Machine(result))
}

pub(super) fn unassign(ctx: &mut MutationCtx<'_>, action: OrchestrationAction) -> Result<Applied, InspectionError> {
    let OrchestrationAction::TaskUnassign { operation_id, root_id, task_id, expected_assignment_generation } = &action else { unreachable!() };
    super::tasks::task_root_authority(ctx.state, ctx.actor, ctx.caller, ctx.session_id, root_id)?;
    if let Some(result) = super::resume::replay(ctx, operation_id, DurableOperationKind::TaskUnassign, &action)? {
        return Ok(Applied::Document(result));
    }
    let outcome = supersede_current(ctx, root_id, task_id, *expected_assignment_generation, None)?;
    let result = OrchestrationActionResult::TaskUnassigned { task_id: task_id.clone(), assignment_generation: outcome.generation, previous_run_id: outcome.previous_run_id, duplicate: false };
    super::resume::record(ctx, operation_id.clone(), DurableOperationKind::TaskUnassign, &action, root_id.clone(), result.clone(), Vec::new())?;
    Ok(Applied::Machine(result))
}

pub(super) struct SupersedeOutcome {
    pub generation: u64,
    pub previous_run_id: Option<String>,
}

pub(super) fn supersede_current(ctx: &mut MutationCtx<'_>, root_id: &str, task_id: &str,
    expected_generation: u64, expected_run_id: Option<&str>) -> Result<SupersedeOutcome, InspectionError> {
    if ctx.locked.tasks(root_id)?.task(task_id)?.checked {
        return Err(error("task_accepted", "Accepted tasks remain read-only"));
    }
    if ctx.state.task_intents.iter().any(|intent| intent.root_id == root_id && intent.task_id == task_id) {
        return Err(error("intent_conflict", "Resolve the durable acceptance transaction before superseding"));
    }
    let index = ctx.state.task_ownership.iter().position(|record| record.root_id == root_id && record.task_id == task_id)
        .ok_or_else(|| error("task_unassigned", "Task has no authorized owner or pending attempt"))?;
    let ownership = &ctx.state.task_ownership[index];
    let pending_generation = ownership.generation.checked_add(1).ok_or_else(|| error("invalid_counter", "Task generation exhausted"))?;
    let previous_run_id = ownership.owner.as_ref().map(|owner| owner.run_id.clone());
    let attempts: Vec<String> = ctx.state.runs.iter().filter(|run| run.root_id == root_id
        && run.task_id.as_deref() == Some(task_id) && (Some(&run.run_id) == previous_run_id.as_ref()
            || (run.stage != RunStage::Closed && run.assignment_generation == Some(pending_generation))))
        .map(|run| run.run_id.clone()).collect();
    if let Some(expected) = expected_run_id {
        let run = ctx.state.runs.iter().find(|run| run.run_id == expected)
            .ok_or_else(|| error("assignment_generation_conflict", "Expected attempt is no longer current"))?;
        if !attempts.iter().any(|id| id == expected) || run.assignment_generation != Some(expected_generation)
            || run.stage == RunStage::Closed {
            return Err(error("assignment_generation_conflict", "Task attempt changed before replacement"));
        }
    } else if ownership.generation != expected_generation {
        return Err(error("assignment_generation_conflict", "Task assignment changed before unassign"));
    }
    if attempts.is_empty() { return Err(error("task_unassigned", "Task has no owner or pending authorization")); }
    let revocation_generation = if expected_run_id.is_some() {
        ownership.generation.max(expected_generation).checked_add(1)
            .ok_or_else(|| error("invalid_counter", "Task revocation generation exhausted"))?
    } else { pending_generation };
    safe_counter(revocation_generation)?;
    let ownership = &mut ctx.state.task_ownership[index];
    ownership.generation = revocation_generation;
    ownership.owner = None;
    for run in &mut ctx.state.runs {
        if attempts.contains(&run.run_id) {
            run.stage = RunStage::Closed;
            run.close_reason = Some(CloseReason::Superseded);
            run.updated_at = now();
            // Semantic revocation never changes retained physical resources or claims.
        }
    }
    for credential in &mut ctx.state.credentials {
        if attempts.contains(&credential.run_id) { credential.state = credentials::CredentialState::Revoked; }
    }
    for intent in &mut ctx.state.scope_intents {
        if attempts.contains(&intent.run_id) && intent.state == IntentState::Pending {
            intent.state = IntentState::Conflict;
        }
    }
    Ok(SupersedeOutcome { generation: revocation_generation, previous_run_id })
}
