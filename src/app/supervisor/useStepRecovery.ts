import { useRef } from "react";
import type { StepSubmission, SupervisorStepsProps } from "./stepInteractions";
import { sameScope, reviewedTargetProblem } from "./stepViewTypes";
import type { StepFocus } from "./useStepFocus";
import { requiresPreview, type StepMutation } from "./useStepMutation";

export type StepRecovery = {
  copy: (text: string) => Promise<void>; readForReview: (updatePreview?: boolean) => Promise<void>;
  resolve: (decision: "use_saved" | "keep_saved" | "apply_reviewed", previewConfirmed?: boolean) => Promise<void>;
  retainReviewedText: (original: StepSubmission, submitted: StepSubmission) => void;
  cancelConfirmation: () => void; confirm: () => void; renewPreview: () => void;
};

export function useStepRecovery(props: SupervisorStepsProps, setReading: (reading: boolean) => void, setResolving: (resolving: boolean) => void,
  mutation: StepMutation, focus: StepFocus): StepRecovery {
  const { task, draft, scope } = props;
  const recoveryInFlight = useRef(false);
  const { notice, guard, submission, preview, finishText, send, acceptOutcome } = mutation;
  const { moveFocusOrigin, requestFocus } = focus;
  async function copy(text: string) {
    try { await navigator.clipboard.writeText(text); notice("Copied for recovery."); }
    catch { notice("Copy was unavailable. Select and copy the displayed source or draft.", "warning"); }
  }
  async function readForReview(updatePreview = false) {
    const operation = draft.operation;
    if (operation.kind !== "unknown" && operation.kind !== "refused" && operation.kind !== "review") return;
    if (recoveryInFlight.current) { notice("Wait for the current saved-state review.", "warning"); return; }
    const original = operation.submitted;
    recoveryInFlight.current = true;
    setReading(true);
    try {
      const result = await props.readSaved(original.scope);
      if (draft.operation.kind === "idle" || draft.operation.submitted !== original) return;
      if (result.kind === "found" && result.task.task_id === original.scope.taskId) {
        draft.operation = { kind: "review", submitted: original, currentTask: result.task,
          reason: operation.kind === "unknown" || (operation.kind === "review" && operation.reason === "unknown") ? "unknown" : "conflict" };
        notice("Saved steps read. Compare the original change before choosing what to keep.", "warning");
        if (updatePreview) preview(submission(original.intent, result.task.task_revision), result.task);
      } else notice(result.kind === "unavailable" ? result.message : "The original task was not found. Your operation and drafts are kept.", "warning");
    } catch (error) { notice(error instanceof Error ? error.message : "Saved steps could not be read.", "warning"); }
    finally { recoveryInFlight.current = false; setReading(false); }
  }
  async function resolve(decision: "use_saved" | "keep_saved" | "apply_reviewed", previewConfirmed = false) {
    const operation = draft.operation;
    if (operation.kind !== "review") return;
    if (recoveryInFlight.current) { notice("Wait for the current saved-state review.", "warning"); return; }
    const original = operation.submitted;
    if (!sameScope(original.scope, scope)) { notice("Return to the original task to resolve this change.", "warning"); return; }
    const reviewed = operation.currentTask;
    if (decision === "apply_reviewed") {
      const problem = reviewedTargetProblem(reviewed, original.intent);
      if (problem) { notice(problem, "warning"); return; }
      if (!guard(true)) return;
    }
    const submitted = decision === "apply_reviewed"
      ? previewConfirmed && draft.confirmation
        ? draft.confirmation.submitted
        : submission(original.intent, reviewed.task_revision)
      : null;
    if (operation.reason === "conflict") {
      if (!submitted) { draft.operation = { kind: "idle" }; draft.confirmation = null; finishText(original, "abandoned"); notice("Current saved steps kept. Your text drafts remain available."); return; }
      retainReviewedText(original, submitted);
      if (requiresPreview(submitted.intent, reviewed)) { draft.operation = { kind: "idle" }; preview(submitted, reviewed); }
      else await send(submitted, reviewed, true);
      return;
    }
    if (submitted && requiresPreview(submitted.intent, reviewed) && !previewConfirmed) {
      preview(submitted, reviewed); return;
    }
    recoveryInFlight.current = true;
    const origin = submitted?.intent.kind === "move" ? moveFocusOrigin() : null;
    setResolving(true);
    try {
      const result = await props.resolveUnknown({ originalScope: original.scope, originalSubmissionId: original.submissionId,
        originalSubmitted: original, reviewedTaskRevision: reviewed.task_revision,
        decision: submitted ? { kind: "apply_reviewed", submitted } : { kind: decision === "use_saved" ? "use_saved" : "keep_saved" } });
      if (!sameScope(result.originalScope, original.scope) || result.originalSubmissionId !== original.submissionId ||
        draft.operation.kind !== "review" || draft.operation.submitted !== original) return;
      if (result.kind === "not_resolved") { notice(result.message, "warning"); return; }
      if (result.kind === "resolved") {
        if (result.task.task_id !== original.scope.taskId) { notice("Resolution did not identify the original task.", "warning"); return; }
        draft.operation = { kind: "idle" }; draft.confirmation = null;
        finishText(original, decision === "use_saved" ? "confirmed" : "abandoned");
        notice("Saved state kept. This does not establish who made the change.");
      } else {
        retainReviewedText(original, result.submitted);
        acceptOutcome(result.outcome, result.submitted, reviewed, origin);
      }
    } catch (error) { notice(error instanceof Error ? error.message : "The original change remains unconfirmed.", "warning"); }
    finally { recoveryInFlight.current = false; setResolving(false); }
  }
  function retainReviewedText(original: StepSubmission, submitted: StepSubmission) {
    const intent = original.intent;
    const textDraft = intent.kind === "add" ? draft.add.get(intent.parentStepId) : intent.kind === "rename" ? draft.rename.get(intent.stepId) : undefined;
    if (textDraft?.retainedSubmission?.submissionId === original.submissionId) textDraft.retainedSubmission = submitted;
  }
  function cancelConfirmation() {
    const intent = draft.confirmation?.submitted.intent;
    draft.confirmation = null;
    if (intent?.kind === "add") requestFocus({ kind: "add", parentStepId: intent.parentStepId });
    else if (intent) requestFocus({ kind: "row", stepId: intent.stepId });
    else requestFocus({ kind: "toolbar_add" });
  }
  function confirm() {
    const confirmation = draft.confirmation;
    const unknownReview = draft.operation.kind === "review" && draft.operation.reason === "unknown";
    if (!confirmation || !task || !guard(unknownReview)) return;
    if (task.task_revision !== confirmation.baseTask.task_revision) { notice("Task changed. Review the updated preview and confirm again.", "warning"); return; }
    if (unknownReview) void resolve("apply_reviewed", true);
    else void send(confirmation.submitted, confirmation.baseTask);
  }
  function renewPreview() {
    const confirmation = draft.confirmation;
    if (!confirmation || !task) return;
    if (draft.operation.kind === "review" && draft.operation.reason === "unknown") {
      void readForReview(true); return;
    }
    if (!guard()) return;
    const submitted = submission(confirmation.submitted.intent, task.task_revision);
    retainReviewedText(confirmation.submitted, submitted); preview(submitted, task);
  }
  return { copy, readForReview, resolve, retainReviewedText, cancelConfirmation, confirm, renewPreview };
}
