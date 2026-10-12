use std::{collections::BTreeMap, time::Duration};

use cockpit_core::orchestration::{Actor, latest_progress_sequences};

use cockpit_protocol::orchestration::{
    DeliveryStage, Message, MessageKind, OrchestrationSnapshot, OrchestrationWaitRequest,
};
use serde::Serialize;

use super::output::emit;
use super::{CliError, Context};

#[derive(Serialize)]
pub(super) struct InboxCounts {
    pub(super) run_id: String,
    pub(super) pending: bool,
    pub(super) through_seq: u64,
    pub(super) counts: Vec<KindCount>,
}
#[derive(Serialize)]
pub(super) struct KindCount {
    pub(super) kind: MessageKind,
    pub(super) count: u64,
}

pub(super) fn inbox_counts(messages: &[Message], run_id: &str, after: u64) -> InboxCounts {
    let mut counts: BTreeMap<&str, KindCount> = BTreeMap::new();
    let latest_progress = latest_progress_sequences(messages, run_id);
    let mut through = after;
    for message in messages.iter().filter(|message| {
        message.to_run_id == run_id
            && message.seq > after
            && message.stage != DeliveryStage::Acked
            && (!message.report.as_ref().is_some_and(|report| {
                report.kind == cockpit_protocol::orchestration::ReportKind::Progress
            }) || latest_progress.contains(&message.seq))
    }) {
        through = through.max(message.seq);
        let key = match message.kind {
            MessageKind::WorkBrief => "work_brief",
            MessageKind::SupervisorBrief => "supervisor_brief",
            MessageKind::Instruction => "instruction",
            MessageKind::Answer => "answer",
            MessageKind::CancelRequest => "cancel_request",
            MessageKind::Report => "report",
            MessageKind::Observation => "observation",
        };
        counts
            .entry(key)
            .or_insert(KindCount {
                kind: message.kind,
                count: 0,
            })
            .count += 1;
    }
    InboxCounts {
        run_id: run_id.to_owned(),
        pending: !counts.is_empty(),
        through_seq: through,
        counts: counts.into_values().collect(),
    }
}

pub(super) async fn inbox_wait(
    context: &Context,
    after: u64,
    timeout: u64,
    // Wait never reads bodies or acknowledges messages.
    json: bool,
) -> Result<(), CliError> {
    let mut snapshot = context.snapshot(None).await?;
    let own = context.own_run(&snapshot)?.run_id.clone();
    let deadline = tokio::time::Instant::now() + Duration::from_secs(timeout);
    loop {
        let counts = inbox_counts(&snapshot.messages, &own, after);
        if counts.pending || tokio::time::Instant::now() >= deadline {
            context.check_caller().await?;
            return emit(&counts, json);
        }
        match next_snapshot(context, &snapshot, deadline).await? {
            Some(next) => snapshot = next,
            None => return emit(&counts, json),
        }
    }
}

pub(super) async fn next_snapshot(
    context: &Context,
    snapshot: &OrchestrationSnapshot,
    deadline: tokio::time::Instant,
) -> Result<Option<OrchestrationSnapshot>, CliError> {
    match tokio::time::timeout_at(deadline, async {
        wait_next(context, snapshot, deadline).await?;
        context.snapshot(None).await
    })
    .await
    {
        Ok(result) => result.map(Some),
        Err(_) => {
            // Only the previously empty result may be returned after cancellation.
            // This authority check is deliberately outside the caller's wait budget.
            context.check_caller().await?;
            Ok(None)
        }
    }
}
pub(super) async fn wait_next(
    context: &Context,
    snapshot: &OrchestrationSnapshot,
    deadline: tokio::time::Instant,
) -> Result<(), CliError> {
    let remaining = deadline.saturating_duration_since(tokio::time::Instant::now());
    if remaining.is_zero() {
        return Ok(());
    }
    let Some(Actor::Agent(caller)) = &context.actor else {
        return Err(CliError::new("credential_required", "Inbox wait requires a private job credential"));
    };
    let root_id = context.authenticated_run(caller)?.root_id;
    let timeout_ms = remaining.min(Duration::from_secs(3)).as_millis().max(1) as u32;
    context
        .service
        .wait_for_root(&OrchestrationWaitRequest {
            after_revision: snapshot.revision,
            after_tasks_token: snapshot.tasks_token.clone(),
            timeout_ms,
        }, &root_id)
        .await?;
    Ok(())
}

