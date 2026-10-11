import { useEffect, useRef, useState, type Dispatch, type SetStateAction } from "react";
import type { CockpitClient } from "../../client/CockpitClient";
import type { OrchestrationSnapshot, Run, SessionSnapshotResponse } from "../../protocol/generated/v1";
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

export function SupervisorView({ client, sessionId, session, runtimeLive, active, startToken, startIntent, navigationError, onClose, onTerminal, onModalChange }: {
  client: CockpitClient; sessionId: string; session: SessionSnapshotResponse | null; runtimeLive: boolean; active: boolean; startToken: number; startIntent?: "reuse_or_resume" | "another";
  navigationError: string | null; onClose(): void; onTerminal(run: Run, snapshot: OrchestrationSnapshot): Promise<void>; onModalChange(open: boolean): void;
}) {
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
  const inputs = {
    client, sessionId, session, runtimeLive, active, startToken, startIntent, navigationError, onClose, onTerminal, onModalChange,
    rootId, setRootId, projectRootMissing, scope, changed, lifecycleDrafts: drafts.lifecycle, draftStorageWarning: drafts.storageWarning,
    ...supervisor, ...state, ...focus, ...startState,
  };
  const modalVisible = !!state.dialog && active && !!supervisor.snapshot;
  useEffect(() => { onModalChange(modalVisible); return () => onModalChange(false); }, [modalVisible, onModalChange]);
  useStartAgentSessionReset(inputs);
  useEffect(() => { scope.view.detailTrail = []; changed(); }, [sessionId, rootId]);
  const model = useSupervisorViewModel(inputs);
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
  const panel = supervisorViewPanel({ ...inputs, ...model, layout });
  const navigation = useSupervisorViewNavigation({ ...inputs, ...model, ...panel });
  const workarea = { ...inputs, ...model, ...panel, ...navigation };
  // Keep layout and passive effects in their original relative order.
  useSupervisorViewRevealLayout(workarea);
  useSupervisorViewOpeningFocus(workarea);
  const startFlow = useStartAgentFlow(workarea);
  const sourceActions = useSupervisorViewSourceActions(workarea);
  const attention = useSupervisorViewAttention(workarea);
  const context = { ...workarea, ...startFlow, ...sourceActions, ...attention };
  useSupervisorViewQuestionFocus(context);
  useSupervisorViewDialogFocus(context);
  useSupervisorViewSelectionFocus(context);
  useSupervisorViewOverlayFocus(context);
  const archiveCounts = useArchiveCounts(context);
  const queue = supervisorViewQueueRows(context);
  useSupervisorViewQueueLayout({ ...context, ...queue });
  return <SupervisorViewSurface view={{ ...context, ...queue, archiveCounts }} />;
}
