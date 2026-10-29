import { useEffect, useRef, useState } from "react";
import type { Run, OrchestrationSnapshot } from "../../protocol/generated/v1";
import type { WorkbenchProps } from "./Workbench";
export interface LiaisonNavigation {
  supervisorTerminal(paneId: string, run?: Run, orchestration?: OrchestrationSnapshot): Promise<void>;
  supervisorNavigationError: string | null;
}
export function useLiaisonNavigation({ state, selection, onFocus }: WorkbenchProps, openConversation: () => void): LiaisonNavigation {
  const snapshot = state.snapshot;
  const [supervisorNavigation, setSupervisorNavigation] = useState<string | null>(null);
  const [supervisorNavigationError, setSupervisorNavigationError] = useState<string | null>(null);
  const supervisorNavigationAck = useRef<{ sessionId: string | null; token: number; resolve(): void; reject(error: Error): void } | null>(null);
  const supervisorTerminal = async (paneId: string, run?: Run, orchestration?: OrchestrationSnapshot) => {
    const pane = snapshot?.panes.find(candidate => candidate.id === paneId);
    if (state.sync !== "live" || !pane || !snapshot?.tabs.some(tab => tab.id === pane.tab_id && tab.space_id === pane.space_id)) throw new Error("Terminal membership is no longer current.");
    // Session server_instance is a SHA-256 prefix, not the raw endpoint identity.
    // Endpoint fencing comes from the fresh run observation and its launch receipt.
    const location = run?.attachment?.location;
    if (run && (orchestration?.runtime.status !== "fresh" || !location || run.attachment?.state === "revoked" || location.session_id !== state.sessionId || location.endpoint_identity !== orchestration.runtime.endpoint_identity || !orchestration.runtime.runs.some(observation => observation.run_id === run.run_id && observation.presence === "present" && observation.pane_id === pane.id && observation.workspace_id === pane.space_id && observation.tab_id === pane.tab_id))) throw new Error("The conversation's current Herdr location is not confirmed.");
    if (supervisorNavigationAck.current) throw new Error("Another terminal navigation is awaiting acknowledgement.");
    const promise = new Promise<void>((resolve, reject) => {
      supervisorNavigationAck.current = { sessionId: state.sessionId, token: state.focusToken + 1, resolve, reject };
    });
    setSupervisorNavigationError(null);
    openConversation();
    setSupervisorNavigation(pane.id);
    onFocus({ kind: "pane", target_id: pane.id }, { spaceId: pane.space_id, tabId: pane.tab_id, paneId: pane.id });
    return promise;
  };
  useEffect(() => {
    if (!supervisorNavigation) return;
    const ack = supervisorNavigationAck.current;
    // Viewer-to-terminal selection can reissue the same pane focus in a layout
    // effect. Follow that intent, but never a different selection's token.
    if (ack && state.focusToken > ack.token && (state.focusPending?.kind === "pane" && state.focusPending.target_id === supervisorNavigation || !state.focusPending && selection.paneId === supervisorNavigation && state.snapshot?.focused_pane_id === supervisorNavigation)) ack.token = state.focusToken;
    const superseded = !!ack && (state.focusToken > ack.token || state.focusToken === ack.token && !!state.focusPending && (state.focusPending.kind !== "pane" || state.focusPending.target_id !== supervisorNavigation));
    if (state.focusError || state.sync !== "live" || ack?.sessionId !== state.sessionId || superseded) {
      const message = superseded ? "Terminal focus was superseded by another selection." : state.focusError?.message ?? (ack?.sessionId !== state.sessionId ? "The session changed before terminal focus was confirmed." : "Connection lost before terminal focus was confirmed.");
      setSupervisorNavigationError(message);
      ack?.reject(new Error(message));
      supervisorNavigationAck.current = null;
      setSupervisorNavigation(null);
      return;
    }
    if (ack && state.focusToken === ack.token && !state.focusPending && state.snapshot?.focused_pane_id === supervisorNavigation) {
      supervisorNavigationAck.current?.resolve();
      supervisorNavigationAck.current = null;
      setSupervisorNavigation(null);
    }
  }, [supervisorNavigation, selection.paneId, state.sessionId, state.sync, state.focusToken, state.focusPending, state.focusError, state.snapshot]);
  useEffect(() => () => {
    supervisorNavigationAck.current?.reject(new Error("Workbench closed before terminal focus was confirmed."));
    supervisorNavigationAck.current = null;
  }, []);
  return { supervisorTerminal, supervisorNavigationError };
}
