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
    // Stable job incarnation and subject, never an ephemeral conversation or
    // credential locator. Distinct local children cannot share a receipt/grant.
    let bytes = serde_json::to_vec(&(&credential.run_id, &credential.attachment_id,
        credential.attachment_epoch, credential.role, &credential.subject_child_id))
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
        OrchestrationActionResult::WorkerStarted { start } => start.duplicate = true,
        OrchestrationActionResult::TaskScopeUpdated { update } => update.duplicate = true,
        OrchestrationActionResult::RunResumed { resume } => resume.duplicate = true,
        OrchestrationActionResult::RunAttached { attach } => attach.duplicate = true,
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
            let job = credentials::validate_job_auth(ctx.state, &agent.auth)?;
            if job.role == JobRole::Child || (job.run_index != index && (job.role != JobRole::Root
                || !is_ancestor(ctx.state, &ctx.state.runs[job.run_index].run_id, run_id))) {
                return Err(error("actor_forbidden", "Only this job or its root may request resume"));
            }
        }
        Actor::Bootstrap(_) => return Err(error("actor_forbidden", "Bootstrap cannot resume a job")),
    }
    Ok(index)
}

fn decision_for(state: &OrchestrationState, index: usize) -> String {
    let run = &state.runs[index];
    let attachment = run.attachment.as_ref();
    let revision = run.checkout_claim_id.as_ref().and_then(|id| state.checkout_claims.iter().find(|c| &c.claim_id == id)).map(|c| c.revision);
    let bytes = Sha256::digest(format!("resume-decision:v1:{}:{:?}:{:?}:{revision:?}", run.run_id,
        attachment.map(|a| &a.attachment_id), attachment.map(|a| a.epoch)).as_bytes());
    let mut uuid = [0u8; 16];
    uuid.copy_from_slice(&bytes[..16]);
    Uuid::from_bytes(uuid).to_string()
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

pub(super) fn resume(ctx: &mut MutationCtx<'_>, action: OrchestrationAction) -> Result<Applied, InspectionError> {
    let OrchestrationAction::RunResume { run_id, operation_id, expected_attachment_epoch, mode, decision_id } = &action else { unreachable!() };
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
    if *mode == ResumeMode::ReplaceExisting {
        let expected_decision = decision_for(ctx.state, index);
        if decision_id.as_deref() != Some(expected_decision.as_str())
            || ctx.state.runs[index].resume.as_ref().and_then(|r| r.decision_id.as_ref()) != Some(&expected_decision) {
            return Err(error("resume_decision_conflict", "Replace requires the current attachment and claim decision"));
        }
    } else if decision_id.is_some() {
        return Err(error("resume_decision_conflict", "A replacement decision is valid only for Replace"));
    }
    // New requests join an unsettled owner operation instead of starting another effect.
    let root_id = ctx.state.runs[index].root_id.clone();
    if let Some(current) = ctx.state.runs[index].resume.as_ref().filter(|r|
        matches!(r.phase, ResumePhase::Reconciling | ResumePhase::Launching)) {
        let result = OrchestrationActionResult::RunResumed { resume: RunResumeResult { run_id: run_id.clone(), resume: current.clone(), duplicate: false } };
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
    apply_observation(ctx.state, index, operation_id, *mode, ctx.resolved)?;
    Ok(Applied::Machine(OrchestrationActionResult::RunResumed { resume: RunResumeResult {
        run_id: run_id.clone(), resume: ctx.state.runs[index].resume.clone().expect("resume installed"), duplicate: false,
    }}))
}

pub(super) fn apply_observation(state: &mut OrchestrationState, index: usize, operation_id: &str, mode: ResumeMode, resolved: Option<&ResolvedMutationInput>) -> Result<(), InspectionError> {
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
        state.runs[index].attachment = Some(observed.clone());
        if state.runs[index].project.is_none() {
            state.runs[index].project = resolved.and_then(|r| r.project.clone());
        }
        if !has_current_binding(state, &state.runs[index]) {
            state.runs[index].attachment.as_mut().expect("observed attachment").state = AttachmentState::Disconnected;
            return block(state, index, "current_extension_unavailable",
                "The live incarnation is preserved, but no current-epoch credential-authenticated attachment proves a compatible loaded extension", false);
        }
        let resume = state.runs[index].resume.as_mut().expect("matched resume");
        resume.phase = ResumePhase::Reattached;
        resume.cause = None;
        resume.updated_at = now();
        return Ok(());
    }
    let proof = resolved.and_then(|r| r.writer_exit_proof.as_ref());
    if let Some(proof) = proof {
        if proof.incarnation.run_id != state.runs[index].run_id {
            return Err(error("writer_exit_unproven", "Exit proof belongs to another job"));
        }
        if let Some(checkout) = proof.checkout.as_ref() {
            project_identity::revalidate_checkout(checkout)?;
        }
        if state.runs[index].dispatch.as_ref().is_some_and(|d| matches!(d.step, DispatchStep::SetupUnknown | DispatchStep::SetupRunning)) {
            return block(state, index, "resume_effect_unknown", "Unsettled setup must be reconciled before replacement", false);
        }
        let current = state.runs[index].attachment.as_ref().ok_or_else(|| error("writer_exit_unproven", "No recorded incarnation to replace"))?;
        let receipts = effects(&state.runs[index]);
        let epoch = current.epoch.checked_add(1).filter(|n| *n <= MAX_SAFE_COUNTER).ok_or_else(|| error("orchestration_state_full", "Attachment epoch exhausted"))?;
        let launch_attempt = state.runs[index].dispatch.as_ref().map(|d| d.launch_attempt).unwrap_or(0).checked_add(1)
            .ok_or_else(|| error("orchestration_state_full", "Launch attempt exhausted"))?;
        let replacement = reserved_attachment(epoch, launch_attempt);
        if state.runs[index].kind == RunKind::Worker {
            let claim_id = state.runs[index].checkout_claim_id.clone().ok_or_else(|| error("writer_exit_unproven", "Worker checkout is not reconciled"))?;
            claims::revoke_and_reserve_replacement(state, &claim_id, proof, replacement, operation_id)?;
        } else {
            claims::revoke_root_and_reserve_replacement(state, proof, replacement, operation_id)?;
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
    if mode == ResumeMode::ReplaceExisting {
        return block(state, index, "same_boot_replacement_unsupported",
            "This OMP runtime cannot prove all same-boot checkout consumers stopped; the current attachment and claim are preserved", true);
    }
    if let Some(cause) = resolved.and_then(|r| r.observation_cause.as_ref()) {
        let decision = cause.class == ResumeCauseClass::Decision;
        block(state, index, &cause.code, &cause.message, decision)?;
        return Ok(());
    }
    block(state, index, "writer_exit_unproven", "The saved writer remains protected; reconnect or obtain verified quiescence", false)
}

fn block(state: &mut OrchestrationState, index: usize, code: &str, message: &str, decision: bool) -> Result<(), InspectionError> {
    let decision_id = decision.then(|| decision_for(state, index));
    let resume = state.runs[index].resume.as_mut().expect("resume installed");
    resume.phase = ResumePhase::Blocked;
    resume.cause = Some(ResumeCause { code: code.into(), message: message.into(), class: if decision { ResumeCauseClass::Decision } else { ResumeCauseClass::Recover } });
    resume.decision_id = decision_id;
    resume.updated_at = now();
    Ok(())
}

pub(super) fn attach(ctx: &mut MutationCtx<'_>, action: OrchestrationAction) -> Result<Applied, InspectionError> {
    let OrchestrationAction::RunAttach { operation_id, attachment_id, expected_attachment_epoch, main_session_id } = &action else { unreachable!() };
    let Actor::Agent(agent) = ctx.actor else { return Err(error("actor_forbidden", "Attachment registration requires a job credential")) };
    let job = credentials::validate_job_auth(ctx.state, &agent.auth)?;
    if job.role == JobRole::Child { return Err(error("report_requires_main", "A child cannot attach its parent job")) }
    if let Some(result) = replay(ctx, operation_id, DurableOperationKind::RunAttach, &action)? { return Ok(Applied::Document(result)); }
    validate_identity(main_session_id)?;
    if job.attachment_id != *attachment_id || job.attachment_epoch != *expected_attachment_epoch {
        return Err(error("attachment_conflict", "Attachment registration does not match the current job epoch"));
    }
    let run = &ctx.state.runs[job.run_index];
    if dispatch::recovery_incarnation_revoked(run) { return Err(error("attempt_stale", "This launch was revoked")); }
    let current = run.attachment.as_ref().expect("authorized attachment");
    let observed = ctx.resolved.and_then(|r| r.attachment_observation.as_ref());
    let process = observed.and_then(|a| a.process.as_ref()).or(agent.process.as_ref());
    if agent.agent_kind != Some(AgentKind::Main) || agent.subagent_id.is_some()
        || agent.actual_agent_kind.as_ref() != Some(&NativeAgentKind::Omp)
        || agent.omp_session_id.as_deref() != Some(main_session_id.as_str()) || process.is_none()
        || current.process.as_ref().is_some_and(|p| process != Some(p))
        || observed.is_some_and(|a| a.attachment_id != *attachment_id || a.epoch != *expected_attachment_epoch
            || a.shell != current.shell || a.machine_id != current.machine_id
            || a.launch_mode != current.launch_mode) {
        return Err(error("attachment_conflict", "A different or unobserved process requires owner Resume proof"));
    }
    let process = process.cloned();
    let run_id = run.run_id.clone();
    let root_id = run.root_id.clone();
    let attachment = ctx.state.runs[job.run_index].attachment.as_mut().expect("authorized attachment");
    if let Some(observed) = observed { attachment.location = observed.location.clone(); }
    attachment.process = process;
    if attachment.machine_id.is_none() {
        attachment.machine_id = attachment.process.as_ref().and_then(crate::process_identity::verified_machine_id);
    }
    attachment.main_session_id = Some(main_session_id.clone());
    attachment.updated_at = now();
    // Dispatch launch acknowledgement alone must not become connected/Working.
    if ctx.state.runs[job.run_index].dispatch.as_ref().is_some_and(|d| d.agent_started && d.step == DispatchStep::Launched)
        || ctx.state.runs[job.run_index].kind == RunKind::Adopted {
        ctx.state.runs[job.run_index].attachment.as_mut().expect("attachment").state = AttachmentState::Connected;
    } else {
        ctx.state.runs[job.run_index].attachment.as_mut().expect("attachment").state = AttachmentState::Connecting;
    }
    if let Some(claim_id) = &job.checkout_claim_id {
        let claim = ctx.state.checkout_claims.iter().find(|c| &c.claim_id == claim_id).expect("authorized claim");
        let revision = claim.revision;
        let holder = claims::AttachmentReference { run_id: run_id.clone(), attachment_id: attachment_id.clone(), attachment_epoch: *expected_attachment_epoch };
        claims::hold_claim(ctx.state, claim_id, &holder, revision)?;
    }
    let result = OrchestrationActionResult::RunAttached { attach: RunAttachResult { run_id, attachment: ctx.state.runs[job.run_index].attachment.clone().expect("attachment"), duplicate: false } };
    record(ctx, operation_id.clone(), DurableOperationKind::RunAttach, &action, root_id, result.clone(), Vec::new())?;
    let run = &mut ctx.state.runs[job.run_index];
    let initial = run.attachment.as_ref().is_some_and(|attachment| attachment.state == AttachmentState::Connected)
        && matches!(run.stage, RunStage::Starting | RunStage::Queued);
    if initial {
        run.stage = if run.kind == RunKind::Worker { RunStage::Working } else { RunStage::Active };
    }
    // The private binding resolves only the missing-bridge diagnostic, not
    // arbitrary setup, launch or physical-owner failures.
    let binding_recovered = run.attachment.as_ref().is_some_and(|attachment| attachment.state == AttachmentState::Connected)
        && run.dispatch.as_ref().is_some_and(|dispatch| dispatch.step == DispatchStep::Launched && dispatch.agent_started
            && dispatch.error.as_ref().is_some_and(|error| matches!(error.code.as_str(),
                "current_extension_unavailable" | "omp_bridge_unbound")));
    if binding_recovered {
        let dispatch = run.dispatch.as_mut().expect("proven launch binding");
        dispatch.error = None;
        dispatch.updated_at = now();
        run.updated_at = dispatch.updated_at.clone();
    }
    if run.attachment.as_ref().is_some_and(|a| a.state == AttachmentState::Connected)
        && run.resume.as_ref().is_some_and(|r| r.phase == ResumePhase::Blocked
            && r.cause.as_ref().is_some_and(|c| c.code == "current_extension_unavailable")) {
        let resume = run.resume.as_mut().expect("matched blocked binding");
        resume.phase = if resume.replacement_attachment_epoch == Some(*expected_attachment_epoch) {
            ResumePhase::Connected
        } else {
            ResumePhase::Reattached
        };
        resume.cause = None;
        resume.updated_at = now();
    }
    if initial {
        let kind = if run.kind == RunKind::Worker { MessageKind::WorkBrief } else { MessageKind::SupervisorBrief };
        if run.parent_run_id.is_none() && matches!(run.kind, RunKind::Supervisor | RunKind::Adopted) {
            run.brief = supervisor_guidance().into();
        }
        let text = run.brief.clone();
        let run_id = run.run_id.clone();
        let phase = format!("launch-{}-{}", run.attempt,
            run.dispatch.as_ref().map(|dispatch| dispatch.launch_attempt).unwrap_or(0));
        brief(ctx.state, &run_id, kind, &text, &phase)?;
    }
    if initial || binding_recovered {
        escalation::recovered(ctx.state, job.run_index)?;
    }
    Ok(Applied::Machine(result))
}

pub(super) fn register_child(ctx: &mut MutationCtx<'_>, action: OrchestrationAction) -> Result<Applied, InspectionError> {
    let OrchestrationAction::JobChildRegister { operation_id, child_id, child_session_id, parent_child_id } = &action else { unreachable!() };
    let Actor::Agent(agent) = ctx.actor else { return Err(error("actor_forbidden", "Child registration requires a main job credential")) };
    let job = credentials::validate_job_auth(ctx.state, &agent.auth)?;
    if job.role != JobRole::Worker { return Err(error("actor_forbidden", "Only a worker main credential can narrow a local child")) }
    if let Some(result) = replay(ctx, operation_id, DurableOperationKind::JobChildRegister, &action)? { return Ok(Applied::Document(result)); }
    validate_identity(child_id)?;
    validate_identity(child_session_id)?;
    let run = &ctx.state.runs[job.run_index];
    let attachment = run.attachment.as_ref().expect("authorized attachment");
    if agent.agent_kind != Some(AgentKind::Subagent) || agent.subagent_id.as_ref() != Some(child_id)
        || agent.omp_session_id.as_ref() != Some(child_session_id)
        || agent.main_omp_session_id != attachment.main_session_id
        || agent.actual_agent_kind.as_ref() != Some(&NativeAgentKind::Omp)
        || agent.process != attachment.process || attachment.process.is_none() {
        return Err(error("child_identity_unproven", "Registration requires actual native child provenance under this main incarnation"));
    }
    if let Some(parent) = parent_child_id {
        if parent == child_id || !ctx.state.subagents.iter().any(|c| c.run_id == run.run_id && c.subagent_id == *parent && c.status == SubagentStatus::Running) {
            return Err(error("child_identity_unproven", "Registered parent child is not current"));
        }
    }
    let run_id = run.run_id.clone();
    let root_id = run.root_id.clone();
    if let Some(child) = ctx.state.subagents.iter_mut().find(|c| c.run_id == run_id && c.subagent_id == *child_id) {
        if child.parent_subagent_id != *parent_child_id || child.bound_omp_session.as_ref().is_some_and(|s| s != child_session_id) {
            return Err(error("child_identity_unproven", "Child identity changed within the current epoch"));
        }
        child.bound_omp_session = Some(child_session_id.clone());
        child.status = SubagentStatus::Running;
        child.updated_at = now();
    } else {
        ctx.state.subagents.push(Subagent { run_id: run_id.clone(), subagent_id: child_id.clone(), parent_subagent_id: parent_child_id.clone(), role: None,
            label: child_id.clone(), status: SubagentStatus::Running, summary: None, bound_omp_session: Some(child_session_id.clone()), last_control: None, updated_at: now() });
    }
    if !ctx.state.credentials.iter().any(|c| c.run_id == run_id && c.role == JobRole::Child && c.subject_child_id.as_ref() == Some(child_id)
        && c.attachment_id == job.attachment_id && c.attachment_epoch == job.attachment_epoch && c.state == credentials::CredentialState::Active) {
        let (record, _) = credentials::stage_credential(ctx.service.base(), &ctx.state.runs[job.run_index], JobRole::Child, Some(child_id))?;
        ctx.state.credentials.push(record);
    }
    let result = OrchestrationActionResult::Done;
    record(ctx, operation_id.clone(), DurableOperationKind::JobChildRegister, &action, root_id, result.clone(), Vec::new())?;
    Ok(Applied::Machine(result))
}

impl OrchestrationService {
    /// Private transport receipt: the token bytes never enter a public DTO.
    pub fn job_child_credential_file(&self, caller: &AgentCaller, child_id: &str) -> Result<std::path::PathBuf, InspectionError> {
        let state = self.store.lock()?.read()?;
        let job = credentials::validate_job_auth(&state, &caller.auth)?;
        if job.role == JobRole::Child { return Err(error("actor_forbidden", "Child cannot retrieve credentials")); }
        let record = state.credentials.iter().find(|c| c.run_id == state.runs[job.run_index].run_id && c.role == JobRole::Child
            && c.subject_child_id.as_deref() == Some(child_id) && c.attachment_id == job.attachment_id
            && c.attachment_epoch == job.attachment_epoch && c.state == credentials::CredentialState::Active)
            .ok_or_else(|| error("child_identity_unproven", "No current narrowed child credential"))?;
        credentials::credential_path(self.base(), record)
    }

    pub fn job_bootstrap_credential_file(&self, caller: &BootstrapCaller, run_id: &str) -> Result<std::path::PathBuf, InspectionError> {
        let state = self.store.lock()?.read()?;
        let run = &state.runs[scoped_index(&state, &caller.session_id, run_id)?];
        let attachment = run.attachment.as_ref().ok_or_else(|| error("caller_mismatch", "Adopted attachment is unavailable"))?;
        let location = attachment.location.as_ref().ok_or_else(|| error("caller_mismatch", "Adopted location is unavailable"))?;
        if run.kind != RunKind::Adopted || run.stage != RunStage::Active
            || caller.agent_kind != Some(AgentKind::Main)
            || caller.actual_agent_kind.as_ref() != Some(&NativeAgentKind::Omp)
            || caller.process != attachment.process || caller.process.is_none()
            || caller.omp_session_id != attachment.main_session_id
            || location.endpoint_identity != caller.endpoint_identity
            || location.workspace_id != caller.workspace_id || location.tab_id != caller.tab_id
            || location.pane_id != caller.pane_id || location.terminal_id != caller.terminal_id
            || caller.terminal_id.is_none()
            || caller.process.as_ref().is_none_or(|p| !matches!(retire::exact_running(p,
                crate::process_identity::kernel_boot_id().as_deref()), Ok(true))) {
            return Err(error("caller_mismatch", "Bootstrap receipt requires the exact live adopted main incarnation"));
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
        if matches!(current.phase, ResumePhase::Connected | ResumePhase::Reattached)
            || stop_pending
            || (current.phase == ResumePhase::Launching && resolved.attachment_observation.is_none())
            || (resolved.writer_exit_proof.is_none() && resolved.attachment_observation.is_none() && same_cause) {
            return Ok(RunResumeResult { run_id: run_id.into(), resume: current.clone(), duplicate: true });
        }
        if resolved.writer_exit_proof.is_some() && state.runs[index].kind == RunKind::Worker {
            let run = &state.runs[index];
            let document = locked.tasks(&run.root_id)?;
            super::tasks::task_transition_target(&state, run, &document)?;
            let task = document.task(run.task_id.as_deref().ok_or_else(|| error("task_not_found", "Worker task is missing"))?)?;
            state.runs[index].task_revision_at_start = Some(task.task_revision.clone());
        }
        apply_observation(&mut state, index, operation_id, ResumeMode::ReattachOrRestart, Some(resolved))?;
        let result = RunResumeResult { run_id: run_id.into(), resume: state.runs[index].resume.clone().expect("matched resume"), duplicate: false };
        let record = state.operations.iter_mut().find(|r| r.operation_id == operation_id).expect("matched operation");
        record.updated_at = now();
        let revision = locked.save(&mut state)?;
        self.revision.send_replace(revision);
        Ok(result)
    }

    pub(crate) fn resume_owner(&self, expected: &Run, resolved: &ResolvedMutationInput) -> Result<RunResumeResult, InspectionError> {
        let current = self.run_for_review(&expected.session_id, &expected.run_id)?;
        if current.attachment.as_ref().map(|a| (&a.attachment_id, a.epoch))
            != expected.attachment.as_ref().map(|a| (&a.attachment_id, a.epoch)) {
            return Err(error("attachment_conflict", "Attachment changed during owner observation"));
        }
        if let Some(resume) = &current.resume {
            let settled = matches!(resume.phase, ResumePhase::Connected | ResumePhase::Reattached);
            if !settled || (resolved.writer_exit_proof.is_none() && resolved.observation_cause.is_none()) {
                return self.advance_resume_observed(&current.run_id, &resume.operation_id, resolved);
            }
            // A later disconnection belongs to a new owner operation; a
            // completed recovery must not permanently disable future recovery.
        }
        let response = self.mutate_resolved(&Actor::Operator(OperatorOrigin::Native), OrchestrationMutationRequest {
            session_id: current.session_id.clone(), expected_revision: None,
            action: OrchestrationAction::RunResume { run_id: current.run_id.clone(), operation_id: id(),
                expected_attachment_epoch: current.attachment.as_ref().map(|a| a.epoch),
                mode: ResumeMode::ReattachOrRestart, decision_id: None },
        }, resolved)?;
        match response.result {
            OrchestrationActionResult::RunResumed { resume } => Ok(resume),
            _ => Err(error("orchestration_state_invalid", "Owner Resume returned a different operation")),
        }
    }
}
