// @vitest-environment jsdom
import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, describe, expect, it, vi } from "vitest";
import type { CockpitClient } from "../../client/CockpitClient";
import type { OrchestrationSnapshot, OrchestrationWaitResponse, Run, RunObservation, SessionSnapshotResponse, TaskBoard, TaskView } from "../../protocol/generated/v1";
import { agentState, taskStatus } from "./SupervisorActions";
import { SupervisorView, taskTitle } from "./SupervisorView";

const at = "2026-10-06T12:00:00Z";
function run(overrides: Partial<Run> = {}): Run {
  return { session_id: "session", prepare_brief: "Guidance", run_id: "root", kind: "supervisor", label: "Project supervisor", root_id: "root", parent_run_id: null, task_id: null, attempt: 1, task_revision_at_propose: null, stage: "active", close_reason: null, dispatch: { launch_tag: "tag", endpoint_identity: "endpoint", recovery: null, agent_started: true, step: "launched", launch_attempt: 1, error: null, updated_at: at }, target: { target: "existing_space", workspace_id: "space" }, setup: null, prepare_plan: null, init_receipt: null, work_plan: null, grants: [], last_report: null, result: null, annotations: [], location: { boot_id: "boot", terminal_id: "terminal", native_session_id: "native", endpoint_identity: "endpoint", session_id: "session", workspace_id: "space", tab_id: "tab", pane_id: "pane", launch_tag: "tag" }, bound_omp_session: "native", supersedes_run_id: null, created_at: at, updated_at: at, ...overrides };
}
function observed(runId = "root", overrides: Partial<RunObservation> = {}): RunObservation {
  return { run_id: runId, presence: "present", actual_omp: true, workspace_id: "space", workspace_label: "Project", tab_id: "tab", tab_label: "Agent", pane_id: "pane", agent_status: "working", state_changed_at: at, ...overrides };
}
function task(overrides: Partial<TaskView> = {}): TaskView {
  return { task: { task_id: "task-a", title: "Improve search", body: "Improve search\nKeep existing behavior.", checked: false, line: 1, task_revision: "task-revision", diagnostic: null }, lane: "working", current_run_id: "worker", ...overrides };
}
function board(rootId = "root", tasks: TaskView[] = []): TaskBoard {
  return { root_id: rootId, path: `/state/${rootId}.md`, doc_revision: "document-revision", unidentified_items: 0, diagnostics: [], tasks };
}
function snapshot(runs: Run[] = [run()], tasks: TaskView[] = []): OrchestrationSnapshot {
  return { session_id: "session", revision: 1, tasks_token: "tasks", roots: runs.filter(run => !run.parent_run_id).map(run => ({ root_id: run.run_id, label: run.label, kind: run.kind, open_runs: 1, needs_you: 0 })), board: board("root", tasks), runs, messages: [], subagents: [], intents: [], assignment_intents: [], attention: [], unmanaged_agents: [], runtime: { status: "fresh", endpoint_identity: "endpoint", observed_at: at, runs: runs.map(run => observed(run.run_id)) } };
}
const session: SessionSnapshotResponse = { session_id: "session", server_instance: "server", version: "fixture", protocol: 20, focused_space_id: "space", focused_tab_id: "tab", focused_pane_id: "pane", spaces: [{ id: "space", label: "Project", number: 1, tab_count: 1, pane_count: 1, focused: true, agent_status: "working", git: null }], tabs: [], panes: [], agents: [] };
let host: HTMLDivElement;
let reactRoot: Root | null = null;
let rerender: (active?: boolean, runtimeLive?: boolean) => Promise<void>;
async function settle() { await act(async () => { for (let index = 0; index < 8; index++) await Promise.resolve(); }); }
function button(text: string): HTMLButtonElement {
  const found = [...document.querySelectorAll<HTMLButtonElement>("button")].find(button => button.textContent?.trim() === text || button.getAttribute("aria-label") === text);
  if (!found) throw new Error(`Missing button ${text}`);
  return found;
}
function enter(field: HTMLTextAreaElement | HTMLInputElement | HTMLSelectElement, value: string) {
  act(() => {
    const prototype = field instanceof HTMLSelectElement ? HTMLSelectElement.prototype : field instanceof HTMLTextAreaElement ? HTMLTextAreaElement.prototype : HTMLInputElement.prototype;
    Object.getOwnPropertyDescriptor(prototype, "value")!.set!.call(field, value);
    field.dispatchEvent(new Event(field instanceof HTMLSelectElement ? "change" : "input", { bubbles: true }));
  });
}
async function mount(initial: OrchestrationSnapshot, boards?: Map<string, TaskBoard>) {
  let state = initial;
  const onTerminal = vi.fn(async () => undefined);
  const onClose = vi.fn();
  const snapshotCall = vi.fn<CockpitClient["orchestrationSnapshot"]>(async request => ({ ...state, board: boards?.get(request.root_id ?? "root") ?? state.board }));
  const mutation = vi.fn<CockpitClient["orchestrationMutate"]>(async request => {
    state = { ...state, revision: state.revision + 1 };
    if (request.action.action === "task_assign") return { revision: state.revision, result: { result: "task_assigned", task: { task_id: request.action.task_id, title: request.action.title, body: request.action.body, checked: false, line: 1, task_revision: "assigned-revision", diagnostic: null }, to_run_id: request.action.root_id, seq: 1, duplicate: false } };
    if (request.action.action === "message_send") return { revision: state.revision, result: { result: "message", to_run_id: request.action.to_run_id, seq: 1, duplicate: false, stale: false } };
    if (request.action.action === "supervisor_start") return { revision: state.revision, result: { result: "run", run_id: "new-root", attempt: 1 } };
    return { revision: state.revision, result: { result: "done" } };
  });
  const pendingWait = new Promise<OrchestrationWaitResponse>(() => undefined);
  const client = { orchestrationSnapshot: snapshotCall, orchestrationWait: vi.fn(() => pendingWait), orchestrationMutate: mutation } as unknown as CockpitClient;
  host = document.createElement("div"); document.body.append(host); reactRoot = createRoot(host);
  rerender = async (active = true, runtimeLive = true) => {
    await act(async () => { reactRoot!.render(<SupervisorView client={client} sessionId="session" session={session} runtimeLive={runtimeLive} active={active} startToken={0} navigationError={null} onClose={onClose} onTerminal={onTerminal} onUnmanagedTerminal={vi.fn(async () => undefined)} onModalChange={vi.fn()} />); });
    await settle();
  };
  await rerender();
  return { mutation, onTerminal, onClose, snapshotCall, replace: (next: OrchestrationSnapshot) => { state = next; } };
}
afterEach(async () => { if (reactRoot) await act(async () => reactRoot!.unmount()); reactRoot = null; host?.remove(); vi.restoreAllMocks(); });

describe("Supervisor truth and canonical task presentation", () => {
  it("never calls a historical ACK, name or bound shell a connected OMP", () => {
    const saved = run();
    const state = snapshot([saved]);
    state.runtime = { status: "fresh", endpoint_identity: "endpoint", observed_at: at, runs: [observed("root", { actual_omp: false, agent_status: "idle" })] };
    expect(agentState(state, saved, true, true).verified).toBe(false);
    expect(agentState(state, saved, true, true).label).toBe("Cannot confirm the agent");
    const pending = run({ stage: "preparing", dispatch: { ...saved.dispatch!, step: "launch_pending" } });
    expect(agentState(state, pending, true, true).kind).toBe("starting");
    state.runtime.runs[0].actual_omp = true;
    expect(agentState(state, saved, true, true).verified).toBe(true);
    expect(state.runtime.runs[0].agent_status).toBe("idle");
    state.runtime.runs[0].agent_status = "working";
    expect(agentState(state, saved, true, true).verified).toBe(true);
    state.runtime.runs[0].agent_status = "blocked";
    expect(agentState(state, saved, true, true).blocked).toBe(true);
    expect(agentState(state, saved, true, true).label).toBe("Agent blocked");
  });
  it("keeps launch and setup errors uncertain rather than claiming a proved failed process", () => {
    const saved = run();
    const state = snapshot([saved]);
    const uncertain = run({ dispatch: { ...saved.dispatch!, step: "launch_unknown", error: { code: "launch_timeout", message: "Startup deadline expired." } } });
    expect(agentState(state, uncertain, true, true).kind).toBe("unknown");
    expect(agentState(state, uncertain, true, true).detail).toContain("Startup deadline expired.");
    expect(agentState(state, uncertain, true, true).label).not.toBe("Agent did not start");
    uncertain.dispatch!.step = "setup_unknown";
    expect(agentState(state, uncertain, true, true).label).toBe("Agent setup is unconfirmed");
  });
  it("labels a verified supervisor ready only when its canonical scope has no open work", () => {
    const supervisor = run();
    expect(agentState(snapshot([supervisor]), supervisor, true, true).label).toBe("Ready for a task");
    expect(agentState(snapshot([supervisor], [task()]), supervisor, true, true).label).toBe("Managing tasks");
    const finished = task({ lane: "accepted", task: { ...task().task, checked: true } });
    expect(agentState(snapshot([supervisor], [finished]), supervisor, true, true).label).toBe("Ready for a task");
    const worker = run({ kind: "worker", run_id: "worker", parent_run_id: "root", stage: "proposed" });
    expect(agentState(snapshot([supervisor, worker]), supervisor, true, true).label).toBe("Managing tasks");
  });
  it("keeps disconnect separate from freshly missing and exposes both recovery choices", async () => {
    const state = snapshot();
    state.runtime = { status: "fresh", endpoint_identity: "endpoint", observed_at: at, runs: [observed("root", { presence: "missing", actual_omp: false, pane_id: null })] };
    await mount(state);
    expect(host.textContent).toContain("Agent terminal is gone");
    expect(button("Restart agent…").disabled).toBe(false);
    expect(button("Close tracking…").disabled).toBe(false);
    expect([...host.querySelectorAll("button")].some(button => button.textContent === "Open terminal")).toBe(false);
    await rerender(true, false);
    expect(host.textContent).toContain("Connection lost");
    expect(host.querySelector('[aria-label="Project supervisor status"]')?.textContent).not.toContain("Agent terminal is gone");
  });
  it("uses explicit Result and acceptance rather than runtime Done or a task checkbox as agent success", () => {
    const worker = run({ kind: "worker", run_id: "worker", parent_run_id: "root", task_id: "task-a", stage: "working" });
    const state = snapshot([run(), worker], [task()]);
    expect(taskStatus(task(), worker, state)).toBe("Working");
    const checked = task({ lane: "accepted", task: { ...task().task, checked: true } });
    expect(taskStatus(checked, worker, state)).toBe("Marked complete in task file");
    worker.stage = "closed"; worker.close_reason = "accepted"; worker.result = { message_id: "result", kind: "result", outcome: "succeeded", summary: "Tests passed", plan: null, at };
    expect(taskStatus(checked, worker, state)).toBe("Completed");
    worker.close_reason = "cancelled";
    expect(taskStatus(task(), worker, state)).toBe("Tracking closed");
  });
  it("keeps ordinary worker questions with the supervisor and retains report/observed provenance", async () => {
    const worker = run({ kind: "worker", run_id: "worker", label: "Search agent", parent_run_id: "root", task_id: "task-a", stage: "working", last_report: { message_id: "question", kind: "needs_input", outcome: null, summary: "Which test should I use?", plan: null, at } });
    await mount(snapshot([run(), worker], [task()]));
    expect(host.querySelector('[aria-label="Needs you"]')).toBeNull();
    expect(host.textContent).toContain("Waiting for supervisor");
    expect(host.textContent).toContain("Reported · Search agent");
    expect(host.textContent).toContain("Observed · Herdr");
    expect(host.querySelector("[role=tablist]")).toBeNull();
    expect(host.querySelector('[aria-label="Agent relationships"]')).not.toBeNull();
    expect(host.querySelector('nav[aria-label="Supervisor panels"]')).not.toBeNull();
    expect(host.querySelector("aside")).toBeNull();
    const row = host.querySelector<HTMLButtonElement>("[data-row-id=task-a]")!;
    act(() => row.focus());
    act(() => row.dispatchEvent(new KeyboardEvent("keydown", { key: "2", bubbles: true })));
    expect(row.getAttribute("aria-expanded")).toBe("false");
    expect(host.querySelector('aside[aria-label="Selected details"]')).toBeNull();
    act(() => row.click()); await settle();
    const detail = host.querySelector('aside[aria-label="Selected details"]')!;
    expect(detail.textContent).toContain("Which test should I use?");
    expect(detail.textContent).toContain("Waiting for supervisor");
    expect(detail.textContent).toContain("Reported · Search agent");
    expect(detail.textContent).toContain("Observed · Herdr");
    expect(detail.querySelector('nav[aria-label="Detail sections"] button[aria-pressed="true"]')!.textContent).toBe("Overview");
  });
  it.each(["progress", "ready", "result"] as const)("keeps ordinary %s report text in the selected overview, not the task card", async kind => {
    const text = "Exact technical report: revision abc123, repository /tmp/worker-checkout.\nKeep every detail unchanged.";
    const worker = run({ kind: "worker", run_id: "worker", label: "Search agent", parent_run_id: "root", task_id: "task-a", stage: kind === "result" ? "reported" : kind === "ready" ? "ready" : "working", last_report: { message_id: "report", kind, outcome: kind === "result" ? "succeeded" : null, summary: text, plan: null, at } });
    if (kind === "result") worker.result = worker.last_report;
    await mount(snapshot([run(), worker], [task({ lane: kind === "result" ? "review" : kind === "ready" ? "ready" : "working" })]));
    const row = host.querySelector("li.supervisor-task")!;
    const evidence = row.querySelector(".supervisor-task-evidence")!;
    expect(evidence.textContent).toContain("Reported · Search agent");
    expect(evidence.querySelector("time")).not.toBeNull();
    expect(evidence.textContent).not.toContain(text);
    expect(row.querySelector(".supervisor-task-detail")).toBeNull();
    expect(host.querySelector('aside[aria-label="Selected details"]')).toBeNull();
    act(() => row.querySelector<HTMLButtonElement>("[data-row-id=task-a]")!.click()); await settle();
    const detail = host.querySelector('aside[aria-label="Selected details"]')!;
    expect(detail.textContent).toContain(text);
    expect(detail.querySelector('nav[aria-label="Detail sections"] button[aria-pressed="true"]')!.textContent).toBe("Overview");
    expect(row.textContent).not.toContain(text);
    expect(row.querySelector(".supervisor-task-detail")).toBeNull();
  });
  it("retains literal root questions and explicit reported failures on the primary surface", async () => {
    const question = "Which repository should I use?";
    const failure = "Compiler rejected the migration. Existing files are unchanged.";
    const supervisor = run({ last_report: { message_id: "root-question", kind: "needs_input", outcome: null, summary: question, plan: null, at } });
    const worker = run({ kind: "worker", run_id: "worker", label: "Search agent", parent_run_id: "root", task_id: "task-a", stage: "reported", last_report: { message_id: "failed-result", kind: "result", outcome: "failed", summary: failure, plan: null, at } });
    worker.result = worker.last_report;
    await mount(snapshot([supervisor, worker], [task({ lane: "review" })]));
    expect(host.querySelector('[aria-label="Needs you"]')!.textContent).toContain(question);
    expect(host.querySelector(".supervisor-task-evidence")!.textContent).toContain(failure);
  });
  it("omits startup observation for closed work while preserving an unproved live launch and named location", async () => {
    const finished = run({ kind: "worker", run_id: "finished-worker", label: "Finished agent", parent_run_id: "root", task_id: "completed-task", stage: "closed", close_reason: "accepted", result: { message_id: "accepted-result", kind: "result", outcome: "succeeded", summary: "Completed successfully.", plan: null, at } });
    finished.last_report = finished.result;
    const pending = run({ kind: "worker", run_id: "pending-worker", label: "Pending agent", parent_run_id: "root", task_id: "pending-task", stage: "preparing", bound_omp_session: null, dispatch: { ...run().dispatch!, step: "plan_failed", error: { code: "launch_failed", message: "OMP could not start in the target terminal." } } });
    const completed = task({ task: { ...task().task, task_id: "completed-task", checked: true }, lane: "accepted", current_run_id: "finished-worker" });
    const unproved = task({ task: { ...task().task, task_id: "pending-task", title: "Pending work" }, lane: "setup", current_run_id: "pending-worker" });
    const state = snapshot([run(), finished, pending], [completed, unproved]);
    state.runtime = { status: "fresh", endpoint_identity: "endpoint", observed_at: at, runs: [observed(), observed("finished-worker", { actual_omp: false, workspace_label: "Named checkout", tab_label: "ck-internal-launch-tag" }), observed("pending-worker", { actual_omp: false })] };
    await mount(state);
    expect(button("Show completed tasks").getAttribute("aria-expanded")).toBe("false");
    act(() => button("Show completed tasks").click()); await settle();
    const closedRow = host.querySelector('[data-row-id="completed-task"]')!.closest("li")!;
    expect(closedRow.querySelector(".supervisor-task-stage")!.textContent).toBe("Completed");
    expect(closedRow.querySelector(".supervisor-observed")).toBeNull();
    expect(closedRow.querySelector(".supervisor-task-evidence")!.textContent).not.toContain("OMP not confirmed");
    const location = closedRow.querySelector<HTMLElement>(".supervisor-location")!;
    expect(location.textContent).toBe("Named checkout");
    expect(location.title).toContain("ck-internal-launch-tag");
    const liveRow = host.querySelector('[data-row-id="pending-task"]')!.closest("li")!;
    expect(liveRow.querySelector(".supervisor-observed")!.textContent).toContain("OMP not confirmed");
    expect(host.textContent).toContain("OMP could not start in the target terminal.");
  });
  it("keeps closed-only tracking in the archive and opens with a real empty Start agent state", async () => {
    await mount(snapshot([run({ stage: "closed", close_reason: "cancelled" })], [task()]));
    expect(host.textContent).toContain("Start an agent to manage your tasks");
    expect(host.querySelector("[data-task-composer]")).toBeNull();
    expect(button("Closed tracking · 1").getAttribute("aria-expanded")).toBe("false");
    expect(button("Start agent").disabled).toBe(false);
  });
  it("does not count closed or missing tracking as connected agents and keeps surviving descendants controllable", async () => {
    const closed = run({ stage: "closed", close_reason: "cancelled" });
    const worker = run({ kind: "worker", run_id: "worker", label: "Search agent", parent_run_id: "root", task_id: "task-a", stage: "working" });
    const state = snapshot([closed, worker], [task()]);
    if (state.runtime.status === "fresh") state.runtime.runs[0] = observed("root", { presence: "missing", actual_omp: false, pane_id: null });
    await mount(state);
    expect(host.querySelector("[data-task-composer]")).toBeNull();
    act(() => button("View saved task context").click()); await settle();
    expect(host.textContent).toContain("Worker agents need control");
    expect(host.textContent).toContain("Agents · 1 connected");
    expect(host.textContent).toContain("Closing this supervisor did not stop them");
    expect(host.textContent).toContain("Tracking closed");
  });
  it("bounds UTF-8 title metadata without touching full task instructions", () => {
    const text = `${"界".repeat(140)}\n\nExact body:  keep  spaces`;
    const title = taskTitle(text);
    expect(new TextEncoder().encode(title).length).toBeLessThanOrEqual(256);
    expect(title.endsWith("…")).toBe(true);
    expect(text).toContain("Exact body:  keep  spaces");
  });
});

describe("Supervisor direct actions and scoped drafts", () => {
  it("starts one new agent tab in the current Space without a setup wizard or focus request", async () => {
    const empty = snapshot([]); empty.board = null;
    const fixture = await mount(empty);
    act(() => button("Start agent").click()); await settle();
    expect(fixture.mutation).toHaveBeenCalledTimes(1);
    expect(fixture.mutation.mock.calls[0][0].action).toEqual({ action: "supervisor_start", target: { target: "existing_space", workspace_id: "space" }, label: null });
    expect(document.querySelector('[role="dialog"]')).toBeNull();
    expect(fixture.onTerminal).not.toHaveBeenCalled();
  });
  it("derives startup waiting from fresh proof and removes it once an empty supervisor is bound", async () => {
    const empty = snapshot([]); empty.board = null;
    const fixture = await mount(empty);
    const pending = run({ run_id: "new-root", root_id: "new-root", stage: "preparing", bound_omp_session: null, dispatch: { ...run().dispatch!, agent_started: false, step: "launch_pending" } });
    const starting = { ...snapshot([pending]), revision: 2, board: board("new-root") };
    if (starting.runtime.status === "fresh") starting.runtime.runs[0].actual_omp = false;
    fixture.mutation.mockImplementationOnce(async () => {
      fixture.replace(starting);
      return { revision: 2, result: { result: "run", run_id: "new-root", attempt: 1 } };
    });
    act(() => button("Start agent").click()); await settle();
    expect(host.textContent).toContain("Waiting for OMP to start and connect.");
    const verified = { ...snapshot([run({ run_id: "new-root", root_id: "new-root" })]), revision: 3, board: board("new-root") };
    fixture.replace(verified);
    await rerender(false); await rerender(true);
    const status = host.querySelector('[aria-label="Project supervisor status"]')!;
    expect(status.textContent).toContain("Ready for a task");
    expect(status.textContent).not.toContain("Managing tasks");
    expect(host.textContent).not.toContain("Waiting for OMP to start and connect.");
    expect(fixture.onTerminal).not.toHaveBeenCalled();
  });
  it("checks a freshly missing launch before explicit restart and accepts the real Done acknowledgement", async () => {
    const original = run();
    const state = snapshot([original]);
    state.runtime = { status: "fresh", endpoint_identity: "endpoint", observed_at: at, runs: [observed("root", { presence: "missing", actual_omp: false, pane_id: null })] };
    const fixture = await mount(state);
    fixture.mutation.mockImplementationOnce(async () => {
      const reviewed = { ...state, revision: 2, runs: [run({ dispatch: { ...original.dispatch!, step: "launch_unknown" } })] };
      fixture.replace(reviewed);
      return { revision: 2, result: { result: "done" } };
    });
    act(() => button("Restart agent…").click()); await settle();
    expect(fixture.mutation.mock.calls[0][0].action).toEqual({ action: "reconcile_run", run_id: "root", recovery: null });
    expect(document.querySelector('[role="dialog"]')?.textContent).toContain("could leave another agent running");
    act(() => button("Restart anyway").click()); await settle();
    expect(fixture.mutation.mock.calls[1][0].action).toEqual({ action: "retry_launch", run_id: "root" });
    expect(document.querySelector('[role="dialog"]')).toBeNull();
    expect(fixture.mutation.mock.calls.some(([request]) => request.action.action === "supervisor_start")).toBe(false);
  });
  it("retains a genuine escalation answer until actual message delivery and leaves Escape in its textarea alone", async () => {
    const supervisor = run({ last_report: { message_id: "question", kind: "needs_input", outcome: null, summary: "Which checkout should I use?", plan: null, at } });
    const fixture = await mount(snapshot([supervisor]));
    const answer = document.querySelector<HTMLTextAreaElement>('[aria-label="Needs you"] textarea')!;
    expect(answer.getAttribute("aria-describedby")).toContain("supervisor-question-question");
    enter(answer, "Use the existing checkout.");
    act(() => answer.dispatchEvent(new KeyboardEvent("keydown", { key: "Escape", bubbles: true })));
    expect(fixture.onClose).not.toHaveBeenCalled();
    fixture.mutation.mockResolvedValueOnce({ revision: 1, result: { result: "done" } });
    act(() => button("Send answer").click()); await settle();
    const submitted = fixture.mutation.mock.calls[0][0].action;
    expect(submitted.action).toBe("message_send");
    expect(answer.value).toBe("Use the existing checkout.");
    act(() => button("Retry same message").click()); await settle();
    expect(fixture.mutation.mock.calls[1][0].action).toEqual(submitted);
    expect(document.querySelector<HTMLTextAreaElement>('[aria-label="Needs you"] textarea')!.value).toBe("");
    expect(host.textContent).toContain("Answer sent. Waiting for the agent.");
  });
  it("assigns the exact single-field body and waits for TaskAssigned, retaining one UUID after an unknown response", async () => {
    const fixture = await mount(snapshot());
    const unknown = Promise.reject(new Error("transport interrupted")); unknown.catch(() => undefined);
    fixture.mutation.mockReturnValueOnce(unknown);
    const text = "  Improve search\n\nKeep  all instruction spacing.\n";
    enter(host.querySelector<HTMLTextAreaElement>("[data-task-composer]")!, text);
    act(() => button("Give task").click()); await settle();
    const submitted = fixture.mutation.mock.calls[0][0].action;
    expect(submitted.action).toBe("task_assign");
    if (submitted.action !== "task_assign") throw new Error("Wrong operation");
    expect(submitted.body).toBe(text);
    expect(submitted.title).toBe("Improve search");
    expect(submitted.task_id).toMatch(/^[0-9a-f-]{36}$/);
    expect(host.querySelector<HTMLTextAreaElement>("[data-task-composer]")!.value).toBe(text);
    expect(host.querySelector<HTMLTextAreaElement>("[data-task-composer]")!.readOnly).toBe(true);
    await rerender(false); await rerender(true);
    expect(host.querySelector<HTMLTextAreaElement>("[data-task-composer]")!.value).toBe(text);
    act(() => button("Retry same task").click()); await settle();
    expect(fixture.mutation.mock.calls[1][0].action).toEqual(submitted);
    expect(host.querySelector<HTMLTextAreaElement>("[data-task-composer]")!.value).toBe("");
    expect(host.textContent).toContain("Assigned to Project supervisor");
  });
  it("never treats authoring-only Task as assignment delivery success", async () => {
    const fixture = await mount(snapshot());
    fixture.mutation.mockResolvedValueOnce({ revision: 2, result: { result: "task", task: task().task } });
    enter(host.querySelector<HTMLTextAreaElement>("[data-task-composer]")!, "Give real work");
    act(() => button("Give task").click()); await settle();
    expect(host.querySelector<HTMLTextAreaElement>("[data-task-composer]")!.value).toBe("Give real work");
    expect(host.textContent).toContain("Could not confirm assignment");
    expect(host.textContent).not.toContain("Assigned to Project supervisor");
  });
  it("preserves root-specific task and answer drafts across root changes, hide, refresh and offline state", async () => {
    const first = run({ last_report: { message_id: "ask", kind: "needs_input", outcome: null, summary: "Which checkout?", plan: null, at } });
    const second = run({ run_id: "second", root_id: "second", label: "Other supervisor" });
    const fixture = await mount(snapshot([first, second]), new Map([["root", board()], ["second", board("second")]]));
    enter(host.querySelector<HTMLTextAreaElement>("[data-task-composer]")!, "First root draft");
    const answer = [...host.querySelectorAll<HTMLTextAreaElement>("textarea")].find(field => field.id !== host.querySelector("[data-task-composer]")!.id)!;
    enter(answer, "Use the existing checkout");
    enter(host.querySelector<HTMLSelectElement>('select[aria-label="Agent"]')!, "second"); await settle();
    expect(host.querySelector<HTMLTextAreaElement>("[data-task-composer]")!.value).toBe("");
    enter(host.querySelector<HTMLTextAreaElement>("[data-task-composer]")!, "Second root draft");
    enter(host.querySelector<HTMLSelectElement>('select[aria-label="Agent"]')!, "root"); await settle();
    await rerender(false); await rerender(true, false);
    expect(host.querySelector<HTMLTextAreaElement>("[data-task-composer]")!.value).toBe("First root draft");
    expect([...host.querySelectorAll<HTMLTextAreaElement>("textarea")].some(field => field.value === "Use the existing checkout")).toBe(true);
    expect(button("Give task").disabled).toBe(true);
    expect(fixture.onTerminal).not.toHaveBeenCalled();
  });
  it("resolves an assignment conflict with the current canonical CAS and never deletes Markdown", async () => {
    const state = snapshot([run()], [task({ current_run_id: null, lane: "queued" })]);
    state.assignment_intents = [{ root_id: "root", task_id: "task-a", state: "conflict" }];
    const fixture = await mount(state);
    act(() => button("Assign current task").click()); await settle();
    expect(fixture.mutation.mock.calls[0][0].action).toEqual({ action: "task_assignment_resolve", root_id: "root", task_id: "task-a", expected_task_revision: "task-revision", assign: true });
    act(() => button("Keep unassigned").click()); await settle();
    expect(fixture.mutation.mock.calls[1][0].action).toEqual({ action: "task_assignment_resolve", root_id: "root", task_id: "task-a", expected_task_revision: null, assign: false });
    expect(state.board!.tasks[0].task.body).toBe(task().task.body);
  });
  it("opens task detail and moves lane focus without requesting a terminal; editing cancellation retains the draft", async () => {
    const worker = run({ kind: "worker", run_id: "worker", label: "Search agent", parent_run_id: "root", task_id: "task-a", stage: "working" });
    const next = task({ task: { ...task().task, task_id: "task-b", title: "Update docs" }, current_run_id: null, lane: "queued" });
    const state = snapshot([run(), worker], [task(), next]);
    const fixture = await mount(state);
    const first = host.querySelector<HTMLButtonElement>("[data-row-id=task-a]")!;
    act(() => first.focus());
    act(() => first.dispatchEvent(new KeyboardEvent("keydown", { key: "ArrowLeft", bubbles: true })));
    expect(document.activeElement).toBe(host.querySelector("[data-row-id=task-b]"));
    expect(first.getAttribute("aria-expanded")).toBe("false");
    act(() => first.click()); await settle();
    act(() => button("Actions").click()); await settle();
    expect(button("Actions").getAttribute("aria-pressed")).toBe("true");
    act(() => button("Edit task…").click()); await settle();
    const title = document.querySelector<HTMLInputElement>('[role="dialog"] input')!;
    enter(title, "Retained edit draft");
    act(() => button("Cancel").click()); await settle();
    act(() => button("Edit task…").click()); await settle();
    expect(document.querySelector<HTMLInputElement>('[role="dialog"] input')!.value).toBe("Retained edit draft");
    await rerender(false);
    fixture.snapshotCall.mockRejectedValue(new Error("transport offline"));
    await rerender(true);
    expect(button("Save task").disabled).toBe(true);
    expect(button("Cancel").disabled).toBe(false);
    expect(document.querySelector<HTMLInputElement>('[role="dialog"] input')!.value).toBe("Retained edit draft");
    act(() => button("Cancel").click()); await settle();
    expect(document.querySelector('[role="dialog"]')).toBeNull();
    await rerender(false);
    fixture.snapshotCall.mockResolvedValue(state);
    await rerender(true);
    act(() => button("Edit task…").click()); await settle();
    expect(document.querySelector<HTMLInputElement>('[role="dialog"] input')!.value).toBe("Retained edit draft");
    expect(fixture.onTerminal).not.toHaveBeenCalled();
  });
  it("navigates populated board lanes while preserving native button activation and Escape focus return", async () => {
    const queuedFirst = task({ task: { ...task().task, task_id: "queued-a", title: "First queued task" }, lane: "queued", current_run_id: null });
    const queuedSecond = task({ task: { ...task().task, task_id: "queued-b", title: "Second queued task" }, lane: "queued", current_run_id: null });
    const readyFirst = task({ task: { ...task().task, task_id: "ready-a", title: "First ready task" }, lane: "ready", current_run_id: null });
    const readySecond = task({ task: { ...task().task, task_id: "ready-b", title: "Second ready task" }, lane: "ready", current_run_id: null });
    const working = task({ task: { ...task().task, task_id: "working-a", title: "Working task" }, lane: "working", current_run_id: null });
    const fixture = await mount(snapshot([run()], [queuedFirst, readyFirst, queuedSecond, working, readySecond]));
    const card = (id: string) => host.querySelector<HTMLButtonElement>(`li.supervisor-task button[data-row-id="${id}"]`)!;
    const press = (target: HTMLElement, key: string) => {
      const event = new KeyboardEvent("keydown", { key, bubbles: true, cancelable: true });
      act(() => target.dispatchEvent(event));
      return event;
    };
    expect([...host.querySelectorAll(".supervisor-lane")].map(lane => lane.getAttribute("aria-label"))).toEqual(["Queued tasks", "Preparing tasks", "Ready tasks", "Working tasks", "Review tasks", "Done tasks"]);
    expect(host.querySelector('aside[aria-label="Selected details"]')).toBeNull();
    act(() => card("queued-a").focus());
    expect(press(card("queued-a"), "ArrowDown").defaultPrevented).toBe(true);
    expect(document.activeElement).toBe(card("queued-b"));
    expect(card("queued-b").tabIndex).toBe(0);
    expect(card("queued-a").tabIndex).toBe(-1);
    expect(press(card("queued-b"), "ArrowRight").defaultPrevented).toBe(true);
    expect(document.activeElement).toBe(card("ready-b"));
    expect(card("ready-b").tabIndex).toBe(0);
    expect(card("queued-b").tabIndex).toBe(-1);
    expect(host.querySelector('aside[aria-label="Selected details"]')).toBeNull();
    expect(press(card("ready-b"), "ArrowUp").defaultPrevented).toBe(true);
    expect(document.activeElement).toBe(card("ready-a"));
    expect(press(card("ready-a"), "ArrowDown").defaultPrevented).toBe(true);
    expect(document.activeElement).toBe(card("ready-b"));
    expect(press(card("ready-b"), "ArrowRight").defaultPrevented).toBe(true);
    expect(document.activeElement).toBe(card("working-a"));
    expect(card("working-a").getAttribute("aria-expanded")).toBe("false");

    for (const key of ["Enter", " "]) {
      const target = card("working-a");
      expect(target.tagName).toBe("BUTTON");
      expect(target.type).toBe("button");
      expect(press(target, key).defaultPrevented).toBe(false);
      const release = new KeyboardEvent("keyup", { key, bubbles: true, cancelable: true });
      act(() => target.dispatchEvent(release));
      expect(release.defaultPrevented).toBe(false);
      expect(target.getAttribute("aria-expanded")).toBe("false");
      // jsdom does not synthesize a native button click from keyboard events.
      act(() => target.click()); await settle();
      expect(target.getAttribute("aria-expanded")).toBe("true");
      expect(host.querySelector('aside[aria-label="Selected details"]')!.textContent).toContain("Working task");
      const overview = button("Overview");
      act(() => overview.focus());
      expect(document.activeElement).toBe(overview);
      expect(press(overview, "Escape").defaultPrevented).toBe(true);
      await act(async () => { await new Promise<void>(resolve => requestAnimationFrame(() => resolve())); });
      expect(host.querySelector('aside[aria-label="Selected details"]')).toBeNull();
      expect(target.getAttribute("aria-expanded")).toBe("false");
      expect(document.activeElement).toBe(target);
    }
    expect(fixture.mutation).not.toHaveBeenCalled();
    expect(fixture.onTerminal).not.toHaveBeenCalled();
    expect(fixture.onClose).not.toHaveBeenCalled();
  });
  it("attributes automatic grants to the supervisor and keeps raw bootstrap text out of History", async () => {
    const worker = run({ kind: "worker", run_id: "worker", label: "Search agent", parent_run_id: "root", task_id: "task-a", stage: "working", grants: [{ grant_id: "grant", scope: "execute", plan_revision: "technical-hash-not-a-summary", origin: "supervisor", supervisor_run_id: "root", omp_session_id: "native", granted_at: at }] });
    worker.annotations = [{ at, by: { type: "run", run_id: "root" }, text: "Acceptance requested for Result result-secret at task revision revision-secret; origin Supervisor, main session native-secret." }];
    const state = snapshot([run(), worker], [task()]);
    state.messages = [{ message_id: "brief", to_run_id: "root", seq: 1, from: { type: "dispatcher" }, kind: "supervisor_brief", text: "RAW BOOTSTRAP POLICY", report: null, stale: false, escalated_from: null, from_subagent_id: null, stage: "stored", woken_omp_session: null, created_at: at, acked_at: null }];
    await mount(state);
    act(() => button("Activity").click()); await settle();
    expect(button("Activity").getAttribute("aria-pressed")).toBe("true");
    const history = host.querySelector('section[aria-label="History"]')!;
    expect(history.textContent).toContain("Execution authorized · Project supervisor");
    expect(history.textContent).not.toContain("RAW BOOTSTRAP POLICY");
    expect(history.textContent).not.toContain("technical-hash-not-a-summary");
    expect(history.textContent).not.toContain("authorized · You");
    expect(history.textContent).toContain("Result review note for Search agent");
    expect(history.textContent).not.toContain("revision-secret");
    expect(history.textContent).not.toContain("native-secret");
  });
});
