use super::super::*;
use super::tasks::{active_task_root, mutable_task};
use super::reviewed::same_launch_incarnation;

impl OrchestrationService {
    /// Pin an authoritative transport Space before asynchronous checkout
    /// preparation. Later focus changes cannot retarget this incarnation.
    pub(crate) fn record_staging_space(&self, expected: &Run, workspace_id: &str) -> Result<bool, InspectionError> {
        validate_identity(workspace_id)?;
        let locked = self.store.lock()?;
        let mut state = locked.read()?;
        let index = run_index(&state, &expected.run_id)?;
        let run = &state.runs[index];
        if !same_launch_incarnation(run, expected) || !matches!(run.stage, RunStage::Queued | RunStage::Starting)
            || run.attachment.as_ref().is_some_and(|attachment| matches!(attachment.state, AttachmentState::Quiescing | AttachmentState::Revoked)) {
            return Ok(false);
        }
        let dispatch = run.dispatch.as_ref().ok_or_else(|| error("dispatch_missing", "Staging requires a dispatcher attempt"))?;
        if let Some(saved) = &dispatch.staging_workspace_id {
            if saved != workspace_id {
                return Err(error("staging_space_conflict", "This launch already pinned another authoritative staging Space"));
            }
            return Ok(true);
        }
        if dispatch.launch_tag.is_some() || dispatch.latch.is_some() || dispatch.move_operation_id.is_some() {
            return Err(error("staging_space_conflict", "A launched attempt cannot select a new staging Space"));
        }
        if run.kind == RunKind::Worker && run.task_id.as_ref().is_none_or(|task_id|
            projection::current_task_run(&state, &run.root_id, task_id).is_none_or(|current| current.run_id != run.run_id)) {
            return Ok(false);
        }
        state.runs[index].dispatch.as_mut().expect("fenced dispatch").staging_workspace_id = Some(workspace_id.into());
        let revision = locked.save(&mut state)?;
        self.revision.send_replace(revision);
        Ok(true)
    }
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
                && current.attachment.as_ref().and_then(|a| a.conversation.as_ref()) == expected.attachment.as_ref().and_then(|a| a.conversation.as_ref())
                && current.attachment.as_ref().and_then(|a| a.process.as_ref()) == expected.attachment.as_ref().and_then(|a| a.process.as_ref())
                && same_launch_location(current.attachment.as_ref().and_then(|a| a.location.as_ref()), expected.attachment.as_ref().and_then(|a| a.location.as_ref())));
        }
        if !same_launch_incarnation(current, expected)
            || current.attachment.as_ref().and_then(|a| a.process.as_ref()) != expected.attachment.as_ref().and_then(|a| a.process.as_ref()) || current.stage != expected.stage {
            return Ok(false);
        }
        if matches!(current.kind, RunKind::Supervisor | RunKind::Adopted) {
            return Ok(current.parent_run_id.is_none() && current.root_id == current.run_id
                && current.task_id.is_none() && matches!(current.stage, RunStage::Queued | RunStage::Starting | RunStage::Active));
        }
        if !matches!(current.stage, RunStage::Queued | RunStage::Starting | RunStage::Working) {
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
        if current.stage != RunStage::Working
            && current.task_revision_at_start.as_deref() != Some(task.task_revision.as_str()) {
            return Ok(false);
        }
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
        if matches!(&update, DispatchUpdate::SetupPlanned { .. }) {
            let run = &state.runs[index];
            if !matches!(run.stage, RunStage::Queued | RunStage::Starting) || run.setup.is_some() {
                return Err(error("invalid_stage", "Only an unplanned queued start may record setup"));
            }
            if run.kind == RunKind::Worker {
                let document = locked.tasks(&run.root_id)?;
                super::tasks::task_transition_target(&state, run, &document)?;
                let task = mutable_task(&state, &run.root_id, run.task_id.as_deref().expect("task guarded"), &document)?;
                if run.task_revision_at_start.as_deref() != Some(task.task_revision.as_str()) {
                    return Err(error("task_revision_changed", "Canonical task changed before setup"));
                }
            }
        }
        {
            let run = &mut state.runs[index];
            match update {
                DispatchUpdate::SetupPlanned { setup } => {
                    run.setup = Some(setup);
                    run.stage = RunStage::Starting;
                    run.dispatch = Some(dispatch(DispatchStep::SetupPending));
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
                DispatchUpdate::LaunchIntent {
                    launch_tag,
                    launch_attempt,
                    endpoint_identity,
                    scope_unit,
                    latch,
                    staging_workspace_id,
                } => {
                    if run.dispatch.as_ref().and_then(|dispatch| dispatch.staging_workspace_id.as_ref())
                        .is_some_and(|saved| staging_workspace_id.as_ref() != Some(saved)) {
                        return Err(error("staging_space_conflict", "Launch intent differs from its pinned authoritative staging Space"));
                    }
                    let attachment = run.attachment.as_mut().ok_or_else(|| error("attachment_missing", "Launch has no reserved attachment"))?;
                    attachment.shell = None;
                    attachment.process_scope = None;
                    attachment.launch_attempt = launch_attempt;
                    attachment.state = AttachmentState::Connecting;
                    attachment.launch_mode = Some(AttachmentLaunchMode::DirectProcess);
                    attachment.updated_at = now();
                    let d = run
                        .dispatch
                        .get_or_insert_with(|| dispatch(DispatchStep::LaunchIntent));
                    d.step = DispatchStep::LaunchIntent;
                    d.launch_tag = Some(launch_tag);
                    d.launch_attempt = launch_attempt;
                    d.endpoint_identity = Some(endpoint_identity);
                    d.scope_unit = scope_unit;
                    d.pre_move_scope = None;
                    d.latch_state = latch.as_ref().map(|_| AgentLatchState::Awaiting);
                    d.latch = latch;
                    d.source_location = None;
                    d.move_operation_id = None;
                    d.staging_workspace_id = staging_workspace_id;
                    d.agent_started = false;
                    d.error = None;
                    d.updated_at = now();
                }
                DispatchUpdate::TabReceipt { location, launch_shell_identity, process_scope } => {
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
                    if process_scope.as_ref().is_some_and(|scope| d.scope_unit.as_ref() != Some(&scope.unit)
                        || launch_shell_identity.as_ref().is_none_or(|shell| shell.process != scope.member)) {
                        return Err(error("scope_receipt_conflict", "Scope receipt does not match the exact launch unit and process"));
                    }
                    if d.move_operation_id.is_some() && run.attachment.as_ref().and_then(|attachment| attachment.location.as_ref())
                        .is_none_or(|saved| !same_placement_location(saved, &location)) {
                        return Err(error("placement_receipt_conflict", "Late tab receipt cannot overwrite a journalled moved terminal"));
                    }
                    let attachment = run.attachment.as_mut().ok_or_else(|| error("attachment_missing", "Tab receipt has no reserved attachment"))?;
                    if launch_shell_identity.as_ref().is_some_and(|shell|
                        attachment.process.as_ref().is_some_and(|saved| saved != &shell.process))
                        || process_scope.as_ref().is_some_and(|scope|
                            attachment.process_scope.as_ref().is_some_and(|saved| saved != scope)) {
                        return Err(error("attachment_conflict", "Late launch receipt differs from the current physical incarnation"));
                    }
                    if let Some(machine) = launch_shell_identity.as_ref().and_then(|shell| crate::process_identity::verified_machine_id(&shell.process)) {
                        attachment.machine_id = Some(machine);
                    }
                    attachment.location = Some(location);
                    if attachment.launch_mode == Some(AttachmentLaunchMode::DirectProcess) && attachment.process.is_none() {
                        attachment.process = launch_shell_identity.as_ref().map(|shell| shell.process.clone());
                    }
                    if let Some(shell) = launch_shell_identity { attachment.shell = Some(shell); }
                    if let Some(scope) = process_scope { attachment.process_scope = Some(scope); }
                    attachment.updated_at = now();
                }
            }
            run.updated_at = now();
        }
        escalation::failure(&mut state, index)?;
        let revision = locked.save(&mut state)?;
        self.revision.send_replace(revision);
        Ok(revision)
    }

    /// Resolve the checkout before this transaction, then reserve its writer and
    /// publish an immutable epoch credential before any external tab effect.
    pub(crate) fn reserve_dispatch_launch(
        &self,
        expected: &Run,
        project: ProjectIdentity,
        checkout: CheckoutIdentity,
        base_commit: Option<String>,
        input_facts: Option<&ScheduleInputFacts>,
    ) -> Result<(Run, std::path::PathBuf), InspectionError> {
        let locked = self.store.lock()?;
        let mut state = locked.read()?;
        project_identity::revalidate_checkout(&checkout)?;
        let index = run_index(&state, &expected.run_id)?;
        if state.runs[index].stage == RunStage::Closed
            || !same_launch_incarnation(&state.runs[index], expected)
            || state.runs[index].attachment.as_ref().is_some_and(|a| a.process.is_some()) {
            return Err(error("attachment_conflict", "Run changed before writer reservation"));
        }
        if let Some(saved) = &state.runs[index].checkout
            && saved != &checkout {
            return Err(error("checkout_identity_changed", "Saved physical checkout no longer matches"));
        }
        if let Some(saved) = &state.runs[index].project
            && saved.key != project.key {
            return Err(error("project_identity_changed", "Saved project no longer matches"));
        }
        if state.runs[index].stage != RunStage::Starting {
            return Err(error("invalid_stage", "Only a starting run may reserve a launch"));
        }
        if state.runs[index].kind == RunKind::Worker {
            let run = &state.runs[index];
            let document = locked.tasks(&run.root_id)?;
            super::tasks::task_transition_target(&state, run, &document)?;
            let task = mutable_task(&state, &run.root_id, run.task_id.as_deref().expect("task guarded"), &document)?;
            if run.task_revision_at_start.as_deref() != Some(task.task_revision.as_str()) {
                return Err(error("task_revision_changed", "Canonical task changed before launch"));
            }
            let inputs = super::runs::input_snapshot(&state, &locked, &run.session_id, &run.root_id,
                run.task_id.as_deref().expect("task guarded"))?;
            if let Some(verified_base) = super::runs::materialized_inputs(&inputs, &project, input_facts)? {
                if base_commit.as_ref() != Some(&verified_base) {
                    return Err(error("dependency_input_changed", "Checkout HEAD changed after accepted input verification"));
                }
            }
        }
        if state.runs[index].attachment.is_none() {
            state.runs[index].attachment = Some(ExecutionAttachment {
                attachment_id: Uuid::new_v4().to_string(), epoch: 1, state: AttachmentState::Reserved,
                location: None, conversation: None, process: None, shell: None,
                machine_id: None, launch_mode: None, launch_attempt: 0, updated_at: now(),
                process_scope: None,
            });
        }
        let run = &state.runs[index];
        let attachment = run.attachment.as_ref().expect("reserved");
        if matches!(attachment.state, AttachmentState::Revoked | AttachmentState::Quiescing) {
            return Err(error("attachment_conflict", "Attachment cannot launch while quiescing or revoked"));
        }
        let reference = claims::AttachmentReference { run_id: run.run_id.clone(), attachment_id: attachment.attachment_id.clone(), attachment_epoch: attachment.epoch };
        let role = if run.kind == RunKind::Worker { JobRole::Worker } else { JobRole::Root };
        let operation_id = attachment.attachment_id.clone();
        if role == JobRole::Worker {
            claims::reserve_claim(&mut state, &checkout, &reference, &operation_id)?;
        }
        state.runs[index].project = Some(project);
        state.runs[index].checkout = Some(checkout);
        let setup = state.runs[index].setup.as_mut()
            .ok_or_else(|| error("setup_missing", "Launch has no actual setup receipt"))?;
        if setup.base_commit.is_none() {
            setup.base_commit = base_commit;
        }
        let existing = state.credentials.iter().find(|c| c.run_id == reference.run_id
            && c.attachment_id == reference.attachment_id && c.attachment_epoch == reference.attachment_epoch
            && c.role == role && c.assignment_generation == state.runs[index].assignment_generation
            && c.state == credentials::CredentialState::Active);
        let credential_file = if let Some(record) = existing {
            credentials::credential_path(self.base(), record)?
        } else {
            let (record, path) = credentials::stage_credential(self.base(), &state.runs[index], role)?;
            state.credentials.push(record);
            path
        };
        let revision = locked.save(&mut state)?;
        self.revision.send_replace(revision);
        Ok((state.runs[index].clone(), credential_file))
    }
}

impl OrchestrationService {
    /// Late capture is an observation of the already authorized incarnation,
    /// never a new launch or a change to semantic ownership.
    pub(crate) fn record_scope_capture(&self, expected: &Run, shell: NativeShellIdentity,
        scope: OwnedProcessScope) -> Result<bool, InspectionError> {
        self.record_agent_exec_receipt(expected, &shell)?;
        let locked = self.store.lock()?;
        let mut state = locked.read()?;
        let index = run_index(&state, &expected.run_id)?;
        let run = &state.runs[index];
        let Some(attachment) = &run.attachment else { return Ok(false); };
        let Some(prior) = &expected.attachment else { return Ok(false); };
        if run.stage == RunStage::Closed || matches!(attachment.state, AttachmentState::Quiescing | AttachmentState::Revoked)
            || run.attempt != expected.attempt || run.assignment_generation != expected.assignment_generation
            || attachment.attachment_id != prior.attachment_id || attachment.epoch != prior.epoch
            || attachment.location.as_ref().is_none_or(|location|
                prior.location.as_ref().is_none_or(|expected_location|
                    location.endpoint_identity != expected_location.endpoint_identity || location.session_id != expected_location.session_id
                    || location.peer_process != expected_location.peer_process
                    || location.workspace_id != expected_location.workspace_id || location.tab_id != expected_location.tab_id
                    || location.pane_id != expected_location.pane_id || location.terminal_id != expected_location.terminal_id
                    || location.launch_tag != expected_location.launch_tag))
            || run.dispatch.as_ref().is_none_or(|dispatch| dispatch.scope_unit.as_ref() != Some(&scope.unit)
                || expected.dispatch.as_ref().is_none_or(|expected_dispatch|
                    dispatch.launch_tag != expected_dispatch.launch_tag || dispatch.launch_attempt != expected_dispatch.launch_attempt))
            || scope.member != shell.process || attachment.process.as_ref() != Some(&scope.member)
            || attachment.process_scope.is_some() {
            return Ok(false);
        }
        if run.kind == RunKind::Worker {
            let Some(task_id) = &run.task_id else { return Ok(false); };
            if projection::current_task_run(&state, &run.root_id, task_id).is_none_or(|current| current.run_id != run.run_id) {
                return Ok(false);
            }
        }
        state.runs[index].attachment.as_mut().expect("fenced incarnation").process_scope = Some(scope);
        let revision = locked.save(&mut state)?;
        self.revision.send_replace(revision);
        Ok(true)
    }
}

impl OrchestrationService {
    /// Capture the waiting wrapper's actual owned scope before any pane move.
    /// This is diagnostic temporal evidence, not a grant of semantic authority.
    pub(crate) fn record_pre_move_scope(&self, expected: &Run, scope: OwnedProcessScope) -> Result<bool, InspectionError> {
        let locked = self.store.lock()?;
        let mut state = locked.read()?;
        let index = run_index(&state, &expected.run_id)?;
        let run = &state.runs[index];
        if !placement_attempt_matches(&state, run, expected) { return Ok(false); }
        let dispatch = run.dispatch.as_ref().expect("fenced launch");
        if !matches!(dispatch.step, DispatchStep::LaunchIntent | DispatchStep::LaunchPending | DispatchStep::LaunchUnknown)
            || dispatch.latch_state != Some(AgentLatchState::Awaiting) || dispatch.move_operation_id.is_some()
            || dispatch.pre_move_scope.is_some() || dispatch.scope_unit.as_ref() != Some(&scope.unit)
            || run.attachment.as_ref().is_none_or(|attachment| attachment.process.as_ref() != Some(&scope.member)
                || attachment.location.as_ref().is_none_or(|location| location.terminal_id.is_none())) {
            return Ok(false);
        }
        state.runs[index].dispatch.as_mut().expect("fenced launch").pre_move_scope = Some(scope);
        let revision = locked.save(&mut state)?;
        self.revision.send_replace(revision);
        Ok(true)
    }

    /// The latch wrapper execs the harness without changing PID/start identity.
    /// Freshly attest its post-release executable even when a fast context call
    /// already activated the run; never change semantic ownership or history.
    pub(crate) fn record_agent_exec_receipt(&self, expected: &Run, shell: &NativeShellIdentity) -> Result<bool, InspectionError> {
        let Ok(pid) = i32::try_from(shell.process.pid) else { return Ok(false); };
        if crate::process_identity::executable_identity(pid, shell.process.start_ticks).ok().as_ref() != Some(shell) {
            return Ok(false);
        }
        let locked = self.store.lock()?;
        let mut state = locked.read()?;
        let index = run_index(&state, &expected.run_id)?;
        let run = &state.runs[index];
        if !placement_attempt_matches(&state, run, expected) { return Ok(false); }
        let dispatch = run.dispatch.as_ref().expect("fenced launch");
        if !matches!(dispatch.latch_state, Some(AgentLatchState::ReleaseIntent | AgentLatchState::Released))
            || (dispatch.move_operation_id.is_some() && !matches!(dispatch.step, DispatchStep::Placed | DispatchStep::LaunchPending | DispatchStep::Launched))
            || run.attachment.as_ref().is_none_or(|attachment| attachment.process.as_ref() != Some(&shell.process)
                || attachment.location.as_ref().is_none_or(|location| expected.attachment.as_ref()
                    .and_then(|prior| prior.location.as_ref()).is_none_or(|prior| !same_placement_location(location, prior)))
                || attachment.shell.as_ref() == Some(shell)) {
            return Ok(false);
        }
        let attachment = state.runs[index].attachment.as_mut().expect("fenced incarnation");
        attachment.shell = Some(shell.clone());
        if let Some(machine) = crate::process_identity::verified_machine_id(&shell.process) {
            attachment.machine_id = Some(machine);
        }
        attachment.updated_at = now();
        let revision = locked.save(&mut state)?;
        self.revision.send_replace(revision);
        Ok(true)
    }
}

fn same_placement_location(left: &RunLocation, right: &RunLocation) -> bool {
    left.boot_id == right.boot_id && left.terminal_id == right.terminal_id
        && left.peer_process == right.peer_process
        && left.native_session_id == right.native_session_id && left.endpoint_identity == right.endpoint_identity
        && left.session_id == right.session_id && left.workspace_id == right.workspace_id
        && left.tab_id == right.tab_id && left.pane_id == right.pane_id && left.launch_tag == right.launch_tag
}

fn placement_attempt_matches(state: &OrchestrationState, current: &Run, expected: &Run) -> bool {
    let (Some(attachment), Some(prior), Some(dispatch), Some(previous)) =
        (&current.attachment, &expected.attachment, &current.dispatch, &expected.dispatch) else { return false; };
    current.stage != RunStage::Closed && current.run_id == expected.run_id && current.root_id == expected.root_id
        && current.session_id == expected.session_id && current.attempt == expected.attempt
        && current.assignment_generation == expected.assignment_generation && current.task_id == expected.task_id
        && attachment.attachment_id == prior.attachment_id && attachment.epoch == prior.epoch
        && attachment.launch_attempt == prior.launch_attempt
        && !matches!(attachment.state, AttachmentState::Quiescing | AttachmentState::Revoked)
        && dispatch.launch_attempt == previous.launch_attempt && dispatch.launch_tag == previous.launch_tag
        && dispatch.endpoint_identity == previous.endpoint_identity && dispatch.scope_unit == previous.scope_unit
        && dispatch.staging_workspace_id == previous.staging_workspace_id
        && dispatch.latch.is_some() && dispatch.latch == previous.latch
        && (current.kind != RunKind::Worker || current.task_id.as_ref().is_some_and(|task_id|
            projection::current_task_run(state, &current.root_id, task_id).is_some_and(|run| run.run_id == current.run_id)))
}

impl OrchestrationService {
    /// Authorize one pane move. A durable intent is never permission to re-send
    /// an uncertain adapter request; recovery may only reconcile its terminal.
    pub(crate) fn record_move_intent(&self, expected: &Run, source: RunLocation,
        move_operation_id: &str) -> Result<bool, InspectionError> {
        tasks_md::validate_uuid(move_operation_id)?;
        let locked = self.store.lock()?;
        let mut state = locked.read()?;
        let index = run_index(&state, &expected.run_id)?;
        let run = &state.runs[index];
        if !placement_attempt_matches(&state, run, expected) { return Ok(false); }
        let dispatch = run.dispatch.as_ref().expect("fenced launch");
        if !matches!(dispatch.step, DispatchStep::LaunchIntent | DispatchStep::LaunchPending | DispatchStep::LaunchUnknown)
            || dispatch.latch_state != Some(AgentLatchState::Awaiting) || dispatch.move_operation_id.is_some()
            || source.terminal_id.as_ref().is_none_or(|terminal| terminal.is_empty())
            || dispatch.staging_workspace_id.as_ref().is_some_and(|workspace| workspace != &source.workspace_id)
            || run.attachment.as_ref().and_then(|attachment| attachment.location.as_ref())
                .is_none_or(|location| !same_placement_location(location, &source))
            || source.session_id != run.session_id || dispatch.endpoint_identity.as_ref() != Some(&source.endpoint_identity)
            || dispatch.launch_tag.as_ref() != Some(&source.launch_tag) {
            return Ok(false);
        }
        let dispatch = state.runs[index].dispatch.as_mut().expect("fenced launch");
        dispatch.step = DispatchStep::MoveIntent;
        dispatch.source_location = Some(source);
        dispatch.move_operation_id = Some(move_operation_id.into());
        dispatch.error = None;
        dispatch.updated_at = now();
        let revision = locked.save(&mut state)?;
        self.revision.send_replace(revision);
        Ok(true)
    }

    pub(crate) fn record_move_unknown(&self, expected: &Run, failure: ErrorResponse) -> Result<bool, InspectionError> {
        let locked = self.store.lock()?;
        let mut state = locked.read()?;
        let index = run_index(&state, &expected.run_id)?;
        let run = &state.runs[index];
        if !placement_attempt_matches(&state, run, expected) { return Ok(false); }
        let dispatch = run.dispatch.as_ref().expect("fenced launch");
        if dispatch.step != DispatchStep::MoveIntent || dispatch.move_operation_id.is_none()
            || expected.dispatch.as_ref().is_none_or(|previous| previous.move_operation_id != dispatch.move_operation_id) {
            return Ok(false);
        }
        let dispatch = state.runs[index].dispatch.as_mut().expect("fenced launch");
        dispatch.step = DispatchStep::MoveUnknown;
        dispatch.error = Some(failure);
        dispatch.updated_at = now();
        escalation::failure(&mut state, index)?;
        let revision = locked.save(&mut state)?;
        self.revision.send_replace(revision);
        Ok(true)
    }

    /// Commit only the actual moved terminal and the filesystem operation's
    /// freshly bound Space receipt. No semantic state or history is reset.
    pub(crate) fn record_placement(&self, expected: &Run, location: RunLocation,
        setup: SetupSummary) -> Result<bool, InspectionError> {
        let locked = self.store.lock()?;
        let mut state = locked.read()?;
        let index = run_index(&state, &expected.run_id)?;
        let run = &state.runs[index];
        if !placement_attempt_matches(&state, run, expected) { return Ok(false); }
        let dispatch = run.dispatch.as_ref().expect("fenced launch");
        let Some(source) = &dispatch.source_location else { return Ok(false); };
        let Some(previous_setup) = &run.setup else { return Ok(false); };
        let retained_space_rebind = run.kind == RunKind::Worker && previous_setup.workspace_id.as_ref()
            .is_some_and(|workspace| workspace != &source.workspace_id);
        if retained_space_rebind {
            let attachment = run.attachment.as_ref().expect("fenced incarnation");
            let resume = run.resume.as_ref().filter(|resume|
                matches!(resume.phase, ResumePhase::Launching | ResumePhase::Connected)
                    && resume.run_id == run.run_id
                    && resume.replacement_attachment_epoch == Some(attachment.epoch)
                    && resume.previous_attachment_epoch.is_some_and(|previous|
                        previous > 0 && previous.checked_add(1) == Some(attachment.epoch)))
                .ok_or_else(|| error("placement_receipt_conflict", "A retained checkout can receive a new Space only after its exact owner Resume epoch cutover"))?;
            if state.credentials.iter().any(|credential| credential.run_id == run.run_id
                && credential.state == credentials::CredentialState::Active
                && (credential.attachment_id != attachment.attachment_id || credential.attachment_epoch != attachment.epoch))
                || (run.kind == RunKind::Worker && run.checkout_claim_id.as_ref().is_none_or(|claim_id|
                    state.checkout_claims.iter().find(|claim| &claim.claim_id == claim_id).is_none_or(|claim|
                        !matches!(claim.state, claims::ClaimState::Reserved | claims::ClaimState::Held)
                            || claim.transition_operation_id.as_ref() != Some(&resume.operation_id)
                            || claim.holder.as_ref().is_none_or(|holder|
                                holder.run_id != run.run_id || holder.attachment_id != attachment.attachment_id
                                    || holder.attachment_epoch != attachment.epoch)))) {
                return Err(error("placement_receipt_conflict", "Retained checkout rebinding requires revoked old credentials and the current exact writer reservation"));
            }
        }
        let borrowed_resume_rebind = retained_space_rebind
            && matches!(previous_setup.ownership,
                None | Some(cockpit_protocol::projects::WorkspaceCheckoutOwnership::BorrowedDirectory))
            && run.checkout.as_ref().is_some_and(|checkout|
                checkout.path == previous_setup.checkout_path && expected.checkout.as_ref() == Some(checkout));
        if !matches!(dispatch.step, DispatchStep::MoveIntent | DispatchStep::MoveUnknown)
            || dispatch.move_operation_id.is_none()
            || expected.dispatch.as_ref().is_none_or(|previous| previous.move_operation_id != dispatch.move_operation_id)
            || source.terminal_id.is_none() || location.terminal_id != source.terminal_id
            || location.endpoint_identity != source.endpoint_identity || location.session_id != source.session_id
            || location.boot_id != source.boot_id || location.workspace_id == source.workspace_id
            || location.peer_process != source.peer_process
            || location.launch_tag != source.launch_tag || dispatch.launch_tag.as_ref() != Some(&location.launch_tag)
            || location.workspace_id.is_empty() || location.tab_id.is_empty() || location.pane_id.is_empty()
            || setup.workspace_id.as_ref() != Some(&location.workspace_id)
            || setup.ownership != previous_setup.ownership
            || !match setup.ownership {
                Some(cockpit_protocol::projects::WorkspaceCheckoutOwnership::OwnedWorktree |
                    cockpit_protocol::projects::WorkspaceCheckoutOwnership::OwnedAttemptDirectory) => true,
                Some(cockpit_protocol::projects::WorkspaceCheckoutOwnership::BorrowedDirectory) =>
                    borrowed_resume_rebind
                        || matches!(&run.target, Some(DispatchTarget::Setup { request: WorkspaceSetupRequest::Open { .. } })),
                None => borrowed_resume_rebind,
            }
            || (!borrowed_resume_rebind && (previous_setup.operation_id.is_none() || previous_setup.generation.is_none()))
            || setup.operation_id != previous_setup.operation_id || setup.generation != previous_setup.generation
            || setup.checkout_path != previous_setup.checkout_path || setup.repository_id != previous_setup.repository_id
            || setup.branch != previous_setup.branch || setup.base != previous_setup.base
            || setup.base_commit != previous_setup.base_commit || setup.project_workspace_id != previous_setup.project_workspace_id {
            return Err(error("placement_receipt_conflict", "Moved Space receipt differs from the exact launch terminal or prepared checkout operation"));
        }
        let run = &mut state.runs[index];
        run.setup = Some(setup);
        if run.run_id == run.root_id && run.task_id.is_none() {
            if let Some(project) = &mut run.project {
                if project.source_workspace_id.is_none() {
                    project.source_workspace_id = Some(location.workspace_id.clone());
                }
            }
        }
        let attachment = run.attachment.as_mut().expect("fenced incarnation");
        attachment.location = Some(location);
        attachment.updated_at = now();
        let dispatch = run.dispatch.as_mut().expect("fenced launch");
        dispatch.step = DispatchStep::Placed;
        dispatch.error = None;
        dispatch.updated_at = now();
        run.updated_at = now();
        let revision = locked.save(&mut state)?;
        self.revision.send_replace(revision);
        Ok(true)
    }

    fn transition_latch(&self, expected: &Run, from: AgentLatchState, to: AgentLatchState,
        failure: Option<ErrorResponse>, input_facts: Option<&ScheduleInputFacts>) -> Result<Option<Run>, InspectionError> {
        let locked = self.store.lock()?;
        let mut state = locked.read()?;
        let index = run_index(&state, &expected.run_id)?;
        let run = &state.runs[index];
        if !placement_attempt_matches(&state, run, expected) { return Ok(None); }
        let dispatch = run.dispatch.as_ref().expect("fenced launch");
        if dispatch.latch_state != Some(from) || run.attachment.as_ref().and_then(|attachment| attachment.location.as_ref()).is_none()
            || (dispatch.move_operation_id.is_some() && dispatch.step != DispatchStep::Placed && dispatch.step != DispatchStep::Launched)
            || expected.dispatch.as_ref().is_none_or(|previous| previous.move_operation_id != dispatch.move_operation_id) {
            return Ok(None);
        }
        if to == AgentLatchState::ReleaseIntent {
            if let Some(task_id) = &run.task_id {
                let document = locked.tasks(&run.root_id)?;
                let task = document.task(task_id)?;
                dependencies::DependencyGraph::new(&document.tasks).require(task)?;
                let project = run.project.as_ref().ok_or_else(|| error("dependency_input_unavailable", "Launch has no resolved source project"))?;
                let snapshot = super::runs::input_snapshot(&state, &locked, &run.session_id, &run.root_id, task_id)?;
                if let Some(base) = super::runs::materialized_inputs(&snapshot, project, input_facts)? {
                    if run.setup.as_ref().and_then(|setup| setup.base_commit.as_ref()) != Some(&base) {
                        return Err(error("dependency_input_changed", "Launch checkout no longer has the reserved input base"));
                    }
                }
            }
        }
        let registered = has_current_binding(&state, run);
        let run = &mut state.runs[index];
        let dispatch = run.dispatch.as_mut().expect("fenced launch");
        dispatch.latch_state = Some(to);
        dispatch.error = failure;
        if to == AgentLatchState::Released && !registered {
            dispatch.step = DispatchStep::LaunchPending;
        }
        dispatch.updated_at = now();
        run.updated_at = now();
        let result = run.clone();
        if to == AgentLatchState::Unknown { escalation::failure(&mut state, index)?; }
        let revision = locked.save(&mut state)?;
        self.revision.send_replace(revision);
        Ok(Some(result))
    }

    pub(crate) fn record_latch_release_intent(&self, expected: &Run, input_facts: Option<&ScheduleInputFacts>) -> Result<Option<Run>, InspectionError> {
        self.transition_latch(expected, AgentLatchState::Awaiting, AgentLatchState::ReleaseIntent, None, input_facts)
    }

    pub(crate) fn record_latch_released(&self, expected: &Run) -> Result<bool, InspectionError> {
        Ok(self.transition_latch(expected, AgentLatchState::ReleaseIntent, AgentLatchState::Released, None, None)?.is_some())
    }

    pub(crate) fn record_latch_release_unknown(&self, expected: &Run, failure: ErrorResponse) -> Result<bool, InspectionError> {
        Ok(self.transition_latch(expected, AgentLatchState::ReleaseIntent, AgentLatchState::Unknown, Some(failure), None)?.is_some())
    }

    /// ENXIO is the only retryable FIFO-write observation: no writer opened,
    /// therefore no environment bytes could have reached the waiting wrapper.
    pub(crate) fn record_latch_reader_absent(&self, expected: &Run) -> Result<bool, InspectionError> {
        Ok(self.transition_latch(expected, AgentLatchState::ReleaseIntent, AgentLatchState::Awaiting, None, None)?.is_some())
    }
}

impl OrchestrationService {
    /// A nonce-bound startup marker explicitly proved the wrapper could not
    /// create a scope. Clear only this attempt's unfulfilled scope intent.
    pub(crate) fn record_scope_uncontained(&self, expected: &Run) -> Result<bool, InspectionError> {
        let locked = self.store.lock()?;
        let mut state = locked.read()?;
        let index = run_index(&state, &expected.run_id)?;
        let run = &state.runs[index];
        if !placement_attempt_matches(&state, run, expected) { return Ok(false); }
        let dispatch = run.dispatch.as_ref().expect("fenced launch");
        if dispatch.latch_state != Some(AgentLatchState::Awaiting) || dispatch.pre_move_scope.is_some()
            || run.attachment.as_ref().is_some_and(|attachment| attachment.process_scope.is_some())
            || dispatch.move_operation_id.is_some() || dispatch.scope_unit.is_none() {
            return Ok(false);
        }
        state.runs[index].dispatch.as_mut().expect("fenced launch").scope_unit = None;
        let revision = locked.save(&mut state)?;
        self.revision.send_replace(revision);
        Ok(true)
    }
}
