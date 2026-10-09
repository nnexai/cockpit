import type { ReactNode, RefObject } from "react";
import type { OrchestrationSnapshot, TaskView } from "../../protocol/generated/v1";
import { copyText } from "../library/clipboard";
import { ErrorSlot } from "../ErrorSlot";
import { UiIcon } from "../UiIcon";
import type { TaskSourceDialogState } from "./SupervisorDialogs";
import { dependencyCandidateReason, uniqueTask } from "./dependencies";

type TaskSourceFormProps = {
  dialog: TaskSourceDialogState; snapshot: OrchestrationSnapshot; current: TaskView | null; tasks: TaskView[];
  dialogRef: RefObject<HTMLElement | null>; reasonId: string; pickerId: string; children: ReactNode;
  locked: boolean; pending: boolean; sourceUncertain: boolean; available: boolean; rootClosed: boolean;
  attemptsOpen: boolean; acceptancePending: boolean; stale: boolean; docRevision: string | null;
  additions: string[]; removed: string[]; candidates: TaskView[]; validation: string | null; reason: string | null;
  pickerOpen: boolean; option: number; confirmRemoval: string | null; discard: boolean; error: string | null; notice: string | null;
  changed(): void; save(): Promise<void>; resolve(decision: "use_saved" | "keep_saved" | "apply_reviewed"): Promise<void>;
  setPickerOpen(value: boolean): void; setOption(value: number): void; setConfirmRemoval(value: string | null): void;
  setNotice(value: string | null): void; setDiscard(value: boolean): void; onDiscard(): void; onClose(): void;
};

export function TaskSourceForm({ dialog, snapshot, current, tasks, dialogRef, reasonId, pickerId, children,
  locked, pending, sourceUncertain, available, rootClosed, attemptsOpen, acceptancePending, stale, docRevision,
  additions, removed, candidates, validation, reason, pickerOpen, option, confirmRemoval, discard, error, notice,
  changed, save, resolve, setPickerOpen, setOption, setConfirmRemoval, setNotice, setDiscard, onDiscard, onClose }: TaskSourceFormProps) {
  return <form onSubmit={event => { event.preventDefault(); void save(); }}>
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
      {pickerOpen && !attemptsOpen && !sourceUncertain ? <ul id={pickerId} role="listbox" aria-label="Available prerequisite tasks" className="supervisor-prerequisite-picker">{candidates.map((candidate, index) => { const candidateReason = dependencyCandidateReason(tasks, dialog.scope.taskId, candidate.task.task_id); return <li id={`${pickerId}-${index}`} role="option" aria-selected={index === option} aria-disabled={!!candidateReason} key={candidate.task.task_id}><button type="button" tabIndex={-1} disabled={!!candidateReason || locked || dialog.draft.dependsOn.length >= 32} onClick={() => { dialog.draft.dependsOn.push(candidate.task.task_id); dialog.draft.query = ""; setOption(0); changed(); dialogRef.current?.querySelector<HTMLInputElement>('[role="combobox"]')?.focus(); }}>{candidate.task.title} · {candidate.task.checked ? "Accepted" : "Open"}{candidateReason ? ` · ${candidateReason}` : ""}</button></li>; })}</ul> : null}
      {attemptsOpen ? <p>Work is tracked: remove existing prerequisites only. Removal deliberately changes sequencing.</p> : null}
      {attemptsOpen && additions.length ? <button type="button" disabled={locked || sourceUncertain} onClick={() => { if (current) { dialog.draft.dependsOn = dialog.draft.dependsOn.filter(id => current.task.depends_on.includes(id)); setConfirmRemoval(null); changed(); } }}>Keep only my removals</button> : null}
    </> : <>
      <label>Task title<input data-initial value={dialog.draft.title} readOnly={locked || sourceUncertain} aria-invalid={!!validation} aria-describedby={validation ? reasonId : undefined} onChange={event => { dialog.draft.title = event.target.value; changed(); }} /></label>
      <label>Task description<textarea rows={8} value={dialog.draft.description} readOnly={locked || sourceUncertain} onChange={event => { dialog.draft.description = event.target.value; changed(); }} /></label>
      {dialog.mode === "follow_up" ? <><label className="supervisor-followup-wait"><input type="checkbox" checked={dialog.draft.waitForSource} disabled={locked || sourceUncertain} onChange={event => { dialog.draft.waitForSource = event.target.checked; changed(); }} />Also wait for “{dialog.draft.baseSource.title}” to be accepted</label><p>{current?.task.checked ? "The source is already accepted, so this prerequisite is satisfied now. Uncheck for an independent follow-up." : "Provenance alone does not block the follow-up."} Creating a task does not assign it. The supervisor assigns work.</p></> : <p>Saving title/description preserves saved steps and prerequisites.</p>}
    </>}
    {children}
    {confirmRemoval ? <section className="supervisor-source-confirmation"><p>Removing these prerequisites changes when this task may continue: {removed.map(id => uniqueTask(tasks, id)?.task.title ?? id).join(" · ")}</p><button type="button" data-keep-prerequisites onClick={() => setConfirmRemoval(null)}>Keep prerequisites</button><button type="button" disabled={locked || !available || rootClosed || !sourceUncertain && stale || sourceUncertain && current?.task.task_revision !== dialog.draft.reviewed?.task_revision || !!validation || acceptancePending || additions.length > 0 || removed.length === 0 || confirmRemoval !== JSON.stringify([dialog.mode === "relations" ? dialog.draft.reviewed?.task_revision ?? dialog.draft.revision : "", docRevision, dialog.mode === "relations" ? dialog.draft.dependsOn : []])} onClick={() => sourceUncertain ? void resolve("apply_reviewed") : void save()}>Remove prerequisites</button></section> : null}
    <ErrorSlot placement="dialog" message={error} className="supervisor-dialog-error-slot" />
    {notice ? <p role="status">{notice}</p> : null}{reason || validation ? <p id={reasonId} className="supervisor-disabled-reason">{validation ?? reason}</p> : null}
    <div className="supervisor-dialog-draft-actions"><button type="button" disabled={locked} onClick={async () => setNotice(await copyText(dialog.mode === "relations" ? dialog.draft.dependsOn.join("\n") : `${dialog.draft.title}\n\n${dialog.draft.description}`) ? "Draft copied." : "Could not copy. Select draft text and copy manually.")}>Copy draft</button><button type="button" disabled={locked || sourceUncertain} onClick={() => setDiscard(true)}>Discard draft…</button></div>
    {discard ? <p>Discard these unsaved changes? <button type="button" onClick={() => setDiscard(false)}>Keep draft</button><button type="button" onClick={() => { onDiscard(); onClose(); }}>Discard</button></p> : null}
    <footer><button type="button" disabled={locked} onClick={onClose}>Cancel</button><button type="submit" data-primary disabled={!!reason || !!validation || !!confirmRemoval} aria-describedby={reason || validation ? reasonId : undefined}>{pending ? "Saving…" : dialog.mode === "edit" ? "Save task" : dialog.mode === "relations" ? attemptsOpen ? "Remove prerequisites…" : "Save prerequisites" : "Create follow-up"}</button></footer>
  </form>;
}
