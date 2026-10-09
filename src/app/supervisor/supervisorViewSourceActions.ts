import type { Run, TaskView } from "../../protocol/generated/v1";
import type { SupervisorViewInputs, SupervisorViewModel, SupervisorViewActions } from "./supervisorViewTypes";

export function useSupervisorViewSourceActions(context: SupervisorViewInputs & SupervisorViewModel): Omit<SupervisorViewActions, "started" | "start" | "restart"> {
  const { snapshot, live, busy, setTerminalError, onTerminal, connected, runtimeLive, refresh, mutateResult, root, scope, sessionId, setDialog } = context;
  const navigate = async (run: Run) => {
    if (!snapshot || !live || busy) return;
    try { setTerminalError(null); await onTerminal(run, snapshot); }
    catch (cause) { setTerminalError(`Could not open terminal. ${cause instanceof Error ? cause.message : "Check its current location and try again."}`); }
  };
  const check = (run: Run) => { if (!connected || !runtimeLive || !run.dispatch) { refresh(); return; } void mutateResult({ action: "reconcile_run", run_id: run.run_id, recovery: null }); };
  const edit = (task: TaskView) => {
    if (!root) return;
    let draft = scope.edits.get(task.task.task_id);
    if (!draft) { draft = { title: task.task.title, description: task.task.description, revision: task.task.task_revision, baseTask: task.task, baseView: task, submitted: null, reviewed: null }; scope.edits.set(task.task.task_id, draft); }
    setDialog({ mode: "edit", task, draft, scope: { sessionId, rootId: root.run_id, taskId: task.task.task_id } });
  };
  const editPrerequisites = (task: TaskView) => {
    if (!root || !snapshot?.board) return;
    let draft = scope.relations.get(task.task.task_id);
    if (!draft) { draft = { dependsOn: [...task.task.depends_on], baseSet: [...task.task.depends_on], baseTask: task.task, baseView: task, revision: task.task.task_revision, docRevision: snapshot.board.doc_revision, query: "", submitted: null, reviewed: null }; scope.relations.set(task.task.task_id, draft); }
    setDialog({ mode: "relations", task, draft, scope: { sessionId, rootId: root.run_id, taskId: task.task.task_id } });
  };
  const createFollowUp = (task: TaskView) => {
    if (!root || !snapshot?.board) return;
    let draft = scope.followUps.get(task.task.task_id);
    if (!draft) { draft = { taskId: crypto.randomUUID(), title: "", description: "", waitForSource: true, baseSource: task.task, baseView: task, docRevision: snapshot.board.doc_revision, submitted: null, reviewed: null }; scope.followUps.set(task.task.task_id, draft); }
    setDialog({ mode: "follow_up", task, draft, scope: { sessionId, rootId: root.run_id, taskId: task.task.task_id } });
  };
  const resumeSourceDraft = (taskId: string, kind: "edit" | "relations" | "follow_up") => {
    if (!root) return;
    const originalScope = { sessionId, rootId: root.run_id, taskId };
    if (kind === "edit") { const draft = scope.edits.get(taskId); if (draft) setDialog({ mode: "edit", task: draft.baseView, draft, scope: originalScope }); }
    else if (kind === "relations") { const draft = scope.relations.get(taskId); if (draft) setDialog({ mode: "relations", task: draft.baseView, draft, scope: originalScope }); }
    else { const draft = scope.followUps.get(taskId); if (draft) setDialog({ mode: "follow_up", task: draft.baseView, draft, scope: originalScope }); }
  };
  return { navigate, check, edit, editPrerequisites, createFollowUp, resumeSourceDraft };
}
