import { useEffect, useRef, useState } from "react";
import type { CockpitClient } from "../../client/CockpitClient";
import type { ContextSnapshotResponse, RepositoryCandidate } from "../../protocol/generated/v1";

export function SnapshotImport({ client, sessionId, paneId, bindingId, rootId, onImported }: {
  client: CockpitClient; sessionId: string; paneId: string; bindingId: string; rootId: string;
  onImported: (result: ContextSnapshotResponse) => void;
}) {
  const [repositories, setRepositories] = useState<RepositoryCandidate[]>([]);
  const [selected, setSelected] = useState("");
  const [pending, setPending] = useState(false);
  const [status, setStatus] = useState<string | null>(null);
  const identity = `${sessionId}\0${paneId}\0${bindingId}\0${rootId}`;
  const identityRef = useRef(identity); identityRef.current = identity;
  useEffect(() => {
    let active = true; setSelected(""); setStatus(null); setPending(false); setRepositories([]);
    void client.repositories().then(value => { if (active) setRepositories(value.repositories); })
      .catch(error => { if (active) setStatus(error instanceof Error ? error.message : "Repository catalog unavailable."); });
    return () => { active = false; };
  }, [client, identity]);
  const copy = async () => {
    if (!selected || pending) return;
    setPending(true); setStatus("Copying current files into this companion…");
    try {
      const result = await client.contextSnapshot(sessionId, paneId, { binding_id: bindingId, root_id: rootId, repository_id: selected, mode: "working_tree" });
      if (identityRef.current !== identity) return;
      setStatus(`${result.files} files · ${result.bytes} bytes · ${result.copy_mode}${result.dirty ? " · includes working changes" : ""}. ${result.diagnostics.map(item => item.message).join(" ")}`);
      onImported(result);
    } catch (error) {
      if (identityRef.current === identity) setStatus(error instanceof Error ? error.message : "Snapshot could not be completed.");
    } finally { if (identityRef.current === identity) setPending(false); }
  };
  return <details className="context-snapshot"><summary>Import local snapshot</summary>
    <p>Copy tracked files and untracked, non-ignored files from a configured repository. Later source edits leave this copy unchanged.</p>
    <label>Repository<select aria-label="Snapshot repository" value={selected} onChange={event => setSelected(event.target.value)} disabled={pending}>
      <option value="">Choose a repository…</option>{repositories.map(repository => <option value={repository.repository_id} key={repository.repository_id}>{repository.name} · {repository.checkout_path}</option>)}
    </select></label>
    <button type="button" disabled={!selected || pending} onClick={() => void copy()}>{pending ? "Copying…" : "Copy snapshot"}</button>
    {status ? <p role="status">{status}</p> : null}
  </details>;
}
