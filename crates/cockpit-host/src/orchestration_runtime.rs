//! Shared browser/native composition; only the private runtime owner dispatches.
use cockpit_core::{
    InspectionError,
    library::LibraryService,
    orchestration::{
        Actor, OrchestrationService, ResolvedMutationInput, effect_free_reserved_resume_launch,
        is_liaison_candidate, preferred_liaison,
        dispatch::{Dispatcher, DispatcherSettings, resume_supported},
        herdr::{WorkspaceDirectoryEvidence, WorkspaceDirectorySource, OrchestrationHerdr, RuntimeView, RuntimeWorkspace},
        project_identity::{self, replacement_consumers_preflight, resolve_directory},
    },
    process_identity::{same_machine_process_running, verified_machine_id},
    projects::ProjectService,
    project_adapter::ProjectSnapshot,
    ProjectHerdrAdapter,
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

/// Explicit caller selection; at least a repository or Space is required.
/// Session is bound by the configured adapter selection and socket, not by a
/// Herdr-reported session field. No field is filled from current focus.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VoiceScopeRequest {
    pub session_id: String,
    pub workspace_id: Option<String>,
    pub repository_id: Option<String>,
    pub branch: Option<String>,
    pub base: Option<String>,
}

/// Host-observed identity suitable for comparing with a fresh resolution.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VoiceScopeEvidence {
    pub request: VoiceScopeRequest,
    pub endpoint_identity: String,
    /// Cockpit's endpoint-derived server instance, not a Herdr-issued identifier.
    pub endpoint_derived_server_instance: String,
    pub boot_id: Option<String>,
    pub peer_process: Option<NativeProcessIdentity>,
    pub workspace_directory: Option<String>,
    pub workspace_directory_source: Option<WorkspaceDirectorySource>,
    pub project: ProjectIdentity,
    pub checkout: CheckoutIdentity,
}

pub struct OrchestrationRuntime {
    pub service: Arc<OrchestrationService>,
    herdr: Arc<dyn OrchestrationHerdr>,
    agent_send_adapter: Option<Arc<HerdrCliAdapter>>,
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
            quiet_period: Duration::from_secs(configuration.orchestration.quiet_period_seconds.into()),
        };
        let dispatcher =
            Dispatcher::new(service.clone(), projects.clone(), library, herdr.clone(), settings);
        Ok(Self {
            service,
            agent_send_adapter: Some(herdr.clone()),
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
    /// Resolve explicit Voice scope from fresh, independently bound Herdr observations.
    pub async fn resolve_voice_scope(
        &self, request: &VoiceScopeRequest,
    ) -> Result<VoiceScopeEvidence, InspectionError> {
        let adapter = self.agent_send_adapter.as_ref().ok_or_else(|| scope_error("Herdr adapter is unavailable"))?;
        resolve_voice_scope(self.projects.as_ref(), adapter.as_ref(), self.herdr.as_ref(), request).await
    }
    pub async fn inbox(&self, request: &InboxRequest) -> Result<InboxView, InspectionError> {
        self.service.inbox(self.herdr.as_ref(), request).await
    }
    pub async fn send_agent(
        &self, session: &str, reference: &str, text: String, origin: OperatorOrigin, expected_target: Option<AgentSendTarget>,
    ) -> Result<serde_json::Value, InspectionError> {
        let adapter = self.agent_send_adapter.as_ref()
            .ok_or_else(|| InspectionError::new("herdr_unavailable", "Agent delivery is unavailable"))?;
        crate::agent_send::send_to_agent(
            self.service.as_ref(), adapter.as_ref(), session, None, reference, text, origin, false, true, MessageKind::Instruction, expected_target,
            |_, _| Ok(None),
        ).await
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
    /// Resolve an explicit Voice handoff using the same host rules as Inbox.
    pub async fn resolve_handoff_binding(
        &self,
        origin: OperatorOrigin,
        request: &OrchestrationMutationRequest,
    ) -> Result<Option<HandoffBindingRecord>, InspectionError> {
        if !matches!(&request.action, OrchestrationAction::ThreadHandoff { .. }) {
            return Err(InspectionError::new("invalid_handoff", "Expected a thread handoff"));
        }
        let actor = Actor::Operator(origin);
        Ok(resolve_mutation_input(
            self.service.as_ref(), self.projects.as_ref(), self.herdr.as_ref(), &actor, request,
        ).await?.handoff_binding)
    }
    pub async fn liaison_lookup(
        &self,
        request: &LiaisonLookupRequest,
    ) -> Result<LiaisonLookupResponse, InspectionError> {
        lookup_liaison(self.service.as_ref(), self.herdr.as_ref(), request).await
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
        if let Some(mut task) = self.task.lock().await.take()
            && tokio::time::timeout(Duration::from_secs(5), &mut task)
                .await
                .is_err()
            {
                task.abort();
                let _ = task.await;
            }
    }
}

fn verify_voice_repository(
    expected_id: &str, actual_id: &str, common_dir: &str,
    project: &ProjectIdentity, checkout: &CheckoutIdentity,
) -> Result<(), InspectionError> {
    if expected_id != actual_id
        || !matches!(&project.source, ProjectSource::Git { common_dir: actual } if actual == common_dir)
        || checkout.git.as_ref().is_none_or(|git| git.common_dir != common_dir)
    {
        return Err(scope_error("Requested repository does not match the Space checkout"));
    }
    Ok(())
}

fn scope_error(message: impl Into<String>) -> InspectionError {
    InspectionError::new("voice_scope_unavailable", message.into())
}

fn valid_scope_value(value: &str) -> bool {
    !value.is_empty() && value.len() <= 4096 && !value.chars().any(char::is_control)
}

fn validate_voice_observation(
    request: &VoiceScopeRequest, endpoint: &str, snapshot: &ProjectSnapshot,
    explicit: &WorkspaceDirectoryEvidence, runtime: &RuntimeView,
) -> Result<(), InspectionError> {
    let snap = &snapshot.snapshot;
    if !valid_scope_value(endpoint) || !valid_scope_value(&snap.server_instance)
        || snapshot.endpoint_identity != endpoint || explicit.endpoint_identity != endpoint
        || runtime.endpoint_identity != endpoint || snap.session_id != request.session_id
        || explicit.session_id != request.session_id || explicit.server_instance != snap.server_instance
        || explicit.workspace_id != request.workspace_id
        || snapshot.boot_id != explicit.boot_id || snapshot.boot_id != runtime.boot_id
        || snapshot.peer_process != explicit.peer_process || snapshot.peer_process != runtime.peer_process
        || snapshot.peer_process.is_none()
        || snapshot.boot_id.as_deref().is_some_and(|id| !valid_scope_value(id))
    {
        return Err(scope_error("Herdr scope provenance is missing or changed"));
    }
    if let Some(workspace_id) = request.workspace_id.as_deref() {
        if snap.spaces.iter().filter(|space| space.id == workspace_id).count() != 1 {
            return Err(scope_error("Requested Space is missing or ambiguous in the attested snapshot"));
        }
        let workspace = authoritative_workspace(runtime, workspace_id)?;
        let directory = explicit.directory.as_deref().ok_or_else(|| scope_error("Space directory evidence is missing"))?;
        let source = explicit.directory_source.ok_or_else(|| scope_error("Space directory source is missing"))?;
        if !Path::new(directory).is_absolute() || workspace.cwd != directory {
            return Err(scope_error("Space directory differs from the runtime observation"));
        }
        if source == WorkspaceDirectorySource::WorktreeCheckout
            && snap.spaces.iter().find(|space| space.id == workspace_id)
                .and_then(|space| space.git.as_ref()).is_none_or(|git| git.checkout_path != directory)
        {
            return Err(scope_error("Space worktree checkout differs from the attested snapshot"));
        }
    } else if explicit.directory.is_some() || explicit.directory_source.is_some() {
        return Err(scope_error("Unexpected Space directory without a requested Space"));
    }
    Ok(())
}

async fn observe_voice_scope(
    adapter: &dyn ProjectHerdrAdapter, herdr: &dyn OrchestrationHerdr, request: &VoiceScopeRequest,
) -> Result<(ProjectSnapshot, WorkspaceDirectoryEvidence), InspectionError> {
    let endpoint = adapter.project_endpoint_identity(&request.session_id).await?;
    let snapshot = adapter.project_snapshot_at_endpoint(&request.session_id, &endpoint).await?;
    let explicit = herdr.workspace_directory_evidence(&request.session_id, request.workspace_id.as_deref(), &endpoint).await?;
    let runtime = herdr.runtime(&request.session_id).await?;
    validate_voice_observation(request, &endpoint, &snapshot, &explicit, &runtime)?;
    if adapter.project_endpoint_identity(&request.session_id).await? != endpoint {
        return Err(scope_error("Herdr endpoint changed while resolving scope"));
    }
    Ok((snapshot, explicit))
}

async fn find_voice_repository(
    projects: &ProjectService, id: Option<&str>,
) -> Result<Option<cockpit_protocol::projects::RepositoryCandidate>, InspectionError> {
    let Some(id) = id else { return Ok(None) };
    let listed = projects.repositories().await?;
    let mut matching = listed.repositories.into_iter().filter(|repo|
        repo.repository_id == id && !repo.is_linked_worktree);
    let repository = matching.next().ok_or_else(|| scope_error("Requested configured repository is missing"))?;
    if matching.next().is_some() {
        return Err(scope_error("Requested configured repository is ambiguous"));
    }
    Ok(Some(repository))
}

async fn resolve_voice_scope(
    projects: &ProjectService, adapter: &dyn ProjectHerdrAdapter, herdr: &dyn OrchestrationHerdr,
    request: &VoiceScopeRequest,
) -> Result<VoiceScopeEvidence, InspectionError> {
    if !valid_scope_value(&request.session_id)
        || (request.workspace_id.is_none() && request.repository_id.is_none())
        || request.workspace_id.as_deref().is_some_and(|id| !valid_scope_value(id))
        || request.repository_id.as_deref().is_some_and(|id| !valid_scope_value(id))
        || request.branch.as_deref().is_some_and(|v| !valid_scope_value(v))
        || request.base.as_deref().is_some_and(|v| !valid_scope_value(v))
    {
        return Err(scope_error("Explicit scope contains an empty or invalid selection"));
    }
    let (snapshot, explicit) = observe_voice_scope(adapter, herdr, request).await?;
    let repository = find_voice_repository(projects, request.repository_id.as_deref()).await?;
    let directory = explicit.directory.as_deref().or_else(|| repository.as_ref().map(|r| r.checkout_path.as_str()))
        .ok_or_else(|| scope_error("Scope has no checkout directory"))?;
    let (project, checkout) = resolve_directory(
        Path::new(directory), request.repository_id.clone(), request.workspace_id.clone(),
    ).await?;
    if let Some(repository) = &repository {
        verify_voice_repository(
            request.repository_id.as_deref().expect("repository is present only when requested"),
            &repository.repository_id, &repository.common_dir, &project, &checkout,
        )?;
    }
    let (fresh_snapshot, fresh_explicit) = observe_voice_scope(adapter, herdr, request).await?;
    let fresh_repository = find_voice_repository(projects, request.repository_id.as_deref()).await?;
    if repository.as_ref().map(|r| (&r.repository_id, &r.checkout_path, &r.common_dir))
        != fresh_repository.as_ref().map(|r| (&r.repository_id, &r.checkout_path, &r.common_dir))
    {
        return Err(scope_error("Configured repository changed while resolving scope"));
    }
    let fresh_directory = fresh_explicit.directory.as_deref()
        .or_else(|| fresh_repository.as_ref().map(|r| r.checkout_path.as_str()))
        .ok_or_else(|| scope_error("Scope has no checkout directory"))?;
    let (fresh_project, fresh_checkout) = resolve_directory(
        Path::new(fresh_directory), request.repository_id.clone(), request.workspace_id.clone(),
    ).await?;
    if snapshot.endpoint_identity != fresh_snapshot.endpoint_identity
        || snapshot.snapshot.server_instance != fresh_snapshot.snapshot.server_instance
        || snapshot.boot_id != fresh_snapshot.boot_id || snapshot.peer_process != fresh_snapshot.peer_process
        || explicit != fresh_explicit || project != fresh_project || checkout != fresh_checkout
    {
        return Err(scope_error("Space identity changed while resolving scope"));
    }
    Ok(VoiceScopeEvidence {
        request: request.clone(), endpoint_identity: snapshot.endpoint_identity,
        endpoint_derived_server_instance: snapshot.snapshot.server_instance, boot_id: snapshot.boot_id,
        peer_process: snapshot.peer_process, workspace_directory: explicit.directory,
        workspace_directory_source: explicit.directory_source, project, checkout,
    })
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
    resolved.images = service.resolve_images(request).await?;
    let catalog = cockpit_core::repositories::RepositoryCatalog::new(projects.configuration());
    resolved.output_facts = service.resolve_output_facts(actor, request, &catalog).await?;
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
        OrchestrationAction::ThreadHandoff { binding: Some(binding), .. } => {
            let repository_common_dir = match binding.repository_id.as_deref() {
                Some(id) => Some(cockpit_core::repositories::RepositoryCatalog::new(projects.configuration())
                    .resolve(id).await.map_err(handoff_unavailable)?.common_dir),
                None => None,
            };
            let workspace_checkout = match binding.workspace_id.as_deref() {
                Some(id) => {
                    let runtime = herdr.runtime(&request.session_id).await.map_err(handoff_unavailable)?;
                    let workspace = authoritative_workspace(&runtime, id).map_err(handoff_unavailable)?;
                    let (_, checkout) = resolve_directory(Path::new(&workspace.cwd), None, Some(id.to_owned()))
                        .await.map_err(handoff_unavailable)?;
                    let (branch, head) = if checkout.git.is_some() {
                        (checkout_branch(Path::new(&checkout.path)).await.map_err(handoff_unavailable)?,
                         checkout_head(Path::new(&checkout.path)).await.map_err(handoff_unavailable)?)
                    } else { (None, None) };
                    Some(BoundCheckout {
                        path: checkout.path,
                        common_dir: checkout.git.map(|git| git.common_dir),
                        branch,
                        head,
                    })
                }
                None => None,
            };
            Ok(ResolvedMutationInput {
                handoff_binding: Some(HandoffBindingRecord {
                    requested: binding.clone(), repository_common_dir, workspace_checkout,
                }),
                ..Default::default()
            })
        }
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
                if resolved.writer_exit_proof.is_some()
                    && matches!(run.target.as_ref(),
                        Some(DispatchTarget::ExistingSpace { .. })
                        | Some(DispatchTarget::Setup { request: WorkspaceSetupRequest::Open { .. } }))
                    && let Some(checkout) = resolved.checkout.as_ref().filter(|checkout| checkout.git.is_some())
                {
                    resolved.checkout_branch = checkout_branch(Path::new(&checkout.path)).await?;
                }
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
        OrchestrationAction::SupervisorStart { selected_root_id, target, .. } => {
            let (_, runs) = service.recorded_runs(&request.session_id)?;
            if let Some(index) = preferred_liaison(&runs, &request.session_id, selected_root_id.as_deref()) {
                let run = &runs[index];
                if selected_root_id.as_deref() == Some(run.run_id.as_str())
                    && cockpit_core::orchestration::root_checkout_reselection_pending(run)
                    && let Some(DispatchTarget::ExistingSpace { workspace_id }) = target {
                    let boot = cockpit_core::orchestration::project_identity::local_boot_marker()
                        .ok_or_else(|| InspectionError::new("checkout_reselection_unavailable", "Local boot identity is unavailable"))?;
                    let runtime = herdr.runtime(&request.session_id).await?;
                    let identity_reselection = run.dispatch.as_ref().and_then(|dispatch| dispatch.error.as_ref())
                        .is_some_and(|error| error.code == "retained_space_identity_unproven");
                    let cwd = if identity_reselection {
                        if runtime.workspaces.iter().filter(|space| &space.workspace_id == workspace_id).count() != 1 {
                            return Err(InspectionError::new("workspace_missing", "The selected Space is not unique in the fresh runtime"));
                        }
                        &run.checkout.as_ref().ok_or_else(|| InspectionError::new("checkout_reselection_unavailable", "Saved checkout is unavailable"))?.path
                    } else { &authoritative_workspace(&runtime, workspace_id)?.cwd };
                    let (project, checkout) = resolve_directory(
                        Path::new(cwd), None, Some(workspace_id.clone()),
                    ).await?;
                    if cockpit_core::orchestration::project_identity::local_boot_marker().as_deref() != Some(boot.as_str()) {
                        return Err(InspectionError::new("checkout_reselection_unavailable", "Local boot changed during Space observation"));
                    }
                    Ok(ResolvedMutationInput {
                        project: Some(project), reselected_checkout: Some(checkout),
                        reselected_endpoint: Some(runtime.endpoint_identity), reselected_boot: Some(boot), ..Default::default()
                    })
                } else {
                    resolve_resume_input(service, herdr, run).await
                }
            } else {
                Ok(ResolvedMutationInput::default())
            }
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
        OrchestrationAction::RunResume { run_id, confirmed_stopped_attachment_id, .. } => {
            let run = service.run_for_review(&request.session_id, run_id)?;
            match confirmed_stopped_attachment_id {
                Some(id) => Ok(service.observe_confirmed_root_stop(herdr, &run, id).await),
                None => resolve_resume_input(service, herdr, &run).await,
            }
        }
        OrchestrationAction::RunStartFreshSupervisor { run_id, .. } => {
            if !matches!(actor, Actor::Operator(_)) {
                return Err(InspectionError::new("actor_forbidden", "Only the operator may start a fresh supervisor conversation"));
            }
            let run = service.run_for_review(&request.session_id, run_id)?;
            let mut input = service.observe_root_resume(herdr, &run).await;
            if let Some(observed) = input.attachment_observation.as_ref()
                && let Some(location) = observed.location.as_ref()
                && let Some(process) = observed.process.as_ref() {
                let runtime = herdr.runtime(&request.session_id).await?;
                let panes: Vec<_> = runtime.panes.iter().filter(|pane| pane.pane_id == location.pane_id).collect();
                if runtime.endpoint_identity == location.endpoint_identity
                    && runtime.boot_id == location.boot_id && runtime.peer_process == location.peer_process
                    && panes.len() == 1 && panes[0].workspace_id == location.workspace_id
                    && panes[0].tab_id == location.tab_id && panes[0].terminal_id == location.terminal_id
                    && location.terminal_id.as_deref().is_some_and(|id| !id.is_empty())
                    && runtime.panes.iter().filter(|pane| pane.terminal_id == location.terminal_id).count() == 1
                    && !panes[0].launch_pending
                    && matches!(panes[0].agent_status.as_deref(), Some("idle" | "done")) {
                    let info = herdr.pane_process_info(&request.session_id, &runtime.endpoint_identity, &location.pane_id).await?;
                    input.fresh_root_idle = info.pane_id == location.pane_id
                        && info.processes.iter().any(|(pid, _)| *pid == process.pid);
                }
            }
            if input.root_main_recovery.is_none() && !input.fresh_root_idle {
                return Err(InspectionError::new("fresh_supervisor_busy",
                    "The supervisor must be idle with an exactly identified terminal, or its exact exit must be proven, before starting a fresh conversation"));
            }
            Ok(input)
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
    let checkout_branch = if matches!(target, DispatchTarget::ExistingSpace { .. } | DispatchTarget::Setup { request: WorkspaceSetupRequest::Open { .. } }) {
        match checkout.as_ref() {
            Some(checkout) if checkout.git.is_some() => checkout_branch(Path::new(&checkout.path)).await?,
            _ => None,
        }
    } else { None };
    Ok(ResolvedMutationInput { project: Some(project), checkout, checkout_branch, ..Default::default() })
}

fn handoff_unavailable(error: impl std::fmt::Display) -> InspectionError {
    InspectionError::new("handoff_binding_unavailable", error.to_string())
}

async fn checkout_branch(path: &Path) -> Result<Option<String>, InspectionError> {
    let output = tokio::time::timeout(Duration::from_secs(5), cockpit_core::space_git::git_output(
        path, &["symbolic-ref", "--quiet", "--short", "HEAD"],
    )).await.map_err(|_| runtime_error("Git branch observation timed out"))??;
    if output.status.success() {
        let branch = String::from_utf8(output.stdout).map_err(runtime_error)?.trim().to_owned();
        if branch.is_empty() { return Err(runtime_error("Git returned an empty branch")); }
        Ok(Some(branch))
    } else if output.status.code() == Some(1) {
        Ok(None)
    } else {
        Err(runtime_error("Git branch observation failed"))
    }
}

async fn checkout_head(path: &Path) -> Result<Option<String>, InspectionError> {
    let output = tokio::time::timeout(Duration::from_secs(5), cockpit_core::space_git::git_output(
        path, &["rev-parse", "--verify", "--quiet", "HEAD^{commit}"],
    )).await.map_err(|_| runtime_error("Git HEAD observation timed out"))??;
    if !output.status.success() {
        return if output.status.code() == Some(1) {
            Ok(None)
        } else { Err(runtime_error("Git HEAD observation failed")) };
    }
    let head = String::from_utf8(output.stdout).map_err(runtime_error)?.trim().to_owned();
    if !matches!(head.len(), 40 | 64) || !head.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return Err(runtime_error("Git returned an invalid HEAD"));
    }
    Ok(Some(head))
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
                    .or(match run.target.as_ref() {
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

pub async fn lookup_liaison(
    service: &OrchestrationService,
    herdr: &dyn OrchestrationHerdr,
    request: &LiaisonLookupRequest,
) -> Result<LiaisonLookupResponse, InspectionError> {
    if let Err(error) = herdr.runtime(&request.session_id).await {
        return Ok(LiaisonLookupResponse::Unavailable { code: error.code, message: error.message });
    }
    let (revision, runs, bindings) = service.recorded_runs_with_bindings(&request.session_id)?;
    let preferred_root_id = preferred_liaison(
        &runs, &request.session_id, request.selected_root_id.as_deref(),
    ).map(|index| runs[index].run_id.clone());
    let roots = runs.iter().filter(|run|
        is_liaison_candidate(run, &request.session_id)
    ).map(|run| root_candidate(run, run.attachment.as_ref().is_some_and(|a|
        bindings.get(&run.run_id).is_some_and(|bound| bound.0 == a.attachment_id && bound.1 == a.epoch)))).collect();
    Ok(LiaisonLookupResponse::Resolved { lookup_revision: revision, preferred_root_id, roots })
}

fn root_candidate(run: &Run, modern_binding: bool) -> LiaisonCandidate {
    let live = run.attachment.as_ref().filter(|attachment| attachment.state != AttachmentState::Revoked)
        .and_then(|attachment| attachment.process.as_ref().map(|process|
            same_machine_process_running(process, attachment.machine_id.as_deref())));
    let incompatible_live = !modern_binding && matches!(&live, Some(Ok(true)));
    let stale_unsupported = run.resume.as_ref().is_some_and(|record|
        record.phase == ResumePhase::Blocked
            && record.cause.as_ref().is_some_and(|cause| cause.code == "harness_resume_unsupported"))
        && resume_supported(&run.launch_config);
    let refused_launch = run.resume.as_ref()
        .filter(|resume| resume.phase == ResumePhase::Launching)
        .filter(|_| effect_free_reserved_resume_launch(run))
        .and_then(|_| run.dispatch.as_ref()?.error.as_ref());
    let retryable_refusal = refused_launch.is_some_and(|error|
        (error.code == "harness_resume_unsupported" && resume_supported(&run.launch_config))
            || (matches!(error.code.as_str(), "workspace_missing" | "retained_space_checkout_changed" | "retained_space_identity_unproven")
                && run.kind == RunKind::Supervisor));
    let dead_unregistered_launch = project_identity::retryable_dead_unregistered_resume(run);
    let confirmable_unknown_launch = run.resume.as_ref().is_some_and(|resume| resume.phase == ResumePhase::Launching)
        && project_identity::root_stop_confirmable(run);
    let resume = if incompatible_live { RootResumeAvailability::Blocked }
    else if dead_unregistered_launch || confirmable_unknown_launch { RootResumeAvailability::Available }
    else if refused_launch.is_some() {
        if retryable_refusal { RootResumeAvailability::Available }
        else { RootResumeAvailability::Blocked }
    } else { match run.resume.as_ref().map(|r| r.phase) {
        Some(ResumePhase::Reconciling | ResumePhase::Launching) =>
            RootResumeAvailability::InProgress,
        Some(ResumePhase::Blocked) if stale_unsupported => RootResumeAvailability::Available,
        Some(ResumePhase::Blocked) => RootResumeAvailability::Blocked,
        _ => RootResumeAvailability::Available,
    }};
    let connection = if dead_unregistered_launch || refused_launch.is_some() && !incompatible_live {
        RootConnection::Disconnected
    } else if resume == RootResumeAvailability::InProgress {
        RootConnection::Reconnecting
    } else {
        match live {
            Some(Ok(true)) if modern_binding => RootConnection::Connected,
            Some(Ok(true)) => RootConnection::Unknown,
            Some(Ok(false)) => RootConnection::Disconnected,
            _ => RootConnection::Unknown,
        }
    };
    LiaisonCandidate {
        root_id: run.run_id.clone(), connection, resume,
        stop_confirmable: project_identity::root_stop_confirmable(run),
        fresh_available: Some(project_identity::fresh_root_available(run)),
        cause: if incompatible_live { Some(ResumeCause {
            code: "conversation_unregistered".into(),
            message: "The living writer has not registered its conversation with this run's capability. Its process and checkout claim are preserved.".into(),
            class: ResumeCauseClass::Recover,
        }) } else if let Some(error) = refused_launch {
            if retryable_refusal { None } else { Some(ResumeCause {
                code: error.code.clone(),
                message: error.message.clone(),
                class: if error.code == "checkout_identity_changed" {
                    ResumeCauseClass::Decision
                } else {
                    ResumeCauseClass::Recover
                },
            }) }
        } else if stale_unsupported { None } else { run.resume.as_ref().and_then(|r| r.cause.clone()) },
        updated_at: run.updated_at.clone(),
    }
}


pub fn cli_path(role: CliProcessRole) -> Result<PathBuf, InspectionError> {
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

pub fn resolve_cli_path(executable: &Path, role: CliProcessRole) -> PathBuf {
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
            #[cfg(target_os = "macos")]
            if let Some(home) = std::env::var_os("HOME") {
                let installed = PathBuf::from(home)
                    .join("Library/Application Support/cockpit/bin/cockpit-cli");
                if cli_executable_file(&installed) {
                    return installed;
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
