// @vitest-environment jsdom
import { act, type ComponentProps, type ReactNode } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, describe, expect, it, vi } from "vitest";
import type { Grant, OrchestrationActionResult, OrchestrationSnapshot, Report, RetirementState, Run, RunObservation, Subagent, TaskView } from "../../protocol/generated/v1";
import { AgentRecovery, agentState, ObservedEvidence, ProgressTrail, ProvenancePair, SupervisorActions, taskStatus, TextAction, type Mutation } from "./SupervisorActions";
import { messageDraft, newScopeDrafts, newTextDraft } from "./useSupervisorDrafts";
import { retirementView } from "./retirementView";

const at = "2026-10-06T12:00:00Z";
function run(overrides: Partial<Run> = {}): Run {
  return { session_id: "session", prepare_brief: "Guidance", run_id: "worker", kind: "worker", label: "Search agent", root_id: "root", parent_run_id: "root", task_id: "task-a", attempt: 1, task_revision_at_propose: "proposed-revision", stage: "working", close_reason: null, dispatch: { launch_tag: "tag", endpoint_identity: "endpoint", recovery: null, agent_started: true, step: "launched", launch_attempt: 1, error: null, updated_at: at }, target: { target: "existing_space", workspace_id: "space" }, setup: null, prepare_plan: null, init_receipt: null, work_plan: null, grants: [], last_report: null, result: null, annotations: [], location: { boot_id: "boot", terminal_id: "terminal", native_session_id: "native", endpoint_identity: "endpoint", session_id: "session", workspace_id: "space", tab_id: "tab", pane_id: "pane", launch_tag: "tag" }, bound_omp_session: "native", bound_omp_process: null, launch_shell_identity: null, retirement: null, supersedes_run_id: null, created_at: at, updated_at: at, ...overrides };
}
function task(overrides: Partial<TaskView> = {}): TaskView {
  return { task: { task_id: "task-a", title: "Improve search", body: "Improve search\nKeep existing behavior.", description: "Improve search\nKeep existing behavior.", description_editable: true, description_diagnostic: null, steps: [], step_progress: { done: 0, total: 0 }, steps_diagnostic: null, depends_on: [], follow_up_of: null, relations_diagnostic: null, checked: false, line: 1, task_revision: "exact-current-revision", diagnostic: null }, lane: "working", current_run_id: "worker", dependencies: { state: "none", unmet: [], problems: [] }, ...overrides };
}
function observation(runId: string, overrides: Partial<RunObservation> = {}): RunObservation {
  return { run_id: runId, presence: "present", actual_omp: true, workspace_id: "space", workspace_label: "Project", tab_id: "tab", tab_label: "Agent", pane_id: "pane", agent_status: "working", state_changed_at: at, ...overrides };
}
function snapshot(worker = run()): OrchestrationSnapshot {
  const root = run({ run_id: "root", kind: "supervisor", label: "Supervisor", parent_run_id: null, task_id: null, stage: "active" });
  return { session_id: "session", revision: 1, tasks_token: "tasks", roots: [{ root_id: "root", label: "Supervisor", kind: "supervisor", open_runs: 2, needs_you: 0 }], board: { root_id: "root", path: "/state/root.md", doc_revision: "doc", unidentified_items: 0, diagnostics: [], tasks: [task()] }, runs: [root, worker], messages: [], subagents: [], intents: [], assignment_intents: [], attention: [], unmanaged_agents: [], runtime: { status: "fresh", endpoint_identity: "endpoint", observed_at: at, runs: [observation("root"), observation(worker.run_id)] } };
}
const result: Report = { message_id: "result", kind: "result", outcome: "succeeded", summary: "Exact result\nwith a second line.", plan: null, at };
function retiredWorker(state: RetirementState): Run {
  const worker = run({ stage: "closed", close_reason: "accepted", result });
  const process = { pid: 4242, start_ticks: 12345, kernel_boot_id: "kernel-boot" };
  worker.bound_omp_process = process;
  worker.retirement = { retirement_id: "retirement", trigger: "accept", result_message_id: result.message_id, task_revision: "exact-current-revision", identity: { run_attempt: 1, launch_attempt: 1, launch_tag: "tag", endpoint_identity: "endpoint", session_id: "session", workspace_id: "space", tab_id: "tab", pane_id: "pane", terminal_id: "terminal", herdr_boot_id: "boot", omp_session_id: "native", process, shell: { process: { pid: 4343, start_ticks: 54321, kernel_boot_id: "fictional-shell-boot" }, executable_device: "7", executable_inode: "9001", argv_digest: "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa" } }, state, created_at: at, updated_at: at };
  return worker;
}
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
  return { snapshot: snapshot(worker), run: worker, task: task(), section: "actions", stateBlock: { sentence: "Supervisor is reviewing this result", tierLabel: null, waitingSince: at }, path: [], crossView: null, acceptanceConflict: false, scope: newScopeDrafts(), changed: vi.fn(), busy: false, live: true, mutateResult: vi.fn<Mutation>(async () => null), onTerminal: vi.fn(), onEditTask: vi.fn(), onCancelSubagent: vi.fn(), onCloseTracking: vi.fn(), ...overrides };
}
async function openOperator() {
  await act(async () => { const details = host.querySelector<HTMLDetailsElement>(".supervisor-operator")!; details.open = true; details.dispatchEvent(new Event("toggle")); });
}
afterEach(async () => { if (root) await act(async () => root!.unmount()); root = null; host?.remove(); vi.restoreAllMocks(); });

describe("Supervisor detail authority and retained operations", () => {
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

  it("disarms stale exact plans, requires re-review and never turns a collapsed disclosure into authorization", async () => {
    const plan = { plan_revision: "plan-a", text: "First exact plan", created_at: at };
    const worker = run({ stage: "awaiting_prepare", prepare_plan: plan });
    const state = props({ run: worker, snapshot: snapshot(worker) });
    await render(<SupervisorActions {...state} />);
    await openOperator();
    await click(button("Override prepare…"));
    await act(async () => { const details = host.querySelector<HTMLDetailsElement>(".supervisor-operator")!; details.open = false; details.dispatchEvent(new Event("toggle")); });
    expect(host.querySelector<HTMLDetailsElement>(".supervisor-operator")!.open).toBe(true);
    const changed = { ...worker, prepare_plan: { ...plan, plan_revision: "plan-b", text: "Changed exact plan" } };
    await render(<SupervisorActions {...state} run={changed} />);
    expect([...host.querySelectorAll("button")].some(item => item.textContent === "Confirm prepare override")).toBe(false);
    await click(button("Reviewed current plan"));
    expect(state.mutateResult).not.toHaveBeenCalled();
    await click(button("Override prepare…"));
    await act(async () => { button("Confirm prepare override").dispatchEvent(new KeyboardEvent("keydown", { key: "Escape", bubbles: true })); });
    expect([...host.querySelectorAll("button")].some(item => item.textContent === "Confirm prepare override")).toBe(false);
    await click(button("Override prepare…"));
    await click(button("Confirm prepare override"));
    expect(state.mutateResult).toHaveBeenCalledExactlyOnceWith({ action: "grant_prepare", run_id: "worker", plan_revision: "plan-b" });
    await render(<SupervisorActions {...state} run={{ ...worker, stage: "working" }} />);
    expect([...host.querySelectorAll("button")].some(item => item.textContent === "Override prepare…")).toBe(false);
  });

  it("requires deliberate execute authorization for the exact work plan and preserves the optional note", async () => {
    const worker = run({ stage: "ready", work_plan: { plan_revision: "exact-execute", text: "Run the scoped work", created_at: at } });
    const mutation = vi.fn<Mutation>(async () => ({ result: "done" }));
    const state = props({ run: worker, mutateResult: mutation });
    messageDraft(state.scope, "execute-note:worker").text = "Keep the existing resources";
    await render(<SupervisorActions {...state} />);
    await openOperator();
    await click(button("Override execute…"));
    expect(mutation).not.toHaveBeenCalled();
    await click(button("Keep supervisor decision"));
    expect(mutation).not.toHaveBeenCalled();
    await click(button("Override execute…"));
    await click(button("Confirm execute override"));
    expect(mutation).toHaveBeenCalledExactlyOnceWith({ action: "grant_execute", run_id: "worker", plan_revision: "exact-execute", note: "Keep the existing resources" });
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

describe("accepted retirement detail transitions", () => {
  it("keeps accepted work Completed while receipt states do not claim native exit or grant live controls", async () => {
    const checked = task({ task: { ...task().task, checked: true }, lane: "accepted" });
    const worker = retiredWorker({ state: "native_stop_offered", offered_at: at });
    const state = props({ run: worker, task: checked, snapshot: snapshot(worker), section: "overview" });
    for (const retirement of [
      { state: "native_stop_offered", offered_at: at },
      { state: "native_stop_requested", at },
      { state: "native_stopped", at, evidence: "exited_after_shutdown_request" },
      { state: "retired", at, terminal: "closed_by_cockpit" },
    ] satisfies RetirementState[]) {
      worker.retirement!.state = retirement;
      await render(<SupervisorActions {...state} />);
      const status = agentState(state.snapshot, worker, true, true);
      expect(status.kind).toBe("closed");
      expect(status.terminal).toBe(false);
      expect(status.restartable).toBe(false);
      expect(status.verified).toBe(false);
      expect(taskStatus(checked, worker, state.snapshot)).toBe("Completed");
      const block = host.querySelector(".supervisor-state-block")!;
      if (retirement.state === "native_stop_offered" || retirement.state === "native_stop_requested") {
        expect(block.textContent).not.toMatch(/worker stopped|worker retired/);
        expect(retirementView(worker)?.nativeStopped).toBe(false);
      } else expect(retirementView(worker)?.nativeStopped).toBe(true);
      expect([...host.querySelectorAll("button")].some(item => ["Open terminal", "Restart agent…", "Accept explicit result", "Close agent tracking…"].includes(item.textContent ?? ""))).toBe(false);
    }
    expect(state.mutateResult).not.toHaveBeenCalled();
    expect(state.onTerminal).not.toHaveBeenCalled();
  });

  it("renders retained retirement as informational Notice without new approval or retry actions", async () => {
    const worker = retiredWorker({ state: "retained", at, reason: "shared_tab", native_stopped: false });
    const state = props({ run: worker, task: task({ task: { ...task().task, checked: true }, lane: "accepted" }), snapshot: snapshot(worker), section: "overview" });
    await render(<SupervisorActions {...state} />);
    const block = host.querySelector(".supervisor-state-block")!;
    expect(block.querySelector(".supervisor-attention-badge")?.textContent).toBe("Notice");
    expect(block.textContent).toContain("Native stop is not confirmed");
    expect(block.querySelector("button")).toBeNull();
    worker.retirement!.state = { state: "retained", at, reason: "shared_tab", native_stopped: true };
    await render(<SupervisorActions {...state} />);
    expect(block.textContent).toContain("worker stopped · terminal kept");
    expect(block.textContent).not.toContain("Native stop is not confirmed");
    expect(block.querySelector("button")).toBeNull();
    expect(state.mutateResult).not.toHaveBeenCalled();
  });

  it("keeps phase-specific uncertainty as saved evidence without enabling a recovery launch", async () => {
    const worker = retiredWorker({ state: "unknown", at, phase: "native_stop", detail: "Claimed exit and retry permission" });
    const state = props({ run: worker, snapshot: snapshot(worker), section: "overview" });
    await render(<SupervisorActions {...state} />);
    expect(host.querySelector(".supervisor-state-block")?.textContent).toContain("stop not confirmed");
    expect(retirementView(worker)?.nativeStopped).toBe(false);
    worker.retirement!.state = { state: "unknown", at, phase: "terminal_close", detail: "Retry closure automatically" };
    await render(<SupervisorActions {...state} />);
    const block = host.querySelector(".supervisor-state-block")!;
    expect(block.textContent).toContain("terminal close not confirmed");
    expect(block.textContent).toContain("will not retry");
    expect(block.textContent).not.toContain("Retry closure automatically");
    expect(retirementView(worker)?.nativeStopped).toBe(true);
    expect([...host.querySelectorAll("button")].some(item => item.textContent?.includes("Restart") || item.textContent?.includes("Retry"))).toBe(false);
    expect(state.mutateResult).not.toHaveBeenCalled();
  });

  it("preserves legacy and cancelled Tracking closed without taking a surviving pane as control authority", () => {
    const worker = retiredWorker({ state: "native_stopped", at, evidence: "already_exited" });
    const state = snapshot(worker);
    for (const legacy of [{ ...worker, retirement: null }, { ...worker, close_reason: "cancelled" as const }]) {
      const status = agentState(state, legacy, true, true);
      expect(status.label).toBe("Tracking closed");
      expect(status.detail).toContain("not guaranteed to have stopped");
      expect(status.kind).toBe("closed");
      expect(status.terminal).toBe(false);
      expect(status.restartable).toBe(false);
      expect(retirementView(legacy)).toBeNull();
    }
  });
});

describe("Supervisor evidence and recovery boundaries", () => {
  it("derives completed progress only from grants, explicit reports and canonical acceptance", async () => {
    const grant: Grant = { grant_id: "prepare", scope: "prepare", plan_revision: "plan", origin: "supervisor", supervisor_run_id: "root", omp_session_id: "native", granted_at: at };
    const worker = run({ grants: [grant], work_plan: { plan_revision: "work", text: "Work", created_at: at }, result });
    const checked = task({ task: { ...task().task, checked: true }, lane: "accepted" });
    await render(<ProgressTrail run={worker} task={checked} snapshot={snapshot(worker)} />);
    expect([...host.querySelectorAll("li")].map(item => item.textContent)).toEqual(["Assigned", "Prepared · by supervisor", "Executing", "Result reported · succeeded"]);
    await render(<ProgressTrail run={{ ...worker, stage: "closed", close_reason: "accepted", grants: [{ ...grant, origin: "native" }] }} task={checked} snapshot={snapshot(worker)} />);
    expect([...host.querySelectorAll("li")].map(item => item.textContent)).toEqual(["Assigned", "Prepared · by you (operator)", "Executing", "Result reported · succeeded", "Accepted"]);
    await render(<ProgressTrail run={{ ...worker, stage: "closed", close_reason: "accepted" }} task={task()} snapshot={snapshot(worker)} />);
    expect([...host.querySelectorAll("li")].some(item => item.textContent === "Accepted")).toBe(false);
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

  it("orders safe recovery ahead of restart and keeps healthy terminal navigation free of closure", async () => {
    const worker = run();
    const state = snapshot(worker);
    const callbacks = { onCheck: vi.fn(), onRestart: vi.fn(), onCloseTracking: vi.fn(), onTerminal: vi.fn() };
    const cases: { worker: Run; observed: RunObservation; live: boolean; primary: string }[] = [
      { worker, observed: observation("worker", { presence: "missing", actual_omp: false, pane_id: null }), live: true, primary: "Check status" },
      { worker, observed: observation("worker", { presence: "endpoint_changed", actual_omp: false }), live: true, primary: "Check status" },
      { worker, observed: observation("worker"), live: false, primary: "Check connection" },
      { worker: { ...worker, dispatch: { ...worker.dispatch!, step: "setup_unknown" } }, observed: observation("worker", { actual_omp: false }), live: true, primary: "Recover setup…" },
      { worker: { ...worker, dispatch: { ...worker.dispatch!, step: "plan_failed", error: { code: "setup", message: "Failed setup" } } }, observed: observation("worker", { actual_omp: false }), live: true, primary: "Retry setup" },
    ];
    for (const item of cases) {
      for (const callback of Object.values(callbacks)) callback.mockClear();
      state.runtime = { status: "fresh", endpoint_identity: "endpoint", observed_at: at, runs: [item.observed] };
      const status = agentState(state, item.worker, item.live, item.live);
      await render(<AgentRecovery run={item.worker} state={status} busy={false} connected={item.live} runtimeLive={item.live} {...callbacks} />);
      expect(host.querySelector("button")!.textContent).toBe(item.primary);
      expect(host.querySelector("button")!.classList.contains("supervisor-primary")).toBe(true);
      await click(host.querySelector<HTMLButtonElement>("button")!);
      if (["setup_unknown", "plan_failed"].includes(item.worker.dispatch?.step ?? "")) {
        expect(callbacks.onRestart).toHaveBeenCalledOnce();
        expect(callbacks.onCheck).not.toHaveBeenCalled();
      } else {
        expect(callbacks.onCheck).toHaveBeenCalledOnce();
        expect(callbacks.onRestart).not.toHaveBeenCalled();
      }
      expect(callbacks.onTerminal).not.toHaveBeenCalled();
      expect(callbacks.onCloseTracking).not.toHaveBeenCalled();
    }
    state.runtime = { status: "fresh", endpoint_identity: "endpoint", observed_at: at, runs: [observation("worker")] };
    await render(<AgentRecovery run={worker} state={agentState(state, worker, true, true)} busy={false} connected runtimeLive {...callbacks} />);
    expect([...host.querySelectorAll("button")].map(item => item.textContent)).toEqual(["Open terminal"]);
    await click(button("Open terminal"));
    expect(callbacks.onTerminal).toHaveBeenCalledOnce();
    expect(callbacks.onCloseTracking).not.toHaveBeenCalled();
  });

  it("keeps disabled restart focusable but prevents its handler from launching anything", async () => {
    const worker = run();
    const state = snapshot(worker);
    state.runtime = { status: "fresh", endpoint_identity: "endpoint", observed_at: at, runs: [observation("worker", { actual_omp: false, presence: "unobserved", pane_id: null })] };
    const onRestart = vi.fn();
    await render(<AgentRecovery run={worker} state={agentState(state, worker, true, true)} busy={false} connected runtimeLive onCheck={vi.fn()} onRestart={onRestart} onCloseTracking={vi.fn()} onTerminal={vi.fn()} />);
    const restart = button("Restart agent…");
    expect(restart.disabled).toBe(false);
    expect(restart.getAttribute("aria-disabled")).toBe("true");
    expect(reason(restart)).toContain("May open another terminal");
    expect(reason(restart)).toContain("Check status first");
    await click(restart);
    expect(onRestart).not.toHaveBeenCalled();
    const fresh = snapshot(worker);
    await render(<AgentRecovery run={worker} state={agentState(fresh, worker, true, true)} busy={false} connected={false} runtimeLive onCheck={vi.fn()} onRestart={onRestart} onCloseTracking={vi.fn()} onTerminal={vi.fn()} />);
    expect([...host.querySelectorAll("button")].some(item => item.textContent === "Open terminal")).toBe(false);
  });
});
