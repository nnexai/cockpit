use super::super::*;
use cockpit_protocol::orchestration::{
    AttachmentState, NativeProcessIdentity, NativeShellIdentity, ResumePhase,
};

const REGISTRATION_UNAVAILABLE: &str =
    "The physical launch is preserved, but its issued capability has not registered the current conversation";

fn physical_location(run: &Run) -> Option<&RunLocation> {
    run.attachment.as_ref().and_then(|attachment| attachment.location.as_ref())
}

fn physical_session(run: &Run) -> Option<&str> {
    physical_location(run).and_then(|location| location.native_session_id.as_deref())
}

fn physical_process(run: &Run) -> Option<&NativeProcessIdentity> {
    run.attachment.as_ref().and_then(|attachment| attachment.process.as_ref())
}

fn physical_shell(run: &Run) -> Option<&NativeShellIdentity> {
    run.attachment.as_ref().and_then(|attachment| attachment.shell.as_ref())
}

fn same_attachment_epoch(current: &Run, reviewed: &Run) -> bool {
    match (current.attachment.as_ref(), reviewed.attachment.as_ref()) {
        (Some(actual), Some(expected)) => actual.attachment_id == expected.attachment_id
            && actual.epoch == expected.epoch
            && actual.launch_attempt == expected.launch_attempt,
        (None, None) => true,
        _ => false,
    }
}

enum RunLookup {
    Required,
    Optional,
}

enum Reviewed<T> {
    Unchanged(u64),
    Saved(u64, T),
}

impl<T> Reviewed<T> {
    fn revision(self) -> u64 {
        match self {
            Self::Unchanged(revision) | Self::Saved(revision, _) => revision,
        }
    }

    fn value(self) -> Option<T> {
        match self {
            Self::Unchanged(_) => None,
            Self::Saved(_, value) => Some(value),
        }
    }
}

impl OrchestrationService {
    /// Compare and apply under one store lock. A missing optional run or failed
    /// predicate returns the current revision without saving or publishing it.
    fn with_reviewed_run<T>(
        &self,
        reviewed: &Run,
        lookup: RunLookup,
        predicate: impl FnOnce(&OrchestrationState, &Run) -> Result<bool, InspectionError>,
        mutate: impl FnOnce(&mut OrchestrationState, usize) -> Result<T, InspectionError>,
    ) -> Result<Reviewed<T>, InspectionError> {
        let locked = self.store.lock()?;
        let mut state = locked.read()?;
        let index = match lookup {
            RunLookup::Required => run_index(&state, &reviewed.run_id)?,
            RunLookup::Optional => match state.runs.iter().position(|run| run.run_id == reviewed.run_id) {
                Some(index) => index,
                None => return Ok(Reviewed::Unchanged(state.revision)),
            },
        };
        if !predicate(&state, &state.runs[index])? {
            return Ok(Reviewed::Unchanged(state.revision));
        }
        let value = mutate(&mut state, index)?;
        let revision = locked.save(&mut state)?;
        self.revision.send_replace(revision);
        Ok(Reviewed::Saved(revision, value))
    }

    /// Only a caller that knows no launch RPC was issued may record this wait;
    /// uncertain launch intents continue through existing no-replay reconciliation.
    pub(crate) fn dispatch_unstarted_step(
        &self, expected: &Run, step: DispatchStep,
    ) -> Result<Option<Run>, InspectionError> {
        if !matches!(step, DispatchStep::SetupPending | DispatchStep::LaunchIntent) {
            return Err(error("invalid_stage", "Only a known-unsent launch may wait or resume"));
        }
        let applied = self.with_reviewed_run(
            expected,
            RunLookup::Optional,
            |_, run| {
                if !same_launch_incarnation(run, expected) || run.stage != RunStage::Starting
                    || expected.stage != RunStage::Starting
                    || physical_process(run) != physical_process(expected)
                    || physical_shell(run) != physical_shell(expected)
                    || physical_session(run).is_some() || physical_process(run).is_some()
                    || (step == DispatchStep::LaunchIntent && !launch_receipt_coherent(run))
                    || run.dispatch.as_ref().zip(expected.dispatch.as_ref()).is_none_or(|(actual, reviewed)| {
                        actual.step != reviewed.step || actual.agent_started || reviewed.agent_started
                            || actual.updated_at != reviewed.updated_at
                            || !matches!(actual.step, DispatchStep::SetupPending | DispatchStep::LaunchIntent)
                    }) {
                    return Ok(false);
                }
                Ok(true)
            },
            |state, index| {
                let run = &mut state.runs[index];
                let dispatch = run.dispatch.as_mut().expect("matched dispatch");
                dispatch.step = step;
                dispatch.error = None;
                dispatch.updated_at = now();
                run.updated_at = now();
                let result = run.clone();
                Ok(result)
            },
        )?;
        Ok(applied.value())
    }

    /// An accepted start request is not proof that OMP has started.
    pub(crate) fn record_launch_pending(&self, reviewed: &Run) -> Result<u64, InspectionError> {
        let applied = self.with_reviewed_run(
            reviewed,
            RunLookup::Required,
            |_, current| {
                if !same_launch_request(current, reviewed)
                    || physical_session(reviewed)
                        .is_some_and(|bound| physical_session(current) != Some(bound))
                    || current.stage != RunStage::Starting
                    || physical_location(current).is_none()
                    || current.attachment.as_ref().is_none_or(|attachment| {
                        matches!(attachment.state, AttachmentState::Quiescing | AttachmentState::Revoked)
                    })
                    || current
                        .dispatch
                        .as_ref()
                        .is_none_or(|d| d.step != DispatchStep::LaunchIntent)
                {
                    return Ok(false);
                }
                Ok(true)
            },
            |state, index| {
                let run = &mut state.runs[index];
                let dispatch = run.dispatch.as_mut().expect("fenced intent");
                dispatch.step = DispatchStep::LaunchPending;
                dispatch.agent_started = false;
                dispatch.error = None;
                dispatch.updated_at = now();
                let attachment = run.attachment.as_mut().expect("fenced pending attachment");
                if attachment.state != AttachmentState::Connected {
                    attachment.state = AttachmentState::Connecting;
                }
                attachment.launch_attempt = dispatch.launch_attempt;
                attachment.updated_at = dispatch.updated_at.clone();
                run.updated_at = dispatch.updated_at.clone();
                Ok(())
            },
        )?;
        Ok(applied.revision())
    }

    /// Physical readiness cannot grant semantic authority or initialize a task.
    pub(crate) fn record_launch_verified(&self, reviewed: &Run) -> Result<u64, InspectionError> {
        let applied = self.with_reviewed_run(
            reviewed, RunLookup::Required,
            |_, current| {
                Ok(same_launch_incarnation(current, reviewed)
                    && current.stage != RunStage::Closed && physical_location(current).is_some()
                    && current.attachment.as_ref().is_some_and(|attachment|
                        !matches!(attachment.state, AttachmentState::Quiescing | AttachmentState::Revoked))
                    && launch_receipt_coherent(current)
                    && current.dispatch.as_ref().zip(reviewed.dispatch.as_ref()).is_some_and(|(actual, expected)|
                        actual.step == expected.step && actual.updated_at == expected.updated_at
                            && actual.agent_started == expected.agent_started
                            && !(actual.step == DispatchStep::Launched && actual.agent_started)))
            },
            |state, index| {
                let connected = has_current_binding(state, &state.runs[index]);
                let run = &mut state.runs[index];
                let dispatch = run.dispatch.as_mut().expect("fenced launch");
                dispatch.step = DispatchStep::Launched;
                dispatch.agent_started = true;
                dispatch.error = if connected { None } else {
                    Some(ErrorResponse { code: "run_unregistered".into(), message: REGISTRATION_UNAVAILABLE.into() })
                };
                dispatch.updated_at = now();
                let attachment = run.attachment.as_mut().expect("fenced attachment");
                attachment.state = if connected { AttachmentState::Connected } else { AttachmentState::Disconnected };
                attachment.updated_at = dispatch.updated_at.clone();
                run.updated_at = dispatch.updated_at.clone();
                if connected { escalation::recovered(state, index)?; }
                else { escalation::failure(state, index)?; }
                Ok(())
            },
        )?;
        Ok(applied.revision())
    }

    /// Record a read-only review only for the launch/request that was observed.
    /// Lifecycle and inbox progress may legitimately advance during the runtime read.
    pub(crate) fn record_launch_review(
        &self,
        reviewed: &Run,
        step: DispatchStep,
        error: Option<ErrorResponse>,
    ) -> Result<u64, InspectionError> {
        if !matches!(
            step,
            DispatchStep::Launched | DispatchStep::LaunchUnknown | DispatchStep::NeedsReview
        ) {
            return Err(self::error(
                "invalid_stage",
                "Launch review requires a terminal review outcome",
            ));
        }
        let applied = self.with_reviewed_run(
            reviewed,
            RunLookup::Optional,
            |state, current| {
                let (Some(expected), Some(actual)) =
                    (reviewed.dispatch.as_ref(), current.dispatch.as_ref())
                else {
                    return Ok(false);
                };
                if !same_launch_incarnation(current, reviewed)
                    || !matches!(
                        expected.step,
                        DispatchStep::LaunchIntent
                            | DispatchStep::LaunchPending
                            | DispatchStep::LaunchUnknown
                            | DispatchStep::NeedsReview
                    )
                    || actual.agent_started != expected.agent_started
                    || actual.step != expected.step
                    || actual.updated_at != expected.updated_at
                    || actual.recovery != expected.recovery
                    || (step == DispatchStep::Launched && !actual.agent_started)
                    || (step == DispatchStep::Launched && current.attachment.as_ref().is_none_or(|attachment| {
                        matches!(attachment.state, AttachmentState::Quiescing | AttachmentState::Revoked)
                    }))
                {
                    return Ok(false);
                }
                let connected = has_current_binding(state, current);
                let desired_error = if step == DispatchStep::Launched && !connected {
                    Some(("run_unregistered", REGISTRATION_UNAVAILABLE))
                } else {
                    error.as_ref().map(|error| (error.code.as_str(), error.message.as_str()))
                };
                let same_error = actual.error.as_ref()
                    .map(|error| (error.code.as_str(), error.message.as_str())) == desired_error;
                let same_connection = step != DispatchStep::Launched
                    || current.attachment.as_ref().is_some_and(|attachment| {
                        attachment.state == if connected { AttachmentState::Connected } else { AttachmentState::Disconnected }
                    });
                Ok(actual.step != step || !same_error || !same_connection)
            },
            |state, index| {
                let connected = has_current_binding(state, &state.runs[index]);
                let at = now();
                let run = &mut state.runs[index];
                let dispatch = run.dispatch.as_mut().expect("guarded launch review");
                dispatch.step = step;
                dispatch.error = if step == DispatchStep::Launched && !connected {
                    Some(ErrorResponse {
                        code: "run_unregistered".into(),
                        message: REGISTRATION_UNAVAILABLE.into(),
                    })
                } else {
                    error.clone()
                };
                dispatch.updated_at = at.clone();
                run.updated_at = at;
                if step == DispatchStep::Launched {
                    run.attachment.as_mut().expect("guarded launched attachment").state =
                        if connected { AttachmentState::Connected } else { AttachmentState::Disconnected };
                }
                if step == DispatchStep::Launched && connected {
                    escalation::recovered(state, index)?;
                } else {
                    escalation::failure(state, index)?;
                }
                Ok(())
            },
        )?;
        Ok(applied.revision())
    }


    /// Record physical close evidence without granting attachment replacement.
    /// Only the owner Resume path can revoke the old epoch after proving every writer absent.
    pub(crate) fn finish_launch_recovery(&self, reviewed: &Run) -> Result<bool, InspectionError> {
        let applied = self.with_reviewed_run(
            reviewed,
            RunLookup::Required,
            |_, current| {
                if !recovery_review_matches(current, reviewed)
                    || !dispatch::recovery_close_pending(reviewed)
                    || reviewed.dispatch.as_ref().is_none_or(|d| d.error.as_ref().is_none_or(|e| e.code != dispatch::RECOVERY_CLOSE_INTENT))
                {
                    return Ok(false);
                }
                if !dispatch::cancelled_launch_processes_stopped(reviewed)? {
                    return Ok(false);
                }
                Ok(true)
            },
            |state, index| {
                let run = &mut state.runs[index];
                let text = format!(
                    "Owned launch layout absent; owner Resume must prove every protected writer exited before replacement; launch_attempt={}",
                    run.dispatch.as_ref().expect("reviewed recovery").launch_attempt,
                );
                if !run.annotations.iter().any(|annotation| {
                    matches!(annotation.by, ActorRef::Dispatcher) && annotation.text == text
                }) {
                    run.annotations.push(Annotation { by: ActorRef::Dispatcher, text, at: now() });
                    run.updated_at = now();
                }
                Ok(())
            },
        )?;
        Ok(applied.value().is_some())
    }
}

pub(in crate::orchestration) fn retry_review_matches(current: &Run, reviewed: &Run) -> bool {
    same_launch_incarnation(current, reviewed)
        && current.stage == reviewed.stage
        && current.updated_at == reviewed.updated_at
        && physical_process(current) == physical_process(reviewed)
        && physical_shell(current) == physical_shell(reviewed)
        && matches!((current.dispatch.as_ref(), reviewed.dispatch.as_ref()),
            (Some(actual), Some(expected)) if actual.step == expected.step
                && actual.updated_at == expected.updated_at
                && actual.agent_started == expected.agent_started
                && actual.recovery == expected.recovery)
}

fn recovery_review_matches(current: &Run, reviewed: &Run) -> bool {
    retry_review_matches(current, reviewed)
        && current.dispatch.as_ref().zip(reviewed.dispatch.as_ref())
            .is_some_and(|(actual, expected)| actual.error == expected.error)
}

/// Core Resume calls this only after atomically reserving the replacement epoch.
/// The attachment and its freshly reserved checkout ownership remain untouched.
pub(super) fn reset_launch_metadata_after_resume(
    state: &mut OrchestrationState, index: usize,
) -> Result<u32, InspectionError> {
    let run = &state.runs[index];
    let attachment = run.attachment.as_ref()
        .ok_or_else(|| error("attachment_conflict", "Resume replacement attachment is missing"))?;
    let resume = run.resume.as_ref()
        .ok_or_else(|| error("resume_required", "Launch reset requires the recorded owner Resume"))?;
    if attachment.state != AttachmentState::Reserved || attachment.location.is_some()
        || attachment.process.is_some() || attachment.shell.is_some()
        || resume.phase != ResumePhase::Launching
        || resume.replacement_attachment_epoch != Some(attachment.epoch)
        || resume.previous_attachment_epoch.is_none_or(|previous| previous >= attachment.epoch)
        || state.credentials.iter().any(|credential| credential.run_id == run.run_id
            && credential.state == super::super::credentials::CredentialState::Active
            && (credential.attachment_id != attachment.attachment_id || credential.attachment_epoch != attachment.epoch))
    {
        return Err(error("writer_exit_unproven", "Launch reset requires revoked old credentials and a clean reserved replacement epoch"));
    }
    let current_holder = |holder: &super::super::claims::AttachmentReference| {
        holder.run_id == run.run_id && holder.attachment_id == attachment.attachment_id
            && holder.attachment_epoch == attachment.epoch
    };
    if state.checkout_claims.iter().any(|claim| {
        claim.holder.as_ref().is_some_and(|holder| holder.run_id == run.run_id && !current_holder(holder))
    }) || (run.kind == RunKind::Worker && run.checkout_claim_id.as_ref().is_none_or(|claim_id| {
        state.checkout_claims.iter().find(|claim| &claim.claim_id == claim_id).is_none_or(|claim| {
            claim.state != super::super::claims::ClaimState::Reserved
                || claim.transition_operation_id.as_ref() != Some(&resume.operation_id)
                || claim.holder.as_ref().is_none_or(|holder| !current_holder(holder))
        })
    })) {
        return Err(error("writer_exit_unproven", "Launch reset requires the exact replacement writer reservation"));
    }
    let run = &mut state.runs[index];
    let previous = run.dispatch.as_ref().ok_or_else(|| error("invalid_stage", "No launch to resume"))?;
    let attempt = previous.launch_attempt.checked_add(1)
        .ok_or_else(|| error("orchestration_state_full", "Launch attempt exhausted"))?;
    if run.kind == RunKind::Supervisor {
        run.brief = supervisor_guidance().into();
    }
    for message in state.messages.iter_mut().filter(|message| {
        message.to_run_id == run.run_id && matches!(message.kind,
            MessageKind::WorkBrief | MessageKind::SupervisorBrief)
    }) {
        message.stale = true;
    }
    run.stage = RunStage::Starting;
    let mut next = dispatch(DispatchStep::SetupPending);
    next.launch_attempt = attempt;
    run.dispatch = Some(next);
    run.updated_at = now();
    if let Some(attachment) = run.attachment.as_mut() {
        attachment.launch_attempt = attempt;
        attachment.updated_at = run.updated_at.clone();
    }
    Ok(attempt)
}


pub(super) fn same_launch_incarnation(current: &Run, reviewed: &Run) -> bool {
    physical_session(current) == physical_session(reviewed)
        && physical_process(current) == physical_process(reviewed)
        && physical_shell(current) == physical_shell(reviewed)
        && same_launch_request(current, reviewed)
}

fn same_launch_request(current: &Run, reviewed: &Run) -> bool {
    let (Some(actual), Some(expected)) = (current.dispatch.as_ref(), reviewed.dispatch.as_ref())
    else {
        return false;
    };
    current.stage != RunStage::Closed
        && current.run_id == reviewed.run_id
        && current.session_id == reviewed.session_id
        && current.root_id == reviewed.root_id
        && current.task_id == reviewed.task_id
        && current.attempt == reviewed.attempt
        && current.assignment_generation == reviewed.assignment_generation
        && same_attachment_epoch(current, reviewed)
        && actual.launch_attempt == expected.launch_attempt
        && actual.launch_tag == expected.launch_tag
        && actual.endpoint_identity == expected.endpoint_identity
        && same_launch_location(physical_location(current), physical_location(reviewed))
}
