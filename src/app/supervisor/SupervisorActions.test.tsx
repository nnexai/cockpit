// @vitest-environment jsdom
import { act, type ComponentProps, type ReactNode } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, describe, expect, it, vi } from "vitest";
import type { CleanupResourceSummary, OrchestrationActionResult, OrchestrationSnapshot, Report, Run, RunObservation, Subagent, TaskView } from "../../protocol/generated/v1";
import { AgentRecovery, agentState, CleanupFinishAction, finishCleanup, ObservedEvidence, ProgressTrail, ProvenancePair, RunDiagnostics, SupervisorActions, taskStatus, TextAction, type Mutation } from "./SupervisorActions";
import { messageDraft, newScopeDrafts, newTextDraft } from "./useSupervisorDrafts";

const at = "2026-10-06T12:00:00Z";
const mutationScope = { stateStoreId: "store", sessionId: "session", rootId: "root", expectedRevision: null };
function run(overrides: Partial<Run> = {}): Run {
  return { source_output: null, integration: { state: "pending" }, session_id: "session", brief: "Guidance", run_id: "worker", kind: "worker", label: "Search agent", root_id: "root", parent_run_id: "root", task_id: "task-a", attempt: 1, task_revision_at_start: "start-revision", stage: "working", close_reason: null, dispatch: { launch_tag: "tag", endpoint_identity: "endpoint", recovery: null, agent_started: true, step: "launched", launch_attempt: 1, error: null, updated_at: at }, target: { target: "existing_space", workspace_id: "space" }, setup: null, cleanup_policy: "owned_when_safe", last_report: null, result: null, annotations: [], project: null, checkout: null, attachment: { attachment_id: "00000000-0000-4000-8000-000000000001", epoch: 1, state: "connected", launch_mode: null, location: { boot_id: "boot", terminal_id: "terminal", native_session_id: "native", endpoint_identity: "endpoint", session_id: "session", workspace_id: "space", tab_id: "tab", pane_id: "pane", launch_tag: "tag" }, main_session_id: "native", process: null, shell: null, machine_id: null, launch_attempt: 1, updated_at: at }, checkout_claim_id: null, resume: null, retirement: null, created_at: at, updated_at: at, ...overrides };
}
function task(overrides: Partial<TaskView> = {}): TaskView {
  return { task: { task_id: "task-a", title: "Improve search", body: "Improve search\nKeep existing behavior.", description: "Improve search\nKeep existing behavior.", description_editable: true, description_diagnostic: null, steps: [], step_progress: { done: 0, total: 0 }, steps_diagnostic: null, depends_on: [], follow_up_of: null, relations_diagnostic: null, checked: false, line: 1, task_revision: "exact-current-revision", diagnostic: null }, lane: "working", current_run_id: "worker", dependencies: { state: "none", unmet: [], problems: [] }, ...overrides };
}
function observation(runId: string, overrides: Partial<RunObservation> = {}): RunObservation {
  return { run_id: runId, presence: "present", actual_omp: true, workspace_id: "space", workspace_label: "Project", tab_id: "tab", tab_label: "Agent", pane_id: "pane", agent_status: "working", state_changed_at: at, ...overrides };
}
function snapshot(worker = run()): OrchestrationSnapshot {
  const root = run({ run_id: "root", kind: "supervisor", label: "Supervisor", parent_run_id: null, task_id: null, stage: "active" });
  return { state_store_id: "store", cleanup: [], restoration: { reattached: 0, total_attachments: 0, cleanup_pending: 0 }, session_id: "session", revision: 1, tasks_token: "tasks", roots: [{ root_id: "root", label: "Supervisor", kind: "supervisor", open_runs: 2, needs_you: 0 }], board: { root_id: "root", path: "/state/root.md", doc_revision: "doc", unidentified_items: 0, diagnostics: [], tasks: [task()] }, runs: [root, worker], messages: [], questions: [], subagents: [], intents: [], assignment_intents: [], attention: [], unmanaged_agents: [], runtime: { status: "fresh", endpoint_identity: "endpoint", observed_at: at, runs: [observation("root"), observation(worker.run_id)] } };
}
const result: Report = { message_id: "result", kind: "result", outcome: "succeeded", summary: "Exact result\nwith a second line.", plan: null, source_output: null, at };
function subagent(overrides: Partial<Subagent> = {}): Subagent {
  return { run_id: "worker", subagent_id: "scout", parent_subagent_id: null, role: "Read-only scout", label: "Scout", status: "running", summary: "Inspected search", last_control: null, bound_omp_session: null, updated_at: at, ...overrides };
}
let host: HTMLDivElement;
let root: Root | null = null;
async function render(node: ReactNode) {
  if (!root) { host = document.createElement("div"); document.body.append(host); root = createRoot(host); }
  await act(async () => { root!.render(node); });
}
async function click(button: HTMLButtonElement) { await act(async () => { button.click(); }); }
function button(text: string): HTMLButtonElement {
  const found = [...host.querySelectorAll<HTMLButtonElement>("button")].find(item => item.textContent?.trim() === text);
  if (!found) throw new Error(`Missing button: ${text}`);
  return found;
}
function reason(button: HTMLButtonElement): string | null {
  const id = button.getAttribute("aria-describedby");
  return id ? document.getElementById(id)?.textContent ?? null : null;
}
function props(overrides: Partial<ComponentProps<typeof SupervisorActions>> = {}): ComponentProps<typeof SupervisorActions> {
  const worker = run();
  return { snapshot: snapshot(worker), run: worker, task: task(), section: "actions", stateBlock: { sentence: "Supervisor is reviewing this result", tierLabel: null, waitingSince: at }, path: [], crossView: null, acceptanceConflict: false, scope: newScopeDrafts(), changed: vi.fn(), busy: false, live: true, mutateResult: vi.fn<Mutation>(async () => null), captureRequestScope: () => mutationScope, onTerminal: vi.fn(), onEditTask: vi.fn(), onCancelSubagent: vi.fn(), onCloseTracking: vi.fn(), ...overrides };
}
async function openOperator() {
  await act(async () => { const details = host.querySelector<HTMLDetailsElement>(".supervisor-operator")!; details.open = true; details.dispatchEvent(new Event("toggle")); });
}
afterEach(async () => { if (root) await act(async () => root!.unmount()); root = null; host?.remove(); vi.restoreAllMocks(); });

describe("Supervisor detail authority and retained operations", () => {
  it("uses core question receipts for task status, retaining assignment, completion and closure precedence", () => {
    const worker = run({ last_report: { ...result, kind: "needs_input" } });
    const state = snapshot(worker);
    const current = task();
    const question = { run_id: worker.run_id, question_message_id: result.message_id, asked_at: at };
    state.questions = [{ ...question, receipt: { status: "unresolved" } }];
    const unresolved = taskStatus(current, worker, state);
    const answer = { sender: { type: "operator" as const }, message_id: "answer", seq: 1, stage: "stored" as const, created_at: at, acked_at: null };
    state.questions = [{ ...question, receipt: { status: "answer_delivered", answer } }];
    const delivered = taskStatus(current, worker, state);
    state.questions = [{ ...question, receipt: { status: "answer_acknowledged", answer: { ...answer, stage: "acked", acked_at: at } } }];
    const acknowledged = taskStatus(current, worker, state);
    expect(new Set([unresolved, delivered, acknowledged]).size).toBe(3);
    state.questions = [{ ...question, receipt: { status: "answer_delivered", answer: { ...answer, seq: 2 } } }];
    expect(taskStatus(current, worker, state)).toBe(delivered);
    state.assignment_intents = [{ root_id: "root", task_id: current.task.task_id, state: "pending" }];
    expect(taskStatus(current, worker, state)).not.toBe(delivered);
    state.assignment_intents = [];
    expect(taskStatus({ ...current, task: { ...current.task, checked: true } }, worker, state)).not.toBe(delivered);
    expect(taskStatus(current, { ...worker, stage: "closed", close_reason: "cancelled" }, state)).not.toBe(delivered);
    state.questions = [];
    expect(taskStatus(current, worker, state)).toBe("Working");
  });

  it.each(["stored", "woken", "read", "acked"] as const)("shows only recorded sent/ack times and preserves the actual question in detail (%s)", async stage => {
    const sentAt = "2026-10-06T12:01:00Z";
    const ackedAt = "2026-10-06T12:02:00Z";
    const worker = run({ last_report: { message_id: "question", kind: "needs_input", outcome: null, summary: "Which fixture should I use?", plan: null, source_output: null, at } });
    const state = snapshot(worker);
    const answer = { sender: { type: "operator" as const }, message_id: "answer", seq: 1, stage, created_at: sentAt, acked_at: stage === "acked" ? ackedAt : null };
    state.questions = [{ run_id: worker.run_id, question_message_id: "question", asked_at: at, receipt: stage === "acked" ? { status: "answer_acknowledged", answer } : { status: "answer_delivered", answer } }];
    const mutation = vi.fn<Mutation>();
    await render(<SupervisorActions {...props({ run: worker, snapshot: state, section: "overview", mutateResult: mutation })} />);
    const block = host.querySelector(".supervisor-state-block")!;
    const times = [...block.querySelectorAll("time")].map(time => time.dateTime);
    expect(times).toContain(sentAt);
    expect(times.filter(time => time === ackedAt)).toHaveLength(stage === "acked" ? 1 : 0);
    expect(block.querySelector(".supervisor-report-summary")?.textContent).toBe(worker.last_report!.summary);
    expect(block.querySelector("button")).toBeNull();
    expect(mutation).not.toHaveBeenCalled();
  });

  it("exposes explicit Result before description without making navigation a control action", async () => {
    const worker = run({ stage: "reported", result });
    const activate = vi.fn();
    const show = vi.fn();
    const state = props({ run: worker, snapshot: snapshot(worker), section: "overview", path: [{ key: "worker", role: "Worker", label: worker.label, facts: ["Herdr"], current: true, depth: 1, subagent: false, onActivate: activate }], crossView: { label: "Show in Graph", onActivate: show } });
    await render(<SupervisorActions {...state} />);
    const overview = host.querySelector<HTMLElement>(".supervisor-detail-section")!;
    const headings = [...overview.querySelectorAll("h3")].map(item => item.textContent);
    expect(headings.indexOf("Result · succeeded")).toBeGreaterThan(headings.indexOf("State "));
    expect(headings.indexOf("Result · succeeded")).toBeLessThan(headings.indexOf("Task description"));
    expect(overview.querySelectorAll(".supervisor-exact-text")[0].textContent).toBe(result.summary);
    expect(overview.querySelector(".supervisor-path-row")?.getAttribute("aria-current")).toBe("true");
    await click(button(`Worker · ${worker.label}Herdr`));
    await click(button("Show in Graph"));
    expect(activate).toHaveBeenCalledOnce();
    expect(show).toHaveBeenCalledOnce();
    expect(state.mutateResult).not.toHaveBeenCalled();
    expect(state.onTerminal).not.toHaveBeenCalled();
    await render(<SupervisorActions {...state} section="activity" />);
    const activity = host.querySelectorAll(".supervisor-detail-section")[1];
    expect([...activity.querySelectorAll("h3")].some(item => item.textContent?.startsWith("Result"))).toBe(false);
  });

  it("retains path-row focus across selection while resetting task-specific stop confirmation", async () => {
    let state = props({ section: "overview", run: null });
    const selectWorker = () => {
      state = { ...state, run: run(), path: state.path.map(row => ({ ...row, current: row.key === "worker" })) };
      root!.render(<SupervisorActions {...state} />);
    };
    state.path = [
      { key: "task", role: "Task", label: "Improve search", facts: [], current: true, depth: 0, subagent: false, onActivate: vi.fn() },
      { key: "worker", role: "Worker", label: "Search agent", facts: [], current: false, depth: 1, subagent: false, onActivate: selectWorker },
    ];
    await render(<SupervisorActions {...state} />);
    const row = button("Worker · Search agent");
    await act(async () => { row.focus(); row.click(); });
    expect(document.activeElement).toBe(row);
    expect(row.getAttribute("aria-current")).toBe("true");
    expect(state.onTerminal).not.toHaveBeenCalled();
    expect(state.mutateResult).not.toHaveBeenCalled();
    state = { ...state, section: "actions" };
    await render(<SupervisorActions {...state} />);
    await click(button("Request stop…"));
    state = { ...state, task: task({ task: { ...task().task, task_id: "task-b" } }) };
    await render(<SupervisorActions {...state} />);
    expect(button("Request stop…").disabled).toBe(false);
    expect([...host.querySelectorAll("button")].some(item => item.textContent === "Request stop")).toBe(false);
    expect(state.mutateResult).not.toHaveBeenCalled();
  });

  it("requires current canonical identity and explicit success, then accepts the latest exact revision", async () => {
    const worker = run({ stage: "reported", result });
    const state = props({ run: worker, snapshot: snapshot(worker) });
    await render(<SupervisorActions {...state} />);
    await openOperator();
    const blocked: Partial<ComponentProps<typeof SupervisorActions>>[] = [
      { busy: true }, { live: false }, { task: null },
      { task: task({ task: { ...task().task, diagnostic: "duplicate ID" } }) },
      { task: task({ current_run_id: "new-attempt" }) },
      { run: { ...worker, result: null } },
      { run: { ...worker, result: { ...result, outcome: "failed" } } },
      { run: { ...worker, result: { ...result, kind: "progress" } } },
    ];
    for (const override of blocked) {
      await render(<SupervisorActions {...state} {...override} />);
      const accept = button("Accept explicit result");
      expect(accept.disabled).toBe(true);
      expect(reason(accept)).toBeTruthy();
      await click(accept);
    }
    expect(state.mutateResult).not.toHaveBeenCalled();
    const revised = task({ task: { ...task().task, task_revision: "revision-after-external-edit" } });
    await render(<SupervisorActions {...state} task={revised} />);
    await click(button("Accept explicit result"));
    expect(state.mutateResult).toHaveBeenCalledExactlyOnceWith({ action: "accept", run_id: "worker", expected_task_revision: "revision-after-external-edit" });
    await render(<SupervisorActions {...state} run={{ ...worker, stage: "working" }} />);
    expect([...host.querySelectorAll("button")].some(item => item.textContent === "Accept explicit result")).toBe(false);
  });

  it.each(["unknown", "conflict", "pending"] as const)("keeps the original working follow-up action and CAS across %s, changed scope and navigation", async outcome => {
    const mutation = vi.fn<Mutation>().mockImplementationOnce(async action => {
      if (action.action !== "task_scope_update") throw new Error("Expected scope update");
      return outcome === "unknown" ? null : { result: "task_scope_updated", update: { operation_id: action.operation_id, state: outcome, task_revision: null, doc_revision: null, instruction_message_id: null, duplicate: false } };
    }).mockImplementationOnce(async action => {
      if (action.action !== "task_scope_update") throw new Error("Expected scope update");
      return { result: "task_scope_updated", update: { operation_id: action.operation_id, state: "applied", task_revision: "updated", doc_revision: "updated-doc", instruction_message_id: "one-instruction", duplicate: true } };
    });
    let state = props({ mutateResult: mutation });
    const draft = messageDraft(state.scope, "followup:worker");
    draft.text = "Also preserve keyboard focus.\nUse the existing worker.";
    state.changed = () => root!.render(<SupervisorActions {...state} />);
    await render(<SupervisorActions {...state} />);
    await click(button("Send follow-up"));
    const first = mutation.mock.calls[0][0];
    expect(first).toEqual({ action: "task_scope_update", operation_id: expect.any(String), root_id: "root", task_id: "task-a", run_id: "worker", expected_task_revision: "exact-current-revision", expected_doc_revision: "doc", title: null, description: `Improve search\nKeep existing behavior.\n\n${draft.text}` });
    expect(draft.operation?.action).toBe(first);
    expect(draft.text).toBe("Also preserve keyboard focus.\nUse the existing worker.");
    expect(draft.notice).toBeNull();
    expect(host.querySelector<HTMLTextAreaElement>("textarea")!.readOnly).toBe(true);
    state = { ...state, section: "overview", live: false };
    await render(<SupervisorActions {...state} />);
    expect(mutation).toHaveBeenCalledOnce();
    state = { ...state, section: "actions", task: task({ task: { ...task().task, description: "Changed externally", task_revision: "changed-revision" } }), snapshot: { ...state.snapshot, board: { ...state.snapshot.board!, doc_revision: "changed-doc" } } };
    await render(<SupervisorActions {...state} />);
    expect(button("Retry same action").disabled).toBe(true);
    await click(button("Retry same action"));
    expect(mutation).toHaveBeenCalledOnce();
    state = { ...state, live: true };
    await render(<SupervisorActions {...state} />);
    expect(mutation).toHaveBeenCalledOnce();
    await click(button("Retry same action"));
    expect(mutation.mock.calls[1][0]).toBe(first);
    expect(draft.operation).toBeNull();
    expect(draft.text).toBe("");
    expect(draft.notice).toBe("Update sent · task description updated");
    expect(mutation.mock.calls.every(([action]) => action.action === "task_scope_update")).toBe(true);
  });

  it("requires the current editable canonical scope for a working follow-up without removing message sends elsewhere", async () => {
    const state = props();
    messageDraft(state.scope, "followup:worker").text = "Refine the existing task";
    for (const override of [{ task: null }, { task: task({ current_run_id: "other-worker" }) }, { taskWriteUnconfirmed: true }, { task: task({ task: { ...task().task, description_editable: false } }) }]) {
      await render(<SupervisorActions {...state} {...override} />);
      expect(button("Send follow-up").disabled).toBe(true);
      expect(reason(button("Send follow-up"))).toBeTruthy();
      await click(button("Send follow-up"));
    }
    expect(state.mutateResult).not.toHaveBeenCalled();
    const rootRun = state.snapshot.runs[0];
    messageDraft(state.scope, "followup:root").text = "Answer the current question";
    await render(<SupervisorActions {...state} run={rootRun} task={null} />);
    await click(button("Send follow-up"));
    expect(state.mutateResult).toHaveBeenCalledExactlyOnceWith({ action: "message_send", message_id: expect.any(String), to_run_id: "root", kind: "instruction", text: "Answer the current question", in_reply_to: null }, mutationScope);
  });

  it("shows only report-supplied advisory plans and Initialized evidence, never permission controls", async () => {
    const worker = run({ last_report: { ...result, kind: "initialized", summary: "Loaded the task context", plan: "Advisory sequence" } });
    const state = props({ run: worker, snapshot: snapshot(worker), section: "activity" });
    await render(<SupervisorActions {...state} />);
    const activity = host.querySelectorAll(".supervisor-detail-section")[1];
    expect(activity.textContent).toContain("Advisory plan");
    expect(activity.textContent).toContain("Advisory sequence");
    expect(activity.textContent).toContain("Initialized");
    expect(activity.textContent).toContain("Loaded the task context");
    expect(host.textContent).not.toContain("Operator plan override");
    expect(state.mutateResult).not.toHaveBeenCalled();
    await render(<SupervisorActions {...state} run={run()} />);
    expect(host.querySelectorAll(".supervisor-detail-section")[1].textContent).not.toContain("Advisory plan");
    const diagnostics = snapshot(worker);
    diagnostics.messages = [{ message_id: "initialized-report", to_run_id: "root", seq: 1, from: { type: "run", run_id: "worker" }, kind: "report", text: worker.last_report!.summary, in_reply_to: null, report: worker.last_report, stale: false, escalated_from: null, from_subagent_id: null, stage: "stored", woken_omp_session: null, created_at: at, acked_at: null }];
    await render(<RunDiagnostics run={worker} snapshot={diagnostics} />);
    expect(host.querySelector("summary")!.textContent).toContain("Initialized");
    expect(host.textContent).toContain("Run, binding and setup");
    expect(host.textContent).not.toContain("exact plans");
  });

  it("opens result review on a conflict transition without changing the task or terminal", async () => {
    const state = props({ run: run({ stage: "reported", result }) });
    await render(<SupervisorActions {...state} />);
    expect(host.querySelector<HTMLDetailsElement>(".supervisor-operator")!.open).toBe(false);
    await render(<SupervisorActions {...state} acceptanceConflict />);
    expect(host.querySelector<HTMLDetailsElement>(".supervisor-operator")!.open).toBe(true);
    expect(state.mutateResult).not.toHaveBeenCalled();
    expect(state.onTerminal).not.toHaveBeenCalled();
  });

  it("retains the stop identity across unknown delivery and disconnection, targeting only the supervisor", async () => {
    const mutation = vi.fn<Mutation>().mockResolvedValueOnce(null).mockResolvedValueOnce({ result: "message", to_run_id: "root", seq: 1, duplicate: true, stale: false });
    let state = props({ mutateResult: mutation });
    state.changed = () => root!.render(<SupervisorActions {...state} />);
    await render(<SupervisorActions {...state} />);
    await click(button("Request stop…"));
    expect(mutation).not.toHaveBeenCalled();
    await click(button("Request stop"));
    const first = mutation.mock.calls[0][0];
    expect(first.action).toBe("message_send");
    if (first.action !== "message_send") throw new Error("Expected supervisor message");
    expect(first.to_run_id).toBe("root");
    expect(host.textContent).not.toContain("Stop requested.");
    state = { ...state, live: false };
    await render(<SupervisorActions {...state} />);
    expect(button("Retry same stop request").disabled).toBe(true);
    expect(reason(button("Retry same stop request"))).toBeTruthy();
    state = { ...state, live: true };
    await render(<SupervisorActions {...state} />);
    await click(button("Retry same stop request"));
    expect(mutation.mock.calls[1][0]).toEqual(first);
    expect(messageDraft(state.scope, "stop:task-a").operation).toBeNull();
    expect(host.textContent).toContain("this is not proof the process stopped");
  });

  it("blocks stop authority when the supervisor is merely a bound shell", async () => {
    const state = props();
    if (state.snapshot.runtime.status !== "fresh") throw new Error("Expected fresh fixture");
    state.snapshot.runtime.runs[0].actual_omp = false;
    await render(<SupervisorActions {...state} />);
    expect(button("Request stop…").disabled).toBe(true);
    expect(reason(button("Request stop…"))).toContain("verified active supervisor");
    await click(button("Request stop…"));
    expect(state.mutateResult).not.toHaveBeenCalled();
  });

  it.each(["Send back", "Save note"])("locks %s after an unknown outcome until deliberate unlock, preserving drafts across segments", async submitLabel => {
    let state = props({ run: run({ stage: "reported", result }) });
    const key = submitLabel === "Send back" ? "sendback:worker" : "note:worker";
    const draft = messageDraft(state.scope, key);
    draft.text = "Keep exact requested text";
    state.changed = () => root!.render(<SupervisorActions {...state} />);
    await render(<SupervisorActions {...state} />);
    await openOperator();
    await click(button(submitLabel));
    expect(draft.operation).not.toBeNull();
    expect(draft.text).toBe("Keep exact requested text");
    expect(draft.notice).toBeNull();
    state = { ...state, section: "overview" };
    await render(<SupervisorActions {...state} />);
    state = { ...state, section: "actions" };
    await render(<SupervisorActions {...state} />);
    const form = [...host.querySelectorAll<HTMLFormElement>("form")].find(item => item.querySelector("textarea")?.value === draft.text)!;
    expect(form.querySelector("textarea")!.readOnly).toBe(true);
    expect(form.querySelector<HTMLButtonElement>('button[type="submit"]')!.disabled).toBe(true);
    await click(form.querySelector<HTMLButtonElement>('button[type="submit"]')!);
    expect(state.mutateResult).toHaveBeenCalledOnce();
    await click(form.querySelector<HTMLButtonElement>('button[type="button"]')!);
    expect(draft.operation).toBeNull();
    expect(draft.text).toBe("Keep exact requested text");
  });

  it("keeps subagent stored/applied/failed receipts distinct and forbids terminal-result authority", async () => {
    const child = subagent({ last_control: { seq: 1, op: { op: "send", text: "Inspect" }, stage: "stored", error: null, at } });
    const state = props({ subagent: child, section: "overview", run: run({ stage: "reported", result }) });
    await render(<SupervisorActions {...state} />);
    expect(host.querySelector(".supervisor-pair")).toBeNull();
    expect([...host.querySelectorAll("h3")].some(item => item.textContent?.startsWith("Result"))).toBe(false);
    expect([...host.querySelectorAll("button")].some(item => item.textContent === "Accept explicit result")).toBe(false);
    for (const stage of ["stored", "applied", "failed"] as const) {
      await render(<SupervisorActions {...state} section="activity" subagent={{ ...child, last_control: { ...child.last_control!, stage, error: stage === "failed" ? "Control endpoint unavailable" : null } }} />);
      const activity = host.querySelectorAll(".supervisor-detail-section")[1];
      expect(activity.textContent).toContain(`Last control · send · ${stage}`);
      expect(activity.textContent).toContain("Applied delivery is not task completion.");
      if (stage === "failed") expect(activity.textContent).toContain("Control endpoint unavailable");
    }
    expect(state.onTerminal).not.toHaveBeenCalled();
  });

  it("never retries subagent Send automatically or permits controls after the run closes", async () => {
    let state = props({ subagent: subagent() });
    const draft = messageDraft(state.scope, "subagent:worker:scout");
    draft.text = "Inspect only";
    state.changed = () => root!.render(<SupervisorActions {...state} />);
    await render(<SupervisorActions {...state} />);
    await click(button("Send message"));
    expect(draft.operation).not.toBeNull();
    expect(draft.notice).toBeNull();
    expect(button("Check previous action first").disabled).toBe(true);
    await click(button("Check previous action first"));
    expect(state.mutateResult).toHaveBeenCalledOnce();
    await click(button("I reviewed the previous action; unlock draft"));
    for (const status of ["done", "failed", "cancelled"] as const) {
      state = { ...state, subagent: subagent({ status }) };
      await render(<SupervisorActions {...state} />);
      expect(button("Send message").disabled).toBe(true);
      expect(button("Cancel subagent…").disabled).toBe(true);
      expect(reason(button("Send message"))).toContain("Only a running subagent");
      await click(button("Send message"));
      await click(button("Cancel subagent…"));
    }
    expect(state.mutateResult).toHaveBeenCalledOnce();
    expect(state.onCancelSubagent).not.toHaveBeenCalled();
    state = { ...state, live: false, subagent: subagent() };
    await render(<SupervisorActions {...state} />);
    expect(button("Send message").disabled).toBe(true);
    expect(button("Cancel subagent…").disabled).toBe(true);
    state = { ...state, live: true, run: run({ stage: "closed", close_reason: "cancelled" }) };
    await render(<SupervisorActions {...state} />);
    expect(button("Cancel subagent…").disabled).toBe(true);
    expect(reason(button("Cancel subagent…"))).toContain("Tracking is closed");
    expect([...host.querySelectorAll("button")].some(item => item.textContent === "Open parent terminal")).toBe(false);
    await click(button("Cancel subagent…"));
    expect(state.onCancelSubagent).not.toHaveBeenCalled();
  });

  it("retries text with the retained delivery identity and reports success only on confirmed receipt", async () => {
    const draft = newTextDraft(); draft.text = "Exact follow-up";
    const submit = vi.fn<(text: string, id: string) => Promise<OrchestrationActionResult | null>>().mockResolvedValueOnce(null).mockResolvedValueOnce({ result: "message", to_run_id: "worker", seq: 1, duplicate: true, stale: false });
    const node = () => <TextAction label="Follow-up" submitLabel="Send" draft={draft} changed={() => root!.render(node())} busy={false} submit={submit} success="Follow-up confirmed" />;
    await render(node());
    await click(button("Send"));
    expect(draft.text).toBe("Exact follow-up");
    expect(draft.operation).not.toBeNull();
    expect(draft.notice).toBeNull();
    expect(host.querySelector("textarea")!.readOnly).toBe(true);
    await click(button("Retry same message"));
    expect(submit.mock.calls[1]).toEqual(submit.mock.calls[0]);
    expect(draft.operation).toBeNull();
    expect(draft.text).toBe("");
    expect(draft.notice).toBe("Follow-up confirmed");
  });
});

describe("independent Outcome and optional Cleanup actions", () => {
  function keptResource(kind: CleanupResourceSummary["kind"]): CleanupResourceSummary {
    return { resource_id: kind, kind, ownership: "owned", cleanup: { operation_id: null, state: "kept", reason: { class: "user_work", code: "dirty", message: "uncommitted changes" }, effects: [], updated_at: at } };
  }
  it("renders a labeled independent resource list without promoting acceptance to integration", async () => {
    const worker = run({ stage: "closed", close_reason: "accepted", result });
    const state = props({ run: worker, task: task({ task: { ...task().task, checked: true }, lane: "accepted" }), snapshot: snapshot(worker), section: "overview" });
    state.snapshot.cleanup = [{ run_id: worker.run_id, items: [keptResource("worktree"), { ...keptResource("worker_terminal"), cleanup: { ...keptResource("worker_terminal").cleanup, state: "removed", reason: null } }], kept_count: 1, removed_count: 1, unknown_count: 0, waiting_count: 0 }];
    await render(<SupervisorActions {...state} />);
    const overview = host.querySelectorAll(".supervisor-detail-section")[0];
    expect(overview.querySelector('[aria-label="Outcome"]')?.textContent).toContain("Source complete");
    expect(overview.querySelector('[aria-label="Outcome"]')?.textContent).toContain("Waiting for integration");
    expect(overview.querySelector('ul[aria-label="Cleanup"]')?.textContent).toContain("Task worktree · Kept — uncommitted changes");
    expect(overview.querySelector('ul[aria-label="Cleanup"]')?.textContent).toContain("Worker terminal · Removed");
    expect(taskStatus(state.task!, worker, state.snapshot)).toBe("Done · Source complete");
    expect(overview.querySelector('[role="alert"]')).toBeNull();
    expect(state.mutateResult).not.toHaveBeenCalled();
    expect(state.scope.disclosures.completed).toBe(false);
    expect(host.querySelector("summary")?.textContent).toBe("Advanced");
  });
  it.each(["unknown", "pending", "conflict"] as const)("retains original cleanup UUID/action/request scope through %s and remount", async outcome => {
    const scope = newScopeDrafts(), changed = vi.fn();
    const originalScope = { stateStoreId: "store", sessionId: "session", rootId: "root", expectedRevision: 10 };
    const captureRequestScope = vi.fn(() => originalScope);
    const mutation = vi.fn<Mutation>().mockImplementationOnce(async action => {
      if (action.action !== "cleanup_finish") throw new Error("Expected cleanup");
      return outcome === "conflict" ? null : outcome === "pending" ? { result: "done" } : { result: "cleanup_finished", cleanup: { operation_id: action.request.operation_id, items: [], removed_count: 0, kept_count: 0, unknown_count: 1, waiting_count: 0, duplicate: false } };
    }).mockImplementationOnce(async action => {
      if (action.action !== "cleanup_finish") throw new Error("Expected cleanup");
      return { result: "cleanup_finished", cleanup: { operation_id: action.request.operation_id, items: [], removed_count: 1, kept_count: 0, unknown_count: 0, waiting_count: 0, duplicate: true } };
    });
    const input = { rootId: "root", runId: "worker", scope, changed, busy: false, live: true, mutateResult: mutation, captureRequestScope };
    await render(<CleanupFinishAction {...input} />);
    const finish = button("Finish cleanup"); await act(async () => { finish.focus(); finish.click(); });
    const submitted = messageDraft(scope, "cleanup:root:worker").operation;
    expect(submitted?.action).toEqual({ action: "cleanup_finish", request: { operation_id: submitted!.id, root_id: "root", run_id: "worker" } });
    expect(submitted?.requestScope).toBe(originalScope);
    expect(document.activeElement).toBe(finish);
    await render(<div />); await render(<CleanupFinishAction {...input} captureRequestScope={() => ({ ...originalScope, expectedRevision: 999 })} />);
    expect(mutation).toHaveBeenCalledOnce();
    await click(button("Retry same cleanup"));
    expect(mutation.mock.calls[1]).toEqual(mutation.mock.calls[0]);
    expect(messageDraft(scope, "cleanup:root:worker").operation).toBeNull();
    expect(messageDraft(scope, "cleanup:root:worker").notice).toBe("Cleanup finished");
    expect(captureRequestScope).toHaveBeenCalledOnce();
    expect(changed).toHaveBeenCalledTimes(4);
  });
  it("locks the shared cleanup operation across Overview, Routine and queue while pending", async () => {
    const scope = newScopeDrafts();
    let resolve!: (result: OrchestrationActionResult | null) => void;
    const promise = new Promise<OrchestrationActionResult | null>(accept => { resolve = accept; });
    const mutation = vi.fn<Mutation>(() => promise);
    const input = { rootId: "root", runId: "worker", scope, changed: vi.fn(), mutateResult: mutation, captureRequestScope: () => mutationScope };
    const first = finishCleanup(input);
    await finishCleanup(input);
    expect(mutation).toHaveBeenCalledOnce();
    expect(scope.messages.get("cleanup:root:worker")?.operation).not.toBeNull();
    resolve(null); await first;
    expect(scope.messages.get("cleanup:root:worker")?.operation).not.toBeNull();
  });
  it("clears a confirmed safe wait so the next explicit Finish cleanup reevaluates current subtree resources", async () => {
    const scope = newScopeDrafts(), onNotice = vi.fn();
    const mutation = vi.fn<Mutation>(async action => {
      if (action.action !== "cleanup_finish") throw new Error("Expected cleanup");
      return { result: "cleanup_finished", cleanup: { operation_id: action.request.operation_id, items: [], removed_count: 1, kept_count: 0, unknown_count: 0, waiting_count: 1, duplicate: false } };
    });
    const input = { rootId: "root", runId: "worker", scope, changed: vi.fn(), mutateResult: mutation, captureRequestScope: () => mutationScope, onNotice };
    await finishCleanup(input);
    expect(scope.messages.get("cleanup:root:worker")?.operation).toBeNull();
    expect(scope.messages.get("cleanup:root:worker")?.notice).toContain("Cleanup waiting");
    expect(onNotice).toHaveBeenCalledOnce();
    await finishCleanup(input);
    expect(mutation.mock.calls[1]?.[0]).not.toEqual(mutation.mock.calls[0]?.[0]);
    expect(mutation.mock.calls.every(([action]) => action.action === "cleanup_finish" && action.request.run_id === "worker")).toBe(true);
  });
  it("targets the selected task subtree, not every root resource, when only descendants need cleanup", async () => {
    const parent = run({ run_id: "parent-task", task_id: "parent-task-id", stage: "closed", close_reason: "accepted", result });
    const child = run({ run_id: "child-worker", parent_run_id: parent.run_id, stage: "closed", close_reason: "accepted", result });
    const mutation = vi.fn<Mutation>(async () => null);
    const state = props({ run: parent, task: null, snapshot: snapshot(parent), section: "overview", mutateResult: mutation });
    state.snapshot.runs.push(child);
    state.snapshot.cleanup = [{ run_id: child.run_id, items: [keptResource("worktree")], kept_count: 1, removed_count: 0, unknown_count: 0, waiting_count: 0 }];
    await render(<SupervisorActions {...state} />);
    await click(button("Finish cleanup"));
    expect(state.mutateResult).toHaveBeenCalledWith({ action: "cleanup_finish", request: { operation_id: expect.any(String), root_id: "root", run_id: "parent-task" } }, mutationScope);
    expect(state.scope.messages.get("cleanup:root:parent-task")?.operation?.action).toEqual(mutation.mock.calls[0]?.[0]);
  });
  it("does not send cleanup without fresh scope or reconstruct a retained request from newer scope", async () => {
    const scope = newScopeDrafts(), mutateResult = vi.fn<Mutation>(async () => null);
    await finishCleanup({ rootId: "root", runId: "worker", scope, changed: vi.fn(), mutateResult, captureRequestScope: () => null });
    expect(mutateResult).not.toHaveBeenCalled();
    const draft = messageDraft(scope, "cleanup:root:worker");
    const operation = { id: "original", text: "", action: { action: "cleanup_finish" as const, request: { operation_id: "original", root_id: "root", run_id: "worker" } } };
    draft.operation = operation;
    const captureRequestScope = vi.fn(() => mutationScope);
    await finishCleanup({ rootId: "root", runId: "worker", scope, changed: vi.fn(), mutateResult, captureRequestScope });
    expect(mutateResult).not.toHaveBeenCalled();
    expect(captureRequestScope).not.toHaveBeenCalled();
    expect(draft.operation).toBe(operation);
  });
  it("keeps form text and does not submit when fresh request scope cannot be captured", async () => {
    const draft = newTextDraft(); draft.text = "Keep this answer";
    const submit = vi.fn<ComponentProps<typeof TextAction>["submit"]>(async () => null);
    await render(<TextAction label="Answer" submitLabel="Send answer" draft={draft} changed={vi.fn()} busy={false} submit={submit} success="Answer sent" captureRequestScope={() => null} createAction={(text, message_id) => ({ action: "message_send", message_id, to_run_id: "root", kind: "answer", text, in_reply_to: "question" })} />);
    await click(button("Send answer"));
    expect(submit).not.toHaveBeenCalled();
    expect(draft.text).toBe("Keep this answer");
    expect(draft.operation).toBeNull();
  });
});

describe("Supervisor evidence and recovery boundaries", () => {
  it("uses queued and starting stages while requiring genuine live connection, not Initialized or an advisory plan", () => {
    const queued = run({ stage: "queued", dispatch: null, attachment: null });
    expect(agentState(snapshot(queued), queued, true, true).label).toBe("Worker queued");
    const starting = run({ stage: "starting", dispatch: { ...run().dispatch!, step: "launch_pending" }, attachment: null });
    expect(agentState(snapshot(starting), starting, true, true).label).toBe("Starting…");
    expect(taskStatus(task({ lane: "starting" }), starting, snapshot(starting))).toBe("Starting");
    const worker = run({ last_report: { ...result, kind: "initialized", plan: "Suggested sequence" } });
    const savedReportOnly = snapshot(worker);
    if (savedReportOnly.runtime.status !== "fresh") throw new Error("Expected fresh runtime");
    savedReportOnly.runtime.runs[1].actual_omp = false;
    expect(agentState(savedReportOnly, worker, true, true).verified).toBe(false);
    expect(agentState(snapshot(worker), worker, true, true).verified).toBe(true);
    expect(agentState(snapshot(worker), worker, false, true).verified).toBe(false);
  });

  it("derives progress from start and attachment records, explicit reports and canonical acceptance", async () => {
    const worker = run({ result });
    const checked = task({ task: { ...task().task, checked: true }, lane: "accepted" });
    await render(<ProgressTrail run={worker} task={checked} snapshot={snapshot(worker)} />);
    expect([...host.querySelectorAll("li")].map(item => item.textContent)).toEqual(["Assigned", "Started", "Source complete"]);
    await render(<ProgressTrail run={{ ...worker, stage: "closed", close_reason: "accepted" }} task={checked} snapshot={snapshot(worker)} />);
    expect([...host.querySelectorAll("li")].map(item => item.textContent)).toEqual(["Assigned", "Started", "Source complete", "Done"]);
    await render(<ProgressTrail run={{ ...worker, stage: "closed", close_reason: "accepted" }} task={task()} snapshot={snapshot(worker)} />);
    expect([...host.querySelectorAll("li")].some(item => item.textContent === "Done")).toBe(false);
  });

  it("does not turn offline or unavailable observation into missing or a successful runtime result", async () => {
    const worker = run({ last_report: result });
    const state = snapshot(worker);
    await render(<ProvenancePair run={worker} report={result} observed={observation("worker", { presence: "missing", actual_omp: false })} snapshot={state} live={false} compact />);
    expect(host.textContent).toContain("unobserved");
    expect(host.textContent).not.toContain("missing");
    expect(host.querySelector("time")!.dateTime).toBe(at);
    expect(host.querySelector(".supervisor-pair-reported")?.getAttribute("aria-label")).toContain(new Date(at).toLocaleString());
    state.runtime = { status: "unavailable", error: { code: "offline", message: "Unavailable" } };
    await render(<ProvenancePair run={worker} report={result} observed={observation("worker", { agent_status: "done" })} snapshot={state} live compact />);
    expect(host.textContent).toContain("unobserved");
    expect(host.textContent).not.toContain("done");
    await render(<><ProvenancePair run={{ ...worker, stage: "closed" }} report={result} observed={observation("worker")} snapshot={snapshot(worker)} live compact /><ObservedEvidence run={{ ...worker, stage: "closed" }} observed={observation("worker")} snapshot={snapshot(worker)} live /></>);
    expect(host.textContent).toBe("");
  });

  it("resumes a saved worker directly without check-status, setup or restart permission ladder", async () => {
    const worker = run({ dispatch: null, attachment: null });
    const state = snapshot(worker);
    state.runtime = { status: "fresh", endpoint_identity: "endpoint", observed_at: at, runs: [observation("worker", { presence: "missing", actual_omp: false, pane_id: null })] };
    const callbacks = { onRestart: vi.fn(), onTerminal: vi.fn() };
    await render(<AgentRecovery run={worker} state={agentState(state, worker, true, true)} busy={false} connected runtimeLive {...callbacks} />);
    const resume = button("Resume"); await act(async () => { resume.focus(); resume.click(); });
    expect(callbacks.onRestart).toHaveBeenCalledOnce();
    expect(callbacks.onTerminal).not.toHaveBeenCalled();
    expect(document.activeElement).toBe(resume);
    expect([...host.querySelectorAll("button")].map(item => item.textContent)).toEqual(["Resume"]);
  });
  it("keeps unavailable or endpoint-changed Resume focusable but rejects activation", async () => {
    const worker = run(), state = snapshot(worker), onRestart = vi.fn();
    state.runtime = { status: "fresh", endpoint_identity: "endpoint", observed_at: at, runs: [observation("worker", { presence: "endpoint_changed", actual_omp: false })] };
    for (const connected of [true, false]) {
      await render(<AgentRecovery run={worker} state={agentState(state, worker, connected, true)} busy={false} connected={connected} runtimeLive onRestart={onRestart} onTerminal={vi.fn()} />);
      const resume = button("Resume");
      expect(resume.disabled).toBe(false); expect(resume.getAttribute("aria-disabled")).toBe("true");
      expect(reason(resume)).toContain("fresh matching agent identity");
      await click(resume);
    }
    expect(onRestart).not.toHaveBeenCalled();
  });
});
