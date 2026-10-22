import { useEffect, useId, useRef, useState } from "react";
import { UiIcon } from "../UiIcon";
import { rankFuzzyMatches } from "../input/fileNavigation";
import { trapModalTab, useModalFocus } from "../input/modal";
import { StateGlyph, spaceGlyphShape } from "../sidebar/StateGlyph";
import { switchSearchText, type SwitchRow, type SwitchTarget } from "./quickSwitch";

export function QuickSwitcher({ rows, hasSession, loading, live, onChoose, onDismiss }: {
  rows: SwitchRow[];
  hasSession: boolean;
  loading: boolean;
  live: boolean;
  onChoose(target: SwitchTarget): void;
  onDismiss(): void;
}) {
  const ref = useModalFocus<HTMLElement>(onDismiss);
  const inputRef = useRef<HTMLInputElement>(null);
  const activeRowRef = useRef<HTMLButtonElement>(null);
  const listId = useId();
  const [query, setQuery] = useState("");
  const [selection, setSelection] = useState<{ query: string; key: string | null }>({ query: "", key: null });
  const matches = rankFuzzyMatches(query, rows, switchSearchText);
  const active = selection.query === query ? Math.max(0, matches.findIndex(row => row.key === selection.key)) : 0;
  const activeKey = matches[active]?.key ?? null;
  const selectIndex = (index: number) => setSelection({ query, key: matches[index]?.key ?? null });
  useEffect(() => {
    setSelection(current => current.query === query && current.key === activeKey ? current : { query, key: activeKey });
  }, [query, activeKey]);
  useEffect(() => { activeRowRef.current?.scrollIntoView?.({ block: "nearest" }); }, [activeKey, query]);
  const empty = !hasSession ? "No session selected." : loading ? "Loading Spaces and agents…"
    : rows.length === 0 ? "No Spaces or agents in this session." : `No Spaces or agents match “${query}”.`;
  return <div className="overlay-scrim" role="presentation" onPointerDown={event => { if (event.target === event.currentTarget) onDismiss(); }}>
    <section ref={ref} className="command-overlay quick-switcher" role="dialog" aria-modal="true" aria-label="Go to Space or agent" onKeyDown={event => {
      if (event.nativeEvent.isComposing) { if (event.key === "Enter" || event.key === "Escape") event.preventDefault(); return; }
      const step = event.key === "ArrowDown" || (event.ctrlKey && !event.altKey && !event.metaKey && event.key.toLowerCase() === "n") ? 1
        : event.key === "ArrowUp" || (event.ctrlKey && !event.altKey && !event.metaKey && event.key.toLowerCase() === "p") ? -1 : 0;
      if (step) {
        event.preventDefault();
        inputRef.current?.focus();
        selectIndex(matches.length === 0 ? 0 : (active + step + matches.length) % matches.length);
        return;
      }
      if (event.key === "Enter" && event.target === inputRef.current) {
        event.preventDefault();
        const row = matches[active];
        if (row && !row.disabledReason) onChoose({ kind: row.kind, id: row.id });
        return;
      }
      trapModalTab(event, ref.current);
    }}>
      <div className="command-search-box"><UiIcon name="search" /><input ref={inputRef} className="command-search" role="combobox" aria-label="Find a Space or agent" aria-expanded="true" aria-controls={listId} aria-activedescendant={activeKey ? `${listId}-${activeKey}` : undefined} aria-autocomplete="list" autoComplete="off" placeholder="Go to Space or agent…" value={query} onChange={event => setQuery(event.target.value)} /></div>
      {hasSession && !loading ? <div className={`switcher-status${live ? "" : " is-warning"}`} role="status">{!live ? "Herdr is not live; choosing is unavailable" : query.trim() ? `${matches.length} of ${rows.length}` : `${rows.length} Spaces and agents`}</div> : null}
      <div id={listId} className="command-list" role="listbox" aria-label="Spaces and agents">
        {matches.length === 0 ? <p className="command-empty" role="status">{empty}</p> : matches.map((row, index) => {
          const kind = row.kind === "space" ? "Space" : "Agent";
          const matched = new Set(row.matchedIndices);
          const metaStart = Array.from(row.primary).length + 1;
          return <button ref={index === active ? activeRowRef : null} id={`${listId}-${row.key}`} key={row.key} type="button" role="option" aria-selected={index === active} aria-disabled={Boolean(row.disabledReason)} aria-label={`${row.primary}, ${kind}, ${row.meta}${row.current ? ", current" : ""}${row.disabledReason ? `, unavailable: ${row.disabledReason}` : ""}`} className={`switcher-row${index === active ? " is-active" : ""}`} title={row.title} disabled={Boolean(row.disabledReason)} onFocus={() => selectIndex(index)} onMouseMove={() => { if (index !== active) selectIndex(index); }} onClick={() => { if (!row.disabledReason) onChoose({ kind: row.kind, id: row.id }); }}>
            <StateGlyph shape={row.kind === "space" ? spaceGlyphShape(row.status) : row.status.shape} />
            <span className="switcher-text"><span className="switcher-primary">{Array.from(row.primary, (character, offset) => matched.has(offset) ? <mark key={offset}>{character}</mark> : character)}</span><span className="switcher-meta">{Array.from(row.meta, (character, offset) => matched.has(metaStart + offset) ? <mark key={offset}>{character}</mark> : character)}</span>{row.disabledReason ? <span className="switcher-meta">{row.disabledReason}</span> : null}</span>
            {row.current ? <span className="switcher-current">Current</span> : null}<span className="switcher-kind">{kind}</span>
          </button>;
        })}
      </div>
      <footer className="command-footer">↑↓ or Ctrl+N/P choose · Enter go · Esc close</footer>
    </section>
  </div>;
}
