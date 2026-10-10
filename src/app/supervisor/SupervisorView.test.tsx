// @vitest-environment jsdom
import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, describe, expect, it, vi } from "vitest";
import type { CockpitClient } from "../../client/CockpitClient";
import { CockpitClientError } from "../../client/CockpitClient";
import type { OrchestrationSnapshot, OrchestrationWaitResponse, Run, RunObservation, SessionSnapshotResponse, TaskBoard, TaskView } from "../../protocol/generated/v1";
import { agentState, taskStatus } from "./SupervisorActions";
import { SupervisorView } from "./SupervisorView";

const at = "2026-10-06T12:00:00Z";
function run(overrides: Partial<Run> = {}): Run {
  return { session_id: "session", prepare_brief: "Guidance", run_id: "root", kind: "supervisor", label: "Project supervisor", root_id: "root", parent_run_id: null, task_id: null, attempt: 1, task_revision_at_propose: null, stage: "active", close_reason: null, dispatch: { launch_tag: "tag", endpoint_identity: "endpoint", recovery: null, agent_started: true, step: "launched", launch_attempt: 1, error: null, updated_at: at }, target: { target: "existing_space", workspace_id: "space" }, setup: null, prepare_plan: null, init_receipt: null, work_plan: null, grants: [], last_report: null, result: null, annotations: [], location: { boot_id: "boot", terminal_id: "terminal", native_session_id: "native", endpoint_identity: "endpoint", session_id: "session", workspace_id: "space", tab_id: "tab", pane_id: "pane", launch_tag: "tag" }, bound_omp_session: "native", bound_omp_process: null, launch_shell_identity: null, retirement: null, supersedes_run_id: null, created_at: at, updated_at: at, ...overrides };
}
function acceptedWorker(state: NonNullable<Run["retirement"]>["state"]): Run {
  const process = { pid: 42, start_ticks: 100, kernel_boot_id: "kernel-boot" };
  return run({ kind: "worker", run_id: "worker", parent_run_id: "root", task_id: "task-a", stage: "closed", close_reason: "accepted",
    bound_omp_process: process,
    result: { message_id: "accepted-result", kind: "result", outcome: "succeeded", summary: "Accepted result kept", plan: null, at },
    retirement: { retirement_id: "retirement-worker", trigger: "accept", result_message_id: "accepted-result", task_revision: "task-revision",
      identity: { run_attempt: 1, launch_attempt: 1, launch_tag: "tag", endpoint_identity: "endpoint", session_id: "session", workspace_id: "space", tab_id: "tab", pane_id: "pane", terminal_id: "terminal", herdr_boot_id: "boot", omp_session_id: "native", process, shell: { process: { pid: 43, start_ticks: 101, kernel_boot_id: "fictional-shell-boot" }, executable_device: "7", executable_inode: "9002", argv_digest: "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb" } },
      state, created_at: at, updated_at: at },
  });
}
function observed(runId = "root", overrides: Partial<RunObservation> = {}): RunObservation {
  return { run_id: runId, presence: "present", actual_omp: true, workspace_id: "space", workspace_label: "Project", tab_id: "tab", tab_label: "Agent", pane_id: "pane", agent_status: "working", state_changed_at: at, ...overrides };
}
function task(overrides: Partial<TaskView> = {}): TaskView {
  return { task: { task_id: "task-a", title: "Improve search", body: "Improve search\nKeep existing behavior.", description: "Improve search\nKeep existing behavior.", description_editable: true, description_diagnostic: null, steps: [], step_progress: { done: 0, total: 0 }, steps_diagnostic: null, depends_on: [], follow_up_of: null, relations_diagnostic: null, checked: false, line: 1, task_revision: "task-revision", diagnostic: null }, lane: "working", current_run_id: "worker", dependencies: { state: "none", unmet: [], problems: [] }, ...overrides };
}
function board(rootId = "root", tasks: TaskView[] = []): TaskBoard {
  return { root_id: rootId, path: `/state/${rootId}.md`, doc_revision: "document-revision", unidentified_items: 0, diagnostics: [], tasks };
}
function snapshot(runs: Run[] = [run()], tasks: TaskView[] = []): OrchestrationSnapshot {
  return { session_id: "session", revision: 1, tasks_token: "tasks", roots: runs.filter(run => !run.parent_run_id).map(run => ({ root_id: run.run_id, label: run.label, kind: run.kind, open_runs: 1, needs_you: 0 })), board: board("root", tasks), runs, messages: [], questions: runs.filter(run => run.stage !== "closed" && run.last_report?.kind === "needs_input").map(run => ({ run_id: run.run_id, question_message_id: run.last_report!.message_id, asked_at: run.last_report!.at, receipt: { status: "unresolved" } })), subagents: [], intents: [], assignment_intents: [], attention: runs.filter(run => run.run_id === run.root_id && run.last_report?.kind === "needs_input").map(run => ({ kind: "needs_input", run_id: run.run_id, task_id: null, message_seq: 1, since: at })), unmanaged_agents: [], runtime: { status: "fresh", endpoint_identity: "endpoint", observed_at: at, runs: runs.map(run => observed(run.run_id)) } };
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
function press(target: HTMLElement, key: string) {
  const event = new KeyboardEvent("keydown", { key, bubbles: true, cancelable: true });
  act(() => target.dispatchEvent(event));
  return event;
}
async function mount(initial: OrchestrationSnapshot, boards?: Map<string, TaskBoard>) {
  let state = initial;
  const onTerminal = vi.fn(async () => undefined), onClose = vi.fn();
  const snapshotCall = vi.fn<CockpitClient["orchestrationSnapshot"]>(async request => ({ ...state, board: request.root_id ? boards?.get(request.root_id) ?? state.board : state.board }));
  const mutation = vi.fn<CockpitClient["orchestrationMutate"]>(async request => {
    state = { ...state, revision: state.revision + 1 };
    if (request.action.action === "message_send") return { revision: state.revision, result: { result: "message", to_run_id: request.action.to_run_id, seq: 1, duplicate: false, stale: false } };
    if (request.action.action === "supervisor_start") return { revision: state.revision, result: { result: "run", run_id: "new-root", attempt: 1 } };
    return { revision: state.revision, result: { result: "done" } };
  });
  let wake: (() => void) | null = null;
  const client = {
    orchestrationSnapshot: snapshotCall,
    orchestrationWait: vi.fn(() => new Promise<OrchestrationWaitResponse>(resolve => {
      wake = () => resolve({ revision: state.revision, tasks_token: state.tasks_token, changed: true });
    })),
    orchestrationMutate: mutation,
  } as unknown as CockpitClient;
  host = document.createElement("div"); document.body.append(host); reactRoot = createRoot(host);
  rerender = async (active = true, runtimeLive = true) => {
    await act(async () => { reactRoot!.render(<SupervisorView client={client} sessionId="session" session={session} runtimeLive={runtimeLive} active={active} startToken={0} navigationError={null} onClose={onClose} onTerminal={onTerminal} onModalChange={vi.fn()} />); });
    await settle();
  };
  await rerender();
  return {
    mutation, onTerminal, onClose, snapshotCall,
    replace: (next: OrchestrationSnapshot) => { state = next; },
    push: async (next: OrchestrationSnapshot) => { state = next; await act(async () => { wake?.(); }); await settle(); },
  };
}
function workarea(width: number, height: number) {
  const original = HTMLElement.prototype.getBoundingClientRect;
  vi.spyOn(HTMLElement.prototype, "getBoundingClientRect").mockImplementation(function (this: HTMLElement) {
    return this.classList.contains("supervisor-workarea") ? new DOMRect(0, 0, width, height) : original.call(this);
  });
}
async function expandQueue() {
  for (const row of host.querySelectorAll<HTMLButtonElement>(".supervisor-queue-summary")) {
    if (row.getAttribute("aria-expanded") !== "true") { act(() => row.click()); await settle(); }
  }
}
afterEach(async () => { if (reactRoot) await act(async () => reactRoot!.unmount()); reactRoot = null; host?.remove(); vi.restoreAllMocks(); });

describe("Supervisor authority and retained operations", () => {
  it("does not turn a historical binding or ACK into fresh OMP proof", () => {
    const saved = run(), state = snapshot([saved]);
    state.runtime = { status: "fresh", endpoint_identity: "endpoint", observed_at: at, runs: [observed("root", { actual_omp: false })] };
    expect(agentState(state, saved, true, true).verified).toBe(false);
    state.runtime.runs[0].actual_omp = true;
    expect(agentState(state, saved, true, true).verified).toBe(true);
    expect(agentState(state, saved, false, true).verified).toBe(false);
    expect(agentState(state, saved, true, false).verified).toBe(false);
  });
  it("requires explicit successful Result and acceptance, not runtime Done or a checkbox", () => {
    const worker = run({ kind: "worker", run_id: "worker", parent_run_id: "root", task_id: "task-a", stage: "working" });
    const state = snapshot([run(), worker], [task()]);
    const checked = task({ lane: "accepted", task: { ...task().task, checked: true } });
    expect(taskStatus(checked, worker, state)).not.toBe("Completed");
    worker.stage = "closed"; worker.close_reason = "accepted";
    worker.result = { message_id: "result", kind: "result", outcome: "succeeded", summary: "Tests passed", plan: null, at };
    expect(taskStatus(checked, worker, state)).toBe("Completed");
    worker.close_reason = "cancelled";
    expect(taskStatus(checked, worker, state)).not.toBe("Completed");
  });
  it.each(["native_stop", "terminal_close"] as const)("keeps retirement ambiguity in Recover with canonical Done context and no retry/close writes (%s)", async phase => {
    const receiptText = "Opaque receipt text must not select user-facing copy";
    const worker = acceptedWorker({ state: "unknown", at, phase, detail: receiptText });
    const completed = task({ lane: "accepted", task: { ...task().task, checked: true } });
    const state = snapshot([run(), worker], [completed]);
    state.attention = [{ kind: "retirement_unconfirmed", run_id: "worker", task_id: "task-a", message_seq: null, since: at }];
    const fixture = await mount(state);
    act(() => host.querySelector<HTMLButtonElement>('[data-view-segment="graph"]')!.click()); await settle();
    const queue = host.querySelector<HTMLElement>(".supervisor-queue-row.is-recover")!;
    act(() => queue.querySelector<HTMLButtonElement>(".supervisor-queue-summary")!.click()); await settle();
    expect(queue.textContent).not.toContain(receiptText);
    expect(queue.querySelectorAll("button")).toHaveLength(2); // Disclosure plus saved canonical context, never a run operation.
    act(() => button("Show completed task").click()); await settle();
    expect(host.querySelector('[data-view-segment="tasks"]')?.getAttribute("aria-pressed")).toBe("true");
    expect(host.querySelector<HTMLUListElement>(".is-accepted .supervisor-task-list")!.hidden).toBe(false);
    expect(host.querySelector('[data-row-id="task-a"]')?.getAttribute("aria-expanded")).toBe("true");
    expect(host.querySelector(".supervisor-state-block .supervisor-attention-badge")?.textContent).toBe("Recover");
    expect(host.querySelector('aside[aria-label="Selected details"]')?.textContent).toContain(worker.result!.summary);
    expect(taskStatus(completed, worker, state)).toBe("Completed");
    expect(fixture.mutation).not.toHaveBeenCalled();
    expect(fixture.onTerminal).not.toHaveBeenCalled();
  });
  it("exposes canonical Done and the saved retiring worker separately when the task no longer points at that run", async () => {
    const worker = acceptedWorker({ state: "unknown", at, phase: "terminal_close", detail: "Uncertain receipt" });
    const completed = task({ lane: "accepted", current_run_id: null, task: { ...task().task, checked: true } });
    const state = snapshot([run(), worker], [completed]);
    state.attention = [{ kind: "retirement_unconfirmed", run_id: "worker", task_id: "task-a", message_seq: null, since: at }];
    const fixture = await mount(state);
    act(() => host.querySelector<HTMLButtonElement>(".supervisor-queue-row.is-recover .supervisor-queue-summary")!.click()); await settle();
    act(() => button("Show completed task").click()); await settle();
    expect(host.querySelector<HTMLUListElement>(".is-accepted .supervisor-task-list")!.hidden).toBe(false);
    expect(host.querySelector('[data-row-id="task-a"]')?.getAttribute("aria-expanded")).toBe("true");
    act(() => button("View saved worker details").click()); await settle();
    expect(host.querySelector(".supervisor-state-block .supervisor-attention-badge")?.textContent).toBe("Recover");
    expect(host.querySelector('aside[aria-label="Selected details"]')?.textContent).toContain(worker.result!.summary);
    expect(fixture.mutation).not.toHaveBeenCalled();
    expect(fixture.onTerminal).not.toHaveBeenCalled();
  });
  it.each([true, false])("keeps retained retirement Notice informational in saved details without reopening canonical completion (native stopped: %s)", async nativeStopped => {
    const worker = acceptedWorker({ state: "retained", at, reason: nativeStopped ? "shared_tab" : "user_activity", native_stopped: nativeStopped });
    const completed = task({ lane: "accepted", task: { ...task().task, checked: true } });
    const state = snapshot([run(), worker], [completed]);
    const fixture = await mount(state);
    expect(host.querySelector(".supervisor-queue-row.is-notice,.supervisor-queue-row.is-recover")).toBeNull();
    act(() => button("Show completed tasks").click()); await settle();
    act(() => host.querySelector<HTMLButtonElement>('[data-row-id="task-a"]')!.click()); await settle();
    expect(host.querySelector(".supervisor-state-block .supervisor-attention-badge")?.textContent).toBe("Notice");
    expect(agentState(state, worker, true, true)).toMatchObject({ kind: "closed", verified: false, restartable: false, terminal: false });
    expect(taskStatus(completed, worker, state)).toBe("Completed");
    expect(fixture.mutation).not.toHaveBeenCalled();
    expect(fixture.onTerminal).not.toHaveBeenCalled();
  });
  it("starts exactly one OMP tab in the current Space without requesting Herdr focus", async () => {
    const empty = snapshot([]); empty.board = null;
    const fixture = await mount(empty);
    act(() => button("Start agent").click()); await settle();
    expect(fixture.mutation).toHaveBeenCalledTimes(1);
    expect(fixture.mutation.mock.calls[0][0].action).toEqual({ action: "supervisor_start", target: { target: "existing_space", workspace_id: "space" }, label: null });
    expect(fixture.onTerminal).not.toHaveBeenCalled();
  });
  it("blocks a second launch until an unconfirmed start is explicitly reviewed", async () => {
    const fixture = await mount(snapshot([]));
    fixture.mutation.mockResolvedValueOnce({ revision: 1, result: { result: "done" } });
    act(() => button("Start agent").click()); await settle();
    expect(button("Start agent").disabled).toBe(true);
    // The queue deliberately expands only one row; select Recover rather than opening the subsequent Notice.
    act(() => host.querySelector<HTMLButtonElement>(".supervisor-queue-row.is-recover .supervisor-queue-summary")!.click());
    await settle();
    act(() => button("I have reviewed the previous start").click()); await settle();
    expect(button("Start agent").disabled).toBe(false);
    expect(fixture.mutation).toHaveBeenCalledTimes(1);
  });
  it("returns an unconfirmed Start dialog to an enabled review target when the opener is disabled, without unlocking a second launch", async () => {
    const fixture = await mount(snapshot([]));
    fixture.mutation.mockResolvedValueOnce({ revision: 1, result: { result: "done" } });
    const opener = button("Start options…");
    act(() => { opener.focus(); opener.click(); }); await settle();
    const dialog = document.querySelector<HTMLElement>('[role="dialog"]')!;
    act(() => dialog.querySelector("form")!.dispatchEvent(new Event("submit", { bubbles: true, cancelable: true }))); await settle();
    expect(opener.disabled).toBe(true);
    press(dialog, "Escape"); await settle();
    expect(document.querySelector('[role="dialog"]')).toBeNull();
    const reviewTarget = document.activeElement as HTMLElement;
    expect(reviewTarget.matches(".supervisor-queue-row.is-recover .supervisor-queue-summary,.supervisor-summary-counter.is-recover")).toBe(true);
    expect(host.contains(reviewTarget)).toBe(true);
    expect(reviewTarget.matches(":disabled,[aria-disabled='true']")).toBe(false);
    expect(reviewTarget.closest("[hidden],[inert]")).toBeNull();
    expect(reviewTarget.tabIndex).toBeGreaterThanOrEqual(0);
    expect(button("Start agent").disabled).toBe(true);
    expect(fixture.mutation).toHaveBeenCalledTimes(1);
  });
  it("retains an unknown answer and retries the identical operation, without consuming textarea Escape", async () => {
    const supervisor = run({ last_report: { message_id: "question", kind: "needs_input", outcome: null, summary: "Which checkout?", plan: null, at } });
    const fixture = await mount(snapshot([supervisor]));
    const field = host.querySelector<HTMLTextAreaElement>('[aria-label="Needs you"] textarea')!;
    enter(field, "Use the existing checkout");
    expect(press(field, "Escape").defaultPrevented).toBe(false);
    fixture.mutation.mockResolvedValueOnce({ revision: 1, result: { result: "done" } });
    act(() => button("Send answer").click()); await settle();
    const sent = fixture.mutation.mock.calls[0][0].action;
    expect(sent).toMatchObject({ action: "message_send", kind: "answer", to_run_id: "root", text: "Use the existing checkout", in_reply_to: "question" });
    expect(field.value).toBe("Use the existing checkout");
    act(() => button("Retry same message").click()); await settle();
    expect(fixture.mutation.mock.calls[1][0].action).toEqual(sent);
    expect(field.value).toBe("");
    expect(fixture.onClose).not.toHaveBeenCalled();
  });
  it("keeps a rejected old-question answer separate from the next displayed question", async () => {
    const supervisor = run({ last_report: { message_id: "q1", kind: "needs_input", outcome: null, summary: "First question", plan: null, at } });
    const fixture = await mount(snapshot([supervisor]));
    const field = host.querySelector<HTMLTextAreaElement>('[aria-label="Needs you"] textarea')!;
    enter(field, "Answer to the first question");
    fixture.mutation.mockRejectedValueOnce(new CockpitClientError("http_error", "The question changed", { operationCode: "question_not_current" }));
    act(() => button("Send answer").click()); await settle();
    expect(field.value).toBe("Answer to the first question");
    expect(fixture.mutation.mock.calls[0][0].action).toMatchObject({ in_reply_to: "q1" });
    const next = run({ last_report: { ...supervisor.last_report!, message_id: "q2", summary: "Second question" } });
    await fixture.push({ ...snapshot([next]), revision: 2 });
    const nextField = host.querySelector<HTMLTextAreaElement>('[aria-label="Needs you"] textarea')!;
    expect(nextField.value).toBe("");
    enter(nextField, "Answer to the second question");
    act(() => button("Send answer").click()); await settle();
    expect(fixture.mutation.mock.calls[1][0].action).toMatchObject({ in_reply_to: "q2", text: "Answer to the second question" });
    expect(fixture.mutation.mock.calls[1][0].action).not.toEqual(fixture.mutation.mock.calls[0][0].action);
  });

  it("uses the same receipt status in prerequisite links and Dependencies nodes without changing dependency facts", async () => {
    workarea(1100, 900);
    const worker = run({ run_id: "worker", kind: "worker", parent_run_id: "root", task_id: "task-a", stage: "working", last_report: { message_id: "question", kind: "needs_input", outcome: null, summary: "Which fixture?", plan: null, at } });
    const prerequisite = task();
    const dependent = task({ task: { ...task().task, task_id: "task-b", title: "Follow-on work", depends_on: ["task-a"] }, lane: "queued", current_run_id: null, dependencies: { state: "blocked", unmet: [{ task_id: "task-a", reason: "unchecked" }], problems: [] } });
    const state = snapshot([run(), worker], [prerequisite, dependent]);
    state.questions[0]!.receipt = { status: "answer_delivered", answer: { sender: { type: "run", run_id: "root" }, message_id: "answer", seq: 1, stage: "read", created_at: at, acked_at: null } };
    const fixture = await mount(state);
    act(() => host.querySelector<HTMLButtonElement>('[data-row-id="task-b"]')!.click()); await settle();
    const status = taskStatus(prerequisite, worker, state);
    expect(host.querySelector(".supervisor-dependencies .supervisor-path-row")?.getAttribute("aria-label")).toContain(status);
    act(() => host.querySelector<HTMLButtonElement>('[data-view-segment="dependencies"]')!.click()); await settle();
    expect(host.querySelector('[data-row-id="task:task-a"]')?.getAttribute("aria-label")).toContain(status);
    expect(dependent.dependencies).toEqual({ state: "blocked", unmet: [{ task_id: "task-a", reason: "unchecked" }], problems: [] });
    expect(fixture.mutation).not.toHaveBeenCalled();
  });

  it("clears root Decide on linked delivery, keeps receipt/history, and returns to Decide for a new question", async () => {
    workarea(1100, 900);
    const supervisor = run({ last_report: { message_id: "q1", kind: "needs_input", outcome: null, summary: "Which checkout?", plan: null, at } });
    const fixture = await mount(snapshot([supervisor]));
    const state = snapshot([supervisor]);
    const answer = { sender: { type: "operator" as const }, message_id: "answer", seq: 1, stage: "stored" as const, created_at: "2026-10-06T12:01:00Z", acked_at: null };
    state.questions[0]!.receipt = { status: "answer_delivered", answer };
    state.attention = [];
    await fixture.push({ ...state, revision: 2 });
    expect(host.querySelector('[aria-label="Needs you"]')).toBeNull();
    expect(host.querySelector(".supervisor-summary-counter.is-decide")).toBeNull();
    const summary = host.querySelector(".supervisor-summary")!.textContent;
    expect(summary).toContain(taskStatus(task(), supervisor, state));
    const chip = host.querySelector<HTMLButtonElement>(".supervisor-strip-chips .supervisor-strip-chip")!;
    const observedFacts = chip.getAttribute("aria-label");
    act(() => chip.click());
    await settle();
    expect(host.querySelector(".supervisor-state-block .supervisor-report-summary")?.textContent).toBe(supervisor.last_report!.summary);
    expect(host.querySelector(".supervisor-state-block time[datetime='2026-10-06T12:01:00Z']")).not.toBeNull();
    state.questions[0]!.receipt = { status: "answer_acknowledged", answer: { ...answer, stage: "acked", acked_at: "2026-10-06T12:02:00Z" } };
    await fixture.push({ ...state, revision: 3 });
    expect(host.querySelector(".supervisor-summary")!.textContent).toContain(taskStatus(task(), supervisor, state));
    expect(host.querySelector(".supervisor-state-block time[datetime='2026-10-06T12:02:00Z']")).not.toBeNull();
    expect(chip.getAttribute("aria-label")).toBe(observedFacts);
    state.questions[0]!.receipt = { status: "answer_delivered", answer: { ...answer, seq: 2 } };
    await fixture.push({ ...state, revision: 4 });
    expect(host.querySelector(".supervisor-summary")!.textContent).toContain(taskStatus(task(), supervisor, state));
    expect(host.querySelector(".supervisor-state-block time[datetime='2026-10-06T12:02:00Z']")).toBeNull();
    const next = run({ last_report: { ...supervisor.last_report!, message_id: "q2" } });
    await fixture.push({ ...snapshot([next]), revision: 5 });
    expect(host.querySelector('[aria-label="Needs you"]')).not.toBeNull();
    const progressed = run({ last_report: { ...supervisor.last_report!, message_id: "progress", kind: "progress", summary: "Working on the chosen checkout" } });
    await fixture.push({ ...snapshot([progressed]), revision: 6 });
    expect(host.querySelector('[aria-label="Needs you"]')).toBeNull();
    expect(host.querySelector(".supervisor-state-block time[datetime='2026-10-06T12:01:00Z']")).toBeNull();
  });

  it.each(["blocked", "missing", "offline", "recover"] as const)("never masks root runtime/recovery state with an answer receipt (%s)", async condition => {
    workarea(1100, 900);
    const supervisor = run({ last_report: { message_id: "question", kind: "needs_input", outcome: null, summary: "Which checkout?", plan: null, at } });
    const state = snapshot([supervisor]);
    state.questions[0]!.receipt = { status: "answer_delivered", answer: { sender: { type: "operator" }, message_id: "answer", seq: 1, stage: "stored", created_at: at, acked_at: null } };
    state.attention = condition === "recover" ? [{ kind: "runtime_blocked", run_id: "root", task_id: null, message_seq: null, since: at }] : [];
    if (state.runtime.status === "fresh") state.runtime.runs[0] = observed("root", condition === "missing" ? { presence: "missing", actual_omp: false } : condition === "blocked" ? { agent_status: "blocked" } : {});
    await mount(state);
    if (condition === "offline") await rerender(true, false);
    expect(host.querySelector(".supervisor-summary")!.textContent).not.toContain(taskStatus(task(), supervisor, state));
    const chip = host.querySelector<HTMLButtonElement>(".supervisor-strip-chips .supervisor-strip-chip")!;
    act(() => chip.click()); await settle();
    const block = host.querySelector(".supervisor-state-block")!;
    expect(block.querySelector("p")?.textContent).not.toBe(taskStatus(task(), supervisor, state));
    expect(block.querySelector(`time[datetime="${at}"]`)).not.toBeNull();
    expect(block.querySelector(".supervisor-report-summary")?.textContent).toBe(supervisor.last_report!.summary);
  });

  it.each([true, false])("returns focus when core clears the root question (task exists: %s)", async hasTask => {
    const supervisor = run({ last_report: { message_id: "question", kind: "needs_input", outcome: null, summary: "Which checkout?", plan: null, at } });
    const tasks = hasTask ? [task({ lane: "queued", current_run_id: null })] : [];
    const fixture = await mount(snapshot([supervisor], tasks));
    const field = host.querySelector<HTMLTextAreaElement>('[aria-label="Needs you"] textarea')!;
    act(() => field.focus()); enter(field, "Use this checkout");
    fixture.mutation.mockImplementationOnce(async () => { fixture.replace({ ...snapshot([run()], tasks), revision: 2 }); return { revision: 2, result: { result: "message", to_run_id: "root", seq: 1, duplicate: false, stale: false } }; });
    act(() => button("Send answer").click()); await settle();
    expect(host.querySelector('[aria-label="Needs you"]')).toBeNull();
    expect(document.activeElement).toBe(hasTask ? host.querySelector('[data-row-id="task-a"]') : button("Start agent"));
  });
  it("keeps root-specific answer drafts and view choices through switching, hide and offline", async () => {
    const first = run({ last_report: { message_id: "ask", kind: "needs_input", outcome: null, summary: "Which checkout?", plan: null, at } });
    const second = run({ run_id: "second", root_id: "second", label: "Other supervisor" });
    const fixture = await mount(snapshot([first, second]), new Map([["root", board()], ["second", board("second")]]));
    const field = host.querySelector<HTMLTextAreaElement>('[aria-label="Needs you"] textarea')!;
    enter(field, "Retained answer");
    act(() => host.querySelector<HTMLButtonElement>('[data-view-segment="graph"]')!.click()); await settle();
    enter(host.querySelector<HTMLSelectElement>('select[aria-label="Agent"]')!, "second"); await settle();
    expect(host.querySelector('[data-view-segment="tasks"]')?.getAttribute("aria-pressed")).toBe("true");
    enter(host.querySelector<HTMLSelectElement>('select[aria-label="Agent"]')!, "root"); await settle();
    await rerender(false); await rerender(true, false);
    expect(host.querySelector('[data-view-segment="graph"]')?.getAttribute("aria-pressed")).toBe("true");
    expect(host.querySelector<HTMLTextAreaElement>('[aria-label="Needs you"] textarea')!.value).toBe("Retained answer");
    expect(button("Send answer").disabled).toBe(true);
    expect(fixture.onTerminal).not.toHaveBeenCalled();
  });
  it("resolves assignment and acceptance conflicts using canonical revision and exact intent identity", async () => {
    const state = snapshot([run()], [task({ current_run_id: null, lane: "queued" })]);
    state.assignment_intents = [{ root_id: "root", task_id: "task-a", state: "conflict" }];
    const fixture = await mount(state); await expandQueue();
    act(() => button("Assign current task").click()); await settle();
    expect(fixture.mutation.mock.calls[0][0].action).toEqual({ action: "task_assignment_resolve", root_id: "root", task_id: "task-a", expected_task_revision: "task-revision", assign: true });
    act(() => button("Keep unassigned").click()); await settle();
    expect(fixture.mutation.mock.calls[1][0].action).toEqual({ action: "task_assignment_resolve", root_id: "root", task_id: "task-a", expected_task_revision: null, assign: false });
    const next = { ...state, revision: 4, assignment_intents: [], intents: [{ intent_id: "exact-intent", root_id: "root", task_id: "task-a", run_id: "root", expected_task_revision: "old-revision", state: "conflict" as const, origin: null, supervisor_run_id: null, omp_session_id: null, result_message_id: null }], attention: [{ kind: "intent_conflict" as const, run_id: "root", task_id: "task-a", message_seq: null, since: at }] };
    fixture.replace(next); await rerender(false); await rerender(true); await expandQueue();
    act(() => button("Apply acceptance to current task").click()); await settle();
    expect(fixture.mutation.mock.calls[2][0].action).toEqual({ action: "intent_resolve", intent_id: "exact-intent", apply: true });
    expect(state.board!.tasks[0].task.body).toBe(task().task.body);
  });
  it("reconciles a missing launch before allowing an explicit retry", async () => {
    const original = run(), state = snapshot([original]);
    state.runtime = { status: "fresh", endpoint_identity: "endpoint", observed_at: at, runs: [observed("root", { presence: "missing", actual_omp: false, pane_id: null })] };
    const fixture = await mount(state); await expandQueue();
    fixture.mutation.mockImplementationOnce(async () => { fixture.replace({ ...state, revision: 2, runs: [run({ dispatch: { ...original.dispatch!, step: "launch_unknown" } })] }); return { revision: 2, result: { result: "done" } }; });
    act(() => button("Restart agent…").click()); await settle();
    expect(fixture.mutation.mock.calls[0][0].action).toEqual({ action: "reconcile_run", run_id: "root", recovery: null });
    act(() => button("Restart anyway").click()); await settle();
    expect(fixture.mutation.mock.calls[1][0].action).toEqual({ action: "retry_launch", run_id: "root" });
    expect(fixture.mutation.mock.calls.some(([request]) => request.action.action === "supervisor_start")).toBe(false);
  });
  it("retains cancelled edit drafts and disables editing when disconnected", async () => {
    const worker = run({ kind: "worker", run_id: "worker", parent_run_id: "root", task_id: "task-a", stage: "reported" });
    const fixture = await mount(snapshot([run(), worker], [task()]));
    act(() => host.querySelector<HTMLButtonElement>('[data-row-id="task-a"]')!.click()); await settle();
    act(() => button("Actions").click()); await settle();
    act(() => button("Edit task…").click()); await settle();
    enter(document.querySelector<HTMLInputElement>('[role="dialog"] input')!, "Retained edit");
    act(() => button("Cancel").click()); await settle();
    act(() => [...document.querySelectorAll<HTMLButtonElement>("button")].find(button => button.textContent?.startsWith("Edit task…"))!.click()); await settle();
    expect(document.querySelector<HTMLInputElement>('[role="dialog"] input')!.value).toBe("Retained edit");
    await rerender(false); fixture.snapshotCall.mockRejectedValue(new Error("transport offline")); await rerender(true);
    expect(button("Save task").disabled).toBe(true);
    expect(document.querySelector<HTMLInputElement>('[role="dialog"] input')!.value).toBe("Retained edit");
  });
});

describe("Supervisor selection, scroll and panel transitions", () => {
  it("restores independent graph/board/lane offsets and keeps selection and filters on view changes", async () => {
    const worker = run({ kind: "worker", run_id: "worker", parent_run_id: "root", task_id: "task-a", stage: "working" });
    const fixture = await mount(snapshot([run(), worker], [task()]));
    const board = host.querySelector<HTMLDivElement>(".supervisor-board")!, lane = host.querySelector<HTMLUListElement>(".is-working .supervisor-task-list")!;
    board.scrollLeft = 84; lane.scrollTop = 110;
    act(() => host.querySelector<HTMLButtonElement>('[data-row-id="task-a"]')!.click()); await settle();
    act(() => button("Attention · 0").click()); await settle();
    const segment = host.querySelector<HTMLButtonElement>('[data-view-segment="graph"]')!;
    act(() => { segment.focus(); segment.click(); }); await settle();
    expect(document.activeElement).toBe(segment);
    expect(host.querySelector('[data-row-id="task:task-a"]')?.getAttribute("aria-expanded")).toBe("true");
    const graph = host.querySelector<HTMLDivElement>(".supervisor-graph-scroll")!; graph.scrollLeft = 220; graph.scrollTop = 73;
    act(() => host.querySelector<HTMLButtonElement>('[data-view-segment="tasks"]')!.click()); await settle();
    expect(host.querySelector<HTMLDivElement>(".supervisor-board")!.scrollLeft).toBe(84);
    expect(host.querySelector<HTMLUListElement>(".is-working .supervisor-task-list")!.scrollTop).toBe(110);
    expect(button("Attention · 0").getAttribute("aria-pressed")).toBe("true");
    act(() => button("Show in Graph").click()); await settle();
    expect(host.querySelector<HTMLDivElement>(".supervisor-graph-scroll")!.scrollLeft).toBe(220);
    expect(host.querySelector<HTMLDivElement>(".supervisor-graph-scroll")!.scrollTop).toBe(73);
    expect(document.activeElement).toBe(host.querySelector('[data-row-id="task:task-a"]'));
    expect(fixture.onTerminal).not.toHaveBeenCalled();
  });
  it("restores a narrow Board without revealing an unassigned task when there is no selection", async () => {
    const original = HTMLElement.prototype.getBoundingClientRect;
    vi.spyOn(HTMLElement.prototype, "getBoundingClientRect").mockImplementation(function (this: HTMLElement) {
      if (this.classList.contains("supervisor-workarea")) return new DOMRect(0, 0, 360, 700);
      if (this.classList.contains("supervisor-board")) return new DOMRect(0, 120, 360, 580);
      return original.call(this);
    });
    vi.spyOn(Element.prototype, "clientHeight", "get").mockImplementation(function (this: Element) { return this.classList.contains("supervisor-board") ? 580 : 0; });
    vi.spyOn(Element.prototype, "clientWidth", "get").mockImplementation(function (this: Element) { return this.classList.contains("supervisor-board") ? 360 : 0; });
    const fixture = await mount(snapshot([run()], [task({ lane: "queued", current_run_id: null })]));
    const lane = host.querySelector<HTMLDetailsElement>(".supervisor-lane-group.is-queued")!;
    act(() => { lane.open = false; lane.dispatchEvent(new Event("toggle", { bubbles: true })); }); await settle();
    const list = host.querySelector<HTMLDivElement>(".supervisor-board")!; list.scrollTop = 73;
    act(() => host.querySelector<HTMLButtonElement>('[data-view-segment="graph"]')!.click()); await settle();
    act(() => host.querySelector<HTMLButtonElement>('[data-view-segment="tasks"]')!.click()); await settle();
    expect(host.querySelector<HTMLDivElement>(".supervisor-board")!.scrollTop).toBe(73);
    expect(host.querySelector<HTMLDetailsElement>(".supervisor-lane-group.is-queued")!.open).toBe(false);
    expect(host.querySelector('[data-row-id="task-a"]')?.getAttribute("aria-expanded")).toBe("false");
    expect(host.querySelector('aside[aria-label="Selected details"]')).toBeNull();
    expect(fixture.onTerminal).not.toHaveBeenCalled();
  });
  it("moves Board keyboard focus without selection and returns from details Escape", async () => {
    const first = task({ lane: "queued", current_run_id: null });
    const next = task({ task: { ...task().task, task_id: "next", title: "Next task" }, lane: "ready", current_run_id: null });
    const fixture = await mount(snapshot([run()], [first, next]));
    const row = host.querySelector<HTMLButtonElement>('[data-row-id="task-a"]')!;
    act(() => row.focus()); press(row, "ArrowRight");
    const target = host.querySelector<HTMLButtonElement>('[data-row-id="next"]')!;
    expect(document.activeElement).toBe(target);
    expect(target.getAttribute("aria-expanded")).toBe("false");
    expect(press(target, "Enter").defaultPrevented).toBe(false);
    act(() => target.click()); await settle();
    act(() => button("Overview").focus()); press(button("Overview"), "Escape");
    await act(async () => { await new Promise<void>(resolve => requestAnimationFrame(() => resolve())); });
    expect(host.querySelector('aside[aria-label="Selected details"]')).toBeNull();
    expect(document.activeElement).toBe(target);
    expect(fixture.onTerminal).not.toHaveBeenCalled(); expect(fixture.mutation).not.toHaveBeenCalled();
  });
  it("keeps a visible-node click at the same graph offsets, and subagent-off selects its worker", async () => {
    const worker = run({ kind: "worker", run_id: "worker", parent_run_id: "root", task_id: "task-a", stage: "working" });
    const state = snapshot([run(), worker], [task()]);
    state.subagents = [{ run_id: "worker", subagent_id: "child", parent_subagent_id: null, role: "Researcher", label: "Research child", status: "running", summary: null, last_control: null, bound_omp_session: null, updated_at: at }];
    const fixture = await mount(state);
    act(() => host.querySelector<HTMLButtonElement>('[data-view-segment="graph"]')!.click()); await settle();
    const graph = host.querySelector<HTMLDivElement>(".supervisor-graph-scroll")!; graph.scrollLeft = 150; graph.scrollTop = 60;
    act(() => host.querySelector<HTMLButtonElement>('[data-row-id="sub:worker:child"]')!.click()); await settle();
    expect(graph.scrollLeft).toBe(150); expect(graph.scrollTop).toBe(60);
    act(() => host.querySelector<HTMLInputElement>(".supervisor-graph-subagents input")!.click()); await settle();
    expect(host.querySelector('[data-row-id="run:worker"]')?.getAttribute("aria-expanded")).toBe("true");
    expect(document.activeElement).toBe(host.querySelector('[data-row-id="run:worker"]'));
    expect(fixture.onTerminal).not.toHaveBeenCalled();
  });
  it("uses the same core Recover item for card, filter and queue without auto-switching or stealing focus", async () => {
    const worker = run({ kind: "worker", run_id: "worker", parent_run_id: "root", task_id: "task-a", stage: "working" });
    const state = snapshot([run(), worker], [task()]);
    const fixture = await mount(state);
    act(() => host.querySelector<HTMLButtonElement>('[data-view-segment="graph"]')!.click()); await settle();
    const segment = host.querySelector<HTMLButtonElement>('[data-view-segment="graph"]')!;
    act(() => segment.focus());
    const next = { ...state, revision: 2, attention: [{ kind: "runtime_blocked" as const, run_id: "worker", task_id: "task-a", message_seq: null, since: at }] };
    await fixture.push(next);
    expect(segment.getAttribute("aria-pressed")).toBe("true");
    expect(document.activeElement).toBe(segment);
    act(() => host.querySelector<HTMLButtonElement>('[data-view-segment="tasks"]')!.click()); await settle();
    act(() => button("Attention · 1").click()); await settle();
    const row = host.querySelector('[data-row-id="task-a"]')!;
    expect(row.closest("li")?.classList.contains("is-dimmed")).toBe(false);
    expect(row.getAttribute("aria-label")).toContain("Recover");
    expect(host.querySelector('.supervisor-queue-row.is-recover')).not.toBeNull();
    expect(fixture.onTerminal).not.toHaveBeenCalled();
  });
  it("keeps literal failures on cards but routine technical report text only in details", async () => {
    const failure = "Compiler rejected operation. No files lost.";
    const worker = run({ kind: "worker", run_id: "worker", parent_run_id: "root", task_id: "task-a", stage: "reported", last_report: { message_id: "failed", kind: "result", outcome: "failed", summary: failure, plan: null, at } });
    worker.result = worker.last_report;
    const fixture = await mount(snapshot([run(), worker], [task({ lane: "review" })]));
    expect(host.querySelector("li.supervisor-task")?.textContent).toContain(failure);
    const ordinary = run({ ...worker, stage: "working", result: null, last_report: { message_id: "progress", kind: "progress", outcome: null, summary: "Exact technical report /tmp/checkouts revision abc123", plan: null, at } });
    fixture.replace({ ...snapshot([run(), ordinary], [task()]), revision: 2 }); await rerender(false); await rerender(true);
    expect(host.querySelector("li.supervisor-task")?.textContent).not.toContain(ordinary.last_report!.summary);
    act(() => host.querySelector<HTMLButtonElement>('[data-row-id="task-a"]')!.click()); await settle();
    expect(host.querySelector('aside[aria-label="Selected details"]')?.textContent).toContain(ordinary.last_report!.summary);
  });
  it("moves to the next canonical task when the focused task disappears", async () => {
    const next = task({ task: { ...task().task, task_id: "next" }, lane: "queued", current_run_id: null });
    const state = snapshot([run()], [task({ lane: "queued", current_run_id: null }), next]);
    state.assignment_intents = [{ root_id: "root", task_id: "pending", state: "pending" }];
    const fixture = await mount(state); await expandQueue();
    act(() => host.querySelector<HTMLButtonElement>('[data-row-id="task-a"]')!.focus());
    fixture.replace({ ...state, revision: 2, board: board("root", [next]) });
    act(() => button("Check assignment status").click()); await settle();
    expect(document.activeElement).toBe(host.querySelector('[data-row-id="next"]'));
  });
  it.each([550, 700])("uses narrow Graph overlay below 560 and a resizable nonmodal sheet above it (%s)", async height => {
    workarea(360, height);
    const worker = run({ kind: "worker", run_id: "worker", parent_run_id: "root", task_id: "task-a", stage: "working" });
    const fixture = await mount(snapshot([run(), worker], [task()]));
    act(() => host.querySelector<HTMLButtonElement>('[data-view-segment="graph"]')!.click()); await settle();
    const node = host.querySelector<HTMLButtonElement>('[data-row-id="run:worker"]')!;
    act(() => { node.focus(); node.click(); }); await settle();
    expect(host.querySelector(".supervisor-view")?.getAttribute("data-panel")).toBe(height < 560 ? "overlay" : "sheet");
    if (height < 560) expect(document.activeElement).toBe(button("Close details"));
    else {
      expect(document.activeElement).toBe(node);
      const splitter = host.querySelector<HTMLElement>('[role="separator"]')!;
      const initial = Number(splitter.getAttribute("aria-valuenow"));
      press(splitter, "ArrowUp"); await settle();
      expect(Number(splitter.getAttribute("aria-valuenow"))).toBe(initial + 16);
      expect(host.querySelector<HTMLElement>(".supervisor-graph-bottom-inset")!.style.height).toBe(`${initial + 16}px`);
    }
    expect(fixture.onTerminal).not.toHaveBeenCalled();
  });
  it.each([593.609375, 243.999])("caps a Graph sheet to leave one whole node below its header, or uses an overlay when 160px is infeasible (%s)", async available => {
    const original = HTMLElement.prototype.getBoundingClientRect;
    vi.spyOn(HTMLElement.prototype, "getBoundingClientRect").mockImplementation(function (this: HTMLElement) {
      if (this.classList.contains("supervisor-workarea")) return new DOMRect(0, 0, 360, 736);
      if (this.classList.contains("supervisor-graph-scroll")) return new DOMRect(0, 736 - available, 360, available);
      return original.call(this);
    });
    vi.spyOn(Element.prototype, "clientHeight", "get").mockImplementation(function (this: Element) {
      return this.classList.contains("supervisor-graph-scroll") ? Math.round(available) : 0;
    });
    const worker = run({ kind: "worker", run_id: "worker", parent_run_id: "root", task_id: "task-a", stage: "working" });
    const fixture = await mount(snapshot([run(), worker], [task()]));
    act(() => host.querySelector<HTMLButtonElement>('[data-view-segment="graph"]')!.click()); await settle();
    act(() => host.querySelector<HTMLButtonElement>('[data-row-id="run:worker"]')!.click()); await settle();
    const feasibleMax = Math.floor(available - 28 - 48 - 8);
    if (feasibleMax >= 160) {
      expect(host.querySelector(".supervisor-view")?.getAttribute("data-panel")).toBe("sheet");
      const splitter = host.querySelector<HTMLElement>('[role="separator"]')!;
      expect(Number(splitter.getAttribute("aria-valuemax"))).toBe(feasibleMax);
      for (let index = 0; index < 20; index++) { press(splitter, "ArrowUp"); await settle(); }
      expect(Number(splitter.getAttribute("aria-valuenow"))).toBe(feasibleMax);
      expect(host.querySelector<HTMLElement>(".supervisor-graph-bottom-inset")!.style.height).toBe(`${feasibleMax}px`);
    } else {
      expect(host.querySelector(".supervisor-view")?.getAttribute("data-panel")).toBe("overlay");
      expect(host.querySelector('[role="separator"]')).toBeNull();
      expect(document.activeElement).toBe(button("Close details"));
    }
    expect(fixture.onTerminal).not.toHaveBeenCalled();
  });
  it("opens narrow attention from a counter and returns Escape focus to that counter", async () => {
    workarea(360, 600);
    const supervisor = run({ last_report: { message_id: "ask", kind: "needs_input", outcome: null, summary: "Which checkout?", plan: null, at } });
    const fixture = await mount(snapshot([supervisor], [task({ lane: "queued", current_run_id: null })]));
    const counter = host.querySelector<HTMLButtonElement>(".supervisor-summary-counter.is-decide")!;
    act(() => { counter.focus(); counter.click(); }); await settle();
    expect(document.activeElement).toBe(button("Close attention"));
    const close = button("Close attention"); press(close, "Escape"); await settle();
    expect(host.querySelector(".supervisor-queue.is-overlay")).toBeNull();
    await act(async () => { await new Promise<void>(resolve => requestAnimationFrame(() => resolve())); });
    expect(document.activeElement).toBe(counter);
    expect(fixture.onClose).not.toHaveBeenCalled();
  });
  it("remembers narrow lane disclosures across view switches and arrows cross visible lane boundaries", async () => {
    workarea(360, 620);
    const first = task({ lane: "queued", current_run_id: null });
    const next = task({ task: { ...task().task, task_id: "next" }, lane: "ready", current_run_id: null });
    await mount(snapshot([run()], [first, next]));
    const row = host.querySelector<HTMLButtonElement>('[data-row-id="task-a"]')!;
    act(() => row.focus()); press(row, "ArrowDown");
    expect(document.activeElement).toBe(host.querySelector('[data-row-id="next"]'));
    const lane = host.querySelector<HTMLDetailsElement>(".supervisor-lane-group.is-queued")!;
    act(() => { lane.open = false; lane.dispatchEvent(new Event("toggle", { bubbles: true })); }); await settle();
    act(() => host.querySelector<HTMLButtonElement>('[data-view-segment="graph"]')!.click()); await settle();
    act(() => host.querySelector<HTMLButtonElement>('[data-view-segment="tasks"]')!.click()); await settle();
    expect(host.querySelector<HTMLDetailsElement>(".supervisor-lane-group.is-queued")!.open).toBe(false);
  });
  it("loads closed-root counts only on archive-open and rejects a mismatched snapshot identity", async () => {
    const closed = run({ run_id: "closed", root_id: "closed", label: "Closed supervisor", stage: "closed", close_reason: "cancelled" });
    const fixture = await mount(snapshot([run(), closed]));
    expect(fixture.snapshotCall.mock.calls.some(([request]) => request.root_id === "closed")).toBe(false);
    fixture.snapshotCall.mockImplementation(async request => request.root_id === "closed" ? { ...snapshot([run(), closed]), session_id: "wrong-session", board: board("closed", [task()]) } : snapshot([run(), closed]));
    act(() => button("Closed tracking · 1").click()); await settle();
    expect(fixture.snapshotCall.mock.calls.filter(([request]) => request.root_id === "closed")).toHaveLength(1);
    expect(host.querySelector(".supervisor-archive-task-count")?.textContent).toBe("Task count unavailable");
    expect(fixture.mutation).not.toHaveBeenCalled();
  });
  it("clears a disappeared selected graph node and focuses the nearest surviving row without terminal navigation", async () => {
    const worker = run({ kind: "worker", run_id: "worker", parent_run_id: "root", task_id: "task-a", stage: "working" });
    const state = snapshot([run(), worker], [task()]);
    const fixture = await mount(state);
    act(() => host.querySelector<HTMLButtonElement>('[data-view-segment="graph"]')!.click()); await settle();
    const node = host.querySelector<HTMLButtonElement>('[data-row-id="run:worker"]')!;
    act(() => { node.focus(); node.click(); }); await settle();
    await fixture.push({ ...snapshot([run(), run({ ...worker, stage: "closed", close_reason: "accepted" })], [task({ lane: "accepted", task: { ...task().task, checked: true } })]), revision: 2 });
    await act(async () => { await new Promise<void>(resolve => requestAnimationFrame(() => resolve())); });
    expect(host.querySelector('aside[aria-label="Selected details"]')).toBeNull();
    expect(document.activeElement).toBe(host.querySelector('[data-row-id="run:root"]'));
    expect(fixture.onTerminal).not.toHaveBeenCalled();
  });
  it("ignores an archive completion from a closed disclosure generation", async () => {
    const closed = run({ run_id: "closed", root_id: "closed", label: "Closed supervisor", stage: "closed" });
    const state = snapshot([run(), closed]);
    const fixture = await mount(state);
    let complete!: (value: OrchestrationSnapshot) => void;
    const deferred = new Promise<OrchestrationSnapshot>(resolve => { complete = resolve; });
    fixture.snapshotCall.mockImplementation(request => request.root_id === "closed" ? deferred : Promise.resolve(state));
    act(() => button("Closed tracking · 1").click()); await settle();
    act(() => button("Closed tracking · 1").click()); await settle();
    fixture.snapshotCall.mockImplementation(async request => request.root_id === "closed" ? { ...state, board: board("closed", []) } : state);
    act(() => button("Closed tracking · 1").click()); await settle();
    expect(host.querySelector(".supervisor-archive-task-count")?.textContent).toBe("0 tasks");
    complete({ ...state, board: board("closed", [task()]) }); await settle();
    expect(host.querySelector(".supervisor-archive-task-count")?.textContent).toBe("0 tasks");
  });
});
