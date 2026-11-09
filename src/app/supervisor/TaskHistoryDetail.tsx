import type { ReactNode } from "react";
import type { OwnedResourceKind, TaskView } from "../../protocol/generated/v1";
import { formatEventTime, formatGap, type HistoryEvent, type HistoryFact, type TaskHistory } from "./taskHistory";
import type { TaskActivity } from "./taskActivity";

const resourceLabels: Record<OwnedResourceKind, string> = { worker_terminal: "Worker terminal", worktree: "Worktree", branch: "Branch", space: "Space", scratch: "Scratch files", fixture_set: "Test fixtures", attempt_directory: "Attempt folder" };

function EventTime({ at }: { at: string | null }) {
  const time = formatEventTime(at, new Date());
  return time ? <time dateTime={at!} title={time.title}>{time.label}</time> : null;
}
function Facts({ facts }: { facts: HistoryFact[] }) {
  return facts.length ? <dl className="liaison-event-facts">{facts.map((fact, index) => <div key={index}><dt>{fact.label}</dt><dd>{fact.value}</dd></div>)}</dl> : null;
}
function HistoryEventRow({ event }: { event: HistoryEvent }) {
  let label: string, tone = "", sub: ReactNode = null, detail: ReactNode = null;
  switch (event.kind) {
    case "started":
      label = event.replacement ? "Started as replacement" : "Started";
      detail = event.facts.length ? <Facts facts={event.facts} /> : null;
      break;
    case "progress":
      label = `Progress · ${event.items.length + event.earlier} report${event.items.length + event.earlier === 1 ? "" : "s"}`;
      detail = <><ul className="liaison-progress">{event.items.map((item, i) => <li key={i}><span title={item.text}>{item.text}</span><EventTime at={item.at} /></li>)}</ul>{event.earlier > 0 ? <p className="liaison-event-sub">+{event.earlier} earlier reports</p> : null}</>;
      break;
    case "question": {
      label = "Waited for an answer";
      tone = ` is-wait${event.open ? " is-live" : ""}`;
      const gap = event.answer ? formatGap(event.at, event.answer.at) : null;
      const by = event.answer?.by === "you" ? "you" : event.answer?.by === "liaison" ? "the liaison" : event.answer?.by === "cockpit" ? "Cockpit" : "another agent";
      sub = event.answer ? `answered by ${by}${gap ? ` ${gap}${gap === "within a minute" ? "" : " later"}` : ""}` : event.open ? "waiting for an answer" : "not answered";
      break;
    }
    case "interim":
    case "waiting":
    case "resumed":
      label = event.kind === "interim" ? "Interim deliverable" : event.kind === "waiting" ? "Worker said it was waiting" : "Worker resumed";
      tone = event.kind === "resumed" ? "" : " is-wait";
      detail = <p className="liaison-result-summary">{event.summary}</p>;
      break;
    case "result":
      label = `Result reported${event.outcome ? ` · ${event.outcome}` : ""}`;
      tone = event.outcome === "succeeded" ? " is-ok" : event.outcome === "failed" ? " is-bad" : "";
      detail = <><p className="liaison-result-summary">{event.summary}</p><Facts facts={event.facts} /></>;
      break;
    case "unreflected_input": {
      const kind = event.inputKind === "instruction" ? "Instruction" : event.inputKind === "user_directive" ? "User directive" : event.inputKind === "answer" ? "Answer" : event.inputKind === "cancel_request" ? "Cancel request" : "Message";
      const stage = event.stage === "stored" ? event.closed ? "no pickup recorded" : "waiting to be picked up" : event.stage === "read" ? "picked up" : "captured, processing not confirmed";
      label = `${kind} sent ${event.afterResult ? "after" : "before"} the Result`;
      tone = " is-wait";
      sub = `${stage} · from ${event.by}${event.closed ? " · attempt closed; it can't report on this" : ""}`;
      detail = <><p className="liaison-result-summary is-full">{event.text ?? "Text isn't available in this view."}</p>{event.imageCount ? <p className="liaison-event-sub">{event.imageCount} image{event.imageCount === 1 ? "" : "s"} attached; not shown here.</p> : null}<p className="liaison-event-sub">{event.closed ? "Nothing is replayed. If it's still wanted, ask the liaison for a follow-up task." : "The liaison can wait for a new Result or send the task back."}</p></>;
      break;
    }
    case "integrated":
      label = `Integrated into ${event.target}`; tone = " is-ok"; detail = <Facts facts={event.facts} />;
      break;
    case "recovery":
      label = event.phase === "blocked" ? "Recovery blocked" : event.phase === "connected" || event.phase === "reattached" ? "Recovered" : "Recovery in progress";
      tone = event.phase === "blocked" ? " is-bad" : event.phase === "connected" || event.phase === "reattached" ? " is-ok" : " is-recover is-live";
      sub = `${event.cause ? `${event.cause} · ` : ""}last update`;
      break;
  }
  const row = <><span className="liaison-event-title">{label}{detail ? <span className="liaison-event-chevron" aria-hidden="true"> ▸</span> : null}</span><EventTime at={event.at} /></>;
  return <li className={`liaison-event${tone}`}>{detail ? <details><summary>{row}</summary><div className="liaison-event-body">{detail}</div></details> : <div className="liaison-event-line">{row}</div>}{sub ? <div className="liaison-event-sub">{sub}</div> : null}</li>;
}

// [headline, clears when, next step]; Next names who acts, matching next_automatic.
export const waitCopy = {
  recovery_blocked: ["Worker recovery is blocked.", "The recovery blocker is resolved.", "The liaison resolves the blocker"],
  question: ["A worker question needs an answer.", "The question is answered.", "Answer through the liaison"],
  worker_waiting: ["Worker says it is waiting.", "The worker explicitly resumes or the attempt ends.", "The liaison follows up with the worker"],
  review: ["The result is waiting for review.", "The liaison accepts or returns the result.", "The liaison reviews it"],
  prerequisites: ["Prerequisite tasks aren't done yet.", "Those tasks are done.", "Starts automatically"],
  dependency_input: ["A prerequisite's output isn't in the target branch yet.", "That output is integrated into the target branch.", "The liaison integrates the prerequisite"],
  capacity: ["All worker slots for this project are in use.", "A worker slot frees up.", "Starts automatically"],
  resource: ["Waiting for a shared resource.", "The current holder releases it.", "Continues automatically"],
  schedule_hold: ["Scheduling is on hold.", "The hold is resolved.", "The liaison resolves the hold"],
  dispatch: ["Scheduled and waiting to start.", "A worker starts.", "Starts automatically"],
} as const satisfies Record<NonNullable<TaskView["waiting"]>["cause"], readonly [string, string, string]>;

function TaskProgress({ view, history }: { view: TaskView; history: TaskHistory | null }) {
  const integrated = history?.sub?.kind === "integrated";
  const steps = ["Ready", "Assigned", "Review", "Accepted", ...(integrated ? ["Integrated"] : [])];
  const current = integrated ? 4 : history?.outcome.accepted ? 3 : view.lane === "accepted" || view.task.checked ? -1 : view.lane === "review" ? 2 : view.lane === "working" ? 1 : 0;
  return <ol className="liaison-task-progress" aria-label="Task progress">{steps.map((step, index) =>
    <li key={step} className={index < current ? "is-complete" : index === current ? "is-current" : undefined} aria-current={index === current ? "step" : undefined}>
      <span className="liaison-progress-mark" aria-hidden="true" /><span>{step}</span>
    </li>)}</ol>;
}

function WaitingDetail({ waiting }: { waiting: NonNullable<TaskView["waiting"]> }) {
  const [headline, clears, next] = waitCopy[waiting.cause];
  // Raw codes only explain generic holds; named causes already say what is wrong.
  const code = waiting.cause === "schedule_hold" || waiting.cause === "recovery_blocked" ? waiting.code : null;
  return <section className="liaison-wait-detail" aria-label={waiting.cause === "worker_waiting" ? "Worker-stated wait" : "Why is this waiting"}>
    <p className="liaison-wait-heading">{waiting.cause === "worker_waiting" ? "Worker says it is waiting" : "Why is this waiting?"}</p>
    <p className="liaison-wait-headline">{headline}</p>
    {waiting.detail ? <p className="liaison-wait-description">{waiting.detail}</p> : null}
    {waiting.cause === "worker_waiting" ? <p className="liaison-wait-description">Worker claim · Cockpit has not verified it.</p> : null}
    <dl className="liaison-wait-facts">
      {waiting.held_by.length ? <><dt>Held by</dt><dd>{waiting.held_by.map((ref, index) => <span key={`${ref.id}:${index}`}>{index ? ", " : ""}{ref.title}</span>)}</dd></> : null}
      <dt>Clears when</dt><dd>{clears}</dd>
      {waiting.since && formatEventTime(waiting.since, new Date()) ? <><dt>{waiting.cause === "worker_waiting" ? "Stated" : "Since"}</dt><dd><EventTime at={waiting.since} /></dd></> : null}
      {code ? <><dt>Code</dt><dd><code>{code}</code></dd></> : null}
    </dl>
    <p className="liaison-wait-next"><strong>Next:</strong> {next}</p>
  </section>;
}

export function TaskHistoryDetail({ view, history, activity = null, historyTransfer }: { view: TaskView; history: TaskHistory | null; activity?: TaskActivity | null; historyTransfer?: { data: unknown | null; pending: boolean; error: boolean; retrying: boolean; disconnected: boolean; retry(): void } }) {
  const outcome = history?.outcome;
  const rawStage = outcome?.stage ?? (view.task.checked ? "done" : view.lane);
  const stage = view.lane === "working" ? "assigned" : rawStage === "working" ? "assigned" : rawStage;
  const sub = history?.sub;
  const subText = sub?.kind === "integrated" ? `Integrated into ${sub.target}` : sub?.kind === "non_code" ? "Non-code result · no Git integration needed" : sub?.kind === "integration_not_recorded" ? "Integration not recorded" : sub?.kind === "after" ? sub.text : sub?.kind === "reconnecting" ? `Reconnecting worker · ${sub.phase}` : sub?.kind === "not_started" ? "No worker has started yet" : null;
  return <section className="liaison-detail" aria-label={`History of ${view.task.title}`}>
    <div className={`liaison-detail-summary lane-${view.lane}${view.attempt ? ` attempt-${view.attempt.state}` : ""}${activity ? ` activity-${activity.kind}` : ""}`}>
      <span className="liaison-task-dot" aria-hidden="true" /><h2>{view.task.title}</h2>
      <p className="liaison-detail-outcome"><span className={outcome?.stalled ? "stalled" : undefined}>{stage[0].toUpperCase() + stage.slice(1)}</span>{activity ? ` · ${activity.word}` : view.lane === "working" ? " · unconfirmed" : ""}{outcome?.scheduled ? " · scheduled" : ""}{outcome?.accepted ? " · accepted" : ""}{outcome?.attempt ? ` · attempt ${outcome.attempt}` : ""}{outcome?.kept ? <> · <span className="kept">{outcome.kept} resource{outcome.kept === 1 ? "" : "s"} kept</span></> : null}</p>
      {!view.waiting && !view.worker_waiting && subText ? <p className="liaison-detail-sub">{subText}</p> : null}
    </div>
    <TaskProgress view={view} history={history} />
    {view.waiting ? <WaitingDetail waiting={view.waiting} /> : null}
    {view.worker_waiting && view.waiting?.cause !== "worker_waiting" ? <WaitingDetail waiting={view.worker_waiting} /> : null}
    {activity && (activity.kind === "hold" || activity.kind === "idle" || activity.kind === "not-observed" || activity.kind === "runtime-blocked") ? <p className="liaison-history-notice">{activity.detail}</p> : null}
    {history?.liveUnavailable ? <p className="liaison-history-notice" role="status">Live worker state unavailable — showing recorded history.</p> : null}
    {history?.unreflectedNotice ? <p className="liaison-history-notice">{history.unreflectedNotice}. See History below.</p> : null}
    {history?.identity !== "ambiguous" && historyTransfer ? <>
      <p className="liaison-history-notice" role="status" aria-live="polite" aria-atomic="true">{historyTransfer.disconnected ? historyTransfer.data ? "Disconnected — showing previously loaded history." : "History unavailable while disconnected." : historyTransfer.error ? historyTransfer.data ? "Couldn’t update history — showing previously loaded history." : "Couldn’t load history." : historyTransfer.pending ? historyTransfer.data ? "Updating history…" : "Loading history…" : `History loaded for ${view.task.title}.`}
        {historyTransfer.error || historyTransfer.disconnected ? <> <button type="button" aria-label={`Retry history for ${view.task.title}`} aria-disabled={historyTransfer.retrying || historyTransfer.disconnected} onClick={historyTransfer.retrying || historyTransfer.disconnected ? undefined : historyTransfer.retry}>Retry history</button>{historyTransfer.retrying ? " Retrying history…" : null}</> : null}
      </p>
    </> : null}
    {!history ? historyTransfer ? null : <p className="liaison-empty">History unavailable — no task snapshot is available.</p> : history.identity === "ambiguous" ? <p className="liaison-empty">History unavailable — this task's ID appears more than once in the task file.</p> : <>
      {!history.attempts.length && !history.scheduledAt ? <p className="liaison-empty"><strong>{view.task.checked ? "No history to show" : "No attempts yet"}</strong>{view.task.checked ? "Only the task's checked state was found; no attempts or results are recorded for it." : "History appears here once a worker starts."}</p> : <>
        <h3>History</h3>
        {history.attempts.map((attempt, i) => {
          const events = <ol className="liaison-history" aria-label={`Attempt ${attempt.attempt} events`}>{attempt.events.map((event, j) => <HistoryEventRow key={event.kind === "unreflected_input" ? `input:${event.messageId}` : `event:${j}`} event={event} />)}</ol>;
          return i < history.attempts.length - 1 ? <details className="liaison-attempt" key={attempt.runId}><summary><span>Attempt {attempt.attempt} · {attempt.end}</span><span className="liaison-attempt-worker">{attempt.label}</span>{attempt.reason ? <span className="liaison-attempt-why">{attempt.reason}</span> : null}</summary>{events}</details> : <div key={attempt.runId}><div className="liaison-attempt-label"><span>Attempt {attempt.attempt}</span><span>{attempt.label}</span></div>{attempt.reason ? <p className="liaison-attempt-reason">{attempt.reason}</p> : null}{events}</div>;
        })}
        {history.scheduledAt && !history.attempts.some(attempt => attempt.end === "current") ? <ol className="liaison-history"><li className="liaison-event"><div className="liaison-event-line"><span className="liaison-event-title">Scheduled</span><EventTime at={history.scheduledAt} /></div></li></ol> : null}
      </>}
      {history.leftBehind.length ? <><h3>Left behind</h3><ul className="liaison-left">{history.leftBehind.map(row => {
        // Removal/closure is durable evidence, not invalidated by losing live observation.
        const unobserved = history.liveUnavailable && row.state !== "removed" && row.state !== "closed";
        const note = unobserved ? `Last recorded: ${row.state}` : row.reason ?? (row.state === "kept" ? "Work from this attempt is kept; the liaison decides." : null);
        return <li key={row.key} className={unobserved || row.state === "unknown" ? "is-unknown" : row.state === "kept" ? "is-kept" : undefined}><div><span>{resourceLabels[row.kind]}{row.attempt !== null ? ` · attempt ${row.attempt}` : ""}</span><span className="liaison-left-state">{unobserved ? "not observed" : row.kind === "worker_terminal" && row.state === "removed" ? "closed" : row.state}{!unobserved && row.at ? <> · <EventTime at={row.at} /></> : null}</span></div>{note ? <p>{note}</p> : null}</li>;
      })}</ul></> : null}
      {history.attempts.length ? <p className="liaison-history-footnote">Times are recorded events. Acceptance time isn't recorded. Resource rows show the latest recorded state.</p> : null}
    </>}
  </section>;
}
