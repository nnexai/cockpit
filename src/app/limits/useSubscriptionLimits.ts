import { useEffect, useState } from "react";
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

export function useSubscriptionLimits(client: CockpitClient): SubscriptionLimitsState {
  const [state, setState] = useState(() => snapshots.get(client) ?? { snapshot: null, link: "loading" as const, absent: false, now: Date.now() });
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
      timer = setTimeout(refresh, current.snapshot?.collecting ? 3000 : 60_000);
    };
    const refresh = () => {
      if (stopped || current.absent || controller) return;
      if (document.visibilityState === "hidden") { schedule(); return; }
      current = { ...current, now: Date.now() };
      publish();
      const request = new AbortController();
      controller = request;
      void client.quotaStatus(request.signal).then(snapshot => {
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
    publish();
    refresh();
    document.addEventListener("visibilitychange", refresh);
    return () => {
      stopped = true;
      clearTimeout(timer);
      controller?.abort();
      document.removeEventListener("visibilitychange", refresh);
    };
  }, [client]);
  return state;
}
