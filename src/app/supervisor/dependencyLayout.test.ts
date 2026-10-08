import { describe, expect, it } from "vitest";
import type { TaskView } from "../../protocol/generated/v1";
import { dependencyLayout, type DependencyPosition } from "./dependencyLayout";

function task(id: string, content: Partial<TaskView["task"]> = {}): TaskView {
  return {
    task: {
      task_id: id, title: id, body: "Description", description: "Description", description_editable: true,
      description_diagnostic: null, steps: [], step_progress: { done: 0, total: 0 }, steps_diagnostic: null,
      depends_on: [], follow_up_of: null, relations_diagnostic: null, checked: false, line: 1,
      task_revision: `revision-${id}`, diagnostic: null, ...content,
    },
    lane: content.checked ? "accepted" : "queued", current_run_id: null,
    dependencies: { state: "none", unmet: [], problems: [] },
  };
}

function expectFiniteLayout(positions: readonly DependencyPosition[], width: number, height: number, independentTop: number) {
  expect(Number.isFinite(width) && width > 0).toBe(true);
  expect(Number.isFinite(height) && height > 0).toBe(true);
  expect(Number.isFinite(independentTop)).toBe(true);
  for (const position of positions) {
    expect(Number.isFinite(position.x) && Number.isFinite(position.y) && Number.isFinite(position.level)).toBe(true);
    expect(position.x).toBeGreaterThanOrEqual(0);
    expect(position.y).toBeGreaterThanOrEqual(0);
    expect(position.x).toBeLessThan(width);
    expect(position.y).toBeLessThan(height);
    expect(position.level).toBeLessThanOrEqual(positions.length);
  }
}

describe("task dependency layout", () => {
  it("directs prerequisites toward dependents and places predecessor levels before successor levels", () => {
    const tasks = [task("leaf", { depends_on: ["middle", "branch"] }), task("middle", { depends_on: ["root"] }),
      task("branch", { depends_on: ["root"] }), task("root")];
    const layout = dependencyLayout(tasks, false);
    expect(layout.positions).toHaveLength(tasks.length);
    expect(layout.edges).toEqual(expect.arrayContaining([
      { from: "middle", to: "leaf", provenance: false }, { from: "branch", to: "leaf", provenance: false },
      { from: "root", to: "middle", provenance: false }, { from: "root", to: "branch", provenance: false },
    ]));
    expect(layout.edges).toHaveLength(4);
    const positions = new Map(layout.positions.map(position => [position.view.task.task_id, position]));
    for (const edge of layout.edges) expect(positions.get(edge.from)!.level).toBeLessThan(positions.get(edge.to)!.level);
    expectFiniteLayout(layout.positions, layout.width, layout.height, layout.independentTop);
  });

  it("keeps follow-up provenance separate and emits one prerequisite edge when both relations share endpoints", () => {
    const tasks = [task("origin"), task("follow-up", { follow_up_of: "origin" }),
      task("both", { depends_on: ["origin"], follow_up_of: "origin" })];
    const layout = dependencyLayout(tasks, false);
    expect(layout.edges).toHaveLength(2);
    expect(layout.edges).toEqual(expect.arrayContaining([
      { from: "origin", to: "follow-up", provenance: true }, { from: "origin", to: "both", provenance: false },
    ]));
    const positions = new Map(layout.positions.map(position => [position.view.task.task_id, position]));
    expect(positions.get("follow-up")!.level).toBe(positions.get("origin")!.level);
    expect(positions.get("both")!.level).toBeGreaterThan(positions.get("origin")!.level);
    expect(positions.get("follow-up")!.cycle).toBe(false);
    expectFiniteLayout(layout.positions, layout.width, layout.height, layout.independentTop);
  });

  it("retains accepted prerequisite and provenance context while hiding isolated accepted work", () => {
    const tasks = [task("accepted-origin", { checked: true }),
      task("accepted-middle", { checked: true, follow_up_of: "accepted-origin" }),
      task("accepted-prerequisite", { checked: true, depends_on: ["accepted-middle"] }),
      task("active", { depends_on: ["accepted-prerequisite"] }), task("isolated-accepted", { checked: true }), task("independent")];
    const layout = dependencyLayout(tasks, false);
    expect(layout.positions.map(position => position.view.task.task_id).sort()).toEqual([
      "accepted-middle", "accepted-origin", "accepted-prerequisite", "active", "independent",
    ]);
    expect(layout.hiddenCompleted).toBe(1);
    expect(layout.edges).toHaveLength(3);
    expectFiniteLayout(layout.positions, layout.width, layout.height, layout.independentTop);

    const withCompleted = dependencyLayout(tasks, true);
    expect(withCompleted.positions).toHaveLength(tasks.length);
    expect(withCompleted.hiddenCompleted).toBe(0);
    expect(withCompleted.positions.find(position => position.view.task.task_id === "isolated-accepted")?.independent).toBe(true);
  });

  it("keeps cycles and self-cycles visible without losing later independent or downstream tasks", () => {
    const tasks = [task("a", { depends_on: ["c"] }), task("b", { depends_on: ["a"] }), task("c", { depends_on: ["b"] }),
      task("self", { depends_on: ["self"] }), task("downstream", { depends_on: ["c"] }), task("later-independent")];
    const layout = dependencyLayout(tasks, false);
    expect(layout.positions).toHaveLength(tasks.length);
    expect(layout.edges).toHaveLength(5);
    const positions = new Map(layout.positions.map(position => [position.view.task.task_id, position]));
    for (const id of ["a", "b", "c", "self"]) expect(positions.get(id)?.cycle).toBe(true);
    expect(positions.get("a")!.level).toBe(positions.get("b")!.level);
    expect(positions.get("b")!.level).toBe(positions.get("c")!.level);
    expect(positions.get("downstream")!.level).toBeGreaterThan(positions.get("c")!.level);
    expect(positions.get("downstream")!.cycle).toBe(false);
    expect(positions.get("later-independent")).toMatchObject({ independent: true, cycle: false });
    expect(layout.edges).toContainEqual({ from: "self", to: "self", provenance: false });
    expectFiniteLayout(layout.positions, layout.width, layout.height, layout.independentTop);
  });

  it("bounds a larger cyclic component and retains healthy work appearing after it", () => {
    const size = 256;
    const tasks = Array.from({ length: size }, (_, index) => task(`cycle-${index}`, {
      depends_on: [`cycle-${(index + size - 1) % size}`], line: index + 1,
    }));
    tasks.push(task("healthy-after-cycle", { line: size + 1 }));
    const layout = dependencyLayout(tasks, false);
    expect(layout.positions).toHaveLength(size + 1);
    expect(new Set(layout.positions.map(position => position.view.task.task_id)).size).toBe(size + 1);
    expect(layout.positions.filter(position => position.cycle)).toHaveLength(size);
    expect(new Set(layout.positions.filter(position => position.cycle).map(position => position.level)).size).toBe(1);
    expect(layout.positions.find(position => position.view.task.task_id === "healthy-after-cycle")).toMatchObject({ independent: true, cycle: false });
    expect(layout.edges).toHaveLength(size);
    expectFiniteLayout(layout.positions, layout.width, layout.height, layout.independentTop);
  });

  it("excludes duplicate identities and missing references without inventing nodes or choosing a duplicate", () => {
    const tasks = [task("duplicate", { title: "First duplicate" }), task("duplicate", { title: "Second duplicate" }),
      task("dependent", { depends_on: ["duplicate", "missing"], follow_up_of: "missing-origin" }),
      task("healthy"), task("healthy-dependent", { depends_on: ["healthy"] })];
    const layout = dependencyLayout(tasks, true);
    expect(layout.positions.map(position => position.view.task.task_id).sort()).toEqual(["dependent", "healthy", "healthy-dependent"]);
    expect(layout.edges).toEqual([{ from: "healthy", to: "healthy-dependent", provenance: false }]);
    expect(layout.hiddenCompleted).toBe(0);
    expectFiniteLayout(layout.positions, layout.width, layout.height, layout.independentTop);
  });
});
