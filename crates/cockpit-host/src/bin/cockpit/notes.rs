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

pub(super) const NOTES_HELP: &str = "Durable Space Notes: Scratchpad, todos, Kanban board, decisions and todo
comments, stored as Markdown under notes_root in the Cockpit config or
COCKPIT_NOTES_ROOT. On Linux the default is $XDG_DATA_HOME/cockpit/notes,
or $HOME/.local/share/cockpit/notes when XDG_DATA_HOME is unset.
Notes are not the Library and not part of any checkout.

Targets (exactly one; catalog takes none):
  --notes UUID   pinned Notes; needs neither Herdr nor a running Cockpit
  --current      Space of the calling Herdr pane (requires HERDR_ENV=1)
  --space ID     explicit Space; requires --herdr-session and --herdr-socket
Resolve once with `target`, then pin the returned notes_id and the Notes root
(the parent directory of result.info.folder) for every later call.
Export COCKPIT_NOTES_ROOT to that root; set NOTES_ID to the returned UUID.
Cockpit-launched panes should prefer $COCKPIT_CLI_PATH when it is set.

Output: one JSON object on stdout: {\"notes_id\", \"changed\", \"result\"}, where
result.kind names the shape. changed=false means nothing was written.
Errors: {\"error\": {\"code\", \"message\"}} on stdout and \"error: CODE: MESSAGE\"
on stderr. Exit codes: 0 ok; 2 usage, target required or Space unbound;
4 not found or not on board; 9 conflict, decision replaced or already bound;
10 ambiguous/malformed todo or nested tasks; 11 invalid input, too large,
bad encoding, unsafe path or invalid target; 12 busy (lock timeout, nothing
written; retry later); 20 anything else, including unknown write outcome.
If writing stdout fails, only stderr may report notes_outcome_unknown.

Existing-content writes use revisions from a fresh read; append's fence is
optional. Revisions are sha256: followed by 64 lowercase hex digits, or
absent for a missing Scratchpad. Change tokens are not CAS revisions.
On notes_conflict re-read and decide again with the new revision.
On exit 20 a write may have landed: re-read affected content before another
write; never replay blindly. todo add, kanban add, decision create and
comment add have no fence; repeating them creates duplicates.
Editors that ignore Cockpit locks can race.

Use `notes <area> --help` for workflows and `notes <area> <verb> --help`
for arguments. Example variables must come from your actual read results.

Examples:
  cockpit-cli notes --current target
  cockpit-cli notes catalog
  cockpit-cli notes --notes \"$NOTES_ID\" todo list --open
  cockpit-cli notes --notes \"$NOTES_ID\" kanban list";

const TARGET_HELP: &str = "Without flags: resolve the target's Notes (read-only).
result.info has notes_id, folder (absolute), space and change_tokens; change
tokens detect changes and are not revisions. Pin the returned notes_id and
COCKPIT_NOTES_ROOT (the parent directory of folder) for later content calls.
--create makes new Notes and binds the Space; --attach UUID binds existing
Notes. Both require a Space target and write the binding: only on request.
Attach transfers that UUID's association from any other Space, leaving its
content intact and the other Space unbound.
notes_unbound (exit 2): the Space has no Notes yet. notes_already_bound
(exit 9): this Space has different Notes; attach cannot overwrite that
binding. Creating or attaching the already bound UUID changes nothing.

Examples (ATTACH_ID is the UUID selected from catalog):
  cockpit-cli notes --current target
  cockpit-cli notes --current target --create
  cockpit-cli notes --current target --attach \"$ATTACH_ID\"";

const SCRATCHPAD_HELP: &str = "read returns result.document.content and result.document.revision; the
revision is \"absent\" while scratchpad.md does not exist. Reading creates
nothing. Set REVISION to the exact revision returned by your latest read.
append adds text at the end. --expected-revision is optional: pass the
revision you read (or \"absent\") so the append fails if someone edited first.
replace rewrites the whole file, requires --expected-revision, and takes
--stdin or --file only; --text is refused. Input limit: 1 MiB UTF-8.
append requires one of --text, --stdin or --file. Payload flags are mutually
exclusive. The saved Scratchpad is also limited to 1 MiB.
Pin NOTES_ID and COCKPIT_NOTES_ROOT as described in `notes --help`.
On conflict re-read; on exit 20 inspect saved content, never replay blindly.

Examples:
  cockpit-cli notes --notes \"$NOTES_ID\" scratchpad read
  cockpit-cli notes --notes \"$NOTES_ID\" scratchpad append --text \"Isolated flaky test\" --expected-revision \"$REVISION\"
  cockpit-cli notes --notes \"$NOTES_ID\" scratchpad replace --file scratchpad.md --expected-revision \"$REVISION\"";

const TODO_HELP: &str = "todos.md is ordinary Markdown; the todo list and the Kanban board are views
of the same items. list returns result.revision (whole file) and result.todos[]
with id, ref, text, done, lane, revision, line, depth and problems.

Select one existing todo (update, complete, reopen, remove and kanban
promote, move, unboard):
  --id ID --expected-revision REV
      todo has a non-null id and no duplicate_id problem; REV is that todo's
      own revision from your latest list, not result.revision
  --ref REF
      any other todo; REF is its exact ref (L<line>@sha256:<file hash>).
      Any todos.md edit invalidates it. Do not also pass --expected-revision.
      A ref write adopts or repairs the stable id, returned in result.todo
      (except remove, which deletes the todo).
Set TODO_ID, ITEM_REVISION or TODO_REF from the selected todo's latest fields.
add appends a todo (--text, at most 2 KiB); --lane backlog|doing also puts it
on the board. add has no fence. update without --text can adopt a ref-selected
todo without changing its text. complete checks the box; reopen unchecks it
and restores the remembered board lane. remove refuses a todo that contains
nested tasks (exit 10); it retains that todo's comments.
Pin NOTES_ID and COCKPIT_NOTES_ROOT as described in `notes --help`.
On conflict re-read; on exit 20 inspect the list before another write.
Never blindly repeat add: it can create a duplicate.
Malformed metadata or lazy continuation ownership refuses writes (exit 10);
inspect and repair the Markdown rather than guessing another selector.

Examples:
  cockpit-cli notes --notes \"$NOTES_ID\" todo list --open
  cockpit-cli notes --notes \"$NOTES_ID\" todo add --text \"Add regression test\" --lane backlog
  cockpit-cli notes --notes \"$NOTES_ID\" todo complete --id \"$TODO_ID\" --expected-revision \"$ITEM_REVISION\"
  cockpit-cli notes --notes \"$NOTES_ID\" todo update --ref \"$TODO_REF\" --text \"Add regression test for #42\"";

const KANBAN_HELP: &str = "Board membership is todo lane metadata (backlog, doing); the done column is
boarded todos whose checkbox is checked. list returns result.revision and
result.columns with backlog, doing and done. Moves never reorder todos.md.
add is todo add --lane backlog (text at most 2 KiB, no revision fence).
promote gives an unboarded todo the Backlog lane, preserving its checkbox;
a checked todo appears in Done. Already boarded todos keep their lane.
move --to backlog|doing unchecks the box and sets the lane; --to done checks
it and remembers its open lane.
unboard removes board membership and keeps the todo and its comments.
notes_not_on_board (exit 4): promote the todo first.

Selectors: --id with --expected-revision uses the selected todo's own
revision, not the whole-file result.revision. If id is null or duplicate,
use its exact --ref from the list instead (no --expected-revision).
A ref includes the whole-file revision; any todos.md edit invalidates it.
Use fresh item fields for TODO_ID and ITEM_REVISION. See `notes todo --help`.
Pin NOTES_ID and COCKPIT_NOTES_ROOT as described in `notes --help`.
On conflict re-read; on exit 20 inspect the board before another write.
Never blindly repeat add: it can create a duplicate.

Examples:
  cockpit-cli notes --notes \"$NOTES_ID\" kanban list
  cockpit-cli notes --notes \"$NOTES_ID\" kanban promote --id \"$TODO_ID\" --expected-revision \"$ITEM_REVISION\"
  cockpit-cli notes --notes \"$NOTES_ID\" kanban move --id \"$TODO_ID\" --expected-revision \"$ITEM_REVISION\" --to doing";

const DECISION_HELP: &str = "Each decision is its own Markdown record. list --status current|history|all
with optional --query (substring of title or body) returns summaries with
decision_id, title, status, replaces, replaced_by and revision. get --id
returns result.decision with summary, body and path.
Set DECISION_ID and REVISION from the latest list summary or get's summary;
use that decision's revision, not a change token.
create --title accepts optional --decided (YYYY-MM-DD or RFC3339) and an
optional body from --stdin or --file; create has no revision fence.
update --id --expected-revision changes title and/or body in place; supply
at least --title, --stdin or --file. Omitted body is kept, not cleared.
replace --id --expected-revision --title creates a new decision that replaces
the old one; the old record is not rewritten. Its body is optional.
Replacing an already replaced decision fails with notes_decision_replaced
(exit 9). Body input must be UTF-8, at most 256 KiB; the complete decision
record is also bounded to 256 KiB. --text is refused for every decision verb.
--stdin and --file are mutually exclusive.
Pin NOTES_ID and COCKPIT_NOTES_ROOT as described in `notes --help`.
On conflict re-read; on exit 20 inspect decisions before another write.
Never blindly repeat create: it can create a duplicate.

Examples:
  cockpit-cli notes --notes \"$NOTES_ID\" decision list --status all --query fixture
  cockpit-cli notes --notes \"$NOTES_ID\" decision create --title \"Use disposable fixtures\" --file decision.md
  cockpit-cli notes --notes \"$NOTES_ID\" decision replace --id \"$DECISION_ID\" --expected-revision \"$REVISION\" --title \"Use owned fixtures\" --file decision.md";

const COMMENT_HELP: &str = "Comments belong to a todo with a stable id (--todo is that id; a todo whose
id is null gets one from its first --ref write). list and get read; each
comment has comment_id, created, author, body and revision.
add requires one of --text, --stdin or --file (UTF-8, at most 64 KiB) and
accepts optional --author, an unverified label; add has no revision fence.
update requires a new body from one of those mutually exclusive inputs;
update and remove need --expected-revision from that comment's latest
revision, read from list or get. Set COMMENT_ID and REVISION from that record.
Comments are retained when their todo is removed or unboarded.
Pin NOTES_ID and COCKPIT_NOTES_ROOT as described in `notes --help`.
On conflict re-read; on exit 20 inspect the thread before another write.
Never blindly repeat add: it can create a duplicate.

Examples:
  cockpit-cli notes --notes \"$NOTES_ID\" comment list --todo \"$TODO_ID\"
  cockpit-cli notes --notes \"$NOTES_ID\" comment add --todo \"$TODO_ID\" --text \"Repro needs a clean HOME.\"
  cockpit-cli notes --notes \"$NOTES_ID\" comment remove --todo \"$TODO_ID\" --comment \"$COMMENT_ID\" --expected-revision \"$REVISION\"";

#[derive(Debug, Args)]
pub(super) struct NotesArgs {
    #[command(flatten)]
    herdr: HerdrArgs,
    /// Cockpit config path; resolves notes_root unless COCKPIT_NOTES_ROOT is set.
    #[arg(long, global = true, env = "COCKPIT_CONFIG_PATH")]
    config: Option<PathBuf>,
    /// Target the calling Herdr pane's Space; requires HERDR_ENV=1.
    #[arg(long, global = true, conflicts_with_all = ["space", "notes"])]
    current: bool,
    /// Explicit Space ID; requires --herdr-session and --herdr-socket.
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
    /// List available Notes UUIDs without selecting a target (read-only).
    Catalog,
    /// Resolve Notes, or explicitly create/attach a Space binding.
    #[command(after_long_help = TARGET_HELP)]
    Target {
        /// Create and bind Notes to the Space; use only on explicit request.
        #[arg(long, conflicts_with = "attach")]
        create: bool,
        /// Bind an existing Notes UUID, transferring its association; explicit request only.
        #[arg(long)]
        attach: Option<String>,
    },
    /// Read, append or revision-replace the Scratchpad Markdown.
    #[command(after_long_help = SCRATCHPAD_HELP)]
    Scratchpad {
        #[command(subcommand)]
        command: ScratchpadCommand,
    },
    /// List and edit Markdown todos using item revisions or whole-file refs.
    #[command(after_long_help = TODO_HELP)]
    Todo {
        #[command(subcommand)]
        command: TodoCommand,
    },
    /// View and move boarded todos without reordering their Markdown source.
    #[command(after_long_help = KANBAN_HELP)]
    Kanban {
        #[command(subcommand)]
        command: KanbanCommand,
    },
    /// Read, create, edit or replace durable decision records.
    #[command(after_long_help = DECISION_HELP)]
    Decision {
        #[command(subcommand)]
        command: DecisionCommand,
    },
    /// Read and edit durable comments addressed by stable todo IDs.
    #[command(after_long_help = COMMENT_HELP)]
    Comment {
        #[command(subcommand)]
        command: CommentCommand,
    },
}

#[derive(Debug, Args)]
struct Payload {
    /// Literal UTF-8 body; refused by Scratchpad replace and all decision writes.
    #[arg(long, conflicts_with_all = ["stdin", "file"])]
    text: Option<String>,
    /// Read UTF-8 body from stdin; byte limit depends on the verb.
    #[arg(long, conflicts_with = "file")]
    stdin: bool,
    /// Read UTF-8 body from this file; byte limit depends on the verb.
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
    /// Unique stable todo ID from list; requires that item's expected revision.
    #[arg(
        long,
        required_unless_present = "reference",
        conflicts_with = "reference",
        requires = "expected_revision"
    )]
    id: Option<String>,
    /// Exact ref from list (L<line>@sha256:<file hash>); adopts or repairs the ID.
    #[arg(
        long = "ref",
        required_unless_present = "id",
        conflicts_with = "expected_revision"
    )]
    reference: Option<String>,
    /// Selected todo's own revision from list, not the whole-file revision.
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
    /// Boarded and open in Backlog.
    Backlog,
    /// Boarded and open in Doing.
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
    /// Uncheck the todo and set its remembered lane to Backlog.
    Backlog,
    /// Uncheck the todo and set its remembered lane to Doing.
    Doing,
    /// Check the todo and preserve its remembered open lane.
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
    /// Decisions with no replacement successor.
    Current,
    /// Decisions replaced by another record.
    History,
    /// Both current and replaced decisions.
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
    /// Read saved Markdown and its revision without creating a file.
    Read,
    /// Append UTF-8 text, optionally fenced by the last read revision.
    Append {
        #[command(flatten)]
        payload: Payload,
        /// Exact document revision from read, or absent; omit only for unfenced append.
        #[arg(long)]
        expected_revision: Option<String>,
    },
    /// Replace all Scratchpad Markdown using the last read revision.
    Replace {
        #[command(flatten)]
        payload: Payload,
        /// Exact document revision from read, or absent for a missing Scratchpad.
        #[arg(long)]
        expected_revision: String,
    },
}
#[derive(Debug, Subcommand)]
enum TodoCommand {
    /// Read todo items, their selectors and the whole-file revision.
    List {
        /// List only unchecked todos.
        #[arg(long, conflicts_with = "done")]
        open: bool,
        /// List only checked todos.
        #[arg(long)]
        done: bool,
    },
    /// Append a new todo without a revision fence.
    Add {
        /// Todo text, at most 2 KiB UTF-8.
        #[arg(long)]
        text: String,
        /// Also put the new todo on the board in this open lane.
        #[arg(long)]
        lane: Option<Lane>,
    },
    /// Update todo text, or adopt/repair a ref-selected todo without changing text.
    Update {
        #[command(flatten)]
        todo: TodoSelector,
        /// Replacement todo text, at most 2 KiB; omit to keep its text.
        #[arg(long)]
        text: Option<String>,
    },
    /// Check the selected todo's box without changing its remembered lane.
    Complete {
        #[command(flatten)]
        todo: TodoSelector,
    },
    /// Uncheck the selected todo and restore its remembered board lane.
    Reopen {
        #[command(flatten)]
        todo: TodoSelector,
    },
    /// Remove a todo without nested tasks; retain its comment thread.
    Remove {
        #[command(flatten)]
        todo: TodoSelector,
    },
}
#[derive(Debug, Subcommand)]
enum KanbanCommand {
    /// Read board columns, todo selectors and the whole-file revision.
    List,
    /// Append a new todo in Backlog without a revision fence.
    Add {
        /// Todo text, at most 2 KiB UTF-8; added to Backlog.
        #[arg(long)]
        text: String,
    },
    /// Board an unboarded todo with the Backlog lane; preserve its checkbox.
    Promote {
        #[command(flatten)]
        todo: TodoSelector,
    },
    /// Change an existing board todo's column without reordering source.
    Move {
        #[command(flatten)]
        todo: TodoSelector,
        /// Destination; backlog/doing uncheck the box, done checks it.
        #[arg(long)]
        to: Column,
    },
    /// Remove board membership while keeping the todo and comments.
    Unboard {
        #[command(flatten)]
        todo: TodoSelector,
    },
}
#[derive(Debug, Subcommand)]
enum DecisionCommand {
    /// Read decision summaries with per-record revisions.
    List {
        /// Select current, replaced (history), or all decisions.
        #[arg(long, default_value = "current")]
        status: DecisionStatus,
        /// Case-insensitive substring of decision title or body.
        #[arg(long)]
        query: Option<String>,
    },
    /// Read a decision's summary, Markdown body and file path.
    Get {
        /// decision_id from decision list or a returned decision summary.
        #[arg(long)]
        id: String,
    },
    /// Create a decision with an optional file/stdin body; no revision fence.
    Create {
        /// Decision title, at most 512 UTF-8 bytes.
        #[arg(long)]
        title: String,
        /// Optional historical decided date: YYYY-MM-DD or RFC3339 timestamp.
        #[arg(long)]
        decided: Option<String>,
        #[command(flatten)]
        payload: Payload,
    },
    /// Change a decision title and/or body in place with a revision fence.
    Update {
        /// decision_id from decision list or get's summary.
        #[arg(long)]
        id: String,
        /// Exact decision revision from list or get's summary.
        #[arg(long)]
        expected_revision: String,
        /// New title, at most 512 UTF-8 bytes; omit to keep the current title.
        #[arg(long)]
        title: Option<String>,
        #[command(flatten)]
        payload: Payload,
    },
    /// Create a linked successor without rewriting the predecessor.
    Replace {
        /// Current decision_id to replace, from list or get's summary.
        #[arg(long)]
        id: String,
        /// Exact predecessor revision from list or get's summary.
        #[arg(long)]
        expected_revision: String,
        /// Successor title, at most 512 UTF-8 bytes.
        #[arg(long)]
        title: String,
        /// Optional successor decided date: YYYY-MM-DD or RFC3339 timestamp.
        #[arg(long)]
        decided: Option<String>,
        #[command(flatten)]
        payload: Payload,
    },
}
#[derive(Debug, Subcommand)]
enum CommentCommand {
    /// Read a todo's comments and their individual revisions.
    List {
        /// Stable todo ID from todo/kanban list or a returned todo; not a ref.
        #[arg(long)]
        todo: String,
    },
    /// Read one saved comment and its revision.
    Get {
        /// Stable todo ID owning the comment thread.
        #[arg(long)]
        todo: String,
        /// comment_id from comment list or a returned comment.
        #[arg(long)]
        comment: String,
    },
    /// Add a comment body without a revision fence.
    Add {
        /// Unique stable todo ID from todo/kanban list or a returned todo.
        #[arg(long)]
        todo: String,
        /// Optional unverified author label, not authenticated identity.
        #[arg(long)]
        author: Option<String>,
        #[command(flatten)]
        payload: Payload,
    },
    /// Replace a comment body using its individual revision.
    Update {
        /// Stable todo ID owning the comment thread.
        #[arg(long)]
        todo: String,
        /// comment_id from comment list or get.
        #[arg(long)]
        comment: String,
        /// Exact selected comment revision from comment list or get.
        #[arg(long)]
        expected_revision: String,
        #[command(flatten)]
        payload: Payload,
    },
    /// Remove one comment using its individual revision.
    Remove {
        /// Stable todo ID owning the comment thread.
        #[arg(long)]
        todo: String,
        /// comment_id from comment list or get.
        #[arg(long)]
        comment: String,
        /// Exact selected comment revision from comment list or get.
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
    let config = cockpit_core::config::ConfigurationFile::load(args.config.as_deref())?
        .project
        .resolve(None)?;
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
