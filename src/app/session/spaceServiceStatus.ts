import { useEffect, useMemo, useRef, useState } from "react";
import type { CockpitClient } from "../../client/CockpitClient";
import type { SessionSnapshotResponse, SpaceServiceStatus } from "../../protocol/generated/v1";

export const SPACE_SERVICE_POLL_MS = 5_000;
export const SPACE_SERVICE_DEBOUNCE_MS = 100;

function sameServices(left: readonly SpaceServiceStatus[], right: readonly SpaceServiceStatus[]): boolean {
  return left.length === right.length && left.every((service, index) => {
    const other = right[index];
    return service.key === other.key && service.pane_id === other.pane_id && service.tab_id === other.tab_id
      && service.title === other.title && service.status === other.status && service.port === other.port;
  });
}

/** Discover on pane-set changes; only health-port services need recurring reads. */
export function useSpaceServiceStatus(client: CockpitClient, sessionId: string | null, snapshot: SessionSnapshotResponse | null, shown: boolean) {
  const serverInstance = snapshot?.session_id === sessionId ? snapshot.server_instance : null;
  const paneSet = JSON.stringify([...new Set(serverInstance ? snapshot?.panes.map(pane => pane.id) : [])].sort());
  const request = useMemo(() => ({ server_instance: serverInstance ?? "", pane_ids: JSON.parse(paneSet) as string[] }), [serverInstance, paneSet]);
  const previousScope = useRef<{ client: CockpitClient; sessionId: string; serverInstance: string; paneIds: readonly string[] } | null>(null);
  const [result, setResult] = useState<{ client: CockpitClient; sessionId: string; serverInstance: string; services: SpaceServiceStatus[] } | null>(null);
  const latestResult = useRef(result);
  latestResult.current = result;
  useEffect(() => {
    if (!sessionId || !serverInstance || !shown) { previousScope.current = null; return; }
    const previousRequest = previousScope.current;
    const debounce = previousRequest?.client === client && previousRequest.sessionId === sessionId;
    let settleAddedPanes = debounce && previousRequest.serverInstance === serverInstance
      && request.pane_ids.some(id => !previousRequest.paneIds.includes(id));
    previousScope.current = { client, sessionId, serverInstance, paneIds: request.pane_ids };
    let disposed = false;
    let controller: AbortController | null = null;
    let timer: number | undefined;
    const settlementTimers: number[] = [];
    const cancelSettlement = () => { settleAddedPanes = false; settlementTimers.forEach(clearTimeout); };
    const previous = latestResult.current;
    let refreshHealth = previous?.client === client && previous.sessionId === sessionId && previous.serverInstance === serverInstance
      && previous.services.some(service => service.port !== null && request.pane_ids.includes(service.pane_id));
    const poll = () => {
      if (disposed || document.visibilityState === "hidden" || controller) return;
      clearTimeout(timer);
      if (settleAddedPanes) {
        settleAddedPanes = false;
        // Plugin registration follows pane creation and can lag the snapshot event.
        settlementTimers.push(window.setTimeout(poll, 1_000), window.setTimeout(poll, 3_000));
      }
      const current = new AbortController();
      controller = current;
      void client.spaceServiceStatus(sessionId, request, current.signal).then(response => {
        if (!disposed && !current.signal.aborted && response.session_id === sessionId) {
          refreshHealth = response.services.some(service => service.port !== null && request.pane_ids.includes(service.pane_id));
          // An unchanged poll must not re-render the whole sidebar every few seconds.
          setResult(previous => previous && previous.client === client && previous.sessionId === sessionId && previous.serverInstance === serverInstance
            && sameServices(previous.services, response.services) ? previous : { client, sessionId, serverInstance, services: response.services });
        }
      }, () => {
        // Keep the last successful read quietly; there is no separate error or unknown service state.
      }).finally(() => {
        if (controller !== current) return;
        controller = null;
        if (!disposed && document.visibilityState !== "hidden" && refreshHealth) timer = window.setTimeout(poll, SPACE_SERVICE_POLL_MS);
      });
    };
    const visibilityChanged = () => {
      clearTimeout(timer);
      cancelSettlement();
      if (document.visibilityState === "hidden") {
        controller?.abort();
        controller = null;
      } else poll();
    };
    document.addEventListener("visibilitychange", visibilityChanged);
    if (debounce) timer = window.setTimeout(poll, SPACE_SERVICE_DEBOUNCE_MS);
    else poll();
    return () => {
      disposed = true;
      cancelSettlement();
      clearTimeout(timer);
      document.removeEventListener("visibilitychange", visibilityChanged);
      controller?.abort();
    };
  }, [client, sessionId, serverInstance, shown, request]);

  return useMemo(() => {
    const bySpace = new Map<string, SpaceServiceStatus[]>();
    if (!snapshot || snapshot.session_id !== sessionId || result?.client !== client || result.sessionId !== sessionId || result.serverInstance !== serverInstance) return bySpace;
    const panes = new Map(snapshot.panes.map(pane => [pane.id, pane]));
    const tabs = new Map(snapshot.tabs.map(tab => [tab.id, tab]));
    const spaces = new Set(snapshot.spaces.map(space => space.id));
    for (const service of result.services) {
      const pane = panes.get(service.pane_id);
      const tab = pane ? tabs.get(pane.tab_id) : undefined;
      if (!pane || !tab || !spaces.has(pane.space_id) || tab.space_id !== pane.space_id || (service.tab_id !== null && service.tab_id !== tab.id)) continue;
      const services = bySpace.get(pane.space_id);
      if (services) services.push(service);
      else bySpace.set(pane.space_id, [service]);
    }
    return bySpace;
  }, [client, sessionId, serverInstance, snapshot, result]);
}
