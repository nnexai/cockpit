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
    let duplicate = run.attachment.as_ref().is_some_and(|attachment|
        attachment.conversation.is_some() && attachment.state == AttachmentState::Connected)
        && matches!(run.stage, RunStage::Active | RunStage::Working | RunStage::Reported)
        && ctx.state.credentials.iter().find(|record| record.credential_id == caller.auth.credential_id)
            .and_then(|record| record.conversation.as_ref()) == Some(&caller.conversation);
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
    }
    if let Some(claim_id) = &job.checkout_claim_id {
        let revision = ctx.state.checkout_claims.iter().find(|claim| &claim.claim_id == claim_id).expect("validated claim").revision;
        let holder = claims::AttachmentReference { run_id: run_id.clone(), attachment_id: job.attachment_id.clone(), attachment_epoch: job.attachment_epoch };
        claims::hold_claim(ctx.state, claim_id, &holder, revision)?;
    }
    ctx.state.credentials.iter_mut().find(|record| record.credential_id == caller.auth.credential_id)
        .expect("validated capability").conversation = Some(caller.conversation.clone());
    let run = &mut ctx.state.runs[index];
    let attachment = run.attachment.as_mut().expect("validated attachment");
    attachment.conversation = Some(caller.conversation.clone());
    attachment.state = AttachmentState::Connected;
    attachment.updated_at = now();
    if !duplicate {
        run.stage = if job.role == JobRole::Root { RunStage::Active } else { RunStage::Working };
        run.updated_at = now();
        if let Some(dispatch) = &mut run.dispatch {
            dispatch.step = DispatchStep::Launched;
            dispatch.error = None;
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
    let Actor::Agent(caller) = ctx.actor else { return Err(error("actor_forbidden", "Only the owning supervisor may unassign a task")); };
    let job = credentials::validate_caller(ctx.state, caller)?;
    let root = &ctx.state.runs[job.run_index];
    if job.role != JobRole::Root || root.run_id != *root_id || root.stage != RunStage::Active {
        return Err(error("actor_forbidden", "Only the active owning supervisor may unassign a task"));
    }
    if let Some(result) = super::resume::replay(ctx, operation_id, DurableOperationKind::TaskUnassign, &action)? {
        return Ok(Applied::Document(result));
    }
    if ctx.locked.tasks(root_id)?.task(task_id)?.checked {
        return Err(error("task_accepted", "Accepted tasks remain read-only"));
    }
    if ctx.state.task_intents.iter().any(|intent| intent.root_id == *root_id && intent.task_id == *task_id) {
        return Err(error("intent_conflict", "Resolve the durable acceptance transaction before unassign"));
    }
    let ownership_index = ctx.state.task_ownership.iter().position(|record| record.root_id == *root_id && record.task_id == *task_id)
        .ok_or_else(|| error("task_unassigned", "Task has no authorized owner or pending attempt"))?;
    let ownership = &ctx.state.task_ownership[ownership_index];
    if ownership.generation != *expected_assignment_generation {
        return Err(error("assignment_generation_conflict", "Task assignment changed before unassign"));
    }
    let previous_run_id = ownership.owner.as_ref().map(|owner| owner.run_id.clone());
    let pending_generation = ownership.generation.checked_add(1).ok_or_else(|| error("invalid_counter", "Task generation exhausted"))?;
    let attempts: Vec<String> = ctx.state.runs.iter().filter(|run| run.root_id == *root_id
        && run.task_id.as_deref() == Some(task_id.as_str()) && (Some(&run.run_id) == previous_run_id.as_ref()
            || (run.stage != RunStage::Closed && run.assignment_generation == Some(pending_generation))))
        .map(|run| run.run_id.clone()).collect();
    if attempts.is_empty() { return Err(error("task_unassigned", "Task has no owner or pending authorization")); }
    safe_counter(pending_generation)?;
    let ownership = &mut ctx.state.task_ownership[ownership_index];
    ownership.generation = pending_generation;
    ownership.owner = None;
    for run in &mut ctx.state.runs {
        if attempts.contains(&run.run_id) {
            run.stage = RunStage::Closed;
            run.close_reason = Some(CloseReason::Superseded);
            run.updated_at = now();
            // Logical revocation is deliberately not physical stop or claim release.
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
    let result = OrchestrationActionResult::TaskUnassigned { task_id: task_id.clone(), assignment_generation: pending_generation, previous_run_id, duplicate: false };
    super::resume::record(ctx, operation_id.clone(), DurableOperationKind::TaskUnassign, &action, root_id.clone(), result.clone(), Vec::new())?;
    Ok(Applied::Machine(result))
}
