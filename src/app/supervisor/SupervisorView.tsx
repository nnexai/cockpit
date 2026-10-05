import { useEffect, useRef, useState } from "react";
import type { CockpitClient } from "../../client/CockpitClient";
import type { OrchestrationSnapshot, Run, SessionSnapshotResponse, Subagent, TaskLane, TaskView } from "../../protocol/generated/v1";
import { useRovingList } from "../sidebar/useRovingList";
import { SupervisorActions } from "./SupervisorActions";
import { SupervisorDialogs } from "./SupervisorDialogs";
import { useSupervisor } from "./useSupervisor";
import "./supervisor.css";

type Selection = { taskId?: string; runId?: string; subagentId?: string; unmanagedPaneId?: string; activityRowId?: string };
type DialogState = { mode: "start" | "create" | "edit" | "propose"; task: TaskView | null };
type ForestRow = { key: string; depth: number; run: Run; subagent: Subagent | null };
const LANES: { id: TaskLane; label: string }[] = [{ id: "queued", label: "Queued" }, { id: "setup", label: "Setup" }, { id: "ready", label: "Ready for permission" }, { id: "working", label: "Working" }, { id: "review", label: "Review" }];

export function supervisorForest(snapshot: OrchestrationSnapshot, includeSubagents: boolean): ForestRow[] {
  const rows: ForestRow[] = [];
  const visited = new Set<string>();
  const appendRun = (run: Run, depth: number) => {
    if (visited.has(run.run_id)) return;
    visited.add(run.run_id);
    rows.push({ key: run.run_id, depth, run, subagent: null });
    if (includeSubagents) {
      const seen = new Set<string>();
      const children = snapshot.subagents.filter(agent => agent.run_id === run.run_id);
      const appendSubagent = (agent: Subagent, level: number) => {
        if (seen.has(agent.subagent_id)) return;
        seen.add(agent.subagent_id);
        rows.push({ key: `${run.run_id}:${agent.subagent_id}`, depth: level, run, subagent: agent });
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


export function SupervisorView({ client, sessionId, session, runtimeLive, active, startToken, navigationError, onClose, onTerminal, onUnmanagedTerminal, onModalChange }: {
  client: CockpitClient; sessionId: string; session: SessionSnapshotResponse | null; runtimeLive: boolean; active: boolean; startToken: number;
  navigationError: string | null;
  onClose(): void; onTerminal(run: Run, snapshot: OrchestrationSnapshot): Promise<void>; onUnmanagedTerminal(paneId: string): Promise<void>; onModalChange(open: boolean): void;
}) {
  const [rootId, setRootId] = useState<string | null>(null);
  const { snapshot, error, connected, busy, mutate, refresh } = useSupervisor(client, sessionId, rootId, active);
  const [view, setView] = useState<"board" | "agents" | "activity">("board");
  const [selection, setSelection] = useState<Selection>({});
  const [dialog, setDialog] = useState<DialogState | null>(null);
  const [subagents, setSubagents] = useState(true);
  const [blockedOnly, setBlockedOnly] = useState(false);
  const [detailOpen, setDetailOpen] = useState(true);
  const [terminalError, setTerminalError] = useState<string | null>(null);
  const seenStart = useRef(0);
  const root = useRef<HTMLElement>(null);
  const modalVisible = dialog !== null && active && snapshot !== null;
  useEffect(() => { onModalChange(modalVisible); return () => onModalChange(false); }, [modalVisible, onModalChange]);
  useEffect(() => {
    if (startToken > seenStart.current) { seenStart.current = startToken; setDialog({ mode: "start", task: null }); }
  }, [startToken]);
  useEffect(() => { if (active) root.current?.querySelector<HTMLButtonElement>("[role=tab][aria-selected=true]")?.focus(); }, [active]);
  useEffect(() => { if (!subagents) setSelection(current => ({ ...current, subagentId: undefined })); }, [subagents]);
  const tasks = snapshot?.board?.tasks ?? [];
  const task = tasks.find(item => item.task.task_id === selection.taskId) ?? null;
  const run = snapshot?.runs.find(item => item.run_id === (selection.runId ?? task?.current_run_id)) ?? null;
  const subagent = snapshot?.subagents.find(item => item.run_id === run?.run_id && item.subagent_id === selection.subagentId) ?? null;
  const unmanagedAgents = snapshot?.runtime.status === "fresh" && connected && runtimeLive
    ? snapshot.unmanaged_agents.map(agent => ({ pane_id: agent.pane_id, space_id: agent.workspace_id, tab_id: agent.tab_id, name: agent.agent_name, status: agent.agent_status ?? "unknown", space_label: agent.workspace_label, tab_label: agent.tab_label })) : [];
  const unmanaged = unmanagedAgents.find(agent => agent.pane_id === selection.unmanagedPaneId);
  const runtime = snapshot?.runtime;
  const observations = runtime?.status === "fresh" && connected && runtimeLive ? runtime.runs : [];
  const observe = (runId: string | null | undefined) => observations.find(item => item.run_id === runId);
  const selectedSpaceId = observe(run?.run_id)?.workspace_id ?? unmanaged?.space_id;
  const forest = snapshot ? supervisorForest(snapshot, subagents) : [];
  const attention = snapshot?.attention ?? [];
  const selectItem = (next: Selection) => {
    const selectedRun = snapshot?.runs.find(item => item.run_id === next.runId);
    if (selectedRun && snapshot?.board?.root_id !== selectedRun.root_id) setRootId(selectedRun.root_id);
    setSelection(next);
    setTerminalError(null);
  };
  const openTask = (item: TaskView) => { selectItem({ taskId: item.task.task_id }); setDetailOpen(true); setTerminalError(null); };
  const navigate = async (item: Run) => {
    if (!snapshot || !connected || !runtimeLive) { setTerminalError("Herdr observation is unavailable. Reconnect before navigating."); return; }
    try { setTerminalError(null); await onTerminal(item, snapshot); } catch (failure) { setTerminalError(failure instanceof Error ? failure.message : "Could not open terminal"); }
  };
  const history = snapshot ? [
    ...snapshot.messages.map(message => ({ key: `message:${message.message_id}`, runId: message.from.type === "run" ? message.from.run_id : message.to_run_id, taskId: undefined as string | undefined, label: `${message.kind.replaceAll("_", " ")} · ${message.stage}${message.stale ? " · stale evidence" : ""}`, text: message.text, at: message.created_at })),
    ...snapshot.runs.flatMap(item => item.grants.map(grant => ({ key: grant.grant_id, runId: item.run_id, taskId: item.task_id ?? undefined, label: `${grant.scope} granted · you (${grant.origin})`, text: `Plan ${grant.plan_revision}`, at: grant.granted_at }))),
    ...snapshot.runs.flatMap(item => item.annotations.map((note, index) => ({ key: `${item.run_id}:note:${index}`, runId: item.run_id, taskId: item.task_id ?? undefined, label: `Note · ${note.by.type}`, text: note.text, at: note.at }))),
    ...snapshot.subagents.map(agent => ({ key: `${agent.run_id}:${agent.subagent_id}:update`, runId: agent.run_id, taskId: undefined, label: `Subagent ${agent.status} · OMP events`, text: `${agent.label}: ${agent.summary ?? "No summary reported"}`, at: agent.updated_at })),
  ].sort((a, b) => b.at.localeCompare(a.at)) : [];
  const rowIds = view === "board"
    ? LANES.flatMap(lane => tasks.filter(item => item.lane === lane.id).map(item => item.task.task_id))
    : view === "agents"
      ? [...forest.map(row => row.key), ...unmanagedAgents.map(agent => `unmanaged:${agent.pane_id}`)]
      : [...attention.map((item, index) => `${item.kind}:${item.run_id}:${index}`), ...history.map(item => item.key)];
  const selectedRowId = view === "board"
    ? selection.taskId ?? null
    : view === "agents"
      ? selection.unmanagedPaneId ? `unmanaged:${selection.unmanagedPaneId}` : run ? subagent ? `${run.run_id}:${subagent.subagent_id}` : run.run_id : null
      : selection.activityRowId ?? null;
  const { listRef, listProps, tabIndexFor } = useRovingList({
    rowIds,
    selectedId: selectedRowId,
    onEscape: () => { onClose(); return true; },
  });
  return <section ref={root} className="supervisor-view" hidden={!active} aria-label="Supervisor" onKeyDown={event => {
    const target = event.target as HTMLElement;
    if (dialog || target.closest("input,textarea,select,[contenteditable=true]")) return;
    if (["1", "2", "3"].includes(event.key) && !event.ctrlKey && !event.metaKey && !event.altKey) { event.preventDefault(); setView(event.key === "1" ? "board" : event.key === "2" ? "agents" : "activity"); }
  }}>
    <header className="supervisor-header"><strong>Supervisor</strong><label className="supervisor-root-label">Root <select aria-label="Supervisor root" disabled={busy || dialog !== null} value={rootId ?? snapshot?.board?.root_id ?? ""} onChange={event => { setRootId(event.target.value || null); selectItem({  }); }}><option value="">{snapshot?.roots.length ? "Select root" : "No supervisor roots"}</option>{snapshot?.roots.map(item => <option key={item.root_id} value={item.root_id}>{item.label}</option>)}</select></label><button type="button" disabled={busy || !snapshot} onClick={() => setDialog({ mode: "start", task: null })}>Start supervisor…</button><button type="button" onClick={onClose} aria-label="Close Supervisor">Close</button></header>
    <div className="supervisor-tabs" role="tablist" aria-label="Supervisor views" onKeyDown={event => { if (event.key !== "ArrowLeft" && event.key !== "ArrowRight") return; event.preventDefault(); const views = ["board", "agents", "activity"] as const; const next = views[(views.indexOf(view) + (event.key === "ArrowRight" ? 1 : 2)) % 3]; setView(next); event.currentTarget.querySelector<HTMLButtonElement>(`[data-view=${next}]`)?.focus(); }}>
      {(["board", "agents", "activity"] as const).map(item => <button type="button" key={item} role="tab" data-view={item} aria-selected={view === item} aria-controls={`supervisor-${item}`} tabIndex={view === item ? 0 : -1} onClick={() => setView(item)}>{item === "board" ? "Board" : item === "agents" ? "Agents" : "Activity"} <span>{item === "board" ? tasks.length : item === "agents" ? forest.length + unmanagedAgents.length : attention.length}</span></button>)}
      <span className="supervisor-provenance" role="status">{!connected ? "Disconnected · retained state, runtime unobserved" : runtime?.status === "unavailable" || !runtimeLive ? "Herdr unavailable · no absence inferred" : `Herdr observed ${runtime?.status === "fresh" ? new Date(runtime.observed_at).toLocaleTimeString() : ""}`}</span>
    </div>
    <div className="supervisor-error" role={error || terminalError || navigationError ? "alert" : "status"}>{terminalError ?? navigationError ?? error ?? (runtime?.status === "unavailable" ? runtime.error.message : "")}{!connected || error ? <button type="button" onClick={refresh}>Refresh observation</button> : null}</div>
    {!snapshot ? <div className="supervisor-empty">{error ? "Supervisor data unavailable. Your current terminal is unchanged." : "Reading supervisor state…"}</div> : <div className="supervisor-body">
      <div className="supervisor-list-area" ref={listRef} {...listProps}>
      {view === "board" ? <section id="supervisor-board" role="tabpanel" aria-label="Task board"><div className="supervisor-filters"><button type="button" disabled={busy || !snapshot.board} onClick={() => setDialog({ mode: "create", task: null })}>New task…</button><label><input type="checkbox" checked={blockedOnly} onChange={event => setBlockedOnly(event.target.checked)} /> Blocked only (dims others)</label></div>{snapshot.board ? <><p className="supervisor-path">Canonical Markdown · {snapshot.board.path}</p>{snapshot.board.unidentified_items > 0 ? <p>{snapshot.board.unidentified_items} unmarked checklist items <button type="button" disabled={busy} onClick={() => void mutate({ action: "tasks_assign_ids", root_id: snapshot.board!.root_id, expected_doc_revision: snapshot.board!.doc_revision })}>Assign task IDs</button></p> : null}{snapshot.board.diagnostics.map((diagnostic, index) => <p role="alert" key={index}>{diagnostic.code}: {diagnostic.message}</p>)}<nav className="supervisor-lane-jumps" aria-label="Board lanes">{LANES.map(lane => <button type="button" key={lane.id} onClick={() => root.current?.querySelector(`#supervisor-lane-${lane.id}`)?.scrollIntoView({ block: "nearest" })}>{lane.label} {tasks.filter(item => item.lane === lane.id).length}</button>)}</nav><div className="supervisor-lanes">{LANES.map(lane => <section className="supervisor-lane" id={`supervisor-lane-${lane.id}`} key={lane.id}><h2>{lane.label} <span>{tasks.filter(item => item.lane === lane.id).length}</span></h2>{tasks.filter(item => item.lane === lane.id).map(item => {
        const worker = snapshot.runs.find(candidate => candidate.run_id === item.current_run_id);
        const observed = observe(worker?.run_id);
        const flags = attention.filter(flag => flag.task_id === item.task.task_id || Boolean(worker && flag.run_id === worker.run_id));
        const blocked = flags.some(flag => ["needs_input", "runtime_blocked", "dispatch_unknown", "intent_conflict"].includes(flag.kind));
        return <button type="button" data-row-id={item.task.task_id} key={item.task.task_id} className={`supervisor-row${task?.task.task_id === item.task.task_id ? " is-selected" : ""}${blockedOnly && !blocked ? " is-dimmed" : ""}`} tabIndex={tabIndexFor(item.task.task_id)} onClick={() => openTask(item)}><strong>{item.task.title}</strong><span className="supervisor-row-tags">{flags.map((flag, index) => <span key={index}>{flag.kind.replaceAll("_", " ")}</span>)}{item.task.diagnostic ? <span>{item.task.diagnostic}</span> : null}{worker?.result && observed?.agent_status === "working" ? <span>≠ Reported result · Herdr still working</span> : null}{!worker?.result && observed?.agent_status === "done" ? <span>Herdr done · no result report</span> : null}</span><span className="supervisor-location">{observed?.workspace_label ?? worker?.setup?.checkout_path ?? "No assigned location"}</span><span>Reported · {worker?.last_report?.summary ?? "No worker report"}</span><span>Herdr · {observed ? `${observed.agent_status ?? "unknown"} (${observed.presence})` : "unobserved"}</span></button>;
      })}</section>)}</div><details className="supervisor-accepted"><summary>Accepted · {tasks.filter(item => item.lane === "accepted").length}</summary>{tasks.filter(item => item.lane === "accepted").map(item => <button type="button" className="supervisor-row" key={item.task.task_id} onClick={() => openTask(item)}>{item.task.title}</button>)}</details></> : <p>{snapshot.roots.length ? "Select a root to read its canonical tasks." : "Start a supervisor to create tasks and propose workers. Starting opens an interactive agent; work execution remains a separate grant."}</p>}</section> : null}
      {view === "agents" ? <section id="supervisor-agents" role="tabpanel" aria-label="Agents forest"><div className="supervisor-filters"><label><input type="checkbox" checked={subagents} onChange={event => setSubagents(event.target.checked)} /> Subagents</label></div><div role="tree" aria-label="Managed agents">{forest.map(row => {
        const observed = observe(row.run.run_id);
        const selected = row.subagent ? subagent?.subagent_id === row.subagent.subagent_id && run?.run_id === row.run.run_id : run?.run_id === row.run.run_id && !subagent;
        return <button type="button" data-row-id={row.key} role="treeitem" aria-level={row.depth + 1} aria-selected={selected} key={row.key} tabIndex={tabIndexFor(row.key)} style={{ paddingInlineStart: `${12 + Math.min(row.depth, 8) * 16}px` }} className={`supervisor-row supervisor-agent${row.subagent ? " is-subagent" : ""}${selected ? " is-selected" : ""}`} onClick={() => { selectItem({ runId: row.run.run_id, taskId: row.run.task_id ?? undefined, subagentId: row.subagent?.subagent_id }); setDetailOpen(true); }}><strong>{row.subagent?.label ?? row.run.label} <small>{row.subagent?.role ?? (row.subagent ? "subagent" : row.run.kind)}</small></strong><span className={`supervisor-location${selectedSpaceId && observed?.workspace_id === selectedSpaceId ? " is-shared" : ""}`}>{row.subagent ? `in ${row.run.label} · no pane` : observed ? `${observed.workspace_label ?? "Space"} · ${observed.tab_label ?? "tab"}` : "Location unobserved"}</span><span>Reported · {row.subagent?.summary ?? row.run.last_report?.summary ?? "No report"}</span><span>{row.subagent ? `OMP events · ${row.subagent.status}` : `Herdr · ${observed?.agent_status ?? "unobserved"}`} · {row.subagent?.updated_at ?? row.run.updated_at}</span></button>;
      })}</div><h2>Unmanaged current Herdr agents</h2>{runtime?.status === "fresh" && connected && runtimeLive ? unmanagedAgents.length ? unmanagedAgents.map(agent => <button type="button" data-row-id={`unmanaged:${agent.pane_id}`} tabIndex={tabIndexFor(`unmanaged:${agent.pane_id}`)} key={agent.pane_id} className={`supervisor-row${selection.unmanagedPaneId === agent.pane_id ? " is-selected" : ""}`} onClick={() => { selectItem({ unmanagedPaneId: agent.pane_id }); setDetailOpen(true); }}><strong>{agent.name} <small>unmanaged root</small></strong><span className={`supervisor-location${selectedSpaceId === agent.space_id ? " is-shared" : ""}`}>{agent.space_label} · {agent.tab_label}</span><span>Herdr · {agent.status} · no Cockpit task relationship</span></button>) : <p>No unmanaged agents observed.</p> : <p>Herdr is unavailable. Unmanaged agent presence is not inferred.</p>}</section> : null}
      {view === "activity" ? <section id="supervisor-activity" role="tabpanel" aria-label="Activity"><h2>Needs you · {attention.length}</h2>{attention.length ? attention.map((item, index) => <button type="button" data-row-id={`${item.kind}:${item.run_id}:${index}`} tabIndex={tabIndexFor(`${item.kind}:${item.run_id}:${index}`)} className="supervisor-row" key={`${item.kind}:${item.run_id}:${index}`} onClick={() => { selectItem({ runId: item.run_id ?? undefined, taskId: item.task_id ?? undefined, activityRowId: `${item.kind}:${item.run_id}:${index}` }); setDetailOpen(true); }}><strong>{item.kind.replaceAll("_", " ")}</strong><span>{snapshot.runs.find(candidate => candidate.run_id === item.run_id)?.label ?? tasks.find(candidate => candidate.task.task_id === item.task_id)?.task.title ?? "Task conflict"}</span><time>{item.since}</time></button>) : <p>No unresolved decisions in current state.</p>}<h2>Earlier</h2>{history.map(item => <button type="button" data-row-id={item.key} tabIndex={tabIndexFor(item.key)} key={item.key} className="supervisor-row" onClick={() => { selectItem({ runId: item.runId, taskId: item.taskId, activityRowId: item.key }); setDetailOpen(true); }}><strong>{item.label}</strong><span>{item.text}</span><time>{item.at}</time></button>)}</section> : null}
      </div>
      <aside className={`supervisor-detail-rail${detailOpen ? " is-open" : ""}`} aria-label="Selected item"><button type="button" className="supervisor-detail-heading" aria-expanded={detailOpen} onClick={() => setDetailOpen(value => !value)}>{task?.task.title ?? subagent?.label ?? run?.label ?? unmanaged?.name ?? "Details"} · {detailOpen ? "Collapse" : "Expand"}</button>{detailOpen && active ? unmanaged ? <div className="supervisor-detail"><h2>{unmanaged.name}</h2><p>Unmanaged root · Herdr observed {runtimeLive ? unmanaged.status : "unobserved"}. No supervisor relationship or copied task assignment.</p><button type="button" disabled={!runtimeLive} onClick={() => void onUnmanagedTerminal(unmanaged.pane_id).catch(failure => setTerminalError(failure instanceof Error ? failure.message : "Navigation failed"))}>Go to terminal</button></div> : task || run ? <SupervisorActions key={`${run?.run_id ?? task?.task.task_id}:${subagent?.subagent_id ?? "main"}`} snapshot={connected && runtimeLive ? snapshot : { ...snapshot, runtime: { status: "unavailable", error: { code: "runtime_unobserved", message: "Disconnected · previous observations are not current" } } }} run={run} task={task} subagent={subagent} busy={busy || !connected} error={error ?? terminalError} mutate={mutate} onTerminal={navigate} onEditTask={item => setDialog({ mode: "edit", task: item })} onPropose={item => setDialog({ mode: "propose", task: item })} /> : <p className="supervisor-empty">Select a task, agent or activity item. Selection stays local; only Go to terminal requests Herdr focus.</p> : null}</aside>
    </div>}
    {dialog && snapshot && active ? <SupervisorDialogs key={`${dialog.mode}:${dialog.task?.task.task_id ?? snapshot.board?.root_id ?? "root"}`} mode={dialog.mode} snapshot={snapshot} task={dialog.task} client={client} spaces={runtimeLive ? session?.spaces ?? [] : []} busy={busy || !connected} error={error} mutate={mutate} onClose={() => setDialog(null)} /> : null}
  </section>;
}
