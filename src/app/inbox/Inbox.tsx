import { useEffect, useId, useLayoutEffect, useRef, useState, type KeyboardEvent as ReactKeyboardEvent, type ReactNode, type RefObject } from "react";
import { CockpitClientError, type CockpitClient } from "../../client/CockpitClient";
import type { InboxLocation, InboxQuestion, InboxText, InboxThread, InboxView, OrchestrationAction, SessionSnapshotResponse, UnmanagedAgent } from "../../protocol/generated/v1";
import type { AgentAttentionState } from "../session/agentAttention";
import { agentContext, type AgentContext } from "../session/agentContext";
import { deriveAgentQueue } from "../sidebar/agentAttentionModel";
import { InboxQuestionCard } from "./InboxQuestionCard";
import { ReplaceChoice, ReplyTargets, type ReplyTarget } from "./InboxReplyControls";
import { Time } from "./InboxTime";
import { deliveryLabel } from "./inboxDelivery";
import { addBelow, answerDraftKey, carryAnswerDrafts, choiceOutcome, draftBadge, questionKey, strandedAnswerDrafts, threadDraftCount, plainQuestion, type StrandedDraft } from "./inboxDrafts";
import { UiIcon, type UiIconName } from "../UiIcon";
import { InboxFeedback } from "./InboxFeedback";
import { InboxTimeline } from "./InboxTimeline";
import { InboxWork } from "./InboxWork";
import { useInboxImages } from "./useInboxImages";
import "./inbox.css";

export type { InboxLocation } from "../../protocol/generated/v1";
export type InboxProps = { client: CockpitClient; sessionId: string | null; active: boolean; inert?: boolean; currentSpace: { id: string; label: string } | null; attention: AgentAttentionState; snapshot: SessionSnapshotResponse | null; onOpenLocation(location: InboxLocation): Promise<void>; onAttentionChange?(value: { count: number; preview: string | null }): void };

type Pending = { action: OrchestrationAction; text: string; store: string; key: string };
const source = (value: InboxText) => value.source.kind === "owner_card" ? `AI synthesis · ${value.source.harness}` : value.source.kind === "owner" ? `Owner · ${value.source.harness}` : value.source.kind === "worker" ? "Worker claim" : value.source.kind === "user" ? "Your request" : "Recorded fact";
const uuid = /^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$/i;
const threadHeadline = (thread: InboxThread) => thread.title.text.replace(/\s+/g, " ").trim();

function outcomeLine(thread: InboxThread) {
  const question = thread.questions.find(q => q.receipt.status === "unresolved");
  const text = question?.text.text || thread.reply?.text || thread.next?.text;
  const lines = text?.split(/\r?\n/).map(line => line.replace(/<[^>]*>|[*_`#>\[\]]/g, "").trim()).filter(Boolean) ?? [];
  return lines.find(line => line !== threadHeadline(thread)) || (thread.state === "handled" ? "Handled" : thread.state === "needs_you" ? "Awaiting your response" : "Waiting for an update");
}
const directBusyCodes: Record<string, true> = { agent_not_idle: true, herdr_prompt_deferred: true, agent_blocked: true, agent_not_ready: true };
const ownerLabel = (view: InboxView | null, runId: string) => {
  const owner = view?.owners.find(o => o.root_id === runId);
  return owner?.label.trim() && !uuid.test(owner.label.trim()) ? owner.label : owner?.harness ? `${owner.harness} owner` : "Owner";
};
function ownerOptionLabel(view: InboxView, owner: InboxView["owners"][number]) {
  const label = ownerLabel(view, owner.root_id);
  const duplicate = view.owners.some(other => other.root_id !== owner.root_id && ownerLabel(view, other.root_id) === label);
  const identity = duplicate ? ` · ${owner.location ? `${owner.location.workspace_label} / ${owner.location.tab_label} · ` : ""}${owner.root_id}` : "";
  const status = { available: "Available", starting: "Starting", absent: "Not running", unknown: "Status unknown" }[owner.availability];
  return `${label}${identity} · ${status}`;
}
/** One label per asking run: several questions from the same owner share it; only different runs with the same name are told apart. */
function questionLabels(view: InboxView | null, questions: InboxQuestion[]) {
  const runs = Array.from(new Set(questions.map(q => q.run_id)));
  const named = runs.map(run => ownerLabel(view, run));
  const withHarness = runs.map((run, i) => named.filter(x => x === named[i]).length > 1 ? `${named[i]} · ${view?.owners.find(o => o.root_id === run)?.harness || "owner"}` : named[i]!);
  const unique = withHarness.map((label, i) => withHarness.filter(x => x === label).length > 1 ? `${label} · …${runs[i]!.slice(-4)}` : label);
  return questions.map(q => unique[runs.indexOf(q.run_id)]!);
}
/** State word, icon and tone of a thread; an unread result names its outcome in words as well as colour. */
function tone(thread: InboxThread): { kind: string; icon: UiIconName; label: string } {
  if (thread.state === "underway") return { kind: "work", icon: "work", label: "Working" };
  if (thread.state === "handled") return { kind: "done", icon: "check-circle", label: "Handled" };
  if (thread.need === "answer") {
    const outcome = thread.timeline.find(entry => entry.message_id === thread.unread_result_message_id)?.outcome;
    return outcome === "failed" ? { kind: "blocked", icon: "alert", label: "Result · Failed" } : outcome === "succeeded" ? { kind: "done", icon: "check-circle", label: "Result · Succeeded" } : { kind: "review", icon: "eye", label: "Result ready" };
  }
  const open = thread.questions.filter(q => q.receipt.status === "unresolved").length;
  return { kind: "decision", icon: "help", label: open > 1 ? `${open} questions` : thread.need === "decision" ? "Question" : "Needs you" };
}
const srcKind = (value: InboxText) => value.source.kind === "owner_card" ? "ai" : value.source.kind;
const srcLabel = (value: InboxText) => ({ ai: "AI summary", owner: "Owner", worker: "Worker claim", user: "Your request", recorded: "Recorded" })[srcKind(value)];

/** Each session remains mounted so switching sessions retains its local drafts and scroll positions. */
export function Inbox(props: InboxProps) {
  const [sessions, setSessions] = useState<string[]>(() => props.sessionId ? [props.sessionId] : []);
  useEffect(() => { if (props.sessionId) setSessions(s => s.includes(props.sessionId!) ? s : [...s, props.sessionId!]); else props.onAttentionChange?.({ count: 0, preview: null }); }, [props.sessionId, props.onAttentionChange]);
  return <div className="inbox-root" hidden={!props.active} inert={props.inert}>{!props.sessionId && <p className="inbox-empty">Select a session to see your inbox.</p>}{sessions.map(id => <SessionInbox key={id} {...props} sessionId={id} current={props.sessionId === id} visible={props.active && props.sessionId === id} />)}</div>;
}

function SessionInbox({ client, sessionId, visible, current, currentSpace, onOpenLocation, onAttentionChange, attention, snapshot }: Omit<InboxProps, "active" | "sessionId"> & { sessionId: string; visible: boolean; current: boolean }) {
  const [view, setView] = useState<InboxView | null>(null);
  const [error, setError] = useState("");
  const [loadError, setLoadError] = useState("");
  const [selected, setSelected] = useState<string | null>(null);
  const [mobileDetail, setMobileDetail] = useState(false);
  const [wideDetail, setWideDetail] = useState(false);
  const [selectedAgent, setSelectedAgent] = useState<{ agent: UnmanagedAgent; terminalId: string | null } | null>(null);
  const [drafts, setDrafts] = useState<Record<string, string>>({});
  const [selectedQuestion, setSelectedQuestion] = useState<string | null>(null);
  const [selectedOwner, setOwner] = useState("");
  const [preferredOwner, setPreferredOwner] = useState("");
  const owner = selectedOwner || preferredOwner;
  const [taskId, setTaskId] = useState("");
  const [attachedSpace, setAttachedSpace] = useState<{ id: string; label: string } | null>(null);
  const [repository, setRepository] = useState("");
  const [branch, setBranch] = useState("");
  const [base, setBase] = useState("");
  const [pending, setPending] = useState<Pending | null>(null);
  const [busy, setBusy] = useState(false);
  const [acknowledging, setAcknowledging] = useState<{ threadId: string; resultId: string } | null>(null);
  const [readError, setReadError] = useState("");
  const acknowledgementDescription = useId();
  const [followupOpen, setFollowupOpen] = useState(false);
  const [announcement, setAnnouncement] = useState("");
  const [detailView, setDetailView] = useState<"thread" | "work">("thread");
  const [choiceConfirm, setChoiceConfirm] = useState<{ key: string; choice: string } | null>(null);
  const [carried, setCarried] = useState<Record<string, true>>({});
  const [acknowledged, setAcknowledged] = useState<string | null>(null);
  const viewId = useId();
  const followupRef = useRef<HTMLButtonElement>(null);
  const focusAfterRead = useRef<string | null>(null);
  const resultActionsRef = useRef<HTMLDivElement>(null);
  const resultActionsFocused = useRef(false);
  const timelineRef = useRef<HTMLDivElement>(null);
  const [directUncertain, setDirectUncertain] = useState<Record<string, boolean>>({});
  const [directStale, setDirectStale] = useState(false);
  const [directOutcome, setDirectOutcome] = useState<{ pane: string; target: string; kind: "submitted" | "busy"; name: string } | null>(null);
  const [dragging, setDragging] = useState(false);
  const selectionVersion = useRef(0);
  const wake = useRef<(() => void) | null>(null);
  const refreshVersion = useRef(0);
  const revisionFloor = useRef(0);
  const pendingRead = useRef<Promise<InboxView> | null>(null);
  const mounted = useRef(true);
  const lock = useRef(false);
  const liveScope = useRef({ current, snapshot, view, attention });
  const backRef = useRef<HTMLButtonElement>(null);
  const newRef = useRef<HTMLButtonElement>(null);
  const originRef = useRef<HTMLButtonElement | null>(null);
  const detailRef = useRef<HTMLElement>(null);
  const composerRef = useRef<HTMLTextAreaElement>(null);
  liveScope.current = { current, snapshot, view, attention };
  const newOwners = view?.owners.filter(candidate => (candidate.kind === "supervisor" || candidate.kind === "adopted") && candidate.stage !== "closed" && candidate.availability !== "absent") ?? [];
  const newOwner = newOwners.find(candidate => candidate.root_id === selectedOwner)?.root_id ?? newOwners.find(candidate => candidate.availability === "available")?.root_id ?? newOwners.find(candidate => candidate.root_id === preferredOwner)?.root_id ?? newOwners[0]?.root_id ?? "";
  useEffect(() => { mounted.current = true; return () => { mounted.current = false; }; }, []);
  useEffect(() => {
    const detail = detailRef.current;
    if (!detail || typeof ResizeObserver === "undefined") return;
    const observer = new ResizeObserver(entries => setWideDetail(entries[0].contentRect.width >= 960));
    observer.observe(detail);
    return () => observer.disconnect();
  }, []);
  useEffect(() => {
    if (mobileDetail && backRef.current?.offsetParent !== null) backRef.current?.focus();
  }, [mobileDetail]);
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
  useEffect(() => { let live = true; void client.orchestrationLiaisonLookup({ session_id: sessionId, selected_root_id: null }).then(result => { if (live && result.status === "resolved" && result.preferred_root_id) setPreferredOwner(result.preferred_root_id); }).catch(() => {}); return () => { live = false; }; }, [client, sessionId]);
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
  const threadShown = thread?.thread_id ?? null;
  // Open a thread at its open question's first line; without one, at its latest entry.
  useLayoutEffect(() => {
    const timeline = timelineRef.current;
    if (!threadShown || !visible || !timeline) return;
    const card = timeline.querySelector<HTMLElement>("[data-open-question]");
    timeline.scrollTop = card ? Math.max(0, timeline.scrollTop + card.getBoundingClientRect().top - timeline.getBoundingClientRect().top - 12) : timeline.scrollHeight;
  }, [threadShown, visible]);
  useEffect(() => {
    const trackFocus = (event: Event) => {
      resultActionsFocused.current = !!resultActionsRef.current?.contains(event.target as Node);
    };
    document.addEventListener("focusin", trackFocus, true);
    document.addEventListener("pointerdown", trackFocus, true);
    return () => {
      document.removeEventListener("focusin", trackFocus, true);
      document.removeEventListener("pointerdown", trackFocus, true);
    };
  }, []);
  useLayoutEffect(() => {
    const readId = focusAfterRead.current;
    if (!readId || !thread || thread.unread_result_message_id === readId) return;
    focusAfterRead.current = null;
    const active = document.activeElement;
    if (resultActionsFocused.current && (!active || active === document.body || resultActionsRef.current?.contains(active))) {
      (followupRef.current ?? composerRef.current)?.focus();
    }
    resultActionsFocused.current = false;
  }, [thread, announcement]);
  const direct = selectedAgent?.agent;
  const matchedDirect = direct ? matchingAgent(direct) : undefined;
  const liveDirect = matchedDirect?.terminalId === selectedAgent?.terminalId ? matchedDirect : undefined;
  const questions = thread?.questions.filter(q => q.receipt.status === "unresolved") ?? [];
  const labels = questionLabels(view, questions);
  // Selecting never sends. One open question is preselected; with several the user chooses; "" is an explicit follow-up.
  const picked = direct || !thread ? undefined : selectedQuestion === null ? (questions.length === 1 ? questions[0] : undefined) : questions.find(q => questionKey(q) === selectedQuestion);
  const mode = direct ? "direct" : !thread ? "new" : picked ? "answer" : questions.length > 0 && selectedQuestion !== "" ? "choose" : "followup";
  // Follow-up, new-work and direct drafts keep their key (and images); each open question has its own answer draft.
  const followKey = direct ? JSON.stringify(["agent", direct.pane_id]) : selected ?? "new";
  const draftKey = thread && picked ? answerDraftKey(thread.thread_id, picked) : followKey;
  const text = mode === "choose" ? "" : drafts[draftKey] ?? "";
  const images = useInboxImages(client, sessionId, followKey, view?.image_limits);
  const setText = (value: string) => { setDrafts(d => ({ ...d, [draftKey]: value })); };
  const openIds = questions.map(questionKey).join("\n");
  const notifiedStranded = useRef(new Set<string>());
  // A question that changed or was answered elsewhere never receives the old text: the draft moves to its one successor
  // or stays visible for the user to reuse or discard, and the selection returns to follow-up.
  useEffect(() => {
    if (!thread || !current || busy || pending || lock.current) return;
    const stale = !!selectedQuestion && !questions.some(q => questionKey(q) === selectedQuestion);
    const result = carryAnswerDrafts(drafts, thread.thread_id, questions);
    const moved = result.carried.length > 0;
    if (moved || Object.keys(result.drafts).length !== Object.keys(drafts).length) {
      setDrafts(result.drafts);
      if (moved) setCarried(flags => ({ ...flags, ...Object.fromEntries(result.carried.map(k => [k, true as const])) }));
    }
    if (stale || moved) setSelectedQuestion("");
    const leftover = strandedAnswerDrafts(result.drafts, thread.thread_id, questions).filter(item => item.text.trim());
    for (const key of Array.from(notifiedStranded.current)) if (!leftover.some(item => item.key === key)) notifiedStranded.current.delete(key);
    const unseen = leftover.filter(item => !notifiedStranded.current.has(item.key));
    for (const item of unseen) notifiedStranded.current.add(item.key);
    if (moved) setError("The question changed before your answer was delivered. Not sent. Your draft was kept under the updated question; review it, select the question, then send again.");
    else if (unseen.length > 0) setError("The question changed before your answer was delivered. Not sent. Your draft is kept; use it as a follow-up or discard it.");
  }, [thread, current, busy, pending, selectedQuestion, drafts, openIds]);
  useLayoutEffect(() => {
    const field = composerRef.current;
    if (!field || !visible) return;
    field.style.height = "auto";
    field.style.height = `${Math.min(field.scrollHeight, window.innerHeight * .4, mode === "new" ? Infinity : 160)}px`;
  }, [text, draftKey, mode, visible, mobileDetail]);
  const select = (id: string | null) => { selectionVersion.current++; setDirectOutcome(null); setError(""); setReadError(""); setFollowupOpen(false); setAnnouncement(""); setAcknowledged(null); setChoiceConfirm(null); setDetailView("thread"); focusAfterRead.current = null; resultActionsFocused.current = false; setSelected(id); setSelectedAgent(null); setSelectedQuestion(null); setTaskId(""); setMobileDetail(true); };
  const selectAgent = (agent: UnmanagedAgent) => {
    const row = matchingAgent(agent);
    const target = agent.expected_target;
    selectionVersion.current++; setDirectOutcome(null); setError(""); setSelected(`agent:${agent.pane_id}`); setSelectedAgent({ agent: { ...agent, expected_target: target ? { ...target } : target }, terminalId: row?.terminalId ?? null }); setDirectStale(false); setSelectedQuestion(null); setMobileDetail(true);
    // Selection opens the captured result, independently of terminal focus.
    // Never acknowledge a row whose direct target cannot be matched safely.
    if (current && view?.session_id === sessionId && view.observation.status === "fresh" && snapshot?.session_id === sessionId && row?.canSeen && target && (!target.native_session_id || target.native_session_id === row.agent.native_session_id)) void attention.seen(row);
  };
  const latestDirect = direct && view?.unmanaged_agents.find(agent => agent.pane_id === direct.pane_id);
  const directTargetMatches = !!direct?.expected_target && !!latestDirect?.expected_target && JSON.stringify(direct.expected_target) === JSON.stringify(latestDirect.expected_target);
  const directReady = !!direct && !!liveDirect && (liveDirect.agent.status === "idle" || liveDirect.agent.status === "done") && directTargetMatches && direct.expected_target?.pane_id === direct.pane_id && direct.expected_target.workspace_id === direct.workspace_id && direct.expected_target.tab_id === direct.tab_id && direct.expected_target.terminal_id === selectedAgent?.terminalId && (direct.expected_target.agent_name === null || direct.expected_target.agent_name === direct.agent_name) && !directStale && !directUncertain[direct.pane_id];
  const directContext = direct && directTargetMatches && liveDirect ? contextFor(latestDirect!) : null;
  const visibleOutcome = directOutcome && direct && directOutcome.pane === direct.pane_id && directOutcome.target === JSON.stringify(direct.expected_target) ? directOutcome : null;
  const open = (location: InboxLocation | null) => {
    if (!location || !current || view?.observation.status !== "fresh" || snapshot?.session_id !== sessionId || (direct && (!liveDirect || !directTargetMatches))) return;
    // Capture the completion at the click, not after navigation: a later
    // completion must remain unseen even if it arrives while focus is pending.
    const row = direct && liveDirect?.canSeen ? liveDirect : null;
    const target = direct?.expected_target;
    const attempt = selectionVersion.current;
    const instance = snapshot.server_instance;
    void onOpenLocation(location).then(() => {
      if (!row || !target || !mounted.current || selectionVersion.current !== attempt) return;
      const latest = liveScope.current;
      if (!latest.current || latest.snapshot?.session_id !== sessionId || latest.snapshot.server_instance !== instance || latest.view?.observation.status !== "fresh") return;
      const agent = latest.view.unmanaged_agents.find(item => item.pane_id === row.agent.pane_id);
      if (!agent?.expected_target || agent.expected_target.endpoint_identity !== target.endpoint_identity || agent.expected_target.workspace_id !== target.workspace_id || agent.expected_target.tab_id !== target.tab_id || agent.expected_target.pane_id !== target.pane_id || agent.expected_target.terminal_id !== target.terminal_id || agent.expected_target.agent_kind !== target.agent_kind || agent.expected_target.agent_name !== target.agent_name || agent.expected_target.native_session_id !== target.native_session_id || agent.expected_target.native_session_file !== target.native_session_file) return;
      const currentRow = deriveAgentQueue(latest.snapshot, latest.attention.response, latest.attention.availability, true).visible.find(item => item.agent.pane_id === row.agent.pane_id);
      if (!currentRow || currentRow.terminalId !== row.terminalId || currentRow.agent.name !== row.agent.name || currentRow.agent.space_id !== row.agent.space_id || currentRow.agent.tab_id !== row.agent.tab_id || currentRow.agent.native_session_id !== row.agent.native_session_id) return;
      void latest.attention.seen(row);
    }).catch(e => { if (mounted.current && selectionVersion.current === attempt) setError(e instanceof Error ? e.message : "Could not open session"); });
  };
  const execute = async (operation: Pending) => {
    if (lock.current || !view || operation.store !== view.state_store_id) { if (view && operation.store !== view.state_store_id) setError("Session state changed. The original request has been retained and cannot be retried here."); return; }
    lock.current = true; setBusy(true); setError(""); setPending(operation);
    try {
      const response = await client.orchestrationMutate({ session_id: sessionId, expected_revision: null, action: operation.action });
      if (!mounted.current) return;
      revisionFloor.current = Math.max(revisionFloor.current, response.revision);
      if (response.result.result !== "message" || (operation.action.action === "thread_handoff" && response.result.to_run_id !== operation.action.to_run_id) || (operation.action.action === "message_send" && response.result.to_run_id !== operation.action.to_run_id)) throw new Error("Delivery receipt did not match the request");
      if (response.result.result === "message" && response.result.stale) { const answering = operation.action.action === "message_send" && operation.action.kind === "answer"; setPending(null); setError(answering ? "The question changed before your answer was delivered. Not sent. Your draft was kept; review the current question, select it, then send again." : "The question or task changed. Your draft is preserved; review the current work before sending again."); if (answering) setSelectedQuestion(""); }
      else {
        setPending(null);
        setDrafts(d => d[operation.key] === operation.text ? { ...d, [operation.key]: "" } : d);
        setCarried(flags => { if (!flags[operation.key]) return flags; const next = { ...flags }; delete next[operation.key]; return next; });
        setChoiceConfirm(null);
        if (operation.action.action === "message_send" && operation.action.kind === "answer") {
          const answered = JSON.stringify([operation.action.to_run_id, operation.action.in_reply_to]);
          setSelectedQuestion(selection => selection === answered || selection === null ? "" : selection);
          const remaining = questions.filter(q => questionKey(q) !== answered).length;
          setAnnouncement(`Answer sent to ${ownerLabel(view, operation.action.to_run_id)}. Stored, waiting to be picked up.${remaining ? ` ${remaining} question${remaining === 1 ? "" : "s"} still open.` : ""}`);
        }
        if (operation.action.action === "thread_handoff") { images.clear(operation.key, operation.action.images ?? []); setSelected(operation.action.thread_id); }
      }
      refreshVersion.current++; wake.current?.();
    } catch (e) { if (mounted.current) { setError(`${e instanceof Error ? e.message : "Delivery was not confirmed"}. The original request is retained; retry it without changing its identity.`); refreshVersion.current++; wake.current?.(); } }
    finally { lock.current = false; if (mounted.current) setBusy(false); }
  };
  const send = async () => {
    if (!view || !current || snapshot?.session_id !== sessionId || busy || pending || lock.current || view.observation.status !== "fresh") return;
    if (mode === "choose") return;
    const handoff = !direct && !picked;
    if ((!text.trim() && (!handoff || images.ids.length === 0)) || (handoff && !images.ready)) return;
    if (selected && !thread && !direct) { setError("This work is no longer available. Your draft is preserved."); return; }
    if (direct) {
      if (!directReady || visibleOutcome || !direct.expected_target || view.observation.status !== "fresh") return;
      const attempt = selectionVersion.current;
      const outcome = { pane: direct.pane_id, target: JSON.stringify(direct.expected_target), name: direct.agent_name };
      lock.current = true; setBusy(true); setError(""); setDirectOutcome(null);
      try {
        await client.orchestrationAgentSend({ session_id: sessionId, reference: direct.pane_id, text, expected_target: direct.expected_target });
        if (mounted.current) {
          setDrafts(d => d[draftKey] === text ? { ...d, [draftKey]: "" } : d);
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
    let action: OrchestrationAction;
    if (selectedQuestion && !picked) { setError("The selected question changed. Your draft is preserved; choose the current question before sending."); return; }
    if (picked) action = { action: "message_send", message_id: crypto.randomUUID(), to_run_id: picked.run_id, kind: "answer", text, in_reply_to: picked.question_message_id };
    else {
      const target = thread ? (thread.owner_root_ids.length === 1 ? thread.owner_root_ids[0] : owner) : newOwner;
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
      action = { action: "thread_handoff", message_id: crypto.randomUUID(), thread_id: thread?.thread_id ?? crypto.randomUUID(), to_run_id: target, text, ...(images.ids.length ? { images: images.ids } : {}), ...(task && taskRevision ? { task_id: task.task_id, expected_task_revision: taskRevision } : {}), ...(binding ? { binding } : {}) };
    }
    await execute({ action, text, store: view.state_store_id, key: draftKey });
  };
  const markRead = async () => {
    if (!thread?.unread_result_message_id || thread.reply?.message_id !== thread.unread_result_message_id || !view || !current || view.observation.status !== "fresh" || snapshot?.session_id !== sessionId || busy || lock.current) return;
    const attempt = selectionVersion.current;
    const resultId = thread.unread_result_message_id;
    lock.current = true; setBusy(true); setAcknowledging({ threadId: thread.thread_id, resultId }); setReadError("");
    try {
      await client.orchestrationMutate({ session_id: sessionId, expected_revision: null, action: { action: "thread_mark_read", thread_id: thread.thread_id, through_message_id: resultId } });
      if (mounted.current && selectionVersion.current === attempt) { focusAfterRead.current = resultId; setAnnouncement("Acknowledged. You can still follow up."); setAcknowledged(resultId); }
      refreshVersion.current++; wake.current?.();
    } catch (e) { if (mounted.current && selectionVersion.current === attempt) setReadError(`Couldn't acknowledge: ${e instanceof Error ? e.message : "Please try again."}`); }
    finally { lock.current = false; if (mounted.current) { setBusy(false); setAcknowledging(null); } }
  };
  const startFollowup = () => { setSelectedQuestion(""); setChoiceConfirm(null); setFollowupOpen(true); requestAnimationFrame(() => composerRef.current?.focus()); };
  const leaveDetail = () => { setMobileDetail(false); requestAnimationFrame(() => (originRef.current?.isConnected && originRef.current.offsetParent !== null ? originRef.current : newRef.current)?.focus()); };
  const recommendationIsChoice = !!thread?.recommendation && questions.some(q => q.choices.some(c => c.trim().toLowerCase() === thread.recommendation!.text.trim().toLowerCase()));
  const displayed = thread ? [thread.title, ...(!questions.length && thread.next ? [thread.next] : []), ...(thread.recommendation && !recommendationIsChoice ? [thread.recommendation] : []), ...(thread.reply && thread.reply.text !== thread.title.text ? [thread.reply] : []), ...questions.map(q => q.text)] : [];
  const underway = view?.threads.filter(t => t.state === "underway") ?? [];
  const handled = view?.threads.filter(t => t.state === "handled") ?? [];
  const targets = thread ? view?.owners.filter(o => thread.owner_root_ids.includes(o.root_id) && o.location && o.availability === "available").map(o => ({ key: o.root_id, label: ownerLabel(view, o.root_id), full: `Open ${ownerLabel(view, o.root_id)}`, location: o.location })) ?? [] : [];
  const coarsePointer = () => typeof window.matchMedia === "function" && window.matchMedia("(pointer: coarse)").matches;
  const focusField = () => requestAnimationFrame(() => composerRef.current?.focus());
  const openCard = (question: InboxQuestion) => Array.from(timelineRef.current?.querySelectorAll<HTMLElement>("[data-open-question]") ?? []).find(card => card.dataset.openQuestion === questionKey(question));
  // Pointer selection moves into the field on desktop; keyboard and touch selection never steal focus.
  const pickTarget = (key: string, pointer: boolean) => { setChoiceConfirm(null); setError(""); setSelectedQuestion(key); if (pointer && !coarsePointer()) focusField(); };
  const showQuestion = (question: InboxQuestion) => {
    const timeline = timelineRef.current, card = openCard(question);
    if (!timeline || !card) return;
    const top = Math.max(0, timeline.scrollTop + card.getBoundingClientRect().top - timeline.getBoundingClientRect().top - 12);
    const calm = typeof window.matchMedia === "function" && window.matchMedia("(prefers-reduced-motion: reduce)").matches;
    if (typeof timeline.scrollTo === "function") timeline.scrollTo({ top, behavior: calm ? "auto" : "smooth" }); else timeline.scrollTop = top;
    card.focus({ preventScroll: true });
  };
  const choose = (question: InboxQuestion, choice: string, pointer: boolean) => {
    if (!thread) return;
    const key = answerDraftKey(thread.thread_id, question);
    setSelectedQuestion(questionKey(question)); setError("");
    const outcome = choiceOutcome(drafts[key] ?? "", choice);
    if (outcome.kind === "confirm") { setChoiceConfirm({ key, choice }); return; }
    setChoiceConfirm(null);
    if (outcome.kind === "fill") setDrafts(d => ({ ...d, [key]: choice }));
    if (pointer && !coarsePointer()) focusField();
  };
  const resolveChoice = (how: "replace" | "add" | "keep") => {
    const confirming = choiceConfirm;
    setChoiceConfirm(null);
    if (confirming && how !== "keep") setDrafts(d => ({ ...d, [confirming.key]: how === "replace" ? confirming.choice : addBelow(d[confirming.key] ?? "", confirming.choice) }));
    focusField();
  };
  const stranded = thread ? strandedAnswerDrafts(drafts, thread.thread_id, questions).find(item => item.text.trim()) : undefined;
  const reuseStranded = (item: StrandedDraft) => {
    setDrafts(d => { const next = { ...d }, existing = next[followKey] ?? ""; next[followKey] = existing.trim() ? `${existing.replace(/\s+$/, "")}\n\n${item.text}` : item.text; delete next[item.key]; return next; });
    setSelectedQuestion(""); setError(""); focusField();
  };
  const fieldKey = (e: ReactKeyboardEvent<HTMLTextAreaElement>) => {
    if (e.nativeEvent.isComposing || e.nativeEvent.keyCode === 229) return;
    if (e.key === "Enter" && !e.shiftKey) { e.preventDefault(); e.currentTarget.form?.querySelector<HTMLButtonElement>("button.inbox-send:not(:disabled)")?.click(); return; }
    if (e.key !== "Escape" || !thread) return;
    // Escape never discards: it cancels a pending choice replacement, otherwise parks focus on the thing being answered.
    e.preventDefault(); e.stopPropagation();
    if (choiceConfirm?.key === draftKey) { setChoiceConfirm(null); return; }
    (picked ? openCard(picked) : timelineRef.current)?.focus();
  };
  const replyTargets: ReplyTarget[] = thread && questions.length > 0 ? [
    ...questions.map((q, i): ReplyTarget => ({ key: questionKey(q), label: questions.length > 1 ? `Answer Q${i + 1}` : "Answer", icon: "reply", checked: picked === q, hasDraft: !!(drafts[answerDraftKey(thread.thread_id, q)] ?? "").trim() })),
    { key: "", label: "Follow up", icon: "send", checked: mode === "followup", hasDraft: !!(drafts[followKey] ?? "").trim() || images.entries.length > 0 },
  ] : [];
  const ownerSelect = <label className={mode === "new" ? "inbox-owner" : "inbox-pick"} data-empty={!(thread ? owner : newOwner)}>{mode === "new" && <span className="inbox-field-label">Owner</span>}<span className={mode === "new" ? "inbox-owner-control" : undefined}><UiIcon name="user" /><select aria-label="Owner" value={thread ? owner : newOwner} onChange={e => setOwner(e.target.value)} disabled={!!pending}><option value="">Choose owner</option>{(thread ? view?.owners.filter(o => thread.owner_root_ids.includes(o.root_id)) : newOwners)?.map(o => <option key={o.root_id} value={o.root_id}>{ownerOptionLabel(view!, o)}</option>)}</select></span></label>;
  const ownerControl = mode === "new" ? ownerSelect : mode === "followup" && thread ? (thread.owner_root_ids.length === 1 ? (questions.length > 0 ? null : <span className="inbox-dest"><UiIcon name="user" /><span>{ownerLabel(view, thread.owner_root_ids[0]!)}</span></span>) : ownerSelect) : null;
  const followOwner = thread ? ownerLabel(view, thread.owner_root_ids.length === 1 ? thread.owner_root_ids[0]! : owner) : "";
  const recommended = thread?.recommendation?.text.trim().toLowerCase();
  const badge = (t: InboxThread) => draftBadge(threadDraftCount(drafts, t.thread_id), images.count(t.thread_id));
  const resultActions = thread?.unread_result_message_id ? <div ref={resultActionsRef} className="inbox-result-actions" role="group" aria-label="Result actions" onFocusCapture={() => { resultActionsFocused.current = true; }} onBlurCapture={e => { if (!e.relatedTarget || !e.currentTarget.contains(e.relatedTarget as Node)) resultActionsFocused.current = false; }}>
    {readError ? <p role="alert">{readError}</p> : thread.reply?.message_id !== thread.unread_result_message_id && <p>The unread result is not available in this view. Refresh before acknowledging it.</p>}
    <span id={acknowledgementDescription} className="inbox-sr">Marks this result as read. It does not answer questions, accept the work, or notify the owner.</span>
    <div className="inbox-result-buttons"><button ref={followupRef} type="button" className="inbox-btn" onClick={startFollowup}>Follow up</button><button type="button" className="inbox-btn is-primary" aria-describedby={acknowledgementDescription} title="Marks this result as read. It does not answer questions, accept the work, or notify the owner." disabled={thread.reply?.message_id !== thread.unread_result_message_id || busy || !current || view?.observation.status !== "fresh" || snapshot?.session_id !== sessionId} onClick={() => void markRead()}><UiIcon name="check" />{acknowledging?.threadId === thread.thread_id && acknowledging.resultId === thread.unread_result_message_id ? "Acknowledging…" : "Acknowledge"}</button></div>
    <small className="inbox-result-help" aria-hidden="true">Acknowledging does not accept the work or notify the owner.</small>
    {view?.observation.status !== "fresh" && <small>Live sessions are unavailable; actions are paused.</small>}
  </div> : null;
  const recommendationNote = thread?.recommendation && !recommendationIsChoice ? <div className="inbox-recommendation" role="note"><UiIcon name="spark" /><InboxFeedback text={thread.recommendation.text} /></div> : null;
  const sendControl = pending ? <button type="button" className="inbox-send is-retry" disabled={busy || snapshot?.session_id !== sessionId} onClick={() => void execute(pending)}>Retry original request</button> : <button type="submit" className="inbox-send" disabled={!view || snapshot?.session_id !== sessionId || mode === "choose" || (!text.trim() && ((mode !== "new" && mode !== "followup") || images.ids.length === 0)) || ((mode === "new" || mode === "followup") && !images.ready) || busy || view.observation.status !== "fresh" || (!!direct && (!directReady || !!visibleOutcome))}><UiIcon name="send" />{mode === "answer" ? "Answer" : mode === "new" ? "Start" : mode === "direct" ? "Send direct" : "Send"}</button>;
  const composer = (
      <form className="inbox-compose" data-mode={mode} data-result-followup={!!thread?.unread_result_message_id && !followupOpen && !questions.length && !pending && !error || undefined} data-dragging={dragging || undefined} onDragOver={e => { if ((mode === "new" || mode === "followup") && current && view?.image_limits && !pending && !busy && Array.from(e.dataTransfer.types).includes("Files")) { e.preventDefault(); e.dataTransfer.dropEffect = "copy"; setDragging(true); } }} onDragLeave={e => { if (!e.currentTarget.contains(e.relatedTarget as Node | null)) setDragging(false); }} onDrop={e => { setDragging(false); if ((mode === "new" || mode === "followup") && current && view?.image_limits && !pending && !busy && e.dataTransfer.files.length) { e.preventDefault(); images.add(e.dataTransfer.files); } }} onSubmit={e => { e.preventDefault(); void send(); }}>
        <div className="inbox-compose-head">
          {thread && !questions.length && <strong className="inbox-compose-label">Follow up</strong>}
          {thread?.unread_result_message_id && followupOpen && !pending && <button type="button" className="inbox-followup-cancel" onClick={() => { setFollowupOpen(false); requestAnimationFrame(() => followupRef.current?.focus()); }}>Cancel</button>}
          {replyTargets.length > 0 && <ReplyTargets targets={replyTargets} disabled={!!pending} onPick={pickTarget} />}
          {ownerControl}
          {mode === "followup" && thread && thread.tasks.length > 0 && <label className="inbox-pick">About <select aria-label="Follow-up target" value={taskId} onChange={e => setTaskId(e.target.value)} disabled={!!pending}><option value="">Whole thread</option>{thread.tasks.map(task => <option key={`${task.root_id}:${task.task_id}`} value={task.task_id}>{task.title} · {ownerLabel(view, task.root_id)}</option>)}</select></label>}
        </div>
        {thread && questions.length > 0 && <div className="inbox-quote" data-mode={mode}>
          {mode === "answer" && picked ? <><span className="inbox-quote-text"><b>Answering {labels[questions.indexOf(picked)]}</b> · “{plainQuestion(picked.text.text)}”</span><button type="button" className="inbox-link" onClick={() => showQuestion(picked)}>Show question</button></>
            : mode === "followup" ? <span className="inbox-quote-text"><b>Follow-up to {followOwner}</b> · not an answer; {questions.length === 1 ? "1 question stays" : `${questions.length} questions stay`} open</span>
            : <span className="inbox-quote-text"><b>Choose what you are replying to.</b> Select a question above, or Follow up.</span>}
        </div>}
        {error && <p role="alert" className="inbox-note" data-tone="error" title={error}><UiIcon name="alert" />{error}</p>}
        {direct && (directUncertain[direct.pane_id] ? <p role="status" className="inbox-note">Delivery is uncertain. Open the session to inspect it; no automatic retry is safe.</p> : directStale ? <p role="status" className="inbox-note">Agent identity changed or is unavailable. Review and reselect the agent; your draft is preserved.</p> : visibleOutcome ? <p role="status" className="inbox-note">{visibleOutcome.kind === "busy" ? `Not sent. ${visibleOutcome.name} is busy. Your draft is kept; reselect the agent when it is ready.` : `Submitted to ${visibleOutcome.name}’s terminal. Not confirmed read. To send another, reselect the agent once it is ready.`}</p> : !directReady && <p role="status" className="inbox-note">{!direct.expected_target ? "Agent identity is unavailable. Your draft is preserved; reselect when available." : !directTargetMatches || !liveDirect ? "Agent identity changed or is unavailable. Review and reselect the agent; your draft is preserved." : "Direct messages are available only when this agent is ready."}</p>)}
        {view && view.observation.status !== "fresh" && mode !== "direct" && <p role="status" className="inbox-note"><UiIcon name="alert" />Can’t see live sessions right now. Replies are paused; your draft is kept.</p>}
        {mode === "answer" && !!carried[draftKey] && !!text.trim() && <p role="status" className="inbox-note"><UiIcon name="edit" />Draft carried over from the earlier version of this question. Review it before sending.</p>}
        {stranded && <div className="inbox-note inbox-stranded" role="status"><UiIcon name="edit" /><div><p>Unsent answer to a question that is no longer open:</p><blockquote>{stranded.text}</blockquote><div className="inbox-replace-acts"><button type="button" className="inbox-btn" onClick={() => reuseStranded(stranded)}>Use as follow-up</button><button type="button" className="inbox-btn" onClick={() => setDrafts(d => { const next = { ...d }; delete next[stranded.key]; return next; })}>Discard</button></div></div></div>}
        {mode === "answer" && choiceConfirm?.key === draftKey && <ReplaceChoice choice={choiceConfirm.choice} onReplace={() => resolveChoice("replace")} onAdd={() => resolveChoice("add")} onKeep={() => resolveChoice("keep")} />}
        {mode === "answer" && picked && picked.choices.length > 0 && <div className="inbox-choices" role="group" aria-label={`Choices from ${labels[questions.indexOf(picked)]}`}>{picked.choices.map(choice => <button type="button" className="inbox-choice" key={choice} aria-pressed={text === choice} disabled={!!pending} onClick={event => choose(picked, choice, event.detail > 0)}><span className="inbox-choice-radio" aria-hidden="true" /><span>{choice}</span>{recommended !== undefined && choice.trim().toLowerCase() === recommended && <span className="inbox-rec"><UiIcon name="spark" />Recommended</span>}</button>)}</div>}
        {dragging && <p className="inbox-note">Drop images to attach</p>}
        {mode === "followup" && images.controls(!!pending || busy)}
        {(mode === "new" || mode === "followup") && !images.ready && <p role="status" className="inbox-note">{images.entries.some(item => item.status === "preparing") ? "Preparing images…" : "Remove or retry the failed image to send."}</p>}
        {mode === "new" && <span className="inbox-field-label">Your request</span>}
        <div className="inbox-compose-row"><textarea ref={composerRef} aria-label={direct ? "Message agent" : mode === "answer" ? "Answer to the selected question" : mode === "choose" ? "Reply (choose a target first)" : thread ? "Follow up with owner" : "Your request"} placeholder={direct ? `Message ${direct.agent_name}…` : mode === "answer" ? "Type an answer or pick a choice" : mode === "choose" ? "Choose a question to answer, or Follow up" : thread ? questions.length ? "Follow up without answering…" : `Follow up with ${ownerLabel(view, thread.owner_root_ids[0])}…` : "Describe the work or ask a question"} value={text} onChange={e => setText(e.target.value)} onPaste={e => { if ((mode === "new" || mode === "followup") && current && view?.image_limits && !pending && !busy && e.clipboardData.files.length) { if (!Array.from(e.clipboardData.types).includes("text/plain")) e.preventDefault(); images.add(e.clipboardData.files); } }} onKeyDown={fieldKey} disabled={busy || mode === "choose"} readOnly={!!pending} rows={2} enterKeyHint="send" />
          {mode === "new" && images.controls(!!pending || busy)}
          {mode === "new" ? <div className="inbox-compose-footer">{images.picker(!!pending || busy || !current || snapshot?.session_id !== sessionId)}<p className="inbox-compose-hint is-keys">Enter to send · Shift+Enter for a new line</p>{sendControl}</div> : <>{mode === "followup" && images.picker(!!pending || busy || !current || snapshot?.session_id !== sessionId)}{sendControl}</>}</div>
        {mode === "answer" && <p className="inbox-compose-hint">Answers are text only{images.entries.length > 0 ? ` — the ${images.entries.length} image${images.entries.length === 1 ? "" : "s"} in your follow-up draft ${images.entries.length === 1 ? "is" : "are"} not included` : ""}. <button type="button" className="inbox-link" onClick={() => pickTarget("", true)}>Follow up to attach images</button></p>}
        {mode === "direct" && <p className="inbox-compose-hint">Images can be sent with new work or follow-ups, not direct messages.</p>}
        {mode !== "new" && <p className="inbox-compose-hint is-keys">Enter to send · Shift+Enter for a new line{thread && mode !== "choose" ? ` · Esc returns to the ${mode === "answer" ? "question" : "thread"}` : ""}</p>}
        {mode === "new" && <details key={wideDetail ? "wide" : "narrow"} open={wideDetail || undefined} className="inbox-ctx" data-set={!!(attachedSpace || repository || branch || base)}><summary><UiIcon name="right" /><span>Context · optional{(attachedSpace || repository || branch || base) ? " · set" : ""}</span><span className="inbox-ctx-help">Only what you set here is sent with the request.</span></summary><div className="inbox-ctx-body"><label className="is-check"><input type="checkbox" checked={!!attachedSpace} onChange={e => setAttachedSpace(e.target.checked ? currentSpace : null)} disabled={(!currentSpace && !attachedSpace) || !!pending} /> Attach current Space {(attachedSpace?.label || currentSpace?.label) && <span className="inbox-space-chip">{attachedSpace?.label || currentSpace?.label}</span>}</label><div className="inbox-ctx-fields"><label>Repository ID <input value={repository} onChange={e => setRepository(e.target.value)} disabled={!!pending} /></label><label>Branch <input value={branch} onChange={e => setBranch(e.target.value)} disabled={!!pending} /></label><label>Base ref <input value={base} onChange={e => setBase(e.target.value)} disabled={!!pending} /></label></div></div></details>}
      </form>
  );
  return <section className="inbox" data-detail={mobileDetail} hidden={!visible} aria-label="Inbox">
    <nav className="inbox-list" aria-label="Work needing attention" aria-busy={!view && !loadError || undefined} onClickCapture={e => {
      const button = (e.target as HTMLElement).closest<HTMLButtonElement>("button.inbox-row, button.inbox-new");
      if (button) originRef.current = button;
    }} onKeyDown={e => {
      // Arrow keys only move focus between rows; opening a row stays an explicit Enter/Space or click.
      if ((e.key !== "ArrowDown" && e.key !== "ArrowUp") || e.altKey || e.ctrlKey || e.metaKey || e.shiftKey) return;
      const rows = Array.from(e.currentTarget.querySelectorAll<HTMLButtonElement>("button.inbox-row")).filter(row => !row.closest("details:not([open])"));
      const next = rows[rows.indexOf(document.activeElement as HTMLButtonElement) + (e.key === "ArrowDown" ? 1 : -1)];
      if (rows.includes(document.activeElement as HTMLButtonElement) && next) { e.preventDefault(); next.focus(); }
    }}><header><div className="inbox-list-title"><h1>Inbox</h1>{count > 0 && <span className="inbox-count">{count} need you</span>}</div><button ref={newRef} type="button" className="inbox-new" onClick={() => select(null)}><UiIcon name="plus" /> New work</button></header>
      {!view && !loadError && <><p role="status" className="inbox-empty">Loading inbox…</p><div className="inbox-skeleton" aria-hidden="true">{[0, 1, 2, 3].map(i => <div key={i}><span /><div><i /><i /></div></div>)}</div></>}
      {loadError && <div role="status" className="inbox-note" data-tone="error"><UiIcon name="alert" /><div><strong>Inbox unavailable.</strong>{view && <> Showing the last loaded items.</>}<div>{loadError}</div><small>Retrying automatically.</small></div></div>}
      {view?.observation.status === "unavailable" && <p role="status" className="inbox-note" title={view.observation.error.message}><UiIcon name="alert" />Can’t see live sessions right now. Replies are paused; your draft is kept.</p>}
      {attention.availability === "unavailable" && <p role="status" className="inbox-note"><UiIcon name="alert" />Agent attention unavailable.</p>}
      {view && !view.threads.some(t => t.state === "needs_you") && !view.unmanaged_agents.some(relevant) && <p className="inbox-empty"><UiIcon name="check-circle" />Nothing needs you right now.</p>}
      {count > 0 && <section aria-label={`Needs you · ${count}`}><h2 className="inbox-group">Needs you <span>· {count}</span></h2>
        {view?.threads.filter(t => t.state === "needs_you").map(t => <ThreadRow key={t.thread_id} thread={t} badge={badge(t)} selected={selected === t.thread_id} onSelect={() => select(t.thread_id)} />)}
        {view?.unmanaged_agents.filter(relevant).map(a => <AgentRow key={a.pane_id} agent={a} context={contextFor(a)} selected={selected === `agent:${a.pane_id}`} onSelect={() => selectAgent(a)} />)}
      </section>}
      {underway.length > 0 && <details className="inbox-others" open><summary><UiIcon name="right" />Working · {underway.length}</summary>{underway.map(t => <ThreadRow key={t.thread_id} thread={t} badge={badge(t)} selected={selected === t.thread_id} onSelect={() => select(t.thread_id)} />)}</details>}
      {handled.length > 0 && <details className="inbox-others" open={handled.some(t => t.thread_id === selected) || undefined}><summary><UiIcon name="right" />Handled · {handled.length}</summary>{handled.map(t => <ThreadRow key={t.thread_id} thread={t} badge={badge(t)} selected={selected === t.thread_id} onSelect={() => select(t.thread_id)} />)}</details>}
      {view && view.unmanaged_agents.some(a => !relevant(a)) && <details className="inbox-others"><summary><UiIcon name="right" />Other agents · {view.unmanaged_agents.filter(a => !relevant(a)).length}</summary>{view.unmanaged_agents.filter(a => !relevant(a)).map(a => <AgentRow key={a.pane_id} agent={a} context={contextFor(a)} selected={selected === `agent:${a.pane_id}`} quiet onSelect={() => selectAgent(a)} />)}</details>}
    </nav>
    <main ref={detailRef} className="inbox-detail" aria-label={thread ? `Thread: ${thread.title.text}` : undefined} onKeyDown={e => { if (e.key === "Escape" && !e.defaultPrevented && mobileDetail && backRef.current?.offsetParent && !(e.target as HTMLElement).closest("dialog, textarea, input, select, [role=alertdialog]")) leaveDetail(); }}>
      {thread ? <><DetailHeader backRef={backRef} onBack={leaveDetail} tone={tone(thread).kind} icon={tone(thread).icon} title={threadHeadline(thread)} full={thread.title.text} facts={<p className="inbox-facts"><span className="inbox-state">{tone(thread).label}</span>{thread.project && <span>{thread.project}</span>}{thread.owner_root_ids.map(id => <span key={id}>{ownerLabel(view, id)}</span>)}<span>Updated <Time value={thread.updated_at} /></span></p>}>
        {targets.map(target => <button type="button" className="inbox-btn" key={target.key} title={target.full} aria-label={target.full} disabled={view?.observation.status !== "fresh"} onClick={() => open(target.location)}><UiIcon name="terminal" /><span>{targets.length === 1 ? "Open session" : target.label}</span></button>)}
        {view?.owners.filter(o => thread.owner_root_ids.includes(o.root_id) && (!o.location || o.availability !== "available")).map(o => <span className="inbox-offline" key={o.root_id}>{ownerLabel(view, o.root_id)} · {o.availability === "starting" ? "starting" : o.availability === "absent" ? "not running" : "status unknown"}</span>)}
      </DetailHeader><div className="inbox-tabs" role="tablist" aria-label="Thread views" onKeyDown={e => { if (e.key !== "ArrowLeft" && e.key !== "ArrowRight") return; e.preventDefault(); const next = detailView === "thread" ? "work" : "thread"; setDetailView(next); requestAnimationFrame(() => document.getElementById(`${viewId}-${next}`)?.focus()); }}>
        {(["thread", "work"] as const).map(name => <button key={name} type="button" role="tab" id={`${viewId}-${name}`} aria-selected={detailView === name} aria-controls={`${viewId}-${name === "thread" ? "main" : "work"}`} tabIndex={detailView === name ? 0 : -1} onClick={() => setDetailView(name)}>{name === "thread" ? "Thread" : <>Work{" "}<span className="inbox-tab-count">{thread.tasks.length}</span></>}</button>)}
      </div><div className="inbox-body" data-view={detailView}><section className="inbox-main" id={`${viewId}-main`}>
        <span className="inbox-sr" role="status">{announcement}</span>
        <div className="inbox-timeline" ref={timelineRef} role="region" aria-label="Thread timeline" tabIndex={0}><InboxTimeline thread={thread} view={view} client={client} sessionId={sessionId} ownerLabel={id => ownerLabel(view, id)} visible={visible && current} questionCard={question => { const index = questions.indexOf(question), key = answerDraftKey(thread.thread_id, question); return <InboxQuestionCard question={question} label={labels[index]!} imagesLabel={`Attached by ${source(question.text)}`} index={index} total={questions.length} selected={picked === question} updated={!!carried[key] && !!(drafts[key] ?? "").trim()} client={client} sessionId={sessionId} onSelect={pointer => pickTarget(questionKey(question), pointer)} />; }} resultActions={resultActions} acknowledged={acknowledged} recommendation={recommendationNote} /></div>
        {composer}
      </section><InboxWork id={`${viewId}-work`} thread={thread} view={view} client={client} sessionId={sessionId} open={open} liveOpen={view?.observation.status === "fresh"} diagnostics={<Evidence thread={thread} view={view} displayed={displayed} />} />
      </div></> : direct ? <><DetailHeader backRef={backRef} onBack={leaveDetail} tone="unknown" icon="user" title={direct.agent_name} facts={<p className="inbox-facts"><span>{direct.workspace_label} · {direct.tab_label}</span><span>Direct agent · {liveDirect?.agent.status || "status unavailable"}</span></p>}>
        <button type="button" className="inbox-btn" aria-label={`Open ${direct.agent_name} session`} disabled={view?.observation.status !== "fresh"} onClick={() => open({ workspace_id: direct.workspace_id, workspace_label: direct.workspace_label, tab_id: direct.tab_id, tab_label: direct.tab_label, pane_id: direct.pane_id })}><UiIcon name="terminal" /><span>Open session</span></button>
      </DetailHeader><div className="inbox-content">{(directContext?.title || directContext?.activity) && <section aria-label="Session context">{directContext.title && <div className="inbox-update inbox-session-context" dir="auto"><small>{directContext.titleLabel}</small>{directContext.title}</div>}{directContext.activity && <div className="inbox-update inbox-session-context" dir="auto"><small>{directContext.activityLabel} · OMP session</small>{directContext.activity}</div>}</section>}<p className="inbox-placeholder">A direct message goes straight to this agent’s terminal; Cockpit does not manage the agent. Delivery means submitted, not read.</p></div>{composer}</>
      : selected ? <><DetailHeader backRef={backRef} onBack={leaveDetail} tone="unknown" icon="alert" title="Work no longer available" facts={<p className="inbox-facts">It is not in the current inbox. Any draft you typed is kept.</p>} /><div className="inbox-content"><p className="inbox-placeholder">Pick another item from the queue, or start new work.</p></div></>
      : <><DetailHeader backRef={backRef} onBack={leaveDetail} tone="review" icon="plus" title="New work" facts={<p className="inbox-facts">Send a request to a liaison.</p>} />{view ? composer : <div className="inbox-content"><p className="inbox-placeholder" role="status">{loadError ? "The inbox is unavailable, so new work cannot be started yet. Retrying automatically." : "Loading the inbox… new work can be started once it has loaded."}</p></div>}</>}
    </main>
  </section>;
}
function AgentRow({ agent, context, selected, quiet = false, onSelect }: { agent: UnmanagedAgent; context: AgentContext | null; selected: boolean; quiet?: boolean; onSelect(): void }) {
  const preview = context?.activity || context?.title;
  if (!preview) return quiet
    ? <button type="button" className="inbox-row is-quiet" data-tone="unknown" aria-current={selected ? "true" : undefined} onClick={onSelect}><span className="inbox-kic is-sm"><UiIcon name="user" /></span><span className="inbox-row-body"><strong>{agent.agent_name}</strong><span className="inbox-row-next">{agent.workspace_label} · {agent.agent_status || "unknown"}</span></span></button>
    : <button type="button" className="inbox-row" data-tone="decision" aria-current={selected ? "true" : undefined} onClick={onSelect}><span className="inbox-kic" aria-hidden="true"><UiIcon name="user" /></span><span className="inbox-row-body"><span className="inbox-meta"><span className="inbox-meta-t">Direct agent · {agent.workspace_label} · {agent.tab_label}</span></span><strong>{agent.agent_name}</strong><span className="inbox-row-next">{agent.agent_status || "Status unknown"}</span></span></button>;
  return <button type="button" className={`inbox-row${quiet ? " is-quiet" : ""}`} data-tone={quiet ? "unknown" : "decision"} aria-current={selected ? "true" : undefined} onClick={onSelect} title={[agent.workspace_label, agent.tab_label, agent.agent_name, agent.agent_status, context?.title && `${context.titleLabel}: ${context.title}`, context?.activity && `${context.activityLabel}: ${context.activity}`].filter(Boolean).join(" · ")}><span className={`inbox-kic${quiet ? " is-sm" : ""}`} aria-hidden="true"><UiIcon name="user" /></span><span className="inbox-row-body"><span className="inbox-meta"><span className="inbox-meta-t">Direct agent · {agent.workspace_label} · {agent.tab_label} · {agent.agent_status || "Status unknown"}</span></span><strong>{agent.agent_name}</strong>{context?.activity && context.title && <span className="inbox-sr">{context.titleLabel}: {context.title}</span>}<span className="inbox-row-next"><span className="inbox-sr">{context?.activity ? context.activityLabel : context?.titleLabel}: </span>{preview}</span></span></button>;
}
function ThreadRow({ thread, badge, selected, onSelect }: { thread: InboxThread; badge: string | null; selected: boolean; onSelect(): void }) {
  const state = tone(thread), quiet = thread.state !== "needs_you";
  return <button type="button" className={`inbox-row${quiet ? " is-quiet" : ""}`} data-tone={state.kind} aria-current={selected ? "true" : undefined} onClick={onSelect} title={thread.title.text}><span className={`inbox-kic${quiet ? " is-sm" : ""}`} aria-hidden="true"><UiIcon name={state.icon} /></span><span className="inbox-row-body"><span className="inbox-sr">{state.label}: </span><span className="inbox-meta"><span className="inbox-meta-t"><span className="inbox-row-state" aria-hidden="true">{state.label}</span> · <Time value={thread.updated_at} />{thread.project && ` · ${thread.project}`}</span>{badge && <span className="inbox-draft-badge"><UiIcon name="edit" />{badge}</span>}</span><strong>{threadHeadline(thread)}</strong><span className="inbox-row-next">{outcomeLine(thread)}</span></span></button>;
}

/** One header for every detail view: back (narrow only), state icon, title over facts, then actions. */
function DetailHeader({ backRef, onBack, tone: kind, icon, title, full, facts, children }: { backRef: RefObject<HTMLButtonElement | null>; onBack(): void; tone: string; icon: UiIconName; title: string; full?: string; facts: ReactNode; children?: ReactNode }) {
  return <header className="inbox-thread-header" data-tone={kind}>
    <button ref={backRef} type="button" className="inbox-back" aria-label="Back to inbox" onClick={onBack}><UiIcon name="back" /></button>
    <span className="inbox-kic"><UiIcon name={icon} /></span>
    <div className="inbox-ht"><h2 title={full ?? title}>{title}</h2>{facts}</div>
    {children}
  </header>;
}

function CopyValue({ value, label }: { value: string; label?: string }) {
  const [status, setStatus] = useState("");
  return <div className="inbox-copy"><span title={value}>{value}</span><button type="button" aria-label={label ? `Copy ${label} ${value}` : undefined} onClick={() => { if (!navigator.clipboard) { setStatus("Copy unavailable; select the text"); return; } void navigator.clipboard.writeText(value).then(() => setStatus("Copied"), () => setStatus("Copy unavailable; select the text")); }}>Copy</button><span className="inbox-sr" role="status">{status}</span></div>;
}

function Evidence({ thread, view, displayed }: { thread: InboxThread; view: InboxView | null; displayed: InboxText[] }) {
  const binding = thread.request?.handoff.binding?.requested;
  const fields: Array<[string, string | null | undefined]> = [["Repository", binding?.repository_id], ["Space", binding?.workspace_id], ["Branch", binding?.branch], ["Base ref", binding?.base_ref]];
  const namedFields: Array<[string, InboxText | null | undefined]> = [["title", thread.title], ["next move", thread.next], ["recommendation", thread.recommendation], ["update", thread.reply], ...thread.questions.map(q => ["question", q.text] as [string, InboxText])];
  const aiFields = namedFields.filter(([, value]) => value?.source.kind === "owner_card" && (displayed.includes(value) || value === thread.recommendation));
  const labelledMessageIds = new Set([thread.request?.message_id, ...thread.questions.map(q => q.question_message_id)]);
  const messageIds = Array.from(new Set([
    ...[thread.title, thread.next, thread.recommendation, thread.reply, ...thread.facts].map(value => value?.message_id),
    ...thread.receipts.map(receipt => receipt.message_id),
  ].filter((id): id is string => !!id && !labelledMessageIds.has(id))));
  const facts = new Map<string, { fact: InboxText; count: number }>();
  for (const fact of thread.facts) {
    if (fact.source.kind === "worker") continue;
    const text = fact.text.replace(/^Cleanup Removed:\s*$/, "Cleanup removed");
    const key = `${fact.source.kind}:${text}`;
    const existing = facts.get(key);
    if (existing) existing.count++;
    else facts.set(key, { fact: { ...fact, text }, count: 1 });
  }
  return <div className="inbox-evidence">
    {thread.next && thread.next.source.kind === "owner_card" && !displayed.includes(thread.next) && <><h3 className="inbox-ev-h">Next move</h3><InboxFeedback text={thread.next.text} /></>}
    {thread.facts.some(fact => fact.source.kind === "worker") && <><h3 className="inbox-ev-h">Worker-reported evidence</h3>{thread.facts.map((fact, i) => {
      if (fact.source.kind !== "worker") return null;
      const runId = fact.source.run_id;
      const task = thread.tasks.find(task => task.evidence?.run_id === runId && task.evidence.result?.summary === fact.text);
      return <details className="inbox-claim" key={i}><summary><span className="inbox-claim-chevron" aria-hidden="true">▸</span><span className="inbox-tag" data-kind="worker">{task ? "Worker claim" : "Worker-reported evidence"}</span><span>{task && <><strong>{task.title}</strong><br /></>}Run <code>{runId}</code></span></summary><div className="inbox-claim-body inbox-md" dir="auto"><InboxFeedback text={fact.text} /></div></details>;
    })}</>}
    {facts.size > 0 && <><h3 className="inbox-ev-h">Recorded steps</h3><ul className="inbox-evidence-list">{Array.from(facts.values()).map(({ fact, count }, i) => <li className="inbox-fact" key={i}>{srcKind(fact) !== "recorded" && <span className="inbox-tag" data-kind={srcKind(fact)} title={source(fact)}>{srcLabel(fact)}</span>}<InboxFeedback text={fact.text} />{count > 1 && <span>×{count}</span>}</li>)}</ul></>}
    {thread.receipts.length > 0 && <><h3 className="inbox-ev-h">Delivery</h3><ul className="inbox-evidence-list">{thread.receipts.map(r => <li className="inbox-evidence-row" key={r.message_id}><strong>{r.kind === "answer" ? "Answer" : "Request"} to {ownerLabel(view, r.to_run_id)}</strong><span>{deliveryLabel(r.stage)}{r.stale ? " · stale" : ""} · <Time value={r.acked_at || r.created_at} /></span></li>)}</ul></>}
    {thread.tasks.length > 0 && <><h3 className="inbox-ev-h">Tasks</h3><ul className="inbox-evidence-list">{thread.tasks.map(t => <li className="inbox-evidence-row" key={`${t.root_id}:${t.task_id}`}><strong>{t.title}</strong><span>{[t.lane, ...(t.run ? [t.run.stage !== t.lane ? t.run.stage : null, t.run.presence] : [])].filter(Boolean).join(" · ")}</span></li>)}</ul></>}
    {aiFields.length > 0 && <><h3 className="inbox-ev-h">Source</h3><p className="inbox-ev-p">AI summary for {Array.from(new Set(aiFields.map(([name]) => name))).join(", ")} from {Array.from(new Set(aiFields.map(([, v]) => v?.source.kind === "owner_card" ? v.source.harness : ""))).join(", ")}.</p></>}
    <h3 className="inbox-ev-h">IDs</h3><dl className="inbox-kv"><dt>Thread</dt><dd><CopyValue value={thread.thread_id} label="thread ID" /></dd>{fields.map(([label, value]) => value && uuid.test(value) && <div key={label}><dt>{label}</dt><dd><CopyValue value={value} label={`${label.toLowerCase()} ID`} /></dd></div>)}{thread.owner_root_ids.map((id, i) => <div key={`owner-${i}`}><dt>Owner</dt><dd><CopyValue value={id} label="owner ID" /></dd></div>)}{thread.questions.map((q, i) => <div key={`question-${i}`}><dt>Question</dt><dd><CopyValue value={q.question_message_id} label="question ID" /></dd></div>)}{thread.request && <div><dt>Request</dt><dd><CopyValue value={thread.request.message_id} label="request ID" /></dd></div>}{messageIds.map(id => <div key={`message-${id}`}><dt>Message</dt><dd><CopyValue value={id} label="message ID" /></dd></div>)}{thread.receipts.map((r, i) => r.attempt_run_id && <div key={`receipt-${i}`}><dt>Attempt run</dt><dd><CopyValue value={r.attempt_run_id} label="attempt run ID" /></dd></div>)}{thread.tasks.filter(t => t.run).map(t => <div key={t.task_id}><dt>Attempt run</dt><dd><CopyValue value={t.run!.run_id} label="attempt run ID" /></dd></div>)}</dl>
  </div>;
}
