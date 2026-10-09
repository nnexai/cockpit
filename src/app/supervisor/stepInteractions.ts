import type { Task } from "../../protocol/generated/v1";
import type { TaskMutationOutcome } from "./useSupervisor";

export type StepScope = Readonly<{ sessionId: string; rootId: string; taskId: string }>;
export type StepIntent =
  | { kind: "add"; stepId: string; parentStepId: string | null; beforeStepId: string | null; title: string }
  | { kind: "rename"; stepId: string; title: string }
  | { kind: "set_checked"; stepId: string; checked: boolean; scope: "leaf" | "subtree" }
  | { kind: "move"; stepId: string; parentStepId: string | null; beforeStepId: string | null }
  | { kind: "remove"; stepId: string };
export type StepSubmission = Readonly<{
  submissionId: string; scope: StepScope; expectedTaskRevision: string; intent: StepIntent;
}>;
export type StepFocusTarget =
  | { kind: "row"; stepId: string }
  | { kind: "add"; parentStepId: string | null }
  | { kind: "rename"; stepId: string }
  | { kind: "safe_confirmation" }
  | { kind: "toolbar_add" };
export type StepFocusRequest = Readonly<{ serial: number; target: StepFocusTarget }>;
export type StepTextDraft = {
  text: string; baseTaskRevision: string; baseTitle: string | null;
  baseParentStepId: string | null; baseBeforeStepId: string | null;
  retainedSubmission: StepSubmission | null; open: boolean;
};
export type StepConfirmation = { submitted: StepSubmission; baseTask: Task };
export type StepOperationState =
  | { kind: "idle" }
  | { kind: "sending"; submitted: StepSubmission }
  | { kind: "refused"; submitted: StepSubmission; operationCode: string; message: string }
  | { kind: "unknown"; submitted: StepSubmission; message: string }
  | { kind: "review"; submitted: StepSubmission; currentTask: Task; reason: "conflict" | "unknown" };
export type StepDraftState = {
  add: Map<string | null, StepTextDraft>; rename: Map<string, StepTextDraft>;
  expanded: Set<string> | null; focusedStepId: string | null;
  confirmation: StepConfirmation | null; operation: StepOperationState;
  focusRequest: StepFocusRequest | null;
  status: { tone: "pending" | "notice" | "warning" | "error"; text: string } | null;
};
export type StepReadOutcome =
  | { kind: "found"; task: Task } | { kind: "missing" } | { kind: "unavailable"; message: string };
export type StepUnknownResolution = Readonly<{
  originalScope: StepScope; originalSubmissionId: string; originalSubmitted: StepSubmission;
  reviewedTaskRevision: string;
  decision: { kind: "use_saved" } | { kind: "keep_saved" } | { kind: "apply_reviewed"; submitted: StepSubmission };
}>;
export type StepResolutionOutcome =
  | { kind: "resolved"; originalScope: StepScope; originalSubmissionId: string; task: Task }
  | { kind: "not_resolved"; originalScope: StepScope; originalSubmissionId: string;
      code: "resolution_stale" | "read_required" | "different_source_operation" | "busy"; message: string }
  | { kind: "applied"; originalScope: StepScope; originalSubmissionId: string; submitted: StepSubmission;
      outcome: TaskMutationOutcome<StepSubmission> };
export type SupervisorStepsProps = {
  scope: StepScope; task: Task | null; draft: StepDraftState;
  writable: boolean; readOnlyReason: string | null; busy: boolean; taskWriteUnconfirmed: boolean;
  onDraftChanged: () => void;
  submit: (submitted: StepSubmission) => Promise<TaskMutationOutcome<StepSubmission>>;
  readSaved: (scope: StepScope) => Promise<StepReadOutcome>;
  resolveUnknown: (request: StepUnknownResolution) => Promise<StepResolutionOutcome>;
  onCloseDetails: () => void;
};

type Step = Task["steps"][number];

export function newStepDraft(): StepDraftState {
  return { add: new Map(), rename: new Map(), expanded: null, focusedStepId: null,
    confirmation: null, operation: { kind: "idle" }, focusRequest: null, status: null };
}

export function visibleSteps(task: Task, expanded: ReadonlySet<string>): readonly Step[] {
  const result: Step[] = [];
  let hiddenBelow: number | null = null;
  for (let index = 0; index < task.steps.length; index += 1) {
    const step = task.steps[index];
    if (hiddenBelow !== null && step.depth > hiddenBelow) continue;
    hiddenBelow = null;
    result.push(step);
    if (step.step_id !== null && task.steps[index + 1]?.depth > step.depth && !expanded.has(step.step_id)) {
      hiddenBelow = step.depth;
    }
  }
  return result;
}

export function removalFocus(before: readonly Step[], after: readonly Step[], removedStepId: string): StepFocusTarget {
  const index = before.findIndex((step) => step.step_id === removedStepId);
  const removed = before[index];
  if (!removed) return { kind: "toolbar_add" };
  const surviving = new Set(after.flatMap((step) => step.step_id === null ? [] : [step.step_id]));
  let next = index + 1;
  while (next < before.length && before[next].depth > removed.depth) next += 1;
  for (; next < before.length; next += 1) {
    const id = before[next].step_id;
    if (id !== null && surviving.has(id)) return { kind: "row", stepId: id };
  }
  for (let previous = index - 1; previous >= 0; previous -= 1) {
    const step = before[previous];
    if (step.depth === removed.depth && step.parent_step_id === removed.parent_step_id && step.step_id !== null && surviving.has(step.step_id)) {
      return { kind: "row", stepId: step.step_id };
    }
  }
  if (removed.parent_step_id !== null && surviving.has(removed.parent_step_id)) {
    return { kind: "row", stepId: removed.parent_step_id };
  }
  return { kind: "toolbar_add" };
}

export function stepSubtree(task: Task, stepId: string): readonly Step[] {
  const start = task.steps.findIndex((step) => step.step_id === stepId);
  if (start < 0) return [];
  let end = start + 1;
  while (end < task.steps.length && task.steps[end].depth > task.steps[start].depth) end += 1;
  return task.steps.slice(start, end);
}

export function leafProgress(steps: readonly Step[]): { done: number; total: number } {
  let done = 0;
  let total = 0;
  steps.forEach((step, index) => {
    if (step.step_id === null || step.diagnostic !== null || steps[index + 1]?.depth > step.depth) return;
    total += 1;
    if (step.status === "done") done += 1;
  });
  return { done, total };
}

export function defaultExpansion(task: Task): Set<string> {
  return new Set(task.steps.flatMap((step, index) =>
    step.step_id !== null && task.steps[index + 1]?.depth > step.depth &&
    (task.steps.length <= 20 || step.status !== "done") ? [step.step_id] : []));
}

export function titleProblem(text: string, baseTitle: string | null = null): string | null {
  if (!text.trim()) return "Enter a step title.";
  if (/\r|\n/.test(text)) return "Use a single-line step title.";
  const scalars = [...text].length;
  if (scalars > 200 && (baseTitle === null || scalars >= [...baseTitle].length)) {
    return "Use at most 200 characters, or shorten the existing oversized title.";
  }
  return null;
}

export function moveIntent(task: Task, stepId: string, direction: "up" | "down" | "indent" | "outdent"): StepIntent | null {
  const step = task.steps.find((candidate) => candidate.step_id === stepId);
  if (!step) return null;
  const siblings = task.steps.filter((candidate) => candidate.step_id !== null && candidate.parent_step_id === step.parent_step_id && candidate.depth === step.depth);
  const index = siblings.findIndex((candidate) => candidate.step_id === stepId);
  const previous = siblings[index - 1];
  const next = siblings[index + 1];
  if (direction === "up") return previous ? { kind: "move", stepId, parentStepId: step.parent_step_id, beforeStepId: previous.step_id } : null;
  if (direction === "down") return next ? { kind: "move", stepId, parentStepId: step.parent_step_id, beforeStepId: siblings[index + 2]?.step_id ?? null } : null;
  if (direction === "indent") {
    const subtree = stepSubtree(task, stepId);
    if (!previous || subtree.some((candidate) => candidate.depth + 1 > 4)) return null;
    return { kind: "move", stepId, parentStepId: previous.step_id, beforeStepId: null };
  }
  const parent = task.steps.find((candidate) => candidate.step_id === step.parent_step_id);
  if (!parent) return null;
  const parents = task.steps.filter((candidate) => candidate.step_id !== null && candidate.parent_step_id === parent.parent_step_id && candidate.depth === parent.depth);
  const parentIndex = parents.findIndex((candidate) => candidate.step_id === parent.step_id);
  return { kind: "move", stepId, parentStepId: parent.parent_step_id, beforeStepId: parents[parentIndex + 1]?.step_id ?? null };
}

/** Compare durable identity and absolute intent, not a matching label or actor. */
export function intentMatchesSaved(task: Task, intent: StepIntent): boolean {
  const step = task.steps.find((candidate) => candidate.step_id === intent.stepId);
  if (intent.kind === "remove") return !step;
  if (!step) return false;
  if (intent.kind === "rename") return step.title === intent.title;
  if (intent.kind === "set_checked") {
    const subtree = stepSubtree(task, intent.stepId);
    if (intent.scope === "leaf" && subtree.length !== 1) return false;
    return subtree.every((candidate) => candidate.status === (intent.checked ? "done" : "open"));
  }
  const siblings = task.steps.filter((candidate) => candidate.step_id !== null && candidate.parent_step_id === intent.parentStepId && candidate.depth === step.depth);
  const index = siblings.findIndex((candidate) => candidate.step_id === intent.stepId);
  const nextId = siblings[index + 1]?.step_id ?? null;
  return step.parent_step_id === intent.parentStepId && nextId === intent.beforeStepId &&
    (intent.kind === "move" || (step.title === intent.title && step.status === "open" && stepSubtree(task, intent.stepId).length === 1));
}
