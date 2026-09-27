import { useEffect, useId, useRef, useState } from "react";
import type { CockpitClient } from "../../client/CockpitClient";
import type { LibraryOperation, LibraryResolution, ProjectProvider } from "../../protocol/generated/v1";
import { createPortal } from "react-dom";
import { UiIcon } from "../UiIcon";
import { trapDialogKeys, useRestoreFocus } from "./LibraryConfirmDialog";
import { JIRA_KEY, errorText, jiraProviders, libraryInputUrl, lookupFailure, providerFamily, resolutionNote, type LookupFailure } from "./libraryState";
import { useLibraryOperation } from "./useLibraryOperation";
import "../projects/setup.css";
import "../projects/taskSetup.css";
import "./library.css";

const LOOKUP_DELAY_MS = 400;

type Lookup =
  | { status: "idle" }
  | { status: "pending" }
  | { status: "ok"; resolution: LibraryResolution }
  | { status: "error"; failure: LookupFailure };

/** Phase-1 progress text (design §4.7): Library only, so there is no Space step. */
function libraryPhase(operation: LibraryOperation): { text: string; tone: "running" | "done" | "failed"; saved: boolean } {
  const phase = operation.phases.find((candidate) => candidate.phase === "library");
  const partialReason = operation.report?.rows.find((row) => row.outcome === "partial")?.reason;
  const unchanged = operation.report?.rows.find((row) => row.outcome === "unchanged");
  const count = operation.item_ids.length;
  const summary = count > 1 ? ` · ${count} items` : "";
  switch (phase?.state ?? "pending") {
    case "pending":
    case "running":
      if (operation.cancel_requested) return { text: "Cancelling…", tone: "running", saved: false };
      return { text: `Saving to Library…${phase?.total && phase.total > 1 ? ` ${phase.done} of ${phase.total}` : ""}`, tone: "running", saved: false };
    case "done":
      if (operation.cancel_requested) return { text: "Already saved to Library.", tone: "done", saved: true };
      if (count === 0 && unchanged) return { text: `✓ ${unchanged.reason ?? "Already saved in Library"}`, tone: "done", saved: true };
      return { text: `✓ Saved to Library${summary}`, tone: "done", saved: true };
    case "partial":
      return { text: `◐ Saved to Library, partial${partialReason ? `: ${partialReason}` : ""}${summary}`, tone: "done", saved: true };
    case "failed":
      return { text: `✕ Not saved. ${phase?.error?.message ?? phase?.message ?? "The source could not be saved"}. Nothing was added.`, tone: "failed", saved: false };
    case "cancelled":
      return { text: "Cancelled. Nothing was added.", tone: "failed", saved: false };
  }
}

/**
 * Add context (design §4.5, S1): one field for a forge issue, MR or PR link, or
 * a Jira key. The destination is always the Library; Space copies come later
 * and never happen from here. Closing the dialog does not cancel an add.
 */
export function AddContextDialog({ client, onClose, onOpenItem }: {
  client: CockpitClient;
  onClose: () => void;
  onOpenItem?: (itemId: string) => void;
}) {
  const titleId = useId();
  const fieldId = useId();
  const failureId = useId();
  const inputRef = useRef<HTMLInputElement>(null);
  const closeActionRef = useRef<HTMLButtonElement>(null);
  const primaryActionRef = useRef<HTMLButtonElement>(null);
  const lastBeginRef = useRef<(() => Promise<LibraryOperation>) | null>(null);
  const [providers, setProviders] = useState<ProjectProvider[]>([]);
  const [providersLoaded, setProvidersLoaded] = useState(false);
  const [providersError, setProvidersError] = useState<string | null>(null);
  const [input, setInput] = useState("");
  const [jiraProviderId, setJiraProviderId] = useState<string | null>(null);
  const [linked, setLinked] = useState(false);
  const [refreshExisting, setRefreshExisting] = useState(false);
  const [lookup, setLookup] = useState<Lookup>({ status: "idle" });
  const [lookupRetry, setLookupRetry] = useState(0);
  const add = useLibraryOperation(client);
  useRestoreFocus();
  useEffect(() => { inputRef.current?.focus(); }, []);
  useEffect(() => {
    let current = true;
    client.projectConfiguration().then((configuration) => {
      if (current) { setProviders(configuration.providers); setProvidersLoaded(true); }
    }, (cause: unknown) => {
      if (current) { setProvidersError(errorText(cause, "Provider configuration could not be read.")); setProvidersLoaded(true); }
    });
    return () => { current = false; };
  }, [client]);
  const trimmed = input.trim();
  const jira = jiraProviders(providers);
  const jiraKey = JIRA_KEY.test(trimmed);
  const jiraProvider = jiraKey ? jira.find((provider) => provider.id === jiraProviderId) ?? jira[0] : undefined;
  const requestUrl = libraryInputUrl(trimmed, jiraProvider);
  const requestProviderId = jiraProvider?.id ?? null;
  const lookupKey = `${requestUrl}\u0000${requestProviderId ?? ""}\u0000${lookupRetry}\u0000${jiraKey && !providersLoaded}`;
  useEffect(() => {
    if (!trimmed) { setLookup({ status: "idle" }); return; }
    if (jiraKey && !providersLoaded) { setLookup({ status: "pending" }); return; }
    if (jiraKey && jira.length === 0) {
      setLookup({ status: "error", failure: { title: "✕ No Jira provider configured", detail: "Add a Jira instance to the Cockpit configuration file, or paste the issue's link.", retry: false } });
      return;
    }
    let current = true;
    setLookup({ status: "pending" });
    const timer = window.setTimeout(() => {
      client.libraryResolve({ input: requestUrl, provider_id: requestProviderId }).then((resolution) => {
        if (current) setLookup({ status: "ok", resolution });
      }, (cause: unknown) => {
        if (current) setLookup({ status: "error", failure: lookupFailure(cause, requestUrl, providers) });
      });
    }, LOOKUP_DELAY_MS);
    return () => { current = false; window.clearTimeout(timer); };
    // `providers` only improves failure wording; it never re-requests a lookup.
  }, [client, lookupKey]);
  const resolution = lookup.status === "ok" ? lookup.resolution : null;
  const existing = resolution?.existing_item_id ?? null;
  const family = resolution ? providerFamily(providers, resolution.provider_id) : null;
  const forge = family !== null && family.key !== "jira";
  const operation = add.operation;
  const progress = operation ? libraryPhase(operation) : null;
  const finished = operation?.finished ?? false;
  const canAdd = resolution !== null && !add.starting && (!existing || refreshExisting);
  const primaryLabel = existing ? (refreshExisting ? "Refresh from source" : "Already in Library") : "Add to Library";
  const beginAdd = (): Promise<LibraryOperation> => client.libraryAdd({
    input: requestUrl,
    provider_id: requestProviderId ?? resolution!.provider_id,
    hydrate_references: forge && linked,
    follow_space: false,
    download_attachments: false,
    refresh_existing: Boolean(existing) && refreshExisting,
    label: null,
    target: null,
  });
  const submit = () => {
    if (!canAdd || !resolution) return;
    lastBeginRef.current = beginAdd;
    void add.start(beginAdd);
  };
  const addAnother = () => {
    add.reset();
    setInput("");
    setLookup({ status: "idle" });
    setRefreshExisting(false);
    window.requestAnimationFrame(() => inputRef.current?.focus());
  };
  const openedItemId = operation?.item_ids[0] ?? existing;
  // After success focus moves to `Open in Library`; after a failure, to `Retry`.
  useEffect(() => {
    if (add.error && !operation && !add.starting) primaryActionRef.current?.focus();
    else if (finished) primaryActionRef.current?.focus();
    else if (add.starting || operation) closeActionRef.current?.focus();
  }, [add.error, add.starting, finished, operation]);
  const failure = lookup.status === "error" ? lookup.failure : null;
  // On the body: a Context viewer is a size container and would clip a fixed overlay to its pane.
  return createPortal(<div className="setup-overlay library-dialog-overlay" role="presentation">
    <section className="setup-dialog task-setup library-add" role="dialog" aria-modal="true" aria-labelledby={titleId} onKeyDown={(event) => {
      if (event.key === "Enter" && !operation && event.target instanceof HTMLInputElement && event.target.type !== "checkbox" && !event.nativeEvent.isComposing) {
        event.preventDefault();
        submit();
        return;
      }
      trapDialogKeys(event, onClose);
    }}>
      <header className="task-setup-header"><h2 id={titleId}>Add context</h2><button type="button" className="task-setup-close" onClick={onClose} aria-label="Close Add context"><UiIcon name="close" /></button></header>
      <div className="task-setup-body">
        {operation || add.starting ? <div className="library-progress" aria-live="polite">
          <ol className="library-progress-steps">
            <li className={`library-progress-step is-${progress?.tone ?? "running"}`}>
              <span>{progress?.text ?? "Saving to Library…"}</span>
              {operation && !finished && !operation.cancel_requested ? <button type="button" onClick={add.cancel}>Cancel</button> : null}
            </li>
          </ol>
          {add.error ? <p className="task-setup-note" role="status">{add.error}</p> : null}
        </div> : <>
          {add.error ? <p className="task-setup-note is-error" role="alert">{add.error}</p> : null}
          <div className="task-setup-row">
            <label htmlFor={fieldId}>Source</label>
            <div>
              <input ref={inputRef} id={fieldId} type="text" value={input} onChange={(event) => { setInput(event.target.value); setRefreshExisting(false); }}
                placeholder="Issue, MR or PR link, or Jira key" autoComplete="off" spellCheck={false}
                aria-invalid={failure ? "true" : undefined} aria-describedby={failure ? failureId : undefined} />
              {lookup.status === "pending" ? <p className="task-setup-note">Looking up the link…</p> : null}
              {resolution ? <p className="task-setup-note is-valid">✓ {existing ? `Already in Library · ${resolution.title}` : resolutionNote(resolution, providers)}</p> : null}
              {failure ? <div id={failureId} className="library-refusal" role="alert">
                <strong>{failure.title}</strong>
                <span>{failure.detail}</span>
                {failure.retry ? <button type="button" className="task-setup-link" onClick={() => setLookupRetry((value) => value + 1)}>Retry lookup</button> : null}
              </div> : null}
              {providersError ? <p className="task-setup-note">{providersError}</p> : null}
            </div>
          </div>
          {jiraKey && jira.length > 1 ? <div className="task-setup-row">
            <label htmlFor={`${fieldId}-provider`}>Provider</label>
            <div><select id={`${fieldId}-provider`} className="library-select" value={jiraProvider?.id ?? ""} onChange={(event) => setJiraProviderId(event.target.value)}>
              {jira.map((provider) => <option key={provider.id} value={provider.id}>{provider.id} · {provider.base_url}</option>)}
            </select></div>
          </div> : null}
          {resolution && forge && !existing ? <label className="task-setup-check"><input type="checkbox" checked={linked} onChange={(event) => setLinked(event.target.checked)} /> Include linked issues and {family?.review}s within import limits</label> : null}
          {existing ? <label className="task-setup-check"><input type="checkbox" checked={refreshExisting} onChange={(event) => setRefreshExisting(event.target.checked)} /> Refresh from source first</label> : null}
          <div className="task-setup-row">
            <span className="library-row-label">Destination</span>
            <div><p className="library-destination">Library</p></div>
          </div>
        </>}
      </div>
      <footer className="task-setup-footer">
        {!operation && !add.starting ? <>
          {add.error && lastBeginRef.current ? <button ref={primaryActionRef} type="button" className="setup-primary" onClick={() => { if (lastBeginRef.current) void add.start(lastBeginRef.current); }}>Retry</button> : null}
          {existing && onOpenItem ? <button type="button" onClick={() => { onOpenItem(existing); onClose(); }}>Open in Library</button> : null}
          <button type="button" onClick={onClose}>Cancel</button>
          {!add.error ? <button type="button" className="setup-primary" onClick={submit} disabled={!canAdd}>{primaryLabel}</button> : null}
        </> : !finished ? <button ref={closeActionRef} type="button" onClick={onClose}>Close</button> : progress?.saved ? <>
          <button type="button" onClick={addAnother}>Add another</button>
          <button ref={closeActionRef} type="button" onClick={onClose}>Close</button>
          {onOpenItem && openedItemId ? <button ref={primaryActionRef} type="button" className="setup-primary" onClick={() => { onOpenItem(openedItemId); onClose(); }}>Open in Library</button> : null}
        </> : <>
          <button ref={closeActionRef} type="button" onClick={onClose}>Close</button>
          <button ref={primaryActionRef} type="button" className="setup-primary" onClick={() => { if (lastBeginRef.current) void add.start(lastBeginRef.current); }}>Retry</button>
        </>}
      </footer>
    </section>
  </div>, document.body);
}
