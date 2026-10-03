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
const WORKING_MS = 15_000;
const IDLE_MS = 60_000;

export function useSubscriptionLimits(client: CockpitClient, agentsWorking: boolean): SubscriptionLimitsState {
  const [state, setState] = useState(() => snapshots.get(client) ?? { snapshot: null, link: "loading" as const, absent: false, now: Date.now() });
  const working = useRef(agentsWorking);
  working.current = agentsWorking;
  const previousWorking = useRef(agentsWorking);
  const updatePolling = useRef<((refreshNow: boolean) => void) | null>(null);
  useEffect(() => {
    let current: SubscriptionLimitsState = { ...(snapshots.get(client) ?? { snapshot: null, link: "loading", absent: false, now: Date.now() }), absent: false };
    let failures = current.link === "offline" ? 2 : 0;
    let stopped = false;
    let controller: AbortController | null = null;
    let timer: ReturnType<typeof setTimeout> | undefined;
    const publish = () => {
      snapshots.set(client, current);
      setState(current);
    };
    const schedule = () => {
      if (stopped || current.absent) return;
      clearTimeout(timer);
      timer = setTimeout(refresh, current.snapshot?.collecting ? COLLECTING_MS : working.current ? WORKING_MS : IDLE_MS);
    };
    const refresh = () => {
      if (stopped || current.absent || controller) return;
      if (document.visibilityState === "hidden") { schedule(); return; }
      current = { ...current, now: Date.now() };
      publish();
      const request = new AbortController();
      controller = request;
      void client.quotaStatus({ agents_working: working.current }, request.signal).then(snapshot => {
        if (stopped || request.signal.aborted) return;
        failures = 0;
        current = { snapshot, link: "live", absent: false, now: Date.now() };
        publish();
      }, error => {
        if (stopped || request.signal.aborted) return;
        const absent = error instanceof CockpitClientError && error.operationCode === "quota_unavailable";
        current = { ...current, absent, now: Date.now(), link: ++failures >= 2 ? "offline" : current.link };
        publish();
      }).finally(() => {
        if (controller === request) controller = null;
        schedule();
      });
    };
    updatePolling.current = refreshNow => {
      clearTimeout(timer);
      if (refreshNow) refresh();
      else schedule();
    };
    publish();
    refresh();
    document.addEventListener("visibilitychange", refresh);
    return () => {
      stopped = true;
      updatePolling.current = null;
      clearTimeout(timer);
      controller?.abort();
      document.removeEventListener("visibilitychange", refresh);
    };
  }, [client]);
  useEffect(() => {
    if (previousWorking.current !== agentsWorking) updatePolling.current?.(agentsWorking);
    previousWorking.current = agentsWorking;
  }, [agentsWorking]);
  return state;
}
