import type { ReactElement } from "react";
import { UiIcon } from "../UiIcon";
import { leafProgress, stepSubtree, type StepTextDraft, type SupervisorStepsProps } from "./stepInteractions";
import type { Step } from "./stepViewTypes";
import type { StepMutation } from "./useStepMutation";
import type { StepFocus } from "./useStepFocus";
import type { StepTree } from "./useStepTree";

type RowNode = { step: Step; children: RowNode[] };
export type StepRowTreeProps = {
  props: SupervisorStepsProps; id: string; rows: readonly Step[]; expanded: Set<string>;
  mutation: StepMutation; focus: StepFocus; tree: StepTree;
  textEditor: (kind: "add" | "rename", key: string | null, textDraft: StepTextDraft) => ReactElement;
};

export function StepRowTree({ props, id, rows, expanded, mutation, focus, tree, textEditor }: StepRowTreeProps): ReactElement {
  const { task, draft } = props;
  const { uncertain, blockedReason, activate, notice } = mutation;
  const { requestFocus } = focus;
  const { roving, openMenu, collapse, expand } = tree;
  // Nest using the server's preorder/depth only. Untracked rows never receive fabricated roving identities.
  const roots: RowNode[] = [];
  const stack: RowNode[] = [];
  for (const step of rows) {
    const node: RowNode = { step, children: [] };
    while (stack.length && stack[stack.length - 1].step.depth >= step.depth) stack.pop();
    if (stack.length) stack[stack.length - 1].children.push(node); else roots.push(node);
    stack.push(node);
  }
  function renderRows(nodes: RowNode[], listId?: string): ReactElement {
    return <ul id={listId} className="supervisor-step-list">{nodes.map(({ step, children }) => {
      const stepId = step.step_id;
      const branch = task ? (stepId !== null ? stepSubtree(task, stepId).length > 1 : task.steps[task.steps.indexOf(step) + 1]?.depth > step.depth) : false;
      const subtree = task && stepId !== null ? stepSubtree(task, stepId) : [];
      const progress = leafProgress(subtree);
      const descriptionId = `${id}-description-${stepId ?? step.source_offset}`;
      const childId = `${id}-children-${stepId ?? step.source_offset}`;
      const ancestry: string[] = [];
      let parentId = step.parent_step_id;
      const visited = new Set<string>();
      while (task && parentId !== null && !visited.has(parentId)) {
        visited.add(parentId);
        const parent = task.steps.find((candidate) => candidate.step_id === parentId);
        if (!parent) break;
        ancestry.unshift(parent.title); parentId = parent.parent_step_id;
      }
      const operation = draft.operation;
      const unconfirmed = uncertain && operation.kind !== "idle" && operation.submitted.intent.stepId === stepId;
      return <li key={stepId !== null && step.diagnostic === null ? stepId : `source-${step.source_offset}`} className="supervisor-step-item" data-depth={step.depth}>
        <div className={`supervisor-step-row${unconfirmed ? " is-unconfirmed" : ""}`} onContextMenu={(event) => {
          if (stepId === null) return;
          const opener = event.currentTarget.querySelector<HTMLInputElement>('input[type="checkbox"]');
          if (opener) { event.preventDefault(); event.stopPropagation(); openMenu(stepId, opener, { x: event.clientX, y: event.clientY }); }
        }}>
          {branch && stepId !== null ? <button type="button" tabIndex={-1} className="supervisor-step-disclosure" aria-expanded={expanded.has(stepId)} aria-controls={childId}
            aria-label={`${expanded.has(stepId) ? "Collapse" : "Expand"} ${step.title}`} onClick={() => { expanded.has(stepId) ? collapse(stepId) : expand(stepId); requestFocus({ kind: "row", stepId }); }}><UiIcon name={expanded.has(stepId) ? "down" : "right"} /></button>
            : <span className="supervisor-step-disclosure is-empty" aria-hidden="true" />}
          <span className="supervisor-step-check-target"><input type="checkbox" checked={step.status === "done"}
            ref={(element) => { if (element) { element.checked = step.status === "done"; element.indeterminate = step.status === "partial"; } }}
            aria-label={step.title} aria-describedby={descriptionId} aria-disabled={stepId === null || blockedReason !== null}
            tabIndex={stepId === null || step.diagnostic !== null ? -1 : roving.tabIndexFor(stepId)} data-row-id={stepId !== null && step.diagnostic === null ? stepId : undefined}
            onClick={(event) => {
              if (stepId === null || blockedReason !== null) { event.preventDefault(); if (stepId !== null) notice(blockedReason ?? "This checklist line is not tracked.", "warning"); }
              const element = event.currentTarget;
              queueMicrotask(() => { if (element.isConnected) { element.checked = step.status === "done"; element.indeterminate = step.status === "partial"; } });
            }} onChange={(event) => { event.currentTarget.checked = step.status === "done"; event.currentTarget.indeterminate = step.status === "partial"; activate(step); }}
            onContextMenu={(event) => { if (stepId !== null) { event.preventDefault(); event.stopPropagation(); openMenu(stepId, event.currentTarget, { x: event.clientX, y: event.clientY }); } }} /></span>
          <span className="supervisor-step-title">{step.title}{unconfirmed ? <span className="supervisor-step-unconfirmed">Unconfirmed</span> : null}</span>
          {branch && stepId !== null ? <span className="supervisor-step-count" aria-hidden="true">{task?.step_progress ? `${progress.done}/${progress.total}` : "Unavailable"}</span> : null}
          {stepId === null ? <span className="supervisor-step-untracked">Not tracked</span> : <button type="button" tabIndex={-1} className="supervisor-step-more" aria-label={`Actions for ${step.title}`} onClick={(event) => openMenu(stepId, event.currentTarget)}><UiIcon name="more" /></button>}
          <span id={descriptionId} className="supervisor-step-sr-only">{ancestry.length ? `Under ${ancestry.join(" / ")}. ` : "At task root. "}{branch && task?.step_progress ? `${progress.done} of ${progress.total} leaf steps complete. Activating marks all ${progress.total} steps ${step.status === "done" ? "open" : "complete"}.` : branch ? "Leaf progress unavailable." : "Space changes this step; Enter opens actions."}{stepId === null ? " Not tracked; read-only." : ""}</span>
        </div>
        {step.diagnostic ? <p className="supervisor-step-diagnostic">{step.diagnostic}</p> : null}
        {stepId !== null && draft.rename.has(stepId) ? textEditor("rename", stepId, draft.rename.get(stepId)!) : null}
        {children.length ? renderRows(children, childId) : branch ? <ul id={childId} className="supervisor-step-list" hidden /> : null}
        {stepId !== null && draft.add.has(stepId) ? textEditor("add", stepId, draft.add.get(stepId)!) : null}
      </li>;
    })}</ul>;
  }
  return renderRows(roots);
}
