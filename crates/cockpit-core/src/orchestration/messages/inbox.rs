use cockpit_protocol::orchestration::*;
use super::super::DecisionProvenance;

use super::{
    Actor, AuthorizedJob, JobRole, AppendMessage, OrchestrationState, agent, actor_ref, append, duplicate, error,
    current_question, deliverable, is_ancestor, latest_progress_sequences, message_result, now,
    run_index, text_bound, upward,
};
use crate::InspectionError;

pub(super) fn apply(
    state: &mut OrchestrationState,
    actor: &Actor,
    caller: Option<&AuthorizedJob>,
    action: &OrchestrationAction,
) -> Result<Option<OrchestrationActionResult>, InspectionError> {
    let result = match action {
        OrchestrationAction::MessageSend {
            message_id,
            to_run_id,
            kind,
            text,
            in_reply_to,
            ..
        } => send(
            state,
            actor,
            caller,
            Outgoing {
                message_id,
                to_run_id,
                kind: *kind,
                text,
                in_reply_to,
            },
        )?,
        OrchestrationAction::Annotate { run_id, text } => {
            text_bound(text)?;
            let target = run_index(state, run_id)?;
            if let Actor::Agent(_) = actor {
                let (_, index) = agent(actor, caller)?;
                let job = caller.expect("admitted agent");
                let own = &state.runs[index].run_id;
                if own != run_id && (job.role != JobRole::Root || !is_ancestor(state, own, run_id)) {
                    return Err(error(
                        "not_in_subtree",
                        "Annotations require the caller's run or descendant",
                    ));
                }
            }
            if state.runs[target].stage == RunStage::Closed
                && state.runs[target].close_reason == Some(CloseReason::Accepted)
            {
                return Err(error("invalid_stage", "Accepted run history is read-only"));
            }
            let annotation = Annotation {
                by: actor_ref(actor, caller.map(|job| job.run_index), state),
                text: text.clone(),
                at: now(),
            };
            state.runs[target].updated_at = annotation.at.clone();
            state.runs[target].annotations.push(annotation);
            OrchestrationActionResult::Done
        }
        OrchestrationAction::InboxPull { after_seq, limit } => {
            let (_, index) = agent(actor, caller)?;
            let own = &state.runs[index].run_id;
            let latest_progress = latest_progress_sequences(&state.messages, own);
            // Append order is per-recipient sequence order; stop before touching the next page.
            let mut messages = Vec::new();
            let mut read_through_seq = *after_seq;
            for message in state
                .messages
                .iter_mut()
                .filter(|message| {
                    message.to_run_id == *own
                        && message.seq > *after_seq
                        && deliverable(message, &latest_progress)
                })
                .take((*limit).min(100) as usize)
            {
                if message.stage == DeliveryStage::Stored {
                    message.stage = DeliveryStage::Read;
                }
                read_through_seq = message.seq;
                messages.push(message.clone());
            }
            if !messages.is_empty() && read_through_seq > 0 {
                super::super::dispatch::resolve_inbox_hint(state, index, read_through_seq);
            }
            OrchestrationActionResult::Inbox {
                messages,
                read_through_seq,
            }
        }
        OrchestrationAction::InboxAck { through_seq } => {
            let (_, index) = agent(actor, caller)?;
            let own = &state.runs[index].run_id;
            let latest_progress = latest_progress_sequences(&state.messages, own);
            if state.messages.iter().any(|message| {
                message.to_run_id == *own
                    && message.seq <= *through_seq
                    && deliverable(message, &latest_progress)
                    && message.stage == DeliveryStage::Stored
            }) {
                return Err(error(
                    "ack_before_read",
                    "Every deliverable message through the acknowledged sequence must first be captured",
                ));
            }
            let at = now();
            for message in state.messages.iter_mut().filter(|message| {
                message.to_run_id == *own
                    && message.seq <= *through_seq
                    && deliverable(message, &latest_progress)
            }) {
                if message.stage != DeliveryStage::Acked {
                    message.stage = DeliveryStage::Acked;
                    message.acked_at = Some(at.clone());
                }
            }
            super::super::dispatch::resolve_inbox_hint(state, index, *through_seq);
            OrchestrationActionResult::Done
        }
        _ => return Ok(None),
    };
    Ok(Some(result))
}

fn root_self_answer(state: &OrchestrationState, caller: Option<&AuthorizedJob>, target: usize) -> bool {
    let Some(job) = caller else { return false };
    let run = &state.runs[target];
    job.role == JobRole::Root && job.run_index == target
        && run.parent_run_id.is_none() && run.root_id == run.run_id
        && matches!(run.kind, RunKind::Supervisor | RunKind::Adopted)
        && run.stage == RunStage::Active
}

fn recipient(
    state: &OrchestrationState,
    actor: &Actor,
    caller: Option<&AuthorizedJob>,
    target: usize,
    to_run_id: &str,
    kind: MessageKind,
) -> Result<(String, Option<String>), InspectionError> {
    match actor {
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
            Ok((to_run_id.to_owned(), None))
        }
        Actor::Agent(_) => {
            let (_, index) = agent(actor, caller)?;
            let job = caller.expect("admitted agent");
            let own = &state.runs[index].run_id;
            if matches!(kind, MessageKind::Answer | MessageKind::Instruction | MessageKind::CancelRequest)
                && job.role != JobRole::Root
            {
                return Err(error("actor_forbidden", "Only the root job may coordinate descendant runs"));
            }
            match kind {
                MessageKind::Answer => {
                    if !root_self_answer(state, caller, target) {
                        super::super::management_target(
                            state,
                            actor,
                            caller.map(|job| job.run_index),
                            &state.runs[index].session_id,
                            to_run_id,
                        )?;
                    }
                    Ok((to_run_id.to_owned(), None))
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
                    Ok((to_run_id.to_owned(), None))
                }
                MessageKind::Observation => {
                    if !is_ancestor(state, to_run_id, own) {
                        return Err(error(
                            "not_ancestor",
                            "Observations require a strict ancestor",
                        ));
                    }
                    upward(state, index, Some(to_run_id))
                }
                _ => Err(error(
                    "actor_forbidden",
                    "Agent messages are downward instructions/cancellation or upward observations",
                )),
            }
        }
    }
}

struct Outgoing<'a> {
    message_id: &'a str,
    to_run_id: &'a str,
    kind: MessageKind,
    text: &'a str,
    in_reply_to: &'a Option<String>,
}

fn send(
    state: &mut OrchestrationState,
    actor: &Actor,
    caller: Option<&AuthorizedJob>,
    outgoing: Outgoing<'_>,
) -> Result<OrchestrationActionResult, InspectionError> {
    let Outgoing {
        message_id,
        to_run_id,
        kind,
        text,
        in_reply_to,
    } = outgoing;
    let target = run_index(state, to_run_id)?;
    let (recipient, escalated) = recipient(state, actor, caller, target, to_run_id, kind)?;
    let from = actor_ref(actor, caller.map(|job| job.run_index), state);
    // An observation retry does not change its original delivery location if ancestors later close.
    if let Some(existing) = duplicate(state, &from, message_id) {
        if existing.kind != kind
            || existing.text != text
            || existing.in_reply_to != *in_reply_to
            || existing.report.is_some()
            || existing
                .escalated_from
                .as_deref()
                .unwrap_or(&existing.to_run_id)
                != to_run_id
        {
            return Err(error(
                "message_id_conflict",
                "The sender already used this message id for a different message",
            ));
        }
        return Ok(message_result(existing, true));
    }
    let receiver = run_index(state, &recipient)?;
    if state.runs[receiver].stage == RunStage::Closed
        && state.runs[receiver].close_reason == Some(CloseReason::Accepted)
    {
        return Err(error("invalid_stage", "Accepted run history is read-only"));
    }
    let answer_provenance = if kind == MessageKind::Answer {
        if in_reply_to.as_deref().is_none_or(|id| id.trim().is_empty()) {
            return Err(error(
                "invalid_message",
                "Answers require an explicit nonempty question message id",
            ));
        }
        let run = &state.runs[target];
        // Progress and transport receipts cannot resolve or replace a semantic question.
        let current = current_question(&state.messages, &run.run_id);
        if run.stage == RunStage::Closed
            || current.is_none_or(|report| {
                Some(report.message_id.as_str()) != in_reply_to.as_deref()
            })
        {
            return Err(error(
                "question_not_current",
                "The answer must link to the open run's current NeedsInput report",
            ));
        }
        if root_self_answer(state, caller, target) {
            if text.trim().is_empty() {
                return Err(error("invalid_message", "A root human answer requires the actual nonempty answer text"));
            }
            if state.messages.iter().any(|message| {
                message.to_run_id == run.run_id && message.kind == MessageKind::Answer
                    && !message.stale && message.in_reply_to == *in_reply_to
            }) {
                return Err(error("question_already_answered", "The root's current human question already has an explicit answer"));
            }
            Some(DecisionProvenance {
                origin: DecisionOrigin::Supervisor,
                supervisor_run_id: Some(run.run_id.clone()),
                conversation: run.attachment.as_ref().and_then(|attachment| attachment.conversation.clone()),
            })
        } else {
            Some(super::super::management_target(
                state,
                actor,
                caller.map(|job| job.run_index),
                &state.runs[target].session_id,
                to_run_id,
            )?.1)
        }
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
        state,
        AppendMessage {
            from,
            to_run_id: &recipient,
            message_id,
            kind,
            text,
            in_reply_to: in_reply_to.clone(),
            report: None,
            stale: false,
            escalated_from: escalated,
        },
    )?;
    if let Some(provenance) = answer_provenance {
        let annotation = super::super::decision_annotation(
            actor_ref(actor, caller.map(|job| job.run_index), state),
            &provenance,
            &format!("Answer delivered as message {message_id}"),
        );
        state.runs[target].annotations.push(annotation);
    }
    Ok(result)
}
