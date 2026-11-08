import { describe, expect, it } from "vitest";
import { taskActivity } from "./taskActivity";
import { orchestrationFixture, runFixture, taskFixture } from "./testFixtures";

describe("task activity", () => {
  const task = taskFixture("work", { lane: "working", current_run_id: "worker" });
  const run = runFixture({ kind: "worker", run_id: "worker", task_id: "work", stage: "working" });
  const snapshot = orchestrationFixture([task], [run]);
  if (snapshot.runtime.status !== "fresh") throw new Error("expected fresh fixture");
  const freshRuntime = snapshot.runtime;
  const observation = freshRuntime.runs[0];
  const activity = (view = task, fresh = true) => taskActivity(view, snapshot, "root", fresh);

  it("requires a fresh observed working agent", () => {
    expect(activity()?.word).toBe("working");
    expect(activity(task, false)?.word).toBe("unconfirmed");
    expect(taskActivity(task, { ...snapshot, runtime: { status: "unavailable", error: { code: "offline", message: "offline" } } }, "root", true)?.word).toBe("unconfirmed");
    for (const [presence, actual, status, expected] of [
      ["present", true, "idle", "idle"], ["present", true, "blocked", "runtime blocked"],
      ["missing", false, null, "not observed"], ["unobserved", false, null, "unconfirmed"],
      ["present", false, "working", "unconfirmed"], ["present", true, null, "unconfirmed"],
    ] as const) {
      const runtime = { ...freshRuntime, runs: [{ ...observation, presence, actual_agent: actual, agent_status: status }] };
      expect(taskActivity(task, { ...snapshot, runtime }, "root", true)?.word).toBe(expected);
    }
  });

  it("prioritizes explicit waits over observed working", () => {
    const waiting = { cause: "worker_waiting" as const, since: null, held_by: [], code: null, detail: null, next_automatic: false };
    expect(activity({ ...task, worker_waiting: waiting })?.word).toBe("waiting · worker says");
    expect(activity({ ...task, waiting: { ...waiting, cause: "question" } })?.word).toBe("waiting for answer");
  });

  it("shows cancelled tracking on hold without claiming the process stopped", () => {
    const held = { ...task, lane: "ready" as const };
    const cancelled = { ...run, stage: "closed" as const, close_reason: "cancelled" as const };
    const result = taskActivity(held, { ...snapshot, runs: [cancelled] }, "root", true);
    expect(result?.word).toBe("on hold");
    expect(result?.detail).toContain("does not stop the worker process");
    expect(result?.detail).toContain("Terminal observed working");
    expect(taskActivity({ ...held, current_run_id: null }, snapshot, "root", true)).toBeNull();
  });
});
