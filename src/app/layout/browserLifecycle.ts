import type { BrowserAssociation, BrowserCleanupStatus, BrowserResponse, BrowserTarget } from "../../protocol/generated/v1";
import { CockpitClientError, type CockpitClient } from "../../client/CockpitClient";
import type { BrowserPaneRecoveryRegistration } from "../browser/BrowserPane";
import type { BrowserSlot, LeafCtx } from "./tabLayoutStore";

export type { LeafCtx } from "./tabLayoutStore";
type RecoveryEntry = { ctx: LeafCtx; tabId: string; recovery: BrowserPaneRecoveryRegistration; incoming?: BrowserPaneRecoveryRegistration; outgoing: boolean; retired: boolean; retirementAssociationKey?: string | null; error: string | null; settling?: Promise<void> };
type Runtime = { cutover: BrowserCleanupStatus["cutover"] | null; savedTabs: BrowserCleanupStatus["saved_tabs"]; dismissed: Set<string>; recoveries: Map<string, RecoveryEntry>; operations: Map<string, Promise<void>>; generations: Map<string, number>; associations: Map<string, BrowserAssociation>; closing: Set<string> };
const runtimes = new WeakMap<CockpitClient, Runtime>();
const listeners = new Set<() => void>();
const runtime = (ctx: LeafCtx): Runtime => {
  let value = runtimes.get(ctx.client);
  if (!value) { value = { cutover: null, savedTabs: [], dismissed: new Set(), recoveries: new Map(), operations: new Map(), generations: new Map(), associations: new Map(), closing: new Set() }; runtimes.set(ctx.client, value); }
  return value;
};
const keyFor = (ctx: LeafCtx, tabId: string): string => JSON.stringify([ctx.sessionId, ctx.serverInstance, tabId]);
const notify = (): void => { for (const listener of listeners) listener(); };
const reason = (error: unknown): string => error instanceof Error ? error.message : String(error);
export const subscribeBrowserLifecycle = (listener: () => void): (() => void) => { listeners.add(listener); return () => { listeners.delete(listener); }; };
export const browserTarget = (ctx: LeafCtx, tabId: string): BrowserTarget => ({ session_id: ctx.sessionId, tab_id: tabId, pane_id: null, endpoint_path: null });
const slotFor = (ctx: LeafCtx, tabId: string): BrowserSlot | undefined => ctx.getState().sessionId === ctx.sessionId && ctx.getState().serverInstance === ctx.serverInstance ? ctx.getState().tabs[tabId]?.viewers.browser : undefined;
const setSlot = (ctx: LeafCtx, tabId: string, slot: BrowserSlot): void => { if (ctx.getState().sessionId === ctx.sessionId && ctx.getState().serverInstance === ctx.serverInstance && ctx.getState().tabs[tabId]) ctx.dispatch({ type: "browser/state", tabId, slot }); };
const removeLeaf = (ctx: LeafCtx, tabId: string): void => { if (ctx.getState().sessionId === ctx.sessionId && ctx.getState().serverInstance === ctx.serverInstance) ctx.dispatch({ type: "leaf/close-local", tabId, leafId: `${tabId}:browser` }); };
const queue = (ctx: LeafCtx, tabId: string, run: () => Promise<void>): Promise<void> => {
  const rt = runtime(ctx); const key = keyFor(ctx, tabId);
  const next = (rt.operations.get(key) ?? Promise.resolve()).catch(() => undefined).then(run);
  rt.operations.set(key, next);
  void next.finally(() => { if (rt.operations.get(key) === next) rt.operations.delete(key); }).catch(() => undefined);
  return next;
};
function rememberResponse(ctx: LeafCtx, tabId: string, response: BrowserResponse): void {
  const rt = runtime(ctx);
  if (response.association) {
    if (response.association.session_id !== ctx.sessionId || response.association.tab_id !== tabId) throw new Error("Browser response belongs to another tab.");
    rt.associations.set(keyFor(ctx, tabId), response.association);
  }
  if (response.cleanup === "failed" || response.cleanup === "pending") {
    const association = response.association ?? rt.associations.get(keyFor(ctx, tabId));
    if (association && !rt.dismissed.has(association.association_key)) ctx.dispatch({ type: "cleanup/notice", notice: { associationKey: association.association_key, tabId, reason: response.cleanup_reason ?? response.message } });
  }
}
export function applyBrowserCleanupStatus(ctx: LeafCtx, status: BrowserCleanupStatus): void {
  const rt = runtime(ctx); rt.cutover = status.cutover; rt.savedTabs = status.saved_tabs;
  const activeKeys = new Set(status.failures.map((failure) => failure.association_key));
  for (const notice of ctx.getState().cleanupNotices) if (!activeKeys.has(notice.associationKey)) ctx.dispatch({ type: "cleanup/dismiss", associationKey: notice.associationKey });
  for (const failure of status.failures) {
    if (rt.dismissed.has(failure.association_key)) continue;
    if (failure.scope.kind === "tab" && failure.scope.session_id !== ctx.sessionId) continue;
    ctx.dispatch({ type: "cleanup/notice", notice: { associationKey: failure.association_key, tabId: failure.scope.kind === "tab" ? failure.scope.tab_id : null, reason: failure.reason } });
  }
  notify();
}
export function savedTabBrowserWork(ctx: LeafCtx): BrowserCleanupStatus["saved_tabs"] { return runtime(ctx).savedTabs; }
export async function refreshBrowserCleanup(ctx: LeafCtx): Promise<BrowserCleanupStatus> { const status = await ctx.client.browserCleanupStatus(); applyBrowserCleanupStatus(ctx, status); return status; }
export function browserOpenDisabledReason(ctx: LeafCtx, tabId: string): string | null {
  const rt = runtime(ctx);
  if (rt.cutover === null || rt.cutover === "running") return "Stopping previous browser sessions…";
  if (rt.cutover === "failed" && ctx.getState().cleanupNotices.some((notice) => notice.tabId === null)) return "Previous browser cleanup is incomplete.";
  if (ctx.getState().cleanupNotices.some((notice) => notice.tabId === tabId)) return "Browser cleanup is incomplete; retry cleanup or dismiss its notice.";
  const recovery = rt.recoveries.get(keyFor(ctx, tabId));
  if (recovery?.outgoing) return "Retained browser work must be saved or discarded before opening.";
  return null;
}
export function dismissBrowserCleanup(ctx: LeafCtx, associationKey: string): void { runtime(ctx).dismissed.add(associationKey); ctx.dispatch({ type: "cleanup/dismiss", associationKey }); notify(); }
export async function retryBrowserCleanup(ctx: LeafCtx, associationKey?: string): Promise<void> {
  const notices = ctx.getState().cleanupNotices.filter((notice) => !associationKey || notice.associationKey === associationKey);
  for (const notice of notices) {
    runtime(ctx).dismissed.delete(notice.associationKey);
    applyBrowserCleanupStatus(ctx, await ctx.client.browserCleanupRetry({ association_key: notice.associationKey }));
  }
}
export function browserClosePending(ctx: LeafCtx, tabId: string): boolean { return runtime(ctx).closing.has(keyFor(ctx, tabId)); }
export function browserWorkHandoffPending(ctx: LeafCtx, tabId: string): boolean { return Boolean(runtime(ctx).recoveries.get(keyFor(ctx, tabId))?.outgoing); }
export function registerBrowserCloseGuard(ctx: LeafCtx, tabId: string, recovery: BrowserPaneRecoveryRegistration | null): void {
  const rt = runtime(ctx); const key = keyFor(ctx, tabId); const existing = rt.recoveries.get(key);
  if (recovery) {
    // Keep outgoing mutations and the new mounted registration separately until durability is proven.
    if (existing?.outgoing) { existing.incoming = recovery; return; }
    rt.recoveries.set(key, { ctx, tabId, recovery, outgoing: false, retired: false, error: null });
  } else if (existing) {
    if (existing.outgoing && existing.incoming) { existing.incoming = undefined; return; }
    existing.outgoing = true;
    const pending = existing.recovery.guard(); existing.settling = pending;
    void pending.then(() => {
      if (rt.recoveries.get(key) === existing && !existing.retired) {
        if (existing.incoming) rt.recoveries.set(key, { ...existing, recovery: existing.incoming, incoming: undefined, outgoing: false, error: null });
        else rt.recoveries.delete(key);
      }
    }, (error) => { existing.error = reason(error); }).finally(() => { existing.settling = undefined; notify(); });
  }
  notify();
}
export function retainedBrowserWork(ctx: LeafCtx): RecoveryEntry[] { return [...runtime(ctx).recoveries.values()].filter((entry) => Boolean(entry.error)); }
export async function recoverRetainedBrowserWork(ctx: LeafCtx, tabId: string, discard = false): Promise<void> {
  const rt = runtime(ctx); const key = keyFor(ctx, tabId); const entry = rt.recoveries.get(key);
  if (!entry) return;
  try {
    await (discard ? entry.recovery.discard() : entry.recovery.retry());
    await entry.recovery.guard(); entry.error = null;
    if (entry.retired) await retireTabBrowser(entry.ctx, tabId, entry.retirementAssociationKey);
    else if (entry.outgoing) {
      if (entry.incoming) rt.recoveries.set(key, { ...entry, recovery: entry.incoming, incoming: undefined, outgoing: false });
      else rt.recoveries.delete(key);
    }
  } catch (error) { entry.error = reason(error); throw error; }
  finally { notify(); }
}
async function open(ctx: LeafCtx, tabId: string, fresh: boolean, dir: "row" | "col" = "row"): Promise<void> {
  const rt = runtime(ctx);
  if (rt.cutover === null) await refreshBrowserCleanup(ctx);
  const disabled = browserOpenDisabledReason(ctx, tabId); if (disabled) throw new Error(disabled);
  const tab = ctx.getState().tabs[tabId]; if (!tab || Object.keys(tab.terminals).length === 0) throw new Error("No terminal in this tab.");
  const existing = tab.viewers.browser;
  const key = keyFor(ctx, tabId); const generation = (rt.generations.get(key) ?? 0) + 1; rt.generations.set(key, generation);
  rt.closing.delete(key);
  const slot: BrowserSlot = { status: "opening", association: existing?.association ?? null, error: null, requestId: String(generation) };
  ctx.dispatch({ type: "browser/state", tabId, slot, dir, placeBeside: tab.selectedLeafId ?? undefined });
  return queue(ctx, tabId, async () => {
    if (rt.generations.get(key) !== generation) return;
    try {
      // A retry of an unknown outcome must inspect before sending any further mutation.
      if (existing?.status === "outcome_unknown") {
        const inspected = await ctx.client.browserAction({ target: browserTarget(ctx, tabId), action: { kind: "status" } }); rememberResponse(ctx, tabId, inspected);
        if (rt.generations.get(key) !== generation) return;
        if (inspected.connection === "outcome_unknown") { setSlot(ctx, tabId, { ...slot, status: "outcome_unknown", error: inspected.message }); return; }
        if (inspected.connection === "open") { setSlot(ctx, tabId, { ...slot, status: "open", association: inspected.association, error: null }); return; }
      }
      const response = await ctx.client.browserAction({ target: browserTarget(ctx, tabId), action: { kind: fresh ? "open_fresh" : "open", url: null } });
      rememberResponse(ctx, tabId, response);
      if (rt.generations.get(key) !== generation) return;
      if (response.cleanup === "failed" && response.connection !== "open") { removeLeaf(ctx, tabId); return; }
      setSlot(ctx, tabId, { ...slot, association: response.association, status: response.connection === "open" ? "open" : response.connection === "outcome_unknown" ? "outcome_unknown" : "error", error: response.connection === "open" ? null : response.message });
    } catch (error) { if (rt.generations.get(key) === generation) setSlot(ctx, tabId, { ...slot, status: error instanceof CockpitClientError && error.operationCode ? "error" : "outcome_unknown", error: reason(error) }); }
  });
}
export async function openBrowserLeaf(ctx: LeafCtx, tabId: string, dir: "row" | "col"): Promise<void> {
  if (runtime(ctx).cutover === null) await refreshBrowserCleanup(ctx);
  const tab = ctx.getState().tabs[tabId];
  if (tab?.viewers.browser) { ctx.dispatch({ type: "select-leaf", tabId, leafId: `${tabId}:browser` }); return; }
  await open(ctx, tabId, true, dir);
}
export async function reconnectBrowserLeaf(ctx: LeafCtx, tabId: string): Promise<void> { await open(ctx, tabId, false); }
export async function retryBrowserOpen(ctx: LeafCtx, tabId: string): Promise<void> { await open(ctx, tabId, true); }
async function close(ctx: LeafCtx, tabId: string): Promise<void> {
  const rt = runtime(ctx); const key = keyFor(ctx, tabId); const previous = slotFor(ctx, tabId);
  rt.generations.set(key, (rt.generations.get(key) ?? 0) + 1);
  rt.closing.add(key);
  let association = previous?.association ?? rt.associations.get(key) ?? null;
  setSlot(ctx, tabId, { status: "closing", association, error: null });
  return queue(ctx, tabId, async () => {
    association ??= rt.associations.get(key) ?? null;
    const entry = rt.recoveries.get(key);
    try {
      if (entry) await (entry.settling ?? entry.recovery.guard());
    } catch (error) {
      if (entry) entry.error = reason(error);
      setSlot(ctx, tabId, { status: "close_failed", association, error: `Browser close refused: ${reason(error)}` }); notify(); return;
    }
    try {
      if (previous?.status === "close_failed" || previous?.status === "outcome_unknown") {
        const inspected = await ctx.client.browserAction({ target: browserTarget(ctx, tabId), action: { kind: "status" } }); rememberResponse(ctx, tabId, inspected);
        if (inspected.connection === "outcome_unknown") throw new Error(inspected.message);
        if (inspected.connection === "absent" || inspected.connection === "closed") { removeLeaf(ctx, tabId); rt.recoveries.delete(key); rt.closing.delete(key); notify(); return; }
      }
      const response = await ctx.client.browserAction({ target: browserTarget(ctx, tabId), action: { kind: "close" } }); rememberResponse(ctx, tabId, response);
      if (response.connection === "absent" || response.connection === "closed") { removeLeaf(ctx, tabId); rt.recoveries.delete(key); rt.closing.delete(key); notify(); return; }
      setSlot(ctx, tabId, { status: response.connection === "outcome_unknown" ? "outcome_unknown" : "close_failed", association: response.association ?? association, error: response.message });
    } catch (error) {
      setSlot(ctx, tabId, { status: error instanceof CockpitClientError && error.operationCode ? "close_failed" : "outcome_unknown", association, error: reason(error) });
    }
  });
}
export async function closeBrowserLeaf(ctx: LeafCtx, tabId: string): Promise<void> { await close(ctx, tabId); }
export async function retireTabBrowser(ctx: LeafCtx, tabId: string, associationKey?: string | null, serverInstance = ctx.serverInstance): Promise<void> {
  const outgoingCtx = serverInstance === ctx.serverInstance ? ctx : { ...ctx, serverInstance };
  const rt = runtime(outgoingCtx), key = keyFor(outgoingCtx, tabId);
  const entry = rt.recoveries.get(key);
  if (entry) { entry.retired = true; entry.retirementAssociationKey = associationKey ?? rt.associations.get(key)?.association_key ?? null; }
  rt.generations.set(key, (rt.generations.get(key) ?? 0) + 1);
  return queue(outgoingCtx, tabId, async () => {
    // An in-flight old-run Open may supply its receipt after retirement was scheduled.
    const pinnedKey = associationKey ?? rt.associations.get(key)?.association_key ?? null;
    if (entry) entry.retirementAssociationKey = pinnedKey;
    try {
      if (entry) await (entry.settling ?? entry.recovery.guard());
    } catch (error) { if (entry) entry.error = reason(error); notify(); return; }
    // Never resolve a reused tab against the replacement server to discover ownership.
    if (!pinnedKey) { rt.recoveries.delete(key); notify(); return; }
    try {
      const status = await ctx.client.browserCleanupRetry({ association_key: pinnedKey });
      applyBrowserCleanupStatus(ctx, status);
      const failure = status.failures.find((candidate) => candidate.association_key === pinnedKey);
      if (failure) ctx.dispatch({ type: "cleanup/notice", notice: { associationKey: pinnedKey, tabId: serverInstance === ctx.serverInstance ? tabId : null, reason: failure.reason } });
      rt.recoveries.delete(key); rt.associations.delete(key); rt.closing.delete(key);
    } catch (error) {
      ctx.dispatch({ type: "cleanup/notice", notice: { associationKey: pinnedKey, tabId: serverInstance === ctx.serverInstance ? tabId : null, reason: reason(error) } });
    }
    notify();
  });
}
export function dismissBrowserLeaf(ctx: LeafCtx, tabId: string): void { removeLeaf(ctx, tabId); }
