import { describe, expect, it } from "vitest";
import { decideArrival, type ArrivalInput } from "./arrival";

const own: ArrivalInput = { change: "opened", arrival: "own_tab", tabDisplayed: true, blocker: null,
  dockPresent: false, isCurrent: false, focusInsideFrame: false };

const decide = (input: Partial<ArrivalInput> = {}) => decideArrival({ ...own, ...input });

describe("widget arrivals", () => {
  it.each(["opened", "reopened"] as const)("docks an own-tab %s (F1, first-window F7, intentional F9 reopen)", change => {
    // F7 stores without a window; its first window applies this same own-tab policy.
    // F9 refusal emits no event; only an intentional --reopen reaches the frontend.
    expect(decide({ change })).toEqual({ kind: "dock_now", makeCurrent: true, announce: true });
  });

  it.each([
    ["library", "none"], ["drag", "none"], ["zoom", "widgets_button"], ["too_narrow", "widgets_button"],
  ] as const)("defers behind %s and exposes the appropriate indicator (F2)", (blocker, indicator) => {
    expect(decide({ blocker })).toEqual({ kind: "defer_until_clear", indicator });
  });

  it("keeps background tabs and Spaces pending until displayed (F3/F4)", () => {
    expect(decide({ tabDisplayed: false })).toEqual({ kind: "wait_tab", announce: true });
    expect(decide({ tabDisplayed: false, blocker: "zoom" })).toEqual({ kind: "wait_tab", announce: true });
  });

  it.each([true, false])("requires a click for cross-source arrivals, displayed=%s (F5/F6 and first-window F7)", tabDisplayed => {
    expect(decide({ arrival: "cross_source", tabDisplayed })).toEqual({ kind: "wait_click", announce: true });
    expect(decide({ arrival: "cross_source", tabDisplayed, dockPresent: true, blocker: "drag" }))
      .toEqual({ kind: "wait_click", announce: true });
  });

  it.each(["own_tab", "cross_source"] as const)("replaces %s widgets without docking, announcing or changing current (F8)", arrival => {
    expect(decide({ change: "replaced", arrival, dockPresent: false, tabDisplayed: false, blocker: "library" }))
      .toEqual({ kind: "replace_in_place", markUnseen: false });
    expect(decide({ change: "replaced", arrival, dockPresent: true, isCurrent: true, focusInsideFrame: true }))
      .toEqual({ kind: "replace_in_place", markUnseen: false });
    expect(decide({ change: "replaced", arrival, dockPresent: true, isCurrent: false }))
      .toEqual({ kind: "replace_in_place", markUnseen: true });
  });

  it("keeps the current widget when focus is inside its frame (W-22)", () => {
    expect(decide({ dockPresent: true, focusInsideFrame: true }))
      .toEqual({ kind: "dock_now", makeCurrent: false, announce: true });
    expect(decide({ dockPresent: true, focusInsideFrame: false }))
      .toEqual({ kind: "dock_now", makeCurrent: true, announce: true });
    expect(decide({ dockPresent: false, focusInsideFrame: true }))
      .toEqual({ kind: "dock_now", makeCurrent: true, announce: true });
  });

  it("does not present owner metadata or selection updates as arrivals", () => {
    expect(decide({ change: "updated", arrival: "cross_source" })).toEqual({ kind: "none" });
    expect(decide({ change: "updated", dockPresent: true, isCurrent: false })).toEqual({ kind: "none" });
  });
});
