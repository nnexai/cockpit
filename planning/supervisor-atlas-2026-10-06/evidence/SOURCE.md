# Supervisor source evidence snapshot

Captured 2026-10-06T19:26:10.601Z. Atlas citations refer to original source line numbers at this snapshot, not necessarily a later concurrently edited checkout. No product code was changed by the atlas worker. Each section reproduces the selected source as documentation evidence.

## SupervisorView.tsx

```tsx
import { useEffect, useRef, useState, type CSSProperties } from "react";
import { RowSplitter } from "./RowSplitter";
import type { CockpitClient } from "../../client/CockpitClient";
import type { ActorRef, OrchestrationSnapshot, Run, SessionSnapshotResponse, Subagent, TaskView } from "../../protocol/generated/v1";
import { useRovingList } from "../sidebar/useRovingList";
import { StateGlyph } from "../sidebar/StateGlyph";
import { UiIcon } from "../UiIcon";
import { SupervisorGraph } from "./SupervisorGraph";
import { taskLanes, taskNeighbor } from "./boardNavigation";
import { AgentRecovery, agentState, ObservedEvidence, ReportedEvidence, RunDiagnostics, SupervisorActions, taskStatus, TextAction } from "./SupervisorActions";
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
function actorName(actor: ActorRef, snapshot: OrchestrationSnapshot): string {
  if (actor.type === "operator") return "You";
  if (actor.type === "dispatcher") return "Dispatcher";
  return snapshot.runs.find(run => run.run_id === actor.run_id)?.label ?? "Agent";
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
  const [graphHeight, setGraphHeight] = useState<number | null>(null);
  const startDraft = useRef<StartDraft>({ label: "", location: "existing", spaceId: "", directory: "" });
  const lastSession = useRef(sessionId);
  const seenStart = useRef(0);
  const rootRef = useRef<HTMLElement>(null);
  const focusedTask = useRef<string | null>(null);
  const questionHadFocus = useRef(false);
  const startFocus = useRef<{ runId: string | null; invoker: Element | null } | null>(null);
  const lastTaskIds = useRef<string[]>([]);
  const opened = useRef(false);
  const initialRootSnapshot = useRef<OrchestrationSnapshot | null>(null);
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
  useEffect(() => {
    if (snapshot && !rootId && openRoots.length) {
      initialRootSnapshot.current = snapshot;
      setRootId(openRoots[0].root_id);
    }
  }, [snapshot, rootId]);
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
  const focusTaskOrStart = () => {
    const taskId = scope.selectedTask && taskIds.includes(scope.selectedTask) ? scope.selectedTask : taskIds[0];
    if (taskId) focusRow(taskId);
    else rootRef.current?.querySelector<HTMLButtonElement>("[data-start-agent]")?.focus({ preventScroll: true });
  };
  useEffect(() => {
    if (!active) { opened.current = false; return; }
    // Selecting the initial root reloads the board; focus only its scoped snapshot.
    if (!snapshot || snapshot === initialRootSnapshot.current || opened.current || dialog) return;
    initialRootSnapshot.current = null;
    opened.current = true;
    focusTaskOrStart();
  }, [active, snapshot, dialog, root]);
  useEffect(() => {
    if (!snapshot) return;
    const removed = focusedTask.current && !taskIds.includes(focusedTask.current);
    if (removed && active) {
      const index = lastTaskIds.current.indexOf(focusedTask.current!);
      if (scope.selectedTask === focusedTask.current) { scope.selectedTask = null; changed(); }
      const next = taskIds[Math.min(Math.max(index, 0), taskIds.length - 1)];
      if (next) focusRow(next); else rootRef.current?.querySelector<HTMLButtonElement>("[data-start-agent]")?.focus({ preventScroll: true });
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
    if (document.activeElement === startFocus.current.invoker) focusTaskOrStart();
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
      focusTaskOrStart();
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
          const resolve = async (assign: boolean) => {
            await mutateResult({ action: "task_assignment_resolve", root_id: root.run_id, task_id: intent.task_id, expected_task_revision: assign ? canonical?.task.task_revision ?? null : null, assign });
          };
          return <section key={intent.task_id} className="supervisor-needs-you"><h2>{intent.state === "conflict" ? "Task changed elsewhere · Not assigned" : "Task assignment pending"}</h2><p>{canonical?.task.title ?? "The canonical task is not available yet."}</p>{canonical ? <details><summary>Current task</summary><p className="supervisor-exact-text">{canonical.task.body}</p></details> : null}{intent.state === "conflict" ? <div className="supervisor-action-row"><button type="button" disabled={busy || !live || !canonical || !!canonical.task.diagnostic || root.stage === "closed"} onClick={() => void resolve(true)}>Assign current task</button><button type="button" disabled={busy || !connected} onClick={() => void resolve(false)}>Keep unassigned</button></div> : <button type="button" disabled={busy} onClick={refresh}>Check assignment status</button>}</section>;
        })}
        {acceptanceConflicts.map(intent => <section key={intent.intent_id} className="supervisor-needs-you"><h2>Task changed during acceptance</h2><p>{tasks.find(task => task.task.task_id === intent.task_id)?.task.title ?? "Task"} · Review the current task before applying acceptance. Keeping it leaves Markdown unchanged.</p><button type="button" disabled={busy || !live} onClick={() => void mutateResult({ action: "intent_resolve", intent_id: intent.intent_id, apply: true })}>Apply acceptance to current task</button><button type="button" disabled={busy || !connected} onClick={() => void mutateResult({ action: "intent_resolve", intent_id: intent.intent_id, apply: false })}>Keep current task unchanged</button></section>)}
        {snapshot.board?.unidentified_items ? <section className="supervisor-warning"><p>{snapshot.board.unidentified_items} task-file items need identity markers before they can be managed.</p><button type="button" disabled={busy || !connected} onClick={() => void mutateResult({ action: "tasks_assign_ids", root_id: root.run_id, expected_doc_revision: snapshot.board!.doc_revision })}>Identify task-file items</button></section> : null}
        </> : null}
      </div>
      {root ? <>
        <div className="supervisor-narrow-switch" aria-label="Workarea view"><button type="button" aria-pressed={narrowView === "board"} onClick={() => setNarrowView("board")}><UiIcon name="grid" />Board</button><button type="button" aria-pressed={narrowView === "agents"} onClick={() => setNarrowView("agents")}><UiIcon name="branch" />Agents</button></div>
        <div className={`supervisor-graph-surface${narrowView === "agents" ? " is-narrow-active" : ""}`} style={graphHeight === null ? undefined : { "--graph-height": `${graphHeight}px` } as CSSProperties}>
          <SupervisorGraph rows={forest} snapshot={snapshot} live={!!live} connected={connected} runtimeLive={runtimeLive} busy={busy} scope={scope} sharedSpace={sharedSpace} highlightedRun={hoverRun ?? detailRun?.run_id ?? null} changed={changed} onHover={run => { setHoverRun(run?.run_id ?? null); setHoverSpace(observe(run?.run_id)?.workspace_id ?? null); }} onSelect={(run, subagent) => { const selected = scope.selectedRun === run.run_id && scope.selectedSubagent === (subagent?.subagent_id ?? null); scope.selectedTask = null; scope.selectedRun = selected ? null : run.run_id; scope.selectedSubagent = selected ? null : subagent?.subagent_id ?? null; setDetailSection("overview"); changed(); }} onSelectTask={selectTask} onUnmanaged={paneId => void onUnmanagedTerminal(paneId).catch(cause => setTerminalError(`Could not open terminal. ${cause instanceof Error ? cause.message : "Check status."}`))} />
          <RowSplitter className="supervisor-graph-splitter" label="Resize agents overview" target={splitter => splitter.previousElementSibling as HTMLElement | null} grow={1} min={80} max={Math.round(window.innerHeight * 0.6)} onChange={setGraphHeight} onReset={() => setGraphHeight(null)} />
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
    <div className="supervisor-focus-notice" role="status">{focusNotice}</div>
    {dialog && snapshot && active ? <SupervisorDialogs dialog={dialog} snapshot={snapshot} spaces={runtimeLive ? session?.spaces ?? [] : []} startDraft={startDraft.current} changed={changed} busy={busy} available={connected && (dialog.mode === "edit" || dialog.mode === "close" || runtimeLive && snapshot.runtime.status === "fresh")} mutateResult={mutateResult} onStarted={started} onEdited={taskId => { scope.edits.delete(taskId); changed(); }} onStartUnconfirmed={() => setStartUnknown(true)} onClose={() => setDialog(null)} /> : null}
  </section>;
}
```

## SupervisorActions.tsx

```tsx
import { useId, useRef, useState } from "react";
import type { OrchestrationAction, OrchestrationActionResult, OrchestrationSnapshot, PlanRecord, Report, Run, RunObservation, Subagent, TaskView } from "../../protocol/generated/v1";
import { StateGlyph, type GlyphShape } from "../sidebar/StateGlyph";
import { UiIcon } from "../UiIcon";
import { messageDraft, type ScopeDrafts, type TextDraft } from "./useSupervisorDrafts";

export type Mutation = (action: OrchestrationAction) => Promise<OrchestrationActionResult | null>;
export type AgentState = { kind: "ready" | "starting" | "failure" | "missing" | "unknown" | "offline" | "closed"; label: string; detail: string; verified: boolean; blocked: boolean; terminal: boolean; restartable: boolean };
export function agentState(snapshot: OrchestrationSnapshot, run: Run, connected: boolean, runtimeLive: boolean): AgentState {
  const observed = snapshot.runtime.status === "fresh" && connected && runtimeLive ? snapshot.runtime.runs.find(item => item.run_id === run.run_id) : undefined;
  const terminal = observed?.presence === "present" && !!observed.pane_id;
  const restartable = !!run.dispatch && (["plan_failed", "setup_unknown"].includes(run.dispatch.step) || !!observed && observed.presence !== "unobserved" && !observed.actual_omp);
  const state = (kind: AgentState["kind"], label: string, detail: string, verified = false): AgentState => ({ kind, label, detail, verified, blocked: verified && observed?.agent_status === "blocked", terminal, restartable });
  if (run.stage === "closed") return state("closed", "Tracking closed", "Tasks and history are kept. The agent and its workers are not guaranteed to have stopped.");
  if (!connected || !runtimeLive) return state("offline", "Connection lost", "Showing saved tasks. The agent may still be running.");
  if (snapshot.runtime.status !== "fresh") return state("offline", "Cannot check the agent right now", "Showing saved tasks. Herdr observation is unavailable; no absence is inferred.");
  if (observed?.presence === "missing") return state("missing", "Agent terminal is gone", "The last observed terminal is no longer available. Tasks and history are saved.");
  if (observed?.presence === "endpoint_changed") return state("unknown", "Cannot confirm the agent", "The server identity changed. Check status before opening or restarting the agent.");
  if (run.stage === "proposed" || run.stage === "awaiting_prepare") return state("starting", "Waiting for supervisor", "The supervisor is reviewing preparation. No worker process has launched yet.");
  const step = run.dispatch?.step;
  if (step === "plan_failed" && run.dispatch?.error) return state("failure", "Agent did not start", run.dispatch.error.message);
  if (step === "setup_unknown") return state("unknown", "Agent setup is unconfirmed", `${run.dispatch?.error?.message ?? "Setup could not be confirmed."} Existing resources are kept. Review recovery before launching.`);
  if (["launch_unknown", "needs_review"].includes(step ?? "")) return state("unknown", "Agent start is unconfirmed", `${run.dispatch?.error?.message ? `${run.dispatch.error.message} ` : ""}Startup is not confirmed. Another launch could create a duplicate.`);
  const verified = !!observed?.actual_omp && observed.presence === "present" && !!run.bound_omp_session && (step === "launched" || run.kind === "adopted" && !run.dispatch);
  if (verified) {
    const board = snapshot.board?.root_id === run.root_id ? snapshot.board : null;
    const hasWork = !!board?.tasks.some(task => task.lane !== "accepted")
      || snapshot.runs.some(child => child.root_id === run.root_id && child.run_id !== run.run_id && child.stage !== "closed")
      || snapshot.assignment_intents.some(intent => intent.root_id === run.root_id)
      || run.last_report?.kind === "needs_input";
    const rootLabel = hasWork ? "Managing tasks" : board ? "Ready for a task" : "OMP connected";
    return state("ready", observed?.agent_status === "blocked" ? "Agent blocked" : run.kind === "worker" ? run.stage === "initializing" ? "Preparing · OMP connected" : "Agent connected" : rootLabel, "OMP is connected. The supervisor manages preparation, execution and review.", true);
  }
  if (run.stage === "preparing" || run.stage === "initializing" || step === "launch_pending" || step === "launch_intent") return state("starting", "Starting agent…", "Waiting for OMP to start and connect.");
  return state("unknown", "Cannot confirm the agent", "No fresh, bound OMP process is confirmed. Saved reports are not live process evidence.");
}
export function taskStatus(task: TaskView, run: Run | undefined, snapshot: OrchestrationSnapshot): string {
  const assignment = snapshot.assignment_intents.find(intent => intent.root_id === snapshot.board?.root_id && intent.task_id === task.task.task_id);
  if (assignment) return assignment.state === "conflict" ? "Not assigned · task changed elsewhere" : "Assignment pending";
  if (task.task.checked) return run?.result?.outcome === "succeeded" && run.close_reason === "accepted" && task.lane === "accepted" ? "Completed" : "Marked complete in task file";
  if (run?.stage === "closed") return run.close_reason === "failed" ? "Failed · tracking closed" : "Tracking closed";
  if (run?.last_report?.kind === "needs_input") return "Waiting for supervisor";
  if (run?.stage === "reported" || task.lane === "review") return "Reviewing result";
  if (task.lane === "working") return "Working";
  if (task.lane === "setup" || task.lane === "ready") return "Preparing";
  return run ? "Queued" : snapshot.messages.some(message => message.message_id === `assign-${task.task.task_id}` && message.to_run_id === snapshot.board?.root_id) ? "Assigned · waiting for agent" : "Queued · not assigned";
}
export function ReportedEvidence({ report, source, showBody = false }: { report: Report | null | undefined; source: string; showBody?: boolean }) {
  const minutes = report ? Math.max(0, Math.floor((Date.now() - Date.parse(report.at)) / 60_000)) : 0;
  const age = minutes < 1 ? "just now" : minutes < 60 ? `${minutes}m ago` : minutes < 1440 ? `${Math.floor(minutes / 60)}h ago` : `${Math.floor(minutes / 1440)}d ago`;
  return <p className="supervisor-evidence"><span>Reported · {source}</span> {report ? <>{showBody || report.outcome === "failed" ? <span className="supervisor-report-summary">{report.summary}</span> : null}<time dateTime={report.at} title={new Date(report.at).toLocaleString()}>{age}</time></> : <span>No progress reported yet</span>}</p>;
}
export function ObservedEvidence({ run, observed, snapshot, live }: { run: Run; observed: RunObservation | undefined; snapshot: OrchestrationSnapshot; live: boolean }) {
  if (run.stage === "closed") return null;
  const raw = live && snapshot.runtime.status === "fresh" ? observed?.presence === "present" ? observed.actual_omp ? observed.agent_status ?? "unknown" : "OMP not confirmed" : observed?.presence ?? "unobserved" : "unobserved";
  const shape: GlyphShape = raw === "working" || raw === "idle" || raw === "blocked" || raw === "done" ? raw : "unknown";
  return <p className="supervisor-evidence supervisor-observed"><span>Observed · Herdr</span><StateGlyph shape={shape} /><span>{raw.replaceAll("_", " ")}</span>{live && snapshot.runtime.status === "fresh" ? <time dateTime={snapshot.runtime.observed_at} title={new Date(snapshot.runtime.observed_at).toLocaleString()}>{new Date(snapshot.runtime.observed_at).toLocaleTimeString()}</time> : <span>Saved reports only</span>}</p>;
}
export function TextAction({ label, submitLabel, draft, changed, busy, submit, success, doneResult = false, retryable = true, describedBy }: {
  label: string; submitLabel: string; draft: TextDraft; changed(): void; busy: boolean;
  submit(text: string, id: string): Promise<OrchestrationActionResult | null>; success: string; doneResult?: boolean; retryable?: boolean; describedBy?: string;
}) {
  const id = useId();
  const errorId = useId();
  const inFlight = useRef(false);
  const [pending, setPending] = useState(false);
  const send = async () => {
    if (busy || inFlight.current || !draft.text.trim() || draft.operation && !retryable) return;
    draft.operation ??= { id: crypto.randomUUID(), text: draft.text };
    const submitted = draft.operation;
    draft.error = null; draft.notice = null; inFlight.current = true; setPending(true); changed();
    try {
      const result = await submit(submitted.text, submitted.id);
      if (result?.result === (doneResult ? "done" : "message")) {
        if (draft.text === submitted.text) draft.text = "";
        draft.operation = null; draft.notice = success;
      } else draft.error = retryable ? "Delivery was not confirmed. Your draft and operation are kept. Check status, then explicitly retry the same message." : "The action was not confirmed. Check its current receipt before requesting another action; repeating it could duplicate the previous request.";
    } catch { draft.error = "Could not send this message. Your draft is kept."; }
    finally { inFlight.current = false; setPending(false); changed(); }
  };
  return <form className="supervisor-action-form" aria-busy={pending} onSubmit={event => { event.preventDefault(); void send(); }}>
    <label htmlFor={id}>{label}</label>
    <textarea id={id} rows={3} value={draft.text} readOnly={pending || !!draft.operation} aria-describedby={[describedBy, draft.error ? errorId : undefined].filter(Boolean).join(" ") || undefined} onChange={event => { draft.text = event.target.value; draft.notice = null; changed(); }} />
    <div className="supervisor-action-row"><button type="submit" disabled={busy || pending || !draft.text.trim() || !!draft.operation && !retryable}>{pending ? "Sending…" : draft.operation ? retryable ? "Retry same message" : "Check previous action first" : submitLabel}</button></div>
    {draft.operation && !pending ? <p>{retryable ? "Resolve the previous message before editing this draft; retry keeps the same delivery identity." : "After checking the previous control receipt or history, explicitly unlock the retained draft only if a new request is still needed."}</p> : null}
    {draft.operation && !pending && !retryable ? <button type="button" disabled={busy} onClick={() => { draft.operation = null; draft.error = null; changed(); }}>I reviewed the previous action; unlock draft</button> : null}
    <div className={`supervisor-feedback${draft.error ? " is-error" : ""}`} id={errorId} role={draft.error ? "alert" : "status"}>{draft.error || draft.notice ? <><UiIcon name={draft.error ? "info" : "check"} /><span>{draft.error ?? draft.notice}</span></> : null}</div>
  </form>;
}
export function AgentRecovery({ run, state, busy, connected, runtimeLive, onCheck, onRestart, onCloseTracking, onTerminal }: {
  run: Run; state: AgentState; busy: boolean; connected: boolean; runtimeLive: boolean;
  onCheck(): void; onRestart(): void; onCloseTracking(): void; onTerminal(): void;
}) {
  const recovery = ["failure", "missing", "unknown"].includes(state.kind);
  return <section className={`supervisor-agent-status is-${state.kind}`} aria-label={`${run.label} status`}>
    <div className="supervisor-status-heading"><strong>{run.label}</strong><span className="supervisor-state"><StateGlyph shape={state.blocked ? "blocked" : state.verified ? "live" : "unknown"} tone={recovery ? "warning" : state.kind === "offline" || state.kind === "closed" ? "muted" : undefined} />{state.label}</span></div>
    {state.kind !== "ready" ? <p role={recovery ? "alert" : "status"}>{state.detail}</p> : null}
    <div className="supervisor-action-row">
      {state.terminal ? <button type="button" disabled={busy || !connected || !runtimeLive} onClick={onTerminal}><UiIcon name="terminal" />Open terminal</button> : null}
      {state.kind !== "ready" && state.kind !== "closed" ? <button type="button" disabled={busy} onClick={onCheck}>{state.kind === "offline" ? "Check connection" : "Check status"}</button> : null}
      {recovery && run.stage !== "reported" && run.dispatch ? <button type="button" disabled={busy || !connected || !runtimeLive || !state.restartable} onClick={onRestart}>{run.dispatch.step === "plan_failed" ? "Retry setup" : run.dispatch.step === "setup_unknown" ? "Recover setup…" : "Restart agent…"}</button> : null}
      {run.stage !== "closed" ? <button type="button" aria-label="Close tracking…" title="Close tracking…" disabled={busy || !connected} onClick={onCloseTracking}><UiIcon name="close" />Close tracking…</button> : null}
    </div>
    {run.stage === "reported" && recovery ? <p>The explicit result must be reviewed or sent back before restarting.</p> : recovery && !state.restartable ? <p>{run.dispatch ? "Restart requires a current terminal observation. Check status first." : "No supported launch receipt is available for restart. Start a new tracked agent; this agent's tasks and history stay here."}</p> : null}
  </section>;
}
function PlanOverride({ run, plan, scope, busy, mutateResult, note, changed }: { run: Run; plan: PlanRecord | null; scope: "prepare" | "execute"; busy: boolean; mutateResult: Mutation; note: TextDraft; changed(): void }) {
  const [reviewed, setReviewed] = useState(plan?.plan_revision ?? null);
  const [armed, setArmed] = useState<string | null>(null);
  if (!plan) return <p>No current plan is available.</p>;
  const stale = reviewed !== plan.plan_revision;
  return <section className="supervisor-section" onKeyDown={event => {
    if (event.key === "Escape" && armed && !event.nativeEvent.isComposing) { event.preventDefault(); event.stopPropagation(); setArmed(null); }
  }}><h4>{scope === "prepare" ? "Prepare" : "Execute"} override · exact plan</h4><pre className="supervisor-plan">{plan.text}</pre><p>This deliberate operator action replaces the supervisor's decision for this exact plan. Same-user policy, not an OS sandbox.</p>{scope === "execute" ? <label>Optional execution note<textarea value={note.text} onChange={event => { note.text = event.target.value; changed(); }} readOnly={busy} /></label> : null}{stale ? <><p className="supervisor-warning">The plan changed. Review its current text before overriding.</p><button disabled={busy} onClick={() => { setReviewed(plan.plan_revision); setArmed(null); }}>Reviewed current plan</button></> : armed ? <><p>Authorize this exact plan?</p><button disabled={busy || armed !== plan.plan_revision} onClick={() => { const action: OrchestrationAction = scope === "prepare" ? { action: "grant_prepare", run_id: run.run_id, plan_revision: armed } : { action: "grant_execute", run_id: run.run_id, plan_revision: armed, note: note.text || null }; void mutateResult(action).then(result => { setArmed(null); if (!result) setReviewed(null); }); }}>Confirm {scope} override</button><button disabled={busy} onClick={() => setArmed(null)}>Back</button></> : <button disabled={busy} onClick={() => setArmed(plan.plan_revision)}>Override {scope}…</button>}</section>;
}
export function RunDiagnostics({ run, snapshot }: { run: Run; snapshot: OrchestrationSnapshot }) {
  return <div className="supervisor-diagnostics">
    <h3>{run.label} · durable records</h3><p>Launch receipts and saved reports are not current process proof.</p>
    <section className="supervisor-detail-group"><h4>Run, binding, setup and exact plans</h4><pre className="supervisor-plan">{JSON.stringify(run, null, 2)}</pre></section>
    <section className="supervisor-detail-group"><h4>Inbox delivery and provenance</h4>{snapshot.messages.filter(message => message.to_run_id === run.run_id || message.from.type === "run" && message.from.run_id === run.run_id).map(message => <details key={message.message_id}><summary>{message.kind.replaceAll("_", " ")} · {message.stage}{message.stale ? " · stale" : ""} · {message.created_at}</summary><pre className="supervisor-plan">{JSON.stringify(message, null, 2)}</pre></details>)}</section>
  </div>;
}
function RequestStop({ task, supervisor, scope, changed, busy, mutateResult }: { task: TaskView; supervisor: Run; scope: ScopeDrafts; changed(): void; busy: boolean; mutateResult: Mutation }) {
  const [confirm, setConfirm] = useState(false);
  const inFlight = useRef(false);
  const draft = messageDraft(scope, `stop:${task.task.task_id}`);
  const request = async () => {
    if (busy || inFlight.current) return;
    draft.operation ??= { id: crypto.randomUUID(), text: `Please stop work on canonical task ${task.task.task_id}. Inspect its current task and descendant runs, cancel scoped work where appropriate, and report outstanding effects. Existing files and Spaces must stay.` };
    inFlight.current = true;
    try {
      const result = await mutateResult({ action: "message_send", message_id: draft.operation.id, to_run_id: supervisor.run_id, kind: "instruction", text: draft.operation.text });
      if (result?.result === "message" && result.to_run_id === supervisor.run_id) { draft.operation = null; draft.notice = "Stop requested. Waiting for the supervisor; this is not proof the process stopped."; draft.error = null; setConfirm(false); }
      else draft.error = "The stop request was not confirmed. Retry keeps the same message identity.";
    } finally { inFlight.current = false; changed(); }
  };
  return <section>{confirm ? <><p>Ask the supervisor to stop this task? Existing files and Spaces stay.</p><button type="button" disabled={busy} onClick={() => void request()}>{draft.operation ? "Retry same stop request" : "Request stop"}</button><button type="button" disabled={busy} onClick={() => setConfirm(false)}>Keep working</button></> : <button type="button" disabled={busy} onClick={() => setConfirm(true)}>Request stop…</button>}{draft.notice ? <p role="status">{draft.notice}</p> : null}{draft.error ? <p className="supervisor-error" role="alert">{draft.error}</p> : null}</section>;
}
export function SupervisorActions({ snapshot, run, task, subagent, section = "overview", scope, changed, busy, live, mutateResult, onTerminal, onEditTask, onCancelSubagent, onCloseTracking }: {
  snapshot: OrchestrationSnapshot; run: Run | null; task: TaskView | null; subagent?: Subagent | null; section?: "overview" | "activity" | "actions";
  scope: ScopeDrafts; changed(): void; busy: boolean; live: boolean; mutateResult: Mutation;
  onTerminal(run: Run): void; onEditTask(task: TaskView): void; onCancelSubagent(run: Run, subagent: Subagent): void; onCloseTracking(run: Run): void;
}) {
  const observed = snapshot.runtime.status === "fresh" && live && run ? snapshot.runtime.runs.find(item => item.run_id === run.run_id) : undefined;
  const canTerminal = observed?.presence === "present" && !!observed.pane_id;
  const controls = !!run && run.stage !== "closed" && live;
  const supervisor = snapshot.runs.find(root => root.run_id === snapshot.board?.root_id && root.stage === "active");
  const canRequestStop = supervisor && agentState(snapshot, supervisor, live, live).verified && task && !task.task.checked && (!run || run.stage !== "closed");
  const childDraft = run && subagent ? messageDraft(scope, `subagent:${run.run_id}:${subagent.subagent_id}`) : null;
  return <div className="supervisor-task-detail">
    <div className="supervisor-detail-section" hidden={section !== "overview"}>
      {task ? <section className="supervisor-detail-group"><h3>Task description</h3><p className="supervisor-exact-text">{task.task.body}</p>{task.task.diagnostic ? <p className="supervisor-error">{task.task.diagnostic}</p> : null}</section> : null}
      {run ? <section className="supervisor-detail-group">
        <h3>{subagent ? "Subagent" : "Agent"}</h3>
        <p>{subagent ? `${subagent.role ?? "OMP subagent"} · In ${run.label} · no terminal` : `${run.kind === "worker" ? "Task agent" : "Supervisor"} · ${snapshot.runs.find(parent => parent.run_id === run.parent_run_id)?.label ?? "Root agent"}`}</p>
        {subagent ? <>
          <p>OMP events · {subagent.status} · <time dateTime={subagent.updated_at}>{new Date(subagent.updated_at).toLocaleString()}</time></p>
          <p className="supervisor-exact-text">{subagent.summary ?? "No summary reported."}</p>
        </> : <>
          <ReportedEvidence report={run.last_report} source={run.label} showBody /><ObservedEvidence run={run} observed={observed} snapshot={snapshot} live={live} />
          {run.last_report?.kind === "needs_input" && run.kind === "worker" ? <p>Waiting for supervisor · {run.last_report.summary}</p> : null}
        </>}
        {canTerminal ? <button type="button" disabled={busy || !live} onClick={() => onTerminal(run)}>{subagent ? "Open parent terminal" : "Open terminal"}</button> : null}
      </section> : <p>No worker progress reported yet.</p>}
    </div>
    <div className="supervisor-detail-section" hidden={section !== "activity"}>
      {run ? subagent ? <section className="supervisor-detail-group">
        {subagent.last_control ? <><h3>Last control · {subagent.last_control.op.op} · {subagent.last_control.stage}</h3><p>A stored request is not applied control. Applied delivery is not task completion.</p>{subagent.last_control.op.op === "send" ? <p className="supervisor-exact-text">{subagent.last_control.op.text}</p> : null}{subagent.last_control.error ? <p className="supervisor-error">{subagent.last_control.error}</p> : null}<time dateTime={subagent.last_control.at}>{new Date(subagent.last_control.at).toLocaleString()}</time></> : <><h3>Last control</h3><p>No control receipt. No child terminal is assumed.</p></>}
      </section> : <>
        {run.result ? <section className="supervisor-detail-group"><h3>Result · {run.result.outcome ?? "reported"}</h3><p className="supervisor-exact-text">{run.result.summary}</p><p>Explicit report from {run.label} · <time dateTime={run.result.at}>{new Date(run.result.at).toLocaleString()}</time>{run.close_reason === "accepted" ? " · Accepted" : " · Awaiting review"}</p></section> : null}
        {run.work_plan ? <section className="supervisor-detail-group"><h3>Work plan</h3><p className="supervisor-exact-text">{run.work_plan.text}</p></section> : null}
        {run.init_receipt ? <section className="supervisor-detail-group"><h3>Initialization report</h3><p className="supervisor-exact-text">{run.init_receipt.summary}</p><p>Reported by {run.label} · {run.init_receipt.at}</p></section> : null}
      </> : null}
    </div>
    <div className="supervisor-detail-section" hidden={section !== "actions"}>
      {task ? <section className="supervisor-detail-group"><h3>Edit task</h3><button type="button" disabled={busy || !live || !!task.task.diagnostic} onClick={() => onEditTask(task)}>Edit task…</button></section> : null}
      {canRequestStop && task && supervisor ? <section className="supervisor-detail-group"><h3>Request stop</h3><RequestStop task={task} supervisor={supervisor} scope={scope} changed={changed} busy={busy || !live} mutateResult={mutateResult} /></section> : null}
      {run ? subagent && childDraft ? <>
        <section className="supervisor-detail-group"><h3>Message to subagent</h3><TextAction label="Message to subagent" submitLabel="Send message" draft={childDraft} changed={changed} busy={busy || !controls || subagent.status !== "running"} submit={text => mutateResult({ action: "subagent_control", run_id: run.run_id, subagent_id: subagent.subagent_id, op: { op: "send", text } })} success="Control request stored. Check its receipt for applied delivery." retryable={false} /></section>
        <section className="supervisor-detail-group"><h3>Cancel subagent</h3><button type="button" disabled={busy || !controls || subagent.status !== "running"} onClick={() => onCancelSubagent(run, subagent)}>Cancel subagent…</button></section>
      </> : <>
        <section className="supervisor-detail-group"><h3>Follow-up to agent</h3><TextAction label="Follow-up to agent" submitLabel="Send follow-up" draft={messageDraft(scope, `followup:${run.run_id}`)} changed={changed} busy={busy || !controls} submit={(text, message_id) => mutateResult({ action: "message_send", message_id, to_run_id: run.run_id, kind: "instruction", text })} success="Follow-up sent. Waiting for the agent." /></section>
        <section className="supervisor-detail-group"><h3>Durable note</h3><TextAction label="Durable note" submitLabel="Save note" draft={messageDraft(scope, `note:${run.run_id}`)} changed={changed} busy={busy || !live} submit={text => mutateResult({ action: "annotate", run_id: run.run_id, text })} success="Note saved." doneResult retryable={false} /></section>
        {run.stage === "reported" ? <section className="supervisor-detail-group"><h3>Operator result review override</h3><p>The supervisor normally reviews this result. Acceptance uses the exact current canonical task revision, not runtime Done.</p><button type="button" disabled={busy || !live || !task || !!task.task.diagnostic || task.current_run_id !== run.run_id} onClick={() => { if (task) void mutateResult({ action: "accept", run_id: run.run_id, expected_task_revision: task.task.task_revision }); }}>Accept explicit result</button><TextAction label="Requested changes" submitLabel="Send back" draft={messageDraft(scope, `sendback:${run.run_id}`)} changed={changed} busy={busy || !live} submit={text => mutateResult({ action: "send_back", run_id: run.run_id, text })} success="Changes sent back." doneResult retryable={false} /></section> : null}
        {run.stage !== "closed" ? <section className="supervisor-detail-group"><h3>Close agent tracking</h3><button type="button" disabled={busy || !live} onClick={() => onCloseTracking(run)}>Close agent tracking…</button></section> : null}
        {run.stage === "awaiting_prepare" || run.stage === "ready" ? <section className="supervisor-detail-group"><h3>Operator plan override</h3><p>The supervisor handles routine authorization. Use this only for deliberate intervention.</p><PlanOverride run={run} scope={run.stage === "awaiting_prepare" ? "prepare" : "execute"} plan={run.stage === "awaiting_prepare" ? run.prepare_plan : run.work_plan} busy={busy || !live} mutateResult={mutateResult} note={messageDraft(scope, `execute-note:${run.run_id}`)} changed={changed} /></section> : null}
      </> : null}
    </div>
  </div>;
}
```

## SupervisorDialogs.tsx

```tsx
import { useEffect, useId, useRef, useState, type KeyboardEvent } from "react";
import { createPortal } from "react-dom";
import type { DispatchTarget, OrchestrationAction, OrchestrationActionResult, OrchestrationSnapshot, Run, SpaceSummary, Subagent, TaskView } from "../../protocol/generated/v1";
import { useRestoreFocus } from "../library/LibraryConfirmDialog";
import { ErrorSlot } from "../ErrorSlot";
import { UiIcon } from "../UiIcon";
import type { EditDraft } from "./useSupervisorDrafts";
import "../projects/setup.css";

export type StartDraft = { label: string; location: "existing" | "directory" | "dedicated"; spaceId: string; directory: string };
export type SupervisorDialogState = { mode: "start" } | { mode: "edit"; task: TaskView; draft: EditDraft } | { mode: "retry" | "close" | "setup_recovery"; run: Run } | { mode: "subagent_cancel"; run: Run; subagent: Subagent };
export function SupervisorDialogs({ dialog, snapshot, spaces, startDraft, changed, busy, available, mutateResult, onStarted, onEdited, onStartUnconfirmed, onClose }: {
  dialog: SupervisorDialogState; snapshot: OrchestrationSnapshot; spaces: SpaceSummary[]; startDraft: StartDraft;
  changed(): void; busy: boolean; available: boolean; mutateResult(action: OrchestrationAction): Promise<OrchestrationActionResult | null>;
  onStarted(runId: string): void; onEdited(taskId: string): void; onStartUnconfirmed(): void; onClose(): void;
}) {
  const titleId = useId();
  const formId = useId();
  const ref = useRef<HTMLElement>(null);
  const inFlight = useRef(false);
  const [pending, setPending] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [unconfirmed, setUnconfirmed] = useState(false);
  const locked = busy || pending;
  const mode = dialog.mode;
  const title = mode === "start" ? "Start agent" : mode === "edit" ? "Edit task" : mode === "retry" ? "Restart agent" : mode === "close" ? "Close tracking" : mode === "setup_recovery" ? "Recover setup" : "Cancel subagent";
  useRestoreFocus();
  useEffect(() => {
    ref.current?.querySelector<HTMLElement>(mode === "start" && startDraft.location === "existing" && spaces.some(space => space.id === startDraft.spaceId) ? "[data-primary]" : "[data-initial]")?.focus({ preventScroll: true });
  }, []); // Opening owns focus once; polling must not move it.
  const close = () => { if (!inFlight.current && !busy) onClose(); };
  const keys = (event: KeyboardEvent<HTMLElement>) => {
    if (event.nativeEvent.isComposing) return;
    if (event.key === "Escape") { event.preventDefault(); event.stopPropagation(); close(); return; }
    if (event.key !== "Tab") return;
    event.stopPropagation();
    const elements = [...event.currentTarget.querySelectorAll<HTMLElement>("button,input,textarea,select,[tabindex]:not([tabindex='-1'])")].filter(element => !element.matches(":disabled,[hidden]") && !element.closest("[hidden],[inert]"));
    const index = elements.indexOf(document.activeElement as HTMLElement);
    if (event.shiftKey && index <= 0) { event.preventDefault(); elements.at(-1)?.focus(); }
    else if (!event.shiftKey && (index < 0 || index === elements.length - 1)) { event.preventDefault(); elements[0]?.focus(); }
  };
  const submit = async () => {
    if (locked || inFlight.current || unconfirmed && dialog.mode !== "edit") return;
    if (!available) { setError("The required connection is unavailable. Your draft and resources are kept."); return; }
    let action: OrchestrationAction;
    if (dialog.mode === "start") {
      let target: DispatchTarget | null = null;
      if (startDraft.location === "existing") {
        if (!spaces.some(space => space.id === startDraft.spaceId)) { setError("Choose a currently available Space. No different destination will be selected automatically."); return; }
        target = { target: "existing_space", workspace_id: startDraft.spaceId };
      } else if (startDraft.location === "directory") {
        if (!startDraft.directory.trim().startsWith("/")) { setError("Enter an absolute directory path."); return; }
        target = { target: "setup", request: { operation: "open", path: startDraft.directory.trim(), label: startDraft.label.trim() || null, task_name: null, focus: false } };
      }
      action = { action: "supervisor_start", target, label: startDraft.label.trim() || null };
    } else if (dialog.mode === "edit") {
      if (!dialog.draft.title.trim()) { setError("Enter a task title."); return; }
      if (!snapshot.board) { setError("The task document is unavailable. Your edit draft is kept."); return; }
      action = { action: "task_update", root_id: snapshot.board.root_id, task_id: dialog.task.task.task_id, expected_task_revision: dialog.draft.revision, title: dialog.draft.title, body: dialog.draft.body };
    } else if (dialog.mode === "subagent_cancel") action = { action: "subagent_control", run_id: dialog.run.run_id, subagent_id: dialog.subagent.subagent_id, op: { op: "cancel" } };
    else if (dialog.mode === "setup_recovery") action = { action: "reconcile_run", run_id: dialog.run.run_id, recovery: dialog.run.dispatch?.recovery ?? null };
    else action = { action: dialog.mode === "retry" ? "retry_launch" : "cancel_run", run_id: dialog.run.run_id };
    inFlight.current = true; setPending(true); setError(null);
    try {
      const result = await mutateResult(action);
      const confirmed = result && (dialog.mode === "start" ? result.result === "run" : dialog.mode === "edit" ? result.result === "task" : result.result === "done" || result.result === "message");
      if (!result || !confirmed) {
        if (dialog.mode !== "edit") setUnconfirmed(true);
        if (dialog.mode === "start") onStartUnconfirmed();
        setError(dialog.mode === "start" ? "The start was not confirmed. Close these options and check status before another start; the previous request may have opened a terminal." : "The change was not confirmed. Your draft and tracking are kept. Check current status before requesting another action.");
        return;
      }
      if (dialog.mode === "start" && result.result === "run") onStarted(result.run_id);
      if (dialog.mode === "edit" && result.result === "task") onEdited(dialog.task.task.task_id);
      onClose();
    } catch (cause) { setError(cause instanceof Error ? cause.message : "The change was not confirmed. Your draft is kept."); }
    finally { inFlight.current = false; setPending(false); }
  };
  const dialogRun = "run" in dialog ? dialog.run : null;
  const editedTaskId = dialog.mode === "edit" ? dialog.task.task.task_id : null;
  const descendants = dialogRun ? snapshot.runs.filter(run => run.root_id === dialogRun.root_id && run.run_id !== dialogRun.run_id && run.stage !== "closed") : [];
  const currentTask = editedTaskId ? snapshot.board?.tasks.find(task => task.task.task_id === editedTaskId) : null;
  return createPortal(<div className="setup-overlay" role="presentation"><section ref={ref} className="supervisor-dialog" role="dialog" aria-modal="true" aria-labelledby={titleId} aria-busy={locked} tabIndex={-1} onKeyDown={keys}>
    <header className="supervisor-dialog-header"><span className="supervisor-dialog-icon"><UiIcon name={mode === "start" ? "plus" : mode === "edit" ? "edit" : mode === "retry" || mode === "setup_recovery" ? "refresh" : mode === "close" ? "close" : "stop"} /></span><h2 id={titleId}>{title}</h2></header><form id={formId} onSubmit={event => { event.preventDefault(); void submit(); }}>
      {dialog.mode === "start" ? <>
        <p>An OMP supervisor manages tasks and delegates work for you.</p>
        <label>Name (optional)<input value={startDraft.label} disabled={locked} onChange={event => { startDraft.label = event.target.value; changed(); }} /></label>
        <label>Location<select data-initial value={startDraft.location} disabled={locked} onChange={event => { startDraft.location = event.target.value as StartDraft["location"]; changed(); }}><option value="existing">Existing Space</option><option value="directory">Directory</option><option value="dedicated">Dedicated agent folder</option></select></label>
        {startDraft.location === "existing" ? <label>Space<select value={startDraft.spaceId} disabled={locked} onChange={event => { startDraft.spaceId = event.target.value; changed(); }}><option value="">Choose a Space</option>{spaces.map(space => <option key={space.id} value={space.id}>{space.label}</option>)}</select></label> : startDraft.location === "directory" ? <label>Absolute directory<input value={startDraft.directory} disabled={locked} onChange={event => { startDraft.directory = event.target.value; changed(); }} /></label> : <p>Creates a dedicated agent folder and Space. Existing project context is not copied.</p>}
        <p>Starts OMP without switching your terminal focus.</p>
      </> : dialog.mode === "edit" ? <>
        <label>Task title<input data-initial value={dialog.draft.title} disabled={locked} onChange={event => { dialog.draft.title = event.target.value; changed(); }} /></label>
        <label>Task description<textarea rows={8} value={dialog.draft.body} disabled={locked} onChange={event => { dialog.draft.body = event.target.value; changed(); }} /></label>
        {currentTask?.task.task_revision !== dialog.draft.revision ? <><p className="supervisor-warning">Task changed elsewhere. Saving will not overwrite those changes without your review. Your draft is kept.</p>{currentTask ? <details><summary>Review current task</summary><h3>{currentTask.task.title}</h3><p className="supervisor-exact-text">{currentTask.task.body}</p><p>Keeping your draft for the next save replaces the current task text only if this reviewed version is still current.</p><button type="button" disabled={locked || !!currentTask.task.diagnostic} onClick={() => { dialog.draft.revision = currentTask.task.task_revision; changed(); }}>I reviewed the current task; keep my draft for saving</button></details> : null}</> : null}
      </> : dialog.mode === "retry" ? <>
        <p>Restart {dialog.run.label}?</p><p>We cannot confirm whether the previous agent is still running. Restarting creates a new terminal and could leave another agent running. The previous launch is checked again before a new launch.</p>{descendants.length ? <p>Worker agents may still be running. Restart only this agent; keep their tasks and history.</p> : null}
      </> : dialog.mode === "close" ? <>
        <p>Close tracking for {dialog.run.label}?</p><p>Tasks, history, Spaces and worktrees are kept. This does not guarantee the agent or its workers stop.</p>{descendants.length ? <p>{descendants.length} other tracked agents stay open. Live descendants will still need supervision.</p> : null}
      </> : dialog.mode === "subagent_cancel" ? <p>Request cancellation of {dialog.subagent.label}. Its OMP control receipt, not this request, confirms whether it stopped.</p> : <>
        <p>Setup for {dialog.run.label} needs review before launch. Existing resources are kept.</p><p>{dialog.run.dispatch?.recovery === "accept_existing_worktree" ? "Confirm using the existing worktree receipt rather than creating another worktree." : dialog.run.dispatch?.recovery === "retry_environment" ? "Explicitly retry the uncertain setup operation after reviewing its recorded effects." : "Check and reconcile the existing setup only. No new launch is requested by this confirmation."}</p>
        {dialog.run.prepare_plan ? <details><summary>Exact setup plan</summary><p className="supervisor-exact-text">{dialog.run.prepare_plan.text}</p></details> : null}{dialog.run.setup ? <><p>{dialog.run.setup.checkout_path}</p><ul>{dialog.run.setup.effects.map((effect, index) => <li key={index}>{effect}</li>)}</ul>{dialog.run.setup.warnings.map((warning, index) => <p className="supervisor-warning" key={index}>{warning}</p>)}</> : null}
      </>}
    </form>
    <ErrorSlot placement="dialog" message={error} className="supervisor-dialog-error-slot" />
    {!available ? <p className="supervisor-dialog-unavailable">Reconnect before applying this change. You can keep editing or close this dialog without losing your draft.</p> : null}<footer><button type="button" data-initial={mode !== "start" && mode !== "edit" || undefined} disabled={locked} onClick={close}>{mode === "close" ? "Keep tracking" : mode === "retry" ? "Back" : "Cancel"}</button><button type="submit" form={formId} data-primary disabled={locked || !available || unconfirmed && mode !== "edit"}>{pending ? "Working…" : mode === "retry" ? "Restart anyway" : mode === "edit" ? "Save task" : title}</button></footer>
  </section></div>, document.body);
}
```

## SupervisorGraph.tsx

```tsx
import { useId, useMemo } from "react";
import type { OrchestrationSnapshot, Run, Subagent } from "../../protocol/generated/v1";
import { StateGlyph, type GlyphShape } from "../sidebar/StateGlyph";
import { useRovingList } from "../sidebar/useRovingList";
import { UiIcon } from "../UiIcon";
import { agentState } from "./SupervisorActions";
import { graphLayout } from "./graphLayout";
import type { ScopeDrafts } from "./useSupervisorDrafts";

export type SupervisorGraphRow = { key: string; depth: number; run: Run; subagent: Subagent | null };
export type SupervisorGraphProps = {
  rows: SupervisorGraphRow[];
  snapshot: OrchestrationSnapshot;
  live: boolean;
  connected: boolean;
  runtimeLive: boolean;
  busy: boolean;
  scope: ScopeDrafts;
  sharedSpace: string | null | undefined;
  highlightedRun: string | null;
  onHover(run: Run | null): void;
  onSelect(run: Run, subagent: Subagent | null): void;
  onSelectTask?(taskId: string): void;
  onUnmanaged(paneId: string): void;
  changed(): void;
};

export function SupervisorGraph({ rows, snapshot, live, connected, runtimeLive, busy, scope, sharedSpace, highlightedRun, onHover, onSelect, onSelectTask, onUnmanaged, changed }: SupervisorGraphProps) {
  const headingId = useId();
  const fresh = live && connected && runtimeLive && snapshot.runtime.status === "fresh";
  const observations = fresh && snapshot.runtime.status === "fresh" ? snapshot.runtime.runs : [];
  const unmanaged = fresh && scope.showUnmanaged ? snapshot.unmanaged_agents : [];
  const visibleRows = useMemo(() => scope.showSubagents ? rows : rows.filter(row => !row.subagent), [rows, scope.showSubagents]);
  const graph = useMemo(() => {
    const nodes = visibleRows.map(row => ({
      id: row.key,
      parentId: row.subagent
        ? row.subagent.parent_subagent_id ? `${row.run.run_id}:${row.subagent.parent_subagent_id}` : row.run.run_id
        : row.run.parent_run_id,
    }));
    const taskRefs = visibleRows.flatMap(row => {
      if (row.subagent || row.run.kind !== "worker" || snapshot.board?.root_id !== row.run.root_id) return [];
      const task = snapshot.board.tasks.find(task => task.task.task_id === row.run.task_id);
      return task ? [{ id: `task:${row.run.run_id}:${task.task.task_id}`, run: row.run, task: task.task }] : [];
    });
    const layout = graphLayout(nodes);
    const positionsById = new Map(layout.positions.map(position => [position.id, position]));
    return { ...layout, taskRefs, positionsById };
  }, [visibleRows, snapshot.board]);
  const selectedId = scope.selectedRun ? scope.selectedSubagent ? `${scope.selectedRun}:${scope.selectedSubagent}` : scope.selectedRun : null;
  const highlightedIds = new Set<string>();
  if (selectedId) highlightedIds.add(selectedId);
  if (highlightedRun) highlightedIds.add(highlightedRun);
  for (const reference of graph.taskRefs) if (reference.task.task_id === scope.selectedTask) {
    highlightedIds.add(reference.id);
    highlightedIds.add(reference.run.run_id);
  }
  const highlightedEdges = new Set<string>();
  for (const id of highlightedIds) {
    let current = id;
    const visited = new Set<string>();
    while (!visited.has(current)) {
      visited.add(current);
      const edge = graph.edges.find(edge => edge.to === current);
      if (!edge) break;
      highlightedEdges.add(`${edge.from}:${edge.to}`);
      current = edge.from;
    }
  }
  const roving = useRovingList({
    rowIds: [...visibleRows.map(row => row.key), ...(onSelectTask ? graph.taskRefs.map(reference => reference.id) : []), ...unmanaged.map(agent => `other:${agent.pane_id}`)],
    selectedId,
    onEscape: () => {
      scope.selectedRun = null;
      scope.selectedSubagent = null;
      scope.selectedTask = null;
      changed();
      return true;
    },
  });
  const connectedCount = visibleRows.filter(row => !row.subagent && observations.some(observed => observed.run_id === row.run.run_id && observed.actual_omp && observed.presence === "present")).length;

  return <section className="supervisor-graph-band" aria-labelledby={headingId}>
    <div className="supervisor-graph-heading">
      <h2 id={headingId}>Agents · {fresh ? `${connectedCount} connected` : "unobserved"}</h2>
      <label className="supervisor-graph-subagents"><input type="checkbox" checked={scope.showSubagents} disabled={!rows.some(row => !row.subagent)} onChange={event => {
        scope.showSubagents = event.target.checked;
        if (!scope.showSubagents && scope.selectedSubagent) {
          scope.selectedSubagent = null;
          const parent = scope.selectedRun;
          requestAnimationFrame(() => roving.focusRow(parent ?? undefined));
        }
        changed();
      }} />Subagents</label>
      <label className="supervisor-graph-subagents"><input type="checkbox" checked={scope.showUnmanaged} onChange={event => { scope.showUnmanaged = event.target.checked; changed(); }} />Other agents</label>
    </div>
    <div className="supervisor-graph-scroll" ref={roving.listRef} {...roving.listProps}>
      {visibleRows.length ? <div className="supervisor-graph-canvas" role="group" aria-label="Agent relationships" style={{ width: graph.width, height: graph.height }}>
        <svg className="supervisor-graph-edges" width={graph.width} height={graph.height} viewBox={`0 0 ${graph.width} ${graph.height}`} aria-hidden="true" focusable="false">
          {graph.edges.map(edge => <path key={`${edge.from}:${edge.to}`} className={`supervisor-graph-edge${highlightedEdges.has(`${edge.from}:${edge.to}`) ? " is-highlighted" : ""}`} d={edge.path} fill="none" stroke="var(--border-strong)" />)}
        </svg>
        {visibleRows.map(row => {
          const position = graph.positionsById.get(row.key)!;
          const observed = observations.find(observation => observation.run_id === row.run.run_id);
          const state = agentState(snapshot, row.run, connected, runtimeLive);
          const rawStatus = row.subagent ? row.subagent.status : fresh && state.verified ? observed?.agent_status ?? "unknown" : "unobserved";
          const shape: GlyphShape = row.subagent
            ? rawStatus === "running" ? "working" : rawStatus === "done" ? "done" : rawStatus === "failed" ? "blocked" : "unknown"
            : rawStatus === "working" || rawStatus === "idle" || rawStatus === "blocked" || rawStatus === "done" ? rawStatus : "unknown";
          const selected = selectedId === row.key;
          const label = row.subagent?.label ?? row.run.label;
          const role = row.subagent?.role ?? (row.subagent ? "OMP subagent" : row.run.kind === "worker" ? "Task agent" : "Supervisor");
          const space = observed?.presence === "present" ? observed.workspace_label : null;
          const evidence = `${row.subagent ? "OMP events" : "Herdr"} · ${rawStatus.replaceAll("_", " ")}`;
          return <button key={row.key} type="button" data-row-id={row.key} tabIndex={roving.tabIndexFor(row.key)} aria-expanded={selected} aria-label={`${label}, ${role}, ${evidence}${space ? `, Space ${space}` : ""}`} title={row.subagent ? `${evidence} · ${row.subagent.updated_at}${row.subagent.summary ? `\n${row.subagent.summary}` : ""}` : `${state.label}\n${state.detail}`} className={`supervisor-graph-node${selected ? " is-selected" : ""}${highlightedIds.has(row.key) ? " is-highlighted" : ""}${row.subagent ? " is-subagent" : ""}`} style={{ left: position.x, top: position.y, width: position.width, height: position.height }} onMouseEnter={() => onHover(row.run)} onMouseLeave={() => onHover(null)} onClick={() => onSelect(row.run, row.subagent)}>
            <span className="supervisor-graph-node-heading"><StateGlyph shape={shape} /><strong>{label}</strong></span>
            <span className={`supervisor-graph-node-metadata${space && observed?.workspace_id === sharedSpace ? " is-shared" : ""}`} title={`${role} · ${evidence} · ${space ? `Space ${space}${observed?.tab_label ? ` · ${observed.tab_label}` : ""}` : row.subagent ? `In ${row.run.label} · no terminal` : "Space unobserved"}`}>{row.subagent ? role : row.run.kind === "worker" ? "Worker" : "Supervisor"} · {row.subagent ? "OMP " : ""}{rawStatus.replaceAll("_", " ")} · {space ?? (row.subagent ? "no terminal" : "unobserved")}</span>
          </button>;
        })}
      </div> : <p className="supervisor-graph-empty">No agents in this task scope.</p>}
      {graph.taskRefs.length ? <div className="supervisor-graph-task-links" role="group" aria-label="Worker task references">{graph.taskRefs.map(reference => {
        const className = `supervisor-graph-task${highlightedIds.has(reference.id) || highlightedIds.has(reference.run.run_id) ? " is-highlighted" : ""}${scope.selectedTask === reference.task.task_id ? " is-selected" : ""}`;
        const title = `${reference.run.label} → Task · ${reference.task.title}`;
        const content = <><UiIcon name="file" /><span>{title}</span></>;
        return onSelectTask ? <button key={reference.id} type="button" className={className} data-row-id={reference.id} tabIndex={roving.tabIndexFor(reference.id)} title={title} aria-label={`View task ${reference.task.title} assigned to ${reference.run.label}`} onMouseEnter={() => onHover(reference.run)} onMouseLeave={() => onHover(null)} onClick={() => onSelectTask(reference.task.task_id)}>{content}</button> : <span key={reference.id} className={className} title={title}>{content}</span>;
      })}</div> : null}
      <section className="supervisor-graph-unmanaged" aria-label="Unmanaged agents">
        <h3>Other agents <span>Unmanaged · no task relationships</span></h3>
        {unmanaged.length ? <div className="supervisor-graph-unmanaged-nodes">{unmanaged.map(agent => {
          const raw = agent.agent_status ?? "unknown";
          const shape: GlyphShape = raw === "working" || raw === "idle" || raw === "blocked" || raw === "done" ? raw : "unknown";
          return <button key={agent.pane_id} type="button" className="supervisor-graph-unmanaged-node" data-row-id={`other:${agent.pane_id}`} tabIndex={roving.tabIndexFor(`other:${agent.pane_id}`)} disabled={busy} onClick={() => onUnmanaged(agent.pane_id)} aria-label={`Open ${agent.agent_name} terminal, Space ${agent.workspace_label}, Observed Herdr: ${raw}`} title={`${agent.workspace_label} · ${agent.tab_label} · Observed Herdr: ${raw}`}>
            <span className="supervisor-graph-node-heading"><StateGlyph shape={shape} /><strong>{agent.agent_name}</strong></span>
            <span className="supervisor-graph-node-space">Space · {agent.workspace_label}</span>
            <span className="supervisor-graph-unmanaged-open" aria-hidden="true"><UiIcon name="terminal" /></span>
          </button>;
        })}</div> : <p className="supervisor-graph-empty">{fresh ? "No other agents observed." : "Other agents are unobserved while the connection is unavailable."}</p>}
      </section>
    </div>
  </section>;
}
```

## graphLayout.ts

```tsx
export type GraphNode = { id: string; parentId: string | null };
export type GraphPosition = { id: string; x: number; y: number; width: number; height: number };
export type GraphEdge = { from: string; to: string; path: string };
export type GraphLayout = { positions: GraphPosition[]; edges: GraphEdge[]; width: number; height: number };

const CARD_WIDTH = 180;
const CARD_HEIGHT = 36;
const COLUMN_GAP = 36;
const ROW_GAP = 8;
const PADDING = 8;
const compareIds = (a: string, b: string) => a < b ? -1 : a > b ? 1 : 0;

/** A deterministic forest layout. Missing parents stay roots; cycles lose one edge, never gain one. */
export function graphLayout(nodes: readonly GraphNode[]): GraphLayout {
  if (!nodes.length) return { positions: [], edges: [], width: 0, height: 0 };
  const ordered = [...nodes].sort((a, b) => compareIds(a.id, b.id));
  const ids = new Set(ordered.map(node => node.id));
  const parents = new Map<string, string | null>();
  for (const node of ordered) {
    parents.set(node.id, node.parentId !== node.id && node.parentId !== null && ids.has(node.parentId) ? node.parentId : null);
  }

  // Follow parent chains iteratively so even a deep or cyclic snapshot cannot exhaust the stack.
  const settled = new Set<string>();
  for (const id of parents.keys()) {
    const chain: string[] = [];
    const indices = new Map<string, number>();
    let cursor: string | null = id;
    while (cursor !== null && !settled.has(cursor)) {
      const cycleStart = indices.get(cursor);
      if (cycleStart !== undefined) {
        let first = chain[cycleStart];
        for (let index = cycleStart + 1; index < chain.length; index++) {
          if (compareIds(chain[index], first) < 0) first = chain[index];
        }
        parents.set(first, null);
        break;
      }
      indices.set(cursor, chain.length);
      chain.push(cursor);
      cursor = parents.get(cursor) ?? null;
    }
    for (const member of chain) settled.add(member);
  }

  const children = new Map<string, string[]>();
  const roots: string[] = [];
  for (const [id, parent] of parents) {
    if (parent === null) roots.push(id);
    else {
      const siblings = children.get(parent);
      if (siblings) siblings.push(id);
      else children.set(parent, [id]);
    }
  }
  const traversal: { id: string; depth: number }[] = [];
  const pending = roots.map(id => ({ id, depth: 0 })).reverse();
  while (pending.length) {
    const node = pending.pop()!;
    traversal.push(node);
    const descendants = children.get(node.id) ?? [];
    for (let index = descendants.length - 1; index >= 0; index--) pending.push({ id: descendants[index], depth: node.depth + 1 });
  }
  const spans = new Map<string, number>();
  for (let index = traversal.length - 1; index >= 0; index--) {
    const descendants = children.get(traversal[index].id);
    const span = descendants ? descendants.reduce((total, id) => total + spans.get(id)!, 0) : CARD_HEIGHT + ROW_GAP;
    spans.set(traversal[index].id, span);
  }

  const tops = new Map<string, number>();
  let rootTop = PADDING;
  for (const id of roots) {
    tops.set(id, rootTop);
    rootTop += spans.get(id)!;
  }
  const byId = new Map<string, GraphPosition>();
  let width = 0;
  let height = 0;
  for (const { id, depth } of traversal) {
    const top = tops.get(id)!;
    const position = { id, x: PADDING + depth * (CARD_WIDTH + COLUMN_GAP), y: top + (spans.get(id)! - ROW_GAP - CARD_HEIGHT) / 2, width: CARD_WIDTH, height: CARD_HEIGHT };
    byId.set(id, position);
    width = Math.max(width, position.x + CARD_WIDTH + PADDING);
    height = Math.max(height, position.y + CARD_HEIGHT + PADDING);
    let childTop = top;
    for (const child of children.get(id) ?? []) {
      tops.set(child, childTop);
      childTop += spans.get(child)!;
    }
  }
  const edges: GraphEdge[] = [];
  for (const [id, parent] of parents) {
    if (parent === null) continue;
    const from = byId.get(parent)!;
    const to = byId.get(id)!;
    const startX = from.x + from.width;
    const startY = from.y + from.height / 2;
    const endY = to.y + to.height / 2;
    const bendX = (startX + to.x) / 2;
    edges.push({ from: parent, to: id, path: `M ${startX} ${startY} C ${bendX} ${startY}, ${bendX} ${endY}, ${to.x} ${endY}` });
  }
  return { positions: [...parents.keys()].map(id => byId.get(id)!), edges, width, height };
}
```

## RowSplitter.tsx

```tsx
import { useRef, type KeyboardEvent, type PointerEvent } from "react";

interface Props {
  label: string;
  /** Element whose height this splitter controls; measured when a drag or key press starts. */
  target: (splitter: HTMLElement) => HTMLElement | null;
  /** +1 when dragging down grows the target (splitter below it), -1 when dragging up grows it (splitter above it). */
  grow: 1 | -1;
  min: number;
  max: number;
  onChange: (height: number) => void;
  onReset: () => void;
  className?: string;
}

/** Full-width horizontal divider, the same grab-anywhere-on-the-border resize as pane splits. */
export function RowSplitter({ label, target, grow, min, max, onChange, onReset, className }: Props) {
  const drag = useRef<{ pointer: number; y: number; height: number } | null>(null);
  const clamp = (height: number) => Math.round(Math.min(max, Math.max(min, height)));
  const measure = (splitter: HTMLElement) => target(splitter)?.getBoundingClientRect().height ?? min;
  const onPointerDown = (event: PointerEvent<HTMLDivElement>) => {
    if (event.button !== 0) return;
    event.preventDefault();
    event.currentTarget.setPointerCapture(event.pointerId);
    drag.current = { pointer: event.pointerId, y: event.clientY, height: measure(event.currentTarget) };
    document.body.classList.add("is-resizing-panes");
  };
  const onPointerMove = (event: PointerEvent<HTMLDivElement>) => {
    const current = drag.current;
    if (!current || current.pointer !== event.pointerId) return;
    onChange(clamp(current.height + (event.clientY - current.y) * grow));
  };
  const end = (event: PointerEvent<HTMLDivElement>) => {
    if (drag.current?.pointer !== event.pointerId) return;
    drag.current = null;
    document.body.classList.remove("is-resizing-panes");
  };
  const onKeyDown = (event: KeyboardEvent<HTMLDivElement>) => {
    const step = event.shiftKey ? 48 : 16;
    if (event.key === "ArrowUp" || event.key === "ArrowDown") {
      event.preventDefault();
      const direction = (event.key === "ArrowDown" ? 1 : -1) * grow;
      onChange(clamp(measure(event.currentTarget) + direction * step));
    } else if (event.key === "Home") {
      event.preventDefault();
      onReset();
    }
  };
  return <div className={`supervisor-row-splitter ${className ?? ""}`} role="separator" aria-orientation="horizontal" aria-label={label} title="Drag to resize · double-click to reset"
    tabIndex={0} onPointerDown={onPointerDown} onPointerMove={onPointerMove} onPointerUp={end} onPointerCancel={end} onLostPointerCapture={end}
    onDoubleClick={onReset} onKeyDown={onKeyDown} />;
}
```

## supervisor.css

```css
.supervisor-view { container-type: inline-size; position: relative; display: flex; flex: 1 1 0; flex-direction: column; min-width: 0; min-height: 0; overflow: hidden; background: var(--surface); color: var(--text-primary); font: var(--font-size-sm)/var(--line-height-normal) var(--font-sans); }
.supervisor-view[hidden], .supervisor-view [hidden] { display: none !important; }
.supervisor-view .ui-icon, .supervisor-dialog .ui-icon { width: var(--icon-size); height: var(--icon-size); flex: 0 0 var(--icon-size); }
.supervisor-view .sb-glyph { width: var(--icon-badge); height: var(--icon-badge); flex: 0 0 var(--icon-badge); }
.supervisor-header { display: flex; align-items: center; gap: 6px; padding: 7px 12px; min-height: var(--tab-strip-height); box-sizing: border-box; flex-wrap: wrap; border-bottom: 1px solid var(--border); background: var(--chrome-bg); }
.supervisor-brand, .supervisor-header-state { display: inline-flex; align-items: center; gap: 7px; min-width: 0; }
.supervisor-brand strong { font-weight: 500; }
.supervisor-header-state { color: var(--text-secondary); font-size: var(--font-size-control); }
.supervisor-agent-label { min-width: 0; }
.supervisor-agent-label select { max-width: 170px; }
.supervisor-header-panels { display: flex; gap: 2px; margin-left: auto; }
.supervisor-header button, .supervisor-header select { min-height: var(--compact-control-size); height: var(--compact-control-size); }
.supervisor-header-panels button, .supervisor-secondary-button, .supervisor-header > button:last-child { width: var(--compact-control-size); padding: 0; color: var(--text-secondary); background: transparent; border-color: transparent; }
.supervisor-header-panels button:hover:not(:disabled), .supervisor-secondary-button:hover:not(:disabled), .supervisor-header > button:last-child:hover:not(:disabled) { background: var(--surface-hover); color: var(--text-primary); }
.supervisor-header .ui-icon { width: var(--icon-size); height: var(--icon-size); flex: 0 0 var(--icon-size); }
.supervisor-header .supervisor-primary { width: auto; padding: 0 10px; font-weight: 500; }
.supervisor-header-panels button[aria-pressed=true] { background: var(--select-fill); color: var(--accent); border-color: transparent; box-shadow: inset 0 var(--select-edge-size) 0 var(--select-edge); }
.supervisor-workarea { position: relative; display: flex; flex: 1 1 0; min-height: 0; min-width: 0; }
.supervisor-content { display: flex; flex-direction: column; flex: 1 1 0; min-height: 0; min-width: 0; overflow: hidden; }
.supervisor-view h2, .supervisor-view h3, .supervisor-view h4 { font-size: var(--font-size-sm); font-weight: 500; margin: 8px 0; }
.supervisor-view p { margin-block: 6px; }
:where(.supervisor-view, .supervisor-dialog) :where(button) { display: inline-flex; align-items: center; justify-content: center; gap: 6px; min-height: 30px; padding: 4px 9px; border: 1px solid var(--border); border-radius: var(--radius-control); background: var(--surface-raised); color: var(--text-primary); font: var(--font-size-control)/var(--line-height-normal) var(--font-sans); cursor: pointer; max-width: 100%; box-sizing: border-box; }
:where(.supervisor-view, .supervisor-dialog) :where(button):hover:not(:disabled) { background: var(--surface-hover); border-color: var(--border-strong); }
:where(.supervisor-view, .supervisor-dialog) :where(button, select, input):disabled { color: var(--text-muted); cursor: default; }
:where(.supervisor-view, .supervisor-dialog) :where(select, input:not([type=checkbox]), textarea) { padding: 6px 9px; border: 1px solid var(--border); border-radius: var(--radius-control); background: var(--terminal-bg); color: var(--text-primary); font: inherit; min-width: 0; max-width: 100%; box-sizing: border-box; }
:where(.supervisor-view, .supervisor-dialog) :where(textarea) { display: block; width: 100%; min-height: 64px; resize: vertical; }
:where(.supervisor-view, .supervisor-dialog) :where(input[type=checkbox]) { accent-color: var(--accent); }
.supervisor-view :focus-visible, .supervisor-dialog :focus-visible { outline: 2px solid var(--focus-strong); outline-offset: -2px; }
.supervisor-view button[aria-pressed=true], .supervisor-view .supervisor-primary { background: var(--select-fill); border-color: var(--select-edge); color: var(--text-primary); }
.supervisor-topbar-actions { padding-left: 0; }
.supervisor-topbar-actions .tab-strip-action[aria-pressed=true] { background: var(--select-fill); color: var(--accent); border-color: var(--border-strong); box-shadow: inset 0 var(--select-edge-size) 0 var(--select-edge); }
.supervisor-destination, .supervisor-muted { color: var(--text-muted); font-size: var(--font-size-control); }
.supervisor-attention-region { height: 44px; flex: 0 0 44px; overflow: auto; padding: 6px 12px; border-bottom: 1px solid var(--border); background: var(--chrome-bg); box-sizing: border-box; }
.supervisor-error { display: flex; align-items: flex-start; gap: 8px; padding: 8px 10px; color: var(--blocked); overflow-wrap: anywhere; border-inline-start: var(--select-edge-size) solid var(--blocked); background: var(--surface-raised); margin-block: 6px; }
.supervisor-error p { margin: 0; }
.supervisor-warning { color: var(--warning); overflow-wrap: anywhere; }
.supervisor-action-row { display: flex; gap: 6px; flex-wrap: wrap; align-items: center; }
.supervisor-agent-status { padding: 6px 0; border-bottom: 1px solid var(--border); }
.supervisor-status-heading { display: flex; gap: 8px; align-items: center; justify-content: space-between; flex-wrap: wrap; }
.supervisor-state { display: inline-flex; gap: 6px; align-items: center; color: var(--text-secondary); font-size: var(--font-size-control); }
.supervisor-agent-status.is-ready { display: flex; align-items: center; justify-content: space-between; gap: 12px; flex-wrap: wrap; border: 0; }
.supervisor-agent-status.is-ready .supervisor-status-heading > strong { display: none; }
.supervisor-agent-status.is-failure .supervisor-state, .supervisor-agent-status.is-missing .supervisor-state, .supervisor-agent-status.is-unknown .supervisor-state { color: var(--warning); }
.supervisor-agent-status.is-offline .supervisor-state, .supervisor-agent-status.is-closed .supervisor-state { color: var(--text-muted); }
.supervisor-needs-you { padding: 8px 10px; border-inline-start: var(--select-edge-size) solid var(--warning); background: var(--surface-raised); margin-block: 6px; }
.supervisor-needs-you h2 { display: flex; align-items: center; gap: 7px; margin-top: 0; color: var(--warning); }
.supervisor-needs-you[aria-label="Needs you"] { display: grid; grid-template-columns: 1fr 1fr; gap: 12px; }
.supervisor-needs-you .supervisor-action-form { display: grid; grid-template-columns: 1fr auto; align-items: end; gap: 5px 8px; margin: 0; }
.supervisor-needs-you .supervisor-action-form > label { grid-column: 1 / -1; }
.supervisor-needs-you .supervisor-action-form > p, .supervisor-needs-you .supervisor-action-form > .supervisor-error { grid-column: 1 / -1; }
.supervisor-needs-you textarea { min-height: 46px; height: 46px; }
.supervisor-graph-surface { position: relative; flex: 0 0 auto; min-height: 0; }
.supervisor-graph-band { height: var(--graph-height, 220px); min-height: 80px; max-height: 60vh; overflow: auto; border-bottom: 1px solid var(--border); background: var(--chrome-bg); box-sizing: border-box; }
.supervisor-graph-scroll { display: grid; grid-template-columns: max-content 180px 185px; align-items: start; overflow: auto; }
.supervisor-graph-canvas { position: relative; flex: 0 0 auto; }
.supervisor-graph-edges { position: absolute; inset: 0; pointer-events: none; overflow: visible; }
.supervisor-graph-edge { fill: none; stroke: var(--border-strong); stroke-width: 1.5; }
.supervisor-graph-edge.is-highlighted { stroke: var(--accent); stroke-width: 2; }
.supervisor-view .supervisor-graph-node { position: absolute; display: flex; flex-direction: column; align-items: stretch; text-align: left; justify-content: flex-start; gap: 1px; padding: 2px 8px; border-radius: var(--radius-control); background: var(--surface); overflow: hidden; }
.supervisor-graph-node.is-selected { background: var(--select-fill); box-shadow: inset var(--select-edge-size) 0 var(--select-edge); }
.supervisor-graph-node.is-highlighted { border-color: var(--accent); }
.supervisor-graph-node-heading { display: flex; gap: 7px; align-items: center; min-width: 0; }
.supervisor-graph-node-heading strong { font-weight: 500; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
.supervisor-graph-node-space { color: var(--text-muted); font-size: var(--font-size-control); overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
.supervisor-graph-task { justify-content: flex-start !important; min-height: 25px !important; padding: 3px 5px !important; font-size: var(--font-size-2xs) !important; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; width: 100%; }
.supervisor-graph-task.is-highlighted { border-color: var(--accent); background: var(--select-fill); }
.supervisor-graph-unmanaged { border-top: 1px dashed var(--border-strong); padding: 8px 12px; }
.supervisor-graph-unmanaged h3 { margin: 0 0 3px; }
.supervisor-graph-unmanaged-node { border-style: dashed !important; text-align: left; flex-direction: column; align-items: flex-start !important; }
.supervisor-board-surface { display: flex; flex-direction: column; flex: 1 1 0; min-height: 0; min-width: 0; }
.supervisor-task-list-heading { display: flex; justify-content: space-between; align-items: center; padding: 7px 12px; gap: 8px; flex-wrap: wrap; border-bottom: 1px solid var(--border); }
.supervisor-task-list-heading h2 { margin: 0; }
.supervisor-task-list-heading h2 span { margin-left: 6px; color: var(--text-muted); font-size: var(--font-size-control); font-weight: 400; }
.supervisor-task-filters { display: flex; gap: 8px; align-items: center; flex-wrap: wrap; font-size: var(--font-size-control); color: var(--text-secondary); }
.supervisor-task-filters label { display: inline-flex; align-items: center; gap: 5px; }
.supervisor-task-filters label:first-child { padding: 3px 7px; border: 1px solid var(--border); border-radius: var(--radius-control); }
.supervisor-task-filters .is-active { background: var(--select-fill); border-color: var(--accent) !important; }
.supervisor-task-filters select { padding: 3px 6px; font-size: var(--font-size-control); max-width: 150px; }
.supervisor-board { display: flex; align-items: stretch; gap: 8px; flex: 1 1 0; min-height: 0; overflow-x: auto; overflow-y: hidden; padding: 10px; scroll-snap-type: x mandatory; scroll-padding-inline: 10px; }
.supervisor-lane { display: flex; flex-direction: column; flex: 1 0 min(220px, calc(100cqw - 20px)); min-width: min(220px, calc(100cqw - 20px)); min-height: 0; background: var(--chrome-bg); border: 1px solid var(--border); border-radius: var(--radius-panel); scroll-snap-align: start; }
.supervisor-lane-header { display: flex; align-items: center; flex: 0 0 38px; gap: 6px; padding: 7px 8px; box-sizing: border-box; border-bottom: 1px solid var(--border); background: var(--chrome-bg); }
.supervisor-lane-header strong { font-size: var(--font-size-xs); font-weight: 500; }
.supervisor-lane-header > .ui-icon { margin-left: auto; color: var(--border-strong); }
.supervisor-lane-header button { min-height: 24px; padding: 2px; margin-left: auto; border-color: transparent; background: transparent; }
.supervisor-lane-dot { width: 7px; height: 7px; border-radius: var(--radius-pill); background: var(--idle); flex: 0 0 7px; }
.supervisor-lane.is-setup .supervisor-lane-dot, .supervisor-lane.is-ready .supervisor-lane-dot { background: var(--accent); }
.supervisor-lane.is-working .supervisor-lane-dot { background: var(--working); }
.supervisor-lane.is-review .supervisor-lane-dot { background: var(--warning); }
.supervisor-lane.is-accepted .supervisor-lane-dot { background: var(--done); }
.supervisor-lane-count { color: var(--text-muted); font-size: var(--font-size-control); }
.supervisor-lane.is-collapsed { flex-basis: 115px; min-width: 115px; max-width: 115px; }
.supervisor-lane-empty { padding: 8px 10px; color: var(--text-muted); font-size: var(--font-size-control); }
.supervisor-task-list, .supervisor-history { list-style: none; margin: 0; padding: 0; }
.supervisor-task-list { padding: 7px; flex: 1 1 0; min-height: 0; overflow-y: auto; overflow-x: hidden; }
.supervisor-task { padding: 10px 11px; margin-bottom: 9px; border: 1px solid var(--border); border-radius: var(--radius-control); background: var(--surface); box-shadow: inset var(--select-edge-size) 0 transparent; overflow-wrap: anywhere; }
.supervisor-task:hover { border-color: var(--border-strong); }
.supervisor-task.is-selected { background: var(--select-fill); box-shadow: inset var(--select-edge-size) 0 var(--select-edge); }
.supervisor-task.is-linked { border-color: var(--accent); }
.supervisor-task.is-dimmed { opacity: .58; }
.supervisor-view .supervisor-task-toggle { display: flex; flex-direction: column; align-items: flex-start; justify-content: flex-start; gap: 6px; width: 100%; text-align: left; padding: 0; border: 0; background: transparent; font: inherit; }
.supervisor-task-title { font-weight: 500; min-width: 0; overflow-wrap: anywhere; }
.supervisor-task-stage { color: var(--text-secondary); font-size: var(--font-size-control); }
.supervisor-worker-chip { display: flex; align-items: center; gap: 6px; font-size: var(--font-size-control); min-width: 0; }
.supervisor-attention-badge { display: inline-flex; gap: 4px; align-items: center; color: var(--warning); font-size: var(--font-size-control); }
.supervisor-task-evidence { font-size: var(--font-size-control); margin-top: 9px; padding-top: 7px; border-top: 1px solid var(--border); }
.supervisor-task-evidence > :first-child { margin-top: 0; }
.supervisor-evidence { display: flex; align-items: center; gap: 4px 6px; flex-wrap: wrap; color: var(--text-muted); font-size: var(--font-size-control); }
.supervisor-evidence > time { white-space: nowrap; margin-left: auto; order: 2; }
.supervisor-task-evidence .supervisor-observed .sb-glyph { display: none; }
.supervisor-report-summary { color: var(--text-secondary); white-space: pre-wrap; min-width: 0; flex: 1 1 100%; order: 3; overflow-wrap: anywhere; }
.supervisor-location { display: inline-flex; align-items: center; gap: 4px; padding: 2px 5px; border: 1px solid var(--border); border-radius: var(--radius-small); color: var(--text-secondary); font-size: var(--font-size-control); max-width: 100%; box-sizing: border-box; }
.supervisor-location.is-shared { background: var(--select-fill); border-color: var(--accent); }
.supervisor-detail-panel { display: flex; flex-direction: column; flex: 0 0 340px; min-width: 0; max-width: 42%; border-left: 1px solid var(--border-strong); background: var(--surface); }
.supervisor-detail-header { display: flex; align-items: center; gap: 8px; padding: 8px 10px; border-bottom: 1px solid var(--border); background: var(--chrome-bg); }
.supervisor-detail-header strong { font-size: var(--font-size-sm); font-weight: 500; min-width: 0; overflow-wrap: anywhere; }
.supervisor-detail-header button { padding: 4px; border-color: transparent; background: transparent; }
.supervisor-segments, .supervisor-narrow-switch { display: flex; align-items: center; gap: 4px; padding: 6px 10px; border-bottom: 1px solid var(--border); }
.supervisor-segments button { flex: 1; background: transparent; border-color: transparent; }
.supervisor-detail-scroll { flex: 1 1 0; min-height: 0; overflow: auto; padding: 12px 16px 20px; }
.supervisor-task-detail { min-width: 0; overflow-wrap: anywhere; }
.supervisor-detail-group, .supervisor-section { padding-block: 12px; border-bottom: 1px solid var(--border); }
.supervisor-detail-section > .supervisor-detail-group:first-child { padding-top: 0; }
.supervisor-detail-group > h3 { margin: 0 0 6px; color: var(--text-secondary); font-size: var(--font-size-control); }
.supervisor-detail-group .supervisor-report-summary { font-size: var(--font-size-sm); line-height: var(--line-height-relaxed); }
.supervisor-detail-group > button, .supervisor-detail-section > button, .supervisor-needs-you > button { margin: 4px 6px 4px 0; }
.supervisor-exact-text { white-space: pre-wrap; overflow-wrap: anywhere; line-height: var(--line-height-relaxed); tab-size: 4; }
.supervisor-plan { white-space: pre-wrap; overflow-wrap: anywhere; font: var(--font-size-control)/var(--line-height-normal) var(--font-mono); max-width: 100%; margin: 8px 0; padding: 8px; border: 1px solid var(--border); background: var(--terminal-bg); }
.supervisor-action-form { display: flex; flex-direction: column; gap: 8px; margin-block: 12px; }
.supervisor-action-form > label { font-weight: 500; font-size: var(--font-size-control); }
.supervisor-history > li { padding-block: 10px; border-bottom: 1px solid var(--border); }
.supervisor-history strong { font-weight: 500; }
.supervisor-history time { display: block; color: var(--text-muted); font-size: var(--font-size-control); margin-top: 4px; }
.supervisor-empty { display: flex; flex-direction: column; align-items: center; justify-content: center; flex: 1; min-height: 150px; gap: 6px; padding: 24px; text-align: center; color: var(--text-secondary); }
.supervisor-empty > .ui-icon { width: 28px; height: 28px; flex: 0 0 28px; color: var(--text-muted); }
.supervisor-empty p { max-width: 370px; color: var(--text-muted); }
.supervisor-row-splitter { position: absolute; z-index: var(--z-raised); left: 0; right: 0; height: 6px; cursor: row-resize; touch-action: none; }
.supervisor-graph-splitter { bottom: -3px; }
.supervisor-row-splitter::after { content: ""; position: absolute; left: 0; right: 0; top: 2px; height: 2px; background: transparent; transition: background 80ms; }
.supervisor-row-splitter:hover::after, .supervisor-row-splitter:focus-visible::after, .supervisor-row-splitter:active::after { background: var(--accent); }
.supervisor-row-splitter:focus { outline: none; }
.supervisor-focus-notice { min-height: 3px; font-size: var(--font-size-control); color: var(--text-muted); }
.supervisor-archive { position: absolute; right: 90px; top: 48px; z-index: var(--z-floating); width: min(330px, calc(100% - 24px)); padding: 12px; box-sizing: border-box; background: var(--surface-raised); border: 1px solid var(--border-strong); border-radius: var(--radius-panel); box-shadow: 0 8px 24px var(--scrim); }
.supervisor-archive button { margin-top: 6px; }
.supervisor-dialog { width: min(460px, calc(100vw - 24px)); max-height: calc(100dvh - 32px); overflow: auto; padding: 18px; background: var(--surface); border: 1px solid var(--border-strong); border-radius: var(--radius-panel); box-shadow: 0 12px 40px var(--scrim); box-sizing: border-box; color: var(--text-primary); }
.supervisor-dialog-header { display: flex; align-items: center; gap: 9px; border-bottom: 1px solid var(--border); padding-bottom: 12px; margin-bottom: 12px; }
.supervisor-dialog-header h2 { margin: 0; font: 500 var(--font-size-md)/var(--line-height-normal) var(--font-sans); }
.supervisor-dialog-icon { display: flex; align-items: center; color: var(--accent); }
.supervisor-dialog label { display: flex; flex-direction: column; gap: 6px; margin-block: 10px; font-size: var(--font-size-control); }
.supervisor-dialog input, .supervisor-dialog select { width: 100%; }
.supervisor-dialog footer { display: flex; align-items: center; justify-content: flex-end; gap: 8px; flex-wrap: wrap; border-top: 1px solid var(--border); padding-top: 12px; }
.supervisor-dialog-unavailable { color: var(--text-muted); font-size: var(--font-size-control); }
.supervisor-dialog-error-slot { min-height: 42px; }
.supervisor-narrow-switch { display: none; }
@container (max-width: 1050px) { .supervisor-header-state { max-width: 160px; } .supervisor-header-state > span { overflow: hidden; text-overflow: ellipsis; white-space: nowrap; } .supervisor-header-panels { margin-left: auto; } }
@container (max-width: 719px) {
  .supervisor-header { padding: 6px 8px; }
  .supervisor-header-state { flex: 1; }
  .supervisor-header-panels { order: 2; }
  .supervisor-header > .supervisor-primary { margin-left: auto; }
  .supervisor-brand strong { display: none; }
  .supervisor-needs-you[aria-label="Needs you"] { grid-template-columns: 1fr; gap: 6px; }
  .supervisor-needs-you .supervisor-action-form > label, .supervisor-question .supervisor-muted { display: none; }
  .supervisor-narrow-switch { display: flex; }
  .supervisor-graph-surface, .supervisor-board-surface { display: none; }
  .supervisor-graph-surface.is-narrow-active { display: flex; flex: 1; min-height: 0; }
  .supervisor-graph-surface .supervisor-graph-band { height: auto; max-height: none; flex: 1; }
  .supervisor-graph-splitter { display: none; }
  .supervisor-board-surface.is-narrow-active { display: flex; }
  .supervisor-detail-panel { position: absolute; inset: 0; max-width: none; z-index: var(--z-raised); border-left: 0; }
  .supervisor-task-list-heading { padding: 7px 8px; }
  .supervisor-task-filters { gap: 5px; }
  .supervisor-task-filters label:first-child { max-width: 235px; }
}
.supervisor-graph-heading { display: flex; align-items: center; justify-content: space-between; gap: 12px; padding: 3px 10px; height: 30px; box-sizing: border-box; position: sticky; top: 0; z-index: 1; background: var(--chrome-bg); border-bottom: 1px solid var(--border); }
.supervisor-graph-heading h2 { margin: 0; font-size: var(--font-size-control); }
.supervisor-graph-subagents { display: flex; gap: 6px; align-items: center; font-size: var(--font-size-control); color: var(--text-secondary); }
.supervisor-graph-node-space.is-shared { color: var(--accent); }
.supervisor-graph-task > span { overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
.supervisor-graph-unmanaged h3 > span { color: var(--text-muted); font-size: var(--font-size-control); font-weight: 400; margin-left: 8px; }
.supervisor-graph-unmanaged-nodes { display: flex; gap: 8px; }
.supervisor-graph-unmanaged-open { display: flex; gap: 6px; align-items: center; color: var(--text-secondary); }
.supervisor-graph-empty { color: var(--text-muted); font-size: var(--font-size-control); padding: 8px 12px; }
.supervisor-attention-region { display: flex; flex-direction: column; }
.supervisor-attention-region > * { flex-shrink: 0; }
.supervisor-attention-region > .supervisor-needs-you { order: -1; }
.supervisor-attention-region > [aria-label="Needs you"] { order: -2; }
.supervisor-graph-node-space { font-size: var(--font-size-2xs); }
@container (max-width: 719px) { .supervisor-workarea.has-panel > .supervisor-content { visibility: hidden; pointer-events: none; } }
.supervisor-feedback { display: flex; align-items: flex-start; gap: 6px; height: 36px; overflow: auto; font-size: var(--font-size-control); color: var(--text-muted); }
.supervisor-feedback.is-error { color: var(--blocked); }
.supervisor-needs-you .supervisor-feedback { grid-column: 1 / -1; height: 30px; }
.supervisor-graph-unmanaged { flex: 0 0 205px; align-self: stretch; border-top: 0; border-left: 1px dashed var(--border-strong); box-sizing: border-box; }
.supervisor-graph-unmanaged h3 > span { display: block; margin: 5px 0 9px; }
.supervisor-graph-unmanaged-nodes { flex-direction: column; }
.supervisor-attention-region.is-empty { height: 0; flex-basis: 0; padding: 0; border: 0; }
@container (max-width: 479px) { .supervisor-header-panels { flex-basis: 100%; justify-content: flex-end; margin-left: 0; } .supervisor-header-state { max-width: none; } }
@media (max-height: 600px) {
  .supervisor-view.has-attention .supervisor-attention-region { height: 110px; flex-basis: 110px; }
  .supervisor-narrow-switch { display: flex; }
  .supervisor-graph-surface, .supervisor-board-surface { display: none; }
  .supervisor-graph-surface.is-narrow-active { display: flex; flex: 1; min-height: 0; }
  .supervisor-graph-surface .supervisor-graph-band { height: auto; max-height: none; flex: 1; }
  .supervisor-graph-splitter { display: none; }
  .supervisor-board-surface.is-narrow-active { display: flex; }
}
.supervisor-view:not(.has-attention) .supervisor-attention-region { flex-direction: row; align-items: center; gap: 14px; }
.supervisor-view:not(.has-attention) .supervisor-attention-region > .supervisor-agent-status.is-ready { display: contents; }
.supervisor-view:not(.has-attention) .supervisor-attention-region .supervisor-status-heading { display: none; }
.supervisor-view:not(.has-attention) .supervisor-attention-region .supervisor-action-row { order: 3; margin-left: auto; flex-wrap: nowrap; }
.supervisor-view:not(.has-attention) .supervisor-attention-region > .supervisor-evidence { margin: 0; flex-wrap: nowrap; gap: 6px; }
.supervisor-view:not(.has-attention) .supervisor-attention-region > .supervisor-observed { order: 2; }
.supervisor-view:not(.has-attention) .supervisor-attention-region > .supervisor-observed time { display: none; }
.supervisor-view:not(.has-attention) .supervisor-attention-region > .supervisor-observed .sb-glyph { width: 16px; height: 16px; flex-basis: 16px; }
.supervisor-attention-region .supervisor-action-row > button { min-height: 26px; padding: 3px 7px; }
.supervisor-view.has-attention .supervisor-attention-region { height: 164px; flex-basis: 164px; }
.supervisor-view.has-attention .supervisor-graph-band { height: 100px; }
.supervisor-graph-node-heading { height: 18px; flex: 0 0 18px; }
.supervisor-graph-node-heading strong { font-size: var(--font-size-control); line-height: 18px; }
.supervisor-graph-node-metadata { color: var(--text-muted); font-size: var(--font-size-2xs); line-height: 11px; white-space: nowrap; overflow: hidden; text-overflow: ellipsis; }
.supervisor-graph-node-metadata.is-shared { color: var(--accent); }
.supervisor-graph-task-links { display: flex; flex-direction: column; gap: 5px; padding: 8px; border-left: 1px solid var(--border); align-self: stretch; }
.supervisor-graph-task-links::before { content: "Task links"; color: var(--text-muted); font-size: var(--font-size-control); }
.supervisor-graph-unmanaged { padding: 8px; }
.supervisor-graph-unmanaged h3 { font-size: var(--font-size-control); }
.supervisor-graph-unmanaged h3 > span { font-size: var(--font-size-2xs); margin: 3px 0 6px; }
.supervisor-view .supervisor-graph-unmanaged-node { display: grid; grid-template-columns: minmax(0, 1fr) 18px; gap: 2px 4px; padding: 5px 6px; }
.supervisor-graph-unmanaged-node > .supervisor-graph-node-heading { grid-column: 1; }
.supervisor-graph-unmanaged-node > .supervisor-graph-node-space { grid-column: 1; font-size: var(--font-size-2xs); }
.supervisor-graph-unmanaged-open { grid-column: 2; grid-row: 1 / 3; align-self: center; }
@container (max-width: 719px) {
  .supervisor-view:not(.has-attention) .supervisor-attention-region { gap: 8px; padding-inline: 8px; }
  .supervisor-view:not(.has-attention) .supervisor-attention-region > .supervisor-evidence { font-size: var(--font-size-2xs); }
  .supervisor-view:not(.has-attention) .supervisor-attention-region > .supervisor-evidence > span:first-child { max-width: 140px; white-space: nowrap; overflow: hidden; text-overflow: ellipsis; }
  .supervisor-view:not(.has-attention) .supervisor-attention-region > .supervisor-observed { display: none; }
  .supervisor-attention-region .supervisor-action-row > button { font-size: var(--font-size-2xs); }
}
@container (max-width: 479px) {
  .supervisor-view:not(.has-attention) .supervisor-attention-region > .supervisor-evidence { min-width: 0; flex-shrink: 1; }
  .supervisor-view:not(.has-attention) .supervisor-attention-region > .supervisor-evidence > span:first-child { max-width: 90px; }
}
.supervisor-board-surface { container: supervisor-board / inline-size; }
@container (max-width: 479px) { .supervisor-needs-you .supervisor-action-row > button { padding: 2px 5px; gap: 4px; min-height: 24px; font-size: var(--font-size-2xs); } }
@container (max-width: 479px) { .supervisor-needs-you .supervisor-action-row > button[aria-label="Close tracking…"] { width: 26px; padding: 3px; font-size: 0; } }
```

## boardNavigation.ts

```tsx
import type { TaskLane, TaskView } from "../../protocol/generated/v1";

export const taskLanes: readonly { lane: TaskLane; label: string }[] = [
  { lane: "queued", label: "Queued" }, { lane: "setup", label: "Preparing" },
  { lane: "ready", label: "Ready" }, { lane: "working", label: "Working" },
  { lane: "review", label: "Review" }, { lane: "accepted", label: "Done" },
];

/** Focus navigation only: never changes the backend-derived lane or selection. */
export function taskNeighbor(tasks: readonly TaskView[], id: string, key: string, completedOpen: boolean): string | undefined {
  const lanes = taskLanes.filter(item => item.lane !== "accepted" || completedOpen)
    .map(item => tasks.filter(task => task.lane === item.lane));
  const laneIndex = lanes.findIndex(lane => lane.some(task => task.task.task_id === id));
  if (laneIndex < 0) return undefined;
  const lane = lanes[laneIndex];
  const index = lane.findIndex(task => task.task.task_id === id);
  if (key === "ArrowUp" || key === "ArrowDown") return lane[Math.max(0, Math.min(lane.length - 1, index + (key === "ArrowDown" ? 1 : -1)))]?.task.task_id;
  if (key === "Home" || key === "End") return lane[key === "Home" ? 0 : lane.length - 1]?.task.task_id;
  if (key !== "ArrowLeft" && key !== "ArrowRight") return undefined;
  const direction = key === "ArrowRight" ? 1 : -1;
  for (let next = laneIndex + direction; next >= 0 && next < lanes.length; next += direction) {
    if (lanes[next].length) return lanes[next][Math.min(index, lanes[next].length - 1)].task.task_id;
  }
  return id;
}
```

## useSupervisor.ts

```tsx
import { useCallback, useEffect, useRef, useState } from "react";
import type { CockpitClient } from "../../client/CockpitClient";
import type { OrchestrationAction, OrchestrationActionResult, OrchestrationSnapshot } from "../../protocol/generated/v1";

// Exact hashes and stable operation IDs fence these mutations independently of telemetry.
const LOCAL_FENCES: Partial<Record<OrchestrationAction["action"], true>> = {
  task_update: true, task_assign: true, task_assignment_resolve: true,
  tasks_assign_ids: true, grant_prepare: true, grant_execute: true,
  accept: true, message_send: true, annotate: true,
};

export function acceptsSupervisorSnapshot(next: OrchestrationSnapshot, sessionId: string, rootId: string | null, revisionFloor: number): boolean {
  return next.session_id === sessionId && next.revision >= revisionFloor
    && (!rootId || next.board === null || next.board.root_id === rootId);
}

/** One scope generation fences long polls and snapshots. Drafts live outside snapshots. */
export function useSupervisor(client: CockpitClient, sessionId: string, rootId: string | null, active: boolean) {
  const [snapshot, setSnapshot] = useState<OrchestrationSnapshot | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [observationError, setObservationError] = useState<string | null>(null);
  const [connected, setConnected] = useState(false);
  const [busy, setBusy] = useState(false);
  const [refreshToken, setRefreshToken] = useState(0);
  const generation = useRef(0);
  const floor = useRef(0);
  const identity = useRef({ client, sessionId, rootId });
  identity.current = { client, sessionId, rootId };
  const current = useRef<OrchestrationSnapshot | null>(null);
  const pending = useRef(false);
  const refresh = useCallback(() => setRefreshToken(value => value + 1), []);
  useEffect(() => {
    floor.current = 0;
    current.current = null;
    pending.current = false;
    setSnapshot(null);
    setBusy(false);
    setError(null);
    setObservationError(null);
  }, [client, sessionId, rootId]);
  useEffect(() => {
    const scope = ++generation.current;
    let cancelled = false;
    let timer = 0;
    const sleep = () => new Promise<void>(resolve => { timer = window.setTimeout(resolve, 1500); });
    const live = () => !cancelled && generation.current === scope;
    if (!active) { setConnected(false); return () => { cancelled = true; }; }
    const observe = async () => {
      while (live()) {
        try {
          const next = await client.orchestrationSnapshot({ session_id: sessionId, root_id: rootId });
          if (!live()) return;
          if (!acceptsSupervisorSnapshot(next, sessionId, rootId, floor.current)) { await sleep(); continue; }
          floor.current = next.revision;
          current.current = next;
          setSnapshot(next);
          setConnected(true);
          setObservationError(null);
          const wait = await client.orchestrationWait({ after_revision: next.revision, after_tasks_token: next.tasks_token, timeout_ms: 2000 });
          if (!live()) return;
          floor.current = Math.max(floor.current, wait.revision);
          // Even unchanged durable state needs a fresh Herdr observation after the wait.
        } catch (failure) {
          if (!live()) return;
          setConnected(false);
          setObservationError(failure instanceof Error ? failure.message : "Supervisor connection unavailable");
          await sleep();
        }
      }
    };
    void observe();
    return () => { cancelled = true; generation.current++; window.clearTimeout(timer); };
  }, [client, sessionId, rootId, active, refreshToken]);
  const mutateResult = useCallback(async (action: OrchestrationAction): Promise<OrchestrationActionResult | null> => {
    const observed = current.current;
    if (!active || !observed || pending.current) return null;
    const scope = identity.current;
    const sameScope = () => scope.client === identity.current.client && scope.sessionId === identity.current.sessionId && scope.rootId === identity.current.rootId;
    pending.current = true;
    setBusy(true);
    setError(null);
    try {
      const response = await client.orchestrationMutate({ session_id: sessionId, expected_revision: LOCAL_FENCES[action.action] ? null : observed.revision, action });
      if (!sameScope()) return null;
      floor.current = Math.max(floor.current, response.revision);
      refresh();
      return response.result;
    } catch (failure) {
      if (!sameScope()) return null;
      setError(failure instanceof Error ? failure.message : "The action was not confirmed. Review current state before retrying.");
      refresh();
      return null;
    } finally {
      if (sameScope()) { pending.current = false; setBusy(false); }
    }
  }, [client, sessionId, active, refresh]);
  return { snapshot, error: error ?? observationError, connected, busy, mutateResult, refresh };
}
```

## useSupervisorDrafts.ts

```tsx
import { useRef, useState } from "react";

export type TextDraft = { text: string; operation: { id: string; text: string } | null; notice: string | null; error: string | null };
export type EditDraft = { title: string; body: string; revision: string };
export type ScopeDrafts = {
  messages: Map<string, TextDraft>;
  edits: Map<string, EditDraft>;
  selectedTask: string | null;
  selectedRun: string | null;
  selectedSubagent: string | null;
  showSubagents: boolean;
  showUnmanaged: boolean;
  attentionOnly: boolean;
  spaceFilter: string;
  disclosures: Record<"completed" | "agents" | "history" | "diagnostics" | "archive", boolean>;
};
export function newTextDraft(): TextDraft { return { text: "", operation: null, notice: null, error: null }; }
export function newScopeDrafts(): ScopeDrafts {
  return { messages: new Map(), edits: new Map(), selectedTask: null, selectedRun: null, selectedSubagent: null, showSubagents: true, showUnmanaged: true, attentionOnly: false, spaceFilter: "", disclosures: { completed: false, agents: false, history: false, diagnostics: false, archive: false } };
}
export function messageDraft(scope: ScopeDrafts, key: string): TextDraft {
  let draft = scope.messages.get(key);
  if (!draft) { draft = newTextDraft(); scope.messages.set(key, draft); }
  return draft;
}
/** Memory belongs to the mounted workarea, not its current root, selected row or snapshot. */
export function useSupervisorDrafts(sessionId: string) {
  const registry = useRef({ sessionId, scopes: new Map<string, ScopeDrafts>() });
  const [, render] = useState(0);
  if (registry.current.sessionId !== sessionId) registry.current = { sessionId, scopes: new Map() };
  return {
    scope: (rootId: string | null) => {
      const key = rootId ?? "unselected";
      let scope = registry.current.scopes.get(key);
      if (!scope) { scope = newScopeDrafts(); registry.current.scopes.set(key, scope); }
      return scope;
    },
    changed: () => render(value => value + 1),
  };
}
```
