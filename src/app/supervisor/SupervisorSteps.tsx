import { useId, useLayoutEffect, useRef, useState, type KeyboardEvent, type ReactElement } from "react";
import type { Task } from "../../protocol/generated/v1";
import { LibraryMenu, menuAnchor, type LibraryMenuEntry } from "../library/LibraryTree";
import { useRovingList } from "../sidebar/useRovingList";
import { UiIcon } from "../UiIcon";
import type { TaskMutationOutcome } from "./useSupervisor";
import { defaultExpansion, intentMatchesSaved, leafProgress, moveIntent, removalFocus, stepSubtree, titleProblem, visibleSteps,
  type StepFocusTarget, type StepIntent, type StepSubmission, type StepTextDraft, type SupervisorStepsProps } from "./stepInteractions";

export type { SupervisorStepsProps } from "./stepInteractions";
type Step = Task["steps"][number];
type MenuState = { x: number; y: number; stepId: string; task: Task; opener: HTMLElement };
type MoveFocusOrigin = { element: Element | null; opener: HTMLElement | null; focusedStepId: string | null };

function sameScope(left: StepSubmission["scope"], right: StepSubmission["scope"]): boolean {
  return left.sessionId === right.sessionId && left.rootId === right.rootId && left.taskId === right.taskId;
}

function intentText(intent: StepIntent): string {
  switch (intent.kind) {
    case "add": return `Add “${intent.title}” (${intent.stepId}) under ${intent.parentStepId ?? "task root"}.`;
    case "rename": return `Rename ${intent.stepId} to “${intent.title}”.`;
    case "set_checked": return `Mark ${intent.scope === "subtree" ? "subtree" : "step"} ${intent.stepId} ${intent.checked ? "done" : "open"}.`;
    case "move": return `Move ${intent.stepId} under ${intent.parentStepId ?? "task root"}, before ${intent.beforeStepId ?? "end"}.`;
    case "remove": return `Remove the complete subtree ${intent.stepId}.`;
    case "adopt": return `Track all ${intent.mapping.length} checklist lines with the retained proposed identities.`;
  }
}

function reviewedTargetProblem(task: Task, intent: StepIntent): string | null {
  if (task.checked) return "The saved task is complete; its steps are read-only.";
  if (task.step_progress === null || task.steps.some((step) => step.diagnostic !== null) || (task.steps.length === 0 && task.steps_diagnostic !== null)) {
    return "The saved step source is unsafe. Keep the saved state and inspect its diagnostic.";
  }
  if (intent.kind !== "add" && intent.kind !== "adopt" && !task.steps.some((step) => step.step_id === intent.stepId)) {
    return "The original step was removed. Your draft is kept; it cannot be saved to another target.";
  }
  if ((intent.kind === "add" || intent.kind === "move") && intent.parentStepId !== null &&
    !task.steps.some((step) => step.step_id === intent.parentStepId)) {
    return "The original destination was removed. Keep or copy your draft.";
  }
  return null;
}

export function StepsSection(props: SupervisorStepsProps): ReactElement {
  const { task, draft, scope } = props;
  const id = useId();
  const groupRef = useRef<HTMLElement>(null);
  const addButton = useRef<HTMLButtonElement>(null);
  const safeButton = useRef<HTMLButtonElement>(null);
  const textFields = useRef(new Map<StepTextDraft, HTMLInputElement>());
  const consumedFocus = useRef(new WeakMap<typeof draft, number>());
  const previousRows = useRef<readonly Step[]>([]);
  const focusedRowElement = useRef<HTMLElement | null>(null);
  const mountedDraft = useRef(draft);
  const savedMoveFocus = useRef<{ draft: typeof draft; serial: number; taskId: string; revision: string; origin: MoveFocusOrigin } | null>(null);
  const recoveryInFlight = useRef(false);
  const [menu, setMenu] = useState<MenuState | null>(null);
  const [reading, setReading] = useState(false);
  const [resolving, setResolving] = useState(false);
  const expanded = draft.expanded ?? (task ? defaultExpansion(task) : new Set<string>());
  const rows = task ? visibleSteps(task, expanded) : [];
  const trackedRows = rows.filter((step) => step.step_id !== null && step.diagnostic === null);
  const branches = task?.steps.filter((step, index) => step.step_id !== null && task.steps[index + 1]?.depth > step.depth) ?? [];
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
  function focusIntent(submitted: StepSubmission, before: Task | null, after: Task, origin: MoveFocusOrigin | null) {
    const intent = submitted.intent;
    if (intent.kind === "add") {
      if (intent.parentStepId !== null) draft.expanded?.add(intent.parentStepId);
      requestFocus({ kind: "add", parentStepId: intent.parentStepId });
    } else if (intent.kind === "remove") {
      requestFocus(removalFocus(before ? visibleSteps(before, expanded) : previousRows.current,
        visibleSteps(after, draft.expanded ?? expanded), intent.stepId));
    } else if (intent.kind !== "adopt") {
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
  function collapse(stepId: string) {
    if (!task) return;
    if (draft.focusedStepId && stepSubtree(task, stepId).some((step) => step.step_id === draft.focusedStepId)) {
      draft.focusedStepId = stepId; requestFocus({ kind: "row", stepId });
    }
    draft.expanded = new Set(expanded); draft.expanded.delete(stepId); changed();
  }
  function expand(stepId: string) { draft.expanded = new Set(expanded); draft.expanded.add(stepId); changed(); }
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
  function requiresPreview(intent: StepIntent, baseTask: Task): boolean {
    if (intent.kind === "remove" || intent.kind === "adopt") return true;
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
  function cancelConfirmation() {
    const intent = draft.confirmation?.submitted.intent;
    draft.confirmation = null;
    if (intent?.kind === "add") requestFocus({ kind: "add", parentStepId: intent.parentStepId });
    else if (intent && intent.kind !== "adopt") requestFocus({ kind: "row", stepId: intent.stepId });
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

  function textEditor(kind: "add" | "rename", key: string | null, textDraft: StepTextDraft): ReactElement {
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

  const menuEntries: LibraryMenuEntry[] = menu ? (() => {
    const selected = menu.task.steps.find((step) => step.step_id === menu.stepId);
    const disabled = blockedReason !== null;
    function move(direction: "up" | "down" | "indent" | "outdent") {
      const intent = moveIntent(menu!.task, menu!.stepId, direction);
      if (!intent || !guard()) return;
      const submitted = submission(intent, menu!.task.task_revision);
      if (requiresPreview(intent, menu!.task)) preview(submitted, menu!.task); else void send(submitted, menu!.task);
    }
    return [
      { label: "Edit title…", disabled, onSelect: () => openText("rename", menu.stepId, menu.task) },
      { label: "Add sub-step", disabled: disabled || !selected || selected.depth >= 4 || menu.task.steps.filter((step) => step.step_id !== null).length >= 64, onSelect: () => openText("add", menu.stepId, menu.task) },
      "separator",
      ...(["up", "down", "indent", "outdent"] as const).map((direction) => ({
        label: { up: "Move up", down: "Move down", indent: "Indent under previous step", outdent: "Outdent" }[direction],
        disabled: disabled || moveIntent(menu.task, menu.stepId, direction) === null, onSelect: () => move(direction),
      })),
      "separator", { label: "Remove…", destructive: true, disabled, onSelect: () => {
        if (guard()) preview(submission({ kind: "remove", stepId: menu.stepId }, menu.task.task_revision), menu.task);
      } },
    ];
  })() : [];

  // Nest using the server's preorder/depth only. Untracked rows never receive fabricated roving identities.
  type RowNode = { step: Step; children: RowNode[] };
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
      const unconfirmed = uncertain && operation.kind !== "idle" && (operation.submitted.intent.kind === "adopt"
        ? operation.submitted.intent.mapping.some((mapping) => mapping.stepId === stepId || mapping.sourceOffset === step.source_offset)
        : operation.submitted.intent.stepId === stepId);
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

  function confirmationView(): ReactElement | null {
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
      : intent.kind === "adopt" ? `Track all ${intent.mapping.length} checklist lines?`
      : intent.kind === "move" ? `Move “${selected?.title ?? intent.stepId}” and its complete subtree?`
      : `Make “${newParent?.title ?? "the destination"}” a branch?`;
    const action = intent.kind === "remove" ? `Remove ${subtree.length} steps` : intent.kind === "set_checked" ? `Mark ${count} steps open` : intent.kind === "adopt" ? "Track checklist" : "Apply change";
    return <div className="supervisor-step-confirmation" onKeyDown={(event) => { if (event.key === "Escape") { event.preventDefault(); event.stopPropagation(); cancelConfirmation(); } }}>
      <h4>{heading}</h4>
      <p>{intent.kind === "remove" ? "This removes their checklist text from this task. It does not stop the worker or remove files."
        : intent.kind === "adopt" ? "Identity markers are inserted in place. Review every line; state, order and unrelated prose are preserved." : "This changes steps only; the task and Result remain unchanged."}</p>
      {(intent.kind === "remove" || (intent.kind === "move" && intent.parentStepId !== selected?.parent_step_id)) && parent && siblings.length === 1 ? <p>“{parent.title}” becomes a leaf, {parent.status === "done" ? "complete" : "open"}.</p> : null}
      {newParent && stepSubtree(baseTask, newParent.step_id!).length === 1 ? <p>“{newParent.title}” becomes a branch. {intent.kind === "add" ? "Its new child starts open, even when the parent was complete." : "Completion is derived from its new descendant leaves."}</p> : null}
      {intent.kind === "adopt" ? <ul className="supervisor-step-adoption-preview">{baseTask.steps.filter((step) => step.step_id === null).map((step) => <li key={step.source_offset} data-depth={step.depth}>{step.title} · {step.status} · line {step.line}<code>{intent.mapping.find((mapping) => mapping.sourceOffset === step.source_offset)?.stepId}</code></li>)}</ul> : null}
      {stale ? <p className="supervisor-step-warning">Task changed since this preview. Review a new preview before confirming.</p> : null}
      {intent.kind === "remove" ? <><button type="button" onClick={() => void copy(baseTask.body)}>Copy step Markdown</button><p className="supervisor-step-context">Copies the complete saved task continuation so original step source is available for recovery.</p></> : null}
      <div className="supervisor-step-actions"><button type="button" ref={safeButton} onClick={cancelConfirmation}>{intent.kind === "set_checked" ? "Keep completed" : "Keep steps"}</button>
        {stale ? <button type="button" aria-disabled={pending || !props.writable || !identityValid || unsafe} onClick={renewPreview}>Review updated preview</button> : <button type="button" aria-disabled={pending || !props.writable || !identityValid || unsafe} onClick={confirm}>{action}</button>}
      </div>
    </div>;
  }

  const operation = draft.operation;
  const reviewProblem = operation.kind === "review" ? reviewedTargetProblem(operation.currentTask, operation.submitted.intent) : null;
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
      {renderRows(roots)}
    </div>
    {untracked.length ? <div className="supervisor-step-untracked-summary"><p>{untracked.length} checklist lines are not tracked.</p><button type="button" aria-disabled={Boolean(blockedReason || task?.steps_diagnostic || (task?.steps.length ?? 0) > 64)} onClick={() => {
      if (!task || !guard()) return;
      if (task.steps_diagnostic || task.steps.length > 64) { notice(task.steps_diagnostic ?? "Tracking would exceed the 64-step limit.", "warning"); return; }
      preview(submission({ kind: "adopt", mapping: untracked.map((step) => ({ sourceOffset: step.source_offset, stepId: crypto.randomUUID() })) }, task.task_revision), task);
    }}>Track checklist…</button></div> : null}
    <div className={`supervisor-step-status${draft.status ? ` is-${draft.status.tone}` : ""}`} role={draft.status?.tone === "error" ? "alert" : "status"} aria-live="polite">{draft.status?.text ?? ""}</div>
    {operation.kind === "unknown" || operation.kind === "refused" ? <div className="supervisor-step-recovery"><p>{operation.message}</p>{operation.kind === "refused" ? <code>{operation.operationCode}</code> : null}<button type="button" aria-disabled={reading} onClick={() => void readForReview()}>{reading ? "Reading saved steps…" : operation.kind === "unknown" ? "Check saved steps" : "Review current steps"}</button>
      {operation.kind === "refused" ? <button type="button" onClick={() => { draft.operation = { kind: "idle" }; draft.confirmation = null; notice("Saved steps kept. Your drafts remain available."); }}>Keep saved steps</button> : null}</div> : null}
    {operation.kind === "review" ? <div className="supervisor-step-review"><h4>{operation.reason === "unknown" ? "Compare saved steps" : "Review your change against saved steps"}</h4>
      <div className="supervisor-step-compare"><div><strong>Submitted intent</strong><p>{intentText(operation.submitted.intent)}</p><code>{operation.submitted.expectedTaskRevision}</code></div>
        <div><strong>Saved state after read</strong><p>{intentMatchesSaved(operation.currentTask, operation.submitted.intent, draft.confirmation?.baseTask) ? "The exact intended saved state is present. This does not establish who wrote it." : "Saved steps differ from the submitted intent."}</p>
          <ul>{operation.currentTask.steps.map((step) => <li key={step.step_id !== null && step.diagnostic === null ? step.step_id : `source-${step.source_offset}`} data-depth={step.depth}>{step.title} · {step.status} · under {step.parent_step_id ?? "task root"}</li>)}</ul><code>{operation.currentTask.task_revision}</code></div></div>
      {reviewProblem ? <p className="supervisor-step-warning">{reviewProblem}</p> : null}
      <div className="supervisor-step-actions"><button type="button" aria-disabled={resolving} onClick={() => void resolve(operation.reason === "unknown" && intentMatchesSaved(operation.currentTask, operation.submitted.intent, draft.confirmation?.baseTask) ? "use_saved" : "keep_saved")}>{operation.reason === "conflict" ? "Use current" : intentMatchesSaved(operation.currentTask, operation.submitted.intent, draft.confirmation?.baseTask) ? "Use saved state" : "Keep saved state"}</button>
        <button type="button" aria-disabled={pending || !props.writable || !identityValid || reviewProblem !== null} onClick={() => void resolve("apply_reviewed")}>Apply reviewed change</button><button type="button" aria-disabled={reading} onClick={() => void readForReview()}>Read saved steps again</button></div>
    </div> : null}
    {confirmationView()}
    {draft.add.has(null) ? textEditor("add", null, draft.add.get(null)!) : null}
    {orphanAdds.map(([key, textDraft]) => textEditor("add", key, textDraft))}
    {orphanRenames.map(([key, textDraft]) => textEditor("rename", key, textDraft))}
    {task ? <details className="supervisor-step-source"><summary>Canonical task source</summary><pre>{task.body}</pre><button type="button" onClick={() => void copy(task.body)}>Copy task source</button></details> : null}
    {menu ? <div className="supervisor-step-menu-scope" onKeyDownCapture={(event) => {
      if ((event.repeat || event.nativeEvent.isComposing) && (event.key === "Enter" || event.key === " ")) {
        event.preventDefault(); event.stopPropagation();
      }
    }} onKeyDown={(event) => { if (event.defaultPrevented) event.stopPropagation(); }}>
      <LibraryMenu x={menu.x} y={menu.y} label="Step actions" entries={menuEntries} onDismiss={() => {
        setMenu(null);
        if (menu.opener.isConnected) menu.opener.focus({ preventScroll: true });
      }} />
    </div> : null}
  </section>;
}
