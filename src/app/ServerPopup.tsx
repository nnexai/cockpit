import { useCallback, useLayoutEffect, useRef, useState, type CSSProperties } from "react";
import type { CockpitClient } from "../client/CockpitClient";
import type { HerdrPopup } from "../protocol/generated/v1";
import { TerminalPane } from "./TerminalPane";
import { trapModalTab } from "./input/modal";

type Props = { client: CockpitClient; sessionId: string; popup: HerdrPopup; live: boolean; error: string | null; focusEpoch: number; terminalMouseInput: boolean; onReconnect: () => void };
type PopupBounds = { left: number; top: number; width: number; height: number };

function popupBounds(root: HTMLElement | null): PopupBounds {
  const area = root?.closest(".workbench")?.querySelector<HTMLElement>(".workarea-content")
    ?? document.querySelector<HTMLElement>(".workbench .workarea-content");
  const rect = area?.getBoundingClientRect();
  return rect && rect.width > 0 && rect.height > 0
    ? { left: rect.left, top: rect.top, width: rect.width, height: rect.height }
    : { left: 0, top: 0, width: window.innerWidth, height: window.innerHeight };
}

export function ServerPopup({ client, sessionId, popup, live, error, focusEpoch, terminalMouseInput, onReconnect }: Props) {
  const rootRef = useRef<HTMLElement>(null);
  // Size the terminal from the work area on its first mount; fitting it to the
  // whole window first leaves its PTY grid larger than the visible popup.
  const [bounds, setBounds] = useState(() => popupBounds(null));
  const [cell, setCell] = useState({ width: 8, height: 16 });
  const [terminalState, setTerminalState] = useState({ ready: false, error: null as string | null });
  const [attempt, setAttempt] = useState(0);
  const onCellGeometry = useCallback((geometry: { cell_width_px: number; cell_height_px: number }) => {
    if (geometry.cell_width_px > 0 && geometry.cell_height_px > 0) setCell(current => current.width === geometry.cell_width_px && current.height === geometry.cell_height_px ? current : { width: geometry.cell_width_px, height: geometry.cell_height_px });
  }, []);
  useLayoutEffect(() => {
    const opener = document.activeElement instanceof HTMLElement ? document.activeElement : null;
    rootRef.current?.focus({ preventScroll: true });
    return () => {
      requestAnimationFrame(() => {
        if (document.querySelector("[data-server-modal]")) return;
        const target = opener?.isConnected && !opener.closest("[inert]") ? opener : document.querySelector<HTMLElement>('.tab-button[aria-selected="true"]:not(:disabled), .drawer-toggle, .tab-icon-button[aria-controls="cockpit-sidebar"]');
        target?.focus({ preventScroll: true });
      });
    };
  }, []);
  useLayoutEffect(() => {
    const area = rootRef.current?.closest(".workbench")?.querySelector<HTMLElement>(".workarea-content")
      ?? document.querySelector<HTMLElement>(".workbench .workarea-content");
    const measure = () => {
      const next = popupBounds(rootRef.current);
      setBounds(current => current.left === next.left && current.top === next.top && current.width === next.width && current.height === next.height ? current : next);
    };
    measure();
    const observer = typeof ResizeObserver === "undefined" ? null : new ResizeObserver(measure);
    if (area) observer?.observe(area);
    window.addEventListener("resize", measure);
    return () => { observer?.disconnect(); window.removeEventListener("resize", measure); };
  }, []);
  const size = (hint: HerdrPopup["width"], available: number, metric: number, minimum: number) => Math.min(available, Math.max(minimum * metric, hint?.kind === "cells" ? hint.value * metric : available * (hint?.kind === "percent" ? hint.value / 100 : 0.5)));
  const width = size(popup.width, bounds.width, cell.width, 6);
  const height = size(popup.height, bounds.height, cell.height, 4);
  const style: CSSProperties = { left: bounds.left + (bounds.width - width) / 2, top: bounds.top + (bounds.height - height) / 2, width, height };
  const inputLive = live && terminalState.ready && !terminalState.error;
  const message = !live ? `Disconnected from Herdr. The popup is still open; reconnect to continue.${error ? ` ${error}` : ""}` : !terminalState.ready && !terminalState.error ? "Connecting to popup…" : null;
  return <div className="server-popup-scrim" data-server-modal onPointerDown={event => {
    if (event.target !== event.currentTarget) return;
    if (inputLive) rootRef.current?.querySelector<HTMLTextAreaElement>(".xterm-helper-textarea")?.focus({ preventScroll: true });
    else rootRef.current?.focus({ preventScroll: true });
  }} onKeyDown={event => {
    if (!inputLive) {
      if (event.key === "Escape") event.preventDefault();
      if (event.key === "Tab" && !rootRef.current?.querySelector("button:not(:disabled)")) { event.preventDefault(); rootRef.current?.focus({ preventScroll: true }); }
      else trapModalTab(event, rootRef.current);
    }
    event.stopPropagation();
  }} onKeyUp={event => event.stopPropagation()} onKeyPress={event => event.stopPropagation()}>
    <section ref={rootRef} className={`server-popup${!live ? " is-disconnected" : ""}`} role="dialog" aria-modal="true" aria-labelledby="server-popup-title" tabIndex={-1} style={style}>
      <header className="pane-header"><strong id="server-popup-title">{popup.title || "Popup"}</strong><span className="server-popup-status" role="status">{!live ? "Disconnected — still open in Herdr" : terminalState.error ? "Attach failed — still open in Herdr" : !terminalState.ready ? "Connecting" : ""}</span><span className="sr-only">Keyboard input goes to this popup. Escape and Enter are handled by its program.</span></header>
      <div className="server-popup-body">
        <TerminalPane key={attempt} client={client} request={{ session_id: sessionId, pane_id: popup.terminal_id, target_kind: "popup" }} selected presented controlAllowed={inputLive} controlPending={false} focusEpoch={focusEpoch} focusToken={0} terminalMouseInput={terminalMouseInput} deferAttachment={!live} accessibleLabel={`Popup terminal: ${popup.title || "Popup"}`} onStateChange={setTerminalState} onCellGeometry={onCellGeometry} onClosed={onReconnect} onResync={onReconnect} />
        {message && (!live || !terminalState.error) ? <div className="server-popup-message" role={!live ? "alert" : "status"}><span>{message}</span>{!live ? <button type="button" className="recovery-button" onClick={() => { setAttempt(value => value + 1); onReconnect(); }}>Retry connection</button> : null}</div> : null}
      </div>
    </section>
  </div>;
}
