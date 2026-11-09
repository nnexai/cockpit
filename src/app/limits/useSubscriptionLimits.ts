import { useEffect, useRef, useState } from "react";
import { CockpitClientError, type CockpitClient } from "../../client/CockpitClient";
import type { QuotaStatusResponse } from "../../protocol/generated/v1";

export interface SubscriptionLimitsState {
  snapshot: QuotaStatusResponse | null;
  link: "loading" | "live" | "offline";
  absent: boolean;
  now: number;
}

// Client identity prevents snapshots from one host leaking into another; Workbench resync retains it.
const snapshots = new WeakMap<CockpitClient, SubscriptionLimitsState>();
const COLLECTING_MS = 3000;
const REFRESH_MS = 60_000;

function initialState(client: CockpitClient): SubscriptionLimitsState {
  return snapshots.get(client) ?? { snapshot: null, link: "loading", absent: false, now: Date.now() };
}

export function useSubscriptionLimits(client: CockpitClient, agentsWorking: boolean): SubscriptionLimitsState {
  const [rendered, setRendered] = useState(() => ({ client, value: initialState(client) }));
  const working = useRef(agentsWorking);
  working.current = agentsWorking;
  const previousWorking = useRef(agentsWorking);
  const requestRefresh = useRef<(() => void) | null>(null);
  useEffect(() => {
    previousWorking.current = working.current;
    let current: SubscriptionLimitsState = { ...initialState(client), absent: false };
    let failures = current.link === "offline" ? 2 : 0;
    let stopped = false;
    let collectionPolling = true;
    let pending = false;
    let controller: AbortController | null = null;
    let timer: ReturnType<typeof setTimeout> | undefined;
    const publish = () => {
      snapshots.set(client, current);
      setRendered({ client, value: current });
    };
    const schedule = () => {
      clearTimeout(timer);
      if (stopped || current.absent || document.visibilityState === "hidden") return;
      if (current.snapshot?.collecting && collectionPolling) {
        timer = setTimeout(refresh, COLLECTING_MS);
        return;
      }
      const now = Date.now();
      let next = now + REFRESH_MS;
      for (const provider of current.snapshot?.providers ?? []) {
        for (const account of provider.accounts) {
          for (const limit of account.limits) {
            const reset = limit.resets_at_ms;
            if (reset !== null && Number.isFinite(reset) && reset > now) next = Math.min(next, reset);
          }
        }
      }
      timer = setTimeout(refresh, next - now);
    };
    const refresh = () => {
      if (stopped || current.absent || document.visibilityState === "hidden") return;
      clearTimeout(timer);
      if (controller) {
        pending = true;
        return;
      }
      pending = false;
      current = { ...current, now: Date.now() };
      publish();
      const request = new AbortController();
      controller = request;
      void client.quotaStatus({ agents_working: working.current }, request.signal).then(snapshot => {
        if (stopped || request.signal.aborted) return;
        collectionPolling = true;
        failures = 0;
        current = { snapshot, link: "live", absent: false, now: Date.now() };
        publish();
      }, error => {
        collectionPolling = false;
        if (stopped || request.signal.aborted) return;
        const absent = error instanceof CockpitClientError && error.operationCode === "quota_unavailable";
        current = { ...current, absent, now: Date.now(), link: ++failures >= 2 ? "offline" : current.link };
        publish();
      }).finally(() => {
        if (controller === request) controller = null;
        if (stopped) return;
        if (pending && document.visibilityState !== "hidden") refresh();
        else schedule();
      });
    };
    requestRefresh.current = refresh;
    publish();
    refresh();
    const onVisibilityChange = () => {
      clearTimeout(timer);
      if (document.visibilityState !== "hidden") refresh();
    };
    const onFocus = () => refresh();
    document.addEventListener("visibilitychange", onVisibilityChange);
    window.addEventListener("focus", onFocus);
    return () => {
      stopped = true;
      requestRefresh.current = null;
      clearTimeout(timer);
      controller?.abort();
      document.removeEventListener("visibilitychange", onVisibilityChange);
      window.removeEventListener("focus", onFocus);
    };
  }, [client]);
  useEffect(() => {
    if (previousWorking.current !== agentsWorking) requestRefresh.current?.();
    previousWorking.current = agentsWorking;
  }, [agentsWorking]);
  return rendered.client === client ? rendered.value : initialState(client);
}
