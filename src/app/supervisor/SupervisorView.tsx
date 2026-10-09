import { useEffect, useState } from "react";
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

export function SupervisorView({ client, sessionId, session, runtimeLive, active, startToken, navigationError, onClose, onTerminal, onModalChange }: {
  client: CockpitClient; sessionId: string; session: SessionSnapshotResponse | null; runtimeLive: boolean; active: boolean; startToken: number;
  navigationError: string | null; onClose(): void; onTerminal(run: Run, snapshot: OrchestrationSnapshot): Promise<void>; onModalChange(open: boolean): void;
}) {
  const [rootId, setRootId] = useState<string | null>(null);
  const supervisor = useSupervisor(client, sessionId, rootId, active);
  const drafts = useSupervisorDrafts(sessionId);
  const scope = drafts.scope(rootId ?? supervisor.snapshot?.board?.root_id ?? null);
  const changed = drafts.changed;
  const state = useSupervisorViewState();
  const focus = useDetailFocus();
  const startState = useStartAgentFlowState(sessionId);
  const layout = useSupervisorLayout(focus.workareaRef);
  const inputs = {
    client, sessionId, session, runtimeLive, active, startToken, navigationError, onClose, onTerminal, onModalChange,
    rootId, setRootId, scope, changed, ...supervisor, ...state, ...focus, ...startState,
  };
  const modalVisible = !!state.dialog && active && !!supervisor.snapshot;
  useEffect(() => { onModalChange(modalVisible); return () => onModalChange(false); }, [modalVisible, onModalChange]);
  useStartAgentSessionReset(inputs);
  useEffect(() => { scope.view.detailTrail = []; changed(); }, [sessionId, rootId]);
  const model = useSupervisorViewModel(inputs);
  const { snapshot } = supervisor;
  const { openRoots } = model;
  useEffect(() => {
    if (snapshot && !rootId && openRoots.length) { state.initialRootSnapshot.current = snapshot; setRootId(openRoots[0].root_id); }
  }, [snapshot, rootId]);
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
