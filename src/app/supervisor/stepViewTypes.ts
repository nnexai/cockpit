import type { Task } from "../../protocol/generated/v1";
import type { StepIntent, StepSubmission } from "./stepInteractions";

export type Step = Task["steps"][number];
export type StepMenuState = { x: number; y: number; stepId: string; task: Task; opener: HTMLElement };
export type MoveFocusOrigin = { element: Element | null; opener: HTMLElement | null; focusedStepId: string | null };
export type StepNotice = (text: string, tone?: "notice" | "warning" | "error" | "pending") => void;

export function sameScope(left: StepSubmission["scope"], right: StepSubmission["scope"]): boolean {
  return left.sessionId === right.sessionId && left.rootId === right.rootId && left.taskId === right.taskId;
}

export function intentText(intent: StepIntent): string {
  switch (intent.kind) {
    case "add": return `Add “${intent.title}” (${intent.stepId}) under ${intent.parentStepId ?? "task root"}.`;
    case "rename": return `Rename ${intent.stepId} to “${intent.title}”.`;
    case "set_checked": return `Mark ${intent.scope === "subtree" ? "subtree" : "step"} ${intent.stepId} ${intent.checked ? "done" : "open"}.`;
    case "move": return `Move ${intent.stepId} under ${intent.parentStepId ?? "task root"}, before ${intent.beforeStepId ?? "end"}.`;
    case "remove": return `Remove the complete subtree ${intent.stepId}.`;
  }
}

export function reviewedTargetProblem(task: Task, intent: StepIntent): string | null {
  if (task.checked) return "The saved task is complete; its steps are read-only.";
  if (task.step_progress === null || task.steps.some((step) => step.diagnostic !== null) || (task.steps.length === 0 && task.steps_diagnostic !== null)) {
    return "The saved step source is unsafe. Keep the saved state and inspect its diagnostic.";
  }
  if (intent.kind !== "add" && !task.steps.some((step) => step.step_id === intent.stepId)) {
    return "The original step was removed. Your draft is kept; it cannot be saved to another target.";
  }
  if ((intent.kind === "add" || intent.kind === "move") && intent.parentStepId !== null &&
    !task.steps.some((step) => step.step_id === intent.parentStepId)) {
    return "The original destination was removed. Keep or copy your draft.";
  }
  return null;
}
