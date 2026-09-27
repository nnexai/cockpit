import { useEffect, useId, useRef, useState } from "react";
import type { CockpitClient } from "../../client/CockpitClient";
import type { LibraryOperation, LibraryResolution, ProjectProvider } from "../../protocol/generated/v1";
import { createPortal } from "react-dom";
import { UiIcon } from "../UiIcon";
import { trapDialogKeys, useRestoreFocus } from "./LibraryConfirmDialog";
import { JIRA_KEY, errorText, jiraProviders, libraryInputUrl, lookupFailure, providerFamily, resolutionNote, type LibrarySpace, type LookupFailure } from "./libraryState";
import { headerSpaceAction } from "./spaceCopyPresentation";
import { useLibraryOperation, useSpaceContextListing } from "./useLibraryOperation";
import "../projects/setup.css";
import "../projects/taskSetup.css";
import "./library.css";

const LOOKUP_DELAY_MS = 400;

type Lookup =
  | { status: "idle" }
  | { status: "pending" }
  | { status: "ok"; resolution: LibraryResolution }
  | { status: "error"; failure: LookupFailure };

type Tone = "running" | "done" | "failed";

/**
 * Phase 1 (design §4.7): the Library step. `saved` follows the items the
 * operation actually saved, so a cancel or failure after some were published
 * still offers them; `complete` is false when part of the source wasn't added.
 * A Space-only retry starts from saved items.
 */
function libraryPhase(operation: LibraryOperation): { text: string; tone: Tone; saved: boolean; complete: boolean } {
  if (operation.kind === "space_add") return { text: "✓ Saved to Library", tone: "done", saved: true, complete: true };
  const phase = operation.phases.find((candidate) => candidate.phase === "library");
  const partialReason = operation.report?.rows.find((row) => row.outcome === "partial")?.reason;
  const unchanged = operation.report?.rows.find((row) => row.outcome === "unchanged");
  const count = operation.item_ids.length;
  const summary = count > 1 ? ` · ${count} items` : "";
  switch (phase?.state ?? "pending") {
    case "pending":
    case "running":
      if (operation.cancel_requested) return { text: "Cancelling…", tone: "running", saved: false, complete: false };
      return { text: `Saving to Library…${phase?.total && phase.total > 1 ? ` ${phase.done} of ${phase.total}` : ""}`, tone: "running", saved: false, complete: false };
    case "done":
      if (operation.cancel_requested) return { text: "Already saved to Library.", tone: "done", saved: true, complete: true };
      if (count === 0 && unchanged) return { text: `✓ ${unchanged.reason ?? "Already saved in Library"}`, tone: "done", saved: true, complete: true };
      return { text: `✓ Saved to Library${summary}`, tone: "done", saved: true, complete: true };
    case "partial":
      return { text: `◐ Saved to Library, partial${partialReason ? `: ${partialReason}` : ""}${summary}`, tone: "done", saved: true, complete: true };
    case "failed": {
      const reason = (phase?.error?.message ?? phase?.message ?? "The source could not be saved").replace(/\.$/, "");
      if (count > 0) return { text: `◐ Saved to Library${summary}, then stopped: ${reason}. Anything not yet saved wasn't added.`, tone: "failed", saved: true, complete: false };
      return { text: `✕ Not saved. ${reason}. Nothing was added.`, tone: "failed", saved: false, complete: false };
    }
    case "cancelled":
      if (count > 0) return { text: `◐ Saved to Library${summary}, then cancelled. Anything not yet saved wasn't added.`, tone: "done", saved: true, complete: false };
      return { text: "Cancelled. Nothing was added.", tone: "failed", saved: false, complete: false };
  }
}

/**
 * Phase 2 (design §4.7): the copy into the Space, shown once the Library copy is
 * saved. A failure never implies the Library copy was lost, and the files the
 * operation recorded as written tell a partial copy from one that wrote nothing.
 */
function spacePhase(operation: LibraryOperation, space: string): { text: string; tone: Tone } | null {
  const phase = operation.phases.find((candidate) => candidate.phase === "space");
  if (!phase) return null;
  const written = operation.space?.written.length ?? 0;
  const files = `${written} ${written === 1 ? "file was" : "files were"} copied`;
  switch (phase.state) {
    case "pending":
      return operation.finished ? { text: `Not added to ${space}. The Library copy is saved.`, tone: "failed" } : null;
    case "running":
      return { text: `Adding to ${space}…${phase.total && phase.total > 1 ? ` ${phase.done} of ${phase.total}` : ""}`, tone: "running" };
    case "done":
    case "partial": {
      const result = operation.space;
      if (!result || written === 0) return { text: `✓ Already in ${space}. Nothing was written.`, tone: "done" };
      const how = result.copy_mode === "reflink" ? "reflinked" : result.copy_mode === "mixed" ? "reflinked and copied" : "copied (reflink not supported here)";
      return { text: `✓ Added to ${space} · ${how}`, tone: "done" };
    }
    case "failed": {
      const unverified = phase.error?.code === "source_companion_unavailable";
      const reason = unverified ? "Its context folder couldn't be verified" : (phase.error?.message ?? "The copy stopped").replace(/\.$/, "");
      if (written > 0) return { text: `✕ Only partly added to ${space}: ${files} before it stopped. ${reason}. The Library copy is saved; retrying copies the rest.`, tone: "failed" };
      return unverified
        ? { text: `✕ Not added to ${space}. ${reason}. The Library copy is saved; nothing was written to ${space}.`, tone: "failed" }
        : { text: `✕ Not added to ${space}. ${reason}. The Library copy is saved.`, tone: "failed" };
    }
    case "cancelled":
      return { text: written > 0 ? `Only partly added to ${space}: ${files}. The Library copy is saved.` : `Not added to ${space}. The Library copy is saved.`, tone: "failed" };
  }
}

/**
 * The dialog's accepted request outlives the dialog (design §4.7: closing does
 * not cancel). Reopening shows a request still starting, a running add, an
 * outcome that finished while closed, or a failure with its retry, until
 * `Add another` or a new add.
 */
type AcceptedAdd = {
  begin: () => Promise<LibraryOperation>;
  /**
   * The saved items a Space-only copy asked for (no Library step), or null for
   * an add. A retry resends these: an interrupted operation may record fewer.
   */
  spaceItemIds: string[] | null;
  space: LibrarySpace | null;
  /** Settles after `operation` or `error` is recorded. */
  pending: Promise<LibraryOperation>;
  operation: LibraryOperation | null;
  error: string | null;
  seen: boolean;
};
let accepted: AcceptedAdd | null = null;

/**
 * Add context (design §4.5): one field for a forge issue, MR or PR link, a
 * Jira key, or a local folder path. It always saves to the Library first;
 * with a live target Space the destination can also copy the saved item there.
 */
export function AddContextDialog({ client, onClose, onOpenItem, space = null, defaultDestination = "library", openInSpace = null }: {
  client: CockpitClient;
  onClose: () => void;
  onOpenItem?: (itemId: string) => void;
  /** The Space an add can also copy into; absent with no session or Space. */
  space?: LibrarySpace | null;
  /** From the entry point (design §1): `Resources → Add…` presets the Space. */
  defaultDestination?: "library" | "space";
  /** The opening Context pane's companion root; `Open in <Space>` selects a written file there. */
  openInSpace?: { companionRootId: string; open: (path: string) => void } | null;
}) {
  const titleId = useId();
  const fieldId = useId();
  const failureId = useId();
  const destinationId = useId();
  const inputRef = useRef<HTMLInputElement>(null);
  const closeActionRef = useRef<HTMLButtonElement>(null);
  const primaryActionRef = useRef<HTMLButtonElement>(null);
  const restoredRef = useRef(accepted);
  const lastBeginRef = useRef<(() => Promise<LibraryOperation>) | null>(restoredRef.current?.begin ?? null);
  const [operationSpace, setOperationSpace] = useState<LibrarySpace | null>(restoredRef.current?.space ?? null);
  const [spaceItemIds, setSpaceItemIds] = useState<string[] | null>(restoredRef.current?.spaceItemIds ?? null);
  const [restoredError, setRestoredError] = useState<string | null>(restoredRef.current && !restoredRef.current.operation ? restoredRef.current.error : null);
  // Closed before the request was accepted: follow it until it settles.
  const [awaitingStart, setAwaitingStart] = useState(Boolean(restoredRef.current && !restoredRef.current.operation && !restoredRef.current.error));
  const [providers, setProviders] = useState<ProjectProvider[]>([]);
  const [providersLoaded, setProvidersLoaded] = useState(false);
  const [providersError, setProvidersError] = useState<string | null>(null);
  const [input, setInput] = useState("");
  const [label, setLabel] = useState<string | null>(null);
  const [jiraProviderId, setJiraProviderId] = useState<string | null>(null);
  const [linked, setLinked] = useState(false);
  const [refreshExisting, setRefreshExisting] = useState(false);
  const [lookup, setLookup] = useState<Lookup>({ status: "idle" });
  const [lookupRetry, setLookupRetry] = useState(0);
  const spaceChoice = space?.live ? space : null;
  const [destination, setDestination] = useState<"library" | "space">(spaceChoice && defaultDestination === "space" ? "space" : "library");
  const spaceListing = useSpaceContextListing(client, spaceChoice?.target ?? null, spaceChoice !== null);
  const add = useLibraryOperation(client);
  const resume = add.resume;
  useRestoreFocus();
  useEffect(() => {
    const restored = restoredRef.current;
    if (restored?.operation) { resume(restored.operation); return; }
    if (!restored || restored.error) { if (!restored) inputRef.current?.focus(); return; }
    let current = true;
    // A newer request replaces this one; its late result must not overwrite the dialog.
    restored.pending.then((next) => {
      if (!current || accepted !== restored) return;
      setAwaitingStart(false);
      resume(next);
    }, () => {
      if (!current || accepted !== restored) return;
      setAwaitingStart(false);
      setRestoredError(restored.error);
    });
    return () => { current = false; };
  }, [resume]);
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
  const folder = resolution?.kind === "folder";
  const family = resolution && !folder ? providerFamily(providers, resolution.provider_id) : null;
  const forge = family !== null && family.key !== "jira";
  const destinationSpace = destination === "space" ? spaceChoice : null;
  const companion = spaceListing.listing?.companion ?? null;
  // An item already in the target Space: the header's Space state decides whether adding applies.
  const existingInSpace = existing && destinationSpace && spaceListing.listing ? headerSpaceAction(spaceListing.listing.rows.find((row) => row.item_id === existing), destinationSpace.label) : null;
  const spaceAddable = !existingInSpace || existingInSpace.actions.some((action) => action.kind === "add");
  const operation = add.operation;
  const shownSpace = operationSpace?.label ?? "the Space";
  const progress = operation ? libraryPhase(operation) : null;
  const spaceStep = operation && progress?.saved ? spacePhase(operation, shownSpace) : null;
  const finished = operation?.finished ?? false;
  const startError = add.error ?? restoredError;
  const canAdd = resolution !== null && !add.starting && (!existing || refreshExisting || (destinationSpace !== null && spaceAddable));
  const primaryLabel = existing
    ? refreshExisting ? (destinationSpace ? `Refresh and add to ${destinationSpace.label}` : "Refresh from source") : destinationSpace ? `Add to ${destinationSpace.label}` : "Already in Library"
    : destinationSpace ? `Add to Library and ${destinationSpace.label}` : "Add to Library";
  const operationRef = useRef(operation);
  operationRef.current = operation;
  // A displayed, complete success is done with; anything else stays for the next opening.
  useEffect(() => () => {
    const latest = operationRef.current;
    if (!accepted?.seen || !latest?.finished || accepted.operation?.operation_id !== latest.operation_id) return;
    const phase = libraryPhase(latest);
    if (phase.saved && phase.complete && spacePhase(latest, "")?.tone !== "failed") accepted = null;
  }, []);
  useEffect(() => {
    if (finished && operation && accepted?.operation?.operation_id === operation.operation_id) accepted.seen = true;
  }, [finished, operation]);
  const begin = (request: () => Promise<LibraryOperation>, requestSpace: LibrarySpace | null, requestItemIds: string[] | null) => {
    const pending = request();
    const entry: AcceptedAdd = { begin: request, spaceItemIds: requestItemIds, space: requestSpace, operation: null, error: null, seen: false, pending };
    // Record the result on this request only; a newer request owns `accepted`.
    entry.pending = pending.then((next) => { entry.operation = next; return next; }, (cause: unknown) => {
      entry.error = errorText(cause, "The Library operation could not start.");
      throw cause;
    });
    accepted = entry;
    lastBeginRef.current = request;
    setOperationSpace(requestSpace);
    setSpaceItemIds(requestItemIds);
    setRestoredError(null);
    setAwaitingStart(false);
    void add.start(() => entry.pending);
  };
  const submit = () => {
    if (!canAdd || !resolution) return;
    const target = destinationSpace?.target ?? null;
    if (existing && target && !refreshExisting) {
      // Already saved: copy the Library item without asking the provider again.
      const itemIds = [existing];
      begin(() => client.librarySpaceAdd({ target, item_ids: itemIds, follow_ids: [] }), destinationSpace, itemIds);
      return;
    }
    begin(() => client.libraryAdd({
      input: requestUrl,
      provider_id: requestProviderId ?? resolution.provider_id,
      hydrate_references: forge && linked,
      follow_space: false,
      download_attachments: false,
      refresh_existing: Boolean(existing) && refreshExisting,
      label: folder ? label?.trim() || resolution.title : null,
      target,
    }), destinationSpace, null);
  };
  // The same source again, whether the request never started or stopped part way.
  const retryAll = () => { if (lastBeginRef.current) begin(lastBeginRef.current, operationSpace, spaceItemIds); };
  // Copies the saved items again; items already in the Space are left unchanged. A
  // Space-only request resends what it asked for, since an interrupted operation
  // may have recorded only some of them; an add knows its items only from the operation.
  const retrySpace = () => {
    const target = operation?.target ?? operationSpace?.target;
    if (!target || !operation) return;
    const itemIds = [...new Set([...(spaceItemIds ?? []), ...operation.item_ids])];
    begin(() => client.librarySpaceAdd({ target, item_ids: itemIds, follow_ids: [] }), operationSpace, itemIds);
  };
  const addAnother = () => {
    accepted = null;
    add.reset();
    setRestoredError(null);
    setAwaitingStart(false);
    setInput("");
    setLabel(null);
    setLookup({ status: "idle" });
    setRefreshExisting(false);
    window.requestAnimationFrame(() => inputRef.current?.focus());
  };
  const starting = add.starting || awaitingStart;
  const openedItemId = operation?.item_ids[0] ?? spaceItemIds?.[0] ?? existing;
  const spaceFailed = spaceStep?.tone === "failed";
  const written = operation?.space?.written ?? [];
  // A partial copy still opens what it wrote.
  const openablePath = openInSpace && written.length > 0 && operation?.space?.companion_root_id === openInSpace.companionRootId ? written[0] : undefined;
  const libraryIncomplete = finished && progress !== null && !progress.complete && lastBeginRef.current !== null;
  // After success focus moves to `Open in …`; after a failure, to its retry.
  useEffect(() => {
    if (startError && !operation && !starting) primaryActionRef.current?.focus();
    else if (finished) (primaryActionRef.current ?? closeActionRef.current)?.focus();
    else if (starting || operation) closeActionRef.current?.focus();
  }, [startError, starting, finished, operation]);
  const failure = lookup.status === "error" ? lookup.failure : null;
  // On the body: a Context viewer is a size container and would clip a fixed overlay to its pane.
  return createPortal(<div className="setup-overlay library-dialog-overlay" role="presentation">
    <section className="setup-dialog task-setup library-add" role="dialog" aria-modal="true" aria-labelledby={titleId} onKeyDown={(event) => {
      if (event.key === "Enter" && !operation && event.target instanceof HTMLInputElement && event.target.type === "text" && !event.nativeEvent.isComposing) {
        event.preventDefault();
        submit();
        return;
      }
      trapDialogKeys(event, onClose);
    }}>
      <header className="task-setup-header"><h2 id={titleId}>Add context</h2><button type="button" className="task-setup-close" onClick={onClose} aria-label="Close Add context"><UiIcon name="close" /></button></header>
      <div className="task-setup-body">
        {operation || starting ? <div className="library-progress" aria-live="polite">
          <ol className="library-progress-steps">
            <li className={`library-progress-step is-${progress?.tone ?? "running"}`}>
              <span>{progress?.text ?? (spaceItemIds ? `Adding to ${shownSpace}…` : "Saving to Library…")}</span>
              {operation && !finished && !operation.cancel_requested && operation.kind !== "space_add" && !progress?.saved ? <button type="button" onClick={add.cancel}>Cancel</button> : null}
            </li>
            {spaceStep ? <li className={`library-progress-step is-${spaceStep.tone}`}>
              <span key={spaceStep.tone} role={spaceFailed ? "alert" : undefined}>{spaceStep.text}</span>
              {spaceFailed && written.length > 0 ? <ul className="library-progress-files" aria-label={`Copied to ${shownSpace}`}>
                {written.map((path) => <li key={path}><code>{path}</code></li>)}
              </ul> : null}
              {spaceFailed && finished ? <span className="library-progress-actions">
                <button ref={primaryActionRef} type="button" className="setup-primary" onClick={retrySpace}>{`Retry adding to ${shownSpace}`}</button>
                {openablePath && openInSpace ? <button type="button" onClick={() => { openInSpace.open(openablePath); onClose(); }}>{`Open in ${shownSpace}`}</button> : null}
                {onOpenItem && openedItemId ? <button type="button" onClick={() => { onOpenItem(openedItemId); onClose(); }}>Open in Library</button> : null}
              </span> : null}
            </li> : null}
          </ol>
          {add.error ? <p className="task-setup-note" role="status">{add.error}</p> : null}
        </div> : <>
          {startError ? <p className="task-setup-note is-error" role="alert">{startError}</p> : null}
          <div className="task-setup-row">
            <label htmlFor={fieldId}>Source</label>
            <div>
              <input ref={inputRef} id={fieldId} type="text" value={input} onChange={(event) => { setInput(event.target.value); if (event.target.value.trim() !== trimmed) { setLabel(null); setLookup({ status: "idle" }); setRefreshExisting(false); } }}
                placeholder="Issue, MR or PR link, Jira key, or folder path" autoComplete="off" spellCheck={false}
                aria-invalid={failure ? "true" : undefined} aria-describedby={failure ? failureId : undefined} />
              {lookup.status === "pending" ? <p className="task-setup-note">{trimmed.startsWith("/") || trimmed.startsWith("~") ? "Checking the folder…" : "Looking up the link…"}</p> : null}
              {resolution ? <p className="task-setup-note is-valid">✓ {existing ? `Already in Library · ${resolution.title}` : resolutionNote(resolution, providers)}</p> : null}
              {resolution?.diagnostics.map((diagnostic, index) => <p className="task-setup-note" key={`${diagnostic.code}:${index}`}>{diagnostic.message}</p>)}
              {existingInSpace?.text ? <p className="task-setup-note">{existingInSpace.text}</p> : null}
              {failure ? <div id={failureId} className="library-refusal" role="alert">
                <strong>{failure.title}</strong>
                <span>{failure.detail}</span>
                {failure.retry ? <button type="button" className="task-setup-link" onClick={() => setLookupRetry((value) => value + 1)}>Retry lookup</button> : null}
              </div> : null}
              {providersError ? <p className="task-setup-note">{providersError}</p> : null}
            </div>
          </div>
          {folder ? <div className="task-setup-row">
            <label htmlFor={`${fieldId}-label`}>Label</label>
            <div>
              <input id={`${fieldId}-label`} type="text" value={label ?? resolution.title} onChange={(event) => setLabel(event.target.value)} autoComplete="off" />
              <p className="task-setup-note">Copies files into the Library, not a live link. Later source edits stay outside the Library until you explicitly re-copy.</p>
            </div>
          </div> : null}
          {jiraKey && jira.length > 1 ? <div className="task-setup-row">
            <label htmlFor={`${fieldId}-provider`}>Provider</label>
            <div><select id={`${fieldId}-provider`} className="library-select" value={jiraProvider?.id ?? ""} onChange={(event) => setJiraProviderId(event.target.value)}>
              {jira.map((provider) => <option key={provider.id} value={provider.id}>{provider.id} · {provider.base_url}</option>)}
            </select></div>
          </div> : null}
          {resolution && forge && !existing ? <label className="task-setup-check"><input type="checkbox" checked={linked} onChange={(event) => setLinked(event.target.checked)} /> Include linked issues and {family?.review}s within import limits</label> : null}
          {existing ? <label className="task-setup-check"><input type="checkbox" checked={refreshExisting} onChange={(event) => setRefreshExisting(event.target.checked)} /> Refresh from source first</label> : null}
          <div className="task-setup-row">
            <span id={destinationId} className="library-row-label">Destination</span>
            {spaceChoice ? <div role="radiogroup" aria-labelledby={destinationId} className="library-destination-choices">
              <label className="task-setup-check"><input type="radio" name={destinationId} checked={destination === "library"} onChange={() => setDestination("library")} /> Library only</label>
              <label className="task-setup-check"><input type="radio" name={destinationId} checked={destination === "space"} onChange={() => setDestination("space")} /> Library and {spaceChoice.label}</label>
              {destination === "space" ? <p className="task-setup-note">Saved to the Library first, then copied into {spaceChoice.label}. Later Library refreshes don't change {spaceChoice.label} until you update it.</p> : null}
              {companion?.status === "unavailable" ? <p className="task-setup-note is-error">{spaceChoice.label}'s context folder couldn't be verified: {companion.error.message.replace(/\.$/, "")}. The Library copy is saved either way; a failed Space step can be retried.</p> : null}
            </div> : <div>
              <p className="library-destination">Library</p>
              <p className="task-setup-note">{space ? `Herdr isn't live, so this adds to the Library only.` : "Select a Space to also add it there."}</p>
            </div>}
          </div>
        </>}
      </div>
      <footer className="task-setup-footer">
        {!operation && !starting ? <>
          {startError && lastBeginRef.current ? <>
            <button type="button" onClick={addAnother}>Add another</button>
            <button ref={primaryActionRef} type="button" className="setup-primary" onClick={retryAll}>Retry</button>
          </> : null}
          {existing && onOpenItem ? <button type="button" onClick={() => { onOpenItem(existing); onClose(); }}>Open in Library</button> : null}
          <button type="button" onClick={onClose}>Cancel</button>
          {!startError ? <button type="button" className="setup-primary" onClick={submit} disabled={!canAdd}>{primaryLabel}</button> : null}
        </> : !finished || !progress ? <button ref={closeActionRef} type="button" onClick={onClose}>Close</button> : <>
          <button type="button" onClick={addAnother}>Add another</button>
          <button ref={closeActionRef} type="button" onClick={onClose}>Close</button>
          {/* The same source again: primary when nothing was saved, beside the result otherwise. */}
          {libraryIncomplete ? <button ref={progress.saved ? undefined : primaryActionRef} type="button" className={progress.saved ? undefined : "setup-primary"} onClick={retryAll}>Retry</button> : null}
          {!progress.saved || spaceFailed ? null : openablePath && openInSpace
            ? <button ref={primaryActionRef} type="button" className="setup-primary" onClick={() => { openInSpace.open(openablePath); onClose(); }}>{`Open in ${shownSpace}`}</button>
            : onOpenItem && openedItemId ? <button ref={primaryActionRef} type="button" className="setup-primary" onClick={() => { onOpenItem(openedItemId); onClose(); }}>Open in Library</button> : null}
        </>}
      </footer>
    </section>
  </div>, document.body);
}
