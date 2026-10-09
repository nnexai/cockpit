import type { FocusEvent, KeyboardEvent, RefObject } from "react";
import { menuAnchor } from "../library/LibraryTree";
import { useRovingList } from "../sidebar/useRovingList";
import { stepSubtree, type SupervisorStepsProps } from "./stepInteractions";
import type { Step, StepMenuState } from "./stepViewTypes";
import type { StepMutation } from "./useStepMutation";
import type { StepFocus } from "./useStepFocus";

export type StepTree = {
  collapse: (stepId: string) => void; expand: (stepId: string) => void;
  openMenu: (stepId: string, opener: HTMLElement, point?: { x: number; y: number }) => void;
  roving: {
    listRef: RefObject<HTMLDivElement | null>; tabIndexFor: (id: string) => number;
    focusTarget: () => void; focusRow: (id: string | undefined) => void;
    listProps: {
      onFocus: (event: FocusEvent<HTMLElement>) => void; onBlur: (event: FocusEvent<HTMLElement>) => void;
      onKeyDown: (event: KeyboardEvent<HTMLElement>) => void;
    };
  };
};

export function useStepTree(props: SupervisorStepsProps, expanded: Set<string>, trackedRows: Step[],
  setMenu: (menu: StepMenuState | null) => void, mutation: StepMutation, focus: StepFocus): StepTree {
  const { task, draft } = props;
  const { changed, activate } = mutation;
  const { requestFocus } = focus;
  function collapse(stepId: string) {
    if (!task) return;
    if (draft.focusedStepId && stepSubtree(task, stepId).some((step) => step.step_id === draft.focusedStepId)) {
      draft.focusedStepId = stepId; requestFocus({ kind: "row", stepId });
    }
    draft.expanded = new Set(expanded); draft.expanded.delete(stepId); changed();
  }
  function expand(stepId: string) { draft.expanded = new Set(expanded); draft.expanded.add(stepId); changed(); }
  function openMenu(stepId: string, opener: HTMLElement, point?: { x: number; y: number }) {
    if (!task) return;
    opener.focus({ preventScroll: true });
    setMenu({ ...(point ?? menuAnchor(opener)), stepId, task, opener });
  }
  function rowKey(event: KeyboardEvent<HTMLElement>, stepId: string): boolean {
    if (!task) return false;
    const step = task.steps.find((candidate) => candidate.step_id === stepId);
    if (!step) return false;
    if (event.key === "ArrowLeft") {
      if (expanded.has(stepId)) collapse(stepId);
      else if (step.parent_step_id !== null) roving.focusRow(step.parent_step_id);
    } else if (event.key === "ArrowRight") {
      const child = task.steps.find((candidate) => candidate.parent_step_id === stepId && candidate.step_id !== null);
      if (child && !expanded.has(stepId)) expand(stepId); else if (child?.step_id) roving.focusRow(child.step_id);
    } else if (event.key === " " || event.key === "Enter" || event.key === "ContextMenu" || (event.key === "F10" && event.shiftKey)) {
      event.stopPropagation();
      if (!event.repeat && !event.nativeEvent.isComposing) {
        if (event.key === " ") activate(step); else openMenu(stepId, event.target as HTMLElement);
      }
    } else return false;
    event.stopPropagation(); return true;
  }
  const roving = useRovingList({ rowIds: trackedRows.flatMap((step) => step.step_id === null ? [] : [step.step_id]),
    selectedId: draft.focusedStepId, onKey: rowKey, onEscape: () => { props.onCloseDetails(); return true; } });
  return { collapse, expand, openMenu, roving };
}
