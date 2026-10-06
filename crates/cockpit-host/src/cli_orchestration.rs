use std::{collections::BTreeMap, io::Read, path::PathBuf, sync::Arc, time::Duration};

use clap::{Args, Subcommand, ValueEnum};
use cockpit_core::{
    InspectionError,
    config::load_project_configuration,
    extension_adapter::SourcePaneEvidence,
    orchestration::{Actor, AgentCaller, OrchestrationService, herdr::OrchestrationHerdr},
    projects::ProjectService,
};
use cockpit_herdr::HerdrCliAdapter;
use super::endpoint::{AmbientEndpoint, Endpoint, resolve_endpoint};
use cockpit_protocol::{
    orchestration::*,
    projects::{ProjectConfiguration, WorkspaceSetupRequest},
};
use serde::{Deserialize, Serialize};

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
                        .filter(|kind| kind.as_str() == "omp" && !pane.launch_pending)
                        .cloned(),
                    env_run: args.env_run()?,
                    omp_session_id: args.omp_session.clone(),
                    main_omp_session_id: args.omp_main_session.clone(),
                    agent_kind: args.agent_kind.map(Into::into),
                    subagent_id: args.subagent_id.clone(),
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
            let after = self
                .adapter
                .source_adapter()
                .source_pane_evidence(&self.session, &before.pane_id)
                .await?;
            if before.pane_id != after.pane_id
                || before.terminal_id != after.terminal_id
                || before.workspace_id != after.workspace_id
                || before.tab_id != after.tab_id
                || before.endpoint_identity != after.endpoint_identity
            {
                return Err(CliError::new(
                    "caller_mismatch",
                    "caller pane identity changed during orchestration operation",
                ));
            }
            if let Some(Actor::Agent(caller)) = &self.actor {
                let runtime = self.adapter.runtime(&self.session).await?;
                let pane = runtime
                    .panes
                    .iter()
                    .find(|pane| pane.pane_id == caller.pane_id)
                    .ok_or_else(|| {
                        CliError::new(
                            "caller_mismatch",
                            "caller pane disappeared during orchestration operation",
                        )
                    })?;
                if runtime.endpoint_identity != caller.endpoint_identity
                    || runtime.boot_id != caller.boot_id
                    || pane.native_session_id != caller.native_session_id
                    || caller.actual_agent_kind.as_ref().is_some_and(|kind| {
                        pane.agent_kind.as_ref() != Some(kind) || pane.launch_pending
                    })
                    || pane
                        .terminal_id
                        .as_ref()
                        .is_some_and(|terminal| Some(terminal) != caller.terminal_id.as_ref())
                {
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
pub(crate) struct BodyArgs {
    /// Literal UTF-8 body (at most 16 KiB).
    #[arg(long, conflicts_with_all = ["body_file", "stdin"])]
    body: Option<String>,
    /// Read UTF-8 body from a file (at most 16 KiB).
    #[arg(long, conflicts_with = "stdin")]
    body_file: Option<PathBuf>,
    /// Read UTF-8 body from stdin (at most 16 KiB).
    #[arg(long)]
    stdin: bool,
}
impl BodyArgs {
    fn read(self) -> Result<Option<String>, CliError> {
        match self.body {
            Some(body) => bounded(body).map(Some),
            None => read_input(self.body_file, self.stdin),
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
#[derive(Debug, Subcommand)]
pub(crate) enum TaskCommand {
    /// List canonical tasks and their derived board lanes.
    List,
    /// Show a canonical task including its body and revision.
    Show { task: String },
    /// Create a task in the caller run's root; does not start a worker.
    Create {
        #[arg(long)]
        title: String,
        #[command(flatten)]
        body: BodyArgs,
    },
    /// Update canonical Markdown with an exact task revision fence.
    Update {
        task: String,
        #[arg(long)]
        revision: String,
        #[arg(long)]
        title: Option<String>,
        #[command(flatten)]
        body: BodyArgs,
    },
    /// Assign stable IDs to unmarked checklist items with a document revision fence.
    AssignIds {
        #[arg(long)]
        doc_revision: String,
    },
}
impl TaskArgs {
    pub async fn run(self) -> Result<(), CliError> {
        let writing = matches!(
            &self.command,
            TaskCommand::Create { .. } | TaskCommand::Update { .. } | TaskCommand::AssignIds { .. }
        );
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
                        "{}\t[{}] {}\t{:?}\t{}",
                        view.task.task_id,
                        if view.task.checked { 'x' } else { ' ' },
                        view.task.title,
                        view.lane,
                        view.task.task_revision
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
            TaskCommand::Create { title, body } => OrchestrationAction::TaskCreate {
                root_id: root,
                title,
                body: body.read()?.unwrap_or_default(),
            },
            TaskCommand::Update {
                task,
                revision,
                title,
                body,
            } => {
                let body = body.read()?;
                if title.is_none() && body.is_none() {
                    return Err(CliError::usage(
                        "task update requires --title, --body-file or --stdin",
                    ));
                }
                OrchestrationAction::TaskUpdate {
                    root_id: root,
                    task_id: task,
                    expected_task_revision: revision,
                    title,
                    body,
                }
            }
            TaskCommand::AssignIds { doc_revision } => OrchestrationAction::TasksAssignIds {
                root_id: root,
                expected_doc_revision: doc_revision,
            },
        };
        emit(&context.mutate(action).await?, self.common.json)
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
#[command(group(clap::ArgGroup::new("target").required(true).multiple(false).args(["repository", "path", "space"])))]
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
    #[arg(long, requires = "repository", conflicts_with_all = ["path", "space"])]
    branch: Option<String>,
    #[arg(long, requires = "repository", conflicts_with_all = ["path", "space"])]
    base: Option<String>,
    #[arg(long, requires = "repository", conflicts_with_all = ["path", "space"])]
    checkout_path: Option<String>,
    #[arg(long, requires = "repository", conflicts_with_all = ["path", "space"])]
    artifact: Option<String>,
    #[arg(long, requires = "repository", conflicts_with_all = ["path", "space"])]
    linked_artifact: Vec<String>,
    #[arg(long, conflicts_with = "space")]
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
        let target = match (self.repository, self.path, self.space) {
            (Some(repository_id), None, None) => DispatchTarget::Setup {
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
            (None, Some(path), None) => DispatchTarget::Setup {
                request: WorkspaceSetupRequest::Open {
                    path,
                    label: self.label.clone(),
                    task_name: self.task_name,
                    focus: false,
                },
            },
            (None, None, Some(workspace_id)) => DispatchTarget::ExistingSpace { workspace_id },
            _ => {
                return Err(CliError::usage(
                    "propose requires exactly one of --repository, --path, --space",
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
            InboxCommand::Wait { after, timeout } => {
                let mut snapshot = context.snapshot(None).await?;
                let deadline = tokio::time::Instant::now() + Duration::from_secs(timeout);
                loop {
                    let own = context.own_run(&snapshot)?;
                    let result = inbox_counts(&snapshot.messages, &own.run_id, after);
                    if result.pending || tokio::time::Instant::now() >= deadline {
                        return emit(&result, self.common.json);
                    }
                    match next_snapshot(&context, &snapshot, deadline).await? {
                        Some(next) => snapshot = next,
                        None => return emit(&result, self.common.json),
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
        Err(_) => Ok(None),
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
    // Poll at most once a second even without an owner, including cross-process writes.
    let timeout_ms = remaining.min(Duration::from_secs(1)).as_millis().max(1) as u32;
    context
        .service
        .wait(&OrchestrationWaitRequest {
            after_revision: snapshot.revision,
            after_tasks_token: snapshot.tasks_token.clone(),
            timeout_ms,
        })
        .await?;
    context.check_caller().await
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
                    if !result.messages.is_empty() || tokio::time::Instant::now() >= deadline {
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
        for command in ["grant-prepare", "grant-execute", "retry-launch", "start"] {
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
        for command in ["prepare", "execute", "accept", "send-back", "cancel"] {
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
                "--body-file",
                "body"
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
                "--body",
                "Literal body"
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
}
