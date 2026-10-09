import { useMemo } from "react";
import { agentState } from "./SupervisorActions";
import { taskContentReason } from "./dependencies";
import { buildSupervisorGraph, selectionNodeId } from "./topology";
import { GRAPH_GEOMETRY } from "./graphLayout";
import { panelBounds, panelPlacement, type PanelKind, type SupervisorLayout } from "./useSupervisorLayout";
import type { SupervisorViewInputs, SupervisorViewModel, SupervisorViewPanel } from "./supervisorViewTypes";

export function useSupervisorViewModel(context: SupervisorViewInputs): SupervisorViewModel {
  const { snapshot, rootId, connected, runtimeLive, session, scope, hoverSpace, sessionId } = context;
  const root = snapshot?.runs.find(run => run.run_id === (rootId ?? snapshot.board?.root_id) && run.run_id === run.root_id && (run.stage !== "closed" || rootId === run.run_id)) ?? null;
  const openRoots = snapshot?.roots.filter(summary => snapshot.runs.some(run => run.run_id === summary.root_id && run.stage !== "closed")) ?? [];
  const closedRoots = snapshot?.roots.filter(summary => snapshot.runs.some(run => run.run_id === summary.root_id && run.stage === "closed")) ?? [];
  const orphanedWorkers = snapshot?.runs.filter(run => run.stage !== "closed" && !!run.parent_run_id && closedRoots.some(summary => summary.root_id === run.root_id) && (!root || run.root_id === root.run_id)) ?? [];
  const live = connected && runtimeLive && snapshot?.runtime.status === "fresh";
  const destination = runtimeLive && session ? session.spaces.find(space => space.id === session.focused_space_id) : undefined;
  const rootState = root && snapshot ? agentState(snapshot, root, connected, runtimeLive) : null;
  const tasks = root && snapshot?.board?.root_id === root.run_id ? snapshot.board.tasks : [];
  const rootRuns = root ? snapshot?.runs.filter(run => run.root_id === root.run_id) ?? [] : [];
  const selectedAgent = rootRuns.find(run => run.run_id === scope.selectedRun) ?? null;
  // Saved location remains useful, but observed status helpers independently enforce freshness.
  const observations = snapshot?.runtime.status === "fresh" ? snapshot.runtime.runs : [];
  const observe = (runId: string | null | undefined) => observations.find(item => item.run_id === runId);
  const selectedTask = tasks.find(task => task.task.task_id === scope.selectedTask) ?? null;
  const detailRun = selectedTask ? snapshot?.runs.find(run => run.run_id === selectedTask.current_run_id) ?? null : selectedAgent;
  const detailTask = selectedTask ?? tasks.find(task => task.task.task_id === detailRun?.task_id) ?? null;
  const detailSubagent = !selectedTask ? snapshot?.subagents.find(agent => agent.run_id === selectedAgent?.run_id && agent.subagent_id === scope.selectedSubagent) ?? null : null;
  const hasRetainedDraft = (taskId: string) => scope.steps.has(taskId) || scope.edits.has(taskId) || scope.relations.has(taskId) || scope.followUps.has(taskId);
  const retainedDraftTaskId = scope.selectedTask && hasRetainedDraft(scope.selectedTask) ? scope.selectedTask : null;
  const detailOpen = !!selectedTask || !!selectedAgent || !!retainedDraftTaskId;
  const stepTaskId = !detailSubagent ? detailTask?.task.task_id ?? retainedDraftTaskId : null;
  const detailTaskScope = root && stepTaskId ? { sessionId, rootId: root.run_id, taskId: stepTaskId } : null;
  const contentReason = snapshot ? taskContentReason(snapshot, detailTask, !!live) : "The task document is unavailable.";
  const stepReadOnlyReason = contentReason ?? (detailTask?.task.step_progress === null ? "The saved checklist structure is unsafe to edit. Resolve its source diagnostics first." : null);
  const sharedSpace = hoverSpace ?? observe(selectedAgent?.run_id)?.workspace_id ?? observe(selectedTask?.current_run_id)?.workspace_id;
  const mode = scope.view.mode;
  const model = useMemo(() => snapshot && root ? buildSupervisorGraph({ snapshot, root, rootId: root.run_id, tasks, includeSubagents: scope.showSubagents }) : null, [snapshot, root, scope.showSubagents]);
  const selectedNode = selectionNodeId(scope);
  const observedCount = rootRuns.filter(run => run.stage !== "closed" && live && observe(run.run_id)?.presence === "present" && observe(run.run_id)?.actual_omp).length;
  const rootSpace = observe(root?.run_id)?.workspace_id ?? (root?.target?.target === "existing_space" ? root.target.workspace_id : null);
  const banner = !connected || !runtimeLive ? "Connection lost · showing saved tasks · agent may still be running" : snapshot?.runtime.status !== "fresh" ? "Cannot check agents right now · showing saved tasks" : null;
  return { root, openRoots, closedRoots, orphanedWorkers, live, destination, rootState, tasks, rootRuns, selectedAgent, observations, observe, selectedTask, detailRun, detailTask, detailSubagent, hasRetainedDraft, retainedDraftTaskId, detailOpen, stepTaskId, detailTaskScope, contentReason, stepReadOnlyReason, sharedSpace, mode, model, selectedNode, observedCount, rootSpace, banner };
}

export function supervisorViewPanel(context: Pick<SupervisorViewInputs, "scope" | "attentionOpen" | "graphViewportHeight"> & Pick<SupervisorViewModel, "detailOpen" | "mode"> & { layout: SupervisorLayout }): SupervisorViewPanel {
  const { scope, attentionOpen, graphViewportHeight, detailOpen, mode, layout } = context;
  const panelKind: PanelKind | null = attentionOpen && layout.queueMode === "overlay" ? "attention" : detailOpen ? "details" : scope.disclosures.diagnostics ? "diagnostics" : scope.disclosures.history ? "activity" : null;
  const sheetMax = graphViewportHeight === null ? null : Math.floor(graphViewportHeight - GRAPH_GEOMETRY.headerHeight - GRAPH_GEOMETRY.nodeHeight - GRAPH_GEOMETRY.padding);
  const requestedPlacement = panelKind ? panelPlacement(layout, mode, panelKind) : null;
  const placement = requestedPlacement === "sheet" && sheetMax !== null && sheetMax < 160 ? "overlay" : requestedPlacement;
  const bounds = placement ? panelBounds(layout, placement, scope.view, mode !== "tasks" ? sheetMax : null) : null;
  const bottomInset = placement === "sheet" ? bounds?.value ?? 0 : 0;
  return { layout, panelKind, placement, bounds, bottomInset };
}
