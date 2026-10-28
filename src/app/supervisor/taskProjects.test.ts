import { describe, expect, it } from "vitest";
import type { TaskLane } from "../../protocol/generated/v1";
import { groupTasks, projectOf } from "./taskProjects";
import { runFixture, taskFixture } from "./testFixtures";

function task(id: string, key: string | null, lane: TaskLane, line: number, checked = false) {
  const view = taskFixture(id, { project: key === null ? null : { key, label: key }, lane });
  view.task = { ...view.task, line, checked };
  return view;
}

describe("task projects", () => {
  it("distinguishes General from provisional project attribution", () => {
    expect(projectOf(taskFixture("general"))).toEqual({ key: "", label: "General" });
    expect(projectOf(taskFixture("pending", { project: { key: null, label: "cockpit" } }))).toEqual({ key: "provisional:cockpit", label: "cockpit" });
    expect(projectOf(taskFixture("resolved", { project: { key: "repo:cockpit", label: "cockpit" } }))).toEqual({ key: "repo:cockpit", label: "cockpit" });
  });

  it("orders live, idle and done-only projects by activity, with General always last", () => {
    const tasks = [task("general", null, "working", 10), task("idle", "idle", "ready", 2), task("live-old", "alpha", "working", 3), task("live-new", "beta", "review", 4), task("done", "done", "accepted", 5, true)];
    const runs = [runFixture({ task_id: "live-old", updated_at: "2026-01-01T00:00:00Z" }), runFixture({ task_id: "live-new", updated_at: "2026-02-01T00:00:00Z" })];
    expect(groupTasks(tasks, runs).map(group => group.label)).toEqual(["beta", "alpha", "idle", "done", "General"]);
  });

  it("counts the full group and orders open rows by line and done rows by completion", () => {
    const tasks = [task("open-later", "one", "ready", 9), task("old", "one", "accepted", 4, true), task("open-first", "one", "blocked", 1), task("new", "one", "accepted", 2, true)];
    const runs = [runFixture({ task_id: "old", stage: "closed", close_reason: "accepted", updated_at: "2026-01-01T00:00:00Z" }), runFixture({ task_id: "new", stage: "closed", close_reason: "accepted", updated_at: "2026-02-01T00:00:00Z" })];
    const [group] = groupTasks(tasks, runs);
    expect(group).toMatchObject({ open: 2, done: 2, live: false });
    expect(group.tasks.map(view => view.task.task_id)).toEqual(["open-first", "open-later", "new", "old"]);
  });

  it("breaks activity ties by case-insensitive label and falls back to line", () => {
    const tasks = [task("z", "zebra", "ready", 4), task("a", "Alpha", "ready", 4), task("latest", "latest", "ready", 8)];
    expect(groupTasks(tasks, []).map(group => group.label)).toEqual(["latest", "Alpha", "zebra"]);
  });
});
