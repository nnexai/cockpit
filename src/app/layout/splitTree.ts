import type { DropTarget } from "./solveLayout";

export type ViewerKind = "files" | "review" | "browser" | "widget";
export type LeafKind = "terminal" | ViewerKind;
export type LeafId = string;
export type Direction = "row" | "col";
export type Side = "left" | "right" | "top" | "bottom";
export type Leaf = { t: "leaf"; id: LeafId; kind: LeafKind; w: number };
export type Split = { t: "split"; id: string; dir: Direction; kids: LayoutNode[]; w: number };
export type LayoutNode = Leaf | Split;

export function leaves(root: LayoutNode | null): Leaf[] {
  if (!root) return [];
  return root.t === "leaf" ? [root] : root.kids.flatMap(leaves);
}

export function findNode(root: LayoutNode | null, id: string): LayoutNode | null {
  if (!root || root.id === id) return root;
  if (root.t === "leaf") return null;
  for (const kid of root.kids) {
    const found = findNode(kid, id);
    if (found) return found;
  }
  return null;
}

// Allocate ids from the tree rather than global state: reducer replay is deterministic.
function splitId(root: LayoutNode | null): string {
  let n = 1;
  while (findNode(root, `layout-split:${n}`)) n++;
  return `layout-split:${n}`;
}

export function normalize(root: LayoutNode): LayoutNode {
  if (root.t === "leaf") return root;
  const kids: LayoutNode[] = [];
  for (const child of root.kids) {
    const node = normalize(child);
    if (node.t === "split" && node.dir === root.dir) {
      kids.push(...node.kids.map(kid => ({ ...kid, w: kid.w * node.w })));
    } else kids.push(node);
  }
  if (kids.length === 1) return { ...kids[0], w: root.w };
  const sum = kids.reduce((total, kid) => total + kid.w, 0);
  return { ...root, kids: kids.map(kid => ({ ...kid, w: sum > 0 ? kid.w / sum : 1 / kids.length })) };
}

function mapNode(root: LayoutNode, id: string, change: (node: LayoutNode) => LayoutNode): LayoutNode {
  if (root.id === id) return change(root);
  if (root.t === "leaf") return root;
  const kids = root.kids.map(kid => mapNode(kid, id, change));
  return kids.some((kid, i) => kid !== root.kids[i]) ? { ...root, kids } : root;
}

export function compareStablePaneId(a: string, b: string): number {
  const ar = a.match(/\d+|\D+/g) ?? [], br = b.match(/\d+|\D+/g) ?? [];
  for (let i = 0; i < Math.min(ar.length, br.length); i++) {
    let x = ar[i], y = br[i];
    if (/^\d/.test(x) && /^\d/.test(y)) {
      x = x.replace(/^0+/, "") || "0";
      y = y.replace(/^0+/, "") || "0";
      if (x.length !== y.length) return x.length - y.length;
    }
    if (x !== y) return x < y ? -1 : 1;
  }
  if (ar.length !== br.length) return ar.length - br.length;
  return a === b ? 0 : a < b ? -1 : 1;
}

export function firstLoadGrid(sortedLeaves: Leaf[]): LayoutNode | null {
  if (!sortedLeaves.length) return null;
  const cols = Math.ceil(Math.sqrt(sortedLeaves.length));
  const count = Math.ceil(sortedLeaves.length / cols);
  const base = Math.floor(sortedLeaves.length / count), extra = sortedLeaves.length % count;
  let offset = 0, sequence = 0;
  const rows: LayoutNode[] = [];
  for (let i = 0; i < count; i++) {
    const kids = sortedLeaves.slice(offset, offset + base + (i < extra ? 1 : 0)).map(leaf => ({ ...leaf, w: 1 }));
    offset += kids.length;
    rows.push(kids.length === 1 ? kids[0] : { t: "split", id: `layout-split:${++sequence}`, dir: "row", w: 1, kids });
  }
  const root: LayoutNode = rows.length === 1 ? rows[0] : { t: "split", id: `layout-split:${++sequence}`, dir: "col", kids: rows, w: 1 };
  return normalize(root);
}

export function splitLeaf(root: LayoutNode, targetId: LeafId, dir: Direction, before: boolean, leaf: Leaf, share = 0.5): LayoutNode {
  const target = findNode(root, targetId);
  if (!target || target.t !== "leaf" || findNode(root, leaf.id) || !(share > 0 && share < 1)) return root;
  const id = splitId(root);
  return normalize(mapNode(root, targetId, node => ({ t: "split", id, dir, w: node.w,
    kids: before ? [{ ...leaf, w: share }, { ...node, w: 1 - share }] : [{ ...node, w: 1 - share }, { ...leaf, w: share }] })));
}

export function removeLeaf(root: LayoutNode, id: LeafId): { root: LayoutNode | null; absorbedBy: LeafId | null } {
  if (root.t === "leaf") return root.id === id ? { root: null, absorbedBy: null } : { root, absorbedBy: null };
  for (let i = 0; i < root.kids.length; i++) {
    const child = root.kids[i];
    if (child.t === "leaf" && child.id === id) {
      const sibling = root.kids[i - 1] ?? root.kids[i + 1];
      const next = normalize({ ...root, kids: root.kids.filter((_, index) => index !== i) });
      return { root: { ...next, w: root.w }, absorbedBy: leaves(sibling)[0]?.id ?? null };
    }
    if (child.t === "split") {
      const removed = removeLeaf(child, id);
      if (removed.root !== child) {
        const kids = root.kids.slice();
        if (removed.root) kids[i] = removed.root; else kids.splice(i, 1);
        return { root: normalize({ ...root, kids }), absorbedBy: removed.absorbedBy };
      }
    }
  }
  return { root, absorbedBy: null };
}

export function insertRootEdge(root: LayoutNode | null, leaf: Leaf, dir: Direction, before: boolean): LayoutNode {
  if (!root) return { ...leaf, w: 1 };
  if (findNode(root, leaf.id)) return root;
  const count = leaves(root).length, keep = count / (count + 1), newcomer = { ...leaf, w: 1 / (count + 1) };
  if (root.t === "split" && root.dir === dir) {
    const kids = root.kids.map(kid => ({ ...kid, w: kid.w * keep }));
    return normalize({ ...root, kids: before ? [newcomer, ...kids] : [...kids, newcomer] });
  }
  const old = { ...root, w: keep };
  return normalize({ t: "split", id: splitId(root), dir, w: 1, kids: before ? [newcomer, old] : [old, newcomer] });
}

export function swapLeaves(root: LayoutNode, a: LeafId, b: LeafId): LayoutNode {
  const aa = findNode(root, a), bb = findNode(root, b);
  if (!aa || !bb || aa.t !== "leaf" || bb.t !== "leaf" || a === b) return root;
  function swap(node: LayoutNode): LayoutNode {
    if (node.id === a) return { ...bb as Leaf, w: node.w };
    if (node.id === b) return { ...aa as Leaf, w: node.w };
    return node.t === "split" ? { ...node, kids: node.kids.map(swap) } : node;
  }
  return swap(root);
}

export function applyDrop(root: LayoutNode, src: LeafId, drop: DropTarget): LayoutNode {
  const leaf = findNode(root, src);
  if (!leaf || leaf.t !== "leaf") return root;
  if (drop.kind === "swap") return swapLeaves(root, src, drop.target);
  if (drop.kind === "edge" && (src === drop.target || !findNode(root, drop.target))) return root;
  const removed = removeLeaf(root, src).root;
  const dir = drop.side === "left" || drop.side === "right" ? "row" : "col";
  const before = drop.side === "left" || drop.side === "top";
  return drop.kind === "root" ? insertRootEdge(removed, leaf, dir, before)
    : removed ? splitLeaf(removed, drop.target, dir, before, leaf) : root;
}

export function setPairWeights(root: LayoutNode, splitId: string, index: number, wa: number, wb: number): LayoutNode {
  if (!(wa > 0) || !(wb > 0) || !Number.isFinite(wa + wb)) return root;
  return mapNode(root, splitId, node => {
    if (node.t !== "split" || !node.kids[index] || !node.kids[index + 1]) return node;
    const kids = node.kids.slice(), total = kids[index].w + kids[index + 1].w;
    kids[index] = { ...kids[index], w: total * wa / (wa + wb) };
    kids[index + 1] = { ...kids[index + 1], w: total - kids[index].w };
    return { ...node, kids };
  });
}
