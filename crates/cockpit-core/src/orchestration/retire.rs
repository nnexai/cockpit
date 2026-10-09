//! Lease-fenced effects for accepted workers. CloseIntent is a write-ahead,
//! non-replayable effect boundary, not permission to retry a pane close.
//! Fingerprints prove original launch-shell provenance, not shell idleness.
//! After OS-proven worker exit, the approved owned-pane policy permits
//! closing that original shell even during indistinguishable builtin work.
use std::{collections::BTreeMap, time::{Duration, Instant}};
use parking_lot::Mutex;

use cockpit_protocol::orchestration::{
    NativeProcessIdentity, NativeStopEvidence, RetainReason, RetirementIdentity, RetirementPhase,
    RetirementState, RetirementStateKind, Run, RunRetirement,
};

use super::{
    dispatch::Dispatcher,
    herdr::{OrchestrationHerdr, PaneProcessInfo, RuntimeView},
    retirement::{self, OfferDecision, TerminalDecision,
        BUSY_TIMEOUT_SECS, NATIVE_STOP_TIMEOUT_SECS, OBSERVATION_RETRY_SECS,
        OBSERVATION_TIMEOUT_SECS, OFFER_RESPONSE_TIMEOUT_SECS},
    now, OrchestrationService,
};
use crate::{process_identity, InspectionError};

/// One bounded retry entry per active run/incarnation. Only failed read-only
/// observations use this gate; every effect boundary bypasses cached evidence.
#[derive(Default)]
pub(super) struct ObservationBackoff(Mutex<BTreeMap<String, (String, Instant)>>);
impl ObservationBackoff {
    fn ready(&self, run: &Run, record: &RunRetirement) -> bool {
        let entries = self.0.lock();
        !entries.get(&run.run_id).is_some_and(|(id, after)| {
            id == &record.retirement_id && Instant::now() < *after
        })
    }
    fn unavailable(&self, run: &Run, record: &RunRetirement) {
        self.0.lock().insert(run.run_id.clone(), (
            record.retirement_id.clone(),
            Instant::now() + Duration::from_secs(OBSERVATION_RETRY_SECS),
        ));
    }
    fn clear(&self, run_id: &str) { self.0.lock().remove(run_id); }
    fn retain_queue(&self, queue: &[Run]) {
        self.0.lock().retain(|run_id, (id, _)| queue.iter().any(|run| {
            &run.run_id == run_id && run.retirement.as_ref().is_some_and(|r| &r.retirement_id == id)
        }));
    }
}

impl Dispatcher {
    /// Called by the existing owner scheduler, using its existing tick and
    /// active key `retire:<run_id>`. No additional timer or owner is installed.
    pub(super) async fn retire(&self, run: &Run) -> Result<(), InspectionError> {
        retire_run(&self.service, self.herdr.as_ref(), &self.retirement_observations, run).await
    }
    pub(super) fn retain_retirement_observations(&self, queue: &[Run]) {
        self.retirement_observations.retain_queue(queue);
    }
}

pub(super) async fn retire_run(
    service: &OrchestrationService,
    herdr: &dyn OrchestrationHerdr,
    observations: &ObservationBackoff,
    queued: &Run,
) -> Result<(), InspectionError> {
    let Some(_lease) = service.execution_lease(&queued.run_id)? else { return Ok(()); };
    let Some(run) = service.retirement_queue()?.into_iter().find(|run| {
        run.run_id == queued.run_id && run.retirement.as_ref().map(|r| &r.retirement_id)
            == queued.retirement.as_ref().map(|r| &r.retirement_id)
    }) else { observations.clear(&queued.run_id); return Ok(()); };
    let record = run.retirement.as_ref().expect("retirement queue contains records");
    let Some(identity) = record.identity.as_ref() else { return Ok(()); };
    let effect = Effect { service, herdr, observations, run: &run, record, identity };
    match &record.state {
        RetirementState::Waiting { blockers: previous } => {
            let blockers = service.retirement_blockers(&run.run_id)?;
            if !blockers.is_empty() {
                if &blockers != previous { effect.save(RetirementState::Waiting { blockers })?; }
                return Ok(());
            }
            if !previous.is_empty() {
                effect.save(RetirementState::Waiting { blockers })?;
                // The save is authoritative; a later scheduler pass takes the
                // fresh empty-blocker record before attempting the offer.
                return Ok(());
            }
            if !observations.ready(&run, record) { return Ok(()); }
            let boot_id = process_identity::kernel_boot_id();
            if matches!((identity.process.kernel_boot_id.as_deref(), boot_id.as_deref()),
                (Some(expected), Some(actual)) if expected != actual) {
                return effect.retain(RetainReason::IdentityChanged, false);
            }
            let running = match exact_running(&identity.process, boot_id.as_deref()) {
                Ok(running) => running,
                Err(_) => return effect.retain(RetainReason::NativeProcessUnverifiable, false),
            };
            let runtime = match herdr.runtime(&identity.session_id).await {
                Ok(runtime) => runtime,
                Err(_) => return effect.observation_unavailable(&record.created_at, false),
            };
            let info = if runtime.endpoint_identity == identity.endpoint_identity
                && runtime.panes.iter().any(|p| p.pane_id == identity.pane_id) {
                match herdr.pane_process_info(&identity.session_id, &identity.endpoint_identity, &identity.pane_id).await {
                    Ok(info) => Some(info),
                    Err(_) if !running => None,
                    Err(_) => return effect.observation_unavailable(&record.created_at, false),
                }
            } else { None };
            // The pane RPC may have taken time; refresh local incarnation
            // proof before publishing an offer from that observation.
            let running = match exact_running(&identity.process, boot_id.as_deref()) {
                Ok(running) => running,
                Err(_) => return effect.retain(RetainReason::NativeProcessUnverifiable, false),
            };
            match retirement::classify_offer(identity, &runtime, info.as_ref(), running,
                boot_id.as_deref()) {
                OfferDecision::Offer => effect.save(RetirementState::NativeStopOffered { offered_at: now() }),
                OfferDecision::AlreadyExited => effect.save(RetirementState::NativeStopped {
                    at: now(), evidence: NativeStopEvidence::AlreadyExited,
                }),
                OfferDecision::Retain(reason) => effect.retain(reason, false),
            }
        }
        RetirementState::NativeStopOffered { offered_at } => {
            if expired(offered_at, BUSY_TIMEOUT_SECS) {
                effect.retain(RetainReason::WorkerBusyTimeout, false)
            } else if expired(offered_at, OFFER_RESPONSE_TIMEOUT_SECS) {
                effect.retain(RetainReason::WorkerUnresponsive, false)
            } else { Ok(()) }
        }
        RetirementState::NativeStopDeferred { offered_at, .. } => {
            if expired(offered_at, BUSY_TIMEOUT_SECS) {
                effect.retain(RetainReason::WorkerBusyTimeout, false)
            } else { Ok(()) }
        }
        RetirementState::NativeStopRequested { at } => {
            let boot_id = process_identity::kernel_boot_id();
            match exact_running(&identity.process, boot_id.as_deref()) {
                Ok(false) => {
                    // Helpful evidence only. No Herdr result substitutes for
                    // process-incarnation absence, and unavailability must not
                    // hide proven exit.
                    let _ = herdr.pane_process_info(&identity.session_id,
                        &identity.endpoint_identity, &identity.pane_id).await;
                    effect.save(RetirementState::NativeStopped {
                        at: now(), evidence: NativeStopEvidence::ExitedAfterShutdownRequest,
                    })
                }
                proof if expired(at, NATIVE_STOP_TIMEOUT_SECS) => effect.save(RetirementState::Unknown {
                    at: now(), phase: RetirementPhase::NativeStop,
                    detail: if proof.is_err() {
                        "The native process incarnation could not be observed before the deadline."
                    } else { "The native process incarnation is still running after the shutdown deadline." }.into(),
                }),
                _ => Ok(()),
            }
        }
        RetirementState::NativeStopped { at, .. } => {
            if !observations.ready(&run, record) { return Ok(()); }
            match effect.terminal_preflight().await {
                Ok(Ok(())) => {},
                Ok(Err(TerminalDecision::Absent)) => return effect.save(RetirementState::Retired {
                    at: now(), terminal: cockpit_protocol::orchestration::TerminalOutcome::AlreadyAbsent,
                }),
                Ok(Err(TerminalDecision::Retain(reason))) => return effect.retain(reason, true),
                Err(_) => return effect.observation_unavailable(at, true),
            }
            effect.save(RetirementState::CloseIntent { at: now() })?;
            // From here onwards all saves expect CloseIntent, not the stale
            // NativeStopped record. Both preflights are uncached reads.
            match effect.terminal_preflight().await {
                Ok(Ok(())) => {},
                Ok(Err(TerminalDecision::Absent)) => return effect.save_close(RetirementState::Retired {
                    at: now(), terminal: cockpit_protocol::orchestration::TerminalOutcome::AlreadyAbsent,
                }),
                Ok(Err(TerminalDecision::Retain(reason))) => return effect.save_close(retained(reason, true)),
                Err(_) => return effect.save_close(retained(RetainReason::ObservationUnavailable, true)),
            }
            // Re-read durable authority under the execution lease immediately
            // before the sole external mutation. ID-only Herdr close is not an
            // atomic compare-and-close; the approved residual race remains.
            if !service.retirement_effect_authorized(&run.run_id, &record.retirement_id)? {
                return effect.save_close(retained(RetainReason::IdentityChanged, true));
            }
            let response = herdr.close_pane(&identity.session_id,
                &identity.endpoint_identity, &identity.pane_id).await;
            effect.verify_close(Some(response)).await
        }
        RetirementState::CloseIntent { .. } => effect.verify_close(None).await,
        RetirementState::Retired { .. } | RetirementState::Retained { .. }
        | RetirementState::Unknown { .. } => { observations.clear(&run.run_id); Ok(()) }
    }
}

struct Effect<'a> {
    service: &'a OrchestrationService,
    herdr: &'a dyn OrchestrationHerdr,
    observations: &'a ObservationBackoff,
    run: &'a Run,
    record: &'a RunRetirement,
    identity: &'a RetirementIdentity,
}
impl Effect<'_> {
    fn save(&self, next: RetirementState) -> Result<(), InspectionError> {
        self.save_expected(self.record.state.kind(), next)
    }
    fn save_close(&self, next: RetirementState) -> Result<(), InspectionError> {
        self.save_expected(RetirementStateKind::CloseIntent, next)
    }
    fn save_expected(&self, expected: RetirementStateKind, next: RetirementState) -> Result<(), InspectionError> {
        self.service.record_retirement(&self.run.run_id, &self.record.retirement_id, expected, next)?;
        self.observations.clear(&self.run.run_id);
        Ok(())
    }
    fn retain(&self, reason: RetainReason, native_stopped: bool) -> Result<(), InspectionError> {
        self.save(retained(reason, native_stopped))
    }
    fn observation_unavailable(&self, since: &str, native_stopped: bool) -> Result<(), InspectionError> {
        if expired(since, OBSERVATION_TIMEOUT_SECS) {
            self.retain(RetainReason::ObservationUnavailable, native_stopped)
        } else {
            self.observations.unavailable(self.run, self.record);
            Ok(())
        }
    }
    async fn terminal_preflight(&self) -> Result<Result<(), TerminalDecision>, InspectionError> {
        let runtime = self.herdr.runtime(&self.identity.session_id).await?;
        let info = if runtime.endpoint_identity == self.identity.endpoint_identity
            && runtime.panes.iter().any(|p| p.pane_id == self.identity.pane_id) {
            self.herdr.pane_process_info(&self.identity.session_id,
                &self.identity.endpoint_identity, &self.identity.pane_id).await?
        } else {
            PaneProcessInfo { pane_id: self.identity.pane_id.clone(), shell_pid: None,
                foreground_pgid: None, processes: vec![], shell_identity: None }
        };
        Ok(retirement::classify_terminal(self.identity, &runtime, &info))
    }
    async fn verify_close(&self, response: Option<Result<(), InspectionError>>) -> Result<(), InspectionError> {
        // Exactly one read-only reclassification, including recovered intent.
        // A failure becomes terminal uncertainty, not a retryable close job.
        let fresh: Option<RuntimeView> = self.herdr.runtime(&self.identity.session_id).await.ok();
        self.save_close(retirement::classify_close(self.identity, response, fresh.as_ref()))
    }
}

fn retained(reason: RetainReason, native_stopped: bool) -> RetirementState {
    RetirementState::Retained { at: now(), reason, native_stopped }
}
pub(super) fn exact_running(process: &NativeProcessIdentity, boot_id: Option<&str>) -> std::io::Result<bool> {
    if process.kernel_boot_id.as_ref().is_some_and(|expected| {
        boot_id != Some(expected.as_str())
    }) {
        return Err(std::io::Error::other("Kernel boot identity unavailable or changed"));
    }
    let pid = i32::try_from(process.pid)
        .map_err(|_| std::io::Error::other("Native PID is outside the supported range"))?;
    process_identity::incarnation_running(pid, process.start_ticks)
}
fn expired(since: &str, seconds: i64) -> bool {
    // Invalid durable time is not an unlimited permission to keep waiting.
    time::OffsetDateTime::parse(since, &time::format_description::well_known::Rfc3339)
        .map_or(true, |at| (time::OffsetDateTime::now_utc() - at).whole_seconds() >= seconds)
}
