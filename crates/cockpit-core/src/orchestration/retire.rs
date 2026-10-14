//! Receipt-owned physical terminal cleanup, independent of logical assignment.
use cockpit_protocol::orchestration::{NativeProcessIdentity, RetainReason, RetirementIdentity, RetirementState, RetirementStateKind, Run, TerminalOutcome, WorkerCleanupPolicy};
use super::{dispatch::Dispatcher, herdr::{OrchestrationHerdr, RuntimeView}, retirement::{self, TerminalDecision}, now, OrchestrationService};
use crate::{process_identity, InspectionError};

impl Dispatcher {
    pub(super) async fn retire(&self, run: &Run) -> Result<(), InspectionError> {
        retire_run(&self.service, self.herdr.as_ref(), run).await
    }
}

pub(super) async fn retire_run(service: &OrchestrationService, herdr: &dyn OrchestrationHerdr, queued: &Run) -> Result<(), InspectionError> {
    let Some(_lease) = service.execution_lease(&queued.run_id)? else { return Ok(()) };
    let mut run = service.run_for_review(&queued.session_id, &queued.run_id)?;
    let Some(record) = &run.retirement else { return Ok(()) };
    if queued.retirement.as_ref().is_none_or(|queued| queued.retirement_id != record.retirement_id) { return Ok(()) }
    let expected = record.state.kind();
    let reobserve = matches!(record.state, RetirementState::Retained {
        reason: RetainReason::NativeProcessUnverifiable | RetainReason::ObservationUnavailable | RetainReason::ForegroundProcess, ..
    });
    if retirement::terminal(&record.state) && expected != RetirementStateKind::Unknown && !reobserve { return Ok(()) }
    if !service.retirement_authorized(&run.run_id, record)?
        && !matches!(expected, RetirementStateKind::CloseIntent | RetirementStateKind::Unknown) {
        if expected == RetirementStateKind::Waiting {
            service.record_retirement_exact(&run.run_id, record, RetirementState::Retained {
                at: now(), reason: if retirement::record_shape(record) { RetainReason::IdentityChanged }
                    else { RetainReason::IdentityIncomplete }, stopped: false,
            })?;
        }
        return Ok(());
    }
    let Some(identity) = &record.identity else { return Ok(()) };
    let runtime = match herdr.runtime(&identity.session_id).await {
        Ok(runtime) => runtime,
        Err(_) => {
            if expected == RetirementStateKind::CloseIntent {
                service.record_retirement(&run.run_id, &record.retirement_id, expected,
                    retirement::classify_close(identity, None, None))?;
            } else if expected == RetirementStateKind::Waiting {
                service.record_retirement_exact(&run.run_id, record, RetirementState::Retained {
                    at: now(), reason: RetainReason::ObservationUnavailable, stopped: false,
                })?;
            }
            return Ok(());
        }
    };
    if matches!(expected, RetirementStateKind::CloseIntent | RetirementStateKind::Unknown) {
        let next = retirement::classify_close(identity, None, Some(&runtime));
        if matches!(next, RetirementState::Retired { .. }) || expected == RetirementStateKind::CloseIntent {
            service.record_retirement(&run.run_id, &record.retirement_id, expected, next)?;
        }
        return Ok(()); // A recovered or uncertain close is never submitted again.
    }
    let blockers = service.retirement_blockers(&run.run_id)?;
    if !blockers.is_empty() {
        if !reobserve && record.state != (RetirementState::Waiting { blockers: blockers.clone() }) {
            service.record_retirement(&run.run_id, &record.retirement_id, expected, RetirementState::Waiting { blockers })?;
        }
        return Ok(());
    }
    let next = match observe_terminal(&run, identity, &runtime, herdr).await {
        Ok(next) => next,
        Err(_) => {
            if expected == RetirementStateKind::Waiting {
                service.record_retirement_exact(&run.run_id, record, RetirementState::Retained {
                    at: now(), reason: RetainReason::ObservationUnavailable, stopped: false,
                })?;
            }
            return Ok(());
        }
    };
    if reobserve {
        // Fresh physical proof, not a migration, is required to reopen a transient
        // process/observation diagnosis. Other retained reasons are never overridden.
        if matches!(next, Some(RetirementState::Retained { .. })) {
            return Ok(());
        }
        service.retirement_waiting(&run.run_id, record, Vec::new())?;
        run = service.run_for_review(&run.session_id, &run.run_id)?;
    }
    let record = run.retirement.as_ref().expect("retirement");
    if record.state.kind() != RetirementStateKind::Waiting { return Ok(()) }
    if let Some(next) = next {
        if record.state != next {
            service.record_retirement(&run.run_id, &record.retirement_id, RetirementStateKind::Waiting, next)?;
        }
        return Ok(());
    }
    let intent = RetirementState::CloseIntent { at: now() };
    service.record_retirement_exact(&run.run_id, record, intent.clone())?;
    let submitted = service.run_for_review(&run.session_id, &run.run_id)?;
    let Some(saved) = &submitted.retirement else { return Ok(()) };
    if saved.retirement_id != record.retirement_id || saved.state != intent
        || !retirement::identity_matches(&submitted, saved) || !service.retirement_authorized(&submitted.run_id, saved)? { return Ok(()) }
    let identity = saved.identity.as_ref().expect("identity");
    // Reobserve adjacent to the effect. This branch owns a newly saved intent
    // and knows no close has been submitted, so a busy/offline preflight can wait.
    let final_check = match herdr.runtime(&identity.session_id).await {
        Ok(runtime) => observe_terminal(&submitted, identity, &runtime, herdr).await,
        Err(error) => Err(error),
    };
    let blockers = service.retirement_blockers(&submitted.run_id)?;
    match final_check {
        Err(_) | Ok(Some(RetirementState::Waiting { .. })) => {
            service.retirement_waiting(&submitted.run_id, saved, blockers)?;
            return Ok(());
        }
        Ok(Some(next)) => {
            service.record_retirement_exact(&submitted.run_id, saved, next)?;
            return Ok(());
        }
        Ok(None) if !blockers.is_empty() => {
            service.retirement_waiting(&submitted.run_id, saved, blockers)?;
            return Ok(());
        }
        Ok(None) => {}
    }
    let current = service.run_for_review(&submitted.session_id, &submitted.run_id)?;
    if current.retirement.as_ref() != Some(saved) || !retirement::identity_matches(&current, saved)
        || !service.retirement_authorized(&current.run_id, saved)? {
        service.record_retirement_exact(&submitted.run_id, saved,
            RetirementState::Retained { at: now(), reason: RetainReason::IdentityChanged, stopped: false })?;
        return Ok(());
    }
    let response = herdr.close_pane(&identity.session_id, &identity.endpoint_identity, &identity.pane_id).await;
    let fresh = herdr.runtime(&identity.session_id).await.ok();
    service.record_retirement_exact(&submitted.run_id, saved,
        retirement::classify_close(identity, Some(response), fresh.as_ref()))?;
    Ok(())
}

/// None authorizes only this pane close; it is never all-consumer exit proof.
async fn observe_terminal(run: &Run, identity: &RetirementIdentity, runtime: &RuntimeView, herdr: &dyn OrchestrationHerdr) -> Result<Option<RetirementState>, InspectionError> {
    let retained = |reason, stopped| Some(RetirementState::Retained { at: now(), reason, stopped });
    if !retirement::identity_matches(run, run.retirement.as_ref().expect("retirement"))
        || run.cleanup_policy != Some(WorkerCleanupPolicy::OwnedWhenSafe) || run.session_id != identity.session_id {
        return Ok(retained(RetainReason::IdentityChanged, false));
    }
    let exact_absence = retirement::server_fence(identity, runtime).is_ok()
        && !runtime.panes.iter().any(|pane| pane.pane_id == identity.pane_id || pane.terminal_id.as_deref() == Some(identity.terminal_id.as_str()));
    if exact_absence { return Ok(Some(RetirementState::Retired { at: now(), terminal: TerminalOutcome::AlreadyAbsent })); }
    if identity.process.is_none() || identity.shell.is_none() {
        return Ok(retained(RetainReason::NativeProcessUnverifiable, false));
    }
    let info = herdr.pane_process_info(&identity.session_id, &identity.endpoint_identity, &identity.pane_id).await?;
    match retirement::classify_terminal(identity, runtime, &info) {
        Err(TerminalDecision::Absent) => return Ok(Some(RetirementState::Retired { at: now(), terminal: TerminalOutcome::AlreadyAbsent })),
        Err(TerminalDecision::Retain(reason)) => return Ok(retained(reason, false)),
        Ok(()) => {}
    }
    let process = identity.process.as_ref().expect("process");
    match exact_running(process, process_identity::kernel_boot_id().as_deref()) {
        Ok(true) => {}
        Ok(false) => return Ok(retained(RetainReason::NativeProcessUnverifiable, true)),
        Err(_) => return Ok(retained(RetainReason::NativeProcessUnverifiable, false)),
    }
    let pane = runtime.panes.iter().find(|pane| pane.pane_id == identity.pane_id).expect("classified pane");
    if pane.agent_kind.as_ref().is_some_and(|kind| kind != &run.launch_config.harness_kind)
        || run.attachment.as_ref().and_then(|a| a.location.as_ref())
            .and_then(|l| l.native_session_id.as_ref()).zip(pane.native_session_id.as_ref())
            .is_some_and(|(expected, observed)| expected != observed) {
        return Ok(retained(RetainReason::IdentityChanged, false));
    }
    if pane.launch_pending || pane.agent_kind.as_deref() != Some(run.launch_config.harness_kind.as_str())
        || !matches!(pane.agent_status.as_deref(), Some("idle" | "done")) {
        return Ok(Some(RetirementState::Waiting { blockers: Vec::new() }));
    }
    Ok(None)
}

pub(super) fn exact_running(process: &NativeProcessIdentity, boot_id: Option<&str>) -> std::io::Result<bool> {
    if process.kernel_boot_id.as_ref().is_some_and(|expected| boot_id != Some(expected.as_str())) {
        return Err(std::io::Error::other("Kernel boot identity unavailable or changed"));
    }
    let pid = i32::try_from(process.pid).map_err(|_| std::io::Error::other("Process PID is outside the supported range"))?;
    process_identity::incarnation_running(pid, process.start_ticks)
}
