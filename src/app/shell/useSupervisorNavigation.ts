import { useEffect, useRef, useState } from "react";
import type { Run, OrchestrationSnapshot } from "../../protocol/generated/v1";
import type { WorkbenchProps } from "./Workbench";
export interface SupervisorNavigation {
  supervisorTerminal(paneId: string, run?: Run, orchestration?: OrchestrationSnapshot): Promise<void>;
  supervisorNavigationError: string | null;
}
export function useSupervisorNavigation({ state, onFocus }: WorkbenchProps, supervisorOpen: boolean, closeSupervisor: () => void): SupervisorNavigation {
  const snapshot = state.snapshot;
  const [supervisorNavigation, setSupervisorNavigation] = useState<string | null>(null);
  const [supervisorNavigationError, setSupervisorNavigationError] = useState<string | null>(null);
  const supervisorNavigationAck = useRef<{ sessionId: string | null; resolve(): void; reject(error: Error): void } | null>(null);
  const supervisorTerminal = async (paneId: string, run?: Run, orchestration?: OrchestrationSnapshot) => {
    const pane = snapshot?.panes.find(candidate => candidate.id === paneId);
    if (state.sync !== "live" || !pane || !snapshot?.tabs.some(tab => tab.id === pane.tab_id && tab.space_id === pane.space_id)) throw new Error("Terminal membership is no longer current. Refresh observation.");
    // Session server_instance is a SHA-256 prefix, not the raw endpoint identity.
    // Endpoint fencing comes from the fresh run observation and its launch receipt.
    if (run && (orchestration?.runtime.status !== "fresh" || !run.location || run.location.session_id !== state.sessionId || run.location.endpoint_identity !== orchestration.runtime.endpoint_identity || !orchestration.runtime.runs.some(observation => observation.run_id === run.run_id && observation.presence === "present" && observation.pane_id === pane.id && observation.workspace_id === pane.space_id && observation.tab_id === pane.tab_id))) throw new Error("The run's current Herdr location is not confirmed. Reconcile before navigating.");
    if (supervisorNavigationAck.current) throw new Error("Another terminal navigation is awaiting acknowledgement.");
    const promise = new Promise<void>((resolve, reject) => {
      supervisorNavigationAck.current = { sessionId: state.sessionId, resolve, reject };
    });
    setSupervisorNavigationError(null);
    setSupervisorNavigation(pane.id);
    onFocus({ kind: "pane", target_id: pane.id }, { spaceId: pane.space_id, tabId: pane.tab_id, paneId: pane.id });
    return promise;
  };
  useEffect(() => {
    if (!supervisorNavigation) return;
    if (!supervisorOpen || state.focusError || state.sync !== "live" || supervisorNavigationAck.current?.sessionId !== state.sessionId) {
      const message = state.focusError?.message ?? (!supervisorOpen ? "Terminal navigation was cancelled when the view was hidden." : supervisorNavigationAck.current?.sessionId !== state.sessionId ? "The session changed before terminal focus was confirmed." : "Connection lost before terminal focus was confirmed.");
      setSupervisorNavigationError(message);
      supervisorNavigationAck.current?.reject(new Error(message));
      supervisorNavigationAck.current = null;
      setSupervisorNavigation(null);
      return;
    }
    if (!state.focusPending && state.snapshot?.focused_pane_id === supervisorNavigation) {
      supervisorNavigationAck.current?.resolve();
      supervisorNavigationAck.current = null;
      setSupervisorNavigation(null);
      closeSupervisor();
    }
  }, [supervisorNavigation, supervisorOpen, state.sessionId, state.sync, state.focusPending, state.focusError, state.snapshot, closeSupervisor]);
  useEffect(() => () => {
    supervisorNavigationAck.current?.reject(new Error("Workbench closed before terminal focus was confirmed."));
    supervisorNavigationAck.current = null;
  }, []);
  return { supervisorTerminal, supervisorNavigationError };
}
