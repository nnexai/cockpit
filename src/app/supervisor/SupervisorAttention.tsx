import { useEffect, useId, useLayoutEffect, useState, type KeyboardEvent, type ReactNode } from "react";
import { StateGlyph, type GlyphShape } from "../sidebar/StateGlyph";
import { useRovingList } from "../sidebar/useRovingList";
import { reportAge } from "./SupervisorActions";
import { TIER_LABEL, type AttentionTier } from "./attention";

export type QueueAction = { key: string; label: string; onActivate(invoker: HTMLElement): void; disabled?: boolean; ariaDisabled?: boolean; reason?: string; primary?: boolean; consequence?: string };
export type QueueRowView = { id: string; tier: AttentionTier; title: string; since: string | null; body?: ReactNode; actions: QueueAction[]; showIn: QueueAction | null };
export type SupervisorSummaryProps = {
  rootLabel: string; stateLabel: string; stateGlyph: GlyphShape; observedLine: ReactNode; banner: ReactNode;
  counts: Readonly<Record<AttentionTier, number>>; queueMode: "inline" | "overlay";
  onCounter(tier: AttentionTier, invoker: HTMLElement): void;
  shortcut: QueueAction | null;
};
export type AttentionQueueProps = {
  rows: readonly QueueRowView[]; expandedId: string | null; onExpand(id: string | null): void;
  capPx: number | null; variant: "inline" | "overlay";
  focusTier: AttentionTier | null; onFocusedTier(): void; onClose?: () => void;
};
const TIERS: readonly AttentionTier[] = ["decide", "recover", "notice"];
const GLYPHS: Record<AttentionTier, GlyphShape> = { decide: "blocked", recover: "unknown", notice: "idle" };
const COUNTER_WORDS: Record<AttentionTier, string> = { decide: "needs you", recover: "recover", notice: "notice" };

function Action({ action, className = "" }: { action: QueueAction; className?: string }) {
  const id = useId();
  return <div className={`supervisor-queue-action ${className}${action.primary ? " is-primary" : ""}`}>
    <button type="button" className={action.primary ? "supervisor-primary" : undefined} disabled={action.ariaDisabled ? false : action.disabled} aria-disabled={action.ariaDisabled ? !!action.disabled : undefined} aria-describedby={action.reason || action.consequence ? id : undefined} onClick={event => { if (!action.disabled) action.onActivate(event.currentTarget); }}>{action.label}</button>
    {action.reason || action.consequence ? <p className="supervisor-queue-action-note supervisor-muted" id={id}>{action.reason}{action.reason && action.consequence ? " · " : null}{action.consequence}</p> : null}
  </div>;
}

export function SupervisorSummary({ rootLabel, stateLabel, stateGlyph, observedLine, banner, counts, queueMode, onCounter, shortcut }: SupervisorSummaryProps) {
  const total = counts.decide + counts.recover + counts.notice;
  return <section className="supervisor-summary" aria-label="Supervisor status" data-queue-mode={queueMode}>
    <div className="supervisor-summary-line">
      <span className="supervisor-summary-state"><StateGlyph shape={stateGlyph} /><strong>{rootLabel}</strong><span>{stateLabel}</span></span>
      <span className="supervisor-summary-observed">{observedLine}</span>
      <div className="supervisor-summary-counts" aria-live="polite" aria-atomic="true">
        {TIERS.filter(tier => counts[tier] > 0).map(tier => <button key={tier} type="button" className={`supervisor-summary-counter is-${tier}`} onClick={event => onCounter(tier, event.currentTarget)}><StateGlyph shape={GLYPHS[tier]} />{counts[tier]} {COUNTER_WORDS[tier]}</button>)}
      </div>
      {total === 0 && shortcut ? <Action action={shortcut} className="supervisor-summary-shortcut" /> : null}
    </div>
    {banner ? <div className="supervisor-summary-banner" role="status">{banner}</div> : null}
  </section>;
}

export function AttentionQueue({ rows, expandedId, onExpand, capPx, variant, focusTier, onFocusedTier, onClose }: AttentionQueueProps) {
  const bodyPrefix = useId();
  const [more, setMore] = useState(0);
  const [fittedCap, setFittedCap] = useState<number | null>(null);
  const roving = useRovingList({ rowIds: rows.map(row => row.id), selectedId: expandedId });
  useEffect(() => {
    if (focusTier === null) return;
    const row = rows.find(candidate => candidate.tier === focusTier);
    if (row) roving.focusRow(row.id);
    onFocusedTier();
  }, [focusTier, rows, roving.focusRow, onFocusedTier]);
  useLayoutEffect(() => {
    const scroller = roving.listRef.current;
    if (!scroller) return;
    const measure = () => {
      if (variant !== "inline" || scroller.clientHeight === 0) { setMore(0); setFittedCap(null); return; }
      const viewportTop = scroller.getBoundingClientRect().top + scroller.clientTop;
      const rowElements = [...scroller.querySelectorAll<HTMLElement>("[data-queue-row]")];
      const bottoms = rowElements.map(row => row.getBoundingClientRect().bottom - viewportTop);
      const completeBoundaries = capPx === null ? [] : bottoms.filter(bottom => bottom > 0 && bottom <= capPx);
      // End at a whole row when one fits. An oversized expanded row keeps the
      // supplied cap and all its content accessible through ordinary scrolling.
      const nextCap = capPx === null ? null : completeBoundaries.length ? Math.max(...completeBoundaries) : capPx;
      setFittedCap(nextCap);
      const visibleHeight = Math.min(scroller.clientHeight, nextCap ?? scroller.clientHeight);
      setMore(bottoms.filter(bottom => bottom > visibleHeight + 1).length);
    };
    measure();
    scroller.addEventListener("scroll", measure, { passive: true });
    window.addEventListener("resize", measure);
    const observer = typeof ResizeObserver === "undefined" ? null : new ResizeObserver(measure);
    observer?.observe(scroller);
    for (const row of scroller.querySelectorAll<HTMLElement>("[data-queue-row]")) observer?.observe(row);
    return () => { scroller.removeEventListener("scroll", measure); window.removeEventListener("resize", measure); observer?.disconnect(); };
  }, [rows, expandedId, capPx, variant, fittedCap, roving.listRef]);
  const onKeyDown = (event: KeyboardEvent<HTMLDivElement>) => {
    const target = event.target as HTMLElement;
    if (event.defaultPrevented || event.nativeEvent.isComposing || target.closest("input,textarea,select,[contenteditable=true]")) return;
    if (event.key === "Escape") {
      const row = target.closest<HTMLElement>("[data-queue-row]");
      if (row?.dataset.queueRow === expandedId && expandedId !== null) {
        event.preventDefault(); event.stopPropagation();
        roving.focusRow(expandedId);
        onExpand(null);
      }
      return; // Collapsed Escape belongs to the View, never the sidebar hook.
    }
    if (!event.ctrlKey && !event.altKey && !event.metaKey && target.dataset.rowId !== undefined && (event.key === "Enter" || event.key === " ")) {
      event.preventDefault();
      if (!event.repeat) onExpand(expandedId === target.dataset.rowId ? null : target.dataset.rowId);
      return;
    }
    roving.listProps.onKeyDown(event);
  };
  return <section className={`supervisor-queue is-${variant}`} aria-label="Attention" data-variant={variant}>
    {variant === "overlay" ? <header className="supervisor-queue-header"><h2>Attention</h2><button type="button" aria-label="Close attention" onClick={onClose}>Close</button></header> : null}
    <div ref={roving.listRef} className="supervisor-queue-scroll" style={variant === "inline" && capPx !== null ? { maxHeight: Math.min(capPx, fittedCap ?? capPx), overflowY: "auto" } : undefined} onFocus={roving.listProps.onFocus} onBlur={roving.listProps.onBlur} onKeyDown={onKeyDown}>
      {rows.map(row => {
        const expanded = expandedId === row.id;
        const bodyId = `${bodyPrefix}-${encodeURIComponent(row.id)}`;
        const actions = [...row.actions].sort((a, b) => Number(!!b.primary) - Number(!!a.primary));
        const primary = actions.find(action => action.primary) ?? actions[0];
        return <div key={row.id} className={`supervisor-queue-row is-${row.tier}${expanded ? " is-expanded" : ""}`} data-queue-row={row.id}>
          <button type="button" className="supervisor-queue-summary" data-row-id={row.id} tabIndex={roving.tabIndexFor(row.id)} aria-expanded={expanded} aria-controls={expanded ? bodyId : undefined} onClick={() => onExpand(expanded ? null : row.id)}>
            <span className={`supervisor-queue-tier is-${row.tier}`}><StateGlyph shape={GLYPHS[row.tier]} />{TIER_LABEL[row.tier]}</span>
            <span className="supervisor-queue-title" title={row.title}>{row.title}</span>
            {primary ? <span className="supervisor-queue-primary-label">{primary.label}</span> : null}
            {row.since ? <time className="supervisor-queue-age" dateTime={row.since} title={new Date(row.since).toLocaleString()}>{reportAge({ at: row.since })}</time> : null}
          </button>
          {expanded ? <div className="supervisor-queue-body" id={bodyId}>{row.body}<div className="supervisor-queue-actions">{actions.map(action => <Action key={action.key} action={action} />)}{row.showIn ? <Action action={row.showIn} className="supervisor-queue-show-in" /> : null}</div></div> : null}
        </div>;
      })}
    </div>
    {variant === "inline" && more > 0 ? <div className="supervisor-queue-more">{more} more</div> : null}
  </section>;
}
