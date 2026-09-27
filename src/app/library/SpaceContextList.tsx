import { Fragment, useCallback, useEffect, useId, useLayoutEffect, useRef, useState, type FocusEvent } from "react";
import type { CockpitClient } from "../../client/CockpitClient";
import type { ErrorResponse, LibraryOperation, ProjectProvider, SpaceAddAttempt, SpaceCopyRow, SpaceCopyState } from "../../protocol/generated/v1";
import { SpaceCopyConfirmDialog, spaceCopyConflict, type SpaceCopyConfirmation } from "./LibraryConfirmDialog";
import { errorText, providerFamily, type LibrarySpace } from "./libraryState";
import { spaceCopyChip, type SpaceCopyAction } from "./spaceCopyPresentation";
import { useLibraryOperation, type SpaceListingState } from "./useLibraryOperation";
import "./library.css";

/** Rows needing action come first (design §4.8, D23); `Up to date` and `Not linked` last. */
const NEEDS_ACTION: Record<SpaceCopyState, boolean> = {
  library_newer: true,
  missing_in_space: true,
  edited_in_space: true,
  removed_at_source: true,
  not_in_library: true,
  up_to_date: false,
  not_linked: false,
};

/**
 * Explicit update is offered only for copies that are behind or missing
 * (design §4.8); edited, removed-at-source, not-in-Library and unlinked copies
 * are never written by `Update` or `Update all`.
 */
const UPDATABLE: Record<SpaceCopyState, boolean> = {
  library_newer: true,
  missing_in_space: true,
  edited_in_space: false,
  removed_at_source: false,
  not_in_library: false,
  up_to_date: false,
  not_linked: false,
};

// Followed spaces get their Space actions with S6; until then a follow row offers none.
function followRow(row: SpaceCopyRow): boolean {
  return row.follow !== null || row.logical_id.startsWith("follow:");
}

/** A row `Update`/`Restore from Library` may write, and that `Update all (N)` counts. */
export function spaceCopyUpdatable(row: SpaceCopyRow): boolean {
  return UPDATABLE[row.state] && row.item_id !== null && !followRow(row);
}

/** The Space-copy actions a Space surface performs for a row, in `spaceCopyChip` order. */
export function spaceCopyActions(row: SpaceCopyRow): SpaceCopyAction[] {
  return spaceCopyChip(row).actions.filter((action) => {
    switch (action.kind) {
      case "update":
      case "restore":
        return spaceCopyUpdatable(row);
      case "replace":
        return row.item_id !== null && row.edited.length > 0 && !followRow(row);
      case "remove":
        return !followRow(row);
      default:
        return false;
    }
  });
}

export type SpaceUpdateOutcome = { failed: boolean; skipped: number; text: string };

/** Each Library item's Space copy paths when an update started, so its report never follows later listings. */
export type SpaceUpdatePaths = ReadonlyMap<string, readonly string[]>;

/**
 * What a finished explicit update did in one Space (design §4.7, §4.8), or
 * null while it runs. It reads only the operation's own written and skipped
 * paths and the item paths captured when it started: an item counts as
 * updated when one of its files was written. A written file no captured item
 * owns counts for the items the capture didn't know.
 */
export function spaceUpdateOutcome(operation: LibraryOperation, space: string, itemPaths: SpaceUpdatePaths): SpaceUpdateOutcome | null {
  if (!operation.finished) return null;
  const stopped = operation.phases.find((phase) => phase.phase === "space" && (phase.state === "failed" || phase.state === "cancelled"));
  if (stopped) {
    const text = stopped.error?.code === "space_copy_conflict" ? spaceCopyConflict(space) : `Not updated in ${space}. ${stopped.error?.message ?? "The update stopped before it finished."}`;
    return { failed: true, skipped: 0, text };
  }
  const skipped = operation.space?.skipped_edited ?? [];
  const written = new Set(operation.space?.written ?? []);
  const known = new Set([...itemPaths.values()].flat());
  const writtenUnknown = [...written].some((path) => !known.has(path));
  const updated = operation.item_ids.filter((itemId) => {
    const paths = itemPaths.get(itemId);
    return paths ? paths.some((path) => written.has(path)) : writtenUnknown;
  }).length;
  const skippedText = skipped.length > 0 ? ` Skipped ${skipped.length} edited ${skipped.length === 1 ? "copy" : "copies"}.` : "";
  const text = updated > 0 ? `Updated ${updated} ${updated === 1 ? "item" : "items"} in ${space}.${skippedText}` : skipped.length > 0 ? `Nothing updated in ${space}.${skippedText}` : `Nothing in ${space} needed updating.`;
  return { failed: false, skipped: skipped.length, text };
}

/** A finished update whose Space listing couldn't be reread: its result stays unshown until a reread succeeds. */
export function spaceUpdateUnconfirmed(space: string): string {
  return `The update finished, but ${space}'s copies couldn't be reread, so its result isn't shown yet.`;
}

/**
 * Follows one explicit Space update until the Space's listing is reread after
 * it finished, so a surface never shows a result the listing hasn't confirmed.
 * `working` drives the spinner; `busy` also holds conflicting actions while a
 * failed reread leaves the update `unconfirmed`, until a reread succeeds.
 */
export function useSpaceUpdate(client: CockpitClient, listing: SpaceListingState) {
  const received = useRef(listing.received);
  received.current = listing.received;
  const rows = useRef(listing.listing?.rows ?? []);
  rows.current = listing.listing?.rows ?? [];
  const [settleFrom, setSettleFrom] = useState<number | null>(null);
  const [itemPaths, setItemPaths] = useState<SpaceUpdatePaths>(() => new Map());
  const operation = useLibraryOperation(client, () => setSettleFrom(received.current));
  const startOperation = operation.start;
  const start = useCallback((begin: () => Promise<LibraryOperation>) => {
    setItemPaths(new Map(rows.current.flatMap((row) => row.item_id ? [[row.item_id, row.paths] as const] : [])));
    return startOperation(begin);
  }, [startOperation]);
  const awaitingReread = settleFrom !== null && listing.received === settleFrom;
  const settling = awaitingReread && listing.status !== "error";
  const unconfirmed = awaitingReread && listing.status === "error";
  const working = operation.starting || operation.running || settling;
  return { ...operation, start, itemPaths, settling, unconfirmed, working, busy: working || unconfirmed };
}

type Entry = { key: string; group: number; title: string } & ({ kind: "attempt"; attempt: SpaceAddAttempt } | { kind: "row"; row: SpaceCopyRow });

function listEntries(attempts: readonly SpaceAddAttempt[], rows: readonly SpaceCopyRow[]): Entry[] {
  return [
    ...attempts.map((attempt): Entry => ({ key: `attempt:${attempt.item_id ?? attempt.follow_id}`, group: 0, title: attempt.title, kind: "attempt", attempt })),
    ...rows.map((row): Entry => ({ key: `row:${row.logical_id}`, group: NEEDS_ACTION[row.state] ? 1 : 2, title: row.title, kind: "row", row })),
  ];
}

/** `GitLab` `MR`, `Jira` `issue`, `Folder`: the provider and kind chips of a Space row. */
function kindChips(row: SpaceCopyRow, providers: readonly ProjectProvider[]): string[] {
  if (row.kind === "folder_copy") return ["Folder"];
  const family = providerFamily(providers, row.provider_id);
  return [family.name, family.key !== "jira" && row.resource_type === "review" ? family.review : "issue"];
}

/**
 * What one failed add left behind. The Library copy was saved before the Space
 * step began (D10); only a companion that couldn't be verified is known to have
 * received nothing.
 */
export function spaceAddFailure(error: ErrorResponse | null, space: string): string {
  if (error?.code === "source_companion_unavailable") return `Saved to the Library, but ${space}'s context folder couldn't be verified. Nothing was written to ${space}.`;
  return `Saved to the Library, but not added to ${space}. ${error?.message ?? "The Space step stopped."}`;
}

function AttemptEntry({ client, space, attempt, onFocusLeaving, onDismissed }: {
  client: CockpitClient;
  space: LibrarySpace;
  attempt: SpaceAddAttempt;
  onFocusLeaving: () => void;
  onDismissed: () => void;
}) {
  const ref = useRef<HTMLElement>(null);
  const failureId = useId();
  const retry = useLibraryOperation(client);
  const [dismissing, setDismissing] = useState(false);
  const [dismissError, setDismissError] = useState<string | null>(null);
  // A settled attempt disappears; focus moves before its row is removed.
  const onFocusLeavingRef = useRef(onFocusLeaving);
  onFocusLeavingRef.current = onFocusLeaving;
  useLayoutEffect(() => {
    const row = ref.current;
    return () => { if (row?.contains(document.activeElement)) onFocusLeavingRef.current(); };
  }, []);
  const request = { target: space.target, item_ids: attempt.item_id ? [attempt.item_id] : [], follow_ids: attempt.follow_id ? [attempt.follow_id] : [] };
  const pending = attempt.state === "pending" || retry.starting || retry.running;
  const busy = pending || dismissing;
  const dismiss = () => {
    setDismissing(true);
    setDismissError(null);
    client.librarySpaceAttemptsDismiss(request).then(onDismissed, (cause: unknown) => {
      setDismissError(errorText(cause, "The failed add could not be dismissed."));
      setDismissing(false);
    });
  };
  const failure = retry.error ?? dismissError;
  return <article ref={ref} role="listitem" className="context-source-entry">
    <strong className="context-source-title" title={attempt.title}>{attempt.title}</strong>
    <div className="space-context-actions">
      {/* aria-disabled keeps focus on the pressed button while the retry runs. */}
      <button type="button" aria-disabled={busy} aria-describedby={attempt.state === "failed" && !pending ? failureId : undefined}
        onClick={() => { if (!busy) void retry.start(() => client.librarySpaceAdd(request)); }}>{`Retry adding to ${space.label}`}</button>
      <button type="button" aria-disabled={busy} onClick={() => { if (!busy) dismiss(); }}>Dismiss</button>
    </div>
    <div className="context-source-summary">
      {pending
        ? <span className="context-source-chip library-state is-muted"><span className="library-spinner" aria-hidden="true" />Adding…</span>
        : <span className="context-source-chip library-state is-blocked"><span aria-hidden="true">✕</span> Not added — Retry</span>}
    </div>
    {attempt.state === "failed" && !pending ? <p id={failureId} className="context-source-diagnostic" role="alert">{spaceAddFailure(attempt.error, space.label)}</p> : null}
    {failure ? <p className="context-source-diagnostic" role="alert">{failure}</p> : null}
  </article>;
}

function CopyEntry({ client, space, row, providers, listing, updatingAll, onFocusLeaving }: {
  client: CockpitClient;
  space: LibrarySpace;
  row: SpaceCopyRow;
  providers: readonly ProjectProvider[];
  listing: SpaceListingState;
  /** `Update all` in this Space: `busy` holds this row's update actions, `working` shows its spinner. */
  updatingAll: { busy: boolean; working: boolean };
  onFocusLeaving: () => void;
}) {
  const ref = useRef<HTMLElement>(null);
  const titleId = useId();
  const update = useSpaceUpdate(client, listing);
  const [confirmation, setConfirmation] = useState<SpaceCopyConfirmation | null>(null);
  const [conflict, setConflict] = useState(false);
  // A removed copy disappears; focus moves before its row is removed.
  const onFocusLeavingRef = useRef(onFocusLeaving);
  onFocusLeavingRef.current = onFocusLeaving;
  useLayoutEffect(() => {
    const entry = ref.current;
    return () => { if (entry?.contains(document.activeElement)) onFocusLeavingRef.current(); };
  }, []);
  // An action that no longer applies after the reread gives way to the row's next action, or to `Add…`.
  const slotRef = useRef<HTMLDivElement>(null);
  const slotFocused = useRef(false);
  useLayoutEffect(() => {
    if (!slotFocused.current || (document.activeElement !== null && document.activeElement !== document.body)) return;
    const next = slotRef.current?.querySelector("button");
    if (next) next.focus({ preventScroll: true }); else onFocusLeavingRef.current();
  });
  const chip = spaceCopyChip(row);
  const actions = spaceCopyActions(row);
  const updatable = spaceCopyUpdatable(row);
  const busy = update.busy || (updatingAll.busy && updatable);
  const working = update.working || (updatingAll.working && updatable);
  const outcome = update.operation && !update.busy ? spaceUpdateOutcome(update.operation, space.label, update.itemPaths) : null;
  const note = conflict ? spaceCopyConflict(space.label) : update.error ?? (update.unconfirmed ? spaceUpdateUnconfirmed(space.label) : outcome && (outcome.failed || outcome.skipped > 0) ? outcome.text : null);
  const act = (action: SpaceCopyAction) => {
    if (busy) return;
    setConflict(false);
    if (action.kind === "replace" || action.kind === "remove") { setConfirmation({ kind: action.kind, row }); return; }
    const itemId = row.item_id;
    if (itemId) void update.start(() => client.librarySpaceUpdate({ target: space.target, scope: { scope: "selection", item_ids: [itemId], follow_ids: [] }, replace_edited: [] }));
  };
  return <article ref={ref} role="listitem" className="context-source-entry">
    <strong id={titleId} className="context-source-title" title={row.title}>{row.title}</strong>
    <div ref={slotRef} className="space-context-actions" onFocus={() => { slotFocused.current = true; }} onBlur={(event: FocusEvent) => { if (event.relatedTarget) slotFocused.current = false; }}>
      {/* aria-disabled keeps focus on the pressed button while the update runs. */}
      {actions.map((action) => <button key={action.kind} type="button" aria-disabled={busy} aria-describedby={titleId} onClick={() => act(action)}>{action.label}</button>)}
    </div>
    <div className="context-source-summary">
      {kindChips(row, providers).map((label) => <span key={label} className="context-source-chip">{label}</span>)}
      {working
        ? <span className="context-source-chip library-state is-muted"><span className="library-spinner" aria-hidden="true" />Updating…</span>
        : <span className={`context-source-chip library-state is-${chip.tone}`}><span aria-hidden="true">{chip.glyph}</span> {chip.word}</span>}
    </div>
    {chip.notice ? <p className="context-source-diagnostic">{chip.notice}</p> : null}
    {note && !working ? <p className="context-source-diagnostic" role={conflict || update.error || update.unconfirmed || outcome?.failed ? "alert" : "status"}>{note}</p> : null}
    <details className="context-source-details">
      <summary>Details</summary>
      <dl>
        {row.item_id ? <><dt>Library item</dt><dd><code>{row.item_id}</code></dd></> : null}
        {row.paths.map((path) => <Fragment key={path}><dt>Space copy path</dt><dd><code>{path}</code></dd></Fragment>)}
        {row.copy_mode ? <><dt>Copied as</dt><dd>{row.copy_mode === "mixed" ? "reflink and copy" : row.copy_mode}</dd></> : null}
        {row.library_revision_copied ? <><dt>Library version copied</dt><dd><code>{row.library_revision_copied}</code></dd></> : null}
        {row.current_library_revision ? <><dt>Current Library version</dt><dd><code>{row.current_library_revision}</code></dd></> : null}
        {row.edited.map((file) => <Fragment key={file.path}><dt>Edited file</dt><dd><code>{file.path}</code></dd></Fragment>)}
      </dl>
    </details>
    {confirmation ? <SpaceCopyConfirmDialog client={client} space={space} confirmation={confirmation}
      onReplacing={(operation) => void update.start(async () => operation)}
      onConflict={() => { setConflict(true); listing.reload(); }}
      onClose={() => setConfirmation(null)} /> : null}
  </article>;
}

/**
 * Library context held by one Space, inside the `Context resources` overlay
 * (design §4.8): failed adds first with `Retry adding to <Space>` and
 * `Dismiss`, then copies that need action, then the rest. The order is fixed
 * when the list opens, so rows never move under the pointer while it is open.
 * Retrying copies the saved Library item again; it never fetches from the
 * provider. `Update`, `Update all (N)`, a confirmed replace and a confirmed
 * removal write only this Space; each shows its result once the Space's
 * listing has been reread.
 */
export function SpaceContextList({ client, space, state, onAdd }: {
  client: CockpitClient;
  space: LibrarySpace;
  state: SpaceListingState;
  onAdd: () => void;
}) {
  const addRef = useRef<HTMLButtonElement>(null);
  const updateAll = useSpaceUpdate(client, state);
  const [providers, setProviders] = useState<ProjectProvider[]>([]);
  const order = useRef<Map<string, number> | null>(null);
  // Copies can change on disk while Resources is closed: reread on open, and order only fresh rows.
  const [receivedBeforeOpen] = useState(state.received);
  const reload = state.reload;
  useEffect(() => { reload(); }, [reload]);
  useEffect(() => {
    let current = true;
    // Chips fall back to provider ids when configuration is unavailable.
    client.projectConfiguration().then((configuration) => { if (current) setProviders(configuration.providers); }, () => undefined);
    return () => { current = false; };
  }, [client]);
  const listing = state.received > receivedBeforeOpen ? state.listing : null;
  const entries = listing ? listEntries(listing.attempts, listing.rows) : [];
  if (listing) {
    const fresh = entries.filter((entry) => !order.current?.has(entry.key))
      .sort((left, right) => left.group - right.group || left.title.localeCompare(right.title));
    order.current ??= new Map();
    for (const entry of fresh) order.current.set(entry.key, order.current.size);
  }
  entries.sort((left, right) => (order.current?.get(left.key) ?? 0) - (order.current?.get(right.key) ?? 0));
  const companion = listing?.companion;
  const itemCount = listing?.rows.length ?? 0;
  // `Update all (N)` counts `Library newer` and `Missing in Space` copies. It stays available while edited
  // copies exist, so an update can report them as skipped (design scenario 7); it never writes them.
  const eligible = listing?.rows.filter(spaceCopyUpdatable).length ?? 0;
  const updateAllAvailable = eligible > 0 || (listing?.rows.some((row) => row.state === "edited_in_space" && row.item_id !== null && !followRow(row)) ?? false);
  const updateAllOutcome = updateAll.operation && !updateAll.busy ? spaceUpdateOutcome(updateAll.operation, space.label, updateAll.itemPaths) : null;
  const updateAllFailure = updateAll.error ?? (updateAllOutcome?.failed ? updateAllOutcome.text : null);
  return <div className="space-context">
    <div className="space-context-bar">
      <span className="space-context-summary">In <strong>{space.label}</strong>{listing ? ` · ${itemCount} ${itemCount === 1 ? "item" : "items"}${listing.behind > 0 ? ` · ${listing.behind} behind` : ""}` : null}</span>
      <span className="context-toolbar-spacer" />
      <button ref={addRef} type="button" onClick={onAdd}>Add…</button>
      {companion?.status === "available" ? <button type="button" aria-disabled={updateAll.busy || !updateAllAvailable}
        onClick={() => { if (!updateAll.busy && updateAllAvailable) void updateAll.start(() => client.librarySpaceUpdate({ target: space.target, scope: { scope: "all" }, replace_edited: [] })); }}>{`Update all (${eligible})`}</button> : null}
    </div>
    {updateAll.working ? <p className="context-resource-loading" role="status">{`Updating ${space.label}…`}</p> : null}
    {!updateAll.busy && updateAllFailure ? <div className="context-resource-error" role="alert">{updateAllFailure}</div> : null}
    {!updateAll.busy && updateAllOutcome && !updateAllOutcome.failed ? <p className="context-resource-loading" role="status">{updateAllOutcome.text}</p> : null}
    {/* A failed reread after `Update all` replaces its result: the listing, not the operation, confirms it. */}
    {state.status === "error" ? <div className="context-resource-error" role="alert">
      <strong>{updateAll.unconfirmed ? spaceUpdateUnconfirmed(space.label) : `${space.label}'s context couldn't be read.`}</strong><span>{state.error}</span>
      <button type="button" onClick={state.reload}>Retry</button>
    </div> : null}
    {companion?.status === "unavailable" ? <div className="context-resource-error" role="alert">
      <strong>{space.label}'s context folder couldn't be verified.</strong><span>{companion.error.message.replace(/\.$/, "")}. Library items are unaffected, and failed adds stay here to retry.</span>
      <button type="button" onClick={state.reload}>Check again</button>
    </div> : null}
    <div className="context-resource-list" role="list" aria-label={`Library context in ${space.label}`} aria-busy={state.status === "loading" ? "true" : undefined}>
      {!listing && state.status !== "error" ? <p className="context-resource-loading" role="status">Loading…</p> : null}
      {listing && entries.length === 0 && companion?.status === "available" ? <p className="context-resource-empty">Nothing from the Library is in {space.label} yet. Use Add… to save context to the Library and copy it here.</p> : null}
      {entries.map((entry) => entry.kind === "attempt"
        ? <AttemptEntry key={entry.key} client={client} space={space} attempt={entry.attempt} onDismissed={state.reload} onFocusLeaving={() => addRef.current?.focus({ preventScroll: true })} />
        : <CopyEntry key={entry.key} client={client} space={space} row={entry.row} providers={providers} listing={state} updatingAll={updateAll} onFocusLeaving={() => addRef.current?.focus({ preventScroll: true })} />)}
    </div>
  </div>;
}
