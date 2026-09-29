import type { CockpitClient } from "../../client/CockpitClient";
import type { ViewerContext, ViewerSourceSelector } from "../../protocol/generated/v1";
import type { LeafCtx, ViewerSelector } from "./tabLayoutStore";

export type { LeafCtx } from "./tabLayoutStore";

type ViewerKind = "files" | "review";
type ViewerOperation = { tail: Promise<void>; viewer: ViewerContext | null; requestId: string | null };
const operations = new WeakMap<CockpitClient, Map<string, ViewerOperation>>();
const CLIENT_ID_KEY = "cockpit.viewer-client-id";
let windowClientId: string | null = null;

/** A reload in this window reuses its registry scope; other windows stay independent. */
export function getViewerClientId(): string {
  if (windowClientId) return windowClientId;
  try {
    windowClientId = window.sessionStorage.getItem(CLIENT_ID_KEY);
    if (windowClientId) return windowClientId;
  } catch { /* Storage may be unavailable; the in-memory identity remains window-local. */ }
  windowClientId = crypto.randomUUID();
  try { window.sessionStorage.setItem(CLIENT_ID_KEY, windowClientId); } catch { /* See above. */ }
  return windowClientId;
}

function operation(ctx: LeafCtx, tabId: string, kind: ViewerKind): ViewerOperation {
  let registry = operations.get(ctx.client);
  if (!registry) { registry = new Map(); operations.set(ctx.client, registry); }
  const key = JSON.stringify([ctx.sessionId, ctx.serverInstance, ctx.clientId, tabId, kind]);
  let current = registry.get(key);
  if (!current) { current = { tail: Promise.resolve(), viewer: null, requestId: null }; registry.set(key, current); }
  return current;
}

function ownsState(ctx: LeafCtx): boolean {
  const state = ctx.getState();
  return state.sessionId === ctx.sessionId && state.serverInstance === ctx.serverInstance;
}

/** Serialize a slot's opens/releases: backend reuse keeps the same viewer ID. */
function enqueue(current: ViewerOperation, run: () => Promise<void>): Promise<void> {
  const result = current.tail.then(run, run);
  current.tail = result.catch(() => undefined);
  return result;
}

function source(selector: ViewerSelector): ViewerSourceSelector {
  return selector.kind === "review" ? { kind: "review", repository_id: selector.repositoryId } : { kind: selector.kind };
}

export function viewerErrorCode(error: unknown): string | undefined {
  if (!error || typeof error !== "object") return undefined;
  const value = error as { operationCode?: string; code?: string };
  return value.operationCode ?? value.code;
}

export const VIEWER_MISSING_MESSAGE = "This viewer is no longer available. Reopen it to restore access; saved comments and drafts are unchanged.";

export function viewerErrorMessage(error: unknown): string {
  switch (viewerErrorCode(error)) {
    case "viewer_limit": return "Too many open viewers; close viewers in other windows";
    case "viewer_not_found": return VIEWER_MISSING_MESSAGE;
    default: return error instanceof Error ? error.message : String(error);
  }
}

export async function openViewerLeaf(ctx: LeafCtx, tabId: string, kind: ViewerKind, selector: ViewerSelector, dir: "row" | "col", sourcePaneId: string): Promise<void> {
  if (!ownsState(ctx) || !ctx.getState().tabs[tabId]) return;
  const current = operation(ctx, tabId, kind);
  const requestId = crypto.randomUUID();
  current.requestId = requestId;
  ctx.dispatch({ type: "viewer/open-begin", tabId, kind, selector, sourcePaneId, dir, requestId });
  await enqueue(current, async () => {
    if (current.requestId !== requestId || !ownsState(ctx)) return;
    try {
      const context = await ctx.client.viewerOpen(ctx.sessionId, { tab_id: tabId, kind, source_pane_id: sourcePaneId, source: source(selector), client_id: ctx.clientId });
      current.viewer = context;
      if (current.requestId !== requestId) return;
      const slot = ctx.getState().tabs[tabId]?.viewers[kind];
      if (!ownsState(ctx) || !slot || slot.requestId !== requestId) {
        await ctx.client.viewerRelease(ctx.sessionId, context.viewer_id);
        current.viewer = null;
        return;
      }
      ctx.dispatch({ type: "viewer/opened", tabId, kind, context, requestId });
    } catch (error) {
      if (current.requestId === requestId && ownsState(ctx)) ctx.dispatch({ type: "viewer/failed", tabId, kind, error: viewerErrorMessage(error), requestId });
    }
  });
}

export async function closeViewerLeaf(ctx: LeafCtx, tabId: string, kind: ViewerKind): Promise<void> {
  const current = operation(ctx, tabId, kind);
  const context = ownsState(ctx) ? ctx.getState().tabs[tabId]?.viewers[kind]?.context : null;
  if (!current.viewer && context) current.viewer = context;
  current.requestId = null;
  await enqueue(current, async () => {
    try {
      const viewer = current.viewer;
      if (viewer) {
        await ctx.client.viewerRelease(ctx.sessionId, viewer.viewer_id);
        if (current.viewer === viewer) current.viewer = null;
      }
      if (current.requestId === null && ownsState(ctx)) ctx.dispatch({ type: "viewer/closed", tabId, kind });
    } catch (error) {
      if (current.requestId === null && ownsState(ctx)) ctx.dispatch({ type: "viewer/failed", tabId, kind, error: `Could not release viewer: ${viewerErrorMessage(error)}` });
      throw error;
    }
  });
}

/** Retired slots are remembered here even after reconciliation removes their tab. */
export async function releaseViewers(ctx: LeafCtx, tabIds: string[] | "all"): Promise<void> {
  const ids = new Set(tabIds === "all" ? ownsState(ctx) ? Object.keys(ctx.getState().tabs) : [] : tabIds);
  if (tabIds === "all") {
    const registry = operations.get(ctx.client);
    for (const key of registry?.keys() ?? []) {
      const [session, instance, client, tab] = JSON.parse(key) as string[];
      if (session === ctx.sessionId && instance === ctx.serverInstance && client === ctx.clientId) ids.add(tab);
    }
  }
  const releases: Promise<void>[] = [];
  for (const tabId of ids) for (const kind of ["files", "review"] as const) releases.push(closeViewerLeaf(ctx, tabId, kind));
  const results = await Promise.allSettled(releases);
  const failed = results.find((result): result is PromiseRejectedResult => result.status === "rejected");
  if (failed) throw failed.reason;
}
