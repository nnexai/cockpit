import { useEffect, useId, useLayoutEffect, useRef, useState, type KeyboardEvent } from "react";
import { createPortal } from "react-dom";
import type { DispatchTarget, OrchestrationAction, OrchestrationActionResult, OrchestrationSnapshot, Run, SpaceSummary, Subagent, Task, TaskView } from "../../protocol/generated/v1";
import { copyText } from "../library/clipboard";
import { ErrorSlot } from "../ErrorSlot";
import { UiIcon } from "../UiIcon";
import type { EditDraft, FollowUpDraft, RelationDraft } from "./useSupervisorDrafts";
import type { StepScope, StepReadOutcome } from "./stepInteractions";
import type { SourceSubmission, SourceResolution, SourceResolutionOutcome, TaskMutationOutcome } from "./useSupervisor";
import { dependencyCandidateReason, taskContentReason, uniqueTask } from "./dependencies";
import "../projects/setup.css";

export type StartDraft = { label: string; location: "existing" | "directory" | "dedicated"; spaceId: string; directory: string };
export type TaskSourceDialogState = { mode: "edit"; task: TaskView; draft: EditDraft; scope: StepScope } | { mode: "relations"; task: TaskView; draft: RelationDraft; scope: StepScope } | { mode: "follow_up"; task: TaskView; draft: FollowUpDraft; scope: StepScope };
type RuntimeDialogState = { mode: "start" } | { mode: "retry" | "close" | "setup_recovery"; run: Run } | { mode: "subagent_cancel"; run: Run; subagent: Subagent };
export type SupervisorDialogState = RuntimeDialogState | TaskSourceDialogState;
export function SupervisorDialogs({ dialog, snapshot, spaces, startDraft, changed, busy, available, mutateResult, onStarted, onStartUnconfirmed, onCheck, onReturnFocus, onClose }: {
  dialog: RuntimeDialogState; snapshot: OrchestrationSnapshot; spaces: SpaceSummary[]; startDraft: StartDraft;
  changed(): void; busy: boolean; available: boolean; mutateResult(action: OrchestrationAction): Promise<OrchestrationActionResult | null>;
  onStarted(runId: string): void; onStartUnconfirmed(): void; onCheck(run: Run): void; onClose(): void;
  onReturnFocus?(invoker: Element | null): void;
}) {
  const titleId = useId();
  const formId = useId();
  const spaceErrorId = useId();
  const directoryErrorId = useId();
  const primaryReasonId = useId();
  const ref = useRef<HTMLElement>(null);
  const returnFocus = useRef(onReturnFocus);
  returnFocus.current = onReturnFocus;
  const inFlight = useRef(false);
  const [pending, setPending] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [unconfirmed, setUnconfirmed] = useState(false);
  const [fieldError, setFieldError] = useState<"space" | "directory" | null>(null);
  const [copyStatus, setCopyStatus] = useState<string | null>(null);
  const locked = busy || pending;
  const mode = dialog.mode;
  const dialogRun = "run" in dialog ? dialog.run : null;
  const latestRun = dialogRun ? snapshot.runs.find(run => run.run_id === dialogRun.run_id) ?? dialogRun : null;
  const descendantIds = new Set<string>();
  if (dialogRun && (mode === "retry" || mode === "close")) {
    const children = new Map<string, Run[]>();
    for (const run of snapshot.runs) {
      if (run.root_id !== dialogRun.root_id || !run.parent_run_id) continue;
      const siblings = children.get(run.parent_run_id) ?? [];
      siblings.push(run); children.set(run.parent_run_id, siblings);
    }
    const remaining = [dialogRun.run_id];
    const visited = new Set(remaining);
    while (remaining.length) {
      for (const child of children.get(remaining.pop()!) ?? []) {
        if (visited.has(child.run_id)) continue;
        visited.add(child.run_id); remaining.push(child.run_id);
        if (child.stage !== "closed") descendantIds.add(child.run_id);
      }
    }
  }
  const descendants = descendantIds.size ? snapshot.runs.filter(run => descendantIds.has(run.run_id)) : [];
  const observed = snapshot.runtime.status === "fresh" ? snapshot.runtime.runs.find(item => item.run_id === dialogRun?.run_id) : undefined;
  const lastObserved = !available || snapshot.runtime.status !== "fresh" ? "unobserved" : observed?.presence === "missing" ? "terminal gone" : observed?.presence === "endpoint_changed" ? "endpoint changed" : latestRun?.dispatch?.step === "launch_unknown" || latestRun?.dispatch?.step === "needs_review" ? "start unconfirmed" : observed?.presence === "present" ? observed.actual_omp ? observed.agent_status ?? "unknown" : "OMP not confirmed" : "unobserved";
  const chosenSpace = spaces.find(space => space.id === startDraft.spaceId);
  const destination = startDraft.location === "existing" ? chosenSpace ? `New tab in ${chosenSpace.label}` : "Choose a Space for the new tab" : startDraft.location === "directory" ? startDraft.directory.trim() ? `Opens ${startDraft.directory.trim()}` : "Choose an absolute directory" : "Dedicated agent folder";
  const primaryReason = locked ? "Wait for the current operation to finish." : !available ? "Reconnect before applying this change." : unconfirmed ? "Check current status before requesting another action." : null;
  const title = mode === "start" ? "Start agent" : mode === "retry" ? "Restart agent" : mode === "close" ? "Close tracking" : mode === "setup_recovery" ? "Recover setup" : "Cancel subagent";
  useLayoutEffect(() => {
    const invoker = document.activeElement instanceof HTMLElement && document.activeElement !== document.body ? document.activeElement : null;
    const ancestors: HTMLElement[] = [];
    for (let element = invoker?.parentElement; element && element !== document.body; element = element.parentElement) ancestors.push(element);
    return () => {
      if (returnFocus.current) { returnFocus.current(invoker); return; }
      if (invoker?.isConnected && !invoker.matches(":disabled") && !invoker.closest("[hidden],[inert]")) { invoker.focus({ preventScroll: true }); return; }
      const container = ancestors.find(element => element.isConnected && !element.closest("[hidden],[inert]"));
      if (!container) return;
      if (!container.hasAttribute("tabindex")) {
        const outline = container.style.outline;
        container.tabIndex = -1; container.style.outline = "none";
        container.addEventListener("blur", () => { container.removeAttribute("tabindex"); container.style.outline = outline; }, { once: true });
      }
      container.focus({ preventScroll: true });
    };
  }, []);
  useEffect(() => {
    ref.current?.querySelector<HTMLElement>(mode === "start" && startDraft.location === "existing" && spaces.some(space => space.id === startDraft.spaceId) ? "[data-primary]" : "[data-initial]")?.focus({ preventScroll: true });
  }, []); // Opening owns focus once; polling must not move it.
  useEffect(() => {
    const section = ref.current;
    if (locked && section && (!section.contains(document.activeElement) || document.activeElement?.matches(":disabled"))) section.focus({ preventScroll: true });
  }, [locked]);
  const close = () => { if (!inFlight.current && !busy) onClose(); };
  const keys = (event: KeyboardEvent<HTMLElement>) => {
    if (event.nativeEvent.isComposing) return;
    if (event.key === "Escape") { event.preventDefault(); event.stopPropagation(); close(); return; }
    if (event.key !== "Tab") return;
    event.stopPropagation();
    const elements = [...event.currentTarget.querySelectorAll<HTMLElement>("button,input,textarea,select,summary,[tabindex]:not([tabindex='-1'])")].filter(element => {
      if (element.matches(":disabled,[hidden]") || element.closest("[hidden],[inert]")) return false;
      for (let ancestor = element.parentElement; ancestor && ancestor !== event.currentTarget; ancestor = ancestor.parentElement) {
        if (ancestor instanceof HTMLDetailsElement && !ancestor.open && !ancestor.querySelector(":scope > summary")?.contains(element)) return false;
      }
      return true;
    });
    if (elements.length === 0) {
      event.preventDefault();
      event.currentTarget.focus({ preventScroll: true });
      return;
    }
    const index = elements.indexOf(document.activeElement as HTMLElement);
    if (event.shiftKey && index <= 0) { event.preventDefault(); elements.at(-1)?.focus(); }
    else if (!event.shiftKey && (index < 0 || index === elements.length - 1)) { event.preventDefault(); elements[0]?.focus(); }
  };
  const submit = async () => {
    if (locked || inFlight.current || unconfirmed) return;
    if (!available) { setError("The required connection is unavailable. Your draft and resources are kept."); return; }
    let action: OrchestrationAction;
    if (dialog.mode === "start") {
      let target: DispatchTarget | null = null;
      if (startDraft.location === "existing") {
        if (!spaces.some(space => space.id === startDraft.spaceId)) { setFieldError("space"); return; }
        target = { target: "existing_space", workspace_id: startDraft.spaceId };
      } else if (startDraft.location === "directory") {
        if (!startDraft.directory.trim().startsWith("/")) { setFieldError("directory"); return; }
        target = { target: "setup", request: { operation: "open", path: startDraft.directory.trim(), label: startDraft.label.trim() || null, task_name: null, focus: false } };
      }
      action = { action: "supervisor_start", target, label: startDraft.label.trim() || null };
    } else if (dialog.mode === "subagent_cancel") action = { action: "subagent_control", run_id: dialog.run.run_id, subagent_id: dialog.subagent.subagent_id, op: { op: "cancel" } };
    else if (dialog.mode === "setup_recovery") action = { action: "reconcile_run", run_id: dialog.run.run_id, recovery: dialog.run.dispatch?.recovery ?? null };
    else action = { action: dialog.mode === "retry" ? "retry_launch" : "cancel_run", run_id: dialog.run.run_id };
    inFlight.current = true; setPending(true); setError(null);
    try {
      const result = await mutateResult(action);
      const confirmed = result && (dialog.mode === "start" ? result.result === "run" : result.result === "done" || result.result === "message");
      if (!result || !confirmed) {
        setUnconfirmed(true);
        if (dialog.mode === "start") onStartUnconfirmed();
        setError(dialog.mode === "start" ? "The start was not confirmed. Close these options and check status before another start; the previous request may have opened a terminal." : "The change was not confirmed. Your draft and tracking are kept. Check current status before requesting another action.");
        return;
      }
      if (dialog.mode === "start" && result.result === "run") onStarted(result.run_id);
      onClose();
    } catch (cause) {
      setUnconfirmed(true);
      if (dialog.mode === "start") onStartUnconfirmed();
      setError(cause instanceof Error ? cause.message : "The change was not confirmed. Your draft and tracking are kept. Check current status before requesting another action.");
    }
    finally { inFlight.current = false; setPending(false); }
  };
  return createPortal(<div className="setup-overlay" role="presentation"><section ref={ref} className="supervisor-dialog" role="dialog" aria-modal="true" aria-labelledby={titleId} aria-busy={locked} tabIndex={-1} onKeyDown={keys}>
    <header className="supervisor-dialog-header"><span className="supervisor-dialog-icon"><UiIcon name={mode === "start" ? "plus" : mode === "retry" || mode === "setup_recovery" ? "refresh" : mode === "close" ? "close" : "stop"} /></span><h2 id={titleId}>{title}</h2></header><form id={formId} onSubmit={event => { event.preventDefault(); void submit(); }}>
      {dialog.mode === "start" ? <>
        <p>An OMP supervisor manages tasks and delegates work for you.</p>
        <label>Name (optional)<input value={startDraft.label} disabled={locked} onChange={event => { startDraft.label = event.target.value; changed(); }} /></label>
        <label>Location<select data-initial value={startDraft.location} disabled={locked} onChange={event => { startDraft.location = event.target.value as StartDraft["location"]; setFieldError(null); changed(); }}><option value="existing">Existing Space</option><option value="directory">Directory</option><option value="dedicated">Dedicated agent folder</option></select></label>
        {startDraft.location === "existing" ? <>
          <label>Space<select value={startDraft.spaceId} disabled={locked || spaces.length === 0} aria-invalid={fieldError === "space" || undefined} aria-describedby={fieldError === "space" || spaces.length === 0 ? spaceErrorId : undefined} onChange={event => { startDraft.spaceId = event.target.value; setFieldError(null); changed(); }}><option value="">Choose a Space</option>{spaces.map(space => <option key={space.id} value={space.id}>{space.label}</option>)}</select></label>
          {fieldError === "space" ? <p id={spaceErrorId} className="supervisor-dialog-field-error" role="alert">Choose a currently available Space. No different destination will be selected automatically.</p> : spaces.length === 0 ? <p id={spaceErrorId} className="supervisor-dialog-field-reason">No Space is available right now</p> : null}
        </> : startDraft.location === "directory" ? <>
          <label>Absolute directory<input value={startDraft.directory} disabled={locked} aria-invalid={fieldError === "directory" || undefined} aria-describedby={fieldError === "directory" ? directoryErrorId : undefined} onChange={event => { startDraft.directory = event.target.value; setFieldError(null); changed(); }} /></label>
          {fieldError === "directory" ? <p id={directoryErrorId} className="supervisor-dialog-field-error" role="alert">Enter an absolute directory path.</p> : null}
        </> : <p>Creates a dedicated agent folder and Space. Existing project context is not copied.</p>}
      </> : dialog.mode === "retry" ? <>
        <p>Restart {dialog.run.label}?</p><p>We cannot confirm whether the previous agent is still running. Restarting creates a new terminal and could leave another agent running. The previous launch is checked again before a new launch.</p>{descendants.length ? <p>Worker agents may still be running. Restart only this agent; keep their tasks and history.</p> : null}
        <p className="supervisor-dialog-observed">Last observed: {lastObserved}</p>
      </> : dialog.mode === "close" ? <>
        <p>Close tracking for {dialog.run.label}?</p><p>Tasks, history, Spaces and worktrees are kept. This does not guarantee the agent or its workers stop.</p>
        {descendants.length ? <><ul className="supervisor-dialog-descendants">{descendants.slice(0, 3).map(run => <li key={run.run_id}>{run.label}</li>)}</ul>{descendants.length > 3 ? <p>and {descendants.length - 3} more</p> : null}<p>Live descendants will still need supervision.</p></> : null}
      </> : dialog.mode === "subagent_cancel" ? <p>Request cancellation of {dialog.subagent.label}. Its OMP control receipt, not this request, confirms whether it stopped.</p> : <>
        <p>Setup for {dialog.run.label} needs review before launch. Existing resources are kept.</p><p>{dialog.run.dispatch?.recovery === "accept_existing_worktree" ? "Confirm using the existing worktree receipt rather than creating another worktree." : dialog.run.dispatch?.recovery === "retry_environment" ? "Explicitly retry the uncertain setup operation after reviewing its recorded effects." : "Check and reconcile the existing setup only. No new launch is requested by this confirmation."}</p>
        {dialog.run.prepare_plan ? <details><summary>Exact setup plan</summary><p className="supervisor-exact-text">{dialog.run.prepare_plan.text}</p></details> : null}{dialog.run.setup ? <><p>{dialog.run.setup.checkout_path}</p><ul>{dialog.run.setup.effects.map((effect, index) => <li key={index}>{effect}</li>)}</ul>{dialog.run.setup.warnings.map((warning, index) => <p className="supervisor-warning" key={index}>{warning}</p>)}</> : null}
      </>}
    </form>
    {mode === "start" ? <p className="supervisor-dialog-destination">{destination} · OMP starts without switching terminal focus</p> : null}
    <ErrorSlot placement="dialog" message={error} className={`supervisor-dialog-error-slot${error ? "" : " is-empty"}`} />
    {primaryReason ? <p id={primaryReasonId} className="supervisor-dialog-unavailable">{primaryReason}{!available ? " You can keep editing or close this dialog without losing your draft." : ""}</p> : null}
    <footer>
      <button type="button" data-initial={mode !== "start" || undefined} disabled={locked} onClick={close}>{mode === "close" ? "Keep tracking" : mode === "retry" ? "Back" : mode === "subagent_cancel" ? "Keep running" : "Cancel"}</button>
      {dialog.mode === "retry" ? <button type="button" disabled={locked || !available} aria-describedby={!available || locked ? primaryReasonId : undefined} onClick={() => { if (locked || inFlight.current || !available) return; onCheck(latestRun ?? dialog.run); onClose(); }}>Check again first</button> : null}
      <button type="submit" form={formId} data-primary aria-describedby={primaryReason ? primaryReasonId : undefined} disabled={locked || !available || unconfirmed}>{pending ? "Working…" : dialog.mode === "retry" ? "Restart anyway" : dialog.mode === "subagent_cancel" ? "Request cancellation" : dialog.mode === "setup_recovery" ? dialog.run.dispatch?.recovery === "accept_existing_worktree" ? "Use existing worktree" : dialog.run.dispatch?.recovery === "retry_environment" ? "Retry setup" : "Check setup" : title}</button>
    </footer>
  </section></div>, document.body);
}

export function TaskSourceDialog({ dialog, snapshot, changed, busy, available, writeUnconfirmed, submitTask, readSaved, resolveUnknown, onSaved, onDiscard, onClose, onReturnFocus }: {
  dialog: TaskSourceDialogState; snapshot: OrchestrationSnapshot; changed(): void; busy: boolean; available: boolean; writeUnconfirmed: boolean;
  submitTask(submitted: SourceSubmission): Promise<TaskMutationOutcome<SourceSubmission>>;
  readSaved(scope: StepScope): Promise<StepReadOutcome>;
  resolveUnknown(request: SourceResolution): Promise<SourceResolutionOutcome>;
  onSaved(task: Task, created: boolean, reconciled?: boolean): void; onDiscard(): void; onClose(): void; onReturnFocus(invoker: Element | null): void;
}) {
  const headingId = useId(), reasonId = useId(), pickerId = useId();
  const ref = useRef<HTMLElement>(null), inFlight = useRef(false), returnFocus = useRef(onReturnFocus);
  returnFocus.current = onReturnFocus;
  const [pending, setPending] = useState(false), [error, setError] = useState<string | null>(null), [notice, setNotice] = useState<string | null>(null);
  const [confirmRemoval, setConfirmRemoval] = useState<string | null>(null), [discard, setDiscard] = useState(false);
  const [pickerOpen, setPickerOpen] = useState(false), [option, setOption] = useState(0);
  const [missingRead, setMissingRead] = useState(false);
  const locked = busy || pending;
  const current = snapshot.board?.root_id === dialog.scope.rootId ? uniqueTask(snapshot.board.tasks, dialog.task.task.task_id) : null;
  const tasks = snapshot.board?.root_id === dialog.scope.rootId ? snapshot.board.tasks : [];
  const docRevision = snapshot.board?.root_id === dialog.scope.rootId ? snapshot.board.doc_revision : null;
  const rootClosed = snapshot.runs.find(run => run.run_id === dialog.scope.rootId)?.stage === "closed";
  const attemptsOpen = snapshot.runs.some(run => run.root_id === dialog.scope.rootId && run.task_id === dialog.task.task.task_id && run.stage !== "closed");
  const acceptancePending = snapshot.intents.some(intent => intent.root_id === dialog.scope.rootId && intent.task_id === dialog.task.task.task_id);
  const sourceUncertain = !!dialog.draft.submitted;
  const base = dialog.mode === "follow_up" ? dialog.draft.baseSource : dialog.draft.baseTask;
  const stale = !current || current.task.task_revision !== (dialog.mode === "follow_up" ? base.task_revision : dialog.draft.revision)
    || dialog.mode !== "edit" && docRevision !== dialog.draft.docRevision;
  const additions = dialog.mode === "relations" && current ? dialog.draft.dependsOn.filter(id => !current.task.depends_on.includes(id)) : [];
  const removed = dialog.mode === "relations" && current ? current.task.depends_on.filter(id => !dialog.draft.dependsOn.includes(id)) : [];
  const unchanged = dialog.mode === "relations" && current ? current.task.depends_on.length === dialog.draft.dependsOn.length && current.task.depends_on.every(id => dialog.draft.dependsOn.includes(id)) : false;
  const validation = dialog.mode === "relations" ? dialog.draft.dependsOn.length > 32 ? "At most 32 prerequisites are allowed." : null
    : !dialog.draft.title.trim() ? "Enter a task title." : /[\n\r\0]/.test(dialog.draft.title) || new TextEncoder().encode(dialog.draft.title).length > 256 ? "Use a single-line title of at most 256 UTF-8 bytes, without NUL." : dialog.draft.description.includes("\0") ? "Description cannot contain NUL." : new TextEncoder().encode(dialog.draft.description).length > 16384 ? "Description exceeds the 16 KiB task continuation limit. Metadata also counts toward this limit." : null;
  const reason = locked ? "Wait for the current operation." : !available ? "Reconnect before saving." : rootClosed ? "Tracking is closed; tasks cannot be changed." : !current ? "This task no longer exists. Your draft is kept." : sourceUncertain || writeUnconfirmed ? "A task change is unconfirmed. Check saved state and resolve the original operation first." : stale ? "Review the current task and task file before saving." : dialog.mode === "edit" ? taskContentReason(snapshot, current, available) ?? (!current.task.description_editable ? current.task.description_diagnostic ?? "The description cannot be safely edited." : null) : dialog.mode === "relations" ? acceptancePending ? "Resolve the pending acceptance decision before changing prerequisites." : current.task.checked ? "Task is complete; prerequisites are read-only." : current.task.diagnostic ? "Resolve the canonical task identity diagnostic first." : attemptsOpen && additions.length ? "Work is tracked: only removal of existing prerequisites is allowed. Your added rows are kept." : unchanged ? "Prerequisites are unchanged." : null : current.task.diagnostic ? "Resolve the source task identity diagnostic first." : null;
  const candidates = dialog.mode === "relations" ? tasks.filter(task => task.task.task_id !== dialog.task.task.task_id && !dialog.draft.dependsOn.includes(task.task.task_id) && task.task.title.toLowerCase().includes(dialog.draft.query.toLowerCase())).sort((a, b) => Number(a.task.checked) - Number(b.task.checked) || a.task.line - b.task.line) : [];
  useLayoutEffect(() => {
    const invoker = document.activeElement;
    (ref.current?.querySelector<HTMLElement>("[data-initial]:not(:disabled)") ?? ref.current)?.focus({ preventScroll: true });
    return () => returnFocus.current(invoker);
  }, []);
  useEffect(() => { if (confirmRemoval) ref.current?.querySelector<HTMLButtonElement>("[data-keep-prerequisites]")?.focus({ preventScroll: true }); }, [confirmRemoval]);
  const keys = (event: KeyboardEvent<HTMLElement>) => {
    if (event.nativeEvent.isComposing) return;
    if (event.key === "Escape") {
      event.preventDefault(); event.stopPropagation();
      if (confirmRemoval) setConfirmRemoval(null); else if (discard) setDiscard(false); else if (!locked) onClose();
      return;
    }
    if (event.key !== "Tab") return;
    event.stopPropagation();
    const elements = [...event.currentTarget.querySelectorAll<HTMLElement>("button,input,textarea,select,[tabindex]:not([tabindex='-1'])")].filter(element => !element.matches(":disabled,[hidden]") && !element.closest("[hidden],[inert]"));
    const index = elements.indexOf(document.activeElement as HTMLElement);
    if (!elements.length) { event.preventDefault(); ref.current?.focus(); }
    else if (event.shiftKey && index <= 0) { event.preventDefault(); elements.at(-1)?.focus(); }
    else if (!event.shiftKey && (index < 0 || index === elements.length - 1)) { event.preventDefault(); elements[0]?.focus(); }
  };
  const finish = (outcome: TaskMutationOutcome<SourceSubmission>) => {
    if (outcome.kind === "confirmed") { dialog.draft.submitted = null; onSaved(outcome.task, dialog.mode === "follow_up"); onClose(); }
    else if (outcome.kind === "unknown") { dialog.draft.submitted = outcome.submitted; dialog.draft.reviewed = null; setError(outcome.message); }
    else setError(outcome.kind === "refused" ? `${outcome.message} (${outcome.operationCode})` : outcome.reason);
    changed();
  };
  const capture = (): SourceSubmission => {
    const originalScope = dialog.mode === "follow_up" ? { ...dialog.scope, taskId: dialog.draft.taskId } : dialog.scope;
    const common = { root_id: dialog.scope.rootId, task_id: originalScope.taskId };
    const action: SourceSubmission["action"] = dialog.mode === "edit"
      ? { action: "task_update", ...common, expected_task_revision: dialog.draft.revision, title: dialog.draft.title, description: dialog.draft.description }
      : dialog.mode === "relations" ? { action: "task_dependencies_set", ...common, expected_task_revision: dialog.draft.revision, expected_doc_revision: dialog.draft.docRevision, depends_on: [...dialog.draft.dependsOn] }
      : { action: "task_create", ...common, title: dialog.draft.title, description: dialog.draft.description, depends_on: dialog.draft.waitForSource ? [dialog.scope.taskId] : [], follow_up_of: dialog.scope.taskId, expected_doc_revision: dialog.draft.docRevision, source_revision: dialog.draft.baseSource.task_revision };
    return { submissionId: crypto.randomUUID(), scope: originalScope, action };
  };
  const save = async () => {
    if (locked || inFlight.current) return;
    if (validation || reason) { setError(validation ?? reason); return; }
    if (dialog.mode === "relations" && attemptsOpen) {
      const fence = JSON.stringify([dialog.draft.revision, dialog.draft.docRevision, dialog.draft.dependsOn]);
      if (!removed.length || additions.length) return;
      if (confirmRemoval !== fence) { setConfirmRemoval(fence); return; }
    }
    const submitted = capture();
    inFlight.current = true; setPending(true); setError(null);
    try { finish(await submitTask(submitted)); }
    finally { inFlight.current = false; setPending(false); }
  };
  const read = async () => {
    const submitted = dialog.draft.submitted;
    if (!submitted || locked) return;
    inFlight.current = true; setPending(true);
    try {
      const result = await readSaved(submitted.scope);
      dialog.draft.reviewed = result.kind === "found" ? result.task : null;
      setMissingRead(result.kind === "missing");
      setNotice(result.kind === "found" ? "Saved task read. Compare it before explicitly resolving the original operation." : result.kind === "missing" ? "No task with the retained identity exists in the original task file." : result.message);
      changed();
    } finally { inFlight.current = false; setPending(false); }
  };
  const retrySameCreation = async () => {
    const submitted = dialog.draft.submitted;
    if (locked || inFlight.current || dialog.mode !== "follow_up" || !submitted || !missingRead) return;
    inFlight.current = true; setPending(true);
    try {
      const result = await resolveUnknown({ originalSubmitted: submitted, reviewedTaskRevision: null, decision: { kind: "retry_same" } });
      if (result.kind === "not_resolved") setError(result.message);
      else if (result.kind === "applied") {
        if (result.outcome.kind !== "not_sent") dialog.draft.submitted = null;
        setMissingRead(false); finish(result.outcome);
      }
    } finally { inFlight.current = false; setPending(false); }
  };
  const resolve = async (decision: "use_saved" | "keep_saved" | "apply_reviewed") => {
    const submitted = dialog.draft.submitted, reviewed = dialog.draft.reviewed;
    if (!submitted || !reviewed || locked) return;
    if (decision === "apply_reviewed" && dialog.mode === "follow_up") { setError("The saved task already exists. This draft will not replace it. Keep saved state or copy your draft."); return; }
    let next: SourceSubmission | null = null;
    if (decision === "apply_reviewed") {
      if (!current || current.task.task_revision !== reviewed.task_revision || !docRevision) { setError("Read and review the original saved task again."); return; }
      if (!available || rootClosed || validation || dialog.mode === "edit" && (taskContentReason(snapshot, current, available) || !current.task.description_editable) || dialog.mode === "relations" && (acceptancePending || current.task.checked || current.task.diagnostic || attemptsOpen && (additions.length || !removed.length))) { setError("The reviewed source change is not writable. Keep the saved state or resolve its current guard first."); return; }
      if (dialog.mode === "relations" && attemptsOpen) {
        const fence = JSON.stringify([reviewed.task_revision, docRevision, dialog.draft.dependsOn]);
        if (confirmRemoval !== fence) { setConfirmRemoval(fence); return; }
      }
      next = capture();
      if (next.action.action === "task_update") next = { ...next, action: { ...next.action, expected_task_revision: reviewed.task_revision } };
      else if (next.action.action === "task_dependencies_set") next = { ...next, action: { ...next.action, expected_task_revision: reviewed.task_revision, expected_doc_revision: docRevision } };
    }
    inFlight.current = true; setPending(true);
    try {
      const result = await resolveUnknown({ originalSubmitted: submitted, reviewedTaskRevision: reviewed.task_revision, decision: next ? { kind: "apply_reviewed", submitted: next } : { kind: decision as "use_saved" | "keep_saved" } });
      if (result.kind === "not_resolved") setError(result.message);
      else if (result.kind === "applied") {
        if (result.outcome.kind !== "not_sent") dialog.draft.submitted = null;
        finish(result.outcome);
      } else {
        dialog.draft.submitted = null; dialog.draft.reviewed = null; setError(null);
        if (dialog.mode === "edit") { dialog.draft.title = result.task.title; dialog.draft.description = result.task.description; dialog.draft.revision = result.task.task_revision; dialog.draft.baseTask = result.task; }
        else if (dialog.mode === "relations") { dialog.draft.dependsOn = [...result.task.depends_on]; dialog.draft.baseSet = [...result.task.depends_on]; dialog.draft.revision = result.task.task_revision; dialog.draft.baseTask = result.task; dialog.draft.docRevision = docRevision ?? dialog.draft.docRevision; }
        else { onSaved(result.task, true, true); onClose(); }
        changed();
      }
    } finally { inFlight.current = false; setPending(false); }
  };
  const review = (useCurrent: boolean) => {
    if (!current || !docRevision || locked || sourceUncertain) return;
    if (dialog.mode === "edit") {
      if (useCurrent) { dialog.draft.title = current.task.title; dialog.draft.description = current.task.description; }
      dialog.draft.revision = current.task.task_revision; dialog.draft.baseTask = current.task;
    } else if (dialog.mode === "relations") {
      if (useCurrent) dialog.draft.dependsOn = [...current.task.depends_on];
      else {
        const additions = dialog.draft.dependsOn.filter(id => !dialog.draft.baseSet.includes(id)), removals = dialog.draft.baseSet.filter(id => !dialog.draft.dependsOn.includes(id));
        dialog.draft.dependsOn = [...current.task.depends_on.filter(id => !removals.includes(id)), ...additions.filter(id => !current.task.depends_on.includes(id))];
      }
      dialog.draft.baseSet = [...current.task.depends_on]; dialog.draft.baseTask = current.task; dialog.draft.revision = current.task.task_revision; dialog.draft.docRevision = docRevision;
    } else { dialog.draft.baseSource = current.task; dialog.draft.docRevision = docRevision; }
    setConfirmRemoval(null); setError(null); changed();
  };
  const title = dialog.mode === "edit" ? "Edit task" : dialog.mode === "relations" ? "Edit prerequisites" : "Create follow-up";
  const saved = dialog.draft.reviewed;
  const submitted = dialog.draft.submitted?.action;
  const matches = saved && submitted ? submitted.action === "task_update"
    ? saved.title === submitted.title && saved.description === submitted.description
    : submitted.action === "task_dependencies_set"
      ? saved.depends_on.length === submitted.depends_on.length && saved.depends_on.every(id => submitted.depends_on.includes(id))
      : saved.title === submitted.title && saved.description === submitted.description && saved.follow_up_of === submitted.follow_up_of && JSON.stringify(saved.depends_on) === JSON.stringify(submitted.depends_on) : false;
  const comparisonTask = saved ?? current?.task;
  return createPortal(<div className="setup-overlay" role="presentation"><section ref={ref} className="supervisor-dialog" role="dialog" aria-modal="true" aria-labelledby={headingId} aria-busy={locked} tabIndex={-1} onKeyDown={keys}>
    <header className="supervisor-dialog-header"><UiIcon name={dialog.mode === "follow_up" ? "plus" : "edit"} /><h2 id={headingId}>{title}</h2></header>
    <p>{dialog.mode === "follow_up" ? "Follow-up of " : ""}{dialog.task.task.title}</p>
    <form onSubmit={event => { event.preventDefault(); void save(); }}>
      {dialog.mode === "relations" ? <>
        <p>Waits until all of these are accepted. A Result alone does not satisfy a prerequisite.</p>
        <h3>Prerequisites · {dialog.draft.dependsOn.length} of 32</h3>
        {current?.task.relations_diagnostic ? <p className="supervisor-warning" role="alert">{current.task.relations_diagnostic}</p> : null}
        {current?.dependencies.problems.map((problem, index) => <p className="supervisor-warning" key={`${problem.code}:${index}`}>{problem.message} ({problem.code})</p>)}
        {current?.task.relations_diagnostic || current?.dependencies.state === "invalid" ? <p>Repair ambiguous or malformed relationships in the canonical task file: <code className="supervisor-path">{snapshot.board?.path}</code> <button type="button" disabled={locked || !snapshot.board?.path} onClick={async () => setNotice(await copyText(snapshot.board!.path) ? "Task file path copied." : "Could not copy the path.")}>Copy task file path</button></p> : null}
        <ul className="supervisor-prerequisite-editor">{dialog.draft.dependsOn.map(id => <li key={id}><span>{uniqueTask(tasks, id)?.task.title ?? `${tasks.some(view => view.task.task_id === id) ? "Ambiguous task identity" : "Not in this task file"} · ${id}`}{additions.includes(id) && attemptsOpen ? " · addition cannot be saved during work" : ""}</span><button type="button" data-initial={attemptsOpen || undefined} aria-label={`Remove prerequisite ${uniqueTask(tasks, id)?.task.title ?? id}`} disabled={locked || sourceUncertain} onClick={() => { dialog.draft.dependsOn = dialog.draft.dependsOn.filter(value => value !== id); setConfirmRemoval(null); changed(); }}><UiIcon name="close" /></button></li>)}</ul>
        <label>Add prerequisite<input data-initial={!attemptsOpen || undefined} role="combobox" aria-expanded={pickerOpen} aria-controls={pickerId} aria-autocomplete="list" aria-activedescendant={pickerOpen && candidates[option] ? `${pickerId}-${option}` : undefined} value={dialog.draft.query} disabled={locked || sourceUncertain || attemptsOpen || dialog.draft.dependsOn.length >= 32} onChange={event => { dialog.draft.query = event.target.value; setPickerOpen(true); setOption(0); changed(); }} onFocus={() => setPickerOpen(true)} onKeyDown={event => {
          if (event.nativeEvent.isComposing) return;
          if (event.key === "Escape" && pickerOpen) { event.preventDefault(); event.stopPropagation(); setPickerOpen(false); dialog.draft.query = ""; changed(); }
          else if (event.key === "ArrowDown" || event.key === "ArrowUp") { event.preventDefault(); setPickerOpen(true); const step = event.key === "ArrowDown" ? 1 : -1; let index = option + step; while (index >= 0 && index < candidates.length && dependencyCandidateReason(tasks, dialog.scope.taskId, candidates[index].task.task_id)) index += step; if (index >= 0 && index < candidates.length) setOption(index); }
          else if (event.key === "Enter" && pickerOpen) { event.preventDefault(); const candidate = candidates[option]; if (candidate && !dependencyCandidateReason(tasks, dialog.scope.taskId, candidate.task.task_id)) { dialog.draft.dependsOn.push(candidate.task.task_id); dialog.draft.query = ""; setOption(0); changed(); } }
        }} /></label>
        {pickerOpen && !attemptsOpen && !sourceUncertain ? <ul id={pickerId} role="listbox" aria-label="Available prerequisite tasks" className="supervisor-prerequisite-picker">{candidates.map((candidate, index) => { const candidateReason = dependencyCandidateReason(tasks, dialog.scope.taskId, candidate.task.task_id); return <li id={`${pickerId}-${index}`} role="option" aria-selected={index === option} aria-disabled={!!candidateReason} key={candidate.task.task_id}><button type="button" tabIndex={-1} disabled={!!candidateReason || locked || dialog.draft.dependsOn.length >= 32} onClick={() => { dialog.draft.dependsOn.push(candidate.task.task_id); dialog.draft.query = ""; setOption(0); changed(); ref.current?.querySelector<HTMLInputElement>('[role="combobox"]')?.focus(); }}>{candidate.task.title} · {candidate.task.checked ? "Accepted" : "Open"}{candidateReason ? ` · ${candidateReason}` : ""}</button></li>; })}</ul> : null}
        {attemptsOpen ? <p>Work is tracked: remove existing prerequisites only. Removal deliberately changes sequencing.</p> : null}
        {attemptsOpen && additions.length ? <button type="button" disabled={locked || sourceUncertain} onClick={() => { if (current) { dialog.draft.dependsOn = dialog.draft.dependsOn.filter(id => current.task.depends_on.includes(id)); setConfirmRemoval(null); changed(); } }}>Keep only my removals</button> : null}
      </> : <>
        <label>Task title<input data-initial value={dialog.draft.title} readOnly={locked || sourceUncertain} aria-invalid={!!validation} aria-describedby={validation ? reasonId : undefined} onChange={event => { dialog.draft.title = event.target.value; changed(); }} /></label>
        <label>Task description<textarea rows={8} value={dialog.draft.description} readOnly={locked || sourceUncertain} onChange={event => { dialog.draft.description = event.target.value; changed(); }} /></label>
        {dialog.mode === "follow_up" ? <><label className="supervisor-followup-wait"><input type="checkbox" checked={dialog.draft.waitForSource} disabled={locked || sourceUncertain} onChange={event => { dialog.draft.waitForSource = event.target.checked; changed(); }} />Also wait for “{dialog.draft.baseSource.title}” to be accepted</label><p>{current?.task.checked ? "The source is already accepted, so this prerequisite is satisfied now. Uncheck for an independent follow-up." : "Provenance alone does not block the follow-up."} Creating a task does not assign it. The supervisor assigns work.</p></> : <p>Saving title/description preserves saved steps and prerequisites.</p>}
      </>}
      {stale || sourceUncertain ? <section className="supervisor-source-review"><p className="supervisor-warning">{sourceUncertain ? "Change unconfirmed. Read the original saved task before another write." : "Task or task file changed elsewhere. Review current description, steps and prerequisites before advancing either fence."}</p>
        {sourceUncertain ? <button type="button" disabled={locked} onClick={() => void read()}>Check saved {dialog.mode === "follow_up" ? "tasks" : "task"}</button> : null}
        {sourceUncertain && dialog.mode === "follow_up" && missingRead ? <button type="button" disabled={locked || !available || rootClosed} onClick={() => void retrySameCreation()}>Retry same follow-up</button> : null}
        {comparisonTask ? <div className="supervisor-dialog-comparison"><section><h3>{sourceUncertain ? "Original submission" : "Base task"}</h3><pre className="supervisor-plan">{sourceUncertain ? JSON.stringify(dialog.draft.submitted?.action, null, 2) : `${base.title}\n${base.description}\nSteps ${base.step_progress ? `${base.step_progress.done}/${base.step_progress.total}` : "unavailable"}\nPrerequisites ${base.depends_on.join(", ")}`}</pre></section><section><h3>Current saved task</h3><h4>{comparisonTask.title}</h4><p className="supervisor-exact-text">{comparisonTask.description}</p><p>Steps {comparisonTask.step_progress ? `${comparisonTask.step_progress.done}/${comparisonTask.step_progress.total}` : "unavailable"}</p><ul>{comparisonTask.steps.map((step, index) => <li key={step.step_id ?? `untracked-${index}`}>{step.status} · {step.title}</li>)}</ul><p>Prerequisites: {comparisonTask.depends_on.map(id => uniqueTask(tasks, id)?.task.title ?? id).join(" · ") || "none"}</p>{tasks.flatMap(task => task.dependencies.problems).map((problem, index) => <p key={index} className="supervisor-warning">{problem.message}</p>)}</section></div> : null}
        {!sourceUncertain && current ? <div className="supervisor-action-row"><button type="button" disabled={locked} onClick={() => review(false)}>{dialog.mode === "follow_up" ? "Review current source and task file" : dialog.mode === "relations" ? "Apply my changes to current" : "Keep my draft for reviewed save"}</button>{dialog.mode !== "follow_up" ? <button type="button" disabled={locked} onClick={() => review(true)}>Use current</button> : null}</div> : null}
        {sourceUncertain && saved ? <><p>{matches ? "Saved state matches your submission. This does not prove which actor wrote it." : "Saved state differs from your submission."}</p><button type="button" disabled={locked} onClick={() => void resolve(matches ? "use_saved" : "keep_saved")}>{matches ? "Use saved state" : "Keep saved state"}</button>{!matches && dialog.mode !== "follow_up" ? <button type="button" disabled={locked || !current || !!taskContentReason(snapshot, current, available) && dialog.mode === "edit" || dialog.mode === "relations" && (acceptancePending || attemptsOpen && additions.length > 0)} onClick={() => void resolve("apply_reviewed")}>Apply reviewed change</button> : null}</> : null}
      </section> : null}
      {confirmRemoval ? <section className="supervisor-source-confirmation"><p>Removing these prerequisites changes when this task may continue: {removed.map(id => uniqueTask(tasks, id)?.task.title ?? id).join(" · ")}</p><button type="button" data-keep-prerequisites onClick={() => setConfirmRemoval(null)}>Keep prerequisites</button><button type="button" disabled={locked || !available || rootClosed || !sourceUncertain && stale || sourceUncertain && current?.task.task_revision !== dialog.draft.reviewed?.task_revision || !!validation || acceptancePending || additions.length > 0 || removed.length === 0 || confirmRemoval !== JSON.stringify([dialog.mode === "relations" ? dialog.draft.reviewed?.task_revision ?? dialog.draft.revision : "", docRevision, dialog.mode === "relations" ? dialog.draft.dependsOn : []])} onClick={() => sourceUncertain ? void resolve("apply_reviewed") : void save()}>Remove prerequisites</button></section> : null}
      <ErrorSlot placement="dialog" message={error} className="supervisor-dialog-error-slot" />
      {notice ? <p role="status">{notice}</p> : null}{reason || validation ? <p id={reasonId} className="supervisor-disabled-reason">{validation ?? reason}</p> : null}
      <div className="supervisor-dialog-draft-actions"><button type="button" disabled={locked} onClick={async () => setNotice(await copyText(dialog.mode === "relations" ? dialog.draft.dependsOn.join("\n") : `${dialog.draft.title}\n\n${dialog.draft.description}`) ? "Draft copied." : "Could not copy. Select draft text and copy manually.")}>Copy draft</button><button type="button" disabled={locked || sourceUncertain} onClick={() => setDiscard(true)}>Discard draft…</button></div>
      {discard ? <p>Discard these unsaved changes? <button type="button" onClick={() => setDiscard(false)}>Keep draft</button><button type="button" onClick={() => { onDiscard(); onClose(); }}>Discard</button></p> : null}
      <footer><button type="button" disabled={locked} onClick={onClose}>Cancel</button><button type="submit" data-primary disabled={!!reason || !!validation || !!confirmRemoval} aria-describedby={reason || validation ? reasonId : undefined}>{pending ? "Saving…" : dialog.mode === "edit" ? "Save task" : dialog.mode === "relations" ? attemptsOpen ? "Remove prerequisites…" : "Save prerequisites" : "Create follow-up"}</button></footer>
    </form>
  </section></div>, document.body);
}
