use std::{io::Read, path::PathBuf, sync::Arc};

use cockpit_core::{
    config::ConfigurationFile,
    extension_adapter::SourcePaneEvidence,
    orchestration::{
        Actor, AgentCaller, JobRole, OrchestrationService,
    },
};
use cockpit_herdr::HerdrCliAdapter;
use cockpit_host::orchestration_runtime::resolve_mutation_input;
use cockpit_protocol::{
    orchestration::{
        AgentKind, OrchestrationAction, OrchestrationMutationRequest, OrchestrationMutationResponse,
        OrchestrationSnapshot, OrchestrationSnapshotRequest, Run,
    },
    projects::ProjectConfiguration,
};

use super::{
    CliError,
    args::{MAX_TEXT_BYTES, OrchestrationArgs},
    caller::{attested_actor, job_actor, native_process_evidence},
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

pub(super) struct Context {
    pub(super) service: OrchestrationService,
    pub(super) adapter: Arc<HerdrCliAdapter>,
    pub(super) session: String,
    pub(super) actor: Option<Actor>,
    pub(super) evidence: Option<SourcePaneEvidence>,
    pub(super) configuration: ProjectConfiguration,
    pub(super) retirement: bool,
    pub(super) child_registration: bool,
}

impl Context {
    /// Only the exact committed Result receipt may be read after acceptance.
    /// Never use this path as admission for a fresh write or ordinary job tools.
    pub(super) fn committed_report_replay(
        args: &OrchestrationArgs, action: &OrchestrationAction,
    ) -> Result<Option<OrchestrationMutationResponse>, CliError> {
        if !matches!(action, OrchestrationAction::Report { kind: cockpit_protocol::orchestration::ReportKind::Result, .. }) {
            return Ok(None);
        }
        args.validate_identity()?;
        let Some(mut actor) = job_actor(args)? else { return Ok(None) };
        let service = OrchestrationService::open(&args.configuration()?)?;
        let endpoint = args.endpoint()?;
        if let Actor::Agent(caller) = &mut actor {
            caller.session_id = endpoint.session.clone();
        }
        Ok(service.operation_replay(&actor, &OrchestrationMutationRequest {
            session_id: endpoint.session, expected_revision: None, action: action.clone(),
        })?)
    }

    /// CleanupFinish alone has dedicated core admission after acceptance.
    /// This cannot open an ordinary Context or admit another action.
    pub(super) async fn finish_cleanup(
        args: &OrchestrationArgs, action: OrchestrationAction,
    ) -> Result<OrchestrationMutationResponse, CliError> {
        let OrchestrationAction::CleanupFinish { request: finish } = &action else {
            return Err(CliError::new("actor_forbidden", "Dedicated cleanup admission only accepts CleanupFinish"));
        };
        args.validate_identity()?;
        let mut actor = job_actor(args)?.ok_or_else(|| CliError::new(
            "credential_required", "Cleanup finish requires a private job credential",
        ))?;
        let service = OrchestrationService::open(&args.configuration()?)?;
        let Actor::Agent(caller) = &mut actor else {
            return Err(CliError::new("credential_required", "Cleanup finish requires a private job credential"));
        };
        let job = service.cleanup_job_run(caller, finish)?;
        if std::env::var_os("COCKPIT_RUN_ID").is_some_and(|id| id != std::ffi::OsStr::new(&job.run_id)) {
            return Err(CliError::new("credential_mismatch", "Launch run and credential differ"));
        }
        if args.herdr_session.as_ref().is_some_and(|session| *session != job.session_id) {
            return Err(CliError::new("session_mismatch", "Job belongs to another Herdr session"));
        }
        let endpoint = resolve_endpoint(
            args.herdr.clone(), Some(job.session_id.clone()), args.herdr_socket.clone(),
            &AmbientEndpoint::from_process(),
        ).map_err(|error| CliError::new(&error.code, error.message))?;
        let process = args.omp_pid.map(native_process_evidence).transpose()?;
        let pane = crate::current_pane_id(
            endpoint.executable(), endpoint.socket(), Some(&job.session_id), "orchestration",
        ).await.map_err(|message| CliError::new("caller_unbound", message))?;
        let adapter = HerdrCliAdapter::new(endpoint.config);
        let source = adapter.source_adapter().source_pane_evidence(&job.session_id, &pane).await?;
        actor = attested_actor(&adapter, &job.session_id, &source, args, process, Some(actor)).await?;
        if let Actor::Agent(caller) = &mut actor {
            caller.env_run = Some((job.run_id, job.attempt));
        }
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

    pub(super) async fn open(
        args: &OrchestrationArgs,
        caller_required: bool,
    ) -> Result<Self, CliError> {
        Self::open_mode(args, caller_required, false, false, false).await
    }

    pub(super) async fn open_physical(
        args: &OrchestrationArgs,
        caller_required: bool,
    ) -> Result<Self, CliError> {
        Self::open_mode(args, caller_required, true, false, false).await
    }

    pub(super) async fn open_retirement(args: &OrchestrationArgs) -> Result<Self, CliError> {
        Self::open_mode(args, true, true, true, false).await
    }

    pub(super) async fn open_child_registration(args: &OrchestrationArgs) -> Result<Self, CliError> {
        Self::open_mode(args, true, true, false, true).await
    }

    pub(super) async fn open_attachment_replay(
        args: &OrchestrationArgs, child_registration: bool,
    ) -> Result<Self, CliError> {
        Self::open_mode(args, true, false, false, child_registration).await
    }

    fn authorize_read(
        service: &OrchestrationService, caller: &AgentCaller,
        retirement: bool, child_registration: bool,
    ) -> Result<Run, CliError> {
        if retirement {
            return Ok(service.job_run_retirement(caller)?);
        }
        let (run, role, subject) = service.job_role(caller)?;
        let declared_child = caller.agent_kind == Some(AgentKind::Subagent)
            || caller.subagent_id.is_some()
            || caller.provenance.as_ref().is_some_and(|p| p.child_id.is_some());
        if role == JobRole::Child {
            if caller.agent_kind != Some(AgentKind::Subagent)
                || caller.subagent_id.as_ref() != subject.as_ref()
            {
                return Err(CliError::new("actor_forbidden", "Child credential requires its own child context"));
            }
        } else if declared_child && !child_registration {
            return Err(CliError::new("actor_forbidden", "Child calls require a narrowed child credential"));
        }
        Ok(run)
    }

    pub(super) fn authenticated_run(&self, caller: &AgentCaller) -> Result<Run, CliError> {
        Self::authorize_read(&self.service, caller, self.retirement, self.child_registration)
    }

    async fn open_mode(
        args: &OrchestrationArgs,
        caller_required: bool,
        physical: bool,
        retirement: bool,
        child_registration: bool,
    ) -> Result<Self, CliError> {
        args.validate_identity()?;
        let configuration = args.configuration()?;
        let service = OrchestrationService::open(&configuration)?;
        // A present but invalid credential never falls back to bootstrap/operator authority.
        let mut actor = job_actor(args)?;
        let job = match &actor {
            Some(Actor::Agent(caller)) => Some(Self::authorize_read(
                &service, caller, retirement, child_registration,
            )?),
            _ => None,
        };
        if let Some(job) = &job {
            if std::env::var_os("COCKPIT_RUN_ID").is_some_and(|id| id != std::ffi::OsStr::new(&job.run_id)) {
                return Err(CliError::new("credential_mismatch", "Launch run and credential differ"));
            }
            if args.herdr_session.as_ref().is_some_and(|session| *session != job.session_id) {
                return Err(CliError::new("session_mismatch", "Job belongs to another Herdr session"));
            }
        }
        let agent_context = caller_required
            || std::env::var_os("COCKPIT_RUN_ID").is_some()
            || std::env::var_os("COCKPIT_ATTACHMENT_ID").is_some()
            || std::env::var("HERDR_ENV").ok().as_deref() == Some("1");
        if agent_context && actor.is_none() && !physical {
            return Err(CliError::new("credential_required", "This command requires a private job credential"));
        }
        let endpoint = match &job {
            Some(job) => resolve_endpoint(
                args.herdr.clone(), Some(job.session_id.clone()), args.herdr_socket.clone(),
                &AmbientEndpoint::from_process(),
            ).map_err(|error| CliError::new(&error.code, error.message))?,
            None => args.endpoint()?,
        };
        let process = if physical { args.omp_pid.map(native_process_evidence).transpose()? } else { None };
        let pane = if physical {
            Some(crate::current_pane_id(
                endpoint.executable(), endpoint.socket(), Some(&endpoint.session), "orchestration",
            ).await.map_err(|message| CliError::new("caller_unbound", message))?)
        } else {
            None
        };
        let session = endpoint.session;
        let adapter = Arc::new(HerdrCliAdapter::new(endpoint.config));
        let evidence = match pane {
            Some(pane) => {
                let source = adapter.source_adapter().source_pane_evidence(&session, &pane).await?;
                actor = Some(attested_actor(&adapter, &session, &source, args, process, actor.take()).await?);
                Some(source)
            },
            None => None,
        };
        if let (Some(Actor::Agent(caller)), Some(job)) = (&mut actor, &job) {
            caller.session_id = job.session_id.clone();
            if physical {
                caller.env_run = Some((job.run_id.clone(), job.attempt));
            }
        }
        let context = Self { service, adapter, session, actor, evidence, configuration, retirement, child_registration };
        context.check_caller().await?;
        Ok(context)
    }

    pub(super) async fn snapshot(
        &self,
        root: Option<String>,
    ) -> Result<OrchestrationSnapshot, CliError> {
        if self.child_registration {
            return Err(CliError::new("actor_forbidden", "Child bootstrap may only register its credential"));
        }
        self.check_caller().await?;
        let root = if let Some(Actor::Agent(caller)) = &self.actor {
            let job = self.authenticated_run(caller)?;
            if root.as_ref().is_some_and(|root| *root != job.root_id) {
                return Err(CliError::new("actor_forbidden", "Job reads are restricted to its task root"));
            }
            Some(job.root_id)
        } else {
            root
        };
        let request = OrchestrationSnapshotRequest { session_id: self.session.clone(), root_id: root };
        let snapshot = if self.evidence.is_some() {
            self.service.snapshot(&*self.adapter, &request).await?
        } else {
            self.service.snapshot_saved(&request)?
        };
        self.check_caller().await?;
        Ok(snapshot)
    }

    pub(super) fn own_run<'a>(
        &self,
        snapshot: &'a OrchestrationSnapshot,
    ) -> Result<&'a Run, CliError> {
        let Some(Actor::Agent(caller)) = &self.actor else {
            return Err(CliError::new("credential_required", "This command requires a private job credential"));
        };
        let authorized = Self::authorize_read(&self.service, caller, false, false)?;
        snapshot.runs.iter().find(|run| run.run_id == authorized.run_id)
            .ok_or_else(|| CliError::new("caller_mismatch", "Job is outside the selected root"))
    }

    pub(super) async fn task_root(
        &self,
        args: &OrchestrationArgs,
        writing: bool,
    ) -> Result<String, CliError> {
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

    pub(super) async fn mutate(
        &self,
        action: OrchestrationAction,
    ) -> Result<OrchestrationMutationResponse, CliError> {
        if self.child_registration && !matches!(&action, OrchestrationAction::JobChildRegister { .. }) {
            return Err(CliError::new("actor_forbidden", "Child bootstrap may only register its credential"));
        }
        let actor = self.actor.as_ref().ok_or_else(|| {
            CliError::new("credential_required", "Agent mutations require a private job credential")
        })?;
        let request = OrchestrationMutationRequest {
            session_id: self.session.clone(), expected_revision: None, action,
        };
        if let Some(response) = self.service.operation_replay(actor, &request)? {
            return Ok(response);
        }
        self.check_caller().await?;
        let input = if matches!(&request.action,
            OrchestrationAction::SupervisorStart { .. } | OrchestrationAction::RunResume { .. }
                | OrchestrationAction::RunAdopt { .. } | OrchestrationAction::WorkerStart { .. }
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
        let result = self.service.mutate_resolved(actor, request, &input)?;
        // A failure here is explicitly not a promise that the durable write was undone.
        self.check_caller().await.map_err(|error| CliError::new(
            &error.code, format!("{}; durable mutation may already be committed", error.message),
        ))?;
        Ok(result)
    }

}

pub(super) fn required_session(args: &OrchestrationArgs) -> Result<String, CliError> {
    args.omp_session
        .clone()
        .filter(|session| !session.is_empty())
        .ok_or_else(|| CliError::usage("--omp-session is required"))
}
