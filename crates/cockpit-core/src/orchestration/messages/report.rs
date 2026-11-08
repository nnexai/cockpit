use cockpit_protocol::orchestration::*;

use super::{
    Actor, AuthorizedJob, AppendMessage, OrchestrationState, agent, actor_ref, append,
    check_images, duplicate, error, message_result, now, same_report,
    text_bound, upward,
};
use crate::InspectionError;
use super::super::ResolvedMutationInput;

pub(super) fn apply(
    state: &mut OrchestrationState,
    actor: &Actor,
    caller: Option<&AuthorizedJob>,
    action: &OrchestrationAction,
    resolved: Option<&ResolvedMutationInput>,
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
            images,
            ..
        } => {
            let (_, index) = agent(actor, caller)?;
            let images = check_images(state, actor, caller, images, resolved)?;
            if summary.trim().is_empty() {
                return Err(error("invalid_message", "Report summary must not be empty"));
            }
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
                    || existing.images != images
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
            let checkpoint = (*kind == ReportKind::Progress).then(|| WorkerCheckpoint::decode(plan.as_deref())).flatten();
            if let Some(checkpoint) = &checkpoint {
                if outcome.is_some() || source_output.is_some() {
                    return Err(error("invalid_report", "A checkpoint must be progress without outcome or source output"));
                }
                if caller.is_none_or(|job| job.role != super::JobRole::Worker)
                    || run.stage != RunStage::Working
                {
                    return Err(error("invalid_stage", "A checkpoint requires a current working worker"));
                }
                validate_checkpoint(checkpoint)?;
                // Check the encoded liaison notice before mutating the state.
                let notice = serde_json::json!({"event":"waiting","run_id":run.run_id,
                    "receipt_message_id":message_id,"summary":summary}).to_string();
                text_bound(&notice)?;
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
                images,
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
                checkpoint.as_ref(),
            )?;
            if checkpoint.as_ref().is_some_and(|checkpoint| checkpoint.kind == WorkerCheckpointKind::Interim) {
                super::super::threads::link_interim_task(state, index);
            }
            result
        }
        _ => return Ok(None),
    };
    Ok(Some(result))
}

fn validate_checkpoint(checkpoint: &WorkerCheckpoint) -> Result<(), InspectionError> {
    use std::path::{Component, Path};
    if checkpoint.artifacts.len() > 16 || checkpoint.artifacts.iter().any(|artifact| {
        artifact.len() > 4096 || artifact.trim().is_empty()
            || artifact.chars().any(char::is_control)
            || artifact.contains('\\')
            || !Path::new(artifact).is_absolute()
            || Path::new(artifact).components().any(|part| matches!(part, Component::ParentDir))
    }) {
        return Err(error("invalid_report", "Checkpoint artifacts must be bounded absolute paths without traversal"));
    }
    Ok(())
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
    checkpoint: Option<&WorkerCheckpoint>,
) -> Result<(), InspectionError> {
    let run = &mut state.runs[index];
    run.last_report = Some(report.clone());
    // A liaison persists across batches. Its Result is diagnostic history, not
    // worker completion or a source-integration receipt that ends delegation.
    if kind == ReportKind::Result && run.parent_run_id.is_some() {
        run.source_output = report.source_output.clone();
        run.integration = Integration::Pending;
        run.result = Some(report.clone());
        run.stage = RunStage::Reported;
    }
    run.updated_at = now();
    if state.runs[index].parent_run_id.is_some()
        && (checkpoint.is_some()
            || (matches!(kind, ReportKind::Initialized | ReportKind::Result | ReportKind::NeedsInput)
                && recipient != state.runs[index].root_id))
    {
        let root_id = state.runs[index].root_id.clone();
        let run_id = state.runs[index].run_id.clone();
        let event = if let Some(checkpoint) = checkpoint {
            match &checkpoint.kind {
                WorkerCheckpointKind::Interim => "interim",
                WorkerCheckpointKind::Waiting => "waiting",
                WorkerCheckpointKind::Resumed => "resumed",
            }
        } else {
            match kind {
                ReportKind::Initialized => "initialized",
                ReportKind::Result => "result",
                _ => "needs_input",
            }
        };
        let text = if checkpoint.is_some() {
            serde_json::json!({"event":event,"run_id":run_id,"receipt_message_id":message_id,"summary":report.summary}).to_string()
        } else {
            serde_json::json!({"event":event,"run_id":run_id,"receipt_message_id":message_id}).to_string()
        };
        append(state, AppendMessage { thread_id: None, handoff: None, from: ActorRef::Dispatcher,
        to_run_id: &root_id,
        message_id: &format!("manage-{run_id}-{message_id}"),
        kind: MessageKind::Observation,
        text: &text,
        in_reply_to: None,
        report: None,
        images: Vec::new(),
        stale: false,
        escalated_from: None, })?;
    }
    Ok(())
}
