import { describe, expect, it } from "vitest";
import { firstLoadGrid, type Leaf, type LayoutNode } from "./splitTree";
import { computeDrop, solveLayout } from "./solveLayout";

const area = { x: 0, y: 0, width: 1000, height: 600 };
const leaf = (id: string, w = 1): Leaf => ({ t: "leaf", id, kind: "terminal", w });

describe("layout geometry", () => {
  it("freezes nested minima and gives the final child the pixel remainder", () => {
    const root: LayoutNode = { t: "split", id: "root", dir: "row", w: 1, kids: [leaf("a", .1), leaf("b", .9)] };
    const solved = solveLayout(root, area);
    expect(solved.leaves.get("a")).toEqual({ x: 0, y: 0, width: 160, height: 600 });
    expect(solved.leaves.get("b")).toEqual({ x: 168, y: 0, width: 832, height: 600 });
    expect(solved.dividers[0].rect).toEqual({ x: 160, y: 0, width: 8, height: 600 });
    expect(solved.degraded).toBe(false);
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
    expect(computeDrop({ x: 752, y: 300 }, "src", solved, area, 2)).toMatchObject({ kind: "swap", target: "dest", label: "Swap" });
    expect(computeDrop({ x: 520, y: 300 }, "src", solved, area, 2)).toMatchObject({ kind: "edge", side: "left", rect: { width: 248 } });
    expect(computeDrop({ x: 250, y: 300 }, "src", solved, area, 2)).toBeNull();
    expect(computeDrop({ x: -1, y: 300 }, "src", solved, area, 2)).toBeNull();
  });

  it("falls back to the fitting axis, then swap when neither axis can split", () => {
    const solved = new Map([["dest", { x: 100, y: 100, width: 200, height: 400 }]]);
    expect(computeDrop({ x: 102, y: 180 }, "src", solved, area, 2)).toMatchObject({ kind: "edge", side: "top" });
    solved.set("dest", { x: 100, y: 100, width: 200, height: 150 });
    expect(computeDrop({ x: 102, y: 180 }, "src", solved, area, 2)).toMatchObject({ kind: "swap", label: "Swap (pane too small to split)" });
  });
});
