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
    pub subagents: Vec<Subagent>,
    pub intents: Vec<TaskIntent>,
    #[serde(default)]
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
    pub checked: bool,
    pub line: u32,
    pub task_revision: String,
    pub diagnostic: Option<String>,
}
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
pub struct TaskView {
    pub task: Task,
    pub lane: TaskLane,
    pub current_run_id: Option<String>,
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
    #[serde(default)]
    pub supervisor_run_id: Option<String>,
    #[serde(default)]
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
    pub text: String,           // ≤ 16 KiB UTF-8
    pub report: Option<Report>, // kind == Report
    pub stale: bool,            // sender attempt superseded / caller mismatch evidence
    pub escalated_from: Option<String>,
    pub from_subagent_id: Option<String>, // report sent from an OMP subagent context of the sending run
    pub stage: DeliveryStage,
    pub woken_omp_session: Option<String>,
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
    #[serde(default)]
    pub origin: Option<GrantOrigin>,
    #[serde(default)]
    pub supervisor_run_id: Option<String>,
    #[serde(default)]
    pub omp_session_id: Option<String>,
    #[serde(default)]
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
        title: String,
        body: String,
    }, // operator: any root; agent: only its bound run's root_id
    TaskAssign {
        root_id: String,
        task_id: String,
        title: String,
        body: String,
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
        body: Option<String>,
    }, // same rule
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
    }, // operator; new tab, launch_attempt + 1
    ReconcileRun {
        run_id: String,
        recovery: Option<WorkspaceRecoveryAction>,
    }, // operator; explicit
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
    fn legacy_grants_preserve_operator_origin_and_identity() {
        for (name, origin) in [
            ("browser", GrantOrigin::Browser),
            ("native", GrantOrigin::Native),
        ] {
            let legacy = json!({
                "grant_id": "legacy-grant",
                "scope": "execute",
                "plan_revision": "a".repeat(64),
                "origin": name,
                "granted_at": "2026-10-05T12:00:00Z",
            });
            let grant: Grant = serde_json::from_value(legacy.clone()).unwrap();
            assert_eq!(grant.origin, origin);
            assert_eq!(grant.supervisor_run_id, None);
            assert_eq!(grant.omp_session_id, None);
            let mut expected = legacy;
            expected["supervisor_run_id"] = Value::Null;
            expected["omp_session_id"] = Value::Null;
            assert_eq!(serde_json::to_value(grant).unwrap(), expected);
        }
    }

    #[test]
    fn legacy_acceptance_intent_does_not_invent_provenance() {
        let legacy = json!({
            "intent_id": "legacy-intent",
            "root_id": "root",
            "task_id": "task",
            "run_id": "worker",
            "expected_task_revision": "b".repeat(64),
            "state": "pending",
        });
        let intent: TaskIntent = serde_json::from_value(legacy.clone()).unwrap();
        assert_eq!(intent.origin, None);
        assert_eq!(intent.supervisor_run_id, None);
        assert_eq!(intent.omp_session_id, None);
        assert_eq!(intent.result_message_id, None);
        let mut expected = legacy;
        for key in [
            "origin",
            "supervisor_run_id",
            "omp_session_id",
            "result_message_id",
        ] {
            expected[key] = Value::Null;
        }
        assert_eq!(serde_json::to_value(intent).unwrap(), expected);
    }

    #[test]
    fn legacy_snapshot_defaults_assignment_intents_to_empty() {
        let legacy = json!({
            "session_id": "session", "revision": 1, "tasks_token": "c".repeat(64),
            "roots": [], "board": null, "runs": [], "messages": [], "subagents": [],
            "intents": [], "runtime": {
                "status": "unavailable", "error": { "code": "offline", "message": "Offline" },
            },
            "unmanaged_agents": [], "attention": [],
        });
        let snapshot: OrchestrationSnapshot = serde_json::from_value(legacy).unwrap();
        assert!(snapshot.assignment_intents.is_empty());
        assert_eq!(
            serde_json::to_value(snapshot).unwrap()["assignment_intents"],
            json!([])
        );
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
