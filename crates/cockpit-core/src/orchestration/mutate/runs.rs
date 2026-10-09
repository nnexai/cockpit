use super::{Applied, MutationCtx};
use super::super::*;
use super::tasks::{active_task_root, mutable_task, task_transition_target};
use super::reviewed::retry_launch_state;
use cockpit_protocol::projects::WorkspaceRecoveryAction;

pub(in crate::orchestration) fn supervisor_start(
    ctx: &mut MutationCtx<'_>,
    target: Option<DispatchTarget>,
    label: Option<String>,
) -> Result<Applied, InspectionError> {
    let service = ctx.service;
    let actor = ctx.actor;
    let session_id = ctx.session_id;
    let state = &mut *ctx.state;
    operator(actor)?;
    let id = id();
    let label = label.unwrap_or_else(|| "Supervisor".into());
    bounded(&label, 256)?;
    let target = match target {
        Some(t) => unfocused(t),
        None => {
            let path =
                prepare_project_root(&service.store.base().join("supervisors").join(&id))?;
            DispatchTarget::Setup {
                request: WorkspaceSetupRequest::Open {
                    path: path.to_string_lossy().into_owned(),
                    label: Some(label.clone()),
                    task_name: None,
                    focus: false,
                },
            }
        }
    };
    let mut run = new_run(
        &id,
        session_id,
        RunKind::Supervisor,
        label,
        &id,
        None,
        None,
        1,
    );
    run.target = Some(target);
    run.stage = RunStage::Preparing;
    run.dispatch = Some(dispatch(DispatchStep::SetupPending));
    run.prepare_brief = supervisor_guidance().into();
    state.runs.push(run);
    Ok(Applied::Machine(OrchestrationActionResult::Run {
        run_id: id,
        attempt: 1,
    }))
}

pub(in crate::orchestration) fn adopt(
    ctx: &mut MutationCtx<'_>,
    label: String,
) -> Result<Applied, InspectionError> {
    let actor = ctx.actor;
    let caller = ctx.caller;
    let session_id = ctx.session_id;
    let state = &mut *ctx.state;
    let Actor::Agent(agent) = actor else {
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
    run.bound_omp_session = agent.omp_session_id.clone();
    run.bound_omp_process = agent.process.clone();
    run.location = Some(RunLocation {
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
    state.runs.push(run);
    Ok(Applied::Machine(OrchestrationActionResult::Run {
        run_id: id,
        attempt: 1,
    }))
}

pub(in crate::orchestration) fn bind_session(
    ctx: &mut MutationCtx<'_>,
    omp_session_id: String,
) -> Result<Applied, InspectionError> {
    let actor = ctx.actor;
    let caller = ctx.caller;
    let state = &mut *ctx.state;
    let index = required_caller(caller)?;
    if dispatch::recovery_incarnation_revoked(&state.runs[index])
    {
        return Err(error("attempt_stale",
            "The owned launch incarnation was revoked for cancellation; a late SDK binding cannot revive it"));
    }
    let Actor::Agent(agent) = actor else {
        return Err(error("actor_forbidden", "Binding is agent-only"));
    };
    validate_identity(&omp_session_id)?;
    if agent.agent_kind != Some(AgentKind::Main)
        || agent.omp_session_id.as_deref() != Some(omp_session_id.as_str())
    {
        return Err(error(
            "report_requires_main",
            "Only the main native session may bind this run",
        ));
    }
    if state.runs[index]
        .bound_omp_session
        .as_ref()
        .is_some_and(|s| s != &omp_session_id && !same_process_main_rollover(&state.runs[index], agent))
    {
        return Err(error(
            "session_mismatch",
            "Run is already bound to another native session",
        ));
    }
    if state.runs[index].bound_omp_process.as_ref().is_some_and(|process| {
        agent.process.as_ref() != Some(process)
    }) {
        return Err(error("session_mismatch", "Run is already bound to another native process"));
    }
    if state.runs[index].bound_omp_session.as_ref().is_some_and(|s| s != &omp_session_id) {
        // The native OS incarnation is unchanged; only its trusted
        // main SDK session advanced (for example OMP /new).
        state.runs[index].location.as_mut().expect("rollover location")
            .native_session_id = agent.native_session_id.clone();
        state.runs[index].annotations.push(Annotation {
            by: ActorRef::Dispatcher,
            text: "Main SDK session rolled over within the same verified native process and recorded terminal".into(),
            at: now(),
        });
        let run = &state.runs[index];
        let run_id = run.run_id.clone();
        let message_id = format!("sdk-binding-restored:{}:{}:{omp_session_id}", run.run_id, run.attempt);
        let text = if run.kind == RunKind::Worker {
            "Binding restored in the same native process after a main-session rollover. Inspect this Run's existing durable task, plans, grants, Ready/Result receipts and inbox, then resume only work already authorized by its current exact grants. Do not redo completed external writes, reset the task or replay preparation as a new job. Ask the managing supervisor only for a genuinely missing decision or grant."
        } else {
            "Binding restored in the same native process after a main-session rollover. Inspect the existing canonical task board, durable Runs, plans, grants, results and inbox, then resume supervision from that state. Do not redo completed external writes, recreate workers or tasks, or perform the workers' implementation work. Resolve non-blocking questions autonomously and ask the operator only for a genuinely blocking decision."
        };
        messages::append(state, messages::AppendMessage { from: ActorRef::Dispatcher, to_run_id: &run_id, message_id: &message_id, kind: MessageKind::Instruction, text: text, in_reply_to: None, report: None, stale: false, from_subagent_id: None, escalated_from: None })?;
    }
    state.runs[index].bound_omp_process = agent.process.clone();
    state.runs[index].bound_omp_session = Some(omp_session_id);
    Ok(Applied::Machine(OrchestrationActionResult::Done))
}

pub(in crate::orchestration) fn propose(
    ctx: &mut MutationCtx<'_>,
    task_id: String,
    parent_run_id: Option<String>,
    label: Option<String>,
    target: DispatchTarget,
    prepare_brief: String,
    supersedes_run_id: Option<String>,
) -> Result<Applied, InspectionError> {
    let locked = ctx.locked;
    let caller = ctx.caller;
    let session_id = ctx.session_id;
    let state = &mut *ctx.state;
    bounded(&prepare_brief, 16 * 1024)?;
    let parent_id = parent_run_id
        .or_else(|| caller.map(|i| state.runs[i].run_id.clone()))
        .ok_or_else(|| error("root_not_found", "Choose a parent supervisor"))?;
    let parent = run_index(state, &parent_id)?;
    scope_run(state, session_id, parent)?;
    if state.runs[parent].stage == RunStage::Closed {
        return Err(error(
            "invalid_stage",
            "Closed runs cannot dispatch new work",
        ));
    }
    if let Some(index) = caller {
        if index != parent
            && !is_ancestor(state, &state.runs[index].run_id, &parent_id)
        {
            return Err(error(
                "not_in_subtree",
                "Workers may only be proposed beneath the caller",
            ));
        }
    }
    let root = state.runs[parent].root_id.clone();
    active_task_root(state, session_id, &root)?;
    let document = locked.tasks(&root)?;
    let task = mutable_task(state, &root, &task_id, &document)?.clone();
    if dependencies::DependencyGraph::new(&document.tasks).evaluate(&task).state == TaskDependencyState::Invalid {
        return Err(error("task_dependencies_invalid", "Task relationships are structurally invalid"));
    }
    let task_id = task.task_id.clone();
    let open: Vec<_> = state
        .runs
        .iter()
        .filter(|r| {
            r.root_id == root
                && r.task_id.as_deref() == Some(&task_id)
                && r.stage != RunStage::Closed
        })
        .collect();
    if !open.is_empty() {
        if open.len() != 1
            || supersedes_run_id.as_deref() != Some(open[0].run_id.as_str())
        {
            return Err(error(
                "task_has_active_run",
                "An explicit supersedes_run_id is required for this active task",
            ));
        }
        if let Some(index) = caller {
            if !is_ancestor(state, &state.runs[index].run_id, &open[0].run_id) {
                return Err(error(
                    "not_ancestor",
                    "Cannot replace another subtree's worker",
                ));
            }
        }
    } else if supersedes_run_id.is_some() {
        return Err(error(
            "task_has_active_run",
            "Replacement does not identify a live task attempt",
        ));
    }
    let attempt = state
        .runs
        .iter()
        .filter(|r| r.root_id == root && r.task_id.as_deref() == Some(&task_id))
        .map(|r| r.attempt)
        .max()
        .unwrap_or(0)
        .checked_add(1)
        .ok_or_else(|| error("orchestration_state_full", "Task attempt exhausted"))?;
    let id = id();
    let label = label.unwrap_or_else(|| task.title.clone());
    bounded(&label, 256)?;
    let mut run = new_run(
        &id,
        session_id,
        RunKind::Worker,
        label,
        &root,
        Some(parent_id),
        Some(task_id),
        attempt,
    );
    run.target = Some(unfocused(target));
    run.prepare_brief = prepare_brief;
    run.task_revision_at_propose = Some(task.task_revision);
    run.supersedes_run_id = supersedes_run_id;
    run.dispatch = Some(dispatch(DispatchStep::Planning));
    state.runs.push(run);
    Ok(Applied::Machine(OrchestrationActionResult::Run {
        run_id: id,
        attempt,
    }))
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

pub(in crate::orchestration) fn retry_launch(
    ctx: &mut MutationCtx<'_>,
    run_id: String,
) -> Result<Applied, InspectionError> {
    let locked = ctx.locked;
    let actor = ctx.actor;
    let caller = ctx.caller;
    let session_id = ctx.session_id;
    let reviewed = ctx.reviewed;
    let state = &mut *ctx.state;
    let (index, provenance) =
        management_target(state, actor, caller, session_id, &run_id)?;
    if dispatch::recovery_close_pending(&state.runs[index]) {
        return Err(error("recovery_pending", "Owned launch cancellation is still being observed; do not erase its execution barrier"));
    }
    if matches!(actor, Actor::Agent(_)) && reviewed.is_none() {
        return Err(error(
            "retry_preflight_required",
            "Supervisor retry requires the fresh absence preflight",
        ));
    }
    if state.runs[index].kind == RunKind::Worker {
        let document = locked.tasks(&state.runs[index].root_id)?;
        task_transition_target(state, &state.runs[index], &document, false)?;
    }
    let by = actor_ref(actor, caller, state);
    let run = &mut state.runs[index];
    let previous = run
        .dispatch
        .as_ref()
        .ok_or_else(|| error("invalid_stage", "No launch to retry"))?;
    if !matches!(
        previous.step,
        DispatchStep::LaunchUnknown | DispatchStep::NeedsReview
    ) || matches!(run.stage, RunStage::Closed | RunStage::Reported)
    {
        return Err(error(
            "invalid_stage",
            "Only uncertain launches can be retried explicitly",
        ));
    }
    let attempt = retry_launch_state(state, index)?;
    state.runs[index].annotations.push(decision_annotation(
        by,
        &provenance,
        &if reviewed.is_some() {
            format!("Launch retried as attempt {attempt} after absence preflight")
        } else {
            format!("Launch retried as attempt {attempt} by explicit operator decision")
        },
    ));
    Ok(Applied::Machine(OrchestrationActionResult::Done))
}

pub(in crate::orchestration) fn reconcile(
    ctx: &mut MutationCtx<'_>,
    run_id: String,
    recovery: Option<WorkspaceRecoveryAction>,
) -> Result<Applied, InspectionError> {
    let actor = ctx.actor;
    let caller = ctx.caller;
    let session_id = ctx.session_id;
    let state = &mut *ctx.state;
    let (index, provenance) =
        management_target(state, actor, caller, session_id, &run_id)?;
    if dispatch::recovery_close_pending(&state.runs[index]) {
        return Err(error("recovery_pending", "Owned launch cancellation is still being observed; do not erase its execution barrier"));
    }
    if matches!(actor, Actor::Agent(_)) {
        if recovery == Some(cockpit_protocol::projects::WorkspaceRecoveryAction::RetryEnvironment) {
            return Err(error(
                "actor_forbidden",
                "Only the operator may retry an environment with uncertain prior effects",
            ));
        }
        if state.runs[index].dispatch.as_ref().is_some_and(|d| d.step == DispatchStep::LaunchPending) {
            return Err(error("invalid_stage", "Automatic launch proof is still pending"));
        }
    }
    let by = actor_ref(actor, caller, state);
    if state.runs[index].stage == RunStage::Closed {
        return Err(error("invalid_stage", "Closed runs cannot be reconciled"));
    }
    let run = &mut state.runs[index];
    let d = run
        .dispatch
        .as_mut()
        .ok_or_else(|| error("invalid_stage", "No dispatch to reconcile"))?;
    let queued_review = d.step == DispatchStep::LaunchIntent && d.agent_started;
    if !matches!(
        d.step,
        DispatchStep::SetupUnknown
            | DispatchStep::LaunchUnknown
            | DispatchStep::NeedsReview
            | DispatchStep::LaunchPending
            | DispatchStep::PlanFailed
            | DispatchStep::Launched
    ) && !queued_review
    {
        return Err(error(
            "invalid_stage",
            "Only uncertain dispatch or a proven launch can be reconciled",
        ));
    }
    if d.step == DispatchStep::PlanFailed {
        d.step = DispatchStep::Planning;
        d.error = None;
    } else if d.step == DispatchStep::SetupUnknown
        || (d.step == DispatchStep::NeedsReview
            && d.launch_tag.is_none()
            && run.setup.as_ref().is_some_and(|s| s.operation_id.is_some()))
    {
        d.step = DispatchStep::SetupUnknown;
        d.recovery = Some(recovery.ok_or_else(|| {
            error("invalid_stage", "Choose an explicit setup recovery action")
        })?);
        d.error = None;
    } else {
        if recovery.is_some() {
            return Err(error(
                "invalid_stage",
                "Setup recovery action does not apply to launch reconciliation",
            ));
        }
        d.recovery = None;
        if d.step == DispatchStep::Launched && !d.agent_started {
            d.step = DispatchStep::NeedsReview;
            d.error = Some(ErrorResponse {
                code: "launch_receipt_inconsistent".into(),
                message: "Launched run has no proven agent-start receipt; inspect before explicitly retrying".into(),
            });
        } else {
            // Keep the proven start marker and incarnation: this is a
            // read-only review request, not a new launch intent.
            d.step = DispatchStep::LaunchIntent;
            d.error = None;
        }
    }
    d.updated_at = now();
    if matches!(actor, Actor::Agent(_)) || recovery.is_some() {
        run.annotations.push(decision_annotation(
            by,
            &provenance,
            "Dispatch reconciliation requested; setup recovery requires fresh checkout proof",
        ));
    }
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
