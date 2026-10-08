import { describe, expect, it } from "vitest";
import type { TaskLane, TaskView } from "../../protocol/generated/v1";
import { taskNeighbor, visibleTaskIds, type BoardNavOptions } from "./boardNavigation";

function task(id: string, lane: TaskLane): TaskView {
  return { task: { task_id: id, title: id, body: id, description: id, description_editable: true, description_diagnostic: null, steps: [], step_progress: { done: 0, total: 0 }, steps_diagnostic: null, depends_on: [], follow_up_of: null, relations_diagnostic: null, checked: lane === "accepted", line: 1, task_revision: `revision-${id}`, diagnostic: null }, lane, current_run_id: `worker-${id}`, dependencies: { state: "none", unmet: [], problems: [] } };
}
const tasks = [task("queued-a", "queued"), task("ready-a", "ready"), task("queued-b", "queued"), task("working-a", "working"), task("ready-b", "ready"), task("done-a", "accepted")];
const lanes: BoardNavOptions = { arrangement: "lanes", completedOpen: false, collapsedLanes: [] };
const stacked: BoardNavOptions = { arrangement: "stacked", completedOpen: false, collapsedLanes: [] };

describe("task focus-only navigation", () => {
  it.each([
    ["queued-a", "ArrowDown", "queued-b"],
    ["queued-b", "ArrowDown", "queued-b"],
    ["queued-b", "ArrowUp", "queued-a"],
    ["queued-a", "ArrowUp", "queued-a"],
    ["queued-b", "ArrowRight", "ready-b"],
    ["ready-b", "ArrowRight", "working-a"],
    ["working-a", "ArrowLeft", "ready-a"],
    ["ready-b", "ArrowLeft", "queued-b"],
    ["working-a", "ArrowRight", "working-a"],
    ["queued-a", "ArrowLeft", "queued-a"],
    ["ready-b", "Home", "ready-a"],
    ["ready-a", "End", "ready-b"],
  ])("keeps wide lane row affinity: %s %s → %s", (id, key, expected) => {
    expect(taskNeighbor(tasks, id, key, lanes)).toBe(expected);
  });

  it("moves vertically across stacked lane boundaries, with Home/End spanning the visible board", () => {
    expect(visibleTaskIds(tasks, stacked)).toEqual(["queued-a", "queued-b", "ready-a", "ready-b", "working-a"]);
    expect(taskNeighbor(tasks, "queued-b", "ArrowDown", stacked)).toBe("ready-a");
    expect(taskNeighbor(tasks, "ready-a", "ArrowUp", stacked)).toBe("queued-b");
    expect(taskNeighbor(tasks, "ready-a", "Home", stacked)).toBe("queued-a");
    expect(taskNeighbor(tasks, "queued-b", "End", stacked)).toBe("working-a");
    expect(taskNeighbor(tasks, "working-a", "ArrowDown", stacked)).toBe("working-a");
    expect(taskNeighbor(tasks, "queued-a", "ArrowUp", stacked)).toBe("queued-a");
    expect(taskNeighbor(tasks, "ready-a", "ArrowLeft", stacked)).toBeUndefined();
    expect(taskNeighbor(tasks, "ready-a", "ArrowRight", stacked)).toBeUndefined();
  });

  it("skips collapsed stacked lanes and stops targeting cards as their lane closes", () => {
    const options: BoardNavOptions = { ...stacked, collapsedLanes: ["ready"] };
    expect(visibleTaskIds(tasks, options)).toEqual(["queued-a", "queued-b", "working-a"]);
    expect(taskNeighbor(tasks, "queued-b", "ArrowDown", options)).toBe("working-a");
    expect(taskNeighbor(tasks, "working-a", "ArrowUp", options)).toBe("queued-b");
    expect(taskNeighbor(tasks, "ready-a", "Home", options)).toBeUndefined();
    // A remembered stacked disclosure does not remove a wide lane.
    expect(taskNeighbor(tasks, "queued-b", "ArrowRight", { ...lanes, collapsedLanes: ["ready"] })).toBe("ready-b");
  });

  it("makes completed tasks reachable only while their disclosure is open", () => {
    expect(taskNeighbor(tasks, "working-a", "ArrowRight", lanes)).toBe("working-a");
    expect(taskNeighbor(tasks, "working-a", "ArrowRight", { ...lanes, completedOpen: true })).toBe("done-a");
    expect(taskNeighbor(tasks, "working-a", "ArrowDown", stacked)).toBe("working-a");
    expect(taskNeighbor(tasks, "working-a", "ArrowDown", { ...stacked, completedOpen: true })).toBe("done-a");
    expect(visibleTaskIds(tasks, { ...stacked, completedOpen: true }).at(-1)).toBe("done-a");
    expect(taskNeighbor(tasks, "done-a", "ArrowUp", stacked)).toBeUndefined();
    expect(visibleTaskIds(tasks, { ...stacked, completedOpen: true, collapsedLanes: ["accepted"] })).not.toContain("done-a");
  });

  it("does not activate, select, reorder or rewrite canonical tasks when calculating a focus move", () => {
    const frozen = Object.freeze(tasks.map(item => Object.freeze({ ...item, task: Object.freeze({ ...item.task }) })));
    const before = JSON.stringify(frozen);
    for (const options of [lanes, stacked]) {
      for (const key of ["ArrowUp", "ArrowDown", "ArrowLeft", "ArrowRight", "Home", "End"]) taskNeighbor(frozen, "ready-b", key, options);
      expect(taskNeighbor(frozen, "ready-b", "Enter", options)).toBeUndefined();
      expect(taskNeighbor(frozen, "ready-b", " ", options)).toBeUndefined();
      expect(taskNeighbor(frozen, "missing", "Home", options)).toBeUndefined();
      expect(taskNeighbor([], "missing", "Home", options)).toBeUndefined();
    }
    expect(JSON.stringify(frozen)).toBe(before);
    expect(visibleTaskIds([], stacked)).toEqual([]);
    expect(visibleTaskIds(tasks, { ...stacked, collapsedLanes: ["queued", "ready", "working"] })).toEqual([]);
  });
});
