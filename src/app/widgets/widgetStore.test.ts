// @vitest-environment jsdom
import { afterEach, describe, expect, it, vi } from "vitest";
import type { CockpitClient } from "../../client/CockpitClient";
import type { WidgetSummary, WidgetEvent, WidgetContentRequest } from "../../protocol/generated/v1";
import { createSessionLayoutState, layoutReducer, type LeafCtx, type LayoutAction } from "../layout/tabLayoutStore";
import { WidgetStore, widgetKey } from "./widgetStore";

function widget(id = "stats", overrides: Partial<WidgetSummary> = {}): WidgetSummary {
  return { key: { session_id: "session", tab_id: "tab", id }, space_id: "space", title: id, revision: 1, created_seq: 1,
    kind: "html", presentation: "active", content: { sha256: "a".repeat(64), bytes: 10, from: "stdin", name: null }, warnings: [],
    source: { pane_id: "pane", tab_id: "tab", space_id: "space", terminal_id: "terminal", agent_label: "omp", fingerprint_prefix: null, status: "present" },
    arrival: "own_tab", resolved_from: "current_pane", change: "opened", created_at_ms: 1, updated_at_ms: 1, selection: null, ...overrides };
}
function fixture() {
  let state = createSessionLayoutState("session", "server");
  state.tabs.tab = { tabId: "tab", spaceId: "space", root: { t: "leaf", id: "pane", kind: "terminal", w: 1 }, terminals: { pane: "terminal" },
    selectedLeafId: "pane", lastRealLeafId: "pane", zoomLeafId: null, viewers: {}, widgetShare: 0.4, heldMembers: [], heldTerminalIds: {}, bufferedFocus: null,
    focusedPaneId: "pane", revision: 1, selectionRevision: 2 };
  const report = vi.fn(), close = vi.fn();
  const events: Array<(event: WidgetEvent) => void> = [], failures: Array<() => void> = [];
  const client = { subscribeWidgets: vi.fn(async (onEvent: (event: WidgetEvent) => void, onError: () => void) => { events.push(onEvent); failures.push(onError); return { report, close }; }),
    widgetRemove: vi.fn(async () => ({ result: "removed" })),
    widgetContent: vi.fn(async (request: WidgetContentRequest) => ({ ...request, sha256: "a".repeat(64), body: { type: "html", document: "<p>statistics</p>" }, selection: null })) } as unknown as CockpitClient;
  const actions: LayoutAction[] = [];
  const ctx: LeafCtx = { client, sessionId: "session", serverInstance: "server", clientId: "client", getState: () => state,
    dispatch(action) { actions.push(action); state = layoutReducer(state, action).state; } };
  const store = new WidgetStore(client), announce = vi.fn();
  store.bind(ctx, announce);
  store.updateWindow({ session_id: "session", displayed_tab_id: "tab", blocker: null });
  return { store, client, ctx, actions, report, events, failures, announce };
}
afterEach(() => vi.useRealTimers());
describe("widget window state", () => {
  it("opens unselected, replaces without docking and removes the last dock", () => {
    const f = fixture(), first = widget(), key = widgetKey(first.key);
    f.store.accept({ type: "snapshot", sequence: 0, widgets: [] });
    f.store.accept({ type: "upserted", sequence: 1, widget: first });
    expect(f.ctx.getState().tabs.tab.selectedLeafId).toBe("pane");
    expect(f.ctx.getState().tabs.tab.selectionRevision).toBe(2);
    expect(f.ctx.getState().tabs.tab.viewers.widget?.currentId).toBe("stats");
    expect(f.store.everDisplayed.has(key)).toBe(true);
    expect(f.store.tabHasDot("session", "tab")).toBe(false);
    f.actions.length = 0;
    f.store.accept({ type: "upserted", sequence: 2, widget: widget("stats", { revision: 2, change: "replaced" }) });
    expect(f.actions).toEqual([]);
    expect(f.announce).toHaveBeenCalledTimes(1);
    f.store.accept({ type: "removed", sequence: 3, key: first.key, reason: "user" });
    expect(f.ctx.getState().tabs.tab.viewers.widget).toBeUndefined();
    expect(f.store.widgets("session")).toEqual([]);
    expect(f.store.everDisplayed.has(key)).toBe(false);
  });
  it("defers own-tab docking until clear and cross-source until explicit click", () => {
    const f = fixture();
    f.store.updateWindow({ session_id: "session", displayed_tab_id: null, blocker: null });
    f.store.accept({ type: "snapshot", sequence: 0, widgets: [widget()] });
    expect(f.ctx.getState().tabs.tab.viewers.widget).toBeUndefined();
    expect(f.store.tabHasDot("session", "tab")).toBe(true);
    f.store.updateWindow({ session_id: "session", displayed_tab_id: "tab", blocker: "zoom" });
    expect(f.store.needsClick("session", "tab")).toBe(true);
    f.store.updateWindow({ session_id: "session", displayed_tab_id: "tab", blocker: null });
    expect(f.ctx.getState().tabs.tab.viewers.widget?.currentId).toBe("stats");
    f.store.accept({ type: "upserted", sequence: 1, widget: widget("other", { arrival: "cross_source", created_seq: 2 }) });
    expect(f.ctx.getState().tabs.tab.viewers.widget?.currentId).toBe("stats");
    f.store.updateWindow({ session_id: "session", displayed_tab_id: "tab", blocker: null });
    expect(f.store.needsClick("session", "tab")).toBe(true);
    f.store.show("tab");
    expect(f.ctx.getState().tabs.tab.viewers.widget?.currentId).toBe("other");
    expect(f.store.needsClick("session", "tab")).toBe(false);
  });
  it("selects the right neighbour then the left when removing the last current id", () => {
    const f = fixture();
    f.store.accept({ type: "snapshot", sequence: 0, widgets: [widget("a"), widget("b", { created_seq: 2 }), widget("c", { created_seq: 3 })] });
    f.store.current("tab", "b");
    f.store.accept({ type: "removed", sequence: 1, key: widget("b").key, reason: "agent" });
    expect(f.ctx.getState().tabs.tab.viewers.widget?.currentId).toBe("c");
    f.store.accept({ type: "removed", sequence: 2, key: widget("c").key, reason: "agent" });
    expect(f.ctx.getState().tabs.tab.viewers.widget?.currentId).toBe("a");
  });
  it("reports blockers and re-snapshots after reconnect without resetting current id", async () => {
    vi.useFakeTimers();
    const f = fixture(), release = f.store.retain();
    await Promise.resolve();
    f.events[0]({ type: "snapshot", sequence: 10, widgets: [widget("a"), widget("b", { created_seq: 2 })] });
    f.store.current("tab", "a");
    f.store.updateWindow({ session_id: "session", displayed_tab_id: "tab", blocker: "library" });
    expect(f.report).toHaveBeenLastCalledWith({ session_id: "session", displayed_tab_id: "tab", blocker: "library" });
    f.failures[0](); await vi.advanceTimersByTimeAsync(1000);
    f.events[1]({ type: "snapshot", sequence: 0, widgets: [widget("a"), widget("b", { created_seq: 2 })] });
    expect(f.ctx.getState().tabs.tab.viewers.widget?.currentId).toBe("a");
    f.events[0]({ type: "removed", sequence: 99, key: widget("a").key, reason: "user" });
    expect(f.store.widgets("session").map(w => w.key.id)).toEqual(["a", "b"]);
    release();
  });
  it("ignores duplicate events and does not apply an event after a sequence gap", async () => {
    const f = fixture();
    f.store.accept({ type: "snapshot", sequence: 0, widgets: [widget()] });
    f.store.accept({ type: "removed", sequence: 0, key: widget().key, reason: "user" });
    f.store.accept({ type: "removed", sequence: 2, key: widget().key, reason: "user" });
    expect(f.store.widgets("session").map(w => w.key.id)).toEqual(["stats"]);
  });
  it("does not delete an intentional reopen when removal response arrives after its event", async () => {
    const f = fixture();
    let confirm!: (value: { result: "removed" }) => void;
    const promise = new Promise<{ result: "removed" }>(resolve => { confirm = resolve; });
    vi.mocked(f.client.widgetRemove).mockImplementationOnce(() => promise);
    f.store.accept({ type: "snapshot", sequence: 0, widgets: [widget()] });
    const removal = f.store.remove(widget().key);
    f.store.accept({ type: "removed", sequence: 1, key: widget().key, reason: "user" });
    f.store.accept({ type: "upserted", sequence: 2, widget: widget("stats", { revision: 2, change: "reopened" }) });
    confirm({ result: "removed" }); await removal;
    expect(f.store.widgets("session").map(w => w.revision)).toEqual([2]);
    expect(f.ctx.getState().tabs.tab.viewers.widget?.currentId).toBe("stats");
  });
  it("shares one subscription among retainers and preserves a newly bound layout across a remount", async () => {
    const f = fixture();
    const releaseFirst = f.store.retain(), releaseSecond = f.store.retain();
    await Promise.resolve();
    expect(f.client.subscribeWidgets).toHaveBeenCalledTimes(1);
    releaseFirst();
    f.store.accept({ type: "snapshot", sequence: 0, widgets: [] });
    f.store.accept({ type: "upserted", sequence: 1, widget: widget() });
    expect(f.ctx.getState().tabs.tab.viewers.widget?.currentId).toBe("stats");
    f.ctx.dispatch({ type: "widget/undock", tabId: "tab" });
    f.store.bind(f.ctx, f.announce);
    releaseSecond();
    const releaseRemount = f.store.retain();
    await Promise.resolve();
    f.store.accept({ type: "upserted", sequence: 2, widget: widget("new", { created_seq: 2 }) });
    expect(f.ctx.getState().tabs.tab.viewers.widget?.currentId).toBe("new");
    expect(f.client.subscribeWidgets).toHaveBeenCalledTimes(2);
    releaseRemount();
  });
  it("applies source-specific narrow blockers instead of the selected terminal's width", () => {
    const f = fixture();
    let narrow = true;
    f.store.bind(f.ctx, f.announce, () => narrow ? "too_narrow" : null);
    f.store.accept({ type: "snapshot", sequence: 0, widgets: [] });
    f.store.accept({ type: "upserted", sequence: 1, widget: widget() });
    expect(f.ctx.getState().tabs.tab.viewers.widget).toBeUndefined();
    expect(f.store.needsClick("session", "tab")).toBe(true);
    narrow = false;
    f.store.updateWindow({ session_id: "session", displayed_tab_id: "tab", blocker: null });
    expect(f.ctx.getState().tabs.tab.viewers.widget?.currentId).toBe("stats");
    expect(f.store.needsClick("session", "tab")).toBe(false);
  });
  it("keeps typing in the current frame on arrival and clears the unseen dot on navigation", () => {
    const f = fixture();
    f.store.accept({ type: "snapshot", sequence: 0, widgets: [widget("a")] });
    const dock = document.createElement("div"), frame = document.createElement("iframe");
    dock.dataset.widgetTab = "tab"; dock.append(frame); document.body.append(dock);
    try {
      frame.focus();
      f.store.accept({ type: "upserted", sequence: 1, widget: widget("b", { created_seq: 2 }) });
      expect(f.ctx.getState().tabs.tab.viewers.widget?.currentId).toBe("a");
      expect(document.activeElement).toBe(frame);
      expect(f.store.unseen.has(widgetKey(widget("b").key))).toBe(true);
      f.store.cycle("tab", 1);
      expect(f.ctx.getState().tabs.tab.viewers.widget?.currentId).toBe("b");
      expect(f.store.unseen.has(widgetKey(widget("b").key))).toBe(false);
    } finally { dock.remove(); }
  });
  it("does not mark a never-displayed cross-source replacement as unseen", () => {
    const f = fixture();
    f.store.accept({ type: "snapshot", sequence: 0, widgets: [widget("a")] });
    f.store.accept({ type: "upserted", sequence: 1, widget: widget("b", { arrival: "cross_source", created_seq: 2 }) });
    f.store.accept({ type: "upserted", sequence: 2, widget: widget("b", { arrival: "cross_source", created_seq: 2, revision: 2, change: "replaced" }) });
    expect(f.store.unseen.has(widgetKey(widget("b").key))).toBe(false);
    expect(f.store.needsClick("session", "tab")).toBe(true);
    expect(f.ctx.getState().tabs.tab.viewers.widget?.currentId).toBe("a");
  });
  it("keeps owner updates when a concurrent user removal fails", async () => {
    const f = fixture();
    f.store.accept({ type: "snapshot", sequence: 0, widgets: [widget()] });
    vi.mocked(f.client.widgetRemove).mockImplementationOnce(async () => {
      f.store.accept({ type: "upserted", sequence: 1, widget: widget("stats", { revision: 2, change: "replaced" }) });
      throw new Error("Removal unavailable");
    });
    await expect(f.store.remove(widget().key)).rejects.toThrow("Removal unavailable");
    expect(f.store.widgets("session")[0].revision).toBe(2);
  });
  it("restores snapshot order and notices replacements missed while disconnected", () => {
    const f = fixture();
    f.store.accept({ type: "snapshot", sequence: 10, widgets: [widget("b", { created_seq: 2 }), widget("a")] });
    expect(f.ctx.getState().tabs.tab.viewers.widget?.currentId).toBe("b");
    f.store.current("tab", "a");
    f.store.accept({ type: "snapshot", sequence: 20, widgets: [widget("b", { created_seq: 2, revision: 2, change: "updated" }), widget("a")] });
    expect(f.ctx.getState().tabs.tab.viewers.widget?.currentId).toBe("a");
    expect(f.store.unseen.has(widgetKey(widget("b").key))).toBe(true);
  });
});
