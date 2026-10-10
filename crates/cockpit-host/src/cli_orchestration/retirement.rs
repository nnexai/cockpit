use std::time::Duration;

use cockpit_core::orchestration::{Actor, caller::retirement_read_scope};
use cockpit_protocol::orchestration::{
    CloseReason, OrchestrationSnapshot, OrchestrationWaitRequest, RetirementState, Run,
    RunRetirement, RunStage,
};
use serde::Serialize;
use sha2::{Digest, Sha256};

use super::{
    CliError,
    args::{AgentKindArg, OrchestrationArgs, RetirementReceiptArgs},
    context::{Context, required_session},
    output::emit,
    wait::{InboxCounts, inbox_counts},
};

impl Context {
    pub(super) fn retiring_run_for_review(&self) -> Result<Run, CliError> {
        let Some(Actor::Agent(caller)) = &self.actor else {
            return Err(CliError::new(
                "caller_unbound",
                "retirement requires a bound caller",
            ));
        };
        let (id, _) = caller.env_run.as_ref().ok_or_else(|| {
            CliError::new(
                "caller_mismatch",
                "retirement requires the exact inherited run attempt",
            )
        })?;
        let run = self.service.run_for_review(&self.session, id)?;
        retirement_read_scope(&run, caller)?;
        Ok(run)
    }

    pub(super) fn own_retiring_run<'a>(
        &self,
        snapshot: &'a OrchestrationSnapshot,
    ) -> Result<&'a Run, CliError> {
        let Some(Actor::Agent(caller)) = &self.actor else {
            return Err(CliError::new(
                "caller_unbound",
                "retirement requires a bound caller",
            ));
        };
        let (id, _) = caller.env_run.as_ref().ok_or_else(|| {
            CliError::new(
                "caller_mismatch",
                "retirement requires the exact inherited run attempt",
            )
        })?;
        let run = snapshot
            .runs
            .iter()
            .find(|run| run.run_id == *id)
            .ok_or_else(|| CliError::new("caller_mismatch", "caller run is not in this session"))?;
        if run.stage != RunStage::Closed {
            self.own_run(snapshot)?;
        }
        retirement_read_scope(run, caller)?;
        Ok(run)
    }
}

#[derive(Serialize)]
pub(super) struct RetirementRead<'a> {
    pub(super) retirement: Option<&'a RunRetirement>,
}

pub(super) fn retirement_wait_ready(
    initial: Option<&RunRetirement>,
    current: Option<&RunRetirement>,
) -> bool {
    initial != current
        || current.is_some_and(|record| {
            matches!(
                &record.state,
                RetirementState::Retired { .. }
                    | RetirementState::Retained { .. }
                    | RetirementState::Unknown { .. }
            )
        })
}

pub(super) async fn retirement_final_read(context: &Context) -> Result<Run, CliError> {
    context.check_caller().await?;
    context.check_retirement_process()?;
    context.retiring_run_for_review()
}

pub(super) async fn retirement_wait(
    context: &Context,
    wait: bool,
    timeout: u64,
) -> Result<Run, CliError> {
    if !wait {
        return retirement_final_read(context).await;
    }
    context.check_caller().await?;
    context.check_retirement_process()?;
    // Capture the cursor BEFORE the run read, including on every subsequent wait.
    let mut cursor = context
        .service
        .wait(&OrchestrationWaitRequest {
            after_revision: 0,
            after_tasks_token: String::new(),
            timeout_ms: 0,
        })
        .await?;
    let initial = context.retiring_run_for_review()?;
    let deadline = tokio::time::Instant::now() + Duration::from_secs(timeout);
    let mut latest = None;
    loop {
        let own = latest.as_ref().unwrap_or(&initial);
        if retirement_wait_ready(initial.retirement.as_ref(), own.retirement.as_ref())
            || tokio::time::Instant::now() >= deadline
        {
            // No timeout wrapper may cancel this final authority/output fence.
            return retirement_final_read(context).await;
        }
        let remaining = deadline.saturating_duration_since(tokio::time::Instant::now());
        cursor = context
            .service
            .wait(&OrchestrationWaitRequest {
                after_revision: cursor.revision,
                after_tasks_token: cursor.tasks_token,
                timeout_ms: remaining.as_millis().clamp(1, 30_000) as u32,
            })
            .await?;
        latest = Some(context.retiring_run_for_review()?);
    }
}

pub(super) fn require_retirement_caller(args: &OrchestrationArgs) -> Result<(), CliError> {
    if args.omp_pid.is_none()
        || !matches!(args.agent_kind, Some(AgentKindArg::Main))
        || args.subagent_id.is_some()
        || args.env_run()?.is_none()
    {
        return Err(CliError::usage(
            "retirement requires an inherited run attempt, --omp-pid and --agent-kind main",
        ));
    }
    required_session(args)?;
    Ok(())
}

pub(super) async fn read(
    context: &Context,
    wait: bool,
    timeout: u64,
    json: bool,
) -> Result<(), CliError> {
    let own = retirement_wait(context, wait, timeout).await?;
    emit(
        &RetirementRead {
            retirement: own.retirement.as_ref(),
        },
        json,
    )
}

pub(super) async fn receipt(
    context: &Context,
    args: RetirementReceiptArgs,
    json: bool,
) -> Result<(), CliError> {
    context.retiring_run_for_review()?;
    let action = args.action()?;
    context.check_retirement_process()?;
    let result = context.mutate(action).await?;
    context.check_retirement_process().map_err(|error| {
        CliError::new(
            &error.code,
            format!(
                "{}; durable mutation may already be committed",
                error.message,
            ),
        )
    })?;
    emit(&result, json)
}

#[derive(Serialize)]
#[serde(tag = "mode", rename_all = "snake_case")]
pub(super) enum MainWaitRead<'a> {
    Open {
        inbox: InboxCounts,
        retirement: Option<&'a RunRetirement>,
        retirement_token: String,
    },
    RetirementOnly {
        retirement: &'a RunRetirement,
        retirement_token: String,
    },
}

impl MainWaitRead<'_> {
    pub(super) fn ready(&self, after_retirement: Option<&str>) -> bool {
        let (token, retirement, pending) = match self {
            Self::Open {
                inbox,
                retirement,
                retirement_token,
            } => (retirement_token, *retirement, inbox.pending),
            Self::RetirementOnly {
                retirement,
                retirement_token,
            } => (retirement_token, Some(*retirement), false),
        };
        pending
            || after_retirement.is_none_or(|after| after != token)
            || retirement_wait_ready(retirement, retirement)
    }
}

pub(super) fn validate_retirement_token(token: &str) -> Result<(), CliError> {
    if token.len() != 64
        || !token
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
    {
        return Err(CliError::usage(
            "--after-retirement must be 64 lowercase hex characters",
        ));
    }
    Ok(())
}

pub(super) fn retirement_observation_token(
    record: Option<&RunRetirement>,
) -> Result<String, CliError> {
    let bytes = serde_json::to_vec(&record)
        .map_err(|error| CliError::new("orchestration_output", error.to_string()))?;
    Ok(format!("{:x}", Sha256::digest(bytes)))
}

pub(super) fn main_wait_read(
    run: &Run,
    inbox: Option<InboxCounts>,
) -> Result<MainWaitRead<'_>, CliError> {
    let retirement_token = retirement_observation_token(run.retirement.as_ref())?;
    if run.stage == RunStage::Closed {
        if run.close_reason != Some(CloseReason::Accepted) {
            return Err(CliError::new(
                "caller_mismatch",
                "only own accepted retirement may outlive tracking",
            ));
        }
        let retirement = run.retirement.as_ref().ok_or_else(|| {
            CliError::new("caller_mismatch", "closed accepted run has no retirement")
        })?;
        return Ok(MainWaitRead::RetirementOnly {
            retirement,
            retirement_token,
        });
    }
    let inbox = inbox
        .ok_or_else(|| CliError::new("caller_mismatch", "open wait requires own inbox counts"))?;
    if inbox.run_id != run.run_id {
        return Err(CliError::new(
            "caller_mismatch",
            "inbox belongs to another run",
        ));
    }
    Ok(MainWaitRead::Open {
        inbox,
        retirement: run.retirement.as_ref(),
        retirement_token,
    })
}

pub(super) fn completed_main_wait<'a>(
    context: &Context,
    snapshot: &'a OrchestrationSnapshot,
    after: u64,
) -> Result<MainWaitRead<'a>, CliError> {
    // Context::snapshot already completed its fresh postcheck: no fourth runtime.
    context.check_retirement_process()?;
    let run = context.own_retiring_run(snapshot)?;
    let inbox = if run.stage == RunStage::Closed {
        None // Never inspect even secret-containing snapshot.messages after acceptance.
    } else {
        context.own_run(snapshot)?;
        Some(inbox_counts(&snapshot.messages, &run.run_id, after))
    };
    main_wait_read(run, inbox)
}

pub(super) fn main_wait_deadline_run(context: &Context) -> Result<Run, CliError> {
    // Caller has just completed the CPU-owned fresh runtime timeout fence.
    context.check_retirement_process()?;
    context.retiring_run_for_review()
}

pub(super) fn empty_main_wait_read(run: &Run, after: u64) -> Result<MainWaitRead<'_>, CliError> {
    let inbox = if run.stage == RunStage::Closed {
        None
    } else {
        Some(InboxCounts {
            run_id: run.run_id.clone(),
            pending: false,
            through_seq: after,
            counts: vec![],
        })
    };
    main_wait_read(run, inbox)
}

pub(super) fn emit_main_wait_deadline(
    context: &Context,
    after: u64,
    json: bool,
) -> Result<(), CliError> {
    let run = main_wait_deadline_run(context)?;
    // Rebuild mode from CURRENT durable authority. Never emit previous Open mail.
    emit(&empty_main_wait_read(&run, after)?, json)
}
