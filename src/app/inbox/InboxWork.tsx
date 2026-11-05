import { useLayoutEffect, useRef, useState, type ReactNode } from "react";
import type { CockpitClient } from "../../client/CockpitClient";
import type { InboxLocation, InboxTaskLink, InboxThread, InboxView } from "../../protocol/generated/v1";
import { UiIcon, type UiIconName } from "../UiIcon";
import { InboxArtifact } from "./InboxArtifacts";
import { InboxFeedback } from "./InboxFeedback";
import { InboxImages } from "./InboxImages";

type Props = {
  id?: string;
  thread: InboxThread;
  view: InboxView | null;
  client: CockpitClient;
  sessionId: string;
  open(location: InboxLocation | null): void;
  liveOpen: boolean;
  diagnostics: ReactNode;
};

function Chevron() {
  return <span className="inbox-chev"><UiIcon name="right" /></span>;
}

function Section({ icon, label, count, children }: { icon: UiIconName; label: string; count: number; children: ReactNode }) {
  return <section className="inbox-sec"><h4><UiIcon name={icon} />{label}<em>{count}</em></h4>{children}</section>;
}

function Summary({ text }: { text: string }) {
  const [expanded, setExpanded] = useState(false);
  const [clamped, setClamped] = useState(false);
  const ref = useRef<HTMLDivElement>(null);
  useLayoutEffect(() => {
    const node = ref.current;
    if (!node) return;
    const measure = () => {
      if (!expanded) setClamped(node.scrollHeight > node.clientHeight + 1);
    };
    measure();
    if (typeof ResizeObserver === "undefined") return;
    const observer = new ResizeObserver(measure);
    observer.observe(node);
    return () => observer.disconnect();
  }, [expanded, text]);
  return <div className="inbox-task-sum-md" data-overflow={clamped || undefined} data-open={expanded || undefined}>
    <div ref={ref} className="inbox-md inbox-clamp"><InboxFeedback text={text} /></div>
    {(clamped || expanded) && <button type="button" className="inbox-more" onClick={() => setExpanded(value => !value)}>{expanded ? "Show less" : "Show more"}</button>}
  </div>;
}

function gap(asked: string, answered: string | null): string | null {
  if (!answered) return null;
  const elapsed = Date.parse(answered) - Date.parse(asked);
  if (!Number.isFinite(elapsed) || elapsed < 0) return null;
  const minutes = Math.floor(elapsed / 60_000);
  if (minutes < 1) return "less than a minute later";
  if (minutes < 60) return `${minutes} min later`;
  const hours = Math.floor(minutes / 60);
  if (hours < 24) return `${hours} hr later`;
  return `${Math.floor(hours / 24)} days later`;
}

function copy(value: string) {
  if (navigator.clipboard) void navigator.clipboard.writeText(value).catch(() => {});
}

function TaskCard({ task, client, sessionId, open, liveOpen, initiallyOpen }: { task: InboxTaskLink; client: CockpitClient; sessionId: string; open(location: InboxLocation | null): void; liveOpen: boolean; initiallyOpen: boolean }) {
  const evidence = task.evidence;
  const lane = task.lane === "accepted" ? "done" : task.lane;
  const laneLabel = task.lane === "accepted" ? "Done" : task.lane === "working" ? "Working" : task.lane === "blocked" ? "Blocked" : task.lane === "review" ? "In review" : "Ready";
  const visuals = evidence?.artifacts.filter(artifact => artifact.kind === "image" || artifact.kind === "html") ?? [];
  const documents = evidence?.artifacts.filter(artifact => artifact.kind === "markdown" || artifact.kind === "other") ?? [];
  const location = task.run?.presence === "present" ? task.run.location : null;
  return <details className="inbox-task" data-lane={lane} open={initiallyOpen || undefined}>
    <summary><span className="inbox-lane" /><span className="inbox-task-title" title={task.title}>{task.title}</span><Chevron /><span className="inbox-task-status"><b>{laneLabel}</b>{evidence?.integration && <> · merged into {evidence.integration.target} @<code>{evidence.integration.commit.slice(0, 7)}</code></>}{evidence && evidence.attempts > 1 && <> · attempt {evidence.attempt}</>}</span></summary>
    <div className="inbox-task-body">
      {location && <button type="button" className="inbox-btn" disabled={!liveOpen} onClick={() => open(location)} title={liveOpen ? "Open session" : "Live sessions unavailable"}><UiIcon name="terminal" />Open</button>}
      {evidence?.result?.summary && <Summary text={evidence.result.summary} />}
      {!!evidence?.checks.length && <Section icon="check" label="Checks" count={evidence.checks.length}><ul className="inbox-checks">{evidence.checks.map((check, index) => <li className="inbox-check" data-r={check.outcome === "passed" ? "pass" : check.outcome === "failed" ? "fail" : "skip"} key={index}>
        {check.outcome === "not_run" ? <span aria-hidden="true">–</span> : <UiIcon name={check.outcome === "passed" ? "check" : "close"} />}
        <div><span className="inbox-sr">{check.outcome === "passed" ? "Passed" : check.outcome === "failed" ? "Failed" : "Not run"}: </span><InboxFeedback text={check.contract} />{check.command_or_scenario && <code>{check.command_or_scenario}</code>}{check.limitation && <div className="inbox-limit"><InboxFeedback text={check.limitation} /></div>}{check.evidence && <div className="inbox-evid"><InboxFeedback text={check.evidence} /></div>}</div>
      </li>)}</ul></Section>}
      {!!evidence?.commits.length && <Section icon="branch" label="Commits" count={evidence.commits.length}><div className="inbox-shas">{evidence.commits.map((commit, index) => <button type="button" className={`inbox-sha${commit === evidence.head_commit ? " is-head" : ""}`} key={`${commit}:${index}`} title={`Copy ${commit}${commit === evidence.head_commit ? " (head)" : ""}`} aria-label={`Copy commit ${commit}${commit === evidence.head_commit ? ", head" : ""}`} onClick={() => copy(commit)}><span>{commit.slice(0, 7)}</span><UiIcon name="copy" /></button>)}</div></Section>}
      {evidence && (evidence.images.length > 0 || visuals.length > 0) && <Section icon="eye" label="Visuals" count={evidence.images.length + visuals.length}><div className="inbox-shots">{evidence.images.length > 0 && <InboxImages client={client} sessionId={sessionId} images={evidence.images} label="Task images" />}{visuals.map((artifact, index) => <InboxArtifact key={`${artifact.path}:${index}`} client={client} sessionId={sessionId} runId={evidence.run_id} artifact={artifact} />)}</div></Section>}
      {evidence && documents.length > 0 && <Section icon="file" label="Documents" count={documents.length}>{documents.map((artifact, index) => artifact.kind === "markdown" ? <InboxArtifact key={`${artifact.path}:${index}`} client={client} sessionId={sessionId} runId={evidence.run_id} artifact={artifact} /> : <button className="inbox-doc" type="button" key={`${artifact.path}:${index}`} title={`Copy ${artifact.path}`} onClick={() => copy(artifact.path)}><UiIcon name="file" /><span>{artifact.name}</span><UiIcon name="copy" /></button>)}</Section>}
      {!!evidence?.decisions.length && <Section icon="comment" label="Decisions" count={evidence.decisions.length}><ul className="inbox-decs">{evidence.decisions.map((decision, index) => { const elapsed = gap(decision.asked_at, decision.answered_at); return <li className="inbox-dec" key={index}><UiIcon name="forward" /><div><div className="inbox-dec-q"><InboxFeedback text={decision.question} /></div>{decision.answer && <div className="inbox-dec-a"><InboxFeedback text={decision.answer} /></div>}{(decision.by || elapsed) && <p className="inbox-dec-m">{[decision.by, elapsed].filter(Boolean).join(" · ")}</p>}</div></li>; })}</ul></Section>}
    </div>
  </details>;
}

export function InboxWork({ id, thread, client, sessionId, open, liveOpen, diagnostics }: Props) {
  const active = thread.tasks.filter(task => task.lane !== "accepted");
  const done = thread.tasks.filter(task => task.lane === "accepted").sort((a, b) => (b.evidence?.result?.at ?? "").localeCompare(a.evidence?.result?.at ?? ""));
  const card = (task: InboxTaskLink, initiallyOpen: boolean) => <TaskCard key={`${task.root_id}:${task.task_id}`} task={task} client={client} sessionId={sessionId} open={open} liveOpen={liveOpen} initiallyOpen={initiallyOpen} />;
  return <aside id={id} className="inbox-work" aria-label="Work">
    <div className="inbox-work-head"><h3>Work</h3><span>{thread.tasks.length} {thread.tasks.length === 1 ? "task" : "tasks"}</span></div>
    {thread.tasks.length === 0 && <p className="inbox-work-empty">No linked tasks yet.</p>}
    <div className="inbox-tasks">{active.map(task => card(task, true))}{done.slice(0, 2).map(task => card(task, true))}
      {done.length > 2 && <details className="inbox-donemore"><summary><Chevron /><span>{done.length - 2} more done</span></summary><div className="inbox-tasks">{done.slice(2).map(task => card(task, false))}</div></details>}
    </div>
    <details className="inbox-diag"><summary><Chevron />Diagnostics</summary>{diagnostics}</details>
  </aside>;
}
