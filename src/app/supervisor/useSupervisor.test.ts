// @vitest-environment jsdom
import { act, createElement } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, describe, expect, it, vi } from "vitest";
import { CockpitClientError, type CockpitClient } from "../../client/CockpitClient";
import type { OrchestrationAction, OrchestrationActionResult, OrchestrationMutationResponse, OrchestrationSnapshot, OrchestrationWaitResponse, Run, Task } from "../../protocol/generated/v1";
import { acceptsSupervisorSnapshot, useSupervisor, type SourceSubmission, type SourceResolution, type SourceResolutionOutcome, type TaskMutationOutcome } from "./useSupervisor";
import type { StepScope, StepSubmission, StepUnknownResolution, StepResolutionOutcome, StepReadOutcome } from "./stepInteractions";

const snapshot: OrchestrationSnapshot = {
  session_id: "session-a", revision: 12, tasks_token: "external-edit-a",
  roots: [], board: { root_id: "root-a", path: "/state/tasks/root-a.md", doc_revision: "doc-a", unidentified_items: 0, diagnostics: [], tasks: [] },
  runs: [], messages: [], subagents: [], intents: [], assignment_intents: [], attention: [], unmanaged_agents: [],
  runtime: { status: "unavailable", error: { code: "herdr_unavailable", message: "offline" } },
};

describe("supervisor snapshot fences", () => {
  it("rejects an old machine revision after a successful mutation", () => {
    expect(acceptsSupervisorSnapshot(snapshot, "session-a", "root-a", 13)).toBe(false);
    expect(acceptsSupervisorSnapshot({ ...snapshot, revision: 13 }, "session-a", "root-a", 13)).toBe(true);
  });
  it("rejects late responses from another session or selected root", () => {
    expect(acceptsSupervisorSnapshot(snapshot, "session-b", "root-a", 0)).toBe(false);
    expect(acceptsSupervisorSnapshot(snapshot, "session-a", "root-b", 0)).toBe(false);
  });
  it("accepts external Markdown changes without requiring a machine revision increment", () => {
    expect(acceptsSupervisorSnapshot({ ...snapshot, tasks_token: "external-edit-b", board: { ...snapshot.board!, doc_revision: "doc-b" } }, "session-a", "root-a", 12)).toBe(true);
  });
  it("accepts a missing board as absence of canonical tasks, not missing runtime", () => {
    expect(acceptsSupervisorSnapshot({ ...snapshot, board: null }, "session-a", "root-a", 12)).toBe(true);
  });
});

const at = "2026-10-07T12:00:00Z";
function reportedRun(kind: "needs_input" | "result"): Run {
  const report = { message_id: `${kind}-receipt`, kind, outcome: kind === "result" ? "succeeded" as const : null, summary: `${kind}-summary`, plan: null, at };
  return {
    session_id: "session-a", prepare_brief: "", run_id: "worker", kind: "worker", label: "Worker", root_id: "root-a", parent_run_id: "root-a",
    task_id: "task-a", attempt: 1, task_revision_at_propose: null, stage: "reported", close_reason: null,
    dispatch: null, target: null, setup: null, prepare_plan: null, init_receipt: null, work_plan: null, grants: [],
    last_report: report, result: kind === "result" ? report : null, annotations: [], location: null,
    bound_omp_session: null, bound_omp_process: null, launch_shell_identity: null, retirement: null, supersedes_run_id: null, created_at: at, updated_at: at,
  };
}
function observedState(overrides: Partial<OrchestrationSnapshot> = {}): OrchestrationSnapshot {
  return {
    ...snapshot,
    runtime: { status: "fresh", endpoint_identity: "endpoint", observed_at: at, runs: [{
      run_id: "worker", presence: "present", actual_omp: true, workspace_id: "space", workspace_label: "Project",
      tab_id: "tab", tab_label: "Agent", pane_id: "pane", agent_status: "working", state_changed_at: at,
    }] },
    ...overrides,
  };
}
interface Deferred<T> {
  promise: Promise<T>;
  resolve: (value: T) => void;
  reject: (reason: unknown) => void;
}
function deferred<T>(): Deferred<T> {
  let resolve!: (value: T) => void;
  let reject!: (reason: unknown) => void;
  const promise = new Promise<T>((done, fail) => { resolve = done; reject = fail; });
  return { promise, resolve, reject };
}
function transport() {
  const snapshots: Deferred<OrchestrationSnapshot>[] = [];
  const waits: Deferred<OrchestrationWaitResponse>[] = [];
  const mutations: Deferred<OrchestrationMutationResponse>[] = [];
  const client = {
    orchestrationSnapshot: vi.fn(() => { const pending = deferred<OrchestrationSnapshot>(); snapshots.push(pending); return pending.promise; }),
    orchestrationWait: vi.fn(() => { const pending = deferred<OrchestrationWaitResponse>(); waits.push(pending); return pending.promise; }),
    orchestrationMutate: vi.fn(() => { const pending = deferred<OrchestrationMutationResponse>(); mutations.push(pending); return pending.promise; }),
  } as unknown as CockpitClient;
  const take = <T,>(queue: Deferred<T>[]) => {
    const pending = queue.shift();
    if (!pending) throw new Error("No pending transport response");
    return pending;
  };
  return { client, snapshot: () => take(snapshots), wait: () => take(waits), mutation: () => take(mutations) };
}
type Scope = { client: CockpitClient; sessionId: string; rootId: string | null; active: boolean };
interface VisibleState {
  snapshot: OrchestrationSnapshot | null;
  connected: boolean;
  error: string | null;
  busy: boolean;
}
interface HookState extends VisibleState {
  mutateResult: (action: OrchestrationAction) => Promise<OrchestrationActionResult | null>;
  submitStep: (submitted: StepSubmission) => Promise<TaskMutationOutcome<StepSubmission>>;
  submitTask: (submitted: SourceSubmission) => Promise<TaskMutationOutcome<SourceSubmission>>;
  readSaved: (scope: StepScope) => Promise<StepReadOutcome>;
  resolveUnknown: (request: StepUnknownResolution) => Promise<StepResolutionOutcome>;
  resolveSourceUnknown: (request: SourceResolution) => Promise<SourceResolutionOutcome>;
  taskWriteUnconfirmed: (scope: StepScope) => boolean;
}
const mounted: { root: Root; host: HTMLDivElement; unmounted: boolean }[] = [];
async function flush(change: () => void = () => {}) {
  await act(async () => {
    change();
    for (let index = 0; index < 8; index++) await Promise.resolve();
  });
}
async function mount(client: CockpitClient) {
  const host = document.createElement("div");
  document.body.append(host);
  const entry = { root: createRoot(host), host, unmounted: false };
  mounted.push(entry);
  let scope: Scope = { client, sessionId: "session-a", rootId: "root-a", active: true };
  let accepted!: HookState;
  let lastVisible!: VisibleState;
  function Consumer(props: Scope) {
    accepted = useSupervisor(props.client, props.sessionId, props.rootId, props.active);
    lastVisible = { snapshot: accepted.snapshot, connected: accepted.connected, error: accepted.error, busy: accepted.busy };
    return createElement("output", null, JSON.stringify(lastVisible));
  }
  const render = async (change: Partial<Scope> = {}) => {
    scope = { ...scope, ...change };
    await flush(() => entry.root.render(createElement(Consumer, scope)));
  };
  await render();
  return {
    render,
    state: () => JSON.parse(host.querySelector("output")!.textContent!) as VisibleState,
    lastVisible: () => lastVisible,
    hookState: () => accepted,
    mutate: (action: OrchestrationAction) => accepted.mutateResult(action),
    unmount: async () => { await flush(() => entry.root.unmount()); entry.unmounted = true; },
  };
}
afterEach(async () => {
  for (const entry of mounted.splice(0)) {
    if (!entry.unmounted) await flush(() => entry.root.unmount());
    entry.host.remove();
  }
  vi.useRealTimers();
  vi.restoreAllMocks();
});

describe("mounted Supervisor observation", () => {
  it("refreshes runtime-only changes after an unchanged durable wait", async () => {
    vi.useFakeTimers();
    const api = transport(), consumer = await mount(api.client);
    const initial = observedState();
    await flush(() => api.snapshot().resolve(initial));
    expect(consumer.state().snapshot).toEqual(initial);
    const next = observedState({
      runtime: { status: "fresh", endpoint_identity: "endpoint", observed_at: "2026-10-07T12:00:05Z", runs: [{
        run_id: "worker", presence: "missing", actual_omp: false, workspace_id: null, workspace_label: null,
        tab_id: null, tab_label: null, pane_id: null, agent_status: null, state_changed_at: at,
      }] },
    });
    await flush(() => api.wait().resolve({ revision: initial.revision, tasks_token: initial.tasks_token, changed: false }));
    await flush(() => api.snapshot().resolve(next));
    expect(consumer.state()).toEqual({ snapshot: next, connected: true, error: null, busy: false });
    expect(consumer.state().snapshot?.revision).toBe(initial.revision);
  });

  it.each(["needs_input", "result"] as const)("renders a durable %s and new board immediately on a changed wait", async kind => {
    vi.useFakeTimers();
    const api = transport(), consumer = await mount(api.client);
    const initial = observedState();
    await flush(() => api.snapshot().resolve(initial));
    const next = observedState({
      revision: 13, tasks_token: "external-edit-b", runs: [reportedRun(kind)],
      attention: [{ kind: kind === "needs_input" ? "needs_input" : "to_accept", run_id: "worker", task_id: "task-a", message_seq: 1, since: at }],
      board: { ...snapshot.board!, doc_revision: "doc-b", tasks: [{
        task: { task_id: "task-a", title: "Deliver work", body: "Canonical task", description: "Canonical task", description_editable: true, description_diagnostic: null, steps: [], step_progress: { done: 0, total: 0 }, steps_diagnostic: null, depends_on: [], follow_up_of: null, relations_diagnostic: null, checked: false, line: 1, task_revision: "task-b", diagnostic: null },
        lane: kind === "needs_input" ? "working" : "review", current_run_id: "worker", dependencies: { state: "none", unmet: [], problems: [] },
      }] },
    });
    await flush(() => api.wait().resolve({ revision: next.revision, tasks_token: next.tasks_token, changed: true }));
    // No fake clock advance: neither a changed wait nor the following snapshot adds a sleep.
    await flush(() => api.snapshot().resolve(next));
    expect(consumer.state().snapshot).toEqual(next);
    expect(consumer.state().snapshot?.runs[0].last_report?.kind).toBe(kind);
    expect(consumer.state().snapshot?.board?.tasks[0].task.task_revision).toBe("task-b");
  });

  it.each(["root", "client", "session"] as const)("ignores a prior %s snapshot without poisoning the new revision floor", async changed => {
    const api = transport(), replacement = transport(), consumer = await mount(api.client);
    const late = api.snapshot();
    const nextApi = changed === "client" ? replacement : api;
    const newSession = changed === "session" ? "session-b" : "session-a";
    const newRoot = changed === "root" ? "root-b" : "root-a";
    await consumer.render({ client: nextApi.client, sessionId: newSession, rootId: newRoot });
    const next = observedState({ session_id: newSession, board: { ...snapshot.board!, root_id: newRoot } });
    await flush(() => nextApi.snapshot().resolve(next));
    await flush(() => late.resolve(observedState({ revision: 900 })));
    expect(consumer.state().snapshot).toEqual(next);
    await flush(() => nextApi.wait().resolve({ revision: 13, tasks_token: next.tasks_token, changed: true }));
    const newer = { ...next, revision: 13 };
    await flush(() => nextApi.snapshot().resolve(newer));
    expect(consumer.state().snapshot).toEqual(newer);
  });

  it.each(["root", "client", "session"] as const)("ignores a prior %s wait without poisoning the new revision floor", async changed => {
    const api = transport(), replacement = transport(), consumer = await mount(api.client);
    await flush(() => api.snapshot().resolve(observedState()));
    const late = api.wait();
    const nextApi = changed === "client" ? replacement : api;
    const newSession = changed === "session" ? "session-b" : "session-a";
    const newRoot = changed === "root" ? "root-b" : "root-a";
    await consumer.render({ client: nextApi.client, sessionId: newSession, rootId: newRoot });
    const next = observedState({ session_id: newSession, board: { ...snapshot.board!, root_id: newRoot } });
    await flush(() => nextApi.snapshot().resolve(next));
    await flush(() => late.resolve({ revision: 900, tasks_token: "old-scope", changed: true }));
    expect(consumer.state().snapshot).toEqual(next);
    await flush(() => nextApi.wait().resolve({ revision: 13, tasks_token: next.tasks_token, changed: true }));
    const newer = { ...next, revision: 13 };
    await flush(() => nextApi.snapshot().resolve(newer));
    expect(consumer.state().snapshot).toEqual(newer);
  });

  it.each(["snapshot", "wait"] as const)("ignores an inactive generation's late %s and resumes fresh observations", async response => {
    const api = transport(), consumer = await mount(api.client);
    const initial = observedState();
    if (response === "wait") await flush(() => api.snapshot().resolve(initial));
    const lateSnapshot = response === "snapshot" ? api.snapshot() : null;
    const lateWait = response === "wait" ? api.wait() : null;
    await consumer.render({ active: false });
    const inactive = consumer.state();
    await flush(() => {
      lateSnapshot?.resolve(observedState({ revision: 900 }));
      lateWait?.resolve({ revision: 900, tasks_token: "inactive", changed: true });
    });
    expect(consumer.state()).toEqual(inactive);
    expect(consumer.state().connected).toBe(false);
    await consumer.render({ active: true });
    await flush(() => api.snapshot().resolve(initial));
    expect(consumer.state()).toEqual({ snapshot: initial, connected: true, error: null, busy: false });
  });

  it.each(["snapshot", "wait"] as const)("ignores an unmounted generation's late %s", async response => {
    const api = transport(), consumer = await mount(api.client);
    if (response === "wait") await flush(() => api.snapshot().resolve(observedState()));
    const lateSnapshot = response === "snapshot" ? api.snapshot() : null;
    const lateWait = response === "wait" ? api.wait() : null;
    const accepted = consumer.lastVisible();
    await consumer.unmount();
    await flush(() => {
      lateSnapshot?.resolve(observedState({ revision: 900 }));
      lateWait?.resolve({ revision: 900, tasks_token: "unmounted", changed: true });
    });
    expect(consumer.lastVisible()).toBe(accepted);
    const replacement = await mount(api.client);
    const next = observedState();
    await flush(() => api.snapshot().resolve(next));
    expect(replacement.state().snapshot).toEqual(next);
  });

  it("rejects wrong identity and regressed observations while retaining the last accepted state", async () => {
    vi.useFakeTimers();
    const api = transport(), consumer = await mount(api.client);
    const initial = observedState();
    await flush(() => api.snapshot().resolve(initial));
    await flush(() => api.wait().resolve({ revision: 13, tasks_token: "new-board", changed: true }));
    for (const invalid of [
      observedState({ revision: 12 }),
      observedState({ revision: 13, session_id: "session-b" }),
      observedState({ revision: 13, board: { ...snapshot.board!, root_id: "root-b" } }),
    ]) {
      await flush(() => api.snapshot().resolve(invalid));
      expect(consumer.state().snapshot).toEqual(initial);
      await act(async () => { await vi.advanceTimersByTimeAsync(1500); });
    }
    const next = observedState({ revision: 13, tasks_token: "new-board", board: null });
    await flush(() => api.snapshot().resolve(next));
    expect(consumer.state().snapshot).toEqual(next);
  });

  it.each(["snapshot", "wait"] as const)("exposes a failed %s and recovers through the existing retry policy", async response => {
    vi.useFakeTimers();
    const api = transport(), consumer = await mount(api.client);
    const initial = observedState();
    if (response === "wait") await flush(() => api.snapshot().resolve(initial));
    const failure = new Error("Transport disconnected");
    await flush(() => response === "snapshot" ? api.snapshot().reject(failure) : api.wait().reject(failure));
    expect(consumer.state()).toEqual({
      snapshot: response === "wait" ? initial : null, connected: false, error: failure.message, busy: false,
    });
    await act(async () => { await vi.advanceTimersByTimeAsync(1499); });
    expect(consumer.state().connected).toBe(false);
    expect(consumer.state().error).toBe(failure.message);
    await act(async () => { await vi.advanceTimersByTimeAsync(1); });
    const recovered = observedState({ revision: 13 });
    await flush(() => api.snapshot().resolve(recovered));
    expect(consumer.state()).toEqual({ snapshot: recovered, connected: true, error: null, busy: false });
  });
});

describe("mounted Supervisor mutations", () => {
  const action: OrchestrationAction = { action: "cancel_run", run_id: "worker" };
  it.each(["success", "failure"] as const)("refreshes immediately on mutation %s and supersedes an old wait", async outcome => {
    vi.useFakeTimers();
    const api = transport(), consumer = await mount(api.client);
    const initial = observedState();
    await flush(() => api.snapshot().resolve(initial));
    const oldWait = api.wait();
    let result!: Promise<OrchestrationActionResult | null>;
    await flush(() => { result = consumer.mutate(action); });
    expect(consumer.state().busy).toBe(true);
    const blocked = await consumer.mutate(action);
    expect(blocked).toBeNull();
    expect(consumer.state().busy).toBe(true);
    const failure = new Error("Action outcome unavailable");
    await flush(() => {
      const mutation = api.mutation();
      if (outcome === "success") mutation.resolve({ revision: 13, result: { result: "done" } });
      else mutation.reject(failure);
    });
    expect(await result).toEqual(outcome === "success" ? { result: "done" } : null);
    expect(consumer.state().busy).toBe(false);
    expect(consumer.state().error).toBe(outcome === "failure" ? failure.message : null);
    // The refresh is already awaiting its snapshot while the old wait remains unresolved.
    const next = observedState({ revision: 13, board: { ...snapshot.board!, doc_revision: "after-mutation" } });
    await flush(() => api.snapshot().resolve(next));
    expect(consumer.state().snapshot).toEqual(next);
    await flush(() => oldWait.resolve({ revision: 900, tasks_token: "stale-wait", changed: true }));
    await flush(() => api.wait().resolve({ revision: 14, tasks_token: next.tasks_token, changed: true }));
    const newer = { ...next, revision: 14 };
    await flush(() => api.snapshot().resolve(newer));
    expect(consumer.state().snapshot).toEqual(newer);
    if (outcome === "failure") {
      // A healthy observation cannot silently erase an unconfirmed action; an explicit retry can.
      expect(consumer.state().error).toBe(failure.message);
      await flush(() => { result = consumer.mutate(action); });
      expect(consumer.state().error).toBeNull();
      await flush(() => api.mutation().resolve({ revision: 15, result: { result: "done" } }));
      expect(await result).toEqual({ result: "done" });
      const retried = { ...newer, revision: 15 };
      await flush(() => api.snapshot().resolve(retried));
      expect(consumer.state()).toEqual({ snapshot: retried, connected: true, error: null, busy: false });
    }
  });

  it("keeps the mutation revision floor when refresh returns an older snapshot", async () => {
    vi.useFakeTimers();
    const api = transport(), consumer = await mount(api.client);
    const initial = observedState();
    await flush(() => api.snapshot().resolve(initial));
    let result!: Promise<OrchestrationActionResult | null>;
    await flush(() => { result = consumer.mutate(action); });
    await flush(() => api.mutation().resolve({ revision: 14, result: { result: "done" } }));
    expect(await result).toEqual({ result: "done" });
    await flush(() => api.snapshot().resolve(observedState({ revision: 13 })));
    expect(consumer.state().snapshot).toEqual(initial);
    await act(async () => { await vi.advanceTimersByTimeAsync(1500); });
    const current = observedState({ revision: 14 });
    await flush(() => api.snapshot().resolve(current));
    expect(consumer.state().snapshot).toEqual(current);
  });
});

const taskScope: StepScope = { sessionId: "session-a", rootId: "root-a", taskId: "task-a" };
function savedTask(taskId = "task-a", revision = "task-original"): Task {
  return {
    task_id: taskId, title: `Task ${taskId}`, body: "Canonical source", description: "Original description",
    description_editable: true, description_diagnostic: null, steps: [], step_progress: { done: 0, total: 0 },
    steps_diagnostic: null, depends_on: [], follow_up_of: null, relations_diagnostic: null,
    checked: false, line: 1, task_revision: revision, diagnostic: null,
  };
}
function taskSnapshot(tasks: Task[] = [savedTask(), savedTask("task-b")], rootId = "root-a"): OrchestrationSnapshot {
  return observedState({ board: {
    ...snapshot.board!, root_id: rootId, path: `/state/tasks/${rootId}.md`,
    tasks: tasks.map(task => ({ task, lane: "queued", current_run_id: null, dependencies: { state: "none", unmet: [], problems: [] } })),
  } });
}
function stepSubmission(scope: StepScope = taskScope, submissionId = "step-original", revision = "task-original"): StepSubmission {
  return { submissionId, scope, expectedTaskRevision: revision, intent: { kind: "rename", stepId: "step-a", title: "Retained step title" } };
}
function sourceSubmission(scope: StepScope = taskScope, submissionId = "source-original", revision = "task-original", description = "Retained description"): SourceSubmission {
  return { submissionId, scope, action: { action: "task_update", root_id: scope.rootId, task_id: scope.taskId,
    expected_task_revision: revision, title: null, description } };
}
function resolution(original: StepSubmission, revision = "task-reviewed"): StepUnknownResolution {
  return { originalScope: original.scope, originalSubmissionId: original.submissionId, originalSubmitted: original,
    reviewedTaskRevision: revision, decision: { kind: "use_saved" } };
}
async function mountedTasks() {
  vi.useFakeTimers();
  const api = transport(), consumer = await mount(api.client);
  await flush(() => api.snapshot().resolve(taskSnapshot()));
  return { api, consumer };
}
interface TaskFixture {
  api: { client: CockpitClient; snapshot: () => Deferred<OrchestrationSnapshot>; mutation: () => Deferred<OrchestrationMutationResponse> };
  consumer: { hookState: () => HookState };
}
async function makeUnknown({ api, consumer }: TaskFixture, submitted: StepSubmission | SourceSubmission) {
  let result!: Promise<TaskMutationOutcome<StepSubmission | SourceSubmission>>;
  await flush(() => {
    result = "action" in submitted ? consumer.hookState().submitTask(submitted) : consumer.hookState().submitStep(submitted);
  });
  await flush(() => api.mutation().reject(new Error("Connection lost after submission")));
  expect(await result).toEqual({ kind: "unknown", submitted, message: "Connection lost after submission" });
  expect(consumer.hookState().taskWriteUnconfirmed(submitted.scope)).toBe(true);
}
async function readOriginal({ api, consumer }: TaskFixture, scope = taskScope, revision = "task-reviewed") {
  let result!: Promise<StepReadOutcome>;
  await flush(() => { result = consumer.hookState().readSaved(scope); });
  expect(api.client.orchestrationSnapshot).toHaveBeenLastCalledWith({ session_id: scope.sessionId, root_id: scope.rootId });
  const task = savedTask(scope.taskId, revision);
  await flush(() => api.snapshot().resolve({ ...taskSnapshot([task], scope.rootId), session_id: scope.sessionId }));
  expect(await result).toEqual({ kind: "found", task });
  return task;
}

describe("mounted Supervisor task uncertainty", () => {
  it("returns not_sent while busy before invoking transport for either task writer", async () => {
    const { api, consumer } = await mountedTasks();
    let pending!: Promise<OrchestrationActionResult | null>;
    await flush(() => { pending = consumer.mutate({ action: "cancel_run", run_id: "worker" }); });
    expect(consumer.state().busy).toBe(true);
    expect(await consumer.hookState().submitStep(stepSubmission())).toEqual({ kind: "not_sent", reason: "Wait for the current operation." });
    expect(await consumer.hookState().submitTask(sourceSubmission())).toEqual({ kind: "not_sent", reason: "Wait for the current operation." });
    expect(api.client.orchestrationMutate).toHaveBeenCalledTimes(1);
    expect(consumer.hookState().taskWriteUnconfirmed(taskScope)).toBe(false);
    await flush(() => api.mutation().resolve({ revision: 13, result: { result: "done" } }));
    expect(await pending).toEqual({ result: "done" });
  });

  it.each([
    "actor_forbidden", "invalid_stage", "attempt_stale", "session_mismatch", "caller_mismatch", "caller_unbound",
    "task_not_found", "task_id_duplicate", "task_checked", "intent_conflict", "task_revision_conflict",
    "task_dependencies_invalid", "task_blocked", "invalid_task", "task_description_ambiguous", "task_relations_invalid",
    "tasks_full", "task_id_conflict",
  ])("classifies whitelisted %s as refused without installing an uncertainty gate", async operationCode => {
    const { api, consumer } = await mountedTasks();
    let result!: Promise<TaskMutationOutcome<StepSubmission>>;
    await flush(() => { result = consumer.hookState().submitStep(stepSubmission()); });
    await flush(() => api.mutation().reject(new CockpitClientError("http_error", "Rejected before publication", { operationCode })));
    expect(await result).toEqual({ kind: "refused", operationCode, message: "Rejected before publication" });
    expect(consumer.hookState().taskWriteUnconfirmed(taskScope)).toBe(false);
    expect(consumer.state().busy).toBe(false);
  });

  it.each([
    new CockpitClientError("http_error", "Publication could have occurred", { operationCode: "io_error" }),
    new CockpitClientError("http_error", "New code is not a refusal guarantee", { operationCode: "task_invalid" }),
    new Error("task_revision_conflict"),
  ])("retains unknown for untrusted or non-whitelisted failure $message", async failure => {
    const { api, consumer } = await mountedTasks(), submitted = stepSubmission();
    let result!: Promise<TaskMutationOutcome<StepSubmission>>;
    await flush(() => { result = consumer.hookState().submitStep(submitted); });
    await flush(() => api.mutation().reject(failure));
    expect(await result).toEqual({ kind: "unknown", submitted, message: failure.message });
    expect(consumer.hookState().taskWriteUnconfirmed(taskScope)).toBe(true);
    expect(consumer.state().busy).toBe(false);
  });

  it.each(["steps", "description"] as const)("gates both writers for an unknown %s operation but permits an unrelated task", async writer => {
    const fixture = await mountedTasks(), { api, consumer } = fixture;
    await makeUnknown(fixture, writer === "steps" ? stepSubmission() : sourceSubmission());
    expect(await consumer.hookState().submitStep(stepSubmission(taskScope, "another-step"))).toMatchObject({ kind: "not_sent" });
    expect(await consumer.hookState().submitTask(sourceSubmission(taskScope, "another-description"))).toMatchObject({ kind: "not_sent" });
    expect(api.client.orchestrationMutate).toHaveBeenCalledTimes(1);
    const otherScope = { ...taskScope, taskId: "task-b" }, other = sourceSubmission(otherScope), saved = savedTask("task-b", "other-confirmed");
    let result!: Promise<TaskMutationOutcome<SourceSubmission>>;
    await flush(() => { result = consumer.hookState().submitTask(other); });
    expect(api.client.orchestrationMutate).toHaveBeenLastCalledWith({ session_id: "session-a", expected_revision: null, action: other.action });
    await flush(() => api.mutation().resolve({ revision: 13, result: { result: "task", task: saved } }));
    expect(await result).toEqual({ kind: "confirmed", task: saved });
    expect(consumer.hookState().taskWriteUnconfirmed(taskScope)).toBe(true);
    expect(consumer.hookState().taskWriteUnconfirmed(otherScope)).toBe(false);
  });

  it("does not treat successful polling or a saved-task read as explicit resolution", async () => {
    const fixture = await mountedTasks(), { api, consumer } = fixture, original = stepSubmission();
    await makeUnknown(fixture, original);
    const polled = taskSnapshot([savedTask("task-a", "task-reviewed")]);
    await flush(() => api.wait().resolve({ revision: 12, tasks_token: "external-edit-b", changed: true }));
    await flush(() => api.snapshot().resolve(polled));
    expect(consumer.state().snapshot).toEqual(polled);
    expect(consumer.hookState().taskWriteUnconfirmed(taskScope)).toBe(true);
    expect(await consumer.hookState().resolveUnknown(resolution(original))).toMatchObject({ kind: "not_resolved", code: "read_required" });
    await readOriginal(fixture);
    expect(consumer.hookState().taskWriteUnconfirmed(taskScope)).toBe(true);
    expect(await consumer.hookState().submitTask(sourceSubmission())).toMatchObject({ kind: "not_sent" });
    expect(api.client.orchestrationMutate).toHaveBeenCalledTimes(1);
  });

  it.each(["scope", "id", "payload"] as const)("refuses a mismatched original %s without clearing the retained gate", async mismatch => {
    const fixture = await mountedTasks(), { api, consumer } = fixture, original = stepSubmission();
    await makeUnknown(fixture, original);
    await readOriginal(fixture);
    const request = resolution(original);
    const invalid: StepUnknownResolution = mismatch === "scope"
      ? { ...request, originalScope: { ...taskScope, taskId: "task-b" } }
      : mismatch === "id" ? { ...request, originalSubmissionId: "another-submission" }
      : { ...request, originalSubmitted: { ...original, intent: { kind: "rename", stepId: "step-a", title: "Different payload" } } };
    expect(await consumer.hookState().resolveUnknown(invalid)).toMatchObject({ kind: "not_resolved", code: "different_source_operation" });
    expect(consumer.hookState().taskWriteUnconfirmed(taskScope)).toBe(true);
    expect(api.client.orchestrationMutate).toHaveBeenCalledTimes(1);
  });

  it("refuses step resolution of a different SOURCE unknown even with matching scope and submission id", async () => {
    const fixture = await mountedTasks(), { api, consumer } = fixture, source = sourceSubmission();
    await makeUnknown(fixture, source);
    await readOriginal(fixture);
    expect(await consumer.hookState().resolveUnknown(resolution(stepSubmission(taskScope, source.submissionId))))
      .toMatchObject({ kind: "not_resolved", code: "different_source_operation" });
    expect(consumer.hookState().taskWriteUnconfirmed(taskScope)).toBe(true);
    expect(api.client.orchestrationMutate).toHaveBeenCalledTimes(1);
  });

  it("requires the original read revision and explicit use_saved, clearing only that exact operation", async () => {
    const fixture = await mountedTasks(), { api, consumer } = fixture, original = stepSubmission();
    const otherScope = { ...taskScope, taskId: "task-b" }, other = stepSubmission(otherScope, "other-original");
    await makeUnknown(fixture, original);
    await makeUnknown(fixture, other);
    // Reading a different task cannot provide evidence for the original operation.
    await readOriginal(fixture, otherScope, "other-reviewed");
    expect(await consumer.hookState().resolveUnknown(resolution(original))).toMatchObject({ kind: "not_resolved", code: "read_required" });
    const saved = await readOriginal(fixture);
    expect(await consumer.hookState().resolveUnknown(resolution(original, "task-original"))).toMatchObject({ kind: "not_resolved", code: "resolution_stale" });
    expect(consumer.hookState().taskWriteUnconfirmed(taskScope)).toBe(true);
    let resolved!: Promise<StepResolutionOutcome>;
    await flush(() => { resolved = consumer.hookState().resolveUnknown(resolution(original)); });
    expect(await resolved).toEqual({ kind: "resolved", originalScope: taskScope, originalSubmissionId: original.submissionId, task: saved });
    expect(consumer.hookState().taskWriteUnconfirmed(taskScope)).toBe(false);
    expect(consumer.hookState().taskWriteUnconfirmed(otherScope)).toBe(true);
    expect(api.client.orchestrationMutate).toHaveBeenCalledTimes(2);
  });

  it("leaves the original gate intact when ApplyReviewed is busy", async () => {
    const fixture = await mountedTasks(), { api, consumer } = fixture, original = stepSubmission();
    await makeUnknown(fixture, original);
    await readOriginal(fixture);
    const reviewed = stepSubmission(taskScope, "reviewed-submission", "task-reviewed");
    let pending!: Promise<OrchestrationActionResult | null>;
    await flush(() => { pending = consumer.mutate({ action: "cancel_run", run_id: "worker" }); });
    expect(await consumer.hookState().resolveUnknown({ ...resolution(original), decision: { kind: "apply_reviewed", submitted: reviewed } }))
      .toMatchObject({ kind: "not_resolved", code: "busy" });
    expect(consumer.hookState().taskWriteUnconfirmed(taskScope)).toBe(true);
    expect(api.client.orchestrationMutate).toHaveBeenCalledTimes(2);
    await flush(() => api.mutation().resolve({ revision: 13, result: { result: "done" } }));
    await pending;
    let resolved!: Promise<StepResolutionOutcome>;
    await flush(() => { resolved = consumer.hookState().resolveUnknown(resolution(original)); });
    expect(await resolved).toMatchObject({ kind: "resolved", originalSubmissionId: original.submissionId });
  });

  it("records a late submitted outcome as unknown on its original root, not the newly selected task", async () => {
    const { api, consumer } = await mountedTasks(), original = stepSubmission();
    let result!: Promise<TaskMutationOutcome<StepSubmission>>;
    await flush(() => { result = consumer.hookState().submitStep(original); });
    const mutation = api.mutation();
    await consumer.render({ rootId: "root-b" });
    const newScope = { ...taskScope, rootId: "root-b", taskId: "task-b" }, next = taskSnapshot([savedTask("task-b")], "root-b");
    await flush(() => api.snapshot().resolve(next));
    await flush(() => mutation.resolve({ revision: 99, result: { result: "task", task: savedTask("task-a", "late-confirmed") } }));
    expect(await result).toMatchObject({ kind: "unknown", submitted: original });
    expect(consumer.state().snapshot).toEqual(next);
    expect(consumer.state().error).toBeNull();
    expect(consumer.hookState().taskWriteUnconfirmed(taskScope)).toBe(true);
    expect(consumer.hookState().taskWriteUnconfirmed(newScope)).toBe(false);
    const other = sourceSubmission(newScope), saved = savedTask("task-b", "new-root-confirmed");
    let otherResult!: Promise<TaskMutationOutcome<SourceSubmission>>;
    await flush(() => { otherResult = consumer.hookState().submitTask(other); });
    await flush(() => api.mutation().resolve({ revision: 13, result: { result: "task", task: saved } }));
    expect(await otherResult).toEqual({ kind: "confirmed", task: saved });
    // Drain the new root's post-mutation observation before the independent original-task read.
    await flush(() => api.snapshot().resolve({ ...next, revision: 13 }));
    await readOriginal({ api, consumer });
    let resolved!: Promise<StepResolutionOutcome>;
    await flush(() => { resolved = consumer.hookState().resolveUnknown(resolution(original)); });
    expect(await resolved).toMatchObject({ kind: "resolved" });
    expect(consumer.state().snapshot).toEqual({ ...next, revision: 13 });
  });

  it("applies reviewed steps only to the original read revision and retains a new unknown replacement payload", async () => {
    const fixture = await mountedTasks(), { api, consumer } = fixture, original = stepSubmission();
    await makeUnknown(fixture, original);
    await readOriginal(fixture);
    const reviewed: StepSubmission = { ...stepSubmission(taskScope, "reviewed-submission", "task-reviewed"),
      intent: { kind: "rename", stepId: "step-a", title: "Explicitly reviewed replacement" } };
    const request = { ...resolution(original), decision: { kind: "apply_reviewed" as const, submitted: reviewed } };
    expect(await consumer.hookState().resolveUnknown({ ...request, decision: { kind: "apply_reviewed", submitted: { ...reviewed, expectedTaskRevision: "task-original" } } }))
      .toMatchObject({ kind: "not_resolved", code: "resolution_stale" });
    expect(await consumer.hookState().resolveUnknown({ ...request, decision: { kind: "apply_reviewed", submitted: { ...reviewed, scope: { ...taskScope, rootId: "root-b" } } } }))
      .toMatchObject({ kind: "not_resolved", code: "resolution_stale" });
    expect(api.client.orchestrationMutate).toHaveBeenCalledTimes(1);
    let result!: Promise<StepResolutionOutcome>;
    await flush(() => { result = consumer.hookState().resolveUnknown(request); });
    expect(api.client.orchestrationMutate).toHaveBeenLastCalledWith({ session_id: "session-a", expected_revision: null,
      action: { action: "task_step_rename", root_id: "root-a", task_id: "task-a", expected_task_revision: "task-reviewed",
        step_id: "step-a", title: "Explicitly reviewed replacement" } });
    await flush(() => api.mutation().reject(new Error("Replacement confirmation lost")));
    expect(await result).toEqual({ kind: "applied", originalScope: taskScope, originalSubmissionId: original.submissionId,
      submitted: reviewed, outcome: { kind: "unknown", submitted: reviewed, message: "Replacement confirmation lost" } });
    expect(consumer.hookState().taskWriteUnconfirmed(taskScope)).toBe(true);
    expect(await consumer.hookState().resolveUnknown(resolution(original))).toMatchObject({ kind: "not_resolved", code: "different_source_operation" });
    expect(await consumer.hookState().resolveUnknown(resolution(reviewed))).toMatchObject({ kind: "not_resolved", code: "read_required" });
    await readOriginal(fixture, taskScope, "replacement-reviewed");
    let resolved!: Promise<StepResolutionOutcome>;
    await flush(() => { resolved = consumer.hookState().resolveUnknown(resolution(reviewed, "replacement-reviewed")); });
    expect(await resolved).toMatchObject({ kind: "resolved", originalSubmissionId: reviewed.submissionId });
    expect(consumer.hookState().taskWriteUnconfirmed(taskScope)).toBe(false);
  });

  it("requires exact SOURCE identity and read evidence, and resolves only the reviewed original", async () => {
    const fixture = await mountedTasks(), { api, consumer } = fixture, original = sourceSubmission();
    await makeUnknown(fixture, original);
    const otherScope = { ...taskScope, taskId: "task-b" }, other = stepSubmission(otherScope, "other-step");
    await makeUnknown(fixture, other);
    const request: SourceResolution = { originalSubmitted: original, reviewedTaskRevision: "task-reviewed", decision: { kind: "use_saved" } };
    expect(await consumer.hookState().resolveSourceUnknown(request)).toMatchObject({ kind: "not_resolved" });
    const saved = await readOriginal(fixture);
    const changed = sourceSubmission(taskScope, original.submissionId, "task-original", "Different description");
    expect(await consumer.hookState().resolveSourceUnknown({ ...request, originalSubmitted: changed })).toMatchObject({ kind: "not_resolved" });
    expect(await consumer.hookState().resolveSourceUnknown({ ...request, reviewedTaskRevision: "task-original" })).toMatchObject({ kind: "not_resolved" });
    expect(consumer.hookState().taskWriteUnconfirmed(taskScope)).toBe(true);
    let result!: Promise<SourceResolutionOutcome>;
    await flush(() => { result = consumer.hookState().resolveSourceUnknown(request); });
    expect(await result).toEqual({ kind: "resolved", task: saved });
    expect(consumer.hookState().taskWriteUnconfirmed(taskScope)).toBe(false);
    expect(consumer.hookState().taskWriteUnconfirmed(otherScope)).toBe(true);
    expect(api.client.orchestrationMutate).toHaveBeenCalledTimes(2);
  });

  it("retains the original SOURCE gate when busy and the replacement payload when reviewed application becomes unknown", async () => {
    const fixture = await mountedTasks(), { api, consumer } = fixture, original = sourceSubmission();
    await makeUnknown(fixture, original);
    await readOriginal(fixture);
    const reviewed = sourceSubmission(taskScope, "source-reviewed", "task-reviewed", "Reviewed replacement description");
    const request: SourceResolution = { originalSubmitted: original, reviewedTaskRevision: "task-reviewed",
      decision: { kind: "apply_reviewed", submitted: reviewed } };
    let pending!: Promise<OrchestrationActionResult | null>;
    await flush(() => { pending = consumer.mutate({ action: "cancel_run", run_id: "worker" }); });
    expect(await consumer.hookState().resolveSourceUnknown(request)).toMatchObject({ kind: "not_resolved" });
    expect(consumer.hookState().taskWriteUnconfirmed(taskScope)).toBe(true);
    expect(api.client.orchestrationMutate).toHaveBeenCalledTimes(2);
    await flush(() => api.mutation().resolve({ revision: 13, result: { result: "done" } }));
    await pending;
    await flush(() => api.snapshot().resolve({ ...taskSnapshot(), revision: 13 }));
    expect(await consumer.hookState().resolveSourceUnknown({ ...request,
      decision: { kind: "apply_reviewed", submitted: sourceSubmission(taskScope, "stale-review", "task-original") } }))
      .toMatchObject({ kind: "not_resolved" });
    expect(api.client.orchestrationMutate).toHaveBeenCalledTimes(2);
    let result!: Promise<SourceResolutionOutcome>;
    await flush(() => { result = consumer.hookState().resolveSourceUnknown(request); });
    expect(api.client.orchestrationMutate).toHaveBeenLastCalledWith({ session_id: "session-a", expected_revision: null, action: reviewed.action });
    await flush(() => api.mutation().reject(new CockpitClientError("http_error", "Source publication unconfirmed", { operationCode: "io_error" })));
    expect(await result).toEqual({ kind: "applied", outcome: { kind: "unknown", submitted: reviewed, message: "Source publication unconfirmed" } });
    expect(await consumer.hookState().resolveSourceUnknown({ ...request, decision: { kind: "use_saved" } })).toMatchObject({ kind: "not_resolved" });
    expect(await consumer.hookState().resolveSourceUnknown({ ...request, originalSubmitted: reviewed, decision: { kind: "use_saved" } }))
      .toMatchObject({ kind: "not_resolved" });
    expect(consumer.hookState().taskWriteUnconfirmed(taskScope)).toBe(true);
    const saved = await readOriginal(fixture, taskScope, "source-replacement-reviewed");
    let resolved!: Promise<SourceResolutionOutcome>;
    await flush(() => { resolved = consumer.hookState().resolveSourceUnknown({ originalSubmitted: reviewed,
      reviewedTaskRevision: "source-replacement-reviewed", decision: { kind: "use_saved" } }); });
    expect(await resolved).toEqual({ kind: "resolved", task: saved });
    expect(consumer.hookState().taskWriteUnconfirmed(taskScope)).toBe(false);
  });
  it("retains uncertainty when a reviewed saved checklist is globally unsafe despite visible rows", async () => {
    const fixture = await mountedTasks(), original = stepSubmission();
    await makeUnknown(fixture, original);
    const unsafe = { ...savedTask("task-a", "task-reviewed"), step_progress: null, steps_diagnostic: "Unsafe saved structure",
      steps: [{ step_id: "step-a", parent_step_id: null, depth: 0, title: "Visible row", checked: false, status: "open" as const, line: 2, source_offset: 16, diagnostic: null }] };
    let read!: Promise<StepReadOutcome>;
    await flush(() => { read = fixture.consumer.hookState().readSaved(taskScope); });
    await flush(() => fixture.api.snapshot().resolve(taskSnapshot([unsafe])));
    expect(await read).toEqual({ kind: "found", task: unsafe });
    const reviewed = stepSubmission(taskScope, "reviewed", "task-reviewed");
    expect(await fixture.consumer.hookState().resolveUnknown({ ...resolution(original), decision: { kind: "apply_reviewed", submitted: reviewed } }))
      .toMatchObject({ kind: "not_resolved", code: "resolution_stale" });
    expect(fixture.api.client.orchestrationMutate).toHaveBeenCalledTimes(1);
    expect(fixture.consumer.hookState().taskWriteUnconfirmed(taskScope)).toBe(true);
  });
  it("allows an explicitly reviewed safe oversized forest without interpreting its diagnostic text", async () => {
    const fixture = await mountedTasks(), original = stepSubmission();
    await makeUnknown(fixture, original);
    const steps = Array.from({ length: 65 }, (_, index) => ({ step_id: `00000000-0000-4000-8000-${index.toString().padStart(12, "0")}`, parent_step_id: null, depth: 0, title: `Row ${index}`, checked: false, status: "open" as const, line: index + 2, source_offset: index * 80, diagnostic: null }));
    const oversized = { ...savedTask("task-a", "task-reviewed"), steps, step_progress: { done: 0, total: 65 }, steps_diagnostic: "Any diagnostic wording" };
    let read!: Promise<StepReadOutcome>;
    await flush(() => { read = fixture.consumer.hookState().readSaved(taskScope); });
    await flush(() => fixture.api.snapshot().resolve(taskSnapshot([oversized])));
    expect(await read).toEqual({ kind: "found", task: oversized });
    const reviewed: StepSubmission = { ...stepSubmission(taskScope, "reviewed", "task-reviewed"), intent: { kind: "rename", stepId: steps[0].step_id, title: "Reviewed title" } };
    let applied!: Promise<StepResolutionOutcome>;
    await flush(() => { applied = fixture.consumer.hookState().resolveUnknown({ ...resolution(original), decision: { kind: "apply_reviewed", submitted: reviewed } }); });
    expect(fixture.api.client.orchestrationMutate).toHaveBeenCalledTimes(2);
    await flush(() => fixture.api.mutation().resolve({ revision: 2, result: { result: "task", task: { ...oversized, task_revision: "renamed" } } }));
    expect(await applied).toMatchObject({ kind: "applied", outcome: { kind: "confirmed" } });
    expect(fixture.consumer.hookState().taskWriteUnconfirmed(taskScope)).toBe(false);
  });
});
