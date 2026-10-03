import { useEffect, useId, useRef, useState } from "react";
import type { WidgetSummary } from "../../protocol/generated/v1";

export type WidgetChipProps = {
  widget: WidgetSummary;
  width: number;
  live: boolean;
  onGoToAgent(): void;
  location?: string;
};


export function WidgetChip({ widget, width, live, onGoToAgent, location }: WidgetChipProps) {
  const [open, setOpen] = useState(false);
  const id = useId();
  const root = useRef<HTMLDivElement>(null);
  const trigger = useRef<HTMLButtonElement>(null);
  const popover = useRef<HTMLDivElement>(null);
  const source = widget.source;
  const agent = source?.agent_label || "pane";
  const presentation = widget.presentation === "choices" ? "choices" : "HTML";
  const origin = !source ? "CLI" : source.status === "closed" ? "pane closed" : source.status === "restarted" ? `${agent} restarted` : agent;
  const showGlyph = source?.status === "present";
  const label = `${origin} · ${presentation}`;
  const canGo = source?.status === "present";

  useEffect(() => {
    if (!open) return;
    popover.current?.focus({ preventScroll: true });
    const outside = (event: PointerEvent) => {
      if (event.target instanceof Node && !root.current?.contains(event.target)) setOpen(false);
    };
    document.addEventListener("pointerdown", outside);
    return () => document.removeEventListener("pointerdown", outside);
  }, [open]);

  return <div ref={root} className="widget-chip-wrap" onBlur={(event) => {
    if (event.relatedTarget instanceof Node && !event.currentTarget.contains(event.relatedTarget)) setOpen(false);
  }} onKeyDown={(event) => {
    if (!open || event.key !== "Escape") return;
    event.preventDefault();
    event.stopPropagation();
    setOpen(false);
    trigger.current?.focus({ preventScroll: true });
  }}>
    <button ref={trigger} type="button" className="widget-chip"
      aria-label={`${label}, details`} title={`${label}, details`} aria-haspopup="dialog" aria-expanded={open}
      aria-controls={open ? id : undefined} onClick={() => setOpen(!open)}>
      {width <= 340 ? <span aria-hidden="true">◔</span> : <>{showGlyph ? <span aria-hidden="true">◔ </span> : null}{label}</>}
    </button>
    {open ? <div ref={popover} id={id} className="widget-chip-popover" style={{ maxWidth: Math.max(96, width - 64) }} role="dialog" aria-modal="false"
      aria-labelledby={`${id}-title`} tabIndex={-1} onPointerDown={(event) => event.stopPropagation()}>
      <header><strong id={`${id}-title`}>{!source ? "CLI, not in a Herdr pane" : `${origin} · pane ${source.pane_id}`}</strong>
        {canGo ? <button type="button" disabled={!live} aria-describedby={!live ? `${id}-offline` : undefined}
          title={`Moves Herdr focus to ${agent}'s terminal`} onClick={onGoToAgent}>Go to agent</button> : null}
      </header>
      {!live ? <p id={`${id}-offline`} className="widget-source-note">Agent status unavailable: Herdr isn't live.</p>
        : source?.status === "unknown" ? <p className="widget-source-note">Agent status unavailable.</p> : null}
      <dl className="widget-facts">
        <dt>Page</dt><dd>{widget.presentation === "choices"
          ? "Choices drawn by Cockpit from the agent's list. No agent page runs."
          : "Agent-authored HTML. Trusted scripts run in the widget page."}</dd>
        <dt>Where</dt><dd>{location || `Space ${widget.space_id} · Tab ${widget.key.tab_id}`} · id {widget.key.id} · revision {widget.revision}<br />Cockpit-owned, local to this tab
          {widget.arrival === "cross_source" && source ? <><br />Published from Space {source.space_id} · Tab {source.tab_id}</> : null}</dd>
        <dt>Selection</dt><dd>{widget.selection ? <>
          Stored {new Date(widget.selection.at_ms).toLocaleTimeString()} · revision {widget.selection.revision}
          {widget.selection.read_at_ms !== null ? <> · read via CLI {new Date(widget.selection.read_at_ms).toLocaleTimeString()}</> : " · not read via CLI yet"}
        </> : "None yet"}</dd>
      </dl>
      <details><summary>Technical details</summary>
        <dl className="widget-facts widget-technical-facts">
          <dt>Widget</dt><dd>{widget.key.id} · revision {widget.revision}</dd>
          <dt>SHA-256</dt><dd className="widget-hash">{widget.content.sha256}</dd>
          <dt>Size</dt><dd>{widget.content.bytes.toLocaleString()} bytes / {widget.kind === "choices" ? "64 KiB" : "1 MiB"} limit</dd>
          <dt>Input</dt><dd>{widget.content.from}{widget.content.from === "file" && widget.content.name ? ` · ${widget.content.name.split(/[\\/]/).at(-1)}` : ""}</dd>
          <dt>Source</dt><dd>{source ? <>Pane {source.pane_id} · terminal {source.terminal_id}<br />Space {source.space_id} · Tab {source.tab_id}<br />Status: {source.status}<br />Fingerprint: {source.fingerprint_prefix || "none"}</> : "none"}</dd>
          <dt>Target</dt><dd>Session {widget.key.session_id} · Space {widget.space_id} · Tab {widget.key.tab_id}<br />Resolved from: {widget.resolved_from}</dd>
          <dt>Renderer</dt><dd>{widget.presentation === "choices" ? "Native choices; no agent HTML or scripts" : "Trusted HTML iframe; agent scripts enabled"}</dd>
        </dl>
        {widget.warnings.length ? <section className="widget-warnings"><strong>Warnings</strong><ul>{widget.warnings.map((warning, index) => <li key={index}>{warning}</li>)}</ul></section> : null}
      </details>
    </div> : null}
  </div>;
}
