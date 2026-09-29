import { describe, expect, it } from "vitest";
import type { BrowserAssociation, ViewerContext } from "../../protocol/generated/v1";
import { leaves } from "./splitTree";
import { reconcileSnapshot, runtimeSource, type LayoutSnapshot } from "./reconcile";
import { createSessionLayoutState, layoutReducer, type LayoutAction, type SessionLayoutState } from "./tabLayoutStore";

function snapshot(ids = ["p1", "p2"], focused = "p1"): LayoutSnapshot {
  return { session_id: "session", server_instance: "server", focused_space_id: "space", focused_tab_id: "tab", focused_pane_id: focused,
    tabs: [{ id: "tab", space_id: "space", focused_pane_id: focused }],
    panes: ids.map(id => ({ id, terminal_id: `term-${id}`, tab_id: "tab", space_id: "space" })) };
}
function initial(ids = ["p1", "p2"]): SessionLayoutState {
  return reconcileSnapshot(createSessionLayoutState("session"), snapshot(ids)).state;
}
function apply(state: SessionLayoutState, action: LayoutAction): SessionLayoutState { return layoutReducer(state, action).state; }
function files(state: SessionLayoutState): SessionLayoutState {
  return apply(state, { type: "viewer/open-begin", tabId: "tab", kind: "files", sourcePaneId: "p1", selector: { kind: "files_folder" }, dir: "row" });
}
const created = { pane_id: "p3", terminal_id: "term-p3", tab_id: "tab", space_id: "space" };
function begin(state: SessionLayoutState): SessionLayoutState {
  return apply(state, { type: "creation/begin", creation: { token: 7, tabId: "tab", placeBeside: "tab:files", dir: "col", sourcePaneId: "p1" } });
}

describe("tab layouts follow membership, not geometry or routine focus", () => {
  it("seeds in stable id order and keeps the grid after snapshot enumeration changes", () => {
    const seed = reconcileSnapshot(createSessionLayoutState("session"), snapshot(["p10", "p2", "p1"], "p2")).state;
    expect(leaves(seed.tabs.tab.root).map(leaf => leaf.id)).toEqual(["p1", "p2", "p10"]);
    expect(seed.tabs.tab.selectedLeafId).toBe("p2");
    const reordered = reconcileSnapshot(seed, snapshot(["p2", "p1", "p10"], "p2")).state;
    expect(reordered.tabs.tab.root).toBe(seed.tabs.tab.root);
  });

  it("keeps a selected viewer on unchanged focus and a superseded exact echo", () => {
    let state = files(initial());
    state = reconcileSnapshot(state, snapshot()).state;
    expect(state.tabs.tab.selectedLeafId).toBe("tab:files");
    state = apply(state, { type: "select-leaf", tabId: "tab", leafId: "p2" });
    state = apply(state, { type: "focus/register", token: 8, kind: "pane", targetId: "p2", tabId: "tab" });
    state = apply(state, { type: "select-leaf", tabId: "tab", leafId: "tab:files" });
    const echoed = reconcileSnapshot(state, snapshot(["p1", "p2"], "p2"), [{ token: 8, kind: "pane", targetId: "p2" }]);
    expect(echoed.state.tabs.tab.selectedLeafId).toBe("tab:files");
    expect(echoed.effects).toContainEqual({ type: "consume-echo", token: 8 });
  });

  it("keeps a tab's remembered viewer on its exact tab echo but follows external Space and tab changes", () => {
    const base = snapshot();
    base.tabs.push({ id: "other-tab", space_id: "other-space", focused_pane_id: "p9" });
    base.panes.push({ id: "p9", terminal_id: "term9", tab_id: "other-tab", space_id: "other-space" });
    let state = reconcileSnapshot(createSessionLayoutState("session"), base).state;
    state = apply(state, { type: "viewer/open-begin", tabId: "other-tab", kind: "files", sourcePaneId: "p9", selector: { kind: "files_folder" }, dir: "row" });
    state = apply(state, { type: "activate-tab", tabId: "other-tab" });
    state = apply(state, { type: "focus/register", token: 10, kind: "tab", targetId: "other-tab", tabId: "other-tab" });
    const focused = { ...base, focused_space_id: "other-space", focused_tab_id: "other-tab", focused_pane_id: "p9" };
    const echoed = reconcileSnapshot(state, focused, [{ token: 10, kind: "tab", targetId: "other-tab" }]);
    expect(echoed.state.tabs["other-tab"].selectedLeafId).toBe("other-tab:files");
    expect(echoed.state.activeSpaceId).toBe("other-space");
    const external = reconcileSnapshot(state, focused, [{ token: 10, kind: "space", targetId: "space" }]);
    expect(external.state.tabs["other-tab"].selectedLeafId).toBe("p9");
    expect(external.state.activeTabId).toBe("other-tab");
    expect(external.state.activeSpaceId).toBe("other-space");
  });

  it("follows a nonmatching request target and restores zoom even across reconnect", () => {
    let state = files(initial());
    state = apply(state, { type: "zoom-toggle", tabId: "tab", leafId: "tab:files" });
    const stale = layoutReducer(state, { type: "snapshot", snapshot: snapshot([], "p2"), sync: "disconnected" });
    expect(stale.state).toBe(state); expect(stale.effects).toEqual([]);
    const external = reconcileSnapshot(stale.state, snapshot(["p1", "p2"], "p2"), [{ token: 9, kind: "pane", targetId: "p99" }]);
    expect(external.state.tabs.tab.selectedLeafId).toBe("p2");
    expect(external.state.tabs.tab.zoomLeafId).toBeNull();
    expect(external.effects).toContainEqual({ type: "cancel-transient", reason: "external-focus" });
  });

  it("inserts external panes at the right edge and counts zoom-hidden viewers", () => {
    let state = files(initial());
    state = apply(state, { type: "zoom-toggle", tabId: "tab", leafId: "tab:files" });
    const updated = reconcileSnapshot(state, snapshot(["p1", "p2", "p4"])).state;
    expect(updated.tabs.tab.zoomLeafId).toBe("tab:files");
    expect(leaves(updated.tabs.tab.root).at(-1)).toMatchObject({ id: "p4", w: .25 });
  });

  it("uses last-real, confirmed Herdr focus and in-order terminals as runtime fallback", () => {
    let state = files(initial());
    expect(runtimeSource(state.tabs.tab, "p2")).toBe("p2");
    expect(runtimeSource(state.tabs.tab, "tab:files")).toBe("p1");
    const tab = { ...state.tabs.tab, lastRealLeafId: "removed", focusedPaneId: "p2" };
    expect(runtimeSource(tab, "tab:files")).toBe("p2");
    expect(runtimeSource({ ...tab, focusedPaneId: "missing" }, "tab:files")).toBe("p1");
    expect(runtimeSource({ ...tab, root: null, terminals: {}, focusedPaneId: null }, null)).toBeNull();
  });

  it("retires viewers only on confirmed last-real loss and releases their live context", () => {
    let state = files(initial(["p1"]));
    state = apply(state, { type: "viewer/opened", tabId: "tab", kind: "files", context: { viewer_id: "viewer" } as ViewerContext });
    state = apply(state, { type: "browser/state", tabId: "tab", slot: { status: "opening", association: null, error: null } });
    const stale = layoutReducer(state, { type: "snapshot", snapshot: snapshot([]), sync: "stale" });
    expect(stale.state.tabs.tab).toBe(state.tabs.tab);
    const retired = reconcileSnapshot(state, snapshot([]));
    expect(retired.state.tabs.tab).toBeUndefined();
    expect(retired.effects).toContainEqual({ type: "viewer-release", tabId: "tab", kind: "files", viewerId: "viewer" });
    expect(retired.effects).toContainEqual({ type: "browser-retire", tabId: "tab", associationKey: null, serverInstance: "server" });
  });

  it("pins the outgoing browser identity before a server restart can reuse a tab id", () => {
    let state = initial(["p1"]);
    state = apply(state, { type: "browser/state", tabId: "tab",
      slot: { status: "opening", association: null, error: null, requestId: "browser-open" } });
    state = apply(state, { type: "browser/state", tabId: "tab",
      slot: { status: "open", association: { association_key: "outgoing-key" } as BrowserAssociation, error: null, requestId: "browser-open" } });
    const restarted = reconcileSnapshot(state, { ...snapshot(), server_instance: "new-server" });
    expect(restarted.effects).toContainEqual({ type: "browser-retire", tabId: "tab",
      associationKey: "outgoing-key", serverInstance: "server" });
    expect(restarted.state.tabs.tab.viewers.browser).toBeUndefined();
  });

  it("rebuilds on server identity change and reinserts reused pane ids at the external edge", () => {
    const state = files(initial());
    const changed = snapshot(); changed.panes[0].terminal_id = "replacement";
    const reused = reconcileSnapshot(state, changed).state;
    expect(reused.tabs.tab.terminals.p1).toBe("replacement");
    expect(leaves(reused.tabs.tab.root).at(-1)?.id).toBe("p1");
    const restarted = reconcileSnapshot(state, { ...snapshot(), server_instance: "new-server" }).state;
    expect(restarted.serverInstance).toBe("new-server");
    expect(leaves(restarted.tabs.tab.root).map(leaf => leaf.id)).toEqual(["p1", "p2"]);
    expect(restarted.tabs.tab.viewers).toEqual({});
  });

  it("preserves a tab moved between Spaces but treats cross-tab terminal moves as external", () => {
    let state = files(initial());
    const movedTab = snapshot(); movedTab.tabs[0].space_id = "other";
    movedTab.panes.forEach(pane => pane.space_id = "other"); movedTab.focused_space_id = "other";
    const moved = reconcileSnapshot(state, movedTab).state;
    expect(moved.tabs.tab.spaceId).toBe("other");
    expect(moved.tabs.tab.root).toBe(state.tabs.tab.root);
    const crossTab = snapshot(["p1"]);
    crossTab.tabs.push({ id: "dest", space_id: "space", focused_pane_id: "p9" });
    crossTab.panes.push({ id: "p9", terminal_id: "term-p9", tab_id: "dest", space_id: "space" });
    state = reconcileSnapshot(state, crossTab).state;
    crossTab.panes.push({ id: "p2", terminal_id: "term-p2", tab_id: "dest", space_id: "space" });
    const destination = reconcileSnapshot(state, crossTab).state;
    expect(leaves(destination.tabs.dest.root).map(leaf => leaf.id)).toEqual(["p9", "p2"]);
  });
});

describe("creation identity with snapshots outrunning the receipt", () => {
  it("holds concurrent members then places the receipt beside its viewer and others externally", () => {
    let state = begin(files(initial()));
    state = reconcileSnapshot(state, snapshot(["p1", "p2", "p3", "p4"], "p3")).state;
    expect(state.tabs.tab.heldMembers).toEqual(["p3", "p4"]);
    expect(leaves(state.tabs.tab.root).map(leaf => leaf.id)).toEqual(["p1", "tab:files", "p2"]);
    const settled = layoutReducer(state, { type: "creation/settled", token: 7, created }).state;
    expect(settled.tabs.tab.selectedLeafId).toBe("p3");
    expect(settled.tabs.tab.lastRealLeafId).toBe("p3");
    expect(leaves(settled.tabs.tab.root).map(leaf => leaf.id)).toEqual(["p1", "tab:files", "p3", "p2", "p4"]);
    expect(leaves(settled.tabs.tab.root).at(-1)?.w).toBeCloseTo(.2);
    expect(settled.tabs.tab.heldMembers).toEqual([]);
  });

  it("classifies buffered concurrent external focus after settlement, never by a tab-wide echo", () => {
    let state = begin(files(initial()));
    state = reconcileSnapshot(state, snapshot(["p1", "p2", "p3", "p4"], "p4")).state;
    expect(state.tabs.tab.selectedLeafId).toBe("tab:files");
    const settled = layoutReducer(state, { type: "creation/settled", token: 7, created });
    expect(settled.state.tabs.tab.selectedLeafId).toBe("p4");
    expect(settled.effects).toContainEqual({ type: "cancel-transient", reason: "external-focus" });
  });

  it("does not buffer known-pane external focus and does not replay older buffered focus", () => {
    let state = begin(files(initial()));
    state = reconcileSnapshot(state, snapshot(["p1", "p2", "p3", "p4"], "p4")).state;
    state = reconcileSnapshot(state, snapshot(["p1", "p2", "p3", "p4"], "p2")).state;
    expect(state.tabs.tab.selectedLeafId).toBe("p2");
    expect(layoutReducer(state, { type: "creation/settled", token: 7, created }).state.tabs.tab.selectedLeafId).toBe("p2");
  });

  it("releases all held members externally on uncertain outcome and follows buffered focus", () => {
    let state = begin(files(initial()));
    state = reconcileSnapshot(state, snapshot(["p1", "p2", "p3"], "p3")).state;
    const settled = layoutReducer(state, { type: "creation/settled", token: 7, created: null }).state;
    expect(leaves(settled.tabs.tab.root).at(-1)).toMatchObject({ id: "p3", w: .25 });
    expect(settled.tabs.tab.selectedLeafId).toBe("p3");
    expect(settled.pendingCreation).toBeNull();
  });

  it("never selects the created terminal over a newer viewer selection", () => {
    let state = begin(files(initial()));
    state = reconcileSnapshot(state, snapshot(["p1", "p2", "p3"], "p3")).state;
    state = apply(state, { type: "select-leaf", tabId: "tab", leafId: "tab:files" });
    expect(layoutReducer(state, { type: "creation/settled", token: 7, created }).state.tabs.tab.selectedLeafId).toBe("tab:files");
  });
});
