import { useEffect, useReducer, useRef, useState } from "react";
import type { LeafCtx } from "./tabLayoutStore";
import { dismissBrowserCleanup, refreshBrowserCleanup, retryBrowserCleanup, subscribeBrowserLifecycle } from "./browserLifecycle";

export function BrowserCleanupNotices({ ctx, activeTabId }: { ctx: LeafCtx; activeTabId: string | null }) {
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
  if (!notices.length && !error) return null;
  return <div className="browser-cleanup-notices">
    {error ? <div className="browser-recovery-strip" role="alert"><span>{error}</span><button type="button" disabled={Boolean(busy)} onClick={() => setInspectionEpoch((epoch) => epoch + 1)}>Retry inspection</button></div> : null}
    {notices.map((notice) => <div key={notice.associationKey} className="browser-recovery-strip" role="alert">
      <span>Browser cleanup is incomplete: {notice.reason}</span>
      <button type="button" disabled={Boolean(busy)} onClick={() => void retry(notice.associationKey)}>Retry cleanup</button>
      <button type="button" disabled={Boolean(busy)} onClick={() => dismissBrowserCleanup(ctx, notice.associationKey)}>Dismiss</button>
    </div>)}
  </div>;
}
