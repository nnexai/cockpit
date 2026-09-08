use std::{net::SocketAddr, path::PathBuf, sync::Arc};

use clap::{Args, Parser, Subcommand};
use cockpit_core::{
    CockpitService,
    browser::BrowserService,
    config::{load_browser_configuration, load_project_configuration},
    projects::ProjectService,
};
use cockpit_herdr::{HerdrCliAdapter, HerdrCliConfig};
use cockpit_protocol::browser::{
    BrowserAction, BrowserFeedbackAckRequest, BrowserFeedbackRequest, BrowserRequest, BrowserTarget,
};
use cockpit_protocol::projects::ProjectConfiguration;
use cockpit_protocol::v1::CockpitMode;

use cockpit_host::{
    BrowserRuntime,
    server::{ServerConfig, serve},
};

#[derive(Debug, Parser)]
#[command(
    name = "cockpit",
    version,
    about = "Local Cockpit gateway and status client"
)]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Debug, Subcommand)]
enum Command {
    /// Inspect the configured Herdr installation.
    Status(StatusArgs),
    /// Serve the browser client and HTTP API in the foreground.
    Serve(ServeArgs),
    /// Inspect effective non-secret project configuration.
    Configuration(ProjectArgs),
    /// Control the browser associated with a Herdr Space.
    Browser(BrowserArgs),
}
#[derive(Debug, Clone, Args)]
struct HerdrArgs {
    /// Herdr executable to invoke.
    #[arg(long, env = "COCKPIT_HERDR_EXECUTABLE")]
    herdr: Option<PathBuf>,
    /// Named Herdr session to inspect. Required with --herdr-socket.
    #[arg(long, env = "COCKPIT_HERDR_SESSION")]
    herdr_session: Option<String>,
    /// Herdr socket path override. Requires an explicit logical session identity.
    #[arg(long, env = "COCKPIT_HERDR_SOCKET")]
    herdr_socket: Option<PathBuf>,
}

#[derive(Debug, Args)]
struct StatusArgs {
    #[command(flatten)]
    herdr: HerdrArgs,
    /// Emit the response as JSON.
    #[arg(long)]
    json: bool,
}

#[derive(Debug, Args)]
struct BrowserArgs {
    #[command(flatten)]
    herdr: HerdrArgs,
    #[arg(value_enum)]
    action: BrowserActionName,
    #[arg(value_enum)]
    feedback_action: Option<BrowserFeedbackActionName>,
    #[arg(long, conflicts_with = "space")]
    current: bool,
    #[arg(long)]
    space: Option<String>,
    #[arg(long)]
    url: Option<String>,
    #[arg(long)]
    config: Option<PathBuf>,
    #[arg(long = "id")]
    ids: Vec<String>,
}

#[derive(Debug, Clone, clap::ValueEnum)]
enum BrowserActionName {
    Open,
    Status,
    Show,
    Close,
    Feedback,
}

#[derive(Debug, Clone, clap::ValueEnum)]
enum BrowserFeedbackActionName {
    Ack,
}
#[derive(Debug, Args)]
struct ProjectArgs {
    /// Versioned Cockpit project configuration file.
    #[arg(long)]
    config: Option<PathBuf>,
    /// Catalog root for existing local repositories; may be repeated.
    #[arg(long = "repository-root")]
    repository_roots: Vec<PathBuf>,
}

fn project_configuration(args: &ProjectArgs) -> Result<ProjectConfiguration, String> {
    load_project_configuration(
        args.config.as_deref(),
        (!args.repository_roots.is_empty()).then_some(args.repository_roots.as_slice()),
    )
    .map_err(|error| error.to_string())
}

#[derive(Debug, Args)]
struct ServeArgs {
    #[command(flatten)]
    herdr: HerdrArgs,
    #[command(flatten)]
    project: ProjectArgs,
    /// Loopback address on which to listen.
    #[arg(long, default_value = "127.0.0.1:0")]
    bind: SocketAddr,
    /// Root directory containing the browser build.
    #[arg(long, default_value = "dist")]
    static_dir: PathBuf,
    /// Disable live Herdr inspection and return deterministic unavailable status.
    #[arg(long)]
    test_mode: bool,
}

fn make_service(
    herdr: HerdrArgs,
    mode: CockpitMode,
    projects: Option<ProjectConfiguration>,
) -> Result<CockpitService, String> {
    let config = HerdrCliConfig::from_options(herdr.herdr, herdr.herdr_session, herdr.herdr_socket)
        .map_err(|error| error.to_string())?;
    let adapter = Arc::new(HerdrCliAdapter::new(config));
    let service = CockpitService::new(mode, adapter.clone());
    match projects {
        Some(config) => {
            let projects = ProjectService::new(config.clone(), adapter.clone())
                .map_err(|error| error.to_string())?;
            let service = service.with_projects(projects);
            let contexts = cockpit_core::context::ContextService::new(
                config.clone(),
                adapter.extension_adapter(),
                service
                    .projects()
                    .map_err(|error| error.to_string())?
                    .clone(),
            );
            let sources = cockpit_core::sources::SourceService::new(
                &config,
                cockpit_providers::configured_providers(&config)
                    .map_err(|error| error.to_string())?,
            )
            .map_err(|error| error.to_string())?;
            let contexts = contexts.with_sources(Arc::new(sources));
            let service = service.with_contexts(contexts);
            let reviews = cockpit_core::review::ReviewService::new(
                config.clone(),
                adapter.extension_adapter(),
                service
                    .contexts()
                    .map_err(|error| error.to_string())?
                    .clone(),
            )
            .map_err(|error| error.to_string())?;
            let service = service.with_reviews(reviews);
            let comments = cockpit_core::comments::CommentsService::new(
                config,
                service
                    .contexts()
                    .map_err(|error| error.to_string())?
                    .clone(),
            )
            .map_err(|error| error.to_string())?;
            let comments = comments
                .with_paste_adapter(adapter.paste_adapter())
                .with_reviews(
                    service
                        .reviews()
                        .map_err(|error| error.to_string())?
                        .clone(),
                );
            Ok(service.with_comments(comments))
        }
        None => Ok(service),
    }
}

async fn browser_owner_runtime(
    herdr: HerdrArgs,
    config_path: Option<&std::path::Path>,
    state_root: PathBuf,
) -> Result<Arc<BrowserRuntime>, String> {
    let config = load_browser_configuration(config_path).map_err(|error| error.to_string())?;
    let herdr_config =
        HerdrCliConfig::from_options(herdr.herdr, herdr.herdr_session, herdr.herdr_socket)
            .map_err(|error| error.to_string())?;
    let adapter = Arc::new(HerdrCliAdapter::new(herdr_config));
    let paste_adapter = adapter.paste_adapter();
    let service = Arc::new(
        BrowserService::new(config, state_root.clone(), adapter.clone())
            .map_err(|error| error.to_string())?
            .with_paste_adapter(paste_adapter),
    );
    let runtime = BrowserRuntime::start(state_root, service)
        .await
        .map_err(|error| error.to_string())?;
    Ok(Arc::new(runtime))
}

async fn run(cli: Cli) -> Result<(), String> {
    match cli.command {
        Command::Status(args) => {
            let service = make_service(args.herdr, CockpitMode::Normal, None)?;
            let output = if args.json {
                serde_json::to_string_pretty(&service.status().await)
            } else {
                serde_json::to_string(&service.status().await)
            }
            .map_err(|error| error.to_string())?;
            println!("{output}");
            Ok(())
        }
        Command::Serve(args) => {
            let mode = if args.test_mode {
                CockpitMode::Test
            } else {
                CockpitMode::Normal
            };
            let projects = if args.test_mode {
                None
            } else {
                Some(project_configuration(&args.project)?)
            };
            let browser_runtime = if let Some(projects_config) = projects.as_ref() {
                Some(
                    browser_owner_runtime(
                        args.herdr.clone(),
                        args.project.config.as_deref(),
                        PathBuf::from(&projects_config.state_root),
                    )
                    .await?,
                )
            } else {
                None
            };
            let service = make_service(args.herdr, mode, projects)?;
            serve(ServerConfig {
                bind: args.bind,
                static_dir: args.static_dir,
                service,
                browser_runtime,
            })
            .await
            .map_err(|error| error.to_string())
        }
        Command::Configuration(args) => {
            let configuration = project_configuration(&args)?;
            println!(
                "{}",
                serde_json::to_string_pretty(&configuration).map_err(|error| error.to_string())?
            );
            Ok(())
        }
        Command::Browser(args) => run_browser(args).await,
    }
}
fn inherited_session_from_socket(path: &std::path::Path) -> Option<String> {
    let stem = path.file_stem()?.to_str()?;
    if stem != "herdr" {
        return Some(stem.trim_start_matches("herdr-").to_owned());
    }
    path.parent()?.file_name()?.to_str().map(str::to_owned)
}

async fn run_browser(args: BrowserArgs) -> Result<(), String> {
    let herdr_executable = args
        .herdr
        .herdr
        .clone()
        .unwrap_or_else(|| PathBuf::from("herdr"));
    let inherited_socket = std::env::var_os("HERDR_SOCKET_PATH").map(PathBuf::from);
    let inherited_session = std::env::var("HERDR_SESSION_NAME")
        .ok()
        .or_else(|| std::env::var("HERDR_SESSION").ok())
        .or_else(|| {
            inherited_socket
                .as_ref()
                .and_then(|path| inherited_session_from_socket(path))
        });
    let target = if args.current {
        if std::env::var("HERDR_ENV").ok().as_deref() != Some("1") {
            return Err(
                "browser --current requires HERDR_ENV=1 in the inherited Herdr caller environment"
                    .to_owned(),
            );
        }
        let output = tokio::process::Command::new(&herdr_executable)
            .args(["pane", "current", "--current"])
            .output()
            .await
            .map_err(|error| format!("cannot resolve current Herdr pane: {error}"))?;
        if !output.status.success() {
            return Err("Herdr pane current lookup failed".to_owned());
        }
        let value: serde_json::Value = serde_json::from_slice(&output.stdout)
            .map_err(|error| format!("invalid Herdr current-pane response: {error}"))?;
        let pane = value
            .get("result")
            .and_then(|result| result.get("pane"))
            .ok_or_else(|| "Herdr current-pane response has no pane evidence".to_owned())?;
        let session_id = inherited_session.clone().ok_or_else(|| "current Herdr pane has no resolvable session identity; use explicit --herdr-session --herdr-socket --space".to_owned())?;
        let pane_id = pane
            .get("pane_id")
            .and_then(|v| v.as_str())
            .ok_or_else(|| "Herdr current-pane response has no pane_id".to_owned())?;
        BrowserTarget {
            session_id,
            space_id: None,
            pane_id: Some(pane_id.to_owned()),
            endpoint_path: inherited_socket
                .as_ref()
                .and_then(|path| path.to_str())
                .map(str::to_owned),
        }
    } else {
        let session_id = args.herdr.herdr_session.clone().ok_or_else(|| {
            "explicit browser targets require --herdr-session, --herdr-socket, and --space"
                .to_owned()
        })?;
        let _socket = args
            .herdr
            .herdr_socket
            .clone()
            .or(inherited_socket)
            .ok_or_else(|| {
                "explicit browser targets require --herdr-session, --herdr-socket, and --space"
                    .to_owned()
            })?;
        let space_id = args.space.clone().ok_or_else(|| {
            "explicit browser targets require --herdr-session, --herdr-socket, and --space"
                .to_owned()
        })?;
        BrowserTarget {
            session_id,
            space_id: Some(space_id),
            pane_id: None,
            endpoint_path: None,
        }
    };
    let herdr_config = HerdrCliConfig::from_options(
        Some(herdr_executable),
        args.herdr.herdr_session.clone().or(inherited_session),
        args.herdr
            .herdr_socket
            .clone()
            .or_else(|| std::env::var_os("HERDR_SOCKET_PATH").map(PathBuf::from)),
    )
    .map_err(|error| error.to_string())?;
    let adapter = Arc::new(HerdrCliAdapter::new(herdr_config));
    let paste_adapter = adapter.paste_adapter();
    let browser_config =
        load_browser_configuration(args.config.as_deref()).map_err(|error| error.to_string())?;
    let project_config = load_project_configuration(args.config.as_deref(), None)
        .map_err(|error| error.to_string())?;
    let state_root = PathBuf::from(project_config.state_root);
    let service = Arc::new(
        BrowserService::new(browser_config, state_root.clone(), adapter.clone())
            .map_err(|error| error.to_string())?
            .with_paste_adapter(paste_adapter),
    );
    let runtime = BrowserRuntime::connect(state_root, service)
        .await
        .map_err(|error| error.to_string())?;
    match args.action {
        BrowserActionName::Feedback => {
            let response = match args.feedback_action {
                None => {
                    if !args.ids.is_empty() {
                        return Err(
                            "browser feedback fetch does not accept --id; use `feedback ack`"
                                .to_owned(),
                        );
                    }
                    serde_json::to_string_pretty(
                        &runtime
                            .feedback(BrowserFeedbackRequest { target })
                            .await
                            .map_err(|error| error.to_string())?,
                    )
                    .map_err(|error| error.to_string())?
                }
                Some(BrowserFeedbackActionName::Ack) => {
                    if args.ids.is_empty() {
                        return Err("browser feedback ack requires at least one --id".to_owned());
                    }
                    serde_json::to_string_pretty(
                        &runtime
                            .acknowledge_feedback(BrowserFeedbackAckRequest {
                                target,
                                ids: args.ids,
                            })
                            .await
                            .map_err(|error| error.to_string())?,
                    )
                    .map_err(|error| error.to_string())?
                }
            };
            println!("{response}");
        }
        action => {
            if args.feedback_action.is_some() || !args.ids.is_empty() {
                return Err("feedback options are only valid for `browser feedback`".to_owned());
            }
            let action = match action {
                BrowserActionName::Open => BrowserAction::Open { url: args.url },
                BrowserActionName::Status => BrowserAction::Status,
                BrowserActionName::Show => BrowserAction::Show,
                BrowserActionName::Close => BrowserAction::Close,
                BrowserActionName::Feedback => unreachable!(),
            };
            let response = runtime
                .execute(BrowserRequest { target, action })
                .await
                .map_err(|error| error.to_string())?;
            println!(
                "{}",
                serde_json::to_string_pretty(&response).map_err(|error| error.to_string())?
            );
        }
    }
    runtime
        .shutdown()
        .await
        .map_err(|error| error.to_string())?;
    Ok(())
}

#[tokio::main]
async fn main() {
    if let Err(error) = run(Cli::parse()).await {
        eprintln!("cockpit: {error}");
        std::process::exit(1);
    }
}
