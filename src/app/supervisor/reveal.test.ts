// @vitest-environment jsdom
import { afterEach, describe, expect, it, vi } from "vitest";
import { isCovered, nearestScroll, readOffset, revealNearest, writeOffset, type Insets } from "./reveal";

const viewport = { left: 100, top: 200, width: 400, height: 300 };
const noInsets: Insets = { top: 0, right: 0, bottom: 0, left: 0 };
const current = { left: 200, top: 300 };
afterEach(() => { document.body.replaceChildren(); vi.restoreAllMocks(); });

describe("nearest single-port reveal", () => {
  it.each([
    [{ left: 120, top: 240, width: 240, height: 48 }, current],
    [{ left: 120, top: 180, width: 240, height: 48 }, { left: 200, top: 280 }],
    [{ left: 120, top: 480, width: 240, height: 48 }, { left: 200, top: 328 }],
    [{ left: 80, top: 240, width: 240, height: 48 }, { left: 180, top: 300 }],
    [{ left: 400, top: 240, width: 240, height: 48 }, { left: 340, top: 300 }],
  ])("moves only the axis that clips target %j", (target, expected) => {
    expect(nearestScroll(viewport, target, noInsets, current)).toEqual(expected);
  });

  it("accounts for sticky header, sheet and horizontal insets at their exact boundaries", () => {
    const insets = { top: 28, bottom: 160, left: 12, right: 8 };
    expect(nearestScroll(viewport, { left: 112, top: 228, width: 240, height: 48 }, insets, current)).toEqual(current);
    expect(nearestScroll(viewport, { left: 112, top: 227, width: 240, height: 48 }, insets, current)).toEqual({ left: 200, top: 299 });
    expect(nearestScroll(viewport, { left: 112, top: 300, width: 240, height: 48 }, insets, current)).toEqual({ left: 200, top: 308 });
    expect(nearestScroll(viewport, { left: 260, top: 228, width: 240, height: 48 }, insets, current)).toEqual({ left: 208, top: 300 });
  });

  it("aligns an oversized target's start and never requests negative offsets", () => {
    expect(nearestScroll(viewport, { left: 120, top: 240, width: 600, height: 400 }, noInsets, current)).toEqual({ left: 220, top: 340 });
    expect(nearestScroll(viewport, { left: -300, top: -300, width: 240, height: 48 }, noInsets, current)).toEqual({ left: 0, top: 0 });
  });

  it("reveals in its own client scrollport without scrolling or focusing an ancestor", () => {
    const ancestor = document.createElement("div"), scroller = document.createElement("div"), target = document.createElement("button");
    document.body.append(ancestor); ancestor.append(scroller); scroller.append(target);
    writeOffset(ancestor, { left: 91, top: 72 });
    writeOffset(scroller, current);
    Object.defineProperties(scroller, { clientLeft: { value: 2 }, clientTop: { value: 3 }, clientWidth: { value: 380 }, clientHeight: { value: 280 } });
    vi.spyOn(scroller, "getBoundingClientRect").mockReturnValue(viewport as DOMRect);
    const targetRect = vi.spyOn(target, "getBoundingClientRect").mockReturnValue({ left: 400, top: 450, width: 240, height: 48 } as DOMRect);
    const ancestorReveal = vi.fn(() => { throw new Error("Ancestor reveal is forbidden"); });
    Object.assign(target, { scrollIntoView: ancestorReveal });
    const invoker = document.createElement("button"); ancestor.append(invoker); invoker.focus();
    expect(isCovered(scroller, target, { top: 28, bottom: 100 })).toBe(true);
    expect(revealNearest(scroller, target, { top: 28, bottom: 100 })).toBe(true);
    expect(readOffset(scroller)).toEqual({ left: 358, top: 415 });
    expect(readOffset(ancestor)).toEqual({ left: 91, top: 72 });
    expect(ancestorReveal).not.toHaveBeenCalled();
    expect(document.activeElement).toBe(invoker);
    targetRect.mockReturnValue({ left: 110, top: 240, width: 240, height: 48 } as DOMRect);
    expect(isCovered(scroller, target, { top: 28, bottom: 100 })).toBe(false);
    expect(revealNearest(scroller, target, { top: 28, bottom: 100 })).toBe(false);
    expect(readOffset(scroller)).toEqual({ left: 358, top: 415 });
    expect(revealNearest(scroller, invoker)).toBe(false);
  });

  it("waits for a measurable client box before treating a target as covered", () => {
    const scroller = document.createElement("div"), target = document.createElement("button"); scroller.append(target);
    expect(isCovered(scroller, target)).toBe(false);
    expect(revealNearest(scroller, target)).toBe(false);
  });
});
