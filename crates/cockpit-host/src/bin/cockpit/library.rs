use std::{collections::HashSet, path::Path, time::Duration};

use clap::{Args, Subcommand};
use cockpit_protocol::library::{
    LibraryAddRequest, LibraryOperation, LibraryPhaseName, LibraryPhaseState,
};

use super::{HerdrArgs, ProjectArgs, context_item_json, resolve_space_context};

pub(crate) const LIBRARY_HELP: &str = "Save one input to the global Library, then select the saved items in a Space.
Existing items are selected without refetching or overwriting their saved content.
Inputs: provider URLs, Jira JQL, Confluence space/page URLs, or absolute/~/ folder
paths. Jira queries and Confluence spaces require --follow. Relative folder paths
are not resolved against the process cwd. Reference depth is 0; attachments are
not downloaded and existing items are not refreshed.

Providers are read-only: this command never writes to a remote provider. Tokens
must already be stored through Cockpit; this command does not accept credentials.
Each CLI process uses its own provider request pacer, independent of the host.

Use --current inside Herdr (HERDR_ENV=1) to resolve the originating pane's Space.
Otherwise provide --space ID, --herdr-session SESSION and --herdr-socket PATH.
Use --config PATH or COCKPIT_CONFIG_PATH for the same Library configuration as the app.

The command waits for the local operation to finish. Interrupting it keeps items
already saved; the operation may subsequently be reported as interrupted.
After an operation starts, stdout is one JSON object with target, space_label,
pane_id, library_root, operation and selected items (item_id, title, kind, path).
Exit 0 means every phase completed; exit 1 means partial/failure/cancellation or
an operational error; exit 2 means invalid command usage. Saved items are retained
on partial failure.

Examples:
  cockpit-cli library add --current https://git.example/team/project/issues/42
  cockpit-cli library add --current --provider jira --follow 'project = OPS'
  cockpit-cli library add --space w1 --herdr-session work --herdr-socket /tmp/herdr.sock /home/me/docs";

#[derive(Debug, Args)]
pub(crate) struct LibraryArgs {
    #[command(subcommand)]
    pub(crate) command: LibraryCommand,
}

#[derive(Debug, Subcommand)]
pub(crate) enum LibraryCommand {
    /// Save one input and select its saved items in the target Space.
    #[command(after_long_help = LIBRARY_HELP)]
    Add(LibraryAddArgs),
}

#[derive(Debug, Args)]
pub(crate) struct LibraryAddArgs {
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
    /// Configured provider identity, when the input needs disambiguation.
    #[arg(long)]
    provider: Option<String>,
    /// Follow a Jira query or Confluence space using the configured limits.
    #[arg(long)]
    follow: bool,
    /// Provider URL, Jira JQL, Confluence space/page URL, or absolute/~/ folder path.
    input: String,
}

pub(crate) async fn run(args: LibraryArgs) -> Result<u8, String> {
    match args.command {
        LibraryCommand::Add(args) => add(args).await,
    }
}

async fn add(args: LibraryAddArgs) -> Result<u8, String> {
    let context = resolve_space_context(
        args.herdr,
        args.project,
        args.current,
        args.space,
        "library add",
    )
    .await?;
    let preflight = context
        .library
        .space_listing(&context.target)
        .await
        .map_err(|error| error.to_string())?;
    // Check the original source and target immediately before dispatch, never
    // resolve a replacement target if the pane moved during the preflight.
    context.verify_source_identity("library add").await?;
    let mut operation = context
        .library
        .start_add(LibraryAddRequest {
            input: args.input,
            provider_id: args.provider,
            reference_depth: 0,
            follow: args.follow,
            follow_mode: None,
            download_attachments: false,
            refresh_existing: false,
            label: None,
            target: Some(context.target.clone()),
        })
        .await
        .map_err(|error| error.to_string())?;
    let mut observation_error = None;
    while !operation.finished {
        tokio::time::sleep(Duration::from_millis(250)).await;
        match context.library.operation(&operation.operation_id).await {
            Ok(current) => operation = current,
            Err(error) => {
                observation_error = Some(error.to_string());
                break;
            }
        }
    }
    let mut exit = add_exit(&operation);
    // A Space can disappear after items have been saved. Still report the
    // operation and original target, rather than losing the partial result.
    let listing = match context.library.space_listing(&context.target).await {
        Ok(listing) => Some(listing),
        Err(error) => {
            eprintln!("cockpit: library add could not read selected items: {error}");
            exit = 1;
            None
        }
    };
    let root = Path::new(&preflight.library_root);
    let selected = operation
        .space
        .as_ref()
        .map(|space| {
            space
                .item_ids
                .iter()
                .map(String::as_str)
                .collect::<HashSet<_>>()
        })
        .unwrap_or_default();
    let items = listing
        .as_ref()
        .map(|listing| {
            listing
                .items
                .iter()
                .filter(|item| selected.contains(item.item_id.as_str()))
                .map(|item| context_item_json(root, item))
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    println!("{}", serde_json::to_string_pretty(&serde_json::json!({
        "target": context.target,
        "space_label": listing.as_ref().map(|listing| &listing.space_label).unwrap_or(&preflight.space_label),
        "pane_id": context.pane_id,
        "library_root": preflight.library_root,
        "operation": operation,
        "items": items,
    })).map_err(|error| error.to_string())?);
    if let Some(error) = observation_error {
        eprintln!(
            "cockpit: library add could not observe operation {}: {error}",
            operation.operation_id
        );
        exit = 1;
    }
    if operation.finished && add_exit(&operation) != 0 {
        let phases = operation
            .phases
            .iter()
            .filter(|phase| phase.state != LibraryPhaseState::Done)
            .map(|phase| {
                let name = match phase.phase {
                    LibraryPhaseName::Library => "library",
                    LibraryPhaseName::Space => "space",
                };
                let state = match phase.state {
                    LibraryPhaseState::Pending => "pending",
                    LibraryPhaseState::Running => "running",
                    LibraryPhaseState::Done => "done",
                    LibraryPhaseState::Partial => "partial",
                    LibraryPhaseState::Failed => "failed",
                    LibraryPhaseState::Cancelled => "cancelled",
                };
                format!("{name}={state}")
            })
            .collect::<Vec<_>>()
            .join(", ");
        eprintln!("cockpit: library add finished: {phases}");
    }
    Ok(exit)
}

fn add_exit(operation: &LibraryOperation) -> u8 {
    if operation.finished
        && operation
            .phases
            .iter()
            .all(|phase| phase.state == LibraryPhaseState::Done)
    {
        0
    } else {
        1
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use clap::Parser;
    use cockpit_protocol::library::{LibraryOperationKind, LibraryPhase};

    #[test]
    fn add_parser_requires_input_and_one_target() {
        for arguments in [
            vec!["cockpit", "library", "add"],
            vec!["cockpit", "library", "add", "/home/me/docs"],
            vec!["cockpit", "library", "add", "--current"],
        ] {
            assert_eq!(
                crate::Cli::try_parse_from(arguments).unwrap_err().kind(),
                clap::error::ErrorKind::MissingRequiredArgument
            );
        }
        assert_eq!(
            crate::Cli::try_parse_from([
                "cockpit",
                "library",
                "add",
                "--current",
                "--space",
                "w1",
                "/home/me/docs",
            ])
            .unwrap_err()
            .kind(),
            clap::error::ErrorKind::ArgumentConflict
        );
    }

    #[test]
    fn add_parser_accepts_current_and_explicit_targets_with_provider_follow() {
        for target in [
            vec!["--current"],
            vec![
                "--space",
                "w1",
                "--herdr-session",
                "fixture",
                "--herdr-socket",
                "/fixture/herdr.sock",
            ],
        ] {
            let cli = crate::Cli::try_parse_from(
                ["cockpit", "library", "add"]
                    .into_iter()
                    .chain(target)
                    .chain(["--provider", "jira", "--follow", "project = OPS"]),
            )
            .unwrap();
            let crate::Command::Library(args) = cli.command else {
                panic!("expected Library command")
            };
            let LibraryCommand::Add(args) = args.command;
            assert_eq!(args.provider.as_deref(), Some("jira"));
            assert!(args.follow);
            assert_eq!(args.input, "project = OPS");
        }
    }

    #[test]
    fn add_exit_requires_every_phase_to_finish_successfully() {
        let mut operation = LibraryOperation {
            operation_id: "add-operation".into(),
            kind: LibraryOperationKind::Add,
            phases: [LibraryPhaseName::Library, LibraryPhaseName::Space]
                .into_iter()
                .map(|phase| LibraryPhase {
                    phase,
                    state: LibraryPhaseState::Done,
                    done: 1,
                    total: Some(1),
                    message: None,
                    error: None,
                })
                .collect(),
            item_ids: vec!["saved-item".into()],
            report: None,
            space: None,
            target: None,
            cancel_requested: false,
            finished: true,
            created_at: "0".into(),
            updated_at: "0".into(),
        };
        assert_eq!(add_exit(&operation), 0);
        for phase in 0..operation.phases.len() {
            for state in [
                LibraryPhaseState::Partial,
                LibraryPhaseState::Failed,
                LibraryPhaseState::Cancelled,
                LibraryPhaseState::Pending,
                LibraryPhaseState::Running,
            ] {
                operation.phases[phase].state = state;
                assert_eq!(add_exit(&operation), 1, "phase {phase}: {state:?}");
            }
            operation.phases[phase].state = LibraryPhaseState::Done;
        }
        operation.finished = false;
        assert_eq!(add_exit(&operation), 1);
    }
}
