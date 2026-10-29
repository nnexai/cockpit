import { useCallback, useEffect, useRef, useState } from "react";
import type { CockpitClient } from "../../client/CockpitClient";
import type { OrchestrationAction, OrchestrationActionResult, OrchestrationSnapshot } from "../../protocol/generated/v1";

export type LiaisonAction = Extract<OrchestrationAction, { action: "supervisor_start" | "run_resume" | "run_start_fresh_supervisor" }>;
export type MutationRequestScope = Readonly<{ stateStoreId: string; sessionId: string; rootId: string | null }>;
export function acceptsSupervisorSnapshot(next: OrchestrationSnapshot, sessionId: string, rootId: string | null, revisionFloor: number): boolean {
  return next.session_id === sessionId && next.revision >= revisionFloor && (!rootId || next.board === null || next.board.root_id === rootId);
}

/** One subscription per workbench, shared by the terminal rail and task overview. */
export function useSupervisor(client: CockpitClient, sessionId: string, rootId: string | null, active: boolean, sessionLive: boolean) {
  const [snapshot, setSnapshot] = useState<OrchestrationSnapshot | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [connected, setConnected] = useState(false);
  const [busy, setBusy] = useState(false);
  const [refreshToken, setRefreshToken] = useState(0);
  const generation = useRef(0), floor = useRef(0), pending = useRef(false);
  const mutationFailure = useRef<unknown>(null);
  const identity = useRef({ client, sessionId, rootId, active, connected: connected && sessionLive });
  identity.current = { client, sessionId, rootId, active, connected: connected && sessionLive };
  const current = useRef<OrchestrationSnapshot | null>(null);
  const refresh = useCallback(() => setRefreshToken(value => value + 1), []);
  useEffect(() => {
    floor.current = 0; current.current = null; setSnapshot(null); setError(null); setConnected(false);
  }, [client, sessionId, rootId]);
  useEffect(() => {
    const scope = ++generation.current;
    let cancelled = false, timer = 0;
    const sleep = () => new Promise<void>(resolve => { timer = window.setTimeout(resolve, 1500); });
    const live = () => !cancelled && generation.current === scope;
    if (!active || !sessionLive) { setConnected(false); return () => { cancelled = true; }; }
    const observe = async () => {
      while (live()) {
        try {
          const next = await client.orchestrationSnapshot({ session_id: sessionId, root_id: rootId });
          if (!live()) return;
          if (!acceptsSupervisorSnapshot(next, sessionId, rootId, floor.current)) { await sleep(); continue; }
          floor.current = next.revision; current.current = next; setSnapshot(next); setConnected(true);
          let wait = await client.orchestrationWait({ root_id: rootId, after_revision: next.revision, after_tasks_token: next.tasks_token, timeout_ms: 30_000 });
          while (live() && !wait.changed) {
            wait = await client.orchestrationWait({ root_id: rootId, after_revision: wait.revision, after_tasks_token: wait.tasks_token, timeout_ms: 30_000 });
          }
          if (!live()) return;
          floor.current = Math.max(floor.current, wait.revision);
        } catch {
          if (!live()) return;
          setConnected(false); await sleep();
        }
      }
    };
    void observe();
    return () => { cancelled = true; generation.current++; window.clearTimeout(timer); };
  }, [client, sessionId, rootId, active, sessionLive, refreshToken]);
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
  return { snapshot, connected, busy, error, refresh, captureMutationScope, mutateResult, mutationFailure };
}
