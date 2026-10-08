import { useEffect, useLayoutEffect, useMemo, useRef, useState, type CSSProperties } from "react";
import type { CockpitClient } from "../../client/CockpitClient";
import type { AttentionKind, OrchestrationSnapshot, Run, SessionSnapshotResponse, TaskLane, TaskView } from "../../protocol/generated/v1";
import { useRovingList } from "../sidebar/useRovingList";
import { UiIcon } from "../UiIcon";
import { SupervisorGraph } from "./SupervisorGraph";
import { taskLanes, taskNeighbor, visibleTaskIds } from "./boardNavigation";
import { agentState, questionForRun, questionLabel, recoveryActions, SupervisorActions, taskStatus, TextAction, type PathRowView } from "./SupervisorActions";
import { SupervisorDialogs, TaskSourceDialog, type StartDraft, type SupervisorDialogState } from "./SupervisorDialogs";
import { messageDraft, stepDraft, useSupervisorDrafts } from "./useSupervisorDrafts";
import { useSupervisor } from "./useSupervisor";
import { deriveAttention, rootAttentionSummary, TIER_LABEL, TIER_ORDER, type AttentionTier, type LocalCondition } from "./attention";
import { AttentionQueue, SupervisorSummary, type QueueAction, type QueueRowView } from "./SupervisorAttention";
import { activityRows, ClosedTracking, SupervisorActivity, SupervisorDiagnostics, type ClosedTaskCount } from "./SupervisorActivity";
import { AgentsStrip, TaskCard, type StripChip } from "./SupervisorTasks";
import { buildSupervisorGraph, nodeFacts, nodeSelection, pathNodes, selectionNodeId, type TopologyNode } from "./topology";
import { PanelSplitter } from "./PanelSplitter";
import { GRAPH_GEOMETRY } from "./graphLayout";
import { panelBounds, panelPlacement, queueCap, useSupervisorLayout, type PanelKind } from "./useSupervisorLayout";
import { isCovered, readOffset, revealNearest, writeOffset } from "./reveal";
import { retirementView } from "./retirementView";
import { SupervisorDependencies } from "./SupervisorDependencies";
import { dependentCount, taskContentReason } from "./dependencies";
import "./supervisor.css";

const ATTENTION_PROBLEM: Record<AttentionKind, string> = {
  needs_input: "Needs your answer",
  runtime_blocked: "Blocked without a question",
  dispatch_unknown: "Dispatch unconfirmed",
  exited_without_report: "Exited without a Result",
  idle_without_report: "Idle without a Result",
  brief_unread: "Brief not read yet",
  plan_changed: "Plan changed",
  intent_conflict: "Task changed during acceptance",
  awaits_prepare: "Waiting for supervisor",
  awaits_execute: "Waiting for supervisor",
  to_accept: "Supervisor is reviewing",
  retirement_unconfirmed: "Worker retirement unconfirmed",
};

export function SupervisorView({ client, sessionId, session, runtimeLive, active, startToken, navigationError, onClose, onTerminal, onModalChange }: {
  client: CockpitClient; sessionId: string; session: SessionSnapshotResponse | null; runtimeLive: boolean; active: boolean; startToken: number;
  navigationError: string | null; onClose(): void; onTerminal(run: Run, snapshot: OrchestrationSnapshot): Promise<void>; onModalChange(open: boolean): void;
}) {
  const [rootId, setRootId] = useState<string | null>(null);
  const { snapshot, error, connected, busy, mutateResult, refresh, submitStep, submitTask, readSaved, resolveUnknown, resolveSourceUnknown, taskWriteUnconfirmed } = useSupervisor(client, sessionId, rootId, active);
  const drafts = useSupervisorDrafts(sessionId);
  const scope = drafts.scope(rootId ?? snapshot?.board?.root_id ?? null);
  const changed = drafts.changed;
  const [dialog, setDialog] = useState<SupervisorDialogState | null>(null);
  const [terminalError, setTerminalError] = useState<string | null>(null);
  const [notice, setNotice] = useState<string | null>(null);
  const [startPending, setStartPending] = useState(false);
  const [startUnknown, setStartUnknown] = useState(false);
  const startLock = useRef(false);
  const [pendingRestart, setPendingRestart] = useState<string | null>(null);
  const [hoverSpace, setHoverSpace] = useState<string | null>(null);
  const [hoverRun, setHoverRun] = useState<string | null>(null);
  const [detailSection, setDetailSection] = useState<"overview" | "activity" | "actions">("overview");
  const [attentionOpen, setAttentionOpen] = useState(false);
  const [focusTier, setFocusTier] = useState<AttentionTier | null>(null);
  const [focusNotice, setFocusNotice] = useState<string | null>(null);
  const [inlineQueueCap, setInlineQueueCap] = useState<number | null>(null);
  const [graphViewportHeight, setGraphViewportHeight] = useState<number | null>(null);
  const [archiveCounts, setArchiveCounts] = useState<ReadonlyMap<string, ClosedTaskCount>>(new Map());
  const archiveGeneration = useRef(0);
  const detailInvoker = useRef<HTMLElement | null>(null);
  const panelInvoker = useRef<HTMLElement | null>(null);
  const counterInvoker = useRef<HTMLElement | null>(null);
  const returnFocusIntent = useRef<{ invoker: Element | null } | null>(null);
  const startDraft = useRef<StartDraft>({ label: "", location: "existing", spaceId: "", directory: "" });
  const lastSession = useRef(sessionId);
  const seenStart = useRef(0);
  const rootRef = useRef<HTMLElement>(null);
  const workareaRef = useRef<HTMLDivElement>(null);
  const graphRef = useRef<HTMLDivElement>(null);
  const laneRefs = useRef<Partial<Record<TaskLane, HTMLUListElement | null>>>({});
  const focusedTask = useRef<string | null>(null);
  const focusedGraph = useRef<string | null>(null);
  const focusWithinDetail = useRef(false);
  const questionHadFocus = useRef(false);
  const startFocus = useRef<{ runId: string | null; invoker: Element | null } | null>(null);
  const createdSelection = useRef<{ sessionId: string; rootId: string; taskId: string; focus: boolean } | null>(null);
  const lastTaskIds = useRef<string[]>([]);
  const opened = useRef(false);
  const initialRootSnapshot = useRef<OrchestrationSnapshot | null>(null);
  const revealIntent = useRef<{ focus: boolean } | null>(null);
  const explicitFocus = useRef(false);
  const previousGraph = useRef<{ scope: typeof scope; ids: readonly string[] } | null>(null);
  const layout = useSupervisorLayout(workareaRef);
  const mode = scope.view.mode;
  const modalVisible = !!dialog && active && !!snapshot;
  useEffect(() => { onModalChange(modalVisible); return () => onModalChange(false); }, [modalVisible, onModalChange]);
  useEffect(() => {
    if (lastSession.current === sessionId) return;
    lastSession.current = sessionId; setRootId(null); setDialog(null); setTerminalError(null); setNotice(null); setStartUnknown(false); setPendingRestart(null); setAttentionOpen(false);
    startDraft.current = { label: "", location: "existing", spaceId: "", directory: "" };
  }, [sessionId]);
  useEffect(() => { scope.view.detailTrail = []; changed(); }, [sessionId, rootId]);
  const root = snapshot?.runs.find(run => run.run_id === (rootId ?? snapshot.board?.root_id) && run.run_id === run.root_id && (run.stage !== "closed" || rootId === run.run_id)) ?? null;
  const openRoots = snapshot?.roots.filter(summary => snapshot.runs.some(run => run.run_id === summary.root_id && run.stage !== "closed")) ?? [];
  const closedRoots = snapshot?.roots.filter(summary => snapshot.runs.some(run => run.run_id === summary.root_id && run.stage === "closed")) ?? [];
  const orphanedWorkers = snapshot?.runs.filter(run => run.stage !== "closed" && !!run.parent_run_id && closedRoots.some(summary => summary.root_id === run.root_id) && (!root || run.root_id === root.run_id)) ?? [];
  useEffect(() => {
    if (snapshot && !rootId && openRoots.length) { initialRootSnapshot.current = snapshot; setRootId(openRoots[0].root_id); }
  }, [snapshot, rootId]);
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
  const navOptions = { arrangement: layout.narrow ? "stacked" as const : "lanes" as const, completedOpen: scope.disclosures.completed, collapsedLanes: scope.view.collapsedLanes };
  const taskIds = visibleTaskIds(tasks, navOptions);
  const { listRef, listProps, tabIndexFor, focusRow } = useRovingList({ rowIds: taskIds, selectedId: scope.selectedTask, onEscape: () => { escapeLayer(); return true; } });
  const model = useMemo(() => snapshot && root ? buildSupervisorGraph({ snapshot, root, rootId: root.run_id, tasks, includeSubagents: scope.showSubagents }) : null, [snapshot, root, scope.showSubagents]);
  const selectedNode = selectionNodeId(scope);
  const panelKind: PanelKind | null = attentionOpen && layout.queueMode === "overlay" ? "attention" : detailOpen ? "details" : scope.disclosures.diagnostics ? "diagnostics" : scope.disclosures.history ? "activity" : null;
  const sheetMax = graphViewportHeight === null ? null : Math.floor(graphViewportHeight - GRAPH_GEOMETRY.headerHeight - GRAPH_GEOMETRY.nodeHeight - GRAPH_GEOMETRY.padding);
  const requestedPlacement = panelKind ? panelPlacement(layout, mode, panelKind) : null;
  const placement = requestedPlacement === "sheet" && sheetMax !== null && sheetMax < 160 ? "overlay" : requestedPlacement;
  const bounds = placement ? panelBounds(layout, placement, scope.view, mode !== "tasks" ? sheetMax : null) : null;
  const bottomInset = placement === "sheet" ? bounds?.value ?? 0 : 0;
  const rowElement = (id: string | null) => [...rootRef.current?.querySelectorAll<HTMLElement>("[data-row-id]") ?? []].find(element => element.dataset.rowId === id);
  const selectedVisibleId = () => mode !== "tasks" ? selectedNode : scope.selectedTask ?? (scope.selectedRun ? tasks.find(task => task.current_run_id === scope.selectedRun)?.task.task_id ?? null : null);
  const revealSelection = (focus: boolean, coveredOnly = false) => {
    const element = rowElement(selectedVisibleId());
    const scroller = mode !== "tasks" ? graphRef.current : layout.narrow ? listRef.current : element?.closest<HTMLUListElement>(".supervisor-task-list");
    if (!element || !scroller) {
      if (focus) rootRef.current?.querySelector<HTMLButtonElement>(`[data-view-segment="${mode}"]`)?.focus({ preventScroll: true });
      return;
    }
    const insets = mode !== "tasks" ? { top: 28, bottom: bottomInset } : {};
    if (!coveredOnly || isCovered(scroller, element, insets)) revealNearest(scroller, element, insets);
    if (focus) element.focus({ preventScroll: true });
  };
  const focusTaskOrStart = () => {
    if (mode === "dependencies") {
      const id = scope.selectedTask ?? tasks.find(task => !task.task.checked)?.task.task_id;
      if (id) rowElement(`task:${id}`)?.focus({ preventScroll: true });
      else rootRef.current?.querySelector<HTMLButtonElement>('[data-view-segment="dependencies"]')?.focus();
      return;
    }
    if (mode === "graph" && model?.nodes.length) {
      const id = selectedNode && model.byId.has(selectedNode) ? selectedNode : model.nodes[0].id;
      rowElement(id)?.focus({ preventScroll: true }); return;
    }
    const id = scope.selectedTask && taskIds.includes(scope.selectedTask) ? scope.selectedTask : taskIds[0];
    if (id) focusRow(id); else rootRef.current?.querySelector<HTMLButtonElement>("[data-start-agent]")?.focus({ preventScroll: true });
  };
  const saveOffsets = () => {
    const scroller = mode !== "tasks" ? graphRef.current : listRef.current;
    if (scroller) scope.view.offsets[mode] = readOffset(scroller);
    for (const { lane } of taskLanes) { const list = laneRefs.current[lane]; if (list && !layout.narrow) scope.view.laneScroll[lane] = list.scrollTop; }
  };
  useLayoutEffect(() => {
    if (!root || !active) return;
    const scroller = mode !== "tasks" ? graphRef.current : listRef.current;
    if (scroller && scope.view.offsets[mode]) writeOffset(scroller, scope.view.offsets[mode]!);
    if (mode === "tasks" && !layout.narrow) for (const { lane } of taskLanes) { const list = laneRefs.current[lane]; if (list) list.scrollTop = scope.view.laneScroll[lane] ?? 0; }
    revealSelection(revealIntent.current?.focus ?? false, true);
    revealIntent.current = null;
    return saveOffsets;
  }, [scope, mode, active, !!root]);
  useLayoutEffect(() => {
    if (!revealIntent.current || !active) return;
    revealSelection(revealIntent.current.focus); revealIntent.current = null;
  });
  useLayoutEffect(() => {
    const graph = graphRef.current, workarea = workareaRef.current;
    if (mode === "tasks" || !active || !graph || !workarea) { setGraphViewportHeight(null); return; }
    const measure = () => {
      const available = Math.min(graph.clientHeight, workarea.getBoundingClientRect().bottom - graph.getBoundingClientRect().top);
      const next = available > 0 ? available : null;
      setGraphViewportHeight(current => current === next ? current : next);
    };
    measure();
    const observer = typeof ResizeObserver === "undefined" ? null : new ResizeObserver(measure);
    observer?.observe(graph); observer?.observe(workarea);
    return () => observer?.disconnect();
  }, [mode, active, rootId, !!root, layout.width, layout.height, layout.queueMode]);
  useLayoutEffect(() => { if (detailOpen && mode !== "tasks") revealSelection(false, true); }, [layout.width, layout.height, bounds?.value, placement]);
  useEffect(() => {
    if (!active) { opened.current = false; return; }
    if (!snapshot || snapshot === initialRootSnapshot.current || opened.current || dialog) return;
    initialRootSnapshot.current = null; opened.current = true;
    if (placement === "overlay" && panelKind) rootRef.current?.querySelector<HTMLButtonElement>(".supervisor-detail-header button,.supervisor-queue-header button")?.focus({ preventScroll: true });
    else focusTaskOrStart();
  }, [active, snapshot, dialog, root]);
  useEffect(() => {
    if (!snapshot || snapshot.board?.root_id !== rootId || !active) return;
    const id = focusedTask.current;
    if (id && !taskIds.includes(id) && mode === "tasks") {
      const index = lastTaskIds.current.indexOf(id);
      if (scope.selectedTask === id && !hasRetainedDraft(id)) { scope.selectedTask = null; changed(); }
      const next = taskIds[Math.min(Math.max(index, 0), taskIds.length - 1)];
      if (next) focusRow(next); else rootRef.current?.querySelector<HTMLButtonElement>("[data-start-agent]")?.focus({ preventScroll: true });
      focusedTask.current = next ?? null; setFocusNotice(next ? "The focused task changed elsewhere. Focus moved to the next available task." : "The focused task changed elsewhere. Focus moved to Start agent.");
    }
    // Closed graph rows and removed canonical tasks are no longer selectable; kept Done tasks remain selectable in Tasks.
    const gone = scope.selectedTask && !tasks.some(task => task.task.task_id === scope.selectedTask) && !hasRetainedDraft(scope.selectedTask) && createdSelection.current?.taskId !== scope.selectedTask || scope.selectedRun && (!selectedAgent || selectedAgent.stage === "closed") || scope.selectedSubagent && !detailSubagent;
    const graphItemRemoved = mode === "graph" && selectedNode && previousGraph.current?.scope === scope && previousGraph.current.ids.includes(selectedNode) && !model?.byId.has(selectedNode) && !retainedDraftTaskId;
    if (gone || graphItemRemoved) {
      const hadFocus = focusWithinDetail.current || focusedGraph.current === selectedNode || !!rowElement(selectedNode)?.contains(document.activeElement);
      scope.selectedTask = null; scope.selectedRun = null; scope.selectedSubagent = null; changed();
      if (hadFocus) {
        const oldIndex = selectedNode ? previousGraph.current?.ids.indexOf(selectedNode) ?? 0 : 0;
        const next = model?.layout.order[Math.min(Math.max(oldIndex, 0), model.layout.order.length - 1)];
        requestAnimationFrame(() => {
          if (mode === "graph" && next) rowElement(next)?.focus({ preventScroll: true });
          else if (mode === "graph") rootRef.current?.querySelector<HTMLButtonElement>('[data-view-segment="graph"]')?.focus({ preventScroll: true });
          else focusTaskOrStart();
        });
      }
    }
    lastTaskIds.current = taskIds;
    previousGraph.current = { scope, ids: model?.layout.order ?? [] };
  }, [snapshot, active, mode]);
  const started = (runId: string) => { if (startFocus.current) startFocus.current.runId = runId; setRootId(runId); setStartUnknown(false); setNotice(null); };
  const start = async () => {
    if (!snapshot || !connected || busy || startLock.current || startUnknown || rootState?.kind === "starting") return;
    if (!destination) { setDialog({ mode: "start" }); return; }
    startFocus.current = { runId: null, invoker: document.activeElement };
    startLock.current = true; setStartPending(true); setNotice(null);
    try {
      const result = await mutateResult({ action: "supervisor_start", target: { target: "existing_space", workspace_id: destination.id }, label: null });
      if (result?.result === "run") started(result.run_id);
      else { setStartUnknown(true); setNotice("The start was not confirmed. Check status before starting another agent; a terminal may already have opened."); }
    } finally { startLock.current = false; setStartPending(false); }
  };
  useEffect(() => { if (startToken > seenStart.current && active && snapshot && connected) { seenStart.current = startToken; void start(); } }, [startToken, active, snapshot, connected]);
  useEffect(() => {
    if (!active || !rootState?.verified || !root || startFocus.current?.runId !== root.run_id) return;
    if (document.activeElement === startFocus.current.invoker) focusTaskOrStart(); startFocus.current = null;
  }, [active, root, rootState?.verified]);
  const navigate = async (run: Run) => {
    if (!snapshot || !live || busy) return;
    try { setTerminalError(null); await onTerminal(run, snapshot); }
    catch (cause) { setTerminalError(`Could not open terminal. ${cause instanceof Error ? cause.message : "Check its current location and try again."}`); }
  };
  const check = (run: Run) => { if (!connected || !runtimeLive || !run.dispatch) { refresh(); return; } void mutateResult({ action: "reconcile_run", run_id: run.run_id, recovery: null }); };
  const restart = async (run: Run) => {
    if (!connected || !runtimeLive || busy || !run.dispatch) return;
    if (run.dispatch.step === "setup_unknown") { setDialog({ mode: "setup_recovery", run }); return; }
    if (run.dispatch.step === "plan_failed") { await mutateResult({ action: "reconcile_run", run_id: run.run_id, recovery: null }); return; }
    if (run.dispatch.step === "launch_unknown" || run.dispatch.step === "needs_review") { setDialog({ mode: "retry", run }); return; }
    setNotice("Checking the previous launch before restart…");
    if (await mutateResult({ action: "reconcile_run", run_id: run.run_id, recovery: null })) setPendingRestart(run.run_id);
    else setNotice("The previous launch could not be checked. No new agent was requested; tasks and history are kept.");
  };
  useEffect(() => {
    if (!pendingRestart || !snapshot || !active) return;
    if (!connected || !runtimeLive || snapshot.runtime.status !== "fresh") { setPendingRestart(null); setNotice("The previous launch is unobserved. Check the connection before restarting. No new agent was requested."); return; }
    const run = snapshot.runs.find(item => item.run_id === pendingRestart);
    if (!run || run.stage === "closed") { setPendingRestart(null); return; }
    if (run.dispatch?.step === "launch_unknown" || run.dispatch?.step === "needs_review") { setPendingRestart(null); setNotice(null); setDialog({ mode: "retry", run }); }
    else if (agentState(snapshot, run, connected, runtimeLive).verified) { setPendingRestart(null); setNotice("The original OMP agent is still connected. No new launch was started."); }
  }, [pendingRestart, snapshot, active, connected, runtimeLive]);
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
  const assignmentIntents = snapshot?.assignment_intents.filter(intent => intent.root_id === root?.run_id) ?? [];
  const local: LocalCondition[] = [];
  if (startUnknown) local.push({ kind: "start_unknown" });
  if (terminalError) local.push({ kind: "terminal_error", message: terminalError });
  if (navigationError) local.push({ kind: "navigation_error" });
  if (error && connected) local.push({ kind: "change_unconfirmed", message: error });
  if (notice) local.push({ kind: "notice", message: notice });
  for (const run of orphanedWorkers) local.push({ kind: "orphaned_worker", runId: run.run_id });
  if (snapshot) for (const run of rootRuns) if (run.stage !== "closed" && !["proposed", "awaiting_prepare"].includes(run.stage) && ["failure", "missing", "unknown"].includes(agentState(snapshot, run, connected, runtimeLive).kind)) local.push({ kind: "agent_status", runId: run.run_id, taskId: run.task_id });
  if (root?.last_report?.outcome === "failed") local.push({ kind: "root_failed_report", runId: root.run_id });
  for (const intent of assignmentIntents) local.push({ kind: "assignment", taskId: intent.task_id, state: intent.state });
  if (root && snapshot?.board?.unidentified_items) local.push({ kind: "unidentified_items", count: snapshot.board.unidentified_items });
  for (const task of tasks) {
    if (task.dependencies.state === "invalid" || task.task.relations_diagnostic) local.push({ kind: "relation_diagnostic", taskId: task.task.task_id, cause: task.dependencies.problems.map(problem => problem.message).join(" · ") || task.task.relations_diagnostic || "Unreadable prerequisites." });
    const currentWorker = snapshot?.runs.find(run => run.run_id === task.current_run_id);
    if (currentWorker && currentWorker.stage !== "closed" && !task.task.checked && (currentWorker.grants.some(grant => grant.scope === "execute") || currentWorker.stage === "working" || currentWorker.stage === "reported") && ["blocked", "invalid"].includes(task.dependencies.state)) local.push({ kind: "dependency_regression", taskId: task.task.task_id });
  }
  const attention = snapshot ? deriveAttention({ snapshot, rootId: root?.run_id ?? null, local }) : null;
  const decide = attention?.items.some(item => item.tier === "decide" && item.runId === root?.run_id) ?? false;
  const rootQuestion = root && snapshot ? questionForRun(snapshot, root) : null;
  const rootReceiptLabel = root && rootQuestion && rootQuestion.receipt.status !== "unresolved"
    && rootState?.kind === "ready" && !rootState.blocked && attention?.tierForRun(root.run_id) !== "recover"
    ? questionLabel(rootQuestion) : null;
  const question = decide && rootQuestion?.receipt.status === "unresolved"
    && root?.last_report?.kind === "needs_input" && root.last_report.message_id === rootQuestion.question_message_id ? root.last_report : null;
  const answer = question && root ? messageDraft(scope, `answer:${root.run_id}:${question.message_id}`) : null;
  useEffect(() => {
    if (!active || !questionHadFocus.current || decide) return;
    questionHadFocus.current = false;
    if (attentionOpen) { setAttentionOpen(false); requestAnimationFrame(() => focusTaskOrStart()); }
    else focusTaskOrStart();
  }, [active, decide]);
  const closeAttention = () => { setAttentionOpen(false); requestAnimationFrame(() => { if (counterInvoker.current?.isConnected) counterInvoker.current.focus({ preventScroll: true }); }); };
  const closePanel = () => { scope.disclosures.history = false; scope.disclosures.diagnostics = false; changed(); requestAnimationFrame(() => panelInvoker.current?.isConnected && panelInvoker.current.focus({ preventScroll: true })); };
  const closeDetail = () => {
    const fallback = selectedVisibleId(); scope.selectedTask = null; scope.selectedRun = null; scope.selectedSubagent = null; scope.view.detailTrail = []; changed();
    requestAnimationFrame(() => { if (detailInvoker.current?.isConnected && rootRef.current?.contains(detailInvoker.current)) detailInvoker.current.focus({ preventScroll: true }); else { const row = rowElement(fallback); if (row) row.focus({ preventScroll: true }); else rootRef.current?.querySelector<HTMLButtonElement>(`[data-view-segment="${mode}"]`)?.focus({ preventScroll: true }); } });
  };
  function escapeLayer(target?: HTMLElement) {
    if (target?.closest(".supervisor-queue-row.is-expanded")) { scope.view.queueOpenRow = "collapsed"; changed(); }
    else if (attentionOpen) closeAttention(); else if (detailOpen) closeDetail();
    else if (scope.disclosures.history || scope.disclosures.diagnostics) closePanel();
    else if (scope.disclosures.archive) { scope.disclosures.archive = false; changed(); }
    else onClose();
  }
  useLayoutEffect(() => {
    const intent = returnFocusIntent.current;
    if (!intent || dialog) return;
    returnFocusIntent.current = null;
    const view = rootRef.current;
    if (!active || !view) return;
    const candidates = [
      intent.invoker,
      ...view.querySelectorAll<HTMLElement>(".supervisor-queue-row.is-decide .supervisor-queue-summary,.supervisor-queue-row.is-recover .supervisor-queue-summary,.supervisor-summary-counter.is-decide,.supervisor-summary-counter.is-recover"),
      view.querySelector<HTMLElement>(`[data-view-segment="${mode}"]`),
      view.querySelector<HTMLElement>(".supervisor-start-options"),
      view.querySelector<HTMLElement>('[data-start-agent]'),
      view.querySelector<HTMLElement>('[aria-label="Hide Supervisor"]'),
    ];
    for (const candidate of candidates) {
      if (!(candidate instanceof HTMLElement) || !candidate.isConnected || !view.contains(candidate) ||
        candidate.matches(":disabled,[aria-disabled='true']") || candidate.closest("[hidden],[inert]")) continue;
      let concealed = false;
      for (let ancestor: HTMLElement | null = candidate.parentElement; ancestor && ancestor !== view; ancestor = ancestor.parentElement) {
        if (ancestor instanceof HTMLDetailsElement && !ancestor.open && !ancestor.querySelector(":scope > summary")?.contains(candidate)) { concealed = true; break; }
      }
      if (concealed) continue;
      candidate.focus({ preventScroll: true });
      if (document.activeElement === candidate) break;
    }
  });
  const clearPanels = () => { scope.disclosures.history = false; scope.disclosures.diagnostics = false; };
  const select = (selection: { task: string } | { run: string; subagent: string | null }, invoker: HTMLElement | null, toggle: boolean, reveal: boolean, focus = false) => {
    scope.view.detailTrail = [];
    if (invoker) detailInvoker.current = invoker;
    const same = "task" in selection ? scope.selectedTask === selection.task : scope.selectedRun === selection.run && scope.selectedSubagent === selection.subagent;
    scope.selectedTask = "task" in selection && !(toggle && same) ? selection.task : null;
    scope.selectedRun = "run" in selection && !(toggle && same) ? selection.run : null;
    scope.selectedSubagent = "run" in selection && !(toggle && same) ? selection.subagent : null;
    clearPanels(); setDetailSection("overview"); if (reveal) revealIntent.current = { focus }; changed();
    if (focus) explicitFocus.current = true;
  };
  const navigateRelation = (taskId: string, back = false) => {
    const target = tasks.find(task => task.task.task_id === taskId);
    if (!target) return;
    const trail = back ? scope.view.detailTrail.slice(0, -1) : detailTask ? [...scope.view.detailTrail, detailTask.task.task_id].slice(-8) : [];
    if (target.task.checked) scope.disclosures.completed = true;
    select({ task: taskId }, rowElement(mode === "tasks" ? taskId : `task:${taskId}`) ?? null, false, true);
    scope.view.detailTrail = trail; changed();
    requestAnimationFrame(() => rootRef.current?.querySelector<HTMLButtonElement>("[data-relation-back],.supervisor-detail-header button")?.focus({ preventScroll: true }));
    setFocusNotice(`Showing ${target.task.title}${detailTask && !back ? ` · related to ${detailTask.task.title}` : ""}`);
  };
  useEffect(() => {
    const next = scope.view.detailTrail.filter(id => tasks.some(task => task.task.task_id === id));
    if (next.length !== scope.view.detailTrail.length) { scope.view.detailTrail = next; changed(); }
  }, [snapshot, scope]);
  useEffect(() => {
    const created = createdSelection.current;
    if (!created || !active || dialog || created.sessionId !== sessionId || created.rootId !== root?.run_id || !tasks.some(task => task.task.task_id === created.taskId)) return;
    createdSelection.current = null;
    revealSelection(created.focus);
  }, [snapshot, active, dialog, mode]);
  const switchView = (next: "tasks" | "graph" | "dependencies", focus = false) => {
    saveOffsets(); scope.view.mode = next; revealIntent.current = { focus }; changed();
    if (focus) explicitFocus.current = true;
    setFocusNotice(`${next === "graph" ? "Graph" : next === "dependencies" ? "Dependencies" : "Tasks"} view · ${model ? model.counts.supervisors + model.counts.workers + model.counts.subagents : 0} agents · ${tasks.filter(task => task.lane !== "accepted").length} tasks`);
  };
  useLayoutEffect(() => {
    if (explicitFocus.current) { explicitFocus.current = false; return; }
    if (active && placement === "overlay" && panelKind) rootRef.current?.querySelector<HTMLButtonElement>(".supervisor-detail-header button,.supervisor-queue-header button")?.focus({ preventScroll: true });
  }, [active, panelKind, placement]);
  // Fetch only on archive-open. Four concurrent requests bound provider load; identity/generation fence every completion.
  const closedKey = closedRoots.map(root => root.root_id).sort().join("\u0000");
  useEffect(() => {
    const generation = ++archiveGeneration.current;
    if (!scope.disclosures.archive || !active || !snapshot) return;
    const ids = closedRoots.map(root => root.root_id);
    setArchiveCounts(new Map(ids.map(id => [id, { status: "loading" } as ClosedTaskCount])));
    let cursor = 0, cancelled = false;
    const load = async () => {
      while (!cancelled && cursor < ids.length) {
        const id = ids[cursor++]; let count: ClosedTaskCount;
        try { const result = await client.orchestrationSnapshot({ session_id: sessionId, root_id: id }); count = result.session_id === sessionId && result.board?.root_id === id ? { status: "loaded", count: result.board.tasks.length } : { status: "unavailable" }; }
        catch { count = { status: "unavailable" }; }
        if (cancelled || archiveGeneration.current !== generation) return;
        setArchiveCounts(previous => { const next = new Map(previous); next.set(id, count); return next; });
      }
    };
    for (let index = 0; index < Math.min(4, ids.length); index++) void load();
    return () => { cancelled = true; archiveGeneration.current++; };
  }, [client, sessionId, rootId, active, scope.disclosures.archive, closedKey]);
  const dimFor = (tier: AttentionTier | null, runId: string | null) => scope.attentionOnly && !tier ? "attention filter" : scope.spaceFilter && observe(runId)?.workspace_id !== scope.spaceFilter ? "Space filter" : null;
  const showItem = (taskId: string | null, runId: string | null, invoker: HTMLElement) => {
    const task = tasks.find(task => task.task.task_id === taskId || mode === "tasks" && task.current_run_id === runId);
    const focus = attentionOpen;
    if (focus) setAttentionOpen(false);
    if (task) select({ task: task.task.task_id }, invoker, false, true, focus);
    else if (runId) select({ run: runId, subagent: null }, invoker, false, true, focus);
  };
  const rows: QueueRowView[] = attention?.items.map(item => {
    const run = snapshot!.runs.find(run => run.run_id === item.runId);
    const retirement = run ? retirementView(run) : null;
    const retirementUnconfirmed = item.sources.some(source => source.origin === "core" && source.kind === "retirement_unconfirmed");
    const task = tasks.find(task => task.task.task_id === item.taskId || task.current_run_id === item.runId || retirement && task.task.task_id === run?.task_id);
    const actions: QueueAction[] = []; const bodies = [];
    let title = task?.task.title ?? run?.label ?? "Supervisor";
    for (const source of item.sources) {
      if (source.origin === "core") {
        const problem = source.kind === "retirement_unconfirmed" ? retirement?.label ?? ATTENTION_PROBLEM[source.kind] : ATTENTION_PROBLEM[source.kind];
        if (source === item.sources[0]) title = `${problem} · ${title}`;
        if (source.kind === "needs_input" && question && answer && root) bodies.push(<section key="question" className="supervisor-needs-you" aria-label="Needs you"><p id={`supervisor-question-${question.message_id}`} className="supervisor-exact-text">{question.summary}</p><TextAction label="Answer" submitLabel="Send answer" describedBy={`supervisor-question-${question.message_id}`} draft={answer} changed={changed} busy={busy || !live || !rootState?.verified} submit={(text, message_id) => mutateResult({ action: "message_send", message_id, to_run_id: root.run_id, kind: "answer", text, in_reply_to: question.message_id })} success="Answer sent. Waiting for the agent." /></section>);
        if (source.kind === "retirement_unconfirmed" && retirement) bodies.push(<p key="retirement" className="supervisor-exact-text">{retirement.detail}</p>);
        if (source.kind === "intent_conflict") for (const intent of snapshot!.intents.filter(intent => intent.root_id === root?.run_id && intent.state === "conflict" && (intent.run_id === item.runId || intent.task_id === item.taskId))) {
          actions.push({ key: `apply:${intent.intent_id}`, label: "Apply acceptance to current task", primary: true, disabled: busy || !live, onActivate: () => void mutateResult({ action: "intent_resolve", intent_id: intent.intent_id, apply: true }) }, { key: `keep:${intent.intent_id}`, label: "Keep current task unchanged", disabled: busy || !connected, onActivate: () => void mutateResult({ action: "intent_resolve", intent_id: intent.intent_id, apply: false }) });
          bodies.push(<p key={intent.intent_id}>Review the current task before applying acceptance. Keeping it leaves Markdown unchanged.</p>);
        }
      } else {
        const condition = source.condition;
        if (condition.kind === "assignment" && root) {
          const canonical = tasks.find(task => task.task.task_id === condition.taskId);
          title = `${condition.state === "conflict" ? "Task changed elsewhere · Not assigned" : "Task assignment pending"} · ${canonical?.task.title ?? "Task"}`;
          if (canonical) bodies.push(<details key="canonical"><summary>Current task</summary><p className="supervisor-exact-text">{canonical.task.body}</p></details>);
          if (condition.state === "conflict") actions.push({ key: "assign", label: "Assign current task", primary: true, disabled: busy || !live || !canonical || !!canonical.task.diagnostic || root.stage === "closed", onActivate: () => void mutateResult({ action: "task_assignment_resolve", root_id: root.run_id, task_id: condition.taskId, expected_task_revision: canonical?.task.task_revision ?? null, assign: true }) }, { key: "unassign", label: "Keep unassigned", disabled: busy || !connected, onActivate: () => void mutateResult({ action: "task_assignment_resolve", root_id: root.run_id, task_id: condition.taskId, expected_task_revision: null, assign: false }) });
          else actions.push({ key: "assignment-check", label: "Check assignment status", primary: true, disabled: busy, onActivate: refresh });
        } else if (condition.kind === "orphaned_worker" && run) {
          title = `Worker agent needs control · ${run.label}`; bodies.push(<p key="orphan">Closing this supervisor did not stop its workers. Tasks and history are kept.</p>);
          actions.push({ key: "saved-context", label: "View saved task context", onActivate: () => setRootId(run.root_id), disabled: busy });
        } else if (condition.kind === "start_unknown") {
          title = "Previous start unconfirmed"; bodies.push(<p key="start">Review the tracked agents before another start; a terminal may already have opened.</p>);
          actions.push({ key: "start-check", label: "Check status", primary: true, onActivate: refresh, disabled: busy }, { key: "start-reviewed", label: "I have reviewed the previous start", disabled: busy, onActivate: () => { setStartUnknown(false); setNotice("Previous start reviewed. Any existing terminal and tracking remain unchanged."); } });
        } else if (condition.kind === "terminal_error" || condition.kind === "change_unconfirmed" || condition.kind === "navigation_error") {
          title = condition.kind === "navigation_error" ? "Could not open terminal. Its current focus or location was not confirmed." : condition.message;
          actions.push({ key: "check", label: "Check status", primary: true, onActivate: refresh, disabled: busy });
        } else if (condition.kind === "notice") { title = condition.message; actions.push({ key: "dismiss", label: "Dismiss notice", onActivate: () => setNotice(null) }); }
        else if (condition.kind === "root_failed_report" && run) { title = `Reported failure · ${run.label}`; bodies.push(<p key="failure" className="supervisor-exact-text">{run.last_report?.summary}</p>); }
        else if (condition.kind === "unidentified_items" && root && snapshot?.board) { title = `${condition.count} task-file items need identity markers`; actions.push({ key: "identify", label: "Identify task-file items", primary: true, disabled: busy || !connected, onActivate: () => void mutateResult({ action: "tasks_assign_ids", root_id: root.run_id, expected_doc_revision: snapshot.board!.doc_revision }) }); }
        else if (condition.kind === "agent_status" && run && snapshot) title = `${agentState(snapshot, run, connected, runtimeLive).label} · ${run.label}`;
        else if (condition.kind === "relation_diagnostic" && task) {
          title = `Prerequisites need fixing · ${task.task.title}`; bodies.push(<p key="relations">{condition.cause}</p>);
          actions.push({ key: "fix-prerequisites", label: "Fix prerequisites…", primary: true, disabled: busy || !live || root?.stage === "closed", onActivate: () => editPrerequisites(task) });
        } else if (condition.kind === "dependency_regression" && task) {
          title = `Prerequisites changed during work · ${task.task.title}`;
          bodies.push(<p key="regression">The current agent was not automatically stopped. New guarded work, execution and acceptance wait for the supervisor to resolve prerequisites.</p>);
          actions.push({ key: "dependencies", label: "Show dependencies", onActivate: invoker => { switchView("dependencies"); showItem(task.task.task_id, task.current_run_id, invoker); } });
        }
      }
    }
    if (retirementUnconfirmed && retirement && run && task && task.current_run_id !== run.run_id) actions.push({
      key: "saved-retirement", label: "View saved worker details",
      onActivate: invoker => { if (attentionOpen) setAttentionOpen(false); select({ run: run.run_id, subagent: null }, invoker, false, false); },
    });
    if (item.tier === "recover" && run && snapshot && !retirementUnconfirmed) {
      const state = agentState(snapshot, run, connected, runtimeLive);
      const recovery = recoveryActions(run, state, { busy: busy || startPending || pendingRestart === run.run_id, connected, runtimeLive: runtimeLive && snapshot.runtime.status === "fresh" });
      for (const action of recovery) actions.push({
        key: action.kind, label: action.label, primary: action.primary || action.kind === "terminal" && !recovery.some(item => item.primary), disabled: action.disabled, ariaDisabled: action.ariaDisabled,
        reason: action.reason, consequence: action.consequence,
        onActivate: () => {
          if (action.disabled) return;
          if (action.kind === "terminal") void navigate(run);
          else if (action.kind === "check") check(run);
          else if (action.kind === "close") setDialog({ mode: "close", run });
          else void restart(run);
        },
      });
      bodies.push(<p key="recovery-detail" className="supervisor-muted">{state.detail}</p>);
    }
    if (task && dependentCount(task, tasks)) bodies.push(<p key="holds">{dependentCount(task, tasks)} tasks are waiting on this</p>);
    return {
      id: item.id, tier: item.tier, title, since: item.since,
      body: bodies.length ? <>{bodies}</> : undefined, actions,
      showIn: retirementUnconfirmed ? task ? {
        key: "show", label: "Show completed task",
        onActivate: invoker => { scope.disclosures.completed = true; if (mode !== "tasks") switchView("tasks"); showItem(task.task.task_id, item.runId, invoker); },
      } : run ? {
        key: "show", label: "View saved worker details",
        onActivate: invoker => { if (attentionOpen) setAttentionOpen(false); select({ run: run.run_id, subagent: null }, invoker, false, false); },
      } : null : task || run && rootRuns.includes(run) || root ? {
        key: "show", label: mode === "graph" ? "Show in Graph" : mode === "dependencies" ? "Show in Dependencies" : "Show in Tasks",
        onActivate: invoker => showItem(task?.task.task_id ?? item.taskId, item.runId ?? root?.run_id ?? null, invoker),
      } : null,
    };
  }) ?? [];
  const expandedId = scope.view.queueOpenRow && rows.some(row => row.id === scope.view.queueOpenRow) ? scope.view.queueOpenRow : scope.view.queueOpenRow === null ? rows.find(row => row.tier === "decide")?.id ?? null : null;
  const path: PathRowView[] = model && selectedNode ? pathNodes(model, selectedNode, true).map((node, depth) => {
    const facts = nodeFacts(node, { model, snapshot: snapshot!, live: !!live, connected, runtimeLive });
    return { key: node.id, role: facts.role, label: facts.title, facts: [facts.status, facts.provenance, facts.relation].filter((fact): fact is string => !!fact), current: node.id === selectedNode, depth, subagent: node.kind === "subagent", onActivate: () => select(nodeSelection(node), null, false, true) };
  }) : [];
  const tier = selectedTask ? attention?.tierForTask(selectedTask) : detailRun ? attention?.tierForRun(detailRun.run_id) : null;
  const owned = detailRun ? attention?.ownedForRun(detailRun.run_id) : null;
  const detailState = detailRun && snapshot ? agentState(snapshot, detailRun, connected, runtimeLive) : null;
  const detailQuestion = detailRun && !detailSubagent && snapshot ? questionForRun(snapshot, detailRun) : null;
  const stateSentence = detailQuestion
    ? detailState && (detailState.kind !== "ready" || detailState.blocked || tier === "recover") ? detailState.label : questionLabel(detailQuestion)
    : owned ? owned.kind === "to_accept" ? "Supervisor is reviewing this result" : "Waiting for supervisor"
    : detailState?.label ?? (detailTask ? taskStatus(detailTask, undefined, snapshot!) : "No current worker");
  const chips: StripChip[] = model && snapshot ? model.nodes.filter(node => node.run && !node.subagent && node.run.stage !== "closed").map(node => { const facts = nodeFacts(node, { model, snapshot, live: !!live, connected, runtimeLive }); return { runId: node.run!.run_id, label: facts.title, glyph: facts.glyph === "document" ? "unknown" : facts.glyph, status: facts.status, tier: attention?.tierForRun(node.run!.run_id) ?? null, selected: scope.selectedRun === node.run!.run_id }; }).sort((a, b) => (a.tier ? TIER_ORDER[a.tier] : 3) - (b.tier ? TIER_ORDER[b.tier] : 3)) : [];
  const observedCount = rootRuns.filter(run => run.stage !== "closed" && live && observe(run.run_id)?.presence === "present" && observe(run.run_id)?.actual_omp).length;
  const rootSpace = observe(root?.run_id)?.workspace_id ?? (root?.target?.target === "existing_space" ? root.target.workspace_id : null);
  const banner = !connected || !runtimeLive ? "Connection lost · showing saved tasks · agent may still be running" : snapshot?.runtime.status !== "fresh" ? "Cannot check agents right now · showing saved tasks" : null;
  useLayoutEffect(() => {
    const workarea = workareaRef.current, view = rootRef.current;
    if (!workarea || !view || layout.queueMode !== "inline") { setInlineQueueCap(null); return; }
    const chrome = [view.querySelector<HTMLElement>(".supervisor-summary"), view.querySelector<HTMLElement>(".supervisor-viewbar"), view.querySelector<HTMLElement>(".supervisor-strip")].filter((element): element is HTMLElement => !!element);
    const measure = () => {
      const height = workarea.getBoundingClientRect().height;
      if (!height) { setInlineQueueCap(null); return; }
      const chromeHeight = chrome.reduce((sum, element) => sum + element.getBoundingClientRect().height, 0);
      const wholeViewHeight = Math.max(height, view.getBoundingClientRect().height);
      // capPx bounds the scrollport, not the complete queue. Keep its footer and border inside the budget.
      const footer = Math.max(24, view.querySelector<HTMLElement>(".supervisor-queue-more")?.getBoundingClientRect().height ?? 0);
      const ceiling = height * (mode === "graph" ? 0.3 : 0.4);
      const boardBudget = mode === "tasks" ? height - chromeHeight - wholeViewHeight * 0.5 : ceiling;
      const next = Math.max(38, Math.floor(Math.min(ceiling, boardBudget) - footer - 1));
      setInlineQueueCap(current => current === next ? current : next);
    };
    measure();
    const observer = typeof ResizeObserver === "undefined" ? null : new ResizeObserver(measure);
    observer?.observe(workarea); observer?.observe(view);
    for (const element of chrome) observer?.observe(element);
    return () => observer?.disconnect();
  }, [layout.queueMode, layout.width, layout.height, mode, rootId, rows.length, banner, active]);
  const openPanel = (kind: "history" | "diagnostics", invoker: HTMLElement) => {
    const next = !scope.disclosures[kind]; scope.selectedTask = null; scope.selectedRun = null; scope.selectedSubagent = null; clearPanels(); scope.disclosures[kind] = next; panelInvoker.current = invoker; changed();
    if (next && !layout.narrow) setFocusNotice(`${kind === "history" ? "Activity" : "Diagnostics"} panel opened`);
  };
  const renderTask = (task: TaskView) => {
    const worker = snapshot!.runs.find(run => run.run_id === task.current_run_id), observed = observe(worker?.run_id);
    const tier = attention?.tierForTask(task) ?? null;
    return <TaskCard key={task.task.task_id} rowId={task.task.task_id} tabIndex={tabIndexFor(task.task.task_id)} view={{ task, worker, observed, snapshot: snapshot!, live: !!live, status: taskStatus(task, worker, snapshot!), tier, dimReason: dimFor(tier, worker?.run_id ?? null), selected: scope.selectedTask === task.task.task_id, linked: !!worker && (hoverRun === worker.run_id || selectedAgent?.run_id === worker.run_id), subagentCount: snapshot!.subagents.filter(agent => agent.run_id === worker?.run_id).length, showSpace: !!scope.spaceFilter || observed?.workspace_id !== rootSpace, sharedSpace }} onFocus={() => { focusedTask.current = task.task.task_id; }} onSelect={() => select({ task: task.task.task_id }, rowElement(task.task.task_id) ?? null, true, false)} onHover={entering => { setHoverRun(entering ? worker?.run_id ?? null : null); setHoverSpace(entering ? observed?.workspace_id ?? null : null); }} />;
  };
  const renderLane = (lane: TaskLane, label: string) => {
    const laneTasks = tasks.filter(task => task.lane === lane);
    const open = lane === "accepted" ? scope.disclosures.completed : laneTasks.length > 0 && !scope.view.collapsedLanes.includes(lane);
    const list = <ul ref={element => { laneRefs.current[lane] = element; }} className="supervisor-task-list" hidden={layout.narrow ? !open : lane === "accepted" && !open} onScroll={event => { if (!layout.narrow) scope.view.laneScroll[lane] = event.currentTarget.scrollTop; }}>{laneTasks.map(renderTask)}</ul>;
    if (layout.narrow) return <details key={lane} className={`supervisor-lane-group is-${lane}`} open={open} onToggle={event => { const next = event.currentTarget.open; if (next === open) return; if (lane === "accepted") scope.disclosures.completed = next; else scope.view.collapsedLanes = next ? scope.view.collapsedLanes.filter(item => item !== lane) : [...scope.view.collapsedLanes.filter(item => item !== lane), lane]; changed(); }}><summary className="supervisor-lane-header"><span className="supervisor-lane-dot" /><strong>{label}</strong><span className="supervisor-lane-count">{laneTasks.length}</span><UiIcon name={open ? "down" : "right"} /></summary>{list}</details>;
    return <section key={lane} className={`supervisor-lane is-${lane}${lane === "accepted" && !open ? " is-collapsed" : ""}`} aria-label={`${label} tasks`}><header className="supervisor-lane-header"><span className="supervisor-lane-dot" /><strong>{label}</strong><span className="supervisor-lane-count">{laneTasks.length}</span>{lane === "accepted" ? <button type="button" aria-label={open ? "Hide completed tasks" : "Show completed tasks"} aria-expanded={open} onClick={() => { scope.disclosures.completed = !open; changed(); }}><UiIcon name={open ? "down" : "right"} /></button> : null}</header>{list}{!laneTasks.length ? <p className="supervisor-lane-empty">No tasks</p> : null}</section>;
  };
  const startDisabled = busy || startPending || !snapshot || !connected || startUnknown || rootState?.kind === "starting";
  return <section ref={rootRef} className="supervisor-view" hidden={!active} aria-label="Supervisor"
    data-view={mode} data-narrow={layout.narrow} data-short={layout.short} data-compact={layout.compact}
    data-panel={placement ?? "none"} data-queue={layout.queueMode}
    onFocusCapture={event => {
      const target = event.target as HTMLElement;
      if (target.closest("li.supervisor-task")) focusedTask.current = target.closest("li.supervisor-task")?.querySelector<HTMLElement>("[data-row-id]")?.dataset.rowId ?? null;
      focusedGraph.current = target.closest<HTMLElement>(".supervisor-graph-node")?.dataset.rowId ?? null;
      focusWithinDetail.current = !!target.closest(".supervisor-detail-panel");
      questionHadFocus.current = !!target.closest('[aria-label="Needs you"]');
    }}
    onKeyDown={event => {
      if (event.key !== "Escape" || event.nativeEvent.isComposing || event.defaultPrevented || dialog ||
        (event.target as HTMLElement).closest("input,textarea,select,[contenteditable=true]")) return;
      event.preventDefault(); event.stopPropagation(); escapeLayer(event.target as HTMLElement);
    }}>
    <header className="supervisor-header"><span className="supervisor-brand"><UiIcon name="branch" /><strong>Supervisor</strong></span>{openRoots.length > 1 ? <label className="supervisor-agent-label"><select aria-label="Agent" value={root?.stage !== "closed" ? root?.run_id ?? "" : ""} disabled={busy || startPending || !!dialog} onChange={event => { saveOffsets(); setRootId(event.target.value); setTerminalError(null); setAttentionOpen(false); }}><option value="" disabled>Choose an agent</option>{openRoots.map(summary => { const counts = rootAttentionSummary(snapshot!, summary.root_id); return <option key={summary.root_id} value={summary.root_id}>{summary.label} · {counts.decide} need you{counts.recover ? " · ⚠ recover" : ""}</option>; })}</select></label> : null}<nav className="supervisor-header-panels" aria-label="Supervisor panels"><button type="button" aria-label="Activity" title="Activity" aria-pressed={scope.disclosures.history} onClick={event => openPanel("history", event.currentTarget)}><UiIcon name="comment" /></button><button type="button" aria-label="Diagnostics" title="Diagnostics" aria-pressed={scope.disclosures.diagnostics} onClick={event => openPanel("diagnostics", event.currentTarget)}><UiIcon name="info" /></button>{closedRoots.length ? <button type="button" aria-label={`Closed tracking · ${closedRoots.length}`} title={`Closed tracking · ${closedRoots.length}`} aria-expanded={scope.disclosures.archive} onClick={() => { scope.disclosures.archive = !scope.disclosures.archive; changed(); }}><UiIcon name="folder" /></button> : null}</nav><button type="button" className={root?.stage !== "closed" && rootState?.verified ? "supervisor-start-secondary" : "supervisor-primary"} title={destination ? `New tab in ${destination.label}` : "Choose an available location when starting."} data-start-agent disabled={startDisabled} onClick={() => void start()}><UiIcon name="plus" />{startPending ? "Starting…" : "Start agent"}</button><button type="button" className="supervisor-start-options" aria-label="Start options…" title="Start options…" disabled={startDisabled} onClick={() => { if (!startDraft.current.spaceId && destination) startDraft.current.spaceId = destination.id; setDialog({ mode: "start" }); }}><UiIcon name="more" /><span>Start options…</span></button><button type="button" aria-label="Hide Supervisor" title="Hide Supervisor" onClick={onClose}><UiIcon name="close" /></button></header>
    {scope.disclosures.archive && snapshot ? <ClosedTracking closedRoots={closedRoots} runs={snapshot.runs} loadedRootId={snapshot.board?.root_id ?? null} taskCount={snapshot.board?.tasks.length ?? 0} taskCounts={archiveCounts} busy={busy} dialogOpen={!!dialog} onView={id => { saveOffsets(); setRootId(id); scope.disclosures.archive = false; changed(); }} returnTo={root?.stage === "closed" && openRoots.length ? { label: openRoots[0].label, onActivate: () => setRootId(openRoots[0].root_id) } : null} /> : null}
    <div ref={workareaRef} className="supervisor-workarea" style={{ "--detail-size": `${bounds?.value ?? 340}px`, "--sheet-size": `${bottomInset}px` } as CSSProperties}><div className="supervisor-content">
      {!snapshot ? <div className="supervisor-empty"><UiIcon name="branch" /><h2>{error ? "Could not load Supervisor" : "Loading Supervisor…"}</h2>{error ? <><p>Your terminals are unchanged. The Supervisor connection could not be established.</p><button type="button" onClick={refresh}>Retry load</button></> : <p role="status">Reading saved tasks and fresh agent observations.</p>}</div> : <>
        <SupervisorSummary
          rootLabel={root?.label ?? "No supervisor selected"} stateLabel={rootReceiptLabel ?? rootState?.label ?? "Start an agent"}
          stateGlyph={rootState?.blocked ? "blocked" : rootState?.verified ? "live" : "unknown"}
          observedLine={live ? `${observedCount} agents observed · ${snapshot.runtime.status === "fresh" ? new Date(snapshot.runtime.observed_at).toLocaleTimeString() : ""}` : "Agents · unobserved"}
          banner={banner} counts={attention?.counts ?? { decide: 0, recover: 0, notice: 0 }} queueMode={layout.queueMode}
          onCounter={(tier, invoker) => {
            counterInvoker.current = invoker;
            if (layout.queueMode === "overlay") { setFocusTier(null); setAttentionOpen(true); }
            else setFocusTier(tier);
          }}
          shortcut={root && rootState?.terminal ? { key: "terminal", label: "Open terminal", disabled: busy || !live, onActivate: () => void navigate(root) } : null}
        />
        {rows.length > 0 && layout.queueMode === "inline" ? <AttentionQueue rows={rows} expandedId={expandedId} onExpand={id => { scope.view.queueOpenRow = id ?? "collapsed"; changed(); }} capPx={inlineQueueCap ?? (queueCap(layout, mode) || null)} variant="inline" focusTier={focusTier} onFocusedTier={() => setFocusTier(null)} /> : null}
        {root && model ? <><div className="supervisor-viewbar"><div className="supervisor-viewbar-switch" role="group" aria-label="Workarea view"><button type="button" data-view-segment="tasks" aria-pressed={mode === "tasks"} onClick={() => switchView("tasks")}><UiIcon name="grid" />Tasks {tasks.filter(task => task.lane !== "accepted").length}</button><button type="button" data-view-segment="graph" aria-pressed={mode === "graph"} onClick={() => switchView("graph")}><UiIcon name="branch" />Graph {model.nodes.length}</button><button type="button" data-view-segment="dependencies" aria-pressed={mode === "dependencies"} aria-label={`Dependencies, ${tasks.filter(task => !task.task.checked && ["blocked", "invalid"].includes(task.dependencies.state)).length} tasks waiting`} onClick={() => switchView("dependencies")}>Dependencies {tasks.filter(task => !task.task.checked && ["blocked", "invalid"].includes(task.dependencies.state)).length || ""}</button></div><div className="supervisor-viewbar-filters" aria-label="Task filters"><button type="button" aria-pressed={scope.attentionOnly} onClick={() => { scope.attentionOnly = !scope.attentionOnly; changed(); }}>Attention · {attention?.total ?? 0}</button><label>Space<select aria-label="Task Space" value={scope.spaceFilter} onChange={event => { scope.spaceFilter = event.target.value; changed(); }}><option value="">All Spaces</option>{[...new Map(observations.filter(item => item.workspace_id && item.workspace_label && rootRuns.some(run => run.run_id === item.run_id)).map(item => [item.workspace_id!, item.workspace_label!])).entries()].map(([id, label]) => <option key={id} value={id}>{label}</option>)}</select></label></div></div>
        {mode === "graph" ? <SupervisorGraph
          model={model} snapshot={snapshot} live={!!live} connected={connected} runtimeLive={runtimeLive}
          selectedNodeId={selectedNode} highlightedRunId={hoverRun ?? detailRun?.run_id ?? null}
          tierFor={node => node.task ? attention?.tierForTask(node.task) ?? null : node.run ? attention?.tierForRun(node.run.run_id) ?? null : null}
          dimFor={node => dimFor(node.task ? attention?.tierForTask(node.task) ?? null : node.run ? attention?.tierForRun(node.run.run_id) ?? null : null, node.run?.run_id ?? node.assignedRunId)}
          showSubagents={scope.showSubagents}
          onShowSubagents={next => {
            scope.showSubagents = next;
            if (!next && scope.selectedSubagent && scope.selectedRun) {
              scope.selectedSubagent = null; revealIntent.current = { focus: true }; setDetailSection("overview");
            }
            changed();
          }}
          sharedSpace={sharedSpace} bottomInset={bottomInset} scrollRef={graphRef}
          onSelect={(node: TopologyNode) => select(nodeSelection(node), rowElement(node.id) ?? null, true, false)}
          onHover={id => { setHoverRun(id); setHoverSpace(observe(id)?.workspace_id ?? null); }}
          onEscape={() => { escapeLayer(); return true; }}
        /> : mode === "dependencies" ? <SupervisorDependencies tasks={tasks} snapshot={snapshot} selectedTaskId={scope.selectedTask} showCompleted={scope.disclosures.completed} onShowCompleted={() => { scope.disclosures.completed = true; changed(); }} narrow={layout.narrow} scrollRef={graphRef} attentionOnly={scope.attentionOnly} tierFor={task => attention?.tierForTask(task) ?? null} dimFor={task => scope.spaceFilter && observe(task.current_run_id)?.workspace_id !== scope.spaceFilter ? "Space filter" : null} onSelect={(task, invoker) => select({ task: task.task.task_id }, invoker, true, false)} onEscape={() => { escapeLayer(); return true; }} bottomInset={bottomInset} /> : <section className="supervisor-board-surface" aria-label="Tasks">
          {!layout.narrow && !layout.short ? <AgentsStrip
            heading={live ? `Agents · ${observedCount} observed` : "Agents · unobserved"} chips={chips}
            subagentCount={snapshot.subagents.filter(agent => rootRuns.some(run => run.run_id === agent.run_id && run.stage !== "closed")).length}
            onSelect={(id, invoker) => select({ run: id, subagent: null }, invoker, true, false)}
            onGraph={() => {
              switchView("graph");
              requestAnimationFrame(() => rootRef.current?.querySelector<HTMLButtonElement>('[data-view-segment="graph"]')?.focus({ preventScroll: true }));
            }}
          /> : null}
          {tasks.length ? <div ref={listRef} {...listProps} className="supervisor-board"
            onScroll={event => { scope.view.offsets.tasks = readOffset(event.currentTarget); }}
            onKeyDown={event => {
              const id = (event.target as HTMLElement).dataset.rowId;
              if (id && !event.ctrlKey && !event.altKey && !event.metaKey && !event.nativeEvent.isComposing) {
                const next = taskNeighbor(tasks, id, event.key, navOptions);
                if (next) {
                  event.preventDefault();
                  const element = rowElement(next);
                  const scroller = layout.narrow ? listRef.current : element?.closest<HTMLUListElement>(".supervisor-task-list");
                  if (element && scroller) { revealNearest(scroller, element); element.focus({ preventScroll: true }); }
                  return;
                }
              }
              listProps.onKeyDown(event);
            }}>
            {taskLanes.map(({ lane, label }) => renderLane(lane, label))}
          </div> : <div className="supervisor-empty">
            <h2>No tasks yet</h2>
            <button type="button" disabled={busy || !live || !rootState?.terminal} onClick={() => void navigate(root)}>Open terminal</button>
          </div>}
        </section>}
        </> : <div className="supervisor-empty"><UiIcon name="branch" /><h2>Start an agent to manage your tasks.</h2></div>}
      </>}
    </div>
    {snapshot && active && panelKind ? <>{bounds ? <PanelSplitter label="Resize details" controls="supervisor-detail" orientation={bounds.orientation} value={bounds.value} min={bounds.min} max={bounds.max} grow={-1} onChange={value => { if (placement === "sheet") scope.view.sheetHeight = value; else scope.view.detailWidth = value; changed(); }} onReset={() => { if (placement === "sheet") scope.view.sheetHeight = null; else scope.view.detailWidth = null; changed(); }} className={placement === "sheet" ? "supervisor-sheet-splitter" : undefined} /> : null}<aside className={`supervisor-detail-panel${placement === "sheet" ? " supervisor-sheet" : ""}`} id="supervisor-detail" aria-label={panelKind === "details" ? "Selected details" : panelKind === "attention" ? "Attention" : panelKind === "diagnostics" ? "Diagnostics" : "Activity"}>
      {panelKind === "attention" ? <AttentionQueue
        rows={rows} expandedId={expandedId}
        onExpand={id => { scope.view.queueOpenRow = id ?? "collapsed"; changed(); }}
        capPx={null} variant="overlay" focusTier={focusTier}
        onFocusedTier={() => setFocusTier(null)} onClose={closeAttention}
      /> : <>
        <header className="supervisor-detail-header">
          {panelKind === "details" && scope.view.detailTrail.length ? <button type="button" data-relation-back onClick={() => navigateRelation(scope.view.detailTrail.at(-1)!, true)}>‹ {tasks.find(task => task.task.task_id === scope.view.detailTrail.at(-1))?.task.title ?? "Back"}</button> : null}
          <button type="button"
            aria-label={panelKind === "details" ? "Close details" : `Close ${panelKind === "activity" ? "activity" : "diagnostics"} panel`}
            onClick={panelKind === "details" ? closeDetail : closePanel}><UiIcon name="back" /></button>
          <strong>{panelKind === "details" ? detailTask?.task.title ?? detailSubagent?.label ?? detailRun?.label ?? "Task unavailable · drafts kept" : panelKind === "activity" ? "Activity" : "Diagnostics"}</strong>
        </header>
        {panelKind === "details" ? <>
          <nav className="supervisor-segments" aria-label="Detail sections">
            {(["overview", "activity", "actions"] as const).map(section =>
              <button type="button" key={section} aria-pressed={detailSection === section}
                onClick={() => setDetailSection(section)}>{section[0].toUpperCase() + section.slice(1)}</button>)}
          </nav>
          <div className="supervisor-detail-scroll">
            <SupervisorActions
              section={detailSection} snapshot={snapshot} run={detailRun} task={detailTask}
              subagent={detailSubagent} scope={scope} changed={changed} busy={busy} live={!!live}
              mutateResult={mutateResult} onTerminal={run => void navigate(run)} onEditTask={edit}
              onCloseTracking={run => setDialog({ mode: "close", run })}
              onCancelSubagent={(run, subagent) => setDialog({ mode: "subagent_cancel", run, subagent })}
              stateBlock={{ sentence: stateSentence, tierLabel: tier ? TIER_LABEL[tier] : null, waitingSince: owned?.since ?? null }}
              path={path}
              onResumeSourceDraft={resumeSourceDraft}
              onNavigateTask={navigateRelation} onEditPrerequisites={editPrerequisites} onCreateFollowUp={createFollowUp}
              taskWriteUnconfirmed={!!detailTaskScope && taskWriteUnconfirmed(detailTaskScope)}
              steps={detailTaskScope ? { scope: detailTaskScope, task: detailTask?.task ?? null, draft: stepDraft(scope, detailTaskScope.taskId), writable: !stepReadOnlyReason, readOnlyReason: stepReadOnlyReason, busy, taskWriteUnconfirmed: taskWriteUnconfirmed(detailTaskScope), onDraftChanged: changed, submit: submitStep, readSaved, resolveUnknown, onCloseDetails: closeDetail } : undefined}
              crossView={null}
              crossViews={(["tasks", "graph", "dependencies"] as const).filter(next => next !== mode).map(next => ({ label: next === "tasks" ? "Show in Tasks" : next === "graph" ? "Show in Graph" : "Show in Dependencies", onActivate: () => { if (next === "tasks" && detailTask?.task.checked) scope.disclosures.completed = true; switchView(next, true); } }))}
              acceptanceConflict={snapshot.intents.some(intent => intent.state === "conflict" &&
                (intent.task_id === detailTask?.task.task_id || intent.run_id === detailRun?.run_id))}
            />
          </div>
        </> : <div className="supervisor-detail-scroll">
          {panelKind === "activity" ? <SupervisorActivity rows={activityRows(snapshot, rootRuns, tasks)}
            onLink={link => {
              if (link.kind === "task") select({ task: link.taskId }, null, false, true);
              else select({ run: link.runId, subagent: null }, null, false, true);
            }} /> : <SupervisorDiagnostics snapshot={snapshot} rootRuns={rootRuns} busy={busy} connected={connected}
            onIdentify={() => {
              if (snapshot.board) void mutateResult({ action: "tasks_assign_ids", root_id: snapshot.board.root_id, expected_doc_revision: snapshot.board.doc_revision });
            }}
            onCopyPath={() => setFocusNotice("Canonical task path copied")} />}
        </div>}
      </>}
    </aside></> : null}</div>
    <div className="supervisor-focus-notice" role="status">{focusNotice}</div>
    {dialog && snapshot && active ? dialog.mode === "edit" || dialog.mode === "relations" || dialog.mode === "follow_up" ? <TaskSourceDialog key={`${dialog.mode}:${dialog.scope.rootId}:${dialog.scope.taskId}`} dialog={dialog} snapshot={snapshot} changed={changed} busy={busy} available={!!live} writeUnconfirmed={taskWriteUnconfirmed(dialog.draft.submitted?.scope ?? dialog.scope)} submitTask={submitTask} readSaved={readSaved} resolveUnknown={resolveSourceUnknown} onSaved={(task, created, reconciled) => { if (dialog.mode === "edit") scope.edits.delete(dialog.scope.taskId); else if (dialog.mode === "relations") scope.relations.delete(dialog.scope.taskId); else scope.followUps.delete(dialog.scope.taskId); if (created) { createdSelection.current = { sessionId: dialog.scope.sessionId, rootId: dialog.scope.rootId, taskId: task.task_id, focus: document.activeElement instanceof HTMLElement && !!document.activeElement.closest('[role="dialog"]') }; scope.selectedTask = task.task_id; scope.selectedRun = null; scope.selectedSubagent = null; scope.view.detailTrail = []; setDetailSection("overview"); setFocusNotice(`${reconciled ? "Showing saved task" : "Follow-up created"} · ${task.title}`); } changed(); }} onDiscard={() => { if (dialog.mode === "edit") scope.edits.delete(dialog.scope.taskId); else if (dialog.mode === "relations") scope.relations.delete(dialog.scope.taskId); else scope.followUps.delete(dialog.scope.taskId); changed(); }} onClose={() => setDialog(null)} onReturnFocus={invoker => { returnFocusIntent.current = { invoker }; }} /> : <SupervisorDialogs dialog={dialog} snapshot={snapshot} spaces={runtimeLive ? session?.spaces ?? [] : []} startDraft={startDraft.current} changed={changed} busy={busy} available={connected && (dialog.mode === "close" || runtimeLive && snapshot.runtime.status === "fresh")} mutateResult={mutateResult} onStarted={started} onStartUnconfirmed={() => setStartUnknown(true)} onCheck={check} onClose={() => setDialog(null)} onReturnFocus={invoker => { returnFocusIntent.current = { invoker }; }} /> : null}
  </section>;
}
