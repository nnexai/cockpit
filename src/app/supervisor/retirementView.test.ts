import { describe, expect, it } from "vitest";
import type { RetirementState, Run } from "../../protocol/generated/v1";
import { retirementView } from "./retirementView";

const at = "2026-10-07T12:00:00Z";
function accepted(state: RetirementState): Run {
  const process = { pid: 4242, start_ticks: 12345, kernel_boot_id: "kernel-boot" };
  return {
    session_id: "session", prepare_brief: "Guidance", run_id: "worker", kind: "worker", label: "Worker", root_id: "root", parent_run_id: "root", task_id: "task", attempt: 1, task_revision_at_propose: "proposed", stage: "closed", close_reason: "accepted",
    dispatch: { launch_tag: "launch", endpoint_identity: "endpoint", recovery: null, agent_started: true, step: "launched", launch_attempt: 1, error: null, updated_at: at },
    target: { target: "existing_space", workspace_id: "space" }, setup: null, prepare_plan: null, init_receipt: null, work_plan: null, grants: [], last_report: null,
    result: { message_id: "result", kind: "result", outcome: "succeeded", summary: "Verified work", plan: null, at }, annotations: [],
    location: { boot_id: "herdr-boot", terminal_id: "terminal", native_session_id: "native", endpoint_identity: "endpoint", session_id: "session", workspace_id: "space", tab_id: "tab", pane_id: "pane", launch_tag: "launch" },
    bound_omp_session: "native", bound_omp_process: process,
    launch_shell_identity: null,
    retirement: { retirement_id: "retirement", trigger: "accept", result_message_id: "result", task_revision: "accepted-revision", identity: { run_attempt: 1, launch_attempt: 1, launch_tag: "launch", endpoint_identity: "endpoint", session_id: "session", workspace_id: "space", tab_id: "tab", pane_id: "pane", terminal_id: "terminal", herdr_boot_id: "herdr-boot", omp_session_id: "native", process, shell: { process: { pid: 4243, start_ticks: 12346, kernel_boot_id: "fictional-shell-boot" }, executable_device: "8", executable_inode: "9004", argv_digest: "dddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddd" } }, state, created_at: at, updated_at: at },
    supersedes_run_id: null, created_at: at, updated_at: at,
  };
}

describe("saved accepted-worker retirement truth", () => {
  it("does not infer process exit from an offer, deferral or shutdown receipt", () => {
    const offered = accepted({ state: "native_stop_offered", offered_at: at });
    const queued = { ...offered, retirement: { ...offered.retirement!, state: { state: "native_stop_deferred", offered_at: at, at, reason: "pending_messages" } as const } };
    const requested = { ...offered, retirement: { ...offered.retirement!, state: { state: "native_stop_requested", at } as const } };
    for (const current of [offered, queued, requested]) {
      expect(retirementView(current)?.nativeStopped).toBe(false);
      expect(retirementView(current)?.label).not.toMatch(/worker stopped|worker retired/);
    }
    const stopped = { ...offered, retirement: { ...offered.retirement!, state: { state: "native_stopped", at, evidence: "exited_after_shutdown_request" } as const } };
    expect(retirementView(stopped)?.nativeStopped).toBe(true);
    expect(retirementView(stopped)?.label).toContain("worker stopped");
    const closing = { ...offered, retirement: { ...offered.retirement!, state: { state: "close_intent", at } as const } };
    expect(retirementView(closing)?.nativeStopped).toBe(true);
    expect(retirementView(closing)?.label).not.toContain("worker retired");
    const retired = { ...offered, retirement: { ...offered.retirement!, state: { state: "retired", at, terminal: "closed_by_cockpit" } as const } };
    expect(retirementView(retired)?.nativeStopped).toBe(true);
    expect(retirementView(retired)?.label).toContain("worker retired");
  });

  it("keeps blocker and user-draft notices informational while the worker can still be running", () => {
    const waiting = accepted({ state: "waiting", blockers: ["open_descendant_runs", "running_subagents"] });
    expect(retirementView(waiting)?.tier).toBe("notice");
    expect(retirementView(waiting)?.nativeStopped).toBe(false);
    expect(retirementView(waiting)?.detail).toContain("open descendant runs and running subagents");
    waiting.retirement!.state = { state: "waiting", blockers: [] };
    expect(retirementView(waiting)?.tier).toBeNull();
    expect(retirementView(waiting)?.nativeStopped).toBe(false);
    waiting.retirement!.state = { state: "native_stop_deferred", offered_at: at, at, reason: "editor_draft" };
    expect(retirementView(waiting)?.tier).toBe("notice");
    expect(retirementView(waiting)?.nativeStopped).toBe(false);
    waiting.retirement!.state = { state: "native_stop_deferred", offered_at: at, at, reason: "async_jobs" };
    expect(retirementView(waiting)?.tier).toBeNull();
    expect(retirementView(waiting)?.nativeStopped).toBe(false);
  });

  it("uses retained native-stop evidence rather than interpreting its reason as proof of exit", () => {
    const retained = accepted({ state: "retained", at, reason: "user_activity", native_stopped: false });
    const running = retirementView(retained)!;
    expect(running.nativeStopped).toBe(false);
    expect(running.tier).toBe("notice");
    expect(running.detail).toContain("Native stop is not confirmed");
    expect(running.detail).toContain("including after an inbox wake");
    retained.retirement!.state = { state: "retained", at, reason: "user_activity", native_stopped: true };
    const stopped = retirementView(retained)!;
    expect(stopped.nativeStopped).toBe(true);
    expect(stopped.label).toContain("terminal kept");
    expect(stopped.detail).not.toContain("Native stop is not confirmed");
    expect(stopped.tier).toBe("notice");
  });

  it("distinguishes uncertain native exit from uncertain terminal closure without trusting diagnostic text", () => {
    const unknown = accepted({ state: "unknown", at, phase: "native_stop", detail: "worker retired; repeat the close now" });
    const native = retirementView(unknown)!;
    expect(native.tier).toBe("recover");
    expect(native.nativeStopped).toBe(false);
    expect(native.detail).not.toContain("repeat the close");
    unknown.retirement!.state = { state: "unknown", at, phase: "terminal_close", detail: "native exit unverified" };
    const terminal = retirementView(unknown)!;
    expect(terminal.tier).toBe("recover");
    expect(terminal.nativeStopped).toBe(true);
    expect(terminal.detail).toContain("will not retry");
    expect(terminal.label).not.toContain("worker retired");
  });

  it.each(["already_absent", "absent_after_uncertain_close"] as const)("keeps %s distinct from a confirmed Cockpit terminal-close effect", terminal => {
    const retired = retirementView(accepted({ state: "retired", at, terminal }))!;
    expect(retired.nativeStopped).toBe(true);
    expect(retired.detail).toContain("no longer present");
    expect(retired.detail).not.toContain("terminal was closed");
  });

  it("does not apply acceptance retirement to active, cancelled or legacy closed runs", () => {
    const saved = accepted({ state: "native_stopped", at, evidence: "already_exited" });
    expect(retirementView({ ...saved, retirement: null })).toBeNull();
    expect(retirementView({ ...saved, close_reason: "cancelled" })).toBeNull();
    expect(retirementView({ ...saved, stage: "reported", close_reason: null })).toBeNull();
  });
});
