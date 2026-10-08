// @vitest-environment jsdom
import { act, type ReactNode } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, describe, expect, it, vi } from "vitest";
import type { Message, OrchestrationSnapshot, Run, TaskView } from "../../protocol/generated/v1";
import { activityRows, ClosedTracking, SupervisorActivity, SupervisorDiagnostics, type ClosedTaskCount } from "./SupervisorActivity";

const at = "2026-10-07T12:00:00Z";
function run(overrides: Partial<Run> = {}): Run {
  return { session_id: "session", prepare_brief: "Guidance", run_id: "root", kind: "supervisor", label: "Project supervisor", root_id: "root", parent_run_id: null, task_id: null, attempt: 1, task_revision_at_propose: null, stage: "active", close_reason: null, dispatch: { launch_tag: "tag", endpoint_identity: "endpoint", recovery: null, agent_started: true, step: "launched", launch_attempt: 1, error: null, updated_at: at }, target: { target: "existing_space", workspace_id: "space" }, setup: null, prepare_plan: null, init_receipt: null, work_plan: null, grants: [], last_report: null, result: null, annotations: [], location: null, bound_omp_session: "native", bound_omp_process: null, launch_shell_identity: null, retirement: null, supersedes_run_id: null, created_at: at, updated_at: at, ...overrides };
}
function task(): TaskView {
  return { task: { task_id: "task-a", title: "Improve search", body: "Keep existing behavior.", description: "Keep existing behavior.", description_editable: true, description_diagnostic: null, steps: [], step_progress: { done: 0, total: 0 }, steps_diagnostic: null, depends_on: [], follow_up_of: null, relations_diagnostic: null, checked: false, line: 1, task_revision: "task-revision", diagnostic: null }, lane: "working", current_run_id: "worker", dependencies: { state: "none", unmet: [], problems: [] } };
}
function message(overrides: Partial<Message> = {}): Message {
  return { message_id: "message", to_run_id: "root", seq: 1, from: { type: "dispatcher" }, kind: "instruction", text: "Inspect search behavior", in_reply_to: null, report: null, stale: false, escalated_from: null, from_subagent_id: null, stage: "stored", woken_omp_session: null, created_at: at, acked_at: null, ...overrides };
}
function snapshot(runs: Run[] = [run()]): OrchestrationSnapshot {
  return { session_id: "session", revision: 1, tasks_token: "tasks", roots: runs.filter(run => run.run_id === run.root_id).map(run => ({ root_id: run.run_id, label: run.label, kind: run.kind, open_runs: 1, needs_you: 0 })), board: { root_id: "root", path: "/state/tasks.md", doc_revision: "document-revision", unidentified_items: 0, diagnostics: [], tasks: [task()] }, runs, messages: [], questions: [], subagents: [], intents: [], assignment_intents: [], attention: [], unmanaged_agents: [], runtime: { status: "fresh", endpoint_identity: "endpoint", observed_at: at, runs: runs.map(run => ({ run_id: run.run_id, presence: "present", actual_omp: true, workspace_id: "space", workspace_label: "Project", tab_id: "tab", tab_label: "Agent", pane_id: "pane", agent_status: "working", state_changed_at: at })) } };
}
let host: HTMLDivElement;
let root: Root | null = null;
async function render(element: ReactNode) {
  if (!root) { host = document.createElement("div"); document.body.append(host); root = createRoot(host); }
  await act(async () => { root!.render(element); });
}
function button(text: string) {
  const found = [...host.querySelectorAll<HTMLButtonElement>("button")].find(button => button.textContent?.trim() === text);
  if (!found) throw new Error(`Missing button ${text}`);
  return found;
}
async function click(button: HTMLButtonElement) { await act(async () => { button.click(); }); }
afterEach(async () => {
  if (root) await act(async () => root!.unmount());
  root = null; host?.remove(); vi.restoreAllMocks(); vi.unstubAllGlobals();
});

describe("Supervisor readable activity boundaries", () => {
  it("keeps technical bootstrap and receipt identities out of readable records without hiding them from Diagnostics", async () => {
    const worker = run({ kind: "worker", run_id: "worker", label: "Search agent", parent_run_id: "root", task_id: "task-a", stage: "working", grants: [{ grant_id: "grant", scope: "execute", plan_revision: "plan-secret", origin: "supervisor", supervisor_run_id: "root", omp_session_id: "session-secret", granted_at: at }] });
    worker.annotations = ["Acceptance requested for Result ", "Accepted Result at exact task revision ", "Recovered acceptance of Result "].map(prefix => ({ at, by: { type: "run", run_id: "root" }, text: `${prefix}result-secret at revision-secret; session-secret` }));
    const state = snapshot([run(), worker]);
    state.messages = [
      ...(["supervisor_brief", "prepare_brief", "work_brief"] as const).map((kind, index) => message({ message_id: kind, kind, seq: index + 1, text: "BOOTSTRAP-SECRET", to_run_id: "worker" })),
      message({ message_id: "assign-task-a", text: "ASSIGNMENT-SECRET", to_run_id: "root" }),
      message({ message_id: "observation", kind: "observation", text: "OBSERVATION-SECRET", to_run_id: "worker" }),
    ];
    await render(<SupervisorActivity rows={activityRows(state, state.runs, [task()])} onLink={() => undefined} />);
    const activity = host.textContent!;
    for (const secret of ["BOOTSTRAP-SECRET", "ASSIGNMENT-SECRET", "OBSERVATION-SECRET", "plan-secret", "result-secret", "revision-secret", "session-secret"]) expect(activity).not.toContain(secret);
    expect(activity).toContain("Execution authorized · Project supervisor");
    expect(activity).not.toContain("authorized · You");
    expect(activity).toContain("Result review note for Search agent");
    await render(<SupervisorDiagnostics snapshot={state} rootRuns={state.runs} busy={false} connected onIdentify={() => undefined} onCopyPath={() => undefined} />);
    const records = host.querySelector(".supervisor-diagnostics-records")!;
    for (const secret of ["BOOTSTRAP-SECRET", "ASSIGNMENT-SECRET", "OBSERVATION-SECRET", "plan-secret", "result-secret", "revision-secret", "session-secret"]) expect(records.textContent).toContain(secret);
    expect(host.querySelector(".supervisor-diagnostics-summary")!.compareDocumentPosition(records) & Node.DOCUMENT_POSITION_FOLLOWING).toBeTruthy();
  });

  it("routes assignment and worker records to tasks and root records to runs across local-day groups", async () => {
    const worker = run({ kind: "worker", run_id: "worker", label: "Search agent", parent_run_id: "root", task_id: "task-a" });
    const other = run({ run_id: "other", root_id: "other", label: "Other supervisor" });
    const state = snapshot([run(), worker, other]);
    const today = new Date(2026, 9, 7, 0, 5).toISOString();
    const yesterday = new Date(2026, 9, 6, 23, 55).toISOString();
    state.messages = [
      message({ message_id: "assign-task-a", created_at: today }),
      message({ message_id: "worker-report", to_run_id: "worker", kind: "report", report: { message_id: "worker-report", kind: "progress", outcome: null, summary: "Search indexed", plan: null, at: today }, created_at: today, stale: true }),
      message({ message_id: "root-note", created_at: yesterday }),
      message({ message_id: "other-note", to_run_id: "other", text: "Outside this root", created_at: today }),
    ];
    const rows = activityRows(state, [state.runs[0], worker], [task()]);
    expect(rows.map(row => row.day)).toEqual(["2026-10-07", "2026-10-07", "2026-10-06"]);
    const onLink = vi.fn();
    await render(<SupervisorActivity rows={rows} onLink={onLink} />);
    expect(host.textContent).not.toContain("Outside this root");
    const days = host.querySelectorAll(".supervisor-activity-day");
    expect(days).toHaveLength(2);
    expect(days[0].querySelectorAll("li")).toHaveLength(2);
    expect(days[0].textContent).toContain("stale evidence");
    await click(days[0].querySelectorAll<HTMLButtonElement>("button")[0]);
    await click(days[0].querySelectorAll<HTMLButtonElement>("button")[1]);
    await click(days[1].querySelector<HTMLButtonElement>("button")!);
    expect(onLink.mock.calls.map(([link]) => link)).toEqual([{ kind: "task", taskId: "task-a" }, { kind: "task", taskId: "task-a" }, { kind: "run", runId: "root" }]);
  });
});

describe("Supervisor diagnostics consumer transitions", () => {
  it("copies only the selected canonical path, reports failures honestly, and discards status after selection changes", async () => {
    const writeText = vi.fn(async (_value: string) => undefined);
    vi.stubGlobal("navigator", { clipboard: { writeText } });
    const onCopyPath = vi.fn();
    let state = snapshot();
    const view = () => <SupervisorDiagnostics snapshot={state} rootRuns={state.runs} busy={false} connected onIdentify={() => undefined} onCopyPath={onCopyPath} />;
    await render(view()); await click(button("Copy task path"));
    expect(writeText).toHaveBeenCalledWith("/state/tasks.md");
    expect(host.querySelector('[role="status"]')!.textContent).toContain("copied");
    expect(onCopyPath).toHaveBeenCalledTimes(1);
    state = { ...state, board: { ...state.board!, root_id: "other", path: "/state/other.md" } };
    await render(view());
    expect(host.querySelector('[role="status"]')!.textContent).toBe("");
    writeText.mockRejectedValueOnce(new Error("Clipboard unavailable"));
    await click(button("Copy task path"));
    expect(writeText).toHaveBeenLastCalledWith("/state/other.md");
    expect(host.querySelector('[role="status"]')!.textContent).toContain("Could not copy");
    expect(onCopyPath).toHaveBeenCalledTimes(1);
    state = { ...state, board: null };
    await render(view()); await click(button("Copy task path"));
    expect(button("Copy task path").disabled).toBe(true);
    expect(writeText).toHaveBeenCalledTimes(2);
  });

  it("keeps Identify locked offline or during a mutation, then permits it after the lock clears", async () => {
    const state = snapshot(); state.board!.unidentified_items = 2;
    const onIdentify = vi.fn();
    const view = (connected: boolean, busy: boolean) => <SupervisorDiagnostics snapshot={state} rootRuns={state.runs} busy={busy} connected={connected} onIdentify={onIdentify} onCopyPath={() => undefined} />;
    await render(view(false, false)); await click(button("Identify task-file items"));
    expect(button("Identify task-file items").disabled).toBe(true);
    expect(host.querySelector(`#${button("Identify task-file items").getAttribute("aria-describedby")}`)).not.toBeNull();
    await render(view(true, true)); await click(button("Identify task-file items"));
    expect(onIdentify).not.toHaveBeenCalled();
    await render(view(true, false)); await click(button("Identify task-file items"));
    expect(button("Identify task-file items").disabled).toBe(false);
    expect(onIdentify).toHaveBeenCalledTimes(1);
  });

  it("does not promote dispatch, binding or delivery receipts into proof of a current process", async () => {
    const state = snapshot();
    state.messages = [message({ message_id: "newer", seq: 3, stage: "stored" }), message({ message_id: "older", seq: 2, stage: "acked" })];
    const view = (connected: boolean) => <SupervisorDiagnostics snapshot={state} rootRuns={state.runs} busy={false} connected={connected} onIdentify={() => undefined} onCopyPath={() => undefined} />;
    const cells = () => [...host.querySelectorAll(".supervisor-diagnostics-table-scroll tbody td")].map(cell => cell.textContent);
    await render(view(true));
    expect(cells()).toEqual(["launched", "present · actual OMP yes · working", "Bound session yes", "stored"]);
    if (state.runtime.status === "fresh") state.runtime.runs[0] = { ...state.runtime.runs[0], presence: "missing", actual_omp: false, agent_status: null };
    for (const stage of ["woken", "read", "acked"] as const) {
      state.messages[0] = { ...state.messages[0], stage };
      await render(view(true));
      expect(cells()).toEqual(["launched", "missing · actual OMP no", "Bound session yes", stage]);
    }
    await render(view(false));
    expect(cells()[1]).toBe("Saved · missing · actual OMP no");
    state.runtime = { status: "unavailable", error: { code: "offline", message: "Could not observe runtime" } };
    await render(view(true));
    expect(cells()[1]).toBe("Observation unavailable");
    expect(cells()[3]).toBe("acked");
  });
});

describe("Closed tracking archive transitions", () => {
  it("shows every root count state independently and counts retained open descendants without inventing closure dates", async () => {
    const closed = run({ stage: "closed", updated_at: "2026-10-07T10:00:00Z" });
    const second = run({ run_id: "second", root_id: "second", label: "Second supervisor", stage: "closed" });
    const worker = run({ kind: "worker", run_id: "worker", parent_run_id: "root", stage: "working" });
    const nested = run({ kind: "worker", run_id: "nested", parent_run_id: "worker", stage: "reported" });
    const finished = run({ kind: "worker", run_id: "finished", parent_run_id: "root", stage: "closed" });
    const state = snapshot([closed, second, worker, nested, finished]);
    const taskCounts = new Map<string, ClosedTaskCount>([["root", { status: "loading" }], ["second", { status: "unavailable" }]]);
    const onView = vi.fn(); const onReturn = vi.fn();
    const view = (busy = false, dialogOpen = false) => <ClosedTracking closedRoots={state.roots} runs={state.runs} loadedRootId="root" taskCount={99} taskCounts={taskCounts} busy={busy} dialogOpen={dialogOpen} onView={onView} returnTo={{ label: "Live supervisor", onActivate: onReturn }} />;
    await render(view());
    const items = () => host.querySelectorAll(".supervisor-archive-list > li");
    expect(items()[0].textContent).toContain("Loading task count");
    expect(items()[0].textContent).not.toContain("99 tasks");
    expect(items()[1].textContent).toContain("Task count unavailable");
    expect(items()[0].textContent).toContain("2 descendants still open");
    expect(items()[0].querySelector("time")!.dateTime).toBe(closed.updated_at);
    expect(items()[0].querySelector("time")!.parentElement!.textContent).toMatch(/^Updated /);
    taskCounts.set("root", { status: "loaded", count: 3 }); taskCounts.set("second", { status: "loaded", count: 0 });
    await render(view());
    expect(items()[0].textContent).toContain("3 tasks"); expect(items()[1].textContent).toContain("0 tasks");
    await click(button("View Second supervisor tasks and history")); expect(onView).toHaveBeenCalledWith("second");
    await render(view(true)); await click(button("View Project supervisor tasks and history")); await click(button("Return to Live supervisor"));
    await render(view(false, true)); await click(button("View Project supervisor tasks and history")); await click(button("Return to Live supervisor"));
    expect(onView).toHaveBeenCalledTimes(1); expect(onReturn).not.toHaveBeenCalled();
    await render(view()); await click(button("Return to Live supervisor")); expect(onReturn).toHaveBeenCalledTimes(1);
  });
});
