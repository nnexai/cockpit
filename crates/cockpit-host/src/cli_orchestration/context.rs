use std::{io::Read, path::PathBuf, sync::Arc};

use cockpit_core::{
    config::ConfigurationFile,
    extension_adapter::SourcePaneEvidence,
    orchestration::{
        Actor, OrchestrationService,
        caller::{bound_session_conflicts, run_location_matches},
    },
};
use cockpit_herdr::HerdrCliAdapter;
use cockpit_host::orchestration_runtime::{RetryAuthority, retry_launch_preflight};
use cockpit_protocol::{
    orchestration::{
        OrchestrationAction, OrchestrationMutationRequest, OrchestrationMutationResponse,
        OrchestrationSnapshot, OrchestrationSnapshotRequest, Run, RunStage,
    },
    projects::ProjectConfiguration,
};

use super::{
    CliError,
    args::{MAX_TEXT_BYTES, OrchestrationArgs},
    caller::{attested_actor, native_process_evidence},
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
}

impl Context {
    pub(super) async fn open(
        args: &OrchestrationArgs,
        caller_required: bool,
    ) -> Result<Self, CliError> {
        args.validate_identity()?;
        let process = args.omp_pid.map(native_process_evidence).transpose()?;
        let endpoint = args.endpoint()?;
        let caller = caller_required || std::env::var("HERDR_ENV").ok().as_deref() == Some("1");
        let pane = if caller {
            Some(
                crate::current_pane_id(
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
            Some(source) => Some(attested_actor(&adapter, &session, source, args, process).await?),
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

    pub(super) async fn snapshot(
        &self,
        root: Option<String>,
    ) -> Result<OrchestrationSnapshot, CliError> {
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

    pub(super) fn own_run<'a>(
        &self,
        snapshot: &'a OrchestrationSnapshot,
    ) -> Result<&'a Run, CliError> {
        let Some(Actor::Agent(caller)) = &self.actor else {
            return Err(CliError::new(
                "caller_unbound",
                "this command requires a Herdr caller pane",
            ));
        };
        let matches =
            |run: &&Run| run.stage != RunStage::Closed && run_location_matches(run, caller);
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
        if bound_session_conflicts(run, caller) {
            return Err(CliError::new(
                "session_mismatch",
                "OMP main session differs from the run's bound session",
            ));
        }
        Ok(run)
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

    pub(super) async fn retry_launch(
        &self,
        run_id: String,
    ) -> Result<OrchestrationMutationResponse, CliError> {
        self.check_caller().await?;
        let actor = self.actor.as_ref().ok_or_else(|| {
            CliError::new("caller_unbound", "agent mutations require HERDR_ENV=1")
        })?;
        let reviewed = self.service.run_for_review(&self.session, &run_id)?;
        retry_launch_preflight(self.adapter.as_ref(), &reviewed, RetryAuthority::Supervisor)
            .await?;
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
                format!(
                    "{}; durable mutation may already be committed",
                    error.message
                ),
            )
        })?;
        Ok(result)
    }
}

pub(super) fn required_session(args: &OrchestrationArgs) -> Result<String, CliError> {
    args.omp_session
        .clone()
        .filter(|session| !session.is_empty())
        .ok_or_else(|| CliError::usage("--omp-session is required"))
}
