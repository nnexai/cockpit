import { useCallback, useEffect, useRef, useState } from "react";
import type { CockpitClient } from "../../client/CockpitClient";
import type { OrchestrationAction, OrchestrationSnapshot } from "../../protocol/generated/v1";

// These operations carry an exact task/plan hash, an idempotent message ID, or append-only intent.
// Unrelated worker telemetry must not invalidate a human's already reviewed plan.
const LOCAL_FENCES: Partial<Record<OrchestrationAction["action"], true>> = {
  task_update: true, tasks_assign_ids: true, grant_prepare: true, grant_execute: true,
  accept: true, message_send: true, annotate: true,
};

export function acceptsSupervisorSnapshot(next: OrchestrationSnapshot, sessionId: string, rootId: string | null, revisionFloor: number): boolean {
  return next.session_id === sessionId && next.revision >= revisionFloor
    && (!rootId || next.board === null || next.board.root_id === rootId);
}

/** One scope generation fences long polls and snapshots. Drafts live outside snapshots. */
export function useSupervisor(client: CockpitClient, sessionId: string, rootId: string | null, active: boolean) {
  const [snapshot, setSnapshot] = useState<OrchestrationSnapshot | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [observationError, setObservationError] = useState<string | null>(null);
  const [connected, setConnected] = useState(false);
  const [busy, setBusy] = useState(false);
  const [refreshToken, setRefreshToken] = useState(0);
  const generation = useRef(0);
  const floor = useRef(0);
  const identity = useRef({ client, sessionId, rootId });
  identity.current = { client, sessionId, rootId };
  const current = useRef<OrchestrationSnapshot | null>(null);
  const pending = useRef(false);
  const refresh = useCallback(() => setRefreshToken(value => value + 1), []);
  useEffect(() => {
    floor.current = 0;
    current.current = null;
    pending.current = false;
    setSnapshot(null);
    setBusy(false);
    setError(null);
    setObservationError(null);
  }, [client, sessionId, rootId]);
  useEffect(() => {
    const scope = ++generation.current;
    let cancelled = false;
    let timer = 0;
    const sleep = () => new Promise<void>(resolve => { timer = window.setTimeout(resolve, 1500); });
    const live = () => !cancelled && generation.current === scope;
    if (!active) { setConnected(false); return () => { cancelled = true; }; }
    const observe = async () => {
      while (live()) {
        try {
          const next = await client.orchestrationSnapshot({ session_id: sessionId, root_id: rootId });
          if (!live()) return;
          if (!acceptsSupervisorSnapshot(next, sessionId, rootId, floor.current)) { await sleep(); continue; }
          floor.current = next.revision;
          current.current = next;
          setSnapshot(next);
          setConnected(true);
          setObservationError(null);
          const wait = await client.orchestrationWait({ after_revision: next.revision, after_tasks_token: next.tasks_token, timeout_ms: 2000 });
          if (!live()) return;
          floor.current = Math.max(floor.current, wait.revision);
          // Even unchanged durable state needs a fresh Herdr observation after the wait.
        } catch (failure) {
          if (!live()) return;
          setConnected(false);
          setObservationError(failure instanceof Error ? failure.message : "Supervisor connection unavailable");
          await sleep();
        }
      }
    };
    void observe();
    return () => { cancelled = true; generation.current++; window.clearTimeout(timer); };
  }, [client, sessionId, rootId, active, refreshToken]);
  const mutate = useCallback(async (action: OrchestrationAction): Promise<boolean> => {
    const observed = current.current;
    if (!active || !observed || pending.current) return false;
    const scope = identity.current;
    const sameScope = () => scope.client === identity.current.client && scope.sessionId === identity.current.sessionId && scope.rootId === identity.current.rootId;
    pending.current = true;
    setBusy(true);
    setError(null);
    try {
      const response = await client.orchestrationMutate({ session_id: sessionId, expected_revision: LOCAL_FENCES[action.action] ? null : observed.revision, action });
      if (!sameScope()) return false;
      floor.current = Math.max(floor.current, response.revision);
      refresh();
      return true;
    } catch (failure) {
      if (!sameScope()) return false;
      setError(failure instanceof Error ? failure.message : "The action was not confirmed. Review current state before retrying.");
      refresh();
      return false;
    } finally {
      if (sameScope()) { pending.current = false; setBusy(false); }
    }
  }, [client, sessionId, active, refresh]);
  return { snapshot, error: error ?? observationError, connected, busy, mutate, refresh };
}
