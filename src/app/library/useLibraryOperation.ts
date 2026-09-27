import { useCallback, useEffect, useRef, useState } from "react";
import type { CockpitClient } from "../../client/CockpitClient";
import type { LibraryItemSummary, LibraryListing, LibraryOperation, ProjectProvider, SpaceContextListing, SpaceTarget } from "../../protocol/generated/v1";
import { errorText, sameSpaceTarget } from "./libraryState";

/**
 * Every mounted Library surface rereads its listing when any of them finishes an
 * operation. The event's `detail` is the finished operation, or null for a
 * change without one (a removal), so a Context pane can expand the files a
 * Space copy wrote into its companion root.
 */
export const LIBRARY_CHANGED_EVENT = "cockpit:library-changed";

export function announceLibraryChanged(operation: LibraryOperation | null = null): void {
  window.dispatchEvent(new CustomEvent<LibraryOperation | null>(LIBRARY_CHANGED_EVENT, { detail: operation }));
}

const POLL_MS = 750;
const MAX_LISTING_PAGES = 200;

type TrackedOperation = { operation: LibraryOperation; stop?: () => void };
const tracked = new Map<string, TrackedOperation>();
const trackerListeners = new Set<() => void>();

function notifyTracker(): void {
  for (const listener of trackerListeners) listener();
}

// Space adds and updates copy saved items into a Space; the Library items themselves don't change.
function pendingLibraryItemIds(): Set<string> {
  return new Set([...tracked.values()].filter(({ operation }) => !operation.finished && operation.kind !== "space_add" && operation.kind !== "space_update").flatMap(({ operation }) => operation.item_ids));
}

function storeOperation(operation: LibraryOperation): void {
  const previous = tracked.get(operation.operation_id)?.operation;
  const entry = tracked.get(operation.operation_id) ?? { operation };
  entry.operation = operation;
  tracked.set(operation.operation_id, entry);
  notifyTracker();
  if (operation.finished && !previous?.finished) announceLibraryChanged(operation);
  if (operation.finished) entry.stop?.();
  let finishedCount = 0;
  for (const [id, current] of tracked) {
    if (!current.operation.finished) continue;
    finishedCount += 1;
    if (finishedCount > 32) tracked.delete(id);
  }
}

function startTracking(client: CockpitClient, operation: LibraryOperation): void {
  storeOperation(operation);
  if (operation.finished || tracked.get(operation.operation_id)?.stop) return;
  let timer: number | undefined;
  let stopped = false;
  const cleanup = () => {
    stopped = true;
    window.clearTimeout(timer);
    document.removeEventListener("visibilitychange", poll);
    const entry = tracked.get(operation.operation_id);
    if (entry?.stop === cleanup) entry.stop = undefined;
  };
  const poll = async () => {
    if (stopped) return;
    if (document.visibilityState === "hidden") {
      document.addEventListener("visibilitychange", poll, { once: true });
      return;
    }
    try {
      const next = await client.libraryOperation(operation.operation_id);
      if (stopped) return;
      storeOperation(next);
      if (next.finished) return;
    } catch {
      // Keep the last confirmed phase and retry while the app is visible.
    }
    timer = window.setTimeout(() => { void poll(); }, POLL_MS);
  };
  tracked.get(operation.operation_id)!.stop = cleanup;
  timer = window.setTimeout(() => { void poll(); }, POLL_MS);
}

export type LibraryOperationState = {
  operation: LibraryOperation | null;
  error: string | null;
  starting: boolean;
  running: boolean;
  pendingItemIds: ReadonlySet<string>;
  start: (begin: () => Promise<LibraryOperation>) => Promise<LibraryOperation | null>;
  /** Follows an operation this surface accepted before it last unmounted. */
  resume: (operation: LibraryOperation) => void;
  cancel: () => void;
  reset: () => void;
};

/** Follows one operation in app-lifetime tracking; unmounting never cancels it. */
export function useLibraryOperation(client: CockpitClient, onFinished?: (operation: LibraryOperation) => void): LibraryOperationState {
  const [operation, setOperation] = useState<LibraryOperation | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [starting, setStarting] = useState(false);
  const [pendingItemIds, setPendingItemIds] = useState<ReadonlySet<string>>(() => pendingLibraryItemIds());
  const generation = useRef(0);
  const onFinishedRef = useRef(onFinished);
  onFinishedRef.current = onFinished;
  const finishedNotified = useRef<string | null>(null);
  useEffect(() => {
    const update = () => setPendingItemIds(pendingLibraryItemIds());
    trackerListeners.add(update);
    update();
    return () => { trackerListeners.delete(update); };
  }, []);
  useEffect(() => {
    const operationId = operation?.operation_id;
    if (!operationId) return;
    const update = () => {
      const next = tracked.get(operationId)?.operation;
      if (!next) return;
      setOperation(next);
      if (next.finished && finishedNotified.current !== operationId) {
        finishedNotified.current = operationId;
        onFinishedRef.current?.(next);
      }
    };
    trackerListeners.add(update);
    update();
    return () => { trackerListeners.delete(update); };
  }, [operation?.operation_id]);
  const start = useCallback(async (begin: () => Promise<LibraryOperation>) => {
    const token = ++generation.current;
    setStarting(true);
    setError(null);
    setOperation(null);
    try {
      const next = await begin();
      if (token !== generation.current) return null;
      setOperation(next);
      startTracking(client, next);
      return next;
    } catch (cause) {
      if (token === generation.current) setError(errorText(cause, "The Library operation could not start."));
      return null;
    } finally {
      if (token === generation.current) setStarting(false);
    }
  }, [client]);
  const resume = useCallback((previous: LibraryOperation) => {
    generation.current += 1;
    setError(null);
    setStarting(false);
    const latest = tracked.get(previous.operation_id)?.operation;
    // A finished record that aged out of tracking was already announced once.
    if (!latest && previous.finished) { setOperation(previous); return; }
    setOperation(latest ?? previous);
    startTracking(client, latest ?? previous);
  }, [client]);
  const operationId = operation?.operation_id ?? null;
  const cancel = useCallback(() => {
    if (!operationId) return;
    void client.libraryOperationCancel(operationId).then((next) => {
      storeOperation(next);
      setOperation(next);
    }, (cause: unknown) => setError(errorText(cause, "The operation could not be cancelled.")));
  }, [client, operationId]);
  const reset = useCallback(() => {
    generation.current += 1;
    setOperation(null);
    setError(null);
    setStarting(false);
  }, []);
  return { operation, error, starting, running: operation !== null && !operation.finished, pendingItemIds, start, resume, cancel, reset };
}

export type LibraryListingState = {
  status: "idle" | "loading" | "ready" | "error";
  listing: LibraryListing | null;
  error: string | null;
  providers: ProjectProvider[];
  reload: () => void;
};

async function readAllPages(client: CockpitClient): Promise<LibraryListing> {
  // A generation change between pages means the index moved; start over once.
  for (let attempt = 0; attempt < 2; attempt += 1) {
    const first = await client.libraryListing(null);
    const items: LibraryItemSummary[] = [...first.items];
    let next = first.next_offset;
    let pages = 1;
    let moved = false;
    while (next !== null && pages < MAX_LISTING_PAGES) {
      const page = await client.libraryListing(next);
      if (page.generation !== first.generation) { moved = true; break; }
      items.push(...page.items);
      next = page.next_offset;
      pages += 1;
    }
    if (!moved) return { ...first, items, next_offset: next };
  }
  throw new Error("The Library changed while it was being read; reload it.");
}

/**
 * Reads the whole Library listing while `active`, plus the configured
 * providers used for labels. Nothing is requested while inactive, so a Context
 * pane that never shows the Library root never touches the Library.
 */
export function useLibraryListing(client: CockpitClient, active: boolean): LibraryListingState {
  const [state, setState] = useState<{ status: LibraryListingState["status"]; listing: LibraryListing | null; error: string | null }>({ status: "idle", listing: null, error: null });
  const [providers, setProviders] = useState<ProjectProvider[]>([]);
  const [revision, setRevision] = useState(0);
  const reload = useCallback(() => setRevision((value) => value + 1), []);
  useEffect(() => {
    if (!active) return;
    let current = true;
    setState((previous) => ({ ...previous, status: "loading" }));
    readAllPages(client).then((listing) => {
      if (current) setState({ status: "ready", listing, error: null });
    }, (cause: unknown) => {
      if (current) setState((previous) => ({ status: "error", listing: previous.listing, error: errorText(cause, "The Library could not be read.") }));
    });
    return () => { current = false; };
  }, [active, client, revision]);
  useEffect(() => {
    if (!active) return;
    let current = true;
    // Labels fall back to provider ids when configuration is unavailable.
    client.projectConfiguration().then((configuration) => { if (current) setProviders(configuration.providers); }, () => undefined);
    return () => { current = false; };
  }, [active, client]);
  useEffect(() => {
    if (!active) return;
    window.addEventListener(LIBRARY_CHANGED_EVENT, reload);
    return () => window.removeEventListener(LIBRARY_CHANGED_EVENT, reload);
  }, [active, reload]);
  return { ...state, providers, reload };
}

export type SpaceListingState = {
  status: "idle" | "loading" | "ready" | "error";
  /** Always the listing of the requested Space; never another Space's rows. */
  listing: SpaceContextListing | null;
  error: string | null;
  /** Counts listings received, so a surface can wait for one read after it opened. */
  received: number;
  reload: () => void;
};

/**
 * Reads one Space's Library copies and durable add attempts while `active`
 * (design §4.8, D10). A listing is kept only for the Space that asked for it:
 * changing the target drops it, and a late response for another Space is
 * ignored. While an attempt is pending it rereads until the attempt settles.
 */
export function useSpaceContextListing(client: CockpitClient, target: SpaceTarget | null, active: boolean): SpaceListingState {
  const sessionId = target?.session_id ?? null;
  const spaceId = target?.space_id ?? null;
  const [state, setState] = useState<{ status: SpaceListingState["status"]; listing: SpaceContextListing | null; error: string | null; received: number }>({ status: "idle", listing: null, error: null, received: 0 });
  const [revision, setRevision] = useState(0);
  const reload = useCallback(() => setRevision((value) => value + 1), []);
  useEffect(() => {
    if (!active || !sessionId || !spaceId) return;
    const request = { target: { session_id: sessionId, space_id: spaceId } };
    const controller = new AbortController();
    setState((previous) => ({ ...previous, status: "loading" }));
    client.librarySpaceList(request, controller.signal).then((listing) => {
      if (controller.signal.aborted) return;
      if (sameSpaceTarget(listing.target, request.target)) setState((previous) => ({ status: "ready", listing, error: null, received: previous.received + 1 }));
      else setState((previous) => ({ ...previous, status: "error", error: "Cockpit answered for a different Space." }));
    }, (cause: unknown) => {
      if (!controller.signal.aborted) setState((previous) => ({ ...previous, status: "error", error: errorText(cause, "This Space's context could not be read.") }));
    });
    return () => controller.abort();
  }, [active, client, sessionId, spaceId, revision]);
  useEffect(() => {
    if (!active || !sessionId || !spaceId) return;
    window.addEventListener(LIBRARY_CHANGED_EVENT, reload);
    return () => window.removeEventListener(LIBRARY_CHANGED_EVENT, reload);
  }, [active, reload, sessionId, spaceId]);
  const listing = state.listing && target && sameSpaceTarget(state.listing.target, target) ? state.listing : null;
  const attemptPending = listing?.attempts.some((attempt) => attempt.state === "pending") ?? false;
  useEffect(() => {
    if (!active || !attemptPending) return;
    const timer = window.setTimeout(reload, POLL_MS);
    return () => window.clearTimeout(timer);
  }, [active, attemptPending, listing, reload]);
  const status = !listing && state.status === "ready" ? "loading" : state.status;
  return { status, listing, error: state.error, received: state.received, reload };
}
