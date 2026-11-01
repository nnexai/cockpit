// @vitest-environment jsdom
import { act } from "react";
import { createRoot } from "react-dom/client";
import { afterEach, beforeEach, expect, it, vi } from "vitest";
import { useVisibleViewport } from "./useVisibleViewport";

Object.assign(globalThis, { IS_REACT_ACT_ENVIRONMENT: true });

function Viewport() {
  useVisibleViewport();
  return null;
}

let frames: Map<number, FrameRequestCallback>;
let nextFrame: number;
beforeEach(() => {
  frames = new Map();
  nextFrame = 0;
  vi.spyOn(window, "requestAnimationFrame").mockImplementation(callback => {
    frames.set(++nextFrame, callback);
    return nextFrame;
  });
  vi.spyOn(window, "cancelAnimationFrame").mockImplementation(id => { frames.delete(id); });
});
afterEach(() => {
  vi.restoreAllMocks();
  vi.unstubAllGlobals();
});

function flushFrame() {
  const pending = [...frames.values()];
  frames.clear();
  for (const callback of pending) callback(0);
}

function mount() {
  const container = document.createElement("div");
  document.body.append(container);
  const root = createRoot(container);
  act(() => root.render(<Viewport />));
  return () => { act(() => root.unmount()); container.remove(); };
}

it("coalesces viewport changes, freezes during zoom and cleans up", () => {
  const viewport = Object.assign(new EventTarget(), { height: 844, offsetTop: 0, scale: 1 });
  vi.stubGlobal("visualViewport", viewport);
  const scroll = vi.spyOn(window, "scrollTo").mockImplementation(() => { vi.stubGlobal("scrollY", 0); });
  const unmount = mount();
  const style = document.documentElement.style;
  expect(frames.size).toBe(1);
  flushFrame();
  expect(style.getPropertyValue("--app-height")).toBe("844px");
  expect(style.getPropertyValue("--app-top")).toBe("0px");

  viewport.height = 480;
  viewport.offsetTop = 24;
  viewport.dispatchEvent(new Event("resize"));
  viewport.dispatchEvent(new Event("scroll"));
  expect(frames.size).toBe(1);
  flushFrame();
  expect(style.getPropertyValue("--app-height")).toBe("480px");
  expect(style.getPropertyValue("--app-top")).toBe("24px");

  viewport.height = 480.5;
  viewport.offsetTop = 24.5;
  viewport.dispatchEvent(new Event("resize"));
  flushFrame();
  expect(style.getPropertyValue("--app-height")).toBe("480px");
  expect(style.getPropertyValue("--app-top")).toBe("24px");

  viewport.scale = 2;
  viewport.height = 240;
  viewport.offsetTop = 50;
  vi.stubGlobal("scrollY", 30);
  viewport.dispatchEvent(new Event("resize"));
  flushFrame();
  expect(style.getPropertyValue("--app-height")).toBe("480px");
  expect(style.getPropertyValue("--app-top")).toBe("24px");
  expect(window.scrollY).toBe(30);
  viewport.scale = 1;
  viewport.height = 600;
  viewport.offsetTop = 8;
  viewport.dispatchEvent(new Event("resize"));
  flushFrame();
  expect(style.getPropertyValue("--app-height")).toBe("600px");
  expect(style.getPropertyValue("--app-top")).toBe("8px");
  expect(window.scrollY).toBe(0);

  vi.stubGlobal("scrollY", 30);
  document.dispatchEvent(new FocusEvent("focusin", { bubbles: true }));
  flushFrame();
  expect(scroll).toHaveBeenCalledWith(0, 0);
  viewport.dispatchEvent(new Event("resize"));
  expect(frames.size).toBe(1);
  unmount();
  expect(frames.size).toBe(0);
  expect(style.getPropertyValue("--app-height")).toBe("");
  expect(style.getPropertyValue("--app-top")).toBe("");
  viewport.dispatchEvent(new Event("resize"));
  document.dispatchEvent(new FocusEvent("focusin"));
  window.dispatchEvent(new Event("resize"));
  expect(frames.size).toBe(0);
});

it("uses the window height when VisualViewport is unavailable", () => {
  vi.stubGlobal("visualViewport", undefined);
  vi.stubGlobal("innerHeight", 700);
  const unmount = mount();
  flushFrame();
  expect(document.documentElement.style.getPropertyValue("--app-height")).toBe("700px");
  expect(document.documentElement.style.getPropertyValue("--app-top")).toBe("0px");
  vi.stubGlobal("innerHeight", 500);
  window.dispatchEvent(new Event("resize"));
  flushFrame();
  expect(document.documentElement.style.getPropertyValue("--app-height")).toBe("500px");
  unmount();
});
