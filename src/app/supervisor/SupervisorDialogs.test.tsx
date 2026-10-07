// @vitest-environment jsdom
import { act, type ComponentProps } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, describe, expect, it, vi } from "vitest";
import type { OrchestrationActionResult, OrchestrationSnapshot, Run, SpaceSummary, Subagent, TaskView } from "../../protocol/generated/v1";
import { SupervisorDialogs, type SupervisorDialogState } from "./SupervisorDialogs";

const at = "2026-10-07T12:00:00Z";
function run(overrides: Partial<Run> = {}): Run {
  return { session_id: "session", prepare_brief: "Guidance", run_id: "root", kind: "supervisor", label: "Project supervisor", root_id: "root", parent_run_id: null, task_id: null, attempt: 1, task_revision_at_propose: null, stage: "active", close_reason: null, dispatch: { launch_tag: "tag", endpoint_identity: "endpoint", recovery: null, agent_started: true, step: "launch_unknown", launch_attempt: 1, error: null, updated_at: at }, target: { target: "existing_space", workspace_id: "space" }, setup: null, prepare_plan: null, init_receipt: null, work_plan: null, grants: [], last_report: null, result: null, annotations: [], location: null, bound_omp_session: null, bound_omp_process: null, launch_shell_identity: null, retirement: null, supersedes_run_id: null, created_at: at, updated_at: at, ...overrides };
}
function task(): TaskView {
  return { task: { task_id: "task", title: "Current title", body: "Current body", checked: false, line: 1, task_revision: "current-revision", diagnostic: null }, lane: "queued", current_run_id: null };
}
function snapshot(runs: Run[] = [run()], tasks: TaskView[] = [task()]): OrchestrationSnapshot {
  return { session_id: "session", revision: 1, tasks_token: "tasks", roots: [], board: { root_id: "root", path: "/state/tasks.md", doc_revision: "document", unidentified_items: 0, diagnostics: [], tasks }, runs, messages: [], subagents: [], intents: [], assignment_intents: [], attention: [], unmanaged_agents: [], runtime: { status: "fresh", endpoint_identity: "endpoint", observed_at: at, runs: runs.map(item => ({ run_id: item.run_id, presence: "missing", actual_omp: false, workspace_id: null, workspace_label: null, tab_id: null, tab_label: null, pane_id: null, agent_status: null, state_changed_at: at })) } };
}
const space: SpaceSummary = { id: "space", label: "Project", number: 1, tab_count: 1, pane_count: 1, focused: true, agent_status: "idle", git: null };
const subagent: Subagent = { run_id: "root", subagent_id: "sub", parent_subagent_id: null, role: "scout", label: "Research", status: "running", summary: null, last_control: null, updated_at: at };
let root: Root | null = null;
let host: HTMLDivElement;
let opener: HTMLButtonElement;
type Props = ComponentProps<typeof SupervisorDialogs>;
async function mount(dialog: SupervisorDialogState, overrides: Partial<Props> = {}) {
  host = document.createElement("div"); opener = document.createElement("button");
  opener.textContent = "Open dialog"; document.body.append(opener, host); opener.focus(); root = createRoot(host);
  let open = true;
  const render = () => root!.render(open ? <SupervisorDialogs {...props} /> : null);
  const props: Props = { dialog, snapshot: snapshot(), spaces: [space], startDraft: { label: "", location: "existing", spaceId: "space", directory: "" }, changed: render, busy: false, available: true, mutateResult: vi.fn(async () => ({ result: "done" as const })), onStarted: vi.fn(), onEdited: vi.fn(), onStartUnconfirmed: vi.fn(), onCheck: vi.fn(), onClose: vi.fn(() => { open = false; render(); }), ...overrides };
  await act(async () => render());
  return { props, rerender: async (next: Partial<Props>) => { Object.assign(props, next); await act(async () => render()); } };
}
function button(label: string): HTMLButtonElement {
  const found = [...document.querySelectorAll<HTMLButtonElement>("[role=dialog] button")].find(item => item.textContent?.trim() === label);
  if (!found) throw new Error(`Missing button ${label}`);
  return found;
}
function primary(): HTMLButtonElement { return document.querySelector<HTMLButtonElement>("[data-primary]")!; }
async function click(element: HTMLElement) { await act(async () => element.click()); }
async function submit() { await act(async () => document.querySelector("form")!.dispatchEvent(new Event("submit", { bubbles: true, cancelable: true }))); }
function key(key: string, shiftKey = false) { act(() => document.activeElement!.dispatchEvent(new KeyboardEvent("keydown", { key, shiftKey, bubbles: true, cancelable: true }))); }
function enter(field: HTMLInputElement | HTMLSelectElement, value: string) {
  act(() => {
    Object.getOwnPropertyDescriptor(field instanceof HTMLSelectElement ? HTMLSelectElement.prototype : HTMLInputElement.prototype, "value")!.set!.call(field, value);
    field.dispatchEvent(new Event(field instanceof HTMLSelectElement ? "change" : "input", { bubbles: true }));
  });
}
afterEach(async () => { if (root) await act(async () => root!.unmount()); root = null; host?.remove(); opener?.remove(); vi.clearAllMocks(); vi.unstubAllGlobals(); });

describe("Supervisor dialog mutation boundaries", () => {
  it("prevents invalid destinations and clears the inline error after a valid selection", async () => {
    const fixture = await mount({ mode: "start" }, { startDraft: { label: " Agent ", location: "existing", spaceId: "gone", directory: "" }, mutateResult: vi.fn(async () => ({ result: "run" as const, run_id: "new-root", attempt: 1 })) });
    await submit();
    const field = document.querySelectorAll<HTMLSelectElement>("select")[1];
    expect(field.getAttribute("aria-invalid")).toBe("true");
    expect(document.getElementById(field.getAttribute("aria-describedby")!)?.getAttribute("role")).toBe("alert");
    expect(fixture.props.mutateResult).not.toHaveBeenCalled();
    enter(field, "space");
    expect(field.hasAttribute("aria-invalid")).toBe(false);
    await submit();
    expect(fixture.props.mutateResult).toHaveBeenCalledExactlyOnceWith({ action: "supervisor_start", target: { target: "existing_space", workspace_id: "space" }, label: "Agent" });
    expect(fixture.props.onStarted).toHaveBeenCalledWith("new-root");
    expect(document.activeElement).toBe(opener);
  });
  it("never substitutes another Space when none are available", async () => {
    const fixture = await mount({ mode: "start" }, { spaces: [] });
    const field = document.querySelectorAll<HTMLSelectElement>("select")[1];
    expect(field.disabled).toBe(true);
    expect(document.getElementById(field.getAttribute("aria-describedby")!)?.textContent).toBeTruthy();
    await submit();
    expect(fixture.props.mutateResult).not.toHaveBeenCalled();
  });
  it("requires an absolute directory and keeps setup launches unfocused", async () => {
    const fixture = await mount({ mode: "start" }, { startDraft: { label: " Agent ", location: "directory", spaceId: "space", directory: "relative" }, mutateResult: vi.fn(async () => ({ result: "run" as const, run_id: "new-root", attempt: 1 })) });
    await submit();
    const field = document.querySelectorAll<HTMLInputElement>("input")[1];
    expect(field.getAttribute("aria-invalid")).toBe("true");
    expect(fixture.props.mutateResult).not.toHaveBeenCalled();
    enter(field, " /project "); await submit();
    expect(fixture.props.mutateResult).toHaveBeenCalledExactlyOnceWith({ action: "supervisor_start", label: "Agent", target: { target: "setup", request: { operation: "open", path: "/project", label: "Agent", task_name: null, focus: false } } });
  });
  it("keeps dedicated-folder startup as a real supervisor launch", async () => {
    const fixture = await mount({ mode: "start" }, { startDraft: { label: "", location: "dedicated", spaceId: "space", directory: "" }, mutateResult: vi.fn(async () => ({ result: "run" as const, run_id: "new-root", attempt: 1 })) });
    await submit();
    expect(fixture.props.mutateResult).toHaveBeenCalledExactlyOnceWith({ action: "supervisor_start", target: null, label: null });
  });
  it.each(["Keep my draft", "Use current"])("%s rebases the reviewed revision without silently submitting", async choice => {
    const draft = { title: "My title", body: "My body", revision: "old-revision" };
    const fixture = await mount({ mode: "edit", task: task(), draft }, { mutateResult: vi.fn(async () => ({ result: "task" as const, task: task().task })) });
    await click(button(choice));
    expect(fixture.props.mutateResult).not.toHaveBeenCalled();
    const text = choice === "Use current" ? { title: "Current title", body: "Current body" } : { title: "My title", body: "My body" };
    expect(draft).toEqual({ ...text, revision: "current-revision" });
    await submit();
    expect(fixture.props.mutateResult).toHaveBeenCalledExactlyOnceWith({ action: "task_update", root_id: "root", task_id: "task", expected_task_revision: "current-revision", ...text });
    expect(fixture.props.onEdited).toHaveBeenCalledWith("task");
  });
  it("retains an edit across an unknown result and fences a later save against the reviewed revision", async () => {
    const draft = { title: "My title", body: "My body", revision: "current-revision" };
    const fixture = await mount({ mode: "edit", task: task(), draft }, { mutateResult: vi.fn(async () => null) });
    await submit();
    const changed = task(); changed.task.task_revision = "newer-revision";
    await fixture.rerender({ snapshot: snapshot([run()], [changed]) });
    expect(draft).toEqual({ title: "My title", body: "My body", revision: "current-revision" });
    await submit();
    expect(vi.mocked(fixture.props.mutateResult).mock.calls[1][0]).toMatchObject({ expected_task_revision: "current-revision", title: "My title", body: "My body" });
    expect(fixture.props.onEdited).not.toHaveBeenCalled();
    expect(document.querySelector("[role=dialog]")).not.toBeNull();
  });
  it("guards a deleted task even on direct form submission and preserves a copyable draft", async () => {
    const draft = { title: "My title", body: "My body", revision: "current-revision" };
    const fixture = await mount({ mode: "edit", task: task(), draft });
    await fixture.rerender({ snapshot: snapshot([run()], []) });
    expect(primary().disabled).toBe(true);
    await submit();
    expect(fixture.props.mutateResult).not.toHaveBeenCalled();
    const writeText = vi.fn(async () => undefined);
    vi.stubGlobal("navigator", Object.create(navigator, { clipboard: { value: { writeText } } }));
    await click(button("Copy draft"));
    expect(writeText).toHaveBeenCalledExactlyOnceWith("My title\n\nMy body");
    expect(document.querySelector("[role=status]")?.textContent).toContain("copied");
    expect(draft).toEqual({ title: "My title", body: "My body", revision: "current-revision" });
  });
  it("checks the latest run without retrying its launch, then restores the opener", async () => {
    const original = run(); const fixture = await mount({ mode: "retry", run: original });
    const latest = run({ label: "Updated supervisor", dispatch: { ...original.dispatch!, step: "needs_review" } });
    const next = snapshot([latest]);
    if (next.runtime.status === "fresh") next.runtime.runs[0].presence = "endpoint_changed";
    await fixture.rerender({ snapshot: next });
    expect(document.querySelector(".supervisor-dialog-observed")?.textContent).toContain("endpoint changed");
    await click(button("Check again first"));
    expect(fixture.props.onCheck).toHaveBeenCalledExactlyOnceWith(latest);
    expect(fixture.props.mutateResult).not.toHaveBeenCalled();
    expect(document.querySelector("[role=dialog]")).toBeNull();
    expect(document.activeElement).toBe(opener);
  });
  it("closes only the selected tracking after showing true open descendants, not sibling agents", async () => {
    const worker = run({ run_id: "worker", kind: "worker", parent_run_id: "root", label: "Chosen worker" });
    const intermediate = run({ run_id: "closed", parent_run_id: "worker", stage: "closed", label: "Closed intermediate" });
    const children = ["one", "two", "three", "four", "five"].map((id, index) => run({ run_id: id, parent_run_id: index === 0 ? "closed" : "worker", label: `Descendant ${id}` }));
    const sibling = run({ run_id: "sibling", parent_run_id: "root", label: "Unrelated sibling" });
    const fixture = await mount({ mode: "close", run: worker }, { snapshot: snapshot([run(), worker, intermediate, ...children, sibling]) });
    const labels = [...document.querySelectorAll(".supervisor-dialog-descendants li")].map(item => item.textContent);
    expect(labels).toEqual(["Descendant one", "Descendant two", "Descendant three"]);
    expect(document.querySelector("[role=dialog]")?.textContent).toContain("and 2 more");
    expect(document.querySelector("[role=dialog]")?.textContent).not.toContain("Unrelated sibling");
    await submit();
    expect(fixture.props.mutateResult).toHaveBeenCalledExactlyOnceWith({ action: "cancel_run", run_id: "worker" });
  });
  it.each(["accept_existing_worktree", "retry_environment", null] as const)("recovers setup using the explicit %s choice without launching", async recovery => {
    const saved = run(); saved.dispatch = { ...saved.dispatch!, step: "setup_unknown", recovery };
    const fixture = await mount({ mode: "setup_recovery", run: saved });
    await click(primary());
    expect(fixture.props.mutateResult).toHaveBeenCalledExactlyOnceWith({ action: "reconcile_run", run_id: "root", recovery });
  });
  it("keeps a subagent running when cancellation is dismissed", async () => {
    const fixture = await mount({ mode: "subagent_cancel", run: run(), subagent });
    await click(button("Keep running"));
    expect(fixture.props.mutateResult).not.toHaveBeenCalled();
  });
  it("sends cancellation to the selected internal subagent, not its parent run", async () => {
    const fixture = await mount({ mode: "subagent_cancel", run: run(), subagent });
    await click(button("Request cancellation"));
    expect(fixture.props.mutateResult).toHaveBeenCalledExactlyOnceWith({ action: "subagent_control", run_id: "root", subagent_id: "sub", op: { op: "cancel" } });
  });
  it.each(["null", "throw"])("keeps non-edit outcomes nonretryable after %s uncertainty", async outcome => {
    const fixture = await mount({ mode: "retry", run: run() }, { mutateResult: vi.fn(async () => { if (outcome === "throw") throw new Error("Transport lost"); return null; }) });
    await submit();
    expect(primary().disabled).toBe(true);
    await submit();
    expect(fixture.props.mutateResult).toHaveBeenCalledExactlyOnceWith({ action: "retry_launch", run_id: "root" });
    expect(document.querySelector("[role=dialog]")).not.toBeNull();
  });
  it("locks Escape and repeated requests in flight, then restores focus after confirmation", async () => {
    let resolve!: (result: OrchestrationActionResult) => void;
    const promise = new Promise<OrchestrationActionResult>(accept => { resolve = accept; });
    const fixture = await mount({ mode: "retry", run: run() }, { mutateResult: vi.fn(() => promise) });
    await submit(); key("Escape"); await submit();
    expect(fixture.props.onClose).not.toHaveBeenCalled();
    expect(fixture.props.mutateResult).toHaveBeenCalledTimes(1);
    await act(async () => resolve({ result: "done" }));
    expect(document.activeElement).toBe(opener);
  });
  it.each(["close", "subagent_cancel"] as const)("contains focus while every %s dialog control is disabled in flight", async mode => {
    let resolve!: (result: OrchestrationActionResult) => void;
    const promise = new Promise<OrchestrationActionResult>(accept => { resolve = accept; });
    const dialog: SupervisorDialogState = mode === "close" ? { mode, run: run() } : { mode, run: run(), subagent };
    const fixture = await mount(dialog, { mutateResult: vi.fn(() => promise) });
    act(() => primary().focus());
    await click(primary());
    const section = document.querySelector<HTMLElement>("[role=dialog]")!;
    expect(document.activeElement).toBe(section);
    for (const shiftKey of [false, true]) {
      const event = new KeyboardEvent("keydown", { key: "Tab", shiftKey, bubbles: true, cancelable: true });
      act(() => section.dispatchEvent(event));
      expect(event.defaultPrevented).toBe(true);
      expect(document.activeElement).toBe(section);
    }
    key("Escape");
    expect(fixture.props.onClose).not.toHaveBeenCalled();
    expect(document.querySelector("[role=dialog]")).toBe(section);
    await act(async () => resolve({ result: "done" }));
    expect(document.activeElement).toBe(opener);
  });
  it("wraps both ends of the focus trap and does not steal a field's focus on polling", async () => {
    const fixture = await mount({ mode: "edit", task: task(), draft: { title: "Draft", body: "Body", revision: "current-revision" } });
    const first = document.querySelector<HTMLInputElement>("[data-initial]")!;
    const last = primary();
    act(() => last.focus()); key("Tab"); expect(document.activeElement).toBe(first);
    key("Tab", true); expect(document.activeElement).toBe(last);
    act(() => first.focus()); await fixture.rerender({ snapshot: { ...snapshot(), revision: 2 } });
    expect(document.activeElement).toBe(first);
    key("Escape");
    expect(document.querySelector("[role=dialog]")).toBeNull();
    expect(document.activeElement).toBe(opener);
  });
});
