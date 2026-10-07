// @vitest-environment jsdom
import { act, useState } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, describe, expect, it, vi } from "vitest";
import { PanelSplitter } from "./PanelSplitter";

Object.assign(globalThis, { IS_REACT_ACT_ENVIRONMENT: true });
let root: Root | null = null;
let host: HTMLDivElement;
afterEach(() => {
  if (root) act(() => root!.unmount());
  root = null;
  host?.remove();
  vi.restoreAllMocks();
  vi.unstubAllGlobals();
});

function mount(orientation: "vertical" | "horizontal", grow: 1 | -1 = -1) {
  host = document.createElement("div"); document.body.append(host); root = createRoot(host);
  const reset = vi.fn();
  function Panel() {
    const [value, setValue] = useState(340);
    return <><button data-invoker>Task</button><PanelSplitter label="Resize details" controls="details" orientation={orientation} grow={grow}
      min={280} max={400} value={value} onChange={setValue} onReset={() => { reset(); setValue(340); }} /><aside id="details" style={{ [orientation === "vertical" ? "width" : "height"]: value }} /></>;
  }
  act(() => root!.render(<Panel />));
  const splitter = host.querySelector<HTMLDivElement>('[role="separator"]')!;
  const captured = new Set<number>();
  const capture = vi.fn((id: number) => { captured.add(id); });
  const release = vi.fn((id: number) => { captured.delete(id); });
  Object.assign(splitter, { setPointerCapture: capture, hasPointerCapture: (id: number) => captured.has(id), releasePointerCapture: release });
  return { splitter, reset, capture, release };
}
function key(target: HTMLElement, name: string, shiftKey = false) {
  const event = new KeyboardEvent("keydown", { bubbles: true, cancelable: true, key: name, shiftKey });
  act(() => { target.dispatchEvent(event); });
  return event;
}
function pointer(target: HTMLElement, type: string, coordinate: number, id = 1, button = 0) {
  const event = new MouseEvent(type, { bubbles: true, cancelable: true, button, clientX: coordinate, clientY: coordinate });
  Object.defineProperty(event, "pointerId", { value: id });
  act(() => { target.dispatchEvent(event); });
}

describe("details splitter interactions", () => {
  it.each([
    ["vertical", "ArrowLeft", "ArrowRight", "ArrowUp", "width"],
    ["horizontal", "ArrowUp", "ArrowDown", "ArrowLeft", "height"],
  ] as const)("resizes %s details with keyboard, retaining focus and reporting the applied geometry", (orientation, growKey, shrinkKey, ignoredKey, axis) => {
    const { splitter, reset } = mount(orientation);
    act(() => splitter.focus());
    expect(key(splitter, growKey).defaultPrevented).toBe(true);
    expect(host.querySelector("aside")!.style[axis]).toBe("356px");
    expect(splitter.getAttribute("aria-valuenow")).toBe("356");
    expect(splitter.getAttribute("aria-valuetext")).toBe("356 px");
    expect(splitter.getAttribute("aria-valuemin")).toBe("280");
    expect(splitter.getAttribute("aria-valuemax")).toBe("400");
    key(splitter, growKey, true);
    expect(host.querySelector("aside")!.style[axis]).toBe("400px");
    key(splitter, shrinkKey, true); key(splitter, shrinkKey, true); key(splitter, shrinkKey, true);
    expect(host.querySelector("aside")!.style[axis]).toBe("280px");
    expect(key(splitter, ignoredKey).defaultPrevented).toBe(false);
    expect(document.activeElement).toBe(splitter);
    key(splitter, "Home");
    expect(host.querySelector("aside")!.style[axis]).toBe("340px");
    key(splitter, growKey);
    act(() => { splitter.dispatchEvent(new MouseEvent("dblclick", { bubbles: true })); });
    expect(host.querySelector("aside")!.style[axis]).toBe("340px");
    expect(reset).toHaveBeenCalledTimes(2);
  });

  it.each(["vertical", "horizontal"] as const)("captures %s drag and keeps its starting value across controlled rerenders", orientation => {
    const { splitter, capture, release } = mount(orientation);
    const invoker = host.querySelector<HTMLButtonElement>("button")!;
    act(() => invoker.focus());
    pointer(splitter, "pointerdown", 100, 2, 2);
    expect(capture).not.toHaveBeenCalled();
    pointer(splitter, "pointerdown", 100);
    expect(capture).toHaveBeenCalledWith(1);
    pointer(splitter, "pointermove", 80, 2);
    expect(splitter.getAttribute("aria-valuenow")).toBe("340");
    pointer(splitter, "pointermove", 80);
    expect(splitter.getAttribute("aria-valuenow")).toBe("360");
    pointer(splitter, "pointermove", 70);
    expect(splitter.getAttribute("aria-valuenow")).toBe("370");
    pointer(splitter, "pointermove", 0);
    expect(splitter.getAttribute("aria-valuenow")).toBe("400");
    pointer(splitter, "pointerup", 0);
    expect(release).toHaveBeenCalledWith(1);
    pointer(splitter, "pointermove", 200);
    expect(splitter.getAttribute("aria-valuenow")).toBe("400");
    expect(document.activeElement).toBe(invoker);
    expect(document.body.classList.contains("is-resizing-panes")).toBe(false);
  });

  it.each(["pointercancel", "lostpointercapture", "unmount"])("ends drag on %s without leaving the resize cursor or accepting moves", end => {
    const { splitter } = mount("horizontal");
    pointer(splitter, "pointerdown", 100);
    expect(document.body.classList.contains("is-resizing-panes")).toBe(true);
    if (end === "unmount") { act(() => root!.unmount()); root = null; }
    else { pointer(splitter, end, 100); pointer(splitter, "pointermove", 0); expect(splitter.getAttribute("aria-valuenow")).toBe("340"); }
    expect(document.body.classList.contains("is-resizing-panes")).toBe(false);
  });

  it("uses the declared positive grow direction", () => {
    const { splitter } = mount("vertical", 1);
    key(splitter, "ArrowRight");
    expect(splitter.getAttribute("aria-valuenow")).toBe("356");
    pointer(splitter, "pointerdown", 100); pointer(splitter, "pointermove", 110);
    expect(splitter.getAttribute("aria-valuenow")).toBe("366");
  });
});
