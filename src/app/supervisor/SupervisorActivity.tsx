import { useState } from "react";
import type { ActorRef, Message, OrchestrationSnapshot, RootSummary, Run, TaskView } from "../../protocol/generated/v1";
import { copyText } from "../library/clipboard";
import { RunDiagnostics } from "./SupervisorActions";

export type ActivityLink = { kind: "task"; taskId: string } | { kind: "run"; runId: string };
export type ActivityRow = { id: string; day: string; actor: string; what: string; at: string; stale: boolean; link: ActivityLink | null; detail: string };
export type SupervisorActivityProps = { rows: readonly ActivityRow[]; onLink(link: ActivityLink): void };
export type SupervisorDiagnosticsProps = { snapshot: OrchestrationSnapshot; rootRuns: readonly Run[]; busy: boolean; connected: boolean; onIdentify(): void; onCopyPath(): void };
export type ClosedTaskCount = { status: "loading" } | { status: "loaded"; count: number } | { status: "unavailable" };
export type ClosedTrackingProps = {
  closedRoots: readonly RootSummary[];
  runs: readonly Run[];
  loadedRootId: string | null;
  taskCount: number;
  taskCounts: ReadonlyMap<string, ClosedTaskCount>;
  busy: boolean;
  dialogOpen: boolean;
  onView(rootId: string): void;
  returnTo: { label: string; onActivate(): void } | null;
};

function actorName(actor: ActorRef, snapshot: OrchestrationSnapshot): string {
  if (actor.type === "operator") return "You";
  if (actor.type === "dispatcher") return "Dispatcher";
  return snapshot.runs.find(run => run.run_id === actor.run_id)?.label ?? "Agent";
}
function localDay(at: string): string {
  const date = new Date(at);
  if (!Number.isFinite(date.getTime())) return "unknown";
  return `${date.getFullYear()}-${String(date.getMonth() + 1).padStart(2, "0")}-${String(date.getDate()).padStart(2, "0")}`;
}
function runLink(run: Run): ActivityLink {
  return run.kind === "worker" && run.task_id ? { kind: "task", taskId: run.task_id } : { kind: "run", runId: run.run_id };
}

/** Activity is the readable record; exact plans, sessions and receipts stay in Diagnostics. */
export function activityRows(snapshot: OrchestrationSnapshot, rootRuns: readonly Run[], tasks: readonly TaskView[]): ActivityRow[] {
  const scopedRuns = new Map(rootRuns.map(run => [run.run_id, run]));
  const assignedTasks = new Map(tasks.map(task => [`assign-${task.task.task_id}`, task]));
  const row = (event: Omit<ActivityRow, "day">): ActivityRow => ({ ...event, day: localDay(event.at) });
  return [
    ...snapshot.messages.filter(message => scopedRuns.has(message.to_run_id)).map(message => {
      const recipient = scopedRuns.get(message.to_run_id)!;
      const task = assignedTasks.get(message.message_id);
      const systemBrief = ["supervisor_brief", "prepare_brief", "work_brief"].includes(message.kind);
      const pointer = message.kind === "observation" || !!task;
      const summary = message.report?.summary ?? (task ? `Task assigned: ${task.task.title}` : systemBrief ? `Guidance delivered to ${recipient.label}` : pointer ? `Lifecycle update for ${recipient.label}` : message.text);
      return row({
        id: `message:${message.message_id}`, actor: actorName(message.from, snapshot),
        what: `${message.kind.replaceAll("_", " ")} · ${summary}`, at: message.created_at, stale: message.stale,
        link: task ? { kind: "task", taskId: task.task.task_id } : runLink(recipient),
        detail: systemBrief || pointer ? `${summary}. Exact delivery records are available in Diagnostics.` : message.report?.summary ?? message.text,
      });
    }),
    ...rootRuns.flatMap(run => run.grants.map(grant => {
      const actor = grant.origin === "supervisor" ? snapshot.runs.find(root => root.run_id === grant.supervisor_run_id)?.label ?? "Supervisor" : `You (${grant.origin})`;
      return row({
        id: grant.grant_id, actor, what: `${grant.scope === "prepare" ? "Preparation" : "Execution"} authorized · ${actor} · ${run.label}`,
        at: grant.granted_at, stale: false, link: runLink(run),
        detail: `An exact ${grant.scope === "prepare" ? "preparation" : "work"} plan was authorized for ${run.label}. Actor, actual OMP session and exact revision are retained in Diagnostics.`,
      });
    })),
    ...rootRuns.flatMap(run => run.annotations.map((note, index) => {
      const receiptNote = ["Acceptance requested for Result ", "Accepted Result at exact task revision ", "Recovered acceptance of Result "].some(prefix => note.text.startsWith(prefix));
      const summary = receiptNote ? `Result review note for ${run.label}` : note.text;
      return row({
        id: `${run.run_id}:note:${index}`, actor: actorName(note.by, snapshot), what: `Note · ${summary}`,
        at: note.at, stale: false, link: runLink(run),
        detail: receiptNote ? `${summary}. Exact result, revision and native session references are retained in Diagnostics.` : note.text,
      });
    })),
  ].sort((a, b) => b.at.localeCompare(a.at));
}

function age(at: string): string {
  const timestamp = Date.parse(at);
  if (!Number.isFinite(timestamp)) return "Time unavailable";
  const minutes = Math.max(0, Math.floor((Date.now() - timestamp) / 60_000));
  return minutes < 1 ? "just now" : minutes < 60 ? `${minutes}m ago` : minutes < 1440 ? `${Math.floor(minutes / 60)}h ago` : `${Math.floor(minutes / 1440)}d ago`;
}
export function SupervisorActivity({ rows, onLink }: SupervisorActivityProps) {
  const days = new Map<string, ActivityRow[]>();
  for (const row of rows) {
    const group = days.get(row.day);
    if (group) group.push(row); else days.set(row.day, [row]);
  }
  return <section className="supervisor-activity" aria-label="Activity">
    <h3>Activity</h3>
    {rows.length ? [...days].map(([day, events]) => <section className="supervisor-activity-day" key={day}>
      <h4>{day === "unknown" ? "Date unavailable" : new Date(`${day}T12:00:00`).toLocaleDateString(undefined, { weekday: "long", year: "numeric", month: "short", day: "numeric" })}</h4>
      <ul className="supervisor-history">{events.map(event => <li key={event.id}>
        <div className="supervisor-activity-row"><span className="supervisor-actor-chip">{event.actor}</span><span className="supervisor-activity-what">{event.what}</span>
          {event.link ? <button type="button" className="supervisor-activity-link" aria-label={`Show ${event.link.kind} · ${event.what}`} onClick={() => onLink(event.link!)}>Show {event.link.kind}</button> : null}
          <time dateTime={event.at} title={new Date(event.at).toLocaleString()}>{age(event.at)}</time>
          {event.stale ? <span className="supervisor-stale-evidence">· stale evidence</span> : null}
        </div>
        <p className="supervisor-exact-text">{event.detail}</p>
      </li>)}</ul>
    </section>) : <p>No recorded activity in this scope.</p>}
  </section>;
}

export function SupervisorDiagnostics({ snapshot, rootRuns, busy, connected, onIdentify, onCopyPath }: SupervisorDiagnosticsProps) {
  const [copyState, setCopyState] = useState<{ path: string; copied: boolean } | null>(null);
  const board = snapshot.board;
  const latestMessages = new Map<string, Message>();
  for (const message of snapshot.messages) {
    const previous = latestMessages.get(message.to_run_id);
    if (!previous || message.seq > previous.seq) latestMessages.set(message.to_run_id, message);
  }
  const copyPath = async () => {
    if (!board) return;
    const copied = await copyText(board.path);
    setCopyState({ path: board.path, copied });
    if (copied) onCopyPath();
  };
  const observation = (runId: string) => {
    if (snapshot.runtime.status !== "fresh") return "Observation unavailable";
    const observed = snapshot.runtime.runs.find(item => item.run_id === runId);
    if (!observed) return "Unobserved";
    return `${connected ? "" : "Saved · "}${observed.presence.replaceAll("_", " ")} · actual OMP ${observed.actual_omp ? "yes" : "no"}${observed.agent_status ? ` · ${observed.agent_status}` : ""}`;
  };
  const identifyReason = busy ? "Wait for the current change to finish." : !connected ? "Reconnect before identifying task-file items." : null;
  return <section className="supervisor-diagnostics-panel" aria-label="Diagnostics">
    <h3>Diagnostics</h3><p className="supervisor-diagnostics-banner">Launch receipts and saved reports are not current process proof.</p>
    <table className="supervisor-diagnostics-summary"><caption>Task source summary</caption><tbody>
      <tr><th scope="row">Canonical task source</th><td><span className="supervisor-task-path">{board?.path ?? "No selected task document"}</span> <button type="button" disabled={!board} onClick={() => void copyPath()}>Copy task path</button>
        <span className="supervisor-copy-status" role="status">{board && copyState?.path === board.path ? copyState.copied ? "Task path copied." : "Could not copy task path." : ""}</span>
      </td></tr>
      <tr><th scope="row">Board diagnostics</th><td>{board?.diagnostics.length ? <ul>{board.diagnostics.map((diagnostic, index) => <li className="supervisor-error" key={index}>{diagnostic.message}</li>)}</ul> : "No board diagnostics"}</td></tr>
      <tr><th scope="row">Unidentified items</th><td>{board?.unidentified_items ?? 0}{board?.unidentified_items ? <> task-file checklist items need identity markers. <button type="button" disabled={busy || !connected} aria-describedby={identifyReason ? "supervisor-identify-reason" : undefined} onClick={() => { if (!busy && connected) onIdentify(); }}>Identify task-file items</button>{identifyReason ? <p className="supervisor-disabled-reason" id="supervisor-identify-reason">{identifyReason}</p> : null}</> : null}</td></tr>
    </tbody></table>
    <div className="supervisor-diagnostics-table-scroll"><table className="supervisor-diagnostics-summary"><caption>Run summary</caption><thead><tr><th scope="col">Run</th><th scope="col">Dispatch step</th><th scope="col">Observation</th><th scope="col">Binding</th><th scope="col">Last delivery stage</th></tr></thead>
      <tbody>{rootRuns.map(run => <tr key={run.run_id}><th scope="row">{run.label}</th><td>{run.dispatch?.step.replaceAll("_", " ") ?? "No dispatch record"}</td><td>{observation(run.run_id)}</td><td>Bound session {run.bound_omp_session ? "yes" : "no"}</td><td>{latestMessages.get(run.run_id)?.stage ?? "No delivery record"}{latestMessages.get(run.run_id)?.stale ? " · stale evidence" : ""}</td></tr>)}</tbody>
    </table></div>
    <p className="supervisor-delivery-stages">Stored ≠ woken ≠ read ≠ acked.</p>
    <div className="supervisor-diagnostics-records">{rootRuns.map(run => <details key={run.run_id}><summary>{run.label} · exact records</summary><RunDiagnostics run={run} snapshot={snapshot} /></details>)}
      <details><summary>Runtime and transaction records</summary><pre className="supervisor-plan">{JSON.stringify({ runtime: snapshot.runtime, intents: snapshot.intents, assignment_intents: snapshot.assignment_intents }, null, 2)}</pre></details>
    </div>
  </section>;
}

export function ClosedTracking({ closedRoots, runs, loadedRootId, taskCount, taskCounts, busy, dialogOpen, onView, returnTo }: ClosedTrackingProps) {
  return <section className="supervisor-archive" aria-label="Closed tracking">
    <p>Tasks and history remain available. Closing tracking did not kill agents or remove resources.</p>
    <ul className="supervisor-archive-list">{closedRoots.map(summary => {
      const root = runs.find(run => run.run_id === summary.root_id);
      const openDescendants = runs.filter(run => run.root_id === summary.root_id && run.run_id !== summary.root_id && run.stage !== "closed").length;
      const count = taskCounts.get(summary.root_id) ?? (loadedRootId === summary.root_id ? { status: "loaded" as const, count: taskCount } : { status: "loading" as const });
      return <li key={summary.root_id}>
        <strong>{summary.label}</strong><div className="supervisor-archive-facts">{root ? <span>Updated <time dateTime={root.updated_at}>{new Date(root.updated_at).toLocaleString()}</time></span> : <span>Updated time unavailable</span>}
          <span className="supervisor-archive-task-count" role="status">{count.status === "loaded" ? `${count.count} tasks` : count.status === "loading" ? "Loading task count…" : "Task count unavailable"}</span><span>{openDescendants} descendants still open</span>
        </div><button type="button" disabled={busy || dialogOpen} onClick={() => { if (!busy && !dialogOpen) onView(summary.root_id); }}>View {summary.label} tasks and history</button>
      </li>;
    })}</ul>
    {returnTo ? <button type="button" disabled={busy || dialogOpen} onClick={() => { if (!busy && !dialogOpen) returnTo.onActivate(); }}>Return to {returnTo.label}</button> : null}
  </section>;
}
