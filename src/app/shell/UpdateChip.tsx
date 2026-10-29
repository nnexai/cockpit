import { useEffect, useState } from "react";
import type { CockpitClient, UpdateStatus } from "../../client/CockpitClient";

export function UpdateChip({ client, onUpdate }: { client: CockpitClient; onUpdate(sourcePath: string): void }) {
  const [status, setStatus] = useState<UpdateStatus | null>(null);
  useEffect(() => {
    let stopped = false;
    let timer: number;
    let pending = false;
    let refresh = false;
    const poll = async () => {
      if (pending) { refresh = true; return; }
      pending = true;
      try {
        const next = await client.updateStatus();
        if (!stopped) setStatus(next);
      } catch {
        if (!stopped) setStatus(null);
      } finally {
        pending = false;
        if (!stopped && refresh) { refresh = false; void poll(); }
        else if (!stopped) timer = window.setTimeout(poll, 600_000);
      }
    };
    void poll();
    const onFocus = () => { clearTimeout(timer); void poll(); };
    window.addEventListener("focus", onFocus);
    return () => { stopped = true; clearTimeout(timer); window.removeEventListener("focus", onFocus); };
  }, [client]);

  if (!status?.commits_behind_main || !status.source_path) return null;
  const noun = status.commits_behind_main === 1 ? "commit" : "commits";
  return <button type="button" className="tab-strip-action update-chip" title={`${status.commits_behind_main} ${noun} on main since ${status.installed_commit?.slice(0, 8)} · ${status.source_path}`} onClick={() => onUpdate(status.source_path!)}>↑ {status.commits_behind_main} {noun} · install</button>;
}
