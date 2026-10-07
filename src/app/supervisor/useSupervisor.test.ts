// @vitest-environment jsdom
import { act, createElement } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, describe, expect, it, vi } from "vitest";
import type { CockpitClient } from "../../client/CockpitClient";
import type { OrchestrationAction, OrchestrationActionResult, OrchestrationMutationResponse, OrchestrationSnapshot, OrchestrationWaitResponse, Run } from "../../protocol/generated/v1";
import { acceptsSupervisorSnapshot, useSupervisor } from "./useSupervisor";

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
        task: { task_id: "task-a", title: "Deliver work", body: "Canonical task", checked: false, line: 1, task_revision: "task-b", diagnostic: null },
        lane: kind === "needs_input" ? "working" : "review", current_run_id: "worker",
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
