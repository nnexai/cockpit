use super::*;
use super::herdr::{PaneProcessInfo, RuntimeView};

pub(crate) const OFFER_RESPONSE_TIMEOUT_SECS: i64 = 120;
pub(crate) const BUSY_TIMEOUT_SECS: i64 = 30 * 60;
pub(crate) const NATIVE_STOP_TIMEOUT_SECS: i64 = 120;
pub(crate) const OBSERVATION_TIMEOUT_SECS: i64 = 10 * 60;
pub(crate) const OBSERVATION_RETRY_SECS: u64 = 30;

pub(crate) fn terminal(state: &RetirementState) -> bool {
    matches!(state, RetirementState::Retired { .. } | RetirementState::Retained { .. } | RetirementState::Unknown { .. })
}

fn identity_for(run: &Run) -> Option<RetirementIdentity> {
    if !launch_receipt_coherent(run) { return None; }
    let location = run.location.as_ref()?;
    let dispatch = run.dispatch.as_ref()?;
    let process = run.bound_omp_process.as_ref()?;
    let shell = run.launch_shell_identity.as_ref()?;
    let omp_session_id = run.bound_omp_session.as_ref()?;
    let terminal_id = location.terminal_id.as_ref()?;
    if run.attempt == 0 || dispatch.launch_attempt == 0 || !dispatch.agent_started
        || process.pid == 0 || process.pid > i32::MAX as u32 || process.start_ticks == 0 || process.start_ticks > MAX_SAFE_COUNTER
        || shell.process.pid == 0 || shell.process.pid > i32::MAX as u32
        || shell.process.pid == process.pid || shell.process.start_ticks == 0 || shell.process.start_ticks > MAX_SAFE_COUNTER
        || [&shell.executable_device, &shell.executable_inode].iter().any(|value|
            value.parse::<u64>().is_err() || !value.bytes().all(|byte| byte.is_ascii_digit())
            || (value.len() > 1 && value.starts_with('0')))
        || shell.argv_digest.len() != 64
        || !shell.argv_digest.bytes().all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
        || !optional_available(&shell.process.kernel_boot_id, &process.kernel_boot_id)
        || [omp_session_id, terminal_id, &location.endpoint_identity, &location.session_id,
            &location.workspace_id, &location.tab_id, &location.pane_id].iter().any(|s| s.is_empty()) {
        return None;
    }
    Some(RetirementIdentity {
        run_attempt: run.attempt, launch_attempt: dispatch.launch_attempt,
        launch_tag: location.launch_tag.clone(), endpoint_identity: location.endpoint_identity.clone(),
        session_id: location.session_id.clone(), workspace_id: location.workspace_id.clone(),
        tab_id: location.tab_id.clone(), pane_id: location.pane_id.clone(),
        terminal_id: terminal_id.clone(), herdr_boot_id: location.boot_id.clone(),
        omp_session_id: omp_session_id.clone(), process: process.clone(),
        shell: shell.clone(),
    })
}

pub(crate) fn retirement_for_accepted(run: &Run, trigger: RetirementTrigger, task_revision: &str, at: &str) -> Option<RunRetirement> {
    if run.kind != RunKind::Worker || run.dispatch.is_none() { return None; }
    let result = run.result.as_ref()?;
    if result.outcome != Some(ReportOutcome::Succeeded) { return None; }
    let identity = identity_for(run);
    let state = if identity.is_some() { RetirementState::Waiting { blockers: Vec::new() } }
        else { RetirementState::Retained { at: at.into(), reason: RetainReason::IdentityIncomplete, native_stopped: false } };
    Some(RunRetirement {
        retirement_id: id(), trigger, result_message_id: result.message_id.clone(), task_revision: task_revision.into(),
        identity, state, created_at: at.into(), updated_at: at.into(),
    })
}

pub(crate) fn close_accepted(state: &mut OrchestrationState, index: usize, trigger: RetirementTrigger, task_revision: &str) {
    let at = now();
    let run = &mut state.runs[index];
    // Recovery must not backfill retirement for an acceptance already committed.
    let already_closed = run.stage == RunStage::Closed;
    run.stage = RunStage::Closed;
    run.close_reason = Some(CloseReason::Accepted);
    run.updated_at = at.clone();
    if !already_closed && run.retirement.is_none() {
        run.retirement = retirement_for_accepted(run, trigger, task_revision, &at);
        if let Some(retirement) = &run.retirement {
            run.annotations.push(Annotation { at, by: ActorRef::Dispatcher,
                text: format!("Accepted-worker retirement {} created for Result {} at task revision {}: {:?}.",
                    retirement.retirement_id, retirement.result_message_id, retirement.task_revision, retirement.state) });
        }
    }
}

pub(crate) fn blockers(state: &OrchestrationState, run: &Run) -> Vec<RetirementBlocker> {
    let mut result = Vec::new();
    if state.runs.iter().any(|child| child.stage != RunStage::Closed && is_ancestor(state, &run.run_id, &child.run_id)) {
        result.push(RetirementBlocker::OpenDescendantRuns);
    }
    if state.subagents.iter().any(|child| child.run_id == run.run_id && child.status == SubagentStatus::Running) {
        result.push(RetirementBlocker::RunningSubagents);
    }
    result
}

pub(crate) fn identity_matches(run: &Run, retirement: &RunRetirement) -> bool {
    let (Some(identity), Some(location), Some(dispatch)) =
        (&retirement.identity, &run.location, &run.dispatch) else { return false; };
    run.kind == RunKind::Worker && run.stage == RunStage::Closed && run.close_reason == Some(CloseReason::Accepted)
        && launch_receipt_coherent(run) && dispatch.agent_started
        && run.attempt == identity.run_attempt && dispatch.launch_attempt == identity.launch_attempt
        && location.launch_tag == identity.launch_tag && location.endpoint_identity == identity.endpoint_identity
        && location.session_id == identity.session_id && location.workspace_id == identity.workspace_id
        && location.tab_id == identity.tab_id && location.pane_id == identity.pane_id
        && location.terminal_id.as_deref() == Some(identity.terminal_id.as_str())
        && location.boot_id == identity.herdr_boot_id
        && run.bound_omp_session.as_deref() == Some(identity.omp_session_id.as_str())
        && run.bound_omp_process.as_ref() == Some(&identity.process)
        && run.launch_shell_identity.as_ref() == Some(&identity.shell)
        && run.result.as_ref().is_some_and(|result| result.outcome == Some(ReportOutcome::Succeeded)
            && result.message_id == retirement.result_message_id)
}

pub(crate) fn caller_location_matches(run: &Run, caller: &AgentCaller) -> bool {
    run.retirement.as_ref().and_then(|r| r.identity.as_ref()).is_some_and(|identity|
        caller.endpoint_identity == identity.endpoint_identity && caller.session_id == identity.session_id
        && caller.workspace_id == identity.workspace_id && caller.tab_id == identity.tab_id
        && caller.pane_id == identity.pane_id && caller.terminal_id.as_deref() == Some(identity.terminal_id.as_str())
        && optional_available(&identity.herdr_boot_id, &caller.boot_id))
}

pub(crate) fn apply_native_receipt(run: &mut Run, caller: &AgentCaller, retirement_id: &str, outcome: NativeStopReceipt) -> Result<(), InspectionError> {
    let retirement = run.retirement.as_ref().ok_or_else(|| error("retirement_not_found", "No retirement record"))?;
    let identity = retirement.identity.as_ref().ok_or_else(|| error("caller_mismatch", "Retirement identity is incomplete"))?;
    if caller.agent_kind != Some(AgentKind::Main) || caller.actual_agent_kind != Some(NativeAgentKind::Omp)
        || !caller.env_run.as_ref().is_some_and(|(run_id, attempt)| run_id == &run.run_id && *attempt == identity.run_attempt)
        || !identity_matches(run, retirement) || !caller_location_matches(run, caller)
        || caller.omp_session_id.as_deref() != Some(identity.omp_session_id.as_str())
        || caller.process.as_ref() != Some(&identity.process) {
        return Err(error("caller_mismatch", "Only the exact accepted worker main process may report retirement"));
    }
    if retirement.retirement_id != retirement_id {
        return Err(error("retirement_state_changed", "Retirement incarnation changed"));
    }
    let offered_at = match &retirement.state {
        RetirementState::NativeStopOffered { offered_at } | RetirementState::NativeStopDeferred { offered_at, .. } => offered_at,
        _ => return Err(error("retirement_state_changed", "Retirement no longer admits a native receipt")),
    };
    let at = now();
    let (next, explanation) = match outcome {
        NativeStopReceipt::ShutdownRequested => (RetirementState::NativeStopRequested { at: at.clone() }, None),
        NativeStopReceipt::Deferred { reason } => (RetirementState::NativeStopDeferred { offered_at: offered_at.clone(), reason, at: at.clone() }, None),
        NativeStopReceipt::Refused { reason, text } => {
            bounded(&text, 1024)?;
            (RetirementState::Retained { at: at.clone(), native_stopped: false,
                reason: match reason { NativeRefuseReason::UserActivity => RetainReason::UserActivity, NativeRefuseReason::NativeRefused => RetainReason::NativeRefused } }, Some(text))
        },
    };
    set_state(run, next, &at);
    if let Some(text) = explanation {
        run.annotations.push(Annotation { at, by: ActorRef::Dispatcher, text: format!("Native retirement refusal explanation: {text}") });
    }
    Ok(())
}

fn set_state(run: &mut Run, next: RetirementState, at: &str) {
    let retirement = run.retirement.as_mut().expect("retirement checked");
    run.annotations.push(Annotation { at: at.into(), by: ActorRef::Dispatcher,
        text: format!("Retirement {}: {:?} → {:?}.", retirement.retirement_id, retirement.state, next) });
    retirement.state = next;
    retirement.updated_at = at.into();
    run.updated_at = at.into();
}

#[derive(Clone, Copy)]
enum TransitionGuard {
    Any,
    Evidence(NativeStopEvidence),
    NativeStopped(bool),
    Terminal(TerminalOutcome),
    Phase(RetirementPhase),
}

impl TransitionGuard {
    fn allows(self, next: &RetirementState) -> bool {
        match (self, next) {
            (Self::Any, _) => true,
            (Self::Evidence(expected), RetirementState::NativeStopped { evidence, .. }) => expected == *evidence,
            (Self::NativeStopped(expected), RetirementState::Retained { native_stopped, .. }) => expected == *native_stopped,
            (Self::Terminal(expected), RetirementState::Retired { terminal, .. }) => expected == *terminal,
            (Self::Phase(expected), RetirementState::Unknown { phase, .. }) => expected == *phase,
            _ => false,
        }
    }
}

// Owner effects only: native receipts have their own payload and caller fences.
const OWNER_TRANSITIONS: &[(RetirementStateKind, RetirementStateKind, TransitionGuard)] = &[
    (RetirementStateKind::Waiting, RetirementStateKind::Waiting, TransitionGuard::Any),
    (RetirementStateKind::Waiting, RetirementStateKind::NativeStopOffered, TransitionGuard::Any),
    (RetirementStateKind::Waiting, RetirementStateKind::NativeStopped, TransitionGuard::Evidence(NativeStopEvidence::AlreadyExited)),
    (RetirementStateKind::Waiting, RetirementStateKind::Retained, TransitionGuard::NativeStopped(false)),
    (RetirementStateKind::NativeStopOffered, RetirementStateKind::Retained, TransitionGuard::NativeStopped(false)),
    (RetirementStateKind::NativeStopDeferred, RetirementStateKind::Retained, TransitionGuard::NativeStopped(false)),
    (RetirementStateKind::NativeStopRequested, RetirementStateKind::NativeStopped, TransitionGuard::Evidence(NativeStopEvidence::ExitedAfterShutdownRequest)),
    (RetirementStateKind::NativeStopRequested, RetirementStateKind::Unknown, TransitionGuard::Phase(RetirementPhase::NativeStop)),
    (RetirementStateKind::NativeStopped, RetirementStateKind::CloseIntent, TransitionGuard::Any),
    (RetirementStateKind::NativeStopped, RetirementStateKind::Retired, TransitionGuard::Terminal(TerminalOutcome::AlreadyAbsent)),
    (RetirementStateKind::NativeStopped, RetirementStateKind::Retained, TransitionGuard::NativeStopped(true)),
    (RetirementStateKind::CloseIntent, RetirementStateKind::Retired, TransitionGuard::Any),
    (RetirementStateKind::CloseIntent, RetirementStateKind::Retained, TransitionGuard::NativeStopped(true)),
    (RetirementStateKind::CloseIntent, RetirementStateKind::Unknown, TransitionGuard::Phase(RetirementPhase::TerminalClose)),
];

fn owner_transition(from: RetirementStateKind, next: &RetirementState) -> bool {
    let next_kind = next.kind();
    OWNER_TRANSITIONS.iter().any(|&(source, target, guard)|
        source == from && target == next_kind && guard.allows(next))
}

impl OrchestrationService {
    pub(crate) fn retirement_queue(&self) -> Result<Vec<Run>, InspectionError> {
        Ok(self.store.lock()?.read()?.runs.into_iter().filter(|run| run.stage == RunStage::Closed
            && run.close_reason == Some(CloseReason::Accepted) && run.retirement.as_ref().is_some_and(|r| !terminal(&r.state))).collect())
    }

    pub(crate) fn retirement_blockers(&self, run_id: &str) -> Result<Vec<RetirementBlocker>, InspectionError> {
        let state = self.store.lock()?.read()?;
        Ok(blockers(&state, &state.runs[run_index(&state, run_id)?]))
    }

    pub(crate) fn retirement_effect_authorized(&self, run_id: &str, retirement_id: &str) -> Result<bool, InspectionError> {
        let state = self.store.lock()?.read()?;
        let run = &state.runs[run_index(&state, run_id)?];
        Ok(run.retirement.as_ref().is_some_and(|r| r.retirement_id == retirement_id
            && matches!(r.state, RetirementState::CloseIntent { .. }) && identity_matches(run, r)))
    }

    pub(crate) fn record_retirement(&self, run_id: &str, retirement_id: &str, expected: RetirementStateKind, next: RetirementState) -> Result<u64, InspectionError> {
        let locked = self.store.lock()?;
        let mut state = locked.read()?;
        let index = run_index(&state, run_id)?;
        if matches!(next, RetirementState::NativeStopOffered { .. })
            && !blockers(&state, &state.runs[index]).is_empty() {
            return Err(error("retirement_state_changed", "Worker has open descendants or running subagents"));
        }
        let run = &mut state.runs[index];
        let retirement = run.retirement.as_ref().ok_or_else(|| error("retirement_state_changed", "Retirement is absent"))?;
        if retirement.retirement_id != retirement_id || retirement.state.kind() != expected || terminal(&retirement.state) {
            return Err(error("retirement_state_changed", "Retirement state or incarnation changed"));
        }
        if !owner_transition(expected, &next) {
            return Err(error("retirement_state_changed", "Unauthorized retirement transition"));
        }
        let next = if identity_matches(run, retirement) { next } else {
            RetirementState::Retained { at: now(), reason: RetainReason::IdentityChanged,
                native_stopped: matches!(expected, RetirementStateKind::NativeStopped | RetirementStateKind::CloseIntent) }
        };
        set_state(run, next, &now());
        let revision = locked.save(&mut state)?;
        self.revision.send_replace(revision);
        Ok(revision)
    }
}

#[derive(Debug, PartialEq, Eq)]
pub(crate) enum OfferDecision { Offer, AlreadyExited, Retain(RetainReason) }
#[derive(Debug, PartialEq, Eq)]
pub(crate) enum TerminalDecision { Absent, Retain(RetainReason) }

fn runtime_fence(identity: &RetirementIdentity, runtime: &RuntimeView) -> Result<(), RetainReason> {
    if identity.endpoint_identity != runtime.endpoint_identity { return Err(RetainReason::EndpointChanged); }
    if !optional_available(&identity.herdr_boot_id, &runtime.boot_id) { return Err(RetainReason::IdentityChanged); }
    Ok(())
}

fn pinned_pane<'a>(identity: &RetirementIdentity, runtime: &'a RuntimeView) -> Result<&'a herdr::RuntimePane, TerminalDecision> {
    let by_id = runtime.panes.iter().find(|p| p.pane_id == identity.pane_id);
    let by_terminal = runtime.panes.iter().find(|p| p.terminal_id.as_deref() == Some(identity.terminal_id.as_str()));
    let Some(pane) = by_id else {
        return Err(if by_terminal.is_none() { TerminalDecision::Absent } else { TerminalDecision::Retain(RetainReason::PaneMoved) });
    };
    if pane.terminal_id.as_deref() != Some(identity.terminal_id.as_str())
        || pane.workspace_id != identity.workspace_id || pane.tab_id != identity.tab_id
        || runtime.panes.iter().filter(|p| p.terminal_id.as_deref() == Some(identity.terminal_id.as_str())).count() != 1 {
        return Err(TerminalDecision::Retain(RetainReason::PaneMoved));
    }
    Ok(pane)
}

pub(crate) fn classify_offer(identity: &RetirementIdentity, runtime: &RuntimeView, info: Option<&PaneProcessInfo>, running: bool, kernel_boot_id: Option<&str>) -> OfferDecision {
    if let Err(reason) = runtime_fence(identity, runtime) { return OfferDecision::Retain(reason); }
    if matches!((&identity.process.kernel_boot_id, kernel_boot_id), (Some(expected), Some(actual)) if expected != actual) {
        return OfferDecision::Retain(RetainReason::IdentityChanged);
    }
    if identity.process.kernel_boot_id.is_some() && kernel_boot_id.is_none() {
        return OfferDecision::Retain(RetainReason::NativeProcessUnverifiable);
    }
    let pane = match pinned_pane(identity, runtime) {
        Ok(pane) => pane,
        Err(TerminalDecision::Absent) if !running => return OfferDecision::AlreadyExited,
        Err(TerminalDecision::Absent) => return OfferDecision::Retain(RetainReason::ProcessPaneMismatch),
        Err(TerminalDecision::Retain(reason)) => return OfferDecision::Retain(reason),
    };
    if !running {
        return if pane.agent_kind.as_deref().is_some_and(NativeAgentKind::is_omp) || pane.launch_pending { OfferDecision::Retain(RetainReason::ProcessPaneMismatch) }
            else { OfferDecision::AlreadyExited };
    }
    if !pane.agent_kind.as_deref().is_some_and(NativeAgentKind::is_omp) || pane.launch_pending
        || pane.native_session_id.as_ref().is_some_and(|s| s != &identity.omp_session_id) {
        return OfferDecision::Retain(RetainReason::ProcessPaneMismatch);
    }
    match info {
        Some(info) if info.pane_id == identity.pane_id && info.processes.iter().any(|(pid, _)| *pid == identity.process.pid) => OfferDecision::Offer,
        Some(_) => OfferDecision::Retain(RetainReason::ProcessPaneMismatch),
        None => OfferDecision::Retain(RetainReason::NativeProcessUnverifiable),
    }
}

/// Checks original-launch-shell foreground provenance, not whether the shell is
/// idle. After proven worker exit, the owned-pane policy permits closing that
/// original shell even if it is executing indistinguishable builtin work.
pub(crate) fn classify_terminal(identity: &RetirementIdentity, runtime: &RuntimeView, info: &PaneProcessInfo) -> Result<(), TerminalDecision> {
    runtime_fence(identity, runtime).map_err(TerminalDecision::Retain)?;
    let pane = pinned_pane(identity, runtime)?;
    if pane.tab_label != identity.launch_tag { return Err(TerminalDecision::Retain(RetainReason::TabRenamed)); }
    if runtime.panes.iter().any(|p| p.pane_id != pane.pane_id && p.workspace_id == pane.workspace_id && p.tab_id == pane.tab_id) {
        return Err(TerminalDecision::Retain(RetainReason::SharedTab));
    }
    if pane.agent_kind.is_some() || pane.launch_pending { return Err(TerminalDecision::Retain(RetainReason::ForegroundProcess)); }
    if info.pane_id != identity.pane_id || info.shell_pid.is_none() || info.foreground_pgid != info.shell_pid
        || info.processes.is_empty() || info.processes.iter().any(|(pid, _)| Some(*pid) != info.shell_pid) {
        return Err(TerminalDecision::Retain(RetainReason::ForegroundProcess));
    }
    let shell = info.shell_identity.as_ref()
        .ok_or(TerminalDecision::Retain(RetainReason::NativeProcessUnverifiable))?;
    if info.shell_pid != Some(identity.shell.process.pid) || shell != &identity.shell {
        // PID/PGID alone survive exec. Require the pinned original executable and
        // full argv fingerprint, including command arguments such as bash -c.
        return Err(TerminalDecision::Retain(RetainReason::ForegroundProcess));
    }
    Ok(())
}

pub(crate) fn classify_close(identity: &RetirementIdentity, response: Option<Result<(), InspectionError>>, fresh: Option<&RuntimeView>) -> RetirementState {
    let at = now();
    let absence = fresh.filter(|runtime| runtime_fence(identity, runtime).is_ok()).map(|runtime|
        !runtime.panes.iter().any(|p| p.pane_id == identity.pane_id || p.terminal_id.as_deref() == Some(identity.terminal_id.as_str())));
    match response {
        Some(Err(error)) if error.code != "herdr_outcome_unknown" && error.code != "pane_not_found" =>
            RetirementState::Retained { at, reason: RetainReason::HerdrRefused, native_stopped: true },
        Some(Err(error)) if error.code == "pane_not_found" && absence == Some(false) =>
            RetirementState::Retained { at, reason: RetainReason::PaneMoved, native_stopped: true },
        response if absence == Some(true) => RetirementState::Retired { at, terminal: match response {
            Some(Ok(())) => TerminalOutcome::ClosedByCockpit,
            Some(Err(error)) if error.code == "pane_not_found" => TerminalOutcome::AlreadyAbsent,
            _ => TerminalOutcome::AbsentAfterUncertainClose,
        } },
        _ => RetirementState::Unknown { at, phase: RetirementPhase::TerminalClose,
            detail: "A single read-only close verification could not confirm terminal absence; pane.close will not be retried.".into() },
    }
}
