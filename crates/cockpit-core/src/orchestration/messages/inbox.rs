use cockpit_protocol::orchestration::*;

use super::{
    Actor, AppendMessage, OrchestrationState, agent, actor_ref, append, duplicate, error,
    is_ancestor, main_session, message_result, now, run_index, text_bound, upward,
};
use crate::InspectionError;

pub(super) fn apply(
    state: &mut OrchestrationState,
    actor: &Actor,
    caller: Option<usize>,
    action: &OrchestrationAction,
) -> Result<Option<OrchestrationActionResult>, InspectionError> {
    let result = match action {
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
                            super::super::management_target(
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
                Some(super::super::management_target(
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
                state,
                AppendMessage {
                    from,
                    to_run_id: &recipient,
                    message_id,
                    kind: *kind,
                    text,
                    in_reply_to: in_reply_to.clone(),
                    report: None,
                    stale: false,
                    from_subagent_id: None,
                    escalated_from: escalated,
                },
            )?;
            if let Some(provenance) = answer_provenance {
                let annotation = super::super::decision_annotation(
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
        _ => return Ok(None),
    };
    Ok(Some(result))
}
