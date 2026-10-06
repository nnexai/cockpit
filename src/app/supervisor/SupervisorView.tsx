import { useEffect, useId, useRef, useState } from "react";
import type { CockpitClient } from "../../client/CockpitClient";
import type { ActorRef, OrchestrationSnapshot, Run, SessionSnapshotResponse, Subagent, TaskView } from "../../protocol/generated/v1";
import { useRovingList } from "../sidebar/useRovingList";
import { StateGlyph } from "../sidebar/StateGlyph";
import { UiIcon } from "../UiIcon";
import { SupervisorGraph } from "./SupervisorGraph";
import { taskLanes, taskNeighbor } from "./boardNavigation";
import { AgentRecovery, agentState, ObservedEvidence, ReportedEvidence, RunDiagnostics, SupervisorActions, taskStatus, TextAction, type Mutation } from "./SupervisorActions";
import { SupervisorDialogs, type StartDraft, type SupervisorDialogState } from "./SupervisorDialogs";
import { messageDraft, useSupervisorDrafts, type ScopeDrafts } from "./useSupervisorDrafts";
import { useSupervisor } from "./useSupervisor";
import "./supervisor.css";

type ForestRow = { key: string; depth: number; run: Run; subagent: Subagent | null };
export function supervisorForest(snapshot: OrchestrationSnapshot, includeSubagents: boolean): ForestRow[] {
  const rows: ForestRow[] = [];
  const visited = new Set<string>();
  const appendRun = (run: Run, depth: number) => {
    if (visited.has(run.run_id)) return;
    visited.add(run.run_id); rows.push({ key: run.run_id, depth, run, subagent: null });
    if (includeSubagents) {
      const seen = new Set<string>();
      const children = snapshot.subagents.filter(agent => agent.run_id === run.run_id);
      const appendSubagent = (agent: Subagent, level: number) => {
        if (seen.has(agent.subagent_id)) return;
        seen.add(agent.subagent_id); rows.push({ key: `${run.run_id}:${agent.subagent_id}`, depth: level, run, subagent: agent });
        children.filter(child => child.parent_subagent_id === agent.subagent_id).forEach(child => appendSubagent(child, level + 1));
      };
      children.filter(agent => !agent.parent_subagent_id || !children.some(parent => parent.subagent_id === agent.parent_subagent_id)).forEach(agent => appendSubagent(agent, depth + 1));
      children.forEach(agent => appendSubagent(agent, depth + 1));
    }
    snapshot.runs.filter(child => child.parent_run_id === run.run_id).forEach(child => appendRun(child, depth + 1));
  };
  snapshot.runs.filter(run => !run.parent_run_id || !snapshot.runs.some(parent => parent.run_id === run.parent_run_id)).forEach(run => appendRun(run, 0));
  snapshot.runs.forEach(run => appendRun(run, 0));
  return rows;
}
/** A bounded metadata title; instruction body is always sent unchanged. */
export function taskTitle(text: string): string {
  const first = text.split(/\r?\n/).find(line => line.trim())?.trim() ?? "";
  const encoder = new TextEncoder();
  if (encoder.encode(first).length <= 256) return first;
  let title = "";
  let bytes = 0;
  for (const character of first) { const count = encoder.encode(character).length; if (bytes + count > 253) break; title += character; bytes += count; }
  return `${title}…`;
}
function actorName(actor: ActorRef, snapshot: OrchestrationSnapshot): string {
  if (actor.type === "operator") return "You";
  if (actor.type === "dispatcher") return "Dispatcher";
  return snapshot.runs.find(run => run.run_id === actor.run_id)?.label ?? "Agent";
}
function TaskComposer({ root, scope, snapshot, canAssign, busy, changed, mutateResult, refresh }: {
  root: Run; scope: ScopeDrafts; snapshot: OrchestrationSnapshot; canAssign: boolean; busy: boolean; changed(): void; mutateResult: Mutation; refresh(): void;
}) {
  const id = useId();
  const errorId = useId();
  const inFlight = useRef(false);
  const [pending, setPending] = useState(false);
  const draft = scope.task;
  const submit = async () => {
    if (busy || inFlight.current || !canAssign || !draft.text.trim()) return;
    if (!draft.operation && new TextEncoder().encode(draft.text).length > 16 * 1024) {
      draft.error = "This task exceeds the 16 KiB instruction limit. The complete draft is kept; shorten it before assigning.";
      changed(); return;
    }
    draft.operation ??= { id: crypto.randomUUID(), text: draft.text };
    const submitted = draft.operation;
    draft.error = null; draft.notice = null; inFlight.current = true; setPending(true); changed();
    try {
      const result = await mutateResult({ action: "task_assign", root_id: root.run_id, task_id: submitted.id, title: taskTitle(submitted.text), body: submitted.text });
      if (result?.result === "task_assigned" && result.task.task_id === submitted.id && result.to_run_id === root.run_id) {
        if (draft.text === submitted.text) draft.text = "";
        draft.operation = null; draft.notice = `Assigned to ${root.label}. Waiting for the agent.`;
      } else draft.error = "Could not confirm assignment. Your draft and task identity are kept. Check status, then explicitly retry the same task; do not submit a changed duplicate.";
    } catch { draft.error = "Could not assign task. Your draft is kept."; }
    finally { inFlight.current = false; setPending(false); changed(); }
  };
  const intent = snapshot.assignment_intents.find(item => item.root_id === root.run_id && item.task_id === draft.operation?.id);
  return <form className="supervisor-task-composer" aria-busy={pending} onSubmit={event => { event.preventDefault(); void submit(); }}><label htmlFor={id}>Task</label><textarea id={id} data-task-composer rows={3} placeholder="What should the agent do?" value={draft.text} readOnly={pending || !!draft.operation} aria-describedby={!canAssign || draft.error ? errorId : undefined} onChange={event => { draft.text = event.target.value; draft.notice = null; changed(); }} /><div className="supervisor-composer-footer"><span>{root.stage === "closed" ? "Tracking is closed. Start an agent to assign new work." : !canAssign ? "Wait for a fresh, connected OMP supervisor before assigning work. Your draft stays here." : "Or give work directly in the supervisor's terminal."}</span><button type="submit" disabled={busy || pending || !canAssign || !draft.text.trim() || intent?.state === "conflict"}>{pending ? "Assigning task…" : draft.operation ? "Retry same task" : "Give task"}</button></div>{!canAssign ? <p id={errorId} className="supervisor-muted">Assignment is unavailable until the agent is connected.</p> : null}{draft.error ? <div className="supervisor-error" id={canAssign ? errorId : undefined} role="alert"><p>{draft.error}</p><button type="button" disabled={busy} onClick={refresh}>Check assignment status</button></div> : null}{draft.operation && !pending ? <p className="supervisor-muted">The submitted draft is kept unchanged until its previous assignment is resolved.</p> : null}{draft.notice ? <p role="status">{draft.notice}</p> : null}</form>;
}
export function SupervisorView({ client, sessionId, session, runtimeLive, active, startToken, navigationError, onClose, onTerminal, onUnmanagedTerminal, onModalChange }: {
  client: CockpitClient; sessionId: string; session: SessionSnapshotResponse | null; runtimeLive: boolean; active: boolean; startToken: number;
  navigationError: string | null; onClose(): void; onTerminal(run: Run, snapshot: OrchestrationSnapshot): Promise<void>; onUnmanagedTerminal(paneId: string): Promise<void>; onModalChange(open: boolean): void;
}) {
  const [rootId, setRootId] = useState<string | null>(null);
  const { snapshot, error, connected, busy, mutateResult, refresh } = useSupervisor(client, sessionId, rootId, active);
  const drafts = useSupervisorDrafts(sessionId);
  const scope = drafts.scope(rootId ?? snapshot?.board?.root_id ?? null);
  const changed = drafts.changed;
  const [dialog, setDialog] = useState<SupervisorDialogState | null>(null);
  const [terminalError, setTerminalError] = useState<string | null>(null);
  const [notice, setNotice] = useState<string | null>(null);
  const [startPending, setStartPending] = useState(false);
  const [startUnknown, setStartUnknown] = useState(false);
  const startLock = useRef(false);
  const [pendingRestart, setPendingRestart] = useState<string | null>(null);
  const [hoverSpace, setHoverSpace] = useState<string | null>(null);
  const detailInvoker = useRef<HTMLElement | null>(null);
  const [hoverRun, setHoverRun] = useState<string | null>(null);
  const [detailSection, setDetailSection] = useState<"overview" | "activity" | "actions">("overview");
  const [narrowView, setNarrowView] = useState<"board" | "agents">("board");
  const startDraft = useRef<StartDraft>({ label: "", location: "existing", spaceId: "", directory: "" });
  const lastSession = useRef(sessionId);
  const seenStart = useRef(0);
  const rootRef = useRef<HTMLElement>(null);
  const focusedTask = useRef<string | null>(null);
  const questionHadFocus = useRef(false);
  const startFocus = useRef<{ runId: string | null; invoker: Element | null } | null>(null);
  const lastTaskIds = useRef<string[]>([]);
  const opened = useRef(false);
  const [focusNotice, setFocusNotice] = useState<string | null>(null);
  const modalVisible = !!dialog && active && !!snapshot;
  useEffect(() => { onModalChange(modalVisible); return () => onModalChange(false); }, [modalVisible, onModalChange]);
  useEffect(() => {
    if (lastSession.current === sessionId) return;
    lastSession.current = sessionId; setRootId(null); setDialog(null); setTerminalError(null); setNotice(null); setStartUnknown(false); setPendingRestart(null);
    startDraft.current = { label: "", location: "existing", spaceId: "", directory: "" };
  }, [sessionId]);
  const root = snapshot?.runs.find(run => run.run_id === (rootId ?? snapshot.board?.root_id) && run.run_id === run.root_id && (run.stage !== "closed" || rootId === run.run_id)) ?? null;
  const openRoots = snapshot?.roots.filter(summary => snapshot.runs.some(run => run.run_id === summary.root_id && run.stage !== "closed")) ?? [];
  const closedRoots = snapshot?.roots.filter(summary => snapshot.runs.some(run => run.run_id === summary.root_id && run.stage === "closed")) ?? [];
  const orphanedWorkers = snapshot?.runs.filter(run => run.stage !== "closed" && !!run.parent_run_id && closedRoots.some(summary => summary.root_id === run.root_id)) ?? [];
  useEffect(() => { if (snapshot && !rootId && openRoots.length) setRootId(openRoots[0].root_id); }, [snapshot, rootId]);
  const live = connected && runtimeLive && snapshot?.runtime.status === "fresh";
  const destination = runtimeLive && session ? session.spaces.find(space => space.id === session.focused_space_id) : undefined;
  const rootState = root && snapshot ? agentState(snapshot, root, connected, runtimeLive) : null;
  const tasks = root && snapshot?.board?.root_id === root.run_id ? snapshot.board.tasks : [];
  const rootRuns = root ? snapshot?.runs.filter(run => run.root_id === root.run_id) ?? [] : [];
  const descendants = rootRuns.filter(run => run.run_id !== root?.run_id && run.stage !== "closed");
  const selectedAgent = rootRuns.find(run => run.run_id === scope.selectedRun) ?? null;
  const observations = live && snapshot?.runtime.status === "fresh" ? snapshot.runtime.runs : [];
  const observe = (runId: string | null | undefined) => observations.find(item => item.run_id === runId);
  const forest = snapshot ? supervisorForest(snapshot, scope.showSubagents).filter(row => row.run.root_id === root?.run_id && row.run.stage !== "closed") : [];
  const openTasks = tasks.filter(task => task.lane !== "accepted");
  const taskIds = taskLanes.flatMap(({ lane }) => lane === "accepted" && !scope.disclosures.completed ? [] : tasks.filter(task => task.lane === lane).map(task => task.task.task_id));
  const { listRef, listProps, tabIndexFor, focusRow } = useRovingList({ rowIds: taskIds, selectedId: scope.selectedTask, onEscape: () => { if (scope.selectedTask) { scope.selectedTask = null; changed(); } else onClose(); return true; } });
  useEffect(() => {
    if (!active) { opened.current = false; return; }
    if (!snapshot || opened.current || dialog) return;
    opened.current = true;
    const selected = rootRef.current?.querySelector<HTMLButtonElement>("[data-row-id][aria-expanded=true]");
    (selected ?? rootRef.current?.querySelector<HTMLElement>(root && root.stage !== "closed" ? "[data-task-composer]" : "[data-start-agent]"))?.focus({ preventScroll: true });
  }, [active, snapshot, dialog, root]);
  useEffect(() => {
    const removed = focusedTask.current && !taskIds.includes(focusedTask.current);
    if (removed && active) {
      const index = lastTaskIds.current.indexOf(focusedTask.current!);
      if (scope.selectedTask === focusedTask.current) { scope.selectedTask = null; changed(); }
      const next = taskIds[Math.min(Math.max(index, 0), taskIds.length - 1)];
      if (next) focusRow(next); else rootRef.current?.querySelector<HTMLTextAreaElement>("[data-task-composer]")?.focus();
      focusedTask.current = next ?? null; setFocusNotice("The focused task changed elsewhere. Focus moved to the next available task.");
    }
    lastTaskIds.current = taskIds;
  }, [tasks, scope.disclosures.completed, active]);
  const started = (runId: string) => {
    if (startFocus.current) startFocus.current.runId = runId;
    setRootId(runId); setStartUnknown(false); setNotice(null);
  };
  const start = async () => {
    if (!snapshot || !connected || busy || startLock.current || startUnknown || rootState?.kind === "starting") return;
    if (!destination) { setDialog({ mode: "start" }); return; }
    startFocus.current = { runId: null, invoker: document.activeElement };
    startLock.current = true; setStartPending(true); setNotice(null);
    try {
      const result = await mutateResult({ action: "supervisor_start", target: { target: "existing_space", workspace_id: destination.id }, label: null });
      if (result?.result === "run") started(result.run_id);
      else { setStartUnknown(true); setNotice("The start was not confirmed. Check status before starting another agent; a terminal may already have opened."); }
    } finally { startLock.current = false; setStartPending(false); }
  };
  useEffect(() => {
    if (startToken <= seenStart.current || !active || !snapshot || !connected) return;
    seenStart.current = startToken; void start();
  }, [startToken, active, snapshot, connected]);
  useEffect(() => {
    if (!active || !rootState?.verified || !root || startFocus.current?.runId !== root.run_id) return;
    if (document.activeElement === startFocus.current.invoker) rootRef.current?.querySelector<HTMLTextAreaElement>("[data-task-composer]")?.focus({ preventScroll: true });
    startFocus.current = null;
  }, [active, root, rootState?.verified]);
  const navigate = async (run: Run) => {
    if (!snapshot || !live || busy) return;
    try { setTerminalError(null); await onTerminal(run, snapshot); }
    catch (cause) { setTerminalError(`Could not open terminal. ${cause instanceof Error ? cause.message : "Check its current location and try again."}`); }
  };
  const check = (run: Run) => {
    if (!connected || !runtimeLive || !run.dispatch) { refresh(); return; }
    void mutateResult({ action: "reconcile_run", run_id: run.run_id, recovery: null });
  };
  const restart = async (run: Run) => {
    if (!connected || !runtimeLive || busy || !run.dispatch) return;
    if (run.dispatch.step === "setup_unknown") { setDialog({ mode: "setup_recovery", run }); return; }
    if (run.dispatch.step === "plan_failed") {
      await mutateResult({ action: "reconcile_run", run_id: run.run_id, recovery: null }); return;
    }
    if (run.dispatch?.step === "launch_unknown" || run.dispatch?.step === "needs_review") { setDialog({ mode: "retry", run }); return; }
    setNotice("Checking the previous launch before restart…");
    if (await mutateResult({ action: "reconcile_run", run_id: run.run_id, recovery: null })) setPendingRestart(run.run_id);
    else setNotice("The previous launch could not be checked. No new agent was requested; tasks and history are kept.");
  };
  useEffect(() => {
    if (!pendingRestart || !snapshot || !active) return;
    if (!connected || !runtimeLive || snapshot.runtime.status !== "fresh") { setPendingRestart(null); setNotice("The previous launch is unobserved. Check the connection before restarting. No new agent was requested."); return; }
    const run = snapshot.runs.find(item => item.run_id === pendingRestart);
    if (!run || run.stage === "closed") { setPendingRestart(null); return; }
    if (run.dispatch?.step === "launch_unknown" || run.dispatch?.step === "needs_review") { setPendingRestart(null); setNotice(null); setDialog({ mode: "retry", run }); }
    else if (agentState(snapshot, run, connected, runtimeLive).verified) { setPendingRestart(null); setNotice("The original OMP agent is still connected. No new launch was started."); }
  }, [pendingRestart, snapshot, active, connected, runtimeLive]);
  const edit = (task: TaskView) => {
    let draft = scope.edits.get(task.task.task_id);
    if (!draft) { draft = { title: task.task.title, body: task.task.body, revision: task.task.task_revision }; scope.edits.set(task.task.task_id, draft); }
    setDialog({ mode: "edit", task, draft });
  };
  const disclosures = (key: keyof ScopeDrafts["disclosures"], open: boolean) => { if (scope.disclosures[key] !== open) { scope.disclosures[key] = open; changed(); } };
  const recovery = (run: Run) => snapshot ? <AgentRecovery key={run.run_id} run={run} state={agentState(snapshot, run, connected, runtimeLive)} busy={busy || startPending || pendingRestart === run.run_id} connected={connected} runtimeLive={runtimeLive && snapshot.runtime.status === "fresh"} onCheck={() => check(run)} onRestart={() => void restart(run)} onCloseTracking={() => setDialog({ mode: "close", run })} onTerminal={() => void navigate(run)} /> : null;
  const failures = descendants.filter(run => snapshot && ["failure", "missing", "unknown"].includes(agentState(snapshot, run, connected, runtimeLive).kind) && !["proposed", "awaiting_prepare"].includes(run.stage));
  const question = root?.stage !== "closed" && root?.last_report?.kind === "needs_input" ? root.last_report : null;
  const answer = question && root ? messageDraft(scope, `answer:${root.run_id}:${question.message_id}`) : null;
  const answered = question && root && snapshot?.messages.some(message => message.to_run_id === root.run_id && message.kind === "answer" && !message.stale && message.created_at >= question.at);
  useEffect(() => {
    if (active && questionHadFocus.current && (!question || answered)) {
      questionHadFocus.current = false;
      rootRef.current?.querySelector<HTMLTextAreaElement>("[data-task-composer]")?.focus({ preventScroll: true });
    }
  }, [active, question?.message_id, answered]);
  const assignmentIntents = snapshot?.assignment_intents.filter(intent => intent.root_id === root?.run_id) ?? [];
  const acceptanceConflicts = snapshot?.intents.filter(intent => intent.root_id === root?.run_id && intent.state === "conflict") ?? [];
  const history = snapshot ? [
    ...snapshot.messages.filter(message => rootRuns.some(run => run.run_id === message.to_run_id)).map(message => {
      const recipient = snapshot.runs.find(run => run.run_id === message.to_run_id)?.label ?? "Agent";
      const task = message.message_id.startsWith("assign-") ? tasks.find(task => `assign-${task.task.task_id}` === message.message_id) : undefined;
      const systemBrief = ["supervisor_brief", "prepare_brief", "work_brief"].includes(message.kind);
      const pointer = message.kind === "observation" || !!task;
      const summary = message.report?.summary ?? (task ? `Task assigned: ${task.task.title}` : systemBrief ? `Guidance delivered to ${recipient}` : pointer ? `Lifecycle update for ${recipient}` : message.text);
      return { id: `message:${message.message_id}`, label: `${message.kind.replaceAll("_", " ")} · ${actorName(message.from, snapshot)}`, summary, at: message.created_at, detail: systemBrief || pointer ? `${summary}. Exact delivery records are available in Diagnostics.` : message.report?.summary ?? message.text, stale: message.stale };
    }),
    ...rootRuns.flatMap(run => run.grants.map(grant => ({ id: grant.grant_id, label: `${grant.scope === "prepare" ? "Preparation" : "Execution"} authorized · ${grant.origin === "supervisor" ? snapshot.runs.find(root => root.run_id === grant.supervisor_run_id)?.label ?? "Supervisor" : `You (${grant.origin})`}`, summary: run.label, at: grant.granted_at, detail: `An exact ${grant.scope === "prepare" ? "preparation" : "work"} plan was authorized for ${run.label}. Actor, actual OMP session and exact revision are retained in Diagnostics.`, stale: false }))),
    ...rootRuns.flatMap(run => run.annotations.map((note, index) => {
      const receiptNote = ["Acceptance requested for Result ", "Accepted Result at exact task revision ", "Recovered acceptance of Result "].some(prefix => note.text.startsWith(prefix));
      const summary = receiptNote ? `Result review note for ${run.label}` : note.text;
      return { id: `${run.run_id}:note:${index}`, label: `Note · ${actorName(note.by, snapshot)}`, summary, at: note.at, detail: receiptNote ? `${summary}. Exact result, revision and native session references are retained in Diagnostics.` : note.text, stale: false };
    })),
  ].sort((a, b) => b.at.localeCompare(a.at)) : [];
  const sharedSpace = hoverSpace ?? observe(selectedAgent?.run_id)?.workspace_id ?? observe(tasks.find(task => task.task.task_id === scope.selectedTask)?.current_run_id)?.workspace_id;
  const selectedTask = tasks.find(task => task.task.task_id === scope.selectedTask) ?? null;
  const detailRun = selectedTask ? snapshot?.runs.find(run => run.run_id === selectedTask.current_run_id) ?? null : selectedAgent;
  const detailTask = selectedTask ?? tasks.find(task => task.task.task_id === detailRun?.task_id) ?? null;
  const detailSubagent = !selectedTask ? snapshot?.subagents.find(agent => agent.run_id === selectedAgent?.run_id && agent.subagent_id === scope.selectedSubagent) ?? null : null;
  const detailOpen = !!selectedTask || !!selectedAgent;
  const hasAttention = !!question && !answered || failures.length > 0 || !!root && (rootState?.kind !== "ready" || !!rootState?.blocked || root.last_report?.outcome === "failed") || orphanedWorkers.length > 0 || assignmentIntents.length > 0 || acceptanceConflicts.length > 0 || !!snapshot?.board?.unidentified_items || !!notice || startUnknown || !!terminalError || !!navigationError || !!error;
  useEffect(() => {
    const width = rootRef.current?.getBoundingClientRect().width ?? 0;
    const invoker = document.activeElement as HTMLElement | null;
    if (detailOpen && invoker?.dataset.rowId && (invoker.dataset.rowId === scope.selectedTask || !!scope.selectedTask && invoker.dataset.rowId.endsWith(`:${scope.selectedTask}`) || invoker.dataset.rowId === scope.selectedRun || invoker.dataset.rowId === `${scope.selectedRun}:${scope.selectedSubagent}`)) detailInvoker.current = invoker;
    if (active && width > 0 && width < 720 && (detailOpen || scope.disclosures.history || scope.disclosures.diagnostics)) rootRef.current?.querySelector<HTMLButtonElement>(".supervisor-detail-header button")?.focus({ preventScroll: true });
  }, [active, scope.selectedTask, scope.selectedRun, scope.selectedSubagent, scope.disclosures.history, scope.disclosures.diagnostics]);
  const selectTask = (id: string) => { scope.selectedTask = scope.selectedTask === id ? null : id; scope.selectedRun = null; scope.selectedSubagent = null; setDetailSection("overview"); changed(); };
  const closeDetail = () => {
    const taskId = scope.selectedTask;
    const agentId = scope.selectedRun ? scope.selectedSubagent ? `${scope.selectedRun}:${scope.selectedSubagent}` : scope.selectedRun : null;
    scope.selectedTask = null; scope.selectedRun = null; scope.selectedSubagent = null; changed();
    requestAnimationFrame(() => {
      if (detailInvoker.current?.isConnected && rootRef.current?.contains(detailInvoker.current)) { detailInvoker.current.focus(); return; }
      if (taskId) focusRow(taskId);
      else if (agentId) [...rootRef.current?.querySelectorAll<HTMLElement>("[data-row-id]") ?? []].find(element => element.dataset.rowId === agentId)?.focus();
    });
  };
  const renderTask = (task: TaskView) => {
    if (!snapshot) return null;
    const worker = snapshot.runs.find(run => run.run_id === task.current_run_id);
    const observed = observe(worker?.run_id);
    const selected = scope.selectedTask === task.task.task_id;
    const needsAttention = !!worker && (worker.last_report?.kind === "needs_input" || ["failure", "missing", "unknown"].includes(agentState(snapshot, worker, connected, runtimeLive).kind)) || assignmentIntents.some(intent => intent.task_id === task.task.task_id) || acceptanceConflicts.some(intent => intent.task_id === task.task.task_id);
    const dimmed = scope.attentionOnly && !needsAttention || !!scope.spaceFilter && observed?.workspace_id !== scope.spaceFilter;
    const linked = !!worker && (hoverRun === worker.run_id || selectedAgent?.run_id === worker.run_id);
    return <li key={task.task.task_id} className={`supervisor-task${selected ? " is-selected" : ""}${dimmed ? " is-dimmed" : ""}${linked ? " is-linked" : ""}`} onMouseEnter={() => { setHoverRun(worker?.run_id ?? null); setHoverSpace(observed?.workspace_id ?? null); }} onMouseLeave={() => { setHoverRun(null); setHoverSpace(null); }}>
      <button type="button" data-row-id={task.task.task_id} tabIndex={tabIndexFor(task.task.task_id)} className="supervisor-task-toggle" aria-expanded={selected} aria-controls="supervisor-detail" onFocus={() => { focusedTask.current = task.task.task_id; }} onClick={() => selectTask(task.task.task_id)}>
        <span className="supervisor-task-title">{task.task.title}</span><span className="supervisor-task-stage">{taskStatus(task, worker, snapshot)}</span>
        {needsAttention ? <span className="supervisor-attention-badge"><UiIcon name="info" />Needs attention</span> : null}
        {worker ? <span className="supervisor-worker-chip"><StateGlyph shape={observed?.actual_omp && observed.presence === "present" && ["working", "idle", "blocked", "done"].includes(observed.agent_status ?? "") ? observed.agent_status as "working" | "idle" | "blocked" | "done" : "unknown"} />{worker.label}</span> : null}
      </button>
      {worker ? <div className="supervisor-task-evidence">{observed?.workspace_label ? <span className={`supervisor-location${sharedSpace === observed.workspace_id ? " is-shared" : ""}`} title={observed.tab_label ? `${observed.workspace_label} · ${observed.tab_label}` : observed.workspace_label}><UiIcon name="folder" />{observed.workspace_label}</span> : null}<ReportedEvidence report={worker.last_report} source={worker.label} /><ObservedEvidence run={worker} observed={observed} snapshot={snapshot} live={!!live} />
        {observed?.agent_status === "blocked" && worker.last_report?.kind !== "needs_input" ? <p className="supervisor-warning">Agent is blocked; no question reported. <button type="button" disabled={busy || !live} onClick={() => void navigate(worker)}>Open terminal</button><button type="button" disabled={busy} onClick={() => check(worker)}>Check status</button></p> : null}
        {observed?.agent_status === "done" && !worker.result ? <p className="supervisor-muted">Runtime Done · no result reported</p> : null}
      </div> : <p className="supervisor-evidence">No progress reported yet</p>}
    </li>;
  };
  return <section ref={rootRef} className={`supervisor-view${hasAttention ? " has-attention" : ""}`} hidden={!active} aria-label="Supervisor" onFocusCapture={event => {
    const target = event.target as HTMLElement;
    focusedTask.current = target.closest(".supervisor-detail-panel") ? scope.selectedTask : target.closest("li.supervisor-task")?.querySelector<HTMLElement>("[data-row-id]")?.dataset.rowId ?? null;
    questionHadFocus.current = !!target.closest('[aria-label="Needs you"]');
  }} onKeyDown={event => {
    if (event.key !== "Escape" || event.nativeEvent.isComposing || event.defaultPrevented || dialog || (event.target as HTMLElement).closest("input,textarea,select,[contenteditable=true]")) return;
    event.preventDefault(); event.stopPropagation();
    if (detailOpen) closeDetail();
    else if (scope.disclosures.history || scope.disclosures.diagnostics || scope.disclosures.archive) { disclosures("history", false); disclosures("diagnostics", false); disclosures("archive", false); }
    else onClose();
  }}><header className="supervisor-header"><span className="supervisor-brand"><UiIcon name="branch" /><strong>Supervisor</strong></span>
    {rootState ? <span className="supervisor-header-state"><StateGlyph shape={rootState.blocked ? "blocked" : rootState.verified ? "live" : "unknown"} /><span>{rootState.label}</span></span> : null}
    {openRoots.length > 1 ? <label className="supervisor-agent-label"><select aria-label="Agent" value={root?.stage !== "closed" ? root?.run_id ?? "" : ""} disabled={busy || startPending || !!dialog} onChange={event => { setRootId(event.target.value); setTerminalError(null); }}><option value="" disabled>Choose an agent</option>{openRoots.map(root => <option key={root.root_id} value={root.root_id}>{root.label}</option>)}</select></label> : null}
    <nav className="supervisor-header-panels" aria-label="Supervisor panels"><button type="button" aria-label="Activity" title="Activity" aria-pressed={scope.disclosures.history} onClick={() => { scope.selectedTask = null; scope.selectedRun = null; scope.selectedSubagent = null; disclosures("history", !scope.disclosures.history); disclosures("diagnostics", false); changed(); }}><UiIcon name="comment" /></button><button type="button" aria-label="Diagnostics" title="Diagnostics" aria-pressed={scope.disclosures.diagnostics} onClick={() => { scope.selectedTask = null; scope.selectedRun = null; scope.selectedSubagent = null; disclosures("diagnostics", !scope.disclosures.diagnostics); disclosures("history", false); changed(); }}><UiIcon name="info" /></button>{closedRoots.length ? <button type="button" aria-label={`Closed tracking · ${closedRoots.length}`} title={`Closed tracking · ${closedRoots.length}`} aria-expanded={scope.disclosures.archive} onClick={() => disclosures("archive", !scope.disclosures.archive)}><UiIcon name="folder" /></button> : null}</nav>
    <button type="button" className="supervisor-primary" title={`${destination ? `New tab in ${destination.label}` : "Choose an available location when starting."} · Starts OMP without switching terminal focus.`} data-start-agent disabled={busy || startPending || !snapshot || !connected || startUnknown || rootState?.kind === "starting"} onClick={() => void start()}><UiIcon name="plus" />{startPending ? "Starting…" : "Start agent"}</button><button type="button" className="supervisor-secondary-button" aria-label="Start options…" title="Start options…" disabled={busy || startPending || !snapshot || !connected || startUnknown || rootState?.kind === "starting"} onClick={() => { if (!startDraft.current.spaceId && destination) startDraft.current.spaceId = destination.id; setDialog({ mode: "start" }); }}><UiIcon name="more" /></button><button type="button" aria-label="Close Supervisor" title="Close Supervisor" onClick={onClose}><UiIcon name="close" /></button></header>
    {scope.disclosures.archive && snapshot ? <section className="supervisor-archive" aria-label="Closed tracking"><p>Tasks and history remain available. Closing tracking did not kill agents or remove resources.</p>{closedRoots.map(summary => <button type="button" disabled={busy || !!dialog} key={summary.root_id} onClick={() => { setRootId(summary.root_id); disclosures("archive", false); }}>View {summary.label} tasks and history</button>)}{root?.stage === "closed" && openRoots.length ? <button type="button" disabled={busy} onClick={() => setRootId(openRoots[0].root_id)}>Return to {openRoots[0].label}</button> : null}</section> : null}
    <div className={`supervisor-workarea${detailOpen ? " has-detail" : ""}${detailOpen || scope.disclosures.history || scope.disclosures.diagnostics ? " has-panel" : ""}`}><div className="supervisor-content">
      {!snapshot ? <div className="supervisor-empty"><UiIcon name="branch" /><h2>{error ? "Could not load Supervisor" : "Loading Supervisor…"}</h2>{error ? <><p>Your terminals are unchanged. The Supervisor connection could not be established.</p><button type="button" onClick={refresh}>Retry load</button></> : <p role="status">Reading saved tasks and fresh agent observations.</p>}</div> : <>
      <div className={`supervisor-attention-region${!hasAttention && !root && !orphanedWorkers.length ? " is-empty" : ""}`} aria-label="Supervisor status">
        {notice ? <p role="status"><UiIcon name="info" />{notice}</p> : null}{startUnknown ? <div className="supervisor-warning" role="status"><UiIcon name="info" /><button type="button" disabled={busy} onClick={refresh}>Check status</button><p>Review the tracked agents below before another start.</p><button type="button" disabled={busy} onClick={() => { setStartUnknown(false); setNotice("Previous start reviewed. Any existing terminal and tracking remain unchanged."); }}>I have reviewed the previous start</button></div> : null}
        {terminalError || navigationError || error ? <div className="supervisor-error" role="alert"><UiIcon name="info" /><p>{terminalError ?? (navigationError ? "Could not open terminal. Its current focus or location was not confirmed." : !connected ? "Could not refresh Supervisor. Saved tasks and drafts are kept; the agent may still be running." : "The requested change was not confirmed. Your drafts and resources are kept. Check current status before trying again.")}</p><button type="button" disabled={busy} onClick={refresh}>Check status</button></div> : null}
        {!root && orphanedWorkers.length ? <section className="supervisor-warning"><h2>Worker agents need control</h2><p>Closing their supervisor did not stop them. Tasks and history are kept.</p>{orphanedWorkers.map(run => <div key={run.run_id}>{recovery(run)}<button type="button" disabled={busy} onClick={() => setRootId(run.root_id)}>View saved task context</button></div>)}</section> : null}
        {root ? <>{recovery(root)}{root.stage === "closed" && descendants.length ? <section className="supervisor-warning"><h2>Worker agents need control</h2><p>{descendants.length} tracked descendants remain open. Closing this supervisor did not stop them.</p>{descendants.map(run => recovery(run))}</section> : failures.length ? <section className="supervisor-needs-you"><h2>{failures.length === 1 ? "Task needs recovery" : `${failures.length} tasks need recovery`}</h2>{failures.map(run => recovery(run))}</section> : null}
        {!question && root.last_report ? <ReportedEvidence report={root.last_report} source={root.label} /> : null}
        <ObservedEvidence run={root} observed={observe(root.run_id)} snapshot={snapshot} live={!!live} />
        {observe(root.run_id)?.agent_status === "blocked" && !question ? <section className="supervisor-warning"><p>Agent is blocked; no question reported.</p><div className="supervisor-action-row"><button type="button" disabled={busy || !live || !rootState?.terminal} onClick={() => void navigate(root)}>Open terminal</button><button type="button" disabled={busy} onClick={() => check(root)}>Check status</button></div></section> : null}
        {question && answer && !answered ? <section className="supervisor-needs-you" aria-label="Needs you" role="status"><div className="supervisor-question"><h2><UiIcon name="comment" />Needs you · {root.label}</h2><p id={`supervisor-question-${question.message_id}`} className="supervisor-exact-text">{question.summary}</p><p className="supervisor-muted">Reported by {root.label} · <time dateTime={question.at}>{new Date(question.at).toLocaleString()}</time></p></div><TextAction label="Answer" submitLabel="Send answer" describedBy={`supervisor-question-${question.message_id}`} draft={answer} changed={changed} busy={busy || !live || !rootState?.verified} submit={(text, message_id) => mutateResult({ action: "message_send", message_id, to_run_id: root.run_id, kind: "answer", text })} success="Answer sent. Waiting for the agent." /></section> : question && answered ? <p role="status">Answer sent. Waiting for the agent.</p> : null}
        {assignmentIntents.map(intent => {
          const canonical = tasks.find(task => task.task.task_id === intent.task_id);
          const submitted = scope.task.operation?.id === intent.task_id ? scope.task.operation : null;
          const resolve = async (assign: boolean) => {
            const result = await mutateResult({ action: "task_assignment_resolve", root_id: root.run_id, task_id: intent.task_id, expected_task_revision: assign ? canonical?.task.task_revision ?? null : null, assign });
            if (assign ? result?.result === "task_assigned" : result?.result === "done") {
              if (submitted) { if (assign && scope.task.text === submitted.text) scope.task.text = ""; scope.task.operation = null; scope.task.error = null; scope.task.notice = assign ? `Assigned current task to ${root.label}.` : "Task kept unassigned. The original draft is kept."; changed(); }
            }
          };
          return <section key={intent.task_id} className="supervisor-needs-you"><h2>{intent.state === "conflict" ? "Task changed elsewhere · Not assigned" : "Task assignment pending"}</h2><p>{canonical?.task.title ?? "The canonical task is not available yet."}</p>{canonical ? <details><summary>Current task</summary><p className="supervisor-exact-text">{canonical.task.body}</p></details> : null}{submitted ? <details><summary>Original submitted draft</summary><p className="supervisor-exact-text">{submitted.text}</p></details> : null}{intent.state === "conflict" ? <div className="supervisor-action-row"><button type="button" disabled={busy || !live || !canonical || !!canonical.task.diagnostic || root.stage === "closed"} onClick={() => void resolve(true)}>Assign current task</button><button type="button" disabled={busy || !connected} onClick={() => void resolve(false)}>Keep unassigned</button></div> : <button type="button" disabled={busy} onClick={refresh}>Check assignment status</button>}</section>;
        })}
        {acceptanceConflicts.map(intent => <section key={intent.intent_id} className="supervisor-needs-you"><h2>Task changed during acceptance</h2><p>{tasks.find(task => task.task.task_id === intent.task_id)?.task.title ?? "Task"} · Review the current task before applying acceptance. Keeping it leaves Markdown unchanged.</p><button type="button" disabled={busy || !live} onClick={() => void mutateResult({ action: "intent_resolve", intent_id: intent.intent_id, apply: true })}>Apply acceptance to current task</button><button type="button" disabled={busy || !connected} onClick={() => void mutateResult({ action: "intent_resolve", intent_id: intent.intent_id, apply: false })}>Keep current task unchanged</button></section>)}
        {snapshot.board?.unidentified_items ? <section className="supervisor-warning"><p>{snapshot.board.unidentified_items} task-file items need identity markers before they can be managed.</p><button type="button" disabled={busy || !connected} onClick={() => void mutateResult({ action: "tasks_assign_ids", root_id: root.run_id, expected_doc_revision: snapshot.board!.doc_revision })}>Identify task-file items</button></section> : null}
        </> : null}
      </div>
      {root ? <>
        <div className="supervisor-narrow-switch" aria-label="Workarea view"><button type="button" aria-pressed={narrowView === "board"} onClick={() => setNarrowView("board")}><UiIcon name="grid" />Board</button><button type="button" aria-pressed={narrowView === "agents"} onClick={() => setNarrowView("agents")}><UiIcon name="branch" />Agents</button></div>
        <div className={`supervisor-graph-surface${narrowView === "agents" ? " is-narrow-active" : ""}`}>
          <SupervisorGraph rows={forest} snapshot={snapshot} live={!!live} connected={connected} runtimeLive={runtimeLive} busy={busy} scope={scope} sharedSpace={sharedSpace} highlightedRun={hoverRun ?? detailRun?.run_id ?? null} changed={changed} onHover={run => { setHoverRun(run?.run_id ?? null); setHoverSpace(observe(run?.run_id)?.workspace_id ?? null); }} onSelect={(run, subagent) => { const selected = scope.selectedRun === run.run_id && scope.selectedSubagent === (subagent?.subagent_id ?? null); scope.selectedTask = null; scope.selectedRun = selected ? null : run.run_id; scope.selectedSubagent = selected ? null : subagent?.subagent_id ?? null; setDetailSection("overview"); changed(); }} onSelectTask={selectTask} onUnmanaged={paneId => void onUnmanagedTerminal(paneId).catch(cause => setTerminalError(`Could not open terminal. ${cause instanceof Error ? cause.message : "Check status."}`))} />
        </div>
        <section className={`supervisor-board-surface${narrowView === "board" ? " is-narrow-active" : ""}`} aria-label="Tasks">
          <div className="supervisor-task-list-heading"><h2>Tasks <span>{openTasks.length} open</span></h2><div className="supervisor-task-filters" aria-label="Task filters"><label className={scope.attentionOnly ? "is-active" : ""}><input type="checkbox" checked={scope.attentionOnly} onChange={event => { scope.attentionOnly = event.target.checked; changed(); }} />Needs attention (dims other tasks)</label><label>Space<select aria-label="Task Space" value={scope.spaceFilter} onChange={event => { scope.spaceFilter = event.target.value; changed(); }}><option value="">All Spaces</option>{[...new Map(observations.filter(item => item.workspace_id && item.workspace_label && rootRuns.some(run => run.run_id === item.run_id)).map(item => [item.workspace_id!, item.workspace_label!])).entries()].map(([id, label]) => <option key={id} value={id}>{label}</option>)}</select></label></div></div>
          <div ref={listRef} {...listProps} className="supervisor-board" onKeyDown={event => {
            const id = (event.target as HTMLElement).dataset.rowId;
            if (id && !event.ctrlKey && !event.altKey && !event.metaKey && !event.nativeEvent.isComposing) {
              const next = taskNeighbor(tasks, id, event.key, scope.disclosures.completed);
              if (next) { event.preventDefault(); focusRow(next); return; }
            }
            listProps.onKeyDown(event);
          }}>{taskLanes.map(({ lane, label }, index) => <section key={lane} className={`supervisor-lane is-${lane}${lane === "accepted" && !scope.disclosures.completed ? " is-collapsed" : ""}`} aria-label={`${label} tasks`}>
            <header className="supervisor-lane-header"><span className="supervisor-lane-dot" /><strong>{label}</strong><span className="supervisor-lane-count">{tasks.filter(task => task.lane === lane).length}</span>{lane === "accepted" ? <button type="button" aria-label={scope.disclosures.completed ? "Hide completed tasks" : "Show completed tasks"} aria-expanded={scope.disclosures.completed} onClick={() => disclosures("completed", !scope.disclosures.completed)}><UiIcon name={scope.disclosures.completed ? "down" : "right"} /></button> : index < taskLanes.length - 1 ? <UiIcon name="right" /> : null}</header>
            <ul className="supervisor-task-list" hidden={lane === "accepted" && !scope.disclosures.completed}>{tasks.filter(task => task.lane === lane).map(renderTask)}</ul>
            {!tasks.some(task => task.lane === lane) ? <p className="supervisor-lane-empty">No tasks</p> : null}
          </section>)}</div>
        </section>
      </> : <div className="supervisor-empty"><UiIcon name="branch" /><h2>Start an agent to manage your tasks.</h2><p>Give it work here or in its terminal. It handles delegation, preparation, execution and review.</p></div>}
      </>}
    </div>
    {snapshot && active && (detailOpen || scope.disclosures.history || scope.disclosures.diagnostics) ? <aside className={`supervisor-detail-panel${detailOpen ? " is-selection" : ""}`} id="supervisor-detail" aria-label={detailOpen ? "Selected details" : scope.disclosures.diagnostics ? "Diagnostics" : "History"} onKeyDown={event => {
      if (event.key !== "Escape" || event.defaultPrevented || event.nativeEvent.isComposing || (event.target as HTMLElement).closest("input,textarea,select,[contenteditable=true]")) return;
      event.preventDefault(); event.stopPropagation();
      if (detailOpen) closeDetail(); else { disclosures("history", false); disclosures("diagnostics", false); }
    }}>
      <header className="supervisor-detail-header"><button type="button" aria-label={detailOpen ? "Close details" : "Close activity panel"} onClick={() => { if (detailOpen) closeDetail(); else { disclosures("history", false); disclosures("diagnostics", false); } }}><UiIcon name="back" /></button><strong>{detailOpen ? detailTask?.task.title ?? detailSubagent?.label ?? detailRun?.label : scope.disclosures.diagnostics ? "Diagnostics" : "Activity"}</strong></header>
      {detailOpen ? <><nav className="supervisor-segments" aria-label="Detail sections">{(["overview", "activity", "actions"] as const).map(section => <button type="button" key={section} aria-pressed={detailSection === section} onClick={() => setDetailSection(section)}>{section[0].toUpperCase() + section.slice(1)}</button>)}</nav><div className="supervisor-detail-scroll"><SupervisorActions key={`${scope.selectedTask ?? scope.selectedRun}:${scope.selectedSubagent ?? ""}`} section={detailSection} snapshot={snapshot} run={detailRun} task={detailTask} subagent={detailSubagent} scope={scope} changed={changed} busy={busy} live={!!live} mutateResult={mutateResult} onTerminal={run => void navigate(run)} onEditTask={edit} onCloseTracking={run => setDialog({ mode: "close", run })} onCancelSubagent={(run, subagent) => setDialog({ mode: "subagent_cancel", run, subagent })} /></div></> : <div className="supervisor-detail-scroll">
        {scope.disclosures.history ? <section aria-label="History"><h3>Earlier</h3>{history.length ? <ul className="supervisor-history">{history.map(event => <li key={event.id}><strong>{event.label}{event.stale ? " · stale evidence" : ""}</strong><time dateTime={event.at}>{new Date(event.at).toLocaleString()}</time><p className="supervisor-exact-text">{event.detail}</p></li>)}</ul> : <p>No recorded history in this scope.</p>}</section> : null}
        {scope.disclosures.diagnostics ? <section aria-label="Diagnostics"><p>Canonical task source · {snapshot.board?.path ?? "No selected task document"}</p>{snapshot.board?.unidentified_items ? <p>{snapshot.board.unidentified_items} task-file checklist items need identity markers. <button type="button" disabled={busy || !connected} onClick={() => void mutateResult({ action: "tasks_assign_ids", root_id: snapshot.board!.root_id, expected_doc_revision: snapshot.board!.doc_revision })}>Identify task-file items</button></p> : null}{snapshot.board?.diagnostics.map((diagnostic, index) => <p className="supervisor-error" key={index}>{diagnostic.message}</p>)}{rootRuns.map(run => <RunDiagnostics key={run.run_id} run={run} snapshot={snapshot} />)}<h3>Fresh runtime and transaction records</h3><pre className="supervisor-plan">{JSON.stringify({ runtime: snapshot.runtime, intents: snapshot.intents, assignment_intents: snapshot.assignment_intents }, null, 2)}</pre></section> : null}
      </div>}
    </aside> : null}
    </div>
    {snapshot && root ? <TaskComposer root={root} scope={scope} snapshot={snapshot} canAssign={!!rootState?.verified && root.stage === "active" && !!live} busy={busy || startPending} changed={changed} mutateResult={mutateResult} refresh={refresh} /> : null}
    <div className="supervisor-focus-notice" role="status">{focusNotice}</div>
    {dialog && snapshot && active ? <SupervisorDialogs dialog={dialog} snapshot={snapshot} spaces={runtimeLive ? session?.spaces ?? [] : []} startDraft={startDraft.current} changed={changed} busy={busy} available={connected && (dialog.mode === "edit" || dialog.mode === "close" || runtimeLive && snapshot.runtime.status === "fresh")} mutateResult={mutateResult} onStarted={started} onEdited={taskId => { scope.edits.delete(taskId); changed(); }} onStartUnconfirmed={() => setStartUnknown(true)} onClose={() => setDialog(null)} /> : null}
  </section>;
}
