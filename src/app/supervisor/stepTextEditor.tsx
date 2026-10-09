import type { ReactElement } from "react";
import { titleProblem, type StepTextDraft, type SupervisorStepsProps } from "./stepInteractions";
import type { StepMutation } from "./useStepMutation";
import type { StepFocus } from "./useStepFocus";
import type { StepText } from "./useStepText";
import type { StepRecovery } from "./useStepRecovery";

export type StepTextEditorProps = {
  props: SupervisorStepsProps; id: string; kind: "add" | "rename"; draftKey: string | null; textDraft: StepTextDraft;
  mutation: StepMutation; focus: StepFocus;
  text: StepText; recovery: StepRecovery;
};

export function StepTextEditor({ props, id, kind, draftKey: key, textDraft, mutation, focus, text, recovery }: StepTextEditorProps): ReactElement {
  const { task, draft } = props;
  const { identityValid, blockedReason, notice, changed } = mutation;
  const { requestFocus, textFields } = focus;
  const { submitText } = text;
  const { copy } = recovery;
    const targetExists = identityValid && (key === null || task?.steps.some((step) => step.step_id === key));
    const parent = task?.steps.find((step) => step.step_id === (kind === "add" ? key : textDraft.baseParentStepId));
    const problem = titleProblem(textDraft.text, kind === "rename" ? textDraft.baseTitle : null);
    const fieldId = `${id}-${kind}-${key ?? "root"}`;
    function keep() { textDraft.open = false; notice("Draft kept · not saved."); requestFocus(kind === "rename" && key !== null ? { kind: "row", stepId: key } : { kind: "toolbar_add" }); }
    function discard() {
      if (kind === "add") draft.add.delete(key); else if (key !== null) draft.rename.delete(key);
      requestFocus(kind === "rename" && key !== null && targetExists ? { kind: "row", stepId: key } : { kind: "toolbar_add" });
    }
    return <div key={`${kind}-${key ?? "root"}`} className="supervisor-step-composer" data-editor-kind={kind}>
      {!targetExists ? <p>Step or task was removed elsewhere. Your draft is kept.</p> : null}
      {!textDraft.open && targetExists ? <button type="button" onClick={() => { textDraft.open = true; requestFocus(kind === "add" ? { kind: "add", parentStepId: key } : { kind: "rename", stepId: key! }); }}>Resume {kind === "add" ? "add" : "title"} draft</button> : <form onSubmit={(event) => { event.preventDefault(); if (targetExists) submitText(kind, key, textDraft); }} onKeyDown={(event) => {
        if (event.key === "Escape") { event.preventDefault(); event.stopPropagation(); keep(); }
        else if (event.key === "Enter") { event.stopPropagation(); if (event.repeat || event.nativeEvent.isComposing) event.preventDefault(); }
      }}>
        <label htmlFor={fieldId}>Step title</label>
        <input id={fieldId} ref={(element) => { if (element) textFields.current.set(textDraft, element); else textFields.current.delete(textDraft); }} value={textDraft.text}
          onChange={(event) => {
            if (!textDraft.baseTaskRevision && task) textDraft.baseTaskRevision = task.task_revision;
            textDraft.text = event.target.value; changed();
          }} aria-invalid={problem !== null}
          aria-describedby={`${fieldId}-context ${fieldId}-validation`} />
        <p id={`${fieldId}-context`} className="supervisor-step-context">{parent ? `Under: ${parent.title}` : "At task root"}</p>
        <p id={`${fieldId}-validation`} className="supervisor-step-validation">{problem}</p>
        {textDraft.baseTaskRevision && task && textDraft.baseTaskRevision !== task.task_revision ? <p className="supervisor-step-warning">Task changed since this draft began. Submission requires explicit review.</p> : null}
        <div className="supervisor-step-actions">
          {targetExists ? <><button type="submit" aria-disabled={Boolean(blockedReason || problem)}>{kind === "add" ? "Add" : "Save title"}</button><button type="button" onClick={keep}>Keep draft</button></> : null}
        </div>
      </form>}
      <div className="supervisor-step-actions"><button type="button" onClick={() => void copy(textDraft.text)}>Copy draft</button><button type="button" onClick={discard}>Discard draft</button></div>
    </div>;
}
