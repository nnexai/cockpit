import { describe, expect, it } from "vitest";
import type { Run, TaskView } from "../../protocol/generated/v1";
import { dependencyCandidateReason, dependencyChain, dependencySummary, dependentCount, uniqueTask } from "./dependencies";

function task(id: string, content: Partial<TaskView["task"]> = {}, dependencies: TaskView["dependencies"] = { state: "none", unmet: [], problems: [] }): TaskView {
  return {
    task: {
      task_id: id, title: id, body: "Description", description: "Description", description_editable: true,
      description_diagnostic: null, steps: [], step_progress: { done: 0, total: 0 }, steps_diagnostic: null,
      depends_on: [], follow_up_of: null, relations_diagnostic: null, checked: false, line: 1,
      task_revision: `revision-${id}`, diagnostic: null, ...content,
    },
    lane: content.checked ? "accepted" : "queued", current_run_id: null, dependencies,
  };
}

function resultRun(taskId: string): Run {
  const at = "2026-10-08T00:00:00Z";
  const result: Run["result"] = { message_id: "result", kind: "result", outcome: "succeeded", summary: "Work succeeded", plan: null, at };
  return {
    session_id: "session", prepare_brief: "", run_id: "worker", kind: "worker", label: "Worker", root_id: "root",
    parent_run_id: "root", task_id: taskId, attempt: 1, task_revision_at_propose: `revision-${taskId}`,
    stage: "reported", close_reason: null, dispatch: null, target: null, setup: null, prepare_plan: null,
    init_receipt: null, work_plan: null, grants: [], last_report: result, result, annotations: [], location: null,
    bound_omp_session: null, bound_omp_process: null, launch_shell_identity: null, retirement: null,
    supersedes_run_id: null, created_at: at, updated_at: at,
  };
}

describe("canonical task dependencies", () => {
  it("resolves only a unique canonical task, including checked tasks", () => {
    const accepted = task("prerequisite", { checked: true });
    expect(uniqueTask([accepted], "prerequisite")).toBe(accepted);
    expect(uniqueTask([accepted], "missing")).toBeNull();
    expect(uniqueTask([accepted, task("prerequisite")], "prerequisite")).toBeNull();
  });

  it("keeps a successful Result blocked until canonical acceptance satisfies the prerequisite", () => {
    const prerequisite = task("prerequisite", { title: "Prepare inputs" });
    const worker = resultRun(prerequisite.task.task_id);
    prerequisite.current_run_id = worker.run_id;
    prerequisite.lane = "review";
    const dependent = task("dependent", { depends_on: ["prerequisite"] }, {
      state: "blocked", unmet: [{ task_id: "prerequisite", reason: "unchecked" }], problems: [],
    });
    expect(worker.result?.outcome).toBe("succeeded");
    expect(dependencySummary(dependent, [prerequisite, dependent])).toBe("Waiting on “Prepare inputs”");

    prerequisite.task.checked = true;
    prerequisite.lane = "accepted";
    dependent.dependencies = { state: "satisfied", unmet: [], problems: [] };
    expect(dependencySummary(dependent, [prerequisite, dependent])).toBeNull();
  });

  it("trusts canonical dependency state rather than recomputing from locally visible checked flags", () => {
    const prerequisite = task("prerequisite");
    const dependent = task("dependent", { depends_on: ["prerequisite"] }, { state: "satisfied", unmet: [], problems: [] });
    expect(dependencySummary(dependent, [prerequisite, dependent])).toBeNull();

    prerequisite.task.checked = true;
    dependent.dependencies = { state: "blocked", unmet: [{ task_id: "prerequisite", reason: "unchecked" }], problems: [] };
    expect(dependencySummary(dependent, [prerequisite, dependent])).toBe("Waiting on “prerequisite”");
  });

  it("uses identifiers for missing or ambiguous prerequisites, never an arbitrary duplicate title", () => {
    const dependent = task("dependent", { depends_on: ["missing", "duplicate"] }, {
      state: "blocked", unmet: [{ task_id: "missing", reason: "missing" }, { task_id: "duplicate", reason: "ambiguous" }], problems: [],
    });
    const tasks = [dependent, task("duplicate", { title: "First duplicate" }), task("duplicate", { title: "Second duplicate" })];
    expect(dependencySummary(dependent, tasks)).toBe("Waiting on 2 · “missing” +1");
    dependent.dependencies.unmet = [{ task_id: "duplicate", reason: "ambiguous" }];
    expect(dependencySummary(dependent, tasks)).toBe("Waiting on “duplicate”");
  });

  it("surfaces canonical invalidity and suppresses prerequisite summaries for accepted tasks", () => {
    const dependent = task("dependent", { relations_diagnostic: "Unreadable metadata" }, {
      state: "invalid", unmet: [], problems: [{ code: "cycle", message: "Dependency cycle" }],
    });
    expect(dependencySummary(dependent, [dependent])).toBe("Prerequisites need fixing · Dependency cycle");
    dependent.dependencies.problems = [];
    expect(dependencySummary(dependent, [dependent])).toBe("Prerequisites need fixing · Unreadable metadata");
    dependent.task.checked = true;
    expect(dependencySummary(dependent, [dependent])).toBeNull();
  });

  it("refuses missing, ambiguous and diagnostic prerequisite candidates", () => {
    const tasks = [task("edited"), task("duplicate"), task("duplicate"), task("diagnostic", { diagnostic: "Duplicate identity" }),
      task("invalid", {}, { state: "invalid", unmet: [], problems: [{ code: "cycle", message: "Dependency cycle" }] })];
    for (const id of ["missing", "duplicate", "diagnostic", "invalid"]) {
      expect(dependencyCandidateReason(tasks, "edited", id)).toBeTruthy();
    }
    expect(dependencyCandidateReason(tasks, "edited", "edited")).toMatch(/itself/);
  });

  it("disables direct and transitive cycle candidates without disabling independent healthy work", () => {
    const tasks = [task("edited"), task("direct", { depends_on: ["edited"] }), task("transitive", { depends_on: ["direct"] }), task("healthy")];
    expect(dependencyCandidateReason(tasks, "edited", "direct")).toMatch(/cycle/);
    expect(dependencyCandidateReason(tasks, "edited", "transitive")).toMatch(/cycle/);
    expect(dependencyCandidateReason(tasks, "edited", "healthy")).toBeNull();
  });

  it("highlights upstream and downstream chains without expanding siblings through shared relatives", () => {
    const tasks = [task("ancestor"), task("upstream", { depends_on: ["ancestor"] }), task("selected", { depends_on: ["upstream"] }),
      task("sibling", { depends_on: ["upstream"] }), task("unrelated"), task("downstream", { depends_on: ["selected", "unrelated"] }),
      task("leaf", { depends_on: ["downstream"] })];
    expect([...dependencyChain(tasks, ["selected"])].sort()).toEqual(["ancestor", "downstream", "leaf", "selected", "upstream"]);
  });

  it("bounds chain traversal across cycles and unions explicitly selected independent chains", () => {
    const tasks = [task("a", { depends_on: ["b"] }), task("b", { depends_on: ["a"] }), task("self", { depends_on: ["self"] }), task("independent")];
    expect([...dependencyChain(tasks, ["a", "self", "independent"])].sort()).toEqual(["a", "b", "independent", "self"]);
  });

  it("counts only unchecked dependents with canonical unmet prerequisites", () => {
    const prerequisite = task("prerequisite");
    const blocked = { state: "blocked", unmet: [{ task_id: "prerequisite", reason: "unchecked" }], problems: [] } satisfies TaskView["dependencies"];
    const tasks = [prerequisite, task("waiting", { depends_on: ["prerequisite"] }, blocked), task("accepted", { checked: true }, blocked),
      task("satisfied", { depends_on: ["prerequisite"] }, { state: "satisfied", unmet: [], problems: [] }), task("independent")];
    expect(dependentCount(prerequisite, tasks)).toBe(1);
  });
});
