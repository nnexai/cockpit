import { describe, expect, it } from "vitest";
import { dependencySummary, uniqueTask } from "./dependencies";
import { taskFixture } from "./testFixtures";

describe("read-only task prerequisites", () => {
  it("never chooses an ambiguous canonical task identity", () => {
    const first = taskFixture("duplicate"), second = taskFixture("duplicate");
    expect(uniqueTask([first, second], "duplicate")).toBeNull();
    expect(uniqueTask([first], "duplicate")).toBe(first);
  });
  it("shows only unmet prerequisite names and hides completed task dependencies", () => {
    const first = taskFixture("First"), second = taskFixture("Second");
    const dependent = taskFixture("dependent", { lane: "blocked", dependencies: { state: "blocked", unmet: [{ task_id: "First", reason: "unchecked" }, { task_id: "Second", reason: "unchecked" }], problems: [] } });
    expect(dependencySummary(dependent, [first, second, dependent])).toContain("First");
    expect(dependencySummary(dependent, [first, second, dependent])).not.toContain("Second");
    expect(dependencySummary({ ...dependent, task: { ...dependent.task, checked: true } }, [first, dependent])).toBeNull();
  });
  it("does not expose raw IDs or detailed repair problems for unreadable prerequisites", () => {
    const dependent = taskFixture("dependent", { lane: "blocked", dependencies: { state: "invalid", unmet: [{ task_id: "private-id", reason: "missing" }], problems: [] } });
    expect(dependencySummary(dependent, [dependent])).not.toContain("private-id");
  });
});
