import { useCallback, useEffect, useLayoutEffect, useMemo, useReducer, useRef } from "react";
import type { BrowserViewViewportRequest } from "../../protocol/generated/v1";
import { BrowserPane, type BrowserPaneRecoveryRegistration } from "../browser/BrowserPane";
import type { BrowserSlot, LeafCtx } from "./tabLayoutStore";
import type { Rect } from "./solveLayout";
import { browserClosePending, browserTarget, browserWorkHandoffPending, closeBrowserLeaf, dismissBrowserLeaf, reconnectBrowserLeaf, registerBrowserCloseGuard, retryBrowserOpen, subscribeBrowserLifecycle } from "./browserLifecycle";

export function boundedBrowserViewport(width: number, height: number, devicePixelRatio: number): BrowserViewViewportRequest {
  return {
    css_width: Math.max(1, Math.min(2560, Math.round(Number.isFinite(width) && width > 0 ? width : 800))),
    css_height: Math.max(1, Math.min(1600, Math.round(Number.isFinite(height) && height > 0 ? height : 600))),
    device_pixel_ratio: Math.max(0.1, Math.min(16, Number.isFinite(devicePixelRatio) && devicePixelRatio > 0 ? devicePixelRatio : 1)),
  };
}
export function BrowserLeaf({ ctx, tabId, slot, rect, selected, inputActive, liveInputEnabled = false, onSelect }: { ctx: LeafCtx; tabId: string; slot: BrowserSlot; rect: Rect; selected: boolean; inputActive: boolean; liveInputEnabled?: boolean; onSelect(): void }) {
  const [, redraw] = useReducer((value: number) => value + 1, 0);
  const [actionError, setActionError] = useReducer((_value: string | null, error: string | null) => error, null);
  useEffect(() => subscribeBrowserLifecycle(redraw), []);
  const bodyRef = useRef<HTMLDivElement>(null);
  const selectionRevision = ctx.getState().tabs[tabId]?.selectionRevision;
  useLayoutEffect(() => {
    const body = bodyRef.current;
    if (!selected || !body || body.closest("[data-suppress-attach-focus]")) return;
    if (document.activeElement !== body && body.contains(document.activeElement)) return;
    const active = document.activeElement;
    if (active instanceof HTMLElement && active.closest("[data-pane-header]")?.closest("[data-leaf-id]") === body.closest("[data-leaf-id]")) return;
    const destination = body.querySelector<HTMLElement>(".browser-surface") ?? body.querySelector<HTMLElement>('[aria-label="Page URL"]') ?? body;
    destination.focus({ preventScroll: true });
  }, [selected, slot.status, selectionRevision]);
  const target = useMemo(() => browserTarget(ctx, tabId), [ctx.sessionId, tabId]);
  const dpr = window.devicePixelRatio;
  const viewport = useMemo(() => boundedBrowserViewport(rect.width, rect.height, dpr), [rect.width, rect.height, dpr]);
  const registerGuard = useCallback((registration: BrowserPaneRecoveryRegistration | null) => registerBrowserCloseGuard(ctx, tabId, registration), [ctx.client, ctx.sessionId, tabId]);
  const failedClose = slot.status === "close_failed" || (slot.status === "outcome_unknown" && browserClosePending(ctx, tabId));
  const pending = slot.status === "opening" || slot.status === "closing";
  const run = async (action: () => Promise<void>): Promise<void> => { setActionError(null); try { await action(); } catch (error) { setActionError(error instanceof Error ? error.message : String(error)); } };
  return <div ref={bodyRef} tabIndex={-1} className="browser-leaf" onFocusCapture={onSelect} onPointerDownCapture={onSelect} aria-current={selected ? "true" : undefined}>
    {actionError ? <div className="browser-recovery-strip" role="alert">{actionError}</div> : null}
    {slot.error || pending ? <div className="browser-recovery-strip" role={slot.error ? "alert" : "status"}>
      <span>{slot.error ?? (slot.status === "opening" ? "Starting browser for this tab…" : "Closing browser…")}</span>
      {!pending ? <>
        <button type="button" onClick={() => void run(() => failedClose ? closeBrowserLeaf(ctx, tabId) : retryBrowserOpen(ctx, tabId))}>{failedClose ? "Retry close" : "Retry"}</button>
        <button type="button" onClick={() => failedClose ? dismissBrowserLeaf(ctx, tabId) : void run(() => closeBrowserLeaf(ctx, tabId))}>{failedClose ? "Dismiss" : "Close"}</button>
      </> : null}
    </div> : null}
    {slot.association ? <BrowserPane client={ctx.client} target={target} viewport={viewport} clientId={ctx.clientId}
      inputActive={inputActive} liveInputEnabled={liveInputEnabled && slot.status === "open" && !browserWorkHandoffPending(ctx, tabId)} onInteractionFocus={onSelect}
      onReconnect={() => reconnectBrowserLeaf(ctx, tabId)} onCloseBrowser={() => closeBrowserLeaf(ctx, tabId)} registerCloseGuard={registerGuard}
      onFeedback={(ids, operationId, acknowledgeDuplicateRisk) => ctx.client.sendBrowserFeedback({ scope: { kind: "tab", target }, ids, operation_id: operationId, acknowledge_duplicate_risk: acknowledgeDuplicateRisk, recipient: null })} /> : null}
  </div>;
}
