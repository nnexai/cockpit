use super::*;
use cockpit_host::{
    backend::{self, BackendProbe, BackendServer, StopReason},
    lifecycle::HerdrEndpoint,
};

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
    if args.test_mode {
        let service = make_service(args.herdr, CockpitMode::Test, None)?;
        return web_cli::serve(
            ServerConfig {
                bind: args.bind,
                static_dir: args.static_dir,
                service,
                browser_runtime: None,
                orchestration_runtime: None,
                backend_context: None,
            },
            args.managed,
            None,
        )
        .await;
    }
    let configuration =
        ConfigurationFile::load(args.project.config.as_deref()).map_err(|e| e.to_string())?;
    let projects = configuration
        .project
        .resolve(
            (!args.project.repository_roots.is_empty())
                .then_some(args.project.repository_roots.as_slice()),
        )
        .map_err(|e| e.to_string())?;
    let state_root = PathBuf::from(&projects.state_root);
    let endpoint = endpoint(&args.herdr)?;
    let available = match backend::ensure_with_config(
        &state_root,
        &cli_path()?,
        &endpoint,
        args.project.config.as_deref(),
    )
    .await
    {
        Ok(_) => true,
        Err(error) => {
            eprintln!("background service unavailable: {error}");
            error.code != "backend_endpoint_mismatch"
        }
    };
    let adapter = Arc::new(HerdrCliAdapter::new(
        HerdrCliConfig::from_options(
            args.herdr.herdr.clone(),
            args.herdr.herdr_session.clone(),
            args.herdr.herdr_socket.clone(),
        )
        .map_err(|e| e.to_string())?,
    ));
    let browser = Arc::new(
        BrowserService::new(
            configuration.browser.resolve().map_err(|e| e.to_string())?,
            state_root.clone(),
            adapter.clone(),
        )
        .map_err(|e| e.to_string())?
        .with_paste_adapter(adapter.paste_adapter()),
    );
    let browser_runtime =
        available.then(|| Arc::new(BrowserRuntime::observer(state_root.clone(), browser)));
    let service = make_service(
        args.herdr.clone(),
        CockpitMode::Normal,
        Some(projects.clone()),
    )?;
    let quota = Arc::new(cockpit_core::quota::QuotaService::new(
        configuration.quota.resolve().map_err(|e| e.to_string())?,
        Path::new(&projects.cache_root),
    ));
    let service = service.with_quota(quota);
    let orchestration = Arc::new(
        cockpit_host::OrchestrationRuntime::new(
            &projects,
            adapter,
            service.projects().map_err(|e| e.to_string())?.clone(),
            service.library().map_err(|e| e.to_string())?.clone(),
            args.project.config.clone(),
            cockpit_host::orchestration_runtime::CliProcessRole::Host,
        )
        .map_err(|e| e.to_string())?,
    );
    let service = service.with_orchestration(orchestration.service.clone());
    let mut context = cockpit_host::transport::operations::backend::BackendContext::new(
        state_root.clone(),
        cli_path()?,
        endpoint.clone(),
        cockpit_host::transport::Transport::Gateway,
        None,
    );
    context.set_config_path(args.project.config.clone());
    web_cli::serve(
        ServerConfig {
            bind: args.bind,
            static_dir: args.static_dir.clone(),
            service,
            browser_runtime,
            orchestration_runtime: Some(orchestration),
            backend_context: Some(context),
        },
        args.managed,
        Some(web_cli::ManagedConfig {
            root: state_root,
            bind: args.bind,
            static_dir: args.static_dir,
            config: args.project.config,
            herdr: endpoint,
        }),
    )
    .await
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
    let mut identity = backend::make_identity(&root, endpoint).map_err(|e| e.to_string())?;
    let mut browser = None;
    let result: Result<u8, String> = async {
        let configuration =
            ConfigurationFile::load(project.config.as_deref()).map_err(|e| e.to_string())?;
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
                adapter,
                service.projects().map_err(|e| e.to_string())?.clone(),
                service.library().map_err(|e| e.to_string())?.clone(),
                project.config,
                cockpit_host::orchestration_runtime::CliProcessRole::Host,
            )
            .map_err(|e| e.to_string())?,
        );
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
        let outcome = server.run().await.map_err(|e| e.to_string());
        orchestration.shutdown().await;
        drop(sync);
        runtime.shutdown().await.map_err(|e| e.to_string())?;
        browser = None;
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
