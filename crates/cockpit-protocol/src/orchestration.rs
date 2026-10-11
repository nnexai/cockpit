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
    /// Opaque canonical durable-store identity, independent of native sessions.
    pub state_store_id: String,
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
    pub cleanup: Vec<RunCleanupSummary>,
    pub restoration: RestorationSummary,
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
    Starting,
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
    Queued,
    Starting,
    Working,
    Reported,
    Active,
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
    // Typed setup intent and actual creation/ownership receipts.
    pub operation_id: Option<String>,
    pub generation: Option<u32>,
    pub workspace_id: Option<String>,
    pub checkout_path: String,
    pub repository_id: Option<String>,
    pub branch: Option<String>,
    pub base: Option<String>,
    /// Actual checkout HEAD observed before managed work starts.
    #[serde(default)]
    pub base_commit: Option<String>,
    pub ownership: Option<WorkspaceCheckoutOwnership>,
    pub effects: Vec<String>,
    pub warnings: Vec<String>,
    /// Source project Space for SpaceWorktree; absent for other targets.
    pub project_workspace_id: Option<String>,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
pub enum DecisionOrigin {
    Browser,
    Native,
    Supervisor,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
pub enum WorkerCleanupPolicy {
    OwnedWhenSafe,
}
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[serde(deny_unknown_fields)]
pub struct WorkerStartResult {
    pub operation_id: String,
    pub run_id: String,
    pub attempt: u32,
    pub duplicate: bool,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
pub enum TaskScopeUpdateState {
    Applied,
    Conflict,
    Pending,
}
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[serde(deny_unknown_fields)]
pub struct TaskScopeUpdateResult {
    pub operation_id: String,
    pub state: TaskScopeUpdateState,
    pub task_revision: Option<String>,
    pub doc_revision: Option<String>,
    pub instruction_message_id: Option<String>,
    pub duplicate: bool,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
pub enum OperatorOrigin {
    Browser,
    Native,
}
/// Host-derived identity; aliases and display labels do not determine the key.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(deny_unknown_fields)]
pub struct ProjectIdentity {
    pub key: String,
    pub machine_id: String,
    pub source: ProjectSource,
    pub repository_id: Option<String>,
    pub source_workspace_id: Option<String>,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
#[ts(tag = "kind", rename_all = "snake_case")]
pub enum ProjectSource {
    Git { common_dir: String },
    Directory { root: String },
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(deny_unknown_fields)]
pub struct DirectoryIdentity {
    pub device: String,
    pub inode: String,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(deny_unknown_fields)]
pub struct CheckoutGitIdentity {
    pub common_dir: String,
    pub worktree_git_dir: String,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(deny_unknown_fields)]
pub struct CheckoutIdentity {
    pub key: String,
    pub machine_id: String,
    pub path: String,
    pub directory_identity: DirectoryIdentity,
    pub git: Option<CheckoutGitIdentity>,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
pub enum AttachmentState {
    Reserved,
    Connecting,
    Connected,
    Disconnected,
    Quiescing,
    Revoked,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
pub enum AttachmentLaunchMode {
    DirectProcess,
    ShellAgent,
}
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[serde(deny_unknown_fields)]
pub struct ExecutionAttachment {
    pub attachment_id: String,
    #[ts(type = "number")]
    pub epoch: u64,
    pub state: AttachmentState,
    pub launch_mode: Option<AttachmentLaunchMode>,
    pub location: Option<RunLocation>,
    pub main_session_id: Option<String>,
    pub process: Option<NativeProcessIdentity>,
    pub shell: Option<NativeShellIdentity>,
    pub machine_id: Option<String>,
    pub launch_attempt: u32,
    pub updated_at: String,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
pub enum ResumeMode {
    ReattachOrRestart,
    ReplaceExisting,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
pub enum ResumePhase {
    Reconciling,
    Reattached,
    Launching,
    Connected,
    Blocked,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
pub enum ResumeCauseClass {
    Recover,
    Decision,
}
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[serde(deny_unknown_fields)]
pub struct ResumeCause {
    pub code: String,
    pub message: String,
    pub class: ResumeCauseClass,
}
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[serde(deny_unknown_fields)]
pub struct ResumeRecord {
    pub operation_id: String,
    pub run_id: String,
    #[ts(type = "number | null")]
    pub previous_attachment_epoch: Option<u64>,
    pub phase: ResumePhase,
    pub cause: Option<ResumeCause>,
    pub decision_id: Option<String>,
    #[ts(type = "number | null")]
    pub replacement_attachment_epoch: Option<u64>,
    pub updated_at: String,
}
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[serde(deny_unknown_fields)]
pub struct ProjectRootLookupRequest {
    pub session_id: String,
    pub workspace_id: String,
    pub selected_root_id: Option<String>,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
pub enum RootConnection {
    Connected,
    Reconnecting,
    Disconnected,
    Unknown,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
pub enum RootResumeAvailability {
    Available,
    InProgress,
    Blocked,
}
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
pub struct ProjectRootCandidate {
    pub root_id: String,
    pub connection: RootConnection,
    pub resume: RootResumeAvailability,
    pub cause: Option<ResumeCause>,
    pub updated_at: String,
}
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[serde(tag = "status", rename_all = "snake_case")]
#[ts(tag = "status", rename_all = "snake_case")]
pub enum ProjectRootLookupResponse {
    Resolved {
        project: ProjectIdentity,
        #[ts(type = "number")]
        lookup_revision: u64,
        preferred_root_id: Option<String>,
        roots: Vec<ProjectRootCandidate>,
    },
    Unavailable { code: String, message: String },
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
pub enum SupervisorStartMode {
    ReuseOrResume,
    Another,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
pub enum SupervisorStartOutcome {
    Reused,
    Resuming,
    Created,
}
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
pub struct SupervisorStartResult {
    pub root_id: String,
    pub outcome: SupervisorStartOutcome,
    pub operation_id: String,
    pub resume_operation_id: Option<String>,
    pub duplicate: bool,
}
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
pub struct RunResumeResult {
    pub run_id: String,
    pub resume: ResumeRecord,
    pub duplicate: bool,
}
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
pub struct RunAttachResult {
    pub run_id: String,
    pub attachment: ExecutionAttachment,
    pub duplicate: bool,
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
#[serde(deny_unknown_fields)]
pub struct AttachmentReference {
    pub run_id: String,
    pub attachment_id: String,
    #[ts(type = "number")]
    pub attachment_epoch: u64,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
pub enum CheckStatus { Passed, Failed, NotRun }
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(deny_unknown_fields)]
pub struct CheckOutcome {
    pub contract: String,
    pub command_or_scenario: String,
    pub outcome: CheckStatus,
    pub evidence: Option<String>,
    pub limitation: Option<String>,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
#[ts(tag = "kind", rename_all = "snake_case")]
pub enum SourceOutput {
    Code { repository_key: String, checkout_key: String, base_commit: String,
        head_commit: String, commits: Vec<String>, task_revision: String,
        result_message_id: String, focused_checks: Vec<CheckOutcome> },
    NonCode { result_message_id: String, task_revision: String,
        artifacts: Vec<String>, focused_checks: Vec<CheckOutcome> },
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
pub enum IntegrationMethod { Ancestor, PatchEquivalent, RecordedAdaptation }
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(deny_unknown_fields)]
pub struct IntegrationMapping {
    pub source_commits: Vec<String>,
    pub target_commits: Vec<String>,
    pub source_paths: Vec<String>,
    pub target_paths: Vec<String>,
    pub explanation: Option<String>,
}
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(tag = "state", rename_all = "snake_case", deny_unknown_fields)]
#[ts(tag = "state", rename_all = "snake_case")]
pub enum Integration {
    #[default]
    Pending,
    NotApplicable { reason: String, result_message_id: String },
    Integrated { operation_id: String, accepted_result_message_id: String,
        accepted_task_revision: String, source: SourceOutput,
        target_repository_key: String, target_ref: String, target_commit: String,
        method: IntegrationMethod, mappings: Vec<IntegrationMapping>,
        checks: Vec<CheckOutcome>, integrator_run_id: String, verified_at: String },
}
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[serde(deny_unknown_fields)]
pub struct IntegrationRecord {
    pub operation_id: String,
    pub run_id: String,
    pub expected_result_message_id: String,
    pub expected_task_revision: String,
    pub integration: Integration,
}
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
pub struct IntegrationRecordResult {
    pub operation_id: String,
    pub run_id: String,
    pub integration: Integration,
    pub checks: Vec<CheckOutcome>,
    pub duplicate: bool,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(deny_unknown_fields)]
pub struct ResourceSourceIdentity {
    pub checkout_key: String,
    pub source_commit: String,
    pub source_tree: String,
    pub input_configuration: String,
    pub toolchain_fingerprint: String,
}
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[serde(deny_unknown_fields)]
pub struct ResourceRequest {
    pub operation_id: String,
    pub resource_key: String,
    pub source_identity: ResourceSourceIdentity,
    pub holder: AttachmentReference,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
pub enum ResourceRequestState { Queued, Held, Released, Blocked }
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[serde(deny_unknown_fields)]
pub struct ResourceRequestRecord {
    pub request_id: String,
    #[ts(type = "number")]
    pub sequence: u64,
    pub resource_key: String,
    pub source_identity: ResourceSourceIdentity,
    pub holder: AttachmentReference,
    pub requester_child_id: Option<String>,
    pub state: ResourceRequestState,
    pub build_processes: Vec<NativeShellIdentity>,
    pub machine_id: String,
    pub blocked_reason: Option<String>,
    pub updated_at: String,
}
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[serde(deny_unknown_fields)]
pub struct ResourceRelease {
    pub request_id: String,
    pub expected_holder: AttachmentReference,
}
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[serde(deny_unknown_fields)]
pub struct ResourceBuildRegistration {
    pub request_id: String,
    pub expected_holder: AttachmentReference,
    pub process: NativeProcessIdentity,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
pub enum OwnedResourceKind { WorkerTerminal, Worktree, Branch, Space, Scratch, FixtureSet }
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
pub enum ResourceOwnership { Owned, Borrowed, Unknown }
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
pub enum CleanupAuthority { OwnedWhenSafe, AuthorityUnproven }
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(deny_unknown_fields)]
pub struct OwnedGitIdentity {
    pub creation_operation_id: String,
    pub machine_id: String,
    pub repository_key: String,
    pub repository_root: String,
    pub repository_device: String,
    pub repository_inode: String,
    pub common_dir: String,
    pub common_device: String,
    pub common_inode: String,
    pub checkout_path: String,
    pub checkout_device: String,
    pub checkout_inode: String,
    pub checkout_key: String,
    pub branch_ref: Option<String>,
    pub creation_tip: String,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(deny_unknown_fields)]
pub struct OwnedSpaceIdentity {
    pub creation_operation_id: String,
    pub endpoint_identity: String,
    pub session_id: String,
    pub workspace_id: String,
    pub checkout_path: String,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(deny_unknown_fields)]
pub struct OwnedFilesystemIdentity {
    pub creation_operation_id: String,
    pub machine_id: String,
    pub root: String,
    pub device: String,
    pub inode: String,
    pub manifest_path: String,
    pub manifest_device: String,
    pub manifest_inode: String,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
pub enum FixtureAdapterId { UiPolishRuntime }
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(deny_unknown_fields)]
pub struct OwnedFixtureIdentity {
    pub adapter: FixtureAdapterId,
    pub creation_operation_id: String,
    pub filesystem: OwnedFilesystemIdentity,
    pub processes: Vec<NativeProcessIdentity>,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
#[ts(tag = "kind", rename_all = "snake_case")]
pub enum OwnedResourceIdentity {
    WorkerTerminal { identity: RetirementIdentity },
    Worktree { identity: OwnedGitIdentity },
    Branch { identity: OwnedGitIdentity },
    Space { identity: OwnedSpaceIdentity },
    Scratch { identity: OwnedFilesystemIdentity },
    FixtureSet { identity: OwnedFixtureIdentity },
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
pub enum CleanupState { Pending, Running, Waiting, WaitingIntegration, Reconciling, Removed, Kept, Stalled, Unknown }
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
pub enum CleanupReasonClass { Transient, Reconcilable, UserWork, Authority }
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(deny_unknown_fields)]
pub struct CleanupReason {
    pub class: CleanupReasonClass,
    pub code: String,
    pub message: String,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
pub enum CleanupEffectState { Intent, Applied, Absent, Refused, Unknown }
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(deny_unknown_fields)]
pub struct CleanupEffectReceipt {
    pub effect_id: String,
    pub kind: OwnedResourceKind,
    pub identity_digest: String,
    pub state: CleanupEffectState,
    pub evidence: Option<String>,
    pub updated_at: String,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(deny_unknown_fields)]
pub struct CleanupItem {
    pub operation_id: Option<String>,
    pub state: CleanupState,
    pub reason: Option<CleanupReason>,
    pub effects: Vec<CleanupEffectReceipt>,
    pub updated_at: String,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(deny_unknown_fields)]
pub struct OwnedResourceRecord {
    pub resource_id: String,
    pub root_id: String,
    pub run_id: String,
    pub kind: OwnedResourceKind,
    pub ownership: ResourceOwnership,
    pub creation_operation_id: Option<String>,
    pub identity: OwnedResourceIdentity,
    pub authority: CleanupAuthority,
    pub consumers: Vec<AttachmentReference>,
    pub integration_result_id: Option<String>,
    pub cleanup: CleanupItem,
}
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[serde(deny_unknown_fields)]
pub struct ResourceRegister {
    pub operation_id: String,
    pub run_id: String,
    pub resource: OwnedResourceIdentity,
}
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[serde(deny_unknown_fields)]
pub struct CleanupFinish {
    pub operation_id: String,
    pub root_id: String,
    #[serde(default)]
    pub run_id: Option<String>,
}
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[serde(deny_unknown_fields)]
pub struct CleanupFinishResult {
    pub operation_id: String,
    pub items: Vec<OwnedResourceRecord>,
    pub removed_count: u32,
    pub waiting_count: u32,
    pub kept_count: u32,
    pub unknown_count: u32,
    pub duplicate: bool,
}
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[serde(deny_unknown_fields)]
pub struct CleanupResourceSummary {
    pub resource_id: String,
    pub kind: OwnedResourceKind,
    pub ownership: ResourceOwnership,
    pub cleanup: CleanupItem,
}
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[serde(deny_unknown_fields)]
pub struct RunCleanupSummary {
    pub run_id: String,
    pub items: Vec<CleanupResourceSummary>,
    pub removed_count: u32,
    pub waiting_count: u32,
    pub kept_count: u32,
    pub unknown_count: u32,
}
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[serde(deny_unknown_fields)]
pub struct RestorationSummary {
    pub reattached: u32,
    pub total_attachments: u32,
    pub cleanup_pending: u32,
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
    pub attachment_id: Option<String>,
    #[ts(type = "number | null")]
    pub attachment_epoch: Option<u64>,
    pub launch_mode: Option<AttachmentLaunchMode>,
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
    /// Independently confirmed native writer-admission seal after consumer
    /// drain. A shutdown request or observed PID absence alone is not proof.
    pub quiescent_stop_at: Option<String>,
    pub state: RetirementState,
    pub created_at: String,
    pub updated_at: String,
}
// Keep the payload-bearing wire state and its Rust-only kind in one definition.
macro_rules! retirement_states {
    ($($variant:ident { $($field:ident: $field_type:ty),* $(,)? }),* $(,)?) => {
        #[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
        #[serde(tag = "state", rename_all = "snake_case")]
        #[ts(tag = "state", rename_all = "snake_case")]
        pub enum RetirementState {
            $($variant { $($field: $field_type),* }),*
        }

        #[derive(Debug, Clone, Copy, PartialEq, Eq)]
        pub enum RetirementStateKind {
            $($variant),*
        }

        impl RetirementState {
            pub const fn kind(&self) -> RetirementStateKind {
                match self {
                    $(Self::$variant { .. } => RetirementStateKind::$variant),*
                }
            }
        }
    };
}

retirement_states! {
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
#[serde(deny_unknown_fields)]
pub struct Run {
    pub session_id: String,
    pub brief: String,
    pub run_id: String,
    pub kind: RunKind,
    pub label: String,
    pub root_id: String,
    pub parent_run_id: Option<String>,
    pub task_id: Option<String>,
    pub attempt: u32,
    pub task_revision_at_start: Option<String>,
    pub stage: RunStage,
    pub close_reason: Option<CloseReason>,
    pub dispatch: Option<DispatchState>,
    pub target: Option<DispatchTarget>,
    pub setup: Option<SetupSummary>,
    /// Standing authority covers only resources proven owned by creation receipts.
    pub cleanup_policy: Option<WorkerCleanupPolicy>,
    pub last_report: Option<Report>,
    pub result: Option<Report>,
    #[serde(default)]
    pub source_output: Option<SourceOutput>,
    #[serde(default)]
    pub integration: Integration,
    pub annotations: Vec<Annotation>,
    pub project: Option<ProjectIdentity>,
    pub checkout: Option<CheckoutIdentity>,
    pub attachment: Option<ExecutionAttachment>,
    pub checkout_claim_id: Option<String>,
    pub resume: Option<ResumeRecord>,
    pub retirement: Option<RunRetirement>, // created with accepted closure; never backfilled
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
    Initialized,
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
    pub source_output: Option<SourceOutput>,
    pub at: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
pub enum MessageKind {
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
    pub origin: Option<DecisionOrigin>,
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
        operation_id: String,
        mode: SupervisorStartMode,
        target: Option<DispatchTarget>,
        label: Option<String>,
        selected_root_id: Option<String>,
        expected_project_key: Option<String>,
    },
    RunResume {
        run_id: String,
        operation_id: String,
        #[ts(type = "number | null")]
        expected_attachment_epoch: Option<u64>,
        mode: ResumeMode,
        decision_id: Option<String>,
    },
    RunAttach {
        operation_id: String,
        attachment_id: String,
        #[ts(type = "number")]
        expected_attachment_epoch: u64,
        main_session_id: String,
    },
    JobChildRegister {
        operation_id: String,
        child_id: String,
        child_session_id: String,
        parent_child_id: Option<String>,
    },
    RetirementNativeReceipt {
        retirement_id: String,
        outcome: NativeStopReceipt,
    }, // bound main session of the accepted retiring worker only
    RunAdopt {
        label: String,
    }, // unbound agent: binds caller pane as Adopted root
    // worker lifecycle
    WorkerStart {
        operation_id: String,
        root_id: String,
        task_id: String,
        expected_task_revision: String,
        expected_doc_revision: String,
        target: DispatchTarget,
        label: Option<String>,
        brief: String,
        cleanup_policy: WorkerCleanupPolicy,
    },
    TaskScopeUpdate {
        operation_id: String,
        root_id: String,
        task_id: String,
        run_id: String,
        expected_task_revision: String,
        expected_doc_revision: String,
        title: Option<String>,
        description: Option<String>,
    },
    Accept {
        run_id: String,
        expected_task_revision: String,
    }, // operator or current bound top-main supervisor root targeting a strict-descendant Worker
    IntegrationRecord { record: IntegrationRecord },
    ResourceRequest { request: ResourceRequest },
    ResourceRelease { release: ResourceRelease },
    ResourceBuildRegister { registration: ResourceBuildRegistration },
    ResourceRegister { request: ResourceRegister },
    CleanupFinish { request: CleanupFinish },
    SendBack {
        run_id: String,
        text: String,
    }, // operator or current bound top-main supervisor root targeting a strict-descendant Worker
    CancelRun {
        run_id: String,
    }, // operator or current bound top-main supervisor root targeting a strict-descendant Worker;
    // closes tracking and requests stop, not process termination
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
        source_output: Option<SourceOutput>,
        to_run_id: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        original_task_revision: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        original_claim_id: Option<String>,
    },
    MessageSend {
        message_id: String,
        to_run_id: String,
        kind: MessageKind,
        text: String,
        #[serde(default)]
        in_reply_to: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        original_task_revision: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        original_claim_id: Option<String>,
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
    WorkerStarted {
        start: WorkerStartResult,
    },
    TaskScopeUpdated {
        update: TaskScopeUpdateResult,
    },
    IntegrationRecorded { receipt: IntegrationRecordResult },
    Resource { request: ResourceRequestRecord, duplicate: bool },
    ResourceRegistered { resource: OwnedResourceRecord, duplicate: bool },
    CleanupFinished { cleanup: CleanupFinishResult },
    SupervisorStarted {
        start: SupervisorStartResult,
    },
    RunResumed {
        resume: RunResumeResult,
    },
    RunAttached {
        attach: RunAttachResult,
    },
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
    fn resume_and_attachment_requests_cannot_assert_credentials_or_exit_proof() {
        let value = json!({
            "action": "run_resume", "run_id": "run", "operation_id": "operation",
            "expected_attachment_epoch": 7, "mode": "reattach_or_restart", "decision_id": null,
        });
        let action: OrchestrationAction = serde_json::from_value(value.clone()).unwrap();
        assert!(matches!(action, OrchestrationAction::RunResume { expected_attachment_epoch: Some(7), mode: ResumeMode::ReattachOrRestart, .. }));
        let mut forged = value;
        forged["writer_exit_proof"] = json!({"all_consumers_absent": true});
        assert!(serde_json::from_value::<OrchestrationAction>(forged).is_err());
        let forged = json!({
            "action": "run_attach", "operation_id": "operation", "attachment_id": "attachment",
            "expected_attachment_epoch": 7, "main_session_id": "conversation", "secret": "not-action-input",
        });
        assert!(serde_json::from_value::<OrchestrationAction>(forged).is_err());
    }

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
}
