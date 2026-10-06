use std::{
    io::{Read, Write},
    path::PathBuf,
    sync::Arc,
};

use clap::{Args, Subcommand, ValueEnum};
use cockpit_core::{InspectionError, notes::NotesService};
use cockpit_herdr::HerdrCliAdapter;
use cockpit_protocol::{notes::*, v1::ErrorResponse};

use super::{
    HerdrArgs, current_pane_id,
    endpoint::{AmbientEndpoint, resolve_endpoint},
};

#[derive(Debug, Args)]
pub(super) struct NotesArgs {
    #[command(flatten)]
    herdr: HerdrArgs,
    #[arg(long, global = true, env = "COCKPIT_CONFIG_PATH")]
    config: Option<PathBuf>,
    #[arg(long, global = true, conflicts_with_all = ["space", "notes"])]
    current: bool,
    #[arg(long, global = true, conflicts_with = "notes")]
    space: Option<String>,
    /// Durable Notes UUID; never connects to Herdr or a Cockpit owner.
    #[arg(long, global = true)]
    notes: Option<String>,
    #[command(subcommand)]
    command: NotesCommand,
}

#[derive(Debug, Subcommand)]
enum NotesCommand {
    Catalog,
    Target {
        #[arg(long, conflicts_with = "attach")]
        create: bool,
        #[arg(long)]
        attach: Option<String>,
    },
    Scratchpad {
        #[command(subcommand)]
        command: ScratchpadCommand,
    },
    Todo {
        #[command(subcommand)]
        command: TodoCommand,
    },
    Kanban {
        #[command(subcommand)]
        command: KanbanCommand,
    },
    Decision {
        #[command(subcommand)]
        command: DecisionCommand,
    },
    Comment {
        #[command(subcommand)]
        command: CommentCommand,
    },
}

#[derive(Debug, Args)]
struct Payload {
    #[arg(long, conflicts_with_all = ["stdin", "file"])]
    text: Option<String>,
    #[arg(long, conflicts_with = "file")]
    stdin: bool,
    #[arg(long)]
    file: Option<PathBuf>,
}

impl Payload {
    fn supplied(&self) -> bool {
        self.text.is_some() || self.stdin || self.file.is_some()
    }

    fn read(
        self,
        limit: usize,
        required: bool,
        allow_text: bool,
    ) -> Result<String, InspectionError> {
        if !allow_text && self.text.is_some() {
            return Err(usage("This verb accepts --stdin or --file, not --text"));
        }
        if !self.supplied() {
            return if required {
                Err(usage("Provide exactly one of --text, --stdin or --file"))
            } else {
                Ok(String::new())
            };
        }
        let bytes = if let Some(text) = self.text {
            text.into_bytes()
        } else if let Some(path) = self.file {
            let file = std::fs::File::open(path)
                .map_err(|error| InspectionError::new("notes_invalid_input", error.to_string()))?;
            read_bounded(file, limit)?
        } else {
            read_bounded(std::io::stdin().lock(), limit)?
        };
        if bytes.len() > limit {
            return Err(InspectionError::new(
                "notes_too_large",
                "Notes input exceeds its byte limit",
            ));
        }
        String::from_utf8(bytes).map_err(|_| {
            InspectionError::new("notes_invalid_encoding", "Notes input must be valid UTF-8")
        })
    }
}

fn read_bounded(reader: impl Read, limit: usize) -> Result<Vec<u8>, InspectionError> {
    let mut bytes = Vec::new();
    reader
        .take((limit + 1) as u64)
        .read_to_end(&mut bytes)
        .map_err(|error| InspectionError::new("notes_invalid_input", error.to_string()))?;
    Ok(bytes)
}

#[derive(Debug, Args)]
struct TodoSelector {
    #[arg(
        long,
        required_unless_present = "reference",
        conflicts_with = "reference",
        requires = "expected_revision"
    )]
    id: Option<String>,
    #[arg(
        long = "ref",
        required_unless_present = "id",
        conflicts_with = "expected_revision"
    )]
    reference: Option<String>,
    #[arg(long, requires = "id")]
    expected_revision: Option<String>,
}

impl TodoSelector {
    fn into_selector(self) -> Result<NotesTodoSelector, InspectionError> {
        match (self.id, self.reference, self.expected_revision) {
            (Some(id), None, Some(expected_revision)) => {
                revision(&expected_revision, false)?;
                if !valid_id(&id, 64) {
                    return Err(usage("--id must be a bounded todo identifier"));
                }
                Ok(NotesTodoSelector::Id {
                    id,
                    expected_revision,
                })
            }
            (None, Some(reference), None) => {
                let Some((line, rev)) = reference.split_once('@') else {
                    return Err(usage("--ref must be L<line>@sha256:<hash>"));
                };
                if line
                    .strip_prefix('L')
                    .filter(|number| {
                        !number.is_empty() && number.bytes().all(|byte| byte.is_ascii_digit())
                    })
                    .and_then(|number| number.parse::<u32>().ok())
                    .filter(|number| *number > 0)
                    .is_none()
                {
                    return Err(usage("--ref must contain a positive line number"));
                }
                revision(rev, false)?;
                Ok(NotesTodoSelector::Ref { reference })
            }
            _ => Err(usage(
                "Select a todo with --id and --expected-revision, or --ref",
            )),
        }
    }
}

#[derive(Debug, Clone, Copy, ValueEnum)]
enum Lane {
    Backlog,
    Doing,
}
impl From<Lane> for NotesLane {
    fn from(lane: Lane) -> Self {
        match lane {
            Lane::Backlog => Self::Backlog,
            Lane::Doing => Self::Doing,
        }
    }
}
#[derive(Debug, Clone, Copy, ValueEnum)]
enum Column {
    Backlog,
    Doing,
    Done,
}
impl From<Column> for NotesColumn {
    fn from(column: Column) -> Self {
        match column {
            Column::Backlog => Self::Backlog,
            Column::Doing => Self::Doing,
            Column::Done => Self::Done,
        }
    }
}
#[derive(Debug, Clone, Copy, ValueEnum)]
enum DecisionStatus {
    Current,
    History,
    All,
}
impl From<DecisionStatus> for NotesDecisionFilter {
    fn from(status: DecisionStatus) -> Self {
        match status {
            DecisionStatus::Current => Self::Current,
            DecisionStatus::History => Self::History,
            DecisionStatus::All => Self::All,
        }
    }
}

#[derive(Debug, Subcommand)]
enum ScratchpadCommand {
    Read,
    Append {
        #[command(flatten)]
        payload: Payload,
        #[arg(long)]
        expected_revision: Option<String>,
    },
    Replace {
        #[command(flatten)]
        payload: Payload,
        #[arg(long)]
        expected_revision: String,
    },
}
#[derive(Debug, Subcommand)]
enum TodoCommand {
    List {
        #[arg(long, conflicts_with = "done")]
        open: bool,
        #[arg(long)]
        done: bool,
    },
    Add {
        #[arg(long)]
        text: String,
        #[arg(long)]
        lane: Option<Lane>,
    },
    Update {
        #[command(flatten)]
        todo: TodoSelector,
        #[arg(long)]
        text: Option<String>,
    },
    Complete {
        #[command(flatten)]
        todo: TodoSelector,
    },
    Reopen {
        #[command(flatten)]
        todo: TodoSelector,
    },
    Remove {
        #[command(flatten)]
        todo: TodoSelector,
    },
}
#[derive(Debug, Subcommand)]
enum KanbanCommand {
    List,
    Add {
        #[arg(long)]
        text: String,
    },
    Promote {
        #[command(flatten)]
        todo: TodoSelector,
    },
    Move {
        #[command(flatten)]
        todo: TodoSelector,
        #[arg(long)]
        to: Column,
    },
    Unboard {
        #[command(flatten)]
        todo: TodoSelector,
    },
}
#[derive(Debug, Subcommand)]
enum DecisionCommand {
    List {
        #[arg(long, default_value = "current")]
        status: DecisionStatus,
        #[arg(long)]
        query: Option<String>,
    },
    Get {
        #[arg(long)]
        id: String,
    },
    Create {
        #[arg(long)]
        title: String,
        #[arg(long)]
        decided: Option<String>,
        #[command(flatten)]
        payload: Payload,
    },
    Update {
        #[arg(long)]
        id: String,
        #[arg(long)]
        expected_revision: String,
        #[arg(long)]
        title: Option<String>,
        #[command(flatten)]
        payload: Payload,
    },
    Replace {
        #[arg(long)]
        id: String,
        #[arg(long)]
        expected_revision: String,
        #[arg(long)]
        title: String,
        #[arg(long)]
        decided: Option<String>,
        #[command(flatten)]
        payload: Payload,
    },
}
#[derive(Debug, Subcommand)]
enum CommentCommand {
    List {
        #[arg(long)]
        todo: String,
    },
    Get {
        #[arg(long)]
        todo: String,
        #[arg(long)]
        comment: String,
    },
    Add {
        #[arg(long)]
        todo: String,
        #[arg(long)]
        author: Option<String>,
        #[command(flatten)]
        payload: Payload,
    },
    Update {
        #[arg(long)]
        todo: String,
        #[arg(long)]
        comment: String,
        #[arg(long)]
        expected_revision: String,
        #[command(flatten)]
        payload: Payload,
    },
    Remove {
        #[arg(long)]
        todo: String,
        #[arg(long)]
        comment: String,
        #[arg(long)]
        expected_revision: String,
    },
}

fn usage(message: impl Into<String>) -> InspectionError {
    InspectionError::new("notes_usage", message.into())
}
fn valid_id(id: &str, limit: usize) -> bool {
    !id.is_empty()
        && id.len() <= limit
        && id
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'_' || byte == b'-')
}
fn revision(value: &str, absent: bool) -> Result<(), InspectionError> {
    if (absent && value == "absent")
        || value.strip_prefix("sha256:").is_some_and(|hash| {
            hash.len() == 64
                && hash
                    .bytes()
                    .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
        })
    {
        Ok(())
    } else {
        Err(usage(
            "--expected-revision must be a SHA-256 revision (or absent for a missing document)",
        ))
    }
}

impl NotesCommand {
    fn into_operation(self) -> Result<NotesOperation, InspectionError> {
        Ok(match self {
            Self::Catalog => NotesOperation::CatalogList,
            Self::Target { create: true, .. } => NotesOperation::TargetCreate,
            Self::Target {
                attach: Some(notes_id),
                ..
            } => NotesOperation::TargetAttach { notes_id },
            Self::Target { .. } => NotesOperation::TargetResolve,
            Self::Scratchpad { command } => match command {
                ScratchpadCommand::Read => NotesOperation::ScratchpadRead,
                ScratchpadCommand::Append {
                    payload,
                    expected_revision,
                } => {
                    if let Some(value) = &expected_revision {
                        revision(value, true)?;
                    }
                    NotesOperation::ScratchpadAppend {
                        text: payload.read(1024 * 1024, true, true)?,
                        expected_revision,
                    }
                }
                ScratchpadCommand::Replace {
                    payload,
                    expected_revision,
                } => {
                    revision(&expected_revision, true)?;
                    NotesOperation::ScratchpadReplace {
                        content: payload.read(1024 * 1024, true, false)?,
                        expected_revision,
                    }
                }
            },
            Self::Todo { command } => match command {
                TodoCommand::List { open, done } => NotesOperation::TodoList {
                    filter: if open {
                        NotesTodoFilter::Open
                    } else if done {
                        NotesTodoFilter::Done
                    } else {
                        NotesTodoFilter::All
                    },
                },
                TodoCommand::Add { text, lane } => NotesOperation::TodoAdd {
                    text,
                    lane: lane.map(Into::into),
                },
                TodoCommand::Update { todo, text } => NotesOperation::TodoUpdate {
                    todo: todo.into_selector()?,
                    text,
                },
                TodoCommand::Complete { todo } => NotesOperation::TodoSetDone {
                    todo: todo.into_selector()?,
                    done: true,
                },
                TodoCommand::Reopen { todo } => NotesOperation::TodoSetDone {
                    todo: todo.into_selector()?,
                    done: false,
                },
                TodoCommand::Remove { todo } => NotesOperation::TodoRemove {
                    todo: todo.into_selector()?,
                },
            },
            Self::Kanban { command } => match command {
                KanbanCommand::List => NotesOperation::KanbanList,
                KanbanCommand::Add { text } => NotesOperation::TodoAdd {
                    text,
                    lane: Some(NotesLane::Backlog),
                },
                KanbanCommand::Promote { todo } => NotesOperation::KanbanPromote {
                    todo: todo.into_selector()?,
                },
                KanbanCommand::Move { todo, to } => NotesOperation::KanbanMove {
                    todo: todo.into_selector()?,
                    to: to.into(),
                },
                KanbanCommand::Unboard { todo } => NotesOperation::KanbanUnboard {
                    todo: todo.into_selector()?,
                },
            },
            Self::Decision { command } => match command {
                DecisionCommand::List { status, query } => NotesOperation::DecisionList {
                    status: status.into(),
                    query,
                },
                DecisionCommand::Get { id } => NotesOperation::DecisionGet { decision_id: id },
                DecisionCommand::Create {
                    title,
                    decided,
                    payload,
                } => NotesOperation::DecisionCreate {
                    title,
                    body: payload.read(256 * 1024, false, false)?,
                    decided,
                },
                DecisionCommand::Update {
                    id,
                    expected_revision,
                    title,
                    payload,
                } => {
                    revision(&expected_revision, false)?;
                    let body = if payload.supplied() {
                        Some(payload.read(256 * 1024, false, false)?)
                    } else {
                        None
                    };
                    if title.is_none() && body.is_none() {
                        return Err(usage("decision update requires --title, --stdin or --file"));
                    }
                    NotesOperation::DecisionUpdate {
                        decision_id: id,
                        expected_revision,
                        title,
                        body,
                    }
                }
                DecisionCommand::Replace {
                    id,
                    expected_revision,
                    title,
                    decided,
                    payload,
                } => {
                    revision(&expected_revision, false)?;
                    NotesOperation::DecisionReplace {
                        decision_id: id,
                        expected_revision,
                        title,
                        body: payload.read(256 * 1024, false, false)?,
                        decided,
                    }
                }
            },
            Self::Comment { command } => match command {
                CommentCommand::List { todo } => NotesOperation::CommentList { todo_id: todo },
                CommentCommand::Get { todo, comment } => NotesOperation::CommentGet {
                    todo_id: todo,
                    comment_id: comment,
                },
                CommentCommand::Add {
                    todo,
                    author,
                    payload,
                } => NotesOperation::CommentAdd {
                    todo_id: todo,
                    body: payload.read(64 * 1024, true, true)?,
                    author,
                },
                CommentCommand::Update {
                    todo,
                    comment,
                    expected_revision,
                    payload,
                } => {
                    revision(&expected_revision, false)?;
                    NotesOperation::CommentUpdate {
                        todo_id: todo,
                        comment_id: comment,
                        expected_revision,
                        body: payload.read(64 * 1024, true, true)?,
                    }
                }
                CommentCommand::Remove {
                    todo,
                    comment,
                    expected_revision,
                } => {
                    revision(&expected_revision, false)?;
                    NotesOperation::CommentRemove {
                        todo_id: todo,
                        comment_id: comment,
                        expected_revision,
                    }
                }
            },
        })
    }
}

async fn execute(args: NotesArgs) -> Result<NotesResponse, InspectionError> {
    let operation = args.command.into_operation()?;
    let config = cockpit_core::config::load_project_configuration(args.config.as_deref(), None)?;
    let mut service = NotesService::new(PathBuf::from(config.notes_root));
    if matches!(operation, NotesOperation::CatalogList) {
        if args.current || args.space.is_some() || args.notes.is_some() {
            return Err(usage("catalog does not accept a target"));
        }
        return service
            .execute(NotesRequest {
                target: NotesTarget::Root,
                operation,
            })
            .await;
    }
    let mut target = if let Some(notes_id) = args.notes {
        NotesTarget::Notes { notes_id }
    } else {
        if !args.current && args.space.is_none() {
            return Err(InspectionError::new(
                "notes_target_required",
                "Provide --notes UUID, --current, or an explicit --space target",
            ));
        }
        let endpoint = resolve_endpoint(
            args.herdr.herdr.clone(),
            args.herdr.herdr_session.clone(),
            args.herdr.herdr_socket.clone(),
            &AmbientEndpoint::from_process(),
        )
        .map_err(|error| usage(error.message))?;
        if !args.current
            && (args.herdr.herdr_session.is_none() || args.herdr.herdr_socket.is_none())
        {
            return Err(usage("--space requires --herdr-session and --herdr-socket"));
        }
        let pane = if args.current {
            Some(
                current_pane_id(
                    endpoint.executable(),
                    endpoint.socket(),
                    Some(&endpoint.session),
                    "notes",
                )
                .await
                .map_err(|error| InspectionError::new("notes_space_unavailable", error))?,
            )
        } else {
            None
        };
        let adapter = Arc::new(HerdrCliAdapter::new(endpoint.config));
        let space_id = if let Some(pane) = pane {
            let source = adapter.source_adapter();
            let before = source.source_pane_evidence(&endpoint.session, &pane).await?;
            let after = source.source_pane_evidence(&endpoint.session, &pane).await?;
            if before.pane_id != pane
                || after.pane_id != pane
                || before.workspace_id != after.workspace_id
                || before.tab_id != after.tab_id
                || before.endpoint_identity != after.endpoint_identity
            {
                return Err(InspectionError::new(
                    "stale_identity",
                    "The originating pane identity changed during lookup",
                ));
            }
            before.workspace_id
        } else {
            args.space
                .ok_or_else(|| usage("No Space target supplied"))?
        };
        service = service.with_herdr(adapter);
        NotesTarget::Space {
            session_id: endpoint.session,
            space_id,
        }
    };
    if !matches!(
        operation,
        NotesOperation::TargetResolve
            | NotesOperation::TargetCreate
            | NotesOperation::TargetAttach { .. }
    ) && matches!(target, NotesTarget::Space { .. })
    {
        let resolved = service
            .execute(NotesRequest {
                target,
                operation: NotesOperation::TargetResolve,
            })
            .await?;
        target = NotesTarget::Notes {
            notes_id: resolved.notes_id.ok_or_else(|| {
                InspectionError::new("notes_unbound", "This Space has no Notes association")
            })?,
        };
    }
    service.execute(NotesRequest { target, operation }).await
}

#[derive(serde::Serialize)]
struct NotesCliError {
    error: ErrorResponse,
}

fn write_json(value: &impl serde::Serialize) -> std::io::Result<()> {
    let mut output = std::io::stdout().lock();
    serde_json::to_writer(&mut output, value).map_err(std::io::Error::other)?;
    output.write_all(b"\n")
}

pub(super) fn print_error(error: InspectionError) -> u8 {
    eprintln!("error: {}: {}", error.code, error.message);
    let exit = match error.code.as_str() {
        "notes_usage" | "notes_target_required" | "notes_unbound" => 2,
        "notes_not_found" | "notes_not_on_board" => 4,
        "notes_conflict" | "notes_decision_replaced" | "notes_already_bound" => 9,
        "notes_todo_ambiguous" | "notes_todo_malformed" | "notes_todo_has_children" => 10,
        "notes_invalid_input"
        | "notes_too_large"
        | "notes_invalid_encoding"
        | "notes_unsafe_path"
        | "notes_invalid_target" => 11,
        "notes_busy" => 12,
        _ => 20,
    };
    let output = NotesCliError {
        error: ErrorResponse {
            code: error.code,
            message: error.message,
        },
    };
    if let Err(error) = write_json(&output) {
        eprintln!("error: notes_outcome_unknown: Cannot write the Notes response: {error}");
        return 20;
    }
    exit
}

pub(super) async fn run(args: NotesArgs) -> u8 {
    match execute(args).await {
        Ok(response) => match write_json(&response) {
            Ok(()) => 0,
            Err(error) => {
                eprintln!("error: notes_outcome_unknown: Cannot write the Notes response: {error}");
                20
            }
        },
        Err(error) => print_error(error),
    }
}
