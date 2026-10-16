import { useEffect, useMemo, useState } from "react";
import type { CockpitClient } from "../../client/CockpitClient";
import type { SessionSnapshotResponse, SpaceServiceStatus } from "../../protocol/generated/v1";

export const SPACE_SERVICE_POLL_MS = 5_000;

/** Read-only discovery continues even when no services have been registered yet. */
export function useSpaceServiceStatus(client: CockpitClient, sessionId: string | null, snapshot: SessionSnapshotResponse | null, shown: boolean) {
  const serverInstance = snapshot?.session_id === sessionId ? snapshot.server_instance : null;
  const [result, setResult] = useState<{ client: CockpitClient; sessionId: string; serverInstance: string; services: SpaceServiceStatus[] } | null>(null);
  useEffect(() => {
    if (!sessionId || !serverInstance || !shown) return;
    let disposed = false;
    let controller: AbortController | null = null;
    let timer: number | undefined;
    const poll = () => {
      if (disposed || document.visibilityState === "hidden" || controller) return;
      const current = new AbortController();
      controller = current;
      void client.spaceServiceStatus(sessionId, current.signal).then(response => {
        if (!disposed && !current.signal.aborted && response.session_id === sessionId) {
          setResult({ client, sessionId, serverInstance, services: response.services });
        }
      }, () => {
        // Keep the last successful read quietly; there is no separate error or unknown service state.
      }).finally(() => {
        if (controller !== current) return;
        controller = null;
        if (!disposed && document.visibilityState !== "hidden") timer = window.setTimeout(poll, SPACE_SERVICE_POLL_MS);
      });
    };
    const visibilityChanged = () => {
      clearTimeout(timer);
      if (document.visibilityState === "hidden") {
        controller?.abort();
        controller = null;
      } else poll();
    };
    document.addEventListener("visibilitychange", visibilityChanged);
    poll();
    return () => {
      disposed = true;
      clearTimeout(timer);
      document.removeEventListener("visibilitychange", visibilityChanged);
      controller?.abort();
    };
  }, [client, sessionId, serverInstance, shown]);

  return useMemo(() => {
    const bySpace = new Map<string, SpaceServiceStatus[]>();
    if (!snapshot || snapshot.session_id !== sessionId || result?.client !== client || result.sessionId !== sessionId || result.serverInstance !== serverInstance) return bySpace;
    const panes = new Map(snapshot.panes.map(pane => [pane.id, pane]));
    const tabs = new Map(snapshot.tabs.map(tab => [tab.id, tab]));
    const spaces = new Set(snapshot.spaces.map(space => space.id));
    for (const service of result.services) {
      const pane = panes.get(service.pane_id);
      const tab = pane ? tabs.get(pane.tab_id) : undefined;
      if (!pane || !tab || !spaces.has(service.space_id) || pane.space_id !== service.space_id || tab.space_id !== service.space_id || (service.tab_id !== null && service.tab_id !== tab.id)) continue;
      const services = bySpace.get(service.space_id);
      if (services) services.push(service);
      else bySpace.set(service.space_id, [service]);
    }
    return bySpace;
  }, [client, sessionId, serverInstance, snapshot, result]);
}
