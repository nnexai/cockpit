import type { AttentionKind, OrchestrationSnapshot, Run, TaskView } from "../../protocol/generated/v1";

export type AttentionTier = "decide" | "recover" | "notice";
export const TIER_ORDER: Readonly<Record<AttentionTier, number>> = { decide: 0, recover: 1, notice: 2 };
export const TIER_LABEL: Readonly<Record<AttentionTier, "Decide" | "Recover" | "Notice">> = { decide: "Decide", recover: "Recover", notice: "Notice" };

export type LocalCondition =
  | { kind: "start_unknown" }
  | { kind: "terminal_error"; message: string }
  | { kind: "navigation_error" }
  | { kind: "change_unconfirmed"; message: string }
  | { kind: "notice"; message: string }
  | { kind: "orphaned_worker"; runId: string }
  | { kind: "agent_status"; runId: string; taskId: string | null }
  | { kind: "root_failed_report"; runId: string }
  | { kind: "assignment"; taskId: string; state: "pending" | "conflict" }
  | { kind: "relation_diagnostic"; taskId: string; cause: string }
  | { kind: "dependency_regression"; taskId: string }
  | { kind: "unidentified_items"; count: number };

export type AttentionSource =
  | { origin: "core"; kind: AttentionKind; messageSeq: number | null; since: string }
  | { origin: "local"; condition: LocalCondition };
export type AttentionSubject = { kind: "run"; runId: string } | { kind: "task"; taskId: string } | { kind: "workarea" };
export type AttentionItem = {
  id: string;
  tier: AttentionTier;
  subject: AttentionSubject;
  runId: string | null;
  taskId: string | null;
  sources: readonly [AttentionSource, ...AttentionSource[]];
  since: string | null;
};
export type SupervisorOwned = { kind: "awaits_prepare" | "awaits_execute" | "to_accept" | "needs_input"; runId: string; taskId: string | null; since: string };
export type AttentionModel = {
  items: readonly AttentionItem[];
  counts: Readonly<Record<AttentionTier, number>>;
  total: number;
  supervisorOwned: readonly SupervisorOwned[];
  tierForRun(runId: string): AttentionTier | null;
  tierForTask(task: TaskView): AttentionTier | null;
  itemsForRun(runId: string): readonly AttentionItem[];
  itemsForTask(task: TaskView): readonly AttentionItem[];
  ownedForRun(runId: string): SupervisorOwned | null;
};

export function coreTier(kind: AttentionKind, run: Run | undefined): AttentionTier | "supervisor" {
  switch (kind) {
    case "needs_input": return run && run.run_id === run.root_id ? "decide" : "supervisor";
    case "awaits_prepare": case "awaits_execute": case "to_accept": return "supervisor";
    case "runtime_blocked": case "dispatch_unknown": case "exited_without_report": case "retirement_unconfirmed": return "recover";
    case "idle_without_report": case "brief_unread": case "plan_changed": case "intent_conflict": return "notice";
  }
}

const LOCAL_TIER: Record<LocalCondition["kind"], AttentionTier> = {
  start_unknown: "recover", terminal_error: "recover", navigation_error: "recover", change_unconfirmed: "recover",
  orphaned_worker: "recover", agent_status: "recover", assignment: "notice", unidentified_items: "notice", notice: "notice", root_failed_report: "notice",
  relation_diagnostic: "notice", dependency_regression: "notice",
};
const PRECEDENCE: Record<AttentionKind | LocalCondition["kind"], number> = {
  needs_input: 0, awaits_prepare: 0, awaits_execute: 0, to_accept: 0,
  retirement_unconfirmed: 0, exited_without_report: 1, dispatch_unknown: 2, runtime_blocked: 3, orphaned_worker: 4, agent_status: 5,
  start_unknown: 6, terminal_error: 7, navigation_error: 8, change_unconfirmed: 9,
  intent_conflict: 0, assignment: 1, idle_without_report: 2, brief_unread: 3, plan_changed: 4,
  relation_diagnostic: 1, dependency_regression: 2,
  root_failed_report: 5, unidentified_items: 6, notice: 7,
};
const emptyCounts = (): Record<AttentionTier, number> => ({ decide: 0, recover: 0, notice: 0 });
const sourceKind = (source: AttentionSource) => source.origin === "core" ? source.kind : source.condition.kind;
const ageOrder = (since: string | null) => since === null ? Infinity : Date.parse(since);
const compareSince = (a: string | null, b: string | null) => a === b ? 0 : a === null ? 1 : b === null ? -1 : ageOrder(a) - ageOrder(b) || a.localeCompare(b);

export function deriveAttention({ snapshot, rootId, local }: { snapshot: OrchestrationSnapshot; rootId: string | null; local: readonly LocalCondition[] }): AttentionModel {
  const runs = new Map(snapshot.runs.map(run => [run.run_id, run]));
  const grouped = new Map<string, AttentionItem>();
  const supervisorOwned: SupervisorOwned[] = [];
  const coreRecover = new Set<string>();
  const add = (tier: AttentionTier, subject: AttentionSubject, runId: string | null, taskId: string | null, source: AttentionSource) => {
    const key = subject.kind === "run" ? `run:${encodeURIComponent(subject.runId)}` : subject.kind === "task" ? `task:${encodeURIComponent(subject.taskId)}` : `local:${sourceKind(source)}`;
    const id = `${tier}:${key}`;
    const previous = grouped.get(id);
    const since = source.origin === "core" ? source.since : null;
    if (previous) {
      grouped.set(id, { ...previous, runId: previous.runId ?? runId, taskId: previous.taskId ?? taskId,
        sources: [...previous.sources, source], since: compareSince(since, previous.since) < 0 ? since : previous.since });
    } else grouped.set(id, { id, tier, subject, runId, taskId, sources: [source], since });
  };
  for (const entry of snapshot.attention) {
    const run = entry.run_id === null ? undefined : runs.get(entry.run_id);
    if (!run || run.root_id !== rootId) continue;
    const tier = coreTier(entry.kind, run);
    const taskId = entry.task_id ?? run.task_id;
    if (tier === "supervisor") {
      if (entry.kind === "awaits_prepare" || entry.kind === "awaits_execute" || entry.kind === "to_accept" || entry.kind === "needs_input") {
        supervisorOwned.push({ kind: entry.kind, runId: run.run_id, taskId, since: entry.since });
      }
      continue;
    }
    if (tier === "recover") coreRecover.add(run.run_id);
    const subject: AttentionSubject = entry.kind === "intent_conflict" && taskId !== null ? { kind: "task", taskId } : { kind: "run", runId: run.run_id };
    add(tier, subject, run.run_id, taskId, { origin: "core", kind: entry.kind, messageSeq: entry.message_seq, since: entry.since });
  }
  for (const condition of local) {
    const runId = "runId" in condition ? condition.runId : null;
    const run = runId === null ? undefined : runs.get(runId);
    if (runId !== null && rootId !== null && run?.root_id !== rootId) continue;
    if (condition.kind === "agent_status" && coreRecover.has(condition.runId)) continue;
    const taskId = "taskId" in condition ? condition.taskId : run?.task_id ?? null;
    const subject: AttentionSubject = runId !== null ? { kind: "run", runId } : taskId !== null ? { kind: "task", taskId } : { kind: "workarea" };
    add(LOCAL_TIER[condition.kind], subject, runId, taskId, { origin: "local", condition });
  }
  const items = [...grouped.values()].map(item => ({ ...item, sources: [...item.sources].sort((a, b) => PRECEDENCE[sourceKind(a)] - PRECEDENCE[sourceKind(b)]) as [AttentionSource, ...AttentionSource[]] }))
    .sort((a, b) => TIER_ORDER[a.tier] - TIER_ORDER[b.tier] || compareSince(a.since, b.since) || a.id.localeCompare(b.id));
  supervisorOwned.sort((a, b) => compareSince(a.since, b.since) || a.runId.localeCompare(b.runId) || a.kind.localeCompare(b.kind));
  const counts = emptyCounts();
  const byRun = new Map<string, AttentionItem[]>();
  const byTask = new Map<string, AttentionItem[]>();
  for (const item of items) {
    counts[item.tier]++;
    if (item.runId !== null) { const list = byRun.get(item.runId) ?? []; list.push(item); byRun.set(item.runId, list); }
    if (item.taskId !== null) { const list = byTask.get(item.taskId) ?? []; list.push(item); byTask.set(item.taskId, list); }
  }
  const itemsForRun = (id: string): readonly AttentionItem[] => byRun.get(id) ?? [];
  const itemsForTask = (task: TaskView): readonly AttentionItem[] => {
    const direct = byTask.get(task.task.task_id) ?? [];
    const worker = task.current_run_id === null ? [] : byRun.get(task.current_run_id) ?? [];
    if (!direct.length) return worker;
    if (!worker.length) return direct;
    const matches = new Set([...direct, ...worker]);
    return items.filter(item => matches.has(item));
  };
  return { items, counts, total: items.length, supervisorOwned, itemsForRun, itemsForTask,
    tierForRun: id => itemsForRun(id)[0]?.tier ?? null,
    tierForTask: task => itemsForTask(task)[0]?.tier ?? null,
    ownedForRun: id => supervisorOwned.find(item => item.runId === id) ?? null };
}

/** Root selector counts queue subjects, not supervisor-owned preparation, execution or review. */
export function rootAttentionSummary(snapshot: OrchestrationSnapshot, rootId: string): Record<AttentionTier, number> {
  return { ...deriveAttention({ snapshot, rootId, local: [] }).counts };
}
