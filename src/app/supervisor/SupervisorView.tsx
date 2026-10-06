import { useEffect, useId, useRef, useState } from "react";
import type { CockpitClient } from "../../client/CockpitClient";
import type { ActorRef, OrchestrationSnapshot, Run, SessionSnapshotResponse, Subagent, TaskView } from "../../protocol/generated/v1";
import { useRovingList } from "../sidebar/useRovingList";
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
  const connectedAgents = rootRuns.filter(run => run.stage !== "closed" && observations.some(observed => observed.run_id === run.run_id && observed.actual_omp && observed.presence === "present")).length;
  const completed = tasks.filter(task => task.lane === "accepted");
  const openTasks = tasks.filter(task => task.lane !== "accepted");
  const taskIds = [...openTasks.map(task => task.task.task_id), ...(scope.disclosures.completed ? completed.map(task => task.task.task_id) : [])];
  const { listRef, listProps, tabIndexFor, focusRow } = useRovingList({ rowIds: taskIds, selectedId: scope.selectedTask, onEscape: () => { if (scope.selectedTask) { scope.selectedTask = null; changed(); } else onClose(); return true; } });
  const agentRows = [...forest.map(row => row.key), ...(live ? snapshot?.unmanaged_agents.map(agent => `other:${agent.pane_id}`) ?? [] : [])];
  const agentRoving = useRovingList({ rowIds: agentRows, selectedId: scope.selectedRun ? scope.selectedSubagent ? `${scope.selectedRun}:${scope.selectedSubagent}` : scope.selectedRun : null, onEscape: () => { scope.disclosures.agents = false; changed(); return true; } });
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
  const renderTask = (task: TaskView) => {
    if (!snapshot) return null;
    const worker = snapshot.runs.find(run => run.run_id === task.current_run_id);
    const observed = observe(worker?.run_id);
    const selected = scope.selectedTask === task.task.task_id;
    const detailId = `supervisor-task-${task.task.task_id}`;
    const needsAttention = !!worker && (worker.last_report?.kind === "needs_input" || ["failure", "missing", "unknown"].includes(agentState(snapshot, worker, connected, runtimeLive).kind)) || assignmentIntents.some(intent => intent.task_id === task.task.task_id) || acceptanceConflicts.some(intent => intent.task_id === task.task.task_id);
    const dimmed = scope.attentionOnly && !needsAttention || !!scope.spaceFilter && observed?.workspace_id !== scope.spaceFilter;
    return <li key={task.task.task_id} className={`supervisor-task${selected ? " is-selected" : ""}${dimmed ? " is-dimmed" : ""}`}><button type="button" data-row-id={task.task.task_id} tabIndex={tabIndexFor(task.task.task_id)} className="supervisor-task-toggle" aria-expanded={selected} aria-controls={detailId} onFocus={() => { focusedTask.current = task.task.task_id; }} onClick={() => { scope.selectedTask = selected ? null : task.task.task_id; changed(); }}><span className="supervisor-task-title">{task.task.title}</span><span className="supervisor-task-stage">{taskStatus(task, worker, snapshot)}{needsAttention ? " · Needs attention" : ""}</span></button>{worker ? <div className="supervisor-task-evidence">{observed?.workspace_label ? <span className={`supervisor-location${sharedSpace === observed.workspace_id ? " is-shared" : ""}`} title={observed.tab_label ? `${observed.workspace_label} · ${observed.tab_label}` : observed.workspace_label}>{observed.workspace_label}</span> : null}<ReportedEvidence report={worker.last_report} source={worker.label} /><ObservedEvidence run={worker} observed={observed} snapshot={snapshot} live={!!live} />{observed?.agent_status === "blocked" && worker.last_report?.kind !== "needs_input" ? <p className="supervisor-warning">Agent is blocked; no question reported. <button type="button" disabled={busy || !live} onClick={() => void navigate(worker)}>Open terminal</button><button type="button" disabled={busy} onClick={() => check(worker)}>Check status</button></p> : null}{observed?.agent_status === "done" && !worker.result ? <p className="supervisor-muted">Runtime Done · no result reported</p> : null}</div> : <p className="supervisor-evidence">No progress reported yet</p>}{selected && active ? <div id={detailId} onKeyDown={event => { if (event.key !== "Escape" || event.defaultPrevented || event.nativeEvent.isComposing || (event.target as HTMLElement).closest("input,textarea,select,[contenteditable=true]")) return; event.preventDefault(); event.stopPropagation(); scope.selectedTask = null; changed(); focusRow(task.task.task_id); }}><SupervisorActions snapshot={snapshot} run={worker ?? null} task={task} scope={scope} changed={changed} busy={busy} live={!!live} mutateResult={mutateResult} onTerminal={run => void navigate(run)} onEditTask={edit} onCloseTracking={run => setDialog({ mode: "close", run })} onCancelSubagent={(run, subagent) => setDialog({ mode: "subagent_cancel", run, subagent })} /></div> : null}</li>;
  };
  return <section ref={rootRef} className="supervisor-view" hidden={!active} aria-label="Supervisor" onFocusCapture={event => {
    const target = event.target as HTMLElement;
    focusedTask.current = target.closest("li.supervisor-task")?.querySelector<HTMLElement>("[data-row-id]")?.dataset.rowId ?? null;
    questionHadFocus.current = !!target.closest('[aria-label="Needs you"]');
  }} onKeyDown={event => {
    if (event.key !== "Escape" || event.nativeEvent.isComposing || event.defaultPrevented || dialog || (event.target as HTMLElement).closest("input,textarea,select,[contenteditable=true]")) return;
    event.preventDefault(); event.stopPropagation();
    const disclosure = (event.target as HTMLElement).closest("details[open]");
    if (disclosure) { (disclosure as HTMLDetailsElement).open = false; disclosure.querySelector<HTMLElement>("summary")?.focus(); }
    else onClose();
  }}><header className="supervisor-header"><strong>Supervisor</strong>{openRoots.length > 1 ? <label className="supervisor-agent-label">Agent<select aria-label="Agent" value={root?.stage !== "closed" ? root?.run_id ?? "" : ""} disabled={busy || startPending || !!dialog} onChange={event => { setRootId(event.target.value); setTerminalError(null); }}><option value="" disabled>Choose an agent</option>{openRoots.map(root => <option key={root.root_id} value={root.root_id}>{root.label}</option>)}</select></label> : null}<button type="button" data-start-agent disabled={busy || startPending || !snapshot || !connected || startUnknown || rootState?.kind === "starting"} onClick={() => void start()}>{startPending ? "Starting…" : "Start agent"}</button><button type="button" className="supervisor-secondary-button" disabled={busy || startPending || !snapshot || !connected || startUnknown || rootState?.kind === "starting"} onClick={() => { if (!startDraft.current.spaceId && destination) startDraft.current.spaceId = destination.id; setDialog({ mode: "start" }); }}>Start options…</button><button type="button" aria-label="Close Supervisor" onClick={onClose}>Close view</button></header>
    <div className="supervisor-scroll"><div className="supervisor-content">
      {!snapshot ? <div className="supervisor-empty"><h2>{error ? "Could not load Supervisor" : "Loading Supervisor…"}</h2>{error ? <><p>Your terminals are unchanged. The Supervisor connection could not be established.</p><button type="button" onClick={refresh}>Retry load</button></> : <p role="status">Reading saved tasks and fresh agent observations.</p>}</div> : <>
        <p className="supervisor-destination">{destination ? `New tab in ${destination.label}` : "Choose an available location when starting."} · Starts OMP without switching terminal focus.</p>
        {notice ? <p role="status">{notice}</p> : null}{startUnknown ? <div className="supervisor-warning"><button type="button" disabled={busy} onClick={refresh}>Check status</button><p>Review the tracked agents below before another start.</p><button type="button" disabled={busy} onClick={() => { setStartUnknown(false); setNotice("Previous start reviewed. Any existing terminal and tracking remain unchanged."); }}>I have reviewed the previous start</button></div> : null}
        {terminalError || navigationError || error ? <div className="supervisor-error" role="alert"><p>{terminalError ?? (navigationError ? "Could not open terminal. Its current focus or location was not confirmed." : !connected ? "Could not refresh Supervisor. Saved tasks and drafts are kept; the agent may still be running." : "The requested change was not confirmed. Your drafts and resources are kept. Check current status before trying again.")}</p><button type="button" disabled={busy} onClick={refresh}>Check status</button></div> : null}
        {!root && orphanedWorkers.length ? <section className="supervisor-warning"><h2>Worker agents need control</h2><p>Closing their supervisor did not stop them. Tasks and history are kept.</p>{orphanedWorkers.map(run => <div key={run.run_id}>{recovery(run)}<button type="button" disabled={busy} onClick={() => setRootId(run.root_id)}>View saved task context</button></div>)}</section> : null}
        {root ? <>{recovery(root)}{root.stage === "closed" && descendants.length ? <section className="supervisor-warning"><h2>Worker agents need control</h2><p>{descendants.length} tracked descendants remain open. Closing this supervisor did not stop them.</p>{descendants.map(run => recovery(run))}</section> : failures.length ? <section className="supervisor-needs-you"><h2>{failures.length === 1 ? "Task needs recovery" : `${failures.length} tasks need recovery`}</h2>{failures.map(run => recovery(run))}</section> : null}
        {!question && root.last_report ? <ReportedEvidence report={root.last_report} source={root.label} /> : null}
        <ObservedEvidence run={root} observed={observe(root.run_id)} snapshot={snapshot} live={!!live} />
        {observe(root.run_id)?.agent_status === "blocked" && !question ? <section className="supervisor-warning"><p>Agent is blocked; no question reported.</p><div className="supervisor-action-row"><button type="button" disabled={busy || !live || !rootState?.terminal} onClick={() => void navigate(root)}>Open terminal</button><button type="button" disabled={busy} onClick={() => check(root)}>Check status</button></div></section> : null}
        {question && answer && !answered ? <section className="supervisor-needs-you" aria-label="Needs you"><h2>Needs you · {root.label}</h2><p id={`supervisor-question-${question.message_id}`} className="supervisor-exact-text">{question.summary}</p><p className="supervisor-muted">Reported by {root.label} · <time dateTime={question.at}>{new Date(question.at).toLocaleString()}</time></p><TextAction label="Answer" submitLabel="Send answer" describedBy={`supervisor-question-${question.message_id}`} draft={answer} changed={changed} busy={busy || !live || !rootState?.verified} submit={(text, message_id) => mutateResult({ action: "message_send", message_id, to_run_id: root.run_id, kind: "answer", text })} success="Answer sent. Waiting for the agent." /></section> : question && answered ? <p role="status">Answer sent. Waiting for the agent.</p> : null}
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
        <TaskComposer root={root} scope={scope} snapshot={snapshot} canAssign={!!rootState?.verified && root.stage === "active" && !!live} busy={busy || startPending} changed={changed} mutateResult={mutateResult} refresh={refresh} />
        <section aria-label="Tasks"><div className="supervisor-task-list-heading"><h2>Open tasks · {openTasks.length}</h2><details className="supervisor-task-filters"><summary>Task filters</summary><label><input type="checkbox" checked={scope.attentionOnly} onChange={event => { scope.attentionOnly = event.target.checked; changed(); }} /> Needs attention (dims other tasks)</label><label>Space<select value={scope.spaceFilter} onChange={event => { scope.spaceFilter = event.target.value; changed(); }}><option value="">All Spaces</option>{[...new Map(observations.filter(item => item.workspace_id && item.workspace_label && rootRuns.some(run => run.run_id === item.run_id)).map(item => [item.workspace_id!, item.workspace_label!])).entries()].map(([id, label]) => <option key={id} value={id}>{label}</option>)}</select></label></details></div>{!tasks.length ? <p className="supervisor-muted">No tasks yet.</p> : null}<div ref={listRef} {...listProps}><ul className="supervisor-task-list">{openTasks.map(renderTask)}</ul><details open={scope.disclosures.completed} onToggle={event => disclosures("completed", event.currentTarget.open)}><summary>Completed · {completed.length}</summary><ul className="supervisor-task-list">{completed.map(renderTask)}</ul></details></div></section>
        </> : <div className="supervisor-empty"><h2>Start an agent to manage your tasks.</h2><p>Give it work here or in its terminal. It handles delegation, preparation, execution and review.</p></div>}
        <div className="supervisor-secondary">
          <details open={scope.disclosures.agents} onToggle={event => disclosures("agents", event.currentTarget.open)}><summary>Agents{root ? ` · ${live ? `${connectedAgents} connected` : "unobserved"}` : ""}</summary>{root && live && !connectedAgents ? <p>No connected agents observed in this task scope. Saved tracking does not mean an agent is running.</p> : !root && orphanedWorkers.length ? <p>Worker recovery is above. Select saved task context to inspect their relationships.</p> : null}<label><input type="checkbox" disabled={!root} checked={scope.showSubagents} onChange={event => { scope.showSubagents = event.target.checked; if (!scope.showSubagents && scope.selectedSubagent) { scope.selectedSubagent = null; requestAnimationFrame(() => agentRoving.focusRow(scope.selectedRun ?? undefined)); } changed(); }} /> Subagents</label><div ref={agentRoving.listRef} {...agentRoving.listProps}><ul className="supervisor-agent-list">{forest.map(row => {
            const observed = observe(row.run.run_id);
            const selected = scope.selectedRun === row.run.run_id && scope.selectedSubagent === (row.subagent?.subagent_id ?? null);
            return <li key={row.key} style={{ paddingInlineStart: `${Math.min(row.depth, 8) * 14}px` }}><button type="button" data-row-id={row.key} tabIndex={agentRoving.tabIndexFor(row.key)} className={`supervisor-agent-row${selected ? " is-selected" : ""}`} aria-expanded={selected} onMouseEnter={() => setHoverSpace(observed?.workspace_id ?? null)} onMouseLeave={() => setHoverSpace(null)} onClick={() => { scope.selectedRun = selected ? null : row.run.run_id; scope.selectedSubagent = selected ? null : row.subagent?.subagent_id ?? null; changed(); }}><span><strong>{row.subagent?.label ?? row.run.label}</strong> · {row.subagent?.role ?? (row.subagent ? "OMP subagent" : row.run.kind === "worker" ? "Task agent" : "Supervisor")}</span><span>{row.subagent ? `${row.subagent.status} · In ${row.run.label} · no terminal` : root?.stage === "closed" ? "Needs control" : agentState(snapshot, row.run, connected, runtimeLive).label}</span>{observed?.workspace_label && !row.subagent ? <span className={`supervisor-location${observed.workspace_id === sharedSpace ? " is-shared" : ""}`} title={observed.tab_label ? `${observed.workspace_label} · ${observed.tab_label}` : observed.workspace_label}>{observed.workspace_label}</span> : null}</button>{!row.subagent ? <><ReportedEvidence report={row.run.last_report} source={row.run.label} /><ObservedEvidence run={row.run} observed={observed} snapshot={snapshot} live={!!live} /></> : row.subagent.summary ? <p className="supervisor-evidence">Reported · OMP events · {row.subagent.summary} · {row.subagent.updated_at}</p> : null}{selected && active ? <SupervisorActions snapshot={snapshot} run={row.run} subagent={row.subagent} task={tasks.find(task => task.task.task_id === row.run.task_id) ?? null} scope={scope} changed={changed} busy={busy} live={!!live} mutateResult={mutateResult} onTerminal={run => void navigate(run)} onEditTask={edit} onCloseTracking={run => setDialog({ mode: "close", run })} onCancelSubagent={(run, subagent) => setDialog({ mode: "subagent_cancel", run, subagent })} /> : null}</li>;
          })}</ul><h3>Other agents</h3><p className="supervisor-muted">Unmanaged · no supervisor role or task relationship is claimed.</p>{live ? snapshot.unmanaged_agents.length ? snapshot.unmanaged_agents.map(agent => <div key={agent.pane_id} className="supervisor-other-agent"><button type="button" data-row-id={`other:${agent.pane_id}`} tabIndex={agentRoving.tabIndexFor(`other:${agent.pane_id}`)} disabled={busy} onClick={() => void onUnmanagedTerminal(agent.pane_id).catch(cause => setTerminalError(`Could not open terminal. ${cause instanceof Error ? cause.message : "Check status."}`))}>Open {agent.agent_name} terminal</button><span>{agent.workspace_label} · {agent.tab_label} · Observed Herdr: {agent.agent_status ?? "unknown"}</span></div>) : <p>No other agents observed.</p> : <p>Other agents are unobserved while the connection is unavailable.</p>}</div></details>
          <details open={scope.disclosures.history} onToggle={event => disclosures("history", event.currentTarget.open)}><summary>History</summary><h3>Earlier</h3>{history.length ? <ul className="supervisor-history">{history.map(event => <li key={event.id}><details><summary><strong>{event.label}{event.stale ? " · stale evidence" : ""}</strong><span className="supervisor-history-summary">{event.summary.split("\n")[0]}</span><time dateTime={event.at}>{new Date(event.at).toLocaleString()}</time></summary><p className="supervisor-exact-text">{event.detail}</p></details></li>)}</ul> : <p>No recorded history in this scope.</p>}</details>
          <details open={scope.disclosures.diagnostics} onToggle={event => disclosures("diagnostics", event.currentTarget.open)}><summary>Diagnostics</summary><p>Canonical task source · {snapshot.board?.path ?? "No selected task document"}</p>{snapshot.board?.unidentified_items ? <p>{snapshot.board.unidentified_items} task-file checklist items need identity markers. <button type="button" disabled={busy || !connected} onClick={() => void mutateResult({ action: "tasks_assign_ids", root_id: snapshot.board!.root_id, expected_doc_revision: snapshot.board!.doc_revision })}>Identify task-file items</button></p> : null}{snapshot.board?.diagnostics.map((diagnostic, index) => <p className="supervisor-error" key={index}>{diagnostic.message}</p>)}{rootRuns.map(run => <RunDiagnostics key={run.run_id} run={run} snapshot={snapshot} />)}<details><summary>Fresh runtime and transaction records</summary><pre className="supervisor-plan">{JSON.stringify({ runtime: snapshot.runtime, intents: snapshot.intents, assignment_intents: snapshot.assignment_intents }, null, 2)}</pre></details></details>
          {closedRoots.length ? <details open={scope.disclosures.archive} onToggle={event => disclosures("archive", event.currentTarget.open)}><summary>Closed tracking · {closedRoots.length}</summary><p>Tasks and history remain available. Closing tracking did not kill agents or remove resources.</p>{closedRoots.map(summary => <button type="button" disabled={busy || !!dialog} key={summary.root_id} onClick={() => setRootId(summary.root_id)}>View {summary.label} tasks and history</button>)}{root?.stage === "closed" && openRoots.length ? <button type="button" disabled={busy} onClick={() => setRootId(openRoots[0].root_id)}>Return to {openRoots[0].label}</button> : null}</details> : null}
        </div>
      </>}
      {focusNotice ? <p className="supervisor-muted" role="status">{focusNotice}</p> : null}
    </div></div>
    {dialog && snapshot && active ? <SupervisorDialogs dialog={dialog} snapshot={snapshot} spaces={runtimeLive ? session?.spaces ?? [] : []} startDraft={startDraft.current} changed={changed} busy={busy} available={connected && (dialog.mode === "edit" || dialog.mode === "close" || runtimeLive && snapshot.runtime.status === "fresh")} mutateResult={mutateResult} onStarted={started} onEdited={taskId => { scope.edits.delete(taskId); changed(); }} onStartUnconfirmed={() => setStartUnknown(true)} onClose={() => setDialog(null)} /> : null}
  </section>;
}
