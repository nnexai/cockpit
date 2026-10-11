use cockpit_protocol::orchestration::*;

use super::{
    Actor, AuthorizedJob, AppendMessage, OrchestrationState, agent, actor_ref, append,
    duplicate, error, main_session, message_result, now, same_report, subagent_context,
    text_bound, upward,
};
use crate::InspectionError;

pub(super) fn apply(
    state: &mut OrchestrationState,
    actor: &Actor,
    caller: Option<&AuthorizedJob>,
    action: &OrchestrationAction,
) -> Result<Option<OrchestrationActionResult>, InspectionError> {
    let result = match action {
        OrchestrationAction::Report {
            message_id,
            kind,
            outcome,
            summary,
            plan,
            source_output,
            to_run_id,
            ..
        } => {
            let (_, index) = agent(actor, caller)?;
            text_bound(summary)?;
            if let Some(plan) = plan {
                text_bound(plan)?;
            }
            if *kind == ReportKind::Result {
                main_session(caller)?;
            }
            let from_subagent_id = subagent_context(caller);
            let from = actor_ref(actor, caller.map(|job| job.run_index), state);
            let report = Report {
                message_id: message_id.clone(),
                kind: *kind,
                outcome: *outcome,
                summary: summary.clone(),
                plan: plan.clone(),
                source_output: source_output.clone(),
                at: now(),
            };
            // Resolve the logical address before dedupe, but retain the original escalation receipt on retry.
            let logical_target = to_run_id
                .as_deref()
                .or(state.runs[index].parent_run_id.as_deref())
                .unwrap_or(&state.runs[index].run_id);
            let own_main_report = from_subagent_id.is_some()
                && to_run_id.as_deref() == Some(state.runs[index].run_id.as_str());
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
            validate_report_stage(&state.runs[index], *kind, outcome.is_some())?;
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
                    stale: false,
                    from_subagent_id: from_subagent_id.clone(),
                    escalated_from: escalated,
                },
            )?;
            publish_report_receipt(
                state,
                index,
                *kind,
                report,
                from_subagent_id.as_deref(),
                &recipient,
                message_id,
            )?;
            result
        }
        _ => return Ok(None),
    };
    Ok(Some(result))
}

fn validate_report_stage(
    run: &Run,
    kind: ReportKind,
    outcome_present: bool,
) -> Result<(), InspectionError> {
    match kind {
        ReportKind::Result => {
            if run.stage != RunStage::Working
                && !(run.parent_run_id.is_none() && run.stage == RunStage::Active)
            {
                return Err(error(
                    "invalid_stage",
                    "Result requires a working run or active root",
                ));
            }
            if !outcome_present {
                return Err(error("invalid_report", "Result requires an outcome"));
            }
        }
        _ => {}
    }
    Ok(())
}

fn publish_report_receipt(
    state: &mut OrchestrationState,
    index: usize,
    kind: ReportKind,
    report: Report,
    from_subagent_id: Option<&str>,
    recipient: &str,
    message_id: &str,
) -> Result<(), InspectionError> {
    if let Some(subagent_id) = from_subagent_id {
        let own = &state.runs[index].run_id;
        if let Some(entry) = state.subagents.iter_mut()
            .find(|entry| entry.run_id == *own && entry.subagent_id == subagent_id)
        {
            entry.summary = Some(report.summary.clone());
            entry.updated_at = report.at.clone();
        }
    }
    let run = &mut state.runs[index];
    // Child reports are evidence, not the main run's outcome or latest view.
    if from_subagent_id.is_none() {
        run.last_report = Some(report.clone());
        if kind == ReportKind::Result {
            run.source_output = report.source_output.clone();
            run.integration = Integration::Pending;
            run.result = Some(report);
            if run.parent_run_id.is_some() {
                run.stage = RunStage::Reported;
            }
        }
        run.updated_at = now();
    }
    if from_subagent_id.is_none()
        && matches!(kind, ReportKind::Initialized | ReportKind::Result | ReportKind::NeedsInput)
        && recipient != state.runs[index].root_id
        && state.runs[index].parent_run_id.is_some()
    {
        let root_id = state.runs[index].root_id.clone();
        let run_id = state.runs[index].run_id.clone();
        let event = match kind {
            ReportKind::Initialized => "initialized",
            ReportKind::Result => "result",
            _ => "needs_input",
        };
        let text = serde_json::json!({"event":event,"run_id":run_id,"receipt_message_id":message_id}).to_string();
        append(state, AppendMessage {
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
        })?;
    }
    Ok(())
}
