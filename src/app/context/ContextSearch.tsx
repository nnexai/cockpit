import { useCallback, useEffect, useId, useMemo, useRef, useState } from "react";
import type {
  ContextInvalidation,
  ContextInvalidationRequest,
  ContextInvalidationResponse,
  ContextKnownRevision,
  ContextSearchRequest,
  ContextSearchResponse,
  ContextSearchResult,
} from "../../protocol/generated/v1";
import { viewerErrorCode } from "../layout/viewerLifecycle";
import type { ContextReader } from "./contextSource";
import { UiIcon } from "../UiIcon";

export type ContextSearchProps = {
  identity: string;
  bindingId: string;
  rootId: string;
  known: readonly ContextKnownRevision[];
  search: (request: ContextSearchRequest, signal: AbortSignal) => Promise<ContextSearchResponse>;
  invalidate: (request: ContextInvalidationRequest, signal: AbortSignal) => Promise<ContextInvalidationResponse>;
  watchFiles: NonNullable<ContextReader["watchFiles"]>;
  onSelect: (result: ContextSearchResult) => void;
  onInvalidate: (invalidations: ContextInvalidation[]) => void;
  onWatchStatus?: (message: string | null, retry: () => void) => void;
  onViewerError?: (error: unknown) => void;
};

function errorText(error: unknown): string {
  if (error instanceof Error && error.message) return error.message;
  if (typeof error === "string" && error) return error;
  return "The Context search could not be completed.";
}

export function ContextSearch({
  identity,
  bindingId,
  rootId,
  known,
  search,
  invalidate,
  watchFiles,
  onSelect,
  onInvalidate,
  onWatchStatus,
  onViewerError,
}: ContextSearchProps) {
  const queryId = useId();
  const [query, setQuery] = useState("");
  const [results, setResults] = useState<ContextSearchResult[]>([]);
  const [truncated, setTruncated] = useState(false);
  const [nextOffset, setNextOffset] = useState<number | null>(null);
  const [searchRevision, setSearchRevision] = useState<string | null>(null);
  const [partialReason, setPartialReason] = useState<string | null>(null);
  const [searching, setSearching] = useState(false);
  const [searchError, setSearchError] = useState<string | null>(null);
  const [checkError, setCheckError] = useState<string | null>(null);
  const searchGenerationRef = useRef(0);
  const checkGenerationRef = useRef(0);
  const searchAbortRef = useRef<AbortController | null>(null);
  const identityRef = useRef(identity);
  const retryWatchRef = useRef<(() => void) | null>(null);
  const onInvalidateRef = useRef(onInvalidate);
  const onViewerErrorRef = useRef(onViewerError);
  onViewerErrorRef.current = onViewerError;
  identityRef.current = identity;
  onInvalidateRef.current = onInvalidate;
  const knownKey = useMemo(
    () => known.map((entry) => `${entry.path}\u0000${entry.revision}`).join("\u0001"),
    [known],
  );
  const knownSnapshot = useMemo(
    () => known.map((entry) => ({ path: entry.path, revision: entry.revision })),
    [knownKey],
  );

  const nextSearchGeneration = useCallback(() => {
    searchGenerationRef.current = searchGenerationRef.current === 0xffff_ffff ? 1 : searchGenerationRef.current + 1;
    return searchGenerationRef.current;
  }, []);
  const nextCheckGeneration = useCallback(() => {
    checkGenerationRef.current = checkGenerationRef.current === 0xffff_ffff ? 1 : checkGenerationRef.current + 1;
    return checkGenerationRef.current;
  }, []);

  const validSearchResponse = useCallback((response: ContextSearchResponse, generation: number, requestQuery: string) => (
    identityRef.current === identity
    && response.binding_id === bindingId
    && response.root_id === rootId
    && response.request_generation === generation
    && response.query === requestQuery
  ), [bindingId, identity, rootId]);

  const runSearch = useCallback(async () => {
    const requestQuery = query.trim();
    if (!requestQuery) return;
    searchAbortRef.current?.abort();
    const controller = new AbortController();
    searchAbortRef.current = controller;
    const generation = nextSearchGeneration();
    setSearching(true);
    setSearchError(null);
    try {
      const response = await search({
        binding_id: bindingId,
        root_id: rootId,
        query: requestQuery,
        request_generation: generation,
        offset: nextOffset ?? undefined,
        revision: searchRevision ?? undefined,
      }, controller.signal);
      if (!controller.signal.aborted && validSearchResponse(response, generation, requestQuery)) {
        setResults((current) => nextOffset === null ? response.results : [...current, ...response.results]);
        setTruncated(response.truncated);
        setNextOffset(response.next_offset ?? null);
        setSearchRevision(response.revision ?? null);
        setPartialReason(response.partial_reason ?? null);
      }
    } catch (error) {
      if (!controller.signal.aborted && identityRef.current === identity && generation === searchGenerationRef.current) {
        setSearchError(errorText(error));
        if (viewerErrorCode(error) === "viewer_not_found") onViewerErrorRef.current?.(error);
      }
    } finally {
      if (!controller.signal.aborted && identityRef.current === identity && generation === searchGenerationRef.current) {
        setSearching(false);
      }
    }
  }, [bindingId, identity, nextOffset, nextSearchGeneration, query, rootId, search, searchRevision, validSearchResponse]);

  useEffect(() => {
    searchAbortRef.current?.abort();
    searchAbortRef.current = null;
    nextSearchGeneration();
    setResults([]);
    setTruncated(false);
    setNextOffset(null);
    setSearchRevision(null);
    setPartialReason(null);
    setSearchError(null);
    setCheckError(null);
    setSearching(false);
  }, [identity, nextSearchGeneration, rootId]);

  useEffect(() => () => {
    searchAbortRef.current?.abort();
    searchAbortRef.current = null;
    nextSearchGeneration();
  }, [nextSearchGeneration]);
  useEffect(() => {
    onWatchStatus?.(checkError, () => retryWatchRef.current?.());
    return () => onWatchStatus?.(null, () => {});
  }, [checkError, onWatchStatus]);

  useEffect(() => {
    if (knownSnapshot.length === 0) return undefined;
    let cancelled = false;
    let activeController: AbortController | null = null;
    let running: Promise<void> | null = null;
    let dirty = false;
    let watching = false;
    const visibilityChanged = () => {
      if (document.visibilityState === "hidden") { dirty = false; activeController?.abort(); }
    };
    document.addEventListener("visibilitychange", visibilityChanged);
    const check = () => {
      dirty = true;
      if (running) return running;
      running = (async () => {
        while (dirty && !cancelled) {
          dirty = false;
          const controller = new AbortController();
          activeController = controller;
          const generation = nextCheckGeneration();
          try {
            const response = await invalidate({ binding_id: bindingId, root_id: rootId,
              request_generation: generation, known: knownSnapshot }, controller.signal);
            if (cancelled || controller.signal.aborted || generation !== checkGenerationRef.current
              || identityRef.current !== identity || response.binding_id !== bindingId
              || response.root_id !== rootId || response.request_generation !== generation) continue;
            setCheckError(response.truncated || response.invalidations.some((item) => item.state === "unavailable")
              ? "Not all visible files could be checked; refresh to retry."
              : watching ? null : "Automatic updates unavailable; displayed data may be stale.");
            if (response.invalidations.length > 0) onInvalidateRef.current(response.invalidations);
          } catch (error) {
            if (!cancelled && !controller.signal.aborted && identityRef.current === identity) {
              setCheckError(errorText(error));
              if (viewerErrorCode(error) === "viewer_not_found") onViewerErrorRef.current?.(error);
            }
          } finally {
            if (activeController === controller) activeController = null;
          }
        }
      })().finally(() => { running = null; });
      return running;
    };
    const subscription = watchFiles(rootId, [...new Set(knownSnapshot.map((entry) => entry.path))], async (event) => {
      if (event.kind === "unavailable") { watching = false; setCheckError("Automatic updates unavailable; displayed data may be stale."); return; }
      if (event.kind === "ready") watching = true;
      await check();
    }, () => {
      watching = false;
      if (!cancelled) setCheckError("Automatic updates unavailable; displayed data may be stale.");
    });
    retryWatchRef.current = () => { watching = false; subscription.retry(); };
    return () => {
      cancelled = true;
      document.removeEventListener("visibilitychange", visibilityChanged);
      retryWatchRef.current = null;
      subscription.close();
      activeController?.abort();
    };
  }, [bindingId, identity, knownSnapshot, nextCheckGeneration, invalidate, rootId, watchFiles]);

  return (
    <section className="context-search" aria-label="Search Context">
      {!onWatchStatus && checkError ? <div><p className="context-search-poll" role="status" aria-live="polite">{checkError}</p><button type="button" onClick={() => retryWatchRef.current?.()}>Retry automatic updates</button></div> : null}
      <details className="viewer-tree-search"><summary><UiIcon name="search" /> Search contents</summary>
      <form className="context-search-form" onSubmit={(event) => { event.preventDefault(); void runSearch(); }}>
        <label htmlFor={queryId}>Search contents</label>
        <input
          id={queryId}
          type="search"
          value={query}
          onChange={(event) => {
            searchAbortRef.current?.abort();
            searchAbortRef.current = null;
            nextSearchGeneration();
            setSearching(false);
            setQuery(event.target.value);
            setResults([]);
            setNextOffset(null);
            setSearchRevision(null);
            setPartialReason(null);
          }}
          maxLength={256}
          placeholder="Find text in context files"
        />
        <button type="submit" disabled={searching || !query.trim()}>{searching ? "Searching…" : nextOffset !== null ? "Load more" : "Search"}</button>
      </form>
      {searchError ? <p className="context-search-error" role="status">{searchError}</p> : null}
      {truncated ? <p className="context-search-truncated" role="status">Search is partial{partialReason ? `: ${partialReason}` : ""}{nextOffset !== null ? "; load more to continue" : ""}.</p> : null}
      {results.length > 0 ? <ol className="context-search-results">{results.map((result) => (
        <li key={`${result.path}\u0000${result.line}\u0000${result.revision}`}>
          <button type="button" onClick={() => onSelect(result)}>
            <code>{result.path}:{result.line}</code>
            <span>{result.excerpt}</span>
          </button>
        </li>
      ))}</ol> : null}
      </details>
    </section>
  );
}
