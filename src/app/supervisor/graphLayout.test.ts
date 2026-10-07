import { describe, expect, it } from "vitest";
import { graphLayout, type GraphNode } from "./graphLayout";

describe("graphLayout", () => {
  it("places branching workers and their children in nonoverlapping left-to-right columns", () => {
    const layout = graphLayout([
      { id: "root", parentId: null },
      { id: "worker-a", parentId: "root" },
      { id: "worker-b", parentId: "root" },
      { id: "child-a", parentId: "worker-a" },
      { id: "child-b", parentId: "worker-a" },
      { id: "child-c", parentId: "worker-b" },
      { id: "other-root", parentId: null },
    ]);
    expect(layout.positions).toHaveLength(7);
    expect(layout.edges).toHaveLength(5);
    for (const edge of layout.edges) {
      const from = layout.positions.find(node => node.id === edge.from)!;
      const to = layout.positions.find(node => node.id === edge.to)!;
      expect(to.x).toBeGreaterThan(from.x + from.width);
      expect(edge.path).toMatch(/^M .* (C|L) /);
    }
    for (let index = 0; index < layout.positions.length; index++) {
      const node = layout.positions[index];
      expect(node.x).toBeGreaterThanOrEqual(0);
      expect(node.y).toBeGreaterThanOrEqual(0);
      expect(node.x + node.width).toBeLessThanOrEqual(layout.width);
      expect(node.y + node.height).toBeLessThanOrEqual(layout.height);
      for (const other of layout.positions.slice(index + 1)) {
        expect(node.x + node.width <= other.x || other.x + other.width <= node.x || node.y + node.height <= other.y || other.y + other.height <= node.y).toBe(true);
      }
    }
    expect(layout.positions.find(node => node.id === "worker-a")!.x).toBe(layout.positions.find(node => node.id === "worker-b")!.x);
    expect(layout.positions.find(node => node.id === "child-a")!.x).toBeGreaterThan(layout.positions.find(node => node.id === "worker-a")!.x);
  });

  it("leaves orphan and self-parent nodes at the root without inventing edges", () => {
    const layout = graphLayout([
      { id: "root", parentId: null },
      { id: "orphan", parentId: "missing" },
      { id: "orphan-child", parentId: "orphan" },
      { id: "self", parentId: "self" },
    ]);
    expect(layout.edges.map(({ from, to }) => [from, to])).toEqual([["orphan", "orphan-child"]]);
    const rootX = layout.positions.find(node => node.id === "root")!.x;
    expect(layout.positions.find(node => node.id === "orphan")!.x).toBe(rootX);
    expect(layout.positions.find(node => node.id === "self")!.x).toBe(rootX);
  });

  it("breaks cycles deterministically, retaining only explicit parent relationships", () => {
    const nodes: GraphNode[] = [
      { id: "a", parentId: "c" },
      { id: "b", parentId: "a" },
      { id: "c", parentId: "b" },
      { id: "leaf", parentId: "c" },
    ];
    const layout = graphLayout(nodes);
    expect(layout.positions).toHaveLength(nodes.length);
    expect(layout.edges.map(({ from, to }) => [from, to])).toEqual([["a", "b"], ["b", "c"], ["c", "leaf"]]);
    for (const edge of layout.edges) expect(nodes.find(node => node.id === edge.to)!.parentId).toBe(edge.from);
    expect(graphLayout([...nodes].reverse())).toEqual(layout);
  });

  it("orders the task column, leaves an unassigned gap and centers each parent on its first and last child", () => {
    const nodes = Object.freeze([
      Object.freeze({ id: "root", parentId: null }),
      Object.freeze({ id: "task-b", parentId: "root", minColumn: 1, order: [0, 2] }),
      Object.freeze({ id: "task-a", parentId: "root", minColumn: 1, order: [0, 1] }),
      Object.freeze({ id: "worker", parentId: "task-a", minColumn: 2 }),
      Object.freeze({ id: "sub-a", parentId: "worker", minColumn: 3 }),
      Object.freeze({ id: "sub-b", parentId: "worker", minColumn: 3 }),
      Object.freeze({ id: "unassigned", parentId: "root", minColumn: 1, order: [3, 0], gapBefore: 0.4 }),
    ]);
    const layout = graphLayout(nodes);
    const positions = new Map(layout.positions.map(position => [position.id, position]));
    expect(layout.order).toEqual(["root", "task-a", "worker", "sub-a", "sub-b", "task-b", "unassigned"]);
    expect(positions.get("sub-a")).toMatchObject({ column: 3, row: 0, x: 824, y: 8, width: 240, height: 48 });
    expect(positions.get("sub-b")).toMatchObject({ column: 3, row: 1, y: 64 });
    expect(positions.get("task-a")!.row).toBe(0.5);
    expect(positions.get("task-b")!.row).toBe(2);
    expect(positions.get("unassigned")!.row).toBe(3.4);
    expect(positions.get("root")!.row).toBe(1.95);
    expect(layout.columns).toBe(4);
    expect(graphLayout([...nodes].reverse())).toEqual(layout);
    expect(layout.parentOf.get("worker")).toBe("task-a");
  });

  it("handles deep parent chains without recursive traversal", () => {
    const nodes = Array.from({ length: 2000 }, (_, index) => ({ id: `node-${index}`, parentId: index ? `node-${index - 1}` : null }));
    const layout = graphLayout(nodes);
    expect(layout.positions).toHaveLength(nodes.length);
    expect(layout.edges).toHaveLength(nodes.length - 1);
    expect(layout.positions.every(node => Number.isFinite(node.x) && Number.isFinite(node.y))).toBe(true);
  });
});
