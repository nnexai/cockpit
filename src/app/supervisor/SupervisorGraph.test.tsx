// @vitest-environment jsdom
import { act, createRef, useState } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, describe, expect, it, vi } from "vitest";
import type { OrchestrationSnapshot, Run, TaskView } from "../../protocol/generated/v1";
import { SupervisorGraph, type SupervisorGraphProps } from "./SupervisorGraph";
import { buildSupervisorGraph, nodeId } from "./topology";
import { TaskCard } from "./SupervisorTasks";

function fixture() {
  const root: Run = { source_output: null, integration: { state: "pending" }, session_id: "session", brief: "", run_id: "root", kind: "supervisor", label: "Supervisor", root_id: "root", parent_run_id: null, task_id: null, attempt: 1, task_revision_at_start: null, stage: "working", close_reason: null, dispatch: null, target: null, setup: null, cleanup_policy: null, last_report: null, result: null, annotations: [], project: null, checkout: null, attachment: null, checkout_claim_id: null, resume: null, retirement: null, created_at: "2026-10-07T00:00:00Z", updated_at: "2026-10-07T00:00:00Z" };
  const tasks: TaskView[] = [{ task: { task_id: "task:one", title: "Audit", body: "", description: "", description_editable: true, description_diagnostic: null, steps: [], step_progress: { done: 0, total: 0 }, steps_diagnostic: null, depends_on: [], follow_up_of: null, relations_diagnostic: null, checked: false, line: 1, task_revision: "revision", diagnostic: null }, lane: "working", current_run_id: "worker", dependencies: { state: "none", unmet: [], problems: [] } }, { task: { task_id: "task:two", title: "Unassigned", body: "", description: "", description_editable: true, description_diagnostic: null, steps: [], step_progress: { done: 0, total: 0 }, steps_diagnostic: null, depends_on: [], follow_up_of: null, relations_diagnostic: null, checked: false, line: 2, task_revision: "revision", diagnostic: null }, lane: "queued", current_run_id: null, dependencies: { state: "none", unmet: [], problems: [] } }];
  const snapshot: OrchestrationSnapshot = { state_store_id: "store", cleanup: [], restoration: { reattached: 0, total_attachments: 0, cleanup_pending: 0 }, session_id: "session", revision: 1, tasks_token: "token", roots: [], board: null, runs: [root, { ...root, run_id: "worker", kind: "worker", parent_run_id: "root", label: "Worker" }], messages: [], questions: [], subagents: [{ run_id: "worker", subagent_id: "child", parent_subagent_id: null, label: "Child", role: "scout", status: "running", summary: null, last_control: null, bound_omp_session: null, updated_at: root.updated_at }], intents: [], assignment_intents: [], unmanaged_agents: [], attention: [], runtime: { status: "fresh", endpoint_identity: "endpoint", observed_at: root.updated_at, runs: [] } };
  const model = buildSupervisorGraph({ snapshot, root, rootId: "root", tasks, includeSubagents: true });
  const props: SupervisorGraphProps = { model, snapshot, live: true, connected: true, runtimeLive: true, selectedNodeId: null, highlightedRunId: null, tierFor: () => null, dimFor: () => null, showSubagents: true, onShowSubagents: () => {}, sharedSpace: null, bottomInset: 0, scrollRef: createRef<HTMLDivElement>(), onSelect: () => {}, onHover: () => {}, onEscape: () => false };
  return props;
}
let host: HTMLDivElement;
let root: Root | null = null;
afterEach(() => { if (root) act(() => root!.unmount()); root = null; host?.remove(); vi.restoreAllMocks(); });
function mount(props: SupervisorGraphProps) {
  host = document.createElement("div"); document.body.append(host); root = createRoot(host);
  function ControlledGraph({ input }: { input: SupervisorGraphProps }) {
    const [selected, select] = useState(input.selectedNodeId);
    return <SupervisorGraph {...input} selectedNodeId={selected} onSelect={node => select(current => current === node.id ? null : node.id)} onEscape={() => { if (!selected) return false; select(null); return true; }} />;
  }
  act(() => root!.render(<ControlledGraph input={props} />));
  return (next: SupervisorGraphProps) => act(() => root!.render(<ControlledGraph input={next} />));
}
function row(id: string): HTMLButtonElement {
  const element = [...host.querySelectorAll<HTMLButtonElement>("[data-row-id]")].find(element => element.dataset.rowId === id);
  if (!element) throw new Error(`Missing graph node ${id}`);
  return element;
}
function key(value: string) { act(() => document.activeElement!.dispatchEvent(new KeyboardEvent("keydown", { key: value, bubbles: true, cancelable: true }))); }

describe("Supervisor graph interactions", () => {
  it("roves in preorder and along effective parent/child edges without selecting", () => {
    const props = fixture(); mount(props);
    act(() => row(nodeId.run("root")).focus());
    key("ArrowRight"); expect(document.activeElement).toBe(row(nodeId.task("task:one")));
    key("ArrowRight"); expect(document.activeElement).toBe(row(nodeId.run("worker")));
    key("ArrowRight"); expect(document.activeElement).toBe(row(nodeId.subagent("worker", "child")));
    key("ArrowLeft"); expect(document.activeElement).toBe(row(nodeId.run("worker")));
    key("ArrowDown"); expect(document.activeElement).toBe(row(nodeId.subagent("worker", "child")));
    key("ArrowDown"); expect(document.activeElement).toBe(row(nodeId.task("task:two")));
    key("ArrowRight"); expect(document.activeElement).toBe(row(nodeId.task("task:two")));
    key("Home"); expect(document.activeElement).toBe(row(nodeId.run("root")));
    key("ArrowLeft"); expect(document.activeElement).toBe(row(nodeId.run("root")));
    key("End"); expect(document.activeElement).toBe(row(nodeId.task("task:two")));
    key("ArrowUp"); expect(document.activeElement).toBe(row(nodeId.subagent("worker", "child")));
    expect(host.querySelectorAll('[data-row-id][tabindex="0"]')).toHaveLength(1);
    expect(host.querySelector('[data-row-id][aria-expanded="true"]')).toBeNull();
  });

  it("keeps filtered nodes reachable and focus stable as attention changes; selection and Escape affect details only", () => {
    const props = fixture(); const update = mount(props);
    const worker = row(nodeId.run("worker"));
    act(() => { worker.focus(); worker.click(); });
    expect(worker.getAttribute("aria-expanded")).toBe("true");
    const scroll = props.scrollRef.current!; scroll.scrollTop = 83; scroll.scrollLeft = 17;
    update({ ...props, tierFor: node => node.kind === "task" ? "recover" : null, dimFor: node => node.kind !== "task" ? "attention filter" : null });
    expect(document.activeElement).toBe(worker);
    expect(worker.getAttribute("aria-expanded")).toBe("true");
    expect(worker.classList.contains("is-dimmed")).toBe(true);
    expect(host.querySelectorAll("[data-row-id]")).toHaveLength(props.model.nodes.length);
    expect(scroll.scrollTop).toBe(83); expect(scroll.scrollLeft).toBe(17);
    key("Escape");
    expect(worker.getAttribute("aria-expanded")).toBe("false");
    expect(document.activeElement).toBe(worker);
    act(() => worker.click()); expect(worker.getAttribute("aria-expanded")).toBe("true");
    act(() => worker.click()); expect(worker.getAttribute("aria-expanded")).toBe("false");
  });

  it("reveals keyboard targets in its own scrollport while a visible click preserves offsets", () => {
    const props = { ...fixture(), bottomInset: 100 }; mount(props);
    const scroll = props.scrollRef.current!;
    Object.defineProperties(scroll, { clientWidth: { value: 300 }, clientHeight: { value: 300 } });
    vi.spyOn(scroll, "getBoundingClientRect").mockReturnValue({ left: 0, top: 0, width: 300, height: 300 } as DOMRect);
    const target = row(nodeId.task("task:one"));
    vi.spyOn(target, "getBoundingClientRect").mockReturnValue({ left: 280, top: 240, width: 240, height: 48 } as DOMRect);
    act(() => row(nodeId.run("root")).focus());
    key("ArrowRight");
    expect(scroll.scrollLeft).toBe(220); expect(scroll.scrollTop).toBe(88);
    expect(document.activeElement).toBe(target);
    act(() => target.click());
    expect(scroll.scrollLeft).toBe(220); expect(scroll.scrollTop).toBe(88);
    expect(target.getAttribute("aria-expanded")).toBe("true");
    expect(host.scrollTop).toBe(0); expect(host.scrollLeft).toBe(0);
  });

  it("adds a kept-task count without drawing completed workers or changing focus/scroll on cleanup updates", () => {
    const props = fixture();
    const task = props.model.nodes.find(node => node.task)?.task!;
    const completed = { ...task, lane: "accepted" as const, current_run_id: null, task: { ...task.task, checked: true } };
    props.snapshot.board = { root_id: "root", path: "tasks", doc_revision: "doc", unidentified_items: 0, diagnostics: [], tasks: [completed] };
    props.snapshot.runs[1] = { ...props.snapshot.runs[1]!, stage: "closed", close_reason: "accepted", task_id: completed.task.task_id };
    props.model = buildSupervisorGraph({ snapshot: props.snapshot, root: props.snapshot.runs[0]!, rootId: "root", tasks: [completed], includeSubagents: true });
    const update = mount(props);
    const rootRow = row(nodeId.run("root")); act(() => rootRow.focus());
    const scroll = props.scrollRef.current!; scroll.scrollTop = 81;
    const updated = { ...props.snapshot, cleanup: [{ run_id: "worker", kept_count: 2, removed_count: 0, waiting_count: 0, unknown_count: 0, items: (["worktree", "branch"] as const).map(kind => ({ resource_id: kind, kind, ownership: "owned" as const, cleanup: { state: "kept" as const, operation_id: null, reason: { class: "user_work" as const, code: "dirty", message: "uncommitted changes" }, effects: [], updated_at: "2026-10-07T00:00:00Z" } })) }] };
    update({ ...props, snapshot: updated });
    expect(host.querySelector(".supervisor-graph-hidden-completed")?.textContent).toBe("1 completed tasks hidden · 1 kept");
    expect(host.querySelector('[data-node-kind="worker"]')).toBeNull();
    expect(document.activeElement).toBe(rootRow); expect(scroll.scrollTop).toBe(81);
  });

  it("names Outcome and Cleanup independently on a completed card even with a removed resource", () => {
    const props = fixture(), task = props.model.nodes.find(node => node.task)?.task!;
    const worker = { ...props.snapshot.runs[1]!, stage: "closed" as const, close_reason: "accepted" as const, result: { message_id: "result", kind: "result" as const, outcome: "succeeded" as const, summary: "Finished", plan: null, source_output: null, at: "2026-10-07T00:00:00Z" } };
    const completed = { ...task, lane: "accepted" as const, task: { ...task.task, checked: true } };
    props.snapshot.cleanup = [{ run_id: worker.run_id, items: [{ resource_id: "terminal", kind: "worker_terminal", ownership: "owned", cleanup: { state: "removed", operation_id: null, reason: null, effects: [], updated_at: worker.updated_at } }], removed_count: 1, kept_count: 0, waiting_count: 0, unknown_count: 0 }];
    host = document.createElement("div"); document.body.append(host); root = createRoot(host);
    act(() => root!.render(<ul><TaskCard view={{ task: completed, worker, snapshot: props.snapshot, observed: undefined, live: false, status: "Done · Source complete", tier: null, dimReason: null, selected: false, linked: false, subagentCount: 0, showSpace: false, sharedSpace: null }} rowId={task.task.task_id} tabIndex={0} onFocus={vi.fn()} onSelect={vi.fn()} onHover={vi.fn()} /></ul>));
    const card = host.querySelector("button")!;
    expect(card.getAttribute("aria-label")).toContain("Outcome: Done, source complete");
    expect(card.getAttribute("aria-label")).toContain("Cleanup: Cleaned up");
    expect(card.textContent).not.toContain("Integrated");
  });
});
