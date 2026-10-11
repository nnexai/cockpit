//! Shared browser/native composition; only the private runtime owner dispatches.
use cockpit_core::{
    InspectionError,
    library::LibraryService,
    orchestration::{
        Actor, OrchestrationService, ResolvedMutationInput, preferred_project_root,
        dispatch::{Dispatcher, DispatcherSettings},
        herdr::{OrchestrationHerdr, RuntimeView, RuntimeWorkspace},
        project_identity::{replacement_consumers_preflight, resolve_directory},
    },
    process_identity::{same_machine_process_running, verified_machine_id},
    projects::ProjectService,
};
use cockpit_herdr::HerdrCliAdapter;
use cockpit_protocol::{
    orchestration::*,
    projects::{ProjectConfiguration, WorkspaceSetupRequest},
};
use std::{
    fs::{self, OpenOptions},
    io::{Read, Write},
    os::unix::fs::OpenOptionsExt,
    path::{Path, PathBuf},
    sync::Arc,
    time::Duration,
};
use tokio::sync::Mutex;
use tokio_util::sync::CancellationToken;

#[derive(Clone, Copy)]
pub enum CliProcessRole {
    Host,
    Native,
}

pub struct OrchestrationRuntime {
    pub service: Arc<OrchestrationService>,
    herdr: Arc<dyn OrchestrationHerdr>,
    projects: Arc<ProjectService>,
    dispatcher: Mutex<Option<Dispatcher>>,
    task: Mutex<Option<tokio::task::JoinHandle<()>>>,
    shutdown: CancellationToken,
}
impl OrchestrationRuntime {
    pub fn new(
        configuration: &ProjectConfiguration,
        herdr: Arc<HerdrCliAdapter>,
        projects: Arc<ProjectService>,
        library: Arc<LibraryService>,
        config_path: Option<PathBuf>,
        cli_role: CliProcessRole,
    ) -> Result<Self, InspectionError> {
        let service = Arc::new(OrchestrationService::open(configuration)?);
        let extension =
            std::path::absolute(extension_path(configuration)?).map_err(extension_error)?;
        // Workers have a different cwd; transport selection must not become cwd-relative there.
        let config_path = cockpit_core::config::configuration_path(config_path.as_deref())?
            .map(std::path::absolute)
            .transpose()
            .map_err(extension_error)?;
        let herdr_socket = herdr
            .config()
            .socket
            .clone()
            .map(std::path::absolute)
            .transpose()
            .map_err(extension_error)?;
        let executable = &herdr.config().executable;
        let herdr_executable = Some(if executable.components().count() > 1 {
            std::path::absolute(executable).map_err(extension_error)?
        } else {
            executable.clone()
        });
        let settings = DispatcherSettings {
            omp_extension: extension,
            agent_kind: "omp".into(),
            start_timeout_ms: 60_000,
            cli_path: cli_path(cli_role)?,
            config_path,
            herdr_socket,
            herdr_executable,
            model: configuration.orchestration.model.clone(),
            extra_args: configuration.orchestration.extra_args.clone(),
        };
        let dispatcher =
            Dispatcher::new(service.clone(), projects.clone(), library, herdr.clone(), settings);
        Ok(Self {
            service,
            herdr,
            projects,
            dispatcher: Mutex::new(Some(dispatcher)),
            task: Mutex::new(None),
            shutdown: CancellationToken::new(),
        })
    }
    pub async fn start_owner(&self, owner: bool) {
        if !owner || self.shutdown.is_cancelled() {
            return;
        }
        let mut task = self.task.lock().await;
        if let Some(dispatcher) = self.dispatcher.lock().await.take() {
            *task = Some(dispatcher.spawn(self.shutdown.clone()));
        }
    }
    pub async fn snapshot(
        &self,
        request: &OrchestrationSnapshotRequest,
    ) -> Result<OrchestrationSnapshot, InspectionError> {
        self.service.snapshot(self.herdr.as_ref(), request).await
    }
    pub async fn mutate(
        &self,
        origin: OperatorOrigin,
        request: OrchestrationMutationRequest,
    ) -> Result<OrchestrationMutationResponse, InspectionError> {
        let actor = Actor::Operator(origin);
        if let Some(response) = self.service.operation_replay(&actor, &request)? {
            return Ok(response);
        }
        let resolved = resolve_mutation_input(
            self.service.as_ref(), self.projects.as_ref(), self.herdr.as_ref(), &actor, &request,
        ).await?;
        let service = self.service.clone();
        tokio::task::spawn_blocking(move || service.mutate_resolved(&actor, request, &resolved))
            .await
            .map_err(|error| InspectionError::new("orchestration_runtime_failed", error.to_string()))?
    }
    pub async fn project_root_lookup(
        &self,
        request: &ProjectRootLookupRequest,
    ) -> Result<ProjectRootLookupResponse, InspectionError> {
        lookup_project_roots(self.service.as_ref(), self.herdr.as_ref(), request).await
    }
    pub async fn wait(
        &self,
        request: &OrchestrationWaitRequest,
    ) -> Result<OrchestrationWaitResponse, InspectionError> {
        self.service.wait(request).await
    }
    pub async fn shutdown(&self) {
        self.shutdown.cancel();
        if let Some(mut task) = self.task.lock().await.take() {
            if tokio::time::timeout(Duration::from_secs(5), &mut task)
                .await
                .is_err()
            {
                task.abort();
                let _ = task.await;
            }
        }
    }
}

/// Fresh physical observations shared by the GUI owner and CLI. Ordinary job
/// mutations need no Herdr connection; only physical admissions inspect it.
pub async fn resolve_mutation_input(
    service: &OrchestrationService,
    projects: &ProjectService,
    herdr: &dyn OrchestrationHerdr,
    actor: &Actor,
    request: &OrchestrationMutationRequest,
) -> Result<ResolvedMutationInput, InspectionError> {
    let mut resolved = resolve_physical_mutation_input(service, projects, herdr, actor, request).await?;
    resolved.output_facts = service.resolve_output_facts(actor, request).await?;
    resolved.resource_source = service.resolve_resource_source(actor, request).await?;
    if matches!(&request.action, OrchestrationAction::ResourceRegister { .. }) {
        resolved.resource_registration = Some(service.resolve_resource_registration(
            projects, actor, &request.session_id, &request.action,
        ).await?);
    }
    Ok(resolved)
}

async fn resolve_physical_mutation_input(
    service: &OrchestrationService,
    projects: &ProjectService,
    herdr: &dyn OrchestrationHerdr,
    actor: &Actor,
    request: &OrchestrationMutationRequest,
) -> Result<ResolvedMutationInput, InspectionError> {
    match &request.action {
        OrchestrationAction::WorkerStart { target, .. } => {
            resolve_start_target(projects, herdr, &request.session_id, target).await
        }
        OrchestrationAction::SupervisorStart {
            target: Some(target), mode, selected_root_id, ..
        } => {
            let mut resolved = resolve_start_target(projects, herdr, &request.session_id, target).await?;
            if *mode == SupervisorStartMode::ReuseOrResume {
                let (_, runs) = service.recorded_runs(&request.session_id)?;
                if let Some(workspace_id) = resolved.project.as_ref().and_then(|p| p.source_workspace_id.as_deref())
                    && unknown_root_in_space(&runs, &request.session_id, workspace_id)
                {
                    return Err(unreconciled_project());
                }
                if let Some(index) = preferred_project_root(
                    &runs, &request.session_id,
                    &resolved.project.as_ref().expect("resolved start project").key,
                    selected_root_id.as_deref(),
                ) {
                    let observed = resolve_resume_input(service, herdr, &runs[index]).await?;
                    resolved.attachment_observation = observed.attachment_observation;
                    resolved.writer_exit_proof = observed.writer_exit_proof;
                    resolved.observation_cause = observed.observation_cause;
                    // Reuse/resume retains the root's original checkout, not the
                    // checkout of a different Space for the same project.
                    resolved.checkout = observed.checkout;
                }
            }
            Ok(resolved)
        }
        // Core creates and binds the dedicated directory in the start transaction.
        OrchestrationAction::SupervisorStart { target: None, .. } => {
            Ok(ResolvedMutationInput::default())
        }
        OrchestrationAction::RunAdopt { .. } => {
            let (endpoint, caller_session, workspace_id, tab_id, pane_id, terminal_id, native_session, process) = match actor {
                Actor::Bootstrap(caller) => (
                    &caller.endpoint_identity, &caller.session_id, &caller.workspace_id,
                    &caller.tab_id, &caller.pane_id, caller.terminal_id.as_ref(),
                    caller.native_session_id.as_ref(), caller.process.as_ref(),
                ),
                Actor::Agent(caller) => (
                    &caller.endpoint_identity, &caller.session_id, &caller.workspace_id,
                    &caller.tab_id, &caller.pane_id, caller.terminal_id.as_ref(),
                    caller.native_session_id.as_ref(), caller.process.as_ref(),
                ),
                Actor::Operator(_) => return Err(InspectionError::new(
                    "actor_forbidden", "Adoption requires a freshly attested native agent pane",
                )),
            };
            let runtime = herdr.runtime(&request.session_id).await?;
            if caller_session != &request.session_id || runtime.endpoint_identity != *endpoint
                || !runtime.panes.iter().any(|pane| pane.pane_id == *pane_id
                    && pane.workspace_id == *workspace_id && pane.tab_id == *tab_id
                    && terminal_id.is_some_and(|id| pane.terminal_id.as_ref() == Some(id))
                    && native_session.is_some_and(|id| pane.native_session_id.as_ref() == Some(id))
                    && pane.agent_kind.as_deref() == Some("omp"))
                || process.and_then(verified_machine_id).is_none()
            {
                return Err(InspectionError::new(
                    "caller_mismatch", "The adopted native process and exact pane must still be present",
                ));
            }
            let workspace = match authoritative_workspace(&runtime, workspace_id) {
                Ok(workspace) => workspace,
                Err(_) => return Ok(ResolvedMutationInput::default()),
            };
            // An attested adopted root remains usable when its project cannot be
            // inspected, but unavailable evidence must never merge it with a root.
            match resolve_directory(Path::new(&workspace.cwd), None, Some(workspace_id.clone())).await {
                Ok((project, checkout)) => Ok(ResolvedMutationInput {
                    project: Some(project), checkout: Some(checkout), ..Default::default()
                }),
                Err(_) => Ok(ResolvedMutationInput::default()),
            }
        }
        OrchestrationAction::RunResume { run_id, .. } => {
            let run = service.run_for_review(&request.session_id, run_id)?;
            resolve_resume_input(service, herdr, &run).await
        }
        _ => Ok(ResolvedMutationInput::default()),
    }
}

async fn resolve_start_target(
    projects: &ProjectService,
    herdr: &dyn OrchestrationHerdr,
    session: &str,
    target: &DispatchTarget,
) -> Result<ResolvedMutationInput, InspectionError> {
    let (project, checkout) = match target {
        DispatchTarget::Setup { request } => {
            let mut request = request.clone();
            match &mut request {
                WorkspaceSetupRequest::Create { focus, .. }
                | WorkspaceSetupRequest::Open { focus, .. } => *focus = false,
            }
            let plan = projects.plan(session, &request).await?;
            match request {
                WorkspaceSetupRequest::Create { .. } => {
                    let repository = plan.repository.ok_or_else(|| InspectionError::new(
                        "project_identity_unavailable", "Create plan has no verified configured repository",
                    ))?;
                    let (project, _) = resolve_directory(
                        Path::new(&repository.checkout_path), Some(repository.repository_id), None,
                    ).await?;
                    match &project.source {
                        ProjectSource::Git { common_dir } if common_dir == &repository.common_dir => {}
                        _ => return Err(InspectionError::new(
                            "repository_identity_conflict", "Configured repository common directory changed",
                        )),
                    }
                    (project, None)
                }
                WorkspaceSetupRequest::Open { .. } => {
                    let (project, checkout) = resolve_directory(
                        Path::new(&plan.checkout_path), plan.repository.map(|r| r.repository_id), None,
                    ).await?;
                    (project, Some(checkout))
                }
            }
        }
        DispatchTarget::ExistingSpace { workspace_id } => {
            let runtime = herdr.runtime(session).await?;
            let workspace = authoritative_workspace(&runtime, workspace_id)?;
            let (project, checkout) = resolve_directory(
                Path::new(&workspace.cwd), None, Some(workspace_id.clone()),
            ).await?;
            (project, Some(checkout))
        }
        DispatchTarget::SpaceWorktree { workspace_id, .. } => {
            let runtime = herdr.runtime(session).await?;
            let workspace = authoritative_workspace(&runtime, workspace_id)?;
            let source = projects.space_repository(session, workspace_id, &workspace.cwd).await?;
            let (project, _) = resolve_directory(
                Path::new(&workspace.cwd), Some(source.repository.repository_id),
                Some(workspace_id.clone()),
            ).await?;
            match &project.source {
                ProjectSource::Git { common_dir } if common_dir == &source.repository.common_dir => {}
                _ => return Err(InspectionError::new(
                    "repository_identity_conflict", "Source Space repository common directory changed",
                )),
            }
            // The source checkout is not the fresh worker's eventual checkout.
            (project, None)
        }
    };
    Ok(ResolvedMutationInput { project: Some(project), checkout, ..Default::default() })
}

fn authoritative_workspace<'a>(
    runtime: &'a RuntimeView,
    workspace_id: &str,
) -> Result<&'a RuntimeWorkspace, InspectionError> {
    let mut matching = runtime.workspaces.iter().filter(|w| w.workspace_id == workspace_id);
    let workspace = matching.next().ok_or_else(|| InspectionError::new(
        "workspace_missing", "The requested Space is not present in the fresh runtime",
    ))?;
    if matching.next().is_some() || !Path::new(&workspace.cwd).is_absolute() {
        return Err(InspectionError::new(
            "workspace_cwd_missing", "Space has no unambiguous authoritative absolute directory",
        ));
    }
    Ok(workspace)
}

fn unknown_root_in_space(runs: &[Run], session: &str, workspace_id: &str) -> bool {
    runs.iter().any(|run| run.session_id == session && run.parent_run_id.is_none()
        && run.run_id == run.root_id && run.stage != RunStage::Closed && run.project.is_none()
        && matches!(run.kind, RunKind::Supervisor | RunKind::Adopted)
        && (run.attachment.as_ref().and_then(|a| a.location.as_ref())
                .is_some_and(|location| location.workspace_id == workspace_id)
            || run.setup.as_ref().and_then(|setup| setup.project_workspace_id.as_deref()) == Some(workspace_id)
            || matches!(run.target.as_ref(),
                Some(DispatchTarget::ExistingSpace { workspace_id: saved }
                    | DispatchTarget::SpaceWorktree { workspace_id: saved, .. }) if saved == workspace_id)))
}

fn unreconciled_project() -> InspectionError {
    InspectionError::new("project_identity_unavailable",
        "An existing open supervisor for this exact Space has unresolved project identity; reconcile that root before starting another automatically")
}

async fn resolve_resume_input(
    service: &OrchestrationService,
    herdr: &dyn OrchestrationHerdr,
    run: &Run,
) -> Result<ResolvedMutationInput, InspectionError> {
    let mut resolved = ResolvedMutationInput {
        project: run.project.clone(), checkout: run.checkout.clone(), ..Default::default()
    };
    // Reattach before inspecting targets or checkout paths. Herdr endpoint
    // failure, pane removal and elapsed time cannot make this process absent.
    let mut live = run.attachment.as_ref().filter(|attachment|
        !matches!(attachment.state, AttachmentState::Revoked | AttachmentState::Quiescing)
        && attachment.process.as_ref().is_some_and(|process| matches!(
            same_machine_process_running(process, attachment.machine_id.as_deref()), Ok(true)))
    ).cloned();
    let mut attested_runtime = None;
    if live.is_none()
        && let Some(saved) = run.attachment.as_ref()
        && !matches!(saved.state, AttachmentState::Revoked | AttachmentState::Quiescing) && saved.machine_id.is_none()
        && let Some(machine) = saved.process.as_ref().and_then(verified_machine_id)
        && let Ok(runtime) = herdr.runtime(&run.session_id).await
        && exact_saved_pane(&runtime, run)
    {
        // Conversion does not stamp historical PIDs with today's machine. This
        // known same-boot process and fresh exact native pane establish it now.
        let mut observed = saved.clone();
        observed.machine_id = Some(machine);
        live = Some(observed);
        attested_runtime = Some(runtime);
    }
    if let Some(mut attachment) = live {
        attachment.state = AttachmentState::Connected;
        attachment.updated_at = time::OffsetDateTime::now_utc()
            .format(&time::format_description::well_known::Rfc3339)
            .map_err(|error| InspectionError::new("observation_time_unavailable", error.to_string()))?;
        resolved.attachment_observation = Some(attachment);
        if resolved.checkout.is_none() {
            let runtime = match attested_runtime {
                Some(runtime) => Some(runtime),
                None => herdr.runtime(&run.session_id).await.ok(),
            };
            if let Some(runtime) = runtime
                && exact_saved_pane(&runtime, run)
                && let Some(location) = run.attachment.as_ref().and_then(|a| a.location.as_ref())
            {
                let workspace = authoritative_workspace(&runtime, &location.workspace_id)?;
                let saved_path = run.setup.as_ref().map(|setup| setup.checkout_path.as_str())
                    .or_else(|| match run.target.as_ref() {
                        Some(DispatchTarget::Setup {
                            request: WorkspaceSetupRequest::Open { path, .. },
                        }) => Some(path.as_str()),
                        _ => None,
                    });
                if let Some(saved_path) = saved_path
                    && fs::canonicalize(saved_path).ok() != fs::canonicalize(&workspace.cwd).ok()
                {
                    return Err(InspectionError::new(
                        "checkout_identity_changed", "The saved live pane's Space directory changed",
                    ));
                }
                let (project, checkout) = resolve_directory(
                    Path::new(&workspace.cwd),
                    run.project.as_ref().and_then(|p| p.repository_id.clone()),
                    Some(location.workspace_id.clone()),
                ).await?;
                if run.project.as_ref().is_some_and(|saved| saved.key != project.key) {
                    return Err(InspectionError::new(
                        "project_identity_changed", "The original live pane no longer matches its saved project",
                    ));
                }
                if resolved.project.is_none() { resolved.project = Some(project); }
                resolved.checkout = Some(checkout);
            }
        }
        return Ok(resolved);
    }
    let proof = if run.parent_run_id.is_none() {
        service.observe_root_exit(run)
    } else if let Some(checkout) = run.checkout.as_ref() {
        // Revalidate the exact saved checkout, never a current Space or target.
        let (project, fresh) = resolve_directory(
            Path::new(&checkout.path),
            run.project.as_ref().and_then(|p| p.repository_id.clone()),
            run.project.as_ref().and_then(|p| p.source_workspace_id.clone()),
        ).await?;
        if &fresh != checkout || run.project.as_ref().is_none_or(|p| p.key != project.key) {
            return Err(InspectionError::new(
                "checkout_identity_changed", "The saved checkout no longer has its recorded physical identity",
            ));
        }
        service.observe_writer_exit(run, checkout)
    } else {
        Err(InspectionError::new("writer_exit_unproven", "Saved worker checkout identity is unavailable"))
    };
    let proof = match proof {
        Ok(proof) => replacement_consumers_preflight(herdr, run).await.map(|()| proof),
        Err(error) => Err(error),
    };
    // Core persists a recoverable blocked Resume with the observer's stable
    // refusal, rather than treating unobserved detached activity as exit.
    match proof {
        Ok(proof) => resolved.writer_exit_proof = Some(proof),
        Err(error) => resolved.observation_cause = Some(ResumeCause {
            code: error.code, message: error.message, class: ResumeCauseClass::Recover,
        }),
    }
    Ok(resolved)
}

fn exact_saved_pane(runtime: &RuntimeView, run: &Run) -> bool {
    let Some(attachment) = run.attachment.as_ref() else { return false; };
    let Some(location) = attachment.location.as_ref() else { return false; };
    location.session_id == run.session_id && runtime.endpoint_identity == location.endpoint_identity
        && runtime.panes.iter().any(|pane|
            pane.workspace_id == location.workspace_id && pane.tab_id == location.tab_id
            && pane.pane_id == location.pane_id
            && location.terminal_id.as_ref().is_some_and(|id| pane.terminal_id.as_ref() == Some(id))
            && location.native_session_id.as_ref().or(attachment.main_session_id.as_ref())
                .is_some_and(|id| pane.native_session_id.as_ref() == Some(id))
            && pane.agent_kind.as_deref() == Some("omp") && !pane.launch_pending)
}

pub async fn lookup_project_roots(
    service: &OrchestrationService,
    herdr: &dyn OrchestrationHerdr,
    request: &ProjectRootLookupRequest,
) -> Result<ProjectRootLookupResponse, InspectionError> {
    let resolved = async {
        let runtime = herdr.runtime(&request.session_id).await?;
        let workspace = authoritative_workspace(&runtime, &request.workspace_id)?;
        resolve_directory(Path::new(&workspace.cwd), None, Some(request.workspace_id.clone())).await
    }.await;
    let (project, _) = match resolved {
        Ok(identity) => identity,
        Err(error) => return Ok(ProjectRootLookupResponse::Unavailable {
            code: error.code, message: error.message,
        }),
    };
    let (revision, runs, bindings) = service.recorded_runs_with_bindings(&request.session_id)?;
    if unknown_root_in_space(&runs, &request.session_id, &request.workspace_id) {
        let error = unreconciled_project();
        return Ok(ProjectRootLookupResponse::Unavailable { code: error.code, message: error.message });
    }
    let preferred_root_id = preferred_project_root(
        &runs, &request.session_id, &project.key, request.selected_root_id.as_deref(),
    ).map(|index| runs[index].run_id.clone());
    let roots = runs.iter().filter(|run|
        run.parent_run_id.is_none() && run.root_id == run.run_id && run.session_id == request.session_id
        && matches!(run.kind, RunKind::Supervisor | RunKind::Adopted)
        && run.stage != RunStage::Closed
        && run.project.as_ref().is_some_and(|p| p.key == project.key)
    ).map(|run| root_candidate(run, run.attachment.as_ref().is_some_and(|a|
        bindings.get(&run.run_id).is_some_and(|bound| bound.0 == a.attachment_id && bound.1 == a.epoch)))).collect();
    Ok(ProjectRootLookupResponse::Resolved { project, lookup_revision: revision, preferred_root_id, roots })
}

fn root_candidate(run: &Run, modern_binding: bool) -> ProjectRootCandidate {
    let live = run.attachment.as_ref().filter(|attachment| attachment.state != AttachmentState::Revoked)
        .and_then(|attachment| attachment.process.as_ref().map(|process|
            same_machine_process_running(process, attachment.machine_id.as_deref())));
    let incompatible_live = !modern_binding && matches!(&live, Some(Ok(true)));
    let resume = if incompatible_live { RootResumeAvailability::Blocked } else { match run.resume.as_ref().map(|r| r.phase) {
        Some(ResumePhase::Reconciling | ResumePhase::Launching) =>
            RootResumeAvailability::InProgress,
        Some(ResumePhase::Blocked) => RootResumeAvailability::Blocked,
        _ => RootResumeAvailability::Available,
    }};
    let connection = if resume == RootResumeAvailability::InProgress {
        RootConnection::Reconnecting
    } else {
        match live {
            Some(Ok(true)) if modern_binding => RootConnection::Connected,
            Some(Ok(true)) => RootConnection::Unknown,
            Some(Ok(false)) => RootConnection::Disconnected,
            _ => RootConnection::Unknown,
        }
    };
    ProjectRootCandidate {
        root_id: run.run_id.clone(), connection, resume,
        cause: if incompatible_live { Some(ResumeCause {
            code: "current_extension_unavailable".into(),
            message: "The living writer has not authenticated its current attachment through this protocol. Its process and checkout claim are preserved; a loaded older extension is not assumed upgraded.".into(),
            class: ResumeCauseClass::Recover,
        }) } else { run.resume.as_ref().and_then(|r| r.cause.clone()) },
        updated_at: run.updated_at.clone(),
    }
}


fn cli_path(role: CliProcessRole) -> Result<PathBuf, InspectionError> {
    if let Some(path) = std::env::var_os("COCKPIT_CLI_EXECUTABLE") {
        let path = PathBuf::from(path);
        return if path.components().count() > 1 {
            std::path::absolute(path).map_err(extension_error)
        } else {
            Ok(path)
        };
    }
    let executable = std::env::current_exe()
        .map_err(|error| InspectionError::new("orchestration_runtime_failed", error.to_string()))?;
    Ok(resolve_cli_path(&executable, role))
}

fn resolve_cli_path(executable: &Path, role: CliProcessRole) -> PathBuf {
    match role {
        // The browser gateway is the actual host/CLI process, regardless of
        // its filename. Installed native GUI binaries are also named cockpit.
        CliProcessRole::Host => executable.to_owned(),
        CliProcessRole::Native => {
            let sibling = executable.with_file_name("cockpit-cli");
            if sibling.is_file() {
                return sibling;
            }
            // Cargo names the GUI cockpit-tauri and the host cockpit. This
            // development-only name must never match installed GUI cockpit.
            if executable
                .file_name()
                .is_some_and(|name| name == "cockpit-tauri")
            {
                let host = executable.with_file_name("cockpit");
                if host != executable && host.is_file() {
                    return host;
                }
            }
            PathBuf::from("cockpit-cli")
        }
    }
}

fn extension_path(configuration: &ProjectConfiguration) -> Result<PathBuf, InspectionError> {
    if let Some(path) = std::env::var_os("COCKPIT_OMP_EXTENSION") {
        return Ok(path.into());
    }
    if let Some(path) = &configuration.orchestration.omp_extension {
        return Ok(path.into());
    }
    let root = Path::new(&configuration.state_root).join("orchestration");
    let directory = root.join("omp");
    private_directory(&directory)?;
    let path = directory.join("cockpit-orchestration.ts");
    let content = include_bytes!("../../../integrations/omp/cockpit-orchestration.ts");
    let existing = OpenOptions::new()
        .read(true)
        .custom_flags(nix::libc::O_NOFOLLOW)
        .open(&path);
    match existing {
        Ok(file) => {
            if !file.metadata().map_err(extension_error)?.is_file() {
                return Err(extension_error("Extension is not a regular file"));
            }
            let mut bytes = Vec::new();
            file.take(content.len() as u64 + 1)
                .read_to_end(&mut bytes)
                .map_err(extension_error)?;
            if bytes.as_slice() == content.as_slice() {
                return Ok(path);
            }
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => return Err(extension_error(error)),
    }
    let temp = directory.join(format!(".extension-{}.tmp", uuid::Uuid::new_v4()));
    let result = (|| {
        let mut file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .custom_flags(nix::libc::O_NOFOLLOW)
            .open(&temp)
            .map_err(extension_error)?;
        file.write_all(content).map_err(extension_error)?;
        file.sync_all().map_err(extension_error)?;
        fs::rename(&temp, &path).map_err(extension_error)?;
        fs::File::open(&directory)
            .and_then(|directory| directory.sync_all())
            .map_err(extension_error)?;
        Ok(path)
    })();
    if result.is_err() {
        let _ = fs::remove_file(temp);
    }
    result
}
fn private_directory(path: &Path) -> Result<(), InspectionError> {
    use std::os::unix::fs::{DirBuilderExt, MetadataExt};
    match fs::DirBuilder::new().mode(0o700).create(path) {
        Ok(()) => {}
        Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {}
        Err(error) => return Err(extension_error(error)),
    }
    let metadata = fs::symlink_metadata(path).map_err(extension_error)?;
    if !metadata.is_dir()
        || metadata.file_type().is_symlink()
        || metadata.uid() != nix::unistd::Uid::current().as_raw()
        || metadata.mode() & 0o077 != 0
    {
        return Err(extension_error(
            "Bundled extension directory must be private and owned by the current user",
        ));
    }
    Ok(())
}
fn extension_error(error: impl std::fmt::Display) -> InspectionError {
    InspectionError::new("omp_extension_unavailable", error.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use cockpit_core::orchestration::herdr::{
        AgentTabRequest, DirectAgentTabRequest, RuntimePane, RuntimeView,
    };
    use cockpit_core::{
        HerdrAdapter, ProjectHerdrAdapter, SessionSubscription, TerminalSession,
        project_adapter::*,
    };
    use cockpit_protocol::v1 as wire;

    #[derive(Clone)]
    struct ObservedHerdr(Option<RuntimeView>);
    #[async_trait::async_trait]
    impl OrchestrationHerdr for ObservedHerdr {
        async fn runtime(&self, _: &str) -> Result<RuntimeView, InspectionError> {
            self.0
                .clone()
                .ok_or_else(|| InspectionError::new("offline", "No fresh endpoint"))
        }
        async fn create_agent_tab(
            &self,
            _: &str,
            _: &AgentTabRequest,
        ) -> Result<RunLocation, InspectionError> {
            panic!("a retry preflight must not create a terminal")
        }
        async fn create_direct_agent_tab(
            &self,
            _: &str,
            _: &DirectAgentTabRequest,
        ) -> Result<RunLocation, InspectionError> {
            panic!("a mutation resolution must not launch a direct process")
        }
        async fn pane_process_info(
            &self,
            _: &str,
            _: &str,
            _: &str,
        ) -> Result<cockpit_core::orchestration::herdr::PaneProcessInfo, InspectionError> {
            Err(InspectionError::new("fixture_process_unavailable", "The restored pane has no trusted process evidence"))
        }
        async fn close_pane(&self, _: &str, _: &str, _: &str) -> Result<(), InspectionError> {
            panic!("a retry preflight must not close a terminal")
        }
    }

    struct InspectionProjectAdapter(Option<ProjectInventory>);
    #[async_trait::async_trait]
    impl HerdrAdapter for InspectionProjectAdapter {
        async fn inspect(&self) -> Result<wire::HerdrCompatibility, InspectionError> { panic!("not inspected") }
        async fn inspect_session(&self, _: &str) -> Result<wire::HerdrCompatibility, InspectionError> { panic!("not inspected") }
        async fn sessions(&self) -> Result<wire::SessionListResponse, InspectionError> { panic!("not inspected") }
        async fn session_snapshot(&self, _: &str) -> Result<wire::SessionSnapshotResponse, InspectionError> { panic!("not inspected") }
        async fn focus(&self, _: &str, _: &wire::FocusRequest) -> Result<wire::FocusResponse, InspectionError> { panic!("resolution must not focus") }
        async fn mutate(&self, _: &str, _: &wire::ResourceMutationRequest) -> Result<wire::ResourceMutationResponse, InspectionError> { panic!("resolution must not mutate Herdr") }
        async fn subscribe_session(&self, _: &str, _: &wire::SessionSnapshotResponse) -> Result<SessionSubscription, InspectionError> { panic!("not subscribed") }
        async fn open_terminal(&self, _: &wire::TerminalOpenRequest) -> Result<TerminalSession, InspectionError> { panic!("resolution must not open a terminal") }
    }
    #[async_trait::async_trait]
    impl ProjectHerdrAdapter for InspectionProjectAdapter {
        async fn project_endpoint_identity(&self, _: &str) -> Result<String, InspectionError> { Ok("endpoint".into()) }
        async fn project_inventory(&self, _: &str, _: &str) -> Result<ProjectInventory, InspectionError> {
            Ok(self.0.clone().expect("configured repository inventory"))
        }
        async fn project_worktree(&self, _: &str, _: &ProjectWorktreeRequest) -> Result<ProjectWorktreeResult, InspectionError> { panic!("resolution must not create a checkout") }
        async fn project_terminal(&self, _: &str, _: &ProjectTerminalRequest) -> Result<ProjectTerminalResult, InspectionError> { panic!("resolution must not create a terminal") }
        async fn project_worktree_dirty(&self, _: &str, _: u32, _: u32) -> Result<bool, InspectionError> { panic!("not inspected") }
        async fn project_close_workspace(&self, _: &str, _: &str, _: &str) -> Result<(), InspectionError> { panic!("resolution must not close a workspace") }
        async fn project_remove_worktree(&self, _: &str, _: &ProjectWorktreeRemoveRequest) -> Result<(), InspectionError> { panic!("resolution must not delete a checkout") }
    }

    fn reviewed() -> Run {
        serde_json::from_value(serde_json::json!({
            "session_id":"fixture","brief":"","run_id":"run","kind":"supervisor",
            "label":"Supervisor","root_id":"run","attempt":1,"stage":"active",
            "task_revision_at_start":null,"cleanup_policy":null,
            "dispatch":{"launch_tag":"tag","endpoint_identity":"endpoint","agent_started":false,
                "step":"launch_unknown","launch_attempt":0,"updated_at":"2026-10-06T00:00:00Z"},
            "annotations":[],
            "attachment":{"attachment_id":"attachment","epoch":1,"state":"disconnected",
                "location":{"endpoint_identity":"endpoint","session_id":"fixture","workspace_id":"space",
                    "tab_id":"tab","pane_id":"pane","launch_tag":"tag","terminal_id":"terminal"},
                "main_session_id":null,"process":null,"shell":null,"machine_id":null,
                "launch_mode":"shell_agent",
                "launch_attempt":0,"updated_at":"2026-10-06T00:00:00Z"},
            "created_at":"2026-10-06T00:00:00Z","updated_at":"2026-10-06T00:00:00Z"
        })).unwrap()
    }

    fn observed() -> RuntimeView {
        RuntimeView {
            endpoint_identity: "endpoint".into(),
            boot_id: None,
            workspaces: vec![],
            panes: vec![RuntimePane {
                workspace_id: "space".into(),
                workspace_label: "Space".into(),
                tab_id: "tab".into(),
                tab_label: "tag".into(),
                pane_id: "pane".into(),
                terminal_id: Some("terminal".into()),
                native_session_id: None,
                agent_name: Some("tag".into()),
                agent_kind: Some("omp".into()),
                launch_pending: false,
                interactive_ready: false,
                agent_status: Some("working".into()),
                state_changed_at: None,
            }],
        }
    }

    fn empty_observation() -> ObservedHerdr {
        let mut runtime = observed();
        runtime.panes.clear();
        ObservedHerdr(Some(runtime))
    }

    struct Fixture {
        root: PathBuf,
        service: OrchestrationService,
    }
    impl Fixture {
        fn new() -> Self {
            let root = std::env::temp_dir().join(format!("cockpit-resume-{}", uuid::Uuid::new_v4()));
            fs::create_dir_all(&root).unwrap();
            let service = OrchestrationService::open(&ProjectConfiguration::for_tests(&root)).unwrap();
            Self { root, service }
        }
        fn projects(&self, inventory: Option<ProjectInventory>) -> ProjectService {
            ProjectService::new(ProjectConfiguration::for_tests(&self.root),
                Arc::new(InspectionProjectAdapter(inventory))).unwrap()
        }
        fn publish(&self, runs: &[Run]) {
            let state = serde_json::json!({
                "schema":2,"revision":7,"runs":runs,"messages":[],"subagents":[],
                "task_intents":[],"scope_intents":[],"assignment_intents":[],"credentials":[],
                "checkout_claims":[],"operations":[]
            });
            let mut file = OpenOptions::new().create(true).truncate(true).write(true)
                .mode(0o600).open(self.service.base().join("state.json")).unwrap();
            file.write_all(&serde_json::to_vec(&state).unwrap()).unwrap();
        }
        fn runtime(&self) -> ObservedHerdr {
            let mut runtime = observed();
            runtime.workspaces.push(RuntimeWorkspace {
                workspace_id: "space".into(), label: "Project".into(),
                cwd: self.root.to_string_lossy().into_owned(),
            });
            ObservedHerdr(Some(runtime))
        }
    }
    impl Drop for Fixture {
        fn drop(&mut self) { fs::remove_dir_all(&self.root).unwrap(); }
    }


    #[cfg(target_os = "linux")]
    fn bind_process(run: &mut Run, pid: u32) {
        let process = NativeProcessIdentity {
            pid,
            start_ticks: cockpit_core::process_identity::start_identity(pid as i32).unwrap(),
            kernel_boot_id: cockpit_core::process_identity::kernel_boot_id(),
        };
        let attachment = run.attachment.as_mut().unwrap();
        attachment.main_session_id = Some("native".into());
        attachment.machine_id = verified_machine_id(&process);
        attachment.process = Some(process);
        run.dispatch.as_mut().unwrap().agent_started = true;
    }

    #[cfg(target_os = "linux")]
    #[tokio::test]
    async fn resume_reattaches_live_process_before_inspecting_missing_checkout_or_endpoint() {
        let fixture = Fixture::new();
        let mut run = reviewed();
        bind_process(&mut run, std::process::id());
        run.kind = RunKind::Worker;
        run.parent_run_id = Some("root".into());
        run.task_id = Some("original-task".into());
        let (_, mut checkout) = resolve_directory(&fixture.root, None, None).await.unwrap();
        checkout.path = fixture.root.join("absent-checkout").to_string_lossy().into_owned();
        run.checkout = Some(checkout.clone());
        run.target = Some(DispatchTarget::ExistingSpace { workspace_id: "retargeted-space".into() });
        let resolved = resolve_resume_input(&fixture.service, &ObservedHerdr(None), &run).await.unwrap();
        assert!(resolved.writer_exit_proof.is_none());
        assert_eq!(resolved.checkout.as_ref(), Some(&checkout));
        let observed = resolved.attachment_observation.unwrap();
        assert_eq!(observed.attachment_id, "attachment");
        assert_eq!(observed.epoch, 1);
        assert_eq!(observed.process, run.attachment.as_ref().unwrap().process);
        assert_eq!(run.task_id.as_deref(), Some("original-task"));
    }


    #[cfg(target_os = "linux")]
    #[tokio::test]
    async fn resume_reboot_proof_is_same_machine_only_and_unknown_roots_stay_protected() {
        let fixture = Fixture::new();
        let mut run = reviewed();
        run.kind = RunKind::Adopted;
        bind_process(&mut run, std::process::id());
        let attachment = run.attachment.as_mut().unwrap();
        let boot = attachment.process.as_mut().unwrap().kernel_boot_id.as_mut().unwrap();
        let replacement = if boot.starts_with('0') { "1" } else { "0" };
        boot.replace_range(0..1, replacement);
        fixture.publish(&[run.clone()]);
        let resolved = resolve_resume_input(&fixture.service, &empty_observation(), &run).await.unwrap();
        assert!(resolved.writer_exit_proof.is_some());
        assert!(resolved.attachment_observation.is_none());
        let mut drifted = empty_observation();
        drifted.0.as_mut().unwrap().endpoint_identity = "new-endpoint".into();
        assert!(resolve_resume_input(&fixture.service, &drifted, &run).await.unwrap().writer_exit_proof.is_some());
        let offline = resolve_resume_input(&fixture.service, &ObservedHerdr(None), &run).await.unwrap();
        assert!(offline.writer_exit_proof.is_none());
        assert_eq!(offline.observation_cause.unwrap().code, "resume_observation_unavailable");
        for occupant in 0..3 {
            let mut restored = observed();
            match occupant {
                0 => restored.panes[0].agent_kind = Some("foreign-agent".into()),
                1 => {
                    let mut shared = restored.panes[0].clone();
                    shared.pane_id = "shared-pane".into();
                    shared.terminal_id = Some("shared-terminal".into());
                    restored.panes.push(shared);
                }
                _ => restored.panes[0].agent_kind = None,
            }
            let refused = resolve_resume_input(&fixture.service, &ObservedHerdr(Some(restored)), &run).await.unwrap();
            assert!(refused.writer_exit_proof.is_none());
            assert!(refused.attachment_observation.is_none());
            let cause = refused.observation_cause.unwrap();
            assert_eq!(cause.code, "consumer_activity_unproven", "occupant {occupant}");
            assert_eq!(cause.class, ResumeCauseClass::Recover);
        }
        for change in 0..3 {
            let mut changed = run.clone();
            let attachment = changed.attachment.as_mut().unwrap();
            match change {
                0 => attachment.machine_id = None,
                1 => attachment.machine_id = Some("foreign-machine".into()),
                _ => attachment.process = None,
            }
            fixture.publish(&[changed.clone()]);
            let resolved = resolve_resume_input(&fixture.service, &ObservedHerdr(None), &changed).await.unwrap();
            assert!(resolved.writer_exit_proof.is_none());
            assert!(resolved.attachment_observation.is_none());
        }
    }

    #[cfg(target_os = "linux")]
    #[tokio::test]
    async fn same_boot_abrupt_exit_preserves_detached_consumer_refusal() {
        let fixture = Fixture::new();
        let mut run = reviewed();
        run.kind = RunKind::Adopted;
        bind_process(&mut run, std::process::id());
        run.attachment.as_mut().unwrap().process.as_mut().unwrap().start_ticks += 1;
        fixture.publish(&[run.clone()]);
        let resolved = resolve_resume_input(&fixture.service, &ObservedHerdr(None), &run).await.unwrap();
        assert!(resolved.writer_exit_proof.is_none());
        assert!(resolved.attachment_observation.is_none());
        let cause = resolved.observation_cause.unwrap();
        assert_eq!(cause.code, "consumer_activity_unproven");
        assert_eq!(cause.class, ResumeCauseClass::Recover);
    }

    #[tokio::test]
    async fn lookup_unavailable_is_explicit_and_never_creates_state() {
        let fixture = Fixture::new();
        let request = ProjectRootLookupRequest {
            session_id: "fixture".into(), workspace_id: "space".into(), selected_root_id: None,
        };
        for runtime in [ObservedHerdr(None), empty_observation()] {
            let response = lookup_project_roots(&fixture.service, &runtime, &request).await.unwrap();
            assert!(matches!(response, ProjectRootLookupResponse::Unavailable { .. }));
        }
        assert!(!fixture.service.base().join("state.json").exists());
    }

    #[cfg(target_os = "linux")]
    #[tokio::test]
    async fn lookup_matches_saved_project_roots_and_uses_selected_then_latest() {
        let fixture = Fixture::new();
        let (project, _) = resolve_directory(&fixture.root, None, Some("space".into())).await.unwrap();
        let mut selected = reviewed();
        selected.run_id = "a".into();
        selected.root_id = "a".into();
        selected.project = Some(project.clone());
        bind_process(&mut selected, std::process::id());
        // Physical liveness, even a saved Connected flag, is not current SDK authentication.
        selected.attachment.as_mut().unwrap().state = AttachmentState::Connected;
        let mut latest = selected.clone();
        latest.run_id = "b".into();
        latest.root_id = "b".into();
        latest.updated_at = "2026-10-07T00:00:00Z".into();
        latest.attachment.as_mut().unwrap().process = None;
        let mut closed = latest.clone();
        closed.run_id = "closed".into();
        closed.root_id = "closed".into();
        closed.stage = RunStage::Closed;
        let mut unavailable = selected.clone();
        unavailable.run_id = "unavailable".into();
        unavailable.root_id = "unavailable".into();
        unavailable.project = None;
        unavailable.attachment.as_mut().unwrap().location.as_mut().unwrap().workspace_id = "other-space".into();
        fixture.publish(&[latest.clone(), selected.clone(), closed, unavailable]);
        assert!(fixture.service.recorded_runs_with_bindings("fixture").unwrap().2.is_empty());
        let before = fs::read(fixture.service.base().join("state.json")).unwrap();
        for (selected_root, preferred) in [
            (Some("a"), "a"), (None, "b"), (Some("closed"), "b"), (Some("unavailable"), "b"),
        ] {
            let request = ProjectRootLookupRequest {
                session_id: "fixture".into(), workspace_id: "space".into(),
                selected_root_id: selected_root.map(str::to_owned),
            };
            let ProjectRootLookupResponse::Resolved {
                project: actual, lookup_revision, preferred_root_id, roots,
            } = lookup_project_roots(&fixture.service, &fixture.runtime(), &request).await.unwrap()
            else { panic!("fresh local project must resolve"); };
            assert_eq!(actual.key, project.key);
            assert_eq!(lookup_revision, 7);
            assert_eq!(preferred_root_id.as_deref(), Some(preferred));
            assert_eq!(roots.len(), 2);
            let legacy = roots.iter().find(|r| r.root_id == "a").unwrap();
            assert_eq!(legacy.connection, RootConnection::Unknown);
            assert_eq!(legacy.resume, RootResumeAvailability::Blocked);
            assert_eq!(roots.iter().find(|r| r.root_id == "b").unwrap().connection, RootConnection::Unknown);
        }
        assert_eq!(before, fs::read(fixture.service.base().join("state.json")).unwrap());
    }

    #[cfg(target_os = "linux")]
    #[tokio::test]
    async fn same_space_unknown_root_blocks_automatic_start_and_lookup_not_another() {
        let fixture = Fixture::new();
        let projects = fixture.projects(None);
        fixture.publish(&[reviewed()]);
        let request = ProjectRootLookupRequest {
            session_id: "fixture".into(), workspace_id: "space".into(), selected_root_id: None,
        };
        let ProjectRootLookupResponse::Unavailable { code, .. } =
            lookup_project_roots(&fixture.service, &fixture.runtime(), &request).await.unwrap()
            else { panic!("unknown associated root must not be hidden as no match"); };
        assert_eq!(code, "project_identity_unavailable");
        let actor = Actor::Operator(OperatorOrigin::Browser);
        let request = start_request(Some(DispatchTarget::ExistingSpace { workspace_id: "space".into() }),
            SupervisorStartMode::ReuseOrResume);
        assert_eq!(resolve_mutation_input(&fixture.service, &projects, &fixture.runtime(), &actor, &request)
            .await.unwrap_err().code, "project_identity_unavailable");
        let another = start_request(Some(DispatchTarget::ExistingSpace { workspace_id: "space".into() }),
            SupervisorStartMode::Another);
        assert!(resolve_mutation_input(&fixture.service, &projects, &fixture.runtime(), &actor, &another)
            .await.unwrap().project.is_some());
        assert_eq!(fixture.service.recorded_runs("fixture").unwrap().1.len(), 1);
    }

    #[cfg(target_os = "linux")]
    #[tokio::test]
    async fn resume_does_not_reconnect_quiescing_live_attachment() {
        let fixture = Fixture::new();
        let mut run = reviewed();
        bind_process(&mut run, std::process::id());
        run.attachment.as_mut().unwrap().state = AttachmentState::Quiescing;
        fixture.publish(&[run.clone()]);
        let resolved = resolve_resume_input(&fixture.service, &ObservedHerdr(None), &run).await.unwrap();
        assert!(resolved.attachment_observation.is_none());
        assert!(resolved.writer_exit_proof.is_none());
        assert_eq!(run.attachment.unwrap().state, AttachmentState::Quiescing);
        assert!(resolved.observation_cause.is_some());
    }

    #[test]
    fn root_lookup_preserves_guarded_refusal_without_inventing_connection() {
        let mut run = reviewed();
        assert_eq!(root_candidate(&run, false).connection, RootConnection::Unknown);
        run.resume = Some(ResumeRecord {
            operation_id: "resume".into(), run_id: run.run_id.clone(),
            previous_attachment_epoch: Some(1), phase: ResumePhase::Blocked,
            cause: Some(ResumeCause {
                code: "same_boot_replacement_unsupported".into(),
                message: "Current attachment and claim are preserved".into(),
                class: ResumeCauseClass::Decision,
            }),
            decision_id: Some("decision".into()), replacement_attachment_epoch: None,
            updated_at: run.updated_at.clone(),
        });
        let candidate = root_candidate(&run, false);
        assert_eq!(candidate.resume, RootResumeAvailability::Blocked);
        assert_eq!(candidate.connection, RootConnection::Unknown);
        assert_eq!(candidate.cause.unwrap().code, "same_boot_replacement_unsupported");
    }
    #[test]
    fn living_process_without_current_authenticated_attachment_is_not_connected() {
        let mut run = reviewed();
        let pid = std::process::id();
        let Some(start_ticks) = cockpit_core::process_identity::start_identity(pid as i32) else { return };
        let Some(machine) = cockpit_core::process_identity::machine_id() else { return };
        let Some(boot) = cockpit_core::process_identity::kernel_boot_id() else { return };
        let attachment = run.attachment.as_mut().unwrap();
        attachment.process = Some(NativeProcessIdentity { pid, start_ticks, kernel_boot_id: Some(boot) });
        attachment.machine_id = Some(machine);
        attachment.state = AttachmentState::Connected;
        let legacy = root_candidate(&run, false);
        assert_eq!(legacy.connection, RootConnection::Unknown);
        assert_eq!(legacy.resume, RootResumeAvailability::Blocked);
        assert!(legacy.cause.is_some());
    }

    fn start_request(target: Option<DispatchTarget>, mode: SupervisorStartMode) -> OrchestrationMutationRequest {
        OrchestrationMutationRequest {
            session_id: "fixture".into(), expected_revision: None,
            action: OrchestrationAction::SupervisorStart {
                operation_id: uuid::Uuid::new_v4().to_string(), mode, target,
                label: None, selected_root_id: None, expected_project_key: None,
            },
        }
    }

    fn worker_start_request(target: DispatchTarget) -> OrchestrationMutationRequest {
        OrchestrationMutationRequest {
            session_id: "fixture".into(), expected_revision: None,
            action: OrchestrationAction::WorkerStart {
                operation_id: uuid::Uuid::new_v4().to_string(),
                root_id: "root".into(), task_id: "task".into(),
                expected_task_revision: "task-revision".into(),
                expected_doc_revision: "doc-revision".into(),
                target, label: None, brief: "Implement the task".into(),
                cleanup_policy: WorkerCleanupPolicy::OwnedWhenSafe,
            },
        }
    }

    #[cfg(target_os = "linux")]
    #[tokio::test]
    async fn setup_open_and_existing_space_resolve_same_exact_plain_directory() {
        let fixture = Fixture::new();
        let projects = fixture.projects(None);
        let actor = Actor::Operator(OperatorOrigin::Browser);
        let mut identities = Vec::new();
        for target in [
            DispatchTarget::ExistingSpace { workspace_id: "space".into() },
            DispatchTarget::Setup { request: WorkspaceSetupRequest::Open {
                path: fixture.root.to_string_lossy().into_owned(), label: None,
                task_name: None, focus: true,
            }},
        ] {
            for request in [
                start_request(Some(target.clone()), SupervisorStartMode::Another),
                worker_start_request(target),
            ] {
                let resolved = resolve_mutation_input(&fixture.service, &projects, &fixture.runtime(), &actor, &request)
                    .await.unwrap();
                assert_eq!(resolved.checkout.as_ref().unwrap().path, fixture.root.to_string_lossy());
                identities.push(resolved.project.unwrap().key);
            }
        }
        assert!(identities.iter().all(|key| key == &identities[0]));
        let invalid_target = DispatchTarget::Setup {
            request: WorkspaceSetupRequest::Open {
                path: ".".into(), label: None, task_name: None, focus: false,
            },
        };
        for invalid in [
            start_request(Some(invalid_target.clone()), SupervisorStartMode::Another),
            worker_start_request(invalid_target),
        ] {
            assert!(resolve_mutation_input(&fixture.service, &projects, &fixture.runtime(), &actor, &invalid).await.is_err());
        }
    }

    #[cfg(target_os = "linux")]
    #[tokio::test]
    async fn configured_create_and_space_worktree_share_project_not_source_checkout() {
        let fixture = Fixture::new();
        let repository_path = fixture.root.join("repository");
        fs::create_dir(&repository_path).unwrap();
        for args in [
            vec!["init", "-q", "-b", "main"],
            vec!["-c", "user.name=Fixture", "-c", "user.email=fixture@example.invalid",
                "commit", "--allow-empty", "-qm", "Fixture"],
        ] {
            assert!(std::process::Command::new("git").args(args).current_dir(&repository_path)
                .status().unwrap().success());
        }
        let (project, checkout) = resolve_directory(&repository_path, None, None).await.unwrap();
        let common_dir = checkout.git.as_ref().unwrap().common_dir.clone();
        let inventory = ProjectInventory {
            endpoint_identity: "endpoint".into(), repository_key: common_dir,
            repository_root: repository_path.to_string_lossy().into_owned(),
            supported_methods: vec![],
            worktrees: vec![ProjectWorktreeEntry {
                checkout_path: repository_path.to_string_lossy().into_owned(), branch: Some("main".into()),
                open_workspace_id: Some("space".into()), is_primary: true,
                is_linked_worktree: false, dirty: Some(false),
            }],
        };
        let projects = fixture.projects(Some(inventory));
        let repositories = projects.repositories().await.unwrap().repositories;
        let repository = repositories.iter().find(|r| r.checkout_path == checkout.path).unwrap();
        let mut runtime = fixture.runtime();
        runtime.0.as_mut().unwrap().workspaces[0].cwd = checkout.path.clone();
        let actor = Actor::Operator(OperatorOrigin::Browser);
        for target in [
            DispatchTarget::Setup { request: WorkspaceSetupRequest::Create {
                repository_id: repository.repository_id.clone(), branch: Some("worker".into()),
                base_ref: Some("main".into()), checkout_path: None, label: None,
                task_name: Some("worker".into()), artifact_url: None, linked_artifact_urls: vec![], focus: true,
            }},
            DispatchTarget::SpaceWorktree { workspace_id: "space".into(), branch: None, base_ref: None },
        ] {
            for request in [
                start_request(Some(target.clone()), SupervisorStartMode::Another),
                worker_start_request(target),
            ] {
                let resolved = resolve_mutation_input(&fixture.service, &projects, &runtime, &actor, &request).await.unwrap();
                assert_eq!(resolved.project.unwrap().key, project.key);
                assert!(resolved.checkout.is_none(), "eventual fresh checkout cannot be the source checkout");
            }
        }
    }

    #[cfg(target_os = "linux")]
    #[tokio::test]
    async fn no_target_start_binds_created_dedicated_directory_in_core() {
        let fixture = Fixture::new();
        let projects = fixture.projects(None);
        let actor = Actor::Operator(OperatorOrigin::Browser);
        let request = start_request(None, SupervisorStartMode::Another);
        let resolved = resolve_mutation_input(&fixture.service, &projects, &ObservedHerdr(None), &actor, &request)
            .await.unwrap();
        assert!(resolved.project.is_none());
        let response = fixture.service.mutate_resolved(&actor, request, &resolved).unwrap();
        let OrchestrationActionResult::SupervisorStarted { start } = response.result else { panic!("start result"); };
        let run = fixture.service.run_for_review("fixture", &start.root_id).unwrap();
        let Some(DispatchTarget::Setup { request: WorkspaceSetupRequest::Open { path, .. } }) = run.target
            else { panic!("dedicated directory target"); };
        let ProjectSource::Directory { root } = run.project.unwrap().source else { panic!("directory project"); };
        assert_eq!(root, path);
        assert_ne!(root, fixture.root.to_string_lossy());
        assert_eq!(run.checkout.unwrap().path, path);
    }

    #[cfg(target_os = "linux")]
    #[tokio::test]
    async fn start_preserves_selected_legacy_live_root_without_retargeting_or_new_writer() {
        let fixture = Fixture::new();
        let projects = fixture.projects(None);
        let (project, checkout) = resolve_directory(&fixture.root, None, None).await.unwrap();
        let mut run = reviewed();
        run.project = Some(project);
        run.checkout = Some(checkout.clone());
        bind_process(&mut run, std::process::id());
        run.attachment.as_mut().unwrap().state = AttachmentState::Connected;
        // This fixture has no authenticated RunAttach; preserving its living writer
        // must not become permission to launch a replacement or report SDK connection.
        fixture.publish(&[run.clone()]);
        let actor = Actor::Operator(OperatorOrigin::Browser);
        let mut request = start_request(Some(DispatchTarget::ExistingSpace { workspace_id: "space".into() }),
            SupervisorStartMode::ReuseOrResume);
        let OrchestrationAction::SupervisorStart { selected_root_id, .. } = &mut request.action
            else { unreachable!() };
        *selected_root_id = Some(run.run_id.clone());
        let resolved = resolve_mutation_input(&fixture.service, &projects, &fixture.runtime(), &actor, &request)
            .await.unwrap();
        assert_eq!(resolved.checkout.as_ref(), Some(&checkout));
        assert_eq!(resolved.attachment_observation.as_ref().unwrap().state, AttachmentState::Connected);
        let response = fixture.service.mutate_resolved(&actor, request.clone(), &resolved).unwrap();
        let OrchestrationActionResult::SupervisorStarted { start } = response.result else { panic!("start result"); };
        assert_eq!(start.outcome, SupervisorStartOutcome::Resuming);
        assert_eq!(start.root_id, run.run_id);
        let (_, runs, bindings) = fixture.service.recorded_runs_with_bindings("fixture").unwrap();
        assert_eq!(runs.len(), 1);
        assert!(bindings.is_empty());
        let preserved = &runs[0];
        assert_eq!(preserved.project, run.project);
        assert_eq!(preserved.checkout.as_ref(), Some(&checkout));
        assert_eq!(serde_json::to_value(&preserved.target).unwrap(), serde_json::to_value(&run.target).unwrap());
        let attachment = preserved.attachment.as_ref().unwrap();
        let original = run.attachment.as_ref().unwrap();
        assert_eq!(attachment.state, AttachmentState::Disconnected);
        assert_eq!(attachment.attachment_id, original.attachment_id);
        assert_eq!(attachment.epoch, original.epoch);
        assert_eq!(attachment.process, original.process);
        assert_eq!(attachment.shell, original.shell);
        assert_eq!(attachment.machine_id, original.machine_id);
        assert_eq!(attachment.launch_mode, original.launch_mode);
        assert_eq!(serde_json::to_value(&preserved.dispatch).unwrap(), serde_json::to_value(&run.dispatch).unwrap());
        let candidate = root_candidate(preserved, false);
        assert_eq!(candidate.connection, RootConnection::Unknown);
        assert_eq!(candidate.resume, RootResumeAvailability::Blocked);
        let before = fs::read(fixture.service.base().join("state.json")).unwrap();
        let offline = OrchestrationRuntime {
            service: Arc::new(OrchestrationService::open(&ProjectConfiguration::for_tests(&fixture.root)).unwrap()),
            projects: Arc::new(projects), herdr: Arc::new(ObservedHerdr(None)),
            dispatcher: Mutex::new(None), task: Mutex::new(None), shutdown: CancellationToken::new(),
        };
        let replayed = offline.mutate(OperatorOrigin::Browser, request).await.unwrap();
        let OrchestrationActionResult::SupervisorStarted { start: replayed } = replayed.result
            else { panic!("replayed start result"); };
        assert!(replayed.duplicate);
        assert_eq!(replayed.root_id, run.run_id);
        assert_eq!(before, fs::read(fixture.service.base().join("state.json")).unwrap());
    }

    #[cfg(target_os = "linux")]
    #[tokio::test]
    async fn adopt_resolves_only_fresh_exact_caller_pane_workspace() {
        let fixture = Fixture::new();
        let projects = fixture.projects(None);
        let mut run = reviewed();
        bind_process(&mut run, std::process::id());
        let caller = cockpit_core::orchestration::BootstrapCaller {
            endpoint_identity: "endpoint".into(), session_id: "fixture".into(), workspace_id: "space".into(),
            tab_id: "tab".into(), pane_id: "pane".into(), boot_id: None, terminal_id: Some("terminal".into()),
            native_session_id: Some("native".into()), omp_session_id: Some("native".into()),
            agent_kind: Some(AgentKind::Main), actual_agent_kind: Some(cockpit_core::orchestration::NativeAgentKind::Omp),
            process: run.attachment.unwrap().process,
        };
        let actor = Actor::Bootstrap(caller);
        let request = OrchestrationMutationRequest {
            session_id: "fixture".into(), expected_revision: None,
            action: OrchestrationAction::RunAdopt { label: "Adopted".into() },
        };
        let mut runtime = fixture.runtime();
        runtime.0.as_mut().unwrap().panes[0].native_session_id = Some("native".into());
        let resolved = resolve_mutation_input(&fixture.service, &projects, &runtime, &actor, &request).await.unwrap();
        assert_eq!(resolved.checkout.unwrap().path, fixture.root.to_string_lossy());
        let mut unavailable = runtime.clone();
        unavailable.0.as_mut().unwrap().workspaces[0].cwd = ".".into();
        let unbound = resolve_mutation_input(&fixture.service, &projects, &unavailable, &actor, &request)
            .await.unwrap();
        assert!(unbound.project.is_none());
        assert!(unbound.checkout.is_none());
        runtime.0.as_mut().unwrap().panes[0].native_session_id = Some("foreign".into());
        assert_eq!(resolve_mutation_input(&fixture.service, &projects, &runtime, &actor, &request)
            .await.unwrap_err().code, "caller_mismatch");
    }

    #[cfg(target_os = "linux")]
    #[tokio::test]
    async fn legacy_process_machine_is_observed_only_with_exact_fresh_native_pane() {
        let fixture = Fixture::new();
        let mut run = reviewed();
        bind_process(&mut run, std::process::id());
        run.attachment.as_mut().unwrap().machine_id = None;
        let mut runtime = fixture.runtime();
        runtime.0.as_mut().unwrap().panes[0].native_session_id = Some("native".into());
        let resolved = resolve_resume_input(&fixture.service, &runtime, &run).await.unwrap();
        assert!(resolved.attachment_observation.unwrap().machine_id.is_some());
        assert!(resolved.checkout.is_some());
        let mut retargeted = run.clone();
        retargeted.target = Some(DispatchTarget::Setup { request: WorkspaceSetupRequest::Open {
            path: fixture.root.join("previous-checkout").to_string_lossy().into_owned(),
            label: None, task_name: None, focus: false,
        }});
        assert_eq!(resolve_resume_input(&fixture.service, &runtime, &retargeted)
            .await.unwrap_err().code, "checkout_identity_changed");
        runtime.0.as_mut().unwrap().endpoint_identity = "replacement-endpoint".into();
        let refused = resolve_resume_input(&fixture.service, &runtime, &run).await.unwrap();
        assert!(refused.attachment_observation.is_none());
        assert!(refused.writer_exit_proof.is_none());
    }

    #[test]
    fn workspace_resolution_refuses_missing_relative_or_ambiguous_directory() {
        let mut runtime = observed();
        assert_eq!(authoritative_workspace(&runtime, "space").unwrap_err().code, "workspace_missing");
        runtime.workspaces.push(RuntimeWorkspace {
            workspace_id: "space".into(), label: "Space".into(), cwd: ".".into(),
        });
        assert_eq!(authoritative_workspace(&runtime, "space").unwrap_err().code, "workspace_cwd_missing");
        runtime.workspaces[0].cwd = "/absolute".into();
        runtime.workspaces.push(runtime.workspaces[0].clone());
        assert_eq!(authoritative_workspace(&runtime, "space").unwrap_err().code, "workspace_cwd_missing");
    }
    #[test]
    fn installed_native_cockpit_uses_sibling_cli_never_gui_self() {
        let root =
            std::env::temp_dir().join(format!("cockpit-installed-cli-{}", std::process::id()));
        fs::create_dir_all(&root).unwrap();
        let native = root.join("cockpit");
        let sibling = root.join("cockpit-cli");
        fs::write(&native, b"native-gui").unwrap();
        fs::write(&sibling, b"actual-cli").unwrap();
        assert_eq!(resolve_cli_path(&native, CliProcessRole::Native), sibling);
        assert_eq!(resolve_cli_path(&native, CliProcessRole::Host), native);
        fs::remove_file(&sibling).unwrap();
        assert_eq!(
            resolve_cli_path(&native, CliProcessRole::Native),
            PathBuf::from("cockpit-cli")
        );
        // Cargo's explicitly named native GUI can use the separate host
        // sibling, while the installed native GUI never resolves to itself.
        assert_eq!(
            resolve_cli_path(&root.join("cockpit-tauri"), CliProcessRole::Native),
            native
        );
        fs::remove_dir_all(root).unwrap();
    }
}
