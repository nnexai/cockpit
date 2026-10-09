import type { Task } from "../../protocol/generated/v1";
import { stepSubtree, titleProblem, type StepIntent, type StepTextDraft, type SupervisorStepsProps } from "./stepInteractions";
import type { StepMutation } from "./useStepMutation";
import type { StepFocus } from "./useStepFocus";

export type StepText = {
  openText: (kind: "add" | "rename", stepId: string | null, baseTask: Task) => void;
  submitText: (kind: "add" | "rename", key: string | null, textDraft: StepTextDraft) => void;
};

export function useStepText(props: SupervisorStepsProps, mutation: StepMutation, focus: StepFocus, expand: (stepId: string) => void): StepText {
  const { task, draft } = props;
  const { guard, submission, notice, preview, send } = mutation;
  const { requestFocus } = focus;
  function openText(kind: "add" | "rename", stepId: string | null, baseTask: Task) {
    if (!guard()) return;
    const step = baseTask.steps.find((candidate) => candidate.step_id === stepId);
    if (kind === "rename" && (!step || stepId === null)) return;
    const key = stepId;
    let textDraft = kind === "add" ? draft.add.get(key) : draft.rename.get(stepId ?? "");
    if (!textDraft) {
      const siblings = step ? baseTask.steps.filter((candidate) => candidate.depth === step.depth && candidate.parent_step_id === step.parent_step_id) : [];
      const index = siblings.findIndex((candidate) => candidate.step_id === stepId);
      textDraft = { text: kind === "rename" ? step?.title ?? "" : "", baseTaskRevision: baseTask.task_revision,
        baseTitle: step?.title ?? null, baseParentStepId: kind === "add" ? stepId : step?.parent_step_id ?? null,
        baseBeforeStepId: kind === "rename" ? siblings[index + 1]?.step_id ?? null : null,
        retainedSubmission: null, open: true };
      if (kind === "add") draft.add.set(key, textDraft); else if (stepId !== null) draft.rename.set(stepId, textDraft);
    }
    textDraft.open = true;
    if (kind === "add" && stepId !== null) expand(stepId);
    requestFocus(kind === "add" ? { kind: "add", parentStepId: stepId } : { kind: "rename", stepId: stepId! });
  }
  function submitText(kind: "add" | "rename", key: string | null, textDraft: StepTextDraft) {
    if (!task || !guard()) return;
    const problem = titleProblem(textDraft.text, kind === "rename" ? textDraft.baseTitle : null);
    if (problem) { notice(problem, "warning"); return; }
    const oldIntent = textDraft.retainedSubmission?.intent;
    const intent: StepIntent = kind === "add" ? { kind: "add", stepId: oldIntent?.kind === "add" ? oldIntent.stepId : crypto.randomUUID(),
      parentStepId: textDraft.baseParentStepId, beforeStepId: textDraft.baseBeforeStepId, title: textDraft.text }
      : { kind: "rename", stepId: key!, title: textDraft.text };
    const submitted = textDraft.retainedSubmission &&
      JSON.stringify(textDraft.retainedSubmission.intent) === JSON.stringify(intent)
      ? textDraft.retainedSubmission : submission(intent, textDraft.baseTaskRevision || task.task_revision);
    textDraft.retainedSubmission = submitted;
    if (submitted.expectedTaskRevision !== task.task_revision) {
      draft.operation = { kind: "review", submitted, currentTask: task, reason: "conflict" };
      notice("Task changed elsewhere. Review the current steps before applying your draft.", "warning"); return;
    }
    const parent = intent.kind === "add" ? task.steps.find((step) => step.step_id === intent.parentStepId) : undefined;
    if (parent && stepSubtree(task, parent.step_id!).length === 1 && parent.status === "done") preview(submitted, task);
    else void send(submitted, task);
  }
  return { openText, submitText };
}
