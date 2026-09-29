import type { LayoutNode, LeafId, Side } from "./splitTree";

export const MIN_W = 160, MIN_H = 100, DIVIDER = 8, RIM = 14, EDGE = .25, DRAG_PX = 4;
export type Rect = { x: number; y: number; width: number; height: number };
export type DividerHandle = {
  splitId: string; index: number; dir: "row" | "col"; rect: Rect;
  a: Rect; b: Rect; minA: number; minB: number; wa: number; wb: number;
};
export type DropTarget =
  | { kind: "swap"; target: LeafId; rect: Rect; label: "Swap" | "Swap (pane too small to split)" }
  | { kind: "edge"; target: LeafId; side: Side; rect: Rect; label: "Left" | "Right" | "Above" | "Below" }
  | { kind: "root"; side: Side; rect: Rect; sharePct: number; label: string };

export function minSize(node: LayoutNode, axis: "x" | "y"): number {
  if (node.t === "leaf") return axis === "x" ? MIN_W : MIN_H;
  const sizes = node.kids.map(kid => minSize(kid, axis));
  return (node.dir === "row") === (axis === "x")
    ? sizes.reduce((a, b) => a + b, 0) + DIVIDER * (sizes.length - 1) : Math.max(...sizes);
}

export function solveLayout(root: LayoutNode, area: Rect): { leaves: Map<LeafId, Rect>; dividers: DividerHandle[]; degraded: boolean } {
  const leafRects = new Map<LeafId, Rect>(), dividers: DividerHandle[] = [];
  let degraded = area.width < minSize(root, "x") || area.height < minSize(root, "y");
  function solve(node: LayoutNode, rect: Rect): void {
    if (node.t === "leaf") { leafRects.set(node.id, rect); return; }
    const row = node.dir === "row", axis = row ? "x" : "y";
    const size = row ? rect.width : rect.height;
    // Even windows smaller than their divider gaps retain a nonnegative box for every leaf.
    const gap = Math.min(DIVIDER, size / Math.max(1, node.kids.length * 2 - 1));
    const available = Math.max(0, size - gap * (node.kids.length - 1));
    const minima = node.kids.map(kid => minSize(kid, axis));
    const totalMin = minima.reduce((a, b) => a + b, 0);
    const sizes = new Array<number>(node.kids.length).fill(0);
    if (available < totalMin) {
      degraded = true;
      minima.forEach((minimum, i) => sizes[i] = available * minimum / totalMin);
    } else {
      const free = new Set(node.kids.map((_, i) => i));
      let remainder = available;
      while (free.size) {
        const weight = [...free].reduce((sum, i) => sum + node.kids[i].w, 0);
        const frozen = [...free].filter(i => remainder * node.kids[i].w / weight < minima[i]);
        if (!frozen.length) {
          for (const i of free) sizes[i] = remainder * node.kids[i].w / weight;
          break;
        }
        for (const i of frozen) { sizes[i] = minima[i]; remainder -= sizes[i]; free.delete(i); }
      }
    }
    const childRects: Rect[] = [];
    let position = row ? rect.x : rect.y;
    const end = position + size;
    for (let i = 0; i < node.kids.length; i++) {
      const extent = i === node.kids.length - 1 ? Math.max(0, end - position)
        : Math.min(Math.round(sizes[i]), Math.max(0, end - position - gap * (node.kids.length - i - 1)));
      const childRect = row ? { ...rect, x: position, width: extent } : { ...rect, y: position, height: extent };
      childRects.push(childRect);
      solve(node.kids[i], childRect);
      position += extent + (i < node.kids.length - 1 ? gap : 0);
    }
    for (let i = 0; i < node.kids.length - 1; i++) {
      const a = childRects[i], b = childRects[i + 1];
      const dividerRect = row ? { x: a.x + a.width, y: rect.y, width: b.x - a.x - a.width, height: rect.height }
        : { x: rect.x, y: a.y + a.height, width: rect.width, height: b.y - a.y - a.height };
      dividers.push({ splitId: node.id, index: i, dir: node.dir, rect: dividerRect, a, b,
        minA: minima[i], minB: minima[i + 1], wa: node.kids[i].w, wb: node.kids[i + 1].w });
    }
  }
  solve(root, { ...area, width: Math.max(0, area.width), height: Math.max(0, area.height) });
  return { leaves: leafRects, dividers, degraded };
}

export function computeDrop(point: { x: number; y: number }, srcId: LeafId, solved: Map<LeafId, Rect>, area: Rect, leafCount: number): DropTarget | null {
  const { x, y } = point, right = area.x + area.width, bottom = area.y + area.height;
  if (leafCount < 2 || x < area.x || x > right || y < area.y || y > bottom) return null;
  const rims: [Side, number][] = [["left", x - area.x], ["right", right - x], ["top", y - area.y], ["bottom", bottom - y]];
  const rim = rims.filter(([, distance]) => distance <= RIM).sort((a, b) => a[1] - b[1])[0];
  if (rim) {
    const side = rim[0], share = 1 / leafCount, sharePct = Math.round(share * 100);
    const rect = side === "left" ? { ...area, width: area.width * share }
      : side === "right" ? { ...area, x: right - area.width * share, width: area.width * share }
      : side === "top" ? { ...area, height: area.height * share }
      : { ...area, y: bottom - area.height * share, height: area.height * share };
    return { kind: "root", side, rect, sharePct, label: `Outer ${side} edge (${sharePct}%)` };
  }
  for (const [target, rect] of solved) {
    if (x < rect.x || x > rect.x + rect.width || y < rect.y || y > rect.y + rect.height) continue;
    if (target === srcId || !rect.width || !rect.height) return null;
    const fx = (x - rect.x) / rect.width, fy = (y - rect.y) / rect.height;
    if (fx >= EDGE && fx <= 1 - EDGE && fy >= EDGE && fy <= 1 - EDGE) return { kind: "swap", target, rect, label: "Swap" };
    let axisX = Math.min(fx, 1 - fx) < Math.min(fy, 1 - fy);
    const okX = rect.width >= 2 * MIN_W + DIVIDER, okY = rect.height >= 2 * MIN_H + DIVIDER;
    if (axisX && !okX) axisX = false; else if (!axisX && !okY) axisX = true;
    if ((axisX && !okX) || (!axisX && !okY)) return { kind: "swap", target, rect, label: "Swap (pane too small to split)" };
    const side = axisX ? fx < .5 ? "left" : "right" : fy < .5 ? "top" : "bottom";
    const half = side === "left" ? { ...rect, width: rect.width / 2 }
      : side === "right" ? { ...rect, x: rect.x + rect.width / 2, width: rect.width / 2 }
      : side === "top" ? { ...rect, height: rect.height / 2 }
      : { ...rect, y: rect.y + rect.height / 2, height: rect.height / 2 };
    return { kind: "edge", target, side, rect: half, label: { left: "Left", right: "Right", top: "Above", bottom: "Below" }[side] as "Left" | "Right" | "Above" | "Below" };
  }
  return null;
}
