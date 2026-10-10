use cockpit_core::{
    extension_adapter::SourcePaneEvidence,
    orchestration::{
        Actor, AgentCaller, NativeAgentKind,
        caller::{declared_main_session, not_ready, settling_caller_matches},
        herdr::OrchestrationHerdr,
    },
    process_identity::{is_ancestor_of_self, kernel_boot_id, start_identity},
};
use cockpit_herdr::HerdrCliAdapter;
use cockpit_protocol::orchestration::NativeProcessIdentity;

use super::{
    CliError,
    args::{AgentKindArg, OrchestrationArgs},
    context::Context,
};

impl OrchestrationArgs {
    pub(super) fn env_run(&self) -> Result<Option<(String, u32)>, CliError> {
        parse_env_run(
            std::env::var("COCKPIT_RUN_ID").ok(),
            std::env::var("COCKPIT_RUN_ATTEMPT").ok(),
        )
    }

    pub(super) fn validate_identity(&self) -> Result<(), CliError> {
        match (self.agent_kind, self.subagent_id.as_deref()) {
            (Some(AgentKindArg::Subagent), None | Some("")) => Err(CliError::usage(
                "--agent-kind subagent requires --subagent-id",
            )),
            (Some(AgentKindArg::Subagent), Some(_))
                if self.omp_session.as_deref().is_none_or(str::is_empty)
                    || self.omp_main_session.as_deref().is_none_or(str::is_empty) =>
            {
                Err(CliError::usage(
                    "subagent context requires actual --omp-session and --omp-main-session",
                ))
            }
            (Some(AgentKindArg::Main) | None, Some(_)) => Err(CliError::usage(
                "--subagent-id requires --agent-kind subagent",
            )),
            _ => Ok(()),
        }
    }
}

pub(super) fn parse_env_run(
    id: Option<String>,
    attempt: Option<String>,
) -> Result<Option<(String, u32)>, CliError> {
    match (id, attempt) {
        (None, None) => Ok(None),
        (Some(id), Some(attempt)) if !id.is_empty() => {
            let attempt = attempt
                .parse::<u32>()
                .ok()
                .filter(|value| *value > 0)
                .ok_or_else(|| {
                    CliError::new(
                        "attempt_stale",
                        "COCKPIT_RUN_ATTEMPT must be a positive u32",
                    )
                })?;
            Ok(Some((id, attempt)))
        }
        _ => Err(CliError::new(
            "caller_mismatch",
            "COCKPIT_RUN_ID and COCKPIT_RUN_ATTEMPT must be supplied together",
        )),
    }
}

pub(super) async fn attested_actor(
    adapter: &HerdrCliAdapter,
    session: &String,
    source: &SourcePaneEvidence,
    args: &OrchestrationArgs,
    process: Option<NativeProcessIdentity>,
) -> Result<Actor, CliError> {
    let runtime = adapter.runtime(session).await?;
    let pane = runtime
        .panes
        .into_iter()
        .find(|pane| pane.pane_id == source.pane_id)
        .ok_or_else(|| {
            CliError::new(
                "caller_mismatch",
                "caller pane is absent from fresh runtime evidence",
            )
        })?;
    if runtime.endpoint_identity != source.endpoint_identity
        || pane.workspace_id != source.workspace_id
        || pane.tab_id != source.tab_id
        || pane
            .terminal_id
            .as_ref()
            .is_some_and(|id| *id != source.terminal_id)
    {
        return Err(CliError::new(
            "caller_mismatch",
            "caller source and runtime evidence disagree",
        ));
    }
    Ok(Actor::Agent(AgentCaller {
        endpoint_identity: source.endpoint_identity.clone(),
        session_id: session.clone(),
        workspace_id: source.workspace_id.clone(),
        tab_id: source.tab_id.clone(),
        pane_id: source.pane_id.clone(),
        boot_id: runtime.boot_id,
        terminal_id: Some(source.terminal_id.clone()),
        native_session_id: pane.native_session_id.clone(),
        actual_agent_kind: pane
            .agent_kind
            .map(NativeAgentKind::from)
            .filter(|kind| kind != &NativeAgentKind::Omp || !pane.launch_pending),
        env_run: args.env_run()?,
        omp_session_id: args.omp_session.clone(),
        main_omp_session_id: args.omp_main_session.clone(),
        agent_kind: args.agent_kind.map(Into::into),
        subagent_id: args.subagent_id.clone(),
        process,
    }))
}

impl Context {
    pub(super) async fn check_caller(&self) -> Result<(), CliError> {
        if let Some(before) = &self.evidence {
            // Runtime parsing validates tab/workspace membership before returning this view.
            let runtime = self.adapter.runtime(&self.session).await?;
            let pane = runtime
                .panes
                .into_iter()
                .find(|pane| pane.pane_id == before.pane_id)
                .ok_or_else(|| {
                    CliError::new(
                        "caller_mismatch",
                        "caller pane disappeared during orchestration operation",
                    )
                })?;
            if runtime.endpoint_identity != before.endpoint_identity
                || pane.workspace_id != before.workspace_id
                || pane.tab_id != before.tab_id
                || pane.terminal_id.as_ref() != Some(&before.terminal_id)
            {
                return Err(CliError::new(
                    "caller_mismatch",
                    "caller pane identity changed during orchestration operation",
                ));
            }
            if let Some(Actor::Agent(caller)) = &self.actor {
                let actual_agent_kind = pane.agent_kind.map(NativeAgentKind::from);
                if runtime.endpoint_identity != caller.endpoint_identity
                    || runtime.boot_id != caller.boot_id
                    || (caller.actual_agent_kind.is_none()
                        && actual_agent_kind
                            .as_ref()
                            .is_some_and(|kind| kind != &NativeAgentKind::Omp))
                    || caller.actual_agent_kind.as_ref().is_some_and(|kind| {
                        actual_agent_kind.as_ref() != Some(kind) || pane.launch_pending
                    })
                    || pane.terminal_id.as_ref() != caller.terminal_id.as_ref()
                {
                    return Err(CliError::new(
                        "caller_mismatch",
                        "caller native runtime identity changed during orchestration operation",
                    ));
                }
                let session_changed = pane.native_session_id != caller.native_session_id;
                let newly_attested = caller.actual_agent_kind.is_none()
                    && actual_agent_kind.as_ref() == Some(&NativeAgentKind::Omp)
                    && !pane.launch_pending;
                if session_changed || newly_attested {
                    let expected = declared_main_session(caller);
                    if (!session_changed || caller.native_session_id.is_none())
                        && expected.is_some_and(|session| !session.is_empty())
                        && pane
                            .native_session_id
                            .as_deref()
                            .is_none_or(|native| Some(native) == expected)
                    {
                        if let Some((id, attempt)) = &caller.env_run {
                            let run = self.service.run_for_review(&self.session, id)?;
                            if settling_caller_matches(&run, caller, *attempt) {
                                self.check_retirement_process()?;
                                return Err(not_ready().into());
                            }
                        }
                    }
                    return Err(CliError::new(
                        "caller_mismatch",
                        "caller native runtime identity changed during orchestration operation",
                    ));
                }
            }
        }
        Ok(())
    }

    pub(super) fn check_retirement_process(&self) -> Result<(), CliError> {
        let Some(Actor::Agent(caller)) = &self.actor else {
            return Err(CliError::new(
                "caller_unbound",
                "retirement requires a bound caller",
            ));
        };
        let process = caller.process.as_ref().ok_or_else(|| {
            CliError::new(
                "caller_mismatch",
                "retirement requires trusted process evidence",
            )
        })?;
        if native_process_evidence(process.pid)? != *process {
            return Err(CliError::new(
                "caller_mismatch",
                "native process incarnation changed",
            ));
        }
        Ok(())
    }
}

pub(super) fn native_process_evidence(pid: u32) -> Result<NativeProcessIdentity, CliError> {
    let mismatch = || {
        CliError::new(
            "caller_mismatch",
            "OMP PID is not a verified live CLI ancestor",
        )
    };
    let signed_pid = i32::try_from(pid).map_err(|_| mismatch())?;
    let before = start_identity(signed_pid).ok_or_else(mismatch)?;
    if !is_ancestor_of_self(signed_pid, 64) {
        return Err(mismatch());
    }
    let boot = kernel_boot_id();
    if start_identity(signed_pid) != Some(before) {
        return Err(mismatch());
    }
    Ok(NativeProcessIdentity {
        pid,
        start_ticks: before,
        kernel_boot_id: boot,
    })
}
