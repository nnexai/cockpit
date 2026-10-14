import type { ReactNode } from "react";
import type { OwnedResourceKind, TaskView } from "../../protocol/generated/v1";
import { formatEventTime, formatGap, type HistoryEvent, type HistoryFact, type TaskHistory } from "./taskHistory";

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
    case "result":
      label = `Result reported${event.outcome ? ` · ${event.outcome}` : ""}`;
      tone = event.outcome === "succeeded" ? " is-ok" : event.outcome === "failed" ? " is-bad" : "";
      detail = <><p className="liaison-result-summary">{event.summary}</p><Facts facts={event.facts} /></>;
      break;
    case "merged":
      label = `Merged into ${event.target}`; tone = " is-ok"; detail = <Facts facts={event.facts} />;
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

export function TaskHistoryDetail({ view, history }: { view: TaskView; history: TaskHistory | null }) {
  const outcome = history?.outcome;
  const stage = outcome?.stage ?? (view.task.checked ? "done" : view.lane);
  const sub = history?.sub;
  const subText = sub?.kind === "merged" ? `Merged into ${sub.target}` : sub?.kind === "no_merge" ? "No merge needed" : sub?.kind === "merge_not_recorded" ? "Merge not recorded" : sub?.kind === "after" ? sub.text : sub?.kind === "reconnecting" ? `Reconnecting worker · ${sub.phase}` : sub?.kind === "not_started" ? "No worker has started yet" : null;
  return <section className="liaison-detail" aria-label={`History of ${view.task.title}`}>
    <div className={`liaison-detail-summary lane-${view.lane}${view.attempt ? ` attempt-${view.attempt.state}` : ""}`}>
      <span className="liaison-task-dot" aria-hidden="true" /><h2>{view.task.title}</h2>
      <p className="liaison-detail-outcome"><span className={outcome?.stalled ? "stalled" : undefined}>{stage[0].toUpperCase() + stage.slice(1)}</span>{outcome?.scheduled ? " · scheduled" : ""}{outcome?.accepted ? " · accepted" : ""}{outcome?.attempt ? ` · attempt ${outcome.attempt}` : ""}{outcome?.kept ? <> · <span className="kept">{outcome.kept} resource{outcome.kept === 1 ? "" : "s"} kept</span></> : null}</p>
      {subText ? <p className="liaison-detail-sub">{subText}</p> : null}
    </div>
    {history?.liveUnavailable ? <p className="liaison-history-notice" role="status">Live worker state unavailable — showing recorded history.</p> : null}
    {!history ? <p className="liaison-empty">History unavailable — no task snapshot is available.</p> : history.identity === "ambiguous" ? <p className="liaison-empty">History unavailable — this task's ID appears more than once in the task file.</p> : <>
      {!history.attempts.length && !history.scheduledAt ? <p className="liaison-empty"><strong>{view.task.checked ? "No history to show" : "No attempts yet"}</strong>{view.task.checked ? "Only the task's checked state was found; no attempts or results are recorded for it." : "History appears here once a worker starts."}</p> : <>
        <h3>History</h3>
        {history.attempts.map((attempt, i) => {
          const events = <ol className="liaison-history" aria-label={`Attempt ${attempt.attempt} events`}>{attempt.events.map((event, j) => <HistoryEventRow key={j} event={event} />)}</ol>;
          return i < history.attempts.length - 1 ? <details className="liaison-attempt" key={attempt.runId}><summary><span>Attempt {attempt.attempt} · {attempt.end}</span><span className="liaison-attempt-worker">{attempt.label}</span>{attempt.reason ? <span className="liaison-attempt-why">{attempt.reason}</span> : null}</summary>{events}</details> : <div key={attempt.runId}><div className="liaison-attempt-label"><span>Attempt {attempt.attempt}</span><span>{attempt.label}</span></div>{attempt.reason ? <p className="liaison-attempt-reason">{attempt.reason}</p> : null}{events}</div>;
        })}
        {history.scheduledAt && !history.attempts.some(attempt => attempt.end === "current") ? <ol className="liaison-history"><li className="liaison-event"><div className="liaison-event-line"><span className="liaison-event-title">Scheduled</span><EventTime at={history.scheduledAt} /></div><p className="liaison-event-sub">{view.dependencies.state === "blocked" ? "Waiting for prerequisites" : "Waiting for dispatch"}</p></li></ol> : null}
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
