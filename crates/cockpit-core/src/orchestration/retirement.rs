use super::*;
use super::herdr::{PaneProcessInfo, RuntimeView};

pub(crate) fn terminal(state: &RetirementState) -> bool {
    matches!(state, RetirementState::Retired { .. } | RetirementState::Retained { .. } | RetirementState::Unknown { .. })
}

pub(super) fn identity_for(run: &Run) -> Option<RetirementIdentity> {
    if !launch_receipt_coherent(run) { return None; }
    let attachment = run.attachment.as_ref()?;
    let location = attachment.location.as_ref()?;
    let dispatch = run.dispatch.as_ref()?;
    let terminal_id = location.terminal_id.as_ref()?;
    if terminal_id.is_empty() || attachment.launch_attempt != dispatch.launch_attempt { return None; }
    Some(RetirementIdentity {
        attachment_id: Some(attachment.attachment_id.clone()), attachment_epoch: Some(attachment.epoch),
        launch_mode: attachment.launch_mode, run_attempt: run.attempt, launch_attempt: dispatch.launch_attempt,
        launch_tag: location.launch_tag.clone(), endpoint_identity: location.endpoint_identity.clone(),
        session_id: location.session_id.clone(), workspace_id: location.workspace_id.clone(),
        tab_id: location.tab_id.clone(), pane_id: location.pane_id.clone(), terminal_id: terminal_id.clone(),
        herdr_boot_id: location.boot_id.clone(), conversation: attachment.conversation.clone(),
        herdr_peer: location.peer_process.clone(),
        process: attachment.process.clone(), shell: attachment.shell.clone(),
    })
}

pub(crate) fn retirement_for_accepted(run: &Run, trigger: RetirementTrigger, task_revision: &str, at: &str) -> Option<RunRetirement> {
    if run.kind != RunKind::Worker || run.dispatch.is_none() || trigger == RetirementTrigger::Replacement { return None; }
    let result = run.result.as_ref()?;
    if result.outcome != Some(ReportOutcome::Succeeded) { return None; }
    let identity = identity_for(run);
    let state = if identity.as_ref().is_some_and(complete_identity) { RetirementState::Waiting { blockers: Vec::new() } }
        else { RetirementState::Retained { at: at.into(), reason: RetainReason::IdentityIncomplete, stopped: false } };
    Some(RunRetirement { retirement_id: id(), trigger, result_message_id: Some(result.message_id.clone()),
        task_revision: Some(task_revision.into()), replacement: None, identity, state, created_at: at.into(), updated_at: at.into() })
}

pub(crate) fn retirement_for_replacement(run: &Run, authority: ReplacementRetirementAuthority, at: &str) -> Option<RunRetirement> {
    if run.kind != RunKind::Worker || run.attachment.as_ref().and_then(|attachment| attachment.launch_mode)
        != Some(AttachmentLaunchMode::DirectProcess) { return None; }
    let identity = identity_for(run);
    let complete = identity.as_ref().is_some_and(complete_identity)
        && authority.previous_run_id == run.run_id && run.assignment_generation == Some(authority.previous_generation);
    let state = if complete { RetirementState::Waiting { blockers: Vec::new() } }
        else { RetirementState::Retained { at: at.into(), reason: RetainReason::IdentityIncomplete, stopped: false } };
    Some(RunRetirement { retirement_id: id(), trigger: RetirementTrigger::Replacement,
        result_message_id: None, task_revision: None, replacement: Some(authority),
        identity, state, created_at: at.into(), updated_at: at.into() })
}

pub(crate) fn complete_peer(peer: &NativeProcessIdentity) -> bool {
    peer.pid > 0 && peer.start_ticks > 0 && peer.kernel_boot_id.as_deref()
        .is_some_and(|boot| uuid::Uuid::parse_str(boot).is_ok())
}

fn complete_identity(identity: &RetirementIdentity) -> bool {
    let (Some(process), Some(shell)) = (&identity.process, &identity.shell) else { return false };
    let legacy = identity.herdr_boot_id.as_deref().is_some_and(|boot| !boot.is_empty());
    let peer = identity.herdr_peer.as_ref().filter(|peer| complete_peer(peer));
    (legacy || peer.is_some()) && peer.is_none_or(|peer|
        process.kernel_boot_id == peer.kernel_boot_id && shell.process.kernel_boot_id == peer.kernel_boot_id)
}

pub(crate) fn record_shape(retirement: &RunRetirement) -> bool {
    if retirement.trigger == RetirementTrigger::Replacement {
        retirement.result_message_id.is_none() && retirement.task_revision.is_none() && retirement.replacement.is_some()
    } else {
        retirement.replacement.is_none() && retirement.result_message_id.as_deref().is_some_and(|pin| !pin.is_empty())
            && retirement.task_revision.as_deref().is_some_and(|pin| !pin.is_empty())
    }
}

pub(crate) fn replacement_authority_matches(state: &OrchestrationState, run: &Run, record: &RunRetirement) -> bool {
    if !record_shape(record) || record.trigger != RetirementTrigger::Replacement
        || run.stage != RunStage::Closed || run.close_reason != Some(CloseReason::Superseded) { return false; }
    let Some(authority) = record.replacement.as_ref() else { return false };
    let Some(task_id) = run.task_id.as_deref() else { return false };
    if authority.previous_run_id != run.run_id || run.assignment_generation != Some(authority.previous_generation)
        || !state.task_ownership.iter().any(|ownership| ownership.root_id == run.root_id && ownership.task_id == task_id
            && ownership.generation > authority.previous_generation) { return false; }
    state.operations.iter().any(|operation| operation.operation_id == authority.operation_id
        && operation.kind == super::store::DurableOperationKind::WorkerReplace
        && operation.root_id == run.root_id && operation.actor_run_id.as_deref() == Some(run.root_id.as_str())
        && matches!(operation.result.as_ref(), Some(OrchestrationActionResult::WorkerReplaced {
            start, previous_run_id, assignment_generation, ..
        }) if previous_run_id == &run.run_id && start.run_id == authority.replacement_run_id
            && start.operation_id == authority.operation_id && *assignment_generation > authority.previous_generation
            && state.runs.iter().any(|successor| successor.run_id == start.run_id && successor.root_id == run.root_id
                && successor.task_id.as_deref() == Some(task_id) && successor.assignment_generation == Some(*assignment_generation))))
}

pub(crate) fn authority_matches(state: &OrchestrationState, run: &Run, record: &RunRetirement) -> bool {
    identity_matches(run, record) && (record.trigger != RetirementTrigger::Replacement
        || replacement_authority_matches(state, run, record))
}

pub(crate) fn close_accepted(state: &mut OrchestrationState, index: usize, trigger: RetirementTrigger, task_revision: &str) {
    let at = now();
    let run = &mut state.runs[index];
    let already_closed = run.stage == RunStage::Closed;
    run.stage = RunStage::Closed;
    run.close_reason = Some(CloseReason::Accepted);
    run.updated_at = at.clone();
    if !already_closed && run.retirement.is_none() {
        run.retirement = retirement_for_accepted(run, trigger, task_revision, &at);
        if let Some(retirement) = &run.retirement {
            run.annotations.push(Annotation { at, by: ActorRef::Dispatcher,
                text: format!("Physical retirement {} recorded; logical acceptance is not consumer-exit proof.", retirement.retirement_id) });
        }
    }
    if let Some(task_id) = &run.task_id {
        if let Some(ownership) = state.task_ownership.iter_mut().find(|ownership|
            ownership.root_id == run.root_id && ownership.task_id == *task_id) {
            ownership.schedule = None;
        }
    }
}

pub(crate) fn blockers(state: &OrchestrationState, run: &Run) -> Vec<RetirementBlocker> {
    let mut blockers = Vec::new();
    if state.runs.iter().any(|child| child.stage != RunStage::Closed && is_ancestor(state, &run.run_id, &child.run_id)) {
        blockers.push(RetirementBlocker::OpenDescendantRuns);
    }
    if run.retirement.as_ref().is_some_and(|record| record.trigger == RetirementTrigger::Replacement)
        && run.attachment.as_ref().is_some_and(|attachment| state.resource_requests.iter().any(|request|
            request.holder.run_id == run.run_id && request.holder.attachment_id == attachment.attachment_id
                && request.holder.attachment_epoch == attachment.epoch
                && matches!(request.state, ResourceRequestState::Held | ResourceRequestState::Queued))) {
        blockers.push(RetirementBlocker::ResourceAdmissions);
    }
    blockers
}

pub(crate) fn identity_matches(run: &Run, retirement: &RunRetirement) -> bool {
    let logical = if retirement.trigger == RetirementTrigger::Replacement {
        run.close_reason == Some(CloseReason::Superseded)
            && run.attachment.as_ref().and_then(|attachment| attachment.launch_mode) == Some(AttachmentLaunchMode::DirectProcess)
            && retirement.replacement.as_ref().is_some_and(|authority| authority.previous_run_id == run.run_id
                && run.assignment_generation == Some(authority.previous_generation))
    } else {
        run.close_reason == Some(CloseReason::Accepted)
            && run.result.as_ref().is_some_and(|result| result.outcome == Some(ReportOutcome::Succeeded)
                && retirement.result_message_id.as_deref() == Some(result.message_id.as_str()))
    };
    run.kind == RunKind::Worker && run.stage == RunStage::Closed && record_shape(retirement) && logical
        && identity_for(run).as_ref() == retirement.identity.as_ref() && retirement.identity.is_some()
}

/// Joins creation and retirement receipts without rewriting late-bound metadata.
/// Missing conversation metadata is compatible; contradictory conversations and
/// any physical-incarnation difference are not.
pub(super) fn receipt_identity_matches(left: &RetirementIdentity, right: &RetirementIdentity) -> bool {
    let RetirementIdentity {
        attachment_id, attachment_epoch, launch_mode, run_attempt, launch_attempt,
        launch_tag, endpoint_identity, session_id, workspace_id, tab_id, pane_id,
        terminal_id, herdr_boot_id, herdr_peer, conversation, process, shell,
    } = left;
    let RetirementIdentity {
        attachment_id: right_attachment_id, attachment_epoch: right_attachment_epoch,
        launch_mode: right_launch_mode, run_attempt: right_run_attempt,
        launch_attempt: right_launch_attempt, launch_tag: right_launch_tag,
        endpoint_identity: right_endpoint_identity, session_id: right_session_id,
        workspace_id: right_workspace_id, tab_id: right_tab_id, pane_id: right_pane_id,
        terminal_id: right_terminal_id, herdr_boot_id: right_herdr_boot_id,
        herdr_peer: right_herdr_peer,
        conversation: right_conversation, process: right_process, shell: right_shell,
    } = right;
    attachment_id == right_attachment_id && attachment_epoch == right_attachment_epoch
        && launch_mode == right_launch_mode && run_attempt == right_run_attempt
        && launch_attempt == right_launch_attempt && launch_tag == right_launch_tag
        && endpoint_identity == right_endpoint_identity && session_id == right_session_id
        && workspace_id == right_workspace_id && tab_id == right_tab_id
        && pane_id == right_pane_id && terminal_id == right_terminal_id
        && herdr_boot_id == right_herdr_boot_id && herdr_peer == right_herdr_peer && process == right_process && shell == right_shell
        && match (conversation, right_conversation) {
            (Some(left), Some(right)) => left == right,
            _ => true,
        }
}

impl OrchestrationService {
    pub(crate) fn retirement_queue(&self) -> Result<Vec<Run>, InspectionError> {
        Ok(self.store.lock()?.read()?.runs.into_iter().filter(|run| run.retirement.as_ref().is_some_and(|r|
            (!terminal(&r.state) || matches!(r.state, RetirementState::Unknown { .. }
                | RetirementState::Retained { reason: RetainReason::NativeProcessUnverifiable | RetainReason::ObservationUnavailable, .. }))
                && run.kind == RunKind::Worker && run.stage == RunStage::Closed
                && (run.close_reason == Some(CloseReason::Accepted)
                    || (run.close_reason == Some(CloseReason::Superseded) && r.trigger == RetirementTrigger::Replacement)))).collect())
    }

    pub(crate) fn retirement_blockers(&self, run_id: &str) -> Result<Vec<RetirementBlocker>, InspectionError> {
        let state = self.store.lock()?.read()?;
        Ok(blockers(&state, &state.runs[run_index(&state, run_id)?]))
    }


    pub(crate) fn retirement_authorized(&self, run_id: &str, expected: &RunRetirement) -> Result<bool, InspectionError> {
        let state = self.store.lock()?.read()?;
        let run = &state.runs[run_index(&state, run_id)?];
        Ok(run.retirement.as_ref() == Some(expected) && authority_matches(&state, run, expected))
    }

    pub(crate) fn record_retirement(&self, run_id: &str, retirement_id: &str, expected: RetirementStateKind, next: RetirementState) -> Result<u64, InspectionError> {
        self.record_retirement_checked(run_id, retirement_id, expected, None, next)
    }

    pub(crate) fn record_retirement_exact(&self, run_id: &str, observed: &RunRetirement, next: RetirementState) -> Result<u64, InspectionError> {
        self.record_retirement_checked(run_id, &observed.retirement_id, observed.state.kind(), Some(observed), next)
    }

    fn record_retirement_checked(&self, run_id: &str, retirement_id: &str, expected: RetirementStateKind, observed: Option<&RunRetirement>, next: RetirementState) -> Result<u64, InspectionError> {
        let locked = self.store.lock()?;
        let mut state = locked.read()?;
        let index = run_index(&state, run_id)?;
        if matches!(next, RetirementState::CloseIntent { .. }) && !blockers(&state, &state.runs[index]).is_empty() {
            return Err(error("retirement_state_changed", "Worker has open descendant runs"));
        }
        let authorized = state.runs[index].retirement.as_ref().is_some_and(|record| authority_matches(&state, &state.runs[index], record));
        let run = &mut state.runs[index];
        let retirement = run.retirement.as_ref().ok_or_else(|| error("retirement_state_changed", "Retirement is absent"))?;
        let absence = matches!(expected, RetirementStateKind::CloseIntent | RetirementStateKind::Unknown)
            && matches!(next, RetirementState::Retired { terminal: TerminalOutcome::AbsentAfterUncertainClose, .. });
        let valid_transition = match expected {
            RetirementStateKind::Waiting => matches!(next, RetirementState::Waiting { .. } | RetirementState::CloseIntent { .. } | RetirementState::Retired { .. } | RetirementState::Retained { .. }),
            RetirementStateKind::CloseIntent => matches!(next, RetirementState::Retired { .. } | RetirementState::Retained { .. } | RetirementState::Unknown { .. }),
            RetirementStateKind::Unknown => absence,
            _ => false,
        };
        if retirement.retirement_id != retirement_id || retirement.state.kind() != expected || !valid_transition
            || observed.is_some_and(|observed| retirement != observed) {
            return Err(error("retirement_state_changed", "Physical retirement state changed"));
        }
        let next = if absence || authorized { next } else {
            RetirementState::Retained { at: now(), reason: if record_shape(retirement) && retirement.identity.is_some() { RetainReason::IdentityChanged }
                else { RetainReason::IdentityIncomplete }, stopped: false }
        };
        let at = now();
        let retirement = run.retirement.as_mut().expect("retirement");
        retirement.state = next;
        retirement.updated_at = at.clone();
        run.updated_at = at;
        // A pane-local close never releases checkout or detached-build claims.
        super::cleanup::sync_retirement_receipts(&mut state, index);
        let revision = locked.save(&mut state)?;
        self.revision.send_replace(revision);
        Ok(revision)
    }

    /// Only the executor may withdraw an intent it knows it has not submitted,
    /// or reobserve the old process-proof retention. Recovered intents never use this.
    pub(crate) fn retirement_waiting(&self, run_id: &str, observed: &RunRetirement, blockers: Vec<RetirementBlocker>) -> Result<(), InspectionError> {
        let locked = self.store.lock()?;
        let mut state = locked.read()?;
        let index = run_index(&state, run_id)?;
        let authorized = state.runs[index].retirement.as_ref().is_some_and(|record| authority_matches(&state, &state.runs[index], record));
        let run = &mut state.runs[index];
        let current = run.retirement.as_ref().ok_or_else(|| error("retirement_state_changed", "Retirement is absent"))?;
        if current != observed
            || !matches!(current.state, RetirementState::CloseIntent { .. }
                | RetirementState::Retained { reason: RetainReason::NativeProcessUnverifiable | RetainReason::ObservationUnavailable, .. }) {
            return Err(error("retirement_state_changed", "Physical retirement state changed"));
        }
        let next = if authorized { RetirementState::Waiting { blockers } }
            else { RetirementState::Retained { at: now(), reason: RetainReason::IdentityChanged, stopped: false } };
        let at = now();
        let current = run.retirement.as_mut().expect("retirement");
        current.state = next;
        current.updated_at = at.clone();
        run.updated_at = at;
        super::cleanup::sync_retirement_receipts(&mut state, index);
        let revision = locked.save(&mut state)?;
        self.revision.send_replace(revision);
        Ok(())
    }
}

#[derive(Debug, PartialEq, Eq)]
pub(crate) enum TerminalDecision { Absent, Retain(RetainReason) }

pub(crate) fn server_fence(identity: &RetirementIdentity, runtime: &RuntimeView) -> Result<(), RetainReason> {
    if identity.endpoint_identity != runtime.endpoint_identity { return Err(RetainReason::EndpointChanged); }
    if identity.herdr_boot_id.as_ref().zip(runtime.boot_id.as_ref()).is_some_and(|(saved, fresh)| saved != fresh) {
        return Err(RetainReason::IdentityChanged);
    }
    let saved_peer = identity.herdr_peer.as_ref().filter(|peer| complete_peer(peer));
    let fresh_peer = runtime.peer_process.as_ref().filter(|peer| complete_peer(peer));
    if saved_peer.zip(fresh_peer).is_some_and(|(saved, fresh)| saved != fresh) {
        return Err(RetainReason::IdentityChanged);
    }
    let legacy_matches = identity.herdr_boot_id.as_ref().zip(runtime.boot_id.as_ref())
        .is_some_and(|(saved, fresh)| !saved.is_empty() && saved == fresh);
    let peer_matches = saved_peer.zip(fresh_peer).is_some_and(|(saved, fresh)| saved == fresh);
    if legacy_matches || peer_matches { Ok(()) } else { Err(RetainReason::IdentityIncomplete) }
}

pub(crate) fn classify_terminal(identity: &RetirementIdentity, runtime: &RuntimeView, info: &PaneProcessInfo) -> Result<(), TerminalDecision> {
    server_fence(identity, runtime).map_err(TerminalDecision::Retain)?;
    let pane = runtime.panes.iter().find(|pane| pane.pane_id == identity.pane_id);
    let same_terminal = runtime.panes.iter().filter(|pane| pane.terminal_id.as_deref() == Some(identity.terminal_id.as_str())).count();
    let Some(pane) = pane else {
        return Err(if same_terminal == 0 { TerminalDecision::Absent } else { TerminalDecision::Retain(RetainReason::PaneMoved) });
    };
    if same_terminal != 1 || pane.terminal_id.as_deref() != Some(identity.terminal_id.as_str())
        || pane.workspace_id != identity.workspace_id || pane.tab_id != identity.tab_id {
        return Err(TerminalDecision::Retain(RetainReason::PaneMoved));
    }
    if pane.tab_label != identity.launch_tag { return Err(TerminalDecision::Retain(RetainReason::TabRenamed)); }
    if runtime.panes.iter().any(|other| other.pane_id != pane.pane_id && other.workspace_id == pane.workspace_id && other.tab_id == pane.tab_id) {
        return Err(TerminalDecision::Retain(RetainReason::SharedTab));
    }
    if identity.launch_mode != Some(AttachmentLaunchMode::DirectProcess) {
        return Err(TerminalDecision::Retain(RetainReason::UserActivity));
    }
    let process = identity.process.as_ref().ok_or(TerminalDecision::Retain(RetainReason::NativeProcessUnverifiable))?;
    let shell = identity.shell.as_ref().ok_or(TerminalDecision::Retain(RetainReason::NativeProcessUnverifiable))?;
    if shell.process != *process || info.pane_id != identity.pane_id
        || pane.agent_kind.as_ref().zip(identity.conversation.as_ref())
            .is_some_and(|(kind, conversation)| kind != &conversation.harness)
        || pane.native_session_id.as_ref().zip(identity.conversation.as_ref())
            .is_some_and(|(native, conversation)| native != &conversation.session_id)
        || info.shell_identity.as_ref() != Some(shell)
        || info.shell_pid != Some(process.pid) || info.foreground_pgid != Some(process.pid)
        || info.processes.len() != 1 || info.processes[0].0 != process.pid {
        return Err(TerminalDecision::Retain(RetainReason::ForegroundProcess));
    }
    Ok(())
}

pub(crate) fn classify_close(identity: &RetirementIdentity, response: Option<Result<(), InspectionError>>, fresh: Option<&RuntimeView>) -> RetirementState {
    let at = now();
    let absence = fresh.filter(|runtime| server_fence(identity, runtime).is_ok()).map(|runtime|
        !runtime.panes.iter().any(|pane| pane.pane_id == identity.pane_id || pane.terminal_id.as_deref() == Some(identity.terminal_id.as_str())));
    match response {
        Some(Err(error)) if error.code != "herdr_outcome_unknown" && error.code != "pane_not_found" =>
            RetirementState::Retained { at, reason: RetainReason::HerdrRefused, stopped: false },
        response if absence == Some(true) => RetirementState::Retired { at, terminal: match response {
            Some(Ok(())) => TerminalOutcome::ClosedByCockpit,
            Some(Err(error)) if error.code == "pane_not_found" => TerminalOutcome::AlreadyAbsent,
            _ => TerminalOutcome::AbsentAfterUncertainClose,
        } },
        _ => RetirementState::Unknown { at, phase: RetirementPhase::TerminalClose,
            detail: "Terminal absence could not be confirmed; pane.close will not be retried.".into() },
    }
}
