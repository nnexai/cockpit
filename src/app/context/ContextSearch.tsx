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

const POLL_INTERVAL_MS = 3_000;

export type ContextSearchProps = {
  identity: string;
  bindingId: string;
  rootId: string;
  known: readonly ContextKnownRevision[];
  search: (request: ContextSearchRequest, signal: AbortSignal) => Promise<ContextSearchResponse>;
  poll: (request: ContextInvalidationRequest, signal: AbortSignal) => Promise<ContextInvalidationResponse>;
  onSelect: (result: ContextSearchResult) => void;
  onInvalidate: (invalidations: ContextInvalidation[]) => void;
  disabled?: boolean;
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
  poll,
  onSelect,
  onInvalidate,
  disabled = false,
}: ContextSearchProps) {
  const queryId = useId();
  const [query, setQuery] = useState("");
  const [results, setResults] = useState<ContextSearchResult[]>([]);
  const [truncated, setTruncated] = useState(false);
  const [searching, setSearching] = useState(false);
  const [searchError, setSearchError] = useState<string | null>(null);
  const [pollError, setPollError] = useState<string | null>(null);
  const searchGenerationRef = useRef(0);
  const pollGenerationRef = useRef(0);
  const searchAbortRef = useRef<AbortController | null>(null);
  const identityRef = useRef(identity);
  const onInvalidateRef = useRef(onInvalidate);
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
  const nextPollGeneration = useCallback(() => {
    pollGenerationRef.current = pollGenerationRef.current === 0xffff_ffff ? 1 : pollGenerationRef.current + 1;
    return pollGenerationRef.current;
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
    if (!requestQuery || disabled) return;
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
      }, controller.signal);
      if (!controller.signal.aborted && validSearchResponse(response, generation, requestQuery)) {
        setResults(response.results);
        setTruncated(response.truncated);
      }
    } catch (error) {
      if (!controller.signal.aborted && identityRef.current === identity && generation === searchGenerationRef.current) {
        setSearchError(errorText(error));
      }
    } finally {
      if (!controller.signal.aborted && identityRef.current === identity && generation === searchGenerationRef.current) {
        setSearching(false);
      }
    }
  }, [bindingId, disabled, identity, nextSearchGeneration, query, rootId, search, validSearchResponse]);

  useEffect(() => {
    searchAbortRef.current?.abort();
    searchAbortRef.current = null;
    nextSearchGeneration();
    setResults([]);
    setTruncated(false);
    setSearchError(null);
    setPollError(null);
    setSearching(false);
  }, [identity, nextSearchGeneration, rootId]);

  useEffect(() => () => {
    searchAbortRef.current?.abort();
    searchAbortRef.current = null;
    nextSearchGeneration();
  }, [nextSearchGeneration]);

  useEffect(() => {
    if (disabled || knownSnapshot.length === 0) return undefined;
    let cancelled = false;
    let timeout: number | undefined;
    let activeController: AbortController | null = null;
    const runPoll = async () => {
      const controller = new AbortController();
      activeController = controller;
      const generation = nextPollGeneration();
      try {
        const response = await poll({
          binding_id: bindingId,
          root_id: rootId,
          request_generation: generation,
          known: knownSnapshot,
        }, controller.signal);
        if (cancelled || activeController !== controller || controller.signal.aborted
          || generation !== pollGenerationRef.current || identityRef.current !== identity
          || response.binding_id !== bindingId || response.root_id !== rootId
          || response.request_generation !== generation) return;
        setPollError(null);
        if (response.invalidations.length > 0) onInvalidateRef.current(response.invalidations);
      } catch (error) {
        if (!cancelled && activeController === controller && !controller.signal.aborted
          && generation === pollGenerationRef.current && identityRef.current === identity) {
          setPollError(errorText(error));
        }
      } finally {
        if (activeController === controller) activeController = null;
        if (!cancelled && identityRef.current === identity) {
          timeout = window.setTimeout(() => { void runPoll(); }, POLL_INTERVAL_MS);
        }
      }
    };
    void runPoll();
    return () => {
      cancelled = true;
      if (timeout !== undefined) window.clearTimeout(timeout);
      activeController?.abort();
    };
  }, [bindingId, disabled, identity, knownSnapshot, nextPollGeneration, poll, rootId]);

  return (
    <section className="context-search" aria-label="Search Context">
      <form className="context-search-form" onSubmit={(event) => { event.preventDefault(); void runSearch(); }}>
        <label htmlFor={queryId}>Search companion</label>
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
          }}
          maxLength={256}
          disabled={disabled}
          placeholder="Find text in companion files"
        />
        <button type="submit" disabled={disabled || searching || !query.trim()}>{searching ? "Searching…" : "Search"}</button>
      </form>
      {searchError ? <p className="context-search-error" role="status">{searchError}</p> : null}
      {pollError ? <p className="context-search-poll" role="status">Visible-file refresh: {pollError}</p> : null}
      {truncated ? <p className="context-search-truncated" role="status">Search stopped at its bounded result, file, or time limit.</p> : null}
      {results.length > 0 ? <ol className="context-search-results">{results.map((result) => (
        <li key={`${result.path}\u0000${result.line}\u0000${result.revision}`}>
          <button type="button" onClick={() => onSelect(result)}>
            <code>{result.path}:{result.line}</code>
            <span>{result.excerpt}</span>
          </button>
        </li>
      ))}</ol> : null}
    </section>
  );
}
