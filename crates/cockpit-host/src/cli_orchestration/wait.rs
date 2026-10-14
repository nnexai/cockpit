use std::time::Duration;

use std::path::PathBuf;

use cockpit_protocol::orchestration::{DeliveryStage, Message, OrchestrationAction, OrchestrationActionResult, ReportKind};
use cockpit_core::orchestration::Actor;
#[cfg(test)]
pub(super) use cockpit_core::orchestration::inbox_counts;

use super::output::emit;
use super::{CliError, Context};

pub(super) async fn inbox_wait(
    context: &Context,
    after: u64,
    timeout: u64,
    // Wait never reads bodies or acknowledges messages.
    json: bool,
) -> Result<(), CliError> {
    let Some(Actor::Agent(caller)) = &context.actor else {
        return Err(CliError::new("credential_required", "Inbox wait requires a private job credential"));
    };
    // Context::open supplies initial live admission; the final fence below
    // rejects stale callers before any counts are emitted. The pending interval
    // consults durable inbox metadata only.
    let result = context.service.wait_inbox(caller, after, Duration::from_secs(timeout)).await;
    context.check_caller().await?;
    emit(&result?, json)
}

pub(super) fn meaningful(message: &Message) -> bool {
    !message.report.as_ref().is_some_and(|report| matches!(report.kind, ReportKind::Progress | ReportKind::Initialized))
}

pub(super) async fn inbox_next(
    context: &Context,
    timeout: u64,
    deliverable: bool,
    capture_file: Option<PathBuf>,
    json: bool,
) -> Result<(), CliError> {
    let Some(Actor::Agent(caller)) = &context.actor else {
        return Err(CliError::new("credential_required", "Inbox next requires a private job credential"));
    };
    // Acknowledgements are prefix-fenced by core. The largest acknowledged
    // recipient sequence therefore cannot skip an earlier unread message.
    let snapshot = context.snapshot(None).await?;
    let own = context.own_run(&snapshot)?;
    let after = snapshot.messages.iter()
        .filter(|message| message.to_run_id == own.run_id && message.stage == DeliveryStage::Acked)
        .map(|message| message.seq).max().unwrap_or(0);
    context.service.wait_inbox(caller, after, Duration::from_secs(timeout)).await?;
    context.check_caller().await?;
    let pulled = context.mutate(OrchestrationAction::InboxPull { after_seq: after, limit: 100 }).await?;
    let OrchestrationActionResult::Inbox { messages, read_through_seq } = pulled.result else {
        return Err(CliError::new("orchestration_output", "Inbox pull returned no captured prefix"));
    };
    let captured = !messages.is_empty();
    let messages: Vec<_> = messages.into_iter().filter(|message| !deliverable || meaningful(message)).collect();
    let remaining = context.service.wait_inbox(caller, read_through_seq, Duration::ZERO).await?;
    let capture_file = match capture_file {
        Some(path) => Some(path),
        None if captured => Some(super::output::default_capture_file(&caller.auth, &own.run_id)?),
        None => None,
    };
    let output = serde_json::json!({
        "run_id": own.run_id,
        "messages": messages,
        "through_sequence": read_through_seq,
        "pending_count": remaining.counts.iter().map(|count| count.count).sum::<u64>(),
        "counts": remaining.counts,
        "capture_file": capture_file,
    });
    // Every acknowledged batch is recoverable even if stdout is interrupted or
    // the final live fence fails. Context exposes the default recovery directory.
    if let Some(path) = &capture_file {
        super::output::capture(path, &output)?;
    }
    if captured {
        context.mutate(OrchestrationAction::InboxAck { through_seq: read_through_seq }).await?;
    }
    context.check_caller().await?;
    emit(&output, json)
}
