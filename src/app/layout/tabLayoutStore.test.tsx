// @vitest-environment jsdom
import { act } from "react";
import { createRoot } from "react-dom/client";
import { describe, expect, it } from "vitest";
import { createSessionLayoutState, layoutReducer, useTabLayouts, type TabLayouts } from "./tabLayoutStore";
import type { LayoutSnapshot } from "./reconcile";
import { leaves } from "./splitTree";

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
