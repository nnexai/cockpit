import { useCallback, useEffect, useRef, useState } from "react";
import type { CockpitClient } from "../../client/CockpitClient";
import type { ClosedRunSummary, OrchestrationAction, OrchestrationActionResult, OrchestrationOverview, Run } from "../../protocol/generated/v1";
import { orchestrationChanges } from "./orchestrationChanges";

export type SupervisorOverview = Omit<OrchestrationOverview, "runs"> & { runs: (Run | ClosedRunSummary)[] };

export type LiaisonAction = Extract<OrchestrationAction, { action: "supervisor_start" | "run_resume" | "run_start_fresh_supervisor" }>;
export type MutationRequestScope = Readonly<{ stateStoreId: string; sessionId: string; rootId: string | null }>;
export function acceptsSupervisorSnapshot(next: OrchestrationOverview, sessionId: string, rootId: string | null, revisionFloor: number): boolean {
  return next.session_id === sessionId && next.revision >= revisionFloor && (!rootId || next.board === null || next.board.root_id === rootId);
}

/** One subscription per workbench, shared by the terminal rail and task overview. */
export function useSupervisor(client: CockpitClient, sessionId: string, rootId: string | null, active: boolean, sessionLive: boolean, serverInstance: string | null) {
  const [snapshot, setSnapshot] = useState<SupervisorOverview | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [connected, setConnected] = useState(false);
  const [busy, setBusy] = useState(false);
  const generation = useRef(0), floor = useRef(0), pending = useRef(false);
  const observer = useRef<{ refresh(): Promise<void>; close(): void } | null>(null);
  const observedScope = useRef<{ client: CockpitClient; sessionId: string; rootId: string | null; serverInstance: string } | null>(null);
  const mutationFailure = useRef<unknown>(null);
  const scopeMatches = observedScope.current?.client === client && observedScope.current.sessionId === sessionId && observedScope.current.rootId === rootId && observedScope.current.serverInstance === serverInstance;
  const identity = useRef({ client, sessionId, rootId, active, connected: connected && sessionLive && scopeMatches });
  identity.current = { client, sessionId, rootId, active, connected: connected && sessionLive && scopeMatches };
  const current = useRef<SupervisorOverview | null>(null);
  const refresh = useCallback(() => { void observer.current?.refresh().catch(() => {}); }, []);
  useEffect(() => {
    floor.current = 0; current.current = null; setSnapshot(null); setError(null); setConnected(false);
  }, [client, sessionId, rootId]);
  useEffect(() => {
    const scope = ++generation.current;
    let cancelled = false;
    let store: string | undefined = current.current?.state_store_id;
    const live = () => !cancelled && generation.current === scope;
    if (!active || !sessionLive || !serverInstance) { setConnected(false); return () => { cancelled = true; }; }
    setConnected(false);
    const read = async (valid: () => boolean, authoritative: boolean) => {
      if (authoritative && live() && valid()) { identity.current.connected = false; setConnected(false); }
      const next = await client.orchestrationSnapshot({ session_id: sessionId, root_id: rootId });
      if (!live() || !valid()) return;
      if (!acceptsSupervisorSnapshot(next, sessionId, rootId, floor.current) || (store !== undefined && next.state_store_id !== store)) throw new Error("Supervisor observation identity or revision changed");
      store ??= next.state_store_id;
      const overview: SupervisorOverview = { ...next, runs: next.runs.map(item => item.run) };
      floor.current = next.revision; current.current = overview; setSnapshot(overview);
      if (authoritative) { observedScope.current = { client, sessionId, rootId, serverInstance }; setConnected(true); }
    };
    const changes = orchestrationChanges(client, async () => {
      await read(live, false);
      if (!live() || !store) throw new Error("Supervisor observation cancelled");
      return { kind: "orchestration", session_id: sessionId, server_instance: serverInstance, state_store_id: store, root_id: rootId };
    }, valid => read(valid, true), () => { if (live()) { identity.current.connected = false; setConnected(false); } });
    observer.current = changes;
    return () => { cancelled = true; generation.current++; changes.close(); if (observer.current === changes) observer.current = null; };
  }, [client, sessionId, rootId, active, sessionLive, serverInstance]);
  const captureMutationScope = useCallback((action: LiaisonAction): MutationRequestScope | null => {
    const observed = current.current;
    return observed ? { stateStoreId: observed.state_store_id, sessionId: identity.current.sessionId, rootId: action.action === "supervisor_start" ? null : identity.current.rootId } : null;
  }, []);
  const mutateResult = useCallback(async (action: LiaisonAction, retainedScope: MutationRequestScope): Promise<OrchestrationActionResult | null> => {
    const observed = current.current, captured = identity.current;
    mutationFailure.current = null;
    if (!captured.active || !captured.connected || !observed || observed.runtime.status !== "fresh" || pending.current) return null;
    if (retainedScope.stateStoreId !== observed.state_store_id || retainedScope.sessionId !== captured.sessionId || action.action !== "supervisor_start" && retainedScope.rootId !== captured.rootId) {
      setError("Return to the original conversation before retrying. The original request is kept."); return null;
    }
    const sameScope = () => captured.client === identity.current.client && captured.sessionId === identity.current.sessionId && captured.rootId === identity.current.rootId;
    pending.current = true; setBusy(true); setError(null);
    try {
      const response = await captured.client.orchestrationMutate({ session_id: retainedScope.sessionId, expected_revision: null, action });
      if (!sameScope()) return null;
      floor.current = Math.max(floor.current, response.revision); refresh(); return response.result;
    } catch (failure) {
      mutationFailure.current = failure;
      if (sameScope()) { setError(failure instanceof Error ? failure.message : "The action was not confirmed. The original request is kept."); refresh(); }
      return null;
    } finally { pending.current = false; setBusy(false); }
  }, [refresh]);
  return { snapshot, connected: connected && sessionLive && scopeMatches, busy, error, refresh, captureMutationScope, mutateResult, mutationFailure };
}
