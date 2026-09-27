import { Fragment, useEffect, useId, useLayoutEffect, useRef, useState } from "react";
import type { CockpitClient } from "../../client/CockpitClient";
import type { ErrorResponse, ProjectProvider, SpaceAddAttempt, SpaceCopyRow, SpaceCopyState } from "../../protocol/generated/v1";
import { errorText, providerFamily, type LibrarySpace } from "./libraryState";
import { spaceCopyChip } from "./spaceCopyPresentation";
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

function CopyEntry({ row, providers }: { row: SpaceCopyRow; providers: readonly ProjectProvider[] }) {
  const chip = spaceCopyChip(row);
  return <article role="listitem" className="context-source-entry">
    <strong className="context-source-title" title={row.title}>{row.title}</strong>
    <div className="context-source-summary">
      {kindChips(row, providers).map((label) => <span key={label} className="context-source-chip">{label}</span>)}
      <span className={`context-source-chip library-state is-${chip.tone}`}><span aria-hidden="true">{chip.glyph}</span> {chip.word}</span>
    </div>
    {chip.notice ? <p className="context-source-diagnostic">{chip.notice}</p> : null}
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
  </article>;
}

/**
 * Library context held by one Space, inside the `Context resources` overlay
 * (design §4.8, S2): failed adds first with `Retry adding to <Space>` and
 * `Dismiss`, then copies that need action, then the rest. The order is fixed
 * when the list opens, so rows never move under the pointer while it is open.
 * Retrying copies the saved Library item again; it never fetches from the
 * provider.
 */
export function SpaceContextList({ client, space, state, onAdd }: {
  client: CockpitClient;
  space: LibrarySpace;
  state: SpaceListingState;
  onAdd: () => void;
}) {
  const addRef = useRef<HTMLButtonElement>(null);
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
  return <div className="space-context">
    <div className="space-context-bar">
      <span className="space-context-summary">In <strong>{space.label}</strong>{listing ? ` · ${itemCount} ${itemCount === 1 ? "item" : "items"}${listing.behind > 0 ? ` · ${listing.behind} behind` : ""}` : null}</span>
      <span className="context-toolbar-spacer" />
      <button ref={addRef} type="button" onClick={onAdd}>Add…</button>
    </div>
    {state.status === "error" ? <div className="context-resource-error" role="alert">
      <strong>{space.label}'s context couldn't be read.</strong><span>{state.error}</span>
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
        : <CopyEntry key={entry.key} row={entry.row} providers={providers} />)}
    </div>
  </div>;
}
