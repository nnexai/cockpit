import { describe, expect, it } from "vitest";
import { firstLoadGrid, type Leaf, type LayoutNode } from "./splitTree";
import { computeDrop, minSize, solveLayout, type Rect } from "./solveLayout";

const area = { x: 0, y: 0, width: 1000, height: 600 };
const leaf = (id: string, w = 1): Leaf => ({ t: "leaf", id, kind: "terminal", w });
const split = (id: string, dir: "row" | "col", kids: LayoutNode[], w = 1): LayoutNode => ({ t: "split", id, dir, kids, w });

describe("layout geometry", () => {
  it("freezes nested minima and gives the final child the pixel remainder", () => {
    const root: LayoutNode = { t: "split", id: "root", dir: "row", w: 1, kids: [leaf("a", .1), leaf("b", .9)] };
    const solved = solveLayout(root, area);
    expect(solved.leaves.get("a")).toEqual({ x: 0, y: 0, width: 160, height: 600 });
    expect(solved.leaves.get("b")).toEqual({ x: 168, y: 0, width: 832, height: 600 });
    expect(solved.dividers[0].rect).toEqual({ x: 160, y: 0, width: 8, height: 600 });
    expect(solved.degraded).toBe(false);
  });

  it("combines recursive minima by summing the split axis and taking the cross-axis maximum", () => {
    const nested = split("root", "row", [
      leaf("a"),
      split("column", "col", [split("wide", "row", [leaf("b"), leaf("c"), leaf("d")]), leaf("e"), leaf("f")]),
    ]);
    expect(minSize(leaf("single"), "x")).toBe(160);
    expect(minSize(leaf("single"), "y")).toBe(100);
    expect(minSize(nested, "x")).toBe(664);
    expect(minSize(nested, "y")).toBe(316);
  });

  it.each([
    [leaf("a"), 160, 100, false],
    [leaf("a"), 159, 100, true],
    [leaf("a"), 160, 99, true],
    [split("row", "row", [leaf("a"), leaf("b")]), 328, 100, false],
    [split("row", "row", [leaf("a"), leaf("b")]), 327, 100, true],
    [split("row", "row", [leaf("a"), leaf("b")]), 328, 99, true],
    [split("col", "col", [leaf("a"), leaf("b")]), 160, 208, false],
    [split("col", "col", [leaf("a"), leaf("b")]), 159, 208, true],
    [split("col", "col", [leaf("a"), leaf("b")]), 160, 207, true],
  ])("marks degradation only below either minimum: %j, %i × %i", (root, width, height, degraded) => {
    expect(solveLayout(root, { x: 7, y: 11, width, height }).degraded).toBe(degraded);
  });

  it("redistributes the remaining weight after freezing a minimum in a three-pane row", () => {
    const root = split("row", "row", [leaf("a", .1), leaf("b", .2), leaf("c", .7)]);
    const solved = solveLayout(root, { x: 7, y: 11, width: 1000, height: 600 });
    expect([...solved.leaves.values()]).toEqual([
      { x: 7, y: 11, width: 160, height: 600 },
      { x: 175, y: 11, width: 183, height: 600 },
      { x: 366, y: 11, width: 641, height: 600 },
    ]);
    expect(solved.dividers.map(handle => handle.rect)).toEqual([
      { x: 167, y: 11, width: 8, height: 600 },
      { x: 358, y: 11, width: 8, height: 600 },
    ]);
    expect(solved.degraded).toBe(false);
  });

  it("reserves a nested subtree's minimum and exposes it to divider resizing", () => {
    const nested = split("column", "col", [
      split("wide", "row", [leaf("b"), leaf("c"), leaf("d")]), leaf("e"), leaf("f"),
    ], .1);
    const root = split("root", "row", [leaf("a", .9), nested]);
    const solved = solveLayout(root, { x: 7, y: 11, width: 1000, height: 600 });
    expect(solved.leaves.get("a")).toEqual({ x: 7, y: 11, width: 496, height: 600 });
    expect(["b", "c", "d"].map(id => solved.leaves.get(id))).toEqual([
      { x: 511, y: 11, width: 160, height: 195 },
      { x: 679, y: 11, width: 160, height: 195 },
      { x: 847, y: 11, width: 160, height: 195 },
    ]);
    expect(solved.dividers.find(handle => handle.splitId === "root")).toMatchObject({
      index: 0, dir: "row", minA: 160, minB: 496, wa: .9, wb: .1,
      a: { x: 7, y: 11, width: 496, height: 600 },
      b: { x: 511, y: 11, width: 496, height: 600 },
    });
  });

  it("rounds intermediate column panes and gives the last pane the exact remaining pixels", () => {
    const solved = solveLayout(split("column", "col", [leaf("a"), leaf("b"), leaf("c")]),
      { x: 7, y: 11, width: 600, height: 1001 });
    expect([...solved.leaves.values()]).toEqual([
      { x: 7, y: 11, width: 600, height: 328 },
      { x: 7, y: 347, width: 600, height: 328 },
      { x: 7, y: 683, width: 600, height: 329 },
    ]);
    expect(solved.dividers.map(handle => handle.rect)).toEqual([
      { x: 7, y: 339, width: 600, height: 8 },
      { x: 7, y: 675, width: 600, height: 8 },
    ]);
  });

  it.each(["row", "col"] as const)("shrinks divider gaps and bounds rounded panes in a tiny %s", dir => {
    const root = split("tiny", dir, [leaf("a"), leaf("b"), leaf("c")]);
    for (const size of [3, 5]) {
      const bounds = { x: 7, y: 11, width: dir === "row" ? size : 600, height: dir === "col" ? size : 600 };
      const solved = solveLayout(root, bounds);
      const rects = [...solved.leaves.values()];
      const start = dir === "row" ? bounds.x : bounds.y;
      const positions = rects.map(rect => dir === "row" ? rect.x : rect.y);
      const extents = rects.map(rect => dir === "row" ? rect.width : rect.height);
      const gap = size / 5;
      expect(extents[0]).toBe(1);
      expect(extents[1]).toBeCloseTo(size === 3 ? .8 : 1);
      expect(extents[2]).toBeCloseTo(size === 3 ? 0 : 1);
      expect(positions[0]).toBe(start);
      expect(positions[1]).toBeCloseTo(start + extents[0] + gap);
      expect(positions[2]).toBeCloseTo(start + extents[0] + extents[1] + 2 * gap);
      expect(positions[2] + extents[2]).toBeCloseTo(start + size);
      for (const handle of solved.dividers) {
        expect(dir === "row" ? handle.rect.width : handle.rect.height).toBeCloseTo(gap);
      }
      expect(solved.degraded).toBe(true);
    }
  });

  it.each(["row", "col"] as const)("reserves all remaining gaps when rounding a very narrow seven-pane %s", dir => {
    const root = split("tiny", dir, Array.from({ length: 7 }, (_, i) => leaf(`p${i}`)));
    const bounds = { x: 7, y: 11, width: dir === "row" ? 6.5 : 600, height: dir === "col" ? 6.5 : 600 };
    const solved = solveLayout(root, bounds);
    const rects = [...solved.leaves.values()];
    expect(rects.map(rect => dir === "row" ? rect.width : rect.height)).toEqual([1, 1, 1, .5, 0, 0, 0]);
    for (const rect of rects) {
      expect(rect.x + rect.width).toBeLessThanOrEqual(bounds.x + bounds.width);
      expect(rect.y + rect.height).toBeLessThanOrEqual(bounds.y + bounds.height);
    }
    expect(solved.dividers.map(handle => dir === "row" ? handle.rect.width : handle.rect.height)).toEqual([.5, .5, .5, .5, .5, .5]);
  });

  it("keeps every leaf in bounds when external membership exceeds minima", () => {
    const root = firstLoadGrid(Array.from({ length: 9 }, (_, i) => leaf(`p${i}`)))!;
    const small = { x: 7, y: 11, width: 310, height: 150 };
    const solved = solveLayout(root, small);
    expect([...solved.leaves.keys()]).toEqual(Array.from({ length: 9 }, (_, i) => `p${i}`));
    for (const rect of solved.leaves.values()) {
      expect(rect.width).toBeGreaterThan(0); expect(rect.height).toBeGreaterThan(0);
      expect(rect.x).toBeGreaterThanOrEqual(small.x); expect(rect.y).toBeGreaterThanOrEqual(small.y);
      expect(rect.x + rect.width).toBeLessThanOrEqual(small.x + small.width);
      expect(rect.y + rect.height).toBeLessThanOrEqual(small.y + small.height);
    }
    expect(solved.degraded).toBe(true);
    expect(solveLayout(root, area).degraded).toBe(false);
  });

  it("targets rim before centre, centre before edge, and ignores source and outside", () => {
    const solved = new Map([["src", { x: 0, y: 0, width: 496, height: 600 }], ["dest", { x: 504, y: 0, width: 496, height: 600 }]]);
    expect(computeDrop({ x: 2, y: 300 }, "src", solved, area, 2)).toMatchObject({ kind: "root", side: "left", sharePct: 50, rect: { width: 500, height: 600 } });
    expect(computeDrop({ x: 752, y: 300 }, "src", solved, area, 2)).toMatchObject({ kind: "swap", target: "dest" });
    expect(computeDrop({ x: 520, y: 300 }, "src", solved, area, 2)).toMatchObject({ kind: "edge", side: "left", rect: { width: 248 } });
    expect(computeDrop({ x: 250, y: 300 }, "src", solved, area, 2)).toBeNull();
    expect(computeDrop({ x: -1, y: 300 }, "src", solved, area, 2)).toBeNull();
  });

  it("falls back to the fitting axis, then swap when neither axis can split", () => {
    const solved = new Map([["dest", { x: 100, y: 100, width: 200, height: 400 }]]);
    expect(computeDrop({ x: 102, y: 180 }, "src", solved, area, 2)).toMatchObject({ kind: "edge", side: "top" });
    solved.set("dest", { x: 100, y: 100, width: 200, height: 150 });
    expect(computeDrop({ x: 102, y: 180 }, "src", solved, area, 2)).toMatchObject({ kind: "swap", target: "dest" });
  });

  it.each([
    ["left", { x: 50, y: 330 }, { x: 50, y: 30, width: 250, height: 600 }],
    ["right", { x: 1050, y: 330 }, { x: 800, y: 30, width: 250, height: 600 }],
    ["top", { x: 550, y: 30 }, { x: 50, y: 30, width: 1000, height: 150 }],
    ["bottom", { x: 550, y: 630 }, { x: 50, y: 480, width: 1000, height: 150 }],
  ])("includes the outer %s boundary and previews one equal root share", (side, point, rect) => {
    const bounds = { x: 50, y: 30, width: 1000, height: 600 };
    const solved = new Map([["src", bounds]]);
    expect(computeDrop(point, "src", solved, bounds, 4)).toMatchObject({ kind: "root", side, sharePct: 25, rect });
    expect(computeDrop(point, "src", solved, bounds, 1)).toBeNull();
  });

  it.each([{ x: 49, y: 330 }, { x: 1051, y: 330 }, { x: 550, y: 29 }, { x: 550, y: 631 }])(
    "rejects points outside each area boundary: %j", point => {
      expect(computeDrop(point, "src", new Map(), { x: 50, y: 30, width: 1000, height: 600 }, 2)).toBeNull();
    },
  );

  it("includes the rim threshold, chooses the closest corner rim, and resolves equal distances stably", () => {
    const bounds = { x: 50, y: 30, width: 1000, height: 600 };
    const solved = new Map([["src", bounds]]);
    expect(computeDrop({ x: 64, y: 330 }, "src", solved, bounds, 2)).toMatchObject({ kind: "root", side: "left" });
    expect(computeDrop({ x: 65, y: 330 }, "src", solved, bounds, 2)).toBeNull();
    expect(computeDrop({ x: 60, y: 32 }, "src", solved, bounds, 2)).toMatchObject({ kind: "root", side: "top" });
    expect(computeDrop({ x: 1048, y: 35 }, "src", solved, bounds, 2)).toMatchObject({ kind: "root", side: "right" });
    expect(computeDrop({ x: 52, y: 32 }, "src", solved, bounds, 2)).toMatchObject({ kind: "root", side: "left" });
  });

  it.each([
    ["left", { x: 100, y: 300 }, { x: 100, y: 100, width: 200, height: 400 }],
    ["right", { x: 500, y: 300 }, { x: 300, y: 100, width: 200, height: 400 }],
    ["top", { x: 300, y: 100 }, { x: 100, y: 100, width: 400, height: 200 }],
    ["bottom", { x: 300, y: 500 }, { x: 100, y: 300, width: 400, height: 200 }],
  ])("includes the pane's %s boundary and previews its matching half", (side, point, rect) => {
    const solved = new Map([["dest", { x: 100, y: 100, width: 400, height: 400 }]]);
    expect(computeDrop(point, "src", solved, area, 2)).toMatchObject({ kind: "edge", target: "dest", side, rect });
  });

  it.each([{ x: 99, y: 300 }, { x: 501, y: 300 }, { x: 300, y: 99 }, { x: 300, y: 501 }])(
    "does not target a pane from outside its bounds: %j", point => {
      const solved = new Map([["dest", { x: 100, y: 100, width: 400, height: 400 }]]);
      expect(computeDrop(point, "src", solved, area, 2)).toBeNull();
    },
  );

  it.each([
    [{ x: 200, y: 300 }, null], [{ x: 400, y: 300 }, null],
    [{ x: 300, y: 200 }, null], [{ x: 300, y: 400 }, null],
    [{ x: 199, y: 300 }, "left"], [{ x: 401, y: 300 }, "right"],
    [{ x: 300, y: 199 }, "top"], [{ x: 300, y: 401 }, "bottom"],
  ])("gives the inclusive central quarter priority over pane edges: %j", (point, side) => {
    const rect = { x: 100, y: 100, width: 400, height: 400 };
    const drop = computeDrop(point, "src", new Map([["dest", rect]]), area, 2);
    expect(drop).toMatchObject(side ? { kind: "edge", side } : { kind: "swap", target: "dest", rect });
  });

  it.each([
    [{ x: 160, y: 120 }, "top"], [{ x: 480, y: 160 }, "right"],
    [{ x: 460, y: 480 }, "bottom"], [{ x: 140, y: 140 }, "top"],
  ])("uses the closest normalized pane edge, preferring the vertical axis on ties: %j", (point, side) => {
    const solved = new Map([["dest", { x: 100, y: 100, width: 400, height: 400 }]]);
    expect(computeDrop(point, "src", solved, area, 2)).toMatchObject({ kind: "edge", side });
  });

  it.each([
    [{ x: 100, y: 100, width: 328, height: 200 }, { x: 100, y: 200 }, "left"],
    [{ x: 100, y: 100, width: 327, height: 200 }, { x: 100, y: 200 }, null],
    [{ x: 100, y: 100, width: 320, height: 208 }, { x: 260, y: 100 }, "top"],
    [{ x: 100, y: 100, width: 320, height: 207 }, { x: 260, y: 100 }, null],
    [{ x: 100, y: 100, width: 400, height: 200 }, { x: 300, y: 100 }, "right"],
    [{ x: 100, y: 100, width: 300, height: 300 }, { x: 100, y: 250 }, "bottom"],
    [{ x: 100, y: 100, width: 400, height: 200 }, { x: 300, y: 299 }, "right"],
  ])("respects both split minima including gaps, falling back only to an axis that fits: %j", (rect: Rect, point, side) => {
    const drop = computeDrop(point, "src", new Map([["dest", rect]]), area, 2);
    expect(drop).toMatchObject(side ? { kind: "edge", target: "dest", side } : { kind: "swap", target: "dest", rect });
  });
});
