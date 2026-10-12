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
            return Err(CliError::new("session_mismatch", "Job belongs to another Herdr session"));
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
        // A present but invalid credential never falls back to operator authority.
        let mut actor = job_actor(args)?;
        let job = if let Some(Actor::Agent(caller)) = &mut actor {
            let job = service.job_registration_run(caller)?;
            Self::check_launch(args, &job)?;
            caller.session_id = job.session_id.clone();
            match job.attachment.as_ref().and_then(|attachment| attachment.conversation.as_ref()) {
                Some(conversation) if conversation != &caller.conversation =>
                    return Err(CliError::new("conversation_conflict", "Credential is registered to another conversation")),
                Some(_) => {},
                None => {
                    service.mutate_resolved(&Actor::Agent(caller.clone()), OrchestrationMutationRequest {
                        session_id: job.session_id.clone(), expected_revision: None,
                        action: OrchestrationAction::RunRegister { operation_id: caller.auth.credential_id.clone() },
                    }, &cockpit_core::orchestration::ResolvedMutationInput::default())?;
                },
            }
            Some(service.job_role(caller)?.0)
        } else { None };
        let agent_context = caller_required
            || std::env::var_os("COCKPIT_RUN_ID").is_some()
            || std::env::var_os("COCKPIT_ATTACHMENT_ID").is_some()
            || args.harness.is_some() || args.session_id.is_some()
            || std::env::var_os("COCKPIT_HARNESS").is_some()
            || std::env::var_os("COCKPIT_SESSION_ID").is_some()
            || std::env::var("HERDR_ENV").ok().as_deref() == Some("1");
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
                return Err(CliError::new("actor_forbidden", "Job reads are restricted to its task root"));
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
                | OrchestrationAction::WorkerStart { .. }
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

