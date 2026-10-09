import { useEffect, useLayoutEffect, useRef } from "react";
import { useRovingList } from "../sidebar/useRovingList";
import { taskLanes, visibleTaskIds } from "./boardNavigation";
import { isCovered, readOffset, revealNearest } from "./reveal";
import type { AttentionTier } from "./attention";
import type { DetailFocus, SupervisorViewContext, SupervisorViewInputs, SupervisorViewModel, SupervisorViewNavigation, SupervisorViewPanel, SupervisorViewAttention } from "./supervisorViewTypes";
import type { TaskLane } from "../../protocol/generated/v1";
import type { ScopeDrafts } from "./useSupervisorDrafts";

/** Focus ownership stays with the mounted workarea, across scopes and polls. */
export function useDetailFocus(): DetailFocus {
  const detailInvoker = useRef<HTMLElement | null>(null);
  const panelInvoker = useRef<HTMLElement | null>(null);
  const counterInvoker = useRef<HTMLElement | null>(null);
  const returnFocusIntent = useRef<{ invoker: Element | null } | null>(null);
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
  const revealIntent = useRef<{ focus: boolean } | null>(null);
  const explicitFocus = useRef(false);
  const previousGraph = useRef<{ scope: ScopeDrafts; ids: readonly string[] } | null>(null);
  return { detailInvoker, panelInvoker, counterInvoker, returnFocusIntent, rootRef, workareaRef, graphRef, laneRefs, focusedTask, focusedGraph, focusWithinDetail, questionHadFocus, startFocus, createdSelection, lastTaskIds, revealIntent, explicitFocus, previousGraph };
}
export function useSupervisorViewNavigation(context: SupervisorViewInputs & SupervisorViewModel & SupervisorViewPanel): SupervisorViewNavigation {
  const { scope, layout, tasks, rootRef, graphRef, laneRefs, selectedNode, mode, bottomInset, attentionOpen, setAttentionOpen, counterInvoker, panelInvoker, changed, detailInvoker, detailOpen, onClose, setDetailSection, revealIntent, explicitFocus, detailTask, setFocusNotice, model, observe } = context;
  const navOptions = { arrangement: layout.narrow ? "stacked" as const : "lanes" as const, completedOpen: scope.disclosures.completed, collapsedLanes: scope.view.collapsedLanes };
  const taskIds = visibleTaskIds(tasks, navOptions);
  const { listRef, listProps, tabIndexFor, focusRow } = useRovingList({ rowIds: taskIds, selectedId: scope.selectedTask, onEscape: () => { escapeLayer(); return true; } });
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
  const switchView = (next: "tasks" | "graph" | "dependencies", focus = false) => {
    saveOffsets(); scope.view.mode = next; revealIntent.current = { focus }; changed();
    if (focus) explicitFocus.current = true;
    setFocusNotice(`${next === "graph" ? "Graph" : next === "dependencies" ? "Dependencies" : "Tasks"} view · ${model ? model.counts.supervisors + model.counts.workers + model.counts.subagents : 0} agents · ${tasks.filter(task => task.lane !== "accepted").length} tasks`);
  };
  const dimFor = (tier: AttentionTier | null, runId: string | null) => scope.attentionOnly && !tier ? "attention filter" : scope.spaceFilter && observe(runId)?.workspace_id !== scope.spaceFilter ? "Space filter" : null;
  const showItem = (taskId: string | null, runId: string | null, invoker: HTMLElement) => {
    const task = tasks.find(task => task.task.task_id === taskId || mode === "tasks" && task.current_run_id === runId);
    const focus = attentionOpen;
    if (focus) setAttentionOpen(false);
    if (task) select({ task: task.task.task_id }, invoker, false, true, focus);
    else if (runId) select({ run: runId, subagent: null }, invoker, false, true, focus);
  };
  const openPanel = (kind: "history" | "diagnostics", invoker: HTMLElement) => {
    const next = !scope.disclosures[kind]; scope.selectedTask = null; scope.selectedRun = null; scope.selectedSubagent = null; clearPanels(); scope.disclosures[kind] = next; panelInvoker.current = invoker; changed();
    if (next && !layout.narrow) setFocusNotice(`${kind === "history" ? "Activity" : "Diagnostics"} panel opened`);
  };
  return { navOptions, taskIds, listRef, listProps, tabIndexFor, focusRow, rowElement, selectedVisibleId, revealSelection, focusTaskOrStart, saveOffsets, closeAttention, closePanel, closeDetail, escapeLayer, select, navigateRelation, switchView, dimFor, showItem, openPanel };
}
export function useSupervisorViewOpeningFocus(context: SupervisorViewInputs & SupervisorViewModel & SupervisorViewNavigation & SupervisorViewPanel) {
  const { active, snapshot, initialRootSnapshot, opened, dialog, placement, panelKind, rootRef, focusTaskOrStart, root, rootId, focusedTask, taskIds, mode, lastTaskIds, scope, hasRetainedDraft, changed, focusRow, setFocusNotice, tasks, createdSelection, selectedAgent, detailSubagent, selectedNode, previousGraph, model, retainedDraftTaskId, focusWithinDetail, focusedGraph, rowElement } = context;
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
}

export function useSupervisorViewQuestionFocus(context: SupervisorViewInputs & SupervisorViewAttention & Pick<SupervisorViewNavigation, "focusTaskOrStart">) {
  const { active, questionHadFocus, decide, attentionOpen, setAttentionOpen, focusTaskOrStart } = context;
  useEffect(() => {
    if (!active || !questionHadFocus.current || decide) return;
    questionHadFocus.current = false;
    if (attentionOpen) { setAttentionOpen(false); requestAnimationFrame(() => focusTaskOrStart()); }
    else focusTaskOrStart();
  }, [active, decide]);
}

export function useSupervisorViewDialogFocus(context: SupervisorViewInputs & Pick<SupervisorViewModel, "mode">) {
  const { returnFocusIntent, dialog, rootRef, active, mode } = context;
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
}

export function useSupervisorViewSelectionFocus(context: SupervisorViewContext) {
  const { scope, tasks, changed, snapshot, createdSelection, active, dialog, sessionId, root, revealSelection, mode } = context;
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
}

export function useSupervisorViewOverlayFocus(context: SupervisorViewInputs & SupervisorViewPanel) {
  const { explicitFocus, active, placement, panelKind, rootRef } = context;
  useLayoutEffect(() => {
    if (explicitFocus.current) { explicitFocus.current = false; return; }
    if (active && placement === "overlay" && panelKind) rootRef.current?.querySelector<HTMLButtonElement>(".supervisor-detail-header button,.supervisor-queue-header button")?.focus({ preventScroll: true });
  }, [active, panelKind, placement]);
}
