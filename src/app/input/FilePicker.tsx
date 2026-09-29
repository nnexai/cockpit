import { useDeferredValue, useEffect, useMemo, useRef, useState, type KeyboardEvent } from "react";
import { filePathParts, prepareFileCandidates, rankPreparedFileMatches, type FileNavigationCandidate, type FileNavigationMatch, type PreparedFileCandidate } from "./fileNavigation";
import "./fileNavigation.css";

export function FilePicker({ candidates, preparedCandidates, loading = false, incomplete = false, mayBeOutOfDate = false, failed = false, onChoose, onDismiss }: {
  candidates: readonly FileNavigationCandidate[];
  preparedCandidates?: readonly PreparedFileCandidate[];
  loading?: boolean;
  incomplete?: boolean;
  mayBeOutOfDate?: boolean;
  failed?: boolean;
  onChoose: (candidate: FileNavigationCandidate) => void;
  onDismiss: () => void;
}) {
  const pickerRef = useRef<HTMLElement>(null);
  const inputRef = useRef<HTMLInputElement>(null);
  const restoreFocusRef = useRef<HTMLElement | null>(null);
  const [query, setQuery] = useState("");
  const [selection, setSelection] = useState<{ query: string; id: string | null }>({ query: "", id: null });
  const deferredQuery = useDeferredValue(query);
  const prepared = useMemo(() => preparedCandidates ?? prepareFileCandidates(candidates), [candidates, preparedCandidates]);
  const matches = useMemo(() => rankPreparedFileMatches(deferredQuery, prepared), [deferredQuery, prepared]);
  const active = selection.query === deferredQuery ? Math.max(0, matches.findIndex((candidate) => candidate.id === selection.id)) : 0;
  const rankingPending = deferredQuery !== query;
  const activeId = matches[active]?.id ?? null;
  const selectIndex = (index: number) => setSelection({ query: deferredQuery, id: matches[index]?.id ?? null });
  useEffect(() => {
    restoreFocusRef.current = globalThis.document.activeElement instanceof HTMLElement ? globalThis.document.activeElement : null;
    inputRef.current?.focus();
    return () => restoreFocusRef.current?.focus();
  }, []);
  useEffect(() => {
    setSelection((current) => current.query === deferredQuery && current.id === activeId ? current : { query: deferredQuery, id: activeId });
  }, [deferredQuery, activeId]);
  useEffect(() => {
    pickerRef.current?.querySelector<HTMLElement>(`[data-file-picker-result-index="${active}"]`)?.scrollIntoView?.({ block: "nearest" });
  }, [active, activeId]);
  const choose = () => {
    if (rankingPending) return;
    const candidate = matches[active];
    if (candidate) onChoose(candidate);
  };
  const onKeyDown = (event: KeyboardEvent<HTMLElement>) => {
    if (event.nativeEvent.isComposing) return;
    if (event.key === "Escape") { event.preventDefault(); onDismiss(); return; }
    if (event.key === "Enter") { event.preventDefault(); choose(); return; }
    if (event.key === "ArrowDown" || (event.ctrlKey && event.key.toLowerCase() === "n")) {
      event.preventDefault(); selectIndex(Math.min(Math.max(0, matches.length - 1), active + 1)); return;
    }
    if (event.key === "ArrowUp" || (event.ctrlKey && event.key.toLowerCase() === "p")) {
      event.preventDefault(); selectIndex(Math.max(0, active - 1));
    }
    if (event.key !== "Tab") return;
    const focusable = [...event.currentTarget.querySelectorAll<HTMLElement>('input:not([disabled]), button:not([disabled])')];
    if (focusable.length === 0) return;
    const current = focusable.indexOf(globalThis.document.activeElement as HTMLElement);
    if (event.shiftKey && current <= 0) {
      event.preventDefault();
      focusable[focusable.length - 1]?.focus();
    } else if (!event.shiftKey && current === focusable.length - 1) {
      event.preventDefault();
      focusable[0]?.focus();
    }
  };
  return <section className="file-picker" role="dialog" aria-label="Go to file" aria-modal="true" ref={pickerRef} onKeyDown={onKeyDown}>
    <input ref={inputRef} type="search" aria-label="Find file" value={query} onChange={(event) => setQuery(event.target.value)} placeholder="Type to find a file" autoComplete="off" />
    <p className="file-picker-status">{failed ? candidates.length > 0 ? "May be out of date" : "Could not load files" : loading ? candidates.length > 0 ? "Refreshing…" : "Indexing files…" : `${candidates.length} files`}{mayBeOutOfDate && !loading && !failed ? " · may be out of date" : ""}{incomplete ? " · index incomplete" : ""}</p>
    <div role="listbox" aria-label="Matching files" className="file-picker-results">
      {matches.map((candidate, index) => <button key={candidate.id} data-file-picker-result-index={index} type="button" role="option" aria-selected={index === active} className={index === active ? "is-active" : ""} title={candidate.path} disabled={rankingPending} onFocus={() => selectIndex(index)} onMouseMove={() => selectIndex(index)} onClick={() => onChoose(candidate)}><FileLabel candidate={candidate} />{candidate.detail ? <span className="file-picker-detail">{candidate.detail}</span> : null}</button>)}
      {!loading && matches.length === 0 ? <p>No matching files.</p> : null}
    </div>
    <p className="file-picker-help">↑↓ or Ctrl+N/P to choose · Enter to open · Esc to close</p>
  </section>;
}

function highlighted(text: string, start: number, matched: ReadonlySet<number>) {
  return Array.from(text, (character, offset) => matched.has(start + offset) ? <mark key={offset}>{character}</mark> : character);
}

/** File name first; its directories below, trimmed from the root side so the
 * nearest parents stay readable. */
function FileLabel({ candidate }: { candidate: FileNavigationMatch }) {
  const parts = filePathParts(candidate.path);
  const matched = new Set(candidate.matchedIndices);
  const stemLength = Array.from(parts.stem).length;
  const depth = parts.directories.length;
  return <span className="file-picker-label">
    <span className="file-picker-name">{highlighted(parts.stem, parts.nameStart, matched)}<span className="file-picker-extension">{highlighted(parts.extension, parts.nameStart + stemLength, matched)}</span></span>
    {depth > 0 ? <span className="file-picker-path">{parts.directories.map((directory, index) => <span key={directory.start} className="file-picker-segment" style={{ flexShrink: 2 ** (depth - 1 - index) }}>{highlighted(directory.text, directory.start, matched)}</span>)}</span> : null}
  </span>;
}
