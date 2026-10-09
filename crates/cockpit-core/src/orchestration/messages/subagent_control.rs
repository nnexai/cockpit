use cockpit_protocol::orchestration::*;
use serde::{Deserialize, Serialize};

use super::{
    Actor, AppendMessage, OrchestrationState, agent, actor_ref, append,
    authenticated_child_session, error, is_ancestor, main_session, now, run_index,
    text_bound,
};
use crate::InspectionError;

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct ControlEnvelope {
    subagent_id: String,
    op: SubagentOp,
}

pub(super) fn apply(
    state: &mut OrchestrationState,
    actor: &Actor,
    caller: Option<usize>,
    action: &OrchestrationAction,
) -> Result<Option<OrchestrationActionResult>, InspectionError> {
    let result = match action {
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
                AppendMessage {
                    from,
                    to_run_id: run_id,
                    message_id: &uuid::Uuid::new_v4().to_string(),
                    kind: MessageKind::SubagentControl,
                    text: &text,
                    in_reply_to: None,
                    report: None,
                    stale: false,
                    from_subagent_id: None,
                    escalated_from: None,
                },
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
