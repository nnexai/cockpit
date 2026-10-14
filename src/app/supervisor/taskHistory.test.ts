import { describe, expect, it } from "vitest";
import type { Message, Report, Run } from "../../protocol/generated/v1";
import { formatEventTime, formatGap, taskHistory } from "./taskHistory";
import { orchestrationFixture, runFixture, taskFixture } from "./testFixtures";

const at = "2026-10-07T12:00:00Z", later = "2026-10-07T12:02:00Z";
function worker(overrides: Partial<Run> = {}) { return runFixture({ kind: "worker", run_id: "worker", root_id: "root", task_id: "task", stage: "working", ...overrides }); }
function report(kind: Report["kind"], messageId: string): Report { return { message_id: messageId, kind, outcome: kind === "result" ? "succeeded" : null, summary: `Summary ${messageId}`, plan: null, source_output: null, at }; }
function message(value: Report, overrides: Partial<Message> = {}): Message { return { message_id: value.message_id, from: { type: "run", run_id: "worker" }, to_run_id: "root", kind: "report", text: value.summary, report: value, in_reply_to: null, stale: false, escalated_from: null, stage: "acked", seq: 1, created_at: value.at, acked_at: later, ...overrides }; }

describe("task history evidence", () => {
  it("isolates root/task attempts and deduplicates reports while preserving superseded attempts", () => {
    const task = taskFixture("task"), result = report("result", "result");
    const first = worker({ run_id: "old", stage: "closed", close_reason: "superseded" });
    const latest = worker({ attempt: 2, stage: "closed", close_reason: "accepted", result, updated_at: "2030-01-01T00:00:00Z" });
    const snapshot = orchestrationFixture([task], [first, latest, worker({ root_id: "foreign" }), worker({ task_id: "another" })]);
    snapshot.messages = [message(result), message(result), message(report("progress", "progress-1")), message(report("progress", "progress-2"))];
    const history = taskHistory(snapshot, "root", task, [task], true);
    expect(history.attempts.map(attempt => [attempt.runId, attempt.end])).toEqual([["old", "replaced"], ["worker", "accepted"]]);
    expect(history.attempts[1].events.filter(event => event.kind === "result")).toHaveLength(1);
    expect(history.attempts[1].events.find(event => event.kind === "started")).toMatchObject({ replacement: true, at });
    expect(history.attempts[1].events.find(event => event.kind === "progress")).toMatchObject({ items: [{ at, text: "Summary progress-1" }, { at, text: "Summary progress-2" }] });
    expect(history.attempts.flatMap(attempt => attempt.events.map(event => event.at))).not.toContain(latest.updated_at);
  });

  it("correlates a closed worker's question with its actual answering actor, not another worker's answer", () => {
    const task = taskFixture("task"), question = report("needs_input", "question");
    const snapshot = orchestrationFixture([task], [worker({ stage: "closed", close_reason: "accepted" })]);
    snapshot.messages = [message(question), message(question, { kind: "answer", report: null, from: { type: "operator" }, to_run_id: "other", in_reply_to: "question" }), message(question, { message_id: "answer", kind: "answer", report: null, from: { type: "run", run_id: "root" }, to_run_id: "worker", in_reply_to: "question", created_at: later })];
    const history = taskHistory(snapshot, "root", task, [task], true);
    expect(history.attempts[0].events.find(event => event.kind === "question")).toEqual({ kind: "question", at, answer: { at: later, by: "liaison" }, open: false });
    expect(formatGap(at, later)).toBe("2 min");
  });

  it("does not mistake a stale answer for an answered current question", () => {
    const task = taskFixture("task"), question = report("needs_input", "question");
    const snapshot = orchestrationFixture([task], [worker()]);
    snapshot.messages = [message(question), message(question, { kind: "answer", report: null, to_run_id: "worker", in_reply_to: "question", stale: true })];
    expect(taskHistory(snapshot, "root", task, [task], true).attempts[0].events.find(event => event.kind === "question")).toMatchObject({ answer: null, open: true });
  });

  it("joins terminal retirement and cleanup once and preserves durable removal when live state is unavailable", () => {
    const task = taskFixture("task"), run = worker({ retirement: { retirement_id: "retirement", trigger: "accept", result_message_id: "result", task_revision: "revision", replacement: null, identity: null, state: { state: "retired", terminal: "closed_by_cockpit", at: later }, created_at: at, updated_at: later } });
    const snapshot = orchestrationFixture([task], [run]);
    snapshot.cleanup = [{ run_id: "worker", removed_count: 1, kept_count: 1, waiting_count: 0, unknown_count: 0, items: [
      { resource_id: "terminal", kind: "worker_terminal", ownership: "owned", cleanup: { operation_id: null, state: "removed", reason: null, effects: [], updated_at: "2030-01-01T00:00:00Z" } },
      { resource_id: "output", kind: "attempt_directory", ownership: "owned", cleanup: { operation_id: null, state: "kept", reason: { code: "outputs_kept", class: "user_work", message: "redacted" }, effects: [], updated_at: later } },
    ] }];
    const history = taskHistory(snapshot, "root", task, [task], false);
    expect(history.leftBehind).toEqual([
      { key: "terminal", kind: "worker_terminal", attempt: null, state: "removed", at: later, reasonClass: null, reason: null },
      { key: "output", kind: "attempt_directory", attempt: null, state: "kept", at: null, reasonClass: "user_work", reason: "Outputs kept" },
    ]);
    expect(history.outcome.kept).toBe(1);
    expect(history.liveUnavailable).toBe(true);
  });

  it("refuses to associate history with duplicate canonical IDs", () => {
    const task = taskFixture("task"), snapshot = orchestrationFixture([task, task], [worker()]);
    expect(taskHistory(snapshot, "root", task, [task, task], true)).toMatchObject({ identity: "ambiguous", attempts: [], leftBehind: [] });
  });

  it("keeps newer open attempts distinct from an older accepted outcome", () => {
    const task = taskFixture("task"), snapshot = orchestrationFixture([task], [worker({ stage: "closed", close_reason: "accepted", attempt: 1 }), worker({ run_id: "new", attempt: 2 })]);
    expect(taskHistory(snapshot, "root", task, [task], true).outcome).toMatchObject({ accepted: false, attempt: 2 });
  });

  it("does not present superseded questions or an unchecked task as currently resolved or waiting", () => {
    const task = taskFixture("task"), question = report("needs_input", "question"), result = { ...report("result", "result"), at: later };
    const snapshot = orchestrationFixture([task], [worker({ stage: "reported", result })]);
    snapshot.messages = [message(question), message(result)];
    expect(taskHistory(snapshot, "root", task, [task], true).attempts[0].events.find(event => event.kind === "question")).toMatchObject({ open: false });
    snapshot.runs = [worker({ stage: "closed", close_reason: "accepted", result })];
    expect(taskHistory(snapshot, "root", task, [task], true).outcome).toMatchObject({ stage: "ready", accepted: false });
  });

  it("omits invalid times and refuses negative answer durations", () => {
    expect(formatEventTime("", new Date(at))).toBeNull();
    expect(formatEventTime("not-a-time", new Date(at))).toBeNull();
    expect(formatGap(later, at)).toBeNull();
  });
});
