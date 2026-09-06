import { useEffect, useMemo, useRef, useState, type KeyboardEvent } from "react";
import { rankFileMatches, type FileNavigationCandidate } from "./fileNavigation";
import "./fileNavigation.css";

export function FilePicker({ candidates, loading = false, incomplete = false, onChoose, onDismiss }: {
  candidates: readonly FileNavigationCandidate[];
  loading?: boolean;
  incomplete?: boolean;
  onChoose: (candidate: FileNavigationCandidate) => void;
  onDismiss: () => void;
}) {
  const pickerRef = useRef<HTMLElement>(null);
  const inputRef = useRef<HTMLInputElement>(null);
  const restoreFocusRef = useRef<HTMLElement | null>(null);
  const [query, setQuery] = useState("");
  const [active, setActive] = useState(0);
  const matches = useMemo(() => rankFileMatches(query, candidates).slice(0, 100), [candidates, query]);
  useEffect(() => {
    restoreFocusRef.current = globalThis.document.activeElement instanceof HTMLElement ? globalThis.document.activeElement : null;
    inputRef.current?.focus();
    return () => restoreFocusRef.current?.focus();
  }, []);
  useEffect(() => { setActive(0); }, [query, candidates]);
  useEffect(() => {
    pickerRef.current?.querySelector<HTMLElement>(`[data-file-picker-result-index="${active}"]`)?.scrollIntoView?.({ block: "nearest" });
  }, [active, matches]);
  const choose = () => {
    const candidate = matches[active];
    if (candidate) onChoose(candidate);
  };
  const onKeyDown = (event: KeyboardEvent<HTMLElement>) => {
    if (event.nativeEvent.isComposing) return;
    if (event.key === "Escape") { event.preventDefault(); onDismiss(); return; }
    if (event.key === "Enter") { event.preventDefault(); choose(); return; }
    if (event.key === "ArrowDown" || (event.ctrlKey && event.key.toLowerCase() === "n")) {
      event.preventDefault(); setActive((current) => Math.min(Math.max(0, matches.length - 1), current + 1)); return;
    }
    if (event.key === "ArrowUp" || (event.ctrlKey && event.key.toLowerCase() === "p")) {
      event.preventDefault(); setActive((current) => Math.max(0, current - 1));
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
    <p className="file-picker-status">{loading ? "Indexing files…" : `${candidates.length} files`}{incomplete ? " · index incomplete" : ""}</p>
    <div role="listbox" aria-label="Matching files" className="file-picker-results">
      {matches.map((candidate, index) => <button key={candidate.id} data-file-picker-result-index={index} type="button" role="option" aria-selected={index === active} className={index === active ? "is-active" : ""} onFocus={() => setActive(index)} onMouseMove={() => setActive(index)} onClick={() => onChoose(candidate)}><code>{candidate.path}</code>{candidate.detail ? <span>{candidate.detail}</span> : null}</button>)}
      {!loading && matches.length === 0 ? <p>No matching files.</p> : null}
    </div>
    <p className="file-picker-help">↑↓ or Ctrl+N/P to choose · Enter to open · Esc to close</p>
  </section>;
}
