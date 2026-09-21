import { useEffect, useRef, useState } from "react";
import type { CockpitClient } from "../../client/CockpitClient";
import type { ProjectProvider, SourceEntry, SourceImportResponse } from "../../protocol/generated/v1";

const IMPORT_OPERATION = "import";
const MAX_ACTIVE_OPERATIONS = 2;

type Feedback = { tone: "status" | "error"; message: string };

function errorMessage(error: unknown, fallback: string): string {
  if (typeof error === "object" && error !== null && "code" in error && typeof error.code === "string") return `${error.code}: ${error instanceof Error && error.message ? error.message : fallback}`;
  return error instanceof Error && error.message ? error.message : fallback;
}

function entrySummary(entry: SourceEntry): string {
  const freshness = entry.freshness === "fresh" && entry.status === "unchanged" ? "fresh, unchanged" : entry.freshness;
  const materialization = entry.status === "materialized" || entry.status === "unchanged"
    ? "materialized"
    : entry.status === "failed"
      ? "materialization failed"
      : entry.status;
  return `${entry.title}: ${freshness}; ${materialization}`;
}

function mergeEntries(current: SourceEntry[], incoming: SourceEntry[]): SourceEntry[] {
  const updates = new Map(incoming.map(entry => [entry.source_id, entry]));
  const next = current.map(entry => updates.get(entry.source_id) ?? entry);
  const existing = new Set(current.map(entry => entry.source_id));
  for (const entry of incoming) if (!existing.has(entry.source_id)) next.push(entry);
  return next;
}

export function SourceImport({ client, sessionId, paneId, bindingId, rootId, onChanged }: {
  client: CockpitClient; sessionId: string; paneId: string; bindingId: string; rootId: string;
  onChanged: (paths: string[]) => void;
}) {
  const [providers, setProviders] = useState<ProjectProvider[]>([]);
  const [providerId, setProviderId] = useState("");
  const [url, setUrl] = useState("");
  const [entries, setEntries] = useState<SourceEntry[]>([]);
  const [hydrate, setHydrate] = useState(false);
  const [configPending, setConfigPending] = useState(false);
  const [configLoaded, setConfigLoaded] = useState(false);
  const [configError, setConfigError] = useState<string | null>(null);
  const [listPending, setListPending] = useState(false);
  const [listLoaded, setListLoaded] = useState(false);
  const [listError, setListError] = useState<string | null>(null);
  const [activeOperations, setActiveOperations] = useState<string[]>([]);
  const [selectedSourceId, setSelectedSourceId] = useState("");
  const [feedback, setFeedback] = useState<Feedback | null>(null);
  const identity = `${sessionId}\0${paneId}\0${bindingId}\0${rootId}`;
  const identityRef = useRef(identity);
  identityRef.current = identity;
  const selectedSourceRef = useRef("");
  selectedSourceRef.current = selectedSourceId;
  const operationRef = useRef(new Map<string, AbortController>());
  const configAbortRef = useRef<AbortController | null>(null);
  const listAbortRef = useRef<AbortController | null>(null);
  const listGenerationRef = useRef(0);
  const configGenerationRef = useRef(0);
  const configPendingRef = useRef(false);

  const selectedSourceAfterList = (next: SourceEntry[], previous: string): string => {
    if (!previous || next.some(entry => entry.source_id === previous)) return previous;
    return next[0]?.source_id ?? "";
  };

  const selectionChangeMessage = (next: SourceEntry[], previous: string, reason: string): string | null => {
    if (!previous || next.some(entry => entry.source_id === previous)) return null;
    return next[0]
      ? `The previously selected source is no longer available after ${reason}; selected ${next[0].title}.`
      : `The previously selected source is no longer available after ${reason}.`;
  };


  const loadConfiguration = () => {
    if (configPendingRef.current) return;
    const generation = ++configGenerationRef.current;
    configPendingRef.current = true;
    setConfigPending(true);
    setConfigError(null);
    void client.projectConfiguration().then(config => {
      if (generation !== configGenerationRef.current || identityRef.current !== identity) return;
      setProviders(config.providers);
      setProviderId(current => config.providers.some(provider => provider.id === current) ? current : config.providers[0]?.id ?? "");
      setConfigLoaded(true);
    }).catch(error => {
      if (generation !== configGenerationRef.current || identityRef.current !== identity) return;
      setConfigError(errorMessage(error, "Source provider configuration could not be loaded."));
    }).finally(() => {
      if (generation === configGenerationRef.current && identityRef.current === identity) {
        configPendingRef.current = false;
        setConfigPending(false);
      }
    });
  };

  const loadSources = () => {
    const generation = ++listGenerationRef.current;
    listAbortRef.current?.abort();
    const controller = new AbortController();
    listAbortRef.current = controller;
    setListPending(true);
    setListError(null);
    void client.sourceList(sessionId, paneId, { binding_id: bindingId, root_id: rootId }, controller.signal).then(result => {
      if (controller.signal.aborted || generation !== listGenerationRef.current || identityRef.current !== identity) return;
      const previous = selectedSourceRef.current;
      const selectionMessage = selectionChangeMessage(result.entries, previous, "source list refresh");
      setEntries(result.entries);
      setSelectedSourceId(selectedSourceAfterList(result.entries, previous));
      setListLoaded(true);
      const diagnostics = result.diagnostics.map(item => `${item.code}: ${item.message}`);
      const messages = [selectionMessage, ...diagnostics].filter((message): message is string => Boolean(message));
      if (messages.length) setFeedback({ tone: "status", message: messages.join(" ") });
    }).catch(error => {
      if (controller.signal.aborted || generation !== listGenerationRef.current || identityRef.current !== identity) return;
      setListError(errorMessage(error, "Sources could not be loaded."));
    }).finally(() => {
      if (generation === listGenerationRef.current && identityRef.current === identity) setListPending(false);
    });
  };

  useEffect(() => {
    const configController = new AbortController();
    const listController = new AbortController();
    identityRef.current = identity;
    setProviders([]);
    setProviderId("");
    setUrl("");
    setEntries([]);
    setHydrate(false);
    setConfigPending(false);
    setConfigLoaded(false);
    setConfigError(null);
    setListPending(false);
    setListLoaded(false);
    setListError(null);
    setActiveOperations([]);
    setSelectedSourceId("");
    selectedSourceRef.current = "";
    setFeedback(null);
    operationRef.current.forEach(active => active.abort());
    operationRef.current.clear();
    configAbortRef.current?.abort();
    listAbortRef.current?.abort();
    configAbortRef.current = configController;
    listAbortRef.current = listController;
    configPendingRef.current = false;
    const configGeneration = ++configGenerationRef.current;
    const listGeneration = ++listGenerationRef.current;
    configPendingRef.current = true;
    setConfigPending(true);
    void client.projectConfiguration().then(config => {
      if (configController.signal.aborted || configGeneration !== configGenerationRef.current || identityRef.current !== identity) return;
      setProviders(config.providers);
      setProviderId(config.providers[0]?.id ?? "");
      setConfigLoaded(true);
    }).catch(error => {
      if (!configController.signal.aborted && configGeneration === configGenerationRef.current && identityRef.current === identity) setConfigError(errorMessage(error, "Source provider configuration could not be loaded."));
    }).finally(() => {
      if (!configController.signal.aborted && configGeneration === configGenerationRef.current && identityRef.current === identity) {
        configPendingRef.current = false;
        setConfigPending(false);
      }
    });
    setListPending(true);
    void client.sourceList(sessionId, paneId, { binding_id: bindingId, root_id: rootId }, listController.signal).then(result => {
      if (listController.signal.aborted || listGeneration !== listGenerationRef.current || identityRef.current !== identity) return;
      const previous = selectedSourceRef.current;
      const selectionMessage = selectionChangeMessage(result.entries, previous, "source list refresh");
      setEntries(result.entries);
      setSelectedSourceId(selectedSourceAfterList(result.entries, previous));
      setListLoaded(true);
      const diagnostics = result.diagnostics.map(item => `${item.code}: ${item.message}`);
      const messages = [selectionMessage, ...diagnostics].filter((message): message is string => Boolean(message));
      if (messages.length) setFeedback({ tone: "status", message: messages.join(" ") });
    }).catch(error => {
      if (!listController.signal.aborted && listGeneration === listGenerationRef.current && identityRef.current === identity) setListError(errorMessage(error, "Sources could not be loaded."));
    }).finally(() => {
      if (!listController.signal.aborted && listGeneration === listGenerationRef.current && identityRef.current === identity) setListPending(false);
    });
    return () => {
      configController.abort();
      listController.abort();
      operationRef.current.forEach(active => active.abort());
      operationRef.current.clear();
    };
  }, [client, identity]);

  const perform = async (sourceId?: string) => {
    const operation = sourceId ?? IMPORT_OPERATION;
    if (operationRef.current.has(operation)) return;
    if (operationRef.current.size >= MAX_ACTIVE_OPERATIONS) {
      setFeedback({ tone: "error", message: "source_import_busy: source refresh capacity is full. Wait for an active operation to finish, then retry." });
      return;
    }
    const controller = new AbortController();
    operationRef.current.set(operation, controller);
    setActiveOperations([...operationRef.current.keys()]);
    setFeedback(null);
    if (sourceId) setSelectedSourceId(sourceId);
    try {
      const request = { binding_id: bindingId, root_id: rootId, hydrate_references: hydrate };
      const result: SourceImportResponse = sourceId
        ? await client.sourceRefresh(sessionId, paneId, { ...request, source_id: sourceId }, controller.signal)
        : await client.sourceImport(sessionId, paneId, { ...request, provider_id: providerId, artifact_url: url.trim() }, controller.signal);
      if (controller.signal.aborted || identityRef.current !== identity || operationRef.current.get(operation) !== controller) return;
      const nextSelection = selectedSourceRef.current || sourceId || result.entries[0]?.source_id || "";
      setSelectedSourceId(nextSelection);
      setEntries(current => mergeEntries(current, result.entries));
      const messages = [...result.entries.map(entrySummary), ...result.diagnostics.map(item => `${item.code}: ${item.message}`)];
      setFeedback({ tone: "status", message: messages.length ? messages.join(". ") : "Source operation completed." });
      onChanged(result.entries.flatMap(entry => entry.relative_path ? [entry.relative_path] : []));
    } catch (error) {
      if (!controller.signal.aborted && identityRef.current === identity) {
        setFeedback({ tone: "error", message: errorMessage(error, sourceId ? "Source refresh failed." : "Source import failed.") });
      }
    } finally {
      if (operationRef.current.get(operation) === controller) {
        operationRef.current.delete(operation);
        if (identityRef.current === identity) setActiveOperations([...operationRef.current.keys()]);
      }
    }
  };

  const importPending = activeOperations.includes(IMPORT_OPERATION);
  const isRefreshPending = (sourceId: string) => activeOperations.includes(sourceId);
  return <details className="context-snapshot context-sources" open>
    <summary>Sources</summary>
    <p>Import a source from the task repository’s configured provider. Generated files keep their provenance; refresh preserves local edits.</p>
    {configPending && <p className="context-resource-loading" role="status">Loading source provider configuration…</p>}
    {configError ? <div className="context-resource-error" role="alert"><strong>Provider configuration unavailable.</strong><span>{configError}</span><button type="button" disabled={configPending} onClick={loadConfiguration}>Retry configuration</button></div> : null}
    {!configPending && !configError && configLoaded && providers.length === 0 ? <p className="context-resource-empty">No source provider is configured. Configure a provider before importing a source.</p> : null}
    {providers.length ? <>
      <label>Provider<select aria-label="Source provider" value={providerId} disabled={importPending} onChange={event => setProviderId(event.target.value)}>{providers.map(provider => <option key={provider.id} value={provider.id}>{provider.id} · {provider.base_url}</option>)}</select></label>
      <label>Source URL<input aria-label="Source URL" type="url" value={url} disabled={importPending} onChange={event => setUrl(event.target.value)} /></label>
      <button type="button" disabled={importPending || !providerId || !url.trim()} onClick={() => void perform()}>{importPending ? "Importing source…" : "Import source"}</button>
    </> : null}
    <label><input type="checkbox" checked={hydrate} disabled={importPending} onChange={event => setHydrate(event.target.checked)} /> Include linked sources from this repository within import limits</label>
    <div className="context-resource-list" role="list" aria-label="Imported sources">
      {listPending && !listLoaded ? <p className="context-resource-loading" role="status">Loading imported sources…</p> : null}
      {listError ? <div className="context-resource-error" role="alert"><strong>Source list unavailable.</strong><span>{listError}</span><button type="button" disabled={listPending} onClick={loadSources}>Retry source list</button></div> : null}
      {!listPending && !listError && listLoaded && entries.length === 0 ? <p className="context-resource-empty">No imported sources yet. Use Import source above to add one.</p> : null}
      {entries.map(entry => <article key={entry.source_id} role="listitem" className={`context-source-entry${entry.source_id === selectedSourceId ? " is-selected" : ""}`} aria-current={entry.source_id === selectedSourceId ? "true" : undefined}>
        <strong title={entry.title}>{entry.title}</strong>
        <span>{entry.provider_id} · {entry.provider_instance} · {entry.resource_type} · {entrySummary(entry)}</span>
        <span className="context-source-identity"><code title={entry.canonical_id}>{entry.canonical_id}</code>{entry.source_revision ? <code title={`Provider revision ${entry.source_revision}`}>rev {entry.source_revision}</code> : null}</span>
        {entry.relative_path ? <code title={entry.relative_path}>{entry.relative_path}</code> : <span className="context-source-unavailable">Unavailable locally; no materialized file</span>}
        {entry.status === "failed" ? <p className="context-source-diagnostic" role="alert">Materialization failed; the provider result is retained. Retry refresh.</p> : null}
        {entry.status === "unsupported" || entry.freshness === "unavailable" ? <p className="context-source-diagnostic" role="status">Provider/resource capability is unavailable for this source.</p> : null}
        {entry.status === "conflict" || entry.freshness === "conflict" ? <p className="context-source-diagnostic" role="status">Local edits preserved; refresh needs reconciliation.</p> : null}
        <button type="button" disabled={isRefreshPending(entry.source_id)} aria-label={`${isRefreshPending(entry.source_id) ? "Refreshing" : "Refresh"} source ${entry.title}`} onClick={() => void perform(entry.source_id)}>{isRefreshPending(entry.source_id) ? "Refreshing…" : "Refresh source"}</button>
      </article>)}
    </div>
    {feedback ? <p className={feedback.tone === "error" ? "context-resource-feedback context-resource-feedback-error" : "context-resource-feedback"} role={feedback.tone === "error" ? "alert" : "status"} aria-live="polite">{feedback.message}</p> : null}
  </details>;
}
