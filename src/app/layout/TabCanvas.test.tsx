// @vitest-environment jsdom
import { act } from "react";
import { createRoot } from "react-dom/client";
import { afterEach, beforeEach, expect, it, vi } from "vitest";
import { TabCanvas, focusSelectedDivider } from "./TabCanvas";
import { PaneChrome } from "./PaneChrome";
import { createSessionLayoutState, layoutReducer, type LayoutAction, type SessionLayoutState, type TabLayoutState } from "./tabLayoutStore";
import { leaves, type LayoutNode } from "./splitTree";

Object.assign(globalThis, { IS_REACT_ACT_ENVIRONMENT: true });
const cleanups: (() => void)[] = [];
beforeEach(() => {
  vi.useFakeTimers();
  vi.stubGlobal("requestAnimationFrame", (callback: FrameRequestCallback) => window.setTimeout(() => callback(0), 16));
  vi.stubGlobal("cancelAnimationFrame", (frame: number) => window.clearTimeout(frame));
});
afterEach(() => {
  for (const cleanup of cleanups.splice(0)) cleanup();
  vi.useRealTimers(); vi.unstubAllGlobals();
});

function mount({ zoomed = false, single = false } = {}) {
  const tree: LayoutNode = { t: "split", id: "row", dir: "row", w: 1, kids: ["a", "b", "c"].map(id => ({ t: "leaf", id, kind: "terminal", w: 1 })) };
  const tab: TabLayoutState = { tabId: "tab", spaceId: "space", root: single ? tree.kids[0] : tree,
    terminals: single ? { a: "ta" } : { a: "ta", b: "tb", c: "tc" }, selectedLeafId: "a", lastRealLeafId: "a", zoomLeafId: zoomed ? "a" : null,
    viewers: {}, heldMembers: [], heldTerminalIds: {}, bufferedFocus: null, focusedPaneId: "a", revision: 0, selectionRevision: 0 };
  let state: SessionLayoutState = { ...createSessionLayoutState("session", "server"), tabs: { tab }, activeTabId: "tab", activeSpaceId: "space" };
  const body = document.createElement("div"); document.body.append(body);
  const root = createRoot(body);
  const registerTransient = (_cancel: () => void) => () => {};
  const dispatch = (action: LayoutAction) => { state = layoutReducer(state, action).state; render(); };
  function render() {
    root.render(<TabCanvas tab={state.tabs.tab} area={{ x: 0, y: 0, width: 1200, height: 800 }} dispatch={dispatch} registerTransient={registerTransient} announce={() => {}} renderLeaf={leaf => <>
      <PaneChrome leaf={leaf} title={`Terminal ${leaf.id}`} selected={state.tabs.tab.selectedLeafId === leaf.id} zoomed={state.tabs.tab.zoomLeafId === leaf.id} onClose={() => {}} onZoom={() => dispatch({ type: "zoom-toggle", tabId: "tab", leafId: leaf.id })} />
      <textarea aria-label={`Editor ${leaf.id}`} defaultValue={`draft ${leaf.id}`} />
    </>} />);
  }
  act(render);
  vi.spyOn(body.querySelector(".tab-canvas")!, "getBoundingClientRect").mockReturnValue({ left: 0, top: 0, right: 1200, bottom: 800, width: 1200, height: 800 } as DOMRect);
  cleanups.push(() => { act(() => root.unmount()); body.remove(); });
  return { body, getTab: () => state.tabs.tab, dispatch: (action: LayoutAction) => act(() => dispatch(action)) };
}
function pointer(target: EventTarget, type: string, clientX: number, clientY = 400) {
  act(() => {
    const event = new MouseEvent(type, { bubbles: true, cancelable: true, button: 0, clientX, clientY });
    Object.defineProperty(event, "pointerId", { value: 1 }); target.dispatchEvent(event);
  });
}
function key(target: EventTarget, name: string, shiftKey = false) {
  act(() => { target.dispatchEvent(new KeyboardEvent("keydown", { bubbles: true, cancelable: true, key: name, shiftKey })); });
}

it.each([
  ["centre swap", 600, ["b", "a", "c"], "swap"],
  ["pane edge", 780, ["b", "a", "c"], "edge"],
  ["outer rim", 1195, ["b", "c", "a"], "root"],
] as const)("preserves editor DOM, text and focus after %s", (_name, x, expectedOrder, kind) => {
  const { body, getTab } = mount();
  const editors = Array.from(body.querySelectorAll("textarea"));
  const hosts = Array.from(body.querySelectorAll("[data-leaf-id]"));
  editors[0].value = "unsaved changed draft"; editors[0].focus();
  pointer(body.querySelector('[data-pane-header="a"]')!, "pointerdown", 100, 10);
  pointer(window, "pointermove", x);
  expect(body.querySelector<HTMLElement>(".pane-drop-preview")!.dataset.dropKind).toBe(kind);
  pointer(window, "pointerup", x);
  expect(leaves(getTab().root).map(leaf => leaf.id)).toEqual(expectedOrder);
  expect(Array.from(body.querySelectorAll("[data-leaf-id]"))).toEqual(hosts);
  for (let i = 0; i < editors.length; i++) expect(body.querySelectorAll("textarea")[i]).toBe(editors[i]);
  expect(editors[0].value).toBe("unsaved changed draft");
  expect(document.activeElement).toBe(editors[0]);
  expect(body.querySelector(".pane-drop-preview")).toBeNull();
});

it("cancels an armed header drop with Escape and never drags from content, a lone pane or zoom", () => {
  const normal = mount(); const tree = normal.getTab().root;
  pointer(normal.body.querySelector("textarea")!, "pointerdown", 100);
  pointer(window, "pointermove", 600); expect(document.querySelector(".pane-drag-ghost")).toBeNull();
  pointer(window, "pointerup", 600);
  pointer(normal.body.querySelector('[data-pane-header="a"]')!, "pointerdown", 100, 10);
  pointer(window, "pointermove", 600); expect(document.querySelector(".pane-drag-ghost")).not.toBeNull();
  key(window, "Escape");
  expect(normal.getTab().root).toBe(tree); expect(document.querySelector(".pane-drag-ghost")).toBeNull();
  for (const fixture of [mount({ zoomed: true }), mount({ single: true })]) {
    pointer(fixture.body.querySelector('[data-pane-header="a"]')!, "pointerdown", 100, 10);
    pointer(window, "pointermove", 600); expect(document.querySelector(".pane-drag-ghost")).toBeNull(); pointer(window, "pointerup", 600);
  }
});

it("resizes live without committing until release, preserves content and supports keyboard/reset", () => {
  const { body, getTab } = mount(); const originalTree = getTab().root;
  const editor = body.querySelector("textarea")!;
  const host = body.querySelector<HTMLElement>('[data-leaf-id="a"]')!;
  const initialWidth = parseFloat(host.style.width);
  const divider = body.querySelector<HTMLElement>('[role="separator"]')!;
  pointer(divider, "pointerdown", 397);
  pointer(window, "pointermove", 493);
  act(() => { vi.advanceTimersByTime(16); });
  expect(parseFloat(host.style.width)).toBeGreaterThan(initialWidth);
  expect(getTab().root).toBe(originalTree);
  pointer(window, "pointerup", 493);
  expect(getTab().root).not.toBe(originalTree);
  expect(body.querySelector("textarea")).toBe(editor);
  const changedWidth = parseFloat(host.style.width);
  key(divider, "ArrowRight", true);
  expect(parseFloat(host.style.width)).toBeGreaterThan(changedWidth);
  const beforePerpendicular = getTab().root;
  key(divider, "ArrowUp");
  expect(getTab().root).toBe(beforePerpendicular);
  const beforeReset = getTab().root;
  act(() => { divider.dispatchEvent(new MouseEvent("dblclick", { bubbles: true })); });
  expect(getTab().root).not.toBe(beforeReset);
  const pair = leaves(getTab().root); expect(pair[0].w).toBeCloseTo(pair[1].w);
  expect(focusSelectedDivider(body.querySelector(".tab-canvas")!, "a")).toBe(true);
  expect(document.activeElement).toBe(divider);
  key(divider, "Escape"); expect(document.activeElement).toBe(editor);
});

it("keeps hidden leaf hosts through zoom and restores only from chrome Escape", () => {
  const { body, getTab, dispatch } = mount();
  const hostA = body.querySelector<HTMLElement>('[data-leaf-id="a"]')!;
  const hostB = body.querySelector<HTMLElement>('[data-leaf-id="b"]')!;
  const editorA = hostA.querySelector("textarea")!;
  dispatch({ type: "zoom-toggle", tabId: "tab", leafId: "a" });
  expect(hostB.hidden).toBe(true); expect(hostB.querySelector("textarea")).toBeNull();
  expect(hostA.querySelector("textarea")).toBe(editorA);
  expect(body.querySelector(".pane-zoom-bar")!.textContent).toContain("Zoomed: Terminal a.");
  key(editorA, "Escape"); expect(getTab().zoomLeafId).toBe("a");
  key(hostA.querySelector("button")!, "Escape"); expect(getTab().zoomLeafId).toBeNull();
  expect(body.querySelector('[data-leaf-id="b"]')).toBe(hostB); expect(hostB.hidden).toBe(false);
});
