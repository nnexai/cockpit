import { describe, expect, it } from "vitest";
import { applyDrop, compareStablePaneId, firstLoadGrid, insertRootEdge, leaves, removeLeaf, setPairWeights, splitLeaf, swapLeaves, type LayoutNode, type Leaf } from "./splitTree";

const leaf = (id: string, w = 1): Leaf => ({ t: "leaf", id, kind: "terminal", w });
const row = (): LayoutNode => ({ t: "split", id: "row", dir: "row", w: 1, kids: [leaf("p1", .6), leaf("p2", .4)] });
const rect = { x: 0, y: 0, width: 400, height: 300 };

describe("immutable split tree placement", () => {
  it("uses arbitrary-precision numeric runs and code-unit ties for stable first load", () => {
    expect(["p10", "p9", "p1", "p01"].sort(compareStablePaneId)).toEqual(["p01", "p1", "p9", "p10"]);
    expect(compareStablePaneId("p9007199254740992", "p9007199254740993")).toBeLessThan(0);
    expect(compareStablePaneId("p99999999999999999999", "p100000000000000000000")).toBeLessThan(0);
    expect(compareStablePaneId("p01:x9", "p1:x10")).toBeLessThan(0);
  });

  it.each([[1, [1]], [2, [2]], [3, [2, 1]], [4, [2, 2]], [5, [3, 2]], [6, [3, 3]], [7, [3, 2, 2]], [8, [3, 3, 2]], [9, [3, 3, 3]], [10, [4, 3, 3]], [17, [5, 4, 4, 4]]])("balances %i terminals in even rows", (count, expected) => {
    const root = firstLoadGrid(Array.from({ length: count as number }, (_, i) => leaf(`p${i + 1}`)))!;
    const rows = root.t === "split" && root.dir === "col" ? root.kids : [root];
    expect(rows.map(node => leaves(node).length)).toEqual(expected);
    expect(leaves(root).map(node => node.id)).toEqual(Array.from({ length: count as number }, (_, i) => `p${i + 1}`));
  });

  it("flattens a same-axis split without changing unrelated slots", () => {
    const original = row();
    const next = splitLeaf(original, "p2", "row", false, leaf("p3"));
    expect(next.t === "split" && next.kids.map(kid => [kid.id, kid.w])).toEqual([["p1", .6], ["p2", .2], ["p3", .2]]);
    expect(original).toEqual(row());
    const removed = removeLeaf(next, "p2");
    expect(removed.absorbedBy).toBe("p1");
    expect(leaves(removed.root).map(node => node.id)).toEqual(["p1", "p3"]);
  });

  it.each([true, false])("assigns the newcomer share without resizing unrelated leaves, before=%s", before => {
    const original = row();
    const next = splitLeaf(original, "p2", "row", before, { ...leaf("widget"), kind: "widget" }, .4);
    const slots = leaves(next);
    expect(slots.map(node => node.id)).toEqual(before ? ["p1", "widget", "p2"] : ["p1", "p2", "widget"]);
    expect(slots.find(node => node.id === "p1")?.w).toBeCloseTo(.6);
    expect(slots.find(node => node.id === "p2")?.w).toBeCloseTo(.24);
    expect(slots.find(node => node.id === "widget")?.w).toBeCloseTo(.16);
    expect(original).toEqual(row());
  });

  it("retains the source-local newcomer ratio in a nested split", () => {
    const next = splitLeaf(row(), "p2", "col", false, { ...leaf("widget"), kind: "widget" }, .3);
    expect(next.t).toBe("split");
    if (next.t !== "split") throw new Error("Expected root row");
    expect(next.kids[0]).toEqual(leaf("p1", .6));
    const pair = next.kids[1];
    expect(pair).toMatchObject({ t: "split", dir: "col", w: .4 });
    expect(leaves(pair).map(node => [node.id, node.w])).toEqual([["p2", .7], ["widget", .3]]);
  });

  it.each([0, 1, -1, Number.NaN, Number.POSITIVE_INFINITY])("rejects a degenerate split share %s", share => {
    const root = row();
    expect(splitLeaf(root, "p2", "row", false, leaf("widget"), share)).toBe(root);
  });

  it("inserts an external pane with equal leaf share while preserving nested ratios", () => {
    const root: LayoutNode = { t: "split", id: "outer", dir: "row", w: 1, kids: [leaf("p1", .6),
      { t: "split", id: "inner", dir: "col", w: .4, kids: [leaf("p2", .5), { ...leaf("f", .5), kind: "files" }] }] };
    const next = insertRootEdge(root, leaf("p4"), "row", false);
    expect(next.t).toBe("split");
    if (next.t !== "split") throw new Error("Expected root row");
    expect(next.kids[0].w).toBeCloseTo(.45);
    expect(next.kids[1].w).toBeCloseTo(.3);
    expect(next.kids[2].w).toBeCloseTo(.25);
    expect(next.kids[1].t === "split" && next.kids[1].kids.map(kid => kid.w)).toEqual([.5, .5]);
    const column: LayoutNode = { t: "split", id: "column", dir: "col", w: 1, kids: [leaf("a", .5), leaf("b", .5)] };
    const wrapped = insertRootEdge(column, leaf("c"), "row", false);
    expect(wrapped.t === "split" && wrapped.dir).toBe("row");
    expect(wrapped.t === "split" && wrapped.kids[1].w).toBeCloseTo(1 / 3);
  });

  it("swaps slot sizes, restructures edge drops, and inserts rim drops after lifting", () => {
    const root: LayoutNode = { t: "split", id: "r", dir: "row", w: 1, kids: [leaf("p1", .5),
      { t: "split", id: "c", dir: "col", w: .5, kids: [leaf("p2", .5), { ...leaf("review", .5), kind: "review" }] }] };
    const swapped = swapLeaves(root, "p1", "review");
    expect(leaves(swapped).map(node => [node.id, node.w])).toEqual([["review", .5], ["p2", .5], ["p1", .5]]);
    const moved = applyDrop(root, "p1", { kind: "edge", target: "p2", side: "bottom", rect, label: "Below" });
    expect(moved.t === "split" && moved.dir).toBe("col");
    expect(leaves(moved).map(node => [node.id, node.w])).toEqual([["p2", .25], ["p1", .25], ["review", .5]]);
    const rim = applyDrop(root, "review", { kind: "root", side: "right", rect, sharePct: 33, label: "Outer right edge" });
    expect(leaves(rim).at(-1)?.w).toBeCloseTo(1 / 3);
  });

  it("resizes only an adjacent pair and keeps the split's stable identity", () => {
    const root = insertRootEdge(row(), leaf("p3"), "row", false);
    const next = setPairWeights(root, root.id, 0, 1, 3);
    expect(next.id).toBe(root.id);
    expect(next.t === "split" && root.t === "split" && next.kids[2]).toEqual(root.t === "split" && root.kids[2]);
    expect(next.t === "split" && next.kids[0].w / next.kids[1].w).toBeCloseTo(1 / 3);
  });
});
