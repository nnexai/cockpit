use std::{collections::BTreeMap, time::Duration};

use cockpit_protocol::orchestration::{
    DeliveryStage, Message, MessageKind, OrchestrationSnapshot, OrchestrationWaitRequest,
    SubagentOp,
};
use serde::{Deserialize, Serialize};

use super::args::{AgentKindArg, OrchestrationArgs};
use super::output::emit;
use super::retirement::{completed_main_wait, emit_main_wait_deadline};
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
    let mut through = after;
    for message in messages.iter().filter(|message| {
        message.to_run_id == run_id
            && message.seq > after
            && message.kind != MessageKind::SubagentControl
            && message.stage != DeliveryStage::Acked
    }) {
        through = through.max(message.seq);
        let key = match message.kind {
            MessageKind::PrepareBrief => "prepare_brief",
            MessageKind::WorkBrief => "work_brief",
            MessageKind::SupervisorBrief => "supervisor_brief",
            MessageKind::Instruction => "instruction",
            MessageKind::Answer => "answer",
            MessageKind::CancelRequest => "cancel_request",
            MessageKind::SubagentControl => "subagent_control",
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
    with_retirement: bool,
    after_retirement: Option<&str>,
    json: bool,
) -> Result<(), CliError> {
    let mut snapshot = context.snapshot(None).await?;
    let deadline = tokio::time::Instant::now() + Duration::from_secs(timeout);
    loop {
        let narrow = if with_retirement {
            Some(completed_main_wait(context, &snapshot, after)?)
        } else {
            None
        };
        let ordinary = if with_retirement {
            None
        } else {
            let own = context.own_run(&snapshot)?;
            Some(inbox_counts(&snapshot.messages, &own.run_id, after))
        };
        if let Some(read) = &narrow {
            if read.ready(after_retirement) {
                return emit(read, json);
            }
        } else if let Some(result) = &ordinary {
            if result.pending {
                return emit(result, json);
            }
        }
        if tokio::time::Instant::now() >= deadline {
            context.check_caller().await?;
            if with_retirement {
                return emit_main_wait_deadline(context, after, json);
            }
            return emit(ordinary.as_ref().expect("ordinary wait counts"), json);
        }
        drop(narrow);
        match next_snapshot(context, &snapshot, deadline).await? {
            Some(next) => snapshot = next,
            None => {
                // next_snapshot has completed the fresh CPU timeout fence.
                if with_retirement {
                    return emit_main_wait_deadline(context, after, json);
                }
                return emit(ordinary.as_ref().expect("ordinary wait counts"), json);
            }
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
    // Reobserve idle runtime even without an owner; durable changes wake this wait early.
    let timeout_ms = remaining.min(Duration::from_secs(3)).as_millis().max(1) as u32;
    context
        .service
        .wait(&OrchestrationWaitRequest {
            after_revision: snapshot.revision,
            after_tasks_token: snapshot.tasks_token.clone(),
            timeout_ms,
        })
        .await?;
    Ok(())
}

#[derive(Deserialize)]
pub(super) struct ControlPayload {
    pub(super) subagent_id: String,
    #[serde(rename = "op")]
    pub(super) _op: SubagentOp,
}
#[derive(Serialize)]
pub(super) struct Controls<'a> {
    pub(super) messages: Vec<&'a Message>,
}
pub(super) fn pending_controls<'a>(
    all_messages: &'a [Message],
    run: &str,
    id: &str,
) -> Result<Controls<'a>, CliError> {
    let mut messages = Vec::new();
    for message in all_messages.iter().filter(|message| {
        message.to_run_id == run
            && message.kind == MessageKind::SubagentControl
            && message.stage != DeliveryStage::Acked
    }) {
        let control: ControlPayload = serde_json::from_str(&message.text).map_err(|error| {
            CliError::new(
                "orchestration_control_invalid",
                format!("invalid stored control at seq {}: {error}", message.seq),
            )
        })?;
        if control.subagent_id == id {
            messages.push(message);
        }
    }
    messages.sort_by_key(|message| message.seq);
    Ok(Controls { messages })
}

pub(super) async fn subagent_controls(
    context: &Context,
    common: &OrchestrationArgs,
    id: &str,
    wait: bool,
    timeout: u64,
) -> Result<(), CliError> {
    if matches!(common.agent_kind, Some(AgentKindArg::Subagent))
        && common.subagent_id.as_deref() != Some(id)
    {
        return Err(CliError::new(
            "actor_forbidden",
            "subagent may only read its own controls",
        ));
    }
    let mut snapshot = context.snapshot(None).await?;
    let deadline =
        tokio::time::Instant::now() + Duration::from_secs(if wait { timeout } else { 0 });
    loop {
        let own = context.own_run(&snapshot)?;
        let result = pending_controls(&snapshot.messages, &own.run_id, id)?;
        if !result.messages.is_empty() {
            return emit(&result, common.json);
        }
        if tokio::time::Instant::now() >= deadline {
            context.check_caller().await?;
            return emit(&result, common.json);
        }
        match next_snapshot(context, &snapshot, deadline).await? {
            Some(next) => snapshot = next,
            None => return emit(&result, common.json),
        }
    }
}
