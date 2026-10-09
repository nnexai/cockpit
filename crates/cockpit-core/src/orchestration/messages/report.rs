use cockpit_protocol::orchestration::*;
use sha2::{Digest, Sha256};

use super::{
    Actor, AppendMessage, OrchestrationState, agent, actor_ref, append, bound_context,
    duplicate, error, main_session, message_result, now, same_report, subagent_context,
    text_bound, upward,
};
use crate::InspectionError;

pub(super) fn apply(
    state: &mut OrchestrationState,
    actor: &Actor,
    caller: Option<usize>,
    stale: bool,
    action: &OrchestrationAction,
) -> Result<Option<OrchestrationActionResult>, InspectionError> {
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
                AppendMessage {
                    from,
                    to_run_id: &recipient,
                    message_id,
                    kind: MessageKind::Report,
                    text: summary,
                    in_reply_to: None,
                    report: Some(report.clone()),
                    stale,
                    from_subagent_id: from_subagent_id.clone(),
                    escalated_from: escalated,
                },
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
                        AppendMessage {
                            from: ActorRef::Dispatcher,
                            to_run_id: &root_id,
                            message_id: &format!("manage-{run_id}-{message_id}"),
                            kind: MessageKind::Observation,
                            text: &text,
                            in_reply_to: None,
                            report: None,
                            stale: false,
                            from_subagent_id: None,
                            escalated_from: None,
                        },
                    )?;
                }
            }
            result
        }
        _ => return Ok(None),
    };
    Ok(Some(result))
}
