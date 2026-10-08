use cockpit_protocol::orchestration::*;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use super::store::OrchestrationState;
use super::{Actor, AgentCaller, actor_ref, error, is_ancestor, now, run_index};
use crate::InspectionError;

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

/// Append under the state lock. The sender's key covers the entire payload, not just text.
#[allow(clippy::too_many_arguments)]
pub(crate) fn append(
    state: &mut OrchestrationState,
    from: ActorRef,
    to_run_id: &str,
    message_id: &str,
    kind: MessageKind,
    text: &str,
    in_reply_to: Option<String>,
    report: Option<Report>,
    stale: bool,
    from_subagent_id: Option<String>,
    escalated_from: Option<String>,
) -> Result<OrchestrationActionResult, InspectionError> {
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
    caller: Option<usize>,
) -> Result<(&'a AgentCaller, usize), InspectionError> {
    match (actor, caller) {
        (Actor::Agent(agent), Some(index)) => Ok((agent, index)),
        (Actor::Agent(_), None) => Err(error("caller_unbound", "The caller is not bound to a run")),
        _ => Err(error(
            "actor_forbidden",
            "This action requires a bound agent",
        )),
    }
}

fn main_session(agent: &AgentCaller, run: &Run) -> Result<(), InspectionError> {
    if agent.agent_kind != Some(AgentKind::Main) {
        return Err(error(
            "report_requires_main",
            "This action requires the run's main OMP context",
        ));
    }
    if agent.omp_session_id.is_none() || agent.omp_session_id != run.bound_omp_session {
        return Err(error(
            "session_mismatch",
            "OMP session does not match the run's bound main session",
        ));
    }
    Ok(())
}

fn bound_context(agent: &AgentCaller, run: &Run) -> Result<(), InspectionError> {
    match agent.agent_kind {
        Some(AgentKind::Main) => main_session(agent, run),
        Some(AgentKind::Subagent) => {
            if agent.omp_session_id.as_deref().is_none_or(str::is_empty)
                || agent.main_omp_session_id.is_none()
                || agent.main_omp_session_id != run.bound_omp_session
                || agent.omp_session_id == run.bound_omp_session
            {
                return Err(error(
                    "session_mismatch",
                    "Subagent context must identify its own session and the run's bound main session",
                ));
            }
            subagent_context(agent)?;
            Ok(())
        }
        None => Err(error(
            "actor_forbidden",
            "An agent context kind is required",
        )),
    }
}

/// Native caller evidence comes from the trusted CLI process probe and the SDK's
/// live registry, not the parent-writable telemetry payload.
pub(super) fn authenticated_child_session<'a>(
    agent: &'a AgentCaller,
    run: &Run,
) -> Result<&'a str, InspectionError> {
    if agent.agent_kind != Some(AgentKind::Subagent)
        || agent.actual_agent_kind.as_deref() != Some("omp")
    {
        return Err(error("actor_forbidden", "An actual native child context is required"));
    }
    bound_context(agent, run)?;
    if agent.process.is_none()
        || run.bound_omp_process.is_none()
        || agent.process != run.bound_omp_process
    {
        return Err(error(
            "session_mismatch",
            "Child process evidence must match the run's authenticated main process",
        ));
    }
    Ok(agent.omp_session_id.as_deref().expect("validated child session"))
}

fn subagent_context(agent: &AgentCaller) -> Result<Option<String>, InspectionError> {
    if agent.agent_kind == Some(AgentKind::Subagent) {
        let id = agent
            .subagent_id
            .as_ref()
            .filter(|id| !id.is_empty())
            .ok_or_else(|| {
                error(
                    "actor_forbidden",
                    "A subagent context requires its own subagent id",
                )
            })?;
        Ok(Some(id.clone()))
    } else {
        Ok(None)
    }
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

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct ControlEnvelope {
    subagent_id: String,
    op: SubagentOp,
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
    if stale && !matches!(action, OrchestrationAction::Report { .. }) {
        return Err(error(
            "attempt_stale",
            "A stale caller may only append report evidence",
        ));
    }
    if !stale && matches!(actor, Actor::Agent(_)) {
        let (agent, index) = agent(actor, caller)?;
        if matches!(
            action,
            OrchestrationAction::Report {
                kind: ReportKind::Ready | ReportKind::Result,
                ..
            }
        ) && agent.agent_kind != Some(AgentKind::Main)
        {
            return Err(error(
                "report_requires_main",
                "Ready and Result require the main OMP context",
            ));
        }
        bound_context(agent, &state.runs[index])?;
    }
    let result = match action {
        OrchestrationAction::Report {
            message_id,
            kind,
            outcome,
            summary,
            plan,
            to_run_id,
        } => {
            let (agent, index) = agent(actor, caller)?;
            text_bound(summary)?;
            if let Some(plan) = plan {
                text_bound(plan)?;
            }
            if matches!(kind, ReportKind::Ready | ReportKind::Result) {
                if agent.agent_kind != Some(AgentKind::Main) {
                    return Err(error(
                        "report_requires_main",
                        "Ready and Result require the main OMP context",
                    ));
                }
                if !stale {
                    main_session(agent, &state.runs[index])?;
                }
            }
            let from_subagent_id = subagent_context(agent)?;
            let from = actor_ref(actor, caller, state);
            let report = Report {
                message_id: message_id.clone(),
                kind: *kind,
                outcome: *outcome,
                summary: summary.clone(),
                plan: plan.clone(),
                at: now(),
            };
            // Resolve the logical address before dedupe, but retain the original escalation receipt on retry.
            let logical_target = to_run_id
                .as_deref()
                .or(state.runs[index].parent_run_id.as_deref())
                .unwrap_or(&state.runs[index].run_id);
            let own_main_report = from_subagent_id.is_some()
                && to_run_id.as_deref() == Some(state.runs[index].run_id.as_str());
            if own_main_report {
                // The owning main session is an ancestor of its internal OMP subagents.
                // This route is append-only evidence, never the main run's receipt or control.
                bound_context(agent, &state.runs[index])?;
            }
            let (recipient, escalated) =
                upward(state, index, to_run_id.as_deref(), own_main_report)?;
            if let Some(existing) = duplicate(state, &from, message_id) {
                if existing.kind != MessageKind::Report
                    || existing.text != *summary
                    || !same_report(existing.report.as_ref(), Some(&report))
                    || existing.in_reply_to.is_some()
                    || existing.from_subagent_id != from_subagent_id
                    || existing
                        .escalated_from
                        .as_deref()
                        .unwrap_or(&existing.to_run_id)
                        != logical_target
                {
                    return Err(error(
                        "message_id_conflict",
                        "The sender already used this message id for a different report",
                    ));
                }
                return Ok(Some(message_result(existing, true)));
            }
            if !stale {
                match kind {
                    ReportKind::Ready => {
                        if !matches!(
                            state.runs[index].stage,
                            RunStage::Initializing | RunStage::Ready
                        ) {
                            return Err(error(
                                "invalid_stage",
                                "Ready requires an initializing or ready run",
                            ));
                        }
                        if plan.as_deref().is_none_or(|plan| plan.trim().is_empty()) {
                            return Err(error(
                                "invalid_plan",
                                "Ready requires a nonempty work plan",
                            ));
                        }
                    }
                    ReportKind::Result => {
                        let run = &state.runs[index];
                        if run.stage != RunStage::Working
                            && !(run.parent_run_id.is_none() && run.stage == RunStage::Active)
                        {
                            return Err(error(
                                "invalid_stage",
                                "Result requires a working run or active root",
                            ));
                        }
                        if outcome.is_none() {
                            return Err(error("invalid_report", "Result requires an outcome"));
                        }
                    }
                    _ => {}
                }
            }
            let work_plan = if !stale && *kind == ReportKind::Ready {
                let text = plan.as_ref().expect("validated work plan");
                let canonical = serde_json::to_vec(text)
                    .map_err(|failure| error("invalid_plan", failure.to_string()))?;
                Some(PlanRecord {
                    plan_revision: format!("{:x}", Sha256::digest(canonical)),
                    text: text.clone(),
                    created_at: report.at.clone(),
                })
            } else {
                None
            };
            let result = append(
                state,
                from,
                &recipient,
                message_id,
                MessageKind::Report,
                summary,
                None,
                Some(report.clone()),
                stale,
                from_subagent_id.clone(),
                escalated,
            )?;
            if !stale {
                if let Some(subagent_id) = &from_subagent_id {
                    let own = &state.runs[index].run_id;
                    if let Some(entry) = state
                        .subagents
                        .iter_mut()
                        .find(|entry| entry.run_id == *own && entry.subagent_id == *subagent_id)
                    {
                        if entry.summary.as_deref() != Some(report.summary.as_str()) {
                            entry.summary = Some(report.summary.clone());
                        }
                        entry.updated_at = report.at.clone();
                    }
                }
                let run = &mut state.runs[index];
                // Subagent reports are evidence, not the main run's outcome or progress receipt.
                if from_subagent_id.is_none() {
                    run.last_report = Some(report.clone());
                    match kind {
                        ReportKind::Ready => {
                            run.init_receipt = Some(report);
                            run.work_plan = work_plan;
                            run.stage = RunStage::Ready;
                        }
                        ReportKind::Result => {
                            run.result = Some(report);
                            if run.parent_run_id.is_some() {
                                run.stage = RunStage::Reported;
                            }
                        }
                        _ => {}
                    }
                    run.updated_at = now();
                }
                if from_subagent_id.is_none()
                    && matches!(
                        kind,
                        ReportKind::Ready | ReportKind::Result | ReportKind::NeedsInput
                    )
                    && recipient != state.runs[index].root_id
                    && state.runs[index].parent_run_id.is_some()
                {
                    let root_id = state.runs[index].root_id.clone();
                    let run_id = state.runs[index].run_id.clone();
                    let event = match kind {
                        ReportKind::Ready => "ready",
                        ReportKind::Result => "result",
                        _ => "needs_input",
                    };
                    let plan_revision = state.runs[index]
                        .work_plan
                        .as_ref()
                        .map(|p| p.plan_revision.as_str());
                    let text = serde_json::json!({"event":event,"run_id":run_id,"receipt_message_id":message_id,"plan_revision":plan_revision}).to_string();
                    append(
                        state,
                        ActorRef::Dispatcher,
                        &root_id,
                        &format!("manage-{run_id}-{message_id}"),
                        MessageKind::Observation,
                        &text,
                        None,
                        None,
                        false,
                        None,
                        None,
                    )?;
                }
            }
            result
        }
        OrchestrationAction::MessageSend {
            message_id,
            to_run_id,
            kind,
            text,
            in_reply_to,
        } => {
            let target = run_index(state, to_run_id)?;
            let (recipient, escalated) = match actor {
                Actor::Operator(_) => {
                    if !matches!(
                        kind,
                        MessageKind::Instruction | MessageKind::Answer | MessageKind::CancelRequest
                    ) {
                        return Err(error(
                            "actor_forbidden",
                            "Operators may send Instruction, Answer, or CancelRequest",
                        ));
                    }
                    (to_run_id.clone(), None)
                }
                Actor::Agent(_) => {
                    let (_, index) = agent(actor, caller)?;
                    let own = &state.runs[index].run_id;
                    match kind {
                        MessageKind::Answer => {
                            super::management_target(
                                state,
                                actor,
                                caller,
                                &state.runs[index].session_id,
                                to_run_id,
                            )?;
                            (to_run_id.clone(), None)
                        }
                        MessageKind::Instruction | MessageKind::CancelRequest => {
                            if !is_ancestor(state, own, to_run_id) {
                                return Err(error(
                                    "not_in_subtree",
                                    "Agent instructions and cancellation requests require a strict descendant",
                                ));
                            }
                            if state.runs[target].stage == RunStage::Closed {
                                return Err(error("invalid_stage", "Cannot instruct a closed run"));
                            }
                            (to_run_id.clone(), None)
                        }
                        MessageKind::Observation => {
                            if !is_ancestor(state, to_run_id, own) {
                                return Err(error(
                                    "not_ancestor",
                                    "Observations require a strict ancestor",
                                ));
                            }
                            upward(state, index, Some(to_run_id), false)?
                        }
                        _ => {
                            return Err(error(
                                "actor_forbidden",
                                "Agent messages are downward instructions/cancellation or upward observations",
                            ));
                        }
                    }
                }
            };
            let from = actor_ref(actor, caller, state);
            // An observation retry does not change its original delivery location if ancestors later close.
            if let Some(existing) = duplicate(state, &from, message_id) {
                if existing.kind != *kind
                    || existing.text != *text
                    || existing.in_reply_to != *in_reply_to
                    || existing.report.is_some()
                    || existing
                        .escalated_from
                        .as_deref()
                        .unwrap_or(&existing.to_run_id)
                        != to_run_id
                    || existing.from_subagent_id.is_some()
                {
                    return Err(error(
                        "message_id_conflict",
                        "The sender already used this message id for a different message",
                    ));
                }
                return Ok(Some(message_result(existing, true)));
            }
            let answer_provenance = if *kind == MessageKind::Answer {
                if in_reply_to.as_deref().is_none_or(|id| id.trim().is_empty()) {
                    return Err(error(
                        "invalid_message",
                        "Answers require an explicit nonempty question message id",
                    ));
                }
                let run = &state.runs[target];
                if run.stage == RunStage::Closed
                    || run.last_report.as_ref().is_none_or(|report| {
                        report.kind != ReportKind::NeedsInput
                            || Some(report.message_id.as_str()) != in_reply_to.as_deref()
                    })
                {
                    return Err(error(
                        "question_not_current",
                        "The answer must link to the open run's current main NeedsInput report",
                    ));
                }
                Some(super::management_target(
                    state,
                    actor,
                    caller,
                    &state.runs[target].session_id,
                    to_run_id,
                )?.1)
            } else {
                if in_reply_to.is_some() {
                    return Err(error(
                        "invalid_message",
                        "Only Answers may link to a question message id",
                    ));
                }
                None
            };
            let result = append(
                state, from, &recipient, message_id, *kind, text, in_reply_to.clone(), None, false, None, escalated,
            )?;
            if let Some(provenance) = answer_provenance {
                let annotation = super::decision_annotation(
                    actor_ref(actor, caller, state),
                    &provenance,
                    &format!("Answer delivered as message {message_id}"),
                );
                state.runs[target].annotations.push(annotation);
            }
            result
        }
        OrchestrationAction::Annotate { run_id, text } => {
            text_bound(text)?;
            let target = run_index(state, run_id)?;
            if let Actor::Agent(_) = actor {
                let (_, index) = agent(actor, caller)?;
                let own = &state.runs[index].run_id;
                if own != run_id && !is_ancestor(state, own, run_id) {
                    return Err(error(
                        "not_in_subtree",
                        "Annotations require the caller's run or descendant",
                    ));
                }
            }
            let annotation = Annotation {
                by: actor_ref(actor, caller, state),
                text: text.clone(),
                at: now(),
            };
            state.runs[target].updated_at = annotation.at.clone();
            state.runs[target].annotations.push(annotation);
            OrchestrationActionResult::Done
        }
        OrchestrationAction::InboxPull { after_seq, limit } => {
            let (agent, index) = agent(actor, caller)?;
            main_session(agent, &state.runs[index])?;
            let own = &state.runs[index].run_id;
            // Append order is per-recipient sequence order; stop before touching the next page.
            let mut messages = Vec::new();
            let mut read_through_seq = *after_seq;
            for message in state
                .messages
                .iter_mut()
                .filter(|message| {
                    message.to_run_id == *own
                        && message.seq > *after_seq
                        && message.kind != MessageKind::SubagentControl
                })
                .take((*limit).min(100) as usize)
            {
                if matches!(message.stage, DeliveryStage::Stored | DeliveryStage::Woken) {
                    message.stage = DeliveryStage::Read;
                }
                read_through_seq = message.seq;
                messages.push(message.clone());
            }
            OrchestrationActionResult::Inbox {
                messages,
                read_through_seq,
            }
        }
        OrchestrationAction::InboxWoken {
            through_seq,
            omp_session_id,
        } => {
            let (agent, index) = agent(actor, caller)?;
            main_session(agent, &state.runs[index])?;
            if agent.omp_session_id.as_deref() != Some(omp_session_id) {
                return Err(error(
                    "session_mismatch",
                    "Wake receipt must name the caller's bound session",
                ));
            }
            let own = &state.runs[index].run_id;
            for message in state.messages.iter_mut().filter(|message| {
                message.to_run_id == *own
                    && message.seq <= *through_seq
                    && message.kind != MessageKind::SubagentControl
            }) {
                if matches!(message.stage, DeliveryStage::Stored | DeliveryStage::Woken) {
                    message.stage = DeliveryStage::Woken;
                    message.woken_omp_session = Some(omp_session_id.clone());
                }
            }
            OrchestrationActionResult::Done
        }
        OrchestrationAction::InboxAck { through_seq } => {
            let (agent, index) = agent(actor, caller)?;
            main_session(agent, &state.runs[index])?;
            let own = &state.runs[index].run_id;
            if state.messages.iter().any(|message| {
                message.to_run_id == *own
                    && message.seq <= *through_seq
                    && message.kind != MessageKind::SubagentControl
                    && matches!(message.stage, DeliveryStage::Stored | DeliveryStage::Woken)
            }) {
                return Err(error(
                    "ack_before_read",
                    "Every message through the acknowledged sequence must first be read",
                ));
            }
            let at = now();
            for message in state.messages.iter_mut().filter(|message| {
                message.to_run_id == *own
                    && message.seq <= *through_seq
                    && message.kind != MessageKind::SubagentControl
            }) {
                if message.stage != DeliveryStage::Acked {
                    message.stage = DeliveryStage::Acked;
                    message.acked_at = Some(at.clone());
                }
            }
            OrchestrationActionResult::Done
        }
        OrchestrationAction::SubagentUpdate {
            subagent_id,
            parent_subagent_id,
            role,
            label,
            status,
            summary,
        } => {
            let (agent, index) = agent(actor, caller)?;
            if subagent_id.is_empty() {
                return Err(error("invalid_subagent", "Subagent id is required"));
            }
            text_bound(label)?;
            if let Some(role) = role {
                text_bound(role)?;
            }
            if let Some(summary) = summary {
                text_bound(summary)?;
            }
            let child_binding = match agent.agent_kind {
                Some(AgentKind::Main) => {
                    main_session(agent, &state.runs[index])?;
                    None
                }
                Some(AgentKind::Subagent) if agent.subagent_id.as_deref() == Some(subagent_id) => {
                    Some(authenticated_child_session(agent, &state.runs[index])?)
                }
                _ => {
                    return Err(error(
                        "actor_forbidden",
                        "A subagent may update only its own telemetry",
                    ));
                }
            };
            let own = &state.runs[index].run_id;
            let mut parent = parent_subagent_id.as_deref();
            let mut depth = 0;
            while let Some(parent_id) = parent {
                if parent_id == subagent_id || depth >= state.subagents.len() {
                    return Err(error(
                        "invalid_subagent_parent",
                        "Subagent nesting must not contain a self-edge or cycle",
                    ));
                }
                let ancestor = state
                    .subagents
                    .iter()
                    .find(|entry| entry.run_id == *own && entry.subagent_id == parent_id)
                    .ok_or_else(|| {
                        error(
                            "invalid_subagent_parent",
                            "Parent subagent must exist in the same run",
                        )
                    })?;
                parent = ancestor.parent_subagent_id.as_deref();
                depth += 1;
            }
            let existing = state
                .subagents
                .iter()
                .position(|entry| entry.run_id == *own && entry.subagent_id == *subagent_id);
            // Explicit child progress takes precedence over an assignment/reminder
            // carried by lifecycle telemetry, including when telemetry arrives later.
            let effective_summary =
                if summary.is_some() || existing.is_none() {
                    state.messages.iter().rev().find_map(|message| {
                    if message.stale || message.from_subagent_id.as_deref() != Some(subagent_id)
                        || !matches!(&message.from, ActorRef::Run { run_id } if run_id == own) {
                        return None;
                    }
                    message.report.as_ref().filter(|report|
                        matches!(report.kind, ReportKind::Progress | ReportKind::NeedsInput))
                        .map(|report| report.summary.as_str())
                }).or(summary.as_deref())
                } else {
                    None
                };
            let at = now();
            if let Some(index) = existing {
                let entry = &mut state.subagents[index];
                if let Some(session) = child_binding {
                    entry.bound_omp_session = Some(session.to_owned());
                }
                entry.parent_subagent_id = parent_subagent_id.clone();
                entry.role = role.clone();
                entry.label = label.clone();
                entry.status = *status;
                if let Some(summary) = effective_summary {
                    if entry.summary.as_deref() != Some(summary) {
                        entry.summary = Some(summary.to_owned());
                    }
                }
                entry.updated_at = at;
            } else {
                state.subagents.push(Subagent {
                    run_id: own.clone(),
                    subagent_id: subagent_id.clone(),
                    parent_subagent_id: parent_subagent_id.clone(),
                    bound_omp_session: child_binding.map(str::to_owned),
                    role: role.clone(),
                    label: label.clone(),
                    status: *status,
                    summary: effective_summary.map(str::to_owned),
                    last_control: None,
                    updated_at: at,
                });
            }
            OrchestrationActionResult::Done
        }
        OrchestrationAction::SubagentControl {
            run_id,
            subagent_id,
            op,
        } => {
            run_index(state, run_id)?;
            if let Actor::Agent(_) = actor {
                let (_, index) = agent(actor, caller)?;
                if !is_ancestor(state, &state.runs[index].run_id, run_id) {
                    return Err(error(
                        "not_ancestor",
                        "Subagent controls require an ancestor of the owning run",
                    ));
                }
            }
            if let SubagentOp::Send { text } = op {
                text_bound(text)?;
            }
            let subagent = state
                .subagents
                .iter()
                .position(|entry| entry.run_id == *run_id && entry.subagent_id == *subagent_id)
                .ok_or_else(|| {
                    error(
                        "subagent_not_found",
                        "Subagent does not exist in the specified run",
                    )
                })?;
            if state.subagents[subagent].status != SubagentStatus::Running {
                return Err(error(
                    "invalid_stage",
                    "Only running subagents can receive controls",
                ));
            }
            let text = serde_json::to_string(&ControlEnvelope {
                subagent_id: subagent_id.clone(),
                op: op.clone(),
            })
            .map_err(|failure| error("invalid_subagent_control", failure.to_string()))?;
            let from = actor_ref(actor, caller, state);
            let result = append(
                state,
                from,
                run_id,
                &uuid::Uuid::new_v4().to_string(),
                MessageKind::SubagentControl,
                &text,
                None,
                None,
                false,
                None,
                None,
            )?;
            if let OrchestrationActionResult::Message { seq, .. } = &result {
                state.subagents[subagent].last_control = Some(SubagentControlState {
                    seq: *seq,
                    op: op.clone(),
                    stage: ControlStage::Stored,
                    error: None,
                    at: now(),
                });
            }
            result
        }
        OrchestrationAction::SubagentControlDone {
            seq,
            applied,
            error: control_error,
        } => {
            let (agent, index) = agent(actor, caller)?;
            if let Some(failure) = control_error {
                text_bound(failure)?;
            }
            let own = &state.runs[index].run_id;
            let message_index = state
                .messages
                .iter()
                .position(|message| {
                    message.to_run_id == *own
                        && message.seq == *seq
                        && message.kind == MessageKind::SubagentControl
                })
                .ok_or_else(|| {
                    error(
                        "subagent_control_not_found",
                        "Sequence is not a control in this run's inbox",
                    )
                })?;
            let envelope: ControlEnvelope =
                serde_json::from_str(&state.messages[message_index].text)
                    .map_err(|failure| error("invalid_subagent_control", failure.to_string()))?;
            match agent.agent_kind {
                Some(AgentKind::Main) => main_session(agent, &state.runs[index])?,
                Some(AgentKind::Subagent)
                    if agent.subagent_id.as_deref() == Some(&envelope.subagent_id) => {}
                _ => {
                    return Err(error(
                        "actor_forbidden",
                        "Only the owning main context or targeted subagent can complete a control",
                    ));
                }
            }
            let subagent_index = state
                .subagents
                .iter()
                .position(|entry| entry.run_id == *own && entry.subagent_id == envelope.subagent_id)
                .ok_or_else(|| {
                    error(
                        "subagent_not_found",
                        "The targeted subagent no longer exists",
                    )
                })?;
            let stage = if *applied {
                ControlStage::Applied
            } else {
                ControlStage::Failed
            };
            if let Some(last) = &state.subagents[subagent_index].last_control {
                if last.seq == *seq && last.stage != ControlStage::Stored {
                    if last.stage != stage || last.error != *control_error {
                        return Err(error(
                            "subagent_control_conflict",
                            "Control completion already has a different receipt",
                        ));
                    }
                    return Ok(Some(OrchestrationActionResult::Done));
                }
            }
            let at = now();
            let message = &mut state.messages[message_index];
            message.stage = DeliveryStage::Acked;
            if message.acked_at.is_none() {
                message.acked_at = Some(at.clone());
            }
            let entry = &mut state.subagents[subagent_index];
            // Older completions must not overwrite the receipt for a newer control.
            if entry
                .last_control
                .as_ref()
                .is_none_or(|last| last.seq <= *seq)
            {
                entry.last_control = Some(SubagentControlState {
                    seq: *seq,
                    op: envelope.op,
                    stage,
                    error: control_error.clone(),
                    at: at.clone(),
                });
            }
            entry.updated_at = at;
            OrchestrationActionResult::Done
        }
        _ => return Ok(None),
    };
    Ok(Some(result))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn native_process() -> NativeProcessIdentity {
        NativeProcessIdentity {
            pid: 41,
            start_ticks: 73,
            kernel_boot_id: Some("test-boot".into()),
        }
    }

    fn run(id: &str, parent: Option<&str>, stage: RunStage) -> Run {
        Run {
            session_id: "herdr-session".into(),
            prepare_brief: String::new(),
            run_id: id.into(),
            kind: if parent.is_some() {
                RunKind::Worker
            } else {
                RunKind::Supervisor
            },
            label: id.into(),
            root_id: "root".into(),
            parent_run_id: parent.map(str::to_owned),
            task_id: parent.map(|_| format!("task-{id}")),
            attempt: 1,
            task_revision_at_propose: None,
            stage,
            close_reason: None,
            dispatch: None,
            target: None,
            setup: None,
            prepare_plan: None,
            init_receipt: None,
            work_plan: None,
            grants: Vec::new(),
            last_report: None,
            result: None,
            annotations: Vec::new(),
            location: None,
            bound_omp_session: Some(format!("omp-{id}")),
            bound_omp_process: Some(native_process()),
            launch_shell_identity: None,
            retirement: None,
            supersedes_run_id: None,
            created_at: now(),
            updated_at: now(),
        }
    }

    fn state() -> OrchestrationState {
        OrchestrationState {
            schema: 1,
            revision: 0,
            runs: vec![
                run("root", None, RunStage::Active),
                run("worker", Some("root"), RunStage::Initializing),
                run("grandchild", Some("worker"), RunStage::Working),
                run("sibling", Some("root"), RunStage::Working),
            ],
            messages: Vec::new(),
            subagents: Vec::new(),
            task_intents: Vec::new(),
            assignment_intents: Vec::new(),
        }
    }

    fn main_actor(run_id: &str) -> Actor {
        Actor::Agent(AgentCaller {
            endpoint_identity: "endpoint".into(),
            session_id: "herdr-session".into(),
            workspace_id: "space".into(),
            tab_id: "tab".into(),
            pane_id: "pane".into(),
            boot_id: None,
            terminal_id: None,
            native_session_id: None,
            env_run: Some((run_id.into(), 1)),
            omp_session_id: Some(format!("omp-{run_id}")),
            main_omp_session_id: Some(format!("omp-{run_id}")),
            agent_kind: Some(AgentKind::Main),
            actual_agent_kind: Some("omp".into()),
            subagent_id: None,
            process: Some(native_process()),
        })
    }

    fn subagent_actor(run_id: &str, subagent_id: &str) -> Actor {
        let Actor::Agent(mut caller) = main_actor(run_id) else {
            unreachable!()
        };
        caller.agent_kind = Some(AgentKind::Subagent);
        caller.subagent_id = Some(subagent_id.into());
        caller.omp_session_id = Some(format!("sub-session-{subagent_id}"));
        Actor::Agent(caller)
    }

    fn report(id: &str, kind: ReportKind, destination: Option<&str>) -> OrchestrationAction {
        OrchestrationAction::Report {
            message_id: id.into(),
            kind,
            outcome: (kind == ReportKind::Result).then_some(ReportOutcome::Succeeded),
            summary: format!("summary-{id}"),
            plan: (kind == ReportKind::Ready).then(|| "Inspect, change, then verify.".into()),
            to_run_id: destination.map(str::to_owned),
        }
    }

    fn send(id: &str, kind: MessageKind, question: Option<&str>) -> OrchestrationAction {
        OrchestrationAction::MessageSend {
            message_id: id.into(),
            to_run_id: "worker".into(),
            kind,
            text: "Proceed with the approved choice".into(),
            in_reply_to: question.map(str::to_owned),
        }
    }

    fn assert_rejected_without_mutation(
        state: &mut OrchestrationState,
        action: &OrchestrationAction,
        code: &str,
    ) {
        let before = serde_json::to_value(&*state).unwrap();
        let operator = Actor::Operator(OperatorOrigin::Browser);
        assert_eq!(
            apply(state, &operator, None, false, action).unwrap_err().code,
            code
        );
        assert_eq!(serde_json::to_value(&*state).unwrap(), before);
    }

    fn mutation(
        state: &mut OrchestrationState,
        actor: &Actor,
        index: usize,
        action: &OrchestrationAction,
    ) -> OrchestrationActionResult {
        apply(state, actor, Some(index), false, action)
            .unwrap()
            .unwrap()
    }

    fn assert_error(
        state: &mut OrchestrationState,
        actor: &Actor,
        index: usize,
        action: &OrchestrationAction,
        code: &str,
    ) {
        assert_eq!(
            apply(state, actor, Some(index), false, action)
                .unwrap_err()
                .code,
            code
        );
    }

    fn update(id: &str, parent: Option<&str>) -> OrchestrationAction {
        OrchestrationAction::SubagentUpdate {
            subagent_id: id.into(),
            parent_subagent_id: parent.map(str::to_owned),
            role: Some("reviewer".into()),
            label: id.into(),
            status: SubagentStatus::Running,
            summary: None,
        }
    }

    #[test]
    fn answers_require_explicit_links_and_other_messages_forbid_links() {
        let mut state = state();
        mutation(
            &mut state,
            &main_actor("worker"),
            1,
            &report("question", ReportKind::NeedsInput, None),
        );
        for question in [None, Some(""), Some(" \t")] {
            assert_rejected_without_mutation(
                &mut state,
                &send("unlinked", MessageKind::Answer, question),
                "invalid_message",
            );
        }
        for kind in [MessageKind::Instruction, MessageKind::CancelRequest] {
            for question in [Some("question"), Some("")] {
                assert_rejected_without_mutation(
                    &mut state,
                    &send("not-answer", kind, question),
                    "invalid_message",
                );
            }
        }
    }

    #[test]
    fn answers_reject_old_questions_but_committed_retries_survive_new_questions() {
        let mut state = state();
        let worker = main_actor("worker");
        let operator = Actor::Operator(OperatorOrigin::Browser);
        mutation(&mut state, &worker, 1, &report("q1", ReportKind::NeedsInput, None));
        let answer = send("a1", MessageKind::Answer, Some("q1"));
        let first = apply(&mut state, &operator, None, false, &answer).unwrap().unwrap();
        assert!(matches!(first, OrchestrationActionResult::Message { duplicate: false, seq: 1, .. }));
        assert_eq!(state.messages.last().unwrap().in_reply_to.as_deref(), Some("q1"));
        mutation(&mut state, &worker, 1, &report("q2", ReportKind::NeedsInput, None));
        assert_rejected_without_mutation(
            &mut state,
            &send("late", MessageKind::Answer, Some("q1")),
            "question_not_current",
        );
        let before = serde_json::to_value(&state).unwrap();
        let retry = apply(&mut state, &operator, None, false, &answer).unwrap().unwrap();
        assert!(matches!(retry, OrchestrationActionResult::Message { duplicate: true, seq: 1, .. }));
        assert_eq!(serde_json::to_value(&state).unwrap(), before);
        for question in [Some("q2"), None, Some("")] {
            assert_rejected_without_mutation(
                &mut state,
                &send("a1", MessageKind::Answer, question),
                "message_id_conflict",
            );
        }
        let mut changed_text = answer;
        if let OrchestrationAction::MessageSend { text, .. } = &mut changed_text {
            *text = "A different choice".into();
        }
        assert_rejected_without_mutation(&mut state, &changed_text, "message_id_conflict");
        apply(
            &mut state,
            &operator,
            None,
            false,
            &send("a2", MessageKind::Answer, Some("q2")),
        ).unwrap();
        assert_eq!(state.messages.last().unwrap().in_reply_to.as_deref(), Some("q2"));
        assert_eq!(state.runs[1].last_report.as_ref().unwrap().message_id, "q2");
    }

    #[test]
    fn answers_require_an_open_current_main_needs_input_report() {
        let mut state = state();
        let worker = main_actor("worker");
        let answer = send("answer", MessageKind::Answer, Some("question"));
        assert_rejected_without_mutation(&mut state, &answer, "question_not_current");
        mutation(&mut state, &worker, 1, &report("question", ReportKind::Progress, None));
        assert_rejected_without_mutation(&mut state, &answer, "question_not_current");
        mutation(&mut state, &worker, 1, &report("question-ni", ReportKind::NeedsInput, None));
        state.runs[1].stage = RunStage::Closed;
        assert_rejected_without_mutation(
            &mut state,
            &send("answer", MessageKind::Answer, Some("question-ni")),
            "question_not_current",
        );
    }

    #[test]
    fn subagent_and_stale_questions_cannot_replace_the_main_question() {
        let mut state = state();
        mutation(
            &mut state,
            &main_actor("worker"),
            1,
            &report("main-question", ReportKind::NeedsInput, None),
        );
        mutation(
            &mut state,
            &subagent_actor("worker", "child"),
            1,
            &report("child-question", ReportKind::NeedsInput, Some("worker")),
        );
        apply(
            &mut state,
            &main_actor("worker"),
            Some(1),
            true,
            &report("stale-question", ReportKind::NeedsInput, None),
        ).unwrap();
        for question in ["child-question", "stale-question"] {
            assert_rejected_without_mutation(
                &mut state,
                &send("answer", MessageKind::Answer, Some(question)),
                "question_not_current",
            );
        }
        apply(
            &mut state,
            &Actor::Operator(OperatorOrigin::Browser),
            None,
            false,
            &send("answer", MessageKind::Answer, Some("main-question")),
        ).unwrap();
        assert_eq!(state.runs[1].last_report.as_ref().unwrap().message_id, "main-question");
    }

    #[test]
    fn dedupe_rejects_changed_payload_and_preserves_original_report_after_stage_change() {
        let mut state = state();
        let actor = main_actor("worker");
        let ready = report("ready", ReportKind::Ready, None);
        mutation(&mut state, &actor, 1, &ready);
        state.runs[1].stage = RunStage::Working;
        let receipt = mutation(&mut state, &actor, 1, &ready);
        assert!(matches!(
            receipt,
            OrchestrationActionResult::Message {
                duplicate: true,
                seq: 1,
                ..
            }
        ));
        assert_eq!(state.messages.len(), 1);
        assert_eq!(state.runs[1].stage, RunStage::Working);
        let OrchestrationAction::Report {
            mut summary,
            message_id,
            kind,
            outcome,
            plan,
            to_run_id,
        } = ready
        else {
            unreachable!()
        };
        summary.push_str(" altered");
        assert_error(
            &mut state,
            &actor,
            1,
            &OrchestrationAction::Report {
                message_id,
                kind,
                outcome,
                summary,
                plan,
                to_run_id,
            },
            "message_id_conflict",
        );
        assert_eq!(
            state.runs[1].init_receipt.as_ref().unwrap().summary,
            "summary-ready"
        );
    }

    #[test]
    fn append_keys_by_sender_and_sequences_by_recipient() {
        let mut state = state();
        let first = append(
            &mut state,
            ActorRef::Operator,
            "worker",
            "same",
            MessageKind::Instruction,
            "hello",
            None,
            None,
            false,
            None,
            None,
        )
        .unwrap();
        assert!(matches!(
            first,
            OrchestrationActionResult::Message {
                seq: 1,
                duplicate: false,
                ..
            }
        ));
        let duplicate = append(
            &mut state,
            ActorRef::Operator,
            "worker",
            "same",
            MessageKind::Instruction,
            "hello",
            None,
            None,
            false,
            None,
            None,
        )
        .unwrap();
        assert!(matches!(
            duplicate,
            OrchestrationActionResult::Message {
                seq: 1,
                duplicate: true,
                ..
            }
        ));
        append(
            &mut state,
            ActorRef::Run {
                run_id: "root".into(),
            },
            "worker",
            "same",
            MessageKind::Instruction,
            "hello",
            None,
            None,
            false,
            None,
            None,
        )
        .unwrap();
        append(
            &mut state,
            ActorRef::Operator,
            "sibling",
            "other",
            MessageKind::Instruction,
            "hello",
            None,
            None,
            false,
            None,
            None,
        )
        .unwrap();
        assert_eq!(
            state
                .messages
                .iter()
                .map(|message| message.seq)
                .collect::<Vec<_>>(),
            vec![1, 2, 1]
        );
        assert_eq!(
            append(
                &mut state,
                ActorRef::Operator,
                "sibling",
                "same",
                MessageKind::Instruction,
                "hello",
                None,
                None,
                false,
                None,
                None
            )
            .unwrap_err()
            .code,
            "message_id_conflict"
        );
        assert_eq!(
            append(
                &mut state,
                ActorRef::Operator,
                "worker",
                "same",
                MessageKind::CancelRequest,
                "hello",
                None,
                None,
                false,
                None,
                None
            )
            .unwrap_err()
            .code,
            "message_id_conflict"
        );
    }

    #[test]
    fn append_bounds_utf8_bytes_and_rejects_sequence_overflow() {
        let mut state = state();
        let oversized = "é".repeat(MAX_TEXT / 2 + 1);
        assert_eq!(
            append(
                &mut state,
                ActorRef::Operator,
                "worker",
                "large",
                MessageKind::Instruction,
                &oversized,
                None,
                None,
                false,
                None,
                None
            )
            .unwrap_err()
            .code,
            "message_too_large"
        );
        append(
            &mut state,
            ActorRef::Operator,
            "worker",
            "first",
            MessageKind::Instruction,
            "hello",
            None,
            None,
            false,
            None,
            None,
        )
        .unwrap();
        state.messages[0].seq = u64::MAX;
        assert_eq!(
            append(
                &mut state,
                ActorRef::Operator,
                "worker",
                "second",
                MessageKind::Instruction,
                "hello",
                None,
                None,
                false,
                None,
                None
            )
            .unwrap_err()
            .code,
            "message_sequence_exhausted"
        );
        assert_eq!(state.messages.len(), 1);
    }

    #[test]
    fn ready_requires_bound_main_session_and_correct_stage() {
        let mut state = state();
        let ready = report("ready", ReportKind::Ready, None);
        assert_error(
            &mut state,
            &subagent_actor("worker", "child"),
            1,
            &ready,
            "report_requires_main",
        );
        let Actor::Agent(mut wrong) = main_actor("worker") else {
            unreachable!()
        };
        wrong.omp_session_id = Some("other-session".into());
        assert_error(
            &mut state,
            &Actor::Agent(wrong),
            1,
            &ready,
            "session_mismatch",
        );
        state.runs[1].bound_omp_session = None;
        assert_error(
            &mut state,
            &main_actor("worker"),
            1,
            &ready,
            "session_mismatch",
        );
        state.runs[1].bound_omp_session = Some("omp-worker".into());
        state.runs[1].stage = RunStage::Working;
        assert_error(
            &mut state,
            &main_actor("worker"),
            1,
            &ready,
            "invalid_stage",
        );
        assert!(state.messages.is_empty());
    }

    #[test]
    fn ready_hashes_plan_replans_and_result_preserves_initialization() {
        let mut state = state();
        let actor = main_actor("worker");
        mutation(
            &mut state,
            &actor,
            1,
            &report("ready", ReportKind::Ready, None),
        );
        let original_hash = state.runs[1]
            .work_plan
            .as_ref()
            .unwrap()
            .plan_revision
            .clone();
        assert_eq!(
            original_hash,
            format!(
                "{:x}",
                Sha256::digest(serde_json::to_vec("Inspect, change, then verify.").unwrap())
            )
        );
        let mut revised = report("revised", ReportKind::Ready, None);
        if let OrchestrationAction::Report { plan, .. } = &mut revised {
            *plan = Some("A different reviewed plan.".into());
        }
        mutation(&mut state, &actor, 1, &revised);
        assert_ne!(
            state.runs[1].work_plan.as_ref().unwrap().plan_revision,
            original_hash
        );
        state.runs[1].stage = RunStage::Working;
        mutation(
            &mut state,
            &actor,
            1,
            &report("result", ReportKind::Result, None),
        );
        assert_eq!(state.runs[1].stage, RunStage::Reported);
        assert_eq!(
            state.runs[1].init_receipt.as_ref().unwrap().message_id,
            "revised"
        );
        assert_eq!(state.runs[1].result.as_ref().unwrap().message_id, "result");
    }

    #[test]
    fn upward_reports_reject_self_siblings_descendants_and_escalate_closed_parent() {
        let mut state = state();
        let actor = main_actor("grandchild");
        for destination in ["grandchild", "sibling"] {
            assert_error(
                &mut state,
                &actor,
                2,
                &report("invalid", ReportKind::Progress, Some(destination)),
                "not_ancestor",
            );
        }
        assert_error(
            &mut state,
            &main_actor("worker"),
            1,
            &report("downward", ReportKind::Progress, Some("grandchild")),
            "not_ancestor",
        );
        mutation(
            &mut state,
            &actor,
            2,
            &report("root-directed", ReportKind::Progress, Some("root")),
        );
        state.runs[1].stage = RunStage::Closed;
        mutation(
            &mut state,
            &actor,
            2,
            &report("escalated", ReportKind::NeedsInput, None),
        );
        assert_eq!(state.messages[1].to_run_id, "root");
        assert_eq!(state.messages[1].escalated_from.as_deref(), Some("worker"));
        state.runs[0].stage = RunStage::Closed;
        let duplicate = mutation(
            &mut state,
            &actor,
            2,
            &report("escalated", ReportKind::NeedsInput, None),
        );
        assert!(matches!(
            duplicate,
            OrchestrationActionResult::Message {
                duplicate: true,
                seq: 2,
                ..
            }
        ));
    }

    #[test]
    fn stale_report_is_evidence_without_applying_outcome() {
        let mut state = state();
        state.runs[1].stage = RunStage::Closed;
        state.runs[1].close_reason = Some(CloseReason::Superseded);
        state.runs[1].bound_omp_session = Some("replacement-session".into());
        let before = serde_json::to_value(&state.runs[1]).unwrap();
        let result = apply(
            &mut state,
            &main_actor("worker"),
            Some(1),
            true,
            &report("stale", ReportKind::Result, None),
        )
        .unwrap()
        .unwrap();
        assert!(matches!(
            result,
            OrchestrationActionResult::Message { stale: true, .. }
        ));
        assert_eq!(serde_json::to_value(&state.runs[1]).unwrap(), before);
        assert_eq!(state.messages[0].to_run_id, "root");
        assert!(state.messages[0].stale);
        assert_eq!(
            state.messages[0].report.as_ref().unwrap().outcome,
            Some(ReportOutcome::Succeeded)
        );
    }

    #[test]
    fn root_reports_to_operator_visible_self_inbox() {
        let mut state = state();
        mutation(
            &mut state,
            &main_actor("root"),
            0,
            &report("need", ReportKind::NeedsInput, None),
        );
        mutation(
            &mut state,
            &main_actor("root"),
            0,
            &report("result", ReportKind::Result, None),
        );
        assert!(
            state
                .messages
                .iter()
                .all(|message| message.to_run_id == "root")
        );
        assert_eq!(state.runs[0].stage, RunStage::Active);
        assert_eq!(state.runs[0].result.as_ref().unwrap().message_id, "result");
    }

    #[test]
    fn subagent_progress_is_tagged_without_overwriting_main_receipt() {
        let mut state = state();
        mutation(
            &mut state,
            &main_actor("worker"),
            1,
            &report("main", ReportKind::Progress, None),
        );
        mutation(
            &mut state,
            &subagent_actor("worker", "child"),
            1,
            &report("child", ReportKind::NeedsInput, None),
        );
        assert_eq!(state.messages[1].from_subagent_id.as_deref(), Some("child"));
        assert_eq!(
            state.runs[1].last_report.as_ref().unwrap().message_id,
            "main"
        );
    }

    #[test]
    fn internal_subagent_reports_to_own_main_are_evidence_only_and_require_real_child_identity() {
        let mut state = state();
        mutation(
            &mut state,
            &main_actor("worker"),
            1,
            &report("main", ReportKind::Progress, None),
        );
        let main_before = serde_json::to_value(&state.runs[1]).unwrap();
        let child = subagent_actor("worker", "child");
        for (id, kind) in [
            ("child-progress", ReportKind::Progress),
            ("child-question", ReportKind::NeedsInput),
        ] {
            let action = report(id, kind, Some("worker"));
            let result = mutation(&mut state, &child, 1, &action);
            assert!(
                matches!(result, OrchestrationActionResult::Message { ref to_run_id, stale: false, .. } if to_run_id == "worker")
            );
            let message = state.messages.last().unwrap();
            assert_eq!(message.from_subagent_id.as_deref(), Some("child"));
            assert_eq!(message.to_run_id, "worker");
            assert_eq!(message.stage, DeliveryStage::Stored);
            assert_eq!(serde_json::to_value(&state.runs[1]).unwrap(), main_before);
        }
        assert_error(
            &mut state,
            &main_actor("worker"),
            1,
            &report("main-self", ReportKind::Progress, Some("worker")),
            "not_ancestor",
        );
        assert_error(
            &mut state,
            &child,
            1,
            &report("child-ready", ReportKind::Ready, Some("worker")),
            "report_requires_main",
        );
        assert_error(
            &mut state,
            &child,
            1,
            &report("child-result", ReportKind::Result, Some("worker")),
            "report_requires_main",
        );
        assert_error(
            &mut state,
            &child,
            1,
            &report("child-sibling", ReportKind::Progress, Some("sibling")),
            "not_ancestor",
        );
        for invalid_identity in ["main-proof", "native-child", "subagent-id"] {
            let Actor::Agent(mut caller) = child.clone() else {
                unreachable!()
            };
            let expected = match invalid_identity {
                "main-proof" => {
                    caller.main_omp_session_id = Some("other-main".into());
                    "session_mismatch"
                }
                "native-child" => {
                    caller.omp_session_id = caller.main_omp_session_id.clone();
                    "session_mismatch"
                }
                _ => {
                    caller.subagent_id = None;
                    "actor_forbidden"
                }
            };
            assert_error(
                &mut state,
                &Actor::Agent(caller),
                1,
                &report(
                    &format!("invalid-{invalid_identity}"),
                    ReportKind::Progress,
                    Some("worker"),
                ),
                expected,
            );
        }
        assert_eq!(state.messages.len(), 3);
        assert_eq!(serde_json::to_value(&state.runs[1]).unwrap(), main_before);
    }

    #[test]
    fn wake_never_reads_and_ack_is_atomic_across_the_whole_prefix() {
        let mut state = state();
        let operator = Actor::Operator(OperatorOrigin::Browser);
        for id in ["first", "second"] {
            apply(
                &mut state,
                &operator,
                None,
                false,
                &OrchestrationAction::MessageSend {
                    message_id: id.into(),
                    to_run_id: "worker".into(),
                    kind: MessageKind::Instruction,
                    text: id.into(),
                    in_reply_to: None,
                },
            )
            .unwrap();
        }
        let actor = main_actor("worker");
        assert_error(
            &mut state,
            &actor,
            1,
            &OrchestrationAction::InboxAck { through_seq: 2 },
            "ack_before_read",
        );
        assert!(
            state
                .messages
                .iter()
                .all(|message| message.stage == DeliveryStage::Stored)
        );
        let wake = OrchestrationAction::InboxWoken {
            through_seq: 2,
            omp_session_id: "omp-worker".into(),
        };
        mutation(&mut state, &actor, 1, &wake);
        mutation(&mut state, &actor, 1, &wake);
        assert!(
            state
                .messages
                .iter()
                .all(|message| message.stage == DeliveryStage::Woken)
        );
        assert_error(
            &mut state,
            &actor,
            1,
            &OrchestrationAction::InboxAck { through_seq: 2 },
            "ack_before_read",
        );
        mutation(
            &mut state,
            &actor,
            1,
            &OrchestrationAction::InboxPull {
                after_seq: 0,
                limit: 1,
            },
        );
        assert_error(
            &mut state,
            &actor,
            1,
            &OrchestrationAction::InboxAck { through_seq: 2 },
            "ack_before_read",
        );
        assert_eq!(state.messages[0].stage, DeliveryStage::Read);
        mutation(
            &mut state,
            &actor,
            1,
            &OrchestrationAction::InboxAck { through_seq: 1 },
        );
        let acked_at = state.messages[0].acked_at.clone();
        mutation(&mut state, &actor, 1, &wake);
        assert_eq!(state.messages[0].stage, DeliveryStage::Acked);
        assert_eq!(state.messages[0].acked_at, acked_at);
        mutation(
            &mut state,
            &actor,
            1,
            &OrchestrationAction::InboxPull {
                after_seq: 1,
                limit: 1,
            },
        );
        mutation(
            &mut state,
            &actor,
            1,
            &OrchestrationAction::InboxAck { through_seq: 2 },
        );
        assert!(
            state
                .messages
                .iter()
                .all(|message| message.stage == DeliveryStage::Acked)
        );
    }

    #[test]
    fn pull_caps_at_one_hundred_and_is_scoped_to_main_inbox() {
        let mut state = state();
        for seq in 0..105 {
            append(
                &mut state,
                ActorRef::Operator,
                "worker",
                &format!("message-{seq}"),
                MessageKind::Instruction,
                "read",
                None,
                None,
                false,
                None,
                None,
            )
            .unwrap();
        }
        let result = mutation(
            &mut state,
            &main_actor("worker"),
            1,
            &OrchestrationAction::InboxPull {
                after_seq: 0,
                limit: u32::MAX,
            },
        );
        let OrchestrationActionResult::Inbox {
            messages,
            read_through_seq,
        } = result
        else {
            panic!("expected inbox");
        };
        assert_eq!(messages.len(), 100);
        assert_eq!(read_through_seq, 100);
        assert_eq!(state.messages[100].stage, DeliveryStage::Stored);
        assert_error(
            &mut state,
            &subagent_actor("worker", "child"),
            1,
            &OrchestrationAction::InboxPull {
                after_seq: 0,
                limit: 1,
            },
            "report_requires_main",
        );
        assert_error(
            &mut state,
            &main_actor("worker"),
            1,
            &OrchestrationAction::InboxWoken {
                through_seq: 1,
                omp_session_id: "other".into(),
            },
            "session_mismatch",
        );
    }

    #[test]
    fn message_and_annotation_authority_is_directional_and_append_only() {
        let mut state = state();
        let actor = main_actor("worker");
        for (target, kind, expected) in [
            ("grandchild", MessageKind::Instruction, None),
            ("grandchild", MessageKind::CancelRequest, None),
            ("root", MessageKind::Observation, None),
            ("sibling", MessageKind::Instruction, Some("not_in_subtree")),
            ("root", MessageKind::Instruction, Some("not_in_subtree")),
            ("worker", MessageKind::Instruction, Some("not_in_subtree")),
            ("root", MessageKind::Answer, Some("actor_forbidden")),
            (
                "grandchild",
                MessageKind::SubagentControl,
                Some("actor_forbidden"),
            ),
        ] {
            let action = OrchestrationAction::MessageSend {
                message_id: format!("{target}-{kind:?}"),
                to_run_id: target.into(),
                kind,
                text: "payload".into(),
                in_reply_to: None,
            };
            if let Some(code) = expected {
                assert_error(&mut state, &actor, 1, &action, code);
            } else {
                mutation(&mut state, &actor, 1, &action);
            }
        }
        assert_eq!(state.runs[2].stage, RunStage::Working);
        for target in ["worker", "grandchild"] {
            mutation(
                &mut state,
                &actor,
                1,
                &OrchestrationAction::Annotate {
                    run_id: target.into(),
                    text: "annotation".into(),
                },
            );
        }
        assert_error(
            &mut state,
            &actor,
            1,
            &OrchestrationAction::Annotate {
                run_id: "root".into(),
                text: "bad".into(),
            },
            "not_in_subtree",
        );
        assert_eq!(state.runs[1].annotations.len(), 1);
        assert_eq!(state.runs[2].annotations.len(), 1);
        let operator = Actor::Operator(OperatorOrigin::Native);
        assert_eq!(
            apply(
                &mut state,
                &operator,
                None,
                false,
                &OrchestrationAction::MessageSend {
                    message_id: "operator-observation".into(),
                    to_run_id: "root".into(),
                    kind: MessageKind::Observation,
                    text: "bad".into(),
                    in_reply_to: None,
                }
            )
            .unwrap_err()
            .code,
            "actor_forbidden"
        );
    }

    #[test]
    fn telemetry_upserts_validates_nesting_and_scopes_subagent_identity() {
        let mut state = state();
        let main = main_actor("worker");
        mutation(&mut state, &main, 1, &update("parent", None));
        mutation(
            &mut state,
            &subagent_actor("worker", "child"),
            1,
            &update("child", Some("parent")),
        );
        mutation(
            &mut state,
            &subagent_actor("worker", "child"),
            1,
            &update("child", Some("parent")),
        );
        assert_eq!(state.subagents.len(), 2);
        assert_error(
            &mut state,
            &main,
            1,
            &update("parent", Some("child")),
            "invalid_subagent_parent",
        );
        assert_error(
            &mut state,
            &main,
            1,
            &update("self", Some("self")),
            "invalid_subagent_parent",
        );
        assert_error(
            &mut state,
            &main,
            1,
            &update("unknown", Some("absent")),
            "invalid_subagent_parent",
        );
        mutation(
            &mut state,
            &main_actor("sibling"),
            3,
            &update("foreign", None),
        );
        assert_error(
            &mut state,
            &main,
            1,
            &update("crossrun", Some("foreign")),
            "invalid_subagent_parent",
        );
        assert_error(
            &mut state,
            &subagent_actor("worker", "child"),
            1,
            &update("parent", None),
            "actor_forbidden",
        );
        assert_eq!(state.subagents[0].parent_subagent_id, None);
    }

    #[test]
    fn only_authenticated_self_updates_bind_native_children() {
        let mut state = state();
        let main = main_actor("worker");
        mutation(&mut state, &main, 1, &update("child", None));
        assert_eq!(state.subagents[0].bound_omp_session, None);

        let child = subagent_actor("worker", "child");
        mutation(&mut state, &child, 1, &update("child", None));
        assert_eq!(
            state.subagents[0].bound_omp_session.as_deref(),
            Some("sub-session-child")
        );
        mutation(&mut state, &main, 1, &update("child", None));
        assert_eq!(
            state.subagents[0].bound_omp_session.as_deref(),
            Some("sub-session-child")
        );

        let Actor::Agent(mut unproven) = child else { unreachable!() };
        unproven.process = None;
        assert_error(
            &mut state,
            &Actor::Agent(unproven.clone()),
            1,
            &update("child", None),
            "session_mismatch",
        );
        unproven.process = Some(NativeProcessIdentity {
            start_ticks: 74,
            ..native_process()
        });
        assert_error(
            &mut state,
            &Actor::Agent(unproven.clone()),
            1,
            &update("child", None),
            "session_mismatch",
        );
        unproven.process = Some(native_process());
        unproven.actual_agent_kind = Some("shell".into());
        assert_error(
            &mut state,
            &Actor::Agent(unproven),
            1,
            &update("child", None),
            "actor_forbidden",
        );
        assert_eq!(
            state.subagents[0].bound_omp_session.as_deref(),
            Some("sub-session-child")
        );
    }

    #[test]
    fn lifecycle_updates_preserve_assignment_and_authoritative_child_progress() {
        let mut state = state();
        let main = main_actor("worker");
        let mut assignment = update("child", None);
        let OrchestrationAction::SubagentUpdate { summary, .. } = &mut assignment else {
            unreachable!()
        };
        *summary = Some("Inspect README and report findings".into());
        mutation(&mut state, &main, 1, &assignment);
        mutation(&mut state, &main, 1, &update("child", None));
        assert_eq!(
            state.subagents[0].summary.as_deref(),
            Some("Inspect README and report findings")
        );
        let main_before = serde_json::to_value(&state.runs[1]).unwrap();
        mutation(
            &mut state,
            &subagent_actor("worker", "child"),
            1,
            &report("child-finding", ReportKind::Progress, None),
        );
        assert_eq!(
            state.subagents[0].summary.as_deref(),
            Some("summary-child-finding")
        );
        let mut terminal = update("child", None);
        let OrchestrationAction::SubagentUpdate { status, .. } = &mut terminal else {
            unreachable!()
        };
        *status = SubagentStatus::Done;
        mutation(&mut state, &main, 1, &terminal);
        assert_eq!(state.subagents[0].status, SubagentStatus::Done);
        assert_eq!(
            state.subagents[0].summary.as_deref(),
            Some("summary-child-finding")
        );
        mutation(&mut state, &main, 1, &assignment);
        assert_eq!(
            state.subagents[0].summary.as_deref(),
            Some("summary-child-finding")
        );
        assert_eq!(serde_json::to_value(&state.runs[1]).unwrap(), main_before);
    }

    #[test]
    fn controls_encode_real_operation_and_done_acks_only_target_sequence() {
        let mut state = state();
        let main = main_actor("worker");
        mutation(&mut state, &main, 1, &update("child", None));
        append(
            &mut state,
            ActorRef::Operator,
            "worker",
            "brief",
            MessageKind::Instruction,
            "read first",
            None,
            None,
            false,
            None,
            None,
        )
        .unwrap();
        let control = OrchestrationAction::SubagentControl {
            run_id: "worker".into(),
            subagent_id: "child".into(),
            op: SubagentOp::Send {
                text: "actual payload".into(),
            },
        };
        let receipt = apply(
            &mut state,
            &Actor::Operator(OperatorOrigin::Browser),
            None,
            false,
            &control,
        )
        .unwrap()
        .unwrap();
        assert!(matches!(
            receipt,
            OrchestrationActionResult::Message { seq: 2, .. }
        ));
        let envelope: serde_json::Value = serde_json::from_str(&state.messages[1].text).unwrap();
        assert_eq!(
            envelope,
            serde_json::json!({ "subagent_id": "child", "op": { "op": "send", "text": "actual payload" } })
        );
        assert_eq!(
            state.subagents[0].last_control.as_ref().unwrap().stage,
            ControlStage::Stored
        );
        let done = OrchestrationAction::SubagentControlDone {
            seq: 2,
            applied: true,
            error: None,
        };
        assert_error(
            &mut state,
            &subagent_actor("worker", "wrong-child"),
            1,
            &done,
            "actor_forbidden",
        );
        assert_error(
            &mut state,
            &main_actor("root"),
            0,
            &done,
            "subagent_control_not_found",
        );
        mutation(&mut state, &subagent_actor("worker", "child"), 1, &done);
        assert_eq!(state.messages[0].stage, DeliveryStage::Stored);
        assert_eq!(state.messages[1].stage, DeliveryStage::Acked);
        assert_eq!(state.subagents[0].status, SubagentStatus::Running);
        assert_eq!(
            state.subagents[0].last_control.as_ref().unwrap().stage,
            ControlStage::Applied
        );
        mutation(&mut state, &main, 1, &done);
        assert_error(
            &mut state,
            &main,
            1,
            &OrchestrationAction::SubagentControlDone {
                seq: 2,
                applied: false,
                error: Some("late failure".into()),
            },
            "subagent_control_conflict",
        );
    }

    #[test]
    fn ordinary_inbox_consumers_cannot_complete_or_starve_pending_controls() {
        let mut state = state();
        let main = main_actor("worker");
        mutation(&mut state, &main, 1, &update("child", None));
        let operator = Actor::Operator(OperatorOrigin::Browser);
        for _ in 0..100 {
            apply(
                &mut state,
                &operator,
                None,
                false,
                &OrchestrationAction::SubagentControl {
                    run_id: "worker".into(),
                    subagent_id: "child".into(),
                    op: SubagentOp::Cancel,
                },
            )
            .unwrap();
        }
        append(
            &mut state,
            ActorRef::Operator,
            "worker",
            "ordinary",
            MessageKind::Instruction,
            "Handle this ordinary message",
            None,
            None,
            false,
            None,
            None,
        )
        .unwrap();
        mutation(
            &mut state,
            &main,
            1,
            &OrchestrationAction::InboxWoken {
                through_seq: 101,
                omp_session_id: "omp-worker".into(),
            },
        );
        assert!(
            state.messages[..100]
                .iter()
                .all(|message| message.stage == DeliveryStage::Stored)
        );
        let result = mutation(
            &mut state,
            &main,
            1,
            &OrchestrationAction::InboxPull {
                after_seq: 0,
                limit: 1,
            },
        );
        let OrchestrationActionResult::Inbox {
            messages,
            read_through_seq,
        } = result
        else {
            panic!("expected inbox");
        };
        assert_eq!(messages.len(), 1);
        assert_eq!(messages[0].message_id, "ordinary");
        assert_eq!(read_through_seq, 101);
        mutation(
            &mut state,
            &main,
            1,
            &OrchestrationAction::InboxAck { through_seq: 101 },
        );
        assert_eq!(state.messages[100].stage, DeliveryStage::Acked);
        assert!(
            state.messages[..100]
                .iter()
                .all(|message| message.stage == DeliveryStage::Stored)
        );
        assert_eq!(
            state.subagents[0].last_control.as_ref().unwrap().stage,
            ControlStage::Stored
        );
        mutation(
            &mut state,
            &subagent_actor("worker", "child"),
            1,
            &OrchestrationAction::SubagentControlDone {
                seq: 100,
                applied: true,
                error: None,
            },
        );
        assert_eq!(state.messages[99].stage, DeliveryStage::Acked);
        assert!(
            state.messages[..99]
                .iter()
                .all(|message| message.stage == DeliveryStage::Stored)
        );
        assert_eq!(
            state.subagents[0].last_control.as_ref().unwrap().stage,
            ControlStage::Applied
        );
    }

    #[test]
    fn control_requires_operator_or_strict_ancestor_and_retains_newer_receipt() {
        let mut state = state();
        mutation(&mut state, &main_actor("worker"), 1, &update("child", None));
        let action = OrchestrationAction::SubagentControl {
            run_id: "worker".into(),
            subagent_id: "child".into(),
            op: SubagentOp::Cancel,
        };
        assert_error(
            &mut state,
            &main_actor("worker"),
            1,
            &action,
            "not_ancestor",
        );
        assert_error(
            &mut state,
            &main_actor("sibling"),
            3,
            &action,
            "not_ancestor",
        );
        mutation(&mut state, &main_actor("root"), 0, &action);
        mutation(&mut state, &main_actor("root"), 0, &action);
        mutation(
            &mut state,
            &main_actor("worker"),
            1,
            &OrchestrationAction::SubagentControlDone {
                seq: 1,
                applied: true,
                error: None,
            },
        );
        assert_eq!(state.subagents[0].last_control.as_ref().unwrap().seq, 2);
        assert_eq!(
            state.subagents[0].last_control.as_ref().unwrap().stage,
            ControlStage::Stored
        );
        mutation(
            &mut state,
            &main_actor("worker"),
            1,
            &OrchestrationAction::SubagentControlDone {
                seq: 2,
                applied: false,
                error: Some("abort unavailable".into()),
            },
        );
        assert_eq!(
            state.subagents[0].last_control.as_ref().unwrap().stage,
            ControlStage::Failed
        );
        assert_eq!(state.subagents[0].status, SubagentStatus::Running);
    }

    #[test]
    fn descendant_operations_require_main_session_proof_and_actual_child_identity() {
        let mut state = state();
        let progress = report("child-progress", ReportKind::Progress, None);
        for (main_proof, actual_session, expected) in [
            (None, Some("child-session"), "session_mismatch"),
            (
                Some("other-main-session"),
                Some("child-session"),
                "session_mismatch",
            ),
            (Some("omp-worker"), None, "session_mismatch"),
            (Some("omp-worker"), Some(""), "session_mismatch"),
        ] {
            let Actor::Agent(mut caller) = subagent_actor("worker", "child") else {
                unreachable!()
            };
            caller.main_omp_session_id = main_proof.map(str::to_owned);
            caller.omp_session_id = actual_session.map(str::to_owned);
            let actor = Actor::Agent(caller);
            assert_error(&mut state, &actor, 1, &progress, expected);
            assert_error(&mut state, &actor, 1, &update("child", None), expected);
            assert_error(
                &mut state,
                &actor,
                1,
                &OrchestrationAction::Annotate {
                    run_id: "worker".into(),
                    text: "subagent annotation".into(),
                },
                expected,
            );
        }
        let Actor::Agent(mut caller) = subagent_actor("worker", "child") else {
            unreachable!()
        };
        caller.subagent_id = None;
        assert_error(
            &mut state,
            &Actor::Agent(caller),
            1,
            &progress,
            "actor_forbidden",
        );
        assert!(state.messages.is_empty());
        assert!(state.subagents.is_empty());
        assert!(state.runs[1].annotations.is_empty());
        mutation(
            &mut state,
            &subagent_actor("worker", "child"),
            1,
            &update("child", None),
        );
        mutation(&mut state, &subagent_actor("worker", "child"), 1, &progress);
        assert_eq!(state.messages[0].from_subagent_id.as_deref(), Some("child"));
    }

    #[test]
    fn main_context_cannot_use_parent_session_proof_in_place_of_actual_session() {
        let mut state = state();
        for actual_session in [None, Some("different-main-session")] {
            let Actor::Agent(mut caller) = main_actor("worker") else {
                unreachable!()
            };
            caller.omp_session_id = actual_session.map(str::to_owned);
            assert_error(
                &mut state,
                &Actor::Agent(caller),
                1,
                &update("child", None),
                "session_mismatch",
            );
        }
        let actor = main_actor("worker");
        mutation(&mut state, &actor, 1, &update("direct", None));
        mutation(&mut state, &actor, 1, &update("nested", Some("direct")));
        assert_eq!(
            state.subagents[1].parent_subagent_id.as_deref(),
            Some("direct")
        );
    }

    #[test]
    fn control_completion_requires_subagent_main_session_proof_before_ack() {
        let mut state = state();
        mutation(&mut state, &main_actor("worker"), 1, &update("child", None));
        mutation(
            &mut state,
            &main_actor("root"),
            0,
            &OrchestrationAction::SubagentControl {
                run_id: "worker".into(),
                subagent_id: "child".into(),
                op: SubagentOp::Cancel,
            },
        );
        let Actor::Agent(mut caller) = subagent_actor("worker", "child") else {
            unreachable!()
        };
        caller.main_omp_session_id = None;
        assert_error(
            &mut state,
            &Actor::Agent(caller),
            1,
            &OrchestrationAction::SubagentControlDone {
                seq: 1,
                applied: true,
                error: None,
            },
            "session_mismatch",
        );
        assert_eq!(state.messages[0].stage, DeliveryStage::Stored);
        assert_eq!(
            state.subagents[0].last_control.as_ref().unwrap().stage,
            ControlStage::Stored
        );
    }
}
