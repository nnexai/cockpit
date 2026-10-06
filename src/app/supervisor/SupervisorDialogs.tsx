import { useEffect, useId, useRef, useState, type KeyboardEvent } from "react";
import { createPortal } from "react-dom";
import type { DispatchTarget, OrchestrationAction, OrchestrationActionResult, OrchestrationSnapshot, Run, SpaceSummary, Subagent, TaskView } from "../../protocol/generated/v1";
import { useRestoreFocus } from "../library/LibraryConfirmDialog";
import { ErrorSlot } from "../ErrorSlot";
import { UiIcon } from "../UiIcon";
import type { EditDraft } from "./useSupervisorDrafts";
import "../projects/setup.css";

export type StartDraft = { label: string; location: "existing" | "directory" | "dedicated"; spaceId: string; directory: string };
export type SupervisorDialogState = { mode: "start" } | { mode: "edit"; task: TaskView; draft: EditDraft } | { mode: "retry" | "close" | "setup_recovery"; run: Run } | { mode: "subagent_cancel"; run: Run; subagent: Subagent };
export function SupervisorDialogs({ dialog, snapshot, spaces, startDraft, changed, busy, available, mutateResult, onStarted, onEdited, onStartUnconfirmed, onClose }: {
  dialog: SupervisorDialogState; snapshot: OrchestrationSnapshot; spaces: SpaceSummary[]; startDraft: StartDraft;
  changed(): void; busy: boolean; available: boolean; mutateResult(action: OrchestrationAction): Promise<OrchestrationActionResult | null>;
  onStarted(runId: string): void; onEdited(taskId: string): void; onStartUnconfirmed(): void; onClose(): void;
}) {
  const titleId = useId();
  const formId = useId();
  const ref = useRef<HTMLElement>(null);
  const inFlight = useRef(false);
  const [pending, setPending] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [unconfirmed, setUnconfirmed] = useState(false);
  const locked = busy || pending;
  const mode = dialog.mode;
  const title = mode === "start" ? "Start agent" : mode === "edit" ? "Edit task" : mode === "retry" ? "Restart agent" : mode === "close" ? "Close tracking" : mode === "setup_recovery" ? "Recover setup" : "Cancel subagent";
  useRestoreFocus();
  useEffect(() => {
    ref.current?.querySelector<HTMLElement>(mode === "start" && startDraft.location === "existing" && spaces.some(space => space.id === startDraft.spaceId) ? "[data-primary]" : "[data-initial]")?.focus({ preventScroll: true });
  }, []); // Opening owns focus once; polling must not move it.
  const close = () => { if (!inFlight.current && !busy) onClose(); };
  const keys = (event: KeyboardEvent<HTMLElement>) => {
    if (event.nativeEvent.isComposing) return;
    if (event.key === "Escape") { event.preventDefault(); event.stopPropagation(); close(); return; }
    if (event.key !== "Tab") return;
    event.stopPropagation();
    const elements = [...event.currentTarget.querySelectorAll<HTMLElement>("button,input,textarea,select,[tabindex]:not([tabindex='-1'])")].filter(element => !element.matches(":disabled,[hidden]") && !element.closest("[hidden],[inert]"));
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
        if (!spaces.some(space => space.id === startDraft.spaceId)) { setError("Choose a currently available Space. No different destination will be selected automatically."); return; }
        target = { target: "existing_space", workspace_id: startDraft.spaceId };
      } else if (startDraft.location === "directory") {
        if (!startDraft.directory.trim().startsWith("/")) { setError("Enter an absolute directory path."); return; }
        target = { target: "setup", request: { operation: "open", path: startDraft.directory.trim(), label: startDraft.label.trim() || null, task_name: null, focus: false } };
      }
      action = { action: "supervisor_start", target, label: startDraft.label.trim() || null };
    } else if (dialog.mode === "edit") {
      if (!dialog.draft.title.trim()) { setError("Enter a task title."); return; }
      if (!snapshot.board) { setError("The task document is unavailable. Your edit draft is kept."); return; }
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
    } catch (cause) { setError(cause instanceof Error ? cause.message : "The change was not confirmed. Your draft is kept."); }
    finally { inFlight.current = false; setPending(false); }
  };
  const dialogRun = "run" in dialog ? dialog.run : null;
  const editedTaskId = dialog.mode === "edit" ? dialog.task.task.task_id : null;
  const descendants = dialogRun ? snapshot.runs.filter(run => run.root_id === dialogRun.root_id && run.run_id !== dialogRun.run_id && run.stage !== "closed") : [];
  const currentTask = editedTaskId ? snapshot.board?.tasks.find(task => task.task.task_id === editedTaskId) : null;
  return createPortal(<div className="setup-overlay" role="presentation"><section ref={ref} className="supervisor-dialog" role="dialog" aria-modal="true" aria-labelledby={titleId} aria-busy={locked} tabIndex={-1} onKeyDown={keys}>
    <header className="supervisor-dialog-header"><span className="supervisor-dialog-icon"><UiIcon name={mode === "start" ? "plus" : mode === "edit" ? "edit" : mode === "retry" || mode === "setup_recovery" ? "refresh" : mode === "close" ? "close" : "stop"} /></span><h2 id={titleId}>{title}</h2></header><form id={formId} onSubmit={event => { event.preventDefault(); void submit(); }}>
      {dialog.mode === "start" ? <>
        <p>An OMP supervisor manages tasks and delegates work for you.</p>
        <label>Name (optional)<input value={startDraft.label} disabled={locked} onChange={event => { startDraft.label = event.target.value; changed(); }} /></label>
        <label>Location<select data-initial value={startDraft.location} disabled={locked} onChange={event => { startDraft.location = event.target.value as StartDraft["location"]; changed(); }}><option value="existing">Existing Space</option><option value="directory">Directory</option><option value="dedicated">Dedicated agent folder</option></select></label>
        {startDraft.location === "existing" ? <label>Space<select value={startDraft.spaceId} disabled={locked} onChange={event => { startDraft.spaceId = event.target.value; changed(); }}><option value="">Choose a Space</option>{spaces.map(space => <option key={space.id} value={space.id}>{space.label}</option>)}</select></label> : startDraft.location === "directory" ? <label>Absolute directory<input value={startDraft.directory} disabled={locked} onChange={event => { startDraft.directory = event.target.value; changed(); }} /></label> : <p>Creates a dedicated agent folder and Space. Existing project context is not copied.</p>}
        <p>Starts OMP without switching your terminal focus.</p>
      </> : dialog.mode === "edit" ? <>
        <label>Task title<input data-initial value={dialog.draft.title} disabled={locked} onChange={event => { dialog.draft.title = event.target.value; changed(); }} /></label>
        <label>Task description<textarea rows={8} value={dialog.draft.body} disabled={locked} onChange={event => { dialog.draft.body = event.target.value; changed(); }} /></label>
        {currentTask?.task.task_revision !== dialog.draft.revision ? <><p className="supervisor-warning">Task changed elsewhere. Saving will not overwrite those changes without your review. Your draft is kept.</p>{currentTask ? <details><summary>Review current task</summary><h3>{currentTask.task.title}</h3><p className="supervisor-exact-text">{currentTask.task.body}</p><p>Keeping your draft for the next save replaces the current task text only if this reviewed version is still current.</p><button type="button" disabled={locked || !!currentTask.task.diagnostic} onClick={() => { dialog.draft.revision = currentTask.task.task_revision; changed(); }}>I reviewed the current task; keep my draft for saving</button></details> : null}</> : null}
      </> : dialog.mode === "retry" ? <>
        <p>Restart {dialog.run.label}?</p><p>We cannot confirm whether the previous agent is still running. Restarting creates a new terminal and could leave another agent running. The previous launch is checked again before a new launch.</p>{descendants.length ? <p>Worker agents may still be running. Restart only this agent; keep their tasks and history.</p> : null}
      </> : dialog.mode === "close" ? <>
        <p>Close tracking for {dialog.run.label}?</p><p>Tasks, history, Spaces and worktrees are kept. This does not guarantee the agent or its workers stop.</p>{descendants.length ? <p>{descendants.length} other tracked agents stay open. Live descendants will still need supervision.</p> : null}
      </> : dialog.mode === "subagent_cancel" ? <p>Request cancellation of {dialog.subagent.label}. Its OMP control receipt, not this request, confirms whether it stopped.</p> : <>
        <p>Setup for {dialog.run.label} needs review before launch. Existing resources are kept.</p><p>{dialog.run.dispatch?.recovery === "accept_existing_worktree" ? "Confirm using the existing worktree receipt rather than creating another worktree." : dialog.run.dispatch?.recovery === "retry_environment" ? "Explicitly retry the uncertain setup operation after reviewing its recorded effects." : "Check and reconcile the existing setup only. No new launch is requested by this confirmation."}</p>
        {dialog.run.prepare_plan ? <details><summary>Exact setup plan</summary><p className="supervisor-exact-text">{dialog.run.prepare_plan.text}</p></details> : null}{dialog.run.setup ? <><p>{dialog.run.setup.checkout_path}</p><ul>{dialog.run.setup.effects.map((effect, index) => <li key={index}>{effect}</li>)}</ul>{dialog.run.setup.warnings.map((warning, index) => <p className="supervisor-warning" key={index}>{warning}</p>)}</> : null}
      </>}
    </form>
    <ErrorSlot placement="dialog" message={error} className="supervisor-dialog-error-slot" />
    {!available ? <p className="supervisor-dialog-unavailable">Reconnect before applying this change. You can keep editing or close this dialog without losing your draft.</p> : null}<footer><button type="button" data-initial={mode !== "start" && mode !== "edit" || undefined} disabled={locked} onClick={close}>{mode === "close" ? "Keep tracking" : mode === "retry" ? "Back" : "Cancel"}</button><button type="submit" form={formId} data-primary disabled={locked || !available || unconfirmed && mode !== "edit"}>{pending ? "Working…" : mode === "retry" ? "Restart anyway" : mode === "edit" ? "Save task" : title}</button></footer>
  </section></div>, document.body);
}
