use super::super::*;

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
        predicate: impl FnOnce(&Run) -> Result<bool, InspectionError>,
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
        if !predicate(&state.runs[index])? {
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
            |run| {
                if !same_launch_incarnation(run, expected) || run.stage != RunStage::Preparing
                    || expected.stage != RunStage::Preparing
                    || run.bound_omp_process != expected.bound_omp_process
                    || run.launch_shell_identity != expected.launch_shell_identity
                    || run.bound_omp_session.is_some() || run.bound_omp_process.is_some()
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
            |current| {
                if !same_launch_request(current, reviewed)
                    || reviewed
                        .bound_omp_session
                        .as_ref()
                        .is_some_and(|bound| current.bound_omp_session.as_ref() != Some(bound))
                    || current.stage != RunStage::Preparing
                    || current.location.is_none()
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
                run.updated_at = dispatch.updated_at.clone();
                Ok(())
            },
        )?;
        Ok(applied.revision())
    }

    /// Caller supplies fresh runtime proof. Commit only that exact bound incarnation.
    pub(crate) fn record_launch_verified(&self, reviewed: &Run) -> Result<u64, InspectionError> {
        let applied = self.with_reviewed_run(
            reviewed,
            RunLookup::Required,
            |current| {
                let request_matches = match (current.dispatch.as_ref(), reviewed.dispatch.as_ref()) {
                    (Some(actual), Some(expected)) => {
                        actual.step == expected.step
                            && actual.updated_at == expected.updated_at
                            && actual.recovery == expected.recovery
                            && actual.agent_started == expected.agent_started
                    }
                    _ => false,
                };
                if !same_launch_incarnation(current, reviewed)
                    || !request_matches
                    || current
                        .bound_omp_session
                        .as_deref()
                        .is_none_or(str::is_empty)
                    || current.location.is_none()
                    || !launch_receipt_coherent(current)
                    || current.dispatch.as_ref().is_none_or(|d| {
                        !matches!(
                            d.step,
                            DispatchStep::LaunchIntent
                                | DispatchStep::LaunchPending
                                | DispatchStep::LaunchUnknown
                                | DispatchStep::NeedsReview
                                | DispatchStep::Launched
                        )
                    })
                {
                    return Ok(false);
                }
                if current.dispatch.as_ref().is_some_and(|d| {
                    d.step == DispatchStep::Launched && d.agent_started && d.error.is_none()
                }) && current.stage != RunStage::Preparing
                {
                    return Ok(false);
                }
                Ok(true)
            },
            |state, index| {
                let run = &mut state.runs[index];
                let dispatch = run.dispatch.as_mut().expect("fenced launch");
                let initial = run.stage == RunStage::Preparing;
                dispatch.step = DispatchStep::Launched;
                dispatch.agent_started = true;
                dispatch.error = None;
                dispatch.updated_at = now();
                if initial {
                    run.stage = if run.kind == RunKind::Worker {
                        RunStage::Initializing
                    } else {
                        RunStage::Active
                    };
                }
                run.updated_at = dispatch.updated_at.clone();
                let kind = if run.kind == RunKind::Worker {
                    MessageKind::PrepareBrief
                } else {
                    MessageKind::SupervisorBrief
                };
                // Generated root policy is delivered when this launch is first proven,
                // even if its pending intent predates the installed CLI capabilities.
                // A historical Launched review must not rewrite or replay the policy.
                if (initial || reviewed.stage == RunStage::Preparing)
                    && run.parent_run_id.is_none()
                    && matches!(run.kind, RunKind::Supervisor | RunKind::Adopted)
                {
                    run.prepare_brief = supervisor_guidance().into();
                }
                let text = run.prepare_brief.clone();
                let phase = format!("launch-{}-{}", run.attempt, dispatch.launch_attempt);
                // Historical launched reviews must never replay execution or initialization.
                if initial || reviewed.stage == RunStage::Preparing {
                    brief(state, &reviewed.run_id, kind, &text, &phase)?;
                }
                escalation::recovered(state, index)?;
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
            |current| {
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
                {
                    return Ok(false);
                }
                Ok(true)
            },
            |state, index| {
                let at = now();
                let run = &mut state.runs[index];
                let dispatch = run.dispatch.as_mut().expect("guarded launch review");
                dispatch.step = step;
                dispatch.error = error;
                dispatch.updated_at = at.clone();
                run.updated_at = at;
                if step == DispatchStep::Launched {
                    escalation::recovered(state, index)?;
                } else {
                    escalation::failure(state, index)?;
                }
                Ok(())
            },
        )?;
        Ok(applied.revision())
    }

    /// Automatic mature review is queued just like explicit ReconcileRun.
    /// A plain Launched snapshot never authorizes a review write.
    pub(crate) fn queue_automatic_launch_review(&self, reviewed: &Run) -> Result<Option<Run>, InspectionError> {
        let applied = self.with_reviewed_run(
            reviewed,
            RunLookup::Required,
            |current| {
                if !recovery_review_matches(current, reviewed)
                    || !dispatch::automatic_recovery_available(current)
                    || current.dispatch.as_ref().is_none_or(|d| d.step != DispatchStep::Launched || !d.agent_started)
                {
                    return Ok(false);
                }
                Ok(true)
            },
            |state, index| {
                let run = &mut state.runs[index];
                let d = run.dispatch.as_mut().expect("mature review");
                d.step = DispatchStep::LaunchIntent;
                d.updated_at = now();
                run.updated_at = d.updated_at.clone();
                let queued = run.clone();
                Ok(queued)
            },
        )?;
        Ok(applied.value())
    }

    /// Write-ahead fence: an uncertain owned close is never replayed.
    pub(crate) fn begin_launch_recovery(&self, reviewed: &Run, preserve_space: bool) -> Result<Option<Run>, InspectionError> {
        let applied = self.with_reviewed_run(
            reviewed,
            RunLookup::Required,
            |current| {
                if !recovery_review_matches(current, reviewed)
                    || !dispatch::automatic_recovery_available(current)
                    || current.dispatch.as_ref().is_none_or(|d| d.step != DispatchStep::LaunchUnknown)
                {
                    return Ok(false);
                }
                Ok(true)
            },
            |state, index| {
                let run = &mut state.runs[index];
                let d = run.dispatch.as_mut().expect("reviewed dispatch");
                d.step = DispatchStep::NeedsReview;
                d.error = Some(ErrorResponse {
                    code: if preserve_space { dispatch::RECOVERY_PRESERVE_INTENT } else { dispatch::RECOVERY_CLOSE_INTENT }.into(),
                    message: if preserve_space { "Creating an ordinary working terminal to preserve the recorded Space before cancellation" } else { "Cancelling the exact owned launch pane before one automatic retry" }.into(),
                });
                d.updated_at = now();
                run.updated_at = d.updated_at.clone();
                run.annotations.push(Annotation {
                    by: ActorRef::Dispatcher,
                    text: format!("{}; launch_attempt={}", dispatch::RECOVERY_ATTEMPT_ANNOTATION, d.launch_attempt),
                    at: now(),
                });
                if preserve_space {
                    let source = serde_json::to_string(run.location.as_ref().expect("owned source"))
                        .map_err(|e| error("orchestration_state_invalid", e.to_string()))?;
                    run.annotations.push(Annotation {
                        by: ActorRef::Dispatcher,
                        text: format!("Working terminal creation intent; source={source}"),
                        at: now(),
                    });
                }
                let receipt = run.clone();
                Ok(receipt)
            },
        )?;
        Ok(applied.value())
    }

    /// Record the real ordinary terminal ACK before authorizing the old close.
    pub(crate) fn record_preserved_working_terminal(&self, reviewed: &Run, receipt: RunLocation) -> Result<Option<Run>, InspectionError> {
        let applied = self.with_reviewed_run(
            reviewed,
            RunLookup::Required,
            |current| {
                if !recovery_review_matches(current, reviewed)
                    || current.dispatch.as_ref().is_none_or(|d| d.error.as_ref()
                        .is_none_or(|e| e.code != dispatch::RECOVERY_PRESERVE_INTENT))
                {
                    return Ok(false);
                }
                Ok(true)
            },
            |state, index| {
                let current = &state.runs[index];
                let source = current.location.as_ref().expect("owned source");
                if receipt.endpoint_identity != source.endpoint_identity
                    || receipt.session_id != source.session_id
                    || receipt.workspace_id != source.workspace_id
                    || !optional_available(&source.boot_id, &receipt.boot_id)
                    || receipt.tab_id == source.tab_id || receipt.pane_id == source.pane_id
                    || receipt.terminal_id.is_none() || receipt.terminal_id == source.terminal_id
                {
                    return Err(error("owned_launch_tab_unsafe", "Working terminal receipt did not preserve the exact source Space with a fresh terminal"));
                }
                let encoded = serde_json::to_string(&receipt)
                    .map_err(|e| error("orchestration_state_invalid", e.to_string()))?;
                let run = &mut state.runs[index];
                run.annotations.push(Annotation {
                    by: ActorRef::Dispatcher,
                    text: format!("Preserved ordinary working terminal receipt: {encoded}"),
                    at: now(),
                });
                let d = run.dispatch.as_mut().expect("preservation intent");
                d.error = Some(ErrorResponse { code: dispatch::RECOVERY_CLOSE_INTENT.into(),
                    message: "Source Space preserved by a recorded ordinary working terminal; cancelling the old owned launch pane".into() });
                d.updated_at = now();
                run.updated_at = d.updated_at.clone();
                let receipt = run.clone();
                Ok(receipt)
            },
        )?;
        Ok(applied.value())
    }

    /// The caller proves the old owned tab and terminal absent after close.
    pub(crate) fn finish_launch_recovery(&self, reviewed: &Run) -> Result<bool, InspectionError> {
        let applied = self.with_reviewed_run(
            reviewed,
            RunLookup::Required,
            |current| {
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
                let attempt = retry_launch_state(state, index)?;
                state.runs[index].annotations.push(Annotation {
                    by: ActorRef::Dispatcher,
                    text: format!("Owned launch layout absent and original shell/native kernel exit proved; automatic launch attempt {attempt}"),
                    at: now(),
                });
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
        && current.bound_omp_process == reviewed.bound_omp_process
        && current.launch_shell_identity == reviewed.launch_shell_identity
        && matches!((current.dispatch.as_ref(), reviewed.dispatch.as_ref()),
            (Some(actual), Some(expected)) if actual.step == expected.step
                && actual.updated_at == expected.updated_at
                && actual.agent_started == expected.agent_started
                && actual.recovery == expected.recovery)
}

fn recovery_review_matches(current: &Run, reviewed: &Run) -> bool {
    same_launch_incarnation(current, reviewed)
        && current.stage == reviewed.stage
        && current.updated_at == reviewed.updated_at
        && current.bound_omp_process == reviewed.bound_omp_process
        && current.launch_shell_identity == reviewed.launch_shell_identity
        && current.dispatch.as_ref().zip(reviewed.dispatch.as_ref()).is_some_and(|(a, b)| {
            a.step == b.step && a.updated_at == b.updated_at && a.agent_started == b.agent_started
                && a.error == b.error
        })
}

/// Both explicit and automatic retries retain task, grants, plans and inbox.
pub(super) fn retry_launch_state(state: &mut OrchestrationState, index: usize) -> Result<u32, InspectionError> {
    let run = &mut state.runs[index];
    let attempt = run.dispatch.as_ref().ok_or_else(|| error("invalid_stage", "No launch to retry"))?
        .launch_attempt.checked_add(1)
        .ok_or_else(|| error("orchestration_state_full", "Launch attempt exhausted"))?;
    run.location = None;
    run.bound_omp_session = None;
    run.bound_omp_process = None;
    run.launch_shell_identity = None;
    if run.kind == RunKind::Supervisor {
        run.prepare_brief = supervisor_guidance().into();
    }
    for message in state.messages.iter_mut().filter(|message| {
        message.to_run_id == run.run_id && matches!(message.kind,
            MessageKind::WorkBrief | MessageKind::PrepareBrief | MessageKind::SupervisorBrief)
    }) {
        message.stale = true;
    }
    run.stage = RunStage::Preparing;
    let mut next = dispatch(DispatchStep::SetupPending);
    next.launch_attempt = attempt;
    run.dispatch = Some(next);
    run.updated_at = now();
    Ok(attempt)
}

pub(super) fn same_launch_incarnation(current: &Run, reviewed: &Run) -> bool {
    current.bound_omp_session == reviewed.bound_omp_session
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
        && actual.launch_attempt == expected.launch_attempt
        && actual.launch_tag == expected.launch_tag
        && actual.endpoint_identity == expected.endpoint_identity
        && same_launch_location(current.location.as_ref(), reviewed.location.as_ref())
}
