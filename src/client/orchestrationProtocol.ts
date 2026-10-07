import type {
  OrchestrationAction, OrchestrationMutationRequest, OrchestrationMutationResponse,
  OrchestrationSnapshotRequest, OrchestrationSnapshot, OrchestrationWaitRequest, OrchestrationWaitResponse,
} from "../protocol/generated/v1";
import { CockpitClientError, validateSessionId } from "./CockpitClient";
import { parseWorkspaceSetupRequest } from "./projectProtocol";

type Validator = (value: unknown) => boolean;
type Fields = Readonly<Record<string, Validator>>;
const encoder = new TextEncoder();
const record = (value: unknown): value is Record<string, unknown> =>
  typeof value === "object" && value !== null && !Array.isArray(value);
const boundedText = (max: number): Validator => value => typeof value === "string"
  && value.length <= max && encoder.encode(value).byteLength <= max && !value.includes("\0");
const text = boundedText(16 * 1024);
const label = boundedText(256);
// Canonical Markdown and live Herdr labels are external data, not mutation input.
// Their source readers own size bounds; applying the input limits would hide valid tasks.
const externalText = (value: unknown): value is string => typeof value === "string";
const path = boundedText(4096);
const id: Validator = value => typeof value === "string" && value.length > 0
  && value.length <= 512 && encoder.encode(value).byteLength <= 512 && !/[\x00-\x1f\x7f]/.test(value);
const hash: Validator = value => typeof value === "string" && /^[a-f0-9]{64}$/.test(value);
const uuid: Validator = value => typeof value === "string"
  && /^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$/i.test(value);
const bool: Validator = value => typeof value === "boolean";
const u64: Validator = value => typeof value === "number" && Number.isSafeInteger(value) && value >= 0;
const u32: Validator = value => u64(value) && (value as number) <= 0xffffffff;
const timestamp: Validator = value => typeof value === "string" && value.length <= 64
  && /^\d{4}-\d{2}-\d{2}T\d{2}:\d{2}:\d{2}(?:\.\d+)?(?:Z|[+-]\d{2}:\d{2})$/.test(value)
  && Number.isFinite(Date.parse(value));
const nullable = (validate: Validator): Validator => value => value === null || validate(value);
const list = (validate: Validator): Validator => value => {
  if (!Array.isArray(value)) return false;
  for (const item of value) if (!validate(item)) return false;
  return true;
};
const oneOf = (...values: readonly string[]): Validator => value => typeof value === "string" && values.includes(value);
const shape = (fields: Fields, strict = false): Validator => value => record(value)
  && Object.entries(fields).every(([key, validate]) => Object.hasOwn(value, key) && validate(value[key]))
  && (!strict || Object.keys(value).every(key => Object.hasOwn(fields, key)));
const session: Validator = value => {
  if (!id(value)) return false;
  try { validateSessionId(value as string); return true; } catch { return false; }
};
function malformed(label: string): never {
  throw new CockpitClientError("malformed_response", `Invalid orchestration ${label}`);
}

const runKind = oneOf("supervisor", "adopted", "worker");
const reportKind = oneOf("progress", "ready", "result", "needs_input");
const reportOutcome = nullable(oneOf("succeeded", "failed"));
const messageKind = oneOf("prepare_brief", "work_brief", "supervisor_brief", "instruction", "answer", "cancel_request", "subagent_control", "report", "observation");
const subagentStatus = oneOf("running", "done", "failed", "cancelled");
const recovery = oneOf("accept_existing_worktree", "retry_environment");
const error = shape({ code: id, message: text });
const actor: Validator = value => record(value) && (
  ((value.type === "operator" || value.type === "dispatcher") && shape({ type: oneOf("operator", "dispatcher") })(value))
  || (value.type === "run" && shape({ type: oneOf("run"), run_id: id })(value))
);
const report = shape({ message_id: id, kind: reportKind, outcome: reportOutcome, summary: text, plan: nullable(text), at: timestamp });
const subagentOp: Validator = value => record(value) && (
  (value.op === "cancel" && shape({ op: oneOf("cancel") }, true)(value))
  || (value.op === "send" && shape({ op: oneOf("send"), text }, true)(value))
);
const setupRequest: Validator = value => {
  try {
    const request = parseWorkspaceSetupRequest(value);
    if (!nullable(label)(request.label) || !nullable(label)(request.task_name)) return false;
    return request.operation === "open" ? path(request.path)
      : id(request.repository_id) && nullable(path)(request.branch) && nullable(path)(request.base_ref)
        && nullable(path)(request.checkout_path) && nullable(path)(request.artifact_url) && list(path)(request.linked_artifact_urls);
  } catch { return false; }
};
const target: Validator = value => record(value) && (
  (value.target === "setup" && shape({ target: oneOf("setup"), request: setupRequest }, true)(value))
  || (value.target === "existing_space" && shape({ target: oneOf("existing_space"), workspace_id: id }, true)(value))
);
const task = shape({ task_id: id, title: externalText, body: externalText, checked: bool, line: u32, task_revision: hash, diagnostic: nullable(id) });
const board = shape({
  root_id: id, path, doc_revision: hash, unidentified_items: u32, diagnostics: list(error),
  tasks: list(shape({ task, lane: oneOf("queued", "setup", "ready", "working", "review", "accepted"), current_run_id: nullable(id) })),
});
const plan = shape({ plan_revision: hash, text, created_at: timestamp });
const setup = shape({
  operation_id: nullable(id), generation: nullable(u32), workspace_id: nullable(id), checkout_path: path,
  repository_id: nullable(id), branch: nullable(path), base: nullable(path),
  ownership: nullable(oneOf("owned_worktree", "borrowed_directory")), effects: list(text), warnings: list(text),
});
const location = shape({
  endpoint_identity: id, session_id: session, workspace_id: id, tab_id: id, pane_id: id, launch_tag: id,
  boot_id: nullable(id), terminal_id: nullable(id), native_session_id: nullable(id),
});
const nativeProcessIdentity = shape({
  pid: u32, start_ticks: u64, kernel_boot_id: nullable(id),
}, true);
const numericIdentity: Validator = value => typeof value === "string" && /^(?:0|[1-9]\d*)$/.test(value)
  && (value.length < 20 || (value.length === 20 && value <= "18446744073709551615"));
const nativeShellIdentity = shape({
  process: nativeProcessIdentity, executable_device: numericIdentity, executable_inode: numericIdentity, argv_digest: hash,
}, true);
const retirementIdentity = shape({
  run_attempt: u32, launch_attempt: u32, launch_tag: id, endpoint_identity: id,
  session_id: session, workspace_id: id, tab_id: id, pane_id: id, terminal_id: id,
  herdr_boot_id: nullable(id), omp_session_id: id, process: nativeProcessIdentity, shell: nativeShellIdentity,
}, true);
const nativeDeferReason = oneOf("busy", "pending_messages", "async_jobs", "live_subagents", "editor_draft");
const retainReason = oneOf(
  "identity_incomplete", "identity_changed", "endpoint_changed", "native_process_unverifiable",
  "process_pane_mismatch", "worker_unresponsive", "worker_busy_timeout", "user_activity", "native_refused",
  "shared_tab", "tab_renamed", "pane_moved", "foreground_process", "observation_unavailable", "herdr_refused",
);
const retirementStates: Readonly<Record<string, Fields>> = {
  waiting: { blockers: list(oneOf("open_descendant_runs", "running_subagents")) },
  native_stop_offered: { offered_at: timestamp },
  native_stop_deferred: { offered_at: timestamp, reason: nativeDeferReason, at: timestamp },
  native_stop_requested: { at: timestamp },
  native_stopped: { at: timestamp, evidence: oneOf("exited_after_shutdown_request", "already_exited") },
  close_intent: { at: timestamp },
  retired: { at: timestamp, terminal: oneOf("closed_by_cockpit", "already_absent", "absent_after_uncertain_close") },
  retained: { at: timestamp, reason: retainReason, native_stopped: bool },
  unknown: { at: timestamp, phase: oneOf("native_stop", "terminal_close"), detail: text },
};
const retirementState: Validator = value => record(value) && typeof value.state === "string"
  && Object.hasOwn(retirementStates, value.state)
  && shape({ state: oneOf(value.state), ...retirementStates[value.state]! }, true)(value);
const retirement: Validator = value => shape({
  retirement_id: uuid, trigger: oneOf("accept", "accept_recovery", "operator_conflict_resolution"),
  result_message_id: id, task_revision: hash, identity: nullable(retirementIdentity), state: retirementState,
  created_at: timestamp, updated_at: timestamp,
}, true)(value) && record(value) && (value.identity !== null
  || (record(value.state) && value.state.state === "retained" && value.state.reason === "identity_incomplete"));
const nativeStopReceipt: Validator = value => record(value) && (
  (value.outcome === "shutdown_requested" && shape({ outcome: oneOf("shutdown_requested") }, true)(value))
  || (value.outcome === "deferred" && shape({ outcome: oneOf("deferred"), reason: nativeDeferReason }, true)(value))
  || (value.outcome === "refused" && shape({
    outcome: oneOf("refused"), reason: oneOf("user_activity", "native_refused"), text: boundedText(1024),
  }, true)(value))
);
const grantOrigin = oneOf("browser", "native", "supervisor");
// Persisted legacy grants/intents acquire explicit nulls when serialized by Rust.
// A supervisor decision always identifies its verified root and actual SDK session;
// operator and legacy decisions must not claim either identity.
const provenance: Validator = value => record(value) && (value.origin === "supervisor"
  ? id(value.supervisor_run_id) && id(value.omp_session_id)
  : value.supervisor_run_id === null && value.omp_session_id === null);
const grant: Validator = value => shape({
  grant_id: id, scope: oneOf("prepare", "execute"), plan_revision: hash, origin: grantOrigin,
  supervisor_run_id: nullable(id), omp_session_id: nullable(id), granted_at: timestamp,
})(value) && provenance(value);
const run: Validator = value => shape({
  session_id: session, prepare_brief: text, run_id: id, kind: runKind, label, root_id: id,
  parent_run_id: nullable(id), task_id: nullable(id), attempt: u32, task_revision_at_propose: nullable(hash),
  stage: oneOf("proposed", "awaiting_prepare", "preparing", "initializing", "ready", "working", "reported", "active", "closed"),
  close_reason: nullable(oneOf("accepted", "cancelled", "superseded", "failed")),
  dispatch: nullable(shape({
    step: oneOf("planning", "plan_failed", "setup_pending", "setup_running", "setup_unknown", "launch_intent", "launch_pending", "launch_unknown", "launched", "needs_review"),
    launch_attempt: u32, error: nullable(error), updated_at: timestamp, launch_tag: nullable(id),
    endpoint_identity: nullable(id), recovery: nullable(recovery), agent_started: bool,
  })),
  target: nullable(target), setup: nullable(setup), prepare_plan: nullable(plan), init_receipt: nullable(report), work_plan: nullable(plan),
  grants: list(grant),
  last_report: nullable(report), result: nullable(report), annotations: list(shape({ by: actor, text, at: timestamp })),
  location: nullable(location), bound_omp_session: nullable(id), bound_omp_process: nullable(nativeProcessIdentity),
  launch_shell_identity: nullable(nativeShellIdentity),
  retirement: nullable(retirement), supersedes_run_id: nullable(id), created_at: timestamp, updated_at: timestamp,
})(value) && record(value) && Array.isArray(value.grants)
  && value.grants.every(grant => record(grant) && (grant.origin !== "supervisor" || grant.supervisor_run_id === value.root_id));
const message = shape({
  message_id: id, to_run_id: id, seq: u64, from: actor, kind: messageKind, text, report: nullable(report), stale: bool,
  escalated_from: nullable(id), from_subagent_id: nullable(id), stage: oneOf("stored", "woken", "read", "acked"),
  woken_omp_session: nullable(id), created_at: timestamp, acked_at: nullable(timestamp),
});
const subagent = shape({
  run_id: id, subagent_id: id, parent_subagent_id: nullable(id), role: nullable(label), label,
  status: subagentStatus, summary: nullable(text), updated_at: timestamp,
  last_control: nullable(shape({ seq: u64, op: subagentOp, stage: oneOf("stored", "applied", "failed"), error: nullable(text), at: timestamp })),
});
const intent: Validator = value => shape({
  intent_id: id, root_id: id, task_id: id, run_id: id, expected_task_revision: hash, state: oneOf("pending", "conflict"),
  origin: nullable(grantOrigin), supervisor_run_id: nullable(id), omp_session_id: nullable(id), result_message_id: nullable(id),
})(value) && provenance(value) && record(value)
  && (value.origin !== "supervisor" || value.supervisor_run_id === value.root_id);
const assignmentIntent = shape({ root_id: id, task_id: uuid, state: oneOf("pending", "conflict") }, true);
const observation: Validator = value => shape({
  run_id: id, presence: oneOf("present", "missing", "endpoint_changed", "unobserved"), workspace_id: nullable(id),
  pane_id: nullable(id), actual_omp: bool,
  workspace_label: nullable(externalText), tab_id: nullable(id), tab_label: nullable(externalText), agent_status: nullable(externalText), state_changed_at: nullable(timestamp),
})(value) && record(value) && (!value.actual_omp || value.presence === "present");
const runtime: Validator = value => record(value) && (
  (value.status === "fresh" && shape({ status: oneOf("fresh"), endpoint_identity: id, observed_at: timestamp, runs: list(observation) })(value))
  || (value.status === "unavailable" && shape({ status: oneOf("unavailable"), error })(value))
);
const attention = shape({
  kind: oneOf("awaits_prepare", "awaits_execute", "plan_changed", "to_accept", "needs_input", "runtime_blocked", "brief_unread", "idle_without_report", "exited_without_report", "dispatch_unknown", "intent_conflict", "retirement_unconfirmed"),
  run_id: nullable(id), task_id: nullable(id), message_seq: nullable(u64), since: timestamp,
});

// The action schemas mirror the tagged, deny_unknown_fields Rust enum. In particular,
// actor/origin can never be supplied by a caller through this transport boundary.
const actions: Readonly<Record<OrchestrationAction["action"], Fields>> = {
  task_create: { root_id: id, title: label, body: text },
  task_assign: { root_id: id, task_id: uuid, title: label, body: text },
  task_assignment_resolve: { root_id: id, task_id: uuid, expected_task_revision: nullable(hash), assign: bool },
  task_update: { root_id: id, task_id: id, expected_task_revision: hash, title: nullable(label), body: nullable(text) },
  tasks_assign_ids: { root_id: id, expected_doc_revision: hash },
  supervisor_start: { target: nullable(target), label: nullable(label) },
  run_bind_session: { omp_session_id: id },
  retirement_native_receipt: { retirement_id: uuid, outcome: nativeStopReceipt },
  run_adopt: { label },
  run_propose: { task_id: id, parent_run_id: nullable(id), label: nullable(label), target, prepare_brief: text, supersedes_run_id: nullable(id) },
  grant_prepare: { run_id: id, plan_revision: hash },
  grant_execute: { run_id: id, plan_revision: hash, note: nullable(text) },
  accept: { run_id: id, expected_task_revision: hash },
  send_back: { run_id: id, text },
  cancel_run: { run_id: id },
  retry_launch: { run_id: id },
  reconcile_run: { run_id: id, recovery: nullable(recovery) },
  intent_resolve: { intent_id: id, apply: bool },
  report: { message_id: id, kind: reportKind, outcome: reportOutcome, summary: text, plan: nullable(text), to_run_id: nullable(id) },
  message_send: { message_id: id, to_run_id: id, kind: messageKind, text },
  annotate: { run_id: id, text },
  inbox_pull: { after_seq: u64, limit: value => u32(value) && (value as number) <= 100 },
  inbox_woken: { through_seq: u64, omp_session_id: id },
  inbox_ack: { through_seq: u64 },
  subagent_update: { subagent_id: id, parent_subagent_id: nullable(id), role: nullable(label), label, status: subagentStatus, summary: nullable(text) },
  subagent_control: { run_id: id, subagent_id: id, op: subagentOp },
  subagent_control_done: { seq: u64, applied: bool, error: nullable(text) },
};
export function parseOrchestrationAction(value: unknown): OrchestrationAction {
  if (!record(value) || typeof value.action !== "string" || !Object.hasOwn(actions, value.action)) malformed("action");
  const fields = actions[value.action as OrchestrationAction["action"]];
  if (!shape({ action: oneOf(value.action), ...fields }, true)(value)) malformed("action");
  if (value.action === "task_assignment_resolve" && value.assign && value.expected_task_revision === null) malformed("assignment revision");
  return value as unknown as OrchestrationAction;
}
export function parseOrchestrationSnapshotRequest(value: unknown): OrchestrationSnapshotRequest {
  if (!shape({ session_id: session, root_id: nullable(id) }, true)(value)) malformed("snapshot request");
  return value as OrchestrationSnapshotRequest;
}
export function parseOrchestrationMutationRequest(value: unknown): OrchestrationMutationRequest {
  if (!shape({ session_id: session, expected_revision: nullable(u64), action: value => { parseOrchestrationAction(value); return true; } }, true)(value)) malformed("mutation request");
  return value as OrchestrationMutationRequest;
}
export function parseOrchestrationWaitRequest(value: unknown): OrchestrationWaitRequest {
  if (!shape({ after_revision: u64, after_tasks_token: hash, timeout_ms: value => u32(value) && (value as number) <= 30_000 }, true)(value)) malformed("wait request");
  return value as OrchestrationWaitRequest;
}
export function parseOrchestrationSnapshot(value: unknown): OrchestrationSnapshot {
  if (!shape({
    session_id: session, revision: u64, tasks_token: hash,
    roots: list(shape({ root_id: id, label, kind: runKind, open_runs: u32, needs_you: u32 })),
    board: nullable(board), runs: list(run), messages: list(message), subagents: list(subagent), intents: list(intent), runtime, attention: list(attention),
    assignment_intents: list(assignmentIntent),
    unmanaged_agents: list(shape({
      workspace_id: id, workspace_label: externalText, tab_id: id, tab_label: externalText, pane_id: id,
      agent_name: externalText, agent_status: nullable(externalText), state_changed_at: nullable(timestamp),
    })),
  })(value)) malformed("snapshot");
  const snapshot = value as OrchestrationSnapshot;
  if (snapshot.runs.some(run => run.session_id !== snapshot.session_id
    || (run.location !== null && run.location.session_id !== snapshot.session_id))) malformed("run session identity");
  return snapshot;
}
export function matchOrchestrationSnapshot(value: unknown, request: OrchestrationSnapshotRequest): OrchestrationSnapshot {
  const snapshot = parseOrchestrationSnapshot(value);
  if (snapshot.session_id !== request.session_id || (request.root_id !== null && snapshot.board !== null && snapshot.board.root_id !== request.root_id)) malformed("snapshot identity");
  return snapshot;
}
const results: Readonly<Record<OrchestrationMutationResponse["result"]["result"], Fields>> = {
  task: { task }, task_ids: { assigned: u32, doc_revision: hash }, run: { run_id: id, attempt: u32 },
  task_assigned: { task, to_run_id: id, seq: u64, duplicate: bool },
  message: { to_run_id: id, seq: u64, duplicate: bool, stale: bool }, inbox: { messages: list(message), read_through_seq: u64 }, done: {},
};
export function parseOrchestrationMutationResponse(value: unknown): OrchestrationMutationResponse {
  if (!record(value) || !u64(value.revision) || !record(value.result)
    || typeof value.result.result !== "string" || !Object.hasOwn(results, value.result.result)) malformed("mutation response");
  const fields = results[value.result.result as OrchestrationMutationResponse["result"]["result"]];
  if (!shape({ result: oneOf(value.result.result), ...fields })(value.result)) malformed("mutation result");
  return value as unknown as OrchestrationMutationResponse;
}
export function parseOrchestrationWaitResponse(value: unknown): OrchestrationWaitResponse {
  if (!shape({ revision: u64, tasks_token: hash, changed: bool })(value)) malformed("wait response");
  return value as OrchestrationWaitResponse;
}
