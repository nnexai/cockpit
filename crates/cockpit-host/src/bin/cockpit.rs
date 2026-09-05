use std::{net::SocketAddr, path::PathBuf, sync::Arc};

use clap::{Args, Parser, Subcommand};
use cockpit_core::{CockpitService, config::load_project_configuration, projects::ProjectService};
use cockpit_herdr::{HerdrCliAdapter, HerdrCliConfig};
use cockpit_protocol::projects::ProjectConfiguration;
use cockpit_protocol::v1::CockpitMode;

use cockpit_host::server::{ServerConfig, serve};

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
}

#[derive(Debug, Args)]
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
        Some(config) => Ok(service.with_projects(
            ProjectService::new(config, adapter).map_err(|error| error.to_string())?,
        )),
        None => Ok(service),
    }
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
            let service = make_service(args.herdr, mode, projects)?;
            serve(ServerConfig {
                bind: args.bind,
                static_dir: args.static_dir,
                service,
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
    }
}

#[tokio::main]
async fn main() {
    if let Err(error) = run(Cli::parse()).await {
        eprintln!("cockpit: {error}");
        std::process::exit(1);
    }
}
