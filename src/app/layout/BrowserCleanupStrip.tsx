import { useEffect, useReducer, useRef, useState } from "react";
import type { BrowserLegacyArchive } from "../../protocol/generated/v1";
import { SavedBrowserWork } from "../browser/SavedBrowserWork";
import type { LeafCtx } from "./tabLayoutStore";
import { dismissBrowserCleanup, recoverRetainedBrowserWork, refreshBrowserCleanup, retainedBrowserWork, retryBrowserCleanup, savedTabBrowserWork, subscribeBrowserLifecycle } from "./browserLifecycle";

export function BrowserCleanupStrip({ ctx, activeTabId }: { ctx: LeafCtx; activeTabId: string | null }) {
  const currentCtx = useRef(ctx); currentCtx.current = ctx;
  const [, redraw] = useReducer((value: number) => value + 1, 0);
  const [cutoverRunning, setCutoverRunning] = useState(false);
  const [archives, setArchives] = useState<BrowserLegacyArchive[]>([]);
  const [error, setError] = useState<string | null>(null);
  const [busy, setBusy] = useState<string | null>(null);
  const [inspectionEpoch, setInspectionEpoch] = useState(0);
  useEffect(() => subscribeBrowserLifecycle(redraw), []);
  useEffect(() => {
    let disposed = false; let timer: number | undefined;
    const load = async (): Promise<void> => {
      try {
        const context = currentCtx.current;
        const [status, legacy] = await Promise.all([refreshBrowserCleanup(context), context.client.browserLegacyList()]);
        if (disposed) return;
        setCutoverRunning(status.cutover === "running"); setArchives(legacy.archives); setError(null);
        if (status.cutover === "running") timer = window.setTimeout(() => void load(), 1000);
      } catch (cause) { if (!disposed) setError(`Could not inspect browser cleanup: ${cause instanceof Error ? cause.message : String(cause)}`); }
    };
    void load();
    return () => { disposed = true; window.clearTimeout(timer); };
  }, [ctx.client, ctx.sessionId, inspectionEpoch]);
  const act = async (key: string, run: () => Promise<void>): Promise<void> => {
    setBusy(key); setError(null);
    try { await run(); }
    catch (cause) { setError(cause instanceof Error ? cause.message : String(cause)); }
    finally { setBusy(null); redraw(); }
  };
  const notices = ctx.getState().cleanupNotices.filter((notice) => notice.tabId === activeTabId || notice.tabId === null || !ctx.getState().tabs[notice.tabId]);
  const retained = retainedBrowserWork(ctx);
  const savedTabs = savedTabBrowserWork(ctx).filter((saved) => (saved.saved_capture_count > 0 || saved.draft_count > 0 || saved.pending_capture) && (saved.session_id !== ctx.sessionId || ctx.getState().tabs[saved.tab_id]?.viewers.browser?.association?.association_key !== saved.association_key));
  if (!cutoverRunning && !notices.length && !retained.length && !archives.length && !savedTabs.length && !error) return null;
  return <div className="browser-cleanup-strip">
    {cutoverRunning ? <div className="browser-recovery-strip" role="status">Stopping previous browser sessions…</div> : null}
    {error ? <div className="browser-recovery-strip" role="alert"><span>{error}</span><button type="button" disabled={Boolean(busy)} onClick={() => setInspectionEpoch((epoch) => epoch + 1)}>Retry inspection</button></div> : null}
    {notices.map((notice) => <div key={notice.associationKey} className="browser-recovery-strip" role="alert">
      <span>Browser cleanup is incomplete: {notice.reason}</span>
      <button type="button" disabled={Boolean(busy)} onClick={() => void act(notice.associationKey, () => retryBrowserCleanup(ctx, notice.associationKey))}>Retry cleanup</button>
      <button type="button" disabled={Boolean(busy)} onClick={() => dismissBrowserCleanup(ctx, notice.associationKey)}>Dismiss</button>
    </div>)}
    {retained.map((entry) => <div key={`${entry.ctx.sessionId}:${entry.ctx.serverInstance}:${entry.tabId}`} className="browser-recovery-strip" role="alert">
      <span>Retained browser work in session {entry.ctx.sessionId}, tab {entry.tabId}: {entry.error}. {entry.recovery.describe()}</span>
      <button type="button" disabled={Boolean(busy)} onClick={() => void act(`recover:${entry.tabId}`, () => recoverRetainedBrowserWork(entry.ctx, entry.tabId))}>Retry retained work</button>
      <button type="button" disabled={Boolean(busy)} onClick={() => void act(`discard:${entry.tabId}`, () => recoverRetainedBrowserWork(entry.ctx, entry.tabId, true))}>Discard retained changes</button>
    </div>)}
    {savedTabs.length ? <details className="browser-legacy-archives"><summary>Saved browser work · {savedTabs.length} {savedTabs.length === 1 ? "tab" : "tabs"}</summary>
      {savedTabs.map((saved) => <section key={saved.association_key} aria-label={`Saved browser work: Tab ${saved.tab_label}`}>
        <h3>Saved browser work: Tab {saved.tab_label}</h3>
        <p>Original tab {saved.tab_id} · Space {saved.space_label} ({saved.space_id}) · Session {saved.session_id} · {saved.saved_capture_count} saved captures · {saved.draft_count} drafts{saved.pending_capture ? " · capture pending" : ""}</p>
        <SavedBrowserWork client={ctx.client} scope={{ kind: "saved_tab", association_key: saved.association_key }} sessionId={ctx.sessionId} />
      </section>)}
    </details> : null}
    {archives.length ? <details className="browser-legacy-archives"><summary>Saved before tabs · {archives.length} {archives.length === 1 ? "Space" : "Spaces"}</summary>
      {archives.map((archive) => {
        const manifest = archive.candidates.filter((candidate) => candidate.state === "pending");
        const results = archive.candidates.filter((candidate) => candidate.state !== "pending");
        return <section key={archive.association_key} aria-label={`Saved before tabs: Space ${archive.space_label}`}>
          <h3>Saved before tabs: Space {archive.space_label}</h3>
          <p>Original Space {archive.space_id} · Session {archive.session_id} · Archived {archive.archived_at} · {archive.saved_capture_count} saved captures · {archive.draft_count} drafts{archive.pending_capture ? " · capture pending" : ""}</p>
          {!archive.session_stopped ? <p role="alert">The previous browser session has not been confirmed stopped. Retry cleanup before removing its artifacts.</p> : null}
          <details><summary>Saved browser work</summary><SavedBrowserWork key={archive.association_key} client={ctx.client} scope={{ kind: "legacy_archive", association_key: archive.association_key }} sessionId={ctx.sessionId} /></details>
          <details><summary>Review {manifest.length} items</summary>
            <p>Deletes the old browser's cookies, logins and site data. Cockpit cannot prove these items are unchanged since the old version created them; only the items listed, exactly as inspected now, are removed.</p>
            {manifest.length ? <ul>{manifest.map((candidate) => <li key={candidate.path}>
              <code>{candidate.path}</code> · {candidate.kind === "directory" ? "folder" : "file"} · device {candidate.dev}, inode {candidate.inode} · {candidate.entry_count} entries · {candidate.total_bytes} bytes · inspected {candidate.captured_at}
            </li>)}</ul> : <p>No unreviewed candidate items.</p>}
            {manifest.length ? <div>
              <button type="button" disabled={Boolean(busy) || !archive.session_stopped} onClick={() => void act(archive.association_key, async () => {
                // The request is exactly the manifest painted above, never a fresh scan or a broader path set.
                const result = await ctx.client.browserLegacyRemove({ association_key: archive.association_key, candidates: manifest }); setArchives(result.archives); await refreshBrowserCleanup(ctx);
              })}>Remove these items</button>
              <button type="button" disabled={Boolean(busy)} onClick={() => void act(archive.association_key, async () => { const result = await ctx.client.browserLegacyKeep({ association_key: archive.association_key }); setArchives(result.archives); await refreshBrowserCleanup(ctx); })}>Keep</button>
            </div> : null}
            {results.length ? <ul aria-label="Reviewed item results">{results.map((candidate) => <li key={candidate.path}><code>{candidate.path}</code> · {candidate.state === "changed" ? "Changed and preserved" : candidate.state === "removed" ? "Removed" : "Kept"}</li>)}</ul> : null}
            {archive.not_candidates.length ? <div><p>Not eligible for automatic removal; preserved:</p><ul>{archive.not_candidates.map((path) => <li key={path}><code>{path}</code></li>)}</ul></div> : null}
          </details>
        </section>;
      })}
    </details> : null}
  </div>;
}
