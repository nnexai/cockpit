import { useCallback, useEffect, useRef, useState } from "react";
import type { CockpitClient } from "../../client/CockpitClient";
import type { LibraryItemSummary, LibraryListing, LibraryOperation, ProjectProvider, SpaceContextListing, SpaceTarget } from "../../protocol/generated/v1";
import { subscribeLocalChanges } from "../localChanges";
import { watchFallback } from "../watchFallback";
import { errorText, sameSpaceTarget } from "./libraryState";

/** Every mounted Library surface rereads its listing after a completed operation or selection change. */
export const LIBRARY_CHANGED_EVENT = "cockpit:library-changed";

export type LibraryChangeDetail = LibraryOperation | { kind: "snapshot_update" } | null;

export function announceLibraryChanged(detail: LibraryChangeDetail = null): void {
  window.dispatchEvent(new CustomEvent<LibraryChangeDetail>(LIBRARY_CHANGED_EVENT, { detail }));
}

const POLL_MS = 750;
const MAX_LISTING_PAGES = 200;

type TrackedOperation = { operation: LibraryOperation; stop?: () => void };
const tracked = new Map<string, TrackedOperation>();
const trackerListeners = new Set<() => void>();
const visibleOperations = new Map<string, number>();
export type LibraryProblem = { id: string; label: string; message: string };
const backgroundProblems = new Map<string, LibraryProblem>();
let rejectedStartId = 0;

export function dismissLibraryProblem(operationId: string): void {
  backgroundProblems.delete(operationId);
  notifyTracker();
}

export function useLibraryProblems(): LibraryProblem[] {
  const [problems, setProblems] = useState(() => [...backgroundProblems.values()]);
  useEffect(() => {
    const update = () => setProblems([...backgroundProblems.values()]);
    trackerListeners.add(update);
    update();
    return () => { trackerListeners.delete(update); };
  }, []);
  return problems;
}

function notifyTracker(): void {
  for (const listener of trackerListeners) listener();
}

// Selecting Library items for a Space does not change the items themselves.
function pendingLibraryItemIds(): Set<string> {
  return new Set([...tracked.values()].filter(({ operation }) => !operation.finished && operation.kind !== "space_add").flatMap(({ operation }) => operation.item_ids));
}

function storeOperation(operation: LibraryOperation, foreground = false): void {
  const previous = tracked.get(operation.operation_id)?.operation;
  const entry = tracked.get(operation.operation_id) ?? { operation };
  entry.operation = operation;
  tracked.set(operation.operation_id, entry);
  if (operation.finished && !previous?.finished && !foreground && !visibleOperations.has(operation.operation_id)
    && (operation.phases.some((phase) => phase.state === "failed") || operation.report?.rows.some((row) => row.outcome === "failed"))) {
    backgroundProblems.set(operation.operation_id, {
      id: operation.operation_id,
      label: `${operation.kind.replaceAll("_", " ")} failed`,
      message: operation.phases.filter((phase) => phase.state === "failed").map((phase) => phase.error?.message ?? phase.message ?? `${phase.phase} step failed`).join(" ")
        || operation.report?.rows.filter((row) => row.outcome === "failed").map((row) => `${row.title}: ${row.reason ?? "failed"}`).join("; ") || "The Library operation failed.",
    });
  }
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

function startTracking(client: CockpitClient, operation: LibraryOperation, foreground = false): void {
  storeOperation(operation, foreground);
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
  const mounted = useRef(true);
  useEffect(() => {
    mounted.current = true;
    return () => { mounted.current = false; };
  }, []);
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
    visibleOperations.set(operationId, (visibleOperations.get(operationId) ?? 0) + 1);
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
    return () => {
      trackerListeners.delete(update);
      const remaining = (visibleOperations.get(operationId) ?? 1) - 1;
      if (remaining > 0) visibleOperations.set(operationId, remaining);
      else visibleOperations.delete(operationId);
    };
  }, [operation?.operation_id]);
  const start = useCallback(async (begin: () => Promise<LibraryOperation>) => {
    const token = ++generation.current;
    setStarting(true);
    setError(null);
    setOperation(null);
    try {
      const next = await begin();
      // Cockpit accepted it: it is polled and announced even when a newer start now owns this surface.
      startTracking(client, next, mounted.current && token === generation.current);
      if (token !== generation.current) return null;
      setOperation(next);
      return next;
    } catch (cause) {
      const message = errorText(cause, "The Library operation could not start.");
      if (mounted.current && token === generation.current) setError(message);
      else {
        const id = `library-start:${++rejectedStartId}`;
        backgroundProblems.set(id, { id, label: "Library operation failed", message });
        notifyTracker();
      }
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
  watchUnavailable: boolean;
  retryWatch: () => void;
};

async function readAllPages(client: CockpitClient, seed: LibraryListing | null, isCurrent: () => boolean): Promise<LibraryListing | null> {
  // A generation change between pages means the index moved; start over once.
  for (let attempt = 0; attempt < 2; attempt += 1) {
    if (!isCurrent()) return null;
    const first = attempt === 0 && seed ? seed : await client.libraryListing(null);
    if (!isCurrent()) return null;
    const items: LibraryItemSummary[] = [...first.items];
    let next = first.next_offset;
    let pages = 1;
    let moved = false;
    while (next !== null && pages < MAX_LISTING_PAGES) {
      const page = await client.libraryListing(next);
      if (!isCurrent()) return null;
      if (page.generation !== first.generation) { moved = true; break; }
      items.push(...page.items);
      next = page.next_offset;
      pages += 1;
    }
    if (!moved) return { ...first, items, next_offset: next };
  }
  throw new Error("The Library changed while it was being read; reload it.");
}

function followSyncKey(listing: LibraryListing): string {
  return JSON.stringify(listing.follows.map((follow) => [follow.follow_id, follow.sync?.last_success_at ?? null, follow.sync?.reason ?? null]));
}

/** Reads a visible Library listing after watch registration and on invalidation. */
export function useLibraryListing(client: CockpitClient, active: boolean): LibraryListingState {
  const [state, setState] = useState<{ status: LibraryListingState["status"]; listing: LibraryListing | null; error: string | null }>({ status: "idle", listing: null, error: null });
  const [providers, setProviders] = useState<ProjectProvider[]>([]);
  const [watchUnavailable, setWatchUnavailable] = useState(false);
  const reloadAction = useRef<() => void>(() => {});
  const retryWatchAction = useRef<() => void>(() => {});
  const applied = useRef<LibraryListing | null>(null);
  const listingClient = useRef(client);
  const announcing = useRef(false);
  const reload = useCallback(() => reloadAction.current(), []);
  const retryWatch = useCallback(() => retryWatchAction.current(), []);
  useEffect(() => {
    if (!active) return;
    if (listingClient.current !== client) {
      listingClient.current = client;
      applied.current = null;
    }
    let stopped = false;
    let epoch = 0;
    let watching = false;
    let pending = false;
    let announce = false;
    let flight: Promise<void> | null = null;
    const visible = () => document.visibilityState !== "hidden";
    const refresh = (notify: boolean): Promise<void> => {
      if (stopped || !visible()) return Promise.resolve();
      pending = true;
      announce ||= notify;
      if (flight) return flight;
      const currentEpoch = epoch;
      const isCurrent = () => !stopped && visible() && epoch === currentEpoch;
      const run = async () => {
        while (pending && isCurrent()) {
          pending = false;
          const notifySnapshot = announce;
          announce = false;
          try {
            const first = await client.libraryListing(null);
            if (!isCurrent()) return;
            const previous = applied.current;
            let listing: LibraryListing;
            const changed = !previous || first.generation !== previous.generation;
            if (changed) {
              const result = await readAllPages(client, first, isCurrent);
              if (!result || !isCurrent()) return;
              listing = result;
            } else if (followSyncKey(first) !== followSyncKey(previous)) {
              listing = { ...previous, follows: first.follows };
            } else {
              listing = previous;
            }
            applied.current = listing;
            if (watching) setWatchUnavailable(false);
            setState((old) => {
              if (old.status === "ready" && old.listing === listing && old.error === null) return old;
              return { status: "ready", listing, error: null };
            });
            if (changed && previous && notifySnapshot) {
              announcing.current = true;
              try { announceLibraryChanged({ kind: "snapshot_update" }); }
              finally { announcing.current = false; }
            }
          } catch (cause) {
            if (isCurrent()) setState((old) => ({ status: "error", listing: old.listing, error: errorText(cause, "The Library could not be read.") }));
          }
        }
      };
      const result = run().finally(() => { if (flight === result) flight = null; });
      flight = result;
      return result;
    };
    const fallback = watchFallback(() => refresh(true));
    const subscription = subscribeLocalChanges(client, { kind: "library_listing" }, async (event) => {
      if (event.kind === "unavailable") {
        fallback.start();
        watching = false;
        setWatchUnavailable(true);
        setState((old) => old.listing ? old : { ...old, status: "error", error: "Automatic updates unavailable." });
        return;
      }
      if (event.kind === "ready") { fallback.stop(); watching = true; }
      await refresh(event.kind === "changed");
    }, () => {
      if (!stopped && visible()) {
        fallback.start();
        watching = false;
        setWatchUnavailable(true);
        setState((old) => old.listing ? old : { ...old, status: "error", error: "Automatic updates unavailable." });
      }
    });
    reloadAction.current = () => { void refresh(false); };
    retryWatchAction.current = () => subscription.retry();
    const changed = () => { if (!announcing.current) void refresh(false); };
    const visibility = () => {
      if (!visible()) {
        epoch += 1;
        watching = false;
        pending = false;
        announce = false;
        flight = null;
      }
    };
    window.addEventListener(LIBRARY_CHANGED_EVENT, changed);
    document.addEventListener("visibilitychange", visibility);
    setState((old) => ({ ...old, status: "loading" }));
    return () => {
      stopped = true;
      epoch += 1;
      reloadAction.current = () => {};
      retryWatchAction.current = () => {};
      fallback.close();
      subscription.close();
      window.removeEventListener(LIBRARY_CHANGED_EVENT, changed);
      document.removeEventListener("visibilitychange", visibility);
    };
  }, [active, client]);
  useEffect(() => {
    if (!active) return;
    let current = true;
    // Labels fall back to provider ids when configuration is unavailable.
    client.projectConfiguration().then((configuration) => { if (current) setProviders(configuration.providers); }, () => undefined);
    return () => { current = false; };
  }, [active, client]);
  return { ...state, providers, reload, watchUnavailable, retryWatch };
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

/** Reads live Library and repository selections for one target Space. */
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
  const status = !listing && state.status === "ready" ? "loading" : state.status;
  return { status, listing, error: state.error, received: state.received, reload };
}
