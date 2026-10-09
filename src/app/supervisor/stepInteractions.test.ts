import { describe, expect, it } from "vitest";
import type { Task } from "../../protocol/generated/v1";
import { defaultExpansion, intentMatchesSaved, leafProgress, moveIntent, newStepDraft, removalFocus, stepSubtree, titleProblem, visibleSteps } from "./stepInteractions";

function step(stepId: string | null, depth = 0, parent: string | null = null, status: Task["steps"][number]["status"] = "open", sourceOffset = 0): Task["steps"][number] {
  return { step_id: stepId, parent_step_id: parent, depth, title: stepId ?? "Untracked", status,
    checked: status === "done", line: sourceOffset + 1, source_offset: sourceOffset, diagnostic: null };
}
function task(steps: Task["steps"]): Task {
  return { task_id: "task", title: "Task", body: "source", description: "prose", description_editable: true, description_diagnostic: null,
    steps, step_progress: { done: 0, total: 0 }, steps_diagnostic: null, depends_on: [], follow_up_of: null,
    relations_diagnostic: null, checked: false, line: 1, task_revision: "original", diagnostic: null };
}
const forest = () => task([step("a", 0, null, "partial"), step("a1", 1, "a", "done"), step("a2", 1, "a"), step("b"), step("c")]);

describe("saved projection and local hierarchy", () => {
  it("creates independent mutable drafts without a canonical task copy", () => {
    const first = newStepDraft(), second = newStepDraft();
    first.expanded = new Set(["a"]);
    first.rename.set("a", { text: "draft", baseTaskRevision: "original", baseTitle: "a", baseParentStepId: null,
      baseBeforeStepId: "b", retainedSubmission: null, open: true });
    expect(second.expanded).toBeNull();
    expect(second.rename.size).toBe(0);
    expect(second.operation).toEqual({ kind: "idle" });
  });
  it("collapses by preorder depth without reparsing or hiding unrelated roots", () => {
    const saved = forest();
    expect(visibleSteps(saved, new Set()).map((node) => node.step_id)).toEqual(["a", "b", "c"]);
    expect(visibleSteps(saved, new Set(["a"]))).toEqual(saved.steps);
    expect(stepSubtree(saved, "a").map((node) => node.step_id)).toEqual(["a", "a1", "a2"]);
    expect(saved.steps[0].status).toBe("partial");
  });
  it("keeps untracked source identities null and honors untracked ancestor depth", () => {
    const saved = task([step(null, 0, null, "open", 10), step(null, 1, null, "done", 20), step("tracked")]);
    expect(visibleSteps(saved, new Set())).toEqual(saved.steps);
    expect(leafProgress(saved.steps)).toEqual({ done: 0, total: 1 });
  });
  it("counts only valid tracked leaves, not raw branch checks", () => {
    const saved = forest(); saved.steps[0].checked = true;
    expect(leafProgress(saved.steps)).toEqual({ done: 1, total: 4 });
    saved.steps[1].diagnostic = "unsafe";
    expect(leafProgress(saved.steps)).toEqual({ done: 0, total: 3 });
  });
  it("reads all saved overflow while choosing initial expansion only once", () => {
    const saved = task([step("done", 0, null, "done"), ...Array.from({ length: 70 }, (_, index) => step(`leaf-${index}`, 1, "done", "done")), step("open"), step("child", 1, "open")]);
    expect(defaultExpansion(saved)).toEqual(new Set(["open"]));
    expect(visibleSteps(saved, new Set(["done", "open"]))).toHaveLength(73);
    expect(defaultExpansion(forest())).toEqual(new Set(["a"]));
  });
});

describe("stable structural intents and removal focus", () => {
  it("translates sibling reorders without using descendant positions as destinations", () => {
    const saved = forest();
    expect(moveIntent(saved, "b", "up")).toEqual({ kind: "move", stepId: "b", parentStepId: null, beforeStepId: "a" });
    expect(moveIntent(saved, "a", "down")).toEqual({ kind: "move", stepId: "a", parentStepId: null, beforeStepId: "c" });
    expect(moveIntent(saved, "c", "down")).toBeNull();
    expect(moveIntent(saved, "a", "up")).toBeNull();
    expect(moveIntent(saved, "b", "indent")).toEqual({ kind: "move", stepId: "b", parentStepId: "a", beforeStepId: null });
  });
  it("outdents an entire subtree immediately after its parent", () => {
    expect(moveIntent(forest(), "a1", "outdent")).toEqual({ kind: "move", stepId: "a1", parentStepId: null, beforeStepId: "b" });
    expect(moveIntent(forest(), "a", "outdent")).toBeNull();
  });
  it("does not offer indentation that increases a deepest descendant past depth four", () => {
    const saved = task([step("previous"), step("moving"), step("deep", 4, "moving")]);
    expect(moveIntent(saved, "moving", "indent")).toBeNull();
    expect(moveIntent(saved, "missing", "up")).toBeNull();
  });
  it("never chooses a removed descendant, then falls back to sibling, parent and Add", () => {
    const saved = forest();
    expect(removalFocus(saved.steps, saved.steps.slice(3), "a")).toEqual({ kind: "row", stepId: "b" });
    expect(removalFocus(saved.steps, saved.steps.slice(0, 4), "c")).toEqual({ kind: "row", stepId: "b" });
    const loneChild = task([step("parent"), step("child", 1, "parent")]);
    expect(removalFocus(loneChild.steps, [loneChild.steps[0]], "child")).toEqual({ kind: "row", stepId: "parent" });
    expect(removalFocus([step("only")], [], "only")).toEqual({ kind: "toolbar_add" });
  });
});

describe("exact intent comparison and title limits", () => {
  it("requires the submitted UUID, parent, order, title and initial open leaf for add", () => {
    const saved = task([step("new"), step("next")]); saved.steps[0].title = "Requested";
    const intent = { kind: "add", stepId: "new", parentStepId: null, beforeStepId: "next", title: "Requested" } as const;
    expect(intentMatchesSaved(saved, intent)).toBe(true);
    expect(intentMatchesSaved(saved, { ...intent, stepId: "same-label-other-id" })).toBe(false);
    expect(intentMatchesSaved(saved, { ...intent, beforeStepId: null })).toBe(false);
    saved.steps[0].status = "done";
    expect(intentMatchesSaved(saved, intent)).toBe(false);
  });
  it("distinguishes explicit leaf and complete subtree state", () => {
    const saved = forest();
    expect(intentMatchesSaved(saved, { kind: "set_checked", stepId: "a", checked: true, scope: "leaf" })).toBe(false);
    expect(intentMatchesSaved(saved, { kind: "set_checked", stepId: "a", checked: true, scope: "subtree" })).toBe(false);
    for (const node of saved.steps.slice(0, 3)) node.status = "done";
    expect(intentMatchesSaved(saved, { kind: "set_checked", stepId: "a", checked: true, scope: "subtree" })).toBe(true);
    expect(intentMatchesSaved(saved, { kind: "remove", stepId: "absent" })).toBe(true);
  });
  it("counts Unicode scalar values and permits safe shortening of oversized saved titles", () => {
    expect(titleProblem("😀".repeat(200))).toBeNull();
    expect(titleProblem("😀".repeat(201))).not.toBeNull();
    expect(titleProblem("x".repeat(250), "x".repeat(300))).toBeNull();
    expect(titleProblem("x".repeat(300), "x".repeat(300))).not.toBeNull();
    expect(titleProblem(" \t ")).not.toBeNull();
    expect(titleProblem("one\ntwo")).not.toBeNull();
  });
});
