import type { ReactElement } from "react";
import { leafProgress, stepSubtree, type SupervisorStepsProps } from "./stepInteractions";
import type { StepMutation } from "./useStepMutation";
import type { StepFocus } from "./useStepFocus";
import type { StepRecovery } from "./useStepRecovery";

export function StepConfirmation({ props, mutation, focus, recovery }: {
  props: SupervisorStepsProps; mutation: StepMutation; focus: StepFocus; recovery: StepRecovery;
}): ReactElement | null {
  const { task, draft } = props;
  const { pending, identityValid, unsafe } = mutation;
  const { safeButton } = focus;
  const { cancelConfirmation, renewPreview, confirm, copy } = recovery;
    const confirmation = draft.confirmation;
    if (!confirmation) return null;
    const { submitted, baseTask } = confirmation;
    const intent = submitted.intent;
    const subtree = intent.kind === "remove" || intent.kind === "set_checked" || intent.kind === "move" ? stepSubtree(baseTask, intent.stepId) : [];
    const selected = subtree[0];
    const parent = selected ? baseTask.steps.find((step) => step.step_id === selected.parent_step_id) : undefined;
    const siblings = selected ? baseTask.steps.filter((step) => step.depth === selected.depth && step.parent_step_id === selected.parent_step_id) : [];
    const newParent = intent.kind === "add" || intent.kind === "move" ? baseTask.steps.find((step) => step.step_id === intent.parentStepId) : undefined;
    const stale = !task || task.task_revision !== baseTask.task_revision;
    const count = leafProgress(subtree).done;
    const heading = intent.kind === "remove" ? `Remove “${selected?.title ?? intent.stepId}”${subtree.length > 1 ? ` and its ${subtree.length - 1} sub-steps` : ""}?`
      : intent.kind === "set_checked" ? `Mark ${count} completed steps open in “${selected?.title ?? intent.stepId}”?`
      : intent.kind === "move" ? `Move “${selected?.title ?? intent.stepId}” and its complete subtree?`
      : `Make “${newParent?.title ?? "the destination"}” a branch?`;
    const action = intent.kind === "remove" ? `Remove ${subtree.length} steps` : intent.kind === "set_checked" ? `Mark ${count} steps open` : "Apply change";
    return <div className="supervisor-step-confirmation" onKeyDown={(event) => { if (event.key === "Escape") { event.preventDefault(); event.stopPropagation(); cancelConfirmation(); } }}>
      <h4>{heading}</h4>
      <p>{intent.kind === "remove" ? "This removes their checklist text from this task. It does not stop the worker or remove files."
        : "This changes steps only; the task and Result remain unchanged."}</p>
      {(intent.kind === "remove" || (intent.kind === "move" && intent.parentStepId !== selected?.parent_step_id)) && parent && siblings.length === 1 ? <p>“{parent.title}” becomes a leaf, {parent.status === "done" ? "complete" : "open"}.</p> : null}
      {newParent && stepSubtree(baseTask, newParent.step_id!).length === 1 ? <p>“{newParent.title}” becomes a branch. {intent.kind === "add" ? "Its new child starts open, even when the parent was complete." : "Completion is derived from its new descendant leaves."}</p> : null}
      {stale ? <p className="supervisor-step-warning">Task changed since this preview. Review a new preview before confirming.</p> : null}
      {intent.kind === "remove" ? <><button type="button" onClick={() => void copy(baseTask.body)}>Copy step Markdown</button><p className="supervisor-step-context">Copies the complete saved task continuation so original step source is available for recovery.</p></> : null}
      <div className="supervisor-step-actions"><button type="button" ref={safeButton} onClick={cancelConfirmation}>{intent.kind === "set_checked" ? "Keep completed" : "Keep steps"}</button>
        {stale ? <button type="button" aria-disabled={pending || !props.writable || !identityValid || unsafe} onClick={renewPreview}>Review updated preview</button> : <button type="button" aria-disabled={pending || !props.writable || !identityValid || unsafe} onClick={confirm}>{action}</button>}
      </div>
    </div>;
}
