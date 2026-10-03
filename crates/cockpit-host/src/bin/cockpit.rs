use std::{net::SocketAddr, path::PathBuf, sync::Arc};

use clap::{Args, Parser, Subcommand};
use cockpit_core::{
    CockpitService,
    browser::BrowserService,
    config::{load_browser_configuration, load_project_configuration},
    library::LibraryService,
    projects::ProjectService,
};
use cockpit_herdr::{HerdrCliAdapter, HerdrCliConfig};
use cockpit_protocol::browser::{
    BrowserAction, BrowserFeedbackAckRequest, BrowserFeedbackRequest, BrowserRequest, BrowserTarget,
    BrowserWorkScope,
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
    /// Control the browser associated with a Herdr tab.
    Browser(BrowserArgs),
    /// Print the live Library context selected by a Herdr Space.
    Context(ContextArgs),
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
    #[arg(long, conflicts_with = "tab")]
    current: bool,
    #[arg(long)]
    tab: Option<String>,
    #[arg(long)]
    url: Option<String>,
    #[arg(long)]
    config: Option<PathBuf>,
    #[arg(long = "id")]
    ids: Vec<String>,
}

#[derive(Debug, Args)]
struct ContextArgs {
    #[command(flatten)]
    herdr: HerdrArgs,
    #[command(flatten)]
    project: ProjectArgs,
    /// Resolve the originating Herdr pane, never the process cwd.
    #[arg(long, conflicts_with = "space", required_unless_present = "space")]
    current: bool,
    /// Explicit Space identity; requires --herdr-session and --herdr-socket.
    #[arg(long, conflicts_with = "current")]
    space: Option<String>,
}

#[derive(Debug, Clone, clap::ValueEnum)]
enum BrowserActionName {
    Open,
    Status,
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
            let credentials = Arc::new(cockpit_core::credentials::ProviderCredentials::new(
                &config,
                cockpit_secrets::os_vault(),
                cockpit_providers::credential_kinds,
            ));
            let sources = Arc::new(
                cockpit_core::sources::SourceService::new(
                    &config,
                    cockpit_providers::configured_providers(&config, credentials.clone())
                        .map_err(|error| error.to_string())?,
                )
                .map_err(|error| error.to_string())?,
            );
            let projects = ProjectService::new(config.clone(), adapter.clone())
                .map_err(|error| error.to_string())?
                .with_sources(sources.clone());
            let service = service.with_projects(projects).with_credentials(credentials);
            service.projects().map_err(|error| error.to_string())?.prewarm_repositories();
            let library = Arc::new(LibraryService::new(config.clone(), sources.clone())
                .with_herdr(adapter.clone()));
            let contexts = cockpit_core::context::ContextService::new(
                config.clone(),
                adapter.source_adapter(),
                service
                    .projects()
                    .map_err(|error| error.to_string())?
                    .clone(),
            ).with_library(library.clone());
            let viewers = Arc::new(cockpit_core::viewer::ViewerService::new(Arc::new(contexts.clone())));
            let contexts = contexts.with_viewers(viewers.clone());
            let service = service.with_contexts(contexts).with_viewers(viewers);
            let reviews = cockpit_core::review::ReviewService::new(
                config.clone(),
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
            Ok(service.with_comments(comments).with_library((*library).clone()))
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
            let quota = projects
                .as_ref()
                .map(|configuration| {
                    cockpit_core::config::load_quota_configuration(args.project.config.as_deref())
                        .map(|quota_configuration| {
                            Arc::new(cockpit_core::quota::QuotaService::new(
                                quota_configuration,
                                std::path::Path::new(&configuration.cache_root),
                            ))
                        })
                        .map_err(|error| error.to_string())
                })
                .transpose()?;
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
            let service = match quota {
                Some(quota) => service.with_quota(quota),
                None => service,
            };
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
        Command::Context(args) => run_context(args).await,
    }
}
fn inherited_session_from_socket(path: &std::path::Path) -> Option<String> {
    let stem = path.file_stem()?.to_str()?;
    if stem != "herdr" {
        return Some(stem.trim_start_matches("herdr-").to_owned());
    }
    path.parent()?.file_name()?.to_str().map(str::to_owned)
}

async fn current_pane_id(
    executable: &std::path::Path,
    socket: Option<&std::path::Path>,
    session: Option<&str>,
    caller: &str,
) -> Result<String, String> {
    if std::env::var("HERDR_ENV").ok().as_deref() != Some("1") {
        return Err(format!("{caller} --current requires HERDR_ENV=1 in the inherited Herdr caller environment"));
    }
    let mut command = tokio::process::Command::new(executable);
    command.env_remove("HERDR_SESSION").env_remove("HERDR_SOCKET_PATH");
    if let Some(socket) = socket {
        command.env("HERDR_SOCKET_PATH", socket);
    } else if let Some(session) = session {
        command.arg("--session").arg(session);
    }
    let output = command.args(["pane", "current", "--current"]).output().await
        .map_err(|error| format!("cannot resolve current Herdr pane: {error}"))?;
    if !output.status.success() {
        return Err("Herdr pane current lookup failed".to_owned());
    }
    let value: serde_json::Value = serde_json::from_slice(&output.stdout)
        .map_err(|error| format!("invalid Herdr current-pane response: {error}"))?;
    value.get("result").and_then(|result| result.get("pane"))
        .and_then(|pane| pane.get("pane_id")).and_then(|id| id.as_str())
        .filter(|id| !id.is_empty())
        .map(str::to_owned)
        .ok_or_else(|| "Herdr current-pane response has no pane_id".to_owned())
}

async fn run_context(args: ContextArgs) -> Result<(), String> {
    let executable = args.herdr.herdr.clone().unwrap_or_else(|| PathBuf::from("herdr"));
    let inherited_socket = std::env::var_os("HERDR_SOCKET_PATH").map(PathBuf::from);
    let inherited_session = std::env::var("HERDR_SESSION_NAME").ok()
        .or_else(|| std::env::var("HERDR_SESSION").ok())
        .or_else(|| inherited_socket.as_deref().and_then(inherited_session_from_socket));
    let socket = args.herdr.herdr_socket.clone().or(inherited_socket);
    let session = args.herdr.herdr_session.clone().or(inherited_session)
        .ok_or_else(|| "context requires a resolvable Herdr session identity".to_owned())?;
    if !args.current && (args.herdr.herdr_session.is_none() || args.herdr.herdr_socket.is_none()) {
        return Err("explicit context targets require --herdr-session, --herdr-socket, and --space".to_owned());
    }
    let pane_id = if args.current {
        Some(current_pane_id(&executable, socket.as_deref(), Some(&session), "context").await?)
    } else {
        None
    };
    let config = HerdrCliConfig::from_options(Some(executable), Some(session.clone()), socket)
        .map_err(|error| error.to_string())?;
    let adapter = Arc::new(HerdrCliAdapter::new(config));
    let source_adapter = adapter.source_adapter();
    let evidence = match pane_id.as_deref() {
        Some(pane_id) => Some(source_adapter.source_pane_evidence(&session, pane_id).await
            .map_err(|error| error.to_string())?),
        None => None,
    };
    let space_id = evidence.as_ref().map(|source| source.workspace_id.clone())
        .or(args.space).ok_or_else(|| "context has no current Space evidence".to_owned())?;
    let project_config = project_configuration(&args.project)?;
    let credentials = Arc::new(cockpit_core::credentials::ProviderCredentials::new(
        &project_config, cockpit_secrets::os_vault(), cockpit_providers::credential_kinds,
    ));
    let sources = Arc::new(cockpit_core::sources::SourceService::new(
        &project_config,
        cockpit_providers::configured_providers(&project_config, credentials)
            .map_err(|error| error.to_string())?,
    ).map_err(|error| error.to_string())?);
    let library = LibraryService::new(project_config, sources).with_herdr(adapter);
    let target = cockpit_protocol::library::SpaceTarget { session_id: session, space_id };
    let listing = library.space_listing(&target).await.map_err(|error| error.to_string())?;
    if let (Some(pane_id), Some(before)) = (pane_id.as_deref(), evidence.as_ref()) {
        let after = source_adapter.source_pane_evidence(&target.session_id, pane_id).await
            .map_err(|error| error.to_string())?;
        if before.pane_id != pane_id || after.pane_id != pane_id
            || before.workspace_id != target.space_id || after.workspace_id != target.space_id
            || before.endpoint_identity != after.endpoint_identity || before.tab_id != after.tab_id
        {
            return Err("current context pane identity changed during lookup".to_owned());
        }
    }
    let root = std::path::Path::new(&listing.library_root);
    let items = listing.items.iter().map(|item| serde_json::json!({
        "item_id": item.item_id,
        "title": item.title,
        "kind": item.kind,
        "path": root.join(item.document_path.as_deref().unwrap_or(&item.item_path)),
    })).collect::<Vec<_>>();
    println!("{}", serde_json::to_string_pretty(&serde_json::json!({
        "target": listing.target, "space_label": listing.space_label, "pane_id": pane_id,
        "library_root": listing.library_root, "items": items,
        "checkout_path": listing.checkout_path, "repository_paths": listing.repository_paths,
        "diagnostics": listing.diagnostics,
    })).map_err(|error| error.to_string())?);
    Ok(())
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
    let effective_socket = args.herdr.herdr_socket.clone().or(inherited_socket);
    let effective_session = args.herdr.herdr_session.clone().or(inherited_session);
    let target = if args.current {
        let pane_id = current_pane_id(&herdr_executable, effective_socket.as_deref(), effective_session.as_deref(), "browser").await?;
        let session_id = effective_session.clone().ok_or_else(|| "current Herdr pane has no resolvable session identity; use explicit --herdr-session --herdr-socket --tab".to_owned())?;
        BrowserTarget {
            session_id,
            tab_id: None,
            pane_id: Some(pane_id.to_owned()),
            endpoint_path: effective_socket
                .as_ref()
                .and_then(|path| path.to_str())
                .map(str::to_owned),
        }
    } else {
        let session_id = args.herdr.herdr_session.clone().ok_or_else(|| {
            "explicit browser targets require --herdr-session, --herdr-socket, and --tab"
                .to_owned()
        })?;
        let _socket = effective_socket.clone().ok_or_else(|| {
            "explicit browser targets require --herdr-session, --herdr-socket, and --tab"
                .to_owned()
        })?;
        let tab_id = args.tab.clone().ok_or_else(|| {
            "explicit browser targets require --herdr-session, --herdr-socket, and --tab"
                .to_owned()
        })?;
        BrowserTarget {
            session_id,
            tab_id: Some(tab_id),
            pane_id: None,
            endpoint_path: None,
        }
    };
    let herdr_config =
        HerdrCliConfig::from_options(Some(herdr_executable), effective_session, effective_socket)
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
            let scope = BrowserWorkScope::Tab { target };
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
                            .feedback(BrowserFeedbackRequest { scope })
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
                                scope,
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
                BrowserActionName::Close => BrowserAction::Close,
                BrowserActionName::Feedback => unreachable!(),
            };
            let response = runtime
                .execute(BrowserRequest {
                    target,
                    action,
                })
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn browser_targets_are_mutually_exclusive() {
        assert!(Cli::try_parse_from([
            "cockpit", "browser", "feedback", "--current", "--tab", "w1:t1",
        ]).is_err());
        assert!(Cli::try_parse_from(["cockpit", "browser", "open", "--space", "w1"]).is_err());
    }

    #[test]
    fn context_requires_current_or_an_explicit_space() {
        assert!(Cli::try_parse_from(["cockpit", "context"]).is_err());
        assert!(Cli::try_parse_from(["cockpit", "context", "--current"]).is_ok());
        assert!(Cli::try_parse_from(["cockpit", "context", "--current", "--space", "w1"]).is_err());
        assert!(Cli::try_parse_from([
            "cockpit", "context", "--herdr-session", "fixture", "--herdr-socket", "/fixture/herdr.sock", "--space", "w1",
        ]).is_ok());
    }

    #[test]
    fn detached_legacy_feedback_is_not_addressable() {
        assert!(Cli::try_parse_from([
            "cockpit", "browser", "feedback", "ack",
            "--legacy", "0123456789abcdef01234567", "--id", "capture-one",
        ]).is_err());
    }
}
