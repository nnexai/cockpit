use std::{collections::BTreeMap, io::Read, path::PathBuf, sync::Arc, time::Duration};

use clap::{Args, Subcommand, ValueEnum};
use cockpit_core::{
    InspectionError,
    config::load_project_configuration,
    extension_adapter::SourcePaneEvidence,
    orchestration::{Actor, AgentCaller, OrchestrationService, herdr::OrchestrationHerdr},
    process_identity::{is_ancestor_of_self, kernel_boot_id, start_identity},
    projects::ProjectService,
};
use cockpit_herdr::HerdrCliAdapter;
use super::endpoint::{AmbientEndpoint, Endpoint, resolve_endpoint};
use cockpit_host::orchestration_runtime::{RetryAuthority, retry_launch_preflight};
use cockpit_protocol::{
    orchestration::*,
    projects::{ProjectConfiguration, WorkspaceRecoveryAction, WorkspaceSetupRequest},
};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

const MAX_TEXT_BYTES: usize = 16 * 1024;

/// Retains the durable service's stable error code at the binary boundary.
#[derive(Debug)]
pub(crate) struct CliError {
    pub(crate) code: String,
    pub(crate) message: String,
}

impl CliError {
    fn new(code: &str, message: impl Into<String>) -> Self {
        Self {
            code: code.to_owned(),
            message: message.into(),
        }
    }
    fn usage(message: impl Into<String>) -> Self {
        Self::new("orchestration_usage", message)
    }
}

impl From<InspectionError> for CliError {
    fn from(error: InspectionError) -> Self {
        Self {
            code: error.code,
            message: error.message,
        }
    }
}

impl From<cockpit_herdr::ConfigError> for CliError {
    fn from(error: cockpit_herdr::ConfigError) -> Self {
        Self {
            code: error.code,
            message: error.message,
        }
    }
}

#[derive(Debug, Clone, Copy, ValueEnum)]
enum AgentKindArg {
    Main,
    Subagent,
}
impl From<AgentKindArg> for AgentKind {
    fn from(kind: AgentKindArg) -> Self {
        match kind {
            AgentKindArg::Main => Self::Main,
            AgentKindArg::Subagent => Self::Subagent,
        }
    }
}

/// These options work both before and after any nested command.
#[derive(Debug, Args)]
struct OrchestrationArgs {
    /// Herdr executable to invoke.
    #[arg(long, global = true, env = "COCKPIT_HERDR_EXECUTABLE")]
    herdr: Option<PathBuf>,
    /// Logical Herdr session; inherited caller identity is used when omitted.
    #[arg(long, global = true, env = "COCKPIT_HERDR_SESSION")]
    herdr_session: Option<String>,
    /// Explicit Herdr socket endpoint, never inferred from UI focus.
    #[arg(long, global = true, env = "COCKPIT_HERDR_SOCKET")]
    herdr_socket: Option<PathBuf>,
    /// Cockpit configuration used by the launching supervisor.
    #[arg(long, global = true, env = "COCKPIT_CONFIG_PATH")]
    config: Option<PathBuf>,
    /// Catalog root for existing local repositories; may be repeated.
    #[arg(long = "repository-root", global = true)]
    repository_roots: Vec<PathBuf>,
    /// Task root. Writes are restricted to the caller's bound run root.
    #[arg(long, global = true)]
    root: Option<String>,
    /// Emit machine-readable JSON; message bodies are untrusted data.
    #[arg(long, global = true)]
    json: bool,
    /// Native OMP session ID supplied by the calling extension.
    #[arg(long, global = true)]
    omp_session: Option<String>,
    /// Native OMP process; verified as this CLI's ancestor, never trusted from JSON.
    #[arg(long, global = true)]
    omp_pid: Option<u32>,
    /// Actual root main OMP session, supplied by the extension for subagent contexts.
    #[arg(long, global = true)]
    omp_main_session: Option<String>,
    /// Calling OMP context; Ready/Result require a bound main session.
    #[arg(long, global = true, value_enum)]
    agent_kind: Option<AgentKindArg>,
    /// Calling OMP subagent ID; required with --agent-kind subagent.
    #[arg(long, global = true)]
    subagent_id: Option<String>,
}

impl OrchestrationArgs {
    fn configuration(&self) -> Result<ProjectConfiguration, CliError> {
        let config = self
            .config
            .clone()
            .or_else(|| std::env::var_os("COCKPIT_CONFIG").map(PathBuf::from));
        Ok(load_project_configuration(
            config.as_deref(),
            (!self.repository_roots.is_empty()).then_some(self.repository_roots.as_slice()),
        )?)
    }

    fn endpoint(&self) -> Result<Endpoint, CliError> {
        resolve_endpoint(
            self.herdr.clone(),
            self.herdr_session.clone(),
            self.herdr_socket.clone(),
            &AmbientEndpoint::from_process(),
        ).map_err(|error| match error.code.as_str() {
            "missing_socket_session" | "session_required" => CliError::usage(error.message),
            _ => CliError::new(&error.code, error.message),
        })
    }

    fn env_run(&self) -> Result<Option<(String, u32)>, CliError> {
        parse_env_run(
            std::env::var("COCKPIT_RUN_ID").ok(),
            std::env::var("COCKPIT_RUN_ATTEMPT").ok(),
        )
    }

    fn validate_identity(&self) -> Result<(), CliError> {
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

fn parse_env_run(
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

struct Context {
    service: OrchestrationService,
    adapter: Arc<HerdrCliAdapter>,
    session: String,
    actor: Option<Actor>,
    evidence: Option<SourcePaneEvidence>,
}

impl Context {
    async fn open(args: &OrchestrationArgs, caller_required: bool) -> Result<Self, CliError> {
        args.validate_identity()?;
        let process = args.omp_pid.map(native_process_evidence).transpose()?;
        let endpoint = args.endpoint()?;
        let caller = caller_required || std::env::var("HERDR_ENV").ok().as_deref() == Some("1");
        let pane = if caller {
            Some(
                super::current_pane_id(
                    endpoint.executable(),
                    endpoint.socket(),
                    Some(&endpoint.session),
                    "orchestration",
                )
                .await
                .map_err(|message| CliError::new("caller_unbound", message))?,
            )
        } else {
            None
        };
        let session = endpoint.session;
        let adapter = Arc::new(HerdrCliAdapter::new(endpoint.config));
        let evidence = match pane {
            Some(pane) => Some(
                adapter
                    .source_adapter()
                    .source_pane_evidence(&session, &pane)
                    .await?,
            ),
            None => None,
        };
        let actor = match &evidence {
            Some(source) => {
                let runtime = adapter.runtime(&session).await?;
                let pane = runtime
                    .panes
                    .iter()
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
                Some(Actor::Agent(AgentCaller {
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
                        .as_ref()
                        .filter(|kind| kind.as_str() != "omp" || !pane.launch_pending)
                        .cloned(),
                    env_run: args.env_run()?,
                    omp_session_id: args.omp_session.clone(),
                    main_omp_session_id: args.omp_main_session.clone(),
                    agent_kind: args.agent_kind.map(Into::into),
                    subagent_id: args.subagent_id.clone(),
                    process,
                }))
            }
            None => None,
        };
        let context = Self {
            service: OrchestrationService::open(&args.configuration()?)?,
            adapter,
            session,
            actor,
            evidence,
        };
        context.check_caller().await?;
        Ok(context)
    }

    async fn check_caller(&self) -> Result<(), CliError> {
        if let Some(before) = &self.evidence {
            // Runtime parsing validates tab/workspace membership before returning this view.
            let runtime = self.adapter.runtime(&self.session).await?;
            let pane = runtime
                .panes
                .iter()
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
                if runtime.endpoint_identity != caller.endpoint_identity
                    || runtime.boot_id != caller.boot_id
                    || (caller.actual_agent_kind.is_none()
                        && pane.agent_kind.as_deref().is_some_and(|kind| kind != "omp"))
                    || caller.actual_agent_kind.as_ref().is_some_and(|kind| {
                        pane.agent_kind.as_ref() != Some(kind) || pane.launch_pending
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
                    && pane.agent_kind.as_deref() == Some("omp")
                    && !pane.launch_pending;
                if session_changed || newly_attested {
                    let expected = if caller.agent_kind == Some(AgentKind::Subagent) {
                        caller.main_omp_session_id.as_deref()
                    } else {
                        caller.omp_session_id.as_deref()
                    };
                    if (!session_changed || caller.native_session_id.is_none())
                        && expected.is_some_and(|session| !session.is_empty())
                        && pane
                            .native_session_id
                            .as_deref()
                            .is_none_or(|native| Some(native) == expected)
                    {
                        if let Some((id, attempt)) = &caller.env_run {
                            let run = self.service.run_for_review(&self.session, id)?;
                            if run.attempt == *attempt
                                && run.session_id == caller.session_id
                                && settling_launch_matches(&run, caller)
                                && run.location.as_ref().is_some_and(|location| {
                                    caller_location_matches(
                                        location,
                                        run.bound_omp_session.as_deref(),
                                        caller,
                                    ) && location.pane_id == caller.pane_id
                                        && location.workspace_id == caller.workspace_id
                                        && location.tab_id == caller.tab_id
                                })
                                && run
                                    .bound_omp_session
                                    .as_deref()
                                    .is_none_or(|bound| Some(bound) == expected)
                                && caller.process.as_ref().is_some_and(|process| {
                                    run.bound_omp_process
                                        .as_ref()
                                        .is_none_or(|bound| bound == process)
                                })
                            {
                                self.check_retirement_process()?;
                                return Err(caller_not_ready());
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

    async fn snapshot(&self, root: Option<String>) -> Result<OrchestrationSnapshot, CliError> {
        self.check_caller().await?;
        let snapshot = self
            .service
            .snapshot(
                &*self.adapter,
                &OrchestrationSnapshotRequest {
                    session_id: self.session.clone(),
                    root_id: root,
                },
            )
            .await?;
        self.check_caller().await?;
        Ok(snapshot)
    }

    fn own_run<'a>(&self, snapshot: &'a OrchestrationSnapshot) -> Result<&'a Run, CliError> {
        let Some(Actor::Agent(caller)) = &self.actor else {
            return Err(CliError::new(
                "caller_unbound",
                "this command requires a Herdr caller pane",
            ));
        };
        let matches = |run: &&Run| {
            run.stage != RunStage::Closed
                && run.location.as_ref().is_some_and(|location| {
                    caller_location_matches(location, run.bound_omp_session.as_deref(), caller)
                })
        };
        let run = if let Some((id, attempt)) = &caller.env_run {
            let run = snapshot
                .runs
                .iter()
                .find(|run| run.run_id == *id)
                .ok_or_else(|| {
                    CliError::new(
                        "caller_mismatch",
                        "COCKPIT_RUN_ID does not identify a run in this session",
                    )
                })?;
            if run.attempt != *attempt {
                return Err(CliError::new(
                    "attempt_stale",
                    "caller run attempt is stale",
                ));
            }
            if !matches(&run) {
                return Err(CliError::new(
                    "caller_mismatch",
                    "run is closed or caller endpoint/pane does not match",
                ));
            }
            run
        } else {
            let mut runs = snapshot.runs.iter().filter(matches);
            let run = runs.next().ok_or_else(|| {
                CliError::new(
                    "caller_unbound",
                    "caller pane is not bound; use run adopt explicitly",
                )
            })?;
            if runs.next().is_some() {
                return Err(CliError::new(
                    "caller_mismatch",
                    "more than one run is bound to this caller pane",
                ));
            }
            run
        };
        let main_session = if caller.agent_kind == Some(AgentKind::Subagent) {
            caller.main_omp_session_id.as_ref()
        } else {
            caller.omp_session_id.as_ref()
        };
        if let Some(omp_session) = main_session {
            if run
                .bound_omp_session
                .as_ref()
                .is_some_and(|bound| bound != omp_session)
            {
                return Err(CliError::new(
                    "session_mismatch",
                    "OMP main session differs from the run's bound session",
                ));
            }
        }
        Ok(run)
    }

    fn retiring_run_for_review(&self) -> Result<Run, CliError> {
        let Some(Actor::Agent(caller)) = &self.actor else {
            return Err(CliError::new("caller_unbound", "retirement requires a bound caller"));
        };
        let (id, _) = caller.env_run.as_ref().ok_or_else(|| {
            CliError::new("caller_mismatch", "retirement requires the exact inherited run attempt")
        })?;
        let run = self.service.run_for_review(&self.session, id)?;
        retirement_read_scope(&run, caller)?;
        Ok(run)
    }

    fn own_retiring_run<'a>(&self, snapshot: &'a OrchestrationSnapshot) -> Result<&'a Run, CliError> {
        let Some(Actor::Agent(caller)) = &self.actor else {
            return Err(CliError::new("caller_unbound", "retirement requires a bound caller"));
        };
        let (id, _) = caller.env_run.as_ref().ok_or_else(|| {
            CliError::new("caller_mismatch", "retirement requires the exact inherited run attempt")
        })?;
        let run = snapshot.runs.iter().find(|run| run.run_id == *id).ok_or_else(|| {
            CliError::new("caller_mismatch", "caller run is not in this session")
        })?;
        if run.stage != RunStage::Closed {
            self.own_run(snapshot)?;
        }
        retirement_read_scope(run, caller)?;
        Ok(run)
    }

    fn check_retirement_process(&self) -> Result<(), CliError> {
        let Some(Actor::Agent(caller)) = &self.actor else {
            return Err(CliError::new("caller_unbound", "retirement requires a bound caller"));
        };
        let process = caller.process.as_ref().ok_or_else(|| {
            CliError::new("caller_mismatch", "retirement requires trusted process evidence")
        })?;
        if native_process_evidence(process.pid)? != *process {
            return Err(CliError::new("caller_mismatch", "native process incarnation changed"));
        }
        Ok(())
    }

    async fn task_root(&self, args: &OrchestrationArgs, writing: bool) -> Result<String, CliError> {
        let snapshot = self.snapshot(None).await?;
        if writing {
            let own = self.own_run(&snapshot)?;
            if args.root.as_ref().is_some_and(|root| *root != own.root_id) {
                return Err(CliError::new(
                    "actor_forbidden",
                    "task writes are restricted to the caller run's root",
                ));
            }
            return Ok(own.root_id.clone());
        }
        if let Some(root) = &args.root {
            return Ok(root.clone());
        }
        if self.actor.is_some() {
            match self.own_run(&snapshot) {
                Ok(own) => return Ok(own.root_id.clone()),
                Err(error) if error.code == "caller_unbound" => {}
                Err(error) => return Err(error),
            }
        }
        match snapshot.roots.as_slice() {
            [root] => Ok(root.root_id.clone()),
            [] => Err(CliError::new("root_not_found", "session has no task roots")),
            _ => Err(CliError::usage(
                "session has multiple task roots; pass --root",
            )),
        }
    }

    async fn mutate(
        &self,
        action: OrchestrationAction,
    ) -> Result<OrchestrationMutationResponse, CliError> {
        self.check_caller().await?;
        let actor = self.actor.as_ref().ok_or_else(|| {
            CliError::new("caller_unbound", "agent mutations require HERDR_ENV=1")
        })?;
        let result = self.service.mutate(
            actor,
            OrchestrationMutationRequest {
                session_id: self.session.clone(),
                expected_revision: None,
                action,
            },
        )?;
        // A failure here is explicitly not a promise that the preceding durable write was undone.
        self.check_caller().await.map_err(|error| {
            CliError::new(
                &error.code,
                format!(
                    "{}; durable mutation may already be committed",
                    error.message
                ),
            )
        })?;
        Ok(result)
    }

    async fn retry_launch(&self, run_id: String) -> Result<OrchestrationMutationResponse, CliError> {
        self.check_caller().await?;
        let actor = self.actor.as_ref().ok_or_else(|| {
            CliError::new("caller_unbound", "agent mutations require HERDR_ENV=1")
        })?;
        let reviewed = self.service.run_for_review(&self.session, &run_id)?;
        retry_launch_preflight(self.adapter.as_ref(), &reviewed, RetryAuthority::Supervisor).await?;
        let result = self.service.mutate_reviewed(
            actor,
            OrchestrationMutationRequest {
                session_id: self.session.clone(),
                expected_revision: None,
                action: OrchestrationAction::RetryLaunch { run_id },
            },
            &reviewed,
        )?;
        self.check_caller().await.map_err(|error| {
            CliError::new(
                &error.code,
                format!("{}; durable mutation may already be committed", error.message),
            )
        })?;
        Ok(result)
    }
}

fn native_process_evidence(pid: u32) -> Result<NativeProcessIdentity, CliError> {
    let mismatch = || CliError::new("caller_mismatch", "OMP PID is not a verified live CLI ancestor");
    let signed_pid = i32::try_from(pid).map_err(|_| mismatch())?;
    let before = start_identity(signed_pid).ok_or_else(mismatch)?;
    if !is_ancestor_of_self(signed_pid, 64) {
        return Err(mismatch());
    }
    let boot = kernel_boot_id();
    if start_identity(signed_pid) != Some(before) {
        return Err(mismatch());
    }
    Ok(NativeProcessIdentity { pid, start_ticks: before, kernel_boot_id: boot })
}

fn caller_not_ready() -> CliError {
    CliError::new(
        "caller_not_ready",
        "Herdr has not yet attested this pane's native OMP; retry",
    )
}

fn startup_launch_matches(run: &Run, caller: &AgentCaller) -> bool {
    run.stage == RunStage::Preparing
        && matches!(run.kind, RunKind::Supervisor | RunKind::Worker)
        && run.close_reason.is_none()
        && run.retirement.is_none()
        && run.dispatch.as_ref().is_some_and(|dispatch| {
            matches!(
                dispatch.step,
                DispatchStep::LaunchIntent
                    | DispatchStep::LaunchPending
                    | DispatchStep::LaunchUnknown
            ) && !dispatch.agent_started
                && dispatch.launch_attempt > 0
                && dispatch.endpoint_identity.as_deref() == Some(caller.endpoint_identity.as_str())
                && caller
                    .native_session_id
                    .as_deref()
                    .is_none_or(|native| caller.omp_session_id.as_deref() == Some(native))
                && run.location.as_ref().is_some_and(|location| {
                    dispatch.launch_tag.as_deref() == Some(location.launch_tag.as_str())
                        && !location.launch_tag.is_empty()
                        && location
                            .native_session_id
                            .as_deref()
                            .is_none_or(|native| caller.omp_session_id.as_deref() == Some(native))
                })
        })
}

fn settling_launch_matches(run: &Run, caller: &AgentCaller) -> bool {
    if startup_launch_matches(run, caller) {
        return true;
    }
    // Launch proof can commit between this command's opening runtime and its
    // fresh postcheck. Retry with newly attested evidence, never accept a stale
    // caller snapshot or revive an observer on a mature/retired run.
    matches!(
        (run.kind, run.stage),
        (RunKind::Supervisor, RunStage::Active) | (RunKind::Worker, RunStage::Initializing)
    ) && run.retirement.is_none()
        && run.close_reason.is_none()
        && run.bound_omp_session == caller.omp_session_id
        && run.bound_omp_process == caller.process
        && run.dispatch.as_ref().is_some_and(|dispatch| {
            dispatch.step == DispatchStep::Launched
                && dispatch.agent_started
                && dispatch.launch_attempt > 0
                && dispatch.endpoint_identity.as_deref() == Some(caller.endpoint_identity.as_str())
                && run.location.as_ref().is_some_and(|location| {
                    dispatch.launch_tag.as_deref() == Some(location.launch_tag.as_str())
                        && !location.launch_tag.is_empty()
                        && location
                            .native_session_id
                            .as_deref()
                            .is_none_or(|native| caller.omp_session_id.as_deref() == Some(native))
                })
        })
}

fn retirement_read_scope(run: &Run, caller: &AgentCaller) -> Result<(), CliError> {
    let mismatch = || CliError::new("caller_mismatch", "retirement is scoped to the exact worker main incarnation");
    if caller.agent_kind != Some(AgentKind::Main)
        || caller.subagent_id.is_some()
        || caller.env_run.as_ref().is_none_or(|(id, _)| id != &run.run_id)
        || run.session_id != caller.session_id
    {
        return Err(mismatch());
    }
    if caller.env_run.as_ref().is_none_or(|(_, attempt)| *attempt != run.attempt) {
        return Err(CliError::new("attempt_stale", "caller run attempt is stale"));
    }
    let Some(session) = caller.omp_session_id.as_deref().filter(|id| !id.is_empty()) else {
        return Err(CliError::new("session_mismatch", "retirement requires the actual main session"));
    };
    if run.bound_omp_session.as_deref() != Some(session) {
        return Err(CliError::new("session_mismatch", "retirement main session differs from binding"));
    }
    let Some(process) = caller.process.as_ref() else {
        return Err(mismatch());
    };
    if run.bound_omp_process.as_ref() != Some(process) {
        return Err(mismatch());
    }
    let Some(location) = &run.location else {
        return Err(mismatch());
    };
    if !caller_location_matches(location, run.bound_omp_session.as_deref(), caller)
        || location.pane_id != caller.pane_id
        || location.workspace_id != caller.workspace_id
        || location.tab_id != caller.tab_id
    {
        return Err(mismatch());
    }
    if run.stage != RunStage::Closed && run.retirement.is_some() {
        return Err(mismatch());
    }
    if caller.actual_agent_kind.as_deref() != Some("omp") {
        if caller.actual_agent_kind.is_none() && startup_launch_matches(run, caller) {
            return Err(caller_not_ready());
        }
        return Err(mismatch());
    }
    if run.stage == RunStage::Closed {
        if run.close_reason != Some(CloseReason::Accepted) || run.kind != RunKind::Worker {
            return Err(mismatch());
        }
        let identity = run.retirement.as_ref().and_then(|retirement| retirement.identity.as_ref())
            .ok_or_else(mismatch)?;
        if identity.run_attempt != run.attempt
            || run.dispatch.as_ref().is_none_or(|dispatch| {
                dispatch.launch_attempt != identity.launch_attempt
                    || dispatch.launch_tag.as_deref() != Some(identity.launch_tag.as_str())
                    || dispatch.endpoint_identity.as_deref() != Some(identity.endpoint_identity.as_str())
            })
            || location.launch_tag != identity.launch_tag
            || location.terminal_id.as_deref() != Some(identity.terminal_id.as_str())
            || identity.omp_session_id != session
            || &identity.process != process
            || run.launch_shell_identity.as_ref() != Some(&identity.shell)
            || identity.endpoint_identity != caller.endpoint_identity
            || identity.session_id != caller.session_id
            || identity.workspace_id != caller.workspace_id
            || identity.tab_id != caller.tab_id
            || identity.pane_id != caller.pane_id
            || caller.terminal_id.as_deref() != Some(identity.terminal_id.as_str())
            || matches!((&identity.herdr_boot_id, &caller.boot_id), (Some(a), Some(b)) if a != b)
        {
            return Err(mismatch());
        }
    }
    Ok(())
}
fn caller_location_matches(
    location: &RunLocation,
    bound_session: Option<&str>,
    caller: &AgentCaller,
) -> bool {
    let known_matches = |expected: &Option<String>, actual: &Option<String>| {
        expected
            .as_ref()
            .is_none_or(|value| actual.as_ref() == Some(value))
    };
    let available_matches = |expected: &Option<String>, actual: &Option<String>| match (
        expected.as_ref(),
        actual.as_ref(),
    ) {
        (Some(expected), Some(actual)) => expected == actual,
        _ => true,
    };
    if location.endpoint_identity != caller.endpoint_identity
        || location.session_id != caller.session_id
        || !available_matches(&location.boot_id, &caller.boot_id)
        || !known_matches(&location.terminal_id, &caller.terminal_id)
        || !available_matches(&location.native_session_id, &caller.native_session_id)
    {
        return false;
    }
    if location.pane_id == caller.pane_id {
        return true;
    }
    // A moved pane keeps its actual terminal plus extension-supplied main session;
    // env run IDs and UI focus are never identity. Core independently applies this fence.
    location.terminal_id.is_some()
        && location.terminal_id == caller.terminal_id
        && bound_session.is_some_and(|bound| {
            caller
                .native_session_id
                .as_deref()
                .is_none_or(|native| native == bound)
                && if caller.agent_kind == Some(AgentKind::Subagent) {
                    caller.main_omp_session_id.as_deref() == Some(bound)
                } else {
                    caller.omp_session_id.as_deref() == Some(bound)
                }
        })
}

#[derive(Debug, Args)]
pub(crate) struct DescriptionArgs {
    /// Literal UTF-8 prose (at most 16 KiB); reserved metadata/checklists are not writable.
    #[arg(long, conflicts_with_all = ["description_file", "stdin"])]
    description: Option<String>,
    /// Read UTF-8 prose from a file (at most 16 KiB).
    #[arg(long, conflicts_with = "stdin")]
    description_file: Option<PathBuf>,
    /// Read UTF-8 prose from stdin (at most 16 KiB).
    #[arg(long)]
    stdin: bool,
}
impl DescriptionArgs {
    fn read(self) -> Result<Option<String>, CliError> {
        match self.description {
            Some(description) => bounded(description).map(Some),
            None => read_input(self.description_file, self.stdin),
        }
    }
}

fn read_input(file: Option<PathBuf>, stdin: bool) -> Result<Option<String>, CliError> {
    match (file, stdin) {
        (Some(_), true) => Err(CliError::usage(
            "file and stdin inputs are mutually exclusive",
        )),
        (Some(path), false) => {
            let input = std::fs::File::open(&path).map_err(|error| {
                CliError::new(
                    "orchestration_input",
                    format!("cannot open {}: {error}", path.display()),
                )
            })?;
            read_bounded(input).map(Some)
        }
        (None, true) => read_bounded(std::io::stdin().lock()).map(Some),
        (None, false) => Ok(None),
    }
}

fn read_bounded(input: impl Read) -> Result<String, CliError> {
    let mut bytes = Vec::new();
    input
        .take((MAX_TEXT_BYTES + 1) as u64)
        .read_to_end(&mut bytes)
        .map_err(|error| CliError::new("orchestration_input", error.to_string()))?;
    if bytes.len() > MAX_TEXT_BYTES {
        return Err(CliError::new("message_too_large", "input exceeds 16 KiB"));
    }
    String::from_utf8(bytes)
        .map_err(|_| CliError::new("orchestration_input", "input must be UTF-8"))
}

fn bounded(text: String) -> Result<String, CliError> {
    if text.len() > MAX_TEXT_BYTES {
        Err(CliError::new("message_too_large", "text exceeds 16 KiB"))
    } else {
        Ok(text)
    }
}

fn emit(value: &impl Serialize, json: bool) -> Result<(), CliError> {
    let output = if json {
        serde_json::to_string(value)
    } else {
        serde_json::to_string_pretty(value)
    }
    .map_err(|error| CliError::new("orchestration_output", error.to_string()))?;
    println!("{output}");
    Ok(())
}

#[derive(Debug, Args)]
pub(crate) struct TaskArgs {
    #[command(flatten)]
    common: OrchestrationArgs,
    #[command(subcommand)]
    command: TaskCommand,
}
#[derive(Debug, Clone, Copy, ValueEnum)]
pub(crate) enum StepScopeArg {
    Leaf,
    Subtree,
}
impl From<StepScopeArg> for TaskStepScope {
    fn from(value: StepScopeArg) -> Self {
        match value {
            StepScopeArg::Leaf => Self::Leaf,
            StepScopeArg::Subtree => Self::Subtree,
        }
    }
}

#[derive(Debug, Args)]
pub(crate) struct StepTaskArgs {
    task: String,
    /// Exact task_revision from task show, including prose, metadata and all steps.
    #[arg(long)]
    revision: String,
}

#[derive(Debug, Subcommand)]
pub(crate) enum TaskCommand {
    /// List canonical tasks, dependency state, step progress and exact document revision.
    List,
    /// Read description, read-only body, diagnostics, step IDs/offsets, relationships and revision.
    Show { task: String },
    /// Create without starting a worker; retain the task ID and inspect before any retry.
    Create {
        /// Stable caller UUID; generated once when omitted and reported before submission.
        #[arg(long)]
        task_id: Option<String>,
        #[arg(long)]
        title: String,
        #[command(flatten)]
        description: DescriptionArgs,
        /// Prerequisite UUID; repeat for each prerequisite (requires exact document revision).
        #[arg(long, requires = "doc_revision")]
        depends_on: Vec<String>,
        /// Follow-up source UUID; provenance only, not an implicit prerequisite.
        #[arg(long, requires_all = ["doc_revision", "source_revision"])]
        follow_up_of: Option<String>,
        /// Exact root document revision from task list --json.
        #[arg(long)]
        doc_revision: Option<String>,
        /// Exact follow-up source task revision from task show.
        #[arg(long, requires = "follow_up_of")]
        source_revision: Option<String>,
    },
    /// Update title/prose only with an exact task fence; preserves metadata and steps.
    Update {
        task: String,
        #[arg(long)]
        revision: String,
        #[arg(long)]
        title: Option<String>,
        #[command(flatten)]
        description: DescriptionArgs,
    },
    /// Replace ALL prerequisites; omit --depends-on to clear. Live work permits removal only.
    DependenciesSet {
        #[command(flatten)]
        task: StepTaskArgs,
        #[arg(long)]
        doc_revision: String,
        #[arg(long)]
        depends_on: Vec<String>,
    },
    /// Add a stable-ID step; omitted parent means top level, omitted before means append.
    StepAdd {
        #[command(flatten)]
        task: StepTaskArgs,
        #[arg(long)]
        step_id: String,
        #[arg(long)]
        parent_step_id: Option<String>,
        #[arg(long)]
        before_step_id: Option<String>,
        #[arg(long)]
        title: String,
    },
    /// Rename a step by its stable UUID.
    StepRename {
        #[command(flatten)]
        task: StepTaskArgs,
        #[arg(long)]
        step_id: String,
        #[arg(long)]
        title: String,
    },
    /// Set checked state explicitly; subtree scope updates every descendant atomically.
    StepSetChecked {
        #[command(flatten)]
        task: StepTaskArgs,
        #[arg(long)]
        step_id: String,
        #[arg(long, action = clap::ArgAction::Set, required = true)]
        checked: bool,
        #[arg(long, value_enum)]
        scope: StepScopeArg,
    },
    /// Move a step with its subtree; omitted parent means top level, before means sibling.
    StepMove {
        #[command(flatten)]
        task: StepTaskArgs,
        #[arg(long)]
        step_id: String,
        #[arg(long)]
        parent_step_id: Option<String>,
        #[arg(long)]
        before_step_id: Option<String>,
    },
    /// Remove a stable-ID step and its subtree atomically.
    StepRemove {
        #[command(flatten)]
        task: StepTaskArgs,
        #[arg(long)]
        step_id: String,
    },
    /// Adopt unmarked steps atomically with offsets from the same fenced task show.
    StepsAdopt {
        #[command(flatten)]
        task: StepTaskArgs,
        /// JSON array of {"source_offset":123,"step_id":"UUID"}; at most 64 entries.
        #[arg(long)]
        mapping: String,
    },
    /// Assign stable task IDs to unmarked root items with a document revision fence.
    AssignIds {
        #[arg(long)]
        doc_revision: String,
    },
}

impl TaskCommand {
    fn action(self, root_id: String) -> Result<OrchestrationAction, CliError> {
        Ok(match self {
            Self::Create {
                task_id, title, description, depends_on, follow_up_of, doc_revision, source_revision,
            } => OrchestrationAction::TaskCreate {
                root_id,
                task_id: task_id.unwrap_or_else(|| uuid::Uuid::new_v4().to_string()),
                title,
                description: description.read()?.unwrap_or_default(),
                depends_on,
                follow_up_of,
                expected_doc_revision: doc_revision,
                source_revision,
            },
            Self::Update { task, revision, title, description } => {
                let description = description.read()?;
                if title.is_none() && description.is_none() {
                    return Err(CliError::usage(
                        "task update requires --title, --description, --description-file or --stdin",
                    ));
                }
                OrchestrationAction::TaskUpdate {
                    root_id, task_id: task, expected_task_revision: revision, title, description,
                }
            }
            Self::DependenciesSet { task, doc_revision, depends_on } => {
                OrchestrationAction::TaskDependenciesSet {
                    root_id, task_id: task.task, expected_task_revision: task.revision,
                    expected_doc_revision: doc_revision, depends_on,
                }
            }
            Self::StepAdd { task, step_id, parent_step_id, before_step_id, title } => {
                OrchestrationAction::TaskStepAdd {
                    root_id, task_id: task.task, expected_task_revision: task.revision,
                    step_id, parent_step_id, before_step_id, title,
                }
            }
            Self::StepRename { task, step_id, title } => OrchestrationAction::TaskStepRename {
                root_id, task_id: task.task, expected_task_revision: task.revision, step_id, title,
            },
            Self::StepSetChecked { task, step_id, checked, scope } => {
                OrchestrationAction::TaskStepSetChecked {
                    root_id, task_id: task.task, expected_task_revision: task.revision,
                    step_id, checked, scope: scope.into(),
                }
            }
            Self::StepMove { task, step_id, parent_step_id, before_step_id } => {
                OrchestrationAction::TaskStepMove {
                    root_id, task_id: task.task, expected_task_revision: task.revision,
                    step_id, parent_step_id, before_step_id,
                }
            }
            Self::StepRemove { task, step_id } => OrchestrationAction::TaskStepRemove {
                root_id, task_id: task.task, expected_task_revision: task.revision, step_id,
            },
            Self::StepsAdopt { task, mapping } => {
                let mapping = bounded(mapping)?;
                let mapping: Vec<TaskStepAdoption> = serde_json::from_str(&mapping)
                    .map_err(|error| CliError::usage(format!("invalid adoption mapping: {error}")))?;
                if mapping.is_empty() || mapping.len() > 64 {
                    return Err(CliError::usage("adoption mapping requires 1–64 offset/UUID entries"));
                }
                OrchestrationAction::TaskStepsAdopt {
                    root_id, task_id: task.task, expected_task_revision: task.revision, mapping,
                }
            }
            Self::AssignIds { doc_revision } => OrchestrationAction::TasksAssignIds {
                root_id, expected_doc_revision: doc_revision,
            },
            Self::List | Self::Show { .. } => {
                return Err(CliError::usage("read-only task command has no mutation action"));
            }
        })
    }
}

fn task_submission_error(error: CliError, task_id: &str) -> CliError {
    CliError::new(
        &error.code,
        format!(
            "{}; task ID {task_id}. If the outcome is unknown, inspect `task show {task_id}` in the same root before any retry; do not append with a new ID",
            error.message,
        ),
    )
}
impl TaskArgs {
    pub async fn run(self) -> Result<(), CliError> {
        let writing = !matches!(&self.command, TaskCommand::List | TaskCommand::Show { .. });
        let context = Context::open(&self.common, writing).await?;
        let root = context.task_root(&self.common, writing).await?;
        let action = match self.command {
            TaskCommand::List => {
                let snapshot = context.snapshot(Some(root)).await?;
                let board = snapshot
                    .board
                    .ok_or_else(|| CliError::new("root_not_found", "task board does not exist"))?;
                if self.common.json {
                    return emit(&board, true);
                }
                println!("{} (revision {})", board.path, board.doc_revision);
                for view in board.tasks {
                    println!(
                        "{}\t[{}] {}\t{:?}\t{}\tdependencies={:?}\tsteps={}",
                        view.task.task_id,
                        if view.task.checked { 'x' } else { ' ' },
                        view.task.title,
                        view.lane,
                        view.task.task_revision,
                        view.dependencies.state,
                        view.task.step_progress.as_ref()
                            .map(|progress| format!("{}/{}", progress.done, progress.total))
                            .unwrap_or_else(|| "unavailable".into()),
                    );
                }
                return Ok(());
            }
            TaskCommand::Show { task } => {
                let snapshot = context.snapshot(Some(root)).await?;
                let view = snapshot
                    .board
                    .and_then(|board| {
                        board
                            .tasks
                            .into_iter()
                            .find(|view| view.task.task_id == task)
                    })
                    .ok_or_else(|| {
                        CliError::new("task_not_found", "task is not in the selected root")
                    })?;
                return emit(&view, self.common.json);
            }
            command => command.action(root)?,
        };
        if let OrchestrationAction::TaskCreate { task_id, .. } = &action {
            // Emit before submission so even a lost response leaves an inspectable identity.
            eprintln!("Task ID: {task_id}");
            let task_id = task_id.clone();
            let result = context.mutate(action).await
                .map_err(|error| task_submission_error(error, &task_id))?;
            emit(&result, self.common.json)
                .map_err(|error| task_submission_error(error, &task_id))
        } else {
            emit(&context.mutate(action).await?, self.common.json)
        }
    }
}

#[derive(Debug, Args)]
pub(crate) struct RunArgs {
    #[command(flatten)]
    common: OrchestrationArgs,
    #[command(subcommand)]
    command: RunCommand,
}
#[derive(Debug, Clone, Copy, ValueEnum)]
pub(crate) enum ReportKindArg {
    Progress,
    Ready,
    Result,
    NeedsInput,
}
impl From<ReportKindArg> for ReportKind {
    fn from(kind: ReportKindArg) -> Self {
        match kind {
            ReportKindArg::Progress => Self::Progress,
            ReportKindArg::Ready => Self::Ready,
            ReportKindArg::Result => Self::Result,
            ReportKindArg::NeedsInput => Self::NeedsInput,
        }
    }
}
#[derive(Debug, Clone, Copy, ValueEnum)]
pub(crate) enum OutcomeArg {
    Succeeded,
    Failed,
}
impl From<OutcomeArg> for ReportOutcome {
    fn from(value: OutcomeArg) -> Self {
        match value {
            OutcomeArg::Succeeded => Self::Succeeded,
            OutcomeArg::Failed => Self::Failed,
        }
    }
}
#[derive(Debug, Clone, Copy, ValueEnum)]
pub(crate) enum MessageKindArg {
    Instruction,
    CancelRequest,
    Answer,
}

#[derive(Debug, Args)]
#[group(skip)]
#[command(group(clap::ArgGroup::new("target").required(true).multiple(false).args(["repository", "path", "space", "space_worktree"])))]
pub(crate) struct ProposeArgs {
    #[arg(long)]
    task: String,
    /// Explicit catalog repository ID; never selected from cwd or current Space.
    #[arg(long)]
    repository: Option<String>,
    /// Open a directory as a borrowed checkout instead of creating a worktree.
    #[arg(long)]
    path: Option<String>,
    /// Launch in an existing Herdr workspace without workspace setup.
    #[arg(long)]
    space: Option<String>,
    /// Create an owned linked worktree from this explicit project Space's repository.
    #[arg(long)]
    space_worktree: Option<String>,
    #[arg(long, conflicts_with_all = ["path", "space"])]
    branch: Option<String>,
    #[arg(long, conflicts_with_all = ["path", "space"])]
    base: Option<String>,
    #[arg(long, requires = "repository", conflicts_with_all = ["path", "space", "space_worktree"])]
    checkout_path: Option<String>,
    #[arg(long, requires = "repository", conflicts_with_all = ["path", "space", "space_worktree"])]
    artifact: Option<String>,
    #[arg(long, requires = "repository", conflicts_with_all = ["path", "space", "space_worktree"])]
    linked_artifact: Vec<String>,
    #[arg(long, conflicts_with_all = ["space", "space_worktree"])]
    task_name: Option<String>,
    /// Preparation instructions only; work waits for exact-plan execute authority.
    #[arg(long, required_unless_present = "brief", conflicts_with = "brief")]
    brief_file: Option<PathBuf>,
    /// Literal preparation instructions (at most 16 KiB).
    #[arg(long, required_unless_present = "brief_file")]
    brief: Option<String>,
    #[arg(long)]
    label: Option<String>,
    #[arg(long)]
    parent: Option<String>,
    #[arg(long)]
    supersedes: Option<String>,
}
impl ProposeArgs {
    fn action(self) -> Result<OrchestrationAction, CliError> {
        let brief = match self.brief {
            Some(brief) => bounded(brief)?,
            None => read_input(self.brief_file, false)?
                .ok_or_else(|| CliError::usage("--brief or --brief-file is required"))?,
        };
        let target = match (self.repository, self.path, self.space, self.space_worktree) {
            (Some(repository_id), None, None, None) => DispatchTarget::Setup {
                request: WorkspaceSetupRequest::Create {
                    repository_id,
                    branch: self.branch,
                    base_ref: self.base,
                    checkout_path: self.checkout_path,
                    label: self.label.clone(),
                    task_name: self.task_name,
                    artifact_url: self.artifact,
                    linked_artifact_urls: self.linked_artifact,
                    focus: false,
                },
            },
            (None, Some(path), None, None) => DispatchTarget::Setup {
                request: WorkspaceSetupRequest::Open {
                    path,
                    label: self.label.clone(),
                    task_name: self.task_name,
                    focus: false,
                },
            },
            (None, None, Some(workspace_id), None) => DispatchTarget::ExistingSpace { workspace_id },
            (None, None, None, Some(workspace_id)) => DispatchTarget::SpaceWorktree {
                workspace_id,
                branch: self.branch,
                base_ref: self.base,
            },
            _ => {
                return Err(CliError::usage(
                    "propose requires exactly one of --repository, --path, --space, --space-worktree",
                ));
            }
        };
        Ok(OrchestrationAction::RunPropose {
            task_id: self.task,
            parent_run_id: self.parent,
            label: self.label,
            target,
            prepare_brief: brief,
            supersedes_run_id: self.supersedes,
        })
    }
}

#[derive(Debug, Clone, Copy, ValueEnum)]
#[value(rename_all = "snake_case")]
pub(crate) enum NativeDeferReasonArg {
    Busy,
    PendingMessages,
    AsyncJobs,
    LiveSubagents,
    EditorDraft,
}
impl From<NativeDeferReasonArg> for NativeDeferReason {
    fn from(reason: NativeDeferReasonArg) -> Self {
        match reason {
            NativeDeferReasonArg::Busy => Self::Busy,
            NativeDeferReasonArg::PendingMessages => Self::PendingMessages,
            NativeDeferReasonArg::AsyncJobs => Self::AsyncJobs,
            NativeDeferReasonArg::LiveSubagents => Self::LiveSubagents,
            NativeDeferReasonArg::EditorDraft => Self::EditorDraft,
        }
    }
}

#[derive(Debug, Clone, Copy, ValueEnum)]
#[value(rename_all = "snake_case")]
pub(crate) enum NativeRefuseReasonArg {
    UserActivity,
    NativeRefused,
}
impl From<NativeRefuseReasonArg> for NativeRefuseReason {
    fn from(reason: NativeRefuseReasonArg) -> Self {
        match reason {
            NativeRefuseReasonArg::UserActivity => Self::UserActivity,
            NativeRefuseReasonArg::NativeRefused => Self::NativeRefused,
        }
    }
}

#[derive(Debug, Args)]
#[group(skip)]
#[command(group(clap::ArgGroup::new("retirement_outcome").required(true).multiple(false).args(["shutdown_requested", "deferred", "refused"])))]
pub(crate) struct RetirementReceiptArgs {
    #[arg(long)]
    retirement: String,
    /// Records intent only; does not prove that the native process stopped.
    #[arg(long)]
    shutdown_requested: bool,
    #[arg(long, value_enum)]
    deferred: Option<NativeDeferReasonArg>,
    /// Explanation only, at most 1 KiB; typed reason determines the outcome.
    #[arg(long, requires = "refuse_reason")]
    refused: Option<String>,
    #[arg(long, value_enum, requires = "refused")]
    refuse_reason: Option<NativeRefuseReasonArg>,
}
impl RetirementReceiptArgs {
    fn action(self) -> Result<OrchestrationAction, CliError> {
        let outcome = match (self.shutdown_requested, self.deferred, self.refused, self.refuse_reason) {
            (true, None, None, None) => NativeStopReceipt::ShutdownRequested,
            (false, Some(reason), None, None) => NativeStopReceipt::Deferred { reason: reason.into() },
            (false, None, Some(text), Some(reason)) if text.len() <= 1024 => {
                NativeStopReceipt::Refused { reason: reason.into(), text }
            }
            (false, None, Some(_), Some(_)) => {
                return Err(CliError::usage("retirement refusal exceeds 1 KiB"));
            }
            _ => return Err(CliError::usage("retirement receipt requires exactly one typed outcome")),
        };
        Ok(OrchestrationAction::RetirementNativeReceipt { retirement_id: self.retirement, outcome })
    }
}

#[derive(Serialize)]
struct RetirementRead<'a> {
    retirement: Option<&'a RunRetirement>,
}

fn retirement_wait_ready(
    initial: Option<&RunRetirement>,
    current: Option<&RunRetirement>,
) -> bool {
    initial != current
        || current.is_some_and(|record| matches!(
            &record.state,
            RetirementState::Retired { .. }
                | RetirementState::Retained { .. }
                | RetirementState::Unknown { .. }
        ))
}

async fn retirement_final_read(context: &Context) -> Result<Run, CliError> {
    context.check_caller().await?;
    context.check_retirement_process()?;
    context.retiring_run_for_review()
}

async fn retirement_wait(context: &Context, wait: bool, timeout: u64) -> Result<Run, CliError> {
    if !wait {
        return retirement_final_read(context).await;
    }
    context.check_caller().await?;
    context.check_retirement_process()?;
    // Capture the cursor BEFORE the run read, including on every subsequent wait.
    let mut cursor = context.service.wait(&OrchestrationWaitRequest {
        after_revision: 0,
        after_tasks_token: String::new(),
        timeout_ms: 0,
    }).await?;
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
        cursor = context.service.wait(&OrchestrationWaitRequest {
            after_revision: cursor.revision,
            after_tasks_token: cursor.tasks_token,
            timeout_ms: remaining.as_millis().clamp(1, 30_000) as u32,
        }).await?;
        latest = Some(context.retiring_run_for_review()?);
    }
}

fn require_retirement_caller(args: &OrchestrationArgs) -> Result<(), CliError> {
    if args.omp_pid.is_none()
        || !matches!(args.agent_kind, Some(AgentKindArg::Main))
        || args.subagent_id.is_some()
        || args.env_run()?.is_none()
    {
        return Err(CliError::usage("retirement requires an inherited run attempt, --omp-pid and --agent-kind main"));
    }
    required_session(args)?;
    Ok(())
}

#[derive(Debug, Clone, Copy, ValueEnum)]
pub(crate) enum RecoveryArg {
    AcceptExistingWorktree,
}

#[derive(Debug, Subcommand)]
pub(crate) enum RunCommand {
    /// List durable runs joined to fresh Herdr observations.
    List {
        #[arg(long)]
        tree: bool,
    },
    /// Show one run, or the run bound to the caller with --self.
    #[command(group(clap::ArgGroup::new("selection").required(true).args(["run", "self_run"])))]
    Show {
        #[arg(conflicts_with = "self_run")]
        run: Option<String>,
        #[arg(long = "self")]
        self_run: bool,
    },
    /// Propose preparation; never grants prepare or execute authority.
    Propose(ProposeArgs),
    /// Prepare a descendant worker after inspecting its exact current setup plan.
    Prepare {
        run: String,
        #[arg(long)]
        plan_revision: String,
    },
    /// Execute a Ready descendant worker after inspecting its exact work plan.
    Execute {
        run: String,
        #[arg(long)]
        plan_revision: String,
        #[arg(long)]
        note: Option<String>,
    },
    /// Accept an explicit successful result against the canonical task revision.
    Accept {
        run: String,
        #[arg(long)]
        task_revision: String,
    },
    /// Send a descendant worker's result back with actionable review feedback.
    SendBack {
        run: String,
        #[arg(long)]
        text: String,
    },
    /// Close descendant tracking and request cancellation; does not guarantee a stop.
    Cancel { run: String },
    /// Read-only review or re-plan of a descendant worker; accept only an inventory-proven worktree.
    Reconcile {
        run: String,
        #[arg(long, value_enum)]
        recovery: Option<RecoveryArg>,
    },
    /// Restart a descendant worker after fresh absence proof; never duplicates a live original.
    RetryLaunch { run: String },
    /// Report upward with a required sender-chosen deduplication ID.
    Report {
        #[arg(long, value_enum)]
        kind: ReportKindArg,
        #[arg(long)]
        message_id: String,
        #[arg(long)]
        summary: String,
        #[arg(long, value_enum)]
        outcome: Option<OutcomeArg>,
        #[arg(long, conflicts_with_all = ["plan_file", "stdin"])]
        plan: Option<String>,
        #[arg(long, conflicts_with = "stdin")]
        plan_file: Option<PathBuf>,
        /// Read a work plan from stdin (at most 16 KiB).
        #[arg(long)]
        stdin: bool,
        #[arg(long)]
        to: Option<String>,
    },
    /// Bind the caller run to the native main OMP session.
    BindSession,
    /// Read only this main session's retirement; never stops a process or closes a pane.
    Retirement {
        #[arg(long)]
        wait: bool,
        #[arg(long, default_value_t = 30, value_parser = clap::value_parser!(u64).range(1..=300))]
        timeout: u64,
    },
    /// Record self-retirement readiness; no native stop success is implied.
    RetirementReceipt(RetirementReceiptArgs),
    /// Send a durable instruction, answer or cancel request to a descendant run.
    Message {
        run: String,
        #[arg(long, value_enum)]
        kind: MessageKindArg,
        #[arg(long)]
        message_id: String,
        #[arg(long)]
        text: String,
    },
    /// Append an annotation without changing reported results.
    Annotate {
        run: String,
        #[arg(long)]
        text: String,
    },
    /// Explicitly adopt an unbound current caller pane as a new root.
    Adopt {
        #[arg(long)]
        label: String,
    },
}
impl RunArgs {
    pub async fn run(self) -> Result<(), CliError> {
        if matches!(&self.command, RunCommand::Retirement { .. } | RunCommand::RetirementReceipt(_)) {
            require_retirement_caller(&self.common)?;
        }
        let caller_required = !matches!(
            &self.command,
            RunCommand::List { .. }
                | RunCommand::Show {
                    self_run: false,
                    ..
                }
        );
        let context = Context::open(&self.common, caller_required).await?;
        let action = match self.command {
            RunCommand::List { tree } => {
                let snapshot = context.snapshot(self.common.root.clone()).await?;
                if self.common.json {
                    return emit(&snapshot, true);
                }
                for run in &snapshot.runs {
                    let depth = if tree {
                        run_depth(run, &snapshot.runs)
                    } else {
                        0
                    };
                    let observation = match &snapshot.runtime {
                        RuntimeObservation::Fresh { runs, .. } => {
                            runs.iter().find(|item| item.run_id == run.run_id)
                        }
                        RuntimeObservation::Unavailable { .. } => None,
                    };
                    println!(
                        "{}{}\t{}\t{:?}\t{}",
                        "  ".repeat(depth),
                        run.run_id,
                        run.label,
                        run.stage,
                        observation
                            .and_then(|item| item.agent_status.as_deref())
                            .unwrap_or("unobserved")
                    );
                }
                return Ok(());
            }
            RunCommand::Show { run, self_run } => {
                let snapshot = context.snapshot(self.common.root.clone()).await?;
                let run = if self_run {
                    context.own_run(&snapshot)?
                } else {
                    snapshot
                        .runs
                        .iter()
                        .find(|item| Some(&item.run_id) == run.as_ref())
                        .ok_or_else(|| {
                            CliError::new(
                                "run_not_found",
                                "run is not in the selected session/root",
                            )
                        })?
                };
                return emit(run, self.common.json);
            }
            RunCommand::Propose(args) => args.action()?,
            RunCommand::Prepare { run, plan_revision } => OrchestrationAction::GrantPrepare {
                run_id: run,
                plan_revision,
            },
            RunCommand::Execute {
                run,
                plan_revision,
                note,
            } => OrchestrationAction::GrantExecute {
                run_id: run,
                plan_revision,
                note: note.map(bounded).transpose()?,
            },
            RunCommand::Accept { run, task_revision } => OrchestrationAction::Accept {
                run_id: run,
                expected_task_revision: task_revision,
            },
            RunCommand::SendBack { run, text } => OrchestrationAction::SendBack {
                run_id: run,
                text: bounded(text)?,
            },
            RunCommand::Cancel { run } => OrchestrationAction::CancelRun { run_id: run },
            RunCommand::Reconcile { run, recovery } => OrchestrationAction::ReconcileRun {
                run_id: run,
                recovery: recovery.map(|RecoveryArg::AcceptExistingWorktree| WorkspaceRecoveryAction::AcceptExistingWorktree),
            },
            RunCommand::RetryLaunch { run } => {
                return emit(&context.retry_launch(run).await?, self.common.json);
            }
            RunCommand::Report {
                kind,
                message_id,
                summary,
                outcome,
                plan,
                plan_file,
                stdin,
                to,
            } => {
                required_session(&self.common)?;
                if self.common.agent_kind.is_none() {
                    return Err(CliError::usage(
                        "run report requires --agent-kind main or subagent",
                    ));
                }
                let plan = match plan {
                    Some(plan) => Some(bounded(plan)?),
                    None => read_input(plan_file, stdin)?,
                };
                OrchestrationAction::Report {
                    message_id,
                    kind: kind.into(),
                    outcome: outcome.map(Into::into),
                    summary: bounded(summary)?,
                    plan,
                    to_run_id: to,
                }
            }
            RunCommand::BindSession => {
                if !matches!(self.common.agent_kind, Some(AgentKindArg::Main)) {
                    return Err(CliError::usage(
                        "run bind-session requires --agent-kind main",
                    ));
                }
                OrchestrationAction::RunBindSession {
                    omp_session_id: required_session(&self.common)?,
                }
            }
            RunCommand::Retirement { wait, timeout } => {
                let own = retirement_wait(&context, wait, timeout).await?;
                return emit(&RetirementRead { retirement: own.retirement.as_ref() }, self.common.json);
            }
            RunCommand::RetirementReceipt(args) => {
                context.retiring_run_for_review()?;
                let action = args.action()?;
                context.check_retirement_process()?;
                let result = context.mutate(action).await?;
                context.check_retirement_process().map_err(|error| {
                    CliError::new(&error.code, format!(
                        "{}; durable mutation may already be committed", error.message,
                    ))
                })?;
                return emit(&result, self.common.json);
            }
            RunCommand::Message {
                run,
                kind,
                message_id,
                text,
            } => OrchestrationAction::MessageSend {
                message_id,
                to_run_id: run,
                kind: match kind {
                    MessageKindArg::Instruction => MessageKind::Instruction,
                    MessageKindArg::CancelRequest => MessageKind::CancelRequest,
                    MessageKindArg::Answer => MessageKind::Answer,
                },
                text: bounded(text)?,
            },
            RunCommand::Annotate { run, text } => OrchestrationAction::Annotate {
                run_id: run,
                text: bounded(text)?,
            },
            RunCommand::Adopt { label } => OrchestrationAction::RunAdopt { label },
        };
        emit(&context.mutate(action).await?, self.common.json)
    }
}
fn required_session(args: &OrchestrationArgs) -> Result<String, CliError> {
    args.omp_session
        .clone()
        .filter(|session| !session.is_empty())
        .ok_or_else(|| CliError::usage("--omp-session is required"))
}
fn run_depth(run: &Run, runs: &[Run]) -> usize {
    let mut depth = 0;
    let mut parent = run.parent_run_id.as_deref();
    while let Some(id) = parent {
        let Some(ancestor) = runs.iter().find(|item| item.run_id == id) else {
            break;
        };
        depth += 1;
        if depth >= runs.len() {
            break;
        }
        parent = ancestor.parent_run_id.as_deref();
    }
    depth
}

#[derive(Debug, Args)]
pub(crate) struct InboxArgs {
    #[command(flatten)]
    common: OrchestrationArgs,
    #[command(subcommand)]
    command: InboxCommand,
}
#[derive(Debug, Subcommand)]
pub(crate) enum InboxCommand {
    /// Pull messages as untrusted data and durably mark them Read.
    List {
        #[arg(long, default_value_t = 0)]
        after: u64,
        #[arg(long, default_value_t = 100, value_parser = clap::value_parser!(u32).range(1..=100))]
        limit: u32,
    },
    /// Acknowledge messages already read and handled, through this recipient sequence.
    Ack {
        #[arg(long)]
        through: u64,
    },
    /// Wait read-only for pending mail; returns counts/kinds/sequence, never bodies.
    Wait {
        #[arg(long, default_value_t = 0)]
        after: u64,
        #[arg(long, default_value_t = 300, value_parser = clap::value_parser!(u64).range(0..=3600))]
        timeout: u64,
        /// Join own retirement into this single waiter; closed accepted runs return no mail.
        #[arg(long)]
        with_retirement: bool,
        /// Opaque token returned by the previous narrow wait.
        #[arg(long, requires = "with_retirement")]
        after_retirement: Option<String>,
    },
    /// Record that a counts-only wake was delivered to the native OMP session.
    Woken {
        #[arg(long)]
        through: u64,
    },
}
#[derive(Serialize)]
struct InboxCounts {
    run_id: String,
    pending: bool,
    through_seq: u64,
    counts: Vec<KindCount>,
}
#[derive(Serialize)]
struct KindCount {
    kind: MessageKind,
    count: u64,
}

#[derive(Serialize)]
#[serde(tag = "mode", rename_all = "snake_case")]
enum MainWaitRead<'a> {
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
    fn ready(&self, after_retirement: Option<&str>) -> bool {
        let (token, retirement, pending) = match self {
            Self::Open { inbox, retirement, retirement_token } => {
                (retirement_token, *retirement, inbox.pending)
            }
            Self::RetirementOnly { retirement, retirement_token } => {
                (retirement_token, Some(*retirement), false)
            }
        };
        pending
            || after_retirement.is_none_or(|after| after != token)
            || retirement_wait_ready(retirement, retirement)
    }
}

fn validate_retirement_token(token: &str) -> Result<(), CliError> {
    if token.len() != 64 || !token.bytes().all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte)) {
        return Err(CliError::usage("--after-retirement must be 64 lowercase hex characters"));
    }
    Ok(())
}

fn retirement_observation_token(record: Option<&RunRetirement>) -> Result<String, CliError> {
    let bytes = serde_json::to_vec(&record)
        .map_err(|error| CliError::new("orchestration_output", error.to_string()))?;
    Ok(format!("{:x}", Sha256::digest(bytes)))
}

fn main_wait_read(run: &Run, inbox: Option<InboxCounts>) -> Result<MainWaitRead<'_>, CliError> {
    let retirement_token = retirement_observation_token(run.retirement.as_ref())?;
    if run.stage == RunStage::Closed {
        if run.close_reason != Some(CloseReason::Accepted) {
            return Err(CliError::new("caller_mismatch", "only own accepted retirement may outlive tracking"));
        }
        let retirement = run.retirement.as_ref().ok_or_else(|| {
            CliError::new("caller_mismatch", "closed accepted run has no retirement")
        })?;
        return Ok(MainWaitRead::RetirementOnly { retirement, retirement_token });
    }
    let inbox = inbox.ok_or_else(|| CliError::new("caller_mismatch", "open wait requires own inbox counts"))?;
    if inbox.run_id != run.run_id {
        return Err(CliError::new("caller_mismatch", "inbox belongs to another run"));
    }
    Ok(MainWaitRead::Open { inbox, retirement: run.retirement.as_ref(), retirement_token })
}

fn completed_main_wait<'a>(
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

fn main_wait_deadline_run(context: &Context) -> Result<Run, CliError> {
    // Caller has just completed the CPU-owned fresh runtime timeout fence.
    context.check_retirement_process()?;
    context.retiring_run_for_review()
}

fn empty_main_wait_read(run: &Run, after: u64) -> Result<MainWaitRead<'_>, CliError> {
    let inbox = if run.stage == RunStage::Closed {
        None
    } else {
        Some(InboxCounts { run_id: run.run_id.clone(), pending: false, through_seq: after, counts: vec![] })
    };
    main_wait_read(run, inbox)
}

fn emit_main_wait_deadline(context: &Context, after: u64, json: bool) -> Result<(), CliError> {
    let run = main_wait_deadline_run(context)?;
    // Rebuild mode from CURRENT durable authority. Never emit previous Open mail.
    emit(&empty_main_wait_read(&run, after)?, json)
}
fn inbox_counts(messages: &[Message], run_id: &str, after: u64) -> InboxCounts {
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
impl InboxArgs {
    pub async fn run(self) -> Result<(), CliError> {
        if let InboxCommand::Wait { with_retirement, after_retirement, .. } = &self.command {
            if let Some(token) = after_retirement {
                validate_retirement_token(token)?;
                if !*with_retirement {
                    return Err(CliError::usage("--after-retirement requires --with-retirement"));
                }
            }
            if *with_retirement {
                require_retirement_caller(&self.common)?;
            }
        }
        let context = Context::open(&self.common, true).await?;
        let action = match self.command {
            InboxCommand::List { after, limit } => OrchestrationAction::InboxPull {
                after_seq: after,
                limit,
            },
            InboxCommand::Ack { through } => OrchestrationAction::InboxAck {
                through_seq: through,
            },
            InboxCommand::Woken { through } => OrchestrationAction::InboxWoken {
                through_seq: through,
                omp_session_id: required_session(&self.common)?,
            },
            InboxCommand::Wait { after, timeout, with_retirement, after_retirement } => {
                let mut snapshot = context.snapshot(None).await?;
                let deadline = tokio::time::Instant::now() + Duration::from_secs(timeout);
                loop {
                    let narrow = if with_retirement {
                        Some(completed_main_wait(&context, &snapshot, after)?)
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
                        if read.ready(after_retirement.as_deref()) {
                            return emit(read, self.common.json);
                        }
                    } else if let Some(result) = &ordinary {
                        if result.pending {
                            return emit(result, self.common.json);
                        }
                    }
                    if tokio::time::Instant::now() >= deadline {
                        context.check_caller().await?;
                        if with_retirement {
                            return emit_main_wait_deadline(&context, after, self.common.json);
                        }
                        return emit(ordinary.as_ref().expect("ordinary wait counts"), self.common.json);
                    }
                    drop(narrow);
                    match next_snapshot(&context, &snapshot, deadline).await? {
                        Some(next) => snapshot = next,
                        None => {
                            // next_snapshot has completed the fresh CPU timeout fence.
                            if with_retirement {
                                return emit_main_wait_deadline(&context, after, self.common.json);
                            }
                            return emit(ordinary.as_ref().expect("ordinary wait counts"), self.common.json);
                        }
                    }
                }
            }
        };
        emit(&context.mutate(action).await?, self.common.json)
    }
}

async fn next_snapshot(
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
async fn wait_next(
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

#[derive(Debug, Args)]
pub(crate) struct SubagentArgs {
    #[command(flatten)]
    common: OrchestrationArgs,
    #[command(subcommand)]
    command: SubagentCommand,
}
#[derive(Debug, Clone, Copy, ValueEnum)]
pub(crate) enum StatusArg {
    Running,
    Done,
    Failed,
    Cancelled,
}
impl From<StatusArg> for SubagentStatus {
    fn from(value: StatusArg) -> Self {
        match value {
            StatusArg::Running => Self::Running,
            StatusArg::Done => Self::Done,
            StatusArg::Failed => Self::Failed,
            StatusArg::Cancelled => Self::Cancelled,
        }
    }
}
#[derive(Debug, Subcommand)]
pub(crate) enum SubagentCommand {
    /// Publish actual OMP subagent lifecycle telemetry, not inferred status.
    Update {
        #[arg(long)]
        id: String,
        #[arg(long)]
        parent: Option<String>,
        #[arg(long)]
        role: Option<String>,
        #[arg(long)]
        label: String,
        #[arg(long, value_enum)]
        status: StatusArg,
        #[arg(long)]
        summary: Option<String>,
    },
    /// Read pending controls for this caller run and subagent ID without marking Read.
    Controls {
        #[arg(long)]
        id: String,
        #[arg(long)]
        wait: bool,
        #[arg(long, default_value_t = 300, value_parser = clap::value_parser!(u64).range(0..=3600))]
        timeout: u64,
    },
    /// Receipt for a control actually applied (or failed) through OMP's own APIs.
    #[command(group(clap::ArgGroup::new("receipt").required(true).args(["applied", "failed"])))]
    ControlDone {
        #[arg(long)]
        seq: u64,
        #[arg(long, conflicts_with = "failed")]
        applied: bool,
        #[arg(long)]
        failed: Option<String>,
    },
    /// Request cancellation of a descendant run's running subagent; not a run cancellation.
    Cancel {
        #[arg(long)]
        run: String,
        #[arg(long)]
        id: String,
    },
    /// Send a real durable control message to a descendant run's running subagent.
    Send {
        #[arg(long)]
        run: String,
        #[arg(long)]
        id: String,
        #[arg(long)]
        text: String,
    },
}
#[derive(Deserialize)]
struct ControlPayload {
    subagent_id: String,
    #[serde(rename = "op")]
    _op: SubagentOp,
}
#[derive(Serialize)]
struct Controls<'a> {
    messages: Vec<&'a Message>,
}
fn pending_controls<'a>(
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
impl SubagentArgs {
    pub async fn run(self) -> Result<(), CliError> {
        let context = Context::open(&self.common, true).await?;
        let action = match self.command {
            SubagentCommand::Update {
                id,
                parent,
                role,
                label,
                status,
                summary,
            } => OrchestrationAction::SubagentUpdate {
                subagent_id: id,
                parent_subagent_id: parent,
                role,
                label,
                status: status.into(),
                summary: summary.map(bounded).transpose()?,
            },
            SubagentCommand::Controls { id, wait, timeout } => {
                if matches!(self.common.agent_kind, Some(AgentKindArg::Subagent))
                    && self.common.subagent_id.as_deref() != Some(&id)
                {
                    return Err(CliError::new(
                        "actor_forbidden",
                        "subagent may only read its own controls",
                    ));
                }
                let mut snapshot = context.snapshot(None).await?;
                let deadline = tokio::time::Instant::now()
                    + Duration::from_secs(if wait { timeout } else { 0 });
                loop {
                    let own = context.own_run(&snapshot)?;
                    let result = pending_controls(&snapshot.messages, &own.run_id, &id)?;
                    if !result.messages.is_empty() {
                        return emit(&result, self.common.json);
                    }
                    if tokio::time::Instant::now() >= deadline {
                        context.check_caller().await?;
                        return emit(&result, self.common.json);
                    }
                    match next_snapshot(&context, &snapshot, deadline).await? {
                        Some(next) => snapshot = next,
                        None => return emit(&result, self.common.json),
                    }
                }
            }
            SubagentCommand::ControlDone {
                seq,
                applied,
                failed,
            } => OrchestrationAction::SubagentControlDone {
                seq,
                applied,
                error: failed.map(bounded).transpose()?,
            },
            SubagentCommand::Cancel { run, id } => OrchestrationAction::SubagentControl {
                run_id: run,
                subagent_id: id,
                op: SubagentOp::Cancel,
            },
            SubagentCommand::Send { run, id, text } => OrchestrationAction::SubagentControl {
                run_id: run,
                subagent_id: id,
                op: SubagentOp::Send {
                    text: bounded(text)?,
                },
            },
        };
        emit(&context.mutate(action).await?, self.common.json)
    }
}

#[derive(Debug, Args)]
pub(crate) struct RouteArgs {
    #[command(flatten)]
    common: OrchestrationArgs,
    #[command(subcommand)]
    command: RouteCommand,
}
#[derive(Debug, Subcommand)]
pub(crate) enum RouteCommand {
    /// Resolve configured routing first, then actual forge-origin matches; never cwd.
    Resolve {
        #[arg(long)]
        artifact: String,
    },
}
impl RouteArgs {
    pub async fn run(self) -> Result<(), CliError> {
        let RouteCommand::Resolve { artifact } = self.command;
        let config = self.common.configuration()?;
        let artifact = cockpit_core::repositories::resolve_artifact(&config, &artifact)?;
        let configured = cockpit_core::orchestration::routing::resolve(&config, &artifact, &[]);
        if configured.source == cockpit_core::orchestration::routing::RouteSource::Configured {
            // Explicit and ambiguous configured mappings both take precedence;
            // neither provider availability nor a local catalog can veto them.
            return emit(&configured, self.common.json);
        }
        let forge = ProjectService::forge_repository_candidates(&config, &artifact).await?;
        let resolution = cockpit_core::orchestration::routing::resolve(&config, &artifact, &forge);
        emit(&resolution, self.common.json)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use clap::Parser;

    #[derive(Debug, Parser)]
    struct TestCli {
        #[command(subcommand)]
        command: TestCommand,
    }
    #[derive(Debug, Subcommand)]
    enum TestCommand {
        Task(TaskArgs),
        Run(RunArgs),
        Inbox(InboxArgs),
        Subagent(SubagentArgs),
        Route(RouteArgs),
    }

    fn parsed_task_action(args: &[&str]) -> Result<OrchestrationAction, CliError> {
        let parsed = TestCli::try_parse_from(
            ["test", "task"].into_iter().chain(args.iter().copied()),
        ).map_err(|error| CliError::usage(error.to_string()))?;
        let TestCommand::Task(args) = parsed.command else { panic!("expected task") };
        args.command.action("root".into())
    }

    #[test]
    fn task_creation_preserves_identity_prose_and_exact_relationship_fences() {
        let id = "08776d1f-6352-4aad-9c6b-48b2094c49b7";
        let action = parsed_task_action(&[
            "create", "--task-id", id, "--title", "Follow up",
            "--description", "Prose\nwith Unicode λ",
            "--depends-on", "first", "--depends-on", "second",
            "--follow-up-of", "source", "--doc-revision", "doc",
            "--source-revision", "source-rev",
        ]).unwrap();
        let encoded = serde_json::to_value(&action).unwrap();
        assert_eq!(encoded["task_id"], id);
        assert_eq!(encoded["description"], "Prose\nwith Unicode λ");
        assert_eq!(encoded["depends_on"], serde_json::json!(["first", "second"]));
        assert_eq!(encoded["follow_up_of"], "source");
        assert_eq!(encoded["expected_doc_revision"], "doc");
        assert_eq!(encoded["source_revision"], "source-rev");
        assert!(encoded.get("body").is_none());

        let generated = parsed_task_action(&["create", "--title", "Independent"]).unwrap();
        let encoded = serde_json::to_value(generated).unwrap();
        let generated_id = encoded["task_id"].as_str().unwrap();
        assert_eq!(uuid::Uuid::parse_str(generated_id).unwrap().get_version_num(), 4);
        assert_eq!(encoded["depends_on"], serde_json::json!([]));
        assert!(encoded["follow_up_of"].is_null());
        let error = task_submission_error(
            CliError::new("caller_mismatch", "durable mutation may already be committed"),
            generated_id,
        );
        assert_eq!(error.code, "caller_mismatch");
        assert!(error.message.contains(&format!("task show {generated_id}")));
        let supplied_error = task_submission_error(CliError::new("unknown", "lost"), id);
        assert!(supplied_error.message.contains(&format!("task show {id}")));

        for args in [
            vec!["create", "--title", "x", "--depends-on", "other"],
            vec!["create", "--title", "x", "--follow-up-of", "source", "--doc-revision", "doc"],
            vec!["create", "--title", "x", "--source-revision", "source-rev"],
        ] {
            assert!(parsed_task_action(&args).is_err());
        }
    }

    #[test]
    fn task_update_distinguishes_omitted_prose_from_explicit_clear_and_rejects_body() {
        let cleared = parsed_task_action(&[
            "update", "task-id", "--revision", "full-rev", "--description", "",
        ]).unwrap();
        assert!(matches!(cleared, OrchestrationAction::TaskUpdate {
            task_id, expected_task_revision, title: None, description: Some(description), ..
        } if task_id == "task-id" && expected_task_revision == "full-rev" && description.is_empty()));
        let title_only = parsed_task_action(&[
            "update", "task-id", "--revision", "rev", "--title", "New",
        ]).unwrap();
        assert!(matches!(title_only, OrchestrationAction::TaskUpdate {
            description: None, title: Some(title), ..
        } if title == "New"));
        assert!(parsed_task_action(&["update", "task-id", "--revision", "rev"]).is_err());
        for command in ["create", "update"] {
            for legacy in ["--body", "--body-file"] {
                let mut args = vec![command];
                if command == "update" {
                    args.extend(["task-id", "--revision", "rev"]);
                }
                args.extend(["--title", "x", legacy, "legacy"]);
                assert!(parsed_task_action(&args).is_err());
            }
        }
    }

    #[test]
    fn prerequisites_replacement_can_clear_and_requires_both_exact_fences() {
        let prefix = ["dependencies-set", "task-id", "--revision", "task-rev"];
        assert!(parsed_task_action(&prefix).is_err());
        let mut args = prefix.to_vec();
        args.extend(["--doc-revision", "doc-rev"]);
        let clear = parsed_task_action(&args).unwrap();
        assert!(matches!(clear, OrchestrationAction::TaskDependenciesSet {
            task_id, expected_task_revision, expected_doc_revision, depends_on, ..
        } if task_id == "task-id" && expected_task_revision == "task-rev"
            && expected_doc_revision == "doc-rev" && depends_on.is_empty()));
        args.extend(["--depends-on", "a", "--depends-on", "b"]);
        let set = parsed_task_action(&args).unwrap();
        assert!(matches!(set, OrchestrationAction::TaskDependenciesSet {
            depends_on, ..
        } if depends_on == ["a", "b"]));
    }

    #[test]
    fn checklist_scope_and_destination_are_explicit_without_document_fences() {
        let check = [
            "step-set-checked", "task-id", "--revision", "task-rev",
            "--step-id", "step-id", "--checked", "false",
        ];
        assert!(parsed_task_action(&check).is_err());
        let mut args = check.to_vec();
        args.extend(["--scope", "subtree"]);
        let encoded = serde_json::to_value(parsed_task_action(&args).unwrap()).unwrap();
        assert_eq!(encoded["checked"], false);
        assert_eq!(encoded["scope"], "subtree");
        assert_eq!(encoded["expected_task_revision"], "task-rev");
        assert!(encoded.get("expected_doc_revision").is_none());
        assert!(parsed_task_action(&[
            "step-set-checked", "task-id", "--revision", "rev",
            "--step-id", "step-id", "--scope", "leaf",
        ]).is_err());
        let leaf = parsed_task_action(&[
            "step-set-checked", "task-id", "--revision", "rev",
            "--step-id", "step-id", "--scope", "leaf", "--checked", "true",
        ]).unwrap();
        assert!(matches!(leaf, OrchestrationAction::TaskStepSetChecked {
            scope: TaskStepScope::Leaf, checked: true, ..
        }));

        let add = parsed_task_action(&[
            "step-add", "task-id", "--revision", "rev", "--step-id", "new-id",
            "--parent-step-id", "parent", "--before-step-id", "sibling", "--title", "New",
        ]).unwrap();
        assert!(matches!(add, OrchestrationAction::TaskStepAdd {
            step_id, parent_step_id: Some(parent), before_step_id: Some(before), title, ..
        } if step_id == "new-id" && parent == "parent" && before == "sibling" && title == "New"));
        let move_to_top = parsed_task_action(&[
            "step-move", "task-id", "--revision", "rev", "--step-id", "existing",
        ]).unwrap();
        assert!(matches!(move_to_top, OrchestrationAction::TaskStepMove {
            parent_step_id: None, before_step_id: None, step_id, ..
        } if step_id == "existing"));
        let rename = parsed_task_action(&[
            "step-rename", "task-id", "--revision", "rev", "--step-id", "existing", "--title", "Renamed",
        ]).unwrap();
        assert!(matches!(rename, OrchestrationAction::TaskStepRename {
            step_id, title, ..
        } if step_id == "existing" && title == "Renamed"));
        let remove = parsed_task_action(&[
            "step-remove", "task-id", "--revision", "rev", "--step-id", "existing",
        ]).unwrap();
        assert!(matches!(remove, OrchestrationAction::TaskStepRemove {
            step_id, expected_task_revision, ..
        } if step_id == "existing" && expected_task_revision == "rev"));
    }

    #[test]
    fn step_adoption_preserves_original_offsets_and_ids_and_rejects_invalid_mapping() {
        let mapping = r#"[{"source_offset":0,"step_id":"first"},{"source_offset":4096,"step_id":"second"}]"#;
        let action = parsed_task_action(&[
            "steps-adopt", "task-id", "--revision", "original-rev", "--mapping", mapping,
        ]).unwrap();
        let OrchestrationAction::TaskStepsAdopt {
            expected_task_revision, mapping, ..
        } = action else { panic!("expected adoption") };
        assert_eq!(expected_task_revision, "original-rev");
        assert_eq!(mapping.len(), 2);
        assert_eq!((mapping[0].source_offset, mapping[0].step_id.as_str()), (0, "first"));
        assert_eq!((mapping[1].source_offset, mapping[1].step_id.as_str()), (4096, "second"));
        for invalid in [
            "[]", "{}", "not JSON",
            r#"[{"source_offset":-1,"step_id":"id"}]"#,
            r#"[{"source_offset":0,"step_id":"id","body":"forbidden"}]"#,
        ] {
            assert!(parsed_task_action(&[
                "steps-adopt", "task-id", "--revision", "rev", "--mapping", invalid,
            ]).is_err());
        }
        let oversized = serde_json::to_string(&vec![
            serde_json::json!({"source_offset":0,"step_id":"id"}); 65
        ]).unwrap();
        assert!(parsed_task_action(&[
            "steps-adopt", "task-id", "--revision", "rev", "--mapping", &oversized,
        ]).is_err());
    }

    #[test]
    fn global_evidence_is_available_after_nested_commands() {
        let parsed = TestCli::try_parse_from([
            "test",
            "run",
            "report",
            "--kind",
            "ready",
            "--message-id",
            "dedupe",
            "--summary",
            "ready",
            "--plan",
            "Exact work plan",
            "--omp-session",
            "native",
            "--agent-kind",
            "main",
            "--herdr-session",
            "fixture",
            "--herdr-socket",
            "/tmp/fixture.sock",
            "--config",
            "/tmp/fixture.toml",
            "--json",
        ])
        .unwrap();
        let TestCommand::Run(args) = parsed.command else {
            panic!("expected run")
        };
        assert_eq!(args.common.omp_session.as_deref(), Some("native"));
        assert!(args.common.json);
        assert_eq!(args.common.config, Some(PathBuf::from("/tmp/fixture.toml")));
    }

    #[test]
    fn operator_only_commands_and_missing_dedupe_are_rejected() {
        for command in ["grant-prepare", "grant-execute", "start"] {
            assert!(TestCli::try_parse_from(["test", "run", command]).is_err());
        }
        assert!(
            TestCli::try_parse_from([
                "test",
                "run",
                "report",
                "--kind",
                "progress",
                "--summary",
                "x"
            ])
            .is_err()
        );
        assert!(
            TestCli::try_parse_from([
                "test",
                "run",
                "message",
                "child",
                "--kind",
                "instruction",
                "--text",
                "x"
            ])
            .is_err()
        );
    }

    #[test]
    fn management_requires_explicit_targets_and_exact_revision_arguments() {
        for command in ["prepare", "execute", "accept", "send-back", "cancel", "reconcile", "retry-launch"] {
            assert!(TestCli::try_parse_from(["test", "run", command]).is_err());
        }
        for command in ["prepare", "execute", "accept", "send-back"] {
            assert!(TestCli::try_parse_from(["test", "run", command, "worker"]).is_err());
        }
        for (command, flag) in [
            ("prepare", "--plan-revision"),
            ("execute", "--plan-revision"),
            ("accept", "--task-revision"),
            ("send-back", "--text"),
        ] {
            assert!(
                TestCli::try_parse_from(["test", "run", command, "worker", flag, "exact"]).is_ok()
            );
            assert!(
                TestCli::try_parse_from([
                    "test",
                    "run",
                    command,
                    "worker",
                    flag,
                    "exact",
                    "--operator"
                ])
                .is_err()
            );
        }
        assert!(
            TestCli::try_parse_from([
                "test",
                "run",
                "prepare",
                "worker",
                "--task-revision",
                "exact"
            ])
            .is_err()
        );
        assert!(
            TestCli::try_parse_from([
                "test",
                "run",
                "accept",
                "worker",
                "--plan-revision",
                "exact"
            ])
            .is_err()
        );
        assert!(TestCli::try_parse_from(["test", "run", "cancel", "worker"]).is_ok());
        assert!(
            TestCli::try_parse_from([
                "test",
                "run",
                "message",
                "worker",
                "--kind",
                "answer",
                "--text",
                "Use the documented checkout.",
                "--message-id",
                "answer-1"
            ])
            .is_ok()
        );
    }

    #[test]
    fn recovery_commands_allow_only_scoped_explicit_worker_actions() {
        for command in ["reconcile", "retry-launch"] {
            assert!(TestCli::try_parse_from(["test", "run", command, "worker"]).is_ok());
            assert!(TestCli::try_parse_from(["test", "run", command, "worker", "--operator"]).is_err());
        }
        let parsed = TestCli::try_parse_from([
            "test", "run", "reconcile", "worker", "--recovery", "accept-existing-worktree",
        ]).unwrap();
        let TestCommand::Run(args) = parsed.command else { panic!("expected run") };
        assert!(matches!(args.command, RunCommand::Reconcile {
            run, recovery: Some(RecoveryArg::AcceptExistingWorktree),
        } if run == "worker"));
        assert!(TestCli::try_parse_from([
            "test", "run", "reconcile", "worker", "--recovery", "retry-environment",
        ]).is_err());
        assert!(TestCli::try_parse_from([
            "test", "run", "retry-launch", "worker", "--recovery", "accept-existing-worktree",
        ]).is_err());
    }

    #[test]
    fn target_and_receipt_groups_are_exclusive() {
        assert!(
            TestCli::try_parse_from([
                "test",
                "run",
                "propose",
                "--task",
                "task",
                "--brief-file",
                "brief",
                "--path",
                "/tmp/a",
                "--space",
                "space"
            ])
            .is_err()
        );
        assert!(
            TestCli::try_parse_from([
                "test",
                "run",
                "propose",
                "--task",
                "task",
                "--brief-file",
                "brief",
                "--path",
                "/tmp/a",
                "--branch",
                "work"
            ])
            .is_err()
        );
        assert!(
            TestCli::try_parse_from(["test", "subagent", "control-done", "--seq", "1"]).is_err()
        );
        assert!(
            TestCli::try_parse_from([
                "test",
                "subagent",
                "control-done",
                "--seq",
                "1",
                "--applied",
                "--failed",
                "failure"
            ])
            .is_err()
        );
    }

    #[test]
    fn proposal_target_group_excludes_task_brief_and_repository_modifiers() {
        let parsed = TestCli::try_parse_from([
            "test",
            "run",
            "propose",
            "--task",
            "task",
            "--repository",
            "repository",
            "--brief",
            "Prepare only",
            "--branch",
            "feature",
            "--base",
            "main",
        ])
        .unwrap();
        let TestCommand::Run(args) = parsed.command else {
            panic!("expected run")
        };
        let RunCommand::Propose(proposal) = args.command else {
            panic!("expected proposal")
        };
        assert_eq!(proposal.task, "task");
        assert_eq!(proposal.repository.as_deref(), Some("repository"));
        assert_eq!(proposal.brief.as_deref(), Some("Prepare only"));
        assert_eq!(proposal.branch.as_deref(), Some("feature"));
        assert_eq!(proposal.base.as_deref(), Some("main"));
        for targets in [
            ["--repository", "repository", "--path", "/tmp/checkout"],
            ["--repository", "repository", "--space", "workspace"],
            ["--path", "/tmp/checkout", "--space", "workspace"],
            ["--space-worktree", "workspace", "--repository", "repository"],
            ["--space-worktree", "workspace", "--path", "/tmp/checkout"],
            ["--space-worktree", "workspace", "--space", "workspace"],
        ] {
            let mut command = vec![
                "test",
                "run",
                "propose",
                "--task",
                "task",
                "--brief",
                "Prepare only",
            ];
            command.extend(targets);
            assert!(TestCli::try_parse_from(command).is_err());
        }
        assert!(
            TestCli::try_parse_from([
                "test",
                "run",
                "propose",
                "--task",
                "task",
                "--brief",
                "Prepare only",
            ])
            .is_err()
        );
    }

    #[test]
    fn project_space_worktree_preserves_branch_base_and_restricts_modifiers() {
        let prefix = ["test", "run", "propose", "--task", "task", "--brief", "Read-only preparation"];
        let mut command = prefix.to_vec();
        command.extend(["--space-worktree", "project", "--branch", "feature", "--base", "main"]);
        let parsed = TestCli::try_parse_from(command).unwrap();
        let TestCommand::Run(args) = parsed.command else { panic!("expected run") };
        let RunCommand::Propose(proposal) = args.command else { panic!("expected proposal") };
        let OrchestrationAction::RunPropose { target, .. } = proposal.action().unwrap() else { panic!("expected proposal action") };
        assert!(matches!(target, DispatchTarget::SpaceWorktree {
            workspace_id, branch: Some(branch), base_ref: Some(base),
        } if workspace_id == "project" && branch == "feature" && base == "main"));
        for target in ["--space", "--path"] {
            for modifier in ["--branch", "--base"] {
                let mut command = prefix.to_vec();
                command.extend([target, "project", modifier, "feature"]);
                assert!(TestCli::try_parse_from(command).is_err());
            }
        }
        for modifier in ["--checkout-path", "--artifact", "--linked-artifact", "--task-name"] {
            let mut command = prefix.to_vec();
            command.extend(["--space-worktree", "project", modifier, "value"]);
            assert!(TestCli::try_parse_from(command).is_err(), "{modifier} must reject --space-worktree");
        }
    }

    #[test]
    fn limits_are_enforced_at_parser_and_input_boundary() {
        assert!(TestCli::try_parse_from(["test", "inbox", "wait", "--timeout", "3601"]).is_err());
        assert!(
            TestCli::try_parse_from([
                "test",
                "subagent",
                "controls",
                "--id",
                "sub",
                "--timeout",
                "3601"
            ])
            .is_err()
        );
        assert!(TestCli::try_parse_from(["test", "inbox", "list", "--limit", "101"]).is_err());
        assert_eq!(
            read_bounded(std::io::Cursor::new(vec![b'a'; MAX_TEXT_BYTES]))
                .unwrap()
                .len(),
            MAX_TEXT_BYTES
        );
        assert_eq!(
            read_bounded(std::io::Cursor::new(vec![b'a'; MAX_TEXT_BYTES + 1]))
                .unwrap_err()
                .code,
            "message_too_large"
        );
        assert!(read_bounded(std::io::Cursor::new(vec![0xff])).is_err());
        assert!(
            TestCli::try_parse_from([
                "test",
                "task",
                "create",
                "--title",
                "x",
                "--stdin",
                "--description-file",
                "description"
            ])
            .is_err()
        );
    }

    #[test]
    fn inherited_attempt_requires_complete_positive_identity() {
        assert!(parse_env_run(None, None).unwrap().is_none());
        assert!(parse_env_run(Some("run".into()), None).is_err());
        assert!(parse_env_run(None, Some("1".into())).is_err());
        assert!(parse_env_run(Some("run".into()), Some("0".into())).is_err());
        assert!(parse_env_run(Some("run".into()), Some("4294967296".into())).is_err());
        assert_eq!(
            parse_env_run(Some("run".into()), Some("2".into())).unwrap(),
            Some(("run".into(), 2))
        );
    }

    fn message(
        run: &str,
        seq: u64,
        kind: MessageKind,
        stage: DeliveryStage,
        text: &str,
    ) -> Message {
        Message {
            message_id: format!("message-{seq}"),
            to_run_id: run.into(),
            seq,
            from: ActorRef::Dispatcher,
            kind,
            text: text.into(),
            report: None,
            stale: false,
            escalated_from: None,
            from_subagent_id: None,
            stage,
            woken_omp_session: None,
            created_at: "2026-10-05T00:00:00Z".into(),
            acked_at: None,
        }
    }

    #[test]
    fn wake_envelope_contains_only_counts_not_bodies_or_other_runs() {
        let messages = vec![
            message(
                "own",
                1,
                MessageKind::Instruction,
                DeliveryStage::Stored,
                "BELOW CURSOR",
            ),
            message(
                "own",
                2,
                MessageKind::Report,
                DeliveryStage::Read,
                "PRIVATE REPORT",
            ),
            message(
                "own",
                3,
                MessageKind::Report,
                DeliveryStage::Woken,
                "PRIVATE REPORT 2",
            ),
            message(
                "own",
                4,
                MessageKind::Instruction,
                DeliveryStage::Acked,
                "ACKED",
            ),
            message(
                "other",
                9,
                MessageKind::Instruction,
                DeliveryStage::Stored,
                "OTHER RUN",
            ),
        ];
        let result = inbox_counts(&messages, "own", 1);
        assert!(result.pending);
        assert_eq!(result.through_seq, 3);
        let value = serde_json::to_value(result).unwrap();
        assert_eq!(
            value["counts"],
            serde_json::json!([{"kind": "report", "count": 2}])
        );
        assert_eq!(value.as_object().unwrap().len(), 4);
        assert!(!value.to_string().contains("PRIVATE"));
        assert!(!inbox_counts(&messages, "own", 3).pending);
        assert_eq!(messages[1].stage, DeliveryStage::Read);
    }

    #[test]
    fn newer_subagent_control_never_advances_ordinary_wake_cursor() {
        let control = r#"{"subagent_id":"child","op":{"op":"send","text":"instruction"}}"#;
        let messages = vec![
            message(
                "own",
                2,
                MessageKind::Instruction,
                DeliveryStage::Stored,
                "ordinary",
            ),
            message(
                "own",
                9,
                MessageKind::SubagentControl,
                DeliveryStage::Stored,
                control,
            ),
        ];
        let ordinary = inbox_counts(&messages, "own", 1);
        assert!(ordinary.pending);
        assert_eq!(ordinary.through_seq, 2);
        assert_eq!(
            serde_json::to_value(ordinary).unwrap()["counts"],
            serde_json::json!([{"kind": "instruction", "count": 1}])
        );
        let after_ordinary = inbox_counts(&messages, "own", 2);
        assert!(!after_ordinary.pending);
        assert_eq!(after_ordinary.through_seq, 2);
        assert!(after_ordinary.counts.is_empty());
        let controls = pending_controls(&messages, "own", "child").unwrap();
        assert_eq!(
            controls
                .messages
                .iter()
                .map(|item| item.seq)
                .collect::<Vec<_>>(),
            vec![9]
        );
        assert_eq!(messages[1].stage, DeliveryStage::Stored);
    }

    #[test]
    fn controls_are_read_only_and_scoped_to_own_run_and_subagent() {
        let own = r#"{"subagent_id":"child","op":{"op":"cancel"}}"#;
        let other = r#"{"subagent_id":"other","op":{"op":"send","text":"message"}}"#;
        let messages = vec![
            message(
                "own",
                4,
                MessageKind::SubagentControl,
                DeliveryStage::Read,
                own,
            ),
            message(
                "own",
                2,
                MessageKind::SubagentControl,
                DeliveryStage::Stored,
                own,
            ),
            message(
                "own",
                3,
                MessageKind::SubagentControl,
                DeliveryStage::Stored,
                other,
            ),
            message(
                "other",
                5,
                MessageKind::SubagentControl,
                DeliveryStage::Stored,
                own,
            ),
            message(
                "own",
                6,
                MessageKind::SubagentControl,
                DeliveryStage::Acked,
                own,
            ),
            message(
                "own",
                7,
                MessageKind::Instruction,
                DeliveryStage::Stored,
                "not JSON",
            ),
        ];
        let result = pending_controls(&messages, "own", "child").unwrap();
        assert_eq!(
            result
                .messages
                .iter()
                .map(|item| item.seq)
                .collect::<Vec<_>>(),
            vec![2, 4]
        );
        assert_eq!(
            serde_json::to_value(result)
                .unwrap()
                .as_object()
                .unwrap()
                .len(),
            1
        );
        assert_eq!(messages[0].stage, DeliveryStage::Read);
        assert_eq!(messages[1].stage, DeliveryStage::Stored);
    }

    #[test]
    fn moved_caller_requires_same_native_terminal_session_boot_and_endpoint() {
        let mut caller = AgentCaller {
            endpoint_identity: "endpoint".into(),
            session_id: "fixture".into(),
            workspace_id: "new-space".into(),
            tab_id: "new-tab".into(),
            pane_id: "new-pane".into(),
            boot_id: Some("boot".into()),
            terminal_id: Some("terminal".into()),
            native_session_id: Some("main".into()),
            actual_agent_kind: Some("omp".into()),
            env_run: Some(("run".into(), 1)),
            omp_session_id: Some("child-native".into()),
            main_omp_session_id: Some("main".into()),
            agent_kind: Some(AgentKind::Subagent),
            subagent_id: Some("child".into()),
            process: None,
        };
        let mut location = RunLocation {
            endpoint_identity: "endpoint".into(),
            session_id: "fixture".into(),
            workspace_id: "old-space".into(),
            tab_id: "old-tab".into(),
            pane_id: "old-pane".into(),
            launch_tag: "tag".into(),
            boot_id: Some("boot".into()),
            terminal_id: Some("terminal".into()),
            native_session_id: Some("main".into()),
        };
        assert!(caller_location_matches(&location, Some("main"), &caller));
        // Herdr 0.9.3 omits boot/native session fields: absence is not disagreement.
        caller.boot_id = None;
        caller.native_session_id = None;
        assert!(caller_location_matches(&location, Some("main"), &caller));
        location.boot_id = None;
        location.native_session_id = None;
        assert!(caller_location_matches(&location, Some("main"), &caller));
        caller.main_omp_session_id = Some("foreign-main".into());
        assert!(!caller_location_matches(&location, Some("main"), &caller));
        caller.main_omp_session_id = Some("main".into());
        caller.boot_id = Some("boot".into());
        location.boot_id = Some("boot".into());
        caller.native_session_id = Some("main".into());
        location.native_session_id = Some("main".into());
        caller.native_session_id = Some("other-session".into());
        assert!(!caller_location_matches(&location, Some("main"), &caller));
        caller.native_session_id = Some("main".into());
        caller.boot_id = Some("restarted".into());
        assert!(!caller_location_matches(&location, Some("main"), &caller));
        caller.boot_id = Some("boot".into());
        caller.pane_id = location.pane_id.clone();
        caller.terminal_id = Some("replacement-terminal".into());
        assert!(!caller_location_matches(&location, Some("main"), &caller));
    }

    #[test]
    fn literal_inputs_and_main_session_evidence_are_discoverable() {
        assert!(
            TestCli::try_parse_from([
                "test",
                "task",
                "create",
                "--title",
                "Task",
                "--description",
                "Literal description"
            ])
            .is_ok()
        );
        assert!(
            TestCli::try_parse_from([
                "test",
                "run",
                "propose",
                "--task",
                "task",
                "--space",
                "workspace",
                "--brief",
                "Prepare only"
            ])
            .is_ok()
        );
        let parsed = TestCli::try_parse_from([
            "test",
            "subagent",
            "controls",
            "--id",
            "child",
            "--wait",
            "--agent-kind",
            "subagent",
            "--subagent-id",
            "child",
            "--omp-session",
            "child-native",
            "--omp-main-session",
            "root-native",
        ])
        .unwrap();
        let TestCommand::Subagent(args) = parsed.command else {
            panic!("expected subagent")
        };
        assert_eq!(args.common.omp_main_session.as_deref(), Some("root-native"));
        assert!(args.common.validate_identity().is_ok());
        let help = TestCli::try_parse_from(["test", "run", "propose", "--help"]).unwrap_err();
        assert_eq!(help.kind(), clap::error::ErrorKind::DisplayHelp);
        assert!(help.to_string().contains("--brief"));
        let help = TestCli::try_parse_from(["test", "subagent", "--help"])
            .unwrap_err()
            .to_string();
        assert!(help.contains("control-done"));
        assert!(help.contains("cancel"));
        assert!(help.contains("send"));
    }

    fn retirement_fixture() -> (Run, AgentCaller) {
        let process = NativeProcessIdentity {
            pid: 1234,
            start_ticks: 5678,
            kernel_boot_id: Some("kernel".into()),
        };
        let caller = AgentCaller {
            endpoint_identity: "endpoint".into(),
            session_id: "fixture".into(),
            workspace_id: "space".into(),
            tab_id: "tab".into(),
            pane_id: "pane".into(),
            boot_id: Some("boot".into()),
            terminal_id: Some("terminal".into()),
            native_session_id: Some("main".into()),
            actual_agent_kind: Some("omp".into()),
            env_run: Some(("worker".into(), 2)),
            omp_session_id: Some("main".into()),
            main_omp_session_id: None,
            agent_kind: Some(AgentKind::Main),
            subagent_id: None,
            process: Some(process.clone()),
        };
        let run = Run {
            session_id: "fixture".into(),
            prepare_brief: String::new(),
            run_id: "worker".into(),
            kind: RunKind::Worker,
            label: "worker".into(),
            root_id: "root".into(),
            parent_run_id: Some("root".into()),
            task_id: Some("task".into()),
            attempt: 2,
            task_revision_at_propose: None,
            stage: RunStage::Working,
            close_reason: None,
            dispatch: Some(DispatchState {
                launch_tag: Some("launch".into()),
                endpoint_identity: Some("endpoint".into()),
                recovery: None,
                agent_started: true,
                step: DispatchStep::Launched,
                launch_attempt: 3,
                error: None,
                updated_at: "2026-10-07T00:00:00Z".into(),
            }),
            target: None,
            setup: None,
            prepare_plan: None,
            init_receipt: None,
            work_plan: None,
            grants: vec![],
            last_report: None,
            result: None,
            annotations: vec![],
            location: Some(RunLocation {
                boot_id: caller.boot_id.clone(),
                terminal_id: caller.terminal_id.clone(),
                native_session_id: caller.native_session_id.clone(),
                endpoint_identity: caller.endpoint_identity.clone(),
                session_id: caller.session_id.clone(),
                workspace_id: caller.workspace_id.clone(),
                tab_id: caller.tab_id.clone(),
                pane_id: caller.pane_id.clone(),
                launch_tag: "launch".into(),
            }),
            bound_omp_session: Some("main".into()),
            bound_omp_process: Some(process),
            launch_shell_identity: Some(NativeShellIdentity {
                process: NativeProcessIdentity {
                    pid: 4321,
                    start_ticks: 5000,
                    kernel_boot_id: Some("kernel".into()),
                },
                executable_device: "40".into(),
                executable_inode: "800".into(),
                argv_digest: "a".repeat(64),
            }),
            retirement: None,
            supersedes_run_id: None,
            created_at: "2026-10-07T00:00:00Z".into(),
            updated_at: "2026-10-07T00:00:00Z".into(),
        };
        (run, caller)
    }

    fn accept_retirement(run: &mut Run, caller: &AgentCaller) {
        run.stage = RunStage::Closed;
        run.close_reason = Some(CloseReason::Accepted);
        run.retirement = Some(RunRetirement {
            retirement_id: "retirement".into(),
            trigger: RetirementTrigger::Accept,
            result_message_id: "result".into(),
            task_revision: "revision".into(),
            identity: Some(RetirementIdentity {
                run_attempt: run.attempt,
                launch_attempt: 3,
                launch_tag: "launch".into(),
                endpoint_identity: caller.endpoint_identity.clone(),
                session_id: caller.session_id.clone(),
                workspace_id: caller.workspace_id.clone(),
                tab_id: caller.tab_id.clone(),
                pane_id: caller.pane_id.clone(),
                terminal_id: caller.terminal_id.clone().unwrap(),
                herdr_boot_id: caller.boot_id.clone(),
                omp_session_id: "main".into(),
                process: caller.process.clone().unwrap(),
                shell: run.launch_shell_identity.clone().unwrap(),
            }),
            state: RetirementState::NativeStopOffered {
                offered_at: "2026-10-07T00:00:01Z".into(),
            },
            created_at: "2026-10-07T00:00:01Z".into(),
            updated_at: "2026-10-07T00:00:01Z".into(),
        });
    }

    #[test]
    fn retirement_scope_survives_only_own_exact_acceptance() {
        let (mut run, caller) = retirement_fixture();
        retirement_read_scope(&run, &caller).unwrap();
        assert!(run.retirement.is_none());
        run.stage = RunStage::Reported;
        retirement_read_scope(&run, &caller).unwrap();
        accept_retirement(&mut run, &caller);
        retirement_read_scope(&run, &caller).unwrap();
        for reason in [CloseReason::Cancelled, CloseReason::Superseded, CloseReason::Failed] {
            run.close_reason = Some(reason);
            assert!(retirement_read_scope(&run, &caller).is_err());
        }
        run.close_reason = Some(CloseReason::Accepted);
        run.retirement = None;
        assert!(retirement_read_scope(&run, &caller).is_err());
    }

    #[test]
    fn retirement_scope_retries_only_exact_unattested_startup() {
        let (mut run, mut caller) = retirement_fixture();
        run.stage = RunStage::Preparing;
        run.dispatch.as_mut().unwrap().step = DispatchStep::LaunchPending;
        run.dispatch.as_mut().unwrap().agent_started = false;
        caller.actual_agent_kind = None;
        for kind in [RunKind::Supervisor, RunKind::Worker] {
            run.kind = kind;
            for step in [
                DispatchStep::LaunchIntent,
                DispatchStep::LaunchPending,
                DispatchStep::LaunchUnknown,
            ] {
                run.dispatch.as_mut().unwrap().step = step;
                assert_eq!(
                    retirement_read_scope(&run, &caller).unwrap_err().code,
                    "caller_not_ready"
                );
            }
        }
        let changed_callers: &[(&str, fn(&mut AgentCaller))] = &[
            ("caller_mismatch", |c| {
                c.env_run = Some(("replacement".into(), 2))
            }),
            ("attempt_stale", |c| c.env_run.as_mut().unwrap().1 += 1),
            ("caller_mismatch", |c| {
                c.endpoint_identity = "replacement".into()
            }),
            ("caller_mismatch", |c| c.pane_id = "replacement".into()),
            ("caller_mismatch", |c| {
                c.terminal_id = Some("replacement".into())
            }),
            ("caller_mismatch", |c| {
                c.native_session_id = Some("replacement".into())
            }),
            ("caller_mismatch", |c| c.process.as_mut().unwrap().pid += 1),
            ("caller_mismatch", |c| {
                c.process.as_mut().unwrap().start_ticks += 1
            }),
            ("caller_mismatch", |c| {
                c.process.as_mut().unwrap().kernel_boot_id = Some("replacement".into())
            }),
            ("session_mismatch", |c| {
                c.omp_session_id = Some("replacement".into())
            }),
            ("caller_mismatch", |c| {
                c.actual_agent_kind = Some("other".into())
            }),
        ];
        for (code, change) in changed_callers {
            let mut changed = caller.clone();
            change(&mut changed);
            assert_eq!(
                retirement_read_scope(&run, &changed).unwrap_err().code,
                *code,
                "{changed:?}"
            );
        }
        let changed_runs: &[(&str, fn(&mut Run))] = &[
            ("session_mismatch", |r| r.bound_omp_session = None),
            ("caller_mismatch", |r| {
                r.bound_omp_process.as_mut().unwrap().start_ticks += 1
            }),
            ("caller_mismatch", |r| {
                r.dispatch.as_mut().unwrap().endpoint_identity = Some("replacement".into())
            }),
            ("caller_mismatch", |r| {
                r.dispatch.as_mut().unwrap().launch_tag = Some("replacement".into())
            }),
            ("caller_mismatch", |r| {
                r.dispatch.as_mut().unwrap().agent_started = true
            }),
            ("caller_mismatch", |r| {
                r.dispatch.as_mut().unwrap().step = DispatchStep::Launched
            }),
            ("caller_mismatch", |r| r.dispatch = None),
            ("caller_mismatch", |r| r.kind = RunKind::Adopted),
            ("caller_mismatch", |r| r.stage = RunStage::Initializing),
            ("caller_mismatch", |r| r.stage = RunStage::Active),
            ("caller_mismatch", |r| r.stage = RunStage::Working),
            ("caller_mismatch", |r| r.stage = RunStage::Reported),
        ];
        for (code, change) in changed_runs {
            let mut changed = run.clone();
            change(&mut changed);
            assert_eq!(
                retirement_read_scope(&changed, &caller).unwrap_err().code,
                *code,
                "{changed:?}"
            );
        }
        caller.actual_agent_kind = Some("omp".into());
        retirement_read_scope(&run, &caller).unwrap();
        let mut accepted = run.clone();
        accept_retirement(&mut accepted, &caller);
        caller.actual_agent_kind = None;
        assert_eq!(
            retirement_read_scope(&accepted, &caller).unwrap_err().code,
            "caller_mismatch"
        );
        run.retirement = accepted.retirement;
        assert_eq!(
            retirement_read_scope(&run, &caller).unwrap_err().code,
            "caller_mismatch"
        );
    }

    #[test]
    fn retirement_scope_rejects_foreign_or_replaced_native_authority() {
        let (mut run, caller) = retirement_fixture();
        let live = run.clone();
        accept_retirement(&mut run, &caller);
        let mutations: &[fn(&mut AgentCaller)] = &[
            |c| c.env_run = Some(("sibling".into(), 2)),
            |c| c.env_run = Some(("worker".into(), 1)),
            |c| c.env_run = None,
            |c| c.agent_kind = Some(AgentKind::Subagent),
            |c| c.subagent_id = Some("child".into()),
            |c| c.omp_session_id = Some("foreign".into()),
            |c| c.process.as_mut().unwrap().pid += 1,
            |c| c.process.as_mut().unwrap().start_ticks += 1,
            |c| c.process.as_mut().unwrap().kernel_boot_id = Some("new-kernel".into()),
            |c| c.process = None,
            |c| c.endpoint_identity = "new-endpoint".into(),
            |c| c.session_id = "other-fixture".into(),
            |c| c.workspace_id = "other-space".into(),
            |c| c.tab_id = "other-tab".into(),
            |c| c.pane_id = "other-pane".into(),
            |c| c.terminal_id = Some("other-terminal".into()),
            |c| c.boot_id = Some("new-boot".into()),
            |c| c.native_session_id = Some("other-native".into()),
            |c| c.actual_agent_kind = None,
        ];
        for mutate in mutations {
            let mut foreign = caller.clone();
            mutate(&mut foreign);
            assert!(retirement_read_scope(&run, &foreign).is_err(), "{foreign:?}");
            assert!(retirement_read_scope(&live, &foreign).is_err(), "{foreign:?}");
        }
        run.dispatch.as_mut().unwrap().launch_attempt += 1;
        assert!(retirement_read_scope(&run, &caller).is_err());
        run.dispatch.as_mut().unwrap().launch_attempt -= 1;
        run.dispatch.as_mut().unwrap().launch_tag = Some("new-launch".into());
        assert!(retirement_read_scope(&run, &caller).is_err());
        run.dispatch.as_mut().unwrap().launch_tag = Some("launch".into());
        run.dispatch.as_mut().unwrap().endpoint_identity = Some("new-endpoint".into());
        assert!(retirement_read_scope(&run, &caller).is_err());
        run.dispatch.as_mut().unwrap().endpoint_identity = Some("endpoint".into());
        run.retirement.as_mut().unwrap().identity.as_mut().unwrap().process.start_ticks += 1;
        assert!(retirement_read_scope(&run, &caller).is_err());
    }

    #[test]
    fn accepted_retirement_read_is_fenced_to_immutable_identity_not_current_binding_alone() {
        let (mut run, caller) = retirement_fixture();
        accept_retirement(&mut run, &caller);
        let mutations: &[fn(&mut RetirementIdentity)] = &[
            |i| i.run_attempt += 1,
            |i| i.launch_attempt += 1,
            |i| i.launch_tag = "other-launch".into(),
            |i| i.endpoint_identity = "other-endpoint".into(),
            |i| i.session_id = "other-session".into(),
            |i| i.workspace_id = "other-space".into(),
            |i| i.tab_id = "other-tab".into(),
            |i| i.pane_id = "other-pane".into(),
            |i| i.terminal_id = "other-terminal".into(),
            |i| i.herdr_boot_id = Some("other-boot".into()),
            |i| i.omp_session_id = "other-main".into(),
            |i| i.process.start_ticks += 1,
        ];
        for mutate in mutations {
            let mut changed = run.clone();
            mutate(changed.retirement.as_mut().unwrap().identity.as_mut().unwrap());
            assert!(retirement_read_scope(&changed, &caller).is_err());
        }
        run.retirement.as_mut().unwrap().identity = None;
        assert!(retirement_read_scope(&run, &caller).is_err());
    }

    #[test]
    fn native_pid_evidence_rejects_self_and_unrelated_live_child() {
        assert!(native_process_evidence(std::process::id()).is_err());
        assert!(native_process_evidence(0).is_err());
        assert!(native_process_evidence(u32::MAX).is_err());
        let mut child = std::process::Command::new("sleep").arg("30").spawn().unwrap();
        let rejected = native_process_evidence(child.id()).is_err();
        child.kill().unwrap();
        child.wait().unwrap();
        assert!(rejected);
    }

    #[test]
    fn retirement_receipt_requires_one_typed_bounded_outcome() {
        let parsed = TestCli::try_parse_from([
            "test", "run", "retirement", "--omp-pid", "1234", "--wait", "--timeout", "30",
        ]).unwrap();
        let TestCommand::Run(args) = parsed.command else { panic!("expected run") };
        assert_eq!(args.common.omp_pid, Some(1234));
        assert!(matches!(args.command, RunCommand::Retirement { wait: true, timeout: 30 }));
        for reason in ["busy", "pending_messages", "async_jobs", "live_subagents", "editor_draft"] {
            assert!(TestCli::try_parse_from([
                "test", "run", "retirement-receipt", "--retirement", "id", "--deferred", reason,
            ]).is_ok());
        }
        for reason in ["user_activity", "native_refused"] {
            assert!(TestCli::try_parse_from([
                "test", "run", "retirement-receipt", "--retirement", "id",
                "--refused", "Explanation", "--refuse-reason", reason,
            ]).is_ok());
        }
        for flags in [
            vec![],
            vec!["--shutdown-requested", "--deferred", "busy"],
            vec!["--refused", "user_activity"],
            vec!["--refuse-reason", "native_refused"],
        ] {
            let mut argv = vec!["test", "run", "retirement-receipt", "--retirement", "id"];
            argv.extend(flags);
            assert!(TestCli::try_parse_from(argv).is_err());
        }
        let action = RetirementReceiptArgs {
            retirement: "id".into(),
            shutdown_requested: false,
            deferred: None,
            refused: Some("user_activity appears here but has no authority".into()),
            refuse_reason: Some(NativeRefuseReasonArg::NativeRefused),
        }.action().unwrap();
        assert!(matches!(action, OrchestrationAction::RetirementNativeReceipt {
            outcome: NativeStopReceipt::Refused { reason: NativeRefuseReason::NativeRefused, .. }, ..
        }));
        assert!(RetirementReceiptArgs {
            retirement: "id".into(),
            shutdown_requested: false,
            deferred: None,
            refused: Some("é".repeat(513)),
            refuse_reason: Some(NativeRefuseReasonArg::UserActivity),
        }.action().is_err());
    }

    #[test]
    fn retirement_wait_tracks_acceptance_and_never_spins_on_unchanged_offer() {
        let (mut run, caller) = retirement_fixture();
        assert!(!retirement_wait_ready(None, run.retirement.as_ref()));
        run.stage = RunStage::Reported;
        assert!(!retirement_wait_ready(None, run.retirement.as_ref()));
        accept_retirement(&mut run, &caller);
        assert!(retirement_wait_ready(None, run.retirement.as_ref()));
        let offered = run.retirement.clone().unwrap();
        assert!(!retirement_wait_ready(Some(&offered), Some(&offered)));
        let mut metadata_only = offered.clone();
        metadata_only.updated_at = "2026-10-07T00:00:02Z".into();
        assert!(retirement_wait_ready(Some(&offered), Some(&metadata_only)));
        let mut deferred = offered.clone();
        deferred.state = RetirementState::NativeStopDeferred {
            offered_at: "2026-10-07T00:00:01Z".into(),
            reason: NativeDeferReason::Busy,
            at: "2026-10-07T00:00:01Z".into(),
        };
        // Same timestamp does not hide a real state change.
        assert!(retirement_wait_ready(Some(&offered), Some(&deferred)));
        assert!(!retirement_wait_ready(Some(&deferred), Some(&deferred)));
        deferred.state = RetirementState::Unknown {
            at: "2026-10-07T00:00:02Z".into(),
            phase: RetirementPhase::NativeStop,
            detail: "stop not confirmed".into(),
        };
        assert!(retirement_wait_ready(Some(&deferred), Some(&deferred)));
    }

    #[tokio::test]
    async fn retirement_commands_reject_missing_native_main_before_endpoint_access() {
        for mut argv in [
            vec!["test", "run", "retirement"],
            vec!["test", "run", "retirement-receipt", "--retirement", "id", "--shutdown-requested"],
        ] {
            let TestCommand::Run(args) = TestCli::try_parse_from(&argv).unwrap().command else {
                panic!("expected run");
            };
            assert_eq!(args.run().await.unwrap_err().code, "orchestration_usage");
            argv.extend(["--omp-pid", "1234", "--agent-kind", "main"]);
            let TestCommand::Run(args) = TestCli::try_parse_from(argv).unwrap().command else {
                panic!("expected run");
            };
            assert_eq!(args.run().await.unwrap_err().code, "orchestration_usage");
        }
    }

    // All authority in these tests comes through the real Unix-socket adapter.
    // Gates select read phases, not permanent RPC-count or cadence assertions.
    struct ReadGate {
        reached: tokio::sync::oneshot::Sender<()>,
        response: tokio::sync::oneshot::Receiver<Option<serde_json::Value>>,
    }

    struct SocketFixture {
        root: PathBuf,
        configuration: ProjectConfiguration,
        context: Arc<Context>,
        payload: Arc<parking_lot::Mutex<serde_json::Value>>,
        gates: Arc<parking_lot::Mutex<std::collections::VecDeque<Option<ReadGate>>>>,
        server: tokio::task::JoinHandle<()>,
    }

    fn socket_payload() -> serde_json::Value {
        let mut payload: serde_json::Value = serde_json::from_str(include_str!(
            "../../cockpit-herdr/tests/fixtures/session-snapshot.json"
        )).unwrap();
        let snapshot = &mut payload["result"]["snapshot"];
        snapshot["boot_id"] = serde_json::json!("boot-one");
        snapshot["panes"][0]["agent"] = serde_json::json!("omp");
        snapshot["agents"][0]["agent"] = serde_json::json!("omp");
        snapshot["agents"][0]["agent_session"] =
            serde_json::json!({"kind": "id", "value": "main-native"});
        payload["result"].clone()
    }

    impl SocketFixture {
        async fn new() -> Self {
            use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
            let root = std::env::temp_dir().join(format!("ck-host-wait-{}", uuid::Uuid::new_v4()));
            std::fs::create_dir_all(&root).unwrap();
            let socket = root.join("herdr.sock");
            let listener = tokio::net::UnixListener::bind(&socket).unwrap();
            let payload = Arc::new(parking_lot::Mutex::new(socket_payload()));
            let gates = Arc::new(parking_lot::Mutex::new(std::collections::VecDeque::<Option<ReadGate>>::new()));
            let server_payload = payload.clone();
            let server_gates = gates.clone();
            let server = tokio::spawn(async move {
                let mut handlers = tokio::task::JoinSet::new();
                loop {
                    tokio::select! {
                        connection = listener.accept() => {
                            let (stream, _) = connection.unwrap();
                            let payload = server_payload.clone();
                            let gates = server_gates.clone();
                            handlers.spawn(async move {
                                let mut reader = BufReader::new(stream);
                                let mut line = String::new();
                                if reader.read_line(&mut line).await.unwrap_or(0) == 0 {
                                    return;
                                }
                                let request: serde_json::Value = serde_json::from_str(&line).unwrap();
                                let result = if request["method"] == "ping" {
                                    serde_json::json!({"type": "pong", "version": "0.9.0", "protocol": 22})
                                } else {
                                    assert_eq!(request["method"], "session.snapshot");
                                    let gate = gates.lock().pop_front().flatten();
                                    if let Some(gate) = gate {
                                        let _ = gate.reached.send(());
                                        match gate.response.await {
                                            Ok(Some(result)) => result,
                                            _ => return,
                                        }
                                    } else {
                                        payload.lock().clone()
                                    }
                                };
                                let response = format!("{}\n", serde_json::json!({
                                    "id": request["id"], "result": result,
                                }));
                                // Cancellation deliberately closes some gated sockets.
                                let _ = reader.into_inner().write_all(response.as_bytes()).await;
                            });
                        }
                        _ = handlers.join_next(), if !handlers.is_empty() => {}
                    }
                }
            });
            let configuration: ProjectConfiguration = serde_json::from_value(serde_json::json!({
                "version": 1, "repository_roots": [],
                "worktree_root": root.join("worktrees"),
                "companion_root": root.join("unused-companions"),
                "state_root": root.join("state"), "cache_root": root.join("cache"),
                "library_root": root.join("library"), "notes_root": root.join("notes"),
                "branch_template": "test/{task}", "checkout_template": "{task}",
                "providers": [], "origins": {},
                "limits": {
                    "catalog_depth": 1, "catalog_entries": 1,
                    "git_timeout_ms": 1000, "git_output_bytes": 1024,
                    "operation_timeout_ms": 1000,
                    "context_preview_bytes": 1024, "context_preview_lines": 10,
                    "context_directory_entries": 10, "context_tree_depth": 1,
                    "library_folder_files": 10, "library_folder_bytes": 1024,
                    "library_file_bytes": 1024, "library_space_pages": 10,
                    "library_attachment_bytes": 1024,
                    "library_item_attachment_bytes": 1024, "library_max_items": 10
                }
            })).unwrap();
            let adapter = Arc::new(HerdrCliAdapter::new(
                cockpit_herdr::HerdrCliConfig::from_options(
                    None, Some("fixture".into()), Some(socket),
                ).unwrap(),
            ));
            let evidence = adapter.source_adapter()
                .source_pane_evidence("fixture", "pane-a").await.unwrap();
            let runtime = adapter.runtime("fixture").await.unwrap();
            let pane = &runtime.panes[0];
            let actor = Actor::Agent(AgentCaller {
                endpoint_identity: evidence.endpoint_identity.clone(),
                session_id: "fixture".into(), workspace_id: evidence.workspace_id.clone(),
                tab_id: evidence.tab_id.clone(), pane_id: evidence.pane_id.clone(),
                boot_id: runtime.boot_id.clone(), terminal_id: pane.terminal_id.clone(),
                native_session_id: pane.native_session_id.clone(),
                actual_agent_kind: pane.agent_kind.clone(), env_run: None,
                omp_session_id: Some("main-native".into()), main_omp_session_id: None,
                agent_kind: Some(AgentKind::Main), subagent_id: None, process: None,
            });
            let context = Arc::new(Context {
                service: OrchestrationService::open(&configuration).unwrap(),
                adapter, session: "fixture".into(), actor: Some(actor), evidence: Some(evidence),
            });
            Self { root, configuration, context, payload, gates, server }
        }

        fn bytes(&self) -> Option<Vec<u8>> {
            match std::fs::read(self.root.join("state/orchestration/state.json")) {
                Ok(bytes) => Some(bytes),
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => None,
                Err(error) => panic!("{error}"),
            }
        }

        fn gate(&self, preceding_reads: usize) -> (
            tokio::sync::oneshot::Receiver<()>,
            tokio::sync::oneshot::Sender<Option<serde_json::Value>>,
        ) {
            let (reached, ready) = tokio::sync::oneshot::channel();
            let (response, receive) = tokio::sync::oneshot::channel();
            let mut gates = self.gates.lock();
            gates.extend((0..preceding_reads).map(|_| None));
            gates.push_back(Some(ReadGate { reached, response: receive }));
            (ready, response)
        }

        async fn adopt(&self) -> String {
            let response = self.context.mutate(OrchestrationAction::RunAdopt {
                label: "Socket-backed host test".into(),
            }).await.unwrap();
            let OrchestrationActionResult::Run { run_id, .. } = response.result else {
                panic!("expected adopted run");
            };
            run_id
        }

        fn operator(&self, action: OrchestrationAction) {
            let revision = self.bytes().map(|bytes| {
                serde_json::from_slice::<serde_json::Value>(&bytes).unwrap()["revision"]
                    .as_u64().unwrap()
            }).unwrap_or(0);
            self.context.service.mutate(
                &Actor::Operator(OperatorOrigin::Browser),
                OrchestrationMutationRequest {
                    session_id: "fixture".into(), expected_revision: Some(revision), action,
                },
            ).unwrap();
        }

        fn send(&self, run_id: &str, id: &str) {
            self.operator(OrchestrationAction::MessageSend {
                message_id: id.into(), to_run_id: run_id.into(),
                kind: MessageKind::Instruction, text: id.into(),
            });
        }
    }

    impl Drop for SocketFixture {
        fn drop(&mut self) {
            self.server.abort();
            let _ = std::fs::remove_dir_all(&self.root);
        }
    }

    #[tokio::test]
    async fn caller_identity_and_membership_fail_before_any_durable_mutation() {
        let fixture = SocketFixture::new().await;
        let original = fixture.payload.lock().clone();
        let changes: &[(&str, fn(&mut serde_json::Value))] = &[
            ("absent pane", |v| v["snapshot"]["panes"] = serde_json::json!([])),
            ("absent tab", |v| v["snapshot"]["tabs"] = serde_json::json!([])),
            ("absent workspace", |v| v["snapshot"]["workspaces"] = serde_json::json!([])),
            ("wrong workspace", |v| v["snapshot"]["panes"][0]["workspace_id"] = serde_json::json!("foreign")),
            ("duplicate pane", |v| {
                let item = v["snapshot"]["panes"][0].clone();
                v["snapshot"]["panes"].as_array_mut().unwrap().push(item);
            }),
            ("duplicate tab", |v| {
                let item = v["snapshot"]["tabs"][0].clone();
                v["snapshot"]["tabs"].as_array_mut().unwrap().push(item);
            }),
            ("duplicate workspace", |v| {
                let item = v["snapshot"]["workspaces"][0].clone();
                v["snapshot"]["workspaces"].as_array_mut().unwrap().push(item);
            }),
            ("agent membership", |v| v["snapshot"]["agents"][0]["tab_id"] = serde_json::json!("foreign")),
            ("terminal", |v| v["snapshot"]["panes"][0]["terminal_id"] = serde_json::json!("replacement")),
            ("missing terminal", |v| { v["snapshot"]["panes"][0].as_object_mut().unwrap().remove("terminal_id"); }),
            ("boot", |v| v["snapshot"]["boot_id"] = serde_json::json!("replacement")),
            ("native session", |v| v["snapshot"]["agents"][0]["agent_session"]["value"] = serde_json::json!("replacement")),
            ("kind", |v| v["snapshot"]["agents"][0]["agent"] = serde_json::json!("other")),
            ("launch pending", |v| v["snapshot"]["agents"][0]["launch_pending"] = serde_json::json!(true)),
            ("moved pane", |v| {
                v["snapshot"]["panes"][0]["pane_id"] = serde_json::json!("pane-b");
                v["snapshot"]["agents"][0]["pane_id"] = serde_json::json!("pane-b");
                v["snapshot"]["layouts"][0]["panes"][0]["pane_id"] = serde_json::json!("pane-b");
                v["snapshot"]["layouts"][0]["focused_pane_id"] = serde_json::json!("pane-b");
                v["snapshot"]["focused_pane_id"] = serde_json::json!("pane-b");
            }),
            ("moved tab", |v| {
                v["snapshot"]["tabs"][0]["tab_id"] = serde_json::json!("tab-b");
                v["snapshot"]["panes"][0]["tab_id"] = serde_json::json!("tab-b");
                v["snapshot"]["agents"][0]["tab_id"] = serde_json::json!("tab-b");
                v["snapshot"]["layouts"][0]["tab_id"] = serde_json::json!("tab-b");
                v["snapshot"]["focused_tab_id"] = serde_json::json!("tab-b");
            }),
            ("moved workspace", |v| {
                v["snapshot"]["workspaces"][0]["workspace_id"] = serde_json::json!("space-b");
                v["snapshot"]["tabs"][0]["workspace_id"] = serde_json::json!("space-b");
                v["snapshot"]["panes"][0]["workspace_id"] = serde_json::json!("space-b");
                v["snapshot"]["agents"][0]["workspace_id"] = serde_json::json!("space-b");
                v["snapshot"]["layouts"][0]["workspace_id"] = serde_json::json!("space-b");
                v["snapshot"]["focused_workspace_id"] = serde_json::json!("space-b");
            }),
        ];
        let before = fixture.bytes();
        for (name, change) in changes {
            let mut replacement = original.clone();
            change(&mut replacement);
            *fixture.payload.lock() = replacement;
            assert!(fixture.context.mutate(OrchestrationAction::RunAdopt {
                label: "Must not commit".into(),
            }).await.is_err(), "{name}");
            assert_eq!(fixture.bytes(), before, "{name}");
        }
        *fixture.payload.lock() = original;
        fixture.adopt().await;
    }

    #[tokio::test]
    async fn postcommit_replacement_reports_uncertainty_and_preserves_committed_write() {
        let fixture = SocketFixture::new().await;
        let (ready, respond) = fixture.gate(1); // mutation's distinct postcheck
        let context = fixture.context.clone();
        let mutation = tokio::spawn(async move {
            context.mutate(OrchestrationAction::RunAdopt { label: "Committed once".into() }).await
        });
        ready.await.unwrap();
        let committed = fixture.bytes().unwrap();
        let mut replacement = fixture.payload.lock().clone();
        replacement["snapshot"]["panes"][0]["terminal_id"] = serde_json::json!("replacement");
        respond.send(Some(replacement)).unwrap();
        let error = mutation.await.unwrap().unwrap_err();
        assert_eq!(error.code, "caller_mismatch");
        assert!(error.message.contains("durable mutation may already be committed"));
        assert_eq!(fixture.bytes().unwrap(), committed);
        let state: serde_json::Value = serde_json::from_slice(&committed).unwrap();
        assert_eq!(state["runs"][0]["label"], "Committed once");
    }

    #[tokio::test]
    async fn postchecked_snapshot_rejects_identity_replaced_during_core_observation() {
        let fixture = SocketFixture::new().await;
        fixture.adopt().await;
        let (ready, respond) = fixture.gate(1); // core runtime, after the precheck
        let context = fixture.context.clone();
        let read = tokio::spawn(async move { context.snapshot(None).await });
        ready.await.unwrap();
        let original = fixture.payload.lock().clone();
        fixture.payload.lock()["snapshot"]["agents"][0]["agent_session"]["value"] =
            serde_json::json!("replacement");
        respond.send(Some(original)).unwrap();
        assert_eq!(read.await.unwrap().unwrap_err().code, "caller_mismatch");
    }

    #[tokio::test]
    async fn durable_change_before_wait_registration_is_delivered_without_lost_wake() {
        let fixture = SocketFixture::new().await;
        let run = fixture.adopt().await;
        let initial = fixture.context.snapshot(None).await.unwrap();
        assert!(!inbox_counts(&initial.messages, &run, 0).pending);
        // Commit between the consumer's empty observation and service subscription.
        fixture.send(&run, "edge-arrival");
        let next = next_snapshot(&fixture.context, &initial,
            tokio::time::Instant::now() + Duration::from_secs(10)).await.unwrap().unwrap();
        let counts = inbox_counts(&next.messages, &run, 0);
        assert!(counts.pending);
        assert_eq!(counts.through_seq, next.messages[0].seq);
        assert_eq!(next.messages[0].text, "edge-arrival");
        assert_eq!(next.messages[0].stage, DeliveryStage::Stored);
        assert_eq!(fixture.context.own_run(&next).unwrap().run_id, run);
    }

    #[tokio::test]
    async fn pending_inbox_and_controls_are_visible_in_first_completed_snapshot() {
        let fixture = SocketFixture::new().await;
        let run = fixture.adopt().await;
        for kind in [ReportKind::NeedsInput, ReportKind::Result] {
            fixture.context.mutate(OrchestrationAction::Report {
                message_id: format!("pending-{kind:?}"), kind,
                outcome: (kind == ReportKind::Result).then_some(ReportOutcome::Succeeded),
                summary: format!("Pending {kind:?}"), plan: None, to_run_id: None,
            }).await.unwrap();
        }
        fixture.context.mutate(OrchestrationAction::SubagentUpdate {
            subagent_id: "child".into(), parent_subagent_id: None, role: None,
            label: "Child".into(), status: SubagentStatus::Running, summary: None,
        }).await.unwrap();
        fixture.operator(OrchestrationAction::SubagentControl {
            run_id: run.clone(), subagent_id: "child".into(), op: SubagentOp::Cancel,
        });
        let snapshot = fixture.context.snapshot(None).await.unwrap();
        let before = fixture.bytes();
        assert!(inbox_counts(&snapshot.messages, &run, 0).pending);
        assert!(snapshot.messages.iter().any(|message|
            message.report.as_ref().is_some_and(|report| report.kind == ReportKind::NeedsInput)));
        assert!(snapshot.messages.iter().any(|message|
            message.report.as_ref().is_some_and(|report| report.kind == ReportKind::Result)));
        let controls = pending_controls(&snapshot.messages, &run, "child").unwrap();
        assert_eq!(controls.messages.len(), 1);
        let control: ControlPayload = serde_json::from_str(&controls.messages[0].text).unwrap();
        assert!(matches!(control._op, SubagentOp::Cancel));
        assert_eq!(fixture.bytes(), before);
        assert!(snapshot.messages.iter().all(|message| message.stage == DeliveryStage::Stored));
    }

    #[tokio::test]
    async fn deadline_cancels_each_read_phase_and_arrivals_remain_durable_for_next_call() {
        for phase in 0..3 {
            let fixture = SocketFixture::new().await;
            let run = fixture.adopt().await;
            fixture.context.mutate(OrchestrationAction::SubagentUpdate {
                subagent_id: "child".into(), parent_subagent_id: None, role: None,
                label: "Child".into(), status: SubagentStatus::Running, summary: None,
            }).await.unwrap();
            let initial = fixture.context.snapshot(None).await.unwrap();
            // A durable change makes service.wait finish immediately; isolate read cancellation.
            fixture.context.mutate(OrchestrationAction::Annotate {
                run_id: run.clone(), text: "Wake without inbox payload".into(),
            }).await.unwrap();
            let (ready, blocked) = fixture.gate(phase);
            let context = fixture.context.clone();
            tokio::time::pause();
            let read = tokio::spawn(async move {
                next_snapshot(&context, &initial,
                    tokio::time::Instant::now() + Duration::from_millis(100)).await
            });
            reach_gate_without_advancing_time(ready).await;
            fixture.send(&run, "deadline-arrival");
            fixture.operator(OrchestrationAction::SubagentControl {
                run_id: run.clone(), subagent_id: "child".into(), op: SubagentOp::Cancel,
            });
            let committed = fixture.bytes();
            tokio::time::advance(Duration::from_millis(100)).await;
            tokio::time::resume();
            assert!(read.await.unwrap().unwrap().is_none());
            drop(blocked);
            assert_eq!(fixture.bytes(), committed);
            let next = fixture.context.snapshot(None).await.unwrap();
            assert!(inbox_counts(&next.messages, &run, 0).pending);
            assert_eq!(next.messages.iter().find(|m| m.text == "deadline-arrival").unwrap().stage,
                DeliveryStage::Stored);
            let controls = pending_controls(&next.messages, &run, "child").unwrap();
            assert_eq!(controls.messages.len(), 1);
            assert_eq!(controls.messages[0].stage, DeliveryStage::Stored);
        }
    }

    #[tokio::test]
    async fn timeout_empty_fence_propagates_replacement_and_unavailable_endpoint() {
        for unavailable in [false, true] {
            let fixture = SocketFixture::new().await;
            let run = fixture.adopt().await;
            let initial = fixture.context.snapshot(None).await.unwrap();
            fixture.context.mutate(OrchestrationAction::Annotate {
                run_id: run, text: "Wake".into(),
            }).await.unwrap();
            let committed = fixture.bytes();
            let (ready, blocked) = fixture.gate(0);
            let (fence_ready, fence) = fixture.gate(0);
            let context = fixture.context.clone();
            tokio::time::pause();
            let read = tokio::spawn(async move {
                next_snapshot(&context, &initial,
                    tokio::time::Instant::now() + Duration::from_millis(100)).await
            });
            reach_gate_without_advancing_time(ready).await;
            tokio::time::advance(Duration::from_millis(100)).await;
            tokio::time::resume();
            fence_ready.await.unwrap();
            assert!(!read.is_finished());
            let mut replacement = fixture.payload.lock().clone();
            replacement["snapshot"]["boot_id"] = serde_json::json!("replacement");
            fence.send(if unavailable { None } else { Some(replacement) }).unwrap();
            let error = read.await.unwrap().unwrap_err();
            assert_eq!(error.code, if unavailable { "disconnected" } else { "caller_mismatch" });
            assert_eq!(fixture.bytes(), committed);
            drop(blocked);
        }
    }

    #[tokio::test]
    async fn caller_supplied_zero_and_short_wait_budgets_return_only_fenced_empty() {
        for budget in [Duration::ZERO, Duration::from_millis(20)] {
            let fixture = SocketFixture::new().await;
            let run = fixture.adopt().await;
            let initial = fixture.context.snapshot(None).await.unwrap();
            let before = fixture.bytes();
            let deadline = tokio::time::Instant::now() + budget;
            tokio::time::pause();
            let context = fixture.context.clone();
            let read = tokio::spawn(async move { next_snapshot(&context, &initial, deadline).await });
            tokio::time::advance(budget + Duration::from_millis(1)).await;
            tokio::time::resume();
            assert!(read.await.unwrap().unwrap().is_none());
            let snapshot = fixture.context.snapshot(None).await.unwrap();
            assert!(!inbox_counts(&snapshot.messages, &run, 0).pending);
            assert!(pending_controls(&snapshot.messages, &run, "child").unwrap().messages.is_empty());
            assert_eq!(fixture.bytes(), before);
        }
    }

    #[tokio::test]
    async fn aborting_wait_observation_never_mutates_or_acknowledges_durable_state() {
        let fixture = SocketFixture::new().await;
        let run = fixture.adopt().await;
        let initial = fixture.context.snapshot(None).await.unwrap();
        fixture.send(&run, "unacked");
        let before = fixture.bytes();
        let (ready, blocked) = fixture.gate(0);
        let context = fixture.context.clone();
        let read = tokio::spawn(async move {
            next_snapshot(&context, &initial,
                tokio::time::Instant::now() + Duration::from_secs(10)).await
        });
        ready.await.unwrap();
        read.abort();
        assert!(read.await.unwrap_err().is_cancelled());
        drop(blocked);
        assert_eq!(fixture.bytes(), before);
        let next = fixture.context.snapshot(None).await.unwrap();
        assert!(inbox_counts(&next.messages, &run, 0).pending);
        assert_eq!(next.messages[0].stage, DeliveryStage::Stored);
    }

    async fn reach_gate_without_advancing_time(mut ready: tokio::sync::oneshot::Receiver<()>) {
        // Keep a runnable task while paused so socket readiness cannot auto-advance
        // the clock to an unrelated transport timeout.
        loop {
            tokio::select! {
                biased;
                result = &mut ready => { result.unwrap(); return; }
                _ = tokio::task::yield_now() => {}
            }
        }
    }

    #[tokio::test]
    async fn watched_change_interrupts_registered_host_wait_without_timer_advance() {
        let fixture = SocketFixture::new().await;
        let run = fixture.adopt().await;
        let initial = fixture.context.snapshot(None).await.unwrap();
        let mut waiting = Box::pin(next_snapshot(&fixture.context, &initial,
            tokio::time::Instant::now() + Duration::from_secs(10)));
        let pending = std::future::poll_fn(|cx| {
            std::task::Poll::Ready(waiting.as_mut().poll(cx).is_pending())
        }).await;
        assert!(pending);
        fixture.send(&run, "watched-arrival");
        let next = waiting.await.unwrap().unwrap();
        assert!(inbox_counts(&next.messages, &run, 0).pending);
        assert_eq!(next.messages[0].text, "watched-arrival");
        assert_eq!(next.messages[0].stage, DeliveryStage::Stored);
    }

    #[tokio::test]
    async fn external_durable_change_is_observed_by_host_wait_and_not_acknowledged() {
        let fixture = SocketFixture::new().await;
        let run = fixture.adopt().await;
        let initial = fixture.context.snapshot(None).await.unwrap();
        let external = OrchestrationService::open(&fixture.configuration).unwrap();
        let mut waiting = Box::pin(next_snapshot(&fixture.context, &initial,
            tokio::time::Instant::now() + Duration::from_secs(10)));
        let pending = std::future::poll_fn(|cx| {
            std::task::Poll::Ready(waiting.as_mut().poll(cx).is_pending())
        }).await;
        assert!(pending);
        external.mutate(&Actor::Operator(OperatorOrigin::Browser), OrchestrationMutationRequest {
            session_id: "fixture".into(), expected_revision: Some(initial.revision),
            action: OrchestrationAction::MessageSend {
                message_id: "external-arrival".into(), to_run_id: run.clone(),
                kind: MessageKind::Instruction, text: "External arrival".into(),
            },
        }).unwrap();
        tokio::time::pause();
        tokio::time::advance(Duration::from_secs(2)).await;
        tokio::time::resume();
        let next = waiting.await.unwrap().unwrap();
        assert!(inbox_counts(&next.messages, &run, 0).pending);
        assert_eq!(next.messages[0].text, "External arrival");
        assert_eq!(next.messages[0].stage, DeliveryStage::Stored);
    }

    struct EndpointChild {
        child: std::process::Child,
        socket: PathBuf,
    }

    impl EndpointChild {
        fn start(root: &std::path::Path) -> Self {
            use std::io::{BufRead, BufReader};
            let socket = root.join("herdr.sock");
            let mut child = std::process::Command::new(std::env::current_exe().unwrap())
                .args(["--ignored", "--exact",
                    "cli_orchestration::tests::socket_endpoint_responder_process", "--nocapture"])
                .env("CK_HOST_TEST_SOCKET", &socket)
                .env("CK_HOST_TEST_PAYLOAD", root.join("payload.json"))
                .stdout(std::process::Stdio::piped()).spawn().unwrap();
            let stdout = child.stdout.take().unwrap();
            let mut owned = Self { child, socket };
            let mut reader = BufReader::new(stdout);
            let mut line = String::new();
            loop {
                line.clear();
                if reader.read_line(&mut line).unwrap() == 0 {
                    panic!("endpoint child exited before readiness: {:?}", owned.child.try_wait());
                }
                if line.contains("CK_HOST_ENDPOINT_READY") {
                    break;
                }
            }
            owned
        }
    }

    impl Drop for EndpointChild {
        fn drop(&mut self) {
            let _ = self.child.kill();
            let _ = self.child.wait();
            let _ = std::fs::remove_file(&self.socket);
        }
    }

    #[test]
    #[ignore = "owned endpoint responder, invoked only by the replacement test"]
    fn socket_endpoint_responder_process() {
        use std::io::{BufRead, BufReader, Write};
        let socket = std::env::var_os("CK_HOST_TEST_SOCKET").unwrap();
        let payload = std::env::var_os("CK_HOST_TEST_PAYLOAD").unwrap();
        let listener = std::os::unix::net::UnixListener::bind(socket).unwrap();
        println!("CK_HOST_ENDPOINT_READY");
        std::io::stdout().flush().unwrap();
        for connection in listener.incoming() {
            let mut reader = BufReader::new(connection.unwrap());
            let mut line = String::new();
            if reader.read_line(&mut line).unwrap_or(0) == 0 {
                continue;
            }
            let request: serde_json::Value = serde_json::from_str(&line).unwrap();
            let result: serde_json::Value =
                serde_json::from_slice(&std::fs::read(&payload).unwrap()).unwrap();
            let response = format!("{}\n", serde_json::json!({
                "id": request["id"], "result": result,
            }));
            let _ = reader.into_inner().write_all(response.as_bytes());
        }
    }

    #[tokio::test]
    async fn same_socket_endpoint_process_replacement_invalidates_before_mutation() {
        let fixture = SocketFixture::new().await;
        let run = fixture.adopt().await;
        fixture.server.abort();
        // Await listener disposal rather than racing a second bind at the same path.
        while !fixture.server.is_finished() {
            tokio::task::yield_now().await;
        }
        std::fs::remove_file(fixture.root.join("herdr.sock")).unwrap();
        std::fs::write(fixture.root.join("payload.json"),
            serde_json::to_vec(&socket_payload()).unwrap()).unwrap();
        let child = EndpointChild::start(&fixture.root);
        let before = fixture.bytes();
        let error = fixture.context.mutate(OrchestrationAction::Annotate {
            run_id: run, text: "Must not commit".into(),
        }).await.unwrap_err();
        assert_eq!(error.code, "caller_mismatch");
        assert_eq!(fixture.bytes(), before);
        drop(child);
    }

    #[test]
    fn inbox_retirement_envelope_never_exposes_mail_after_acceptance() {
        let (mut run, caller) = retirement_fixture();
        let counts = || InboxCounts {
            run_id: run.run_id.clone(),
            pending: true,
            through_seq: 99,
            counts: vec![KindCount { kind: MessageKind::Instruction, count: 3 }],
        };
        let open = serde_json::to_value(main_wait_read(&run, Some(counts())).unwrap()).unwrap();
        assert_eq!(open["mode"], "open");
        validate_retirement_token(open["retirement_token"].as_str().unwrap()).unwrap();
        assert_eq!(open["inbox"]["through_seq"], 99);
        assert!(open["retirement"].is_null());
        let inbox = counts();
        accept_retirement(&mut run, &caller);
        let closed = serde_json::to_value(main_wait_read(&run, Some(inbox)).unwrap()).unwrap();
        assert_eq!(closed["mode"], "retirement_only");
        assert_eq!(closed.as_object().unwrap().len(), 3);
        validate_retirement_token(closed["retirement_token"].as_str().unwrap()).unwrap();
        assert!(closed.get("inbox").is_none());
        run.close_reason = Some(CloseReason::Cancelled);
        assert!(main_wait_read(&run, Some(InboxCounts {
            run_id: run.run_id.clone(), pending: false, through_seq: 0, counts: vec![],
        })).is_err());
        let parsed = TestCli::try_parse_from([
            "test", "inbox", "wait", "--with-retirement", "--omp-pid", "1234",
        ]).unwrap();
        let TestCommand::Inbox(args) = parsed.command else { panic!("expected inbox") };
        assert!(matches!(args.command, InboxCommand::Wait { with_retirement: true, .. }));
    }

    #[cfg(target_os = "linux")]
    #[tokio::test]
    async fn retirement_wait_crosses_acceptance_without_quiet_herdr_reads() {
        let fixture = SocketFixture::new().await;
        let mut caller = match fixture.context.actor.as_ref().unwrap() {
            Actor::Agent(caller) => caller.clone(),
            _ => panic!("expected native caller"),
        };
        let stat = std::fs::read_to_string("/proc/self/stat").unwrap();
        let parent: u32 = stat[stat.rfind(')').unwrap() + 1..]
            .split_whitespace().nth(1).unwrap().parse().unwrap();
        caller.process = Some(native_process_evidence(parent).unwrap());
        caller.env_run = Some(("worker".into(), 2));
        let (mut run, _) = retirement_fixture();
        run.bound_omp_process = caller.process.clone();
        run.bound_omp_session = caller.omp_session_id.clone();
        run.location = Some(RunLocation {
            boot_id: caller.boot_id.clone(),
            terminal_id: caller.terminal_id.clone(),
            native_session_id: caller.native_session_id.clone(),
            endpoint_identity: caller.endpoint_identity.clone(),
            session_id: caller.session_id.clone(),
            workspace_id: caller.workspace_id.clone(),
            tab_id: caller.tab_id.clone(),
            pane_id: caller.pane_id.clone(),
            launch_tag: "launch".into(),
        });
        run.dispatch.as_mut().unwrap().endpoint_identity = Some(caller.endpoint_identity.clone());
        let context = Arc::new(Context {
            service: OrchestrationService::open(&fixture.configuration).unwrap(),
            adapter: fixture.context.adapter.clone(),
            session: "fixture".into(),
            actor: Some(Actor::Agent(caller.clone())),
            evidence: fixture.context.evidence.clone(),
        });
        let publish = |run: &Run, revision| {
            let bytes = serde_json::to_vec(&serde_json::json!({
                "schema": 1, "revision": revision, "runs": [run],
                "messages": [], "subagents": [], "task_intents": [], "assignment_intents": [],
            })).unwrap();
            let temporary = context.service.base().join("retirement-test.next");
            std::fs::write(&temporary, bytes).unwrap();
            std::fs::rename(temporary, context.service.base().join("state.json")).unwrap();
        };
        publish(&run, 1_u64);
        let (initial_ready, initial_response) = fixture.gate(0);
        let (mut final_ready, final_response) = fixture.gate(0);
        let waiting_context = context.clone();
        let waiting = tokio::spawn(async move { retirement_wait(&waiting_context, true, 10).await });
        initial_ready.await.unwrap();
        initial_response.send(Some(fixture.payload.lock().clone())).unwrap();
        // A second Herdr read during the quiet wait would hit the final gate.
        assert!(tokio::time::timeout(Duration::from_millis(1100), &mut final_ready).await.is_err());
        run.stage = RunStage::Reported;
        publish(&run, 2);
        assert!(tokio::time::timeout(Duration::from_millis(1100), &mut final_ready).await.is_err());
        accept_retirement(&mut run, &caller);
        run.retirement.as_mut().unwrap().identity.as_mut().unwrap().omp_session_id =
            caller.omp_session_id.clone().unwrap();
        publish(&run, 3);
        tokio::time::timeout(Duration::from_secs(3), final_ready).await.unwrap().unwrap();
        final_response.send(Some(fixture.payload.lock().clone())).unwrap();
        let accepted = waiting.await.unwrap().unwrap();
        assert_eq!(accepted.stage, RunStage::Closed);
        assert_eq!(accepted.retirement.unwrap().retirement_id, "retirement");
    }

    #[cfg(target_os = "linux")]
    async fn main_wait_socket_context(fixture: &SocketFixture) -> (Arc<Context>, Run, AgentCaller) {
        use std::os::unix::fs::MetadataExt;
        let root_id = main_wait_test_step("adopting socket-backed root", fixture, fixture.adopt()).await;

        let mut caller = match fixture.context.actor.as_ref().unwrap() {
            Actor::Agent(caller) => caller.clone(),
            _ => panic!("expected native caller"),
        };
        let stat = std::fs::read_to_string("/proc/self/stat").unwrap();
        let parent: u32 = stat[stat.rfind(')').unwrap() + 1..]
            .split_whitespace().nth(1).unwrap().parse().unwrap();
        caller.process = Some(native_process_evidence(parent).unwrap());
        caller.env_run = Some(("worker".into(), 2));
        let (mut run, _) = retirement_fixture();
        run.bound_omp_process = caller.process.clone();
        run.root_id = root_id.clone();
        run.parent_run_id = Some(root_id);
        run.bound_omp_session = caller.omp_session_id.clone();
        run.location = Some(RunLocation {
            boot_id: caller.boot_id.clone(),
            terminal_id: caller.terminal_id.clone(),
            native_session_id: caller.native_session_id.clone(),
            endpoint_identity: caller.endpoint_identity.clone(),
            session_id: caller.session_id.clone(),
            workspace_id: caller.workspace_id.clone(),
            tab_id: caller.tab_id.clone(),
            pane_id: caller.pane_id.clone(),
            launch_tag: "launch".into(),
        });
        run.dispatch.as_mut().unwrap().endpoint_identity = Some(caller.endpoint_identity.clone());
        // Use a live CLI ancestor and its executable/argv evidence, not the parser
        // fixture's imaginary PID. Acceptance copies this exact launch baseline.
        let executable = std::fs::metadata(format!("/proc/{parent}/exe")).unwrap();
        let argv = std::fs::read(format!("/proc/{parent}/cmdline")).unwrap();
        run.launch_shell_identity = Some(NativeShellIdentity {
            process: caller.process.clone().unwrap(),
            executable_device: executable.dev().to_string(),
            executable_inode: executable.ino().to_string(),
            argv_digest: format!("{:x}", Sha256::digest(argv)),
        });
        let context = Arc::new(Context {
            service: OrchestrationService::open(&fixture.configuration).unwrap(),
            adapter: fixture.context.adapter.clone(),
            session: caller.session_id.clone(),
            actor: Some(Actor::Agent(caller.clone())),
            evidence: fixture.context.evidence.clone(),
        });
        publish_main_wait_run(fixture, &run);
        (context, run, caller)
    }

    #[cfg(target_os = "linux")]
    fn publish_main_wait_run(fixture: &SocketFixture, run: &Run) {
        // Model an atomic durable acceptance/rebinding by the owner, retaining
        // actual service-created mail and delivery/ACK state byte-for-byte.
        let mut state: serde_json::Value =
            serde_json::from_slice(&fixture.bytes().unwrap()).unwrap();
        state["revision"] = serde_json::json!(state["revision"].as_u64().unwrap() + 1);
        let runs = state["runs"].as_array_mut().unwrap();
        let serialized = serde_json::to_value(run).unwrap();
        if let Some(existing) = runs.iter_mut().find(|existing| existing["run_id"] == run.run_id) {
            *existing = serialized;
        } else {
            runs.push(serialized);
        }
        let temporary = fixture.context.service.base().join("main-wait-test.next");
        std::fs::write(&temporary, serde_json::to_vec(&state).unwrap()).unwrap();
        std::fs::rename(temporary, fixture.context.service.base().join("state.json")).unwrap();
    }

    #[cfg(target_os = "linux")]
    async fn main_wait_test_step<T>(
        phase: &str,
        fixture: &SocketFixture,
        future: impl Future<Output = T>,
    ) -> T {
        tokio::pin!(future);
        // A Tokio timeout cannot guard a forever-runnable barrier while time is
        // paused. Bound scheduler turns instead; exhaustion FAILS with the phase
        // and socket queue, never fabricates a successful consumer result.
        for _ in 0..16_384 {
            let result = std::future::poll_fn(|cx| {
                std::task::Poll::Ready(match future.as_mut().poll(cx) {
                    std::task::Poll::Ready(value) => Some(value),
                    std::task::Poll::Pending => None,
                })
            }).await;
            if let Some(value) = result {
                return value;
            }
            tokio::task::yield_now().await;
        }
        panic!(
            "S7 watchdog exhausted scheduler turns: phase={phase}, queued_reads={}, server_finished={}, clock={:?}",
            fixture.gates.lock().len(), fixture.server.is_finished(), tokio::time::Instant::now(),
        );
    }

    #[cfg(target_os = "linux")]
    async fn main_wait_timeout_fence(
        fixture: &SocketFixture,
        context: Arc<Context>,
        initial: OrchestrationSnapshot,
        run: &Run,
        case: &str,
    ) -> (
        tokio::task::JoinHandle<Result<Option<OrchestrationSnapshot>, CliError>>,
        tokio::sync::oneshot::Sender<Option<serde_json::Value>>,
    ) {
        // A durable revision wakes wait_next immediately. Hold its snapshot
        // precheck so next_snapshot must take the actual timeout branch.
        publish_main_wait_run(fixture, run);
        let (blocked_ready, blocked_response) = fixture.gate(0);
        let (fence_ready, fence_response) = fixture.gate(0);
        tokio::time::pause();
        let deadline = tokio::time::Instant::now() + Duration::from_millis(100);
        let waiting = tokio::spawn(async move {
            next_snapshot(&context, &initial, deadline).await
        });
        main_wait_test_step(&format!("{case}: snapshot precheck gate"), fixture, blocked_ready)
            .await.unwrap();
        // Advance PAST the deadline, including the timer wheel's next tick.
        // Advancing to equality and then spinning a runnable barrier prevents
        // paused-time auto-advance and can strand the timeout at its last tick.
        tokio::time::advance(Duration::from_millis(101)).await;
        assert!(tokio::time::Instant::now() > deadline);
        main_wait_test_step(&format!("{case}: post-timeout authority gate"), fixture, fence_ready)
            .await.unwrap();
        tokio::time::resume();
        assert!(!waiting.is_finished(), "fresh final authority fence must outlive deadline");
        drop(blocked_response);
        (waiting, fence_response)
    }

    #[cfg(target_os = "linux")]
    #[tokio::test]
    async fn main_wait_deadline_reconstructs_accepted_metadata_without_mail_or_writes() {
        let fixture = SocketFixture::new().await;
        let (context, mut run, caller) = main_wait_test_step(
            "initializing accepted-mail context", &fixture, main_wait_socket_context(&fixture),
        ).await;
        let secret = "SECRET instruction body: must never reach accepted main waiter";
        fixture.send(&run.run_id, secret);
        let initial = main_wait_test_step("open snapshot", &fixture, context.snapshot(None)).await.unwrap();
        assert_eq!(initial.messages.iter().find(|m| m.text == secret).unwrap().stage,
            DeliveryStage::Stored);
        let after = initial.messages.iter().map(|m| m.seq).max().unwrap();
        let open = completed_main_wait(&context, &initial, after).unwrap();
        let open_json = serde_json::to_value(&open).unwrap();
        assert_eq!(open_json["mode"], "open");
        assert!(!open.ready(open_json["retirement_token"].as_str()));
        drop(open);
        let (waiting, final_response) =
            main_wait_timeout_fence(&fixture, context.clone(), initial, &run, "acceptance").await;
        // The budget is already exhausted while Open; acceptance happens while
        // its fresh authority fence is blocked, before any final output.
        let late_secret = "SECRET late durable body: no counts or ACK after acceptance";
        fixture.send(&run.run_id, late_secret);
        accept_retirement(&mut run, &caller);
        run.retirement.as_mut().unwrap().identity.as_mut().unwrap().omp_session_id =
            caller.omp_session_id.clone().unwrap();
        publish_main_wait_run(&fixture, &run);
        let committed = fixture.bytes().unwrap();
        let stored: serde_json::Value = serde_json::from_slice(&committed).unwrap();
        assert!(stored["messages"].as_array().unwrap().iter().any(|m| m["text"] == secret));
        assert!(stored["messages"].as_array().unwrap().iter().any(|m| m["text"] == late_secret));
        final_response.send(Some(fixture.payload.lock().clone())).unwrap();
        assert!(main_wait_test_step("accepted deadline completion", &fixture, waiting).await
            .unwrap().unwrap().is_none(), "must exercise deadline reconstruction");
        // These are the production deadline-output consumer calls, not a
        // hand-built envelope or a replay of the previous Open snapshot.
        let current = main_wait_deadline_run(&context).unwrap();
        let deadline_output = serde_json::to_value(empty_main_wait_read(&current, after).unwrap()).unwrap();
        assert_eq!(deadline_output["mode"], "retirement_only");
        assert_eq!(deadline_output.as_object().unwrap().len(), 3);
        assert_eq!(deadline_output["retirement"]["retirement_id"], "retirement");
        validate_retirement_token(deadline_output["retirement_token"].as_str().unwrap()).unwrap();
        assert!(deadline_output.get("inbox").is_none());
        assert!(deadline_output.get("counts").is_none());
        assert!(deadline_output.get("through_seq").is_none());
        let encoded = serde_json::to_string(&deadline_output).unwrap();
        assert!(!encoded.contains(secret));
        assert!(!encoded.contains(late_secret));
        assert_eq!(fixture.bytes().unwrap(), committed, "deadline read must not ACK or write");

        // Also exercise the completed-snapshot consumer with genuine secret
        // messages still present, rather than giving it an empty mail vector.
        let accepted = main_wait_test_step("accepted secret snapshot", &fixture, context.snapshot(None))
            .await.unwrap();
        for body in [secret, late_secret] {
            assert_eq!(accepted.messages.iter().find(|m| m.text == body).unwrap().stage,
                DeliveryStage::Stored);
        }
        let completed = serde_json::to_value(completed_main_wait(&context, &accepted, after).unwrap()).unwrap();
        assert_eq!(completed, deadline_output);
        assert_eq!(fixture.bytes().unwrap(), committed, "completed read must not ACK or write");
    }

    #[cfg(target_os = "linux")]
    #[tokio::test]
    async fn main_wait_consumers_reject_durable_incarnation_replacement_at_deadline() {
        let fixture = SocketFixture::new().await;
        let (context, mut baseline, caller) = main_wait_test_step(
            "initializing durable-replacement context", &fixture, main_wait_socket_context(&fixture),
        ).await;
        fixture.send(&baseline.run_id, "SECRET mail retained during identity rejection");
        accept_retirement(&mut baseline, &caller);
        baseline.retirement.as_mut().unwrap().identity.as_mut().unwrap().omp_session_id =
            caller.omp_session_id.clone().unwrap();
        let replacements: &[(&str, &str, fn(&mut Run))] = &[
            ("run attempt", "attempt_stale", |r| r.attempt += 1),
            ("bound native session", "session_mismatch", |r| r.bound_omp_session = Some("replacement".into())),
            ("bound PID", "caller_mismatch", |r| r.bound_omp_process.as_mut().unwrap().pid += 1),
            ("bound PID incarnation", "caller_mismatch", |r| r.bound_omp_process.as_mut().unwrap().start_ticks += 1),
            ("bound kernel boot", "caller_mismatch", |r| r.bound_omp_process.as_mut().unwrap().kernel_boot_id = Some("replacement".into())),
            ("dispatch launch attempt", "caller_mismatch", |r| r.dispatch.as_mut().unwrap().launch_attempt += 1),
            ("dispatch launch tag", "caller_mismatch", |r| r.dispatch.as_mut().unwrap().launch_tag = Some("replacement".into())),
            ("dispatch endpoint", "caller_mismatch", |r| r.dispatch.as_mut().unwrap().endpoint_identity = Some("replacement".into())),
            ("current launch shell", "caller_mismatch", |r| r.launch_shell_identity.as_mut().unwrap().argv_digest = "b".repeat(64)),
            ("location native session", "caller_mismatch", |r| r.location.as_mut().unwrap().native_session_id = Some("replacement".into())),
            ("location launch tag", "caller_mismatch", |r| r.location.as_mut().unwrap().launch_tag = "replacement".into()),
            ("retirement run attempt", "caller_mismatch", |r| r.retirement.as_mut().unwrap().identity.as_mut().unwrap().run_attempt += 1),
            ("retirement launch attempt", "caller_mismatch", |r| r.retirement.as_mut().unwrap().identity.as_mut().unwrap().launch_attempt += 1),
            ("retirement launch tag", "caller_mismatch", |r| r.retirement.as_mut().unwrap().identity.as_mut().unwrap().launch_tag = "replacement".into()),
            ("retirement endpoint", "caller_mismatch", |r| r.retirement.as_mut().unwrap().identity.as_mut().unwrap().endpoint_identity = "replacement".into()),
            ("retirement session", "caller_mismatch", |r| r.retirement.as_mut().unwrap().identity.as_mut().unwrap().session_id = "replacement".into()),
            ("retirement workspace", "caller_mismatch", |r| r.retirement.as_mut().unwrap().identity.as_mut().unwrap().workspace_id = "replacement".into()),
            ("retirement tab", "caller_mismatch", |r| r.retirement.as_mut().unwrap().identity.as_mut().unwrap().tab_id = "replacement".into()),
            ("retirement pane", "caller_mismatch", |r| r.retirement.as_mut().unwrap().identity.as_mut().unwrap().pane_id = "replacement".into()),
            ("retirement terminal", "caller_mismatch", |r| r.retirement.as_mut().unwrap().identity.as_mut().unwrap().terminal_id = "replacement".into()),
            ("retirement Herdr boot", "caller_mismatch", |r| r.retirement.as_mut().unwrap().identity.as_mut().unwrap().herdr_boot_id = Some("replacement".into())),
            ("retirement native session", "caller_mismatch", |r| r.retirement.as_mut().unwrap().identity.as_mut().unwrap().omp_session_id = "replacement".into()),
            ("retirement PID", "caller_mismatch", |r| r.retirement.as_mut().unwrap().identity.as_mut().unwrap().process.pid += 1),
            ("retirement PID incarnation", "caller_mismatch", |r| r.retirement.as_mut().unwrap().identity.as_mut().unwrap().process.start_ticks += 1),
            ("retirement kernel boot", "caller_mismatch", |r| r.retirement.as_mut().unwrap().identity.as_mut().unwrap().process.kernel_boot_id = Some("replacement".into())),
            ("retirement shell PID", "caller_mismatch", |r| r.retirement.as_mut().unwrap().identity.as_mut().unwrap().shell.process.pid += 1),
            ("retirement shell incarnation", "caller_mismatch", |r| r.retirement.as_mut().unwrap().identity.as_mut().unwrap().shell.process.start_ticks += 1),
            ("retirement shell kernel boot", "caller_mismatch", |r| r.retirement.as_mut().unwrap().identity.as_mut().unwrap().shell.process.kernel_boot_id = Some("replacement".into())),
            ("retirement shell executable device", "caller_mismatch", |r| r.retirement.as_mut().unwrap().identity.as_mut().unwrap().shell.executable_device = "replacement".into()),
            ("retirement shell executable inode", "caller_mismatch", |r| r.retirement.as_mut().unwrap().identity.as_mut().unwrap().shell.executable_inode = "replacement".into()),
            ("retirement shell argv", "caller_mismatch", |r| r.retirement.as_mut().unwrap().identity.as_mut().unwrap().shell.argv_digest = "b".repeat(64)),
            ("missing immutable identity", "caller_mismatch", |r| r.retirement.as_mut().unwrap().identity = None),
        ];
        for (name, code, replace) in replacements {
            publish_main_wait_run(&fixture, &baseline);
            let initial = main_wait_test_step(&format!("{name}: baseline snapshot"), &fixture,
                context.snapshot(None)).await.unwrap();
            assert!(completed_main_wait(&context, &initial, 0).is_ok(), "{name}: valid baseline");
            let (waiting, final_response) =
                main_wait_timeout_fence(&fixture, context.clone(), initial, &baseline, name).await;
            let mut replaced = baseline.clone();
            replace(&mut replaced);
            publish_main_wait_run(&fixture, &replaced);
            let committed = fixture.bytes().unwrap();
            final_response.send(Some(fixture.payload.lock().clone())).unwrap();
            assert!(main_wait_test_step(&format!("{name}: deadline completion"), &fixture, waiting)
                .await.unwrap().unwrap().is_none(), "{name}: actual deadline");
            let error = main_wait_deadline_run(&context).unwrap_err();
            assert_eq!(error.code, *code, "{name}: deadline consumer");
            // The receipt command performs this same durable scope preflight
            // before calling mutate; no receipt is submitted on rejection.
            assert_eq!(context.retiring_run_for_review().unwrap_err().code, *code, "{name}: receipt preflight");
            let snapshot = main_wait_test_step(&format!("{name}: replaced snapshot"), &fixture,
                context.snapshot(None)).await.unwrap();
            let error = completed_main_wait(&context, &snapshot, 0).err().expect("replacement must reject metadata");
            assert_eq!(error.code, *code, "{name}: completed consumer");
            assert_eq!(fixture.bytes().unwrap(), committed, "{name}: no read/receipt/ACK write");
        }
    }

    #[cfg(target_os = "linux")]
    #[tokio::test]
    async fn main_wait_deadline_rejects_runtime_and_live_process_replacement_without_writes() {
        let fixture = SocketFixture::new().await;
        let (context, mut run, caller) = main_wait_test_step(
            "initializing runtime-replacement context", &fixture, main_wait_socket_context(&fixture),
        ).await;
        fixture.send(&run.run_id, "SECRET mail retained at final runtime fence");
        accept_retirement(&mut run, &caller);
        run.retirement.as_mut().unwrap().identity.as_mut().unwrap().omp_session_id =
            caller.omp_session_id.clone().unwrap();
        let runtime_replacements: &[(&str, fn(&mut serde_json::Value))] = &[
            ("native session", |v| v["snapshot"]["agents"][0]["agent_session"]["value"] = serde_json::json!("replacement")),
            ("terminal", |v| v["snapshot"]["panes"][0]["terminal_id"] = serde_json::json!("replacement")),
            ("Herdr boot", |v| v["snapshot"]["boot_id"] = serde_json::json!("replacement")),
            ("native agent kind", |v| v["snapshot"]["agents"][0]["agent"] = serde_json::json!("other")),
            ("launch pending", |v| v["snapshot"]["agents"][0]["launch_pending"] = serde_json::json!(true)),
        ];
        for (name, replace) in runtime_replacements {
            publish_main_wait_run(&fixture, &run);
            let initial = main_wait_test_step(&format!("{name}: runtime baseline snapshot"), &fixture,
                context.snapshot(None)).await.unwrap();
            assert!(completed_main_wait(&context, &initial, 0).is_ok(), "{name}: valid baseline");
            let (waiting, final_response) =
                main_wait_timeout_fence(&fixture, context.clone(), initial, &run, name).await;
            let committed = fixture.bytes().unwrap();
            let mut payload = fixture.payload.lock().clone();
            replace(&mut payload);
            final_response.send(Some(payload)).unwrap();
            assert_eq!(main_wait_test_step(&format!("{name}: rejected final runtime fence"), &fixture,
                waiting).await.unwrap().unwrap_err().code, "caller_mismatch", "{name}");
            assert_eq!(fixture.bytes().unwrap(), committed, "{name}: timeout fence cannot write");
        }
        let process_replacements: &[(&str, fn(&mut NativeProcessIdentity))] = &[
            ("PID", |p| p.pid = u32::MAX),
            ("PID incarnation", |p| p.start_ticks += 1),
            ("kernel boot", |p| p.kernel_boot_id = Some("replacement".into())),
        ];
        for (name, replace) in process_replacements {
            publish_main_wait_run(&fixture, &run);
            let initial = main_wait_test_step(&format!("{name}: process baseline snapshot"), &fixture,
                context.snapshot(None)).await.unwrap();
            let (waiting, final_response) =
                main_wait_timeout_fence(&fixture, context.clone(), initial, &run, name).await;
            final_response.send(Some(fixture.payload.lock().clone())).unwrap();
            assert!(main_wait_test_step(&format!("{name}: process deadline completion"), &fixture,
                waiting).await.unwrap().unwrap().is_none(), "{name}: actual deadline");
            let mut replaced_caller = caller.clone();
            replace(replaced_caller.process.as_mut().unwrap());
            let mut replaced_run = run.clone();
            // Keep durable authority internally coherent with the replacement.
            // Only live ancestor evidence can reject this fabricated incarnation.
            replaced_run.bound_omp_process = replaced_caller.process.clone();
            replaced_run.retirement.as_mut().unwrap().identity.as_mut().unwrap().process =
                replaced_caller.process.clone().unwrap();
            publish_main_wait_run(&fixture, &replaced_run);
            let committed = fixture.bytes().unwrap();
            let replaced_context = Context {
                service: OrchestrationService::open(&fixture.configuration).unwrap(),
                adapter: context.adapter.clone(),
                session: context.session.clone(),
                actor: Some(Actor::Agent(replaced_caller)),
                evidence: context.evidence.clone(),
            };
            assert!(replaced_context.retiring_run_for_review().is_ok(), "{name}: durable scope alone is insufficient");
            assert_eq!(main_wait_deadline_run(&replaced_context).unwrap_err().code,
                "caller_mismatch", "{name}: live process fence");
            let snapshot = main_wait_test_step(&format!("{name}: replaced process snapshot"), &fixture,
                replaced_context.snapshot(None)).await.unwrap();
            assert_eq!(completed_main_wait(&replaced_context, &snapshot, 0).err().unwrap().code,
                "caller_mismatch", "{name}: completed live process fence");
            assert_eq!(fixture.bytes().unwrap(), committed, "{name}: rejected incarnation cannot write");
        }
    }

    #[cfg(target_os = "linux")]
    #[tokio::test]
    async fn main_wait_startup_not_ready_then_attested_observes_late_brief_without_writes() {
        let fixture = SocketFixture::new().await;
        let (baseline, mut run, mut caller) = main_wait_socket_context(&fixture).await;
        run.stage = RunStage::Preparing;
        run.dispatch.as_mut().unwrap().step = DispatchStep::LaunchPending;
        run.dispatch.as_mut().unwrap().agent_started = false;
        publish_main_wait_run(&fixture, &run);
        fixture.payload.lock()["snapshot"]["agents"][0]["launch_pending"] = serde_json::json!(true);
        caller.actual_agent_kind = None;
        let context = Context {
            service: OrchestrationService::open(&fixture.configuration).unwrap(),
            adapter: baseline.adapter.clone(),
            session: baseline.session.clone(),
            actor: Some(Actor::Agent(caller.clone())),
            evidence: baseline.evidence.clone(),
        };
        let committed = fixture.bytes().unwrap();
        let snapshot = context.snapshot(None).await.unwrap();
        assert_eq!(
            completed_main_wait(&context, &snapshot, 0)
                .err()
                .unwrap()
                .code,
            "caller_not_ready"
        );
        assert_eq!(
            main_wait_deadline_run(&context).unwrap_err().code,
            "caller_not_ready"
        );
        assert_eq!(fixture.bytes().unwrap(), committed);
        let replacements: &[(&str, &str, fn(&mut Run))] = &[
            ("session_mismatch", "main session", |r| {
                r.bound_omp_session = Some("replacement".into())
            }),
            ("caller_mismatch", "process incarnation", |r| {
                r.bound_omp_process.as_mut().unwrap().start_ticks += 1
            }),
            ("caller_mismatch", "endpoint", |r| {
                r.location.as_mut().unwrap().endpoint_identity = "replacement".into()
            }),
            ("caller_mismatch", "pane", |r| {
                r.location.as_mut().unwrap().pane_id = "replacement".into()
            }),
            ("caller_mismatch", "dispatch tag", |r| {
                r.dispatch.as_mut().unwrap().launch_tag = Some("replacement".into())
            }),
            ("attempt_stale", "run attempt", |r| r.attempt += 1),
        ];
        for (code, name, replace) in replacements {
            let mut replaced = run.clone();
            replace(&mut replaced);
            publish_main_wait_run(&fixture, &replaced);
            let committed = fixture.bytes().unwrap();
            let snapshot = context.snapshot(None).await.unwrap();
            assert_eq!(
                completed_main_wait(&context, &snapshot, 0)
                    .err()
                    .unwrap()
                    .code,
                *code,
                "{name}"
            );
            assert_eq!(
                main_wait_deadline_run(&context).unwrap_err().code,
                *code,
                "{name}: deadline"
            );
            assert_eq!(fixture.bytes().unwrap(), committed);
        }
        publish_main_wait_run(&fixture, &run);

        // Launch proof and the durable brief come later. The old opening
        // snapshot must ask for fresh evidence, not permanently revoke itself.
        fixture.payload.lock()["snapshot"]["agents"][0]["launch_pending"] =
            serde_json::json!(false);
        run.stage = RunStage::Initializing;
        run.dispatch.as_mut().unwrap().step = DispatchStep::Launched;
        run.dispatch.as_mut().unwrap().agent_started = true;
        publish_main_wait_run(&fixture, &run);
        fixture.send(&run.run_id, "late startup brief");
        let committed = fixture.bytes().unwrap();
        assert_eq!(
            context.check_caller().await.unwrap_err().code,
            "caller_not_ready"
        );
        assert_eq!(fixture.bytes().unwrap(), committed);

        caller.actual_agent_kind = Some("omp".into());
        let fresh = Context {
            service: OrchestrationService::open(&fixture.configuration).unwrap(),
            adapter: baseline.adapter.clone(),
            session: baseline.session.clone(),
            actor: Some(Actor::Agent(caller.clone())),
            evidence: baseline.evidence.clone(),
        };
        let snapshot = fresh.snapshot(None).await.unwrap();
        let read = completed_main_wait(&fresh, &snapshot, 0).unwrap();
        let output = serde_json::to_value(&read).unwrap();
        assert_eq!(output["mode"], "open");
        assert_eq!(output["inbox"]["pending"], true);
        assert_eq!(output["inbox"]["counts"][0]["count"], 1);
        assert!(output["retirement"].is_null());
        assert!(
            !serde_json::to_string(&output)
                .unwrap()
                .contains("late startup brief")
        );
        assert!(main_wait_deadline_run(&fresh).is_ok());
        assert_eq!(
            fixture.bytes().unwrap(),
            committed,
            "observation never ACKs or pulls mail"
        );

        // Missing attestation on the now-mature run is not another startup.
        fixture.payload.lock()["snapshot"]["agents"][0]["launch_pending"] = serde_json::json!(true);
        assert_eq!(
            context.retiring_run_for_review().unwrap_err().code,
            "caller_mismatch"
        );
        assert_eq!(
            fresh.check_caller().await.unwrap_err().code,
            "caller_mismatch"
        );
        assert_eq!(fixture.bytes().unwrap(), committed);
    }

    #[cfg(target_os = "linux")]
    #[tokio::test]
    async fn main_wait_startup_settling_fences_current_and_deadline_observation() {
        let fixture = SocketFixture::new().await;
        let (baseline, mut run, mut caller) = main_wait_socket_context(&fixture).await;
        run.stage = RunStage::Preparing;
        run.dispatch.as_mut().unwrap().step = DispatchStep::LaunchPending;
        run.dispatch.as_mut().unwrap().agent_started = false;
        caller.actual_agent_kind = None;
        caller.native_session_id = None;
        let context = Arc::new(Context {
            service: OrchestrationService::open(&fixture.configuration).unwrap(),
            adapter: baseline.adapter.clone(),
            session: baseline.session.clone(),
            actor: Some(Actor::Agent(caller.clone())),
            evidence: baseline.evidence.clone(),
        });
        let attested = fixture.payload.lock().clone();
        let mut pending = attested.clone();
        pending["snapshot"]["agents"][0]["launch_pending"] = serde_json::json!(true);
        pending["snapshot"]["agents"][0]
            .as_object_mut()
            .unwrap()
            .remove("agent_session");
        let replacements: &[(&str, &str, fn(&mut serde_json::Value), fn(&mut Run))] = &[
            ("expected native session", "caller_not_ready", |_| {}, |_| {}),
            (
                "foreign native session",
                "caller_mismatch",
                |v| {
                    v["snapshot"]["agents"][0]["agent_session"]["value"] =
                        serde_json::json!("replacement")
                },
                |_| {},
            ),
            (
                "foreign boot",
                "caller_mismatch",
                |v| v["snapshot"]["boot_id"] = serde_json::json!("replacement"),
                |_| {},
            ),
            (
                "foreign terminal",
                "caller_mismatch",
                |v| v["snapshot"]["panes"][0]["terminal_id"] = serde_json::json!("replacement"),
                |_| {},
            ),
            (
                "foreign process incarnation",
                "caller_mismatch",
                |_| {},
                |r| r.bound_omp_process.as_mut().unwrap().start_ticks += 1,
            ),
            (
                "foreign dispatch tag",
                "caller_mismatch",
                |_| {},
                |r| r.dispatch.as_mut().unwrap().launch_tag = Some("replacement".into()),
            ),
            ("foreign attempt", "caller_mismatch", |_| {}, |r| r.attempt += 1),
        ];
        for (name, code, replace_runtime, replace_run) in replacements {
            *fixture.payload.lock() = pending.clone();
            publish_main_wait_run(&fixture, &run);
            let initial = context.snapshot(None).await.unwrap();
            assert_eq!(
                completed_main_wait(&context, &initial, 0)
                    .err()
                    .unwrap()
                    .code,
                "caller_not_ready"
            );
            let (waiting, response) =
                main_wait_timeout_fence(&fixture, context.clone(), initial, &run, name).await;
            let mut replaced_run = run.clone();
            replace_run(&mut replaced_run);
            publish_main_wait_run(&fixture, &replaced_run);
            let committed = fixture.bytes().unwrap();
            let mut changed = attested.clone();
            replace_runtime(&mut changed);
            response.send(Some(changed.clone())).unwrap();
            assert_eq!(
                main_wait_test_step(name, &fixture, waiting)
                    .await
                    .unwrap()
                    .unwrap_err()
                    .code,
                *code
            );
            *fixture.payload.lock() = changed;
            assert_eq!(
                context.check_caller().await.unwrap_err().code,
                *code,
                "{name}: current fence"
            );
            assert_eq!(
                fixture.bytes().unwrap(),
                committed,
                "{name}: no observation writes"
            );
        }
    }

}
