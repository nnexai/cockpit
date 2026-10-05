import { useEffect, useId, useRef, useState, type ReactNode } from "react";
import type { OrchestrationAction, OrchestrationSnapshot, PlanRecord, Report, Run, Subagent, TaskView, WorkspaceRecoveryAction } from "../../protocol/generated/v1";

export type SupervisorActionsProps = {
  snapshot: OrchestrationSnapshot;
  run: Run | null;
  task: TaskView | null;
  subagent: Subagent | null;
  busy: boolean;
  error: string | null;
  mutate: (action: OrchestrationAction) => Promise<boolean>;
  onTerminal: (run: Run) => Promise<void>;
  onEditTask: (task: TaskView) => void;
  onPropose: (task: TaskView) => void;
};

function useEscapeDisarm(armed: boolean, disarm: () => void) {
  useEffect(() => {
    if (!armed) return;
    const onKey = (event: globalThis.KeyboardEvent) => {
      if (event.key !== "Escape" || event.isComposing) return;
      event.preventDefault();
      event.stopPropagation();
      disarm();
    };
    document.addEventListener("keydown", onKey);
    return () => document.removeEventListener("keydown", onKey);
  }, [armed, disarm]);
}

function Section({ title, children }: { title: string; children: ReactNode }) {
  return <section className="supervisor-section"><h3>{title}</h3>{children}</section>;
}

function ReportDetail({ report, absent }: { report: Report | null; absent: string }) {
  return report ? <><p>{report.kind} · {report.outcome ?? "no outcome"} · <time>{report.at}</time> · worker report</p><pre className="supervisor-plan">{report.summary}</pre>{report.plan !== null ? <><h4>Reported plan · exact text</h4><pre className="supervisor-plan">{report.plan}</pre></> : null}</> : <p>{absent}</p>;
}

function TextAction({ label, submitLabel, busy, submit }: {
  label: string; submitLabel: string; busy: boolean;
  submit: (text: string, messageId: string) => Promise<boolean>;
}) {
  const id = useId();
  const [text, setText] = useState("");
  const messageId = useRef<string | null>(null);
  return <form className="supervisor-action-form" onSubmit={(event) => {
    event.preventDefault();
    if (busy || !text.trim()) return;
    messageId.current ??= crypto.randomUUID();
    void submit(text, messageId.current).then((ok) => { if (ok) { setText(""); messageId.current = null; } });
  }}><label htmlFor={id}>{label}</label><textarea id={id} rows={3} value={text} disabled={busy} onChange={(event) => { setText(event.target.value); messageId.current = null; }} /><button type="submit" disabled={busy || !text.trim()}>{submitLabel}</button></form>;
}

function PlanGrant({ run, scope, plan, busy, mutate }: {
  run: Run; scope: "prepare" | "execute"; plan: PlanRecord | null; busy: boolean;
  mutate: (action: OrchestrationAction) => Promise<boolean>;
}) {
  const [reviewed, setReviewed] = useState(plan?.plan_revision ?? null);
  const [armedRevision, setArmedRevision] = useState<string | null>(null);
  const [reviewRequired, setReviewRequired] = useState(false);
  const [note, setNote] = useState("");
  const noteId = useId();
  useEffect(() => { if (reviewed === null && plan) setReviewed(plan.plan_revision); }, [plan, reviewed]);
  const changed = !!plan && reviewed !== null && reviewed !== plan.plan_revision;
  useEscapeDisarm(armedRevision !== null, () => setArmedRevision(null));
  const confirm = async () => {
    if (busy || !plan || changed || reviewRequired || armedRevision !== plan.plan_revision || reviewed !== plan.plan_revision) return;
    const ok = await mutate(scope === "prepare"
      ? { action: "grant_prepare", run_id: run.run_id, plan_revision: armedRevision }
      : { action: "grant_execute", run_id: run.run_id, plan_revision: armedRevision, note: note.trim() ? note : null });
    setArmedRevision(null);
    if (!ok) setReviewRequired(true);
  };
  return <Section title={scope === "prepare" ? "Prepare authorization" : "Execute authorization"}>
    {plan ? <><p>Plan revision · SHA-256</p><code>{plan.plan_revision}</code><p>Created <time>{plan.created_at}</time></p><pre className="supervisor-plan">{plan.text}</pre></> : <p>No plan available yet. Authorization is unavailable.</p>}
    <p>{scope === "prepare" ? "Prepare authorizes the exact setup and worker launch above, followed by read-only initialization." : "Execute authorizes the initialized worker to carry out the exact work plan above."} This is a same-user policy, not an OS sandbox. Grants are single-use.</p>
    {scope === "execute" ? <label htmlFor={noteId}>Optional execution note<textarea id={noteId} rows={2} value={note} disabled={busy} onChange={(event) => setNote(event.target.value)} /></label> : null}
    {changed || reviewRequired ? <div className="supervisor-confirmation" role="alert"><p>{changed ? `Plan changed after you opened it (${reviewed} → ${plan?.plan_revision}). Review the changed exact text above before authorizing.` : "Authorization was not confirmed. Review the latest exact plan above before arming again; no retry is automatic."}</p><button type="button" disabled={busy} onClick={() => { setReviewed(plan?.plan_revision ?? null); setArmedRevision(null); setReviewRequired(false); }}>Review new plan</button></div>
      : armedRevision !== null ? <div className="supervisor-confirmation"><p>Confirm authorization bound to revision <code>{armedRevision}</code>.</p><button type="button" disabled={busy || !plan || armedRevision !== plan.plan_revision} onClick={() => void confirm()}>Confirm {scope === "prepare" ? "prepare" : "execute"}: {run.label}</button><button type="button" onClick={() => setArmedRevision(null)}>Disarm</button></div>
        : <button type="button" disabled={busy || !plan || reviewed !== plan.plan_revision} onClick={() => setArmedRevision(plan?.plan_revision ?? null)}>{scope === "prepare" ? "Authorize prepare…" : "Authorize work…"}</button>}
  </Section>;
}

function SupervisorActionDetail({ snapshot, run, task, subagent, busy, error, mutate, onTerminal, onEditTask, onPropose }: SupervisorActionsProps) {
  const [localError, setLocalError] = useState<string | null>(null);
  const [pending, setPending] = useState(false);
  const inFlight = useRef(false);
  const [armed, setArmed] = useState<"cancel" | "retry" | "subagent_cancel" | null>(null);
  const [recovery, setRecovery] = useState<WorkspaceRecoveryAction | null>(null);
  const locked = busy || pending;
  const canControlSubagent = !!run && run.stage !== "closed" && subagent?.status === "running";
  useEffect(() => {
    if (!canControlSubagent && armed === "subagent_cancel") setArmed(null);
  }, [canControlSubagent, armed]);
  useEscapeDisarm(armed !== null, () => setArmed(null));
  const perform = async (action: OrchestrationAction) => {
    if (busy || inFlight.current) return false;
    if (action.action === "subagent_control" && !canControlSubagent) {
      setLocalError("This subagent is no longer running. Its status and control receipts remain available for inspection.");
      return false;
    }
    inFlight.current = true;
    setPending(true);
    setLocalError(null);
    try { return await mutate(action); }
    catch (cause) { setLocalError(cause instanceof Error ? cause.message : "The action could not be completed."); return false; }
    finally { inFlight.current = false; setPending(false); }
  };
  const terminal = async () => {
    if (!run || locked || inFlight.current || observed?.presence !== "present" || !observed.pane_id) return;
    inFlight.current = true;
    setPending(true);
    setLocalError(null);
    try { await onTerminal(run); }
    catch (cause) { setLocalError(cause instanceof Error ? cause.message : "Could not open the terminal."); }
    finally { inFlight.current = false; setPending(false); }
  };
  const observed = snapshot.runtime.status === "fresh" && run ? snapshot.runtime.runs.find((item) => item.run_id === run.run_id) : null;
  const parent = run ? snapshot.runs.find((item) => item.run_id === run.parent_run_id) : null;
  const subParent = subagent ? snapshot.subagents.find((item) => item.run_id === subagent.run_id && item.subagent_id === subagent.parent_subagent_id) : null;
  const selectedTaskId = run?.task_id ?? task?.task.task_id;
  const currentTask = snapshot.board?.tasks.find((item) => item.task.task_id === selectedTaskId) ?? null;
  const intents = snapshot.intents.filter((intent) => intent.state === "conflict" && (run ? intent.run_id === run.run_id : task && intent.task_id === task.task.task_id));
  const messages = run ? snapshot.messages.filter((message) => message.to_run_id === run.run_id) : [];
  const needsInput = run && (run.last_report?.kind === "needs_input" || snapshot.attention.some((item) => item.run_id === run.run_id && item.kind === "needs_input"));
  const confirmArmed = async () => {
    if (!run || !armed) return;
    const action: OrchestrationAction = armed === "subagent_cancel" && subagent
      ? { action: "subagent_control", run_id: run.run_id, subagent_id: subagent.subagent_id, op: { op: "cancel" } }
      : armed === "retry" ? { action: "retry_launch", run_id: run.run_id } : { action: "cancel_run", run_id: run.run_id };
    if (await perform(action)) setArmed(null);
  };
  return <div className="supervisor-detail" aria-busy={locked}>
    <h2>{subagent?.label ?? task?.task.title ?? run?.label ?? "Details"}</h2>
    {error || localError ? <p className="supervisor-error" role="alert">{localError ?? error}</p> : null}
    {!run && !task && !subagent ? <p>Select a task, agent or activity item to inspect its details.</p> : null}
    {task ? <Section title="Task"><dl className="supervisor-facts"><dt>Stage</dt><dd>{task.lane}</dd><dt>Task ID</dt><dd><code>{task.task.task_id}</code></dd><dt>Current revision</dt><dd><code>{currentTask?.task.task_revision}</code></dd><dt>Checked</dt><dd>{currentTask?.task.checked ? "Yes" : "No"}</dd></dl><pre className="supervisor-plan">{task.task.body}</pre>{task.task.diagnostic ? <p role="alert">{task.task.diagnostic}</p> : null}<button type="button" disabled={locked || !!task.task.diagnostic} onClick={() => onEditTask(task)}>Edit task…</button><button type="button" disabled={locked || !!task.task.diagnostic || task.task.checked} onClick={() => onPropose(task)}>Propose worker…</button></Section> : null}
    {run ? <>
      <Section title="Assignment"><dl className="supervisor-facts"><dt>Role</dt><dd>{subagent?.role ?? (subagent ? "OMP subagent" : run.kind)}</dd><dt>Assignment</dt><dd>{task?.task.title ?? run.task_id ?? "Supervisor liaison"}</dd><dt>Parent</dt><dd>{subagent ? subParent?.label ?? subagent.parent_subagent_id ?? `${run.label} · main session` : parent?.label ?? run.parent_run_id ?? "Root"}</dd><dt>Run</dt><dd><code>{run.run_id}</code></dd><dt>Attempt</dt><dd>{run.attempt}</dd><dt>Stage</dt><dd>{run.stage}{run.close_reason ? ` · ${run.close_reason}` : ""}</dd><dt>Proposed task revision</dt><dd><code>{run.task_revision_at_propose ?? "Not task-bound"}</code></dd><dt>Supersedes</dt><dd>{run.supersedes_run_id ?? "None"}</dd></dl></Section>
      <Section title="Observed · Herdr">{snapshot.runtime.status === "unavailable" ? <p role="status">Runtime unavailable: {snapshot.runtime.error.message}. Last receipt is not live truth.</p> : <><p>Observed <time>{snapshot.runtime.observed_at}</time> · endpoint <code>{snapshot.runtime.endpoint_identity}</code></p>{observed ? <dl className="supervisor-facts"><dt>Presence</dt><dd>{observed.presence}</dd><dt>Runtime state</dt><dd>{observed.agent_status ?? "Unknown"}</dd><dt>Since</dt><dd>{observed.state_changed_at ?? "Unknown"}</dd><dt>Location</dt><dd>{observed.workspace_label ?? observed.workspace_id ?? "Unknown Space"} · {observed.tab_label ?? observed.tab_id ?? "Unknown tab"}</dd></dl> : <p>Run is unobserved in the fresh Herdr snapshot.</p>}</>}{subagent ? <p>OMP subagent · in {run.label} · no separate pane. Herdr observes the containing run, not this subagent.</p> : <><button type="button" disabled={locked || observed?.presence !== "present" || !observed.pane_id} onClick={() => void terminal()}>Go to terminal</button>{observed?.presence !== "present" || !observed.pane_id ? <p>Current pane membership is unconfirmed. A launch receipt is not current runtime membership.</p> : null}</>}</Section>
      <Section title="Last worker report"><ReportDetail report={run.last_report} absent="No explicit worker report. Runtime state does not imply task completion or acceptance." /></Section>
      {subagent ? <><Section title="Subagent · worker events"><dl className="supervisor-facts"><dt>Status</dt><dd>{subagent.status}</dd><dt>Reported at</dt><dd>{subagent.updated_at}</dd><dt>ID</dt><dd><code>{subagent.subagent_id}</code></dd></dl><pre className="supervisor-plan">{subagent.summary ?? "No summary reported."}</pre></Section><Section title="Subagent control receipt">{subagent.last_control ? <><p>#{subagent.last_control.seq} · {subagent.last_control.op.op} · {subagent.last_control.stage} · <time>{subagent.last_control.at}</time></p>{subagent.last_control.op.op === "send" ? <pre className="supervisor-plan">{subagent.last_control.op.text}</pre> : null}{subagent.last_control.error ? <p className="supervisor-error" role="alert">{subagent.last_control.error}</p> : null}</> : <p>No control receipt. Stored means queued, not applied by OMP.</p>}<TextAction label="Message this subagent" submitLabel="Send to subagent" busy={locked || !canControlSubagent} submit={(text) => perform({ action: "subagent_control", run_id: run.run_id, subagent_id: subagent.subagent_id, op: { op: "send", text } })} /><button type="button" disabled={locked || !canControlSubagent} onClick={() => setArmed("subagent_cancel")}>Cancel subagent…</button>{!canControlSubagent ? <p>Control unavailable · this subagent is {subagent.status}. Reports and receipts remain available for inspection.</p> : null}</Section></> : <>
        {run.stage === "awaiting_prepare" ? <PlanGrant run={run} scope="prepare" plan={run.prepare_plan} busy={locked} mutate={perform} /> : run.prepare_plan ? <Section title="Prepare plan · exact text"><code>{run.prepare_plan.plan_revision}</code><pre className="supervisor-plan">{run.prepare_plan.text}</pre></Section> : null}
        <Section title="Prepare brief · exact text"><pre className="supervisor-plan">{run.prepare_brief}</pre></Section>
        <Section title="Initialization receipt"><ReportDetail report={run.init_receipt} absent="No initialization receipt. Herdr idle does not mean initialized." /></Section>
        {run.stage === "ready" ? <PlanGrant run={run} scope="execute" plan={run.work_plan} busy={locked} mutate={perform} /> : run.work_plan ? <Section title="Work plan · exact text"><code>{run.work_plan.plan_revision}</code><pre className="supervisor-plan">{run.work_plan.text}</pre></Section> : null}
        <Section title="Setup receipt">{run.setup ? <><dl className="supervisor-facts"><dt>Operation</dt><dd><code>{run.setup.operation_id ?? "No setup operation"}</code></dd><dt>Generation</dt><dd>{run.setup.generation ?? "None"}</dd><dt>Space</dt><dd>{run.setup.workspace_id ?? "Not yet observed"}</dd><dt>Checkout</dt><dd>{run.setup.checkout_path}</dd><dt>Repository</dt><dd>{run.setup.repository_id ?? "None"}</dd><dt>Branch / base</dt><dd>{run.setup.branch ?? "None"} / {run.setup.base ?? "None"}</dd><dt>Ownership</dt><dd>{run.setup.ownership ?? "Unknown"}</dd></dl><ul>{run.setup.effects.map((effect, index) => <li key={index}>{effect}</li>)}</ul>{run.setup.warnings.map((warning, index) => <p key={index}>{warning}</p>)}</> : <p>No setup receipt.</p>}</Section>
        <Section title="Launch receipt · not live truth">{run.location ? <dl className="supervisor-facts">{Object.entries(run.location).map(([key, value]) => <div key={key}><dt>{key.replaceAll("_", " ")}</dt><dd><code>{value ?? "None"}</code></dd></div>)}</dl> : <p>No launch receipt.</p>}<p>OMP session: <code>{run.bound_omp_session ?? "Not bound"}</code></p>{run.dispatch ? <><p>{run.dispatch.step} · launch attempt {run.dispatch.launch_attempt} · agent start {run.dispatch.agent_started ? "recorded" : "not recorded"}</p><p>Launch tag: <code>{run.dispatch.launch_tag ?? "None"}</code> · endpoint: <code>{run.dispatch.endpoint_identity ?? "Unobserved"}</code></p>{run.dispatch.error ? <p role="alert">{run.dispatch.error.code}: {run.dispatch.error.message}</p> : null}</> : null}</Section>
        <Section title="Grants">{run.grants.length ? <ul>{run.grants.map((grant) => <li key={grant.grant_id}>{grant.scope} · {grant.origin} · <time>{grant.granted_at}</time><br /><code>{grant.plan_revision}</code></li>)}</ul> : <p>No grants recorded.</p>}</Section>
        {run.result ? <Section title="Result report"><ReportDetail report={run.result} absent="No result report." /></Section> : null}
        {run.stage === "reported" ? <Section title="Review result"><p>Accept checks the canonical Markdown task using its current revision. Herdr Done is not acceptance.</p><button type="button" disabled={locked || !currentTask || !!currentTask.task.diagnostic || currentTask.current_run_id !== run.run_id} onClick={() => { if (currentTask) void perform({ action: "accept", run_id: run.run_id, expected_task_revision: currentTask.task.task_revision }); }}>Accept report</button><TextAction label="Changes requested" submitLabel="Send back" busy={locked} submit={(text) => perform({ action: "send_back", run_id: run.run_id, text })} /></Section> : null}
        {needsInput ? <Section title="Needs input"><TextAction label="Answer the worker" submitLabel="Send answer" busy={locked || run.stage === "closed"} submit={(text, message_id) => perform({ action: "message_send", message_id, to_run_id: run.run_id, kind: "answer", text })} /></Section> : null}
        <Section title="Instruction"><p>Delivered through the worker inbox; never typed into its terminal.</p><TextAction label="Instruction to worker" submitLabel="Send instruction" busy={locked || run.stage === "closed"} submit={(text, message_id) => perform({ action: "message_send", message_id, to_run_id: run.run_id, kind: "instruction", text })} /></Section>
        <Section title="Annotations">{run.annotations.map((annotation, index) => <div key={index}><p>{annotation.by.type} · <time>{annotation.at}</time></p><pre className="supervisor-plan">{annotation.text}</pre></div>)}<TextAction label="Add a durable note" submitLabel="Annotate" busy={locked} submit={(text) => perform({ action: "annotate", run_id: run.run_id, text })} /></Section>
        <Section title="Inbox delivery">{messages.length ? <ul>{messages.map((message) => <li key={message.message_id}>#{message.seq} · {message.kind} · {message.stage}{message.stale ? " · stale" : ""}<details><summary>Exact message · {message.created_at}</summary><pre className="supervisor-plan">{message.text}</pre></details></li>)}</ul> : <p>No messages recorded.</p>}</Section>
        {run.stage !== "closed" ? <Section title="Recovery and cancellation"><label>Setup recovery<select value={recovery ?? ""} disabled={locked} onChange={(event) => setRecovery(event.target.value === run.dispatch?.recovery ? run.dispatch.recovery : null)}><option value="">Observe and reconcile only</option>{run.dispatch?.recovery ? <option value={run.dispatch.recovery}>{run.dispatch.recovery === "accept_existing_worktree" ? "Accept existing worktree receipt" : "Retry environment setup"}</option> : null}</select></label><button type="button" disabled={locked} onClick={() => void perform({ action: "reconcile_run", run_id: run.run_id, recovery: recovery === run.dispatch?.recovery ? recovery : null })}>Reconcile with Herdr</button>{run.dispatch?.step === "launch_unknown" || run.dispatch?.step === "needs_review" ? <button type="button" disabled={locked} onClick={() => setArmed("retry")}>Retry launch…</button> : null}<button type="button" disabled={locked} onClick={() => setArmed("cancel")}>Cancel run…</button><p>Cancellation does not tear down a Space or checkout.</p></Section> : null}
      </>}
    </> : null}
    {intents.map((intent) => <Section key={intent.intent_id} title="Acceptance conflict"><p>The canonical task changed during acceptance. Apply checks the current task; retain keeps the Markdown unchanged.</p><code>{intent.expected_task_revision}</code><button type="button" disabled={locked} onClick={() => void perform({ action: "intent_resolve", intent_id: intent.intent_id, apply: true })}>Apply acceptance</button><button type="button" disabled={locked} onClick={() => void perform({ action: "intent_resolve", intent_id: intent.intent_id, apply: false })}>Retain current task</button></Section>)}
    {armed ? <section className="supervisor-confirmation" role="group" aria-label="Confirm action"><p>{armed === "retry" ? "The previous launch outcome is unknown. Retrying creates a new tab and may leave a duplicate worker running. Reconcile first when possible. This is an explicit new launch, not a resend." : armed === "subagent_cancel" ? "Request cancellation of this OMP subagent. Wait for its control receipt to learn whether OMP applied it." : "Cancel this run and request the worker to stop. Its Space and checkout will remain; no teardown is performed."}</p><button type="button" disabled={locked} onClick={() => void confirmArmed()}>{armed === "retry" ? "Confirm new launch" : armed === "subagent_cancel" ? "Confirm subagent cancellation" : "Confirm cancel run"}</button><button type="button" onClick={() => setArmed(null)}>Disarm</button></section> : null}
  </div>;
}

export function SupervisorActions(props: SupervisorActionsProps) {
  return <SupervisorActionDetail key={`${props.run?.run_id ?? "none"}:${props.task?.task.task_id ?? "none"}:${props.subagent?.subagent_id ?? "none"}`} {...props} />;
}
