// @vitest-environment jsdom
import { act, createRef, useState } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, describe, expect, it, vi } from "vitest";
import type { OrchestrationSnapshot, Run, TaskView } from "../../protocol/generated/v1";
import { SupervisorGraph, type SupervisorGraphProps } from "./SupervisorGraph";
import { buildSupervisorGraph, nodeId } from "./topology";

function fixture() {
  const root: Run = { session_id: "session", prepare_brief: "", run_id: "root", kind: "supervisor", label: "Supervisor", root_id: "root", parent_run_id: null, task_id: null, attempt: 1, task_revision_at_propose: null, stage: "working", close_reason: null, dispatch: null, target: null, setup: null, prepare_plan: null, init_receipt: null, work_plan: null, grants: [], last_report: null, result: null, annotations: [], location: null, bound_omp_session: null, bound_omp_process: null, launch_shell_identity: null, retirement: null, supersedes_run_id: null, created_at: "2026-10-07T00:00:00Z", updated_at: "2026-10-07T00:00:00Z" };
  const tasks: TaskView[] = [{ task: { task_id: "task:one", title: "Audit", body: "", checked: false, line: 1, task_revision: "revision", diagnostic: null }, lane: "working", current_run_id: "worker" }, { task: { task_id: "task:two", title: "Unassigned", body: "", checked: false, line: 2, task_revision: "revision", diagnostic: null }, lane: "queued", current_run_id: null }];
  const snapshot: OrchestrationSnapshot = { session_id: "session", revision: 1, tasks_token: "token", roots: [], board: null, runs: [root, { ...root, run_id: "worker", kind: "worker", parent_run_id: "root", label: "Worker" }], messages: [], subagents: [{ run_id: "worker", subagent_id: "child", parent_subagent_id: null, label: "Child", role: "scout", status: "running", summary: null, last_control: null, updated_at: root.updated_at }], intents: [], assignment_intents: [], unmanaged_agents: [], attention: [], runtime: { status: "fresh", endpoint_identity: "endpoint", observed_at: root.updated_at, runs: [] } };
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
});
