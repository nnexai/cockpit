import { useEffect, useId, useRef, useState, type CSSProperties, type RefObject } from "react";
import type { QuotaLimit, QuotaStatusResponse } from "../../protocol/generated/v1";
import { UiIcon } from "../UiIcon";
import { formatAgo } from "../library/libraryState";
import { accessibleLabel, errorText, formatLimitValue, formatReset, providerViews, remainingFraction, summaryView, windowLabel, type ProviderView } from "./limitsModel";
import "./limits.css";

export interface SubscriptionLimitsProps {
  snapshot: QuotaStatusResponse | null;
  link: "loading" | "live" | "offline";
  now: number;
  open: boolean;
  onOpenChange: (open: boolean) => void;
  suspended: boolean;
  commandOpener?: RefObject<HTMLElement | null>;
}

function Meter({ limit, stale }: { limit: QuotaLimit; stale: boolean }) {
  const fraction = remainingFraction(limit);
  if (fraction === null) return null;
  const used = 1 - fraction;
  const tone = used > 0.95 ? "alert" : used > 0.8 ? "warning" : "ok";
  return <span aria-hidden="true" className={`limits-meter limits-tone-${tone}${stale ? " is-stale" : ""}`}>
    <span style={{ "--remaining": `${fraction * 100}%` } as CSSProperties} />
  </span>;
}

function Chip({ view, now }: { view: ProviderView; now: number }) {
  return <span className={`limits-chip limits-level-${view.level}${view.stale ? " is-stale" : ""}`}>
    <span className="limits-name">{view.name}</span>
    {view.limiting ? <><span className="limits-window">{windowLabel(view.limiting.window, view.limiting.tier)}</span><span className="limits-chip-meter"><Meter limit={view.limiting} stale={view.stale} /></span></> : null}
    <span className="limits-value">{view.text}</span>
    {view.stale && view.provider.fetched_at_ms !== null ? <span className="limits-age">{formatAgo(view.provider.fetched_at_ms, now)}</span> : null}
    {view.provider.error ? <UiIcon name="info" /> : null}
  </span>;
}

export function SubscriptionLimits({ snapshot, link, now, open, onOpenChange, suspended, commandOpener }: SubscriptionLimitsProps) {
  const id = useId();
  const rootRef = useRef<HTMLDivElement>(null);
  const triggerRef = useRef<HTMLButtonElement>(null);
  const popupRef = useRef<HTMLDivElement>(null);
  const opener = useRef<HTMLElement | null>(null);
  const onChangeRef = useRef(onOpenChange);
  onChangeRef.current = onOpenChange;
  const [hover, setHover] = useState(false);
  const pinned = open && !suspended;
  const shown = (open || hover) && !suspended;
  useEffect(() => {
    if (!suspended) return;
    setHover(false);
    if (open) onChangeRef.current(false);
  }, [suspended, open]);
  useEffect(() => {
    // Don't retain hover across a pin: closing it stays closed until a new mouse entry.
    if (!pinned) return;
    setHover(false);
    // Commands supplies the focus before its overlay opened; pointer activation records the trigger.
    opener.current ??= commandOpener?.current ?? (document.activeElement instanceof HTMLElement ? document.activeElement : null);
    popupRef.current?.focus({ preventScroll: true });
    return () => { opener.current = null; };
  }, [pinned, commandOpener]);
  useEffect(() => {
    if (!shown) return;
    const outside = (event: PointerEvent) => {
      if (!(event.target instanceof Node) || rootRef.current?.contains(event.target)) return;
      setHover(false);
      if (pinned) onChangeRef.current(false);
    };
    document.addEventListener("pointerdown", outside, true);
    return () => document.removeEventListener("pointerdown", outside, true);
  }, [shown, pinned]);
  useEffect(() => {
    if (!hover || pinned || suspended) return;
    // A preview never owns keyboard focus; terminal Escape still passes through.
    const escape = (event: KeyboardEvent) => {
      if (event.key === "Escape") setHover(false);
    };
    document.addEventListener("keydown", escape, true);
    return () => document.removeEventListener("keydown", escape, true);
  }, [hover, pinned, suspended]);
  const views = providerViews(snapshot, now, link === "offline");
  const visible = views.filter(view => view.provider.state !== "not_signed_in");
  const summary = summaryView(views);
  return <div className={`subscription-limits${shown ? " is-open" : ""}`} ref={rootRef}
    onPointerEnter={event => { if (event.pointerType === "mouse" && !suspended && !pinned) setHover(true); }}
    onPointerLeave={event => { if (event.pointerType === "mouse") setHover(false); }}
    onKeyDown={event => {
      if (!pinned || event.key !== "Escape") return;
      event.preventDefault();
      event.stopPropagation();
      const target = opener.current?.isConnected && !opener.current.closest("[inert]") ? opener.current : triggerRef.current;
      setHover(false);
      onOpenChange(false);
      target?.focus({ preventScroll: true });
    }}>
    <button type="button" ref={triggerRef} className="limits-trigger" aria-label={accessibleLabel(views, now, link === "offline")}
      title="Subscription limits" aria-haspopup="dialog" aria-expanded={shown} aria-controls={shown ? id : undefined}
      disabled={suspended} onClick={() => { setHover(false); opener.current = triggerRef.current; onOpenChange(!pinned); }}>
      <span className="limits-chips" aria-hidden="true">{visible.length ? visible.map(view => <Chip key={view.provider.provider} view={view} now={now} />) : <span>Limits —</span>}</span>
      <span className="limits-summary" aria-hidden="true">{summary ? <><Chip view={summary} now={now} />{visible.length > 1 ? <span className="limits-more">+{visible.length - 1}</span> : null}</> : <span>Limits —</span>}</span>
    </button>
    {shown ? <div id={id} ref={popupRef} role="dialog" tabIndex={-1} aria-label="Subscription limits" className="limits-popup" onBlur={event => {
      if (!(event.relatedTarget instanceof Node) || !rootRef.current?.contains(event.relatedTarget)) {
        setHover(false);
        if (pinned) onOpenChange(false);
      }
    }}>
      {link === "offline" ? <p className="limits-offline">Offline — showing the last values Cockpit received.</p> : null}
      {views.map(view => <section key={view.provider.provider} aria-label={view.name} className="limits-provider">
        <header><h3>{view.name}</h3><span>{view.provider.fetched_at_ms === null ? "" : `${view.stale ? "stale · " : "updated "}${formatAgo(view.provider.fetched_at_ms, now)}`}</span></header>
        {view.provider.accounts.map((account, accountIndex) => <div key={accountIndex} className="limits-account">
          {view.provider.accounts.length > 1 ? <h4>Account {accountIndex + 1} <span>updated {formatAgo(account.fetched_at_ms, now)}</span></h4> : null}
          <ul>{account.limits.map((limit, limitIndex) => <li key={`${limit.id}:${limitIndex}`} className="limits-row">
            <span className="limits-row-label">{windowLabel(limit.window, limit.tier)}{view.limiting === limit ? <span title="Limiting window" aria-label="Limiting window"> •</span> : null}</span>
            <span className="limits-row-meter"><Meter limit={limit} stale={view.stale} /></span>
            <span className="limits-row-detail"><span className="limits-row-value">{formatLimitValue(limit, true)}</span>{formatReset(limit.resets_at_ms, now) ? <span className="limits-reset">{formatReset(limit.resets_at_ms, now)}</span> : null}</span>
          </li>)}</ul>
        </div>)}
        {view.provider.state === "pending" ? <p>Reading limits…</p> : view.provider.state === "not_signed_in" ? <p>Not signed in to {view.provider.provider === "copilot" ? "GitHub CLI" : "OMP"}</p> : view.provider.state === "unsupported" ? <p>Not reported for this account</p> : !view.limiting ? <p>{errorText(view.provider.error, view.provider.provider)}</p> : null}
        {view.provider.error && view.limiting ? <p className="limits-error">Last check failed: {errorText(view.provider.error, view.provider.provider)}. {view.provider.fetched_at_ms === null ? "" : `Showing the values from ${formatAgo(view.provider.fetched_at_ms, now)}.`}</p> : null}
      </section>)}
      <footer>Shared OMP usage cache · Copilot via GitHub CLI · checked about every 5 min</footer>
    </div> : null}
  </div>;
}
