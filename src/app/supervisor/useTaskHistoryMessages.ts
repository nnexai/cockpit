import { useCallback, useEffect, useRef, useState } from "react";
import type { CockpitClient } from "../../client/CockpitClient";
import type { TaskHistoryMessages } from "../../protocol/generated/v1";

export type HistoryScope = { sessionId: string; stateStoreId: string; rootId: string; taskId: string };
type HistoryState = { client: CockpitClient; key: string; data: TaskHistoryMessages | null; pending: boolean; error: boolean; retrying: boolean };

/** Keep at most the selected task's evidence, fenced by its complete identity. */
export function useTaskHistoryMessages(client: CockpitClient, scope: HistoryScope | null, revision: number, connected = true, tasksToken = "") {
  const key = scope ? JSON.stringify([scope.sessionId, scope.stateStoreId, scope.rootId, scope.taskId]) : null;
  const [state, setState] = useState<HistoryState | null>(null);
  const cache = useRef<{ client: CockpitClient; key: string; data: TaskHistoryMessages } | null>(null);
  const revisionRef = useRef(revision); revisionRef.current = revision;
  const tokenRef = useRef(tasksToken); tokenRef.current = tasksToken;
  const request = useRef<((retry: boolean) => void) | null>(null);
  const retry = useCallback(() => request.current?.(true), []);
  useEffect(() => {
    if (!scope || !key) { cache.current = null; request.current = null; setState(null); return; }
    if (cache.current && (cache.current.client !== client || cache.current.key !== key)) {
      cache.current = null;
      setState(null);
    }
    if (!connected) { request.current = null; return; }
    const { sessionId, stateStoreId, rootId, taskId } = scope;
    const controller = new AbortController();
    let running = false, cancelled = false, completedFloor = -1, completedToken = "";
    let retained = cache.current?.data ?? null;
    let digest = retained?.digest ?? null;
    const start = (isRetry: boolean) => {
      if (running || cancelled) return;
      running = true;
      const floor = Math.max(revisionRef.current, retained?.revision ?? 0), token = tokenRef.current;
      setState(previous => ({ client, key, data: previous?.client === client && previous.key === key ? previous.data : null, pending: true, error: isRetry && previous?.key === key ? previous.error : false, retrying: isRetry }));
      const load = async () => {
        try {
          const input = { session_id: sessionId, state_store_id: stateStoreId, root_id: rootId, task_id: taskId, ...(digest ? { known_digest: digest } : {}) };
          const response = await client.orchestrationTaskHistory(input, controller.signal);
          if (cancelled) return;
          if (response.session_id !== sessionId || response.state_store_id !== stateStoreId || response.root_id !== rootId || response.task_id !== taskId || response.revision < floor) throw new Error("History identity or revision mismatch");
          if (response.not_modified && (!digest || response.digest !== digest || !retained)) throw new Error("History digest mismatch");
          if (response.not_modified) retained = { ...retained!, revision: response.revision };
          else retained = response;
          digest = response.digest;
          cache.current = { client, key, data: retained };
          completedFloor = response.revision; completedToken = token;
          if (response.revision >= revisionRef.current && token === tokenRef.current) setState({ client, key, data: retained, pending: false, error: false, retrying: false });
        } catch {
          if (cancelled) return;
          completedFloor = floor; completedToken = token;
          setState(previous => ({ client, key, data: previous?.client === client && previous.key === key ? previous.data : null, pending: false, error: true, retrying: false }));
        } finally {
          running = false;
          if (!cancelled && (revisionRef.current > completedFloor || tokenRef.current !== completedToken)) start(false);
        }
      };
      void load();
    };
    request.current = start;
    start(false);
    return () => { cancelled = true; controller.abort(); if (request.current === start) request.current = null; };
  }, [client, key, connected]);
  useEffect(() => { request.current?.(false); }, [revision, tasksToken]);
  const current = key && state?.client === client && state.key === key ? state : null;
  return { data: current?.data ?? null, pending: connected && (current?.pending ?? true), error: connected && (current?.error ?? false), retrying: current?.retrying ?? false, disconnected: !connected, retry };
}
