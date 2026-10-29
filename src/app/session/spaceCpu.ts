import { useEffect, useMemo, useState } from "react";
import type { CockpitClient } from "../../client/CockpitClient";
import type { SessionSnapshotResponse, SpaceCpuStatus } from "../../protocol/generated/v1";

export type SpaceCpu = SpaceCpuStatus;
export const SPACE_CPU_POLL_MS = 5_000;

/** Poll only for a visible sidebar and page. Never retain a stale badge across scopes. */
export function useSpaceCpu(client: CockpitClient, sessionId: string | null, snapshot: SessionSnapshotResponse | null, shown: boolean): ReadonlyMap<string, SpaceCpu> {
  const serverInstance = snapshot?.session_id === sessionId ? snapshot.server_instance : null;
  const paneSet = JSON.stringify((serverInstance ? snapshot?.panes ?? [] : []).map(pane => ({ pane_id: pane.id, terminal_id: pane.terminal_id, space_id: pane.space_id })).sort((a, b) => a.pane_id.localeCompare(b.pane_id)));
  const request = useMemo(() => ({ server_instance: serverInstance ?? "", panes: JSON.parse(paneSet) as { pane_id: string; terminal_id: string; space_id: string }[] }), [serverInstance, paneSet]);
  const [result, setResult] = useState<{ scope: string; spaces: SpaceCpu[] } | null>(null);
  const scope = JSON.stringify([sessionId, serverInstance, paneSet]);
  useEffect(() => {
    if (!sessionId || !serverInstance || !shown) return;
    let disposed = false;
    let timer: number | undefined;
    let controller: AbortController | null = null;
    let generation = 0;
    const poll = () => {
      if (disposed || document.visibilityState !== "visible") return;
      controller = new AbortController();
      const current = ++generation;
      client.spaceCpuStatus(sessionId, request, controller.signal).then(response => {
        if (!disposed && current === generation) setResult({ scope, spaces: response.available && response.session_id === sessionId && response.server_instance === serverInstance ? response.spaces : [] });
      }).catch(() => { if (!disposed && current === generation) setResult(null); }).finally(() => {
        if (!disposed && current === generation && document.visibilityState === "visible") timer = window.setTimeout(poll, SPACE_CPU_POLL_MS);
      });
    };
    const visibilityChanged = () => {
      generation++;
      clearTimeout(timer);
      controller?.abort();
      controller = null;
      setResult(null);
      if (document.visibilityState === "visible") poll();
    };
    document.addEventListener("visibilitychange", visibilityChanged);
    poll();
    return () => { disposed = true; clearTimeout(timer); controller?.abort(); document.removeEventListener("visibilitychange", visibilityChanged); };
  }, [client, sessionId, serverInstance, shown, request, scope]);
  return useMemo(() => new Map(shown && result?.scope === scope ? result.spaces.filter(space => space.active).map(space => [space.space_id, space]) : []), [result, scope, shown]);
}
