import { describe, expect, it } from "vitest";
import type { CleanupResourceSummary, CleanupState, OrchestrationSnapshot, Run, RunCleanupSummary, TaskView } from "../../protocol/generated/v1";
import { cleanupItemView, cleanupResourceLabel, cleanupView, keptTaskCount, outcomeView } from "./retirementView";

const at = "2026-10-07T12:00:00Z";
function resource(state: CleanupState, kind: CleanupResourceSummary["kind"] = "worktree"): CleanupResourceSummary {
  return { resource_id: kind, kind, ownership: "owned", cleanup: { operation_id: null, state, reason: state === "kept" ? { class: "user_work", code: "dirty", message: "uncommitted changes" } : null, effects: [], updated_at: at } };
}
function summary(...items: CleanupResourceSummary[]): RunCleanupSummary {
  return { run_id: "worker", items, removed_count: items.filter(item => item.cleanup.state === "removed").length, kept_count: items.filter(item => item.cleanup.state === "kept").length, waiting_count: items.filter(item => !["removed", "kept", "unknown"].includes(item.cleanup.state)).length, unknown_count: items.filter(item => item.cleanup.state === "unknown").length };
}
function worker(): Run {
  return { session_id: "session", brief: "", run_id: "worker", root_id: "root", kind: "worker", parent_run_id: "root", task_id: "task", label: "Worker", attempt: 1, task_revision_at_start: "revision", stage: "closed", close_reason: "accepted", dispatch: null, target: null, setup: null, cleanup_policy: null, last_report: null, result: { message_id: "result", kind: "result", outcome: "succeeded", summary: "Verified work", plan: null, source_output: null, at }, source_output: { kind: "code", repository_key: "repo", checkout_key: "checkout", base_commit: "base", head_commit: "head", commits: ["head"], task_revision: "revision", result_message_id: "result", focused_checks: [] }, integration: { state: "pending" }, annotations: [], project: null, checkout: null, attachment: null, checkout_claim_id: null, resume: null, retirement: null, created_at: at, updated_at: at };
}

describe("independent source, integration and cleanup truth", () => {
  it("does not infer integration from acceptance or cleanup removal", () => {
    const run = worker();
    expect(outcomeView(run)).toEqual({ label: "Source complete", detail: "Waiting for integration" });
    expect(cleanupView(summary(resource("removed")))?.label).toBe("Cleaned up");
    expect(outcomeView(run)?.label).toBe("Source complete");
    expect(outcomeView({ ...run, stage: "reported", close_reason: null })?.detail).toBe("Awaiting acceptance");
    expect(outcomeView({ ...run, result: { ...run.result!, outcome: "failed" } })).toBeNull();
  });
  it.each(["ancestor", "patch_equivalent", "recorded_adaptation"] as const)("renders only a recorded %s integration while protected later work stays kept", method => {
    const run = worker();
    run.integration = { state: "integrated", operation_id: "integration", accepted_result_message_id: "result", accepted_task_revision: "revision", source: run.source_output!, target_repository_key: "repo", target_ref: "main", target_commit: "target", method, mappings: [], checks: [], integrator_run_id: "root", verified_at: at };
    expect(outcomeView(run)?.label).toBe("Integrated");
    expect(outcomeView(run)?.detail).toContain("main");
    expect(cleanupView(summary(resource("kept")))?.tier).toBe("notice");
  });
  it("reserves not-applicable Done for the matching non-code Result, not borrowed coding work", () => {
    const run = worker(); run.integration = { state: "not_applicable", reason: "Research", result_message_id: "result" };
    expect(outcomeView(run)?.label).toBe("Source complete");
    run.source_output = { kind: "non_code", result_message_id: "result", task_revision: "revision", artifacts: ["research"], focused_checks: [] };
    expect(outcomeView(run)?.label).toBe("Done");
    run.integration.result_message_id = "another-result";
    expect(outcomeView(run)?.label).toBe("Source complete");
  });
  it.each(["pending", "running", "waiting", "waiting_integration", "reconciling"] as const)("keeps %s cleanup quiet and does not claim removal", state => {
    const item = resource(state);
    expect(cleanupView(summary(item))?.tier).toBeNull();
    expect(cleanupItemView(item.cleanup)).toMatch(/^Waiting/);
    expect(cleanupView(summary(item))?.label).not.toBe("Cleaned up");
  });
  it("gives unknown outcomes recovery precedence over protected work and keeps resource rows independent", () => {
    const view = cleanupView(summary(resource("kept"), resource("unknown", "worker_terminal"), resource("removed", "branch")));
    expect(view?.tier).toBe("recover"); expect(view?.label).toBe("Cleanup not confirmed");
    expect(cleanupItemView(resource("kept").cleanup)).toBe("Kept — uncommitted changes");
    expect(cleanupResourceLabel(resource("pending", "fixture_set"))).toBe("Disposable fixture set");
    expect(cleanupView(summary(resource("stalled")))?.tier).toBe("notice");
    expect(cleanupView(undefined)).toBeNull();
    expect(cleanupView(summary())?.label).toBe("No managed resources to clean up");
  });
  it("counts completed tasks once across multiple kept resources and historical runs, excluding another root", () => {
    const run = worker();
    const task: TaskView = { task: { task_id: "task", title: "Work", body: "", description: "", description_editable: true, description_diagnostic: null, steps: [], step_progress: { done: 0, total: 0 }, steps_diagnostic: null, depends_on: [], follow_up_of: null, relations_diagnostic: null, checked: true, line: 1, task_revision: "revision", diagnostic: null }, lane: "accepted", current_run_id: "new-run", dependencies: { state: "none", unmet: [], problems: [] } };
    const state: OrchestrationSnapshot = { state_store_id: "store", session_id: "session", revision: 1, tasks_token: "tasks", roots: [], board: { root_id: "root", path: "tasks", doc_revision: "doc", unidentified_items: 0, diagnostics: [], tasks: [task] }, runs: [run, { ...run, run_id: "old" }, { ...run, run_id: "foreign", root_id: "other", task_id: "foreign-task" }], cleanup: [summary(resource("kept"), resource("kept", "branch")), { ...summary(resource("kept")), run_id: "old" }, { ...summary(resource("kept")), run_id: "foreign" }], restoration: { reattached: 0, total_attachments: 0, cleanup_pending: 0 }, messages: [], questions: [], subagents: [], intents: [], assignment_intents: [], unmanaged_agents: [], attention: [], runtime: { status: "unavailable", error: { code: "offline", message: "Offline" } } };
    expect(keptTaskCount(state, [task, task])).toBe(1);
    expect(keptTaskCount(state, [{ ...task, lane: "working" }])).toBe(0);
  });
});
