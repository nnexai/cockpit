import { describe, expect, it, vi } from "vitest";
import type { BrowserAssociation, BrowserCleanupStatus, BrowserRequest, BrowserResponse } from "../../protocol/generated/v1";
import type { CockpitClient } from "../../client/CockpitClient";
import { createSessionLayoutState, layoutReducer, type LeafCtx, type TabLayoutState } from "./tabLayoutStore";
import { leaves } from "./splitTree";
import { applyBrowserCleanupStatus, browserOpenDisabledReason, closeBrowserLeaf, dismissBrowserCleanup, openBrowserLeaf, reconnectBrowserLeaf, retireTabBrowser } from "./browserLifecycle";

function association(tabId: string): BrowserAssociation {
  return { association_key: `browser:${tabId}`, owner_id: "owner", session_id: "session", tab_id: tabId, tab_label: tabId, space_id: "space", space_label: "Space", playwright_session: tabId, working_directory: "/fixture", profile_path: `/fixture/${tabId}`, invocation: "browser", connection: "open", incarnation: "incarnation", opened_tab: null };
}
function fixture(run: (request: BrowserRequest) => Promise<BrowserResponse>) {
  let state = createSessionLayoutState("session", "server");
  for (const tabId of ["a", "b"]) {
    const tab: TabLayoutState = { tabId, spaceId: "space", root: { t: "leaf", id: `terminal:${tabId}`, kind: "terminal", w: 1 }, terminals: { [`terminal:${tabId}`]: `pty:${tabId}` }, selectedLeafId: `terminal:${tabId}`, lastRealLeafId: `terminal:${tabId}`, zoomLeafId: null, viewers: {}, heldMembers: [], heldTerminalIds: {}, bufferedFocus: null, focusedPaneId: `terminal:${tabId}`, revision: 0, selectionRevision: 0 };
    state.tabs[tabId] = tab;
  }
  const browserAction = vi.fn(run);
  const cleanupStatus: BrowserCleanupStatus = { failures: [] };
  const browserCleanupRetry = vi.fn(async (_request: { association_key: string }) => cleanupStatus);
  const client = { browserAction, browserCleanupRetry, browserCleanupStatus: vi.fn(async (): Promise<BrowserCleanupStatus> => cleanupStatus) } as unknown as CockpitClient;
  const ctx: LeafCtx = { client, sessionId: "session", serverInstance: "server", clientId: "window", getState: () => state, dispatch: (action) => { state = layoutReducer(state, action).state; } };
  applyBrowserCleanupStatus(ctx, cleanupStatus);
  return { ctx, browserAction, browserCleanupRetry, state: () => state,
    replaceServer: (serverInstance: string, replacement: BrowserAssociation) => { state = { ...state, serverInstance, tabs: { ...state.tabs, a: { ...state.tabs.a, viewers: { browser: { status: "open", association: replacement, error: null } } } } }; },
    removeTab: (tabId: string) => { state = { ...state, tabs: Object.fromEntries(Object.entries(state.tabs).filter(([id]) => id !== tabId)) }; } };
}
const response = (tabId: string, connection: BrowserResponse["connection"] = "open", cleanup: BrowserResponse["cleanup"] = "none"): BrowserResponse => ({ association: { ...association(tabId), connection }, connection, cleanup, cleanup_reason: cleanup === "failed" ? "Profile is locked" : null, message: connection });

describe("tab browser lifecycle", () => {
  it("opens fresh once per leaf, reconnects without resetting, and closes only its own tab", async () => {
    const f = fixture(async (request) => response(request.target.tab_id!, request.action.kind === "close" ? "closed" : "open"));
    await openBrowserLeaf(f.ctx, "a", "col"); await openBrowserLeaf(f.ctx, "b", "row");
    expect(f.state().tabs.a.viewers.browser?.status).toBe("open");
    expect(leaves(f.state().tabs.a.root).map((leaf) => leaf.id)).toEqual(["terminal:a", "a:browser"]);
    await openBrowserLeaf(f.ctx, "a", "row");
    expect(f.browserAction).toHaveBeenCalledTimes(2);
    await reconnectBrowserLeaf(f.ctx, "a");
    expect(f.browserAction.mock.calls[2][0].action.kind).toBe("open");
    expect(f.browserAction.mock.calls[0][0].action).toEqual({ kind: "open_fresh", url: null });
    await closeBrowserLeaf(f.ctx, "a");
    expect(f.state().tabs.a.viewers.browser).toBeUndefined();
    expect(f.state().tabs.b.viewers.browser?.status).toBe("open");
    expect(f.browserAction.mock.calls.at(-1)?.[0].target).toEqual({ session_id: "session", tab_id: "a", pane_id: null, endpoint_path: null });
  });


  it("inspects an unknown close before any repeat and gates only a stopped tab's incomplete cleanup", async () => {
    let closeCount = 0;
    const f = fixture(async (request) => {
      if (request.action.kind === "close" && ++closeCount === 1) throw new Error("Connection lost during stop");
      return response(request.target.tab_id!, request.action.kind === "close" ? "closed" : "open", request.action.kind === "close" ? "failed" : "none");
    });
    await openBrowserLeaf(f.ctx, "a", "row"); await closeBrowserLeaf(f.ctx, "a");
    expect(f.state().tabs.a.viewers.browser?.status).toBe("outcome_unknown");
    await closeBrowserLeaf(f.ctx, "a");
    expect(f.browserAction.mock.calls.map(([request]) => request.action.kind)).toEqual(["open_fresh", "close", "status", "close"]);
    expect(f.state().tabs.a.viewers.browser).toBeUndefined();
    expect(browserOpenDisabledReason(f.ctx, "a")).toContain("cleanup");
    expect(browserOpenDisabledReason(f.ctx, "b")).toBeNull();
    dismissBrowserCleanup(f.ctx, "browser:a");
    expect(browserOpenDisabledReason(f.ctx, "a")).toBeNull();
  });

  it("retires the outgoing browser receipt immediately after its tab disappears", async () => {
    const f = fixture(async (request) => response(request.target.tab_id!));
    await openBrowserLeaf(f.ctx, "a", "row");
    f.removeTab("a");
    await retireTabBrowser(f.ctx, "a");
    expect(f.browserCleanupRetry).toHaveBeenCalledExactlyOnceWith({ association_key: "browser:a" });
    expect(f.state().tabs.a).toBeUndefined();
    expect(f.state().tabs.b.viewers.browser).toBeUndefined();
  });

  it("retires only the outgoing receipt when a replacement server reuses the same tab ID", async () => {
    const f = fixture(async (request) => response(request.target.tab_id!));
    await openBrowserLeaf(f.ctx, "a", "row");
    const outgoingKey = f.state().tabs.a.viewers.browser!.association!.association_key;
    const replacement = { ...association("a"), association_key: "replacement:a" };
    f.replaceServer("replacement-server", replacement);
    const replacementCtx = { ...f.ctx, serverInstance: "replacement-server" };
    await retireTabBrowser(replacementCtx, "a", outgoingKey, "server");
    expect(f.browserCleanupRetry).toHaveBeenCalledExactlyOnceWith({ association_key: outgoingKey });
    expect(f.browserAction.mock.calls.map(([request]) => request.action.kind)).toEqual(["open_fresh"]);
    expect(f.state().tabs.a.viewers.browser!.association).toEqual(replacement);
  });
});
