use std::{io::Read, path::PathBuf, sync::Arc};

use cockpit_core::{
    config::ConfigurationFile,
    orchestration::{
        Actor, AgentCaller, OrchestrationService,
    },
};
use cockpit_herdr::HerdrCliAdapter;
use cockpit_host::orchestration_runtime::resolve_mutation_input;
use cockpit_protocol::{
    orchestration::{
        OrchestrationAction, OrchestrationMutationRequest, OrchestrationMutationResponse,
        OrchestrationSnapshot, OrchestrationSnapshotRequest, Run,
    },
    projects::ProjectConfiguration,
};

use super::{
    CliError,
    args::{MAX_TEXT_BYTES, OrchestrationArgs},
    caller::job_actor,
};
use crate::endpoint::{AmbientEndpoint, Endpoint, resolve_endpoint};
pub(super) fn read_input(file: Option<PathBuf>, stdin: bool) -> Result<Option<String>, CliError> {
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

pub(super) fn read_bounded(input: impl Read) -> Result<String, CliError> {
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

pub(super) fn read_json_file<T: serde::de::DeserializeOwned>(path: PathBuf) -> Result<T, CliError> {
    let text = read_input(Some(path), false)?.expect("file input");
    serde_json::from_str(&text)
        .map_err(|error| CliError::new("orchestration_input", error.to_string()))
}

impl OrchestrationArgs {
    pub(super) fn configuration(&self) -> Result<ProjectConfiguration, CliError> {
        let config = self
            .config
            .clone()
            .or_else(|| std::env::var_os("COCKPIT_CONFIG").map(PathBuf::from));
        Ok(ConfigurationFile::load(config.as_deref())?
            .project
            .resolve(
                (!self.repository_roots.is_empty()).then_some(self.repository_roots.as_slice()),
            )?)
    }

    pub(super) fn endpoint(&self) -> Result<Endpoint, CliError> {
        resolve_endpoint(
            self.herdr.clone(),
            self.herdr_session.clone(),
            self.herdr_socket.clone(),
            &AmbientEndpoint::from_process(),
        )
        .map_err(|error| match error.code.as_str() {
            "missing_socket_session" | "session_required" => CliError::usage(error.message),
            _ => CliError::new(&error.code, error.message),
        })
    }
}

pub(super) fn require_job(args: &OrchestrationArgs) -> Result<(), CliError> {
    if args.operator {
        return Err(CliError::new("actor_forbidden",
            "Operator mode cannot act as a job; use an explicit task or run management target"));
    }
    Ok(())
}

pub(super) struct Context {
    pub(super) service: OrchestrationService,
    pub(super) adapter: Arc<HerdrCliAdapter>,
    pub(super) session: String,
    pub(super) actor: Option<Actor>,
    pub(super) configuration: ProjectConfiguration,
}

impl Context {
    /// Only the exact committed Result receipt may be read after acceptance.
    /// Never use this path as admission for a fresh write or ordinary job tools.
    pub(super) fn committed_report_replay(
        args: &OrchestrationArgs, action: &OrchestrationAction,
    ) -> Result<Option<OrchestrationMutationResponse>, CliError> {
        let OrchestrationAction::Report { kind: cockpit_protocol::orchestration::ReportKind::Result, message_id, .. } = action else {
            return Ok(None);
        };
        require_job(args)?;
        let Some(mut actor) = job_actor(args)? else { return Ok(None) };
        let service = OrchestrationService::open(&args.configuration()?)?;
        let Actor::Agent(caller) = &mut actor else { unreachable!() };
        if service.job_registration_run(caller).is_ok() {
            return Ok(None); // Ordinary context handles first registration and normal replay.
        }
        let job = service.committed_report_job_run(caller, message_id)?;
        Self::check_launch(args, &job)?;
        caller.session_id = job.session_id.clone();
        Ok(service.operation_replay(&actor, &OrchestrationMutationRequest {
            session_id: job.session_id, expected_revision: None, action: action.clone(),
        })?)
    }

    /// CleanupFinish alone has dedicated core admission after acceptance.
    /// This cannot open an ordinary Context or admit another action.
    pub(super) async fn finish_cleanup(
        args: &OrchestrationArgs, action: OrchestrationAction,
    ) -> Result<OrchestrationMutationResponse, CliError> {
        require_job(args)?;
        let OrchestrationAction::CleanupFinish { request: finish } = &action else {
            return Err(CliError::new("actor_forbidden", "Dedicated cleanup admission only accepts CleanupFinish"));
        };
        let mut actor = job_actor(args)?.ok_or_else(|| CliError::new(
            "credential_required", "Cleanup finish requires a private job credential",
        ))?;
        let service = OrchestrationService::open(&args.configuration()?)?;
        let Actor::Agent(caller) = &mut actor else {
            return Err(CliError::new("credential_required", "Cleanup finish requires a private job credential"));
        };
        let job = service.cleanup_job_run(caller, finish)?;
        Self::check_launch(args, &job)?;
        caller.session_id = job.session_id.clone();
        let request = OrchestrationMutationRequest {
            session_id: job.session_id, expected_revision: None, action,
        };
        if let Some(response) = service.operation_replay(&actor, &request)? {
            return Ok(response);
        }
        Ok(service.mutate_resolved(
            &actor, request, &cockpit_core::orchestration::ResolvedMutationInput::default(),
        )?)
    }


    pub(super) fn register(
        args: &OrchestrationArgs, operation_id: String,
    ) -> Result<OrchestrationMutationResponse, CliError> {
        require_job(args)?;
        let actor = job_actor(args)?.ok_or_else(|| CliError::new(
            "credential_required", "Registration requires an issued private job credential",
        ))?;
        let service = OrchestrationService::open(&args.configuration()?)?;
        let Actor::Agent(caller) = &actor else { unreachable!() };
        let job = service.job_registration_run(caller)?;
        Self::check_launch(args, &job)?;
        Ok(service.mutate_resolved(&actor, OrchestrationMutationRequest {
            session_id: job.session_id,
            expected_revision: None,
            action: OrchestrationAction::RunRegister { operation_id },
        }, &cockpit_core::orchestration::ResolvedMutationInput::default())?)
    }

    fn check_launch(args: &OrchestrationArgs, job: &Run) -> Result<(), CliError> {
        if std::env::var_os("COCKPIT_RUN_ID").is_some_and(|id| id != std::ffi::OsStr::new(&job.run_id)) {
            return Err(CliError::new("credential_mismatch", "Launch run and credential differ"));
        }
        if args.herdr_session.as_ref().is_some_and(|session| *session != job.session_id) {
            return Err(CliError::new("session_mismatch", "Job belongs to another Herdr session; explicit local management can use --operator with a named --root/TASK/RUN and the actual --herdr-session/--herdr-socket"));
        }
        Ok(())
    }

    pub(super) fn authenticated_run(&self, caller: &AgentCaller) -> Result<Run, CliError> {
        Ok(self.service.job_role(caller)?.0)
    }

    pub(super) async fn open(
        args: &OrchestrationArgs,
        caller_required: bool,
    ) -> Result<Self, CliError> {
        let configuration = args.configuration()?;
        let service = OrchestrationService::open(&configuration)?;
        // Invalid credentials never fall back implicitly; operator authority is explicit.
        let mut actor = if args.operator {
            Some(Actor::Operator(cockpit_protocol::orchestration::OperatorOrigin::Native))
        } else { job_actor(args)? };
        let job = if let Some(Actor::Agent(caller)) = &mut actor {
            let job = service.job_registration_run(caller)?;
            Self::check_launch(args, &job)?;
            caller.session_id = job.session_id.clone();
            Some(service.ensure_registered(caller)?)
        } else { None };
        let agent_context = !args.operator && (caller_required
            || std::env::var_os("COCKPIT_RUN_ID").is_some()
            || std::env::var_os("COCKPIT_ATTACHMENT_ID").is_some()
            || args.harness.is_some() || args.session_id.is_some()
            || std::env::var_os("COCKPIT_HARNESS").is_some()
            || std::env::var_os("COCKPIT_SESSION_ID").is_some()
            || std::env::var("HERDR_ENV").ok().as_deref() == Some("1"));
        if agent_context && actor.is_none() {
            return Err(CliError::new("credential_required", "This command requires a private job credential"));
        }
        let endpoint = match &job {
            Some(job) => resolve_endpoint(
                args.herdr.clone(), Some(job.session_id.clone()), args.herdr_socket.clone(),
                &AmbientEndpoint::from_process(),
            ).map_err(|error| CliError::new(&error.code, error.message))?,
            None => args.endpoint()?,
        };
        let session = endpoint.session;
        let adapter = Arc::new(HerdrCliAdapter::new(endpoint.config));
        let context = Self { service, adapter, session, actor, configuration };
        context.check_caller().await?;
        Ok(context)
    }

    pub(super) async fn snapshot(
        &self,
        root: Option<String>,
    ) -> Result<OrchestrationSnapshot, CliError> {
        self.check_caller().await?;
        let root = if let Some(Actor::Agent(caller)) = &self.actor {
            let job = self.authenticated_run(caller)?;
            if root.as_ref().is_some_and(|root| *root != job.root_id) {
                return Err(CliError::new("actor_forbidden", "Job reads are restricted to its task root; explicit local inspection may use --operator --root ROOT"));
            }
            Some(job.root_id)
        } else {
            root
        };
        let request = OrchestrationSnapshotRequest { session_id: self.session.clone(), root_id: root };
        let snapshot = self.service.snapshot_saved(&request)?;
        self.check_caller().await?;
        Ok(snapshot)
    }

    pub(super) fn own_run<'a>(
        &self,
        snapshot: &'a OrchestrationSnapshot,
    ) -> Result<&'a Run, CliError> {
        if matches!(self.actor, Some(Actor::Operator(_))) {
            return Err(CliError::new("actor_forbidden", "Operator mode has no job identity; name the task or run explicitly"));
        }
        let Some(Actor::Agent(caller)) = &self.actor else {
            return Err(CliError::new("credential_required", "This command requires a private job credential"));
        };
        let authorized = self.authenticated_run(caller)?;
        snapshot.runs.iter().find(|run| run.run_id == authorized.run_id)
            .ok_or_else(|| CliError::new("caller_mismatch", "Job is outside the selected root"))
    }

    pub(super) async fn task_root(
        &self,
        args: &OrchestrationArgs,
        writing: bool,
    ) -> Result<String, CliError> {
        if args.operator && writing {
            return args.root.clone().ok_or_else(|| CliError::usage("operator writes require --root"));
        }
        let snapshot = self.snapshot(None).await?;
        if writing {
            let own = self.own_run(&snapshot)?;
            if args.root.as_ref().is_some_and(|root| *root != own.root_id) {
                return Err(CliError::new(
                    "actor_forbidden",
                    "task writes are restricted to the caller run's root; explicit local management may use --operator --root ROOT",
                ));
            }
            return Ok(own.root_id.clone());
        }
        if let Some(root) = &args.root {
            return Ok(root.clone());
        }
        if matches!(self.actor, Some(Actor::Agent(_))) {
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

    pub(super) async fn mutate(
        &self,
        action: OrchestrationAction,
    ) -> Result<OrchestrationMutationResponse, CliError> {
        let operator = Actor::Operator(cockpit_protocol::orchestration::OperatorOrigin::Native);
        let actor = match self.actor.as_ref() {
            Some(actor) => actor,
            None if matches!(&action, OrchestrationAction::SupervisorStart { .. }) => &operator,
            None => return Err(CliError::new("credential_required", "Agent mutations require a private job credential")),
        };
        let request = OrchestrationMutationRequest {
            session_id: self.session.clone(), expected_revision: None, action,
        };
        if let Some(response) = self.service.operation_replay(actor, &request)? {
            return Ok(response);
        }
        self.check_caller().await?;
        let input = if matches!(&request.action,
            OrchestrationAction::SupervisorStart { .. } | OrchestrationAction::RunResume { .. }
                | OrchestrationAction::WorkerStart { .. } | OrchestrationAction::WorkerReplace { .. }
                | OrchestrationAction::ResourceRegister { .. })
        {
            let projects = cockpit_core::projects::ProjectService::new(
                self.configuration.clone(), self.adapter.clone(),
            )?;
            resolve_mutation_input(&self.service, &projects, self.adapter.as_ref(), actor, &request).await?
        } else {
            let mut input = cockpit_core::orchestration::ResolvedMutationInput::default();
            input.output_facts = self.service.resolve_output_facts(actor, &request).await?;
            input.resource_source = self.service.resolve_resource_source(actor, &request).await?;
            input
        };
        let result = self.service.mutate_resolved(actor, request, &input).map_err(|error| {
            if matches!(actor, Actor::Agent(_)) && matches!(error.code.as_str(), "actor_forbidden" | "session_mismatch") {
                CliError::new(&error.code, format!("{}; explicit local management may use --operator with --root and named TASK/RUN targets in the actual --herdr-session/--herdr-socket", error.message))
            } else { error.into() }
        })?;
        // A failure here is explicitly not a promise that the durable write was undone.
        self.check_caller().await.map_err(|error| CliError::new(
            &error.code, format!("{}; durable mutation may already be committed", error.message),
        ))?;
        Ok(result)
    }

    pub(super) async fn replacement_action(
        &self, root: &str, task: &str, selected_run: Option<&str>,
        operation_id: Option<String>, reason: String,
    ) -> Result<OrchestrationAction, CliError> {
        use cockpit_core::orchestration::JobRole;
        use cockpit_protocol::orchestration::RunStage;
        match &self.actor {
            Some(Actor::Operator(_)) => {}
            Some(Actor::Agent(caller)) => {
                let (own, role) = self.service.job_role(caller)?;
                if role != JobRole::Root || own.root_id != root {
                    return Err(CliError::new("actor_forbidden", "Only the managing root replaces task attempts in job mode; explicit local management may use --operator --root ROOT with a named TASK/RUN"));
                }
            }
            None => return Err(CliError::new("credential_required", "Replacement requires the managing root credential or --operator")),
        }
        let snapshot = self.snapshot(Some(root.into())).await?;
        let ownership = snapshot.task_ownership.iter().find(|ownership| ownership.root_id == root && ownership.task_id == task)
            .ok_or_else(|| CliError::new("task_unassigned", "Task has no current or pending assignment"))?;
        let pending_generation = ownership.generation.checked_add(1);
        let mut attempts = snapshot.runs.iter().filter(|run| {
            run.root_id == root && run.task_id.as_deref() == Some(task)
                && run.stage != RunStage::Closed
                && run.assignment_generation.is_some_and(|generation| {
                    Some(generation) == pending_generation
                        || (ownership.owner.is_some() && generation == ownership.generation)
                })
        });
        // Check uniqueness before a named run narrows the selection: replacement
        // also revokes pending authorizations, so never hide a second attempt.
        let run = attempts.next().filter(|run| {
            attempts.next().is_none()
                && ownership.owner.as_ref().is_none_or(|owner| owner.run_id == run.run_id
                    && run.assignment_generation == Some(ownership.generation))
                && selected_run.is_none_or(|selected| run.run_id == selected)
        }).ok_or_else(|| CliError::new("assignment_conflict", "Selected task attempt is no longer current or is ambiguous; inspect the task before retrying"))?;
        if reason.trim().is_empty() || reason.len() > 4096 {
            return Err(CliError::usage("Replacement --reason must be nonempty and at most 4 KiB"));
        }
        Ok(OrchestrationAction::WorkerReplace {
            operation_id: super::args::submission_id(operation_id, "Operation")?,
            root_id: root.into(), task_id: task.into(), expected_run_id: run.run_id.clone(),
            expected_assignment_generation: run.assignment_generation.expect("selected assignment generation"), reason,
        })
    }

    pub(super) async fn source_output(
        &self, message_id: &str, checks: Vec<String>, artifacts: Vec<String>,
    ) -> Result<cockpit_protocol::orchestration::SourceOutput, CliError> {
        use cockpit_protocol::orchestration::SourceOutput;
        let snapshot = self.snapshot(None).await?;
        let run = self.own_run(&snapshot)?;
        let task_id = run.task_id.as_ref().ok_or_else(|| CliError::new("task_not_assigned", "Result requires an assigned canonical task"))?;
        let task = snapshot.board.as_ref().and_then(|board| board.tasks.iter().find(|view| &view.task.task_id == task_id))
            .ok_or_else(|| CliError::new("task_not_found", "Assigned canonical task is unavailable"))?;
        let focused_checks = checks.into_iter().map(super::args::check_outcome).collect::<Result<Vec<_>, _>>()?;
        if !artifacts.is_empty() {
            return Ok(SourceOutput::NonCode { result_message_id: message_id.into(),
                task_revision: task.task.task_revision.clone(), artifacts, focused_checks });
        }
        let Some(checkout) = run.checkout.as_ref().filter(|checkout| checkout.git.is_some()) else {
            return Err(CliError::usage("Non-code Result needs actual --artifact paths or --source-output-file"));
        };
        let project = run.project.as_ref().ok_or_else(|| CliError::new("source_output_required", "Run has no proven project identity"))?;
        let base_commit = run.setup.as_ref().and_then(|setup| setup.base_commit.as_ref())
            .ok_or_else(|| CliError::new("source_output_required", "Run has no frozen pre-work base commit"))?;
        let directory = std::path::Path::new(&checkout.path);
        let head_commit = cockpit_core::space_git::git_line(directory, &["rev-parse", "--verify", "HEAD^{commit}"]).await?;
        let range = format!("{base_commit}..{head_commit}");
        let output = cockpit_core::space_git::git_output(directory, &["rev-list", "--reverse", "--topo-order", &range]).await?;
        if !output.status.success() {
            return Err(CliError::new("git_read_failed", "Cannot enumerate the actual source commit range"));
        }
        let text = std::str::from_utf8(&output.stdout).map_err(|_| CliError::new("git_read_failed", "Commit range is not UTF-8"))?;
        let commits = text.lines().map(str::to_owned).collect();
        self.check_caller().await?;
        Ok(SourceOutput::Code { repository_key: project.key.clone(), checkout_key: checkout.key.clone(),
            base_commit: base_commit.clone(), head_commit, commits,
            task_revision: task.task.task_revision.clone(), result_message_id: message_id.into(), focused_checks })
    }

}

#[cfg(test)]
#[path = "tests/replacement.rs"]
mod replacement_tests;

