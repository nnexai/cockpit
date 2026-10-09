import type { Task } from "../../protocol/generated/v1";
import type { TaskMutationOutcome } from "./useSupervisor";
import { stepSubtree, leafProgress, type StepIntent, type StepSubmission, type SupervisorStepsProps } from "./stepInteractions";
import { sameScope, type MoveFocusOrigin, type Step, type StepNotice } from "./stepViewTypes";
import type { StepFocus } from "./useStepFocus";

export type StepMutation = {
  identityValid: boolean; unsafe: boolean; uncertain: boolean; pending: boolean; blockedReason: string | null;
  changed: () => void; notice: StepNotice; guard: (reviewed?: boolean) => boolean;
  submission: (intent: StepIntent, revision: string) => StepSubmission;
  finishText: (submitted: StepSubmission, disposition?: "confirmed" | "abandoned") => void;
  acceptOutcome: (outcome: TaskMutationOutcome<StepSubmission>, submitted: StepSubmission, before: Task | null, origin: MoveFocusOrigin | null) => void;
  send: (submitted: StepSubmission, before: Task | null, reviewed?: boolean) => Promise<void>;
  preview: (submitted: StepSubmission, baseTask: Task) => void; activate: (step: Step) => void;
};

export function useStepMutation(props: SupervisorStepsProps, resolving: boolean, focus: StepFocus): StepMutation {
  const { task, draft, scope } = props;
  const uncertain = draft.operation.kind === "unknown" || (draft.operation.kind === "review" && draft.operation.reason === "unknown");
  const pending = props.busy || draft.operation.kind === "sending" || resolving;
  const identityValid = task !== null && task.task_id === scope.taskId;
  // Limit diagnostics are not blanket read-only gates: core may safely check or shrink saved overflow.
  const unsafe = task ? task.steps.some((step) => step.diagnostic !== null) ||
    (task.steps.length === 0 && task.steps_diagnostic !== null) : false;
  const blockedReason = !identityValid ? "This task is unavailable. Your drafts are kept."
    : task?.checked ? "Task is complete; its steps are read-only."
    : !props.writable ? props.readOnlyReason ?? "Steps are read-only."
    : unsafe ? task?.steps_diagnostic ?? "Step identities or source boundaries are unavailable."
    : uncertain ? "Step change unconfirmed. Check saved steps before another change."
    : props.taskWriteUnconfirmed ? "A task source change is unconfirmed. Resolve it in its original editor before changing steps."
    : pending ? "Saving step… Wait before changing another step."
    : draft.operation.kind === "review" || draft.operation.kind === "refused" ? "Review or keep the refused change before another change."
    : null;
  function changed() { props.onDraftChanged(); }
  function notice(text: string, tone: "notice" | "warning" | "error" | "pending" = "notice") {
    draft.status = { tone, text }; changed();
  }
  const { moveFocusOrigin, focusIntent, requestFocus } = focus;
  function guard(reviewed = false): boolean {
    if (!identityValid || !task || task.checked || !props.writable || unsafe || pending ||
      (!reviewed && (uncertain || props.taskWriteUnconfirmed || draft.operation.kind === "review" || draft.operation.kind === "refused"))) {
      notice(blockedReason ?? "This change cannot be sent right now.", "warning"); return false;
    }
    return true;
  }
  function submission(intent: StepIntent, revision: string): StepSubmission {
    return { submissionId: crypto.randomUUID(), scope: { ...scope }, expectedTaskRevision: revision, intent };
  }
  function finishText(submitted: StepSubmission, disposition: "confirmed" | "abandoned" = "confirmed") {
    const intent = submitted.intent;
    const textDraft = intent.kind === "add" ? draft.add.get(intent.parentStepId)
      : intent.kind === "rename" ? draft.rename.get(intent.stepId) : undefined;
    if (!textDraft || textDraft.retainedSubmission?.submissionId !== submitted.submissionId) return;
    if (disposition === "confirmed" && (intent.kind === "add" || intent.kind === "rename") && textDraft.text === intent.title) {
      if (intent.kind === "rename") draft.rename.delete(intent.stepId);
      else { textDraft.text = ""; textDraft.baseTaskRevision = ""; textDraft.retainedSubmission = null; }
    }
    // A later unsent edit keeps its original fence, but never reuses an already saved or explicitly abandoned add UUID.
    textDraft.retainedSubmission = null;
  }
  function acceptOutcome(outcome: TaskMutationOutcome<StepSubmission>, submitted: StepSubmission, before: Task | null, origin: MoveFocusOrigin | null) {
    switch (outcome.kind) {
      case "not_sent":
        draft.operation = { kind: "idle" }; notice(outcome.reason, "warning"); break;
      case "refused":
        draft.operation = { kind: "refused", submitted, operationCode: outcome.operationCode, message: outcome.message };
        notice(`${outcome.message} No change applied.`, "warning"); break;
      case "unknown":
        draft.operation = { kind: "unknown", submitted: outcome.submitted, message: outcome.message };
        notice("Step change unconfirmed. Check saved steps before another change.", "warning"); break;
      case "confirmed":
        if (outcome.task.task_id !== submitted.scope.taskId) {
          draft.operation = { kind: "unknown", submitted, message: "The response did not identify the original task." };
          notice("Step change unconfirmed. Check saved steps before another change.", "warning"); break;
        }
        draft.operation = { kind: "idle" }; draft.confirmation = null;
        finishText(submitted); draft.status = { tone: "notice", text: "Saved steps confirmed." };
        // The parent owns canonical state. This response is used only for one-shot focus and draft cleanup.
        focusIntent(submitted, before, outcome.task, origin); changed(); break;
    }
  }
  async function send(submitted: StepSubmission, before: Task | null, reviewed = false) {
    if (!sameScope(submitted.scope, scope) || !guard(reviewed)) return;
    const origin = submitted.intent.kind === "move" ? moveFocusOrigin() : null;
    draft.operation = { kind: "sending", submitted };
    draft.status = { tone: "pending", text: "Saving step…" }; changed();
    try { acceptOutcome(await props.submit(submitted), submitted, before, origin); }
    catch (error) {
      acceptOutcome({ kind: "unknown", submitted, message: error instanceof Error ? error.message : "The submitted change could not be confirmed." }, submitted, before, origin);
    }
  }
  function preview(submitted: StepSubmission, baseTask: Task) {
    draft.confirmation = { submitted, baseTask }; requestFocus({ kind: "safe_confirmation" });
  }
  function activate(step: Step) {
    if (!task || step.step_id === null || !guard()) return;
    const subtree = stepSubtree(task, step.step_id);
    const intent: StepIntent = { kind: "set_checked", stepId: step.step_id,
      checked: step.status !== "done", scope: subtree.length > 1 ? "subtree" : "leaf" };
    const submitted = submission(intent, task.task_revision);
    if (!intent.checked && leafProgress(subtree).done > 1) preview(submitted, task);
    else void send(submitted, task);
  }
  return { identityValid, unsafe, uncertain, pending, blockedReason, changed, notice, guard, submission,
    finishText, acceptOutcome, send, preview, activate };
}

export function requiresPreview(intent: StepIntent, baseTask: Task): boolean {
    if (intent.kind === "remove") return true;
    if (intent.kind === "set_checked") return !intent.checked && leafProgress(stepSubtree(baseTask, intent.stepId)).done > 1;
    if (intent.kind === "move") {
      const moving = baseTask.steps.find((step) => step.step_id === intent.stepId);
      if (moving && moving.parent_step_id !== null && moving.parent_step_id !== intent.parentStepId &&
        baseTask.steps.filter((step) => step.depth === moving.depth && step.parent_step_id === moving.parent_step_id).length === 1) return true;
    }
    if (intent.kind === "add" || intent.kind === "move") {
      const parent = baseTask.steps.find((step) => step.step_id === intent.parentStepId);
      return Boolean(parent && stepSubtree(baseTask, parent.step_id!).length === 1);
    }
    return false;
}
