//! Receipt-owned physical terminal cleanup, independent of logical assignment.
use std::{collections::BTreeMap, time::{Duration, Instant}};
use parking_lot::Mutex;
use cockpit_protocol::orchestration::{NativeProcessIdentity, RetainReason, RetirementState, RetirementStateKind, Run, RunRetirement, TerminalOutcome};
use super::{dispatch::Dispatcher, herdr::OrchestrationHerdr, retirement::{self, TerminalDecision, OBSERVATION_RETRY_SECS}, now, OrchestrationService};
use crate::{process_identity, InspectionError};

#[derive(Default)]
pub(super) struct ObservationBackoff(Mutex<BTreeMap<String, (String, Instant)>>);
impl ObservationBackoff {
    fn ready(&self, run: &Run, record: &RunRetirement) -> bool {
        !self.0.lock().get(&run.run_id).is_some_and(|(id, after)| id == &record.retirement_id && Instant::now() < *after)
    }
    fn unavailable(&self, run: &Run, record: &RunRetirement) {
        self.0.lock().insert(run.run_id.clone(), (record.retirement_id.clone(), Instant::now() + Duration::from_secs(OBSERVATION_RETRY_SECS)));
    }
    fn clear(&self, run_id: &str) { self.0.lock().remove(run_id); }
    fn retain_queue(&self, queue: &[Run]) {
        self.0.lock().retain(|run_id, (id, _)| queue.iter().any(|run| &run.run_id == run_id && run.retirement.as_ref().is_some_and(|record| &record.retirement_id == id)));
    }
}

impl Dispatcher {
    pub(super) async fn retire(&self, run: &Run) -> Result<(), InspectionError> {
        retire_run(&self.service, self.herdr.as_ref(), &self.retirement_observations, run).await
    }
    pub(super) fn retain_retirement_observations(&self, queue: &[Run]) { self.retirement_observations.retain_queue(queue); }
}

pub(super) async fn retire_run(service: &OrchestrationService, herdr: &dyn OrchestrationHerdr, observations: &ObservationBackoff, queued: &Run) -> Result<(), InspectionError> {
    let Some(_lease) = service.execution_lease(&queued.run_id)? else { return Ok(()) };
    let run = service.run_for_review(&queued.session_id, &queued.run_id)?;
    let Some(record) = &run.retirement else { return Ok(()) };
    if queued.retirement.as_ref().is_none_or(|queued| queued.retirement_id != record.retirement_id) { return Ok(()) }
    if !observations.ready(&run, record) { return Ok(()) }
    let expected = record.state.kind();
    if retirement::terminal(&record.state) && expected != RetirementStateKind::Unknown { return Ok(()) }
    let Some(identity) = &record.identity else { return Ok(()) };
    let runtime = match herdr.runtime(&identity.session_id).await {
        Ok(runtime) => runtime,
        Err(_) => {
            observations.unavailable(&run, record);
            if expected == RetirementStateKind::CloseIntent {
                service.record_retirement(&run.run_id, &record.retirement_id, expected,
                    retirement::classify_close(identity, None, None))?;
            } else if expected == RetirementStateKind::Waiting
                && super::parse_time(&record.created_at).is_none_or(|at|
                    time::OffsetDateTime::now_utc() - at >= time::Duration::minutes(10)) {
                service.record_retirement(&run.run_id, &record.retirement_id, expected,
                    RetirementState::Retained { at: now(), reason: RetainReason::ObservationUnavailable, stopped: false })?;
            }
            return Ok(());
        }
    };
    observations.clear(&run.run_id);
    if matches!(expected, RetirementStateKind::CloseIntent | RetirementStateKind::Unknown) {
        let next = retirement::classify_close(identity, None, Some(&runtime));
        if matches!(next, RetirementState::Retired { .. }) {
            service.record_retirement(&run.run_id, &record.retirement_id, expected, next)?;
        } else if expected == RetirementStateKind::CloseIntent {
            service.record_retirement(&run.run_id, &record.retirement_id, expected, next)?;
        }
        return Ok(()); // An uncertain close is never submitted a second time.
    }
    let blockers = service.retirement_blockers(&run.run_id)?;
    if !blockers.is_empty() {
        if record.state != (RetirementState::Waiting { blockers: blockers.clone() }) {
            service.record_retirement(&run.run_id, &record.retirement_id, expected, RetirementState::Waiting { blockers })?;
        }
        return Ok(());
    }
    let exact_absence = runtime.endpoint_identity == identity.endpoint_identity
        && super::optional_available(&identity.herdr_boot_id, &runtime.boot_id)
        && !runtime.panes.iter().any(|pane| pane.pane_id == identity.pane_id || pane.terminal_id.as_deref() == Some(identity.terminal_id.as_str()));
    if exact_absence {
        service.record_retirement(&run.run_id, &record.retirement_id, expected, RetirementState::Retired { at: now(), terminal: TerminalOutcome::AlreadyAbsent })?;
        return Ok(());
    }
    let process = identity.process.as_ref().or_else(|| identity.shell.as_ref().map(|shell| &shell.process));
    let stopped = process.is_some_and(|process| matches!(exact_running(process, process_identity::kernel_boot_id().as_deref()), Ok(false)));
    if !stopped {
        service.record_retirement(&run.run_id, &record.retirement_id, expected, RetirementState::Retained { at: now(), reason: RetainReason::NativeProcessUnverifiable, stopped: false })?;
        return Ok(());
    }
    let info = match herdr.pane_process_info(&identity.session_id, &identity.endpoint_identity, &identity.pane_id).await {
        Ok(info) => info,
        Err(_) => { observations.unavailable(&run, record); return Ok(()) }
    };
    match retirement::classify_terminal(identity, &runtime, &info) {
        Err(TerminalDecision::Absent) => {
            service.record_retirement(&run.run_id, &record.retirement_id, expected, RetirementState::Retired { at: now(), terminal: TerminalOutcome::AlreadyAbsent })?;
        }
        Err(TerminalDecision::Retain(reason)) => {
            service.record_retirement(&run.run_id, &record.retirement_id, expected, RetirementState::Retained { at: now(), reason, stopped })?;
        }
        Ok(()) => {
            // The RPC enumerates only the foreground group. Main/shell exit
            // and an empty foreground cannot seal detached/asynchronous writer
            // admission or protect an unknown PTY consumer from pane.close.
            // No extension-free exhaustive drain proof is available here.
            service.record_retirement(&run.run_id, &record.retirement_id, expected,
                RetirementState::Retained { at: now(), reason: RetainReason::NativeProcessUnverifiable, stopped })?;
        }
    }
    Ok(())
}

pub(super) fn exact_running(process: &NativeProcessIdentity, boot_id: Option<&str>) -> std::io::Result<bool> {
    if process.kernel_boot_id.as_ref().is_some_and(|expected| boot_id != Some(expected.as_str())) {
        return Err(std::io::Error::other("Kernel boot identity unavailable or changed"));
    }
    let pid = i32::try_from(process.pid).map_err(|_| std::io::Error::other("Process PID is outside the supported range"))?;
    process_identity::incarnation_running(pid, process.start_ticks)
}
