use cockpit_protocol::orchestration::{
    AgentKind, CloseReason, DispatchStep, Run, RunKind, RunLocation, RunStage,
};

use super::{AgentCaller, NativeAgentKind, optional_available, optional_fence, retirement};
use crate::InspectionError;

/// The declared main session, including the parent session of a subagent.
pub fn declared_main_session(caller: &AgentCaller) -> Option<&str> {
    if caller.agent_kind == Some(AgentKind::Subagent) {
        caller.main_omp_session_id.as_deref()
    } else {
        caller.omp_session_id.as_deref()
    }
}

/// Live location admission; unlike retirement's pinned rule, a terminal may move panes.
pub fn location_matches(
    location: &RunLocation,
    bound_session: Option<&str>,
    caller: &AgentCaller,
) -> bool {
    if location.endpoint_identity != caller.endpoint_identity
        || location.session_id != caller.session_id
        || !optional_available(&location.boot_id, &caller.boot_id)
        || !optional_fence(&location.terminal_id, &caller.terminal_id)
        || !optional_available(&location.native_session_id, &caller.native_session_id)
    {
        return false;
    }
    if location.pane_id == caller.pane_id {
        return true;
    }
    // Herdr may change pane IDs when moving the same terminal across Spaces.
    // Preserve a run only with stable terminal and bound native-session evidence.
    location
        .terminal_id
        .as_ref()
        .is_some_and(|id| caller.terminal_id.as_ref() == Some(id))
        && bound_session.is_some_and(|session| {
            caller
                .native_session_id
                .as_deref()
                .is_none_or(|native| native == session)
                && declared_main_session(caller) == Some(session)
        })
}

pub fn run_location_matches(run: &Run, caller: &AgentCaller) -> bool {
    run.location.as_ref().is_some_and(|location| {
        location_matches(location, run.bound_omp_session.as_deref(), caller)
    })
}

/// Read-side precheck: unlike session_matches, missing bindings/declarations pass.
pub fn bound_session_conflicts(run: &Run, caller: &AgentCaller) -> bool {
    declared_main_session(caller).is_some_and(|session| {
        run.bound_omp_session
            .as_deref()
            .is_some_and(|bound| bound != session)
    })
}

fn exact_location_matches(run: &Run, caller: &AgentCaller) -> bool {
    run.location.as_ref().is_some_and(|location| {
        location_matches(location, run.bound_omp_session.as_deref(), caller)
            && location.pane_id == caller.pane_id
            && location.workspace_id == caller.workspace_id
            && location.tab_id == caller.tab_id
    })
}

// Startup read admission, not dispatch's owner-recovery classification.
fn startup_launch_matches(run: &Run, caller: &AgentCaller) -> bool {
    run.stage == RunStage::Preparing
        && matches!(run.kind, RunKind::Supervisor | RunKind::Worker)
        && run.close_reason.is_none()
        && run.retirement.is_none()
        && run.dispatch.as_ref().is_some_and(|dispatch| {
            matches!(
                dispatch.step,
                DispatchStep::LaunchIntent
                    | DispatchStep::LaunchPending
                    | DispatchStep::LaunchUnknown
            ) && !dispatch.agent_started
                && dispatch.launch_attempt > 0
                && dispatch.endpoint_identity.as_deref() == Some(caller.endpoint_identity.as_str())
                && caller
                    .native_session_id
                    .as_deref()
                    .is_none_or(|native| caller.omp_session_id.as_deref() == Some(native))
                && run.location.as_ref().is_some_and(|location| {
                    dispatch.launch_tag.as_deref() == Some(location.launch_tag.as_str())
                        && !location.launch_tag.is_empty()
                        && location
                            .native_session_id
                            .as_deref()
                            .is_none_or(|native| caller.omp_session_id.as_deref() == Some(native))
                })
        })
}

// Settling read admission, not the binding-only same_process_main_rollover rule.
fn settling_launch_matches(run: &Run, caller: &AgentCaller) -> bool {
    if startup_launch_matches(run, caller) {
        return true;
    }
    // Launch proof can commit between this command's opening runtime and its
    // fresh postcheck. Retry with newly attested evidence, never accept a stale
    // caller snapshot or revive an observer on a mature/retired run.
    matches!(
        (run.kind, run.stage),
        (RunKind::Supervisor, RunStage::Active) | (RunKind::Worker, RunStage::Initializing)
    ) && run.retirement.is_none()
        && run.close_reason.is_none()
        && run.bound_omp_session == caller.omp_session_id
        && run.bound_omp_process == caller.process
        && run.dispatch.as_ref().is_some_and(|dispatch| {
            dispatch.step == DispatchStep::Launched
                && dispatch.agent_started
                && dispatch.launch_attempt > 0
                && dispatch.endpoint_identity.as_deref() == Some(caller.endpoint_identity.as_str())
                && run.location.as_ref().is_some_and(|location| {
                    dispatch.launch_tag.as_deref() == Some(location.launch_tag.as_str())
                        && !location.launch_tag.is_empty()
                        && location
                            .native_session_id
                            .as_deref()
                            .is_none_or(|native| caller.omp_session_id.as_deref() == Some(native))
                })
        })
}

/// Exact opening evidence eligible for a retry while native launch proof settles.
pub fn settling_caller_matches(run: &Run, caller: &AgentCaller, attempt: u32) -> bool {
    run.attempt == attempt
        && run.session_id == caller.session_id
        && settling_launch_matches(run, caller)
        && exact_location_matches(run, caller)
        && run
            .bound_omp_session
            .as_deref()
            .is_none_or(|bound| Some(bound) == declared_main_session(caller))
        && caller.process.as_ref().is_some_and(|process| {
            run.bound_omp_process
                .as_ref()
                .is_none_or(|bound| bound == process)
        })
}

pub fn not_ready() -> InspectionError {
    InspectionError::new(
        "caller_not_ready",
        "Herdr has not yet attested this pane's native OMP; retry",
    )
}

/// Retirement read admission retains the CLI subset, not identity_matches' write fence.
pub fn retirement_read_scope(run: &Run, caller: &AgentCaller) -> Result<(), InspectionError> {
    let mismatch = || {
        InspectionError::new(
            "caller_mismatch",
            "retirement is scoped to the exact worker main incarnation",
        )
    };
    if caller.agent_kind != Some(AgentKind::Main)
        || caller.subagent_id.is_some()
        || caller
            .env_run
            .as_ref()
            .is_none_or(|(id, _)| id != &run.run_id)
        || run.session_id != caller.session_id
    {
        return Err(mismatch());
    }
    if caller
        .env_run
        .as_ref()
        .is_none_or(|(_, attempt)| *attempt != run.attempt)
    {
        return Err(InspectionError::new(
            "attempt_stale",
            "caller run attempt is stale",
        ));
    }
    let Some(session) = caller.omp_session_id.as_deref().filter(|id| !id.is_empty()) else {
        return Err(InspectionError::new(
            "session_mismatch",
            "retirement requires the actual main session",
        ));
    };
    if run.bound_omp_session.as_deref() != Some(session) {
        return Err(InspectionError::new(
            "session_mismatch",
            "retirement main session differs from binding",
        ));
    }
    let Some(process) = caller.process.as_ref() else {
        return Err(mismatch());
    };
    if run.bound_omp_process.as_ref() != Some(process) {
        return Err(mismatch());
    }
    let Some(location) = &run.location else {
        return Err(mismatch());
    };
    if !exact_location_matches(run, caller) {
        return Err(mismatch());
    }
    if run.stage != RunStage::Closed && run.retirement.is_some() {
        return Err(mismatch());
    }
    if caller.actual_agent_kind.as_ref() != Some(&NativeAgentKind::Omp) {
        if caller.actual_agent_kind.is_none() && startup_launch_matches(run, caller) {
            return Err(not_ready());
        }
        return Err(mismatch());
    }
    if run.stage == RunStage::Closed {
        if run.close_reason != Some(CloseReason::Accepted) || run.kind != RunKind::Worker {
            return Err(mismatch());
        }
        let identity = run
            .retirement
            .as_ref()
            .and_then(|retirement| retirement.identity.as_ref())
            .ok_or_else(mismatch)?;
        if identity.run_attempt != run.attempt
            || run.dispatch.as_ref().is_none_or(|dispatch| {
                dispatch.launch_attempt != identity.launch_attempt
                    || dispatch.launch_tag.as_deref() != Some(identity.launch_tag.as_str())
                    || dispatch.endpoint_identity.as_deref()
                        != Some(identity.endpoint_identity.as_str())
            })
            || location.launch_tag != identity.launch_tag
            || location.terminal_id.as_deref() != Some(identity.terminal_id.as_str())
            || identity.omp_session_id != session
            || &identity.process != process
            || run.launch_shell_identity.as_ref() != Some(&identity.shell)
            || !retirement::caller_location_matches(run, caller)
        {
            return Err(mismatch());
        }
    }
    Ok(())
}
