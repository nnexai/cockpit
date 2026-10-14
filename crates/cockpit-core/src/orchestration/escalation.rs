use cockpit_protocol::orchestration::*;

use super::{messages, parse_time, store::OrchestrationState};
use crate::InspectionError;

#[derive(Clone, Copy)]
enum EscalationErrorCode {
    OwnedLaunchTabUnsafe,
    AutomaticLaunchCloseUnproven,
    AutomaticSpacePreservationUnproven,
    InvalidReconcile,
    WorkspaceConflict,
    StaleIdentity,
    RepositoryIdentityStale,
    ReconciliationRequiresInspection,
    Other,
}

impl From<&str> for EscalationErrorCode {
    fn from(code: &str) -> Self {
        match code {
            "owned_launch_tab_unsafe" => Self::OwnedLaunchTabUnsafe,
            "automatic_launch_close_unproven" => Self::AutomaticLaunchCloseUnproven,
            "automatic_space_preservation_unproven" => Self::AutomaticSpacePreservationUnproven,
            "invalid_reconcile" => Self::InvalidReconcile,
            "workspace_conflict" => Self::WorkspaceConflict,
            "stale_identity" => Self::StaleIdentity,
            "repository_identity_stale" => Self::RepositoryIdentityStale,
            "reconciliation_requires_inspection" => Self::ReconciliationRequiresInspection,
            _ => Self::Other,
        }
    }
}

fn eligible(run: &Run) -> bool {
    run.kind == RunKind::Worker && !matches!(run.stage, RunStage::Reported | RunStage::Closed)
}

fn incarnation(run: &Run) -> String {
    let attachment = run.attachment.as_ref();
    format!("{}:{}:{}:{}:{}", run.run_id, run.attempt,
        attachment.map_or("unattached", |attachment| attachment.attachment_id.as_str()),
        attachment.map_or(0, |attachment| attachment.epoch),
        run.dispatch.as_ref().expect("dispatch").launch_attempt)
}

fn dispatcher_message(message: &Message, root: &str) -> bool {
    matches!(message.from, ActorRef::Dispatcher)
        && message.kind == MessageKind::Observation
        && message.to_run_id == root
}

/// A placement receipt or latch-write intent is physical uncertainty, not an
/// ordinary harness-observation timeout. An occupant cannot prove these effects.
fn placement_boundary(run: &Run) -> Option<&'static str> {
    let dispatch = run.dispatch.as_ref()?;
    if matches!(dispatch.step, DispatchStep::MoveIntent | DispatchStep::MoveUnknown) {
        Some("move_unresolved")
    } else if matches!(dispatch.latch_state, Some(AgentLatchState::ReleaseIntent | AgentLatchState::Unknown)) {
        Some("latch_unresolved")
    } else {
        None
    }
}

fn next_steps(run: &Run) -> (&'static str, &'static [&'static str], Option<&'static str>) {
    let dispatch = run.dispatch.as_ref().expect("dispatch");
    if let Some(boundary) = placement_boundary(run) {
        let reason = if boundary == "move_unresolved" {
            "The recorded terminal move has no exact outcome receipt. Preserve the staging and destination resources; inspect the recorded source terminal and move operation. Do not replay the move or release the launch latch. Use an isolated replacement only if the current attempt cannot be safely proven."
        } else {
            "The launch latch release has no exact outcome receipt. The agent may already be running. Preserve its terminal, checkout and claims; inspect the recorded release intent. Do not resend the latch write or restart in the same checkout. Use an isolated replacement if the current attempt cannot be safely proven."
        };
        return ("unknown", &["show", "worker_replace"], Some(reason));
    }
    let error_code = dispatch.error.as_ref().map(|error| EscalationErrorCode::from(error.code.as_str()));
    if matches!(error_code,
        Some(EscalationErrorCode::OwnedLaunchTabUnsafe
            | EscalationErrorCode::AutomaticLaunchCloseUnproven
            | EscalationErrorCode::AutomaticSpacePreservationUnproven))
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
        if matches!(error_code,
            Some(EscalationErrorCode::InvalidReconcile
                | EscalationErrorCode::WorkspaceConflict
                | EscalationErrorCode::StaleIdentity
                | EscalationErrorCode::RepositoryIdentityStale
                | EscalationErrorCode::ReconciliationRequiresInspection)) {
            return ("unknown", &["show", "operator"], Some("Setup evidence did not prove one existing checkout; the operator must decide"));
        }
        return ("unknown", &["show", "reconcile_accept_existing_worktree", "operator"], Some("Only checkout-proven recovery is supervisor-owned; environment retry needs the operator"));
    }
    if run.attachment.as_ref().is_some_and(|attachment| {
        attachment.conversation.is_some() && attachment.process.is_none()
    }) {
        return ("unknown", &["show", "reconcile", "operator"], Some("Original OMP process identity was not recorded; only the operator can decide a restart"));
    }
    if dispatch.launch_tag.is_none() {
        return ("none", &["show", "resume"], None);
    }
    if dispatch.step == DispatchStep::LaunchUnknown && !dispatch.agent_started {
        return ("unknown", &["show", "resume"], None);
    }
    ("unknown", &["show", "reconcile", "resume"], None)
}

fn append_failure(state: &mut OrchestrationState, index: usize, unresolved: bool) -> Result<bool, InspectionError> {
    let run = &state.runs[index];
    let dispatch = run.dispatch.as_ref().expect("dispatch");
    let prefix = format!("dispatch-failure:{}:", incarnation(run));
    let code = dispatch.error.as_ref().map(|error| error.code.as_str()).unwrap_or("dispatch_uncertain");
    let (key, semantic) = if let Some(boundary) = placement_boundary(run) {
        (format!("{prefix}{boundary}"), None)
    } else if unresolved {
        (format!("{prefix}launch_unresolved"), None)
    } else {
        let step = serde_json::to_value(dispatch.step).expect("serializable step");
        let semantic = format!("{prefix}{}:{code}:", step.as_str().expect("step string"));
        (format!("{semantic}{}", dispatch.updated_at), Some(semantic))
    };
    if state.messages.iter().any(|message| {
        dispatcher_message(message, &run.root_id) && (message.message_id == key
            || semantic.as_ref().is_some_and(|prefix| message.message_id.starts_with(prefix)
                && message.stage == DeliveryStage::Stored))
    }) {
        return Ok(false);
    }
    let (effect, next, operator_reason) = next_steps(run);
    let detail = dispatch.error.as_ref().map(|error| error.message.as_str()).unwrap_or("Dispatch outcome requires inspection");
    let mut end = detail.len().min(1024);
    while !detail.is_char_boundary(end) { end -= 1; }
    let mut body = serde_json::json!({
        "event": "dispatch_failure", "run_id": run.run_id, "task_id": run.task_id,
        "run_attempt": run.attempt, "launch_attempt": dispatch.launch_attempt,
        "stage": run.stage, "step": dispatch.step, "agent_started": dispatch.agent_started,
        "error": {"code": code, "message": &detail[..end]},
        "automatic": if unresolved { "exhausted" } else { "none" },
        "effect": effect, "next": next, "operator_reason": operator_reason,
    });
    if placement_boundary(run).is_some() {
        body["move_operation_id"] = serde_json::json!(dispatch.move_operation_id);
        body["source_location"] = serde_json::json!(dispatch.source_location);
        body["latch_state"] = serde_json::json!(dispatch.latch_state);
    }
    let text = body.to_string();
    let root = run.root_id.clone();
    messages::append(state, messages::AppendMessage { from: ActorRef::Dispatcher,
    to_run_id: &root,
    message_id: &key,
    kind: MessageKind::Observation,
    text: &text,
    in_reply_to: None,
    report: None,
    stale: false, escalated_from: None, })?;
    Ok(true)
}

/// Called in the same locked transaction as the dispatch failure transition.
pub(super) fn failure(state: &mut OrchestrationState, index: usize) -> Result<bool, InspectionError> {
    let run = &state.runs[index];
    if eligible(run) && run.dispatch.as_ref().is_some_and(|dispatch| {
        dispatch.step == DispatchStep::MoveUnknown || dispatch.latch_state == Some(AgentLatchState::Unknown)
    }) {
        return append_failure(state, index, false);
    }
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
    if !eligible(run) || run.dispatch.as_ref().is_none_or(|dispatch| {
        dispatch.step != DispatchStep::Launched || !dispatch.agent_started
            || run.attachment.as_ref().is_none_or(|attachment| {
                matches!(attachment.state, AttachmentState::Reserved | AttachmentState::Revoked)
                    || attachment.launch_attempt != dispatch.launch_attempt
                    || attachment.conversation.is_none()
            })
    }) {
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
    messages::append(state, messages::AppendMessage { from: ActorRef::Dispatcher,
    to_run_id: &root,
    message_id: &key,
    kind: MessageKind::Observation,
    text: &text,
    in_reply_to: None,
    report: None,
    stale: false, escalated_from: None, })?;
    Ok(true)
}

// Suppress a stale unknown outcome when this fresh snapshot already proves the
// bound OMP occupant. Display names are not launch identity. The automatic
// dispatcher remains the sole writer of the verified launch transition.
fn freshly_proven(
    run: &Run,
    runtime: &super::herdr::RuntimeView,
    kernel_boot_id: &mut Option<Option<String>>,
    machine_id: &mut Option<Option<String>>,
) -> bool {
    let Some(location) = run.attachment.as_ref().and_then(|attachment| attachment.location.as_ref()) else { return false };
    if location.endpoint_identity != runtime.endpoint_identity
        || !super::optional_available(&location.boot_id, &runtime.boot_id)
    { return false; }
    runtime.panes.iter().any(|pane| {
        super::optional_available(&location.native_session_id, &pane.native_session_id)
            && super::projection::actual_agent(run, pane, kernel_boot_id, machine_id)
    })
}

/// Reuse the existing observer, including launches that have no tab receipt.
pub(super) fn unresolved_launches(state: &mut OrchestrationState, session_id: &str, settle_ms: u64, now: time::OffsetDateTime, runtime: &super::herdr::RuntimeView) -> Result<bool, InspectionError> {
    let mut changed = false;
    let mut kernel_boot_id = None;
    let mut machine_id = None;
    for index in 0..state.runs.len() {
        let run = &state.runs[index];
        let physical_boundary = placement_boundary(run).is_some();
        if !physical_boundary && super::dispatch::automatic_recovery_available(run) { continue; }
        if run.session_id != session_id || !eligible(run) || run.dispatch.as_ref().is_none_or(|dispatch| {
            (!physical_boundary && (dispatch.step != DispatchStep::LaunchUnknown || dispatch.agent_started))
                || parse_time(&dispatch.updated_at).is_none_or(|entered| (now - entered).whole_milliseconds() < i128::from(settle_ms))
        }) || (!physical_boundary && freshly_proven(run, runtime, &mut kernel_boot_id, &mut machine_id)) { continue; }
        changed |= append_failure(state, index, true)?;
    }
    Ok(changed)
}
