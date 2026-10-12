// @vitest-environment jsdom
import { act, createRef, useState } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, describe, expect, it, vi } from "vitest";
import type { OrchestrationSnapshot, Run, TaskView } from "../../protocol/generated/v1";
import { SupervisorGraph, type SupervisorGraphProps } from "./SupervisorGraph";
import { buildSupervisorGraph, nodeId } from "./topology";
import { TaskCard, type TaskCardView } from "./SupervisorTasks";

function fixture() {
  const root: Run = { source_output: null, integration: { state: "pending" }, session_id: "session", brief: "", run_id: "root", kind: "supervisor", label: "Supervisor", root_id: "root", parent_run_id: null, task_id: null, attempt: 1, assignment_generation: null, launch_config: { harness_kind: "codex", command: "codex", extra_args: [], resume_args: null, inbox_prompt_hints: false, routes: [] }, task_revision_at_start: null, stage: "working", close_reason: null, dispatch: null, target: null, setup: null, cleanup_policy: null, last_report: null, result: null, annotations: [], project: null, checkout: null, attachment: null, checkout_claim_id: null, resume: null, retirement: null, created_at: "2026-10-07T00:00:00Z", updated_at: "2026-10-07T00:00:00Z" };
  const tasks: TaskView[] = [{ task: { task_id: "task:one", title: "Audit", body: "", description: "", description_editable: true, description_diagnostic: null, steps: [], step_progress: { done: 0, total: 0 }, steps_diagnostic: null, depends_on: [], follow_up_of: null, relations_diagnostic: null, checked: false, line: 1, task_revision: "revision", diagnostic: null }, lane: "working", current_run_id: "worker", dependencies: { state: "none", unmet: [], problems: [] } }, { task: { task_id: "task:two", title: "Unassigned", body: "", description: "", description_editable: true, description_diagnostic: null, steps: [], step_progress: { done: 0, total: 0 }, steps_diagnostic: null, depends_on: [], follow_up_of: null, relations_diagnostic: null, checked: false, line: 2, task_revision: "revision", diagnostic: null }, lane: "queued", current_run_id: null, dependencies: { state: "none", unmet: [], problems: [] } }];
  const snapshot: OrchestrationSnapshot = { state_store_id: "store", cleanup: [], restoration: { reattached: 0, total_attachments: 0, cleanup_pending: 0 }, session_id: "session", revision: 1, tasks_token: "token", roots: [], board: { root_id: "root", path: "tasks", doc_revision: "doc", unidentified_items: 0, diagnostics: [], tasks }, runs: [root, { ...root, run_id: "worker", kind: "worker", parent_run_id: "root", label: "Worker", task_id: tasks[0]!.task.task_id, assignment_generation: 1 }, { ...root, run_id: "nested", kind: "worker", parent_run_id: "worker", label: "Nested worker" }], task_ownership: [{ root_id: "root", task_id: tasks[0]!.task.task_id, generation: 1, owner: { run_id: "worker", conversation: { harness: "codex", session_id: "opaque/conversation:worker" }, claim_operation_id: "claim" } }], messages: [], questions: [], intents: [], assignment_intents: [], unmanaged_agents: [], attention: [], runtime: { status: "fresh", endpoint_identity: "endpoint", observed_at: root.updated_at, runs: [] } };
  const model = buildSupervisorGraph({ snapshot, root, rootId: "root", tasks });
  const props: SupervisorGraphProps = { model, snapshot, live: true, connected: true, runtimeLive: true, selectedNodeId: null, highlightedRunId: null, tierFor: () => null, dimFor: () => null, sharedSpace: null, bottomInset: 0, scrollRef: createRef<HTMLDivElement>(), onSelect: () => {}, onHover: () => {}, onEscape: () => false };
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

function cardView(props = fixture()): TaskCardView {
  return { task: props.snapshot.board!.tasks[0]!, worker: props.snapshot.runs[1], snapshot: props.snapshot, observed: undefined, live: true, status: "Working", tier: null, dimReason: null, selected: false, linked: false, showSpace: false, sharedSpace: null };
}
function mountCard(view: TaskCardView) {
  host = document.createElement("div"); document.body.append(host); root = createRoot(host);
  const render = (next: TaskCardView) => act(() => root!.render(<ul><TaskCard view={next} rowId={next.task.task.task_id} tabIndex={0} onFocus={vi.fn()} onSelect={vi.fn()} onHover={vi.fn()} /></ul>));
  render(view);
  return render;
}

describe("Supervisor graph interactions", () => {
  it("roves in preorder and along effective parent/child edges without selecting", () => {
    const props = fixture(); mount(props);
    act(() => row(nodeId.run("root")).focus());
    key("ArrowRight"); expect(document.activeElement).toBe(row(nodeId.task("task:one")));
    key("ArrowRight"); expect(document.activeElement).toBe(row(nodeId.run("worker")));
    key("ArrowRight"); expect(document.activeElement).toBe(row(nodeId.run("nested")));
    key("ArrowLeft"); expect(document.activeElement).toBe(row(nodeId.run("worker")));
    key("ArrowDown"); expect(document.activeElement).toBe(row(nodeId.run("nested")));
    key("ArrowDown"); expect(document.activeElement).toBe(row(nodeId.task("task:two")));
    key("ArrowRight"); expect(document.activeElement).toBe(row(nodeId.task("task:two")));
    key("Home"); expect(document.activeElement).toBe(row(nodeId.run("root")));
    key("ArrowLeft"); expect(document.activeElement).toBe(row(nodeId.run("root")));
    key("End"); expect(document.activeElement).toBe(row(nodeId.task("task:two")));
    key("ArrowUp"); expect(document.activeElement).toBe(row(nodeId.run("nested")));
    expect(host.querySelectorAll('[data-row-id][tabindex="0"]')).toHaveLength(1);
    expect(host.querySelector('[data-row-id][aria-expanded="true"]')).toBeNull();
  });

  it("shows only canonical workers and updates owner edges without stealing selection or focus", () => {
    const props = fixture(), update = mount(props);
    const worker = row(nodeId.run("worker"));
    act(() => { worker.focus(); worker.click(); });
    expect(host.querySelector(".supervisor-graph-heading")!.textContent).not.toMatch(/subagent/i);
    expect(host.querySelector('input[type="checkbox"]')).toBeNull();
    expect([...host.querySelectorAll("[data-node-kind]")].map(node => node.getAttribute("data-node-kind"))).toEqual(["supervisor", "task", "worker", "worker", "task"]);
    expect(row(nodeId.task("task:one")).getAttribute("aria-label")).toContain("Assigned conversation codex · opaque/conversation:worker");
    const snapshot = { ...props.snapshot, task_ownership: props.snapshot.task_ownership.map(item => ({ ...item, generation: 2, owner: null })) };
    const model = buildSupervisorGraph({ snapshot, root: snapshot.runs[0]!, rootId: "root", tasks: snapshot.board!.tasks });
    update({ ...props, snapshot, model });
    expect(model.byId.get(nodeId.run("worker"))!.parentId).toBe(nodeId.run("root"));
    expect(host.querySelector(".supervisor-graph-edge.is-assigned")).toBeNull();
    expect(row(nodeId.task("task:one")).getAttribute("aria-label")).toContain("Unassigned");
    expect(document.activeElement).toBe(worker);
    expect(worker.getAttribute("aria-expanded")).toBe("true");
  });

  it("counts fresh generic agent observations without requiring an OMP identity", () => {
    const props = fixture();
    if (props.snapshot.runtime.status === "fresh") props.snapshot.runtime.runs.push({ run_id: "worker", presence: "present", actual_agent: true, workspace_id: "space", workspace_label: "Project", tab_id: "tab", tab_label: "Worker", pane_id: "pane", agent_status: "working", state_changed_at: null });
    const update = mount(props);
    expect(host.querySelector("h2")!.textContent).toContain("Agents · 1 connected · 2 tasks");
    update({ ...props, live: false });
    expect(host.querySelector("h2")!.textContent).toContain("Agents · unobserved · 2 tasks");
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
    props.snapshot.runs[2] = { ...props.snapshot.runs[2]!, stage: "closed" };
    props.model = buildSupervisorGraph({ snapshot: props.snapshot, root: props.snapshot.runs[0]!, rootId: "root", tasks: [completed] });
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
    act(() => root!.render(<ul><TaskCard view={{ task: completed, worker, snapshot: props.snapshot, observed: undefined, live: false, status: "Done · Source complete", tier: null, dimReason: null, selected: false, linked: false, showSpace: false, sharedSpace: null }} rowId={task.task.task_id} tabIndex={0} onFocus={vi.fn()} onSelect={vi.fn()} onHover={vi.fn()} /></ul>));
    const card = host.querySelector("button")!;
    expect(card.getAttribute("aria-label")).toContain("Outcome: Done, source complete");
    expect(card.getAttribute("aria-label")).toContain("Cleanup: Cleaned up");
    expect(card.textContent).not.toContain("Integrated");
  });
});

describe("Supervisor task ownership cards", () => {
  it("uses canonical assigned identity even without a worker observation or selected worker", () => {
    const view = cardView();
    const update = mountCard({ ...view, worker: undefined });
    const card = host.querySelector("button")!;
    expect(host.querySelector(".supervisor-worker-chip")!.textContent).toBe("Assigned · Worker · codex");
    expect(card.getAttribute("aria-label")).toContain("Assigned · Worker · codex · opaque/conversation:worker");
    expect(card.textContent).not.toMatch(/subagent|Unassigned/);
    expect(host.querySelectorAll("button")).toHaveLength(1);
    act(() => card.focus());
    update({ ...view, worker: undefined, live: false });
    expect(card.textContent).toContain("Last known assignment · reconnect to check current ownership.");
    expect(card.getAttribute("aria-label")).toContain("Assigned · Worker · codex · opaque/conversation:worker");
    expect(document.activeElement).toBe(card);
  });

  it("keeps canonical ownership fresh when physical runtime observation is unavailable or not an agent", () => {
    const view = cardView();
    view.snapshot.runtime = { status: "unavailable", error: { code: "unsupported_harness_observation", message: "No physical observation" } };
    const update = mountCard({ ...view, semanticLive: true, live: false });
    const card = host.querySelector("button")!;
    act(() => card.focus());
    expect(host.querySelector(".supervisor-worker-chip")!.textContent).toBe("Assigned · Worker · codex");
    expect(card.textContent).not.toContain("Last known assignment");
    expect(card.getAttribute("aria-label")).not.toContain("Last known assignment");
    const observed = { run_id: "worker", presence: "present" as const, actual_agent: false, workspace_id: "space", workspace_label: "Project", tab_id: "tab", tab_label: "Worker", pane_id: "pane", agent_status: null, state_changed_at: null };
    view.snapshot.runtime = { status: "fresh", endpoint_identity: "endpoint", observed_at: view.worker!.updated_at, runs: [observed] };
    update({ ...view, observed, semanticLive: true, live: false });
    expect(card.getAttribute("aria-label")).toContain("Assigned · Worker · codex · opaque/conversation:worker");
    expect(card.textContent).not.toContain("Last known assignment");
    expect(document.activeElement).toBe(card);
    update({ ...view, observed, semanticLive: false, live: true });
    expect(card.textContent).toContain("Last known assignment · reconnect to check current ownership.");
  });

  it("keeps pending registration unassigned and warns about the previous worker independently", () => {
    const view = cardView(), snapshot = view.snapshot;
    snapshot.task_ownership[0] = { ...snapshot.task_ownership[0]!, generation: 2, owner: null };
    snapshot.runs[1] = { ...snapshot.runs[1]!, checkout_claim_id: "protected-checkout", stage: "closed" };
    const pending: Run = { ...snapshot.runs[1]!, run_id: "pending", label: "Pending worker", assignment_generation: 3, stage: "starting", checkout_claim_id: null };
    snapshot.runs.push(pending);
    const update = mountCard({ ...view, worker: pending });
    const card = host.querySelector("button")!;
    act(() => card.focus());
    expect(host.querySelector(".supervisor-worker-chip")!.textContent).toBe("Unassigned · ownership pending");
    expect(card.getAttribute("aria-label")).toContain("Unassigned · ownership pending");
    expect(card.textContent).toContain("Previous worker stop not proven");
    expect(card.textContent).not.toContain("Assigned · Pending worker");
    snapshot.runs[snapshot.runs.length - 1] = { ...pending, stage: "closed" };
    snapshot.task_ownership[0] = { ...snapshot.task_ownership[0]!, generation: 4 };
    update({ ...view, worker: undefined });
    expect(host.querySelector(".supervisor-worker-chip")!.textContent).toBe("Unassigned");
    expect(card.textContent).toContain("Previous worker stop not proven");
    expect(document.activeElement).toBe(card);
  });

  it("retains previous reports and stop uncertainty when a replacement owns the task", () => {
    const view = cardView(), snapshot = view.snapshot;
    snapshot.task_ownership[0] = { ...snapshot.task_ownership[0]!, generation: 2, owner: null };
    const previous: Run = { ...snapshot.runs[1]!, stage: "closed", checkout_claim_id: "protected-checkout", last_report: { message_id: "previous-report", kind: "result", outcome: "succeeded", summary: "Previous work", plan: null, source_output: null, at: "2026-10-07T00:00:00Z" } };
    snapshot.runs[1] = previous;
    const update = mountCard({ ...view, worker: previous });
    const card = host.querySelector("button")!;
    expect(card.textContent).toContain("Unassigned");
    expect(card.textContent).toContain("Previous assignment · Worker · codex");
    expect(card.querySelector("time")?.dateTime).toBe(previous.last_report!.at);
    expect(card.getAttribute("aria-label")).not.toContain("awaiting acceptance");
    const replacement: Run = { ...previous, run_id: "replacement", label: "New worker", stage: "working", assignment_generation: 3, checkout_claim_id: null, last_report: null };
    snapshot.runs.push(replacement);
    snapshot.task_ownership[0] = { ...snapshot.task_ownership[0]!, generation: 3, owner: { run_id: replacement.run_id, conversation: { harness: "claude", session_id: "replacement/session" }, claim_operation_id: "replacement-claim" } };
    update({ ...view, worker: replacement });
    expect(host.querySelector(".supervisor-worker-chip")!.textContent).toBe("Assigned · New worker · claude");
    expect(card.getAttribute("aria-label")).toContain("replacement/session");
    expect(card.textContent).toContain("Previous worker stop not proven");
    snapshot.cleanup = [{ run_id: previous.run_id, items: [{ resource_id: "terminal", kind: "worker_terminal", ownership: "owned", cleanup: { state: "removed", operation_id: null, reason: null, effects: [], updated_at: previous.updated_at } }], removed_count: 1, kept_count: 0, waiting_count: 0, unknown_count: 0 }];
    update({ ...view, worker: replacement });
    expect(card.textContent).not.toContain("Previous worker stop not proven");
    expect(host.querySelector(".supervisor-worker-chip")!.textContent).toBe("Assigned · New worker · claude");
  });

  it("falls back to the canonical harness and full accessible session identity without a worker label", () => {
    const view = cardView();
    view.snapshot.runs[1] = { ...view.snapshot.runs[1]!, label: "" };
    mountCard(view);
    expect(host.querySelector(".supervisor-worker-chip")!.textContent).toBe("Assigned · codex · opaque/conversation:worker");
    expect(host.querySelector("button")!.getAttribute("aria-label")).toContain("Assigned · codex · opaque/conversation:worker");
  });
});
