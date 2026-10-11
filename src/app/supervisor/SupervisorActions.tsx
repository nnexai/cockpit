import { useId, useRef, useState } from "react";
import type { OrchestrationAction, OrchestrationActionResult, OrchestrationSnapshot, QuestionStatus, Report, Run, RunObservation, Subagent, TaskView } from "../../protocol/generated/v1";
import { StateGlyph, type GlyphShape } from "../sidebar/StateGlyph";
import { UiIcon } from "../UiIcon";
import { messageDraft, type ScopeDrafts, type TextDraft } from "./useSupervisorDrafts";
import { cleanupForRun, cleanupItemView, cleanupResourceLabel, cleanupView, outcomeView, taskRun } from "./retirementView";
import { DependenciesSection } from "./SupervisorDependencies";
import { StepsSection } from "./SupervisorSteps";
import type { SupervisorStepsProps } from "./stepInteractions";
import { taskContentReason } from "./dependencies";
import type { MutationRequestScope } from "./useSupervisor";

export type Mutation = (action: OrchestrationAction, requestScope?: MutationRequestScope) => Promise<OrchestrationActionResult | null>;
export type CaptureRequestScope = (action: OrchestrationAction) => MutationRequestScope | null;
export type AgentState = { kind: "ready" | "starting" | "failure" | "missing" | "unknown" | "offline" | "closed"; label: string; detail: string; verified: boolean; blocked: boolean; terminal: boolean; restartable: boolean };
export function agentState(snapshot: OrchestrationSnapshot, run: Run, connected: boolean, runtimeLive: boolean): AgentState {
  if (run.stage === "closed") {
    return { kind: "closed", label: outcomeView(run)?.label ?? "Tracking closed", detail: "Tasks and history are saved. Cleanup is tracked separately.", verified: false, blocked: false, terminal: false, restartable: false };
  }
  const observed = snapshot.runtime.status === "fresh" && connected && runtimeLive ? snapshot.runtime.runs.find(item => item.run_id === run.run_id) : undefined;
  const terminal = observed?.presence === "present" && !!observed.pane_id;
  const resuming = !!run.resume && ["reconciling", "waiting_quiescence", "launching"].includes(run.resume.phase);
  const inspectableRoot = (run.kind === "supervisor" || run.kind === "adopted")
    && run.run_id === run.root_id && run.parent_run_id === null && run.checkout_claim_id === null;
  const restartable = snapshot.runtime.status === "fresh" && connected && runtimeLive
    && (observed?.presence !== "endpoint_changed" || inspectableRoot) && !resuming;
  const state = (kind: AgentState["kind"], label: string, detail: string, verified = false): AgentState => ({ kind, label, detail, verified, blocked: verified && observed?.agent_status === "blocked", terminal, restartable });
  if (!connected || !runtimeLive) return state("offline", "Connection lost", "Showing saved tasks. The agent may still be running.");
  if (snapshot.runtime.status !== "fresh") return state("offline", "Cannot check the agent right now", "Showing saved tasks. Herdr observation is unavailable; no absence is inferred.");
  if (resuming) return state("starting", "Resuming…", "Reconnecting the agent to its saved tasks.");
  if (run.resume?.phase === "blocked") return state("failure", "Can't resume", run.resume.cause?.message ?? "The agent could not be reconnected. Saved tasks and work are kept.");
  if (observed?.presence === "missing") return state("missing", "Agent terminal is gone", "The last observed terminal is no longer available. Tasks and history are saved.");
  if (observed?.presence === "endpoint_changed") return state("unknown", "Cannot confirm the agent", "The server identity changed. Waiting for fresh matching agent identity before resuming.");
  if (run.stage === "queued") return state("starting", "Worker queued", "The worker is waiting to start.");
  const step = run.dispatch?.step;
  if (step === "plan_failed" && run.dispatch?.error) return state("failure", "Agent did not start", run.dispatch.error.message);
  if (step === "setup_unknown") return state("unknown", "Agent setup is unconfirmed", `${run.dispatch?.error?.message ?? "Setup could not be confirmed."} Existing resources are kept. Review recovery before launching.`);
  if (["launch_unknown", "needs_review"].includes(step ?? "")) return state("unknown", "Agent start is unconfirmed", `${run.dispatch?.error?.message ? `${run.dispatch.error.message} ` : ""}Startup is not confirmed. Another launch could create a duplicate.`);
  const attachment = run.attachment;
  const verified = !!observed?.actual_omp && observed.presence === "present"
    && attachment?.state === "connected" && !!attachment.main_session_id
    && (step === "launched" || run.kind === "adopted" && !run.dispatch);
  if (verified) {
    const board = snapshot.board?.root_id === run.root_id ? snapshot.board : null;
    const hasWork = !!board?.tasks.some(task => task.lane !== "accepted")
      || snapshot.runs.some(child => child.root_id === run.root_id && child.run_id !== run.run_id && child.stage !== "closed")
      || snapshot.assignment_intents.some(intent => intent.root_id === run.root_id)
      || run.last_report?.kind === "needs_input";
    const rootLabel = hasWork ? "Managing tasks" : board ? "Ready for a task" : "OMP connected";
    return state("ready", observed?.agent_status === "blocked" ? "Agent blocked" : run.kind === "worker" ? "Agent connected" : rootLabel, "OMP is connected. The supervisor manages work and reviews results.", true);
  }
  if (run.stage === "starting" || step === "launch_pending" || step === "launch_intent") return state("starting", "Starting…", "Waiting for OMP to start and connect.");
  return state("unknown", "Cannot confirm the agent", "No fresh, bound OMP process is confirmed. Saved reports are not live process evidence.");
}
export function questionForRun(snapshot: OrchestrationSnapshot, run: Run): QuestionStatus | null {
  return run.stage === "closed" ? null : snapshot.questions.find(question => question.run_id === run.run_id) ?? null;
}
export function questionLabel(question: QuestionStatus): string {
  switch (question.receipt.status) {
    case "unresolved": return "Waiting for supervisor";
    case "answer_delivered": return "Answer sent · not yet processed";
    case "answer_acknowledged": return "Answer acknowledged · awaiting report";
  }
}
export function taskStatus(task: TaskView, run: Run | undefined, snapshot: OrchestrationSnapshot): string {
  run ??= taskRun(snapshot, task);
  const assignment = snapshot.assignment_intents.find(intent => intent.root_id === snapshot.board?.root_id && intent.task_id === task.task.task_id);
  if (assignment) return assignment.state === "conflict" ? "Not assigned · task changed elsewhere" : "Assignment pending";
  if (task.task.checked) {
    const outcome = run?.stage === "closed" && run.close_reason === "accepted" && task.lane === "accepted" ? outcomeView(run) : null;
    return outcome ? outcome.label === "Done" ? "Done" : `Done · ${outcome.label}` : "Marked complete in task file";
  }
  if (run?.stage === "closed") return run.close_reason === "failed" ? "Failed · tracking closed" : "Tracking closed";
  const question = run ? questionForRun(snapshot, run) : null;
  if (question) return questionLabel(question);
  if (run?.stage === "reported" || task.lane === "review") return run?.result?.outcome === "succeeded" ? "Source complete · awaiting acceptance" : "Reviewing result";
  if (task.lane === "working") return "Working";
  if (task.lane === "starting") return "Starting";
  if (!run && task.dependencies.state === "invalid") return "Queued · prerequisites need fixing";
  if (!run && task.dependencies.state === "blocked") return `Queued · waiting on ${task.dependencies.unmet.length} prerequisites`;
  return run ? "Queued" : snapshot.messages.some(message => message.message_id === `assign-${task.task.task_id}` && message.to_run_id === snapshot.board?.root_id) ? "Assigned · waiting for agent" : "Queued · not assigned";
}
export type StateBlockView = { sentence: string; tierLabel: "Decide" | "Recover" | "Notice" | null; waitingSince: string | null };
export type PathRowView = { key: string; role: string; label: string; facts: string[]; current: boolean; depth: number; subagent: boolean; onActivate(): void };
export type CrossView = { label: "Show in Graph" | "Show in Tasks" | "Show in Dependencies"; onActivate(): void } | null;
export function reportAge(report: Pick<Report, "at">): string {
  const minutes = Math.max(0, Math.floor((Date.now() - Date.parse(report.at)) / 60_000));
  return minutes < 1 ? "just now" : minutes < 60 ? `${minutes}m ago` : minutes < 1440 ? `${Math.floor(minutes / 60)}h ago` : `${Math.floor(minutes / 1440)}d ago`;
}
export function observedStatus(run: Run, observed: RunObservation | undefined, snapshot: OrchestrationSnapshot, live: boolean): { word: string; glyph: GlyphShape } {
  const raw = run.stage !== "closed" && live && snapshot.runtime.status === "fresh" ? observed?.presence === "present" ? observed.actual_omp ? observed.agent_status ?? "unknown" : "OMP not confirmed" : observed?.presence ?? "unobserved" : "unobserved";
  return { word: raw.replaceAll("_", " "), glyph: raw === "working" || raw === "idle" || raw === "blocked" || raw === "done" ? raw : "unknown" };
}
export function ProvenancePair({ run, report, observed, snapshot, live, compact = false }: {
  run: Run; report: Report | null | undefined; observed: RunObservation | undefined; snapshot: OrchestrationSnapshot; live: boolean; compact?: boolean;
}) {
  if (run.stage === "closed") return null;
  const status = observedStatus(run, observed, snapshot, live);
  const reportedAt = report ? new Date(report.at).toLocaleString() : null;
  const observedAt = live && snapshot.runtime.status === "fresh" ? snapshot.runtime.observed_at : null;
  return <div className={`supervisor-pair${compact ? " is-compact" : ""}`}>
    <span className="supervisor-pair-reported" title={reportedAt ?? undefined} aria-label={report ? `Reported by ${run.label}: ${reportAge(report)} · ${reportedAt}` : `Reported by ${run.label}: no progress reported yet`}>
      reported {report ? <time dateTime={report.at}>{reportAge(report)}</time> : "none yet"}
    </span>
    <span className="supervisor-pair-observed" title={observedAt ? new Date(observedAt).toLocaleString() : undefined} aria-label={`Observed by Herdr: ${status.word}${observedAt ? ` · ${new Date(observedAt).toLocaleString()}` : ""}`}>
      observed <StateGlyph shape={status.glyph} /> {status.word}
    </span>
    {!compact ? <><ReportedEvidence report={report} source={run.label} showBody /><ObservedEvidence run={run} observed={observed} snapshot={snapshot} live={live} /></> : null}
  </div>;
}
export function ProgressTrail({ run, task }: { run: Run; task: TaskView | null; snapshot: OrchestrationSnapshot }) {
  const accepted = run.close_reason === "accepted" && task?.task.checked && task.lane === "accepted";
  return <section className="supervisor-detail-group supervisor-trail" aria-label="Progress trail"><h3>Progress trail</h3><ol>
    {task?.current_run_id === run.run_id ? <li>Assigned</li> : null}
    {run.attachment?.main_session_id ? <li>Started</li> : null}
    {run.result?.outcome === "succeeded" ? <li>Source complete</li> : run.result ? <li>Result reported · {run.result.outcome ?? "reported"}</li> : null}
    {accepted ? <li>Done</li> : null}
  </ol></section>;
}
export type RecoveryFlags = { busy: boolean; connected: boolean; runtimeLive: boolean };
export type RecoveryAction = { kind: "terminal" | "restart"; label: string; primary: boolean; consequence?: string; disabled: boolean; ariaDisabled?: boolean; reason?: string };
export function canRestart(state: AgentState, flags: RecoveryFlags, run: Run): boolean {
  return state.restartable && flags.connected && flags.runtimeLive && !flags.busy && run.stage !== "reported" && run.stage !== "closed";
}
function restartReason(state: AgentState, flags: RecoveryFlags, run: Run): string | undefined {
  if (flags.busy) return "Wait for the current operation.";
  if (!flags.connected || !flags.runtimeLive || state.kind === "offline") return "Connection and fresh matching agent identity are required before resuming.";
  if (run.stage === "reported") return "Review or send back the explicit result before resuming.";
  if (run.stage === "closed") return "Tracking is closed.";
  if (!state.restartable) return state.label === "Resuming…" ? "The agent is already resuming." : "Waiting for fresh matching agent identity before resuming.";
}
export function recoveryActions(run: Run, state: AgentState, flags: RecoveryFlags): RecoveryAction[] {
  const actions: RecoveryAction[] = [];
  if (state.kind !== "ready" && run.stage !== "closed" && run.stage !== "reported") {
    actions.push({ kind: "restart", label: state.label === "Resuming…" ? "Resuming…" : "Resume", primary: true, disabled: !canRestart(state, flags, run), ariaDisabled: true, reason: restartReason(state, flags, run) });
  }
  if (state.terminal && state.kind !== "closed" && flags.connected && flags.runtimeLive) {
    actions.push({ kind: "terminal", label: "Open terminal", primary: state.kind === "ready", disabled: flags.busy, reason: flags.busy ? "Wait for the current operation." : undefined });
  }
  return actions;
}
export function ReportedEvidence({ report, source, showBody = false }: { report: Report | null | undefined; source: string; showBody?: boolean }) {
  return <p className="supervisor-evidence"><span>Reported · {source}</span> {report ? <>{showBody || report.outcome === "failed" ? <span className="supervisor-report-summary">{report.summary}</span> : null}<time dateTime={report.at} title={new Date(report.at).toLocaleString()}>{reportAge(report)}</time></> : <span>No progress reported yet</span>}</p>;
}
export function ObservedEvidence({ run, observed, snapshot, live }: { run: Run; observed: RunObservation | undefined; snapshot: OrchestrationSnapshot; live: boolean }) {
  if (run.stage === "closed") return null;
  const status = observedStatus(run, observed, snapshot, live);
  return <p className="supervisor-evidence supervisor-observed"><span>Observed · Herdr</span><StateGlyph shape={status.glyph} /><span>{status.word}</span>{live && snapshot.runtime.status === "fresh" ? <time dateTime={snapshot.runtime.observed_at} title={new Date(snapshot.runtime.observed_at).toLocaleString()}>{new Date(snapshot.runtime.observed_at).toLocaleTimeString()}</time> : null}</p>;
}
function AnswerEvidence({ question, snapshot }: { question: QuestionStatus; snapshot: OrchestrationSnapshot }) {
  if (question.receipt.status === "unresolved") return null;
  const answer = question.receipt.answer;
  const sender = answer.sender.type === "operator" ? "You" : answer.sender.type === "dispatcher" ? "Dispatcher"
    : snapshot.runs.find(run => answer.sender.type === "run" && run.run_id === answer.sender.run_id)?.label ?? "Agent";
  const stage = { stored: "stored", woken: "agent notified", read: "read by agent, not yet acknowledged", acked: "acknowledged" }[answer.stage];
  return <>
    <p className="supervisor-evidence"><span>Answered · {sender}</span><time dateTime={answer.created_at} title={new Date(answer.created_at).toLocaleString()}>{reportAge({ at: answer.created_at })}</time><span>· {stage}</span>{answer.stage === "acked" && answer.acked_at ? <time dateTime={answer.acked_at} title={new Date(answer.acked_at).toLocaleString()}>{reportAge({ at: answer.acked_at })}</time> : null}</p>
    {question.receipt.status === "answer_acknowledged" ? <p className="supervisor-muted">A receipt, not proof that work resumed.</p> : null}
  </>;
}
export function TextAction({ label, submitLabel, draft, changed, busy, submit, success, doneResult = false, retryable = true, describedBy, createAction, captureRequestScope, onNotice }: {
  label: string; submitLabel: string; draft: TextDraft; changed(): void; busy: boolean;
  submit(text: string, id: string, action?: OrchestrationAction, requestScope?: MutationRequestScope): Promise<OrchestrationActionResult | null>; success: string; doneResult?: boolean; retryable?: boolean; describedBy?: string; createAction?(text: string, id: string): OrchestrationAction; captureRequestScope?: CaptureRequestScope; onNotice?(text: string): void;
}) {
  const id = useId();
  const errorId = useId();
  const inFlight = useRef(false);
  const [pending, setPending] = useState(false);
  const send = async () => {
    if (busy || inFlight.current || !draft.text.trim() || draft.operation && !retryable) return;
    if (!draft.operation) {
      const id = crypto.randomUUID();
      const action = createAction?.(draft.text, id);
      const requestScope = action ? captureRequestScope?.(action) : undefined;
      if (action && !requestScope) { draft.error = "Reconnect before sending. Your draft is kept; the original request scope is unavailable."; changed(); return; }
      draft.operation = { id, text: draft.text, ...(action ? { action, requestScope: requestScope ?? undefined } : {}) };
    }
    const submitted = draft.operation;
    if (submitted.action && !submitted.requestScope) { draft.error = "The original request scope is unavailable. Your original request is kept and was not sent."; changed(); return; }
    draft.error = null; draft.notice = null; inFlight.current = true; setPending(true); changed();
    try {
      const result = submitted.action ? await submit(submitted.text, submitted.id, submitted.action, submitted.requestScope) : await submit(submitted.text, submitted.id);
      const confirmed = submitted.action?.action === "task_scope_update"
        ? result?.result === "task_scope_updated" && result.update.operation_id === submitted.id && result.update.state === "applied" && !!result.update.instruction_message_id
        : result?.result === (doneResult ? "done" : "message");
      if (confirmed) {
        if (draft.text === submitted.text) draft.text = "";
        draft.operation = null; draft.notice = success; onNotice?.(success);
      } else draft.error = retryable ? "Delivery was not confirmed. Your draft and original operation are kept. Check status, then explicitly retry the same action." : "The action was not confirmed. Check its current receipt before requesting another action; repeating it could duplicate the previous request.";
    } catch { draft.error = "Could not send this action. Your draft and original operation are kept."; }
    finally { inFlight.current = false; setPending(false); changed(); }
  };
  return <form className="supervisor-action-form" aria-busy={pending} onSubmit={event => { event.preventDefault(); void send(); }}>
    <label htmlFor={id}>{label}</label>
    <textarea id={id} rows={3} value={draft.text} readOnly={pending || !!draft.operation} aria-describedby={[describedBy, draft.error ? errorId : undefined].filter(Boolean).join(" ") || undefined} onChange={event => { draft.text = event.target.value; draft.notice = null; changed(); }} />
    <div className="supervisor-action-row"><button type="submit" aria-describedby={describedBy} disabled={busy || pending || !draft.text.trim() || !!draft.operation && !retryable}>{pending ? "Sending…" : draft.operation ? retryable ? draft.operation.action && draft.operation.action.action !== "message_send" ? "Retry same action" : "Retry same message" : "Check previous action first" : submitLabel}</button></div>
    {draft.operation && !pending ? <p>{retryable ? "Resolve the previous action before editing this draft; retry keeps its original payload and delivery identity." : "After checking the previous control receipt or history, explicitly unlock the retained draft only if a new request is still needed."}</p> : null}
    {draft.operation && !pending && !retryable ? <button type="button" disabled={busy} onClick={() => { draft.operation = null; draft.error = null; changed(); }}>I reviewed the previous action; unlock draft</button> : null}
    <div className={`supervisor-feedback${draft.error ? " is-error" : ""}`} id={errorId} role={draft.error ? "alert" : undefined}>{draft.error || draft.notice ? <><UiIcon name={draft.error ? "info" : "check"} /><span>{draft.error ?? draft.notice}</span></> : null}</div>
  </form>;
}
export function AgentRecovery({ run, state, busy, connected, runtimeLive, onRestart, onTerminal }: {
  run: Run; state: AgentState; busy: boolean; connected: boolean; runtimeLive: boolean;
  onRestart(): void; onTerminal(): void;
}) {
  const id = useId();
  const flags = { busy, connected, runtimeLive };
  const recovery = ["failure", "missing", "unknown"].includes(state.kind);
  return <section className={`supervisor-agent-status is-${state.kind}`} aria-label={`${run.label} status`}>
    <div className="supervisor-status-heading"><strong>{run.label}</strong><span className="supervisor-state"><StateGlyph shape={state.blocked ? "blocked" : state.verified ? "live" : "unknown"} tone={recovery ? "warning" : state.kind === "offline" || state.kind === "closed" ? "muted" : undefined} />{state.label}</span></div>
    {state.kind !== "ready" ? <p role={recovery ? "alert" : undefined}>{state.detail}</p> : null}
    <div className="supervisor-action-row">
      {recoveryActions(run, state, flags).map(action => {
        const descriptionId = `${id}-${action.kind}`;
        return <div className="supervisor-recovery-action" key={action.kind}>
          <button type="button" className={action.primary ? "supervisor-primary" : undefined} aria-label={action.label} disabled={action.ariaDisabled ? undefined : action.disabled} aria-disabled={action.ariaDisabled ? action.disabled : undefined} aria-describedby={action.reason || action.consequence ? descriptionId : undefined} onClick={() => {
            if (action.disabled) return;
            if (action.kind === "restart") onRestart();
            else onTerminal();
          }}>{action.kind === "terminal" ? <UiIcon name="terminal" /> : null}{action.label}</button>
          {action.reason || action.consequence ? <p className="supervisor-disabled-reason" id={descriptionId}>{action.consequence}{action.reason ? <>{action.consequence ? " · " : ""}{action.reason}</> : null}</p> : null}
        </div>;
      })}
    </div>
    {run.stage === "reported" && recovery ? <p>The explicit result must be reviewed or sent back before resuming.</p> : null}
  </section>;
}
export function RunDiagnostics({ run, snapshot }: { run: Run; snapshot: OrchestrationSnapshot }) {
  return <div className="supervisor-diagnostics">
    <h3>{run.label} · durable records</h3><p>Launch receipts and saved reports are not current process proof.</p>
    <section className="supervisor-detail-group"><h4>Run, binding and setup</h4><pre className="supervisor-plan">{JSON.stringify(run, null, 2)}</pre></section>
    <section className="supervisor-detail-group"><h4>Inbox delivery and provenance</h4>{snapshot.messages.filter(message => message.to_run_id === run.run_id || message.from.type === "run" && message.from.run_id === run.run_id).map(message => <details key={message.message_id}><summary>{message.report?.kind === "initialized" ? "Initialized" : message.kind.replaceAll("_", " ")} · {message.stage}{message.stale ? " · stale" : ""} · {message.created_at}</summary><pre className="supervisor-plan">{JSON.stringify(message, null, 2)}</pre></details>)}</section>
  </div>;
}
const cleanupInFlight = new WeakSet<TextDraft>();
type CleanupFinishProps = { rootId: string; runId?: string | null; scope: ScopeDrafts; changed(): void; mutateResult: Mutation; captureRequestScope?: CaptureRequestScope; onNotice?(text: string): void };
export async function finishCleanup({ rootId, runId = null, scope, changed, mutateResult, captureRequestScope, onNotice }: CleanupFinishProps): Promise<void> {
  const draft = messageDraft(scope, `cleanup:${rootId}:${runId ?? "all"}`);
  if (cleanupInFlight.has(draft)) return;
  if (!draft.operation) {
    const id = crypto.randomUUID();
    const action: OrchestrationAction = { action: "cleanup_finish", request: { operation_id: id, root_id: rootId, run_id: runId } };
    const requestScope = captureRequestScope?.(action);
    if (!requestScope) { draft.error = "Reconnect before finishing cleanup. No request was sent; its scope is unavailable."; changed(); return; }
    draft.operation = { id, text: "", action, requestScope };
  }
  const submitted = draft.operation;
  if (submitted.action?.action !== "cleanup_finish") return;
  if (!submitted.requestScope) { draft.error = "The original cleanup scope is unavailable. Your request is kept and was not sent."; changed(); return; }
  cleanupInFlight.add(draft); draft.error = null; draft.notice = null; changed();
  try {
    const result = await mutateResult(submitted.action, submitted.requestScope);
    if (result?.result === "cleanup_finished" && result.cleanup.operation_id === submitted.id) {
      if (result.cleanup.unknown_count) draft.error = "Cleanup not confirmed. The original request is kept; retry uses the same operation.";
      else if (result.cleanup.waiting_count) { draft.notice = "Cleanup waiting. The runtime keeps retrying automatically."; draft.operation = null; }
      else {
        const kept = result.cleanup.items.find(item => item.cleanup.state === "kept");
        draft.notice = kept ? `Cleanup finished · kept: ${kept.cleanup.reason?.message ?? "protected work"}` : "Cleanup finished";
        draft.operation = null;
      }
      if (draft.notice) onNotice?.(draft.notice);
    } else draft.error = "Cleanup not confirmed. The original request and scope are kept; retry uses the same operation.";
  } catch { draft.error = "Cleanup not confirmed. The original request is kept."; }
  finally { cleanupInFlight.delete(draft); changed(); }
}
export function CleanupFinishAction({ busy, live, hideButton = false, ...props }: CleanupFinishProps & { busy: boolean; live: boolean; hideButton?: boolean }) {
  const draft = messageDraft(props.scope, `cleanup:${props.rootId}:${props.runId ?? "all"}`);
  const pending = cleanupInFlight.has(draft);
  const id = useId();
  const reason = busy || pending ? "Wait for the current operation." : !live ? "Reconnect before finishing cleanup." : null;
  return <div className="supervisor-recovery-action">
    {!hideButton ? <button type="button" aria-disabled={!!reason} aria-describedby={reason ? `${id}-reason` : undefined} onClick={() => { if (!reason) void finishCleanup(props); }}>{pending ? "Finishing cleanup…" : draft.operation ? "Retry same cleanup" : "Finish cleanup"}</button> : null}
    {!hideButton && reason ? <p className="supervisor-disabled-reason" id={`${id}-reason`}>{reason}</p> : null}
    {draft.error || draft.notice ? <p className={draft.error ? "supervisor-error" : undefined} role={draft.error ? "alert" : undefined}>{draft.error ?? draft.notice}</p> : null}
  </div>;
}
function RequestStop({ task, supervisor, scope, changed, busy, mutateResult, describedBy, captureRequestScope, onNotice }: { task: TaskView; supervisor: Run; scope: ScopeDrafts; changed(): void; busy: boolean; mutateResult: Mutation; describedBy?: string; captureRequestScope?: CaptureRequestScope; onNotice?(text: string): void }) {
  const [confirm, setConfirm] = useState(false);
  const inFlight = useRef(false);
  const draft = messageDraft(scope, `stop:${task.task.task_id}`);
  const request = async () => {
    if (busy || inFlight.current) return;
    if (!draft.operation) {
      const id = crypto.randomUUID();
      const text = `Please stop work on canonical task ${task.task.task_id}. Inspect its current task and descendant runs, cancel scoped work where appropriate, and report outstanding effects. Existing files and Spaces must stay.`;
      const action: OrchestrationAction = { action: "message_send", message_id: id, to_run_id: supervisor.run_id, kind: "instruction", text, in_reply_to: null };
      const requestScope = captureRequestScope?.(action);
      if (!requestScope) { draft.error = "Reconnect before requesting stop. Your draft is kept; no request was sent."; changed(); return; }
      draft.operation = { id, text, action, requestScope };
    }
    if (!draft.operation.requestScope) { draft.error = "The original stop request scope is unavailable. Your request is kept and was not sent."; changed(); return; }
    inFlight.current = true; changed();
    try {
      const result = await mutateResult(draft.operation.action!, draft.operation.requestScope);
      if (result?.result === "message" && result.to_run_id === supervisor.run_id) { draft.operation = null; draft.notice = "Stop requested. Waiting for the supervisor; this is not proof the process stopped."; draft.error = null; setConfirm(false); onNotice?.(draft.notice); }
      else draft.error = "The stop request was not confirmed. Retry keeps the same message identity.";
    } finally { inFlight.current = false; changed(); }
  };
  return <section>{confirm ? <><p>Ask the supervisor to stop this task? Existing files and Spaces stay.</p><button type="button" disabled={busy} aria-describedby={describedBy} onClick={() => void request()}>{draft.operation ? "Retry same stop request" : "Request stop"}</button><button type="button" disabled={busy} onClick={() => setConfirm(false)}>Keep working</button></> : <button type="button" disabled={busy} aria-describedby={describedBy} onClick={() => setConfirm(true)}>Request stop…</button>}{draft.notice ? <p>{draft.notice}</p> : null}{draft.error ? <p className="supervisor-error" role="alert">{draft.error}</p> : null}</section>;
}
export function SupervisorActions({ snapshot, run, task, subagent, section = "overview", stateBlock, path, crossView, crossViews = [], acceptanceConflict, scope, changed, busy, live, mutateResult, captureRequestScope, onNotice, steps, taskWriteUnconfirmed = false, onNavigateTask, onEditPrerequisites, onCreateFollowUp, onResumeSourceDraft, onTerminal, onEditTask, onCancelSubagent, onCloseTracking }: {
  snapshot: OrchestrationSnapshot; run: Run | null; task: TaskView | null; subagent?: Subagent | null; section?: "overview" | "activity" | "actions";
  stateBlock: StateBlockView; path: readonly PathRowView[]; crossView: CrossView; acceptanceConflict: boolean;
  scope: ScopeDrafts; changed(): void; busy: boolean; live: boolean; mutateResult: Mutation;
  captureRequestScope?: CaptureRequestScope;
  onNotice?(text: string): void;
  onTerminal(run: Run): void; onEditTask(task: TaskView): void; onCancelSubagent(run: Run, subagent: Subagent): void; onCloseTracking(run: Run): void;
  steps?: SupervisorStepsProps; taskWriteUnconfirmed?: boolean; crossViews?: Exclude<CrossView, null>[];
  onNavigateTask?(taskId: string): void; onEditPrerequisites?(task: TaskView): void; onCreateFollowUp?(task: TaskView): void;
  onResumeSourceDraft?(taskId: string, kind: "edit" | "relations" | "follow_up"): void;
}) {
  const id = useId();
  const [operatorOpen, setOperatorOpen] = useState(false);
  const observed = snapshot.runtime.status === "fresh" && live && run ? snapshot.runtime.runs.find(item => item.run_id === run.run_id) : undefined;
  const canTerminal = run?.stage !== "closed" && observed?.presence === "present" && !!observed.pane_id;
  const controls = !!run && run.stage !== "closed" && live;
  const supervisor = snapshot.runs.find(root => root.run_id === snapshot.board?.root_id && root.stage === "active");
  const canRequestStop = supervisor && agentState(snapshot, supervisor, live, live).verified && task && !task.task.checked && (!run || run.stage !== "closed");
  const childDraft = run && subagent ? messageDraft(scope, `subagent:${run.run_id}:${subagent.subagent_id}`) : null;
  const unavailableReason = busy ? "Wait for the current operation." : !live ? "Check connection and current agent status first." : null;
  const contentReason = taskContentReason(snapshot, task, live);
  const editReason = unavailableReason ?? (taskWriteUnconfirmed ? "A task change is unconfirmed. Read and resolve the original operation first." : contentReason ?? (!task?.task.description_editable ? task?.task.description_diagnostic ?? "The description cannot be safely edited." : null));
  const relationReason = unavailableReason ?? (taskWriteUnconfirmed ? "A task change is unconfirmed. Resolve it before changing prerequisites." : snapshot.runs.find(root => root.run_id === snapshot.board?.root_id)?.stage === "closed" ? "Tracking is closed; tasks cannot be changed." : task?.task.checked ? "Task is complete; prerequisites are read-only." : task?.task.diagnostic ? "Resolve the canonical task diagnostic before editing." : acceptanceConflict ? "Resolve the pending acceptance decision before changing prerequisites." : null);
  const createReason = unavailableReason ?? (snapshot.runs.find(root => root.run_id === snapshot.board?.root_id)?.stage === "closed" ? "Tracking is closed; tasks cannot be created." : null);
  const followupDraft = run ? messageDraft(scope, `followup:${run.run_id}`) : null;
  const workingFollowup = run?.kind === "worker" && run.stage === "working";
  let followupReason = unavailableReason ?? (run?.stage === "closed" ? "Tracking is closed; follow-up is unavailable." : null);
  if (!followupReason && run && workingFollowup && !followupDraft?.operation) {
    if (!task || snapshot.board?.root_id !== run.root_id) followupReason = "The canonical task is unavailable.";
    else if (task.current_run_id !== run.run_id || task.task.task_id !== run.task_id) followupReason = "This is not the task's current worker.";
    else if (taskWriteUnconfirmed || acceptanceConflict) followupReason = "Resolve the unconfirmed task change or acceptance decision first.";
    else if (task.task.checked) followupReason = "Task is complete; its scope is read-only.";
    else if (task.task.diagnostic) followupReason = task.task.diagnostic;
    else if (!task.task.description_editable) followupReason = task.task.description_diagnostic ?? "The description cannot be safely edited.";
  }
  const stopReason = unavailableReason ?? (!canRequestStop ? "A verified active supervisor is required to request stop." : null);
  const acceptReason = unavailableReason ?? (!task ? "The canonical task is unavailable." : taskWriteUnconfirmed ? "Resolve the unconfirmed task change before acceptance." : task.dependencies.state === "blocked" || task.dependencies.state === "invalid" ? "Acceptance waits until every prerequisite is uniquely identified and checked in this task file. A Result alone does not satisfy it." : task.task.diagnostic ? "Resolve the canonical task diagnostic before acceptance." : task.current_run_id !== run?.run_id ? "This is not the task's current run." : run?.result?.kind !== "result" || run.result.outcome !== "succeeded" ? "Acceptance requires an explicit successful Result, not runtime Done." : null);
  const childReason = unavailableReason ?? (!controls ? "Tracking is closed; subagent controls are unavailable." : subagent?.status !== "running" ? "Only a running subagent can receive controls." : null);
  const forcedOperatorOpen = acceptanceConflict;
  const cleanupSummary = run && !subagent ? cleanupForRun(snapshot, run.run_id) : undefined;
  const cleanup = cleanupView(cleanupSummary);
  const outcome = run && !subagent ? outcomeView(run) : null;
  const detailTier = cleanup?.tier === "notice" ? "Notice" : cleanup?.tier === "recover" ? "Recover" : stateBlock.tierLabel;
  const hasCleanup = !!run && (snapshot.cleanup?.some(summary => {
    if (!summary.items.length) return false;
    let candidate = snapshot.runs.find(item => item.run_id === summary.run_id);
    for (let depth = 0; candidate && depth <= snapshot.runs.length; depth++) {
      if (candidate.run_id === run.run_id) return true;
      if (candidate.root_id !== run.root_id || !candidate.parent_run_id) return false;
      candidate = snapshot.runs.find(item => item.run_id === candidate!.parent_run_id);
    }
    return false;
  }) || !!scope.messages.get(`cleanup:${run.root_id}:${run.run_id}`)?.operation);
  const finishCleanup = run && !subagent && hasCleanup ? <CleanupFinishAction rootId={run.root_id} runId={run.run_id} scope={scope} changed={changed} busy={busy} live={live && snapshot.runtime.status === "fresh"} mutateResult={mutateResult} captureRequestScope={captureRequestScope} onNotice={onNotice} /> : null;
  const question = run && !subagent ? questionForRun(snapshot, run) : null;
  return <div className="supervisor-task-detail">
    <div className="supervisor-detail-section" hidden={section !== "overview"}>
      {subagent && run ? <p>{subagent.role ?? "OMP subagent"} · In {run.label} · no terminal of its own</p> : null}
      <section className="supervisor-detail-group supervisor-state-block">
        <h3>State {detailTier ? <span className="supervisor-attention-badge">{detailTier}</span> : null}</h3>
        <p>{outcome ? `${run?.close_reason === "accepted" && outcome.label !== "Done" ? "Done · " : ""}${outcome.label}` : stateBlock.sentence}</p>
        {task && run && !subagent && (task.dependencies.state === "blocked" || task.dependencies.state === "invalid") ? <p className="supervisor-muted">A prerequisite is still open or needs repair. This does not stop the running agent.</p> : null}
        {!outcome && stateBlock.waitingSince ? <p className="supervisor-muted">Since <time dateTime={stateBlock.waitingSince} title={new Date(stateBlock.waitingSince).toLocaleString()}>{reportAge({ at: stateBlock.waitingSince })}</time></p> : null}
        {question ? <AnswerEvidence question={question} snapshot={snapshot} /> : null}
        {run ? subagent ? <>
          <p>OMP events · {subagent.status} · <time dateTime={subagent.updated_at}>{new Date(subagent.updated_at).toLocaleString()}</time></p>
          <p className="supervisor-exact-text">{subagent.summary ?? "No summary reported."}</p>
        </> : <>
          <ProvenancePair run={run} report={run.last_report} observed={observed} snapshot={snapshot} live={live} />
          {run.stage === "closed" ? <ReportedEvidence report={run.last_report} source={run.label} showBody /> : null}
        </> : <p>No worker progress reported yet.</p>}
      </section>
      {outcome ? <section className="supervisor-detail-group" aria-label="Outcome"><h3>Outcome</h3><p>{outcome.label}</p><p>{outcome.detail}</p></section> : null}
      {run && !subagent && (cleanupSummary || hasCleanup || run.close_reason === "accepted") ? <section className="supervisor-detail-group">
        <h3>Cleanup</h3>
        {cleanup ? <p role={cleanup.tier === "recover" ? "alert" : undefined}>{cleanup.label}</p> : <p>Cleanup not observed yet</p>}
        {cleanupSummary?.items.length ? <ul aria-label="Cleanup">{cleanupSummary.items.map(item => <li key={item.resource_id}>{cleanupResourceLabel(item)} · {cleanupItemView(item.cleanup)}</li>)}</ul> : null}
        {finishCleanup}
      </section> : null}
      {task && !subagent && !run && (task.dependencies.state === "blocked" || task.dependencies.state === "invalid") && onNavigateTask && onEditPrerequisites ? <DependenciesSection view={task} tasks={snapshot.board?.tasks ?? []} snapshot={snapshot} onNavigate={onNavigateTask} onEdit={onEditPrerequisites} writable={!relationReason} /> : null}
      {!subagent && steps ? <StepsSection {...steps} /> : null}
      {run && !subagent && run.result ? <section className="supervisor-detail-group">
        <h3>Result · {run.result.outcome ?? "reported"}</h3>
        <p className="supervisor-exact-text">{run.result.summary}</p>
        <p>Explicit report from {run.label} · <time dateTime={run.result.at}>{new Date(run.result.at).toLocaleString()}</time>{run.close_reason === "accepted" ? " · Accepted" : " · Awaiting review"}</p>
      </section> : null}
      {run && !subagent ? <ProgressTrail run={run} task={task} snapshot={snapshot} /> : null}
      {task && !subagent && (run || task.dependencies.state !== "blocked" && task.dependencies.state !== "invalid") && onNavigateTask && onEditPrerequisites ? <DependenciesSection view={task} tasks={snapshot.board?.tasks ?? []} snapshot={snapshot} onNavigate={onNavigateTask} onEdit={onEditPrerequisites} writable={!relationReason} /> : null}
      {path.length ? <section className="supervisor-detail-group supervisor-path" aria-label="Relationship path">
        <h3>Relationship path</h3>
        <ol>{path.map(row => <li key={row.key} style={{ paddingInlineStart: `${row.depth * 16}px` }}><button type="button" className={`supervisor-path-row${row.subagent ? " is-subagent" : ""}`} aria-current={row.current ? "true" : undefined} onClick={row.onActivate}>
          <strong>{row.role} · {row.label}</strong>{row.facts.length ? <span>{row.facts.join(" · ")}</span> : null}
        </button></li>)}</ol>
      </section> : null}
      {crossView ? <button type="button" className="supervisor-cross-view" onClick={crossView.onActivate}>{crossView.label}</button> : null}
      {crossViews.map(link => <button type="button" className="supervisor-cross-view" key={link.label} onClick={link.onActivate}>{link.label}</button>)}
      {task?.task.checked ? <p className="supervisor-muted">Completed tasks are not drawn in Graph. Use Tasks or Dependencies for accepted prerequisite context.</p> : null}
      {task ? <section className="supervisor-detail-group"><h3>Task description</h3><p className="supervisor-exact-text">{task.task.description}</p>{task.task.description_diagnostic ? <p className="supervisor-warning">{task.task.description_diagnostic}</p> : null}{task.task.diagnostic ? <p className="supervisor-error">{task.task.diagnostic}</p> : null}</section> : null}
    </div>
    <div className="supervisor-detail-section" hidden={section !== "activity"}>
      {run ? subagent ? <section className="supervisor-detail-group">
        {subagent.last_control ? <><h3>Last control · {subagent.last_control.op.op} · {subagent.last_control.stage}</h3><p>A stored request is not applied control. Applied delivery is not task completion.</p>{subagent.last_control.op.op === "send" ? <p className="supervisor-exact-text">{subagent.last_control.op.text}</p> : null}{subagent.last_control.error ? <p className="supervisor-error">{subagent.last_control.error}</p> : null}<time dateTime={subagent.last_control.at}>{new Date(subagent.last_control.at).toLocaleString()}</time></> : <><h3>Last control</h3><p>No control receipt.</p></>}
      </section> : <>
        {run.last_report?.plan ? <section className="supervisor-detail-group"><h3>Advisory plan</h3><p className="supervisor-exact-text">{run.last_report.plan}</p><p>Reported by {run.label}; not a permission gate.</p></section> : null}
        {run.last_report?.kind === "initialized" ? <section className="supervisor-detail-group"><h3>Initialized</h3><p className="supervisor-exact-text">{run.last_report.summary}</p><p>Reported by {run.label} · {run.last_report.at}</p></section> : null}
      </> : null}
    </div>
    <div className="supervisor-detail-section" hidden={section !== "actions"}>
      <section className="supervisor-detail-group supervisor-routine" aria-label="Routine">
        <h3>Routine</h3>
        {run && canTerminal ? <button type="button" disabled={busy || !live} onClick={() => onTerminal(run)}>{subagent ? "Open parent terminal" : "Open terminal"}</button> : null}
        {finishCleanup}
        {run && subagent && childDraft ? <>
          <TextAction onNotice={onNotice} key={`subagent:${run.run_id}:${subagent.subagent_id}`} label="Message to subagent" submitLabel="Send message" draft={childDraft} changed={changed} busy={busy || !controls || subagent.status !== "running"} describedBy={childReason ? `${id}-child` : undefined} createAction={text => ({ action: "subagent_control", run_id: run.run_id, subagent_id: subagent.subagent_id, op: { op: "send", text } })} captureRequestScope={captureRequestScope} submit={(_text, _id, action, requestScope) => mutateResult(action!, requestScope)} success="Control request stored. Check its receipt for applied delivery." retryable={false} />
          <button type="button" disabled={busy || !controls || subagent.status !== "running"} aria-describedby={childReason ? `${id}-child` : undefined} onClick={() => onCancelSubagent(run, subagent)}>Cancel subagent…</button>
          {childReason ? <p className="supervisor-disabled-reason" id={`${id}-child`}>{childReason}</p> : null}
        </> : <>
          {run ? <>
            <TextAction onNotice={onNotice} key={`followup:${run.run_id}`} label="Follow-up to agent" submitLabel="Send follow-up" draft={followupDraft!} changed={changed} busy={!!followupReason} describedBy={followupReason ? `${id}-followup` : undefined} createAction={(text, operation_id) => workingFollowup && task && snapshot.board ? { action: "task_scope_update", operation_id, root_id: run.root_id, task_id: task.task.task_id, run_id: run.run_id, expected_task_revision: task.task.task_revision, expected_doc_revision: snapshot.board.doc_revision, title: null, description: `${task.task.description}\n\n${text}` } : { action: "message_send", message_id: operation_id, to_run_id: run.run_id, kind: "instruction", text, in_reply_to: null }} captureRequestScope={captureRequestScope} submit={(_text, _id, action, requestScope) => mutateResult(action!, requestScope)} success={workingFollowup || followupDraft?.operation?.action?.action === "task_scope_update" ? "Update sent · task description updated" : "Follow-up sent. Waiting for the agent."} />
            {followupReason ? <p className="supervisor-disabled-reason" id={`${id}-followup`}>{followupReason}</p> : null}
          </> : null}
          {task ? <>
            <button type="button" disabled={busy || !!editReason && !scope.edits.get(task.task.task_id)?.submitted} aria-describedby={editReason ? `${id}-edit` : undefined} onClick={() => onEditTask(task)}>Edit task…{scope.edits.has(task.task.task_id) ? " · draft kept" : ""}</button>
            {editReason ? <p className="supervisor-disabled-reason" id={`${id}-edit`}>{editReason}</p> : null}
            {onEditPrerequisites ? <button type="button" disabled={busy || !!relationReason && !scope.relations.get(task.task.task_id)?.submitted} aria-describedby={relationReason ? `${id}-relations` : undefined} onClick={() => onEditPrerequisites(task)}>Edit prerequisites…{scope.relations.has(task.task.task_id) ? " · draft kept" : ""}</button> : null}
            {relationReason ? <p className="supervisor-disabled-reason" id={`${id}-relations`}>{relationReason}</p> : null}
            {onCreateFollowUp ? <button type="button" disabled={busy || !!createReason && !scope.followUps.get(task.task.task_id)?.submitted} aria-describedby={createReason ? `${id}-create` : undefined} onClick={() => onCreateFollowUp(task)}>Create follow-up…{scope.followUps.has(task.task.task_id) ? " · draft kept" : ""}</button> : null}
            {createReason ? <p className="supervisor-disabled-reason" id={`${id}-create`}>{createReason}</p> : null}
          </> : null}
          {!task && steps && onResumeSourceDraft ? <>
            {scope.edits.has(steps.scope.taskId) ? <button type="button" disabled={busy} onClick={() => onResumeSourceDraft(steps.scope.taskId, "edit")}>Review kept description draft…</button> : null}
            {scope.relations.has(steps.scope.taskId) ? <button type="button" disabled={busy} onClick={() => onResumeSourceDraft(steps.scope.taskId, "relations")}>Review kept prerequisite draft…</button> : null}
            {scope.followUps.has(steps.scope.taskId) ? <button type="button" disabled={busy} onClick={() => onResumeSourceDraft(steps.scope.taskId, "follow_up")}>Review kept follow-up draft…</button> : null}
          </> : null}
          {task && !task.task.checked && (!run || run.stage !== "closed") ? <>
            {supervisor ? <RequestStop key={JSON.stringify([supervisor.run_id, task.task.task_id])} task={task} supervisor={supervisor} scope={scope} changed={changed} busy={busy || !live || !canRequestStop} mutateResult={mutateResult} captureRequestScope={captureRequestScope} onNotice={onNotice} describedBy={stopReason ? `${id}-stop` : undefined} /> : <button type="button" disabled aria-describedby={`${id}-stop`}>Request stop…</button>}
            {stopReason ? <p className="supervisor-disabled-reason" id={`${id}-stop`}>{stopReason}</p> : null}
          </> : null}
          {run ? <>
            <TextAction onNotice={onNotice} key={`note:${run.run_id}`} label="Durable note" submitLabel="Save note" draft={messageDraft(scope, `note:${run.run_id}`)} changed={changed} busy={busy || !live} describedBy={unavailableReason ? `${id}-note` : undefined} createAction={text => ({ action: "annotate", run_id: run.run_id, text })} captureRequestScope={captureRequestScope} submit={(_text, _id, action, requestScope) => mutateResult(action!, requestScope)} success="Note saved." doneResult retryable={false} />
            {unavailableReason ? <p className="supervisor-disabled-reason" id={`${id}-note`}>{unavailableReason}</p> : null}
          </> : null}
        </>}
      </section>
      {run && !subagent ? <details className="supervisor-operator" open={operatorOpen || forcedOperatorOpen} onToggle={event => {
        if (forcedOperatorOpen && !event.currentTarget.open) event.currentTarget.open = true;
        setOperatorOpen(event.currentTarget.open);
      }}>
        <summary>Advanced</summary>
        {run.stage === "reported" ? <section className="supervisor-detail-group">
          <p>The supervisor normally reviews this result. Acceptance uses the exact current canonical task revision, not runtime Done.</p>
          <button type="button" disabled={!!acceptReason} aria-describedby={acceptReason ? `${id}-accept` : undefined} onClick={() => {
            if (!acceptReason && task && run.result?.outcome === "succeeded") void mutateResult({ action: "accept", run_id: run.run_id, expected_task_revision: task.task.task_revision });
          }}>Accept explicit result</button>
          {acceptReason ? <p className="supervisor-disabled-reason" id={`${id}-accept`}>{acceptReason}</p> : null}
          <TextAction onNotice={onNotice} key={`sendback:${run.run_id}`} label="Requested changes" submitLabel="Send back" draft={messageDraft(scope, `sendback:${run.run_id}`)} changed={changed} busy={busy || !live} describedBy={unavailableReason ? `${id}-review` : undefined} createAction={text => ({ action: "send_back", run_id: run.run_id, text })} captureRequestScope={captureRequestScope} submit={(_text, _id, action, requestScope) => mutateResult(action!, requestScope)} success="Changes sent back." doneResult retryable={false} />
          {unavailableReason ? <p className="supervisor-disabled-reason" id={`${id}-review`}>{unavailableReason}</p> : null}
        </section> : null}
        {run.stage !== "closed" ? <section className="supervisor-detail-group">
          <button type="button" className="supervisor-quiet" disabled={busy || !live} aria-describedby={unavailableReason ? `${id}-close` : undefined} onClick={() => onCloseTracking(run)}>Close agent tracking…</button>
          {unavailableReason ? <p className="supervisor-disabled-reason" id={`${id}-close`}>{unavailableReason}</p> : null}
        </section> : null}
      </details> : null}
    </div>
  </div>;
}
