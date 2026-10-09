import { useEffect, useRef, useState } from "react";
import type { ClosedTaskCount } from "./SupervisorActivity";
import type { SupervisorViewInputs, SupervisorViewModel } from "./supervisorViewTypes";

export function useArchiveCounts(context: Pick<SupervisorViewInputs, "client" | "sessionId" | "rootId" | "active" | "scope" | "snapshot"> & Pick<SupervisorViewModel, "closedRoots">) {
  const { client, sessionId, rootId, active, scope, snapshot, closedRoots } = context;
  const [archiveCounts, setArchiveCounts] = useState<ReadonlyMap<string, ClosedTaskCount>>(new Map());
  const archiveGeneration = useRef(0);
  // Fetch only on archive-open. Four concurrent requests bound provider load; identity/generation fence every completion.
  const closedKey = closedRoots.map(root => root.root_id).sort().join("\u0000");
  useEffect(() => {
    const generation = ++archiveGeneration.current;
    if (!scope.disclosures.archive || !active || !snapshot) return;
    const ids = closedRoots.map(root => root.root_id);
    setArchiveCounts(new Map(ids.map(id => [id, { status: "loading" } as ClosedTaskCount])));
    let cursor = 0, cancelled = false;
    const load = async () => {
      while (!cancelled && cursor < ids.length) {
        const id = ids[cursor++]; let count: ClosedTaskCount;
        try { const result = await client.orchestrationSnapshot({ session_id: sessionId, root_id: id }); count = result.session_id === sessionId && result.board?.root_id === id ? { status: "loaded", count: result.board.tasks.length } : { status: "unavailable" }; }
        catch { count = { status: "unavailable" }; }
        if (cancelled || archiveGeneration.current !== generation) return;
        setArchiveCounts(previous => { const next = new Map(previous); next.set(id, count); return next; });
      }
    };
    for (let index = 0; index < Math.min(4, ids.length); index++) void load();
    return () => { cancelled = true; archiveGeneration.current++; };
  }, [client, sessionId, rootId, active, scope.disclosures.archive, closedKey]);
  return archiveCounts;
}
