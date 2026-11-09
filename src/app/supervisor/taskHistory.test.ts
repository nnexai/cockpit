import { describe, expect, it } from "vitest";
import type { Message, Report, Run, TaskView, UnreflectedInput } from "../../protocol/generated/v1";
import type { SupervisorOverview } from "./useSupervisor";
import { formatEventTime, formatGap, taskHistory as projectHistory } from "./taskHistory";
import { orchestrationFixture as overviewFixture, runFixture, taskFixture } from "./testFixtures";

function orchestrationFixture(...args: Parameters<typeof overviewFixture>) {
  return { ...overviewFixture(...args), messages: [] as Message[] };
}
function taskHistory(snapshot: SupervisorOverview & { messages: Message[]; runs: Run[] }, rootId: string, view: TaskView, tasks: readonly TaskView[], fresh: boolean) {
  return projectHistory(snapshot, rootId, view, tasks, fresh, snapshot.messages, snapshot.runs);
}

const at = "2026-10-07T12:00:00Z", later = "2026-10-07T12:02:00Z";
function worker(overrides: Partial<Run> = {}) { return runFixture({ kind: "worker", run_id: "worker", root_id: "root", task_id: "task", stage: "working", ...overrides }); }
function report(kind: Report["kind"], messageId: string): Report { return { message_id: messageId, kind, outcome: kind === "result" ? "succeeded" : null, summary: `Summary ${messageId}`, plan: null, source_output: null, at }; }
function checkpoint(kind: "interim" | "waiting" | "resumed", messageId: string): Report {
  return { ...report("progress", messageId), plan: `cockpit-worker-checkpoint:v1:${JSON.stringify({ kind, artifacts: [] })}` };
}
function message(value: Report, overrides: Partial<Message> = {}): Message { return { message_id: value.message_id, from: { type: "run", run_id: "worker" }, to_run_id: "root", kind: "report", text: value.summary, report: value, in_reply_to: null, stale: false, escalated_from: null, stage: "acked", seq: 1, created_at: value.at, acked_at: later, ...overrides }; }
function input(overrides: Partial<UnreflectedInput> = {}): UnreflectedInput {
  return { run_id: "worker", result_message_id: "result", message_id: "input", seq: 3, kind: "instruction", from: { type: "operator" }, stage: "acked", after_result: true, created_at: at, acked_at: later, ...overrides };
}

  it("keeps run-derived events while message bodies are loading", () => {
    const task = taskFixture("task"), run = worker();
    const history = projectHistory(overviewFixture([task], [run]), "root", task, [task], true, null, [run]);
    expect(history.attempts[0].events.map(event => event.kind)).toEqual(["started"]);
  });
describe("task history evidence", () => {
  it("distinguishes recorded output and integration without guessing missing or mismatched output", () => {
    const task = taskFixture("task", { lane: "accepted" });
    const result = report("result", "result");
    const code = { kind: "code" as const, repository_key: "repo", checkout_key: "checkout", base_commit: "base", head_commit: "head", commits: ["head"], task_revision: "revision-task", result_message_id: "result", focused_checks: [] };
    const nonCode = { kind: "non_code" as const, result_message_id: "result", task_revision: "revision-task", artifacts: ["artifact"], focused_checks: [] };
    const project = (run: Run) => taskHistory(orchestrationFixture([task], [run]), "root", task, [task], true);
    const accepted = worker({ stage: "closed", close_reason: "accepted", result });
    expect(project(accepted).sub).toBeNull();
    expect(project({ ...accepted, source_output: { ...code, result_message_id: "other" } }).sub).toBeNull();
    expect(project({ ...accepted, source_output: code }).sub).toEqual({ kind: "integration_not_recorded" });
    expect(project({ ...accepted, source_output: nonCode }).sub).toBeNull();
    expect(project({ ...accepted, source_output: nonCode, integration: { state: "not_applicable", reason: "non-code", result_message_id: "other" } }).sub).toBeNull();
    expect(project({ ...accepted, source_output: nonCode, integration: { state: "not_applicable", reason: "non-code", result_message_id: "result" } }).sub).toEqual({ kind: "non_code" });
    const integration = { state: "integrated" as const, operation_id: "op", accepted_result_message_id: "result", accepted_task_revision: "revision-task", source: code, target_repository_key: "repo", target_ref: "refs/heads/main", target_commit: "target", method: "patch_equivalent" as const, mappings: [], checks: [], integrator_run_id: "root", verified_at: later };
    const integrated = project({ ...accepted, source_output: code, integration });
    expect(integrated.sub).toEqual({ kind: "integrated", target: "main" });
    expect(integrated.attempts[0].events).toContainEqual(expect.objectContaining({ kind: "integrated", target: "main" }));
  });

  it("does not treat a checked task alone as recorded acceptance", () => {
    const task = taskFixture("task", { lane: "accepted" });
    task.task.checked = true;
    expect(taskHistory(orchestrationFixture([task], []), "root", task, [task], true).outcome.accepted).toBe(false);
  });

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

  it("preserves interim and waiting reports as separate non-final events across attempts", () => {
    const task = taskFixture("task");
    const first = worker({ run_id: "old", stage: "closed", close_reason: "superseded" });
    const result = report("result", "result");
    const latest = worker({ attempt: 2, result });
    const interim = { ...checkpoint("interim", "interim"), summary: "Partial findings\nwith detail" };
    const waiting = { ...checkpoint("waiting", "waiting"), at: later, summary: "Waiting on a dependency" };
    const newer = { ...checkpoint("interim", "newer"), at: later, summary: "New attempt findings" };
    const resumed = { ...checkpoint("resumed", "resumed"), at: later, summary: "Continuing work" };
    const snapshot = orchestrationFixture([task], [first, latest]);
    snapshot.messages = [
      message(interim, { from: { type: "run", run_id: "old" }, stale: true }),
      message(waiting, { from: { type: "run", run_id: "old" }, stale: true }),
      message(waiting, { from: { type: "run", run_id: "old" }, stale: true }),
      message(newer),
      message(resumed),
      message(result),
      message(checkpoint("waiting", "foreign"), { from: { type: "run", run_id: "other" } }),
    ];
    const history = taskHistory(snapshot, "root", task, [task], true);
    expect(history.attempts[0].events.slice(1)).toEqual([
      { kind: "interim", at, summary: "Partial findings\nwith detail" },
      { kind: "waiting", at: later, summary: "Waiting on a dependency" },
    ]);
    expect(history.attempts[1].events.filter(event => event.kind === "interim" || event.kind === "waiting" || event.kind === "resumed")).toEqual([
      { kind: "interim", at: later, summary: "New attempt findings" },
      { kind: "resumed", at: later, summary: "Continuing work" },
    ]);
    expect(history.attempts[0].events.some(event => event.kind === "result" || event.kind === "progress")).toBe(false);
    expect(history.attempts[1].events.filter(event => event.kind === "result")).toHaveLength(1);
  });

  it("keeps malformed or unrelated checkpoint plans as ordinary progress", () => {
    const task = taskFixture("task"), snapshot = orchestrationFixture([task], [worker()]);
    const plans = [
      "cockpit-worker-checkpoint:v1:{",
      'cockpit-worker-checkpoint:v1:{"kind":"waiting","artifacts":[],"extra":true}',
      'cockpit-worker-checkpoint:v1:{"kind":"waiting","artifacts":[1]}',
      'cockpit-worker-checkpoint:v1:{"kind":"unknown","artifacts":[]}',
      'cockpit-worker-checkpoint:v2:{"kind":"waiting","artifacts":[]}',
    ];
    snapshot.messages = plans.map((plan, index) => message({ ...report("progress", String(index)), plan }));
    const events = taskHistory(snapshot, "root", task, [task], true).attempts[0].events;
    expect(events.filter(event => event.kind === "progress")).toMatchObject([{ items: plans.map((_, index) => ({ at, text: `Summary ${index}` })) }]);
    expect(events.some(event => event.kind === "interim" || event.kind === "waiting" || event.kind === "resumed")).toBe(false);
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

  it("derives scoped input history, preserves result ordering under clock skew, and clears on a newer result", () => {
    const task = taskFixture("task"), result = { ...report("result", "result"), at: later };
    const run = worker({ stage: "reported", result });
    const snapshot = orchestrationFixture([task], [run, worker({ run_id: "foreign", root_id: "other" })]);
    snapshot.unreflected_inputs = [input({ seq: 5 }), input({ message_id: "missing", seq: 4 }), input({ run_id: "foreign" })];
    snapshot.messages = [message(result, { message_id: "input", to_run_id: "other", text: "wrong recipient" }), message(result, { message_id: "input", to_run_id: "worker", text: "<img onerror=alert(1)>", images: [{ image_id: "image", mime_type: "image/png", width: 1, height: 1, bytes: 1 }] })];
    const history = taskHistory(snapshot, "root", task, [task], true);
    expect(history.unreflectedNotice).toBe("2 inputs sent after the Result");
    expect(history.attempts[0].events.map(event => event.kind)).toEqual(["started", "result", "unreflected_input", "unreflected_input"]);
    expect(history.attempts[0].events[2]).toMatchObject({ messageId: "missing", text: null, at, sortAt: later });
    expect(history.attempts[0].events[3]).toMatchObject({ messageId: "input", text: "<img onerror=alert(1)>", imageCount: 1, by: "you", closed: false });
    expect(taskHistory(snapshot, "root", task, [task, task], true)).toMatchObject({ identity: "ambiguous", unreflectedNotice: null, attempts: [] });
    snapshot.runs = [{ ...run, result: { ...result, message_id: "new-result" } }];
    expect(taskHistory(snapshot, "root", task, [task], true)).toMatchObject({ unreflectedNotice: null, attempts: [{ events: expect.not.arrayContaining([expect.objectContaining({ kind: "unreflected_input" })]) }] });
  });

  it("keeps closed input history but not its notice, and distinguishes stored-before labels", () => {
    const task = taskFixture("task"), result = report("result", "result");
    const snapshot = orchestrationFixture([task], [worker({ stage: "reported", result })]);
    snapshot.unreflected_inputs = [input({ after_result: false, stage: "stored" }), input({ message_id: "second" })];
    expect(taskHistory(snapshot, "root", task, [task], true).unreflectedNotice).toBe("1 input sent after the Result · 1 sent before it, still waiting to be picked up");
    snapshot.unreflected_inputs.pop();
    expect(taskHistory(snapshot, "root", task, [task], true).unreflectedNotice).toBe("1 input sent before the Result, still waiting to be picked up");
    snapshot.runs = [worker({ stage: "closed", close_reason: "accepted", result })];
    const history = taskHistory(snapshot, "root", task, [task], true);
    expect(history.unreflectedNotice).toBeNull();
    expect(history.attempts[0].events.find(event => event.kind === "unreflected_input")).toMatchObject({ stage: "stored", afterResult: false, closed: true });
  });

  it("omits invalid times and refuses negative answer durations", () => {
    expect(formatEventTime("", new Date(at))).toBeNull();
    expect(formatEventTime("not-a-time", new Date(at))).toBeNull();
    expect(formatGap(later, at)).toBeNull();
  });
});
