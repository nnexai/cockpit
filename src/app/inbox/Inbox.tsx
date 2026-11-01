import { useEffect, useRef, useState } from "react";
import { CockpitClientError, type CockpitClient } from "../../client/CockpitClient";
import type { InboxLocation, InboxQuestion, InboxText, InboxThread, InboxView, OrchestrationAction, SessionSnapshotResponse, UnmanagedAgent } from "../../protocol/generated/v1";
import type { AgentAttentionState } from "../session/agentAttention";
import { agentContext, type AgentContext } from "../session/agentContext";
import { deriveAgentQueue } from "../sidebar/agentAttentionModel";
import { relativeTime } from "../library/libraryState";
import { UiIcon, type UiIconName } from "../UiIcon";
import "./inbox.css";

export type { InboxLocation } from "../../protocol/generated/v1";
export type InboxProps = { client: CockpitClient; sessionId: string | null; active: boolean; currentSpace: { id: string; label: string } | null; attention: AgentAttentionState; snapshot: SessionSnapshotResponse | null; onOpenLocation(location: InboxLocation): Promise<void>; onAttentionChange?(value: { count: number; preview: string | null }): void };

type Pending = { action: OrchestrationAction; text: string; store: string; key: string };
const source = (value: InboxText) => value.source.kind === "owner_card" ? `AI synthesis · ${value.source.harness}` : value.source.kind === "owner" ? `Owner · ${value.source.harness}` : value.source.kind === "worker" ? "Worker claim" : value.source.kind === "user" ? "Your request" : "Recorded fact";
const questionKey = (question: InboxQuestion) => JSON.stringify([question.run_id, question.question_message_id]);
const uuid = /^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$/i;
const directBusyCodes: Record<string, true> = { agent_not_idle: true, herdr_prompt_deferred: true, agent_blocked: true, agent_not_ready: true };
const ownerLabel = (view: InboxView | null, runId: string) => {
  const owner = view?.owners.find(o => o.root_id === runId);
  return owner?.label.trim() && !uuid.test(owner.label.trim()) ? owner.label : owner?.harness ? `${owner.harness} owner` : "Owner";
};
function questionLabels(view: InboxView | null, questions: InboxQuestion[]) {
  const labels = questions.map(q => ownerLabel(view, q.run_id));
  const withHarness = labels.map((label, i) => labels.filter(x => x === label).length > 1 ? `${label} · ${view?.owners.find(o => o.root_id === questions[i].run_id)?.harness || "owner"}` : label);
  return withHarness.map((label, i) => withHarness.filter(x => x === label).length > 1 ? `${label} · …${questions[i].run_id.slice(-4)}` : label);
}
function tone(thread: InboxThread): { kind: string; icon: UiIconName; label: string; action?: string } {
  if (thread.state === "underway") return { kind: "work", icon: "work", label: "Working" };
  if (thread.state === "handled") return { kind: "done", icon: "check-circle", label: "Done" };
  if (thread.need === "answer") return { kind: "review", icon: "eye", label: "Result ready", action: "Review" };
  return { kind: "decision", icon: "help", label: thread.need === "decision" ? "Needs your decision" : "Needs you", action: "Answer" };
}
const srcKind = (value: InboxText) => value.source.kind === "owner_card" ? "ai" : value.source.kind;
const srcLabel = (value: InboxText) => ({ ai: "AI summary", owner: "Owner", worker: "Worker claim", user: "Your request", recorded: "Recorded" })[srcKind(value)];
function Time({ value }: { value: string | null | undefined }) {
  const label = relativeTime(value, Date.now());
  return label && value ? <time dateTime={value} title={new Date(value).toLocaleString()}>{label}</time> : null;
}

/** Each session remains mounted so switching sessions retains its local drafts and scroll positions. */
export function Inbox(props: InboxProps) {
  const [sessions, setSessions] = useState<string[]>(() => props.sessionId ? [props.sessionId] : []);
  useEffect(() => { if (props.sessionId) setSessions(s => s.includes(props.sessionId!) ? s : [...s, props.sessionId!]); else props.onAttentionChange?.({ count: 0, preview: null }); }, [props.sessionId, props.onAttentionChange]);
  return <div className="inbox-root" hidden={!props.active}>{!props.sessionId && <p className="inbox-empty">Select a session to see your inbox.</p>}{sessions.map(id => <SessionInbox key={id} {...props} sessionId={id} current={props.sessionId === id} visible={props.active && props.sessionId === id} />)}</div>;
}

function SessionInbox({ client, sessionId, visible, current, currentSpace, onOpenLocation, onAttentionChange, attention, snapshot }: Omit<InboxProps, "active" | "sessionId"> & { sessionId: string; visible: boolean; current: boolean }) {
  const [view, setView] = useState<InboxView | null>(null);
  const [error, setError] = useState("");
  const [loadError, setLoadError] = useState("");
  const [selected, setSelected] = useState<string | null>(null);
  const [mobileDetail, setMobileDetail] = useState(false);
  const [selectedAgent, setSelectedAgent] = useState<{ agent: UnmanagedAgent; terminalId: string | null } | null>(null);
  const [drafts, setDrafts] = useState<Record<string, string>>({});
  const [selectedQuestion, setSelectedQuestion] = useState("");
  const [owner, setOwner] = useState("");
  const [taskId, setTaskId] = useState("");
  const [attachedSpace, setAttachedSpace] = useState<{ id: string; label: string } | null>(null);
  const [repository, setRepository] = useState("");
  const [branch, setBranch] = useState("");
  const [base, setBase] = useState("");
  const [pending, setPending] = useState<Pending | null>(null);
  const [busy, setBusy] = useState(false);
  const [directUncertain, setDirectUncertain] = useState<Record<string, boolean>>({});
  const [directStale, setDirectStale] = useState(false);
  const [directOutcome, setDirectOutcome] = useState<{ pane: string; target: string; kind: "submitted" | "busy"; name: string } | null>(null);
  const selectionVersion = useRef(0);
  const wake = useRef<(() => void) | null>(null);
  const refreshVersion = useRef(0);
  const revisionFloor = useRef(0);
  const pendingRead = useRef<Promise<InboxView> | null>(null);
  const mounted = useRef(true);
  const lock = useRef(false);
  const liveScope = useRef({ current, snapshot });
  liveScope.current = { current, snapshot };
  useEffect(() => { mounted.current = true; return () => { mounted.current = false; }; }, []);
  useEffect(() => { if (!current) { selectionVersion.current++; setDirectOutcome(null); } }, [current]);
  useEffect(() => {
    if (!current) return;
    let live = true, timer: number | undefined;
    const sleep = () => new Promise<void>(resolve => { timer = window.setTimeout(resolve, 1500); });
    const loop = async () => {
      let floor = revisionFloor.current;
      while (live) {
        try {
          const started = refreshVersion.current;
          const read = pendingRead.current ?? client.orchestrationInbox({ session_id: sessionId });
          pendingRead.current = read;
          let next: InboxView;
          try { next = await read; } finally { if (pendingRead.current === read) pendingRead.current = null; }
          if (!live) return;
          if (next.session_id !== sessionId || next.revision < Math.max(floor, revisionFloor.current)) { await sleep(); continue; }
          if (started !== refreshVersion.current) continue;
          floor = next.revision; revisionFloor.current = floor; setView(next); setLoadError("");
          let revision = next.revision, token = next.tasks_token;
          while (live) {
            const controller = new AbortController();
            const interrupt = () => controller.abort();
            wake.current = interrupt;
            try {
              const result = await client.orchestrationWait({ root_id: null, after_revision: revision, after_tasks_token: token, timeout_ms: 30_000 }, controller.signal);
              if (!live) return;
              if (controller.signal.aborted) break;
              revision = result.revision; token = result.tasks_token; floor = Math.max(floor, revision);
              if (result.changed) break;
            } catch (error) {
              if (!live) return;
              if (controller.signal.aborted) break;
              throw error;
            } finally {
              if (wake.current === interrupt) wake.current = null;
            }
          }
        } catch (e) { if (!live) return; setLoadError(e instanceof Error ? e.message : "Inbox unavailable"); await sleep(); }
      }
    };
    void loop();
    return () => { live = false; wake.current?.(); clearTimeout(timer); };
  }, [client, sessionId, current]);
  useEffect(() => { let live = true; void client.orchestrationLiaisonLookup({ session_id: sessionId, selected_root_id: null }).then(result => { if (live && result.status === "resolved" && result.preferred_root_id) setOwner(current => current || result.preferred_root_id!); }).catch(() => {}); return () => { live = false; }; }, [client, sessionId]);
  const runtimeIdentity = snapshot?.session_id === sessionId ? JSON.stringify([snapshot.server_instance, snapshot.spaces?.map(space => [space.id, space.label]), snapshot.tabs?.map(tab => [tab.id, tab.label]), snapshot.panes.map(pane => [pane.id, pane.terminal_id, pane.space_id, pane.tab_id, pane.agent, pane.agent_status]), snapshot.agents.map(agent => [agent.pane_id, agent.name, agent.status, agent.state_change_seq, agent.native_session_id])]) : null;
  const previousRuntime = useRef<string | null>(null);
  useEffect(() => {
    if (!current) return;
    const previous = previousRuntime.current;
    previousRuntime.current = runtimeIdentity;
    if (previous !== null && previous !== runtimeIdentity) { refreshVersion.current++; wake.current?.(); }
  }, [current, runtimeIdentity]);
  const queue = snapshot?.session_id === sessionId ? deriveAgentQueue(snapshot, attention.response, attention.availability, true).visible : [];
  const matchingAgent = (agent: UnmanagedAgent) => {
    const target = agent.expected_target;
    if (!target || !target.terminal_id || target.pane_id !== agent.pane_id || target.workspace_id !== agent.workspace_id || target.tab_id !== agent.tab_id || (target.agent_name !== null && target.agent_name !== agent.agent_name)) return undefined;
    return queue.find(row => row.agent.pane_id === agent.pane_id && row.agent.space_id === agent.workspace_id && row.agent.tab_id === agent.tab_id && row.agent.name === target.agent_kind && row.terminalId === target.terminal_id && (!target.native_session_id || !row.agent.native_session_id || target.native_session_id === row.agent.native_session_id));
  };
  const relevant = (agent: UnmanagedAgent) => !!matchingAgent(agent)?.flag || (!agent.expected_target && queue.some(row => row.agent.pane_id === agent.pane_id && row.agent.name === agent.agent_name && row.terminalId && row.flag));
  const contextFor = (agent: UnmanagedAgent) => {
    const target = agent.expected_target;
    const row = matchingAgent(agent);
    if (!target || !row || (target.native_session_id && (target.native_session_id !== row.agent.native_session_id || target.terminal_id !== row.agent.terminal_id))) return null;
    const agentForDisplay = target.native_session_id ? row.agent : { ...row.agent, session_title: undefined, working_description: undefined, idle_summary: undefined };
    return agentContext(agentForDisplay, [agent.workspace_label, agent.tab_label, `${agent.workspace_label} · ${agent.tab_label}`]);
  };
  const count = (view?.threads.filter(t => t.state === "needs_you").length ?? 0) + (view?.unmanaged_agents.filter(relevant).length ?? 0);
  const preview = view?.threads.find(t => t.state === "needs_you")?.title.text ?? view?.unmanaged_agents.find(relevant)?.agent_name ?? null;
  useEffect(() => { if (current) onAttentionChange?.({ count, preview }); }, [current, count, preview, onAttentionChange]);
  const thread = view?.threads.find(t => t.thread_id === selected);
  const direct = selectedAgent?.agent;
  const matchedDirect = direct ? matchingAgent(direct) : undefined;
  const liveDirect = matchedDirect?.terminalId === selectedAgent?.terminalId ? matchedDirect : undefined;
  const key = direct ? JSON.stringify(["agent", direct.pane_id]) : selected ?? "new";
  const text = drafts[key] ?? "";
  const setText = (value: string) => { setDrafts(d => ({ ...d, [key]: value })); };
  const select = (id: string | null) => { selectionVersion.current++; setDirectOutcome(null); setError(""); setSelected(id); setSelectedAgent(null); setSelectedQuestion(""); setTaskId(""); setMobileDetail(true); };
  const selectAgent = (agent: UnmanagedAgent) => { selectionVersion.current++; setDirectOutcome(null); setError(""); setSelected(`agent:${agent.pane_id}`); setSelectedAgent({ agent: { ...agent, expected_target: agent.expected_target ? { ...agent.expected_target } : agent.expected_target }, terminalId: matchingAgent(agent)?.terminalId ?? null }); setDirectStale(false); setSelectedQuestion(""); setMobileDetail(true); };
  const latestDirect = direct && view?.unmanaged_agents.find(agent => agent.pane_id === direct.pane_id);
  const directTargetMatches = !!direct?.expected_target && !!latestDirect?.expected_target && JSON.stringify(direct.expected_target) === JSON.stringify(latestDirect.expected_target);
  const directReady = !!direct && !!liveDirect && (liveDirect.agent.status === "idle" || liveDirect.agent.status === "done") && directTargetMatches && direct.expected_target?.pane_id === direct.pane_id && direct.expected_target.workspace_id === direct.workspace_id && direct.expected_target.tab_id === direct.tab_id && direct.expected_target.terminal_id === selectedAgent?.terminalId && (direct.expected_target.agent_name === null || direct.expected_target.agent_name === direct.agent_name) && !directStale && !directUncertain[direct.pane_id];
  const directContext = direct && directTargetMatches && liveDirect ? contextFor(latestDirect!) : null;
  const visibleOutcome = directOutcome && direct && directOutcome.pane === direct.pane_id && directOutcome.target === JSON.stringify(direct.expected_target) ? directOutcome : null;
  const open = (location: InboxLocation | null) => { if (location && view?.observation.status === "fresh" && snapshot?.session_id === sessionId && (!direct || liveDirect)) void onOpenLocation(location).catch(e => setError(e instanceof Error ? e.message : "Could not open session")); };
  const execute = async (operation: Pending) => {
    if (lock.current || !view || operation.store !== view.state_store_id) { if (view && operation.store !== view.state_store_id) setError("Session state changed. The original request has been retained and cannot be retried here."); return; }
    lock.current = true; setBusy(true); setError(""); setPending(operation);
    try {
      const response = await client.orchestrationMutate({ session_id: sessionId, expected_revision: null, action: operation.action });
      if (!mounted.current) return;
      revisionFloor.current = Math.max(revisionFloor.current, response.revision);
      if (response.result.result !== "message" || (operation.action.action === "thread_handoff" && response.result.to_run_id !== operation.action.to_run_id) || (operation.action.action === "message_send" && response.result.to_run_id !== operation.action.to_run_id)) throw new Error("Delivery receipt did not match the request");
      if (response.result.result === "message" && response.result.stale) { setPending(null); setError("The question or task changed. Your draft is preserved; review the current work before sending again."); }
      else { setPending(null); setDrafts(d => d[operation.key] === operation.text ? { ...d, [operation.key]: "" } : d); if (operation.action.action === "thread_handoff") setSelected(operation.action.thread_id); }
      refreshVersion.current++; wake.current?.();
    } catch (e) { if (mounted.current) { setError(`${e instanceof Error ? e.message : "Delivery was not confirmed"}. The original request is retained; retry it without changing its identity.`); refreshVersion.current++; wake.current?.(); } }
    finally { lock.current = false; if (mounted.current) setBusy(false); }
  };
  const send = async () => {
    if (!view || !current || snapshot?.session_id !== sessionId || !text.trim() || busy || pending || lock.current) return;
    if (selected && !thread && !direct) { setError("This work is no longer available. Your draft is preserved."); return; }
    if (direct) {
      if (!directReady || visibleOutcome || !direct.expected_target || view.observation.status !== "fresh") return;
      const attempt = selectionVersion.current;
      const outcome = { pane: direct.pane_id, target: JSON.stringify(direct.expected_target), name: direct.agent_name };
      lock.current = true; setBusy(true); setError(""); setDirectOutcome(null);
      try {
        await client.orchestrationAgentSend({ session_id: sessionId, reference: direct.pane_id, text, expected_target: direct.expected_target });
        if (mounted.current) {
          setDrafts(d => d[key] === text ? { ...d, [key]: "" } : d);
          if (selectionVersion.current === attempt) setDirectOutcome({ ...outcome, kind: "submitted" });
        }
      } catch (e) {
        if (mounted.current) {
          const selectedAttempt = selectionVersion.current === attempt;
          if (e instanceof CockpitClientError && e.operationCode === "stale_identity") {
            if (selectedAttempt) { setDirectStale(true); setError("The agent identity changed. Your draft is preserved; review and reselect the agent."); }
          } else if (e instanceof CockpitClientError && Object.prototype.hasOwnProperty.call(directBusyCodes, e.operationCode ?? "")) {
            if (selectedAttempt) setDirectOutcome({ ...outcome, kind: "busy" });
          } else {
            setDirectUncertain(previous => ({ ...previous, [outcome.pane]: true }));
            if (selectedAttempt) setError(`${e instanceof Error ? e.message : "Delivery not confirmed"}. Check the agent before sending again; your draft is preserved.`);
          }
        }
      } finally { lock.current = false; if (mounted.current) setBusy(false); refreshVersion.current++; wake.current?.(); }
      return;
    }
    const question = thread?.questions.find(q => questionKey(q) === selectedQuestion && q.receipt.status === "unresolved");
    let action: OrchestrationAction;
    if (selectedQuestion && !question) { setError("The selected question changed. Your draft is preserved; choose the current question before sending."); return; }
    if (question) action = { action: "message_send", message_id: crypto.randomUUID(), to_run_id: question.run_id, kind: "answer", text, in_reply_to: question.question_message_id };
    else {
      const target = thread ? (thread.owner_root_ids.length === 1 ? thread.owner_root_ids[0] : owner) : owner;
      if (!target || (thread && !thread.owner_root_ids.includes(target))) { setError("Choose the owner for this request."); return; }
      const task = taskId ? thread?.tasks.find(t => t.task_id === taskId) : undefined;
      if (taskId && !task) { setError("The selected task is no longer available. Your draft is preserved."); return; }
      let taskRevision: string | undefined;
      if (task) {
        if (task.root_id !== target) { setError("Choose the task's owner for this follow-up."); return; }
        lock.current = true; setBusy(true);
        try {
          const observed = await client.orchestrationSnapshot({ session_id: sessionId, root_id: target });
          const found = observed.board?.tasks.find(item => item.task.task_id === task.task_id);
          if (observed.session_id !== sessionId || observed.state_store_id !== view.state_store_id || observed.board?.root_id !== target || !found) throw new Error("Task identity or revision is unavailable.");
          taskRevision = found.task.task_revision;
        } catch (e) { lock.current = false; setError(e instanceof Error ? e.message : "Task revision unavailable"); setBusy(false); return; }
        lock.current = false; setBusy(false);
        if (!liveScope.current.current || liveScope.current.snapshot?.session_id !== sessionId) { setError("Session changed. Your draft is preserved."); return; }
      }
      const binding = !thread && (attachedSpace || repository.trim() || branch.trim() || base.trim()) ? { repository_id: repository.trim() || null, workspace_id: attachedSpace?.id ?? null, branch: branch.trim() || null, base_ref: base.trim() || null } : undefined;
      action = { action: "thread_handoff", message_id: crypto.randomUUID(), thread_id: thread?.thread_id ?? crypto.randomUUID(), to_run_id: target, text, ...(task && taskRevision ? { task_id: task.task_id, expected_task_revision: taskRevision } : {}), ...(binding ? { binding } : {}) };
    }
    await execute({ action, text, store: view.state_store_id, key });
  };
  const markRead = async () => { if (!thread?.unread_result_message_id || !view || view.observation.status !== "fresh" || snapshot?.session_id !== sessionId || busy || lock.current) return; lock.current = true; setBusy(true); try { await client.orchestrationMutate({ session_id: sessionId, expected_revision: null, action: { action: "thread_mark_read", thread_id: thread.thread_id, through_message_id: thread.unread_result_message_id } }); refreshVersion.current++; wake.current?.(); } catch (e) { setError(e instanceof Error ? e.message : "Could not mark read"); } finally { lock.current = false; setBusy(false); } };
  const questions = thread?.questions.filter(q => q.receipt.status === "unresolved") ?? [];
  const chosenQuestion = questions.find(q => questionKey(q) === selectedQuestion);
  const labels = questionLabels(view, questions);
  const mode = direct ? "direct" : !thread ? "new" : chosenQuestion ? "answer" : "followup";
  const recommendationIsChoice = !!thread?.recommendation && questions.some(q => q.choices.some(c => c.trim().toLowerCase() === thread.recommendation!.text.trim().toLowerCase()));
  const displayed = thread ? [thread.title, ...(!questions.length && thread.next ? [thread.next] : []), ...(thread.recommendation && !recommendationIsChoice ? [thread.recommendation] : []), ...(thread.reply && thread.reply.text !== thread.title.text ? [thread.reply] : []), ...questions.map(q => q.text)] : [];
  const ai = displayed.some(v => v.source.kind === "owner_card") || (!!thread?.recommendation && recommendationIsChoice && thread.recommendation.source.kind === "owner_card");
  const worker = displayed.some(v => v.source.kind === "worker") || (!!thread?.recommendation && recommendationIsChoice && thread.recommendation.source.kind === "worker");
  const otherThreads = view?.threads.filter(t => t.state !== "needs_you") ?? [];
  const targets = thread ? [...thread.tasks.filter(t => t.run?.location && t.run.presence === "present").map(t => ({ key: t.task_id, label: t.title, full: `Open ${t.title} · attempt ${t.run!.attempt}`, location: t.run!.location })), ...(view?.owners.filter(o => thread.owner_root_ids.includes(o.root_id) && o.location && o.availability === "available").map(o => ({ key: o.root_id, label: ownerLabel(view, o.root_id), full: `Open ${ownerLabel(view, o.root_id)}`, location: o.location })) ?? [])] : [];
  return <section className="inbox" data-detail={mobileDetail} hidden={!visible} aria-label="Inbox">
    <aside className="inbox-list" aria-label="Work needing attention"><header><h1>Inbox</h1><button type="button" className="inbox-new" onClick={() => select(null)}><UiIcon name="plus" /> New work</button></header>
      {!view && <p className="inbox-empty">{loadError || "Loading inbox…"}</p>}
      {view && loadError && <p role="status" className="inbox-note" title={loadError}><UiIcon name="alert" />Inbox unavailable.</p>}
      {view?.observation.status === "unavailable" && <p role="status" className="inbox-note" title={view.observation.error.message}><UiIcon name="alert" />Can’t see live sessions right now. Replies are paused.</p>}
      {attention.availability === "unavailable" && <p role="status" className="inbox-note"><UiIcon name="alert" />Agent attention unavailable.</p>}
      {view && !view.threads.some(t => t.state === "needs_you") && !view.unmanaged_agents.some(relevant) && <p className="inbox-empty"><UiIcon name="check-circle" />Nothing needs you right now.</p>}
      {view?.threads.filter(t => t.state === "needs_you").map(t => <ThreadRow key={t.thread_id} thread={t} selected={selected === t.thread_id} onSelect={() => select(t.thread_id)} />)}
      {view?.unmanaged_agents.filter(relevant).map(a => <AgentRow key={a.pane_id} agent={a} context={contextFor(a)} selected={selected === `agent:${a.pane_id}`} onSelect={() => selectAgent(a)} />)}
      {otherThreads.length > 0 && <details className="inbox-others"><summary><UiIcon name="right" />Other work · {otherThreads.length}</summary>{(["underway", "handled"] as const).map(state => otherThreads.some(t => t.state === state) && <div key={state}><h2>{state === "underway" ? "Underway" : "Handled"}</h2>{otherThreads.filter(t => t.state === state).map(t => <ThreadRow key={t.thread_id} thread={t} selected={selected === t.thread_id} onSelect={() => select(t.thread_id)} />)}</div>)}</details>}
      {view && view.unmanaged_agents.some(a => !relevant(a)) && <details className="inbox-others"><summary><UiIcon name="right" />Other agents</summary>{view.unmanaged_agents.filter(a => !relevant(a)).map(a => <AgentRow key={a.pane_id} agent={a} context={contextFor(a)} selected={selected === `agent:${a.pane_id}`} quiet onSelect={() => selectAgent(a)} />)}</details>}
    </aside>
    <main className="inbox-detail"><button type="button" className="inbox-back" onClick={() => setMobileDetail(false)}>‹ Inbox</button>
      {thread ? <div className="inbox-content">
        <div className="inbox-head" data-tone={tone(thread).kind}><span className="inbox-kic is-lg"><UiIcon name={tone(thread).icon} /></span><div><h2>{thread.title.text}</h2><p className="inbox-facts">{thread.project && <span>{thread.project}</span>}<Time value={thread.updated_at} /><span className="inbox-state">{tone(thread).label}</span>{ai && <span className="inbox-prov"><UiIcon name="spark" />AI summary</span>}{worker && <span className="inbox-tag" data-kind="worker">Worker claim</span>}</p></div></div>
        {!questions.length && thread.next && <p className="inbox-next"><UiIcon name="right" />{thread.next.text}</p>}
        {questions.map((q, i) => <Question key={questionKey(q)} question={q} label={labels[i]} selected={selectedQuestion === questionKey(q)} draft={text} recommendation={thread.recommendation?.text} onSelect={() => setSelectedQuestion(selectedQuestion === questionKey(q) ? "" : questionKey(q))} onChoice={choice => { setSelectedQuestion(questionKey(q)); setText(choice); }} />)}
        {thread.recommendation && !recommendationIsChoice && <div className="inbox-recommendation" role="note"><UiIcon name="spark" /><span><span className="inbox-sr">AI recommendation: </span>{thread.recommendation.text}</span></div>}
        {thread.reply && thread.reply.text !== thread.title.text && <div className="inbox-update"><small>{thread.reply.source.kind === "owner" || thread.reply.source.kind === "owner_card" ? `Update from ${ownerLabel(view, thread.reply.source.run_id)}` : thread.reply.source.kind === "worker" ? "Worker update" : "Update"}</small>{thread.reply.text}</div>}
        <div className="inbox-actions">{targets.map(target => <button type="button" className="inbox-btn" key={target.key} title={target.full} aria-label={target.full} disabled={view?.observation.status !== "fresh"} onClick={() => open(target.location)}><UiIcon name="terminal" /><span>{targets.length === 1 ? "Open session" : target.label}</span></button>)}
          {view?.owners.filter(o => thread.owner_root_ids.includes(o.root_id) && (!o.location || o.availability !== "available")).map(o => <span className="inbox-offline" key={o.root_id}><UiIcon name="offline" />{ownerLabel(view, o.root_id)} · {o.availability === "starting" ? "starting" : o.availability === "absent" ? "not running" : "status unknown"}</span>)}
          {thread.unread_result_message_id && <button type="button" className={`inbox-btn${thread.need === "answer" ? " is-primary" : ""}`} disabled={busy} onClick={() => void markRead()}><UiIcon name="check" />Mark read</button>}</div>
        <Evidence thread={thread} view={view} displayed={displayed} />
      </div> : direct ? <div className="inbox-content"><div className="inbox-head" data-tone="unknown"><span className="inbox-kic is-lg"><UiIcon name="user" /></span><div><h2>{direct.agent_name}</h2><p className="inbox-facts"><span>{direct.workspace_label} · {direct.tab_label}</span><span>Direct agent · {liveDirect?.agent.status || "status unavailable"}</span></p></div></div>{(directContext?.title || directContext?.activity) && <section aria-label="Session context">{directContext.title && <div className="inbox-update inbox-session-context" dir="auto"><small>{directContext.titleLabel}</small>{directContext.title}</div>}{directContext.activity && <div className="inbox-update inbox-session-context" dir="auto"><small>{directContext.activityLabel} · OMP session</small>{directContext.activity}</div>}</section>}<div className="inbox-actions"><button type="button" className="inbox-btn" onClick={() => open({ workspace_id: direct.workspace_id, workspace_label: direct.workspace_label, tab_id: direct.tab_id, tab_label: direct.tab_label, pane_id: direct.pane_id })}>Open session</button></div></div> : <div className="inbox-content"><h2>New work</h2><p>Send a request to a liaison.</p></div>}
      <form className="inbox-compose" data-mode={mode} onSubmit={e => { e.preventDefault(); void send(); }}>
        <div className="inbox-compose-head">
          {thread && (questions.length > 0 || !!selectedQuestion) && <div className="inbox-seg" role="group" aria-label="Message type"><button type="button" aria-pressed={mode === "followup"} disabled={!!pending} onClick={() => setSelectedQuestion("")}>Follow up</button>{questions.length > 0 && <button type="button" aria-pressed={mode === "answer"} aria-disabled={!chosenQuestion && questions.length > 1} title={!chosenQuestion && questions.length > 1 ? "Pick a question above first" : undefined} disabled={!!pending} onClick={() => { if (questions.length === 1) setSelectedQuestion(questionKey(questions[0])); }}>Answer</button>}</div>}
          {!direct && (chosenQuestion ? <span className="inbox-dest"><UiIcon name="reply" /><span>{labels[questions.indexOf(chosenQuestion)]}</span></span> : thread?.owner_root_ids.length === 1 ? <span className="inbox-dest"><UiIcon name="user" /><span>{ownerLabel(view, thread.owner_root_ids[0])}</span></span> : <label className="inbox-pick" data-empty={!owner}><UiIcon name="user" /><select aria-label="Owner" value={owner} onChange={e => setOwner(e.target.value)} disabled={!!pending}><option value="">Choose owner</option>{view?.owners.filter(o => !thread || thread.owner_root_ids.includes(o.root_id)).map(o => <option key={o.root_id} value={o.root_id}>{ownerLabel(view, o.root_id)} · {o.availability}</option>)}</select></label>)}
          {thread && !chosenQuestion && thread.tasks.length > 0 && <label className="inbox-pick"><select aria-label="Follow-up target" value={taskId} onChange={e => setTaskId(e.target.value)} disabled={!!pending}><option value="">Thread only</option>{thread.tasks.map(task => <option key={`${task.root_id}:${task.task_id}`} value={task.task_id}>{task.title} · {ownerLabel(view, task.root_id)}</option>)}</select></label>}
          {!thread && !direct && <details className="inbox-ctx" data-set={!!(attachedSpace || repository || branch || base)}><summary>＋ Context{(attachedSpace || repository || branch || base) ? " · set" : ""}</summary><div className="inbox-ctx-body"><label className="is-check"><input type="checkbox" checked={!!attachedSpace} onChange={e => setAttachedSpace(e.target.checked ? currentSpace : null)} disabled={(!currentSpace && !attachedSpace) || !!pending} /> Attach {attachedSpace?.label || currentSpace?.label || "current Space"}</label><label>Repository ID <input value={repository} onChange={e => setRepository(e.target.value)} disabled={!!pending} /></label><label>Branch <input value={branch} onChange={e => setBranch(e.target.value)} disabled={!!pending} /></label><label>Base ref <input value={base} onChange={e => setBase(e.target.value)} disabled={!!pending} /></label></div></details>}
        </div>
        {questions.length > 1 && !chosenQuestion && <p className="inbox-note" aria-live="polite">Pick a question above to answer it.</p>}
        {error && <p role="alert" className="inbox-note" data-tone="error" title={error}><UiIcon name="alert" />{error}</p>}
        {direct && (directUncertain[direct.pane_id] ? <p role="status" className="inbox-note">Delivery is uncertain. Open the session to inspect it; no automatic retry is safe.</p> : directStale ? <p role="status" className="inbox-note">Agent identity changed or is unavailable. Review and reselect the agent; your draft is preserved.</p> : visibleOutcome ? <p role="status" className="inbox-note">{visibleOutcome.kind === "busy" ? `Not sent. ${visibleOutcome.name} is busy. Your draft is kept; reselect the agent when it is ready.` : `Submitted to ${visibleOutcome.name}’s terminal. Not confirmed read. To send another, reselect the agent once it is ready.`}</p> : !directReady && <p role="status" className="inbox-note">{!direct.expected_target ? "Agent identity is unavailable. Your draft is preserved; reselect when available." : !directTargetMatches || !liveDirect ? "Agent identity changed or is unavailable. Review and reselect the agent; your draft is preserved." : "Direct messages are available only when this agent is ready."}</p>)}
        <div className="inbox-compose-row"><textarea aria-label={direct ? "Message agent" : chosenQuestion ? "Answer question" : thread ? "Follow up with owner" : "Your request"} placeholder={direct ? `Message ${direct.agent_name}…` : chosenQuestion ? "Type an answer or tap a choice" : thread ? questions.length ? "Add an instruction. Doesn’t answer the question" : "Follow up…" : "Describe the work or ask a question"} value={text} onChange={e => { setText(e.target.value); e.currentTarget.style.height = "auto"; e.currentTarget.style.height = `${Math.min(e.currentTarget.scrollHeight, 96)}px`; }} onKeyDown={e => { if (e.key === "Enter" && !e.shiftKey && !e.nativeEvent.isComposing && e.nativeEvent.keyCode !== 229) { e.preventDefault(); e.currentTarget.form?.querySelector<HTMLButtonElement>('button[type="submit"]:not(:disabled)')?.click(); } }} disabled={!!pending} rows={1} enterKeyHint="send" />
          {pending ? <button type="button" className="inbox-send is-retry" disabled={busy || snapshot?.session_id !== sessionId} onClick={() => void execute(pending)}>Retry original request</button> : <button type="submit" className="inbox-send" disabled={!view || snapshot?.session_id !== sessionId || !text.trim() || busy || view.observation.status !== "fresh" || (!!direct && (!directReady || !!visibleOutcome))}><UiIcon name="send" />{mode === "answer" ? "Answer" : mode === "new" ? "Start" : mode === "direct" ? "Send direct" : "Send"}</button>}</div>
      </form>
    </main>
  </section>;
}
function AgentRow({ agent, context, selected, quiet = false, onSelect }: { agent: UnmanagedAgent; context: AgentContext | null; selected: boolean; quiet?: boolean; onSelect(): void }) {
  const preview = context?.activity || context?.title;
  if (!preview) return quiet
    ? <button type="button" className="inbox-row is-quiet" data-tone="unknown" aria-current={selected} onClick={onSelect}><span className="inbox-kic is-sm"><UiIcon name="user" /></span><span className="inbox-row-body"><strong>{agent.agent_name}</strong><span className="inbox-row-next">{agent.workspace_label} · {agent.agent_status || "unknown"}</span></span></button>
    : <button type="button" className="inbox-row" data-tone="decision" aria-current={selected} onClick={onSelect}><span className="inbox-kic" aria-hidden="true"><UiIcon name="user" /></span><span className="inbox-row-body"><span className="inbox-meta"><span className="inbox-meta-t">{agent.workspace_label} · {agent.tab_label}</span></span><strong>{agent.agent_name}</strong><span className="inbox-row-next">{agent.agent_status || "Status unknown"}</span></span><span className="inbox-act" aria-hidden="true">Open <UiIcon name="right" /></span></button>;
  return <button type="button" className={`inbox-row${quiet ? " is-quiet" : ""}`} data-tone={quiet ? "unknown" : "decision"} aria-current={selected} onClick={onSelect} title={[agent.workspace_label, agent.tab_label, agent.agent_name, agent.agent_status, context?.title && `${context.titleLabel}: ${context.title}`, context?.activity && `${context.activityLabel}: ${context.activity}`].filter(Boolean).join(" · ")}><span className={`inbox-kic${quiet ? " is-sm" : ""}`} aria-hidden="true"><UiIcon name="user" /></span><span className="inbox-row-body"><span className="inbox-meta"><span className="inbox-meta-t">{agent.workspace_label} · {agent.tab_label} · {agent.agent_status || "Status unknown"}</span></span><strong>{agent.agent_name}</strong>{context?.activity && context.title && <span className="inbox-sr">{context.titleLabel}: {context.title}</span>}<span className="inbox-row-next"><span className="inbox-sr">{context?.activity ? context.activityLabel : context?.titleLabel}: </span>{preview}</span></span>{!quiet && <span className="inbox-act" aria-hidden="true">Open <UiIcon name="right" /></span>}</button>;
}
function ThreadRow({ thread, selected, onSelect }: { thread: InboxThread; selected: boolean; onSelect(): void }) {
  const state = tone(thread), quiet = thread.state !== "needs_you";
  return <button type="button" className={`inbox-row${quiet ? " is-quiet" : ""}`} data-tone={state.kind} aria-current={selected} onClick={onSelect}><span className={`inbox-kic${quiet ? " is-sm" : ""}`} aria-hidden="true"><UiIcon name={state.icon} /></span><span className="inbox-row-body"><span className="inbox-meta"><span className="inbox-meta-t">{thread.project || state.label}{thread.project && <span className="inbox-sr"> · {state.label}</span>}</span><Time value={thread.updated_at} /></span><strong>{thread.title.text}</strong><span className="inbox-row-next">{thread.next?.text || thread.questions[0]?.text.text || thread.reply?.text}</span></span>{state.action && <span className="inbox-act" aria-hidden="true">{state.action}<UiIcon name="right" /></span>}</button>;
}
function Question({ question, label, selected, draft, recommendation, onSelect, onChoice }: { question: InboxQuestion; label: string; selected: boolean; draft: string; recommendation?: string; onSelect(): void; onChoice(value: string): void }) {
  return <section className="inbox-question" data-selected={selected}><button type="button" className="inbox-q-head" aria-pressed={selected} title={`Ask from ${label}`} onClick={onSelect}><span className="inbox-kic is-sm"><UiIcon name="help" /></span><span className="inbox-q-who">Ask from {label}</span><Time value={question.asked_at} /><span className="inbox-radio"><UiIcon name="check" /></span></button><p className="inbox-q-text">{question.text.text}</p>{question.choices.length > 0 && <div className="inbox-choices" role="group" aria-label={`Choices from ${label}`}>{question.choices.map(choice => { const recommended = !!recommendation && choice.trim().toLowerCase() === recommendation.trim().toLowerCase(); return <button type="button" className="inbox-chip" key={choice} aria-pressed={selected && draft === choice} onClick={() => onChoice(choice)}>{recommended && <><UiIcon name="spark" /><span className="inbox-sr">Recommended: </span></>}{choice}</button>; })}</div>}</section>;
}
function Evidence({ thread, view, displayed }: { thread: InboxThread; view: InboxView | null; displayed: InboxText[] }) {
  const binding = thread.request?.handoff.binding?.requested;
  const fields: Array<[string, string | null | undefined]> = [["Repository", binding?.repository_id], ["Space", binding?.workspace_id], ["Branch", binding?.branch], ["Base ref", binding?.base_ref]];
  const namedFields: Array<[string, InboxText | null | undefined]> = [["title", thread.title], ["next move", thread.next], ["recommendation", thread.recommendation], ["update", thread.reply], ...thread.questions.map(q => ["question", q.text] as [string, InboxText])];
  const aiFields = namedFields.filter(([, value]) => value?.source.kind === "owner_card" && (displayed.includes(value) || value === thread.recommendation));
  return <details className="inbox-evidence"><summary><UiIcon name="right" />Details</summary>
    {thread.next && <><h3 className="inbox-ev-h">Next move</h3><p className="inbox-ev-p">{thread.next.text}</p></>}
    {thread.request && <><h3 className="inbox-ev-h">Your request</h3><p className="inbox-ev-p">{thread.request.text}</p>{fields.some(([, value]) => value) && <dl className="inbox-kv">{fields.map(([label, value]) => value && <div key={label}><dt>{label}</dt><dd>{uuid.test(value) ? "Recorded" : value}</dd></div>)}</dl>}</>}
    {thread.facts.length > 0 && <><h3 className="inbox-ev-h">Evidence</h3>{thread.facts.map((fact, i) => <p className="inbox-fact" key={i}><span className="inbox-tag" data-kind={srcKind(fact)} title={source(fact)}>{srcLabel(fact)}</span><span>{fact.text}</span></p>)}</>}
    {thread.receipts.length > 0 && <><h3 className="inbox-ev-h">Delivery</h3>{thread.receipts.map(r => <p className="inbox-ev-p" key={r.message_id}>{r.kind === "answer" ? "Answer" : "Request"} to {ownerLabel(view, r.to_run_id)} · {r.stage === "stored" ? "waiting to be picked up" : r.stage === "read" ? "picked up" : "acknowledged"}{r.stale ? " · stale" : ""} · <Time value={r.acked_at || r.created_at} /></p>)}</>}
    {thread.tasks.length > 0 && <><h3 className="inbox-ev-h">Tasks</h3>{thread.tasks.map(t => <p className="inbox-ev-p" key={t.task_id}>{t.title} · {t.lane}{t.run ? ` · ${t.run.stage} · ${t.run.presence}` : ""}</p>)}</>}
    {aiFields.length > 0 && <><h3 className="inbox-ev-h">Source</h3><p className="inbox-ev-p">AI summary for {Array.from(new Set(aiFields.map(([name]) => name))).join(", ")} from {Array.from(new Set(aiFields.map(([, v]) => v?.source.kind === "owner_card" ? v.source.harness : ""))).join(", ")}.</p></>}
    <h3 className="inbox-ev-h">IDs</h3><dl className="inbox-kv"><dt>Thread</dt><dd className="is-id">{thread.thread_id}</dd>{fields.map(([label, value]) => value && uuid.test(value) && <div key={label}><dt>{label}</dt><dd className="is-id">{value}</dd></div>)}{thread.owner_root_ids.map((id, i) => <div key={`owner-${i}`}><dt>Owner</dt><dd className="is-id">{id}</dd></div>)}{thread.questions.map((q, i) => <div key={`question-${i}`}><dt>Question</dt><dd className="is-id">{q.question_message_id}</dd></div>)}{thread.request && <div><dt>Request</dt><dd className="is-id">{thread.request.message_id}</dd></div>}{[thread.title, thread.next, thread.recommendation, thread.reply, ...thread.facts].map((v, i) => v?.message_id && <div key={`field-${i}`}><dt>Message</dt><dd className="is-id">{v.message_id}</dd></div>)}{thread.receipts.map((r, i) => <div key={`receipt-${i}`}><dt>Message</dt><dd className="is-id">{r.message_id}</dd>{r.attempt_run_id && <><dt>Attempt run</dt><dd className="is-id">{r.attempt_run_id}</dd></>}</div>)}{thread.tasks.filter(t => t.run).map(t => <div key={t.task_id}><dt>Attempt run</dt><dd className="is-id">{t.run!.run_id}</dd></div>)}</dl>
  </details>;
}
