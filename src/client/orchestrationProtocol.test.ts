import { describe, expect, it, vi } from "vitest";
import type { OrchestrationAction, OrchestrationSnapshot } from "../protocol/generated/v1";
import { CockpitClientError } from "./CockpitClient";
import { createBrowserClient } from "./browser";
import { createNativeClient } from "./native";
import {
  matchOrchestrationSnapshot, parseOrchestrationAction, parseOrchestrationMutationRequest,
  parseOrchestrationMutationResponse, parseOrchestrationSnapshot, parseOrchestrationSnapshotRequest,
  parseOrchestrationWaitRequest, parseOrchestrationWaitResponse,
} from "./orchestrationProtocol";

const hash = "a".repeat(64);
const at = "2026-10-05T12:00:00Z";
const target = { target: "setup" as const, request: { operation: "open" as const, path: "/repo", label: null, task_name: null, focus: false } };
const task = { task_id: "task", title: "Title", body: "Body", checked: false, line: 3, task_revision: hash, diagnostic: null };
const report = { message_id: "report", kind: "ready" as const, outcome: "succeeded" as const, summary: "Ready", plan: "Work plan", at };
const message = {
  message_id: "message", to_run_id: "run", seq: 1, from: { type: "run" as const, run_id: "root" },
  kind: "report" as const, text: "Ready", report, stale: false, escalated_from: null, from_subagent_id: null,
  stage: "read" as const, woken_omp_session: "omp", created_at: at, acked_at: null,
};
const snapshot: OrchestrationSnapshot = {
  session_id: "session", revision: 2, tasks_token: hash,
  roots: [{ root_id: "root", label: "Supervisor", kind: "supervisor", open_runs: 1, needs_you: 1 }],
  board: { root_id: "root", path: "/tasks/root.md", doc_revision: hash, unidentified_items: 0,
    diagnostics: [{ code: "task_id_duplicate", message: "Duplicate" }], tasks: [{ task, lane: "ready", current_run_id: "run" }] },
  runs: [{
    session_id: "session", prepare_brief: "Prepare", run_id: "run", kind: "worker", label: "Worker", root_id: "root",
    parent_run_id: "root", task_id: "task", attempt: 1, task_revision_at_propose: hash, stage: "ready", close_reason: null,
    dispatch: { step: "launched", launch_attempt: 1, error: { code: "launch_unknown", message: "Review" }, updated_at: at,
      launch_tag: "launch", endpoint_identity: "endpoint", recovery: "retry_environment", agent_started: true },
    target, setup: { operation_id: "operation", generation: 1, workspace_id: "space", checkout_path: "/repo", repository_id: "repo",
      branch: "task", base: "main", ownership: "owned_worktree", effects: ["Create"], warnings: ["Warning"] },
    prepare_plan: { plan_revision: hash, text: "Prepare", created_at: at }, init_receipt: report,
    work_plan: { plan_revision: hash, text: "Work", created_at: at },
    grants: [{ grant_id: "grant", scope: "prepare", plan_revision: hash, origin: "browser", granted_at: at }],
    last_report: report, result: report, annotations: [{ by: { type: "operator" }, text: "Note", at }],
    location: { endpoint_identity: "endpoint", session_id: "session", workspace_id: "space", tab_id: "tab", pane_id: "pane",
      launch_tag: "launch", boot_id: "boot", terminal_id: "terminal", native_session_id: null },
    bound_omp_session: "omp", supersedes_run_id: null, created_at: at, updated_at: at,
  }],
  messages: [message],
  subagents: [{ run_id: "run", subagent_id: "child", parent_subagent_id: null, role: "coder", label: "Child", status: "running", summary: "Working",
    last_control: { seq: 1, op: { op: "send", text: "Report" }, stage: "applied", error: null, at }, updated_at: at }],
  intents: [{ intent_id: "intent", root_id: "root", task_id: "task", run_id: "run", expected_task_revision: hash, state: "pending" }],
  runtime: { status: "fresh", endpoint_identity: "endpoint", observed_at: at,
    runs: [{ run_id: "run", presence: "present", workspace_id: "space", workspace_label: "Space", tab_id: "tab", tab_label: "Worker", pane_id: "moved-pane", agent_status: "idle", state_changed_at: at }] },
  unmanaged_agents: [{ workspace_id: "space", workspace_label: "Space", tab_id: "other-tab", tab_label: "Unmanaged", pane_id: "other-pane",
    agent_name: "omp", agent_status: "idle", state_changed_at: at }],
  attention: [{ kind: "awaits_execute", run_id: "run", task_id: "task", message_seq: 1, since: at }],
};
const actions: OrchestrationAction[] = [
  { action: "task_create", root_id: "root", title: "Title", body: "Body" },
  { action: "task_update", root_id: "root", task_id: "task", expected_task_revision: hash, title: null, body: "Body" },
  { action: "tasks_assign_ids", root_id: "root", expected_doc_revision: hash },
  { action: "supervisor_start", target: null, label: null },
  { action: "run_bind_session", omp_session_id: "omp" },
  { action: "run_adopt", label: "Adopted" },
  { action: "run_propose", task_id: "task", parent_run_id: null, label: null, target, prepare_brief: "Prepare", supersedes_run_id: null },
  { action: "grant_prepare", run_id: "run", plan_revision: hash },
  { action: "grant_execute", run_id: "run", plan_revision: hash, note: null },
  { action: "accept", run_id: "run", expected_task_revision: hash },
  { action: "send_back", run_id: "run", text: "Retry" },
  { action: "cancel_run", run_id: "run" },
  { action: "retry_launch", run_id: "run" },
  { action: "reconcile_run", run_id: "run", recovery: "accept_existing_worktree" },
  { action: "intent_resolve", intent_id: "intent", apply: false },
  { action: "report", message_id: "message", kind: "result", outcome: "succeeded", summary: "Done", plan: null, to_run_id: null },
  { action: "message_send", message_id: "message", to_run_id: "run", kind: "instruction", text: "Work" },
  { action: "annotate", run_id: "run", text: "Note" },
  { action: "inbox_pull", after_seq: 0, limit: 100 },
  { action: "inbox_woken", through_seq: 1, omp_session_id: "omp" },
  { action: "inbox_ack", through_seq: 1 },
  { action: "subagent_update", subagent_id: "child", parent_subagent_id: null, role: null, label: "Child", status: "done", summary: null },
  { action: "subagent_control", run_id: "run", subagent_id: "child", op: { op: "cancel" } },
  { action: "subagent_control_done", seq: 1, applied: false, error: "Failed" },
];

// Exercise required fields recursively, including required-nullable fields in every
// nested DTO. The paths identify the boundary that failed instead of hiding it in a cast.
function fieldPaths(value: unknown, prefix: (string | number)[] = []): (string | number)[][] {
  if (Array.isArray(value)) return value.flatMap((item, index) => fieldPaths(item, [...prefix, index]));
  if (typeof value !== "object" || value === null) return [];
  return Object.entries(value).flatMap(([key, item]) => [[...prefix, key], ...fieldPaths(item, [...prefix, key])]);
}
function replaceField(value: unknown, path: (string | number)[], replacement: unknown, remove = false): unknown {
  const copy = structuredClone(value);
  let owner = copy as Record<string | number, unknown>;
  for (const key of path.slice(0, -1)) owner = owner[key] as Record<string | number, unknown>;
  const key = path[path.length - 1]!;
  if (remove) delete owner[key]; else owner[key] = replacement;
  return copy;
}

describe("orchestration protocol boundary", () => {
  it("accepts all populated DTOs and rejects every missing nested field", () => {
    expect(parseOrchestrationSnapshot(snapshot)).toEqual(snapshot);
    for (const path of fieldPaths(snapshot)) {
      expect(() => parseOrchestrationSnapshot(replaceField(snapshot, path, undefined, true)), path.join(".")).toThrow(CockpitClientError);
      expect(() => parseOrchestrationSnapshot(replaceField(snapshot, path, {})), path.join(".")).toThrow(CockpitClientError);
    }
    expect(parseOrchestrationSnapshot({ ...snapshot, runtime: { status: "unavailable", error: { code: "offline", message: "Offline" } } }).runtime.status).toBe("unavailable");
    expect(() => parseOrchestrationSnapshot({ ...snapshot, runtime: { status: "unavailable", error: { code: "offline", message: 4 } } })).toThrow(CockpitClientError);
    expect(() => parseOrchestrationSnapshot({ ...snapshot, messages: Array(1) })).toThrow(CockpitClientError);
  });

  it.each([
    ["roots", 0, "kind"], ["board", "tasks", 0, "lane"], ["runs", 0, "kind"], ["runs", 0, "stage"],
    ["runs", 0, "close_reason"], ["runs", 0, "dispatch", "step"], ["runs", 0, "dispatch", "recovery"],
    ["runs", 0, "target", "target"], ["runs", 0, "setup", "ownership"], ["runs", 0, "grants", 0, "scope"],
    ["runs", 0, "grants", 0, "origin"], ["runs", 0, "annotations", 0, "by", "type"],
    ["messages", 0, "from", "type"], ["messages", 0, "kind"], ["messages", 0, "stage"],
    ["messages", 0, "report", "kind"], ["messages", 0, "report", "outcome"],
    ["subagents", 0, "status"], ["subagents", 0, "last_control", "op", "op"], ["subagents", 0, "last_control", "stage"],
    ["intents", 0, "state"], ["runtime", "status"], ["runtime", "runs", 0, "presence"], ["attention", 0, "kind"],
  ])("rejects unknown nested enum at %s", (...path) => {
    expect(() => parseOrchestrationSnapshot(replaceField(snapshot, path, "unknown"))).toThrow(CockpitClientError);
  });

  it("rejects unsafe counters, malformed digests, timestamps, booleans and foreign sessions", () => {
    for (const revision of [-1, 1.5, Number.MAX_SAFE_INTEGER + 1, Infinity, "2", null]) {
      expect(() => parseOrchestrationSnapshot({ ...snapshot, revision })).toThrow(CockpitClientError);
      expect(() => parseOrchestrationMutationResponse({ revision, result: { result: "done" } })).toThrow(CockpitClientError);
      expect(() => parseOrchestrationWaitResponse({ revision, tasks_token: hash, changed: false })).toThrow(CockpitClientError);
    }
    for (const [path, bad] of [
      [["messages", 0, "seq"], Number.MAX_SAFE_INTEGER + 1], [["runs", 0, "attempt"], 0x100000000],
      [["tasks_token"], "bad"], [["board", "doc_revision"], "A".repeat(64)], [["runs", 0, "prepare_plan", "plan_revision"], "bad"],
      [["runs", 0, "created_at"], "yesterday"], [["messages", 0, "stale"], 1], [["runs", 0, "session_id"], "other"],
      [["runs", 0, "location", "session_id"], "other"], [["unmanaged_agents", 0, "agent_status"], {}],
    ] as const) expect(() => parseOrchestrationSnapshot(replaceField(snapshot, [...path], bad))).toThrow(CockpitClientError);
    expect(() => matchOrchestrationSnapshot(snapshot, { session_id: "other", root_id: null })).toThrow(CockpitClientError);
    expect(() => matchOrchestrationSnapshot(snapshot, { session_id: "session", root_id: "other" })).toThrow(CockpitClientError);
    expect(parseOrchestrationSnapshot({ ...snapshot, revision: Number.MAX_SAFE_INTEGER }).revision).toBe(Number.MAX_SAFE_INTEGER);
  });

  it("enforces UTF-8 text bounds, not just JavaScript character counts", () => {
    expect(parseOrchestrationAction({ action: "annotate", run_id: "run", text: "é".repeat(8192) })).toBeDefined();
    expect(() => parseOrchestrationAction({ action: "annotate", run_id: "run", text: "é".repeat(8193) })).toThrow(CockpitClientError);
    for (const path of [["messages", 0, "text"], ["runs", 0, "prepare_brief"], ["runs", 0, "work_plan", "text"]]) {
      expect(() => parseOrchestrationSnapshot(replaceField(snapshot, path, "é".repeat(8193)))).toThrow(CockpitClientError);
    }
    expect(parseOrchestrationSnapshot(replaceField(snapshot, ["board", "tasks", 0, "task", "body"], "é".repeat(8193)))).toBeDefined();
    expect(parseOrchestrationSnapshot(replaceField(snapshot, ["runtime", "runs", 0, "workspace_label"], "é".repeat(129)))).toBeDefined();
    expect(() => parseOrchestrationAction({ action: "run_adopt", label: "é".repeat(129) })).toThrow(CockpitClientError);
    expect(() => parseOrchestrationAction({ action: "annotate", run_id: "run", text: "a\0b" })).toThrow(CockpitClientError);
  });

  it("validates every action and rejects missing fields, actor spoofing and unknown fields", () => {
    for (const action of actions) {
      expect(parseOrchestrationAction(action)).toEqual(action);
      expect(parseOrchestrationMutationRequest({ session_id: "session", expected_revision: 2, action }).action).toEqual(action);
      for (const path of fieldPaths(action)) expect(() => parseOrchestrationAction(replaceField(action, path, undefined, true)), `${action.action}:${path.join(".")}`).toThrow(CockpitClientError);
      expect(() => parseOrchestrationAction({ ...action, actor: "operator" })).toThrow(CockpitClientError);
    }
    for (const bad of [null, [], { action: "unknown" }, { action: "__proto__" }]) expect(() => parseOrchestrationAction(bad)).toThrow(CockpitClientError);
    expect(() => parseOrchestrationAction({ action: "inbox_pull", after_seq: 0, limit: 101 })).toThrow(CockpitClientError);
    expect(() => parseOrchestrationAction({ action: "subagent_control", run_id: "run", subagent_id: "child", op: { op: "cancel", text: "extra" } })).toThrow(CockpitClientError);
    expect(() => parseOrchestrationMutationRequest({ session_id: "session", expected_revision: 2, action: actions[0], origin: "native" })).toThrow(CockpitClientError);
    expect(() => parseOrchestrationSnapshotRequest({ session_id: "../session", root_id: null })).toThrow(CockpitClientError);
    expect(() => parseOrchestrationSnapshotRequest({ session_id: "session" })).toThrow(CockpitClientError);
    expect(parseOrchestrationAction({ action: "supervisor_start", target: { target: "existing_space", workspace_id: "space" }, label: null })).toBeDefined();
  });

  it("validates all mutation result variants deeply", () => {
    const results = [{ result: "task", task }, { result: "task_ids", assigned: 1, doc_revision: hash },
      { result: "run", run_id: "run", attempt: 1 }, { result: "message", to_run_id: "run", seq: 1, duplicate: false, stale: true },
      { result: "inbox", messages: [message], read_through_seq: 1 }, { result: "done" }];
    for (const result of results) {
      const response = { revision: 2, result };
      expect(parseOrchestrationMutationResponse(response)).toEqual(response);
      for (const path of fieldPaths(response)) expect(() => parseOrchestrationMutationResponse(replaceField(response, path, undefined, true))).toThrow(CockpitClientError);
    }
    expect(() => parseOrchestrationMutationResponse({ revision: 2, result: { result: "unknown" } })).toThrow(CockpitClientError);
    expect(() => parseOrchestrationMutationResponse({ revision: 2, result: { result: "inbox", messages: [{ ...message, seq: -1 }], read_through_seq: 1 } })).toThrow(CockpitClientError);
  });

  it("validates long-poll timeouts and invalidation tokens", () => {
    const request = { after_revision: 2, after_tasks_token: hash, timeout_ms: 30_000 };
    expect(parseOrchestrationWaitRequest(request)).toEqual(request);
    for (const timeout_ms of [-1, 30_001, 1.5, NaN, "30000"]) expect(() => parseOrchestrationWaitRequest({ ...request, timeout_ms })).toThrow(CockpitClientError);
    for (const path of fieldPaths(request)) expect(() => parseOrchestrationWaitRequest(replaceField(request, path, undefined, true))).toThrow(CockpitClientError);
    expect(() => parseOrchestrationWaitRequest({ ...request, after_revision: Number.MAX_SAFE_INTEGER + 1 })).toThrow(CockpitClientError);
    expect(() => parseOrchestrationWaitResponse({ revision: 2, tasks_token: hash, changed: "yes" })).toThrow(CockpitClientError);
    expect(() => parseOrchestrationWaitResponse({ revision: 2, tasks_token: "bad", changed: false })).toThrow(CockpitClientError);
  });
});

describe("browser/native orchestration parity", () => {
  const snapshotRequest = { session_id: "session", root_id: "root" };
  const mutationRequest = { session_id: "session", expected_revision: 2, action: { action: "cancel_run" as const, run_id: "run" } };
  const waitRequest = { after_revision: 2, after_tasks_token: hash, timeout_ms: 30_000 };
  const mutationResponse = { revision: 3, result: { result: "done" as const } };
  const waitResponse = { revision: 3, tasks_token: hash, changed: true };

  it("uses the exact HTTP paths, verbs and JSON DTOs", async () => {
    const fetch = vi.fn().mockResolvedValueOnce(new Response(JSON.stringify(snapshot)))
      .mockResolvedValueOnce(new Response(JSON.stringify(mutationResponse))).mockResolvedValueOnce(new Response(JSON.stringify(waitResponse)))
      .mockResolvedValueOnce(new Response(JSON.stringify(snapshot)));
    const client = createBrowserClient(fetch);
    expect(await client.orchestrationSnapshot(snapshotRequest)).toEqual(snapshot);
    expect(fetch).toHaveBeenNthCalledWith(1, "/api/v1/sessions/session/orchestration?root_id=root", { headers: { Accept: "application/json" } });
    expect(await client.orchestrationMutate(mutationRequest)).toEqual(mutationResponse);
    expect(fetch).toHaveBeenNthCalledWith(2, "/api/v1/sessions/session/orchestration/mutations", { method: "POST", headers: { Accept: "application/json", "Content-Type": "application/json" }, body: JSON.stringify(mutationRequest) });
    expect(await client.orchestrationWait(waitRequest)).toEqual(waitResponse);
    expect(fetch).toHaveBeenNthCalledWith(3, "/api/v1/orchestration/wait", { method: "POST", headers: { Accept: "application/json", "Content-Type": "application/json" }, body: JSON.stringify(waitRequest) });
    await client.orchestrationSnapshot({ session_id: "session", root_id: null });
    expect(fetch).toHaveBeenNthCalledWith(4, "/api/v1/sessions/session/orchestration", { headers: { Accept: "application/json" } });
  });

  it("invokes the exact native commands with validated request DTOs", async () => {
    const invoke = vi.fn().mockResolvedValueOnce(snapshot).mockResolvedValueOnce(mutationResponse).mockResolvedValueOnce(waitResponse);
    const client = createNativeClient(invoke);
    expect(await client.orchestrationSnapshot(snapshotRequest)).toEqual(snapshot);
    expect(await client.orchestrationMutate(mutationRequest)).toEqual(mutationResponse);
    expect(await client.orchestrationWait(waitRequest)).toEqual(waitResponse);
    expect(invoke.mock.calls).toEqual([["orchestration_snapshot", { request: snapshotRequest }], ["orchestration_mutate", { request: mutationRequest }], ["orchestration_wait", { request: waitRequest }]]);
  });

  it.each(["browser", "native"] as const)("rejects malformed responses and foreign sessions on %s", async transport => {
    for (const badSnapshot of [{ ...snapshot, session_id: "foreign" }, { ...snapshot, runs: [{ ...snapshot.runs[0], grants: [{ origin: "cli" }] }] }]) {
      const client = transport === "browser" ? createBrowserClient(vi.fn(async () => new Response(JSON.stringify(badSnapshot)))) : createNativeClient(vi.fn(async () => badSnapshot));
      await expect(client.orchestrationSnapshot(snapshotRequest)).rejects.toMatchObject({ code: "malformed_response" });
    }
    const malformed = transport === "browser" ? createBrowserClient(vi.fn(async () => new Response('{"revision":9007199254740992,"result":{"result":"done"}}'))) : createNativeClient(vi.fn(async () => ({ revision: Number.MAX_SAFE_INTEGER + 1, result: { result: "done" } })));
    await expect(malformed.orchestrationMutate(mutationRequest)).rejects.toMatchObject({ code: "malformed_response" });
    await expect(malformed.orchestrationWait(waitRequest)).rejects.toMatchObject({ code: "malformed_response" });
  });

  it.each(["browser", "native"] as const)("does not send invalid requests on %s", async transport => {
    const call = vi.fn();
    const client = transport === "browser" ? createBrowserClient(call) : createNativeClient(call);
    await expect(client.orchestrationSnapshot({ ...snapshotRequest, session_id: "../session" })).rejects.toMatchObject({ code: "malformed_response" });
    await expect(client.orchestrationMutate({ ...mutationRequest, expected_revision: Number.MAX_SAFE_INTEGER + 1 })).rejects.toMatchObject({ code: "malformed_response" });
    await expect(client.orchestrationWait({ ...waitRequest, timeout_ms: 30_001 })).rejects.toMatchObject({ code: "malformed_response" });
    expect(call).not.toHaveBeenCalled();
  });
});
