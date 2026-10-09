import type { ReactElement } from "react";
import { intentMatchesSaved, type SupervisorStepsProps } from "./stepInteractions";
import { intentText, reviewedTargetProblem } from "./stepViewTypes";
import type { StepMutation } from "./useStepMutation";
import type { StepRecovery } from "./useStepRecovery";

export function StepRecoveryView({ props, mutation, recovery, reading, resolving }: {
  props: SupervisorStepsProps; mutation: StepMutation; recovery: StepRecovery; reading: boolean; resolving: boolean;
}): ReactElement {
  const { draft } = props;
  const { pending, identityValid, notice } = mutation;
  const { readForReview, resolve } = recovery;
  const operation = draft.operation;
  const reviewProblem = operation.kind === "review" ? reviewedTargetProblem(operation.currentTask, operation.submitted.intent) : null;
  return <>
    {operation.kind === "unknown" || operation.kind === "refused" ? <div className="supervisor-step-recovery"><p>{operation.message}</p>{operation.kind === "refused" ? <code>{operation.operationCode}</code> : null}<button type="button" aria-disabled={reading} onClick={() => void readForReview()}>{reading ? "Reading saved steps…" : operation.kind === "unknown" ? "Check saved steps" : "Review current steps"}</button>
      {operation.kind === "refused" ? <button type="button" onClick={() => { draft.operation = { kind: "idle" }; draft.confirmation = null; notice("Saved steps kept. Your drafts remain available."); }}>Keep saved steps</button> : null}</div> : null}
    {operation.kind === "review" ? <div className="supervisor-step-review"><h4>{operation.reason === "unknown" ? "Compare saved steps" : "Review your change against saved steps"}</h4>
      <div className="supervisor-step-compare"><div><strong>Submitted intent</strong><p>{intentText(operation.submitted.intent)}</p><code>{operation.submitted.expectedTaskRevision}</code></div>
        <div><strong>Saved state after read</strong><p>{intentMatchesSaved(operation.currentTask, operation.submitted.intent) ? "The exact intended saved state is present. This does not establish who wrote it." : "Saved steps differ from the submitted intent."}</p>
          <ul>{operation.currentTask.steps.map((step) => <li key={step.step_id !== null && step.diagnostic === null ? step.step_id : `source-${step.source_offset}`} data-depth={step.depth}>{step.title} · {step.status} · under {step.parent_step_id ?? "task root"}</li>)}</ul><code>{operation.currentTask.task_revision}</code></div></div>
      {reviewProblem ? <p className="supervisor-step-warning">{reviewProblem}</p> : null}
      <div className="supervisor-step-actions"><button type="button" aria-disabled={resolving} onClick={() => void resolve(operation.reason === "unknown" && intentMatchesSaved(operation.currentTask, operation.submitted.intent) ? "use_saved" : "keep_saved")}>{operation.reason === "conflict" ? "Use current" : intentMatchesSaved(operation.currentTask, operation.submitted.intent) ? "Use saved state" : "Keep saved state"}</button>
        <button type="button" aria-disabled={pending || !props.writable || !identityValid || reviewProblem !== null} onClick={() => void resolve("apply_reviewed")}>Apply reviewed change</button><button type="button" aria-disabled={reading} onClick={() => void readForReview()}>Read saved steps again</button></div>
    </div> : null}
  </>;
}
