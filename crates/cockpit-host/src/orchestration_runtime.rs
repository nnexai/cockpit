//! Shared browser/native composition; only the private runtime owner dispatches.
use cockpit_core::{
    InspectionError,
    library::LibraryService,
    orchestration::{
        Actor, OrchestrationService, ResolvedMutationInput, is_project_root_candidate,
        preferred_project_root,
        dispatch::{Dispatcher, DispatcherSettings, resume_supported},
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
    fs,
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
        // Workers have a different cwd; transport selection must not become cwd-relative there.
        let config_path = cockpit_core::config::configuration_path(config_path.as_deref())?
            .map(std::path::absolute)
            .transpose()
            .map_err(runtime_error)?;
        let herdr_socket = herdr
            .config()
            .socket
            .clone()
            .map(std::path::absolute)
            .transpose()
            .map_err(runtime_error)?;
        let executable = &herdr.config().executable;
        let herdr_executable = Some(if executable.components().count() > 1 {
            std::path::absolute(executable).map_err(runtime_error)?
        } else {
            executable.clone()
        });
        let settings = DispatcherSettings {
            harness_kind: configuration.orchestration.harness_kind.clone(),
            command: configuration.orchestration.command.clone(),
            start_timeout_ms: 60_000,
            cli_path: cli_path(cli_role)?,
            config_path,
            herdr_socket,
            herdr_executable,
            extra_args: configuration.orchestration.extra_args.clone(),
            max_parallel: configuration.orchestration.max_parallel,
            resume_args: configuration.orchestration.resume_args.clone(),
            inbox_prompt_hints: configuration.orchestration.inbox_prompt_hints,
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
        match request.root_id.as_deref() {
            Some(root_id) => self.service.wait_for_root(request, root_id).await,
            None => self.service.wait(request).await,
        }
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
        OrchestrationAction::WorkerReplace { expected_run_id, .. } => {
            let run = service.run_for_review(&request.session_id, expected_run_id)?;
            let project = run.project.as_ref().ok_or_else(|| InspectionError::new(
                "replacement_source_unavailable", "The prior attempt has no authoritative project identity",
            ))?;
            let root = service.run_for_review(&request.session_id, &run.root_id)?;
            let source = project.source_workspace_id.clone()
                .or_else(|| run.setup.as_ref().and_then(|setup| setup.project_workspace_id.clone()))
                .or_else(|| run.attachment.as_ref().and_then(|attachment| attachment.location.as_ref())
                    .map(|location| location.workspace_id.clone()))
                .or_else(|| root.project.as_ref().and_then(|project| project.source_workspace_id.clone()));
            let mut resolved = if run.checkout.is_some() {
                let mut resolved = resolve_resume_input(service, herdr, &run).await?;
                let (fresh_project, checkout) = service.resolve_run_checkout(&run).await?;
                resolved.project = Some(fresh_project);
                resolved.checkout = Some(checkout);
                resolved
            } else {
                ResolvedMutationInput { project: Some(project.clone()), ..Default::default() }
            };
            match &project.source {
                ProjectSource::Directory { root: source_root } => {
                    let (fresh, _) = cockpit_core::orchestration::project_identity::resolve_owned_directory(Path::new(source_root))?;
                    if fresh.key != project.key {
                        return Err(InspectionError::new("project_identity_changed", "The original directory source changed"));
                    }
                    resolved.replacement_source = Some(cockpit_core::orchestration::ReplacementSource::Directory {
                        root: source_root.clone(), source_workspace_id: source,
                    });
                }
                ProjectSource::Git { common_dir } => {
                    let runtime = herdr.runtime(&request.session_id).await?;
                    let mut verified_space = None;
                    let mut source_checkout = None;
                    if let Some(source) = source.as_ref()
                        && let Ok(workspace) = authoritative_workspace(&runtime, source)
                        && let Ok(repository) = projects.space_repository(&request.session_id, source, &workspace.cwd).await
                        && &repository.repository.common_dir == common_dir {
                        verified_space = Some(source.clone());
                        source_checkout = Some(workspace.cwd.clone());
                    }
                    if verified_space.is_none() {
                        let Some(DispatchTarget::Setup { request: WorkspaceSetupRequest::Create { repository_id, .. } }) = run.target.as_ref() else {
                            return Err(InspectionError::new("replacement_source_unavailable", "Git replacement has neither its exact repository Space nor an original explicit configured repository target"));
                        };
                        let repository = cockpit_core::repositories::RepositoryCatalog::new(projects.configuration())
                            .resolve(repository_id).await?;
                        if &repository.common_dir != common_dir || project.repository_id.as_ref() != Some(repository_id) {
                            return Err(InspectionError::new("repository_identity_conflict", "Original configured Git target provenance changed"));
                        }
                        source_checkout = Some(repository.checkout_path);
                    }
                    let path = run.checkout.as_ref().map(|checkout| checkout.path.as_str())
                        .or(source_checkout.as_deref())
                        .ok_or_else(|| InspectionError::new("replacement_source_unavailable", "Git source has no authoritative checkout"))?;
                    let output = tokio::time::timeout(Duration::from_secs(5), cockpit_core::space_git::git_output(
                        Path::new(path), &["rev-parse", "--verify", "HEAD^{commit}"],
                    )).await.map_err(|_| InspectionError::new("replacement_source_timeout", "Committed HEAD observation timed out"))??;
                    let commit = String::from_utf8(output.stdout).map_err(runtime_error)?.trim().to_owned();
                    if !output.status.success() || !matches!(commit.len(), 40 | 64)
                        || !commit.bytes().all(|byte| byte.is_ascii_hexdigit()) {
                        return Err(InspectionError::new("replacement_source_unavailable", "Prior checkout has no verified committed HEAD"));
                    }
                    resolved.replacement_source = Some(cockpit_core::orchestration::ReplacementSource::GitHead {
                        commit, source_workspace_id: verified_space,
                    });
                }
            }
            Ok(resolved)
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
                    resolved.root_main_recovery = observed.root_main_recovery;
                    resolved.root_attachment_expected = observed.root_attachment_expected;
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
        OrchestrationAction::RunAdopt { location, .. } => {
            if !matches!(actor, Actor::Operator(_)) {
                return Err(InspectionError::new("actor_forbidden", "Only the operator can adopt a selected pane"));
            }
            let runtime = herdr.runtime(&request.session_id).await?;
            let mut panes = runtime.panes.iter().filter(|pane| pane.pane_id == location.pane_id);
            let pane = panes.next();
            if location.session_id != request.session_id
                || runtime.endpoint_identity != location.endpoint_identity
                || panes.next().is_some()
                || location.boot_id.as_ref().is_some_and(|boot| runtime.boot_id.as_ref() != Some(boot))
                || pane.is_none_or(|pane| pane.workspace_id != location.workspace_id
                    || pane.tab_id != location.tab_id
                    || location.terminal_id.as_ref().is_none_or(|id| id.is_empty() || pane.terminal_id.as_ref() != Some(id))
                    || location.native_session_id.as_ref().is_some_and(|id| pane.native_session_id.as_ref() != Some(id)))
                || runtime.panes.iter().filter(|pane| pane.terminal_id == location.terminal_id).count() != 1
            {
                return Err(InspectionError::new("caller_mismatch", "The selected exact endpoint/pane/terminal must still be present"));
            }
            let observed = ExecutionAttachment {
                attachment_id: uuid::Uuid::new_v4().to_string(), epoch: 1,
                state: AttachmentState::Connecting, launch_mode: None,
                location: Some(location.clone()), conversation: None, process: None,
                shell: None, machine_id: None, launch_attempt: 0,
                process_scope: None,
                updated_at: time::OffsetDateTime::now_utc()
                    .format(&time::format_description::well_known::Rfc3339)
                    .map_err(runtime_error)?,
            };
            let mut resolved = ResolvedMutationInput {
                attachment_observation: Some(observed), ..Default::default()
            };
            // Missing project/process observations do not invent physical ownership.
            if let Ok(workspace) = authoritative_workspace(&runtime, &location.workspace_id)
                && let Ok((project, checkout)) = resolve_directory(
                    Path::new(&workspace.cwd), None, Some(location.workspace_id.clone()),
                ).await
            {
                resolved.project = Some(project);
                resolved.checkout = Some(checkout);
            }
            Ok(resolved)
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
        DispatchTarget::OwnedAttemptDirectory { source_root, source_workspace_id } => {
            let (mut project, _) = cockpit_core::orchestration::project_identity::resolve_owned_directory(Path::new(source_root))?;
            project.source_workspace_id = source_workspace_id.clone();
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
    if run.kind != RunKind::Worker {
        return Ok(service.observe_root_resume(herdr, run).await);
    }
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
    let proof = if let Some(checkout) = run.checkout.as_ref() {
        // Revalidate the exact saved checkout, never a current Space or target.
        let (project, fresh) = service.resolve_run_checkout(run).await?;
        if &fresh != checkout || run.project.as_ref().is_none_or(|p| p.key != project.key) {
            return Err(InspectionError::new(
                "checkout_identity_changed", "The saved checkout no longer has its recorded physical identity",
            ));
        }
        let scope = match run.attachment.as_ref().and_then(|attachment| attachment.process_scope.as_ref()) {
            Some(receipt) => cockpit_core::process_scope::observe(receipt).await.ok(),
            None => None,
        };
        let cwd_clear = cockpit_core::orchestration::project_identity::checkout_consumers_clear(herdr, run, checkout).await.unwrap_or(false);
        service.observe_writer_exit(run, checkout, scope.as_ref(), cwd_clear)
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
        && location.boot_id.as_ref().zip(runtime.boot_id.as_ref()).is_none_or(|(saved, current)| saved == current)
        && runtime.panes.iter().filter(|pane| pane.pane_id == location.pane_id).count() == 1
        && location.terminal_id.as_ref().is_some_and(|terminal|
            runtime.panes.iter().filter(|pane| pane.terminal_id.as_ref() == Some(terminal)).count() == 1)
        && runtime.panes.iter().any(|pane|
            pane.workspace_id == location.workspace_id && pane.tab_id == location.tab_id
            && pane.pane_id == location.pane_id
            && location.terminal_id.as_ref().is_some_and(|id| pane.terminal_id.as_ref() == Some(id))
            && location.native_session_id.as_ref()
                .is_none_or(|id| pane.native_session_id.as_ref() == Some(id))
            && pane.agent_kind.as_deref() == Some(run.launch_config.harness_kind.as_str())
            && !pane.launch_pending)
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
        is_project_root_candidate(run, &request.session_id, &project.key)
    ).map(|run| root_candidate(run, run.attachment.as_ref().is_some_and(|a|
        bindings.get(&run.run_id).is_some_and(|bound| bound.0 == a.attachment_id && bound.1 == a.epoch)))).collect();
    Ok(ProjectRootLookupResponse::Resolved { project, lookup_revision: revision, preferred_root_id, roots })
}

fn root_candidate(run: &Run, modern_binding: bool) -> ProjectRootCandidate {
    let live = run.attachment.as_ref().filter(|attachment| attachment.state != AttachmentState::Revoked)
        .and_then(|attachment| attachment.process.as_ref().map(|process|
            same_machine_process_running(process, attachment.machine_id.as_deref())));
    let incompatible_live = !modern_binding && matches!(&live, Some(Ok(true)));
    let stale_unsupported = run.resume.as_ref().is_some_and(|record|
        record.phase == ResumePhase::Blocked
            && record.cause.as_ref().is_some_and(|cause| cause.code == "harness_resume_unsupported"))
        && resume_supported(&run.launch_config);
    let resume = if incompatible_live { RootResumeAvailability::Blocked } else { match run.resume.as_ref().map(|r| r.phase) {
        Some(ResumePhase::Reconciling | ResumePhase::Launching) =>
            RootResumeAvailability::InProgress,
        Some(ResumePhase::Blocked) if stale_unsupported => RootResumeAvailability::Available,
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
            code: "conversation_unregistered".into(),
            message: "The living writer has not registered its conversation with this run's capability. Its process and checkout claim are preserved.".into(),
            class: ResumeCauseClass::Recover,
        }) } else if stale_unsupported { None } else { run.resume.as_ref().and_then(|r| r.cause.clone()) },
        updated_at: run.updated_at.clone(),
    }
}


fn cli_path(role: CliProcessRole) -> Result<PathBuf, InspectionError> {
    let selected = if let Some(path) = std::env::var_os("COCKPIT_CLI_EXECUTABLE") {
        PathBuf::from(path)
    } else {
        let executable = std::env::current_exe()
            .map_err(|error| InspectionError::new("orchestration_runtime_failed", error.to_string()))?;
        resolve_cli_path(&executable, role)
    };
    resolve_literal_cli_path(&selected, std::env::var_os("PATH").as_deref())
}

/// Freeze PATH resolution in the owner, before workers change cwd or environment.
fn resolve_literal_cli_path(selected: &Path, path: Option<&std::ffi::OsStr>) -> Result<PathBuf, InspectionError> {
    let candidate = if selected.components().count() > 1 {
        std::path::absolute(selected).map_err(runtime_error)?
    } else {
        path.and_then(|path| std::env::split_paths(path).find_map(|directory| {
            let candidate = std::path::absolute(directory.join(selected)).ok()?;
            cli_executable_file(&candidate).then_some(candidate)
        })).ok_or_else(|| InspectionError::new("cockpit_cli_unavailable", "The exact Cockpit CLI executable was not found on the owner's PATH"))?
    };
    if !cli_executable_file(&candidate) {
        return Err(InspectionError::new("cockpit_cli_unavailable", "The selected Cockpit CLI is not an executable regular file"));
    }
    Ok(candidate)
}

fn cli_executable_file(path: &Path) -> bool {
    fs::metadata(path).is_ok_and(|metadata| {
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            metadata.is_file() && metadata.permissions().mode() & 0o111 != 0
        }
        #[cfg(not(unix))]
        { metadata.is_file() }
    })
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

fn runtime_error(error: impl std::fmt::Display) -> InspectionError {
    InspectionError::new("orchestration_runtime_unavailable", error.to_string())
}

#[cfg(test)]
mod tests;
