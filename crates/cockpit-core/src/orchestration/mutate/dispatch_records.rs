use super::super::*;
use super::tasks::{active_task_root, mutable_task};
use super::reviewed::same_launch_incarnation;

impl OrchestrationService {
    /// Re-read the queue hint and canonical source together, releasing the
    /// filesystem lock before the dispatcher performs any adapter await.
    pub(crate) fn dispatch_task_eligible(&self, expected: &Run) -> Result<bool, InspectionError> {
        let locked = self.store.lock()?;
        let state = locked.read()?;
        let Some(current) = state.runs.iter().find(|run| run.run_id == expected.run_id) else {
            return Ok(false);
        };
        // An explicitly adopted active root has no dispatcher launch record.
        // It remains a valid taskless root, fenced by its actual native binding.
        if current.kind == RunKind::Adopted && current.dispatch.is_none() && expected.dispatch.is_none() {
            return Ok(current.stage == RunStage::Active && current.stage == expected.stage
                && current.kind == expected.kind && current.parent_run_id.is_none()
                && current.run_id == current.root_id && current.task_id.is_none()
                && current.run_id == expected.run_id && current.root_id == expected.root_id
                && current.session_id == expected.session_id && current.attempt == expected.attempt
                && current.bound_omp_session == expected.bound_omp_session
                && current.bound_omp_process == expected.bound_omp_process
                && same_launch_location(current.location.as_ref(), expected.location.as_ref()));
        }
        if !same_launch_incarnation(current, expected)
            || current.bound_omp_process != expected.bound_omp_process || current.stage != expected.stage {
            return Ok(false);
        }
        if matches!(current.kind, RunKind::Supervisor | RunKind::Adopted) {
            return Ok(current.parent_run_id.is_none() && current.root_id == current.run_id
                && current.task_id.is_none() && matches!(current.stage, RunStage::Preparing | RunStage::Active));
        }
        if !matches!(current.stage, RunStage::Preparing | RunStage::Initializing | RunStage::Ready | RunStage::Working) {
            return Ok(false);
        }
        let Some(task_id) = &current.task_id else { return Ok(false) };
        if active_task_root(&state, &current.session_id, &current.root_id).is_err()
            || projection::current_task_run(&state, &current.root_id, task_id)
                .is_none_or(|run| run.run_id != current.run_id) {
            return Ok(false);
        }
        let document = locked.tasks(&current.root_id)?;
        let task = match mutable_task(&state, &current.root_id, task_id, &document) {
            Ok(task) => task,
            Err(failure) if matches!(failure.code.as_str(),
                "task_not_found" | "task_id_duplicate" | "task_checked" | "intent_conflict") => return Ok(false),
            Err(failure) => return Err(failure),
        };
        Ok(matches!(dependencies::DependencyGraph::new(&document.tasks).evaluate(task).state,
            TaskDependencyState::None | TaskDependencyState::Satisfied))
    }

    pub(crate) fn record_dispatch(
        &self,
        run_id: &str,
        update: DispatchUpdate,
    ) -> Result<u64, InspectionError> {
        let locked = self.store.lock()?;
        let mut state = locked.read()?;
        let index = run_index(&state, run_id)?;
        if state.runs[index].stage == RunStage::Closed {
            return Err(error("invalid_stage", "Closed run cannot dispatch"));
        }
        let mut setup_notice = None;
        {
            let run = &mut state.runs[index];
            match update {
                DispatchUpdate::SetupPlanned {
                    setup,
                    prepare_plan,
                } => {
                    bounded(&prepare_plan.text, 16 * 1024)?;
                    run.setup = Some(setup);
                    run.prepare_plan = Some(prepare_plan);
                    if run.kind == RunKind::Worker {
                        setup_notice = Some((
                            run.root_id.clone(),
                            run.run_id.clone(),
                            run.prepare_plan
                                .as_ref()
                                .expect("stored plan")
                                .plan_revision
                                .clone(),
                        ));
                        run.stage = RunStage::AwaitingPrepare;
                    }
                    run.dispatch = Some(dispatch(if run.kind == RunKind::Worker {
                        DispatchStep::Planning
                    } else {
                        DispatchStep::SetupPending
                    }));
                }
                DispatchUpdate::PlanFailed { error } => {
                    let mut d = dispatch(DispatchStep::PlanFailed);
                    d.error = Some(error);
                    run.dispatch = Some(d);
                }
                DispatchUpdate::Step { step, error } => {
                    let d = run.dispatch.get_or_insert_with(|| dispatch(step));
                    d.step = step;
                    d.error = error;
                    d.updated_at = now();
                }
                DispatchUpdate::SetupDone {
                    workspace_id,
                    checkout_path,
                } => {
                    let setup = run
                        .setup
                        .as_mut()
                        .ok_or_else(|| error("invalid_stage", "Setup summary missing"))?;
                    setup.workspace_id = Some(workspace_id);
                    setup.checkout_path = checkout_path;
                }
                DispatchUpdate::SetupProgress { generation } => {
                    let setup = run
                        .setup
                        .as_mut()
                        .ok_or_else(|| error("invalid_stage", "Setup summary missing"))?;
                    setup.generation = Some(generation);
                    let d = run
                        .dispatch
                        .get_or_insert_with(|| dispatch(DispatchStep::SetupRunning));
                    d.recovery = None;
                    d.step = DispatchStep::SetupRunning;
                    d.updated_at = now();
                }
                DispatchUpdate::LaunchIntent {
                    launch_tag,
                    launch_attempt,
                    endpoint_identity,
                } => {
                    run.launch_shell_identity = None;
                    let d = run
                        .dispatch
                        .get_or_insert_with(|| dispatch(DispatchStep::LaunchIntent));
                    d.step = DispatchStep::LaunchIntent;
                    d.launch_tag = Some(launch_tag);
                    d.launch_attempt = launch_attempt;
                    d.endpoint_identity = Some(endpoint_identity);
                    d.agent_started = false;
                    d.error = None;
                    d.updated_at = now();
                }
                DispatchUpdate::TabReceipt { location, launch_shell_identity } => {
                    let d = run
                        .dispatch
                        .as_ref()
                        .ok_or_else(|| error("invalid_stage", "Launch intent missing"))?;
                    if d.launch_tag.as_deref() != Some(&location.launch_tag)
                        || d.endpoint_identity.as_deref() != Some(&location.endpoint_identity)
                        || run.session_id != location.session_id
                    {
                        return Err(error(
                            "caller_mismatch",
                            "Tab receipt does not match persisted launch intent",
                        ));
                    }
                    run.location = Some(location);
                    run.launch_shell_identity = launch_shell_identity;
                }
            }
            run.updated_at = now();
        }
        if let Some((root_id, worker_id, revision)) = setup_notice {
            let text = serde_json::json!({"event":"setup_ready","run_id":worker_id,"plan_revision":revision}).to_string();
            messages::append(&mut state, messages::AppendMessage { from: ActorRef::Dispatcher, to_run_id: &root_id, message_id: &format!("setup-ready-{worker_id}-{revision}"), kind: MessageKind::Observation, text: &text, in_reply_to: None, report: None, stale: false, from_subagent_id: None, escalated_from: None })?;
        }
        escalation::failure(&mut state, index)?;
        let revision = locked.save(&mut state)?;
        self.revision.send_replace(revision);
        Ok(revision)
    }
}
