use super::{Applied, MutationCtx};
use super::super::*;
use super::tasks::{mutable_task, task_root_authority};
use cockpit_protocol::projects::WorkspaceRecoveryAction;

pub(in crate::orchestration) fn supervisor_start(
    ctx: &mut MutationCtx<'_>,
    action: OrchestrationAction,
) -> Result<Applied, InspectionError> {
    use super::super::store::DurableOperationKind;
    let OrchestrationAction::SupervisorStart {
        operation_id, mode, target, label, selected_root_id, expected_project_key,
    } = &action else { unreachable!() };
    operator(ctx.actor)?;
    if let Some(result) = super::resume::replay(ctx, operation_id, DurableOperationKind::SupervisorStart, &action)? {
        return Ok(Applied::Document(result));
    }
    let mut resolved_project = ctx.resolved.and_then(|r| r.project.clone());
    if target.is_some() && resolved_project.is_none() {
        return Err(error("project_identity_unavailable", "Start requires a host-resolved project identity"));
    }
    if expected_project_key.as_ref().is_some_and(|key| resolved_project.as_ref().is_none_or(|p| &p.key != key)) {
        return Err(error("project_identity_conflict", "The focused project changed before Start"));
    }
    if let Some(checkout) = ctx.resolved.and_then(|r| r.checkout.as_ref()) {
        project_identity::revalidate_checkout(checkout)?;
    }
    if *mode == SupervisorStartMode::ReuseOrResume {
        if let Some(workspace_id) = resolved_project.as_ref().and_then(|p| p.source_workspace_id.as_deref()) {
            let unresolved = ctx.state.runs.iter().any(|run| run.session_id == ctx.session_id
                && run.parent_run_id.is_none() && run.stage != RunStage::Closed
                && matches!(run.kind, RunKind::Supervisor | RunKind::Adopted) && run.project.is_none()
                && (run.attachment.as_ref().and_then(|a| a.location.as_ref()).is_some_and(|l| l.workspace_id == workspace_id)
                    || run.setup.as_ref().and_then(|s| s.project_workspace_id.as_deref()) == Some(workspace_id)
                    || match run.target.as_ref() {
                        Some(DispatchTarget::ExistingSpace { workspace_id: source })
                        | Some(DispatchTarget::SpaceWorktree { workspace_id: source, .. }) => source == workspace_id,
                        _ => false,
                    }));
            if unresolved {
                return Err(error("project_identity_unavailable", "An associated open root requires owner project reconciliation before reuse"));
            }
        }
        if let Some(project) = &resolved_project {
            if let Some(index) = preferred_project_root(&ctx.state.runs, ctx.session_id, &project.key, selected_root_id.as_deref()) {
                let run = &ctx.state.runs[index];
                let root_id = run.run_id.clone();
                let observed_connected = has_current_binding(ctx.state, run)
                    && ctx.resolved.and_then(|r| r.attachment_observation.as_ref()).is_some_and(|observed|
                    run.attachment.as_ref().is_some_and(|current| observed.attachment_id == current.attachment_id
                        && observed.epoch == current.epoch && observed.process == current.process
                        && observed.shell == current.shell && observed.machine_id == current.machine_id
                        && observed.launch_mode == current.launch_mode
                        && !matches!(current.state, AttachmentState::Quiescing | AttachmentState::Revoked)
                        && observed.process.is_some() && observed.state == AttachmentState::Connected));
                let (outcome, resume_operation_id) = if observed_connected {
                    ctx.state.runs[index].attachment = ctx.resolved.and_then(|r| r.attachment_observation.clone());
                    (SupervisorStartOutcome::Reused, None)
                } else {
                    let resume_id = id();
                    let epoch = ctx.state.runs[index].attachment.as_ref().map(|a| a.epoch);
                    super::resume::resume(ctx, OrchestrationAction::RunResume {
                        run_id: root_id.clone(), operation_id: resume_id.clone(),
                        expected_attachment_epoch: epoch,
                        mode: ResumeMode::ReattachOrRestart, decision_id: None,
                    })?;
                    let effective = ctx.state.runs[index].resume.as_ref().map(|r| r.operation_id.clone()).unwrap_or(resume_id);
                    (SupervisorStartOutcome::Resuming, Some(effective))
                };
                let result = OrchestrationActionResult::SupervisorStarted { start: SupervisorStartResult {
                    root_id: root_id.clone(), outcome, operation_id: operation_id.clone(), resume_operation_id, duplicate: false,
                }};
                super::resume::record(ctx, operation_id.clone(), DurableOperationKind::SupervisorStart, &action,
                    root_id, result.clone(), Vec::new())?;
                return Ok(Applied::Machine(result));
            }
        }
    }
    let run_id = id();
    let label = label.clone().unwrap_or_else(|| "Supervisor".into());
    bounded(&label, 256)?;
    let mut checkout = ctx.resolved.and_then(|r| r.checkout.clone());
    let target = match target.clone() {
        Some(target) => unfocused(target),
        None => {
            let path = prepare_project_root(&ctx.service.store.base().join("supervisors").join(&run_id))?;
            let (project, directory) = project_identity::resolve_owned_directory(&path)?;
            resolved_project = Some(project);
            checkout = Some(directory);
            DispatchTarget::Setup { request: WorkspaceSetupRequest::Open {
                path: path.to_string_lossy().into_owned(), label: Some(label.clone()), task_name: None, focus: false,
            }}
        }
    };
    let mut run = new_run(&run_id, ctx.session_id, RunKind::Supervisor, label, &run_id, None, None, 1);
    run.target = Some(target);
    run.project = resolved_project;
    run.checkout = checkout;
    run.stage = RunStage::Starting;
    run.dispatch = Some(dispatch(DispatchStep::SetupPending));
    run.brief = supervisor_guidance().into();
    ctx.state.runs.push(run);
    let result = OrchestrationActionResult::SupervisorStarted { start: SupervisorStartResult {
        root_id: run_id.clone(), outcome: SupervisorStartOutcome::Created,
        operation_id: operation_id.clone(), resume_operation_id: None, duplicate: false,
    }};
    super::resume::record(ctx, operation_id.clone(), DurableOperationKind::SupervisorStart, &action,
        run_id, result.clone(), Vec::new())?;
    Ok(Applied::Machine(result))
}

pub(in crate::orchestration) fn adopt(
    ctx: &mut MutationCtx<'_>,
    label: String,
) -> Result<Applied, InspectionError> {
    let actor = ctx.actor;
    let caller = ctx.caller;
    let session_id = ctx.session_id;
    let state = &mut *ctx.state;
    let Actor::Bootstrap(agent) = actor else {
        return Err(error(
            "actor_forbidden",
            "Only a real agent pane can adopt itself",
        ));
    };
    if caller.is_some() {
        return Err(error("caller_mismatch", "Pane already belongs to a run"));
    }
    if agent.agent_kind != Some(AgentKind::Main)
        || agent.omp_session_id.as_ref().is_none_or(|s| s.is_empty())
        || agent.actual_agent_kind.as_ref() != Some(&NativeAgentKind::Omp)
    {
        return Err(error(
            "report_requires_main",
            "Adoption requires the main OMP native session",
        ));
    }
    bounded(&label, 256)?;
    let id = id();
    let mut run = new_run(
        &id,
        session_id,
        RunKind::Adopted,
        label,
        &id,
        None,
        None,
        1,
    );
    run.stage = RunStage::Active;
    run.project = ctx.resolved.and_then(|r| r.project.clone());
    run.checkout = ctx.resolved.and_then(|r| r.checkout.clone());
    let attachment = run.attachment.as_mut().expect("new adopted attachment");
    attachment.main_session_id = agent.omp_session_id.clone();
    attachment.process = agent.process.clone();
    attachment.machine_id = crate::process_identity::machine_id();
    attachment.state = AttachmentState::Connecting;
    attachment.location = Some(RunLocation {
        boot_id: agent.boot_id.clone(),
        terminal_id: agent.terminal_id.clone(),
        native_session_id: agent.native_session_id.clone(),
        endpoint_identity: agent.endpoint_identity.clone(),
        session_id: agent.session_id.clone(),
        workspace_id: agent.workspace_id.clone(),
        tab_id: agent.tab_id.clone(),
        pane_id: agent.pane_id.clone(),
        launch_tag: String::new(),
    });
    let (credential, _) = credentials::stage_credential(ctx.service.base(), &run, JobRole::Root, None)?;
    state.credentials.push(credential);
    state.runs.push(run);
    Ok(Applied::Machine(OrchestrationActionResult::Run {
        run_id: id,
        attempt: 1,
    }))
}

pub(in crate::orchestration) fn worker_start(
    ctx: &mut MutationCtx<'_>,
    action: OrchestrationAction,
) -> Result<Applied, InspectionError> {
    use super::super::store::DurableOperationKind;
    let OrchestrationAction::WorkerStart {
        operation_id, root_id, task_id, expected_task_revision, expected_doc_revision,
        target, label, brief, cleanup_policy,
    } = &action else { unreachable!() };
    task_root_authority(ctx.state, ctx.actor, ctx.caller, ctx.session_id, root_id)?;
    if let Some(result) = super::resume::replay(ctx, operation_id, DurableOperationKind::WorkerStart, &action)? {
        return Ok(Applied::Document(result));
    }
    bounded(brief, 16 * 1024)?;
    let document = ctx.locked.tasks(root_id)?;
    let task = mutable_task(ctx.state, root_id, task_id, &document)?;
    if task.task_revision != *expected_task_revision || document.doc_revision != *expected_doc_revision {
        return Err(error("task_revision_conflict", "Canonical task or document changed before Start"));
    }
    if dependencies::DependencyGraph::new(&document.tasks).evaluate(task).state == TaskDependencyState::Invalid {
        return Err(error("task_dependencies_invalid", "Task relationships are structurally invalid"));
    }
    if ctx.state.runs.iter().any(|run| run.root_id == *root_id
        && run.task_id.as_deref() == Some(task.task_id.as_str()) && run.stage != RunStage::Closed) {
        return Err(error("task_has_active_run", "Resume the current task worker rather than starting a second attempt"));
    }
    let attempt = ctx.state.runs.iter().filter(|run| run.root_id == *root_id
        && run.task_id.as_deref() == Some(task.task_id.as_str()))
        .map(|run| run.attempt).max().unwrap_or(0).checked_add(1)
        .ok_or_else(|| error("orchestration_state_full", "Task attempt exhausted"))?;
    let run_id = id();
    let label = label.clone().unwrap_or_else(|| task.title.clone());
    bounded(&label, 256)?;
    let mut run = new_run(&run_id, ctx.session_id, RunKind::Worker, label, root_id,
        Some(root_id.clone()), Some(task.task_id.clone()), attempt);
    run.target = Some(unfocused(target.clone()));
    run.brief = brief.clone();
    run.task_revision_at_start = Some(task.task_revision.clone());
    run.cleanup_policy = Some(*cleanup_policy);
    run.stage = RunStage::Queued;
    run.dispatch = Some(dispatch(DispatchStep::Planning));
    ctx.state.runs.push(run);
    let result = OrchestrationActionResult::WorkerStarted { start: WorkerStartResult {
        operation_id: operation_id.clone(), run_id, attempt, duplicate: false,
    }};
    super::resume::record(ctx, operation_id.clone(), DurableOperationKind::WorkerStart, &action,
        root_id.clone(), result.clone(), Vec::new())?;
    Ok(Applied::Machine(result))
}

pub(in crate::orchestration) fn cancel(
    ctx: &mut MutationCtx<'_>,
    run_id: String,
) -> Result<Applied, InspectionError> {
    let actor = ctx.actor;
    let caller = ctx.caller;
    let session_id = ctx.session_id;
    let state = &mut *ctx.state;
    let (index, provenance) =
        management_target(state, actor, caller, session_id, &run_id)?;
    if dispatch::recovery_close_pending(&state.runs[index]) {
        return Err(error("recovery_pending", "Owned launch cancellation is still being observed; tracking cannot be closed until its bounded effect settles"));
    }
    if state.runs[index].stage == RunStage::Closed {
        return Err(error("invalid_stage", "Run is already closed"));
    }
    state.runs[index].stage = RunStage::Closed;
    state.runs[index].close_reason = Some(CloseReason::Cancelled);
    let from = actor_ref(actor, caller, state);
    messages::append(state, messages::AppendMessage { from: from, to_run_id: &run_id, message_id: &format!("cancel-{}", id()), kind: MessageKind::CancelRequest, text: "Tracking is closed for this run. Please stop and report outstanding effects. This advisory request does not guarantee process termination; resources and descendants are retained.", in_reply_to: None, report: None, stale: false, from_subagent_id: None, escalated_from: None })?;
    let annotation = decision_annotation(
        actor_ref(actor, caller, state),
        &provenance,
        "Tracking closed; advisory cancellation recorded; descendants retained",
    );
    state.runs[index].annotations.push(annotation);
    Ok(Applied::Machine(OrchestrationActionResult::Done))
}


pub(in crate::orchestration) fn reconcile(
    ctx: &mut MutationCtx<'_>,
    run_id: String,
    recovery: Option<WorkspaceRecoveryAction>,
) -> Result<Applied, InspectionError> {
    let (index, provenance) = management_target(ctx.state, ctx.actor, ctx.caller, ctx.session_id, &run_id)?;
    if dispatch::recovery_close_pending(&ctx.state.runs[index]) {
        return Err(error("recovery_pending", "Owned launch cancellation is still being observed"));
    }
    if ctx.state.runs[index].stage == RunStage::Closed {
        return Err(error("invalid_stage", "Closed runs cannot be reconciled"));
    }
    if matches!(ctx.actor, Actor::Agent(_)) && recovery == Some(WorkspaceRecoveryAction::RetryEnvironment) {
        return Err(error("actor_forbidden", "Only the operator may retry an environment with uncertain prior effects"));
    }
    let by = actor_ref(ctx.actor, ctx.caller, ctx.state);
    let run = &mut ctx.state.runs[index];
    let d = run.dispatch.as_mut().ok_or_else(|| error("invalid_stage", "No setup to reconcile"))?;
    if d.step != DispatchStep::SetupUnknown
        && !(d.step == DispatchStep::NeedsReview && d.launch_tag.is_none()
            && run.setup.as_ref().is_some_and(|setup| setup.operation_id.is_some())) {
        return Err(error("invalid_stage", "Only uncertain setup accepts environment reconciliation; use Resume for attachment recovery"));
    }
    d.step = DispatchStep::SetupUnknown;
    d.recovery = Some(recovery.ok_or_else(|| error("invalid_stage", "Choose an explicit setup recovery action"))?);
    d.error = None;
    d.updated_at = now();
    run.annotations.push(decision_annotation(by, &provenance,
        "Setup reconciliation requested; environment recovery requires fresh checkout proof"));
    Ok(Applied::Machine(OrchestrationActionResult::Done))
}

fn unfocused(mut target: DispatchTarget) -> DispatchTarget {
    if let DispatchTarget::Setup { request } = &mut target {
        match request {
            WorkspaceSetupRequest::Create { focus, .. }
            | WorkspaceSetupRequest::Open { focus, .. } => *focus = false,
        }
    }
    target
}
