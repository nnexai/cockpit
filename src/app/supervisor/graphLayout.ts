export const GRAPH_GEOMETRY = { nodeWidth: 240, nodeHeight: 48, columnGap: 32, rowPitch: 56, padding: 8, headerHeight: 28 } as const;
export type GraphNode = { id: string; parentId: string | null; minColumn?: number; order?: readonly (string | number)[]; gapBefore?: number };
export type GraphPosition = { id: string; column: number; row: number; x: number; y: number; width: number; height: number };
export type GraphEdge = { from: string; to: string; path: string };
export type GraphLayout = {
  positions: GraphPosition[]; edges: GraphEdge[]; width: number; height: number; columns: number;
  order: string[]; parentOf: ReadonlyMap<string, string | null>;
};
const compareIds = (a: string, b: string) => a < b ? -1 : a > b ? 1 : 0;

export function edgePath(from: GraphPosition, to: GraphPosition): string {
  const x = from.x + from.width, y = from.y + from.height / 2;
  const endY = to.y + to.height / 2, bend = (x + to.x) / 2;
  return y === endY ? `M ${x} ${y} L ${to.x} ${endY}` : `M ${x} ${y} C ${bend} ${y}, ${bend} ${endY}, ${to.x} ${endY}`;
}

/** Iterative deterministic forest. Missing parents and the smallest member of each cycle stay roots. */
export function graphLayout(nodes: readonly GraphNode[]): GraphLayout {
  const byId = new Map<string, GraphNode>();
  for (const node of [...nodes].sort((a, b) => compareIds(a.id, b.id))) if (!byId.has(node.id)) byId.set(node.id, node);
  const parents = new Map<string, string | null>();
  for (const node of byId.values()) parents.set(node.id, node.parentId !== node.id && node.parentId !== null && byId.has(node.parentId) ? node.parentId : null);
  const settled = new Set<string>();
  for (const id of parents.keys()) {
    const chain: string[] = [];
    const indices = new Map<string, number>();
    let cursor: string | null = id;
    while (cursor !== null && !settled.has(cursor)) {
      const cycleStart = indices.get(cursor);
      if (cycleStart !== undefined) {
        let first = chain[cycleStart];
        for (let index = cycleStart + 1; index < chain.length; index++) if (compareIds(chain[index], first) < 0) first = chain[index];
        parents.set(first, null);
        break;
      }
      indices.set(cursor, chain.length); chain.push(cursor);
      cursor = parents.get(cursor) ?? null;
    }
    for (const member of chain) settled.add(member);
  }
  const compareNodes = (a: string, b: string) => {
    const left = byId.get(a)!.order ?? [], right = byId.get(b)!.order ?? [];
    for (let i = 0; i < Math.min(left.length, right.length); i++) {
      const diff = typeof left[i] === "number" && typeof right[i] === "number"
        ? (left[i] as number) - (right[i] as number) : compareIds(String(left[i]), String(right[i]));
      if (diff) return diff;
    }
    return left.length - right.length || compareIds(a, b);
  };
  const children = new Map<string, string[]>();
  const roots: string[] = [];
  for (const [id, parent] of parents) {
    if (parent === null) roots.push(id);
    else {
      const siblings = children.get(parent);
      if (siblings) siblings.push(id); else children.set(parent, [id]);
    }
  }
  roots.sort(compareNodes);
  for (const siblings of children.values()) siblings.sort(compareNodes);
  const order: string[] = [];
  const positions = new Map<string, GraphPosition>();
  const pending = roots.map(id => ({ id, parentColumn: -1, exit: false })).reverse();
  let nextRow = 0, width = 0, height = 0, columns = 0;
  while (pending.length) {
    const { id, parentColumn, exit } = pending.pop()!;
    const descendants = children.get(id) ?? [];
    if (!exit) {
      order.push(id);
      nextRow += Math.max(0, byId.get(id)!.gapBefore ?? 0);
      const column = Math.max(parentColumn + 1, byId.get(id)!.minColumn ?? 0);
      positions.set(id, { id, column, row: 0, x: GRAPH_GEOMETRY.padding + column * (GRAPH_GEOMETRY.nodeWidth + GRAPH_GEOMETRY.columnGap),
        y: 0, width: GRAPH_GEOMETRY.nodeWidth, height: GRAPH_GEOMETRY.nodeHeight });
      pending.push({ id, parentColumn, exit: true });
      for (let i = descendants.length - 1; i >= 0; i--) pending.push({ id: descendants[i], parentColumn: column, exit: false });
    } else {
      const position = positions.get(id)!;
      position.row = descendants.length ? (positions.get(descendants[0])!.row + positions.get(descendants[descendants.length - 1])!.row) / 2 : nextRow++;
      position.y = Math.round(GRAPH_GEOMETRY.padding + GRAPH_GEOMETRY.rowPitch * position.row);
      width = Math.max(width, position.x + position.width + GRAPH_GEOMETRY.padding);
      height = Math.max(height, position.y + position.height + GRAPH_GEOMETRY.padding);
      columns = Math.max(columns, position.column + 1);
    }
  }
  const edges: GraphEdge[] = [];
  for (const id of order) {
    const parent = parents.get(id);
    if (parent !== null && parent !== undefined) edges.push({ from: parent, to: id, path: edgePath(positions.get(parent)!, positions.get(id)!) });
  }
  return { positions: order.map(id => positions.get(id)!), edges, width, height, columns, order, parentOf: parents };
}
