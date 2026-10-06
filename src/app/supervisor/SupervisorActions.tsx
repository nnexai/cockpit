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
