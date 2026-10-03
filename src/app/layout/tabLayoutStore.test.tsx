// @vitest-environment jsdom
import { act } from "react";
import { createRoot } from "react-dom/client";
import { describe, expect, it } from "vitest";
import { createSessionLayoutState, layoutReducer, useTabLayouts, type LayoutAction, type SessionLayoutState, type TabLayouts } from "./tabLayoutStore";
import type { LayoutSnapshot } from "./reconcile";
import { findNode, leaves } from "./splitTree";

(globalThis as typeof globalThis & { IS_REACT_ACT_ENVIRONMENT?: boolean }).IS_REACT_ACT_ENVIRONMENT = true;
const snapshot: LayoutSnapshot = { server_instance: "server", focused_space_id: "space", focused_tab_id: "tab", focused_pane_id: "p1",
  tabs: [{ id: "tab", space_id: "space", focused_pane_id: "p1" }],
  panes: [{ id: "p1", terminal_id: "term1", space_id: "space", tab_id: "tab" }, { id: "p2", terminal_id: "term2", space_id: "space", tab_id: "tab" }] };

it("ignores a stale drag commit after a structural membership change", () => {
  const initial = layoutReducer(createSessionLayoutState("session"), { type: "snapshot", snapshot }).state;
  const revision = initial.tabs.tab.revision;
  const updated = layoutReducer(initial, { type: "snapshot", snapshot: { ...snapshot,
    panes: [...snapshot.panes, { id: "p3", terminal_id: "term3", space_id: "space", tab_id: "tab" }] } }).state;
  const dropped = layoutReducer(updated, { type: "drop", tabId: "tab", src: "p1", revision,
    target: { kind: "swap", target: "p2", rect: { x: 0, y: 0, width: 400, height: 300 }, label: "Swap" } }).state;
  expect(dropped).toBe(updated);
  expect(leaves(dropped.tabs.tab.root).map(leaf => leaf.id)).toEqual(["p1", "p2", "p3"]);
});

it("keeps source-specific unsaved view state and ignores stale open completion after close", () => {
  let state = layoutReducer(createSessionLayoutState("session"), { type: "snapshot", snapshot }).state;
  state = layoutReducer(state, { type: "viewer/open-begin", tabId: "tab", kind: "files", selector: { kind: "files_folder" }, sourcePaneId: "p1", dir: "row", requestId: "first" }).state;
  state = layoutReducer(state, { type: "viewer/source-view", tabId: "tab", kind: "files", sourceId: "source1", view: { commentEditor: { text: "unsaved" } } }).state;
  const arrangement = state.tabs.tab.root;
  state = layoutReducer(state, { type: "viewer/open-begin", tabId: "tab", kind: "files", selector: { kind: "files_context" }, sourcePaneId: "p2", dir: "col", requestId: "second" }).state;
  expect(state.tabs.tab.root).toBe(arrangement);
  expect(state.tabs.tab.viewers.files?.viewsBySource.source1).toEqual({ commentEditor: { text: "unsaved" } });
  const ignored = layoutReducer(state, { type: "viewer/failed", tabId: "tab", kind: "files", error: "old request", requestId: "first" });
  expect(ignored.state).toBe(state);
  state = layoutReducer(state, { type: "viewer/closed", tabId: "tab", kind: "files" }).state;
  expect(state.tabs.tab.selectedLeafId).toBe("p1");
  expect(state.tabs.tab.viewers.files).toBeUndefined();
  expect(layoutReducer(state, { type: "viewer/failed", tabId: "tab", kind: "files", error: "late", requestId: "second" }).state).toBe(state);
});

describe("App-owned session layouts", () => {
  it("updates state synchronously and binds delayed completions to their original session", async () => {
    const host = document.createElement("div"), root = createRoot(host);
    let current!: TabLayouts;
    function Harness({ sessionId }: { sessionId: string }) { current = useTabLayouts(sessionId); return null; }
    try {
      await act(async () => root.render(<Harness sessionId="old" />));
      const old = current;
      await act(async () => {
        old.dispatch({ type: "snapshot", snapshot });
        expect(old.getState().tabs.tab.terminals.p1).toBe("term1");
      });
      await act(async () => root.render(<Harness sessionId="new" />));
      await act(async () => current.dispatch({ type: "snapshot", snapshot }));
      const newSession = current;
      await act(async () => old.dispatch({ type: "select-leaf", tabId: "tab", leafId: "p2" }));
      expect(newSession.getState().sessionId).toBe("new");
      expect(newSession.getState().tabs.tab.selectedLeafId).toBe("p1");
      expect(old.getState().tabs.tab.selectedLeafId).toBe("p2");
      await act(async () => root.render(<Harness sessionId="old" />));
      expect(current.state.tabs.tab.selectedLeafId).toBe("p2");
    } finally { await act(async () => root.unmount()); }
  });
});

describe("tab-local widget dock", () => {
  const initial = () => layoutReducer(createSessionLayoutState("session"), { type: "snapshot", snapshot }).state;
  const apply = (state: SessionLayoutState, action: LayoutAction) => layoutReducer(state, action).state;
  const dock = (state: SessionLayoutState, besideLeafId = "p1", currentId = "stats") =>
    apply(state, { type: "widget/dock", tabId: "tab", besideLeafId, currentId });
  const undock = (state: SessionLayoutState) => apply(state, { type: "widget/undock", tabId: "tab" });

  it("inserts beside the explicit source with no automatic selection, zoom or focus change", () => {
    const before = apply(initial(), { type: "zoom-toggle", tabId: "tab", leafId: "p1" });
    const result = layoutReducer(before, { type: "widget/dock", tabId: "tab", besideLeafId: "p2", currentId: "stats" });
    const tab = result.state.tabs.tab;
    expect(leaves(tab.root).map(leaf => leaf.id)).toEqual(["p1", "p2", "tab:widget"]);
    expect(findNode(tab.root, "p1")?.w).toBeCloseTo(.5);
    expect(findNode(tab.root, "p2")?.w).toBeCloseTo(.3);
    expect(findNode(tab.root, "tab:widget")).toMatchObject({ kind: "widget", w: .2 });
    expect(tab.selectedLeafId).toBe(before.tabs.tab.selectedLeafId);
    expect(tab.selectionRevision).toBe(before.tabs.tab.selectionRevision);
    expect(tab.lastRealLeafId).toBe(before.tabs.tab.lastRealLeafId);
    expect(tab.zoomLeafId).toBe(before.tabs.tab.zoomLeafId);
    expect(tab.viewers.widget).toEqual({ currentId: "stats", previousSelectedLeafId: null });
    expect(result.state.observedFocus).toBe(before.observedFocus);
    expect(result.state.focusIntents).toBe(before.focusIntents);
    expect(result.effects).toEqual([]);
  });

  it("ignores missing or non-leaf placement and current changes after undock", () => {
    const before = initial();
    expect(dock(before, "missing")).toBe(before);
    expect(dock(before, before.tabs.tab.root!.id)).toBe(before);
    expect(apply(before, { type: "widget/current", tabId: "tab", currentId: "late" })).toBe(before);
    const closed = undock(dock(before));
    expect(apply(closed, { type: "widget/current", tabId: "tab", currentId: "late" })).toBe(closed);
    expect(undock(closed)).toBe(closed);
  });

  it("changes the current widget without reinserting the dock or selecting it", () => {
    const before = dock(initial());
    const current = apply(before, { type: "widget/current", tabId: "tab", currentId: "errors" });
    const repeated = dock(current, "p2", "diagram");
    for (const state of [current, repeated]) {
      expect(state.tabs.tab.root).toBe(before.tabs.tab.root);
      expect(state.tabs.tab.revision).toBe(before.tabs.tab.revision);
      expect(state.tabs.tab.selectionRevision).toBe(before.tabs.tab.selectionRevision);
      expect(state.tabs.tab.selectedLeafId).toBe("p1");
    }
    expect(current.tabs.tab.viewers.widget?.currentId).toBe("errors");
    expect(repeated.tabs.tab.viewers.widget?.currentId).toBe("diagram");
  });

  it("remembers the resized source-pair share through unselected undock and reopen", () => {
    let state = dock(initial());
    const root = state.tabs.tab.root!;
    state = apply(state, { type: "resize-commit", tabId: "tab", splitId: root.id, index: 0, wa: 1, wb: 3 });
    state = apply(state, { type: "select-leaf", tabId: "tab", leafId: "p2" });
    const before = state.tabs.tab;
    state = undock(state);
    expect(state.tabs.tab.selectedLeafId).toBe("p2");
    expect(state.tabs.tab.selectionRevision).toBe(before.selectionRevision);
    expect(state.tabs.tab.lastRealLeafId).toBe("p2");
    expect(state.tabs.tab.viewers.widget).toBeUndefined();
    expect(state.tabs.tab.widgetShare).toBeCloseTo(.75);
    const reopened = dock(state);
    const source = findNode(reopened.tabs.tab.root, "p1")!;
    const widget = findNode(reopened.tabs.tab.root, "tab:widget")!;
    expect(widget.w / (widget.w + source.w)).toBeCloseTo(.75);
    expect(reopened.tabs.tab.selectedLeafId).toBe("p2");
    expect(reopened.tabs.tab.selectionRevision).toBe(before.selectionRevision);
  });

  it("returns from a selected dock to the last selected leaf, not its source", () => {
    let state = dock(initial());
    state = apply(state, { type: "select-leaf", tabId: "tab", leafId: "p2" });
    state = apply(state, { type: "select-leaf", tabId: "tab", leafId: "tab:widget" });
    state = apply(state, { type: "select-leaf", tabId: "tab", leafId: "tab:widget" });
    expect(state.tabs.tab.viewers.widget?.previousSelectedLeafId).toBe("p2");
    const revision = state.tabs.tab.selectionRevision;
    state = undock(state);
    expect(state.tabs.tab.selectedLeafId).toBe("p2");
    expect(state.tabs.tab.selectionRevision).toBe(revision + 1);
    expect(state.tabs.tab.widgetShare).toBeCloseTo(.4);
  });

  it("falls back to the beside terminal when the prior selection has disappeared", () => {
    let state = dock(initial());
    state = apply(state, { type: "select-leaf", tabId: "tab", leafId: "p2" });
    state = apply(state, { type: "zoom-toggle", tabId: "tab", leafId: "tab:widget" });
    state = apply(state, { type: "snapshot", snapshot: { ...snapshot, panes: snapshot.panes.slice(0, 1) } });
    expect(state.tabs.tab.selectedLeafId).toBe("tab:widget");
    state = undock(state);
    expect(state.tabs.tab.selectedLeafId).toBe("p1");
    expect(state.tabs.tab.lastRealLeafId).toBe("p1");
    expect(state.tabs.tab.zoomLeafId).toBeNull();
  });

  it("closes the dock locally and retains its share for later arrivals", () => {
    let state = dock(initial());
    state = apply(state, { type: "leaf/close-local", tabId: "tab", leafId: "tab:widget" });
    expect(state.tabs.tab.viewers.widget).toBeUndefined();
    expect(findNode(state.tabs.tab.root, "tab:widget")).toBeNull();
    expect(state.tabs.tab.selectedLeafId).toBe("p1");
    expect(state.tabs.tab.widgetShare).toBeCloseTo(.4);
  });
});
