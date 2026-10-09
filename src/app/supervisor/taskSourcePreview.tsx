import type { OrchestrationSnapshot, TaskView } from "../../protocol/generated/v1";
import type { TaskSourceDialogState } from "./SupervisorDialogs";
import { taskContentReason, uniqueTask } from "./dependencies";

type TaskSourcePreviewProps = {
  dialog: TaskSourceDialogState; snapshot: OrchestrationSnapshot; current: TaskView | null; tasks: TaskView[];
  stale: boolean; sourceUncertain: boolean; locked: boolean; available: boolean; rootClosed: boolean;
  missingRead: boolean; acceptancePending: boolean; attemptsOpen: boolean; additions: string[];
  read(): Promise<void>; retrySameCreation(): Promise<void>; review(useCurrent: boolean): void;
  resolve(decision: "use_saved" | "keep_saved" | "apply_reviewed"): Promise<void>;
};

export function TaskSourcePreview({ dialog, snapshot, current, tasks, stale, sourceUncertain, locked, available, rootClosed,
  missingRead, acceptancePending, attemptsOpen, additions, read, retrySameCreation, review, resolve }: TaskSourcePreviewProps) {
  const base = dialog.mode === "follow_up" ? dialog.draft.baseSource : dialog.draft.baseTask;
  const saved = dialog.draft.reviewed;
  const submitted = dialog.draft.submitted?.action;
  const matches = saved && submitted ? submitted.action === "task_update"
    ? saved.title === submitted.title && saved.description === submitted.description
    : submitted.action === "task_dependencies_set"
      ? saved.depends_on.length === submitted.depends_on.length && saved.depends_on.every(id => submitted.depends_on.includes(id))
      : saved.title === submitted.title && saved.description === submitted.description && saved.follow_up_of === submitted.follow_up_of && JSON.stringify(saved.depends_on) === JSON.stringify(submitted.depends_on) : false;
  const comparisonTask = saved ?? current?.task;
  return stale || sourceUncertain ? <section className="supervisor-source-review"><p className="supervisor-warning">{sourceUncertain ? "Change unconfirmed. Read the original saved task before another write." : "Task or task file changed elsewhere. Review current description, steps and prerequisites before advancing either fence."}</p>
    {sourceUncertain ? <button type="button" disabled={locked} onClick={() => void read()}>Check saved {dialog.mode === "follow_up" ? "tasks" : "task"}</button> : null}
    {sourceUncertain && dialog.mode === "follow_up" && missingRead ? <button type="button" disabled={locked || !available || rootClosed} onClick={() => void retrySameCreation()}>Retry same follow-up</button> : null}
    {comparisonTask ? <div className="supervisor-dialog-comparison"><section><h3>{sourceUncertain ? "Original submission" : "Base task"}</h3><pre className="supervisor-plan">{sourceUncertain ? JSON.stringify(dialog.draft.submitted?.action, null, 2) : `${base.title}\n${base.description}\nSteps ${base.step_progress ? `${base.step_progress.done}/${base.step_progress.total}` : "unavailable"}\nPrerequisites ${base.depends_on.join(", ")}`}</pre></section><section><h3>Current saved task</h3><h4>{comparisonTask.title}</h4><p className="supervisor-exact-text">{comparisonTask.description}</p><p>Steps {comparisonTask.step_progress ? `${comparisonTask.step_progress.done}/${comparisonTask.step_progress.total}` : "unavailable"}</p><ul>{comparisonTask.steps.map((step, index) => <li key={step.step_id ?? `untracked-${index}`}>{step.status} · {step.title}</li>)}</ul><p>Prerequisites: {comparisonTask.depends_on.map(id => uniqueTask(tasks, id)?.task.title ?? id).join(" · ") || "none"}</p>{tasks.flatMap(task => task.dependencies.problems).map((problem, index) => <p key={index} className="supervisor-warning">{problem.message}</p>)}</section></div> : null}
    {!sourceUncertain && current ? <div className="supervisor-action-row"><button type="button" disabled={locked} onClick={() => review(false)}>{dialog.mode === "follow_up" ? "Review current source and task file" : dialog.mode === "relations" ? "Apply my changes to current" : "Keep my draft for reviewed save"}</button>{dialog.mode !== "follow_up" ? <button type="button" disabled={locked} onClick={() => review(true)}>Use current</button> : null}</div> : null}
    {sourceUncertain && saved ? <><p>{matches ? "Saved state matches your submission. This does not prove which actor wrote it." : "Saved state differs from your submission."}</p><button type="button" disabled={locked} onClick={() => void resolve(matches ? "use_saved" : "keep_saved")}>{matches ? "Use saved state" : "Keep saved state"}</button>{!matches && dialog.mode !== "follow_up" ? <button type="button" disabled={locked || !current || !!taskContentReason(snapshot, current, available) && dialog.mode === "edit" || dialog.mode === "relations" && (acceptancePending || attemptsOpen && additions.length > 0)} onClick={() => void resolve("apply_reviewed")}>Apply reviewed change</button> : null}</> : null}
  </section> : null;
}
