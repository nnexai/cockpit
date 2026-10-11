use cockpit_protocol::orchestration::*;

use std::collections::{HashMap, HashSet};

use super::store::OrchestrationState;
use super::{Actor, AgentCaller, AuthorizedJob, NativeAgentKind, actor_ref, error, is_ancestor, now, run_index};
use super::credentials::{JobRole, validate_job_auth};
use crate::InspectionError;

mod inbox;
mod report;
mod subagent_control;

const MAX_TEXT: usize = 16 * 1024;

fn text_bound(text: &str) -> Result<(), InspectionError> {
    if text.len() > MAX_TEXT {
        return Err(error(
            "message_too_large",
            "Message text must not exceed 16 KiB",
        ));
    }
    Ok(())
}

fn same_actor(left: &ActorRef, right: &ActorRef) -> bool {
    match (left, right) {
        (ActorRef::Operator, ActorRef::Operator) | (ActorRef::Dispatcher, ActorRef::Dispatcher) => {
            true
        }
        (ActorRef::Run { run_id: left }, ActorRef::Run { run_id: right }) => left == right,
        _ => false,
    }
}

fn same_report(left: Option<&Report>, right: Option<&Report>) -> bool {
    match (left, right) {
        (None, None) => true,
        (Some(left), Some(right)) => {
            left.message_id == right.message_id
                && left.kind == right.kind
                && left.outcome == right.outcome
                && left.summary == right.summary
                && left.plan == right.plan
                && left.source_output == right.source_output
        }
        _ => false,
    }
}

fn duplicate<'a>(
    state: &'a OrchestrationState,
    from: &ActorRef,
    message_id: &str,
) -> Option<&'a Message> {
    state
        .messages
        .iter()
        .find(|message| message.message_id == message_id && same_actor(&message.from, from))
}

fn message_result(message: &Message, duplicate: bool) -> OrchestrationActionResult {
    OrchestrationActionResult::Message {
        to_run_id: message.to_run_id.clone(),
        seq: message.seq,
        duplicate,
        stale: message.stale,
    }
}
/// Progress is a replaceable delivery view; durable message IDs remain available for replay.
pub fn latest_progress_sequences(messages: &[Message], run_id: &str) -> HashSet<u64> {
    let mut latest = HashMap::new();
    for message in messages.iter().filter(|message| {
        message.to_run_id == run_id
            && message.report.as_ref().is_some_and(|report| report.kind == ReportKind::Progress)
    }) {
        let sender = match &message.from {
            ActorRef::Run { run_id } => Some(run_id.as_str()),
            _ => None,
        };
        latest.insert((sender, message.from_subagent_id.as_deref()), message.seq);
    }
    latest.into_values().collect()
}

fn deliverable(message: &Message, latest_progress: &HashSet<u64>) -> bool {
    !message.report.as_ref().is_some_and(|report| report.kind == ReportKind::Progress)
        || latest_progress.contains(&message.seq)
}

pub(crate) fn current_question<'a>(messages: &'a [Message], run_id: &str) -> Option<&'a Report> {
    messages.iter().rev().find_map(|message| {
        if message.stale || message.from_subagent_id.is_some()
            || !matches!(&message.from, ActorRef::Run { run_id: sender } if sender == run_id)
        {
            return None;
        }
        message.report.as_ref().filter(|report| {
            matches!(report.kind, ReportKind::NeedsInput | ReportKind::Result)
        })
    }).filter(|report| report.kind == ReportKind::NeedsInput)
}

pub(crate) struct AppendMessage<'a> {
    pub from: ActorRef,
    pub to_run_id: &'a str,
    pub message_id: &'a str,
    pub kind: MessageKind,
    pub text: &'a str,
    pub in_reply_to: Option<String>,
    pub report: Option<Report>,
    pub stale: bool,
    pub from_subagent_id: Option<String>,
    pub escalated_from: Option<String>,
}

/// Append under the state lock. The sender's key covers the entire payload, not just text.
pub(crate) fn append(
    state: &mut OrchestrationState,
    message: AppendMessage<'_>,
) -> Result<OrchestrationActionResult, InspectionError> {
    let AppendMessage {
        from, to_run_id, message_id, kind, text, in_reply_to, report, stale,
        from_subagent_id, escalated_from,
    } = message;
    if message_id.is_empty() {
        return Err(error("invalid_message_id", "A message id is required"));
    }
    text_bound(text)?;
    if let Some(report) = &report {
        text_bound(&report.summary)?;
        if let Some(plan) = &report.plan {
            text_bound(plan)?;
        }
        if kind != MessageKind::Report || report.message_id != message_id {
            return Err(error(
                "invalid_message",
                "Report metadata must match the message kind and id",
            ));
        }
    } else if kind == MessageKind::Report {
        return Err(error(
            "invalid_message",
            "Report messages require report metadata",
        ));
    }
    run_index(state, to_run_id)?;
    if let Some(existing) = duplicate(state, &from, message_id) {
        if existing.to_run_id != to_run_id
            || existing.kind != kind
            || existing.text != text
            || existing.in_reply_to != in_reply_to
            || !same_report(existing.report.as_ref(), report.as_ref())
            || existing.from_subagent_id != from_subagent_id
            || existing.escalated_from != escalated_from
        {
            return Err(error(
                "message_id_conflict",
                "The sender already used this message id for a different payload",
            ));
        }
        return Ok(message_result(existing, true));
    }
    let seq = state
        .messages
        .iter()
        .filter(|message| message.to_run_id == to_run_id)
        .map(|message| message.seq)
        .max()
        .unwrap_or(0)
        .checked_add(1)
        .filter(|seq| *seq <= super::MAX_SAFE_COUNTER)
        .ok_or_else(|| {
            error(
                "message_sequence_exhausted",
                "JSON-safe recipient message sequence is exhausted",
            )
        })?;
    let message = Message {
        message_id: message_id.to_owned(),
        to_run_id: to_run_id.to_owned(),
        seq,
        from,
        kind,
        text: text.to_owned(),
        in_reply_to,
        report,
        stale,
        escalated_from,
        from_subagent_id,
        stage: DeliveryStage::Stored,
        woken_omp_session: None,
        created_at: now(),
        acked_at: None,
    };
    let result = message_result(&message, false);
    state.messages.push(message);
    Ok(result)
}

fn agent<'a>(
    actor: &'a Actor,
    caller: Option<&AuthorizedJob>,
) -> Result<(&'a AgentCaller, usize), InspectionError> {
    match (actor, caller) {
        (Actor::Agent(agent), Some(job)) => Ok((agent, job.run_index)),
        _ => Err(error("actor_forbidden", "This action requires an admitted job credential")),
    }
}

fn main_session(caller: Option<&AuthorizedJob>) -> Result<(), InspectionError> {
    if caller.is_none_or(|job| job.role == JobRole::Child) {
        return Err(error("report_requires_main", "This action requires the job's main role"));
    }
    Ok(())
}

fn bound_context(
    state: &OrchestrationState,
    agent: &AgentCaller,
    caller: Option<usize>,
) -> Result<AuthorizedJob, InspectionError> {
    let job = validate_job_auth(state, &agent.auth)?;
    if caller != Some(job.run_index) {
        return Err(error("caller_unbound", "The admitted job does not match the caller"));
    }
    if job.role != JobRole::Child
        && (agent.agent_kind == Some(AgentKind::Subagent)
            || agent.subagent_id.is_some()
            || agent.provenance.as_ref().is_some_and(|provenance| {
                provenance.agent_kind == Some(AgentKind::Subagent) || provenance.child_id.is_some()
            }))
    {
        return Err(error("actor_forbidden", "A claimed child context requires its narrowed child credential"));
    }
    Ok(job)
}

/// A child credential identifies the sender even when its physical context is offline.
/// Only freshly matching native evidence may refresh the diagnostic session binding.
fn authenticated_child_session<'a>(
    state: &OrchestrationState,
    agent: &'a AgentCaller,
    job: &AuthorizedJob,
) -> Result<Option<&'a str>, InspectionError> {
    if job.role != JobRole::Child {
        return Err(error("actor_forbidden", "A child credential is required"));
    }
    let attachment = state.runs[job.run_index].attachment.as_ref();
    let session = agent.omp_session_id.as_deref().filter(|session| !session.is_empty());
    Ok(if agent.actual_agent_kind.as_ref() == Some(&NativeAgentKind::Omp)
        && agent.agent_kind == Some(AgentKind::Subagent)
        && agent.subagent_id == job.subject_child_id
        && attachment.is_some_and(|attachment| {
            attachment.process.is_some()
                && agent.process == attachment.process
                && attachment.main_session_id.is_some()
                && agent.main_omp_session_id == attachment.main_session_id
                && agent.omp_session_id != attachment.main_session_id
        })
    {
        session
    } else {
        None
    })
}

/// Actual native effects remain fenced to the current attached incarnation.
fn native_control_context(
    state: &OrchestrationState,
    agent: &AgentCaller,
    job: &AuthorizedJob,
) -> Result<(), InspectionError> {
    if job.role == JobRole::Child {
        return authenticated_child_session(state, agent, job)?
            .map(|_| ())
            .ok_or_else(|| error("session_mismatch", "Native child control requires current physical evidence"));
    }
    let attachment = state.runs[job.run_index].attachment.as_ref();
    if agent.actual_agent_kind.as_ref() != Some(&NativeAgentKind::Omp)
        || agent.agent_kind != Some(AgentKind::Main)
        || !attachment.is_some_and(|attachment| {
            attachment.main_session_id.is_some()
                && agent.omp_session_id == attachment.main_session_id
                && attachment.process.is_some()
                && agent.process == attachment.process
        })
    {
        return Err(error("session_mismatch", "Native control requires the current main incarnation"));
    }
    Ok(())
}

fn subagent_context(caller: Option<&AuthorizedJob>) -> Option<String> {
    caller.and_then(|job| job.subject_child_id.clone())
}

/// Closed ancestors escalate upward; a closed root remains an operator-visible evidence inbox.
fn upward(
    state: &OrchestrationState,
    sender: usize,
    requested: Option<&str>,
    own_main_report: bool,
) -> Result<(String, Option<String>), InspectionError> {
    let run = &state.runs[sender];
    let target = requested
        .or(run.parent_run_id.as_deref())
        .unwrap_or(&run.run_id);
    if !(own_main_report && requested == Some(run.run_id.as_str()))
        && (target != run.run_id || run.parent_run_id.is_some())
    {
        if !is_ancestor(state, target, &run.run_id) {
            return Err(error(
                "not_ancestor",
                "Reports and observations may only address strict ancestors",
            ));
        }
    }
    let mut recipient = run_index(state, target)?;
    for _ in 0..state.runs.len() {
        if state.runs[recipient].stage != RunStage::Closed {
            break;
        }
        let Some(parent) = state.runs[recipient].parent_run_id.as_deref() else {
            break;
        };
        recipient = run_index(state, parent)?;
    }
    let recipient_id = state.runs[recipient].run_id.clone();
    let escalated = (recipient_id != target).then(|| target.to_owned());
    Ok((recipient_id, escalated))
}

pub(crate) fn apply(
    state: &mut OrchestrationState,
    actor: &Actor,
    caller: Option<usize>,
    stale: bool,
    action: &OrchestrationAction,
) -> Result<Option<OrchestrationActionResult>, InspectionError> {
    if !matches!(
        action,
        OrchestrationAction::Report { .. }
            | OrchestrationAction::MessageSend { .. }
            | OrchestrationAction::Annotate { .. }
            | OrchestrationAction::InboxPull { .. }
            | OrchestrationAction::InboxWoken { .. }
            | OrchestrationAction::InboxAck { .. }
            | OrchestrationAction::SubagentUpdate { .. }
            | OrchestrationAction::SubagentControl { .. }
            | OrchestrationAction::SubagentControlDone { .. }
    ) {
        return Ok(None);
    }
    let job = match actor {
        Actor::Agent(agent) => Some(bound_context(state, agent, caller)?),
        Actor::Operator(_) => None,
        Actor::Bootstrap(_) => return Err(error("actor_forbidden", "Physical bootstrap does not authorize ordinary messages")),
    };
    if stale {
        return Err(error("attempt_stale", "The original task or claim is no longer current"));
    }
    if let Some(job) = &job {
        if job.role == JobRole::Child {
            match action {
                OrchestrationAction::Report {
                    kind: ReportKind::Initialized | ReportKind::Result, ..
                } => return Err(error("report_requires_main", "Initialized and Result require the job's main role")),
                OrchestrationAction::Report {
                    kind: ReportKind::Progress | ReportKind::NeedsInput, ..
                }
                | OrchestrationAction::MessageSend { kind: MessageKind::Observation, .. }
                | OrchestrationAction::SubagentUpdate { .. }
                | OrchestrationAction::SubagentControlDone { .. } => {}
                _ => return Err(error("actor_forbidden", "Children may only report their own progress or question")),
            }
            if !matches!(action,
                OrchestrationAction::SubagentControlDone { .. }
                    | OrchestrationAction::SubagentUpdate { status: SubagentStatus::Cancelled, .. }
            ) {
                let child = state.subagents.iter().find(|child| {
                    child.run_id == state.runs[job.run_index].run_id
                        && Some(child.subagent_id.as_str()) == job.subject_child_id.as_deref()
                }).ok_or_else(|| error("subagent_not_found", "Child operations require a registered subject"))?;
                if child.status != SubagentStatus::Running {
                    return Err(error("invalid_stage", "Only a running registered child may update ordinary job state"));
                }
            }
        }
    }
    let caller = job.as_ref();
    match action {
        OrchestrationAction::Report { .. } => {
            report::apply(state, actor, caller, action)
        }
        OrchestrationAction::MessageSend { .. }
        | OrchestrationAction::Annotate { .. }
        | OrchestrationAction::InboxPull { .. }
        | OrchestrationAction::InboxWoken { .. }
        | OrchestrationAction::InboxAck { .. } => {
            inbox::apply(state, actor, caller, action)
        }
        OrchestrationAction::SubagentUpdate { .. }
        | OrchestrationAction::SubagentControl { .. }
        | OrchestrationAction::SubagentControlDone { .. } => {
            subagent_control::apply(state, actor, caller, action)
        }
        _ => Ok(None),
    }
}

#[cfg(test)]
mod tests;
