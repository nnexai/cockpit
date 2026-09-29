// @vitest-environment jsdom
import { act } from "react";
import { createRoot } from "react-dom/client";
import { afterEach, beforeEach, expect, it, vi } from "vitest";
import { LIBRARY_TREE, TreeSplitter } from "./ViewerLayout";

Object.assign(globalThis, { IS_REACT_ACT_ENVIRONMENT: true });

beforeEach(() => {
  vi.useFakeTimers();
  Element.prototype.setPointerCapture = vi.fn();
});
afterEach(() => vi.useRealTimers());

function mount(width: number, shown?: number) {
  const onChange = vi.fn();
  const body = document.createElement("div");
  document.body.append(body);
  const root = createRoot(body);
  act(() => root.render(<><nav /><TreeSplitter width={width} onChange={onChange} layout={LIBRARY_TREE} /></>));
  const splitter = body.querySelector<HTMLElement>('[role="separator"]')!;
  // jsdom lays nothing out: report the width the grid column really gave the list.
  if (shown !== undefined) {
    vi.spyOn(splitter.previousElementSibling!, "getBoundingClientRect").mockReturnValue({ width: shown } as DOMRect);
  }
  return { body, splitter, onChange, unmount: () => { act(() => root.unmount()); body.remove(); } };
}

// jsdom has no PointerEvent: a mouse event carrying the pointer id is what React's handlers read.
const pointer = (target: Element, type: string, clientX: number) => act(() => { const event = new MouseEvent(type, { bubbles: true, button: 0, clientX }); Object.defineProperty(event, "pointerId", { value: 1 }); target.dispatchEvent(event); });
const key = (target: Element, name: string, shiftKey = false) => act(() => { target.dispatchEvent(new KeyboardEvent("keydown", { bubbles: true, key: name, shiftKey })); });

it("drags the Library tree splitter and reports the final width once", () => {
  const { body, splitter, onChange, unmount } = mount(296);
  pointer(splitter, "pointerdown", 300);
  pointer(splitter, "pointermove", 340);
  pointer(splitter, "pointermove", 360);
  act(() => { vi.advanceTimersByTime(32); });
  expect(body.style.getPropertyValue("--viewer-tree-width")).toBe("356");
  expect(onChange).not.toHaveBeenCalled();
  pointer(splitter, "pointerup", 360);
  expect(onChange).toHaveBeenCalledExactlyOnceWith(356);
  unmount();
});

it("leaves the layout at the released width even when the pending frame was dropped", () => {
  const { splitter, onChange, unmount } = mount(296);
  pointer(splitter, "pointerdown", 300);
  pointer(splitter, "pointermove", 340);
  act(() => { vi.advanceTimersByTime(32); });
  pointer(splitter, "pointermove", 300);
  pointer(splitter, "pointerup", 300);
  expect(onChange).toHaveBeenCalledExactlyOnceWith(296);
  // React sees no change from 296, so only the direct write can undo the frame that showed 336.
  expect(splitter.parentElement?.style.getPropertyValue("--viewer-tree-width")).toBe("296");
  expect(splitter.getAttribute("aria-valuenow")).toBe("296");
  unmount();
});

it("resizes from the width the list shows when a narrow viewer caps the stored one", () => {
  const { splitter, onChange, unmount } = mount(395, 344);
  pointer(splitter, "pointerdown", 344);
  pointer(splitter, "pointermove", 324);
  pointer(splitter, "pointerup", 324);
  expect(onChange).toHaveBeenLastCalledWith(324);
  key(splitter, "ArrowRight");
  expect(onChange).toHaveBeenLastCalledWith(360);
  unmount();
});

it("steps with the arrow keys, resets with Home and never leaves the allowed range", () => {
  const { splitter, onChange, unmount } = mount(296);
  key(splitter, "ArrowLeft");
  expect(onChange).toHaveBeenLastCalledWith(280);
  key(splitter, "ArrowRight", true);
  expect(onChange).toHaveBeenLastCalledWith(344);
  key(splitter, "Home");
  expect(onChange).toHaveBeenLastCalledWith(LIBRARY_TREE.fallback);
  unmount();
  const small = mount(LIBRARY_TREE.min);
  key(small.splitter, "ArrowLeft", true);
  expect(small.onChange).toHaveBeenLastCalledWith(LIBRARY_TREE.min);
  small.unmount();
});
