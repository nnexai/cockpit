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
                && current.attachment.as_ref().and_then(|a| a.main_session_id.as_ref()) == expected.attachment.as_ref().and_then(|a| a.main_session_id.as_ref())
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
                    let attachment = run.attachment.as_mut().ok_or_else(|| error("attachment_missing", "Launch has no reserved attachment"))?;
                    attachment.shell = None;
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
                    let attachment = run.attachment.as_mut().ok_or_else(|| error("attachment_missing", "Tab receipt has no reserved attachment"))?;
                    attachment.machine_id = launch_shell_identity.as_ref().and_then(|shell| crate::process_identity::verified_machine_id(&shell.process));
                    attachment.location = Some(location);
                    attachment.shell = launch_shell_identity;
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
    ) -> Result<(Run, std::path::PathBuf), InspectionError> {
        let locked = self.store.lock()?;
        let mut state = locked.read()?;
        project_identity::revalidate_checkout(&checkout)?;
        let index = run_index(&state, &expected.run_id)?;
        if state.runs[index].stage == RunStage::Closed
            || !same_launch_incarnation(&state.runs[index], expected)
            || state.runs[index].attachment.as_ref().is_some_and(|a| a.process.is_some() || a.main_session_id.is_some()) {
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
        }
        if state.runs[index].attachment.is_none() {
            state.runs[index].attachment = Some(ExecutionAttachment {
                attachment_id: Uuid::new_v4().to_string(), epoch: 1, state: AttachmentState::Reserved,
                location: None, main_session_id: None, process: None, shell: None,
                machine_id: None, launch_mode: None, launch_attempt: 0, updated_at: now(),
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
            && c.role == role && c.subject_child_id.is_none() && c.state == credentials::CredentialState::Active);
        let credential_file = if let Some(record) = existing {
            credentials::credential_path(self.base(), record)?
        } else {
            let (record, path) = credentials::stage_credential(self.base(), &state.runs[index], role, None)?;
            state.credentials.push(record);
            path
        };
        let revision = locked.save(&mut state)?;
        self.revision.send_replace(revision);
        Ok((state.runs[index].clone(), credential_file))
    }
}
