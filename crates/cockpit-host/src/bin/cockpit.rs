use std::{net::SocketAddr, path::PathBuf, sync::Arc};

use clap::{Args, Parser, Subcommand};
use cockpit_core::{CockpitService, HerdrAdapter};
use cockpit_herdr::{HerdrCliAdapter, HerdrCliConfig};
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
struct ServeArgs {
    #[command(flatten)]
    herdr: HerdrArgs,
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

fn make_service(herdr: HerdrArgs, mode: CockpitMode) -> Result<CockpitService, String> {
    let config = HerdrCliConfig::from_options(herdr.herdr, herdr.herdr_session, herdr.herdr_socket)
        .map_err(|error| error.to_string())?;
    let inspector: Arc<dyn HerdrAdapter> = Arc::new(HerdrCliAdapter::new(config));
    Ok(CockpitService::new(mode, inspector))
}

async fn run(cli: Cli) -> Result<(), String> {
    match cli.command {
        Command::Status(args) => {
            let service = make_service(args.herdr, CockpitMode::Normal)?;
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
            let service = make_service(args.herdr, mode)?;
            serve(ServerConfig {
                bind: args.bind,
                static_dir: args.static_dir,
                service,
            })
            .await
            .map_err(|error| error.to_string())
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
