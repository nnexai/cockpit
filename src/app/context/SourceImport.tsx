import { useEffect, useRef, useState } from "react";
import type { CockpitClient } from "../../client/CockpitClient";
import type { ProjectProvider, SourceEntry, SourceImportResponse } from "../../protocol/generated/v1";

export function SourceImport({ client, sessionId, paneId, bindingId, rootId, onChanged }: {
  client: CockpitClient; sessionId: string; paneId: string; bindingId: string; rootId: string;
  onChanged: (paths: string[]) => void;
}) {
  const [providers, setProviders] = useState<ProjectProvider[]>([]);
  const [providerId, setProviderId] = useState("");
  const [url, setUrl] = useState("");
  const [entries, setEntries] = useState<SourceEntry[]>([]);
  const [hydrate, setHydrate] = useState(false);
  const [pending, setPending] = useState(false);
  const [status, setStatus] = useState<string | null>(null);
  const identity = `${sessionId}\0${paneId}\0${bindingId}\0${rootId}`;
  const identityRef = useRef(identity); identityRef.current = identity;
  const abortRef = useRef<AbortController | null>(null);
  const busyRef = useRef(false);
  useEffect(() => {
    const controller = new AbortController(); abortRef.current = controller;
    setHydrate(false); setEntries([]); setProviders([]); setUrl(""); setProviderId(""); setStatus(null); setPending(false); busyRef.current = false;
    void Promise.all([client.projectConfiguration(), client.sourceList(sessionId, paneId, { binding_id: bindingId, root_id: rootId }, controller.signal)]).then(([config, result]) => {
      if (!controller.signal.aborted) { setProviders(config.providers); setProviderId(config.providers[0]?.id ?? ""); setEntries(result.entries); }
    }).catch(error => { if (!controller.signal.aborted) setStatus(error instanceof Error ? error.message : "Sources could not be loaded."); });
    return () => controller.abort();
  }, [client, identity]);
  const perform = async (sourceId?: string) => {
    if (busyRef.current) return;
    busyRef.current = true; setPending(true); setStatus(null);
    abortRef.current?.abort(); const controller = new AbortController(); abortRef.current = controller;
    try {
      const request = { binding_id: bindingId, root_id: rootId, hydrate_references: hydrate };
      const result: SourceImportResponse = sourceId
        ? await client.sourceRefresh(sessionId, paneId, { ...request, source_id: sourceId }, controller.signal)
        : await client.sourceImport(sessionId, paneId, { ...request, provider_id: providerId, artifact_url: url.trim() }, controller.signal);
      if (controller.signal.aborted || identityRef.current !== identity) return;
      setEntries(current => [...current.filter(entry => !result.entries.some(next => next.source_id === entry.source_id)), ...result.entries.map(next => ({ ...next, relative_path: next.relative_path ?? current.find(entry => entry.source_id === next.source_id)?.relative_path ?? null }))]);
      setStatus([...result.entries.map(entry => `${entry.title}: ${entry.status === "conflict" ? "local edits preserved; refresh needs reconciliation" : entry.status}`), ...result.diagnostics.map(item => item.message)].join(". "));
      onChanged(result.entries.flatMap(entry => entry.relative_path ? [entry.relative_path] : []));
    } catch (error) {
      if (!controller.signal.aborted && identityRef.current === identity) setStatus(error instanceof Error ? error.message : "Source could not be imported.");
    } finally { if (identityRef.current === identity) { setPending(false); busyRef.current = false; } }
  };
  return <details className="context-snapshot context-sources"><summary>Sources</summary>
    <p>Import a source from the task repository’s configured provider. Generated files keep their provenance; refresh preserves local edits.</p>
    {providers.length ? <>
      <label>Provider<select aria-label="Source provider" value={providerId} disabled={pending} onChange={event => setProviderId(event.target.value)}>{providers.map(provider => <option key={provider.id} value={provider.id}>{provider.id} · {provider.base_url}</option>)}</select></label>
      <label>Source URL<input aria-label="Source URL" type="url" value={url} disabled={pending} onChange={event => setUrl(event.target.value)} /></label>
      <button type="button" disabled={pending || !providerId || !url.trim()} onClick={() => void perform()}>{pending ? "Loading source…" : "Import source"}</button>
    </> : <p>No source provider is configured.</p>}
    <label><input type="checkbox" checked={hydrate} disabled={pending} onChange={event => setHydrate(event.target.checked)} /> Include linked sources from this repository within import limits</label>
    {entries.map(entry => <div key={entry.source_id} className="context-source-entry"><strong>{entry.title}</strong><span>{entry.resource_type} · {entry.freshness}</span><code title={entry.relative_path ?? undefined}>{entry.relative_path}</code><button type="button" disabled={pending} aria-label={`Refresh source ${entry.title}`} onClick={() => void perform(entry.source_id)}>Refresh source</button></div>)}
    {status ? <p role="status">{status}</p> : null}
  </details>;
}
