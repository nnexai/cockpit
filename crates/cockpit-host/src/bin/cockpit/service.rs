use super::*;
use cockpit_host::{
    backend::{self, BackendProbe, BackendServer, StopReason},
    lifecycle::HerdrEndpoint,
};
use cockpit_core::ProjectHerdrAdapter;
use cockpit_host::voice::{owner::{ScopeIdentity, SessionValidator, VoiceOwner}, socket::Server as VoiceServer};
use cockpit_protocol::orchestration::{HandoffRecipient, OrchestrationAction, OrchestrationMutationRequest};

fn voice_executable(path: PathBuf) -> PathBuf {
    if path.is_absolute() {
        return path;
    }
    if path.components().count() == 1 {
        if let Some(paths) = std::env::var_os("PATH") {
            for directory in std::env::split_paths(&paths).filter(|p| !p.as_os_str().is_empty()) {
                let directory = if directory.is_absolute() {
                    directory
                } else if let Ok(cwd) = std::env::current_dir() {
                    cwd.join(directory)
                } else {
                    continue;
                };
                let candidate = directory.join(&path);
                use std::os::unix::fs::PermissionsExt;
                if std::fs::metadata(&candidate)
                    .is_ok_and(|m| m.is_file() && m.permissions().mode() & 0o111 != 0)
                {
                    return candidate;
                }
            }
        }
    }
    // VoiceRpc requires an absolute executable; preserve the configured relative
    // path against the service's cwd rather than its isolated child directory.
    std::env::current_dir().map(|cwd| cwd.join(&path)).unwrap_or(path)
}

fn voice_boot_proven(boot: Option<&str>, peer: &cockpit_protocol::orchestration::NativeProcessIdentity) -> bool {
    let valid = |value: &str| !value.is_empty() && value.len() <= 4096 && !value.chars().any(char::is_control);
    boot.is_none_or(valid)
        && peer.kernel_boot_id.as_deref().is_none_or(valid)
        && (boot.is_some_and(valid) || peer.kernel_boot_id.as_deref().is_some_and(valid))
}

fn voice_validator(
    orchestration: Arc<cockpit_host::OrchestrationRuntime>,
    adapter: Arc<HerdrCliAdapter>,
) -> SessionValidator {
    Arc::new(move |session, root, binding, origin| {
        let orchestration = orchestration.clone();
        let adapter = adapter.clone();
        Box::pin(async move {
            let endpoint = adapter.project_endpoint_identity(&session).await.map_err(|e| e.to_string())?;
            let snapshot = adapter.project_snapshot_at_endpoint(&session, &endpoint).await.map_err(|e| e.to_string())?;
            if endpoint.is_empty() || snapshot.endpoint_identity != endpoint
                || snapshot.snapshot.session_id != session || snapshot.snapshot.server_instance.is_empty()
            {
                return Err("Herdr session or endpoint identity is unavailable or changed".into());
            }
            let scope = if let Some(binding) = binding {
                let request = OrchestrationMutationRequest {
                    session_id: session.clone(),
                    expected_revision: None,
                    action: OrchestrationAction::ThreadHandoff {
                        message_id: "voice-preflight".into(),
                        thread_id: "voice-preflight".into(),
                        to_run_id: root,
                        text: "voice preflight".into(),
                        recipient: HandoffRecipient::Owner,
                        images: vec![],
                        task_id: None,
                        expected_task_revision: None,
                        binding: Some(binding),
                    },
                };
                Some(orchestration.resolve_handoff_binding(origin, &request).await
                    .map_err(|e| e.to_string())?
                    .ok_or("canonical handoff binding is unavailable")?)
            } else {
                None
            };
            if scope.is_some() {
                let after = adapter.project_snapshot_at_endpoint(&session, &endpoint).await.map_err(|e| e.to_string())?;
                if after.endpoint_identity != endpoint || after.snapshot.session_id != session
                    || after.snapshot.server_instance != snapshot.snapshot.server_instance
                    || after.boot_id != snapshot.boot_id || after.peer_process != snapshot.peer_process
                {
                    return Err("Herdr session or endpoint identity is unavailable or changed".into());
                }
            }
            if adapter.project_endpoint_identity(&session).await.map_err(|e| e.to_string())? != endpoint {
                return Err("Herdr session or endpoint identity is unavailable or changed".into());
            }
            let boot_id = snapshot.boot_id;
            let peer_process = snapshot.peer_process.filter(|p| p.pid != 0 && p.start_ticks != 0)
                .ok_or("Herdr peer identity is unavailable")?;
            if !voice_boot_proven(boot_id.as_deref(), &peer_process) {
                return Err("Herdr boot identity is unavailable".into());
            }
            Ok(ScopeIdentity {
                session,
                server_instance: snapshot.snapshot.server_instance,
                endpoint_identity: endpoint,
                boot_id,
                peer_process,
                scope,
            })
        })
    })
}

#[derive(Debug, Args)]
pub struct ServiceArgs {
    #[command(flatten)]
    herdr: HerdrArgs,
    #[command(flatten)]
    project: ProjectArgs,
    #[command(subcommand)]
    command: ServiceCommand,
}

#[derive(Debug, Subcommand)]
enum ServiceCommand {
    /// Run the backend in the foreground.
    Run,
    /// Start the backend if needed.
    Start {
        #[arg(long)]
        json: bool,
    },
    /// Inspect the backend.
    Status {
        #[arg(long)]
        json: bool,
    },
    /// Stop the current backend.
    Stop {
        #[arg(long)]
        operator: bool,
        #[arg(long)]
        expected_instance: Option<uuid::Uuid>,
    },
    /// Restart the current backend.
    Restart {
        #[arg(long)]
        operator: bool,
        #[arg(long)]
        relaunch_app: bool,
        #[arg(long)]
        expected_instance: Option<uuid::Uuid>,
    },
    #[command(hide = true)]
    RestartHelper {
        #[arg(long)]
        expected_instance: Option<uuid::Uuid>,
        #[arg(long)]
        relaunch_app: bool,
    },
}

pub(super) fn endpoint(args: &HerdrArgs) -> Result<HerdrEndpoint, String> {
    let mut env: std::collections::BTreeMap<String, String> = std::env::vars().collect();
    for (key, value) in [
        (
            "COCKPIT_HERDR_EXECUTABLE",
            args.herdr
                .as_ref()
                .map(|v| v.to_string_lossy().into_owned()),
        ),
        ("COCKPIT_HERDR_SESSION", args.herdr_session.clone()),
        (
            "COCKPIT_HERDR_SOCKET",
            args.herdr_socket
                .as_ref()
                .map(|v| v.to_string_lossy().into_owned()),
        ),
    ] {
        if let Some(value) = value {
            env.insert(key.to_owned(), value);
        }
    }
    HerdrEndpoint::resolve(&env, std::env::var_os("PATH").as_deref()).map_err(|e| e.to_string())
}

pub(super) fn operator_guard(operator: bool) -> Result<(), String> {
    if !operator
        && (std::env::var_os("COCKPIT_JOB_CREDENTIAL_FILE").is_some()
            || std::env::var_os("COCKPIT_RUN_ID").is_some())
    {
        return Err("agent callers must pass --operator for this operation".into());
    }
    Ok(())
}

pub(super) fn cli_path() -> Result<PathBuf, String> {
    cockpit_host::orchestration_runtime::cli_path(
        cockpit_host::orchestration_runtime::CliProcessRole::Host,
    )
    .map_err(|e| e.to_string())
}

pub(super) async fn serve_gateway(args: ServeArgs) -> Result<(), String> {
    let web = web_cli::WebConfig::new(args.bind, args.static_dir, web_cli::config_file(&args.project)?)?;
    if args.test_mode {
        let service = make_service(args.herdr, CockpitMode::Test, None)?;
        return cockpit_host::server::serve(ServerConfig {
            bind: web.bind,
            static_dir: web.static_dir,
            service,
            browser_runtime: None,
            orchestration_runtime: None,
            backend_context: None,
        }).await.map_err(|e| e.to_string());
    }
    let projects = project_configuration(&args.project)?;
    let root = PathBuf::from(projects.state_root);
    let endpoint = endpoint(&args.herdr)?;
    web_cli::start(&root, &endpoint, &args.project, web).await
}

pub async fn run(args: ServiceArgs) -> Result<u8, String> {
    let projects = project_configuration(&args.project)?;
    let root = PathBuf::from(&projects.state_root);
    let endpoint = endpoint(&args.herdr)?;
    match args.command {
        ServiceCommand::Run => run_owner(args.herdr, args.project, root, endpoint).await,
        ServiceCommand::Start { json } => {
            let info = backend::ensure_with_config(
                &root,
                &cli_path()?,
                &endpoint,
                args.project.config.as_deref(),
            )
            .await
            .map_err(|e| e.to_string())?;
            print_json(
                &serde_json::json!({"state":"running", "instance":info}),
                json,
            )?;
            Ok(0)
        }
        ServiceCommand::Status { json } => {
            let status = match backend::probe(&root, &endpoint).await {
                BackendProbe::Running(info) => {
                    serde_json::json!({"state":"running", "instance":info})
                }
                BackendProbe::Starting => serde_json::json!({"state":"starting"}),
                BackendProbe::Stopped { last_exit } => {
                    serde_json::json!({"state":"stopped", "last_exit":last_exit})
                }
                BackendProbe::LegacyOwner => serde_json::json!({"state":"legacy_owner"}),
                BackendProbe::Unresponsive => serde_json::json!({"state":"unresponsive"}),
                BackendProbe::EndpointMismatch { running, expected } => {
                    serde_json::json!({"state":"endpoint_mismatch", "running":running, "expected":expected})
                }
            };
            print_json(&status, json)?;
            Ok(0)
        }
        ServiceCommand::Stop {
            operator,
            expected_instance,
        } => {
            operator_guard(operator)?;
            let id = match expected_instance {
                Some(id) => id,
                None => match backend::probe(&root, &endpoint).await {
                    BackendProbe::Running(info) => info.instance_id,
                    BackendProbe::Stopped { .. } => return Ok(0),
                    _ => return Err("backend is not safely stoppable".into()),
                },
            };
            backend::stop(&root, id).await.map_err(|e| e.to_string())?;
            Ok(0)
        }
        ServiceCommand::Restart {
            operator,
            relaunch_app,
            expected_instance,
        } => {
            operator_guard(operator)?;
            let ticket = backend::spawn_restart_helper_with_config(
                &root,
                expected_instance,
                relaunch_app,
                &endpoint,
                args.project.config.as_deref(),
            )
            .await
            .map_err(|e| e.to_string())?;
            print_json(&ticket, true)?;
            Ok(0)
        }
        ServiceCommand::RestartHelper {
            expected_instance,
            relaunch_app,
        } => {
            backend::run_restart_helper(&root, expected_instance, &endpoint, relaunch_app)
                .await
                .map_err(|e| e.to_string())?;
            Ok(0)
        }
    }
}

fn print_json(value: &impl serde::Serialize, pretty: bool) -> Result<(), String> {
    let text = if pretty {
        serde_json::to_string_pretty(value)
    } else {
        serde_json::to_string(value)
    }
    .map_err(|e| e.to_string())?;
    println!("{text}");
    Ok(())
}

async fn run_owner(
    herdr: HerdrArgs,
    project: ProjectArgs,
    root: PathBuf,
    endpoint: HerdrEndpoint,
) -> Result<u8, String> {
    let Some(lock) = BackendServer::try_lock(&root).map_err(|e| e.to_string())? else {
        return match backend::probe(&root, &endpoint).await {
            BackendProbe::Running(_) => Ok(0),
            _ => Err("backend lock is held but the matching service is unavailable".into()),
        };
    };
    let mut identity = backend::make_identity(&root, endpoint.clone()).map_err(|e| e.to_string())?;
    let mut browser = None;
    let result: Result<u8, String> = async {
        let configuration =
            ConfigurationFile::load(project.config.as_deref()).map_err(|e| e.to_string())?;
        let quota_config = configuration.quota.resolve().map_err(|e| e.to_string())?;
        let omp_executable = voice_executable(quota_config.omp_executable.clone());
        let projects = configuration
            .project
            .resolve(
                (!project.repository_roots.is_empty())
                    .then_some(project.repository_roots.as_slice()),
            )
            .map_err(|e| e.to_string())?;
        let runtime = browser_owner_runtime(
            herdr.clone(),
            configuration.browser.resolve().map_err(|e| e.to_string())?,
            root.clone(),
        )
        .await?;
        if !runtime.is_owner().await {
            backend::write_last_exit(&root, &identity, "legacy_owner_present", None)
                .map_err(|e| e.to_string())?;
            return Ok(3);
        }
        browser = Some(runtime.clone());
        let service = make_service(herdr.clone(), CockpitMode::Normal, Some(projects.clone()))?;
        let adapter = Arc::new(HerdrCliAdapter::new(
            HerdrCliConfig::from_options(herdr.herdr, herdr.herdr_session, herdr.herdr_socket)
                .map_err(|e| e.to_string())?,
        ));
        let orchestration = Arc::new(
            cockpit_host::OrchestrationRuntime::new(
                &projects,
                adapter.clone(),
                service.projects().map_err(|e| e.to_string())?.clone(),
                service.library().map_err(|e| e.to_string())?.clone(),
                project.config.clone(),
                cockpit_host::orchestration_runtime::CliProcessRole::Host,
            )
            .map_err(|e| e.to_string())?,
        );
        let voice = Arc::new(VoiceOwner::new(
            orchestration.clone(),
            omp_executable,
            std::env::var_os("PI_CODING_AGENT_DIR"),
            PathBuf::from(&projects.cache_root),
            voice_validator(orchestration.clone(), adapter),
        ));
        let service = service.with_quota(Arc::new(cockpit_core::quota::QuotaService::new(
            quota_config,
            Path::new(&projects.cache_root),
        ))).with_orchestration(orchestration.service.clone());
        let web = Arc::new(web_cli::Controller::new(
            service.clone(), runtime.clone(), orchestration.clone(), root.clone(), endpoint,
            project.config.clone(), web_cli::config_file(&project)?,
        ));
        let sync = service
            .library()
            .map_err(|e| e.to_string())?
            .start_sync(
                configuration
                    .library_sync
                    .resolve()
                    .map_err(|e| e.to_string())?,
            )
            .map_err(|e| e.to_string())?;
        identity.dispatcher = Some(true);
        identity.library_sync = Some(true);
        let server_lock = lock.try_clone().map_err(|e| e.to_string())?;
        orchestration.start_owner(true).await;
        let mut server = match BackendServer::bind(&root, server_lock, identity.clone()) {
            Ok(server) => server,
            Err(error) => {
                orchestration.shutdown().await;
                return Err(error.to_string());
            }
        };
        {
            let web = web.clone();
            server.set_web_handler(Arc::new(move |request| {
                let web = web.clone();
                Box::pin(async move { web.handle(request).await })
            }));
        }
        let voice_server = match VoiceServer::bind(&root, {
            let voice = voice.clone();
            move |request| {
                let voice = voice.clone();
                async move { voice.handle(request).await }
            }
        }) {
            Ok(server) => server,
            Err(error) => {
                orchestration.shutdown().await;
                return Err(error.to_string());
            }
        };
        let outcome = server.run().await.map_err(|e| e.to_string());
        let web_outcome = web.shutdown().await;
        voice_server.shutdown().await;
        voice.shutdown().await;
        orchestration.shutdown().await;
        drop(sync);
        runtime.shutdown().await.map_err(|e| e.to_string())?;
        browser = None;
        let outcome = outcome.and_then(|reason| web_outcome.map(|()| reason));
        let code = match outcome.as_ref() {
            Ok(StopReason::Explicit) => "explicit",
            Ok(StopReason::Signal) => "signal",
            Err(_) => "startup_failed",
        };
        backend::write_last_exit(
            &root,
            &identity,
            code,
            outcome.as_ref().err().map(String::as_str),
        )
        .map_err(|e| e.to_string())?;
        drop(server);
        outcome.map(|_| 0)
    }
    .await;
    if let Some(browser) = browser {
        let _ = browser.shutdown().await;
    }
    if let Err(error) = &result {
        let _ = backend::write_last_exit(&root, &identity, "startup_failed", Some(error));
    }
    drop(lock);
    result
}
