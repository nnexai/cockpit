use crate::projects::{WorkspaceCheckoutOwnership, WorkspaceRecoveryAction, WorkspaceSetupRequest};
use crate::v1::ErrorResponse;
use serde::{Deserialize, Serialize};
use ts_rs::TS;

// ---------- requests ----------
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[serde(deny_unknown_fields)]
pub struct OrchestrationSnapshotRequest {
    pub session_id: String,
    pub root_id: Option<String>,
}
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[serde(deny_unknown_fields)]
pub struct OrchestrationWaitRequest {
    #[ts(type = "number")]
    pub after_revision: u64,
    pub after_tasks_token: String,
    pub timeout_ms: u32,
} // timeout ≤ 30_000
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
pub struct OrchestrationWaitResponse {
    #[ts(type = "number")]
    pub revision: u64,
    pub tasks_token: String,
    pub changed: bool,
}
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[serde(deny_unknown_fields)]
pub struct OrchestrationMutationRequest {
    pub session_id: String,
    #[ts(type = "number | null")]
    pub expected_revision: Option<u64>, // GUI sends it for every operator action; CLI omits
    pub action: OrchestrationAction,
}
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
pub struct OrchestrationMutationResponse {
    #[ts(type = "number")]
    pub revision: u64,
    pub result: OrchestrationActionResult,
}

// ---------- snapshot ----------
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
pub struct OrchestrationSnapshot {
    pub session_id: String,
    #[ts(type = "number")]
    pub revision: u64,
    pub tasks_token: String, // hash of (name, len, mtime, inode) of tasks/*.md; wait invalidation
    pub roots: Vec<RootSummary>, // root runs whose session_id == request.session_id
    pub board: Option<TaskBoard>, // for request.root_id, else the only root, else None
    pub runs: Vec<Run>,      // every run under roots of this session
    pub messages: Vec<Message>, // for those runs; bodies included (≤16 KiB each)
    pub questions: Vec<QuestionStatus>,
    pub subagents: Vec<Subagent>,
    pub intents: Vec<TaskIntent>,
    pub assignment_intents: Vec<TaskAssignmentIntent>,
    pub runtime: RuntimeObservation,
    pub unmanaged_agents: Vec<UnmanagedAgent>,
    pub attention: Vec<Attention>, // Activity "Needs you", derived
}
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
pub struct UnmanagedAgent {
    pub workspace_id: String,
    pub workspace_label: String,
    pub tab_id: String,
    pub tab_label: String,
    pub pane_id: String,
    pub agent_name: String,
    pub agent_status: Option<String>,
    pub state_changed_at: Option<String>,
}
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
pub struct RootSummary {
    pub root_id: String,
    pub label: String,
    pub kind: RunKind,
    pub open_runs: u32,
    pub needs_you: u32,
}
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
pub struct TaskBoard {
    pub root_id: String,
    pub path: String,
    pub doc_revision: String,
    pub unidentified_items: u32,
    pub diagnostics: Vec<ErrorResponse>,
    pub tasks: Vec<TaskView>,
}
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
pub struct Task {
    pub task_id: String,
    pub title: String,
    pub body: String,
    pub description: String,
    pub description_editable: bool,
    pub description_diagnostic: Option<String>,
    pub steps: Vec<TaskStep>,
    pub step_progress: Option<TaskStepProgress>,
    pub steps_diagnostic: Option<String>,
    pub depends_on: Vec<String>,
    pub follow_up_of: Option<String>,
    pub relations_diagnostic: Option<String>,
    pub checked: bool,
    pub line: u32,
    pub task_revision: String,
    pub diagnostic: Option<String>,
}
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
pub struct TaskStep {
    pub step_id: Option<String>,
    pub parent_step_id: Option<String>,
    pub depth: u32,
    pub title: String,
    pub checked: bool,
    pub status: TaskStepStatus,
    pub line: u32,
    pub source_offset: u32,
    pub diagnostic: Option<String>,
}
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
pub struct TaskStepProgress {
    pub done: u32,
    pub total: u32,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
pub enum TaskStepStatus {
    Open,
    Partial,
    Done,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
pub enum TaskStepScope {
    Leaf,
    Subtree,
}
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
pub struct TaskDependencies {
    pub state: TaskDependencyState,
    pub unmet: Vec<TaskDependencyBlocker>,
    pub problems: Vec<ErrorResponse>,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
pub enum TaskDependencyState {
    None,
    Satisfied,
    Blocked,
    Invalid,
}
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
pub struct TaskDependencyBlocker {
    pub task_id: String,
    pub reason: TaskDependencyBlockerReason,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
pub enum TaskDependencyBlockerReason {
    Unchecked,
    Missing,
    Ambiguous,
}
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
pub struct TaskView {
    pub task: Task,
    pub lane: TaskLane,
    pub current_run_id: Option<String>,
    pub dependencies: TaskDependencies,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
pub enum TaskLane {
    Queued,
    Setup,
    Ready,
    Working,
    Review,
    Accepted,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
pub enum RunKind {
    Supervisor,
    Adopted,
    Worker,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
pub enum RunStage {
    Proposed,        // worker: setup plan being computed
    AwaitingPrepare, // plan ready; needs GrantPrepare
    Preparing,       // granted (or supervisor start); dispatcher running setup/launch
    Initializing,    // launched; prepare brief in inbox; waiting for Ready report
    Ready,           // init receipt filed (work plan); needs GrantExecute
    Working,         // execute granted; work brief in inbox
    Reported,        // result report received; needs Accept or SendBack
    Active,          // supervisor/adopted root launched or bound
    Closed,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
pub enum CloseReason {
    Accepted,
    Cancelled,
    Superseded,
    Failed,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
pub enum DispatchStep {
    Planning,
    PlanFailed,
    SetupPending,
    SetupRunning,
    SetupUnknown,
    LaunchIntent,
    LaunchPending,
    LaunchUnknown,
    Launched,
    NeedsReview,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[serde(tag = "target", rename_all = "snake_case")]
#[ts(tag = "target", rename_all = "snake_case")]
pub enum DispatchTarget {
    // tag = "target"
    Setup { request: WorkspaceSetupRequest }, // existing Create/Open; `focus` forced false
    ExistingSpace { workspace_id: String },   // launch in an existing Space; no setup
    /// Owned linked worktree from an explicit project Space's configured repository.
    SpaceWorktree {
        workspace_id: String,
        branch: Option<String>,
        base_ref: Option<String>,
    },
}
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
pub struct SetupSummary {
    // what the user authorizes
    pub operation_id: Option<String>,
    pub generation: Option<u32>,
    pub workspace_id: Option<String>,
    pub checkout_path: String,
    pub repository_id: Option<String>,
    pub branch: Option<String>,
    pub base: Option<String>,
    pub ownership: Option<WorkspaceCheckoutOwnership>,
    pub effects: Vec<String>,
    pub warnings: Vec<String>,
    /// Source project Space for SpaceWorktree; absent for other targets.
    pub project_workspace_id: Option<String>,
}
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
pub struct PlanRecord {
    pub plan_revision: String,
    pub text: String,
    pub created_at: String,
} // revision = sha256 of canonical JSON
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
pub struct Grant {
    pub grant_id: String,
    pub scope: GrantScope,
    pub plan_revision: String,
    pub origin: GrantOrigin,
    pub supervisor_run_id: Option<String>,
    pub omp_session_id: Option<String>,
    pub granted_at: String,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
pub enum GrantScope {
    Prepare,
    Execute,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
pub enum GrantOrigin {
    Browser,
    Native,
    Supervisor,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
pub enum OperatorOrigin {
    Browser,
    Native,
}
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
pub struct RunLocation {
    pub boot_id: Option<String>,
    pub terminal_id: Option<String>,
    pub native_session_id: Option<String>, // launch receipt, not live truth
    pub endpoint_identity: String,
    pub session_id: String,
    pub workspace_id: String,
    pub tab_id: String,
    pub pane_id: String,
    pub launch_tag: String,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
pub struct NativeProcessIdentity {
    pub pid: u32,
    #[ts(type = "number")]
    pub start_ticks: u64,
    pub kernel_boot_id: Option<String>,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
pub struct NativeShellIdentity {
    pub process: NativeProcessIdentity,
    pub executable_device: String,
    pub executable_inode: String,
    pub argv_digest: String,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
pub enum RetirementTrigger {
    Accept,
    AcceptRecovery,
    OperatorConflictResolution,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
pub struct RetirementIdentity {
    pub run_attempt: u32,
    pub launch_attempt: u32,
    pub launch_tag: String,
    pub endpoint_identity: String,
    pub session_id: String,
    pub workspace_id: String,
    pub tab_id: String,
    pub pane_id: String,
    pub terminal_id: String,
    pub herdr_boot_id: Option<String>,
    pub omp_session_id: String,
    pub process: NativeProcessIdentity,
    pub shell: NativeShellIdentity,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
pub struct RunRetirement {
    pub retirement_id: String,
    pub trigger: RetirementTrigger,
    pub result_message_id: String,
    pub task_revision: String,
    pub identity: Option<RetirementIdentity>,
    pub state: RetirementState,
    pub created_at: String,
    pub updated_at: String,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(tag = "state", rename_all = "snake_case")]
#[ts(tag = "state", rename_all = "snake_case")]
pub enum RetirementState {
    Waiting {
        blockers: Vec<RetirementBlocker>,
    },
    NativeStopOffered {
        offered_at: String,
    },
    NativeStopDeferred {
        offered_at: String,
        reason: NativeDeferReason,
        at: String,
    },
    NativeStopRequested {
        at: String,
    },
    NativeStopped {
        at: String,
        evidence: NativeStopEvidence,
    },
    CloseIntent {
        at: String,
    },
    Retired {
        at: String,
        terminal: TerminalOutcome,
    },
    Retained {
        at: String,
        reason: RetainReason,
        native_stopped: bool,
    },
    Unknown {
        at: String,
        phase: RetirementPhase,
        detail: String,
    },
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
pub enum RetirementBlocker {
    OpenDescendantRuns,
    RunningSubagents,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
pub enum NativeDeferReason {
    Busy,
    PendingMessages,
    AsyncJobs,
    LiveSubagents,
    EditorDraft,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
pub enum NativeStopEvidence {
    ExitedAfterShutdownRequest,
    AlreadyExited,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
pub enum TerminalOutcome {
    ClosedByCockpit,
    AlreadyAbsent,
    AbsentAfterUncertainClose,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
pub enum RetainReason {
    IdentityIncomplete,
    IdentityChanged,
    EndpointChanged,
    NativeProcessUnverifiable,
    ProcessPaneMismatch,
    WorkerUnresponsive,
    WorkerBusyTimeout,
    UserActivity,
    NativeRefused,
    SharedTab,
    TabRenamed,
    PaneMoved,
    ForegroundProcess,
    ObservationUnavailable,
    HerdrRefused,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
pub enum RetirementPhase {
    NativeStop,
    TerminalClose,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
pub enum NativeRefuseReason {
    UserActivity,
    NativeRefused,
}
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[serde(tag = "outcome", rename_all = "snake_case")]
#[ts(tag = "outcome", rename_all = "snake_case")]
pub enum NativeStopReceipt {
    ShutdownRequested,
    Deferred {
        reason: NativeDeferReason,
    },
    Refused {
        reason: NativeRefuseReason,
        text: String,
    },
}
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
pub struct Run {
    pub session_id: String,
    pub prepare_brief: String,
    pub run_id: String,
    pub kind: RunKind,
    pub label: String,
    pub root_id: String,
    pub parent_run_id: Option<String>,
    pub task_id: Option<String>,
    pub attempt: u32,
    pub task_revision_at_propose: Option<String>,
    pub stage: RunStage,
    pub close_reason: Option<CloseReason>,
    pub dispatch: Option<DispatchState>,
    pub target: Option<DispatchTarget>,
    pub setup: Option<SetupSummary>,
    pub prepare_plan: Option<PlanRecord>, // setup summary + prepare brief; GrantPrepare binds here
    pub init_receipt: Option<Report>,     // the Ready report; preserved after execute
    pub work_plan: Option<PlanRecord>,    // from the Ready report; GrantExecute binds here
    pub grants: Vec<Grant>,
    pub last_report: Option<Report>,
    pub result: Option<Report>,
    pub annotations: Vec<Annotation>,
    pub location: Option<RunLocation>,
    pub bound_omp_session: Option<String>, // set by RunBindSession; cleared by RetryLaunch
    pub bound_omp_process: Option<NativeProcessIdentity>, // trusted bind evidence; cleared by RetryLaunch
    pub launch_shell_identity: Option<NativeShellIdentity>, // captured before launch; cleared by RetryLaunch
    pub retirement: Option<RunRetirement>, // created with accepted closure; never backfilled
    pub supersedes_run_id: Option<String>, // from RunPropose; applied on GrantPrepare
    pub created_at: String,
    pub updated_at: String,
}
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
pub struct DispatchState {
    pub launch_tag: Option<String>,
    pub endpoint_identity: Option<String>,
    pub recovery: Option<WorkspaceRecoveryAction>,
    pub agent_started: bool,
    pub step: DispatchStep,
    pub launch_attempt: u32,
    pub error: Option<ErrorResponse>,
    pub updated_at: String,
}
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
pub struct Annotation {
    pub by: ActorRef,
    pub text: String,
    pub at: String,
}
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[serde(tag = "type", rename_all = "snake_case")]
#[ts(tag = "type", rename_all = "snake_case")]
pub enum ActorRef {
    Operator,
    Run { run_id: String },
    Dispatcher,
} // tag = "type"

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
pub enum ReportKind {
    Progress,
    Ready,
    Result,
    NeedsInput,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
pub enum ReportOutcome {
    Succeeded,
    Failed,
}
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
pub struct Report {
    pub message_id: String,
    pub kind: ReportKind,
    pub outcome: Option<ReportOutcome>,
    pub summary: String,
    pub plan: Option<String>,
    pub at: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
pub enum MessageKind {
    PrepareBrief,
    WorkBrief,
    SupervisorBrief,
    Instruction,
    Answer,
    CancelRequest,
    SubagentControl,
    Report,
    Observation,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
pub enum DeliveryStage {
    Stored,
    Woken,
    Read,
    Acked,
}
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
pub struct Message {
    pub message_id: String, // sender-chosen idempotency key (uuid)
    pub to_run_id: String,
    #[ts(type = "number")]
    pub seq: u64, // seq is per recipient, monotonic
    pub from: ActorRef,
    pub kind: MessageKind,
    pub text: String, // ≤ 16 KiB UTF-8
    pub in_reply_to: Option<String>,
    pub report: Option<Report>, // kind == Report
    pub stale: bool,            // sender attempt superseded / caller mismatch evidence
    pub escalated_from: Option<String>,
    pub from_subagent_id: Option<String>, // report sent from an OMP subagent context of the sending run
    pub stage: DeliveryStage,
    pub woken_omp_session: Option<String>,
    pub created_at: String,
    pub acked_at: Option<String>,
}
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
pub struct QuestionStatus {
    pub run_id: String,
    pub question_message_id: String,
    pub asked_at: String,
    pub receipt: QuestionReceipt,
}
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[serde(tag = "status", rename_all = "snake_case")]
#[ts(tag = "status", rename_all = "snake_case")]
pub enum QuestionReceipt {
    Unresolved,
    AnswerDelivered { answer: AnswerReceipt },
    AnswerAcknowledged { answer: AnswerReceipt },
}
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
pub struct AnswerReceipt {
    pub sender: ActorRef,
    pub message_id: String,
    #[ts(type = "number")]
    pub seq: u64,
    pub stage: DeliveryStage,
    pub created_at: String,
    pub acked_at: Option<String>,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
pub enum SubagentStatus {
    Running,
    Done,
    Failed,
    Cancelled,
}
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
pub struct Subagent {
    pub run_id: String,
    pub subagent_id: String,                // subagent_id = OMP ctx.agent.id
    pub parent_subagent_id: Option<String>, // OMP ctx.agent.parentId; None = child of the run's main session
    pub bound_omp_session: Option<String>,
    pub role: Option<String>,
    pub label: String,
    pub status: SubagentStatus,
    pub summary: Option<String>,
    pub last_control: Option<SubagentControlState>,
    pub updated_at: String,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
pub enum ControlStage {
    Stored,
    Applied,
    Failed,
}
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
pub struct SubagentControlState {
    #[ts(type = "number")]
    pub seq: u64,
    pub op: SubagentOp,
    pub stage: ControlStage,
    pub error: Option<String>,
    pub at: String,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
pub enum AgentKind {
    Main,
    Subagent,
} // from OMP ctx.agent.kind, supplied by the extension

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
pub enum IntentState {
    Pending,
    Conflict,
}
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
pub struct TaskIntent {
    pub intent_id: String,
    pub root_id: String,
    pub task_id: String,
    pub run_id: String,
    pub expected_task_revision: String,
    pub state: IntentState,
    pub origin: Option<GrantOrigin>,
    pub supervisor_run_id: Option<String>,
    pub omp_session_id: Option<String>,
    pub result_message_id: Option<String>,
}
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
pub struct TaskAssignmentIntent {
    pub root_id: String,
    pub task_id: String,
    pub state: IntentState,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
pub enum Presence {
    Present,
    Missing,
    EndpointChanged,
    Unobserved,
}
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
pub struct RunObservation {
    pub run_id: String,
    pub presence: Presence,
    pub actual_omp: bool,
    pub workspace_id: Option<String>,
    pub workspace_label: Option<String>,
    pub tab_id: Option<String>,
    pub tab_label: Option<String>,
    pub pane_id: Option<String>, // fresh pane identity; launch receipts may be stale after a move
    pub agent_status: Option<String>, // Herdr AgentStatus string verbatim
    pub state_changed_at: Option<String>,
}
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[serde(tag = "status", rename_all = "snake_case")]
#[ts(tag = "status", rename_all = "snake_case")]
pub enum RuntimeObservation {
    // tag = "status"
    Fresh {
        endpoint_identity: String,
        observed_at: String,
        runs: Vec<RunObservation>,
    },
    Unavailable {
        error: ErrorResponse,
    },
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
pub enum AttentionKind {
    AwaitsPrepare,
    AwaitsExecute,
    PlanChanged,
    ToAccept,
    NeedsInput,
    RuntimeBlocked,
    BriefUnread,
    IdleWithoutReport,
    ExitedWithoutReport,
    DispatchUnknown,
    IntentConflict,
    RetirementUnconfirmed,
}
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
pub struct Attention {
    pub kind: AttentionKind,
    pub run_id: Option<String>,
    pub task_id: Option<String>,
    #[ts(type = "number | null")]
    pub message_seq: Option<u64>,
    pub since: String,
}

// ---------- actions ----------
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[serde(tag = "action", rename_all = "snake_case")]
#[ts(tag = "action", rename_all = "snake_case")]
#[serde(deny_unknown_fields)]
pub enum OrchestrationAction {
    // tag = "action"
    // tasks: operator or agent
    TaskCreate {
        root_id: String,
        task_id: String,
        title: String,
        description: String,
        depends_on: Vec<String>,
        follow_up_of: Option<String>,
        expected_doc_revision: Option<String>,
        source_revision: Option<String>,
    }, // ordinary independent creation: own-root agent; relationships: root main or operator
    TaskAssign {
        root_id: String,
        task_id: String,
        title: String,
        description: String,
    }, // operator; creates the canonical task and assigns it to the selected root atomically
    TaskAssignmentResolve {
        root_id: String,
        task_id: String,
        expected_task_revision: Option<String>,
        assign: bool,
    }, // operator; resolves only an existing conflicted assignment intent
    TaskUpdate {
        root_id: String,
        task_id: String,
        expected_task_revision: String,
        title: Option<String>,
        description: Option<String>,
    }, // task content authority; raw canonical body is read-only
    TaskDependenciesSet {
        root_id: String,
        task_id: String,
        expected_task_revision: String,
        expected_doc_revision: String,
        depends_on: Vec<String>,
    }, // root main or operator; live attempts permit remove-only dependency edits
    TaskStepAdd {
        root_id: String,
        task_id: String,
        expected_task_revision: String,
        step_id: String,
        parent_step_id: Option<String>,
        before_step_id: Option<String>,
        title: String,
    },
    TaskStepRename {
        root_id: String,
        task_id: String,
        expected_task_revision: String,
        step_id: String,
        title: String,
    },
    TaskStepSetChecked {
        root_id: String,
        task_id: String,
        expected_task_revision: String,
        step_id: String,
        checked: bool,
        scope: TaskStepScope,
    },
    TaskStepMove {
        root_id: String,
        task_id: String,
        expected_task_revision: String,
        step_id: String,
        parent_step_id: Option<String>,
        before_step_id: Option<String>,
    },
    TaskStepRemove {
        root_id: String,
        task_id: String,
        expected_task_revision: String,
        step_id: String,
    },
    TasksAssignIds {
        root_id: String,
        expected_doc_revision: String,
    },
    // roots
    SupervisorStart {
        target: Option<DispatchTarget>,
        label: Option<String>,
    }, // operator; None = Open `<state_root>/orchestration/supervisors/<root_id>/`
    RunBindSession {
        omp_session_id: String,
    }, // main-session extension at session_start
    RetirementNativeReceipt {
        retirement_id: String,
        outcome: NativeStopReceipt,
    }, // bound main session of the accepted retiring worker only
    RunAdopt {
        label: String,
    }, // unbound agent: binds caller pane as Adopted root
    // worker lifecycle
    RunPropose {
        task_id: String,
        parent_run_id: Option<String>,
        label: Option<String>,
        target: DispatchTarget,
        prepare_brief: String,
        supersedes_run_id: Option<String>,
    }, // agent (default parent = caller) or operator
    GrantPrepare {
        run_id: String,
        plan_revision: String,
    }, // operator or current bound top-main supervisor root targeting a strict-descendant Worker
    GrantExecute {
        run_id: String,
        plan_revision: String,
        note: Option<String>,
    }, // operator or current bound top-main supervisor root targeting a strict-descendant Worker
    Accept {
        run_id: String,
        expected_task_revision: String,
    }, // operator or current bound top-main supervisor root targeting a strict-descendant Worker
    SendBack {
        run_id: String,
        text: String,
    }, // operator or current bound top-main supervisor root targeting a strict-descendant Worker
    CancelRun {
        run_id: String,
    }, // operator or current bound top-main supervisor root targeting a strict-descendant Worker;
    // closes tracking and requests stop, not process termination
    RetryLaunch {
        run_id: String,
    }, // operator or current bound top-main supervisor root targeting a strict-descendant Worker;
    // requires fresh absence review for supervisors; new tab, launch_attempt + 1
    ReconcileRun {
        run_id: String,
        recovery: Option<WorkspaceRecoveryAction>,
    }, // operator or current bound top-main supervisor root targeting a strict-descendant Worker;
    // supervisors may accept a proven existing worktree, never retry an uncertain environment
    IntentResolve {
        intent_id: String,
        apply: bool,
    }, // operator
    // messaging: agent (and operator for Instruction/Answer/CancelRequest)
    Report {
        message_id: String,
        kind: ReportKind,
        outcome: Option<ReportOutcome>,
        summary: String,
        plan: Option<String>,
        to_run_id: Option<String>,
    },
    MessageSend {
        message_id: String,
        to_run_id: String,
        kind: MessageKind,
        text: String,
        #[serde(default)]
        in_reply_to: Option<String>,
    },
    Annotate {
        run_id: String,
        text: String,
    },
    InboxPull {
        #[ts(type = "number")]
        after_seq: u64,
        limit: u32,
    }, // agent: returns messages, marks them Read
    InboxWoken {
        #[ts(type = "number")]
        through_seq: u64,
        omp_session_id: String,
    }, // agent extension
    InboxAck {
        #[ts(type = "number")]
        through_seq: u64,
    }, // agent
    SubagentUpdate {
        subagent_id: String,
        parent_subagent_id: Option<String>,
        role: Option<String>,
        label: String,
        status: SubagentStatus,
        summary: Option<String>,
    }, // agent extension
    SubagentControl {
        run_id: String,
        subagent_id: String,
        op: SubagentOp,
    }, // operator or ancestor run
    SubagentControlDone {
        #[ts(type = "number")]
        seq: u64,
        applied: bool,
        error: Option<String>,
    }, // extension that executed the control
}
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[serde(tag = "op", rename_all = "snake_case")]
#[ts(tag = "op", rename_all = "snake_case")]
pub enum SubagentOp {
    Cancel,
    Send { text: String },
} // tag = "op"

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[serde(tag = "result", rename_all = "snake_case")]
#[ts(tag = "result", rename_all = "snake_case")]
pub enum OrchestrationActionResult {
    // tag = "result"
    Task {
        task: Task,
    },
    TaskAssigned {
        task: Task,
        to_run_id: String,
        #[ts(type = "number")]
        seq: u64,
        duplicate: bool,
    },
    TaskIds {
        assigned: u32,
        doc_revision: String,
    },
    Run {
        run_id: String,
        attempt: u32,
    },
    Message {
        to_run_id: String,
        #[ts(type = "number")]
        seq: u64,
        duplicate: bool,
        stale: bool,
    },
    Inbox {
        messages: Vec<Message>,
        #[ts(type = "number")]
        read_through_seq: u64,
    },
    Done,
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::{Value, json};

    #[test]
    fn project_space_worktree_target_preserves_explicit_source_and_branch() {
        let value = json!({
            "target": "space_worktree", "workspace_id": "project",
            "branch": "feature", "base_ref": "main",
        });
        let target: DispatchTarget = serde_json::from_value(value.clone()).unwrap();
        assert!(matches!(&target, DispatchTarget::SpaceWorktree {
            workspace_id, branch: Some(branch), base_ref: Some(base),
        } if workspace_id == "project" && branch == "feature" && base == "main"));
        assert_eq!(serde_json::to_value(target).unwrap(), value);
    }

    #[test]
    fn native_stop_evidence_round_trips_without_receipt_inference() {
        for evidence in ["exited_after_shutdown_request", "already_exited"] {
            let value = json!({
                "state": "native_stopped", "at": "2026-10-05T12:00:00Z", "evidence": evidence,
            });
            let state: RetirementState = serde_json::from_value(value.clone()).unwrap();
            assert_eq!(serde_json::to_value(state).unwrap(), value);
        }
        assert!(
            serde_json::from_value::<RetirementState>(json!({
                "state": "native_stopped", "at": "2026-10-05T12:00:00Z",
                "evidence": "shutdown_requested",
            }))
            .is_err()
        );
    }

    #[test]
    fn native_refusal_requires_a_typed_reason_separate_from_text() {
        let value = json!({
            "action": "retirement_native_receipt",
            "retirement_id": "7b613f19-4a52-41fa-8864-a880cd69ef50",
            "outcome": {
                "outcome": "refused", "reason": "native_refused",
                "text": "user_activity is diagnostic text only",
            },
        });
        let action: OrchestrationAction = serde_json::from_value(value.clone()).unwrap();
        assert!(matches!(
            &action,
            OrchestrationAction::RetirementNativeReceipt {
                outcome: NativeStopReceipt::Refused {
                    reason: NativeRefuseReason::NativeRefused,
                    ..
                },
                ..
            }
        ));
        assert_eq!(serde_json::to_value(action).unwrap(), value);
        let mut missing = value.clone();
        missing["outcome"].as_object_mut().unwrap().remove("reason");
        assert!(serde_json::from_value::<OrchestrationAction>(missing).is_err());
        let mut unknown = value;
        unknown["outcome"]["reason"] = json!("unknown");
        assert!(serde_json::from_value::<OrchestrationAction>(unknown).is_err());
    }

    #[test]
    fn message_send_preserves_optional_instruction_link_and_explicit_answer_link() {
        let instruction = json!({
            "action": "message_send", "message_id": "instruction", "to_run_id": "worker",
            "kind": "instruction", "text": "Proceed",
        });
        let action: OrchestrationAction = serde_json::from_value(instruction.clone()).unwrap();
        assert!(matches!(action, OrchestrationAction::MessageSend { in_reply_to: None, .. }));
        let mut linked = instruction;
        linked["message_id"] = json!("answer");
        linked["kind"] = json!("answer");
        linked["in_reply_to"] = json!("question");
        let action: OrchestrationAction = serde_json::from_value(linked.clone()).unwrap();
        assert!(matches!(
            &action,
            OrchestrationAction::MessageSend { in_reply_to: Some(question), .. }
                if question == "question"
        ));
        assert_eq!(serde_json::to_value(action).unwrap(), linked);
    }

    #[test]
    fn question_receipts_round_trip_without_inventing_acknowledgment() {
        for (status, answer) in [
            ("unresolved", None),
            ("answer_delivered", Some(("stored", Value::Null))),
            ("answer_acknowledged", Some(("acked", json!("2026-10-08T12:01:00Z")))),
        ] {
            let mut receipt = json!({ "status": status });
            if let Some((stage, acked_at)) = answer {
                receipt["answer"] = json!({
                    "sender": { "type": "run", "run_id": "root" },
                    "message_id": "answer", "seq": 7, "stage": stage,
                    "created_at": "2026-10-08T12:00:00Z", "acked_at": acked_at,
                });
            }
            let value = json!({
                "run_id": "worker", "question_message_id": "question",
                "asked_at": "2026-10-08T11:00:00Z", "receipt": receipt,
            });
            let question: QuestionStatus = serde_json::from_value(value.clone()).unwrap();
            assert_eq!(serde_json::to_value(question).unwrap(), value);
        }
    }

    #[test]
    fn supervisor_grant_provenance_round_trips_without_operator_promotion() {
        let value = json!({
            "grant_id": "supervisor-grant", "scope": "prepare", "plan_revision": "d".repeat(64),
            "origin": "supervisor", "supervisor_run_id": "root", "omp_session_id": "actual-omp",
            "granted_at": "2026-10-05T12:00:00Z",
        });
        let grant: Grant = serde_json::from_value(value.clone()).unwrap();
        assert_eq!(grant.origin, GrantOrigin::Supervisor);
        assert_eq!(serde_json::to_value(grant).unwrap(), value);
        assert!(serde_json::from_value::<OperatorOrigin>(json!("supervisor")).is_err());
    }
}
