import { useEffect, useId, useLayoutEffect, useRef, useState, type KeyboardEvent } from "react";
import { createPortal } from "react-dom";
import type { DispatchTarget, OrchestrationAction, OrchestrationActionResult, OrchestrationSnapshot, Run, SpaceSummary, Subagent, TaskView } from "../../protocol/generated/v1";
import { copyText } from "../library/clipboard";
import { ErrorSlot } from "../ErrorSlot";
import { UiIcon } from "../UiIcon";
import type { EditDraft } from "./useSupervisorDrafts";
import "../projects/setup.css";

export type StartDraft = { label: string; location: "existing" | "directory" | "dedicated"; spaceId: string; directory: string };
export type SupervisorDialogState = { mode: "start" } | { mode: "edit"; task: TaskView; draft: EditDraft } | { mode: "retry" | "close" | "setup_recovery"; run: Run } | { mode: "subagent_cancel"; run: Run; subagent: Subagent };
export function SupervisorDialogs({ dialog, snapshot, spaces, startDraft, changed, busy, available, mutateResult, onStarted, onEdited, onStartUnconfirmed, onCheck, onReturnFocus, onClose }: {
  dialog: SupervisorDialogState; snapshot: OrchestrationSnapshot; spaces: SpaceSummary[]; startDraft: StartDraft;
  changed(): void; busy: boolean; available: boolean; mutateResult(action: OrchestrationAction): Promise<OrchestrationActionResult | null>;
  onStarted(runId: string): void; onEdited(taskId: string): void; onStartUnconfirmed(): void; onCheck(run: Run): void; onClose(): void;
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
  const editedTaskId = dialog.mode === "edit" ? dialog.task.task.task_id : null;
  const currentTask = editedTaskId ? snapshot.board?.tasks.find(task => task.task.task_id === editedTaskId) : null;
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
  const primaryReason = locked ? "Wait for the current operation to finish." : !available ? "Reconnect before applying this change." : unconfirmed && mode !== "edit" ? "Check current status before requesting another action." : mode === "edit" && !currentTask ? snapshot.board ? "This task no longer exists." : "The task document is unavailable." : null;
  const title = mode === "start" ? "Start agent" : mode === "edit" ? "Edit task" : mode === "retry" ? "Restart agent" : mode === "close" ? "Close tracking" : mode === "setup_recovery" ? "Recover setup" : "Cancel subagent";
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
    if (locked || inFlight.current || unconfirmed && dialog.mode !== "edit") return;
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
    } else if (dialog.mode === "edit") {
      if (!dialog.draft.title.trim()) { setError("Enter a task title."); return; }
      if (!snapshot.board) { setError("The task document is unavailable. Your edit draft is kept."); return; }
      if (!currentTask) { setError("This task no longer exists. Your edit draft is kept."); return; }
      action = { action: "task_update", root_id: snapshot.board.root_id, task_id: dialog.task.task.task_id, expected_task_revision: dialog.draft.revision, title: dialog.draft.title, body: dialog.draft.body };
    } else if (dialog.mode === "subagent_cancel") action = { action: "subagent_control", run_id: dialog.run.run_id, subagent_id: dialog.subagent.subagent_id, op: { op: "cancel" } };
    else if (dialog.mode === "setup_recovery") action = { action: "reconcile_run", run_id: dialog.run.run_id, recovery: dialog.run.dispatch?.recovery ?? null };
    else action = { action: dialog.mode === "retry" ? "retry_launch" : "cancel_run", run_id: dialog.run.run_id };
    inFlight.current = true; setPending(true); setError(null);
    try {
      const result = await mutateResult(action);
      const confirmed = result && (dialog.mode === "start" ? result.result === "run" : dialog.mode === "edit" ? result.result === "task" : result.result === "done" || result.result === "message");
      if (!result || !confirmed) {
        if (dialog.mode !== "edit") setUnconfirmed(true);
        if (dialog.mode === "start") onStartUnconfirmed();
        setError(dialog.mode === "start" ? "The start was not confirmed. Close these options and check status before another start; the previous request may have opened a terminal." : "The change was not confirmed. Your draft and tracking are kept. Check current status before requesting another action.");
        return;
      }
      if (dialog.mode === "start" && result.result === "run") onStarted(result.run_id);
      if (dialog.mode === "edit" && result.result === "task") onEdited(dialog.task.task.task_id);
      onClose();
    } catch (cause) {
      if (dialog.mode !== "edit") setUnconfirmed(true);
      if (dialog.mode === "start") onStartUnconfirmed();
      setError(cause instanceof Error ? cause.message : "The change was not confirmed. Your draft and tracking are kept. Check current status before requesting another action.");
    }
    finally { inFlight.current = false; setPending(false); }
  };
  return createPortal(<div className="setup-overlay" role="presentation"><section ref={ref} className="supervisor-dialog" role="dialog" aria-modal="true" aria-labelledby={titleId} aria-busy={locked} tabIndex={-1} onKeyDown={keys}>
    <header className="supervisor-dialog-header"><span className="supervisor-dialog-icon"><UiIcon name={mode === "start" ? "plus" : mode === "edit" ? "edit" : mode === "retry" || mode === "setup_recovery" ? "refresh" : mode === "close" ? "close" : "stop"} /></span><h2 id={titleId}>{title}</h2></header><form id={formId} onSubmit={event => { event.preventDefault(); void submit(); }}>
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
      </> : dialog.mode === "edit" ? <>
        <label>Task title<input data-initial value={dialog.draft.title} disabled={locked} onChange={event => { dialog.draft.title = event.target.value; changed(); }} /></label>
        <label>Task description<textarea rows={8} value={dialog.draft.body} disabled={locked} onChange={event => { dialog.draft.body = event.target.value; changed(); }} /></label>
        {!currentTask ? <>
          <p className="supervisor-warning">{snapshot.board ? "This task no longer exists." : "The task document is unavailable."} Your draft is kept.</p>
          <button type="button" disabled={locked} onClick={async () => { const copied = await copyText(`${dialog.draft.title}\n\n${dialog.draft.body}`); setCopyStatus(copied ? "Draft copied." : "Could not copy the draft. Select the text and copy it manually."); }}>Copy draft</button>
          {copyStatus ? <p role="status">{copyStatus}</p> : null}
        </> : currentTask.task.task_revision !== dialog.draft.revision ? <>
          <p className="supervisor-warning">Task changed elsewhere. Saving will not overwrite those changes without your review. Your draft is kept.</p>
          <div className="supervisor-dialog-comparison">
            <section aria-label="Your draft"><h3>Your draft</h3><h4>{dialog.draft.title}</h4><p className="supervisor-exact-text">{dialog.draft.body}</p></section>
            <section aria-label="Current task"><h3>Current task</h3><h4>{currentTask.task.title}</h4><p className="supervisor-exact-text">{currentTask.task.body}</p></section>
          </div>
          <p>Keeping your draft for the next save replaces the current task text only if this reviewed version is still current.</p>
          <div className="supervisor-dialog-draft-actions">
            <button type="button" disabled={locked || !!currentTask.task.diagnostic} onClick={() => { dialog.draft.revision = currentTask.task.task_revision; changed(); }}>Keep my draft</button>
            <button type="button" disabled={locked} onClick={() => { dialog.draft.title = currentTask.task.title; dialog.draft.body = currentTask.task.body; dialog.draft.revision = currentTask.task.task_revision; changed(); }}>Use current</button>
          </div>
        </> : null}
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
      <button type="button" data-initial={mode !== "start" && mode !== "edit" || undefined} disabled={locked} onClick={close}>{mode === "close" ? "Keep tracking" : mode === "retry" ? "Back" : mode === "subagent_cancel" ? "Keep running" : "Cancel"}</button>
      {dialog.mode === "retry" ? <button type="button" disabled={locked || !available} aria-describedby={!available || locked ? primaryReasonId : undefined} onClick={() => { if (locked || inFlight.current || !available) return; onCheck(latestRun ?? dialog.run); onClose(); }}>Check again first</button> : null}
      <button type="submit" form={formId} data-primary aria-describedby={primaryReason ? primaryReasonId : undefined} disabled={locked || !available || unconfirmed && mode !== "edit" || mode === "edit" && !currentTask}>{pending ? "Working…" : dialog.mode === "retry" ? "Restart anyway" : dialog.mode === "edit" ? "Save task" : dialog.mode === "subagent_cancel" ? "Request cancellation" : dialog.mode === "setup_recovery" ? dialog.run.dispatch?.recovery === "accept_existing_worktree" ? "Use existing worktree" : dialog.run.dispatch?.recovery === "retry_environment" ? "Retry setup" : "Check setup" : title}</button>
    </footer>
  </section></div>, document.body);
}
