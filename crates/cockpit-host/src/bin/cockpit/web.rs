use super::*;
use cockpit_host::{
    backend::{self, BackendProbe},
    server::{self, ServerConfig},
};
use serde::{Deserialize, Serialize};
use tokio::sync::{Mutex, oneshot};

#[derive(Debug, Args)]
pub struct WebArgs {
    #[command(flatten)]
    herdr: HerdrArgs,
    #[command(flatten)]
    project: ProjectArgs,
    #[command(subcommand)]
    command: WebCommand,
}
#[derive(Debug, Subcommand)]
enum WebCommand {
    Start {
        #[arg(long)]
        bind: SocketAddr,
        #[arg(long)]
        static_dir: Option<PathBuf>,
    },
    Status {
        #[arg(long)]
        json: bool,
    },
    Stop {
        #[arg(long)]
        operator: bool,
    },
    Restart {
        #[arg(long)]
        operator: bool,
    },
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub(super) struct WebConfig {
    pub bind: SocketAddr,
    pub static_dir: Option<PathBuf>,
    pub config_file: Option<PathBuf>,
}
impl WebConfig {
    pub fn new(
        bind: SocketAddr,
        static_dir: Option<PathBuf>,
        config_file: Option<PathBuf>,
    ) -> Result<Self, String> {
        server::validate_bind(bind).map_err(|e| e.to_string())?;
        let static_dir = static_dir
            .map(|path| {
                if path.is_absolute() {
                    Ok(path)
                } else {
                    std::env::current_dir().map(|cwd| cwd.join(path))
                }
            })
            .transpose()
            .map_err(|e| e.to_string())?;
        let config_file = config_file
            .map(|path| std::fs::canonicalize(path).map_err(|e| e.to_string()))
            .transpose()?;
        Ok(Self {
            bind,
            static_dir,
            config_file,
        })
    }
}
#[derive(Clone, Serialize, Deserialize)]
struct WebInfo {
    instance_id: uuid::Uuid,
    config: WebConfig,
}
struct Running {
    info: WebInfo,
    requested: WebConfig,
    stop: Option<oneshot::Sender<()>>,
    task: tokio::task::JoinHandle<Result<(), server::ServerError>>,
}

pub(super) struct Controller {
    state: Mutex<(bool, Option<Running>)>,
    service: CockpitService,
    browser: Arc<BrowserRuntime>,
    orchestration: Arc<cockpit_host::OrchestrationRuntime>,
    root: PathBuf,
    endpoint: cockpit_host::lifecycle::HerdrEndpoint,
    config_path: Option<PathBuf>,
    config_file: Option<PathBuf>,
}
impl Controller {
    pub fn new(
        service: CockpitService,
        browser: Arc<BrowserRuntime>,
        orchestration: Arc<cockpit_host::OrchestrationRuntime>,
        root: PathBuf,
        endpoint: cockpit_host::lifecycle::HerdrEndpoint,
        config_path: Option<PathBuf>,
        config_file: Option<PathBuf>,
    ) -> Self {
        Self {
            state: Mutex::new((false, None)),
            service,
            browser,
            orchestration,
            root,
            endpoint,
            config_path,
            config_file,
        }
    }
    pub async fn handle(&self, request: serde_json::Value) -> serde_json::Value {
        match self.dispatch(request).await {
            Ok(info) => serde_json::json!({"ok":true, "info":info}),
            Err(message) => {
                serde_json::json!({"ok":false, "code":"web_unavailable", "message":message})
            }
        }
    }
    async fn dispatch(&self, request: serde_json::Value) -> Result<Option<WebInfo>, String> {
        let mut locked = self.state.lock().await;
        if request["op"] == "web_freeze" {
            if locked.0 {
                return Err("backend restart already in progress".into());
            }
            locked.0 = true;
            if locked
                .1
                .as_ref()
                .is_some_and(|running| running.task.is_finished())
            {
                locked.1 = None;
            }
            return Ok(locked.1.as_ref().map(|running| running.info.clone()));
        }
        if locked.0 && (request["op"] == "web_start" || request["op"] == "web_stop") {
            return Err("backend restart in progress".into());
        }
        let state = &mut locked.1;
        if state
            .as_ref()
            .is_some_and(|running| running.task.is_finished())
        {
            *state = None;
        }
        match request["op"].as_str() {
            Some("web_status") => Ok(state.as_ref().map(|running| running.info.clone())),
            Some("web_start") => {
                let config: WebConfig = serde_json::from_value(request["payload"].clone())
                    .map_err(|e| e.to_string())?;
                let config = WebConfig::new(config.bind, config.static_dir, config.config_file)?;
                if config.config_file != self.config_file {
                    return Err("backend configuration file differs".into());
                }
                if let Some(running) = state.as_ref() {
                    if running.stop.is_none() {
                        return Err("web listener shutdown outcome is pending".into());
                    }
                    if running.requested != config && running.info.config != config {
                        return Err(
                            "web listener already running with different configuration".into()
                        );
                    }
                    return Ok(Some(running.info.clone()));
                }
                let mut context = cockpit_host::transport::operations::backend::BackendContext::new(
                    self.root.clone(),
                    service_cli::cli_path()?,
                    self.endpoint.clone(),
                    cockpit_host::transport::Transport::Gateway,
                    None,
                );
                context.set_config_path(self.config_path.clone());
                let server_config = ServerConfig {
                    bind: config.bind,
                    static_dir: config.static_dir.clone(),
                    service: self.service.clone(),
                    browser_runtime: Some(self.browser.clone()),
                    orchestration_runtime: Some(self.orchestration.clone()),
                    backend_context: Some(context),
                };
                let (stop, stop_rx) = oneshot::channel();
                let (ready, ready_rx) = oneshot::channel();
                let mut task = tokio::spawn(server::serve_shared(
                    server_config,
                    async {
                        let _ = stop_rx.await;
                    },
                    ready,
                ));
                let address = tokio::select! {
                    result = &mut task => return Err(match result { Ok(Err(e)) => e.to_string(), Ok(Ok(())) => "web listener exited during startup".into(), Err(e) => e.to_string() }),
                    result = ready_rx => result.map_err(|e| e.to_string())?,
                };
                let info = WebInfo {
                    instance_id: uuid::Uuid::new_v4(),
                    config: WebConfig {
                        bind: address,
                        static_dir: config.static_dir.clone(),
                        config_file: config.config_file.clone(),
                    },
                };
                *state = Some(Running {
                    info: info.clone(),
                    requested: config,
                    stop: Some(stop),
                    task,
                });
                Ok(Some(info))
            }
            Some("web_stop") => {
                if let Some(running) = state.as_ref() {
                    if request["payload"]["web_instance"].as_str()
                        != Some(running.info.instance_id.to_string().as_str())
                    {
                        return Err("web listener instance changed".into());
                    }
                }
                if let Some(running) = state.as_mut() {
                    if let Some(stop) = running.stop.take() {
                        let _ = stop.send(());
                    }
                    match tokio::time::timeout(Duration::from_secs(10), &mut running.task).await {
                        Ok(result) => {
                            let result = result
                                .map_err(|e| e.to_string())?
                                .map_err(|e| e.to_string());
                            *state = None;
                            result?;
                        }
                        Err(_) => {
                            return Err(
                                "web listener shutdown outcome is unknown; retry status or stop"
                                    .into(),
                            );
                        }
                    }
                }
                Ok(None)
            }
            _ => Err("unknown web operation".into()),
        }
    }
    pub async fn shutdown(&self) -> Result<(), String> {
        let mut state = self.state.lock().await;
        state.0 = true;
        if let Some(running) = state.1.as_mut() {
            if let Some(stop) = running.stop.take() {
                let _ = stop.send(());
            }
            match tokio::time::timeout(Duration::from_secs(10), &mut running.task).await {
                Ok(result) => result
                    .map_err(|e| e.to_string())?
                    .map_err(|e| e.to_string())?,
                Err(_) => return Err("web listener shutdown outcome is unknown".into()),
            }
            state.1 = None;
        }
        Ok(())
    }
}

fn check_image(owner: &cockpit_host::lifecycle::LifecycleInfo) -> Result<(), String> {
    let (_, image) = cockpit_host::installation::startup_image().map_err(|e| e.to_string())?;
    if owner.image != image {
        return Err("backend executable image differs from this CLI".into());
    }
    Ok(())
}
pub(super) fn config_file(project: &ProjectArgs) -> Result<Option<PathBuf>, String> {
    cockpit_core::config::configuration_path(project.config.as_deref())
        .map_err(|e| e.to_string())?
        .map(|path| std::fs::canonicalize(path).map_err(|e| e.to_string()))
        .transpose()
}

async fn status(
    root: &Path,
    endpoint: &cockpit_host::lifecycle::HerdrEndpoint,
) -> Result<Option<(uuid::Uuid, WebInfo)>, String> {
    match backend::probe(root, endpoint).await {
        BackendProbe::Stopped { .. } => Ok(None),
        BackendProbe::Running(owner) => {
            check_image(&owner)?;
            let reply = backend::web_call(
                root,
                owner.instance_id,
                "web_status",
                serde_json::Value::Null,
            )
            .await
            .map_err(|e| e.to_string())?;
            let info: Option<WebInfo> =
                serde_json::from_value(reply["info"].clone()).map_err(|e| e.to_string())?;
            Ok(info.map(|info| (owner.instance_id, info)))
        }
        other => Err(format!("backend is not available: {other:?}")),
    }
}
pub(super) async fn start(
    root: &Path,
    endpoint: &cockpit_host::lifecycle::HerdrEndpoint,
    project: &ProjectArgs,
    config: WebConfig,
) -> Result<(), String> {
    let owner = backend::ensure_with_config(
        root,
        &service_cli::cli_path()?,
        endpoint,
        project.config.as_deref(),
    )
    .await
    .map_err(|e| e.to_string())?;
    check_image(&owner)?;
    let response = backend::web_call(
        root,
        owner.instance_id,
        "web_start",
        serde_json::to_value(config).map_err(|e| e.to_string())?,
    )
    .await
    .map_err(|e| e.to_string())?;
    println!("{}", response["info"]);
    Ok(())
}
async fn stop(root: &Path, owner: uuid::Uuid, info: &WebInfo) -> Result<(), String> {
    backend::web_call(
        root,
        owner,
        "web_stop",
        serde_json::json!({"web_instance":info.instance_id}),
    )
    .await
    .map_err(|e| e.to_string())?;
    Ok(())
}
pub async fn run(args: WebArgs) -> Result<(), String> {
    let root = PathBuf::from(project_configuration(&args.project)?.state_root);
    let endpoint = service_cli::endpoint(&args.herdr)?;
    match args.command {
        WebCommand::Start { bind, static_dir } => {
            start(
                &root,
                &endpoint,
                &args.project,
                WebConfig::new(bind, static_dir, config_file(&args.project)?)?,
            )
            .await?
        }
        WebCommand::Status { json } => {
            let value = serde_json::json!({"managed":status(&root, &endpoint).await?.map(|(_, info)| info)});
            println!(
                "{}",
                if json {
                    serde_json::to_string_pretty(&value)
                } else {
                    serde_json::to_string(&value)
                }
                .map_err(|e| e.to_string())?
            );
        }
        WebCommand::Stop { operator } => {
            service_cli::operator_guard(operator)?;
            if let Some((owner, info)) = status(&root, &endpoint).await? {
                stop(&root, owner, &info).await?;
            }
        }
        WebCommand::Restart { operator } => {
            service_cli::operator_guard(operator)?;
            let (owner, info) = status(&root, &endpoint)
                .await?
                .ok_or("no web listener is running")?;
            stop(&root, owner, &info).await?;
            let response = backend::web_call(
                &root,
                owner,
                "web_start",
                serde_json::to_value(info.config).map_err(|e| e.to_string())?,
            )
            .await
            .map_err(|e| e.to_string())?;
            println!("{}", response["info"]);
        }
    }
    Ok(())
}
