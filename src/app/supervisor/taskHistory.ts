import type { CleanupReasonClass, CleanupState, Message, OrchestrationSnapshot, OwnedResourceKind, Report, ReportOutcome, ResumePhase, Run, TaskView } from "../../protocol/generated/v1";
import { dependencySummary, uniqueTask } from "./dependencies";

export type HistoryFact = { label: string; value: string };
export type HistoryEvent =
  | { kind: "started"; at: string; replacement: boolean; facts: HistoryFact[] }
  | { kind: "progress"; at: string; items: { at: string; text: string }[]; earlier: number }
  | { kind: "question"; at: string; answer: { at: string; by: "you" | "liaison" | "agent" | "cockpit" } | null; open: boolean }
  | { kind: "result"; at: string; outcome: ReportOutcome | null; summary: string; facts: HistoryFact[] }
  | { kind: "merged"; at: string | null; target: string; facts: HistoryFact[] }
  | { kind: "recovery"; at: string; phase: ResumePhase; cause: string | null };
export type HistoryAttempt = { runId: string; attempt: number; label: string; end: "current" | "accepted" | "replaced" | "failed" | "cancelled"; reason: string | null; events: HistoryEvent[] };
export type LeftBehindRow = {
  key: string; kind: OwnedResourceKind; attempt: number | null;
  state: "removed" | "kept" | "closed" | "closing" | "waiting" | "cleaning" | "stalled" | "unknown";
  at: string | null; reasonClass: CleanupReasonClass | null; reason: string | null;
};
export type TaskHistory = {
  taskId: string; identity: "unique" | "ambiguous";
  outcome: { stage: string; scheduled: boolean; accepted: boolean; attempt: number | null; kept: number; stalled: boolean };
  sub: { kind: "merged"; target: string } | { kind: "no_merge" } | { kind: "merge_not_recorded" } | { kind: "after"; text: string } | { kind: "reconnecting"; phase: ResumePhase } | { kind: "not_started" } | null;
  scheduledAt: string | null; attempts: HistoryAttempt[]; leftBehind: LeftBehindRow[]; liveUnavailable: boolean;
};

const humanize = (code: string) => code.replaceAll("_", " ").replace(/^./, c => c.toUpperCase());
const timestamp = (value: string | null) => value ? Date.parse(value) : NaN;
const compareTime = (a: string | null, b: string | null) => (Number.isFinite(timestamp(a)) ? timestamp(a) : Infinity) - (Number.isFinite(timestamp(b)) ? timestamp(b) : Infinity);
const branch = (ref: string) => ref.replace(/^refs\/heads\//, "");
const cleanupState = (state: CleanupState): LeftBehindRow["state"] => state === "pending" || state === "running" || state === "reconciling" ? "cleaning" : state === "waiting_integration" ? "waiting" : state;

function resultFacts(run: Run, report: Report): HistoryFact[] {
  const source = report.source_output ?? (run.source_output?.result_message_id === report.message_id ? run.source_output : null);
  if (!source) return [];
  const facts: HistoryFact[] = [];
  const checks = new Map<string, number>();
  for (const check of source.focused_checks) checks.set(check.outcome, (checks.get(check.outcome) ?? 0) + 1);
  if (checks.size) facts.push({ label: "Checks", value: [...checks].map(([state, count]) => `${count} ${state.replaceAll("_", " ")}`).join(" · ") });
  facts.push(source.kind === "code" ? { label: "Commits", value: `${source.commits.length} · head ${source.head_commit.slice(0, 7)}` } : { label: "Artifacts", value: String(source.artifacts.length) });
  return facts;
}

/** Derive one selected task, not a second lifecycle store. Generic updated_at is never an event time. */
export function taskHistory(snapshot: OrchestrationSnapshot, rootId: string, view: TaskView, tasks: readonly TaskView[], runtimeFresh: boolean): TaskHistory {
  const stage = view.task.checked || view.lane === "accepted" ? "done" : view.attempt?.state === "starting" ? "starting" : view.attempt?.state === "stalled" ? "stalled" : view.lane;
  const history: TaskHistory = { taskId: view.task.task_id, identity: "unique", outcome: { stage, scheduled: view.scheduled, accepted: false, attempt: null, kept: 0, stalled: view.attempt?.state === "stalled" }, sub: null, scheduledAt: null, attempts: [], leftBehind: [], liveUnavailable: !runtimeFresh };
  if (!uniqueTask(tasks, view.task.task_id)) return { ...history, identity: "ambiguous" };
  const runs = snapshot.runs.filter(run => run.root_id === rootId && run.task_id === view.task.task_id && run.kind !== "supervisor").sort((a, b) => a.attempt - b.attempt);
  const runIds = new Set(runs.map(run => run.run_id));
  const reports = new Map<string, Map<string, Report>>();
  const answers = new Map<string, Map<string, Message>>();
  for (const message of snapshot.messages) {
    if (message.stale) continue;
    if (message.kind === "report" && message.from.type === "run" && runIds.has(message.from.run_id) && message.report) {
      let bucket = reports.get(message.from.run_id);
      if (!bucket) reports.set(message.from.run_id, bucket = new Map());
      bucket.set(message.report.message_id, message.report);
    }
    if (message.kind === "answer" && runIds.has(message.to_run_id) && message.in_reply_to) {
      let bucket = answers.get(message.to_run_id);
      if (!bucket) answers.set(message.to_run_id, bucket = new Map());
      const previous = bucket.get(message.in_reply_to);
      if (!previous || compareTime(message.created_at, previous.created_at) < 0) bucket.set(message.in_reply_to, message);
    }
  }
  const cleanup = new Map(snapshot.cleanup.map(summary => [summary.run_id, summary.items]));
  for (const [index, run] of runs.entries()) {
    const prior = runs[index - 1];
    const replaced = runs.some(old => old.retirement?.replacement?.replacement_run_id === run.run_id) || prior?.close_reason === "superseded";
    const facts: HistoryFact[] = [];
    if (run.setup?.branch) facts.push({ label: "Branch", value: run.setup.branch });
    if (run.setup?.base_commit) facts.push({ label: "Base", value: run.setup.base_commit.slice(0, 7) });
    if (replaced && prior) facts.push({ label: "Replaces", value: `attempt ${prior.attempt}` });
    const events: HistoryEvent[] = [{ kind: "started", at: run.created_at, replacement: replaced, facts }];
    const bucket = reports.get(run.run_id) ?? new Map<string, Report>();
    // Results survive independently of the message projection. The same ID is one event.
    if (run.result && !bucket.has(run.result.message_id)) bucket.set(run.result.message_id, run.result);
    const latestQuestionOrResult = [...bucket.values()].filter(report => report.kind === "needs_input" || report.kind === "result").sort((a, b) => compareTime(a.at, b.at)).at(-1);
    const progress: { at: string; text: string }[] = [];
    for (const report of bucket.values()) {
      if (report.kind === "progress") progress.push({ at: report.at, text: report.summary });
      if (report.kind === "result") events.push({ kind: "result", at: report.at, outcome: report.outcome, summary: report.summary, facts: resultFacts(run, report) });
      if (report.kind === "needs_input") {
        const answer = answers.get(run.run_id)?.get(report.message_id);
        const by = answer?.from.type === "operator" ? "you" : answer?.from.type === "dispatcher" ? "cockpit" : answer?.from.type === "run" && answer.from.run_id === rootId ? "liaison" : "agent";
        events.push({ kind: "question", at: report.at, answer: answer ? { at: answer.created_at, by } : null, open: run.stage !== "closed" && !answer && latestQuestionOrResult?.message_id === report.message_id });
      }
    }
    if (progress.length) {
      progress.sort((a, b) => compareTime(a.at, b.at));
      events.push({ kind: "progress", at: progress[0].at, items: progress.slice(-10), earlier: Math.max(0, progress.length - 10) });
    }
    if (run.integration.state === "integrated") events.push({ kind: "merged", at: run.integration.verified_at || null, target: branch(run.integration.target_ref), facts: [
      { label: "Method", value: humanize(run.integration.method) },
      { label: "Target", value: `${branch(run.integration.target_ref)} · ${run.integration.target_commit.slice(0, 7)}` },
    ] });
    if (run.stage !== "closed" && run.resume) events.push({ kind: "recovery", at: run.resume.updated_at, phase: run.resume.phase, cause: run.resume.cause ? humanize(run.resume.cause.code) : null });
    const rest = events.splice(1).sort((a, b) => compareTime(a.at, b.at));
    events.push(...rest);
    const end: HistoryAttempt["end"] = run.stage !== "closed" ? "current" : run.close_reason === "superseded" ? "replaced" : run.close_reason ?? "failed";
    const errorCode = run.resume?.cause?.code ?? run.dispatch?.error?.code;
    const reason = errorCode ? humanize(errorCode) : end === "replaced" ? "Replaced by a later attempt" : end === "cancelled" ? "Cancelled" : null;
    history.attempts.push({ runId: run.run_id, attempt: run.attempt, label: run.label, end, reason, events });
    const retirement = run.retirement?.state;
    const terminalAt = retirement?.state === "retired" || retirement?.state === "retained" ? retirement.at : null;
    const items = cleanup.get(run.run_id) ?? [];
    for (const item of items) history.leftBehind.push({ key: item.resource_id, kind: item.kind, attempt: runs.length > 1 ? run.attempt : null, state: cleanupState(item.cleanup.state), at: item.kind === "worker_terminal" ? terminalAt : null, reasonClass: item.cleanup.reason?.class ?? null, reason: item.cleanup.reason ? humanize(item.cleanup.reason.code) : null });
    if (retirement && !items.some(item => item.kind === "worker_terminal")) history.leftBehind.push({
      key: `${run.run_id}:terminal`, kind: "worker_terminal", attempt: runs.length > 1 ? run.attempt : null,
      state: retirement.state === "retired" ? "closed" : retirement.state === "retained" ? "kept" : retirement.state === "unknown" ? "unknown" : retirement.state === "close_intent" ? "closing" : "waiting",
      at: terminalAt, reasonClass: retirement.state === "retained" ? "authority" : null, reason: retirement.state === "retained" ? humanize(retirement.reason) : null,
    });
  }
  history.leftBehind.sort((a, b) => (b.attempt ?? 0) - (a.attempt ?? 0) || Number(b.kind === "worker_terminal") - Number(a.kind === "worker_terminal"));
  const newest = runs.at(-1);
  history.outcome.attempt = newest?.attempt ?? null;
  history.outcome.accepted = stage === "done" && newest?.close_reason === "accepted";
  history.outcome.kept = history.leftBehind.filter(row => row.state === "kept").length;
  history.scheduledAt = snapshot.task_ownership.find(item => item.root_id === rootId && item.task_id === view.task.task_id)?.schedule?.scheduled_at ?? null;
  const dependency = dependencySummary(view, tasks);
  history.sub = stage === "done" && newest?.close_reason === "accepted" ? newest.integration.state === "integrated" ? { kind: "merged", target: branch(newest.integration.target_ref) } : { kind: newest.integration.state === "not_applicable" ? "no_merge" : "merge_not_recorded" }
    : dependency ? { kind: "after", text: dependency } : newest?.stage !== "closed" && newest?.resume && ["reconciling", "launching"].includes(newest.resume.phase) ? { kind: "reconnecting", phase: newest.resume.phase } : !runs.length ? { kind: "not_started" } : null;
  return history;
}

/** Approximate completion ordering only, never displayed as an acceptance timestamp. */
export function orderDoneTasks(tasks: readonly TaskView[], runs: readonly Run[]): TaskView[] {
  const completed = new Map<string, string>();
  for (const run of runs) if (run.task_id && run.stage === "closed" && run.close_reason === "accepted" && (completed.get(run.task_id) ?? "") < run.updated_at) completed.set(run.task_id, run.updated_at);
  return tasks.slice().sort((a, b) => (completed.get(b.task.task_id) ?? "").localeCompare(completed.get(a.task.task_id) ?? "") || b.task.line - a.task.line);
}

export function formatEventTime(iso: string | null, now: Date): { label: string; title: string } | null {
  if (!Number.isFinite(timestamp(iso))) return null;
  const date = new Date(iso!);
  const today = date.toDateString() === now.toDateString();
  return { label: date.toLocaleString(undefined, { ...(today ? {} : { month: "short", day: "numeric" }), hour: "2-digit", minute: "2-digit", hourCycle: "h23" }), title: date.toLocaleString() };
}

export function formatGap(fromIso: string, toIso: string): string | null {
  const gap = timestamp(toIso) - timestamp(fromIso);
  if (!Number.isFinite(gap) || gap < 0) return null;
  const minutes = Math.floor(gap / 60000);
  return minutes < 1 ? "within a minute" : minutes < 60 ? `${minutes} min` : minutes < 1440 ? `${Math.floor(minutes / 60)} h${minutes % 60 ? ` ${minutes % 60} min` : ""}` : `${Math.floor(minutes / 1440)} d`;
}
