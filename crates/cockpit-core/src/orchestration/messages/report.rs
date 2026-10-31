use cockpit_protocol::orchestration::*;

use super::{
    Actor, AuthorizedJob, AppendMessage, OrchestrationState, agent, actor_ref, append,
    duplicate, error, message_result, now, same_report,
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
            thread_id,
            card,
            ..
        } => {
            let (_, index) = agent(actor, caller)?;
            text_bound(summary)?;
            let run = &state.runs[index];
            if thread_id.is_some() || card.is_some() {
                if caller.is_none_or(|job| job.role != super::JobRole::Root)
                    || run.parent_run_id.is_some() || run.root_id != run.run_id {
                    return Err(error("thread_forbidden", "Only a root may publish thread reports"));
                }
            }
            if let Some(thread_id) = thread_id {
                super::super::threads::validate_thread_id(thread_id)?;
            }
            if let Some(card) = card {
                validate_card(card, *kind, thread_id.is_some())?;
            }
            if let Some(plan) = plan {
                text_bound(plan)?;
            }
            let from = actor_ref(actor, caller.map(|job| job.run_index), state);
            let report = Report { card: card.clone(), message_id: message_id.clone(),
            kind: *kind,
            outcome: *outcome,
            summary: summary.clone(),
            plan: plan.clone(),
            source_output: source_output.clone(),
            at: now(), };
            // Resolve the logical address before dedupe, but retain the original escalation receipt on retry.
            let logical_target = to_run_id
                .as_deref()
                .or(state.runs[index].parent_run_id.as_deref())
                .unwrap_or(&state.runs[index].run_id);
            let (recipient, escalated) = upward(state, index, to_run_id.as_deref())?;
            if let Some(existing) = duplicate(state, &from, message_id) {
                if existing.kind != MessageKind::Report
                    || existing.text != *summary
                    || !same_report(existing.report.as_ref(), Some(&report))
                    || existing.thread_id != *thread_id
                    || existing.handoff.is_some()
                    || existing.in_reply_to.is_some()
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
            if let Some(thread_id) = thread_id {
                super::super::threads::check_report_authority(state, thread_id, &state.runs[index].run_id)?;
            }
            validate_report_stage(&state.runs[index], *kind, outcome.is_some())?;
            let result = append(
                state,
                AppendMessage { thread_id: thread_id.clone(), handoff: None, from,
                to_run_id: &recipient,
                message_id,
                kind: MessageKind::Report,
                text: summary,
                in_reply_to: None,
                report: Some(report.clone()),
                stale: false,
                escalated_from: escalated, },
            )?;
            publish_report_receipt(
                state,
                index,
                *kind,
                report,
                &recipient,
                message_id,
            )?;
            result
        }
        _ => return Ok(None),
    };
    Ok(Some(result))
}

fn validate_card(card: &ThreadCard, kind: ReportKind, has_thread: bool) -> Result<(), InspectionError> {
    let single_line = |value: &str, max: usize| !value.trim().is_empty()
        && value.len() <= max && !value.contains(['\n', '\r']);
    if !has_thread || !single_line(&card.title, 80) || !single_line(&card.next, 160)
        || card.recommendation.as_ref().is_some_and(|value| value.len() > 600)
        || card.project.as_ref().is_some_and(|value| value.len() > 80)
        || card.choices.len() > 4
        || (!card.choices.is_empty() && kind != ReportKind::NeedsInput)
        || card.choices.iter().any(|value| !single_line(value, 60))
    {
        return Err(error("invalid_card", "Invalid thread report card"));
    }
    Ok(())
}

fn validate_report_stage(
    run: &Run,
    kind: ReportKind,
    outcome_present: bool,
) -> Result<(), InspectionError> {
    if kind == ReportKind::Result {
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
    Ok(())
}

fn publish_report_receipt(
    state: &mut OrchestrationState,
    index: usize,
    kind: ReportKind,
    report: Report,
    recipient: &str,
    message_id: &str,
) -> Result<(), InspectionError> {
    let run = &mut state.runs[index];
    run.last_report = Some(report.clone());
    // A liaison persists across batches. Its Result is diagnostic history, not
    // worker completion or a source-integration receipt that ends delegation.
    if kind == ReportKind::Result && run.parent_run_id.is_some() {
        run.source_output = report.source_output.clone();
        run.integration = Integration::Pending;
        run.result = Some(report);
        run.stage = RunStage::Reported;
    }
    run.updated_at = now();
    if matches!(kind, ReportKind::Initialized | ReportKind::Result | ReportKind::NeedsInput)
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
        append(state, AppendMessage { thread_id: None, handoff: None, from: ActorRef::Dispatcher,
        to_run_id: &root_id,
        message_id: &format!("manage-{run_id}-{message_id}"),
        kind: MessageKind::Observation,
        text: &text,
        in_reply_to: None,
        report: None,
        stale: false,
        escalated_from: None, })?;
    }
    Ok(())
}
