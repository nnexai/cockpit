// @vitest-environment jsdom
import { act } from "react";
import { createRoot } from "react-dom/client";
import { expect, it, vi } from "vitest";
import type { CockpitClient } from "../../client/CockpitClient";
import type { WidgetContent, WidgetSummary } from "../../protocol/generated/v1";
import { createSessionLayoutState, layoutReducer } from "../layout/tabLayoutStore";
import type { LeafCtx } from "../layout/tabLayoutStore";
import { getWidgetStore } from "./widgetStore";
import { WidgetDock } from "./WidgetDock";

Object.assign(globalThis, { IS_REACT_ACT_ENVIRONMENT: true });

const summary: WidgetSummary = {
  key: { session_id: "session", tab_id: "tab", id: "stats" }, space_id: "space", title: "Requests",
  revision: 1, created_seq: 1, kind: "html", presentation: "active",
  content: { sha256: "a".repeat(64), bytes: 2048, from: "file", name: "stats.html" }, warnings: [],
  source: { pane_id: "p1", tab_id: "tab", space_id: "space", terminal_id: "terminal", agent_label: "omp", fingerprint_prefix: "123456abcdef", status: "present" },
  arrival: "own_tab", resolved_from: "current_pane", change: "opened", created_at_ms: 1000, updated_at_ms: 1000, selection: null,
};

function fixture(client: CockpitClient) {
  let state = layoutReducer(createSessionLayoutState("session"), { type: "snapshot", snapshot: {
    server_instance: "server", focused_space_id: "space", focused_tab_id: "tab", focused_pane_id: "p1",
    tabs: [{ id: "tab", space_id: "space", focused_pane_id: "p1" }],
    panes: [{ id: "p1", terminal_id: "terminal", space_id: "space", tab_id: "tab" }],
  } }).state;
  const ctx: LeafCtx = {
    client, sessionId: "session", serverInstance: "server", clientId: "window",
    getState: () => state,
    dispatch: (action) => { state = layoutReducer(state, action).state; },
  };
  const store = getWidgetStore(client);
  store.bind(ctx, () => {});
  store.updateWindow({ session_id: "session", displayed_tab_id: "tab", blocker: null });
  return { ctx, store };
}

it("retains the visible preview while fetching a replacement and swaps only once its new frame loads", async () => {
  const initial: WidgetContent = { key: summary.key, revision: 1, sha256: summary.content.sha256, body: { type: "html", document: "<p>Original chart</p>" }, selection: null };
  // The project's ES2022 library has no Promise.withResolvers.
  let finish!: (content: WidgetContent) => void;
  const pending = new Promise<WidgetContent>((resolve) => { finish = resolve; });
  const widgetContent = vi.fn().mockResolvedValueOnce(initial).mockReturnValueOnce(pending);
  const client = { widgetContent } as unknown as CockpitClient;
  const { ctx, store } = fixture(client);
  store.accept({ type: "snapshot", sequence: 1, widgets: [summary] });
  const host = document.createElement("div");
  // Keep jsdom's automatic about:blank load separate from the controlled srcDoc-ready transition.
  const frameLoad = new Event("load");
  host.addEventListener("load", (event) => {
    if (event.target instanceof HTMLIFrameElement && event !== frameLoad) event.stopImmediatePropagation();
  }, true);
  document.body.append(host);
  const root = createRoot(host);
  const onSelect = vi.fn();
  const onGoToAgent = vi.fn();
  const render = () => root.render(<WidgetDock ctx={ctx} tab={ctx.getState().tabs.tab} width={600}
    selected={false} inputBlocked={false} live onSelect={onSelect} onZoom={() => {}} onGoToAgent={onGoToAgent} />);
  try {
    await act(async () => render());
    const original = host.querySelector<HTMLIFrameElement>("iframe")!;
    expect(original.srcdoc).toContain("Original chart");
    const updated: WidgetSummary = { ...summary, revision: 2, change: "replaced", updated_at_ms: 2000,
      content: { ...summary.content, sha256: "b".repeat(64) } };
    await act(async () => {
      store.accept({ type: "upserted", sequence: 2, widget: updated });
      render();
    });
    expect(widgetContent).toHaveBeenLastCalledWith({ key: summary.key, revision: 2 }, expect.any(AbortSignal));
    expect(host.querySelectorAll("iframe")).toHaveLength(1);
    expect(host.querySelector("iframe")).toBe(original);
    expect(original.srcdoc).toContain("Original chart");
    await act(async () => finish({ key: summary.key, revision: 2, sha256: updated.content.sha256,
      body: { type: "html", document: "<p>Updated chart</p>" }, selection: null }));
    expect(host.querySelector("iframe")).toBe(original);
    expect(host.querySelectorAll("iframe")).toHaveLength(2);
    const incoming = host.querySelector<HTMLIFrameElement>("iframe[data-pending]")!;
    expect(incoming.srcdoc).toContain("Updated chart");
    expect(incoming.getAttribute("aria-hidden")).toBe("true");
    window.dispatchEvent(new MessageEvent("message", { source: original.contentWindow,
      data: { ...dockFrameIdentity(original), type: "cockpit.widget.intent" } }));
    await act(async () => original.focus());
    expect(document.activeElement).toBe(original);
    await act(async () => incoming.dispatchEvent(frameLoad));
    expect(host.querySelectorAll("iframe")).toHaveLength(1);
    expect(host.querySelector("iframe")).toBe(incoming);
    expect(ctx.getState().tabs.tab.selectedLeafId).toBe("p1");
    expect(onSelect).not.toHaveBeenCalled();
    expect(document.activeElement).toBe(host.querySelector(".widget-dock"));
    expect(onGoToAgent).not.toHaveBeenCalled();
  } finally {
    await act(async () => root.unmount());
    host.remove();
  }
});

it("renders fetched declarative choices in the dock without mounting an agent iframe", async () => {
  const choices: WidgetSummary = { ...summary, kind: "choices", presentation: "choices" };
  const content: WidgetContent = { key: choices.key, revision: choices.revision, sha256: choices.content.sha256, selection: null,
    body: { type: "choices", spec: { prompt: "Which view should I keep?", choices: [
      { id: "latency", label: "Latency", detail: "per route" }, { id: "errors", label: "Errors", detail: null },
    ] } } };
  const widgetSelect = vi.fn().mockResolvedValue({ at_ms: 3000 });
  const client = { widgetContent: vi.fn().mockResolvedValue(content), widgetSelect } as unknown as CockpitClient;
  const { ctx, store } = fixture(client);
  store.accept({ type: "snapshot", sequence: 1, widgets: [choices] });
  const host = document.createElement("div");
  document.body.append(host);
  const root = createRoot(host);
  const onSelect = vi.fn();
  const onGoToAgent = vi.fn();
  try {
    await act(async () => root.render(<WidgetDock ctx={ctx} tab={ctx.getState().tabs.tab} width={600}
      selected={false} inputBlocked={false} live onSelect={onSelect} onZoom={() => {}} onGoToAgent={onGoToAgent} />));
    expect(host.querySelector("iframe")).toBeNull();
    const group = host.querySelector<HTMLElement>('[role="radiogroup"]')!;
    expect(group.querySelector("legend")!.textContent).toBe("Which view should I keep?");
    await act(async () => group.querySelector<HTMLInputElement>('input[value="errors"]')!.click());
    expect(widgetSelect).toHaveBeenCalledWith({ key: choices.key, revision: choices.revision,
      value: { type: "choice", choice_id: "errors" } });
    expect(group.querySelector<HTMLInputElement>('input[value="errors"]')!.checked).toBe(true);
    expect(ctx.getState().tabs.tab.selectedLeafId).toBe("p1");
    expect(onSelect).not.toHaveBeenCalled();
    expect(onGoToAgent).not.toHaveBeenCalled();
  } finally {
    await act(async () => root.unmount());
    host.remove();
  }
});

function dockFrameIdentity(frame: HTMLIFrameElement): { nonce: string; revision: number } {
  const state = frame.srcdoc.match(/const state = JSON.parse\(("(?:[^"\\]|\\.)*")\);/)!;
  return JSON.parse(JSON.parse(state[1]));
}

it("binds opaque page selections to the exact owner and restores retained selected null on remount", async () => {
  const content: WidgetContent = { key: summary.key, revision: 1, sha256: summary.content.sha256,
    body: { type: "html", document: "<p>Results</p>" }, selection: {
      id: summary.key.id, revision: 1, status: "selected", value_json: "null", at_ms: 2000, removed_at_ms: null,
    } };
  const widgetSelect = vi.fn().mockResolvedValue({ at_ms: 3000 });
  const terminalInput = vi.fn();
  const client = { widgetContent: vi.fn().mockResolvedValue(content), widgetSelect, terminalInput } as unknown as CockpitClient;
  const { ctx, store } = fixture(client);
  store.accept({ type: "snapshot", sequence: 1, widgets: [summary] });
  const host = document.createElement("div");
  document.body.append(host);
  const root = createRoot(host);
  const render = () => root.render(<WidgetDock ctx={ctx} tab={ctx.getState().tabs.tab} width={600}
    selected={false} inputBlocked={false} live onSelect={() => {}} onZoom={() => {}} onGoToAgent={() => {}} />);
  try {
    await act(async () => render());
    const frame = host.querySelector<HTMLIFrameElement>("iframe")!;
    expect(dockFrameIdentity(frame)).toMatchObject({ selection: null, hasSelection: true });
    const valueJson = JSON.stringify({ script: "<script>host()</script>", command: "terminal.paste", answer: [1, true, null] });
    await act(async () => {
      window.dispatchEvent(new MessageEvent("message", { source: frame.contentWindow, data: {
        ...dockFrameIdentity(frame), type: "cockpit.widget.select", value_json: valueJson,
        key: { ...summary.key, id: "other" }, revision: 1,
      } }));
    });
    expect(widgetSelect).toHaveBeenCalledExactlyOnceWith({ key: summary.key, revision: 1, value: { type: "page", value_json: valueJson } });
    expect(terminalInput).not.toHaveBeenCalled();
    expect(store.contentState(summary).content?.selection?.value_json).toBe(valueJson);
    await act(async () => root.render(null));
    await act(async () => render());
    const remounted = host.querySelector<HTMLIFrameElement>("iframe")!;
    expect(dockFrameIdentity(remounted)).toMatchObject({ selection: JSON.parse(valueJson), hasSelection: true });
    await act(async () => {
      await store.selectPage({ ...summary.key, id: "other" }, 1, "<p>Results</p>", "1");
      await store.selectPage(summary.key, 2, "<p>Results</p>", "1");
      await store.selectPage(summary.key, 1, "<p>Wrong content</p>", "1");
    });
    expect(widgetSelect).toHaveBeenCalledTimes(1);
  } finally { await act(async () => root.unmount()); host.remove(); }
});

it("does not install a late selection response after the owner replaces the content", async () => {
  const content: WidgetContent = { key: summary.key, revision: 1, sha256: summary.content.sha256,
    body: { type: "html", document: "<p>Before</p>" }, selection: null };
  const widgetSelect = vi.fn().mockResolvedValue({ at_ms: 3000 });
  const client = { widgetContent: vi.fn().mockResolvedValue(content), widgetSelect } as unknown as CockpitClient;
  const { store } = fixture(client);
  store.accept({ type: "snapshot", sequence: 1, widgets: [summary] });
  store.fetchContent(summary);
  await act(async () => {});
  const pending = store.selectPage(summary.key, 1, "<p>Before</p>", "1");
  const replacement: WidgetSummary = { ...summary, revision: 2, change: "replaced" };
  store.accept({ type: "upserted", sequence: 2, widget: replacement });
  await pending;
  expect(widgetSelect).toHaveBeenCalledTimes(1);
  expect(store.contentState(replacement).content).toBeNull();
});

it.each(["agent", "retired", "snapshot", "user", "localUser"] as const)("keeps dialog DOM focus when a layout-selected dock disappears via %s", async (reason) => {
  const { ctx, store } = fixture({ widgetRemove: vi.fn().mockResolvedValue({ result: "removed" }) } as unknown as CockpitClient);
  store.accept({ type: "snapshot", sequence: 1, widgets: [summary] });
  ctx.dispatch({ type: "select-leaf", tabId: "tab", leafId: "tab:widget" });
  const host = document.createElement("div");
  host.innerHTML = '<div data-leaf-id="p1"><textarea></textarea></div><section role="dialog" aria-modal="true"><button>Confirm</button></section>';
  document.body.append(host);
  const dialogControl = host.querySelector("button")!;
  const terminal = host.querySelector("textarea")!;
  const focus = vi.spyOn(terminal, "focus");
  const animation = vi.fn();
  vi.stubGlobal("requestAnimationFrame", animation);
  try {
    dialogControl.focus();
    if (reason === "snapshot") store.accept({ type: "snapshot", sequence: 2, widgets: [] });
    else if (reason === "localUser") await store.remove(summary.key);
    else store.accept({ type: "removed", sequence: 2, key: summary.key, reason });
    expect(ctx.getState().tabs.tab.viewers.widget).toBeUndefined();
    expect(ctx.getState().tabs.tab.selectedLeafId).toBe("p1");
    expect(document.activeElement).toBe(dialogControl);
    expect(focus).not.toHaveBeenCalled();
    expect(animation).not.toHaveBeenCalled();
  } finally { vi.unstubAllGlobals(); host.remove(); }
});

it.each(["agent", "retired", "snapshot", "user", "localUser"] as const)("preserves layout return selection and applies only user DOM restoration for %s", async (reason) => {
  const { ctx, store } = fixture({ widgetRemove: vi.fn().mockResolvedValue({ result: "removed" }) } as unknown as CockpitClient);
  store.accept({ type: "snapshot", sequence: 1, widgets: [summary] });
  ctx.dispatch({ type: "select-leaf", tabId: "tab", leafId: "tab:widget" });
  const host = document.createElement("div");
  host.innerHTML = '<div data-leaf-id="p1"><textarea></textarea></div><div data-widget-tab="tab" tabindex="0"></div>';
  document.body.append(host);
  const dock = host.querySelector<HTMLElement>("[data-widget-tab]")!;
  const terminal = host.querySelector("textarea")!;
  const focus = vi.spyOn(terminal, "focus");
  const animation = vi.fn((callback: FrameRequestCallback) => { callback(0); return 1; });
  vi.stubGlobal("requestAnimationFrame", animation);
  try {
    dock.focus();
    if (reason === "snapshot") store.accept({ type: "snapshot", sequence: 2, widgets: [] });
    else if (reason === "localUser") await store.remove(summary.key);
    else store.accept({ type: "removed", sequence: 2, key: summary.key, reason });
    expect(ctx.getState().tabs.tab.selectedLeafId).toBe("p1");
    const userRemoval = reason === "user" || reason === "localUser";
    expect(focus).toHaveBeenCalledTimes(userRemoval ? 1 : 0);
    expect(document.activeElement).toBe(userRemoval ? terminal : dock);
  } finally { vi.unstubAllGlobals(); host.remove(); }
});

it("selects only the local widget leaf after a live iframe reports actual focused interaction", async () => {
  const content: WidgetContent = { key: summary.key, revision: 1, sha256: summary.content.sha256,
    body: { type: "html", document: "<input>" }, selection: null };
  const mutate = vi.fn();
  const terminalInput = vi.fn();
  const client = { widgetContent: vi.fn().mockResolvedValue(content), mutate, terminalInput } as unknown as CockpitClient;
  const { ctx, store } = fixture(client);
  store.accept({ type: "snapshot", sequence: 1, widgets: [summary] });
  const host = document.createElement("div");
  document.body.append(host);
  const root = createRoot(host);
  const onSelect = vi.fn(() => ctx.dispatch({ type: "select-leaf", tabId: "tab", leafId: "tab:widget" }));
  try {
    await act(async () => root.render(<WidgetDock ctx={ctx} tab={ctx.getState().tabs.tab} width={600}
      selected={false} inputBlocked={false} live onSelect={onSelect} onZoom={() => {}} onGoToAgent={() => {}} />));
    const frame = host.querySelector<HTMLIFrameElement>("iframe")!;
    expect(ctx.getState().tabs.tab.selectedLeafId).toBe("p1");
    await act(async () => {
      frame.focus();
      window.dispatchEvent(new MessageEvent("message", { source: frame.contentWindow,
        data: { ...dockFrameIdentity(frame), type: "cockpit.widget.intent", nonce: "wrong" } }));
    });
    expect(onSelect).not.toHaveBeenCalled();
    await act(async () => {
      window.dispatchEvent(new MessageEvent("message", { source: frame.contentWindow,
        data: { ...dockFrameIdentity(frame), type: "cockpit.widget.intent" } }));
    });
    expect(document.activeElement).toBe(frame);
    expect(onSelect).toHaveBeenCalledOnce();
    expect(ctx.getState().tabs.tab.selectedLeafId).toBe("tab:widget");
    expect(mutate).not.toHaveBeenCalled();
    expect(terminalInput).not.toHaveBeenCalled();
  } finally { await act(async () => root.unmount()); host.remove(); }
});

it.each(["agent", "retired", "user"] as const)("uses actual header DOM ownership, not stale layout selection, for %s last removal", (reason) => {
  const { ctx, store } = fixture({} as CockpitClient);
  store.accept({ type: "snapshot", sequence: 1, widgets: [summary] });
  expect(ctx.getState().tabs.tab.selectedLeafId).toBe("p1");
  const host = document.createElement("div");
  host.innerHTML = '<div data-leaf-id="p1"><textarea></textarea></div><header data-pane-header="tab:widget"><button>Remove</button></header>';
  document.body.append(host);
  const remove = host.querySelector("button")!;
  const terminal = host.querySelector("textarea")!;
  const focus = vi.spyOn(terminal, "focus");
  vi.stubGlobal("requestAnimationFrame", (callback: FrameRequestCallback) => { callback(0); return 1; });
  try {
    remove.focus();
    store.accept({ type: "removed", sequence: 2, key: summary.key, reason });
    expect(ctx.getState().tabs.tab.selectedLeafId).toBe("p1");
    expect(focus).toHaveBeenCalledTimes(reason === "user" ? 1 : 0);
    expect(document.activeElement).toBe(reason === "user" ? terminal : remove);
  } finally { vi.unstubAllGlobals(); host.remove(); }
});

it.each([false, true])("retains explicit last-remove focus intent across disabled-header blur without overriding a new dialog (%s)", async (dialogOpened) => {
  const client = { widgetRemove: vi.fn().mockResolvedValue({ result: "removed" }) } as unknown as CockpitClient;
  const { ctx, store } = fixture(client);
  store.accept({ type: "snapshot", sequence: 1, widgets: [summary] });
  const host = document.createElement("div");
  host.innerHTML = '<div data-leaf-id="p1"><textarea></textarea></div><header data-pane-header="tab:widget"><button>Remove</button></header><section role="dialog" aria-modal="true"><button>Confirm</button></section>';
  document.body.append(host);
  const remove = host.querySelector<HTMLButtonElement>("header button")!;
  const dialog = host.querySelector<HTMLButtonElement>("section button")!;
  const terminal = host.querySelector("textarea")!;
  const focus = vi.spyOn(terminal, "focus");
  vi.stubGlobal("requestAnimationFrame", (callback: FrameRequestCallback) => { callback(0); return 1; });
  try {
    remove.focus();
    const pending = store.remove(summary.key);
    remove.disabled = true;
    remove.blur();
    if (dialogOpened) dialog.focus();
    await pending;
    expect(ctx.getState().tabs.tab.selectedLeafId).toBe("p1");
    expect(focus).toHaveBeenCalledTimes(dialogOpened ? 0 : 1);
    expect(document.activeElement).toBe(dialogOpened ? dialog : terminal);
  } finally { vi.unstubAllGlobals(); host.remove(); }
});
