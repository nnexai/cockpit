use super::{Applied, MutationCtx};
use super::super::*;
use super::super::store::{DurableEffectReceipt, DurableOperationKind, DurableOperationRecord};
use sha2::{Digest, Sha256};

pub(in crate::orchestration) fn request_digest(action: &OrchestrationAction, session_id: &str) -> Result<String, InspectionError> {
    let bytes = serde_json::to_vec(&(session_id, action)).map_err(|_| error("orchestration_state_invalid", "Cannot encode durable request"))?;
    Ok(format!("{:x}", Sha256::digest(bytes)))
}

pub(in crate::orchestration) fn principal_key(ctx: &MutationCtx<'_>) -> Result<String, InspectionError> {
    let Actor::Agent(agent) = ctx.actor else { return Ok("operator".into()) };
    let credential = ctx.state.credentials.iter().find(|c| c.credential_id == agent.auth.credential_id)
        .ok_or_else(|| error("job_auth_invalid", "Authenticated operation principal is missing"))?;
    // Stable capability incarnation plus pinned assignment; session does not confer authority.
    credential_principal(credential)
}

pub(in crate::orchestration) fn credential_principal(credential: &credentials::JobCredentialRecord) -> Result<String, InspectionError> {
    let bytes = serde_json::to_vec(&(&credential.run_id, &credential.attachment_id,
        credential.attachment_epoch, credential.role, credential.assignment_generation))
        .map_err(|_| error("orchestration_state_invalid", "Cannot encode operation principal"))?;
    Ok(format!("{:x}", Sha256::digest(bytes)))
}

pub(in crate::orchestration) fn replay(ctx: &MutationCtx<'_>, operation_id: &str, kind: DurableOperationKind, action: &OrchestrationAction) -> Result<Option<OrchestrationActionResult>, InspectionError> {
    if matches!(kind, DurableOperationKind::Report | DurableOperationKind::MessageSend) {
        validate_identity(operation_id)?;
    } else {
        tasks_md::validate_uuid(operation_id)?;
    }
    let principal = principal_key(ctx)?;
    let Some(record) = ctx.state.operations.iter().find(|r| r.operation_id == operation_id && r.principal_key == principal) else { return Ok(None) };
    let actor_run_id = ctx.caller.map(|i| ctx.state.runs[i].run_id.as_str());
    if record.kind != kind || record.request_digest != request_digest(action, ctx.session_id)? || record.actor_run_id.as_deref() != actor_run_id {
        return Err(error("operation_conflict", "Operation ID was already used for a different request or actor"));
    }
    let mut result = if let (Some(run_id), Some(resume_operation_id)) = (&record.run_id, &record.resume_operation_id) {
        if let Some(current) = ctx.state.runs.iter().find(|run| &run.run_id == run_id)
            .and_then(|run| run.resume.as_ref()).filter(|resume| &resume.operation_id == resume_operation_id) {
            OrchestrationActionResult::RunResumed { resume: RunResumeResult {
                run_id: run_id.clone(), resume: current.clone(), duplicate: false,
            }}
        } else {
            record.result.clone().ok_or_else(|| error("operation_pending", "The durable operation has no settled receipt"))?
        }
    } else {
        record.result.clone().ok_or_else(|| error("operation_pending", "The durable operation has no completed receipt yet"))?
    };
    match &mut result {
        OrchestrationActionResult::SupervisorStarted { start } => start.duplicate = true,
        OrchestrationActionResult::WorkerStarted { start } | OrchestrationActionResult::WorkerReplaced { start, .. } => start.duplicate = true,
        OrchestrationActionResult::TaskScopeUpdated { update } => update.duplicate = true,
        OrchestrationActionResult::RunResumed { resume } => resume.duplicate = true,
        OrchestrationActionResult::RunAttached { attach } => attach.duplicate = true,
        OrchestrationActionResult::RunRegistered { duplicate, .. }
        | OrchestrationActionResult::TaskUnassigned { duplicate, .. }
        | OrchestrationActionResult::TaskScheduled { duplicate, .. } => *duplicate = true,
        OrchestrationActionResult::IntegrationRecorded { receipt } => receipt.duplicate = true,
        OrchestrationActionResult::Resource { request, duplicate } => {
            *request = ctx.state.resource_requests.iter().find(|r| r.request_id == request.request_id)
                .cloned().ok_or_else(|| error("operation_conflict", "Resource request receipt no longer has its canonical record"))?;
            *duplicate = true;
        }
        OrchestrationActionResult::ResourceRegistered { resource, duplicate } => {
            *resource = ctx.state.owned_resources.iter().find(|r| r.resource_id == resource.resource_id)
                .cloned().ok_or_else(|| error("operation_conflict", "Registered resource receipt has no canonical record"))?;
            *duplicate = true;
        }
        OrchestrationActionResult::CleanupFinished { cleanup } => {
            let items = cleanup.items.iter().map(|item| ctx.state.owned_resources.iter()
                .find(|r| r.resource_id == item.resource_id).cloned()
                .ok_or_else(|| error("operation_conflict", "Cleanup receipt has no canonical resource"))).collect::<Result<Vec<_>, _>>()?;
            *cleanup = super::super::cleanup::finish_result(&cleanup.operation_id, items, true);
        }
        OrchestrationActionResult::Message { duplicate, .. } => *duplicate = true,
        _ => {}
    }
    Ok(Some(result))
}

pub(in crate::orchestration) fn record(ctx: &mut MutationCtx<'_>, operation_id: String, kind: DurableOperationKind, action: &OrchestrationAction, root_id: String, result: OrchestrationActionResult, effect_receipts: Vec<DurableEffectReceipt>) -> Result<(), InspectionError> {
    let timestamp = now();
    let principal_key = principal_key(ctx)?;
    let (run_id, resume_operation_id, result) = match result {
        OrchestrationActionResult::RunResumed { resume } => (
            Some(resume.run_id), Some(resume.resume.operation_id), None,
        ),
        result => (None, None, Some(result)),
    };
    ctx.state.operations.push(DurableOperationRecord {
        operation_id, actor_run_id: ctx.caller.map(|i| ctx.state.runs[i].run_id.clone()), root_id,
        principal_key,
        kind, request_digest: request_digest(action, ctx.session_id)?, run_id, resume_operation_id,
        result, effect_receipts, created_at: timestamp.clone(), updated_at: timestamp,
    });
    Ok(())
}

fn settle_current_resume(state: &mut OrchestrationState, index: usize) {
    let run = &state.runs[index];
    let Some(resume) = &run.resume else { return };
    for operation in &mut state.operations {
        if operation.run_id.as_deref() == Some(run.run_id.as_str())
            && operation.resume_operation_id.as_deref() == Some(resume.operation_id.as_str()) {
            operation.result = Some(OrchestrationActionResult::RunResumed { resume: RunResumeResult {
                run_id: run.run_id.clone(), resume: resume.clone(), duplicate: false,
            }});
            operation.updated_at = now();
        }
    }
}

fn resume_target(ctx: &MutationCtx<'_>, run_id: &str) -> Result<usize, InspectionError> {
    let index = scoped_index(ctx.state, ctx.session_id, run_id)?;
    match ctx.actor {
        Actor::Operator(_) => {}
        Actor::Agent(agent) => {
            let job = credentials::validate_caller(ctx.state, agent)?;
            if job.run_index != index && (job.role != JobRole::Root
                || !is_ancestor(ctx.state, &ctx.state.runs[job.run_index].run_id, run_id)) {
                return Err(error("actor_forbidden", "Only this job or its root may request resume"));
            }
        }
    }
    Ok(index)
}


fn effects(run: &Run) -> Vec<DurableEffectReceipt> {
    let mut receipts = Vec::new();
    if let (Some(attachment), Some(dispatch)) = (&run.attachment, &run.dispatch) {
        receipts.push(DurableEffectReceipt::Launch { attachment_id: attachment.attachment_id.clone(), attachment_epoch: attachment.epoch, dispatch: dispatch.clone() });
    }
    if let Some(setup) = &run.setup { receipts.push(DurableEffectReceipt::Setup { setup: setup.clone() }); }
    if let Some(retirement) = &run.retirement { receipts.push(DurableEffectReceipt::Stop { retirement: retirement.clone() }); }
    receipts
}

fn startup_registration_pending(state: &OrchestrationState, run: &Run) -> bool {
    let (Some(resume), Some(attachment), Some(dispatch)) = (&run.resume, &run.attachment, &run.dispatch) else { return false };
    if run.stage != RunStage::Starting || resume.phase != ResumePhase::Launching
        || resume.run_id != run.run_id || resume.replacement_attachment_epoch != Some(attachment.epoch)
        || resume.previous_attachment_epoch.is_none_or(|previous|
            previous == 0 || previous.checked_add(1) != Some(attachment.epoch))
        || matches!(attachment.state, AttachmentState::Quiescing | AttachmentState::Revoked)
        || dispatch.error.is_some()
        || !matches!(dispatch.latch_state, Some(AgentLatchState::Awaiting | AgentLatchState::ReleaseIntent | AgentLatchState::Released))
        || !matches!(dispatch.step, DispatchStep::LaunchIntent |
            DispatchStep::MoveIntent | DispatchStep::Placed | DispatchStep::LaunchPending | DispatchStep::Launched)
        || state.credentials.iter().any(|credential| credential.run_id == run.run_id
            && credential.state == credentials::CredentialState::Active
            && (credential.attachment_id != attachment.attachment_id || credential.attachment_epoch != attachment.epoch))
        || !state.operations.iter().any(|operation| operation.operation_id == resume.operation_id
            && operation.kind == DurableOperationKind::RunResume && operation.run_id.as_ref() == Some(&run.run_id)) {
        return false;
    }
    run.kind != RunKind::Worker || run.checkout_claim_id.as_ref().is_some_and(|claim_id|
        state.checkout_claims.iter().any(|claim| &claim.claim_id == claim_id
            && matches!(claim.state, claims::ClaimState::Reserved | claims::ClaimState::Held)
            && claim.transition_operation_id.as_ref() == Some(&resume.operation_id)
            && claim.holder.as_ref().is_some_and(|holder| holder.run_id == run.run_id
                && holder.attachment_id == attachment.attachment_id && holder.attachment_epoch == attachment.epoch)))
}

/// A reserved replacement refused before any physical launch effect.
pub fn effect_free_reserved_resume_launch(run: &Run) -> bool {
    let (Some(resume), Some(attachment), Some(dispatch)) = (&run.resume, &run.attachment, &run.dispatch) else { return false };
    run.stage == RunStage::Starting
        && resume.phase == ResumePhase::Launching
        && resume.run_id == run.run_id
        && resume.replacement_attachment_epoch == Some(attachment.epoch)
        && resume.previous_attachment_epoch.is_some_and(|previous|
            previous > 0 && previous.checked_add(1) == Some(attachment.epoch))
        && attachment.state == AttachmentState::Reserved
        && dispatch.error.is_some()
        && attachment.location.is_none()
        && attachment.process.is_none()
        && attachment.shell.is_none()
        && attachment.process_scope.is_none()
        && dispatch.step == DispatchStep::NeedsReview
        && dispatch.launch_attempt > 0
        && dispatch.launch_attempt == attachment.launch_attempt
        && !dispatch.agent_started
        && dispatch.launch_tag.is_none()
        && dispatch.endpoint_identity.is_none()
        && dispatch.scope_unit.is_none()
        && dispatch.pre_move_scope.is_none()
        && dispatch.latch.is_none()
        && dispatch.latch_state.is_none()
        && dispatch.source_location.is_none()
        && dispatch.move_operation_id.is_none()
        && dispatch.staging_workspace_id.is_none()
        && dispatch.recovery.is_none()
}

/// Only explicit Start from the retained Space may re-select this root checkout.
pub fn root_checkout_reselection_pending(run: &Run) -> bool {
    matches!(run.kind, RunKind::Supervisor | RunKind::Adopted)
        && run.run_id == run.root_id
        && run.parent_run_id.is_none()
        && run.checkout_claim_id.is_none()
        && run.checkout.is_some()
        && run.setup.is_some()
        && effect_free_reserved_resume_launch(run)
        && run.dispatch.as_ref().and_then(|dispatch| dispatch.error.as_ref())
            .is_some_and(|error| error.code == "checkout_identity_changed")
}

fn resume_operation_open(state: &OrchestrationState, run: &Run) -> bool {
    let (Some(resume), Some(attachment)) = (&run.resume, &run.attachment) else { return false };
    state.operations.iter().any(|operation| operation.operation_id == resume.operation_id
        && operation.kind == DurableOperationKind::RunResume
        && operation.run_id.as_ref() == Some(&run.run_id) && operation.result.is_none())
        && !state.credentials.iter().any(|credential| credential.run_id == run.run_id
            && credential.state == credentials::CredentialState::Active
            && (credential.attachment_id != attachment.attachment_id || credential.attachment_epoch != attachment.epoch))
}

fn retryable_unsupported_resume_launch(state: &OrchestrationState, run: &Run) -> bool {
    effect_free_reserved_resume_launch(run)
        && dispatch::resume_supported(&run.launch_config)
        && run.attachment.as_ref().is_some_and(|attachment| attachment.conversation.is_some())
        && run.dispatch.as_ref().and_then(|dispatch| dispatch.error.as_ref())
            .is_some_and(|error| error.code == "harness_resume_unsupported")
        && resume_operation_open(state, run)
}

fn requeue_effect_free_launch(run: &mut Run) {
    let dispatch = run.dispatch.as_mut().expect("effect-free dispatch checked");
    dispatch.step = DispatchStep::SetupPending;
    dispatch.error = None;
    dispatch.updated_at = now();
    run.updated_at = dispatch.updated_at.clone();
}

fn reselect_root_checkout(ctx: &mut MutationCtx<'_>, index: usize, fresh: &CheckoutIdentity) -> Result<(), InspectionError> {
    operator(ctx.actor)?;
    let run = &ctx.state.runs[index];
    if !root_checkout_reselection_pending(run) || !resume_operation_open(ctx.state, run)
        || ctx.state.checkout_claims.iter().any(|claim| claim.state != claims::ClaimState::Released
            && claim.holder.as_ref().is_some_and(|holder| holder.run_id == run.run_id)) {
        return Err(error("checkout_reselection_unavailable", "Only a claim-free root waiting on a changed checkout can be re-selected"));
    }
    let saved = run.checkout.as_ref().expect("pending checkout checked");
    if fresh.path != saved.path || fresh.key != saved.key || fresh.git != saved.git {
        return Err(error("checkout_reselection_identity_mismatch", "Re-selection must retain the same canonical checkout path, key and Git directories"));
    }
    let project = ctx.resolved.and_then(|resolved| resolved.project.as_ref());
    if project.is_none() || project.map(|project| &project.key) != run.project.as_ref().map(|project| &project.key) {
        return Err(error("project_identity_conflict", "Re-selection must retain the root project"));
    }
    let workspace_id = run.setup.as_ref().and_then(|setup| setup.workspace_id.as_ref());
    if workspace_id.is_none() || project.and_then(|project| project.source_workspace_id.as_ref()) != workspace_id {
        return Err(error("checkout_reselection_space_mismatch", "Re-selection must originate in the root's retained Space"));
    }
    project_identity::revalidate_checkout(fresh)?;
    let run = &mut ctx.state.runs[index];
    run.checkout = Some(fresh.clone());
    requeue_effect_free_launch(run);
    Ok(())
}

pub(super) fn resume(ctx: &mut MutationCtx<'_>, action: OrchestrationAction) -> Result<Applied, InspectionError> {
    resume_inner(ctx, action, false)
}

pub(super) fn resume_from_start(ctx: &mut MutationCtx<'_>, action: OrchestrationAction) -> Result<Applied, InspectionError> {
    resume_inner(ctx, action, true)
}

fn resume_inner(ctx: &mut MutationCtx<'_>, action: OrchestrationAction, from_start: bool) -> Result<Applied, InspectionError> {
    let OrchestrationAction::RunResume { run_id, operation_id, expected_attachment_epoch } = &action else { unreachable!() };
    let index = resume_target(ctx, run_id)?;
    if let Some(result) = replay(ctx, operation_id, DurableOperationKind::RunResume, &action)? {
        return Ok(Applied::Document(result));
    }
    if ctx.state.runs[index].stage == RunStage::Closed {
        return Err(error("invalid_stage", "Closed history is not reopened by Resume"));
    }
    if ctx.state.runs[index].kind == RunKind::Worker {
        let run = &ctx.state.runs[index];
        let document = ctx.locked.tasks(&run.root_id)?;
        super::tasks::task_transition_target(ctx.state, run, &document)?;
        if ctx.resolved.is_some_and(|resolved| resolved.writer_exit_proof.is_some()) {
            let task = document.task(run.task_id.as_deref().ok_or_else(|| error("task_not_found", "Worker task is missing"))?)?;
            ctx.state.runs[index].task_revision_at_start = Some(task.task_revision.clone());
        }
    }
    let old_epoch = ctx.state.runs[index].attachment.as_ref().map(|a| a.epoch);
    if expected_attachment_epoch.is_some() && *expected_attachment_epoch != old_epoch {
        return Err(error("attachment_conflict", "Attachment epoch changed before Resume"));
    }
    // New requests join an unsettled owner operation instead of starting another effect.
    let root_id = ctx.state.runs[index].root_id.clone();
    if let Some(current) = ctx.state.runs[index].resume.as_ref().filter(|r|
        matches!(r.phase, ResumePhase::Reconciling | ResumePhase::Launching)) {
        let result = OrchestrationActionResult::RunResumed { resume: RunResumeResult { run_id: run_id.clone(), resume: current.clone(), duplicate: false } };
        if let Some(fresh) = ctx.resolved.and_then(|resolved| resolved.reselected_checkout.as_ref()).filter(|_| from_start) {
            reselect_root_checkout(ctx, index, fresh)?;
        } else if retryable_unsupported_resume_launch(ctx.state, &ctx.state.runs[index]) {
            requeue_effect_free_launch(&mut ctx.state.runs[index]);
        }
        record(ctx, operation_id.clone(), DurableOperationKind::RunResume, &action, root_id, result.clone(), Vec::new())?;
        return Ok(Applied::Machine(result));
    }
    settle_current_resume(ctx.state, index);
    let resume = ResumeRecord { operation_id: operation_id.clone(), run_id: run_id.clone(), previous_attachment_epoch: old_epoch,
        phase: ResumePhase::Reconciling, cause: None, decision_id: None, replacement_attachment_epoch: None, updated_at: now() };
    ctx.state.runs[index].resume = Some(resume.clone());
    record(ctx, operation_id.clone(), DurableOperationKind::RunResume, &action, root_id,
        OrchestrationActionResult::RunResumed { resume: RunResumeResult {
            run_id: run_id.clone(), resume, duplicate: false,
        }}, Vec::new())?;
    apply_observation(ctx.state, index, operation_id, ctx.resolved)?;
    Ok(Applied::Machine(OrchestrationActionResult::RunResumed { resume: RunResumeResult {
        run_id: run_id.clone(), resume: ctx.state.runs[index].resume.clone().expect("resume installed"), duplicate: false,
    }}))
}

pub(super) fn apply_observation(state: &mut OrchestrationState, index: usize, operation_id: &str, resolved: Option<&ResolvedMutationInput>) -> Result<(), InspectionError> {
    if state.runs[index].resume.as_ref().is_none_or(|r| r.operation_id != operation_id) {
        return Err(error("operation_conflict", "Resume observation does not match the active operation"));
    }
    if state.runs[index].resume.as_ref().is_some_and(|r| r.phase == ResumePhase::Launching)
        && resolved.is_none_or(|r| r.attachment_observation.is_none()) {
        return Ok(());
    }
    let observed = resolved.and_then(|r| r.attachment_observation.as_ref());
    if let Some(observed) = observed {
        let current = state.runs[index].attachment.as_ref().ok_or_else(|| error("attachment_conflict", "No current attachment to reattach"))?;
        if current.state == AttachmentState::Quiescing {
            return Ok(());
        }
        if observed.attachment_id != current.attachment_id || observed.epoch != current.epoch
            || observed.process != current.process || observed.shell != current.shell
            || observed.conversation != current.conversation || observed.launch_attempt != current.launch_attempt
            || observed.launch_mode != current.launch_mode
            || current.machine_id.as_ref().is_some_and(|m| observed.machine_id.as_ref() != Some(m))
            || observed.machine_id.is_none() || observed.process.is_none()
            || observed.state != AttachmentState::Connected {
            return Err(error("attachment_conflict", "Observation must identify the same live local incarnation"));
        }
        if current.machine_id.is_none()
            && observed.process.as_ref().and_then(crate::process_identity::verified_machine_id) != observed.machine_id {
            return Err(error("attachment_conflict", "Missing machine metadata requires freshly verified local process evidence"));
        }
        if state.runs[index].kind != RunKind::Worker
            && (resolved.and_then(|input| input.root_attachment_expected.as_ref()).is_some_and(|expected|
                !project_identity::same_root_attachment(current, expected))
                || (!project_identity::same_location(current.location.as_ref(), observed.location.as_ref())
                    && resolved.and_then(|input| input.root_attachment_expected.as_ref()).is_none())) {
            return Err(error("attachment_conflict", "Root location changed during the physical observation"));
        }
        if let Some(expected) = resolved.and_then(|input| input.root_attachment_expected.as_ref()) {
            project_identity::revalidate_root_observation(state, &state.runs[index], expected)?;
        }
        // A live scoped FIFO helper is physical presence, not a failed semantic
        // registration. Preserve the already-authorized replacement operation
        // while placement/exec and the credentialed claim are still in flight.
        if startup_registration_pending(state, &state.runs[index])
            && !has_current_binding(state, &state.runs[index]) {
            return Ok(());
        }
        if state.runs[index].kind != RunKind::Worker
            && !project_identity::same_location(current.location.as_ref(), observed.location.as_ref())
            && let Some(dispatch) = state.runs[index].dispatch.as_mut() {
            dispatch.endpoint_identity = observed.location.as_ref().map(|location| location.endpoint_identity.clone());
            dispatch.updated_at = now();
        }
        state.runs[index].attachment = Some(observed.clone());
        if state.runs[index].project.is_none() {
            state.runs[index].project = resolved.and_then(|r| r.project.clone());
        }
        if !has_current_binding(state, &state.runs[index]) {
            state.runs[index].attachment.as_mut().expect("observed attachment").state = AttachmentState::Disconnected;
            return block(state, index, "run_unregistered",
                "The live incarnation is preserved but its current private capability has not claimed the attempt");
        }
        let resume = state.runs[index].resume.as_mut().expect("matched resume");
        resume.phase = if resume.replacement_attachment_epoch == Some(observed.epoch) {
            ResumePhase::Connected
        } else {
            ResumePhase::Reattached
        };
        resume.cause = None;
        resume.updated_at = now();
        return Ok(());
    }
    let writer_proof = resolved.and_then(|r| r.writer_exit_proof.as_ref());
    let root_proof = resolved.and_then(|r| r.root_main_recovery.as_ref());
    if writer_proof.is_some() || root_proof.is_some() {
        if state.runs[index].kind == RunKind::Worker {
            let proof = writer_proof.ok_or_else(|| error("writer_exit_unproven", "Root-main proof cannot transfer a worker claim"))?;
            if proof.incarnation.run_id != state.runs[index].run_id || root_proof.is_some() {
                return Err(error("writer_exit_unproven", "Exit proof belongs to another job"));
            }
            if let Some(checkout) = proof.checkout.as_ref() {
                project_identity::revalidate_checkout(checkout)?;
            }
        } else if root_proof.is_none() || writer_proof.is_some() {
            return Err(error("root_main_exit_unproven", "Root-main recovery requires its dedicated physical observation"));
        }
        if state.runs[index].dispatch.as_ref().is_some_and(|d| matches!(d.step, DispatchStep::SetupUnknown | DispatchStep::SetupRunning)) {
            return block(state, index, "resume_effect_unknown", "Unsettled setup must be reconciled before replacement");
        }
        let current = state.runs[index].attachment.as_ref().ok_or_else(|| error("writer_exit_unproven", "No recorded incarnation to replace"))?;
        if current.conversation.is_some() && !dispatch::resume_supported(&state.runs[index].launch_config) {
            return block(state, index, "harness_resume_unsupported", "The configured harness has no same-conversation resume arguments; the existing attachment is preserved");
        }
        let receipts = effects(&state.runs[index]);
        let epoch = current.epoch.checked_add(1).filter(|n| *n <= MAX_SAFE_COUNTER).ok_or_else(|| error("orchestration_state_full", "Attachment epoch exhausted"))?;
        let launch_attempt = state.runs[index].dispatch.as_ref().map(|d| d.launch_attempt).unwrap_or(current.launch_attempt).checked_add(1)
            .ok_or_else(|| error("orchestration_state_full", "Launch attempt exhausted"))?;
        let mut replacement = reserved_attachment(epoch, launch_attempt);
        replacement.conversation = current.conversation.clone();
        if state.runs[index].kind == RunKind::Worker {
            let claim_id = state.runs[index].checkout_claim_id.clone().ok_or_else(|| error("writer_exit_unproven", "Worker checkout is not reconciled"))?;
            claims::revoke_and_reserve_replacement(state, &claim_id, writer_proof.expect("worker proof validated"), replacement, operation_id)?;
        } else {
            claims::revoke_root_and_reserve_replacement(state, root_proof.expect("root proof validated"), replacement, operation_id)?;
            if state.runs[index].kind == RunKind::Adopted && state.runs[index].dispatch.is_none() {
                // The observer verified the retained physical checkout and
                // existing Space before revocation. No setup/ownership effect
                // is invented; the ordinary dispatcher opens a new liaison tab.
                let proof = root_proof.expect("root proof validated");
                let checkout = proof.retained_checkout.as_ref().expect("adopted checkout revalidated");
                let location = proof.attachment.location.as_ref().expect("root location revalidated");
                let run = &mut state.runs[index];
                run.target = Some(DispatchTarget::ExistingSpace { workspace_id: location.workspace_id.clone() });
                run.setup = Some(SetupSummary {
                    operation_id: None, generation: None, workspace_id: Some(location.workspace_id.clone()),
                    checkout_path: checkout.path.clone(), repository_id: run.project.as_ref().and_then(|project| project.repository_id.clone()),
                    branch: None, base: None, base_commit: None, ownership: None,
                    effects: Vec::new(), warnings: Vec::new(), project_workspace_id: None,
                });
                let mut previous = dispatch(DispatchStep::Launched);
                previous.launch_attempt = proof.attachment.launch_attempt;
                run.dispatch = Some(previous);
            }
        }
        if let Some(operation) = state.operations.iter_mut().find(|record| record.operation_id == operation_id) {
            operation.effect_receipts.extend(receipts);
        }
        let resume = state.runs[index].resume.as_mut().expect("matched resume");
        resume.phase = ResumePhase::Launching;
        resume.replacement_attachment_epoch = Some(epoch);
        resume.cause = None;
        resume.updated_at = now();
        super::reviewed::reset_launch_metadata_after_resume(state, index)?;
        return Ok(());
    }
    if let Some(cause) = resolved.and_then(|r| r.observation_cause.as_ref()) {
        block(state, index, &cause.code, &cause.message)?;
        return Ok(());
    }
    block(state, index, "writer_exit_unproven", "The saved writer remains protected; reconnect or obtain verified quiescence")
}

fn block(state: &mut OrchestrationState, index: usize, code: &str, message: &str) -> Result<(), InspectionError> {
    let resume = state.runs[index].resume.as_mut().expect("resume installed");
    resume.phase = ResumePhase::Blocked;
    resume.cause = Some(ResumeCause { code: code.into(), message: message.into(), class: ResumeCauseClass::Recover });
    resume.decision_id = None;
    resume.updated_at = now();
    Ok(())
}

pub(super) fn attach(ctx: &mut MutationCtx<'_>, action: OrchestrationAction) -> Result<Applied, InspectionError> {
    operator(ctx.actor)?;
    let OrchestrationAction::RunAttach { operation_id, attachment_id, expected_attachment_epoch, conversation } = &action else { unreachable!() };
    if let Some(result) = replay(ctx, operation_id, DurableOperationKind::RunAttach, &action)? { return Ok(Applied::Document(result)); }
    let index = ctx.state.runs.iter().position(|run| run.session_id == ctx.session_id
        && run.attachment.as_ref().is_some_and(|a| a.attachment_id == *attachment_id && a.epoch == *expected_attachment_epoch))
        .ok_or_else(|| error("attachment_conflict", "Physical attachment epoch changed"))?;
    let run = &ctx.state.runs[index];
    let current = run.attachment.as_ref().expect("selected attachment");
    let observed = ctx.resolved.and_then(|r| r.attachment_observation.as_ref())
        .ok_or_else(|| error("attachment_observation_unavailable", "Physical attach requires fresh exact evidence"))?;
    if current.conversation.as_ref() != Some(conversation)
        || observed.conversation != current.conversation || observed.attachment_id != current.attachment_id
        || observed.epoch != current.epoch || observed.process != current.process
        || observed.shell != current.shell || observed.machine_id != current.machine_id
        || observed.launch_mode != current.launch_mode || observed.launch_attempt != current.launch_attempt
        || observed.state != AttachmentState::Connected || run.stage == RunStage::Closed {
        return Err(error("attachment_conflict", "Physical observation differs from the registered incarnation"));
    }
    let root_id = run.root_id.clone();
    let run_id = run.run_id.clone();
    ctx.state.runs[index].attachment = Some(observed.clone());
    let result = OrchestrationActionResult::RunAttached { attach: RunAttachResult {
        run_id, attachment: observed.clone(), duplicate: false,
    }};
    record(ctx, operation_id.clone(), DurableOperationKind::RunAttach, &action, root_id, result.clone(), Vec::new())?;
    Ok(Applied::Machine(result))
}

impl OrchestrationService {
    /// Operator-only bootstrap transport, scoped to the exact borrowed placement.
    pub fn job_adoption_credential_file(&self, actor: &Actor, session_id: &str, run_id: &str, location: &RunLocation) -> Result<std::path::PathBuf, InspectionError> {
        operator(actor)?;
        let state = self.store.lock()?.read_shared()?;
        let run = &state.runs[scoped_index(&state, session_id, run_id)?];
        let attachment = run.attachment.as_ref().ok_or_else(|| error("attachment_conflict", "Adopted attachment missing"))?;
        if run.kind != RunKind::Adopted || run.stage == RunStage::Closed
            || !project_identity::same_location(attachment.location.as_ref(), Some(location)) {
            return Err(error("attachment_conflict", "Adoption credential requires the exact current placement"));
        }
        let record = state.credentials.iter().find(|c| c.run_id == run.run_id && c.role == JobRole::Root
            && c.attachment_id == attachment.attachment_id && c.attachment_epoch == attachment.epoch
            && c.state == credentials::CredentialState::Active)
            .ok_or_else(|| error("job_auth_invalid", "Adopted credential is not active"))?;
        credentials::credential_path(self.base(), record)
    }

    /// Owner-only observations advance the existing operation, never issue a second launch.
    pub fn advance_resume_observed(&self, run_id: &str, operation_id: &str, resolved: &ResolvedMutationInput) -> Result<RunResumeResult, InspectionError> {
        let locked = self.store.lock()?;
        let mut state = locked.read()?;
        let index = run_index(&state, run_id)?;
        if !state.operations.iter().any(|r| r.operation_id == operation_id && r.kind == DurableOperationKind::RunResume) {
            return Err(error("operation_conflict", "Resume operation is not recorded"));
        }
        let current = state.runs[index].resume.as_ref().ok_or_else(|| error("operation_conflict", "No current Resume"))?;
        if current.operation_id != operation_id {
            return Err(error("operation_conflict", "A different Resume operation is current"));
        }
        let same_cause = match (&current.cause, &resolved.observation_cause) {
            (Some(left), Some(right)) => left.code == right.code && left.message == right.message && left.class == right.class,
            (None, None) => true,
            _ => false,
        };
        let stop_pending = state.runs[index].attachment.as_ref().is_some_and(|a| a.state == AttachmentState::Quiescing)
            && resolved.attachment_observation.is_some();
        let unchanged_observation = resolved.attachment_observation.as_ref().is_some_and(|observed|
            state.runs[index].attachment.as_ref().is_some_and(|attachment|
                project_identity::same_root_attachment(attachment, observed))
            && has_current_binding(&state, &state.runs[index]));
        if (matches!(current.phase, ResumePhase::Connected | ResumePhase::Reattached) && unchanged_observation)
            || stop_pending
            || (current.phase == ResumePhase::Launching && resolved.attachment_observation.is_none())
            || (resolved.writer_exit_proof.is_none() && resolved.root_main_recovery.is_none()
                && resolved.attachment_observation.is_none() && same_cause) {
            return Ok(RunResumeResult { run_id: run_id.into(), resume: current.clone(), duplicate: true });
        }
        if resolved.writer_exit_proof.is_some() && state.runs[index].kind == RunKind::Worker {
            let run = &state.runs[index];
            let document = locked.tasks(&run.root_id)?;
            super::tasks::task_transition_target(&state, run, &document)?;
            let task = document.task(run.task_id.as_deref().ok_or_else(|| error("task_not_found", "Worker task is missing"))?)?;
            state.runs[index].task_revision_at_start = Some(task.task_revision.clone());
        }
        let waiting_registration = startup_registration_pending(&state, &state.runs[index]);
        apply_observation(&mut state, index, operation_id, Some(resolved))?;
        if waiting_registration && startup_registration_pending(&state, &state.runs[index]) {
            return Ok(RunResumeResult { run_id: run_id.into(),
                resume: state.runs[index].resume.clone().expect("matched resume"), duplicate: true });
        }
        let result = RunResumeResult { run_id: run_id.into(), resume: state.runs[index].resume.clone().expect("matched resume"), duplicate: false };
        let record = state.operations.iter_mut().find(|r| r.operation_id == operation_id).expect("matched operation");
        record.updated_at = now();
        let revision = locked.save(&mut state)?;
        self.revision.send_replace(revision);
        Ok(result)
    }

    pub(crate) fn resume_startup_pending(&self, expected: &Run) -> Result<bool, InspectionError> {
        let locked = self.store.lock()?;
        let state = locked.read_shared()?;
        let current = &state.runs[scoped_index(&state, &expected.session_id, &expected.run_id)?];
        Ok(current.assignment_generation == expected.assignment_generation
            && current.attachment.as_ref().zip(expected.attachment.as_ref()).is_some_and(|(current, expected)|
                project_identity::same_root_attachment(current, expected))
            && startup_registration_pending(&state, current))
    }

    pub(crate) fn resume_owner(&self, expected: &Run, resolved: &ResolvedMutationInput) -> Result<RunResumeResult, InspectionError> {
        let current = self.run_for_review(&expected.session_id, &expected.run_id)?;
        if current.attachment.as_ref().map(|a| (&a.attachment_id, a.epoch))
            != expected.attachment.as_ref().map(|a| (&a.attachment_id, a.epoch)) {
            return Err(error("attachment_conflict", "Attachment changed during owner observation"));
        }
        if let Some(resume) = &current.resume {
            let settled = matches!(resume.phase, ResumePhase::Connected | ResumePhase::Reattached);
            if !settled || (resolved.writer_exit_proof.is_none() && resolved.root_main_recovery.is_none() && resolved.observation_cause.is_none()) {
                return self.advance_resume_observed(&current.run_id, &resume.operation_id, resolved);
            }
            // A later disconnection belongs to a new owner operation; a
            // completed recovery must not permanently disable future recovery.
        }
        let response = self.mutate_resolved(&Actor::Operator(OperatorOrigin::Native), OrchestrationMutationRequest {
            session_id: current.session_id.clone(), expected_revision: None,
            action: OrchestrationAction::RunResume { run_id: current.run_id.clone(), operation_id: id(),
                expected_attachment_epoch: current.attachment.as_ref().map(|a| a.epoch) },
        }, resolved)?;
        match response.result {
            OrchestrationActionResult::RunResumed { resume } => Ok(resume),
            _ => Err(error("orchestration_state_invalid", "Owner Resume returned a different operation")),
        }
    }
}
