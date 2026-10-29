use std::path::PathBuf;

use clap::{Args, Subcommand, ValueEnum};
use cockpit_protocol::{
    orchestration::{
        DispatchTarget, MessageKind, OrchestrationAction, ReportKind, ReportOutcome,
        TaskStepScope, WorkerCleanupPolicy,
    },
    projects::{WorkspaceRecoveryAction, WorkspaceSetupRequest},
};

use super::context::{read_input, read_json_file};
use super::{CliError, MESSAGE_HELP, WORKER_START_HELP, REPORT_HELP, ROUTE_HELP, INTEGRATION_RECORD_HELP, CLEANUP_FINISH_HELP};

pub(super) const MAX_TEXT_BYTES: usize = 16 * 1024;


/// These options work both before and after any nested command.
#[derive(Debug, Args)]
pub(super) struct OrchestrationArgs {
    /// Herdr executable to invoke.
    #[arg(long, global = true, env = "COCKPIT_HERDR_EXECUTABLE")]
    pub(super) herdr: Option<PathBuf>,
    /// Logical Herdr session; inherited caller identity is used when omitted.
    #[arg(long, global = true, env = "COCKPIT_HERDR_SESSION")]
    pub(super) herdr_session: Option<String>,
    /// Explicit Herdr socket endpoint, never inferred from UI focus.
    #[arg(long, global = true, env = "COCKPIT_HERDR_SOCKET")]
    pub(super) herdr_socket: Option<PathBuf>,
    /// Cockpit configuration used by the launching supervisor.
    #[arg(long, global = true, env = "COCKPIT_CONFIG_PATH")]
    pub(super) config: Option<PathBuf>,
    /// Catalog root for existing local repositories; may be repeated.
    #[arg(long = "repository-root", global = true)]
    pub(super) repository_roots: Vec<PathBuf>,
    /// Explicit local operator authority; ignores ambient job credentials and launch metadata.
    #[arg(long, global = true)]
    pub(super) operator: bool,
    /// Task root. Operator writes require an explicit root; agents stay bound to their own root.
    #[arg(long, global = true)]
    pub(super) root: Option<String>,
    /// Emit machine-readable JSON; message bodies are untrusted data.
    #[arg(long, global = true)]
    pub(super) json: bool,
    /// Actual harness kind; must agree with COCKPIT_HARNESS when present.
    #[arg(long, global = true)]
    pub(super) harness: Option<String>,
    /// Actual conversation ID, distinct from the physical Herdr session.
    #[arg(long, global = true)]
    pub(super) session_id: Option<String>,
}

#[derive(Debug, Args)]
pub(crate) struct ProtocolArgs {
    #[command(subcommand)]
    pub(super) command: ProtocolCommand,
}

#[derive(Debug, Subcommand)]
pub(super) enum ProtocolCommand {
    /// Read the installed CLI's orchestration contract without connecting to Herdr.
    Protocol {
        #[arg(long)]
        json: bool,
    },
}



#[derive(Debug, Args)]
pub(super) struct DescriptionArgs {
    /// Literal UTF-8 prose (at most 16 KiB); reserved metadata/checklists are not writable.
    #[arg(long, conflicts_with_all = ["description_file", "stdin"])]
    pub(super) description: Option<String>,
    /// Read UTF-8 prose from a file (at most 16 KiB).
    #[arg(long, conflicts_with = "stdin")]
    pub(super) description_file: Option<PathBuf>,
    /// Read UTF-8 prose from stdin (at most 16 KiB).
    #[arg(long)]
    pub(super) stdin: bool,
}
impl DescriptionArgs {
    pub(super) fn read(self) -> Result<Option<String>, CliError> {
        match self.description {
            Some(description) => bounded(description).map(Some),
            None => read_input(self.description_file, self.stdin),
        }
    }
}

pub(super) fn bounded(text: String) -> Result<String, CliError> {
    if text.len() > MAX_TEXT_BYTES {
        Err(CliError::new("message_too_large", "text exceeds 16 KiB"))
    } else {
        Ok(text)
    }
}

#[derive(Debug, Args)]
pub(crate) struct TaskArgs {
    #[command(flatten)]
    pub(super) common: OrchestrationArgs,
    #[command(subcommand)]
    pub(super) command: TaskCommand,
}
#[derive(Debug, Clone, Copy, ValueEnum)]
pub(super) enum StepScopeArg {
    Leaf,
    Subtree,
}
impl From<StepScopeArg> for TaskStepScope {
    fn from(value: StepScopeArg) -> Self {
        match value {
            StepScopeArg::Leaf => Self::Leaf,
            StepScopeArg::Subtree => Self::Subtree,
        }
    }
}

#[derive(Debug, Args)]
pub(super) struct StepTaskArgs {
    /// Canonical task UUID from task list.
    pub(super) task: String,
    /// Optional exact task revision; otherwise captured internally before one core CAS.
    #[arg(long)]
    pub(super) revision: Option<String>,
}

fn revision_fence(revision: Option<String>) -> Result<String, CliError> {
    revision.ok_or_else(|| CliError::usage("Task mutation needs an internally resolved canonical revision"))
}

#[derive(Debug, Subcommand)]
pub(super) enum TaskCommand {
    /// List canonical tasks, dependency state, step progress and exact document revision.
    List,
    /// Read description, read-only body, diagnostics, step IDs/offsets, relationships and revision.
    Show {
        /// Canonical task UUID from task list.
        #[arg(required_unless_present = "self_task", conflicts_with = "self_task")]
        task: Option<String>,
        /// Read the task assigned to this credentialed worker.
        #[arg(long = "self")]
        self_task: bool,
    },
    /// Read direct prerequisite scope and valid accepted Result/source references; no publication or mutation.
    Inputs {
        /// Canonical task UUID; omission selects the caller's assigned task.
        #[arg(conflicts_with = "self_task")]
        task: Option<String>,
        #[arg(long = "self")]
        self_task: bool,
    },
    /// Authorize autonomous execution once dependencies and a parallel slot permit it.
    Schedule {
        task: String,
        #[arg(long)]
        operation_id: Option<String>,
        /// Optional exact task/document fences; omitted fences use the current canonical scope.
        #[arg(long)]
        task_revision: Option<String>,
        #[arg(long)]
        doc_revision: Option<String>,
        #[command(flatten)]
        launch: Box<LaunchArgs>,
    },
    /// Stop future scheduling; does not cancel or stop an existing attempt.
    ScheduleCancel {
        task: String,
        #[arg(long)]
        operation_id: Option<String>,
    },
    /// Replace the current or pending attempt with internal generation/run fencing.
    Restart {
        task: String,
        #[arg(long)]
        operation_id: Option<String>,
        #[arg(long)]
        reason: String,
    },
    /// Revoke a task assignment without stopping its physical worker or releasing resources.
    Unassign {
        task: String,
        #[arg(long)]
        operation_id: String,
        #[arg(long)]
        expected_assignment_generation: u64,
    },
    /// Create without starting a worker; retain the task ID and inspect before any retry.
    #[command(
        after_long_help = "Choose --task-id once and retain it. create does not launch a worker.
After any uncertain submission, inspect task show with that same ID in the
same root before retrying; never append a second task with a new ID.
Relationship document revisions are resolved when omitted; follow-up needs
source_revision from task show SOURCE --json (task.task_revision)."
    )]
    Create {
        /// Stable caller UUID; generated once when omitted and reported before submission.
        #[arg(long)]
        task_id: Option<String>,
        /// Task title; description prose is supplied separately.
        #[arg(long)]
        title: String,
        #[command(flatten)]
        description: DescriptionArgs,
        /// Prerequisite task reference; repeat for each prerequisite.
        #[arg(long)]
        depends_on: Vec<String>,
        /// Follow-up source UUID; provenance only, not an implicit prerequisite.
        #[arg(long, requires = "source_revision")]
        follow_up_of: Option<String>,
        /// Optional exact root document revision; resolved when omitted.
        #[arg(long)]
        doc_revision: Option<String>,
        /// Exact follow-up source task revision from task show.
        #[arg(long, requires = "follow_up_of")]
        source_revision: Option<String>,
    },
    /// Update title/prose with a current or explicit task fence.
    Update {
        /// Canonical task UUID from task list.
        task: String,
        /// Optional exact task revision; resolved when omitted.
        #[arg(long)]
        revision: Option<String>,
        /// New task title; omit to preserve the current title.
        #[arg(long)]
        title: Option<String>,
        #[command(flatten)]
        description: DescriptionArgs,
    },
    /// Update a Working worker's canonical scope and deliver its durable instruction atomically.
    ScopeUpdate {
        /// Canonical task UUID owned by the selected live worker.
        task: String,
        /// Optional operation UUID; generated and printed when omitted.
        #[arg(long)]
        operation_id: Option<String>,
        /// Optional live worker reference; defaults to the task's current Working run.
        #[arg(long)]
        run: Option<String>,
        /// Optional exact task revision; resolved when omitted.
        #[arg(long)]
        revision: Option<String>,
        /// Optional exact document revision; resolved when omitted.
        #[arg(long)]
        doc_revision: Option<String>,
        #[arg(long)]
        title: Option<String>,
        #[command(flatten)]
        description: DescriptionArgs,
    },
    /// Replace ALL prerequisites; omit --depends-on to clear. Live work permits removal only.
    DependenciesSet {
        #[command(flatten)]
        task: StepTaskArgs,
        /// Exact doc_revision from task list --json, in addition to the task fence.
        #[arg(long)]
        doc_revision: String,
        /// Complete prerequisite UUID set; repeat for each, omit all to clear.
        #[arg(long)]
        depends_on: Vec<String>,
    },
    /// Add a stable-ID step; omitted parent means top level, omitted before means append.
    StepAdd {
        #[command(flatten)]
        task: StepTaskArgs,
        /// Stable caller-chosen UUID for the new step; retain it after uncertain writes.
        #[arg(long)]
        step_id: String,
        /// Existing parent step UUID from task show; omit for top level.
        #[arg(long)]
        parent_step_id: Option<String>,
        /// Existing sibling UUID to insert before; omit to append.
        #[arg(long)]
        before_step_id: Option<String>,
        /// New checklist step title.
        #[arg(long)]
        title: String,
    },
    /// Rename a step by its stable UUID.
    StepRename {
        #[command(flatten)]
        task: StepTaskArgs,
        /// Existing step UUID from task show.
        #[arg(long)]
        step_id: String,
        /// Replacement step title.
        #[arg(long)]
        title: String,
    },
    /// Set checked state explicitly; subtree scope updates every descendant atomically.
    #[command(
        after_long_help = "Use the stable task/step IDs from task show --self --json.
The CLI captures the current task revision internally; --revision preserves an
explicit fence. leaf requires no children; subtree updates every descendant.
Checklist completion is not supervisor acceptance."
    )]
    StepSetChecked {
        #[command(flatten)]
        task: StepTaskArgs,
        /// Existing step UUID from task show.
        #[arg(long)]
        step_id: String,
        /// Explicit checked state: true or false, never an implicit toggle.
        #[arg(long, action = clap::ArgAction::Set, required = true)]
        checked: bool,
        /// leaf for a childless step; subtree for the step and all descendants.
        #[arg(long, value_enum)]
        scope: StepScopeArg,
    },
    /// Move a step with its subtree; omitted parent means top level, before means sibling.
    StepMove {
        #[command(flatten)]
        task: StepTaskArgs,
        /// Existing step UUID to move with its subtree.
        #[arg(long)]
        step_id: String,
        /// Destination parent UUID from task show; omit for top level.
        #[arg(long)]
        parent_step_id: Option<String>,
        /// Destination sibling UUID to move before; omit to append.
        #[arg(long)]
        before_step_id: Option<String>,
    },
    /// Remove a stable-ID step and its subtree atomically.
    StepRemove {
        #[command(flatten)]
        task: StepTaskArgs,
        /// Existing step UUID; removal includes every descendant.
        #[arg(long)]
        step_id: String,
    },
    /// Assign stable task IDs to unmarked root items with a document revision fence.
    AssignIds {
        /// Exact doc_revision from task list --json.
        #[arg(long)]
        doc_revision: String,
    },
}

impl TaskCommand {
    pub(super) fn step_args(&mut self) -> Option<&mut StepTaskArgs> {
        match self {
            Self::DependenciesSet { task, .. } | Self::StepAdd { task, .. }
                | Self::StepRename { task, .. } | Self::StepSetChecked { task, .. }
                | Self::StepMove { task, .. } | Self::StepRemove { task, .. } => Some(task),
            _ => None,
        }
    }

    pub(super) fn action(self, root_id: String) -> Result<OrchestrationAction, CliError> {
        Ok(match self {
            Self::Unassign { task, operation_id, expected_assignment_generation } =>
                OrchestrationAction::TaskUnassign {
                    root_id, task_id: task, operation_id, expected_assignment_generation,
                },
            Self::Create {
                task_id,
                title,
                description,
                depends_on,
                follow_up_of,
                doc_revision,
                source_revision,
            } => OrchestrationAction::TaskCreate {
                root_id,
                task_id: task_id.unwrap_or_else(|| uuid::Uuid::new_v4().to_string()),
                title,
                description: description.read()?.unwrap_or_default(),
                depends_on,
                follow_up_of,
                expected_doc_revision: doc_revision,
                source_revision,
            },
            Self::Update {
                task,
                revision,
                title,
                description,
            } => {
                let description = description.read()?;
                if title.is_none() && description.is_none() {
                    return Err(CliError::usage(
                        "task update requires --title, --description, --description-file or --stdin",
                    ));
                }
                OrchestrationAction::TaskUpdate {
                    root_id,
                    task_id: task,
                    expected_task_revision: revision_fence(revision)?,
                    title,
                    description,
                }
            }
            Self::ScopeUpdate {
                task, operation_id, run, revision, doc_revision, title, description,
            } => {
                let description = description.read()?;
                if title.is_none() && description.is_none() {
                    return Err(CliError::usage(
                        "task scope-update requires --title, --description, --description-file or --stdin",
                    ));
                }
                OrchestrationAction::TaskScopeUpdate {
                    operation_id: submission_id(operation_id, "Operation")?, root_id, task_id: task,
                    run_id: run.ok_or_else(|| CliError::usage("Scope update needs a current worker"))?,
                    expected_task_revision: revision_fence(revision)?,
                    expected_doc_revision: revision_fence(doc_revision)?,
                    title, description,
                }
            }
            Self::DependenciesSet {
                task,
                doc_revision,
                depends_on,
            } => OrchestrationAction::TaskDependenciesSet {
                root_id,
                task_id: task.task,
                expected_task_revision: revision_fence(task.revision)?,
                expected_doc_revision: doc_revision,
                depends_on,
            },
            Self::StepAdd {
                task,
                step_id,
                parent_step_id,
                before_step_id,
                title,
            } => OrchestrationAction::TaskStepAdd {
                root_id,
                task_id: task.task,
                expected_task_revision: revision_fence(task.revision)?,
                step_id,
                parent_step_id,
                before_step_id,
                title,
            },
            Self::StepRename {
                task,
                step_id,
                title,
            } => OrchestrationAction::TaskStepRename {
                root_id,
                task_id: task.task,
                expected_task_revision: revision_fence(task.revision)?,
                step_id,
                title,
            },
            Self::StepSetChecked {
                task,
                step_id,
                checked,
                scope,
            } => OrchestrationAction::TaskStepSetChecked {
                root_id,
                task_id: task.task,
                expected_task_revision: revision_fence(task.revision)?,
                step_id,
                checked,
                scope: scope.into(),
            },
            Self::StepMove {
                task,
                step_id,
                parent_step_id,
                before_step_id,
            } => OrchestrationAction::TaskStepMove {
                root_id,
                task_id: task.task,
                expected_task_revision: revision_fence(task.revision)?,
                step_id,
                parent_step_id,
                before_step_id,
            },
            Self::StepRemove { task, step_id } => OrchestrationAction::TaskStepRemove {
                root_id,
                task_id: task.task,
                expected_task_revision: revision_fence(task.revision)?,
                step_id,
            },
            Self::ScheduleCancel { task, operation_id } => OrchestrationAction::TaskScheduleCancel {
                root_id, task_id: task, operation_id: submission_id(operation_id, "Operation")?,
            },
            Self::AssignIds { doc_revision } => OrchestrationAction::TasksAssignIds {
                root_id,
                expected_doc_revision: doc_revision,
            },
            Self::List | Self::Show { .. } | Self::Inputs { .. } | Self::Schedule { .. } | Self::Restart { .. } => {
                return Err(CliError::usage(
                    "read-only task command has no mutation action",
                ));
            }
        })
    }
}

#[derive(Debug, Args)]
pub(crate) struct RunArgs {
    #[command(flatten)]
    pub(super) common: OrchestrationArgs,
    #[command(subcommand)]
    pub(super) command: RunCommand,
}
#[derive(Debug, Clone, Copy, ValueEnum)]
pub(super) enum ReportKindArg {
    Progress,
    Initialized,
    Result,
    NeedsInput,
}
impl From<ReportKindArg> for ReportKind {
    fn from(kind: ReportKindArg) -> Self {
        match kind {
            ReportKindArg::Progress => Self::Progress,
            ReportKindArg::Initialized => Self::Initialized,
            ReportKindArg::Result => Self::Result,
            ReportKindArg::NeedsInput => Self::NeedsInput,
        }
    }
}
#[derive(Debug, Clone, Copy, ValueEnum)]
pub(super) enum OutcomeArg {
    Succeeded,
    Failed,
}
impl From<OutcomeArg> for ReportOutcome {
    fn from(value: OutcomeArg) -> Self {
        match value {
            OutcomeArg::Succeeded => Self::Succeeded,
            OutcomeArg::Failed => Self::Failed,
        }
    }
}
#[derive(Debug, Clone, Copy, ValueEnum)]
pub(super) enum MessageKindArg {
    Instruction,
    CancelRequest,
    Answer,
}

#[derive(Debug, Clone, Copy, ValueEnum)]
pub(super) enum CleanupPolicyArg {
    OwnedWhenSafe,
}
impl From<CleanupPolicyArg> for WorkerCleanupPolicy {
    fn from(policy: CleanupPolicyArg) -> Self {
        match policy {
            CleanupPolicyArg::OwnedWhenSafe => Self::OwnedWhenSafe,
        }
    }
}

#[derive(Debug, Args)]
pub(super) struct WorkerStartArgs {
    /// Stable operation UUID; generated and printed when omitted. Retain it after uncertainty.
    #[arg(long)]
    pub(super) operation_id: Option<String>,
    /// Optional exact task revision; otherwise read internally before the single CAS submission.
    #[arg(long)]
    pub(super) task_revision: Option<String>,
    /// Optional exact document revision; otherwise read internally before submission.
    #[arg(long)]
    pub(super) doc_revision: Option<String>,
    #[arg(long)]
    pub(super) task: String,
    #[command(flatten)]
    pub(super) launch: LaunchArgs,
}

#[derive(Debug, Args)]
#[group(skip)]
#[command(group(clap::ArgGroup::new("target").multiple(false).args(["repository", "path", "space", "space_worktree"])))]
pub(super) struct LaunchArgs {
    /// Standing authority only for resources proven owned by this launch.
    #[arg(long, value_enum, default_value = "owned-when-safe")]
    pub(super) cleanup_policy: CleanupPolicyArg,
    #[arg(long)]
    pub(super) repository: Option<String>,
    #[arg(long)]
    /// Open path; schedule shares a uniquely verified Git Space if clean, lone and without conflicting claims, isolates it otherwise; other paths are borrowed.
    pub(super) path: Option<String>,
    #[arg(long)]
    /// Intentional borrowed Space reuse; no shared-reader capability or teardown ownership.
    pub(super) space: Option<String>,
    /// Create an isolated owned worktree for concurrent writers or a different base or branch.
    #[arg(long)]
    pub(super) space_worktree: Option<String>,
    #[arg(long, conflicts_with_all = ["path", "space"], requires = "target")]
    pub(super) branch: Option<String>,
    #[arg(long, conflicts_with_all = ["path", "space"], requires = "target")]
    pub(super) base: Option<String>,
    #[arg(long, requires = "repository")]
    pub(super) checkout_path: Option<String>,
    #[arg(long, requires = "repository")]
    pub(super) artifact: Option<String>,
    #[arg(long, requires = "repository")]
    pub(super) linked_artifact: Vec<String>,
    #[arg(long, conflicts_with_all = ["space", "space_worktree"], requires = "target")]
    pub(super) task_name: Option<String>,
    /// Additional instructions only; canonical task prose is not copied into a new task store.
    #[arg(long, conflicts_with = "brief")]
    pub(super) brief_file: Option<PathBuf>,
    #[arg(long)]
    pub(super) brief: Option<String>,
    #[arg(long)]
    pub(super) label: Option<String>,
}

pub(super) struct LaunchInput {
    pub(super) target: Option<DispatchTarget>,
    pub(super) brief: String,
    pub(super) cleanup_policy: WorkerCleanupPolicy,
    pub(super) label: Option<String>,
}

impl LaunchArgs {
    pub(super) fn read(self) -> Result<LaunchInput, CliError> {
        let brief = match self.brief {
            Some(brief) => bounded(brief)?,
            None => read_input(self.brief_file, false)?.unwrap_or_default(),
        };
        let target = match (self.repository, self.path, self.space, self.space_worktree) {
            (Some(repository_id), None, None, None) => Some(DispatchTarget::Setup {
                request: WorkspaceSetupRequest::Create {
                    repository_id, branch: self.branch, base_ref: self.base,
                    checkout_path: self.checkout_path, label: self.label.clone(),
                    task_name: self.task_name, artifact_url: self.artifact,
                    linked_artifact_urls: self.linked_artifact, focus: false,
                },
            }),
            (None, Some(path), None, None) => Some(DispatchTarget::Setup {
                request: WorkspaceSetupRequest::Open {
                    path, label: self.label.clone(), task_name: self.task_name, focus: false,
                },
            }),
            (None, None, Some(workspace_id), None) => Some(DispatchTarget::ExistingSpace { workspace_id }),
            (None, None, None, Some(workspace_id)) => Some(DispatchTarget::SpaceWorktree {
                workspace_id, branch: self.branch, base_ref: self.base,
            }),
            (None, None, None, None) => None,
            _ => return Err(CliError::usage("Choose exactly one project placement")),
        };
        Ok(LaunchInput { target, brief, cleanup_policy: self.cleanup_policy.into(), label: self.label })
    }
}

pub(super) fn submission_id(id: Option<String>, kind: &str) -> Result<String, CliError> {
    let id = id.unwrap_or_else(|| uuid::Uuid::new_v4().to_string());
    eprintln!("{kind} ID: {id}");
    Ok(id)
}

pub(super) fn check_outcome(input: String) -> Result<cockpit_protocol::orchestration::CheckOutcome, CliError> {
    use cockpit_protocol::orchestration::{CheckOutcome, CheckStatus};
    let input = bounded(input)?;
    let parts: Vec<_> = input.splitn(4, "::").collect();
    if parts.len() < 3 || parts[0].trim().is_empty() || parts[1].trim().is_empty() {
        return Err(CliError::usage("--check requires contract::command::passed|failed|not_run[::limitation]"));
    }
    let outcome = match parts[2] {
        "passed" => CheckStatus::Passed,
        "failed" => CheckStatus::Failed,
        "not_run" => CheckStatus::NotRun,
        _ => return Err(CliError::usage("Check outcome must be passed, failed or not_run")),
    };
    Ok(CheckOutcome { contract: parts[0].into(), command_or_scenario: parts[1].into(),
        outcome, evidence: None, limitation: parts.get(3).map(|text| (*text).into()) })
}

impl WorkerStartArgs {
    pub(super) fn action(self, root_id: String) -> Result<OrchestrationAction, CliError> {
        let launch = self.launch.read()?;
        let task_revision = self.task_revision.ok_or_else(|| CliError::usage("WorkerStart needs an internally resolved task revision"))?;
        let doc_revision = self.doc_revision.ok_or_else(|| CliError::usage("WorkerStart needs an internally resolved document revision"))?;
        let target = launch.target.ok_or_else(|| CliError::usage("worker-start requires explicit project placement; task schedule can choose placement automatically"))?;
        let operation_id = submission_id(self.operation_id, "Operation")?;
        eprintln!("Task revision: {task_revision}\nDocument revision: {doc_revision}");
        Ok(OrchestrationAction::WorkerStart {
            operation_id, root_id, task_id: self.task,
            expected_task_revision: task_revision, expected_doc_revision: doc_revision,
            target, brief: launch.brief, cleanup_policy: launch.cleanup_policy, label: launch.label,
        })
    }
}

#[derive(Debug, Clone, Copy, ValueEnum)]
pub(super) enum RecoveryArg {
    AcceptExistingWorktree,
}

#[derive(Debug, Subcommand)]
pub(super) enum RunCommand {
    /// Reuse/resume the session liaison, or create it.
    Start {
        #[arg(long)]
        operation_id: String,
        /// Optional transport staging Space; the liaison uses its own directory.
        #[arg(long, conflicts_with_all = ["repository", "path", "space_worktree"])]
        space: Option<String>,
        #[arg(long, conflicts_with_all = ["space", "path", "space_worktree"])]
        repository: Option<String>,
        #[arg(long, conflicts_with_all = ["space", "repository", "space_worktree"])]
        path: Option<String>,
        #[arg(long, conflicts_with_all = ["space", "repository", "path"])]
        space_worktree: Option<String>,
        #[arg(long, conflicts_with_all = ["space", "path"])]
        branch: Option<String>,
        #[arg(long, conflicts_with_all = ["space", "path"])]
        base: Option<String>,
        #[arg(long, requires = "repository")]
        checkout_path: Option<String>,
        #[arg(long)]
        label: Option<String>,
        #[arg(long)]
        selected_root_id: Option<String>,
    },
    /// Reconcile attachment state without replacing a live writer.
    Resume {
        run: String,
        #[arg(long)]
        operation_id: String,
        #[arg(long)]
        expected_attachment_epoch: Option<u64>,
        /// Human operator only: confirm the saved supervisor main has stopped (roots that recorded no process identity).
        #[arg(long = "confirm-stopped-attachment", requires = "expected_attachment_epoch")]
        confirm_stopped_attachment: Option<String>,
    },
    /// Explicit low-level registration; ordinary credentialed calls register implicitly.
    Register {
        #[arg(long)]
        operation_id: String,
    },
    /// Register if unbound and read role, conversation, assignment and inbox counts.
    Context,
    /// Read canonical task scope and the short launch bootstrap.
    Brief,
    /// Resolve the root's exact current human question after receiving the actual chat answer.
    Answer {
        #[arg(long)]
        message_id: Option<String>,
        /// Exact unresolved root NeedsInput message ID.
        #[arg(long = "in-reply-to")]
        question: String,
        /// The actual human answer; not a synthetic progress/acceptance receipt.
        #[arg(long)]
        text: String,
    },
    /// Replace this descendant's current attempt; inspect and fence internally.
    WorkerReplace {
        run: String,
        #[arg(long)]
        operation_id: Option<String>,
        #[arg(long)]
        reason: String,
    },
    /// Complete a worker task with actual source commits and supplied check evidence; never closes the liaison.
    Done {
        #[arg(long)]
        message_id: Option<String>,
        #[arg(long)]
        summary: String,
        #[arg(long)]
        failed: bool,
        /// contract::command::passed|failed|not_run[::limitation]; repeat for focused checks.
        #[arg(long = "check")]
        checks: Vec<String>,
        /// Actual output artifacts for a non-code task.
        #[arg(long = "artifact")]
        artifacts: Vec<String>,
        /// Explicit bounded SourceOutput JSON instead of automatic local Git evidence.
        #[arg(long, conflicts_with_all = ["checks", "artifacts"], requires = "message_id")]
        source_output_file: Option<PathBuf>,
    },
    /// Read a saved durable snapshot; not fresh Herdr liveness or runtime-health proof.
    List {
        /// Indent runs by their parent hierarchy.
        #[arg(long)]
        tree: bool,
    },
    /// Show one run, or the run bound to the caller with --self.
    #[command(group(clap::ArgGroup::new("selection").required(true).args(["run", "self_run"])))]
    Show {
        /// Run UUID from run list; flags do not grant control authority.
        #[arg(conflicts_with = "self_run")]
        run: Option<String>,
        /// Select the run authorized by the calling private capability.
        #[arg(long = "self")]
        self_run: bool,
    },
    /// Adapter-only immediate canonical-task start; normally use task create then schedule.
    #[command(after_long_help = WORKER_START_HELP)]
    WorkerStart(WorkerStartArgs),
    /// Accept an explicit successful result against the canonical task revision.
    #[command(
        after_long_help = "Review the explicit successful Result with run show RUN --json and read
task show TASK --json for the current task.task_revision. Only the fresh
bound root may accept its current assigned descendant. Idle/exited, checklist
completion and Result delivery are not acceptance. Inspect state first if an
operation's outcome is uncertain; never repeat it blindly."
    )]
    Accept {
        /// Strict descendant worker with a reviewed successful Result.
        run: String,
        /// Optional exact task revision; resolved when omitted.
        #[arg(long)]
        task_revision: Option<String>,
    },
    /// Send a descendant worker's result back with actionable review feedback.
    SendBack {
        /// Strict descendant worker whose Result needs correction.
        run: String,
        /// Actionable review feedback (at most 16 KiB).
        #[arg(long)]
        text: String,
    },
    /// Close descendant tracking and request cancellation; does not guarantee a stop.
    Cancel {
        /// Strict descendant worker run UUID; a stop is not guaranteed.
        run: String,
    },
    /// Reconcile uncertain workspace setup; existing-worktree recovery needs proven inventory.
    Reconcile {
        /// Strict descendant worker run UUID.
        run: String,
        /// Accept existing worktree setup only after proving its inventory.
        #[arg(long, value_enum)]
        recovery: Option<RecoveryArg>,
    },
    /// Report upward with a required sender-chosen deduplication ID.
    #[command(after_long_help = REPORT_HELP)]
    Report {
        /// Receipt kind; Initialized/Result require bound main authority.
        #[arg(long, value_enum)]
        kind: ReportKindArg,
        /// Stable message ID; generated and printed when omitted. Reuse only exact payloads.
        #[arg(long)]
        message_id: Option<String>,
        /// Truthful evidence or one blocking question (at most 16 KiB).
        #[arg(long)]
        summary: String,
        /// Required for Result; succeeded or failed does not accept the task.
        #[arg(long, value_enum)]
        outcome: Option<OutcomeArg>,
        /// Structured SourceOutput JSON for a Result (at most 16 KiB).
        #[arg(long, requires = "message_id")]
        source_output_file: Option<PathBuf>,
        /// Optional diagnostic plan (at most 16 KiB); never grants work authority.
        #[arg(long, conflicts_with_all = ["plan_file", "stdin"])]
        plan: Option<String>,
        /// UTF-8 diagnostic plan file (at most 16 KiB), alternative to --plan/--stdin.
        #[arg(long, conflicts_with = "stdin")]
        plan_file: Option<PathBuf>,
        /// Read a diagnostic plan from stdin (at most 16 KiB).
        #[arg(long)]
        stdin: bool,
        /// Ancestor delivery recipient; omit for default parent/self delivery.
        #[arg(long)]
        to: Option<String>,
        /// Retained source task revision captured before durable outbox delivery.
        #[arg(long)]
        original_task_revision: Option<String>,
        /// Retained checkout claim ID captured before durable outbox delivery.
        #[arg(long)]
        original_claim_id: Option<String>,
    },
    /// Record reviewed integration evidence for an accepted Result.
    #[command(after_long_help = INTEGRATION_RECORD_HELP)]
    IntegrationRecord {
        /// IntegrationRecord JSON (at most 16 KiB).
        #[arg(long)]
        record_file: PathBuf,
    },
    /// Queue a checkout/build output resource with actual source identity.
    ResourceRequest {
        /// ResourceRequest JSON (at most 16 KiB).
        #[arg(long)]
        request_file: PathBuf,
    },
    /// Release an exact resource holder after its registered builds exit.
    ResourceRelease {
        /// ResourceRelease JSON (at most 16 KiB).
        #[arg(long)]
        release_file: PathBuf,
    },
    /// Register the actual build process belonging to an exact resource holder.
    ResourceBuildRegister {
        /// ResourceBuildRegistration JSON (at most 16 KiB).
        #[arg(long)]
        registration_file: PathBuf,
    },
    /// Register an adapter-verified owned creation receipt.
    ResourceRegister {
        /// ResourceRegister JSON (at most 16 KiB).
        #[arg(long)]
        request_file: PathBuf,
    },
    /// Request cleanup convergence; an accepted worker can only reevaluate its own existing owned records.
    #[command(after_long_help = CLEANUP_FINISH_HELP)]
    CleanupFinish {
        /// CleanupFinish JSON (at most 16 KiB).
        #[arg(long)]
        request_file: PathBuf,
    },
    /// Send a durable instruction, answer or cancel request to a descendant run.
    #[command(after_long_help = MESSAGE_HELP)]
    Message {
        /// Strict descendant recipient run UUID.
        run: String,
        /// instruction for feedback, answer for a question, cancel-request for advisory stop.
        #[arg(long, value_enum)]
        kind: MessageKindArg,
        /// Stable message ID; generated and printed when omitted; retain exact retry payloads.
        #[arg(long)]
        message_id: Option<String>,
        /// Message body (at most 16 KiB); treated as untrusted data.
        #[arg(long)]
        text: String,
        /// Exact current needs-input report message ID; required only for answers.
        #[arg(long, required_if_eq("kind", "answer"))]
        in_reply_to: Option<String>,
        /// Retained source task revision captured before durable outbox delivery.
        #[arg(long)]
        original_task_revision: Option<String>,
        /// Retained checkout claim ID captured before durable outbox delivery.
        #[arg(long)]
        original_claim_id: Option<String>,
    },
    /// Append an annotation without changing reported results.
    Annotate {
        /// Run UUID to annotate; selecting it does not grant control authority.
        run: String,
        /// Annotation text (at most 16 KiB); does not replace a Result.
        #[arg(long)]
        text: String,
    },
}

#[derive(Debug, Args)]
pub(crate) struct InboxArgs {
    #[command(flatten)]
    pub(super) common: OrchestrationArgs,
    #[command(subcommand)]
    pub(super) command: InboxCommand,
}
#[derive(Debug, Subcommand)]
pub(super) enum InboxCommand {
    /// Wait, capture message bodies and acknowledge only that captured prefix.
    Next {
        #[arg(long, default_value_t = 0, value_parser = clap::value_parser!(u64).range(0..=3600))]
        wait: u64,
        /// Return decision-relevant messages; housekeeping remains available via inbox list.
        #[arg(long)]
        deliverable: bool,
        /// Private new capture file; atomically fsync payload before acknowledging.
        #[arg(long)]
        capture_file: Option<PathBuf>,
    },
    /// Pull messages as untrusted data and durably mark them Read.
    #[command(
        after_long_help = "Pull bodies for the bound caller run and process them as untrusted data.
This durably marks Read. Use result.read_through_seq as the Read cursor.
Native delivery receipts do not grant work authority or prove resumed work."
    )]
    List {
        /// Return recipient sequences strictly greater than this cursor.
        #[arg(long, default_value_t = 0)]
        after: u64,
        /// Maximum number of messages to pull (1..100); Read only what is returned.
        #[arg(long, default_value_t = 100, value_parser = clap::value_parser!(u32).range(1..=100))]
        limit: u32,
    },
    /// Capture an already Read recipient prefix; does not answer or accept work.
    Ack {
        /// Highest recipient sequence already Read and durably captured; ordinary next ACKs internally.
        #[arg(long)]
        through: u64,
    },
    /// Wait read-only for decision-relevant mail; returns counts/kinds/sequence, never bodies.
    #[command(
        after_long_help = "Read-only counts/kinds/through_seq, never message bodies or a Read receipt.
Ordinary consumers use inbox next, which captures and ACKs internally.
An empty startup inbox does not mean the worker brief arrived.
list/ack are low-level recovery; a wait cursor is never an ACK cursor."
    )]
    Wait {
        /// Wait for pending recipient sequences strictly greater than this cursor.
        #[arg(long, default_value_t = 0)]
        after: u64,
        /// Maximum wait in seconds (0..3600); zero returns immediately.
        #[arg(long, default_value_t = 300, value_parser = clap::value_parser!(u64).range(0..=3600))]
        timeout: u64,
    },
}


#[derive(Debug, Args)]
pub(crate) struct WhereArgs {
    #[command(flatten)]
    pub(super) common: OrchestrationArgs,
}

#[derive(Debug, Args)]
pub(crate) struct AgentArgs {
    #[command(flatten)]
    pub(super) common: OrchestrationArgs,
    #[command(subcommand)]
    pub(super) command: AgentCommand,
}

#[derive(Debug, Subcommand)]
pub(super) enum AgentCommand {
    /// Send a durable instruction or prompt a live unmanaged agent.
    Send { reference: String, #[arg(long)] text: String },
}

#[derive(Debug, Args)]
pub(crate) struct RouteArgs {
    #[command(flatten)]
    pub(super) common: OrchestrationArgs,
    #[command(subcommand)]
    pub(super) command: RouteCommand,
}
#[derive(Debug, Subcommand)]
pub(super) enum RouteCommand {
    /// Resolve configured routing first, then actual forge-origin matches; never cwd.
    #[command(after_long_help = ROUTE_HELP)]
    Resolve {
        /// Explicit artifact URL to resolve against configured and forge routing.
        #[arg(long)]
        artifact: String,
    },
}

pub(super) enum RunOperation {
    List { tree: bool },
    Show { run: Option<String>, self_run: bool },
    Context,
    Brief,
    Answer { message_id: Option<String>, question: String, text: String },
    WorkerStart(WorkerStartArgs),
    WorkerReplace { run: String, operation_id: Option<String>, reason: String },
    Done { message_id: Option<String>, summary: String, failed: bool, checks: Vec<String>,
        artifacts: Vec<String>, source_output_file: Option<PathBuf> },
    Mutation(OrchestrationAction),
}

impl RunCommand {
    pub(super) fn operation(self, common: &OrchestrationArgs) -> Result<RunOperation, CliError> {
        let action = match self {
            Self::Start { operation_id, space, repository, path, space_worktree,
                branch, base, checkout_path, label, selected_root_id } => {
                let target = match (space, repository, path, space_worktree) {
                    (Some(workspace_id), None, None, None) => Some(DispatchTarget::ExistingSpace { workspace_id }),
                    (None, Some(repository_id), None, None) => Some(DispatchTarget::Setup {
                        request: WorkspaceSetupRequest::Create {
                            repository_id, branch, base_ref: base, checkout_path, label: label.clone(),
                            task_name: None, artifact_url: None, linked_artifact_urls: vec![], focus: false,
                        },
                    }),
                    (None, None, Some(path), None) => Some(DispatchTarget::Setup {
                        request: WorkspaceSetupRequest::Open {
                            path, label: label.clone(), task_name: None, focus: false,
                        },
                    }),
                    (None, None, None, Some(workspace_id)) => Some(DispatchTarget::SpaceWorktree {
                        workspace_id, branch, base_ref: base,
                    }),
                    (None, None, None, None) if branch.is_none() && base.is_none() => None,
                    _ => return Err(CliError::usage("start requires one explicit placement, or no placement options")),
                };
                OrchestrationAction::SupervisorStart {
                    operation_id, target, label, selected_root_id,
                }
            },
            Self::Resume { run, operation_id, expected_attachment_epoch, confirm_stopped_attachment } =>
                OrchestrationAction::RunResume { run_id: run, operation_id, expected_attachment_epoch,
                    confirmed_stopped_attachment_id: confirm_stopped_attachment },
            Self::Register { operation_id } => OrchestrationAction::RunRegister { operation_id },
            Self::Context => return Ok(RunOperation::Context),
            Self::Brief => return Ok(RunOperation::Brief),
            Self::Answer { message_id, question, text } => {
                if question.trim().is_empty() || text.trim().is_empty() {
                    return Err(CliError::usage("run answer requires the exact question ID and actual nonempty human answer"));
                }
                return Ok(RunOperation::Answer { message_id, question, text: bounded(text)? });
            },
            Self::WorkerReplace { run, operation_id, reason } =>
                return Ok(RunOperation::WorkerReplace { run, operation_id, reason: bounded(reason)? }),
            Self::Done { message_id, summary, failed, checks, artifacts, source_output_file } =>
                return Ok(RunOperation::Done { message_id, summary: bounded(summary)?, failed, checks, artifacts, source_output_file }),
            Self::List { tree } => return Ok(RunOperation::List { tree }),
            Self::Show { run, self_run } => return Ok(RunOperation::Show { run, self_run }),
            Self::WorkerStart(args) => return Ok(RunOperation::WorkerStart(args)),
            Self::Accept { run, task_revision } => OrchestrationAction::Accept {
                run_id: run,
                expected_task_revision: revision_fence(task_revision)?,
            },
            Self::SendBack { run, text } => OrchestrationAction::SendBack {
                run_id: run,
                text: bounded(text)?,
            },
            Self::Cancel { run } => OrchestrationAction::CancelRun { run_id: run },
            Self::Reconcile { run, recovery } => OrchestrationAction::ReconcileRun {
                run_id: run,
                recovery: recovery.map(|RecoveryArg::AcceptExistingWorktree| {
                    WorkspaceRecoveryAction::AcceptExistingWorktree
                }),
            },
            Self::Report {
                kind,
                message_id,
                summary,
                outcome,
                source_output_file,
                plan,
                plan_file,
                stdin,
                to,
                original_task_revision,
                original_claim_id,
            } => {
                let message_id = submission_id(message_id, "Message")?;
                let plan = match plan {
                    Some(plan) => Some(bounded(plan)?),
                    None => read_input(plan_file, stdin)?,
                };
                OrchestrationAction::Report {
                    message_id,
                    kind: kind.into(),
                    outcome: outcome.map(Into::into),
                    summary: bounded(summary)?,
                    plan,
                    to_run_id: to,
                    source_output: source_output_file.map(read_json_file).transpose()?,
                    original_task_revision,
                    original_claim_id,
                }
            },
            Self::IntegrationRecord { record_file } => OrchestrationAction::IntegrationRecord {
                record: read_json_file(record_file)?,
            },
            Self::ResourceRequest { request_file } => OrchestrationAction::ResourceRequest {
                request: read_json_file(request_file)?,
            },
            Self::ResourceRelease { release_file } => OrchestrationAction::ResourceRelease {
                release: read_json_file(release_file)?,
            },
            Self::ResourceBuildRegister { registration_file } => OrchestrationAction::ResourceBuildRegister {
                registration: read_json_file(registration_file)?,
            },
            Self::ResourceRegister { request_file } => OrchestrationAction::ResourceRegister {
                request: read_json_file(request_file)?,
            },
            Self::CleanupFinish { request_file } => {
                let request: cockpit_protocol::orchestration::CleanupFinish = read_json_file(request_file)?;
                if common.operator {
                    let root = common.root.as_ref().ok_or_else(|| CliError::usage("operator cleanup requires --root"))?;
                    if *root != request.root_id {
                        return Err(CliError::new("actor_forbidden", "Cleanup request must match the explicit --root"));
                    }
                }
                OrchestrationAction::CleanupFinish { request }
            },
            Self::Message {
                run,
                kind,
                message_id,
                text,
                in_reply_to,
                original_task_revision,
                original_claim_id,
            } => OrchestrationAction::MessageSend {
                message_id: submission_id(message_id, "Message")?,
                to_run_id: run,
                kind: match kind {
                    MessageKindArg::Instruction => MessageKind::Instruction,
                    MessageKindArg::CancelRequest => MessageKind::CancelRequest,
                    MessageKindArg::Answer => MessageKind::Answer,
                },
                text: bounded(text)?,
                in_reply_to,
                original_task_revision,
                original_claim_id,
            },
            Self::Annotate { run, text } => OrchestrationAction::Annotate {
                run_id: run,
                text: bounded(text)?,
            },
        };
        Ok(RunOperation::Mutation(action))
    }

    pub(super) fn preflight(&self, common: &OrchestrationArgs) -> Result<(), CliError> {
        if matches!(self, Self::Register { .. } | Self::Context | Self::Brief
            | Self::Answer { .. } | Self::Done { .. } | Self::Report { .. }
            | Self::Show { self_run: true, .. }
            | Self::ResourceRequest { .. } | Self::ResourceBuildRegister { .. }) {
            super::context::require_job(common)?;
        }
        if let Self::Message {
            kind, in_reply_to, ..
        } = self
        {
            match (kind, in_reply_to.as_deref()) {
                (MessageKindArg::Answer, Some(question)) if !question.trim().is_empty() => {}
                (MessageKindArg::Answer, _) => {
                    return Err(CliError::usage(
                        "Answers require a nonempty --in-reply-to question message ID.",
                    ));
                }
                (_, Some(_)) => {
                    return Err(CliError::usage(
                        "--in-reply-to is only valid for answers; use instruction for nonquestion feedback.",
                    ));
                }
                (_, None) => {}
            }
        }
        Ok(())
    }

    pub(super) fn caller_required(&self) -> bool {
        !matches!(self, Self::Start { .. } | Self::List { .. } | Self::Show { self_run: false, .. })
    }
}
