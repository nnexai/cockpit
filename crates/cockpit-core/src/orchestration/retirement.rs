use super::*;
use super::herdr::{PaneProcessInfo, RuntimeView};

pub(crate) const OBSERVATION_RETRY_SECS: u64 = 30;

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
        process: attachment.process.clone(), shell: attachment.shell.clone(),
    })
}

pub(crate) fn retirement_for_accepted(run: &Run, trigger: RetirementTrigger, task_revision: &str, at: &str) -> Option<RunRetirement> {
    if run.kind != RunKind::Worker || run.dispatch.is_none() { return None; }
    let result = run.result.as_ref()?;
    if result.outcome != Some(ReportOutcome::Succeeded) { return None; }
    let identity = identity_for(run);
    let state = if identity.as_ref().is_some_and(|identity| identity.process.is_some() && identity.shell.is_some()) { RetirementState::Waiting { blockers: Vec::new() } }
        else { RetirementState::Retained { at: at.into(), reason: RetainReason::IdentityIncomplete, stopped: false } };
    Some(RunRetirement { retirement_id: id(), trigger, result_message_id: result.message_id.clone(),
        task_revision: task_revision.into(), identity, state, created_at: at.into(), updated_at: at.into() })
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
}

pub(crate) fn blockers(state: &OrchestrationState, run: &Run) -> Vec<RetirementBlocker> {
    if state.runs.iter().any(|child| child.stage != RunStage::Closed && is_ancestor(state, &run.run_id, &child.run_id)) {
        vec![RetirementBlocker::OpenDescendantRuns]
    } else { Vec::new() }
}

pub(crate) fn identity_matches(run: &Run, retirement: &RunRetirement) -> bool {
    run.kind == RunKind::Worker && run.stage == RunStage::Closed
        && run.close_reason == Some(CloseReason::Accepted)
        && run.result.as_ref().is_some_and(|result| result.outcome == Some(ReportOutcome::Succeeded)
            && result.message_id == retirement.result_message_id)
        && identity_for(run).as_ref() == retirement.identity.as_ref() && retirement.identity.is_some()
}

/// Joins creation and retirement receipts without rewriting late-bound metadata.
/// Missing conversation metadata is compatible; contradictory conversations and
/// any physical-incarnation difference are not.
pub(super) fn receipt_identity_matches(left: &RetirementIdentity, right: &RetirementIdentity) -> bool {
    let RetirementIdentity {
        attachment_id, attachment_epoch, launch_mode, run_attempt, launch_attempt,
        launch_tag, endpoint_identity, session_id, workspace_id, tab_id, pane_id,
        terminal_id, herdr_boot_id, conversation, process, shell,
    } = left;
    let RetirementIdentity {
        attachment_id: right_attachment_id, attachment_epoch: right_attachment_epoch,
        launch_mode: right_launch_mode, run_attempt: right_run_attempt,
        launch_attempt: right_launch_attempt, launch_tag: right_launch_tag,
        endpoint_identity: right_endpoint_identity, session_id: right_session_id,
        workspace_id: right_workspace_id, tab_id: right_tab_id, pane_id: right_pane_id,
        terminal_id: right_terminal_id, herdr_boot_id: right_herdr_boot_id,
        conversation: right_conversation, process: right_process, shell: right_shell,
    } = right;
    attachment_id == right_attachment_id && attachment_epoch == right_attachment_epoch
        && launch_mode == right_launch_mode && run_attempt == right_run_attempt
        && launch_attempt == right_launch_attempt && launch_tag == right_launch_tag
        && endpoint_identity == right_endpoint_identity && session_id == right_session_id
        && workspace_id == right_workspace_id && tab_id == right_tab_id
        && pane_id == right_pane_id && terminal_id == right_terminal_id
        && herdr_boot_id == right_herdr_boot_id && process == right_process && shell == right_shell
        && match (conversation, right_conversation) {
            (Some(left), Some(right)) => left == right,
            _ => true,
        }
}

impl OrchestrationService {
    pub(crate) fn retirement_queue(&self) -> Result<Vec<Run>, InspectionError> {
        Ok(self.store.lock()?.read()?.runs.into_iter().filter(|run| run.retirement.as_ref().is_some_and(|r|
            (!terminal(&r.state) || matches!(r.state, RetirementState::Unknown { .. }
                | RetirementState::Retained { reason: RetainReason::NativeProcessUnverifiable, .. }))
                && run.kind == RunKind::Worker && run.stage == RunStage::Closed
                && run.close_reason == Some(CloseReason::Accepted))).collect())
    }

    pub(crate) fn retirement_blockers(&self, run_id: &str) -> Result<Vec<RetirementBlocker>, InspectionError> {
        let state = self.store.lock()?.read()?;
        Ok(blockers(&state, &state.runs[run_index(&state, run_id)?]))
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
        let next = if absence || identity_matches(run, retirement) { next } else {
            RetirementState::Retained { at: now(), reason: RetainReason::IdentityChanged, stopped: false }
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
        let run = &mut state.runs[index];
        let current = run.retirement.as_ref().ok_or_else(|| error("retirement_state_changed", "Retirement is absent"))?;
        if current != observed
            || !matches!(current.state, RetirementState::CloseIntent { .. }
                | RetirementState::Retained { reason: RetainReason::NativeProcessUnverifiable, .. }) {
            return Err(error("retirement_state_changed", "Physical retirement state changed"));
        }
        let next = if identity_matches(run, current) { RetirementState::Waiting { blockers } }
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

pub(super) fn runtime_fence(identity: &RetirementIdentity, runtime: &RuntimeView) -> Result<(), RetainReason> {
    if identity.endpoint_identity != runtime.endpoint_identity { return Err(RetainReason::EndpointChanged); }
    if identity.herdr_boot_id.as_ref().is_some_and(|expected| runtime.boot_id.as_ref() != Some(expected)) {
        return Err(RetainReason::IdentityChanged);
    }
    Ok(())
}

pub(crate) fn classify_terminal(identity: &RetirementIdentity, runtime: &RuntimeView, info: &PaneProcessInfo) -> Result<(), TerminalDecision> {
    runtime_fence(identity, runtime).map_err(TerminalDecision::Retain)?;
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
    let absence = fresh.filter(|runtime| runtime_fence(identity, runtime).is_ok()).map(|runtime|
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
