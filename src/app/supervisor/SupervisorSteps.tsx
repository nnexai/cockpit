import { useId, useState, type ReactElement } from "react";
import { UiIcon } from "../UiIcon";
import { defaultExpansion, visibleSteps, type StepTextDraft, type SupervisorStepsProps } from "./stepInteractions";
import type { StepMenuState } from "./stepViewTypes";
import { useStepFocus, useStepFocusEffect } from "./useStepFocus";
import { useStepMutation } from "./useStepMutation";
import { useStepRecovery } from "./useStepRecovery";
import { useStepText } from "./useStepText";
import { useStepTree } from "./useStepTree";
import { StepTextEditor } from "./stepTextEditor";
import { StepRowTree } from "./stepRowTree";
import { StepConfirmation } from "./stepConfirmation";
import { StepRecoveryView } from "./stepRecoveryView";
import { StepMenu } from "./stepMenu";

export type { SupervisorStepsProps } from "./stepInteractions";

export function StepsSection(props: SupervisorStepsProps): ReactElement {
  const { task, draft } = props;
  const id = useId();
  const [menu, setMenu] = useState<StepMenuState | null>(null);
  const [reading, setReading] = useState(false);
  const [resolving, setResolving] = useState(false);
  const expanded = draft.expanded ?? (task ? defaultExpansion(task) : new Set<string>());
  const rows = task ? visibleSteps(task, expanded) : [];
  const trackedRows = rows.filter((step) => step.step_id !== null && step.diagnostic === null);
  const branches = task?.steps.filter((step, index) => step.step_id !== null && task.steps[index + 1]?.depth > step.depth) ?? [];
  const focus = useStepFocus(props, expanded, menu);
  const mutation = useStepMutation(props, resolving, focus);
  const tree = useStepTree(props, expanded, trackedRows, setMenu, mutation, focus);
  useStepFocusEffect(props, rows, tree.roving, focus);
  const recovery = useStepRecovery(props, setReading, setResolving, mutation, focus);
  const text = useStepText(props, mutation, focus, tree.expand);
  const { groupRef, addButton, focusedRowElement, requestFocus } = focus;
  const { blockedReason, notice, changed } = mutation;
  const { openText } = text;
  const { copy } = recovery;
  const { roving } = tree;
  function textEditor(kind: "add" | "rename", key: string | null, textDraft: StepTextDraft): ReactElement {
    return <StepTextEditor key={`${kind}-${key ?? "root"}`} props={props} id={id} kind={kind} draftKey={key}
      textDraft={textDraft} mutation={mutation} focus={focus} text={text} recovery={recovery} />;
  }
  const untracked = task?.steps.filter((step) => step.step_id === null) ?? [];
  const countLimit = (task?.steps.filter((step) => step.step_id !== null).length ?? 0) >= 64;
  const orphanAdds = [...draft.add].filter(([parentId]) => parentId !== null && !rows.some((step) => step.step_id === parentId));
  const orphanRenames = [...draft.rename].filter(([stepId]) => !rows.some((step) => step.step_id === stepId));
  return <section ref={groupRef} className="supervisor-group supervisor-steps" aria-label={`Steps for ${task?.title ?? "unavailable task"}`}>
    <div className="supervisor-steps-header"><h3>Steps</h3><span className="supervisor-step-count">{task?.step_progress ? task.step_progress.total > 0 ? `${task.step_progress.done} of ${task.step_progress.total}` : untracked.length ? "Not tracked" : "No steps" : "Unavailable"}</span></div>
    {task?.step_progress && task.step_progress.total > 0 ? <progress className="supervisor-step-meter" value={task.step_progress.done} max={task.step_progress.total} aria-hidden="true" /> : null}
    <p className="supervisor-step-context">Steps record work; a Result still needs review.</p>
    <p className="supervisor-step-sr-only" id={`${id}-instructions`}>Use arrows to navigate, Space to change a step, Enter to open actions, Escape to close details.</p>
    {blockedReason ? <p className="supervisor-step-readonly">{blockedReason}</p> : null}
    {task?.steps_diagnostic ? <p className="supervisor-step-diagnostic">Steps diagnostic: {task.steps_diagnostic}</p> : null}
    {countLimit ? <p className="supervisor-step-diagnostic">64-step limit reached. Saved rows remain visible; safe shrinking changes are available.</p> : null}
    {task?.steps.some((step) => step.depth > 4) ? <p className="supervisor-step-diagnostic">Saved steps exceed five levels. All saved rows are retained.</p> : null}
    <div className="supervisor-step-toolbar"><button type="button" ref={addButton} aria-disabled={Boolean(blockedReason || countLimit)} onClick={() => {
      if (countLimit) { notice("64-step limit reached. Remove steps before adding.", "warning"); return; }
      if (task) openText("add", null, task); else notice("The original task is unavailable. Your drafts are kept.", "warning");
    }}><UiIcon name="plus" />Add step</button>
      {branches.length ? <button type="button" onClick={() => {
        if (branches.every((step) => step.step_id !== null && expanded.has(step.step_id))) {
          const focused = task?.steps.find((step) => step.step_id === draft.focusedStepId);
          if (focused && focused.depth > 0) {
            const index = task!.steps.indexOf(focused);
            const root = task!.steps.slice(0, index).reverse().find((step) => step.depth === 0 && step.step_id !== null);
            if (root?.step_id) requestFocus({ kind: "row", stepId: root.step_id });
          }
          draft.expanded = new Set();
        } else draft.expanded = new Set(branches.flatMap((step) => step.step_id === null ? [] : [step.step_id]));
        changed();
      }}>{branches.every((step) => step.step_id !== null && expanded.has(step.step_id)) ? "Collapse all" : "Expand all"}</button> : null}
    </div>
    {task && !task.steps.length ? <p>{props.writable && !task.checked ? "No steps yet." : "No steps recorded."}</p> : null}
    <div ref={roving.listRef} aria-describedby={`${id}-instructions`} onFocus={(event) => {
      roving.listProps.onFocus(event); const stepId = (event.target as HTMLElement).dataset.rowId;
      if (stepId) { focusedRowElement.current = event.target as HTMLElement; draft.focusedStepId = stepId; changed(); }
    }} onBlur={roving.listProps.onBlur} onKeyDown={(event) => {
      if (event.nativeEvent.isComposing && (event.target as HTMLElement).dataset.rowId) {
        if (event.key === " " || event.key === "Enter" || event.key === "ContextMenu" || (event.key === "F10" && event.shiftKey)) {
          event.preventDefault(); event.stopPropagation();
        }
        return;
      }
      roving.listProps.onKeyDown(event);
      if (event.defaultPrevented) event.stopPropagation();
    }} onKeyUp={(event) => { if ((event.target as HTMLElement).dataset.rowId && event.key === " ") { event.preventDefault(); event.stopPropagation(); } }}>
      <StepRowTree props={props} id={id} rows={rows} expanded={expanded} mutation={mutation} focus={focus} tree={tree} textEditor={textEditor} />
    </div>
    {untracked.length ? <div className="supervisor-step-untracked-summary"><p>{untracked.length} checklist lines are not tracked.</p></div> : null}
    <div className={`supervisor-step-status${draft.status ? ` is-${draft.status.tone}` : ""}`} role={draft.status?.tone === "error" ? "alert" : "status"} aria-live="polite">{draft.status?.text ?? ""}</div>
    <StepRecoveryView props={props} mutation={mutation} recovery={recovery} reading={reading} resolving={resolving} />
    <StepConfirmation props={props} mutation={mutation} focus={focus} recovery={recovery} />
    {draft.add.has(null) ? textEditor("add", null, draft.add.get(null)!) : null}
    {orphanAdds.map(([key, textDraft]) => textEditor("add", key, textDraft))}
    {orphanRenames.map(([key, textDraft]) => textEditor("rename", key, textDraft))}
    {task ? <details className="supervisor-step-source"><summary>Canonical task source</summary><pre>{task.body}</pre><button type="button" onClick={() => void copy(task.body)}>Copy task source</button></details> : null}
    <StepMenu menu={menu} setMenu={setMenu} mutation={mutation} text={text} />
  </section>;
}
