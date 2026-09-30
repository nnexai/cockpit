import { useEffect, useReducer, useRef, useState, type ReactNode } from "react";
import type { LeafCtx } from "./tabLayoutStore";
import { dismissBrowserCleanup, refreshBrowserCleanup, retryBrowserCleanup, subscribeBrowserLifecycle } from "./browserLifecycle";
import { ErrorSlot } from "../ErrorSlot";

export function BrowserCleanupNotices({ ctx, activeTabId, fallback }: { ctx: LeafCtx; activeTabId: string | null; fallback?: ReactNode }) {
  const currentCtx = useRef(ctx); currentCtx.current = ctx;
  const [, redraw] = useReducer((value: number) => value + 1, 0);
  const [error, setError] = useState<string | null>(null);
  const [busy, setBusy] = useState<string | null>(null);
  const [inspectionEpoch, setInspectionEpoch] = useState(0);
  useEffect(() => subscribeBrowserLifecycle(redraw), []);
  useEffect(() => {
    let disposed = false;
    void refreshBrowserCleanup(currentCtx.current).then(() => {
      if (!disposed) setError(null);
    }, (cause) => {
      if (!disposed) setError(`Could not inspect browser cleanup: ${cause instanceof Error ? cause.message : String(cause)}`);
    });
    return () => { disposed = true; };
  }, [ctx.client, ctx.sessionId, ctx.serverInstance, inspectionEpoch]);
  const retry = async (associationKey: string): Promise<void> => {
    setBusy(associationKey); setError(null);
    try { await retryBrowserCleanup(currentCtx.current, associationKey); }
    catch (cause) { setError(cause instanceof Error ? cause.message : String(cause)); }
    finally { setBusy(null); redraw(); }
  };
  const notices = ctx.getState().cleanupNotices.filter((notice) => notice.tabId === activeTabId || notice.tabId === null || !ctx.getState().tabs[notice.tabId]);
  const notice = notices[0];
  if (!notice && !error) return fallback ?? <ErrorSlot placement="pane" />;
  return <ErrorSlot placement="pane"
    message={error ?? (notice ? `Browser cleanup is incomplete: ${notice.reason}${notices.length > 1 ? ` · ${notices.length - 1} more problems` : ""}` : null)}
    actions={error ? <>
      <button type="button" disabled={Boolean(busy)} onClick={() => setInspectionEpoch((epoch) => epoch + 1)}>Retry inspection</button>
      <button type="button" disabled={Boolean(busy)} onClick={() => setError(null)}>Dismiss</button>
    </> : notice ? <>
      <button type="button" disabled={Boolean(busy)} onClick={() => void retry(notice.associationKey)}>Retry cleanup</button>
      <button type="button" disabled={Boolean(busy)} onClick={() => dismissBrowserCleanup(ctx, notice.associationKey)}>Dismiss</button>
    </> : null} />;
}
