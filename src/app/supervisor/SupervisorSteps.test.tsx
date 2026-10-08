// @vitest-environment jsdom
import { act, useReducer } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, describe, expect, it, vi } from "vitest";
import type { Task } from "../../protocol/generated/v1";
import { StepsSection } from "./SupervisorSteps";
import { leafProgress, newStepDraft, type StepReadOutcome, type StepSubmission, type SupervisorStepsProps } from "./stepInteractions";
import type { TaskMutationOutcome } from "./useSupervisor";

function step(stepId: string | null, depth = 0, parent: string | null = null, status: Task["steps"][number]["status"] = "open", sourceOffset = 0): Task["steps"][number] {
  return { step_id: stepId, parent_step_id: parent, depth, title: stepId ?? `Legacy ${sourceOffset}`, checked: status === "done",
    status, line: sourceOffset + 1, source_offset: sourceOffset, diagnostic: null };
}
function task(steps: Task["steps"] = [], overrides: Partial<Task> = {}): Task {
  return { task_id: "task", title: "Task", body: "Original source with identity markers", description: "Prose", description_editable: true,
    description_diagnostic: null, steps, step_progress: leafProgress(steps), steps_diagnostic: null,
    depends_on: [], follow_up_of: null, relations_diagnostic: null, checked: false, line: 1,
    task_revision: "original", diagnostic: null, ...overrides };
}
let host: HTMLDivElement;
let root: Root | null = null;
let state: SupervisorStepsProps;
async function settle() { await act(async () => { for (let index = 0; index < 8; index += 1) await Promise.resolve(); }); }
async function mount(saved: Task | null, overrides: Partial<SupervisorStepsProps> = {}) {
  state = { scope: { sessionId: "session", rootId: "root", taskId: "task" }, task: saved, draft: newStepDraft(),
    writable: true, readOnlyReason: null, busy: false, taskWriteUnconfirmed: false, onDraftChanged: vi.fn(),
    submit: vi.fn<SupervisorStepsProps["submit"]>(async () => ({ kind: "not_sent", reason: "Test writer is busy." })),
    readSaved: vi.fn<SupervisorStepsProps["readSaved"]>(async () => ({ kind: "missing" })),
    resolveUnknown: vi.fn<SupervisorStepsProps["resolveUnknown"]>(async (request) => ({ kind: "not_resolved", originalScope: request.originalScope,
      originalSubmissionId: request.originalSubmissionId, code: "read_required", message: "Read the original task first." })),
    onCloseDetails: vi.fn(), ...overrides };
  function Harness() {
    const [, refresh] = useReducer((value: number) => value + 1, 0);
    return <StepsSection {...state} onDraftChanged={() => { state.onDraftChanged(); refresh(); }} />;
  }
  host = document.createElement("div"); document.body.append(host); root = createRoot(host);
  await act(async () => { root!.render(<Harness />); });
  await settle();
  return { push: async (changes: Partial<SupervisorStepsProps>) => { Object.assign(state, changes); await act(async () => root!.render(<Harness />)); await settle(); } };
}
function button(text: string): HTMLButtonElement {
  const result = [...document.querySelectorAll<HTMLButtonElement>("button")].find((element) => element.textContent?.trim() === text || element.getAttribute("aria-label") === text);
  if (!result) throw new Error(`Missing button ${text}`);
  return result;
}
function checkbox(name: string): HTMLInputElement {
  const result = [...host.querySelectorAll<HTMLInputElement>('input[type="checkbox"]')].find((element) => element.getAttribute("aria-label") === name);
  if (!result) throw new Error(`Missing checkbox ${name}`);
  return result;
}
function press(target: HTMLElement, key: string, options: KeyboardEventInit = {}) {
  const event = new KeyboardEvent("keydown", { key, bubbles: true, cancelable: true, ...options });
  act(() => { target.dispatchEvent(event); }); return event;
}
function enter(field: HTMLInputElement, value: string) {
  act(() => { Object.getOwnPropertyDescriptor(HTMLInputElement.prototype, "value")!.set!.call(field, value);
    field.dispatchEvent(new Event("input", { bubbles: true })); });
}
async function click(target: HTMLElement) { await act(async () => target.click()); await settle(); }
async function submitForm(field: HTMLInputElement) { await act(async () => { field.form!.dispatchEvent(new Event("submit", { bubbles: true, cancelable: true })); }); await settle(); }
function titleField(): HTMLInputElement {
  const field = host.querySelector<HTMLInputElement>('.supervisor-step-composer input');
  if (!field) throw new Error("Missing title field"); return field;
}
afterEach(async () => { if (root) await act(async () => root!.unmount()); root = null; host?.remove(); vi.restoreAllMocks(); });

describe("saved hierarchy, native state and keyboard ownership", () => {
  it("renders derived mixed state even when the saved branch byte is checked", async () => {
    const parent = step("parent", 0, null, "partial"); parent.checked = true;
    await mount(task([parent, step("done", 1, "parent", "done"), step("open", 1, "parent")], { step_progress: { done: 1, total: 2 } }));
    expect(checkbox("parent").checked).toBe(false);
    expect(checkbox("parent").indeterminate).toBe(true);
    expect(host.querySelectorAll('input[data-row-id][tabindex="0"]')).toHaveLength(1);
    await click(checkbox("parent"));
    expect(state.submit).toHaveBeenCalledWith(expect.objectContaining({ expectedTaskRevision: "original",
      intent: { kind: "set_checked", stepId: "parent", checked: true, scope: "subtree" } }));
    expect(checkbox("parent").checked).toBe(false);
    expect(checkbox("parent").indeterminate).toBe(true);
    expect(state.task?.checked).toBe(false);
  });
  it("uses a separate roving scope, menu Enter and local Escape without submitting navigation", async () => {
    await mount(task([step("parent"), step("child", 1, "parent"), step("last")]));
    act(() => checkbox("parent").focus());
    const propagation = vi.fn(); window.addEventListener("keydown", propagation);
    expect(press(checkbox("parent"), "ArrowDown").defaultPrevented).toBe(true);
    expect(document.activeElement).toBe(checkbox("child"));
    press(checkbox("child"), "ArrowLeft"); expect(document.activeElement).toBe(checkbox("parent"));
    press(checkbox("parent"), "ArrowLeft"); expect(host.querySelector('[aria-label="Collapse parent"]')).toBeNull();
    press(checkbox("parent"), "ArrowRight"); press(checkbox("parent"), "ArrowRight");
    expect(document.activeElement).toBe(checkbox("child"));
    press(checkbox("child"), "End"); expect(document.activeElement).toBe(checkbox("last"));
    press(checkbox("last"), "Enter"); await settle();
    expect(document.querySelector('[role="menu"]')).not.toBeNull();
    press(button("Edit title…"), "Escape"); await settle();
    expect(document.activeElement).toBe(checkbox("last"));
    press(checkbox("last"), "Escape");
    expect(state.onCloseDetails).toHaveBeenCalledOnce();
    expect(state.submit).not.toHaveBeenCalled(); expect(propagation).not.toHaveBeenCalled();
    window.removeEventListener("keydown", propagation);
  });
  it("guards repeated/composing Space and retains busy checkbox focus without an accepted second write", async () => {
    const ui = await mount(task([step("a"), step("b")]));
    act(() => checkbox("a").focus());
    press(checkbox("a"), " ", { repeat: true }); press(checkbox("a"), " ", { isComposing: true });
    expect(state.submit).not.toHaveBeenCalled();
    await ui.push({ busy: true });
    await click(checkbox("a"));
    expect(checkbox("a").getAttribute("aria-disabled")).toBe("true");
    expect(checkbox("a").disabled).toBe(false); expect(document.activeElement).toBe(checkbox("a"));
    expect(state.submit).not.toHaveBeenCalled(); expect(host.textContent).toContain("Wait before changing another step");
  });
  it("does not focus or reset user expansion on polling, but safely relocates an externally removed focused row", async () => {
    const ui = await mount(task([step("parent"), step("child", 1, "parent"), step("next")]));
    await click(button("Collapse parent"));
    act(() => checkbox("next").focus());
    const serial = state.draft.focusRequest?.serial;
    await ui.push({ task: task([step("parent", 0, null, "done"), step("child", 1, "parent", "done"), step("next")], { task_revision: "tick" }) });
    expect(document.activeElement).toBe(checkbox("next"));
    expect(state.draft.expanded?.has("parent")).toBe(false); expect(state.draft.focusRequest?.serial).toBe(serial);
    await ui.push({ task: task([step("parent")], { task_revision: "removed" }) });
    expect(document.activeElement).toBe(checkbox("parent")); expect(host.textContent).toContain("focused step was removed");
  });
});

describe("snapshot-bound confirmations and text drafts", () => {
  it("focuses the safe reset choice, restores native state on cancel, and requires a second confirmation after a race", async () => {
    const doneTask = task([step("parent", 0, null, "done"), step("one", 1, "parent", "done"), step("two", 1, "parent", "done")], { step_progress: { done: 2, total: 2 } });
    const ui = await mount(doneTask);
    await click(checkbox("parent"));
    expect(document.activeElement).toBe(button("Keep completed")); expect(state.draft.confirmation?.baseTask).toBe(doneTask);
    expect(state.submit).not.toHaveBeenCalled();
    await click(button("Keep completed")); expect(checkbox("parent").checked).toBe(true);
    expect(checkbox("parent").indeterminate).toBe(false);
    await click(checkbox("parent"));
    await ui.push({ task: { ...doneTask, task_revision: "raced" } });
    expect(host.textContent).toContain("Task changed since this preview");
    await click(button("Review updated preview")); expect(state.submit).not.toHaveBeenCalled();
    expect(state.draft.confirmation?.submitted.expectedTaskRevision).toBe("raced");
    await click(button("Mark 2 steps open"));
    expect(state.submit).toHaveBeenCalledWith(expect.objectContaining({ expectedTaskRevision: "raced", intent: { kind: "set_checked", stepId: "parent", checked: false, scope: "subtree" } }));
  });
  it("removes the complete subtree only after review and shows last-child parent conversion", async () => {
    const ui = await mount(task([step("parent", 0, null, "partial"), step("child", 1, "parent", "partial"),
      step("grandchild", 2, "child"), step("complete-grandchild", 2, "child", "done")]));
    await click(button("Actions for child")); await click(button("Remove…"));
    expect(document.activeElement).toBe(button("Keep steps")); expect(host.textContent).toContain("its 2 sub-steps");
    expect(host.textContent).toContain("“parent” becomes a leaf, open");
    expect(state.submit).not.toHaveBeenCalled();
    state.submit = vi.fn<SupervisorStepsProps["submit"]>(async () => {
      const saved = task([step("parent")], { task_revision: "removed" }); state.task = saved;
      return { kind: "confirmed", task: saved };
    });
    await ui.push({ submit: state.submit });
    await click(button("Remove 3 steps"));
    expect(state.submit).toHaveBeenCalledWith(expect.objectContaining({ intent: { kind: "remove", stepId: "child" } }));
    expect(document.activeElement).toBe(checkbox("parent"));
    expect(state.task?.checked).toBe(false); await ui.push({ task: state.task });
  });
  it("keeps escaped add/rename drafts, captures original revisions, and applies a new explicitly reviewed fence without new durable IDs", async () => {
    const ui = await mount(task([step("existing")]));
    await click(button("Add step")); const field = titleField(); enter(field, "New work");
    press(field, "Escape"); expect(state.draft.add.get(null)?.text).toBe("New work");
    expect(state.submit).not.toHaveBeenCalled();
    await ui.push({ task: task([step("existing")], { task_revision: "worker-change" }) });
    await click(button("Resume add draft")); await submitForm(titleField());
    expect(state.submit).not.toHaveBeenCalled(); expect(state.draft.operation.kind).toBe("review");
    const original = state.draft.add.get(null)!.retainedSubmission!;
    expect(original.expectedTaskRevision).toBe("original");
    await click(button("Apply reviewed change"));
    const sent = vi.mocked(state.submit).mock.calls[0][0];
    expect(sent.expectedTaskRevision).toBe("worker-change"); expect(sent.intent).toEqual(original.intent);
    expect(sent.submissionId).not.toBe(original.submissionId);
    expect(state.draft.add.get(null)?.text).toBe("New work");
  });
  it("retains a removed rename target for copy/discard without resurrecting or retargeting it", async () => {
    const ui = await mount(task([step("removed")]));
    await click(button("Actions for removed")); await click(button("Edit title…")); enter(titleField(), "Kept draft");
    await ui.push({ task: task([], { task_revision: "gone" }) });
    expect(host.textContent).toContain("removed elsewhere"); expect(titleField().value).toBe("Kept draft");
    expect([...host.querySelectorAll("button")].some((element) => element.textContent === "Save title")).toBe(false);
    await ui.push({ task: null });
    expect(titleField().value).toBe("Kept draft"); await click(button("Discard draft"));
    expect(state.draft.rename.size).toBe(0); expect(state.submit).not.toHaveBeenCalled();
  });
  it("captures the row menu's original revision even when a new snapshot arrives before selection", async () => {
    const ui = await mount(task([step("first"), step("second")]));
    await click(button("Actions for second"));
    await ui.push({ task: task([step("first"), step("second")], { task_revision: "worker-tick" }) });
    await click(button("Move up"));
    expect(state.submit).toHaveBeenCalledWith(expect.objectContaining({ expectedTaskRevision: "original",
      intent: { kind: "move", stepId: "second", parentStepId: null, beforeStepId: "first" } }));
  });
  it("previews both reparent conversions before outdenting the last child", async () => {
    await mount(task([step("parent", 0, null, "done"), step("child", 1, "parent", "done"), step("next")]));
    await click(button("Actions for child")); await click(button("Outdent"));
    expect(state.submit).not.toHaveBeenCalled();
    expect(host.textContent).toContain("“parent” becomes a leaf, complete");
    expect(document.activeElement).toBe(button("Keep steps"));
    await click(button("Apply change"));
    expect(state.submit).toHaveBeenCalledWith(expect.objectContaining({
      intent: { kind: "move", stepId: "child", parentStepId: null, beforeStepId: "next" } }));
  });
  it.each([false, true])("reveals the saved move destination and retains row focus without reopening other collapsed branches (existing=%s)", async (existingBranch) => {
    const before = [step("parent"), ...(existingBranch ? [step("existing", 1, "parent")] : []),
      step("moved"), step("other"), step("hidden", 1, "other")];
    const ui = await mount(task(before));
    if (existingBranch) await click(button("Collapse parent"));
    await click(button("Collapse other"));
    act(() => checkbox("moved").focus());
    state.submit = vi.fn<SupervisorStepsProps["submit"]>(async () => {
      const saved = task([step("parent"), ...(existingBranch ? [step("existing", 1, "parent")] : []),
        step("moved", 1, "parent"), step("other"), step("hidden", 1, "other")], { task_revision: "moved" });
      state.task = saved;
      return { kind: "confirmed", task: saved };
    });
    await ui.push({ submit: state.submit });
    await click(button("Actions for moved")); await click(button("Indent under previous step"));
    if (!existingBranch) await click(button("Apply change"));
    expect(document.activeElement).toBe(checkbox("moved"));
    expect(state.draft.expanded?.has("parent")).toBe(true);
    expect(state.draft.expanded?.has("other")).toBe(false);
    expect(host.querySelector('[aria-label="hidden"]')).toBeNull();
  });
  it("waits for the confirmed saved projection before focusing a reparented row", async () => {
    const before = [step("parent"), step("moved", 1, "parent"), step("other", 1, "parent"), step("next")];
    const after = task([step("parent"), step("other", 1, "parent"), step("moved"), step("next")], { task_revision: "confirmed" });
    const ui = await mount(task(before), { submit: async () => ({ kind: "confirmed", task: after }) });
    const precedingRow = checkbox("moved");
    await click(button("Actions for moved")); await click(button("Outdent"));
    expect(document.activeElement).not.toBe(precedingRow);
    await ui.push({ task: task(before, { task_revision: "preceding-poll" }) });
    expect(document.activeElement).not.toBe(precedingRow);
    await ui.push({ task: after });
    expect(document.activeElement).toBe(checkbox("moved"));
  });
  it.each(["row", "control", "task"] as const)("does not steal focus after navigation while a confirmed move awaits projection (%s)", async (navigation) => {
    const before = task([step("parent"), step("moved", 1, "parent"), step("other", 1, "parent"), step("next")]);
    const after = task([step("parent"), step("other", 1, "parent"), step("moved"), step("next")], { task_revision: "confirmed" });
    const ui = await mount(before, { submit: async () => ({ kind: "confirmed", task: after }) });
    const originalDraft = state.draft;
    await click(button("Actions for moved")); await click(button("Outdent"));
    if (navigation === "task") {
      await ui.push({ task: task([step("elsewhere")], { task_id: "other-task" }),
        scope: { ...state.scope, taskId: "other-task" }, draft: newStepDraft() });
      act(() => checkbox("elsewhere").focus());
      await ui.push({ task: before, scope: { ...state.scope, taskId: "task" }, draft: originalDraft });
      await ui.push({ task: after });
      expect(document.activeElement).not.toBe(checkbox("moved"));
    } else {
      const destination = navigation === "row" ? checkbox("next") : button("Add step");
      act(() => destination.focus());
      await ui.push({ task: after });
      expect(document.activeElement).toBe(destination);
    }
  });
  it("keeps navigation made before the move response arrives", async () => {
    const before = task([step("parent"), step("moved", 1, "parent"), step("other", 1, "parent"), step("next")]);
    const after = task([step("parent"), step("other", 1, "parent"), step("moved"), step("next")], { task_revision: "confirmed" });
    let finish!: (outcome: TaskMutationOutcome<StepSubmission>) => void;
    const response = new Promise<TaskMutationOutcome<StepSubmission>>((resolve) => { finish = resolve; });
    const ui = await mount(before, { submit: () => response });
    await click(button("Actions for moved")); await click(button("Outdent"));
    const destination = checkbox("next"); act(() => destination.focus());
    await act(async () => finish({ kind: "confirmed", task: after }));
    await ui.push({ task: after });
    expect(document.activeElement).toBe(destination);
  });
  it("guards composing/repeated Enter in a title form without clipping or discarding the draft", async () => {
    await mount(task()); await click(button("Add step")); enter(titleField(), "Retained");
    expect(press(titleField(), "Enter", { repeat: true }).defaultPrevented).toBe(true);
    expect(press(titleField(), "Enter", { isComposing: true }).defaultPrevented).toBe(true);
    expect(state.submit).not.toHaveBeenCalled(); expect(state.draft.add.get(null)?.text).toBe("Retained");
  });
  it("keeps the composer focused after confirmation and clears only the matching submitted text", async () => {
    const ui = await mount(task()); await click(button("Add step")); enter(titleField(), "One");
    state.submit = vi.fn<SupervisorStepsProps["submit"]>(async (submitted: StepSubmission) => {
      if (submitted.intent.kind !== "add") throw new Error("Expected add");
      const saved = task([{ ...step(submitted.intent.stepId), title: submitted.intent.title }], { task_revision: "added" });
      state.task = saved; return { kind: "confirmed", task: saved };
    });
    await ui.push({ submit: state.submit });
    await submitForm(titleField());
    expect(titleField().value).toBe(""); expect(document.activeElement).toBe(titleField());
    expect(state.draft.add.get(null)?.retainedSubmission).toBeNull();
    enter(titleField(), "Second"); expect(state.draft.add.get(null)?.baseTaskRevision).toBe("added");
  });
  it("keeps a later text edit through an in-flight add without reusing its saved UUID or accepting a second write", async () => {
    let resolveReceipt!: (outcome: TaskMutationOutcome<StepSubmission>) => void;
    const receipt = new Promise<TaskMutationOutcome<StepSubmission>>((resolve) => { resolveReceipt = resolve; });
    await mount(task(), { submit: vi.fn<SupervisorStepsProps["submit"]>(() => receipt) });
    await click(button("Add step")); enter(titleField(), "Sent text"); await submitForm(titleField());
    const original = state.draft.add.get(null)!.retainedSubmission!;
    enter(titleField(), "Later text"); await submitForm(titleField());
    expect(state.submit).toHaveBeenCalledOnce(); expect(state.draft.operation.kind).toBe("sending");
    if (original.intent.kind !== "add") throw new Error("Expected add");
    const saved = task([{ ...step(original.intent.stepId), title: original.intent.title }], { task_revision: "confirmed-add" });
    state.task = saved;
    await act(async () => { resolveReceipt({ kind: "confirmed", task: saved }); }); await settle();
    expect(titleField().value).toBe("Later text"); expect(state.draft.add.get(null)?.retainedSubmission).toBeNull();
    expect(state.draft.add.get(null)?.baseTaskRevision).toBe("original");
    await submitForm(titleField());
    const next = state.draft.add.get(null)!.retainedSubmission!;
    expect(next.intent.kind).toBe("add");
    if (next.intent.kind !== "add") throw new Error("Expected new add");
    expect(next.intent.stepId).not.toBe(original.intent.stepId);
    expect(state.draft.operation.kind).toBe("review"); expect(state.submit).toHaveBeenCalledOnce();
  });
});

describe("unknown original-identity reconciliation and definite refusals", () => {
  it("requires explicit original-target read and resolution; matching Found and polling do not clear the shared gate", async () => {
    const saved = task([step("a")]);
    const ui = await mount(saved, { submit: vi.fn<SupervisorStepsProps["submit"]>(async (submitted) => {
      state.taskWriteUnconfirmed = true; return { kind: "unknown", submitted, message: "Lost response" };
    }) });
    await click(checkbox("a"));
    const operation = state.draft.operation;
    if (operation.kind !== "unknown") throw new Error("Expected unknown");
    const original = operation.submitted;
    const found = task([step("a", 0, null, "done")], { task_revision: "saved" });
    state.readSaved = vi.fn<SupervisorStepsProps["readSaved"]>(async () => ({ kind: "found", task: found }));
    await ui.push({ task: found }); expect(state.draft.operation.kind).toBe("unknown");
    await click(button("Check saved steps"));
    expect(state.readSaved).toHaveBeenCalledWith(original.scope); expect(state.taskWriteUnconfirmed).toBe(true);
    expect(state.resolveUnknown).not.toHaveBeenCalled(); expect(state.draft.operation.kind).toBe("review");
    state.resolveUnknown = vi.fn<SupervisorStepsProps["resolveUnknown"]>(async (request) => {
      state.taskWriteUnconfirmed = false;
      return { kind: "resolved", originalScope: request.originalScope, originalSubmissionId: request.originalSubmissionId, task: found };
    });
    await ui.push({ resolveUnknown: state.resolveUnknown });
    await click(button("Use saved state"));
    expect(state.resolveUnknown).toHaveBeenCalledWith({ originalScope: original.scope, originalSubmissionId: original.submissionId,
      originalSubmitted: original, reviewedTaskRevision: "saved", decision: { kind: "use_saved" } });
    expect(state.draft.operation.kind).toBe("idle"); expect(state.taskWriteUnconfirmed).toBe(false);
    expect(host.textContent).toContain("does not establish who"); expect(state.submit).toHaveBeenCalledOnce();
  });
  it("retains the add UUID and original payload through mismatch, failed resolution and explicit reviewed application", async () => {
    const ui = await mount(task(), { submit: vi.fn<SupervisorStepsProps["submit"]>(async (submitted) => { state.taskWriteUnconfirmed = true;
      return { kind: "unknown", submitted, message: "Unconfirmed" }; }) });
    await click(button("Add step")); enter(titleField(), "Original title"); await submitForm(titleField());
    const original = state.draft.add.get(null)!.retainedSubmission!;
    enter(titleField(), "Later unsent draft");
    const found = task([], { task_revision: "read-revision" });
    state.readSaved = vi.fn<SupervisorStepsProps["readSaved"]>(async () => ({ kind: "found", task: found }));
    await ui.push({ readSaved: state.readSaved });
    await click(button("Check saved steps")); await click(button("Keep saved state"));
    expect(state.draft.operation.kind).toBe("review"); expect(state.taskWriteUnconfirmed).toBe(true);
    const resolver = vi.fn<SupervisorStepsProps["resolveUnknown"]>(async (request) => {
      if (request.decision.kind !== "apply_reviewed") throw new Error("Expected reviewed application");
      return { kind: "applied", originalScope: request.originalScope, originalSubmissionId: request.originalSubmissionId,
        submitted: request.decision.submitted, outcome: { kind: "unknown", submitted: request.decision.submitted, message: "Again unconfirmed" } };
    });
    state.resolveUnknown = resolver;
    await ui.push({ resolveUnknown: state.resolveUnknown });
    await click(button("Apply reviewed change"));
    const request = resolver.mock.calls[0][0]; expect(request.originalSubmitted).toBe(original);
    if (request.decision.kind !== "apply_reviewed") throw new Error("Expected apply");
    expect(request.decision.submitted.intent).toEqual(original.intent); expect(request.decision.submitted.expectedTaskRevision).toBe("read-revision");
    expect(request.decision.submitted.submissionId).not.toBe(original.submissionId);
    expect(state.draft.operation.kind).toBe("unknown"); expect(state.draft.add.get(null)?.text).toBe("Later unsent draft");
    expect(state.submit).toHaveBeenCalledOnce();
  });
  it("routes a late original-target read only to its retained draft after changing tasks", async () => {
    let resolveRead!: (outcome: StepReadOutcome) => void;
    const savedRead = new Promise<StepReadOutcome>((resolve) => { resolveRead = resolve; });
    const originalDraft = newStepDraft();
    const original: StepSubmission = { submissionId: "original-submission", scope: { sessionId: "session", rootId: "root", taskId: "task" },
      expectedTaskRevision: "original", intent: { kind: "rename", stepId: "a", title: "Original intent" } };
    originalDraft.operation = { kind: "unknown", submitted: original, message: "Lost receipt" };
    const ui = await mount(task([step("a")]), { draft: originalDraft, taskWriteUnconfirmed: true,
      readSaved: vi.fn<SupervisorStepsProps["readSaved"]>(() => savedRead) });
    await click(button("Check saved steps"));
    const otherDraft = newStepDraft();
    await ui.push({ scope: { sessionId: "session", rootId: "other-root", taskId: "other-task" },
      task: task([step("other")], { task_id: "other-task" }), draft: otherDraft, taskWriteUnconfirmed: false });
    await act(async () => { resolveRead({ kind: "found", task: task([step("a")], { task_revision: "original-read" }) }); });
    await settle();
    expect(originalDraft.operation.kind).toBe("review"); expect(otherDraft.operation.kind).toBe("idle");
    expect(state.readSaved).toHaveBeenCalledWith(original.scope); expect(state.resolveUnknown).not.toHaveBeenCalled();
    expect(state.submit).not.toHaveBeenCalled(); expect(checkbox("other")).toBeTruthy();
  });
  it("does not send a removed draft target through the reviewed-apply path", async () => {
    const retained = newStepDraft();
    const submitted: StepSubmission = { submissionId: "kept", scope: { sessionId: "session", rootId: "root", taskId: "task" },
      expectedTaskRevision: "before", intent: { kind: "rename", stepId: "removed", title: "Kept title" } };
    retained.operation = { kind: "review", submitted, reason: "unknown", currentTask: task([], { task_revision: "gone" }) };
    await mount(task([], { task_revision: "gone" }), { draft: retained, taskWriteUnconfirmed: true });
    await click(button("Apply reviewed change"));
    expect(state.submit).not.toHaveBeenCalled(); expect(state.resolveUnknown).not.toHaveBeenCalled();
    expect(retained.operation.kind).toBe("review");
  });
  it.each(["unsafe", "oversized"] as const)("uses structural validity for reviewed application to an %s forest", async (source) => {
    const steps = source === "unsafe" ? [step("a")] : Array.from({ length: 65 }, (_, index) => step(index === 0 ? "a" : `step-${index}`));
    const reviewed = task(steps, {
      task_revision: "reviewed",
      step_progress: source === "unsafe" ? null : { done: 0, total: 65 },
      steps_diagnostic: source === "unsafe" ? "Malformed managed boundaries" : "Saved count exceeds the authoring limit",
    });
    const retained = newStepDraft();
    const submitted: StepSubmission = {
      submissionId: "retained-check",
      scope: { sessionId: "session", rootId: "root", taskId: "task" },
      expectedTaskRevision: "before",
      intent: { kind: "set_checked", stepId: "a", checked: true, scope: "leaf" },
    };
    const review = { kind: "review" as const, submitted, reason: "unknown" as const, currentTask: reviewed };
    retained.operation = review;
    await mount(task([step("a")]), { draft: retained, taskWriteUnconfirmed: true });
    const apply = button("Apply reviewed change");
    expect(apply.getAttribute("aria-disabled")).toBe(source === "unsafe" ? "true" : "false");
    await click(apply);
    expect(state.submit).not.toHaveBeenCalled();
    if (source === "unsafe") {
      expect(state.resolveUnknown).not.toHaveBeenCalled();
    } else {
      expect(state.resolveUnknown).toHaveBeenCalledOnce();
    }
    expect(retained.operation).toBe(review);
    expect(state.taskWriteUnconfirmed).toBe(true);
  });
  it("shows guaranteed refusals separately, then permits only explicit read/review against a fresh revision", async () => {
    const ui = await mount(task([step("a")]), { submit: vi.fn<SupervisorStepsProps["submit"]>(async () => ({ kind: "refused", operationCode: "task_revision_conflict", message: "Task changed elsewhere." })) });
    await click(checkbox("a")); expect(state.draft.operation.kind).toBe("refused");
    expect(host.textContent).toContain("No change applied"); expect(button("Review current steps")).toBeTruthy();
    state.readSaved = vi.fn<SupervisorStepsProps["readSaved"]>(async () => ({ kind: "found", task: task([step("a")], { task_revision: "reviewed" }) }));
    await ui.push({ readSaved: state.readSaved });
    await click(button("Review current steps")); await click(button("Apply reviewed change"));
    expect(state.submit).toHaveBeenLastCalledWith(expect.objectContaining({ expectedTaskRevision: "reviewed" }));
    expect(state.resolveUnknown).not.toHaveBeenCalled();
  });
});

describe("untracked, diagnostic and saved overflow states", () => {
  it("tracks every untracked row atomically with proposed-once UUIDs, original revision and safe preview", async () => {
    await mount(task([step(null, 0, null, "done", 10), step(null, 1, null, "open", 20)]));
    expect(host.querySelectorAll("[data-row-id]")).toHaveLength(0);
    await click(button("Track checklist…"));
    const submitted = state.draft.confirmation!.submitted;
    expect(document.activeElement).toBe(button("Keep steps")); expect(state.submit).not.toHaveBeenCalled();
    if (submitted.intent.kind !== "adopt") throw new Error("Expected adoption");
    expect(submitted.intent.mapping.map((entry) => entry.sourceOffset)).toEqual([10, 20]);
    expect(new Set(submitted.intent.mapping.map((entry) => entry.stepId)).size).toBe(2);
    await click(button("Track checklist")); expect(state.submit).toHaveBeenCalledWith(submitted);
  });
  it("keeps all oversized/deep saved rows readable while safe checks remain available", async () => {
    await mount(task(Array.from({ length: 70 }, (_, index) => step(`leaf-${index}`)), { steps_diagnostic: "Tracked count exceeds limit.", step_progress: { done: 0, total: 70 } }));
    expect(host.querySelectorAll('input[type="checkbox"]')).toHaveLength(70);
    expect(button("Add step").getAttribute("aria-disabled")).toBe("true");
    await click(checkbox("leaf-69")); expect(state.submit).toHaveBeenCalledOnce();
    expect(host.textContent).toContain("Saved rows remain visible");
  });
  it("makes unsafe source and accepted tasks read-only without hiding saved mixed marks or source", async () => {
    const unsafe = step("a", 0, null, "partial"); unsafe.diagnostic = "Duplicate identity";
    const ui = await mount(task([unsafe], { steps_diagnostic: "Duplicate identity", step_progress: null }));
    await click(checkbox("a")); expect(state.submit).not.toHaveBeenCalled(); expect(host.textContent).toContain("Unavailable");
    expect(checkbox("a").indeterminate).toBe(true); expect(host.querySelector("pre")?.textContent).toBe(state.task?.body);
    await ui.push({ task: task([step("a", 0, null, "partial")], { checked: true }) });
    await click(checkbox("a")); expect(state.submit).not.toHaveBeenCalled(); expect(host.textContent).toContain("Task is complete");
  });
});
