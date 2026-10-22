use std::{
    io::Write,
    path::{Path, PathBuf},
    sync::Arc,
};

use clap::{Args, Subcommand};
use cockpit_core::{InspectionError, space_tags::SpaceTagService};
use cockpit_herdr::HerdrCliAdapter;
use cockpit_protocol::v1::{ErrorResponse, SpaceTagChangeRequest, SpaceTagsResponse};

use super::{
    HerdrArgs, current_pane_id, current_pane_space,
    endpoint::{AmbientEndpoint, resolve_endpoint},
};

pub(super) const TAGS_HELP: &str = "Organise Spaces with tags shared with the Cockpit sidebar. Uses the same
state_root/space-tags store as the app; no HTTP owner, Library or credentials
are needed. Tags contain 1-32 characters and match case-insensitively;
whitespace is normalised and new spellings join the catalog.

Targets:
  default / --current  Space of the calling Herdr pane; requires HERDR_ENV=1
  --space ID           explicit Space; requires --herdr-session and --herdr-socket
  list --all           every tagged live Space in the session; refuses --current
                       and --space. Outside Herdr, requires both endpoint flags.
COCKPIT_HERDR_SESSION and COCKPIT_HERDR_SOCKET also supply explicit selectors.
There is no ambient default-session fallback outside a calling Herdr pane.

Output is always one JSON object on stdout; no --json flag.
Space: {session_id, server_instance, space_id, changed, tags, catalog}.
Session (list --all): {session_id, server_instance, catalog, spaces}.
changed compares the tags before and after this command, not exclusive ownership
of a concurrent edit. list always reports changed:false. An unknown explicit
Space lists empty tags; add/remove reject a Space that is no longer live.
list may prune stale stored records. Removing a tag keeps it in the catalog.

add/remove are idempotent. Each mutation reads a fresh server_instance and
refuses to write if Herdr restarts; there is no automatic retry.
Exit codes: 0 success; 2 usage/target required; 4 unavailable target;
9 changed identity (nothing written); 11 invalid tag/limit; 12 store busy;
20 other failure (outcome may be unknown). On exit 9 re-run; on exit 20 list
before re-running add/remove. Errors are also JSON on stdout and text on stderr.
Pass -- before a tag that starts with -.

Examples:
  cockpit-cli tags add \"needs review\"
  cockpit-cli tags --current list
  cockpit-cli tags remove \"needs review\"
  cockpit-cli tags list --all
  cockpit-cli tags --space SPACE --herdr-session SESSION --herdr-socket SOCKET add testing";

#[derive(Debug, Args)]
pub(super) struct TagsArgs {
    #[command(flatten)]
    herdr: HerdrArgs,
    /// Cockpit config path; resolves state_root.
    #[arg(long, global = true, env = "COCKPIT_CONFIG_PATH")]
    config: Option<PathBuf>,
    /// Target the calling Herdr pane's Space (the default); requires HERDR_ENV=1.
    #[arg(long, global = true, conflicts_with = "space")]
    current: bool,
    /// Explicit Space ID; requires --herdr-session and --herdr-socket.
    #[arg(long, global = true)]
    space: Option<String>,
    #[command(subcommand)]
    command: TagsCommand,
}

#[derive(Debug, Subcommand)]
enum TagsCommand {
    /// Show the target Space's tags and the tag catalog.
    List {
        /// Show every tagged live Space in the session instead of one Space.
        #[arg(long)]
        all: bool,
    },
    /// Assign TAG (1-32 characters; matched case-insensitively).
    Add { tag: String },
    /// Unassign TAG from the Space; the catalog keeps it.
    Remove { tag: String },
}

#[derive(Debug, PartialEq, Eq)]
enum TagsTarget {
    Current,
    Space(String),
    Session,
}

fn usage(message: impl Into<String>) -> InspectionError {
    InspectionError::new("tags_usage", message)
}

fn select_target(
    current: bool,
    space: Option<String>,
    all: bool,
    in_herdr: bool,
    explicit_endpoint: bool,
) -> Result<TagsTarget, InspectionError> {
    if all {
        if current || space.is_some() {
            return Err(usage("list --all does not accept --current or --space"));
        }
        if !in_herdr && !explicit_endpoint {
            return Err(usage("list --all outside Herdr requires --herdr-session and --herdr-socket"));
        }
        return Ok(TagsTarget::Session);
    }
    if let Some(space) = space {
        if !explicit_endpoint {
            return Err(usage("--space requires --herdr-session and --herdr-socket"));
        }
        return Ok(TagsTarget::Space(space));
    }
    if !in_herdr {
        return Err(InspectionError::new(
            "tags_target_required",
            "Run inside a Herdr pane (default or --current), or provide --space ID with --herdr-session and --herdr-socket",
        ));
    }
    Ok(TagsTarget::Current)
}

#[derive(serde::Serialize)]
struct SpaceView {
    session_id: String,
    server_instance: String,
    space_id: String,
    changed: bool,
    tags: Vec<String>,
    catalog: Vec<String>,
}

fn space_view(
    response: SpaceTagsResponse,
    space_id: String,
    before: Option<&[String]>,
) -> SpaceView {
    let tags = response.spaces.into_iter()
        .find(|space| space.space_id == space_id)
        .map(|space| space.tags)
        .unwrap_or_default();
    SpaceView {
        session_id: response.session_id,
        server_instance: response.server_instance,
        space_id,
        changed: before.is_some_and(|before| before != tags),
        tags,
        catalog: response.catalog,
    }
}

#[derive(serde::Serialize)]
#[serde(untagged)]
enum TagsOutput {
    Space(SpaceView),
    Session(SpaceTagsResponse),
}

async fn execute(args: TagsArgs) -> Result<TagsOutput, InspectionError> {
    let explicit_endpoint = args.herdr.herdr_session.as_ref()
        .is_some_and(|session| !session.trim().is_empty())
        && args.herdr.herdr_socket.as_ref()
            .is_some_and(|socket| !socket.as_os_str().is_empty());
    let target = select_target(
        args.current,
        args.space,
        matches!(args.command, TagsCommand::List { all: true }),
        std::env::var("HERDR_ENV").as_deref() == Ok("1"),
        explicit_endpoint,
    )?;
    let config = cockpit_core::config::ConfigurationFile::load(args.config.as_deref())?
        .project
        .resolve(None)?;
    let endpoint = resolve_endpoint(
        args.herdr.herdr,
        args.herdr.herdr_session,
        args.herdr.herdr_socket,
        &AmbientEndpoint::from_process(),
    ).map_err(|error| usage(error.message))?;
    let pane = if target == TagsTarget::Current {
        Some(current_pane_id(
            endpoint.executable(),
            endpoint.socket(),
            Some(&endpoint.session),
            "tags",
        ).await.map_err(|error| InspectionError::new("tags_current_unavailable", error))?)
    } else {
        None
    };
    let adapter = Arc::new(HerdrCliAdapter::new(endpoint.config));
    let space_id = match target {
        TagsTarget::Current => Some(current_pane_space(
            &adapter, &endpoint.session, pane.as_deref().expect("current target resolved a pane"),
        ).await?),
        TagsTarget::Space(space) => Some(space),
        TagsTarget::Session => None,
    };
    let service = SpaceTagService::new(Path::new(&config.state_root), adapter)?;
    let before = service.list(&endpoint.session).await?;
    let Some(space_id) = space_id else {
        return Ok(TagsOutput::Session(before));
    };
    let (tag, assigned) = match args.command {
        TagsCommand::List { .. } => {
            return Ok(TagsOutput::Space(space_view(before, space_id, None)));
        }
        TagsCommand::Add { tag } => (tag, true),
        TagsCommand::Remove { tag } => (tag, false),
    };
    let after = service.change(&endpoint.session, &SpaceTagChangeRequest {
        space_id: space_id.clone(),
        server_instance: before.server_instance,
        tag,
        assigned,
    }).await?;
    let before_tags = before.spaces.iter()
        .find(|space| space.space_id == space_id)
        .map(|space| space.tags.as_slice())
        .unwrap_or_default();
    Ok(TagsOutput::Space(space_view(after, space_id, Some(before_tags))))
}

#[derive(serde::Serialize)]
struct TagsCliError {
    error: ErrorResponse,
}

fn exit_code(code: &str) -> u8 {
    match code {
        "tags_usage" | "tags_target_required" => 2,
        "space_tags_space_unavailable" | "tags_current_unavailable" => 4,
        "space_tags_target_changed" | "stale_identity" => 9,
        "invalid_space_tag" | "space_tags_limit" => 11,
        "space_tags_lock" => 12,
        _ => 20,
    }
}

fn write_json(value: &impl serde::Serialize) -> std::io::Result<()> {
    let mut output = std::io::stdout().lock();
    serde_json::to_writer(&mut output, value).map_err(std::io::Error::other)?;
    output.write_all(b"\n")
}

pub(super) fn print_error(error: InspectionError) -> u8 {
    eprintln!("error: {}: {}", error.code, error.message);
    let exit = exit_code(&error.code);
    let output = TagsCliError {
        error: ErrorResponse { code: error.code, message: error.message },
    };
    if let Err(error) = write_json(&output) {
        eprintln!("error: tags_outcome_unknown: Cannot write the tags response: {error}");
        return 20;
    }
    exit
}

pub(super) async fn run(args: TagsArgs) -> u8 {
    match execute(args).await {
        Ok(response) => match write_json(&response) {
            Ok(()) => 0,
            Err(error) => {
                eprintln!("error: tags_outcome_unknown: Cannot write the tags response: {error}");
                20
            }
        },
        Err(error) => print_error(error),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use cockpit_protocol::v1::SpaceTags;
    use serde_json::json;

    #[test]
    fn target_selection_fences_explicit_and_ambient_endpoints() {
        assert_eq!(select_target(false, None, false, true, false).unwrap(), TagsTarget::Current);
        assert_eq!(select_target(true, None, false, true, false).unwrap(), TagsTarget::Current);
        for current in [false, true] {
            assert_eq!(select_target(current, None, false, false, true).unwrap_err().code, "tags_target_required");
        }
        assert_eq!(select_target(false, Some("w1".into()), false, false, true).unwrap(), TagsTarget::Space("w1".into()));
        for in_herdr in [false, true] {
            assert_eq!(select_target(false, Some("w1".into()), false, in_herdr, false).unwrap_err().code, "tags_usage");
        }
        assert_eq!(select_target(false, None, true, true, false).unwrap(), TagsTarget::Session);
        assert_eq!(select_target(false, None, true, false, true).unwrap(), TagsTarget::Session);
        assert_eq!(select_target(false, None, true, false, false).unwrap_err().code, "tags_usage");
        assert_eq!(select_target(true, None, true, true, true).unwrap_err().code, "tags_usage");
        assert_eq!(select_target(false, Some("w1".into()), true, true, true).unwrap_err().code, "tags_usage");
    }

    fn response() -> SpaceTagsResponse {
        SpaceTagsResponse {
            session_id: "fixture".into(),
            server_instance: "incarnation".into(),
            catalog: vec!["review".into(), "testing".into()],
            spaces: vec![SpaceTags {
                space_id: "w1".into(),
                tags: vec!["review".into(), "testing".into()],
            }],
        }
    }

    #[test]
    fn space_output_preserves_catalog_and_reports_semantic_no_ops() {
        let expected = json!({
            "session_id": "fixture", "server_instance": "incarnation", "space_id": "w1",
            "changed": false, "tags": ["review", "testing"], "catalog": ["review", "testing"],
        });
        let before = vec!["review".to_owned(), "testing".to_owned()];
        assert_eq!(serde_json::to_value(space_view(response(), "w1".into(), None)).unwrap(), expected);
        assert_eq!(serde_json::to_value(space_view(response(), "w1".into(), Some(&before))).unwrap(), expected);
        assert!(space_view(response(), "w1".into(), Some(&[])).changed);
        let absent = space_view(response(), "untagged".into(), Some(&[]));
        assert!(absent.tags.is_empty());
        assert!(!absent.changed);
        let removed = space_view(response(), "untagged".into(), Some(&before));
        assert!(removed.changed);
        assert_eq!(removed.catalog, before);
        let session = serde_json::to_value(TagsOutput::Session(response())).unwrap();
        assert!(session.get("space_id").is_none());
        assert_eq!(session["spaces"][0]["space_id"], "w1");
    }
}
