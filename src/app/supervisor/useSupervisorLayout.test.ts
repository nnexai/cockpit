// @vitest-environment jsdom
import { act, createElement, useRef } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, describe, expect, it, vi } from "vitest";
import { panelBounds, panelPlacement, queueCap, useSupervisorLayout, type SupervisorLayout } from "./useSupervisorLayout";

Object.assign(globalThis, { IS_REACT_ACT_ENVIRONMENT: true });
let root: Root | null = null;
let host: HTMLDivElement;
afterEach(() => {
  if (root) act(() => root!.unmount());
  root = null; host?.remove(); vi.restoreAllMocks(); vi.unstubAllGlobals();
});

function mount(width: number, height: number, windowHeight: number) {
  let rect = { width, height };
  let resize: (() => void) | undefined;
  const disconnect = vi.fn();
  vi.stubGlobal("innerHeight", windowHeight);
  vi.spyOn(HTMLElement.prototype, "getBoundingClientRect").mockImplementation(() => rect as DOMRect);
  vi.stubGlobal("ResizeObserver", class {
    constructor(callback: () => void) { resize = callback; }
    observe() {}
    disconnect = disconnect;
  });
  let layout!: SupervisorLayout;
  function Workarea() {
    const ref = useRef<HTMLDivElement>(null);
    layout = useSupervisorLayout(ref);
    return createElement("div", { ref, "data-panel": panelPlacement(layout, "graph", "details"), "data-queue": layout.queueMode });
  }
  host = document.createElement("div"); document.body.append(host); root = createRoot(host);
  act(() => root!.render(createElement(Workarea)));
  return {
    layout: () => layout,
    disconnect,
    measure(nextWidth: number, nextHeight: number) { rect = { width: nextWidth, height: nextHeight }; act(() => resize!()); },
    resizeWindow(nextHeight: number) { vi.stubGlobal("innerHeight", nextHeight); act(() => window.dispatchEvent(new Event("resize"))); },
  };
}

describe("responsive Supervisor layout transitions", () => {
  it("changes panel and queue modes at measured width and independent window-height boundaries", () => {
    const fixture = mount(720, 560, 601);
    expect(host.firstElementChild?.getAttribute("data-panel")).toBe("side");
    expect(host.firstElementChild?.getAttribute("data-queue")).toBe("inline");
    fixture.measure(719, 560);
    expect(host.firstElementChild?.getAttribute("data-panel")).toBe("sheet");
    expect(host.firstElementChild?.getAttribute("data-queue")).toBe("overlay");
    expect(panelPlacement(fixture.layout(), "tasks", "details")).toBe("overlay");
    expect(panelPlacement(fixture.layout(), "graph", "activity")).toBe("overlay");
    expect(panelPlacement(fixture.layout(), "graph", "diagnostics")).toBe("overlay");
    fixture.measure(719, 559);
    expect(host.firstElementChild?.getAttribute("data-panel")).toBe("overlay");
    fixture.measure(720, 559); fixture.resizeWindow(600);
    expect(host.firstElementChild?.getAttribute("data-panel")).toBe("side");
    expect(host.firstElementChild?.getAttribute("data-queue")).toBe("overlay");
    fixture.resizeWindow(601);
    expect(host.firstElementChild?.getAttribute("data-queue")).toBe("inline");
    expect(panelPlacement(fixture.layout(), "graph", "attention")).toBe("overlay");
  });

  it("keeps an unmeasured workarea wide until observation, then crosses compact width separately", () => {
    const fixture = mount(0, 0, 800);
    expect(fixture.layout().measured).toBe(false);
    expect(fixture.layout().narrow).toBe(false);
    expect(fixture.layout().compact).toBe(false);
    const saved = { detailWidth: null, sheetHeight: null };
    expect(panelBounds(fixture.layout(), "side", saved)).toMatchObject({ min: 280, max: 340, value: 340 });
    fixture.measure(480, 600);
    expect(fixture.layout().narrow).toBe(true);
    expect(fixture.layout().compact).toBe(false);
    fixture.measure(479, 600);
    expect(fixture.layout().compact).toBe(true);
    act(() => root!.unmount()); root = null;
    expect(fixture.disconnect).toHaveBeenCalledOnce();
  });

  it("clamps restored sizes after shrinking without destroying the saved preference", () => {
    const fixture = mount(1200, 800, 1000);
    const saved = { detailWidth: 590, sheetHeight: 590 };
    expect(panelBounds(fixture.layout(), "side", saved)).toMatchObject({ min: 280, max: 600, value: 590 });
    fixture.measure(721, 800);
    expect(panelBounds(fixture.layout(), "side", saved)).toMatchObject({ max: 360, value: 360 });
    fixture.measure(1200, 800);
    expect(panelBounds(fixture.layout(), "side", saved)?.value).toBe(590);
    expect(panelBounds(fixture.layout(), "side", { detailWidth: 100, sheetHeight: null })?.value).toBe(280);
    fixture.measure(360, 561);
    expect(panelBounds(fixture.layout(), "sheet", saved)).toMatchObject({ orientation: "horizontal", min: 160, max: 420, defaultValue: 281, value: 420 });
    expect(panelBounds(fixture.layout(), "sheet", { detailWidth: null, sheetHeight: 100 })?.value).toBe(160);
    expect(panelBounds(fixture.layout(), "overlay", saved)).toBeNull();
    expect(queueCap(fixture.layout(), "graph")).toBe(168);
    expect(queueCap(fixture.layout(), "tasks")).toBe(224);
    expect(saved).toEqual({ detailWidth: 590, sheetHeight: 590 });
  });

  it("retains a complete node reveal area when the sheet ceiling shrinks below 75 percent", () => {
    const fixture = mount(360, 736, 800);
    const saved = { detailWidth: null, sheetHeight: 552 };
    expect(panelBounds(fixture.layout(), "sheet", saved, 510)).toMatchObject({ min: 160, max: 510, value: 510 });
    expect(panelBounds(fixture.layout(), "sheet", saved, 700)).toMatchObject({ max: 552, value: 552 });
    expect(panelBounds(fixture.layout(), "sheet", saved, 300)).toMatchObject({ max: 300, defaultValue: 300, value: 300 });
    expect(panelBounds(fixture.layout(), "sheet", saved, 509.609)).toMatchObject({ max: 509, value: 509 });
    expect(panelBounds(fixture.layout(), "sheet", saved, 160)).toMatchObject({ min: 160, max: 160, defaultValue: 160, value: 160 });
    expect(panelBounds(fixture.layout(), "sheet", saved, 159.9)).toBeNull();
    expect(panelBounds(fixture.layout(), "sheet", saved, null)?.max).toBe(552);
    expect(saved.sheetHeight).toBe(552);
  });

  it("remeasures on window resize when ResizeObserver is unavailable", () => {
    vi.stubGlobal("ResizeObserver", undefined);
    let rect = { width: 720, height: 700 };
    vi.stubGlobal("innerHeight", 800);
    vi.spyOn(HTMLElement.prototype, "getBoundingClientRect").mockImplementation(() => rect as DOMRect);
    function Workarea() {
      const ref = useRef<HTMLDivElement>(null), layout = useSupervisorLayout(ref);
      return createElement("div", { ref, "data-panel": panelPlacement(layout, "graph", "details") });
    }
    host = document.createElement("div"); document.body.append(host); root = createRoot(host);
    act(() => root!.render(createElement(Workarea)));
    rect = { width: 360, height: 560 };
    act(() => window.dispatchEvent(new Event("resize")));
    expect(host.firstElementChild?.getAttribute("data-panel")).toBe("sheet");
  });
});
