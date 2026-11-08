use cockpit_protocol::orchestration::*;
use super::super::{DecisionProvenance, ResolvedMutationInput};

use super::{
    Actor, AuthorizedJob, JobRole, AppendMessage, OrchestrationState, agent, actor_ref, append, duplicate, error,
    current_question_for, deliverable, is_ancestor, latest_progress_sequences, message_result, now,
    run_index, text_bound, upward, check_images,
};
use crate::InspectionError;

pub(super) fn apply(
    state: &mut OrchestrationState,
    actor: &Actor,
    caller: Option<&AuthorizedJob>,
    action: &OrchestrationAction,
    resolved: Option<&ResolvedMutationInput>,
) -> Result<Option<OrchestrationActionResult>, InspectionError> {
    let result = match action {
        OrchestrationAction::MessageSend {
            message_id,
            to_run_id,
            kind,
            text,
            in_reply_to,
            images,
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
                images,
            },
            resolved,
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

fn self_answer(state: &OrchestrationState, caller: Option<&AuthorizedJob>, target: usize) -> bool {
    caller.is_some_and(|job| job.run_index == target)
        && if top_level_root(&state.runs[target]) { state.runs[target].stage == RunStage::Active }
           else { state.runs[target].stage != RunStage::Closed }
}

fn top_level_root(run: &Run) -> bool {
    run.parent_run_id.is_none() && run.root_id == run.run_id
        && matches!(run.kind, RunKind::Supervisor | RunKind::Adopted)
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
                MessageKind::Instruction | MessageKind::UserDirective | MessageKind::Answer | MessageKind::CancelRequest
            ) {
                return Err(error(
                    "actor_forbidden",
                    "Operators may send Instruction, UserDirective, Answer, or CancelRequest",
                ));
            }
            Ok((to_run_id.to_owned(), None))
        }
        Actor::Agent(_) => {
            let (_, index) = agent(actor, caller)?;
            let job = caller.expect("admitted agent");
            let own = &state.runs[index].run_id;
            if matches!(kind, MessageKind::Answer | MessageKind::Instruction | MessageKind::UserDirective | MessageKind::CancelRequest)
                && job.role != JobRole::Root && !(kind == MessageKind::Answer && self_answer(state, caller, target))
            {
                return Err(error("actor_forbidden", "Only the root job may coordinate descendant runs"));
            }
            match kind {
                MessageKind::Answer => {
                    if !self_answer(state, caller, target) {
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
                MessageKind::Instruction | MessageKind::UserDirective | MessageKind::CancelRequest => {
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
    images: &'a [String],
}

fn send(
    state: &mut OrchestrationState,
    actor: &Actor,
    caller: Option<&AuthorizedJob>,
    outgoing: Outgoing<'_>,
    resolved: Option<&ResolvedMutationInput>,
) -> Result<OrchestrationActionResult, InspectionError> {
    let Outgoing {
        message_id,
        to_run_id,
        kind,
        text,
        in_reply_to,
        images,
    } = outgoing;
    if text.trim().is_empty() {
        return Err(error("invalid_message", "Message text must not be empty"));
    }
    let images = check_images(state, actor, caller, images, resolved)?;
    let target = run_index(state, to_run_id)?;
    let (recipient, escalated) = recipient(state, actor, caller, target, to_run_id, kind)?;
    let from = actor_ref(actor, caller.map(|job| job.run_index), state);
    // An observation retry does not change its original delivery location if ancestors later close.
    if let Some(existing) = duplicate(state, &from, message_id) {
        if existing.kind != kind
            || existing.text != text
            || existing.in_reply_to != *in_reply_to
            || existing.report.is_some()
            || existing.handoff.is_some()
            || existing.images != images
            || (kind != MessageKind::Answer && existing.thread_id.is_some())
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
    let mut answer_thread = None;
    let mut worker_self_answer = false;
    let answer_provenance = if kind == MessageKind::Answer {
        if in_reply_to.as_deref().is_none_or(|id| id.trim().is_empty()) {
            return Err(error(
                "invalid_message",
                "Answers require an explicit nonempty question message id",
            ));
        }
        let run = &state.runs[target];
        // Progress and transport receipts cannot resolve or replace a semantic question.
        let question = state.messages.iter().rev().find(|message| {
            !message.stale && message.message_id == *in_reply_to.as_ref().unwrap()
                && matches!(&message.from, ActorRef::Run { run_id } if run_id == &run.run_id)
                && message.report.as_ref().is_some_and(|report| report.kind == ReportKind::NeedsInput)
        });
        let current = question.and_then(|question| current_question_for(&state.messages, &run.run_id, question.thread_id.as_deref()));
        if run.stage == RunStage::Closed
            || current.is_none_or(|message| Some(message.message_id.as_str()) != in_reply_to.as_deref())
        {
            return Err(error(
                "question_not_current",
                "The answer must link to the open run's current NeedsInput report",
            ));
        }
        let question = current.expect("current question checked");
        answer_thread = Some(question.thread_id.clone().unwrap_or_else(|| question.message_id.clone()));
        if top_level_root(run) && state.messages.iter().any(|message| {
            message.to_run_id == run.run_id && message.kind == MessageKind::Answer
                && !message.stale && message.in_reply_to == *in_reply_to
        }) {
            return Err(error("question_already_answered", "The root's current human question already has an explicit answer"));
        }
        if self_answer(state, caller, target) {
            if text.trim().is_empty() {
                return Err(error("invalid_message", "A human answer requires the actual nonempty answer text"));
            }
            worker_self_answer = !top_level_root(run);
            (!worker_self_answer).then(|| DecisionProvenance {
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
    let notice = if worker_self_answer {
        let root = state.runs[target].root_id.clone();
        run_index(state, &root)?;
        let notice_id = format!("resolved-{}-{message_id}", state.runs[target].run_id);
        let notice_text = serde_json::json!({
            "event": "question_resolved_in_session",
            "run_id": state.runs[target].run_id,
            "question_message_id": in_reply_to,
            "answer_message_id": message_id,
            "answer_text": text,
        }).to_string();
        text_bound(&notice_text)?;
        Some((root, notice_id, notice_text))
    } else { None };
    let result = append(
        state,
        AppendMessage { thread_id: answer_thread, handoff: None, from,
        to_run_id: &recipient,
        message_id,
        kind,
        text,
        in_reply_to: in_reply_to.clone(),
        report: None,
        images,
        stale: false,
        escalated_from: escalated, },
    )?;
    if let Some((root, notice_id, text)) = notice {
        append(state, AppendMessage {
            from: ActorRef::Dispatcher, to_run_id: &root, message_id: &notice_id,
            kind: MessageKind::Observation, text: &text, in_reply_to: None,
            report: None, thread_id: None, handoff: None, images: Vec::new(), stale: false, escalated_from: None,
        })?;
    }
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
