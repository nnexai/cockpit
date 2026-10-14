import { describe, expect, it } from "vitest";
import type { TaskView } from "../../protocol/generated/v1";
import { dependencyLayout } from "./dependencyLayout";
import { taskFixture } from "./testFixtures";

function task(id: string, content: Partial<TaskView["task"]> = {}): TaskView {
  const view = taskFixture(id);
  return { ...view, task: { ...view.task, ...content } };
}
describe("task dependency layout", () => {
  it("directs prerequisites left-to-right, keeps task nodes reachable and avoids overlaps", () => {
    const tasks = [task("leaf", { depends_on: ["middle", "branch"] }), task("middle", { depends_on: ["root"] }), task("branch", { depends_on: ["root"] }), task("root"), task("independent")];
    const layout = dependencyLayout(tasks), positions = new Map(layout.positions.map(position => [position.view.task.task_id, position]));
    expect(layout.edges).toHaveLength(4);
    for (const edge of layout.edges) expect(positions.get(edge.from)!.level).toBeLessThan(positions.get(edge.to)!.level);
    expect(layout.positions).toHaveLength(tasks.length);
    for (let index = 0; index < layout.positions.length; index++) for (const other of layout.positions.slice(index + 1)) {
      const position = layout.positions[index];
      expect(position.x + 240 <= other.x || other.x + 240 <= position.x || position.y + 48 <= other.y || other.y + 48 <= position.y).toBe(true);
    }
  });
  it("does not turn follow-up provenance into dependency edges", () => {
    const layout = dependencyLayout([task("origin"), task("follow-up", { follow_up_of: "origin" }), task("dependent", { depends_on: ["origin"], follow_up_of: "origin" })]);
    expect(layout.edges).toEqual([{ from: "origin", to: "dependent" }]);
  });
  it("keeps cycles and self-cycles visible without losing downstream tasks", () => {
    const tasks = [task("a", { depends_on: ["c"] }), task("b", { depends_on: ["a"] }), task("c", { depends_on: ["b"] }), task("self", { depends_on: ["self"] }), task("downstream", { depends_on: ["c"] }), task("independent")];
    const layout = dependencyLayout(tasks), positions = new Map(layout.positions.map(position => [position.view.task.task_id, position]));
    expect(layout.positions).toHaveLength(tasks.length);
    for (const id of ["a", "b", "c", "self"]) expect(positions.get(id)?.cycle).toBe(true);
    expect(positions.get("downstream")!.level).toBeGreaterThan(positions.get("c")!.level);
    expect(positions.get("downstream")!.cycle).toBe(false);
    expect(layout.edges).toContainEqual({ from: "self", to: "self" });
  });
  it("bounds a large cyclic component and preserves healthy work after it", () => {
    const size = 256;
    const tasks = Array.from({ length: size }, (_, index) => task(`cycle-${index}`, { depends_on: [`cycle-${(index + size - 1) % size}`], line: index + 1 }));
    tasks.push(task("healthy", { line: size + 1 }));
    const layout = dependencyLayout(tasks);
    expect(layout.positions).toHaveLength(size + 1);
    expect(layout.positions.filter(position => position.cycle)).toHaveLength(size);
    expect(new Set(layout.positions.filter(position => position.cycle).map(position => position.level)).size).toBe(1);
    for (const position of layout.positions) expect(Number.isFinite(position.x) && Number.isFinite(position.y)).toBe(true);
  });
  it("excludes duplicate identities and missing references without inventing nodes", () => {
    const layout = dependencyLayout([task("duplicate"), task("duplicate"), task("dependent", { depends_on: ["duplicate", "missing"] }), task("healthy"), task("healthy-dependent", { depends_on: ["healthy"] })]);
    expect(layout.positions.map(position => position.view.task.task_id).sort()).toEqual(["dependent", "healthy", "healthy-dependent"]);
    expect(layout.edges).toEqual([{ from: "healthy", to: "healthy-dependent" }]);
  });
});
