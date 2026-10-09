import { useLayoutEffect, useRef, type RefObject } from "react";
import type { Task } from "../../protocol/generated/v1";
import { defaultExpansion, removalFocus, visibleSteps, type StepDraftState, type StepFocusTarget, type StepSubmission, type StepTextDraft, type SupervisorStepsProps } from "./stepInteractions";
import type { MoveFocusOrigin, Step, StepMenuState } from "./stepViewTypes";
import type { StepTree } from "./useStepTree";

type SavedMoveFocus = { draft: StepDraftState; serial: number; taskId: string; revision: string; origin: MoveFocusOrigin };
export type StepFocus = {
  groupRef: RefObject<HTMLElement | null>; addButton: RefObject<HTMLButtonElement | null>; safeButton: RefObject<HTMLButtonElement | null>;
  textFields: RefObject<Map<StepTextDraft, HTMLInputElement>>; consumedFocus: RefObject<WeakMap<StepDraftState, number>>;
  previousRows: RefObject<readonly Step[]>; focusedRowElement: RefObject<HTMLElement | null>;
  mountedDraft: RefObject<StepDraftState>; savedMoveFocus: RefObject<SavedMoveFocus | null>;
  moveFocusOrigin: () => MoveFocusOrigin; moveFocusAllowed: (origin: MoveFocusOrigin) => boolean;
  requestFocus: (target: StepFocusTarget, savedMove?: { task: Task; origin: MoveFocusOrigin }) => void;
  focusIntent: (submitted: StepSubmission, before: Task | null, after: Task, origin: MoveFocusOrigin | null) => void;
};

export function useStepFocus(props: SupervisorStepsProps, expanded: Set<string>, menu: StepMenuState | null): StepFocus {
  const { task, draft } = props;
  const groupRef = useRef<HTMLElement>(null);
  const addButton = useRef<HTMLButtonElement>(null);
  const safeButton = useRef<HTMLButtonElement>(null);
  const textFields = useRef(new Map<StepTextDraft, HTMLInputElement>());
  const consumedFocus = useRef(new WeakMap<typeof draft, number>());
  const previousRows = useRef<readonly Step[]>([]);
  const focusedRowElement = useRef<HTMLElement | null>(null);
  const mountedDraft = useRef(draft);
  const savedMoveFocus = useRef<{ draft: typeof draft; serial: number; taskId: string; revision: string; origin: MoveFocusOrigin } | null>(null);
  function changed() { props.onDraftChanged(); }
  function moveFocusOrigin(): MoveFocusOrigin {
    return { element: document.activeElement, opener: menu?.opener ?? null, focusedStepId: draft.focusedStepId };
  }
  function moveFocusAllowed(origin: MoveFocusOrigin): boolean {
    return draft.focusedStepId === origin.focusedStepId && (document.activeElement === document.body ||
      document.activeElement === origin.element || document.activeElement === origin.opener);
  }
  function requestFocus(target: StepFocusTarget, savedMove?: { task: Task; origin: MoveFocusOrigin }) {
    if (savedMove && mountedDraft.current !== draft) return;
    draft.focusRequest = { serial: (draft.focusRequest?.serial ?? 0) + 1, target };
    savedMoveFocus.current = savedMove ? { draft, serial: draft.focusRequest.serial, taskId: savedMove.task.task_id,
      revision: savedMove.task.task_revision, origin: savedMove.origin } : null;
    changed();
  }
  function focusIntent(submitted: StepSubmission, before: Task | null, after: Task, origin: MoveFocusOrigin | null) {
    const intent = submitted.intent;
    if (intent.kind === "add") {
      if (intent.parentStepId !== null) draft.expanded?.add(intent.parentStepId);
      requestFocus({ kind: "add", parentStepId: intent.parentStepId });
    } else if (intent.kind === "remove") {
      requestFocus(removalFocus(before ? visibleSteps(before, expanded) : previousRows.current,
        visibleSteps(after, draft.expanded ?? expanded), intent.stepId));
    } else {
      if (intent.kind === "move") {
        if (mountedDraft.current !== draft || !origin || !moveFocusAllowed(origin)) return;
        const revealed = new Set(draft.expanded ?? expanded);
        let parentId = after.steps.find((step) => step.step_id === intent.stepId)?.parent_step_id ?? null;
        while (parentId !== null) {
          revealed.add(parentId);
          parentId = after.steps.find((step) => step.step_id === parentId)?.parent_step_id ?? null;
        }
        draft.expanded = revealed;
      }
      requestFocus({ kind: "row", stepId: intent.stepId }, intent.kind === "move" && origin ? { task: after, origin } : undefined);
    }
  }
  return { groupRef, addButton, safeButton, textFields, consumedFocus, previousRows, focusedRowElement, mountedDraft,
    savedMoveFocus, moveFocusOrigin, moveFocusAllowed, requestFocus, focusIntent };
}

export function useStepFocusEffect(props: SupervisorStepsProps, rows: readonly Step[], roving: StepTree["roving"], focus: StepFocus) {
  const { task, draft } = props;
  const { mountedDraft, savedMoveFocus, consumedFocus, previousRows, focusedRowElement, moveFocusAllowed,
    safeButton, addButton, textFields, requestFocus } = focus;
  function changed() { props.onDraftChanged(); }
  useLayoutEffect(() => {
    if (mountedDraft.current !== draft) {
      const old = savedMoveFocus.current;
      if (old) { consumedFocus.current.set(old.draft, old.serial); savedMoveFocus.current = null; }
      mountedDraft.current = draft; previousRows.current = []; focusedRowElement.current = null;
    }
    if (task && draft.expanded === null) { draft.expanded = defaultExpansion(task); changed(); }
    const request = draft.focusRequest;
    const saved = savedMoveFocus.current;
    const savedRequest = saved?.draft === draft && saved.serial === request?.serial ? saved : null;
    if (savedRequest && !moveFocusAllowed(savedRequest.origin)) {
      consumedFocus.current.set(draft, savedRequest.serial); savedMoveFocus.current = null;
    }
    const projectionReady = !savedRequest || (task?.task_id === savedRequest.taskId && task.task_revision === savedRequest.revision);
    if (request && projectionReady && consumedFocus.current.get(draft) !== request.serial) {
      const target = request.target;
      let element: HTMLElement | null | undefined;
      if (target.kind === "row") element = [...(roving.listRef.current?.querySelectorAll<HTMLElement>("[data-row-id]") ?? [])].find((row) => row.dataset.rowId === target.stepId);
      else if (target.kind === "safe_confirmation") element = safeButton.current;
      else if (target.kind === "toolbar_add") element = addButton.current;
      else {
        const textDraft = target.kind === "add" ? draft.add.get(target.parentStepId) : draft.rename.get(target.stepId);
        element = textDraft ? textFields.current.get(textDraft) : null;
      }
      // A move must focus its new rendered row, never consume against the preceding projection.
      if (element) {
        consumedFocus.current.set(draft, request.serial); savedMoveFocus.current = null;
        element.focus({ preventScroll: true });
      }
    }
    const removedElement = focusedRowElement.current;
    const removedId = removedElement?.dataset.rowId;
    if (removedElement && removedId && !removedElement.isConnected && document.activeElement === document.body &&
      !task?.steps.some((step) => step.step_id === removedId)) {
      focusedRowElement.current = null;
      draft.status = { tone: "notice", text: "The focused step was removed. Focus moved to a surviving step." };
      requestFocus(removalFocus(previousRows.current, rows, removedId));
    }
    previousRows.current = rows;
  });
}
