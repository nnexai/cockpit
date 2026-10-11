import { useEffect, useRef, useState, type Dispatch, type SetStateAction } from "react";
import type { SupervisorViewProps } from "./supervisorViewTypes";
import { useSupervisorDrafts } from "./useSupervisorDrafts";
import { useSupervisor } from "./useSupervisor";
import { useSupervisorLayout } from "./useSupervisorLayout";
import { useSupervisorViewState } from "./supervisorViewState";
import { useDetailFocus, useSupervisorViewNavigation, useSupervisorViewOpeningFocus, useSupervisorViewQuestionFocus, useSupervisorViewDialogFocus, useSupervisorViewSelectionFocus, useSupervisorViewOverlayFocus } from "./useDetailFocus";
import { useStartAgentFlow, useStartAgentFlowState, useStartAgentSessionReset } from "./useStartAgentFlow";
import { useArchiveCounts } from "./useArchiveCounts";
import { useSupervisorViewModel, supervisorViewPanel } from "./supervisorViewModel";
import { useSupervisorViewSourceActions } from "./supervisorViewSourceActions";
import { useSupervisorViewAttention, supervisorViewQueueRows } from "./supervisorViewAttention";
import { useSupervisorViewRevealLayout, useSupervisorViewQueueLayout } from "./supervisorViewLayoutEffects";
import { SupervisorViewSurface } from "./supervisorViewSurface";
import "./supervisor.css";

export function SupervisorView({ client, sessionId, session, runtimeLive, active, startToken, startIntent, navigationError, onClose, onTerminal, onModalChange }: SupervisorViewProps) {
  const [rootId, updateRootId] = useState<string | null>(null);
  const [projectRootMissing, setProjectRootMissing] = useState(false);
  const rootSelectionVersion = useRef(0);
  const setRootId: Dispatch<SetStateAction<string | null>> = value => { rootSelectionVersion.current++; setProjectRootMissing(false); updateRootId(value); };
  const supervisor = useSupervisor(client, sessionId, rootId, active);
  const stateStore = useRef<string | null>(null);
  const hydratedStore = useRef<string | null>(null);
  if (supervisor.snapshot?.session_id === sessionId) stateStore.current = supervisor.snapshot.state_store_id;
  const drafts = useSupervisorDrafts(stateStore.current);
  if (supervisor.snapshot?.state_store_id && hydratedStore.current !== supervisor.snapshot.state_store_id) {
    supervisor.hydrateDraftUncertainty(supervisor.snapshot.state_store_id, drafts.retainedSubmissions());
    hydratedStore.current = supervisor.snapshot.state_store_id;
  }
  const scope = drafts.scope(rootId ?? supervisor.snapshot?.board?.root_id ?? null);
  const changed = drafts.changed;
  const state = useSupervisorViewState();
  const focus = useDetailFocus();
  const startState = useStartAgentFlowState(sessionId, drafts.lifecycle, changed);
  const layout = useSupervisorLayout(focus.workareaRef);
  const modalVisible = !!state.dialog && active && !!supervisor.snapshot;
  useEffect(() => { onModalChange(modalVisible); return () => onModalChange(false); }, [modalVisible, onModalChange]);
  useStartAgentSessionReset({
    lastSession: startState.lastSession, sessionId, setRootId, setDialog: state.setDialog,
    setTerminalError: state.setTerminalError, setNotice: state.setNotice, setAttentionOpen: state.setAttentionOpen,
  });
  useEffect(() => { scope.view.detailTrail = []; changed(); }, [sessionId, rootId]);
  const model = useSupervisorViewModel({
    snapshot: supervisor.snapshot, rootId, connected: supervisor.connected, runtimeLive, session, scope,
    hoverSpace: state.hoverSpace, sessionId, draftStorageWarning: drafts.storageWarning, projectRootMissing,
  });
  const { snapshot } = supervisor;
  useEffect(() => {
    const workspaceId = runtimeLive ? session?.focused_space_id : null;
    if (!active || !supervisor.connected || !snapshot || snapshot.session_id !== sessionId || !workspaceId) return;
    let cancelled = false;
    const selectionVersion = rootSelectionVersion.current;
    void client.orchestrationProjectRootLookup({ session_id: sessionId, workspace_id: workspaceId, selected_root_id: rootId }).then(result => {
      if (cancelled || startState.startLock.current || rootSelectionVersion.current !== selectionVersion) return;
      if (result.status === "unavailable") { state.setNotice(`Can't identify this project's supervisor · ${result.message}`); return; }
      setProjectRootMissing(!result.preferred_root_id);
      if (result.preferred_root_id && result.roots.some(candidate => candidate.root_id === result.preferred_root_id)) {
        state.initialRootSnapshot.current = snapshot; setRootId(result.preferred_root_id);
      }
    }).catch(cause => { if (!cancelled) state.setNotice(cause instanceof Error ? cause.message : "This project's supervisor could not be identified. Your terminals are unchanged."); });
    return () => { cancelled = true; };
  }, [active, client, sessionId, session?.focused_space_id, runtimeLive, supervisor.connected, snapshot?.state_store_id]);
  const panel = supervisorViewPanel({
    scope, attentionOpen: state.attentionOpen, graphViewportHeight: state.graphViewportHeight,
    detailOpen: model.detailOpen, mode: model.mode, layout,
  });
  const navigation = useSupervisorViewNavigation({
    scope, layout, tasks: model.tasks, rootRef: focus.rootRef, graphRef: focus.graphRef, laneRefs: focus.laneRefs,
    selectedNode: model.selectedNode, mode: model.mode, bottomInset: panel.bottomInset,
    attentionOpen: state.attentionOpen, setAttentionOpen: state.setAttentionOpen,
    counterInvoker: focus.counterInvoker, panelInvoker: focus.panelInvoker, changed, detailInvoker: focus.detailInvoker,
    detailOpen: model.detailOpen, onClose, setDetailSection: state.setDetailSection,
    revealIntent: focus.revealIntent, explicitFocus: focus.explicitFocus, detailTask: model.detailTask,
    setFocusNotice: state.setFocusNotice, model: model.model, observe: model.observe,
  });
  // Keep layout and passive effects in their original relative order.
  useSupervisorViewRevealLayout({
    root: model.root, active, mode: model.mode, graphRef: focus.graphRef, listRef: navigation.listRef, scope, layout,
    laneRefs: focus.laneRefs, revealSelection: navigation.revealSelection, revealIntent: focus.revealIntent,
    saveOffsets: navigation.saveOffsets, workareaRef: focus.workareaRef, setGraphViewportHeight: state.setGraphViewportHeight,
    rootId, detailOpen: model.detailOpen, bounds: panel.bounds, placement: panel.placement,
  });
  useSupervisorViewOpeningFocus({
    active, snapshot, initialRootSnapshot: state.initialRootSnapshot, opened: state.opened, dialog: state.dialog,
    placement: panel.placement, panelKind: panel.panelKind, rootRef: focus.rootRef,
    focusTaskOrStart: navigation.focusTaskOrStart, root: model.root, rootId, focusedTask: focus.focusedTask,
    taskIds: navigation.taskIds, mode: model.mode, lastTaskIds: focus.lastTaskIds, scope,
    hasRetainedDraft: model.hasRetainedDraft, changed, focusRow: navigation.focusRow, setFocusNotice: state.setFocusNotice,
    tasks: model.tasks, createdSelection: focus.createdSelection, selectedAgent: model.selectedAgent,
    detailSubagent: model.detailSubagent, selectedNode: model.selectedNode, previousGraph: focus.previousGraph,
    model: model.model, retainedDraftTaskId: model.retainedDraftTaskId, rowElement: navigation.rowElement,
  });
  const startFlow = useStartAgentFlow({
    snapshot, connected: supervisor.connected, busy: supervisor.busy, startLock: startState.startLock,
    startUnknown: startState.startUnknown, rootState: model.rootState, destination: model.destination,
    setDialog: state.setDialog, startFocus: focus.startFocus, setStartPending: startState.setStartPending,
    setNotice: state.setNotice, mutateResult: supervisor.mutateResult, captureMutationScope: supervisor.captureMutationScope,
    setRootId, setStartUnknown: startState.setStartUnknown, startToken, startIntent, seenStart: startState.seenStart,
    active, root: model.root, focusTaskOrStart: navigation.focusTaskOrStart, runtimeLive,
    resumeOperations: startState.resumeOperations, startDraft: startState.startDraft, changed, sessionId,
  });
  const sourceActions = useSupervisorViewSourceActions({
    snapshot, live: model.live, busy: supervisor.busy, setTerminalError: state.setTerminalError,
    onTerminal, connected: supervisor.connected, runtimeLive, refresh: supervisor.refresh,
    mutateResult: supervisor.mutateResult, root: model.root, scope, sessionId, setDialog: state.setDialog,
  });
  const attention = useSupervisorViewAttention({
    snapshot, root: model.root, startUnknown: startState.startUnknown, terminalError: state.terminalError,
    navigationError, error: supervisor.error, connected: supervisor.connected, orphanedWorkers: model.orphanedWorkers,
    rootRuns: model.rootRuns, runtimeLive, tasks: model.tasks, rootState: model.rootState, scope,
  });
  useSupervisorViewQuestionFocus({
    active, questionHadFocus: focus.questionHadFocus, decide: attention.decide, attentionOpen: state.attentionOpen,
    setAttentionOpen: state.setAttentionOpen, focusTaskOrStart: navigation.focusTaskOrStart,
  });
  useSupervisorViewDialogFocus({
    returnFocusIntent: focus.returnFocusIntent, dialog: state.dialog, rootRef: focus.rootRef, active, mode: model.mode,
  });
  useSupervisorViewSelectionFocus({
    scope, tasks: model.tasks, changed, snapshot, createdSelection: focus.createdSelection, active,
    dialog: state.dialog, sessionId, root: model.root, revealSelection: navigation.revealSelection, mode: model.mode,
  });
  useSupervisorViewOverlayFocus({
    explicitFocus: focus.explicitFocus, active, placement: panel.placement, panelKind: panel.panelKind, rootRef: focus.rootRef,
  });
  const archiveCounts = useArchiveCounts({
    client, sessionId, rootId, active, scope, snapshot, closedRoots: model.closedRoots,
  });
  const queue = supervisorViewQueueRows({
    attention: attention.attention, snapshot, tasks: model.tasks, question: attention.question, answer: attention.answer,
    root: model.root, changed, busy: supervisor.busy, live: model.live, rootState: model.rootState,
    mutateResult: supervisor.mutateResult, captureMutationScope: supervisor.captureMutationScope,
    connected: supervisor.connected, refresh: supervisor.refresh, setRootId, setNotice: state.setNotice, runtimeLive,
    editPrerequisites: sourceActions.editPrerequisites, switchView: navigation.switchView, showItem: navigation.showItem,
    attentionOpen: state.attentionOpen, setAttentionOpen: state.setAttentionOpen, select: navigation.select,
    startPending: startState.startPending, start: startFlow.start, navigate: sourceActions.navigate, restart: startFlow.restart,
    scope, mode: model.mode, rootRuns: model.rootRuns, model: model.model, selectedNode: model.selectedNode,
    selectedTask: model.selectedTask, detailRun: model.detailRun, detailSubagent: model.detailSubagent, detailTask: model.detailTask,
  });
  useSupervisorViewQueueLayout({
    workareaRef: focus.workareaRef, rootRef: focus.rootRef, layout, setInlineQueueCap: state.setInlineQueueCap,
    mode: model.mode, rootId, rows: queue.rows, banner: model.banner, active,
  });
  return <SupervisorViewSurface view={{
    client, sessionId, session, runtimeLive, active, startToken, startIntent, navigationError, onClose, onTerminal, onModalChange,
    rootId, setRootId, projectRootMissing, scope, changed, lifecycleDrafts: drafts.lifecycle, draftStorageWarning: drafts.storageWarning,
    ...supervisor, ...state, ...focus, ...startState, ...model, ...panel, ...navigation, ...startFlow, ...sourceActions, ...attention,
    ...queue, archiveCounts,
  }} />;
}
