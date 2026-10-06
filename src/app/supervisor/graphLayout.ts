export type GraphNode = { id: string; parentId: string | null };
export type GraphPosition = { id: string; x: number; y: number; width: number; height: number };
export type GraphEdge = { from: string; to: string; path: string };
export type GraphLayout = { positions: GraphPosition[]; edges: GraphEdge[]; width: number; height: number };

const CARD_WIDTH = 180;
const CARD_HEIGHT = 36;
const COLUMN_GAP = 36;
const ROW_GAP = 8;
const PADDING = 8;
const compareIds = (a: string, b: string) => a < b ? -1 : a > b ? 1 : 0;

/** A deterministic forest layout. Missing parents stay roots; cycles lose one edge, never gain one. */
export function graphLayout(nodes: readonly GraphNode[]): GraphLayout {
  if (!nodes.length) return { positions: [], edges: [], width: 0, height: 0 };
  const ordered = [...nodes].sort((a, b) => compareIds(a.id, b.id));
  const ids = new Set(ordered.map(node => node.id));
  const parents = new Map<string, string | null>();
  for (const node of ordered) {
    parents.set(node.id, node.parentId !== node.id && node.parentId !== null && ids.has(node.parentId) ? node.parentId : null);
  }

  // Follow parent chains iteratively so even a deep or cyclic snapshot cannot exhaust the stack.
  const settled = new Set<string>();
  for (const id of parents.keys()) {
    const chain: string[] = [];
    const indices = new Map<string, number>();
    let cursor: string | null = id;
    while (cursor !== null && !settled.has(cursor)) {
      const cycleStart = indices.get(cursor);
      if (cycleStart !== undefined) {
        let first = chain[cycleStart];
        for (let index = cycleStart + 1; index < chain.length; index++) {
          if (compareIds(chain[index], first) < 0) first = chain[index];
        }
        parents.set(first, null);
        break;
      }
      indices.set(cursor, chain.length);
      chain.push(cursor);
      cursor = parents.get(cursor) ?? null;
    }
    for (const member of chain) settled.add(member);
  }

  const children = new Map<string, string[]>();
  const roots: string[] = [];
  for (const [id, parent] of parents) {
    if (parent === null) roots.push(id);
    else {
      const siblings = children.get(parent);
      if (siblings) siblings.push(id);
      else children.set(parent, [id]);
    }
  }
  const traversal: { id: string; depth: number }[] = [];
  const pending = roots.map(id => ({ id, depth: 0 })).reverse();
  while (pending.length) {
    const node = pending.pop()!;
    traversal.push(node);
    const descendants = children.get(node.id) ?? [];
    for (let index = descendants.length - 1; index >= 0; index--) pending.push({ id: descendants[index], depth: node.depth + 1 });
  }
  const spans = new Map<string, number>();
  for (let index = traversal.length - 1; index >= 0; index--) {
    const descendants = children.get(traversal[index].id);
    const span = descendants ? descendants.reduce((total, id) => total + spans.get(id)!, 0) : CARD_HEIGHT + ROW_GAP;
    spans.set(traversal[index].id, span);
  }

  const tops = new Map<string, number>();
  let rootTop = PADDING;
  for (const id of roots) {
    tops.set(id, rootTop);
    rootTop += spans.get(id)!;
  }
  const byId = new Map<string, GraphPosition>();
  let width = 0;
  let height = 0;
  for (const { id, depth } of traversal) {
    const top = tops.get(id)!;
    const position = { id, x: PADDING + depth * (CARD_WIDTH + COLUMN_GAP), y: top + (spans.get(id)! - ROW_GAP - CARD_HEIGHT) / 2, width: CARD_WIDTH, height: CARD_HEIGHT };
    byId.set(id, position);
    width = Math.max(width, position.x + CARD_WIDTH + PADDING);
    height = Math.max(height, position.y + CARD_HEIGHT + PADDING);
    let childTop = top;
    for (const child of children.get(id) ?? []) {
      tops.set(child, childTop);
      childTop += spans.get(child)!;
    }
  }
  const edges: GraphEdge[] = [];
  for (const [id, parent] of parents) {
    if (parent === null) continue;
    const from = byId.get(parent)!;
    const to = byId.get(id)!;
    const startX = from.x + from.width;
    const startY = from.y + from.height / 2;
    const endY = to.y + to.height / 2;
    const bendX = (startX + to.x) / 2;
    edges.push({ from: parent, to: id, path: `M ${startX} ${startY} C ${bendX} ${startY}, ${bendX} ${endY}, ${to.x} ${endY}` });
  }
  return { positions: [...parents.keys()].map(id => byId.get(id)!), edges, width, height };
}
