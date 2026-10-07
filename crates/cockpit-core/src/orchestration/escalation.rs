use cockpit_protocol::orchestration::*;

use super::{messages, parse_time, store::OrchestrationState};
use crate::InspectionError;

fn eligible(run: &Run) -> bool {
    run.kind == RunKind::Worker && !matches!(run.stage, RunStage::Reported | RunStage::Closed)
}

fn incarnation(run: &Run) -> String {
    format!("{}:{}:{}", run.run_id, run.attempt, run.dispatch.as_ref().expect("dispatch").launch_attempt)
}

fn dispatcher_message(message: &Message, root: &str) -> bool {
    matches!(message.from, ActorRef::Dispatcher)
        && message.kind == MessageKind::Observation
        && message.to_run_id == root
}

fn next_steps(run: &Run) -> (&'static str, &'static [&'static str], Option<&'static str>) {
    let dispatch = run.dispatch.as_ref().expect("dispatch");
    if dispatch.error.as_ref().is_some_and(|error| matches!(error.code.as_str(),
        "owned_launch_tab_unsafe" | "automatic_launch_close_unproven" | "automatic_space_preservation_unproven"))
    {
        return ("unknown", &["show", "reconcile", "operator"],
            Some("Owned launch cancellation or source-Space preservation was refused or remains unproven; inspect the exact recorded Space, tab and terminal. Do not repeat uncertain creation or closure."));
    }
    if dispatch.step == DispatchStep::PlanFailed {
        return ("none", &["show", "reconcile"], None);
    }
    let unknown_setup = dispatch.launch_tag.is_none()
        && run.setup.as_ref().is_some_and(|setup| setup.operation_id.is_some() && setup.workspace_id.is_none());
    if unknown_setup {
        if dispatch.error.as_ref().is_some_and(|error| matches!(error.code.as_str(),
            "invalid_reconcile" | "workspace_conflict" | "stale_identity" | "repository_identity_stale" | "reconciliation_requires_inspection")) {
            return ("unknown", &["show", "operator"], Some("Setup evidence did not prove one existing checkout; the operator must decide"));
        }
        return ("unknown", &["show", "reconcile_accept_existing_worktree", "operator"], Some("Only checkout-proven recovery is supervisor-owned; environment retry needs the operator"));
    }
    if run.bound_omp_session.is_some() && run.bound_omp_process.is_none() {
        return ("unknown", &["show", "reconcile", "operator"], Some("Original OMP process identity was not recorded; only the operator can decide a restart"));
    }
    if dispatch.launch_tag.is_none() {
        return ("none", &["show", "retry_launch"], None);
    }
    if dispatch.step == DispatchStep::LaunchUnknown && !dispatch.agent_started {
        return ("unknown", &["show", "retry_launch"], None);
    }
    ("unknown", &["show", "reconcile", "retry_launch"], None)
}

fn append_failure(state: &mut OrchestrationState, index: usize, unresolved: bool) -> Result<bool, InspectionError> {
    let run = &state.runs[index];
    let dispatch = run.dispatch.as_ref().expect("dispatch");
    let prefix = format!("dispatch-failure:{}:", incarnation(run));
    let code = dispatch.error.as_ref().map(|error| error.code.as_str()).unwrap_or("dispatch_uncertain");
    let (key, semantic) = if unresolved {
        (format!("{prefix}launch_unresolved"), None)
    } else {
        let step = serde_json::to_value(dispatch.step).expect("serializable step");
        let semantic = format!("{prefix}{}:{code}:", step.as_str().expect("step string"));
        (format!("{semantic}{}", dispatch.updated_at), Some(semantic))
    };
    if state.messages.iter().any(|message| {
        dispatcher_message(message, &run.root_id) && (message.message_id == key
            || semantic.as_ref().is_some_and(|prefix| message.message_id.starts_with(prefix)
                && matches!(message.stage, DeliveryStage::Stored | DeliveryStage::Woken)))
    }) {
        return Ok(false);
    }
    let (effect, next, operator_reason) = next_steps(run);
    let detail = dispatch.error.as_ref().map(|error| error.message.as_str()).unwrap_or("Dispatch outcome requires inspection");
    let mut end = detail.len().min(1024);
    while !detail.is_char_boundary(end) { end -= 1; }
    let text = serde_json::json!({
        "event": "dispatch_failure", "run_id": run.run_id, "task_id": run.task_id,
        "run_attempt": run.attempt, "launch_attempt": dispatch.launch_attempt,
        "stage": run.stage, "step": dispatch.step, "agent_started": dispatch.agent_started,
        "error": {"code": code, "message": &detail[..end]},
        "automatic": if unresolved { "exhausted" } else { "none" },
        "effect": effect, "next": next, "operator_reason": operator_reason,
    }).to_string();
    let root = run.root_id.clone();
    messages::append(state, ActorRef::Dispatcher, &root, &key, MessageKind::Observation, &text, None, false, None, None)?;
    Ok(true)
}

/// Called in the same locked transaction as the dispatch failure transition.
pub(super) fn failure(state: &mut OrchestrationState, index: usize) -> Result<bool, InspectionError> {
    let run = &state.runs[index];
    if super::dispatch::recovery_close_pending(run)
        || (run.dispatch.as_ref().is_some_and(|d| d.step == DispatchStep::LaunchUnknown)
            && super::dispatch::automatic_recovery_available(run))
    {
        return Ok(false);
    }
    if !eligible(run) || run.dispatch.as_ref().is_none_or(|dispatch| !matches!(dispatch.step,
        DispatchStep::PlanFailed | DispatchStep::NeedsReview)
        && !(dispatch.step == DispatchStep::SetupUnknown && dispatch.recovery.is_none())
        && !(dispatch.step == DispatchStep::LaunchUnknown && dispatch.agent_started)) {
        return Ok(false);
    }
    append_failure(state, index, false)
}

/// A success notice is meaningful only after a failure of this exact incarnation.
pub(super) fn recovered(state: &mut OrchestrationState, index: usize) -> Result<bool, InspectionError> {
    let run = &state.runs[index];
    if !eligible(run) || run.dispatch.as_ref().is_none_or(|dispatch| dispatch.step != DispatchStep::Launched || !dispatch.agent_started) {
        return Ok(false);
    }
    let incarnation = incarnation(run);
    let failure_prefix = format!("dispatch-failure:{incarnation}:");
    let recovered_prefix = format!("dispatch-recovered:{incarnation}:");
    let mut failure_seq = 0;
    let mut recovered_seq = 0;
    for message in state.messages.iter().filter(|message| dispatcher_message(message, &run.root_id)) {
        if message.message_id.starts_with(&failure_prefix) { failure_seq = failure_seq.max(message.seq); }
        if message.message_id.starts_with(&recovered_prefix) { recovered_seq = recovered_seq.max(message.seq); }
    }
    if failure_seq <= recovered_seq { return Ok(false); }
    let dispatch = run.dispatch.as_ref().expect("dispatch");
    let key = format!("{recovered_prefix}{}", dispatch.updated_at);
    let text = serde_json::json!({"event": "dispatch_recovered", "run_id": run.run_id,
        "run_attempt": run.attempt, "launch_attempt": dispatch.launch_attempt, "step": dispatch.step}).to_string();
    let root = run.root_id.clone();
    messages::append(state, ActorRef::Dispatcher, &root, &key, MessageKind::Observation, &text, None, false, None, None)?;
    Ok(true)
}

// Suppress a stale unknown outcome when this fresh snapshot already proves the
// bound OMP occupant. Display names are not launch identity. The automatic
// dispatcher remains the sole writer of the verified launch transition.
fn freshly_proven(
    run: &Run,
    runtime: &super::herdr::RuntimeView,
    kernel_boot_id: &mut Option<Option<String>>,
) -> bool {
    let Some(location) = run.location.as_ref() else { return false };
    if location.endpoint_identity != runtime.endpoint_identity
        || !super::optional_available(&location.boot_id, &runtime.boot_id)
    { return false; }
    runtime.panes.iter().any(|pane| {
        super::optional_available(&location.native_session_id, &pane.native_session_id)
            && super::projection::actual_omp(run, pane, kernel_boot_id)
    })
}

/// Reuse the existing observer, including launches that have no tab receipt.
pub(super) fn unresolved_launches(state: &mut OrchestrationState, session_id: &str, settle_ms: u64, now: time::OffsetDateTime, runtime: &super::herdr::RuntimeView) -> Result<bool, InspectionError> {
    let mut changed = false;
    let mut kernel_boot_id = None;
    for index in 0..state.runs.len() {
        let run = &state.runs[index];
        if super::dispatch::automatic_recovery_available(run) { continue; }
        if run.session_id != session_id || !eligible(run) || run.dispatch.as_ref().is_none_or(|dispatch| {
            dispatch.step != DispatchStep::LaunchUnknown || dispatch.agent_started
                || parse_time(&dispatch.updated_at).is_none_or(|entered| (now - entered).whole_milliseconds() < i128::from(settle_ms))
        }) || freshly_proven(run, runtime, &mut kernel_boot_id) { continue; }
        changed |= append_failure(state, index, true)?;
    }
    Ok(changed)
}
