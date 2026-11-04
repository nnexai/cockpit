import { Fragment, useLayoutEffect, useRef, useState } from "react";
import type { CockpitClient } from "../../client/CockpitClient";
import type { InboxEntry, InboxThread, InboxView } from "../../protocol/generated/v1";
import { UiIcon } from "../UiIcon";
import { relativeTime } from "../library/libraryState";
import { InboxFeedback } from "./InboxFeedback";
import { InboxImages } from "./InboxImages";
import "./inbox-detail.css";

export type TimelineItem =
  | { kind: "entry"; entry: InboxEntry; answers: InboxEntry[] }
  | { kind: "updates"; entries: InboxEntry[] };

/** Preserve chronological order while keeping answers with their question. */
export function groupTimeline(entries: InboxEntry[]): TimelineItem[] {
  const answers = new Map<string, InboxEntry[]>();
  for (const entry of entries) {
    if (entry.kind === "answer" && entry.in_reply_to) {
      const list = answers.get(entry.in_reply_to) ?? [];
      list.push(entry);
      answers.set(entry.in_reply_to, list);
    }
  }
  const items: TimelineItem[] = [];
  let updates: InboxEntry[] = [];
  const flush = () => {
    if (updates.length >= 3) items.push({ kind: "updates", entries: updates });
    else for (const entry of updates) items.push({ kind: "entry", entry, answers: [] });
    updates = [];
  };
  for (const entry of entries) {
    if (entry.kind === "update") { updates.push(entry); continue; }
    flush();
    if (entry.kind !== "answer") items.push({ kind: "entry", entry, answers: entry.kind === "question" ? answers.get(entry.message_id) ?? [] : [] });
  }
  flush();
  return items;
}

function Time({ value }: { value: string }) {
  const label = relativeTime(value, Date.now());
  return label ? <time dateTime={value} title={new Date(value).toLocaleString()}>{label}</time> : null;
}

function gap(from: string, to: string): string | null {
  const minutes = Math.max(0, Math.round((Date.parse(to) - Date.parse(from)) / 60000));
  return Number.isFinite(minutes) ? `${minutes} min later` : null;
}

function Update({ entry, client, sessionId }: { entry: InboxEntry; client: CockpitClient; sessionId: string }) {
  return <details className="inbox-upd"><summary><div className="inbox-upd-t"><InboxFeedback text={entry.text} /></div><Time value={entry.at} /><span className="inbox-chev"><UiIcon name="right" /></span></summary><div className="inbox-md"><InboxFeedback text={entry.text} /><InboxImages client={client} sessionId={sessionId} images={entry.images} label="Update images" /></div></details>;
}

function ResultText({ text }: { text: string }) {
  const ref = useRef<HTMLDivElement>(null);
  const [expanded, setExpanded] = useState(false);
  const [overflow, setOverflow] = useState(false);
  useLayoutEffect(() => {
    const node = ref.current;
    if (!node) return;
    const measure = () => {
      const original = node.style.maxHeight;
      node.style.maxHeight = "calc(3 * 1.55 * var(--font-size-sm))";
      setOverflow(node.scrollHeight > node.clientHeight + 1);
      node.style.maxHeight = original;
    };
    measure();
    if (typeof ResizeObserver === "undefined") return;
    const observer = new ResizeObserver(measure);
    observer.observe(node);
    return () => observer.disconnect();
  }, [text]);
  return <div data-open={expanded ? "" : undefined} data-overflow={overflow ? "" : undefined}><div ref={ref} className="inbox-md inbox-clamp"><InboxFeedback text={text} /></div>{overflow && <button type="button" className="inbox-more" aria-expanded={expanded} onClick={() => setExpanded(value => !value)}>{expanded ? "Show less" : "Show more"}</button>}</div>;
}

export function InboxTimeline({ thread, view, client, sessionId, ownerLabel }: { thread: InboxThread; view: InboxView | null; client: CockpitClient; sessionId: string; ownerLabel(runId: string): string }) {
  const items = groupTimeline(thread.timeline);
  let lastDay = "";
  return <ol className="inbox-tl" aria-label="Timeline">
    {thread.timeline_omitted > 0 && <li className="inbox-e inbox-day"><span>{thread.timeline_omitted} earlier entries</span></li>}
    {items.map((item, index) => {
      const entry = item.kind === "updates" ? item.entries[0] : item.entry;
      const date = new Date(entry.at);
      const day = Number.isNaN(date.getTime()) ? "" : date.toDateString();
      const showDay = day !== lastDay && !!day;
      lastDay = day;
      const content = item.kind === "updates" ? <li className="inbox-e is-compact"><span className="inbox-dot is-run" /><details className="inbox-run"><summary><span>{item.entries.length} updates</span><Time value={entry.at} /><span>–</span><Time value={item.entries[item.entries.length - 1].at} /><span className="inbox-chev"><UiIcon name="right" /></span></summary><ol className="inbox-run-list">{item.entries.map(update => <li key={update.message_id}><Update entry={update} client={client} sessionId={sessionId} /></li>)}</ol></details></li> : entry.kind === "update" ? <li className="inbox-e is-compact"><span className={`inbox-dot${thread.state === "underway" && index === items.length - 1 ? " is-live" : ""}`} /><Update entry={entry} client={client} sessionId={sessionId} /></li> : entry.kind === "question" ? (() => {
        const unresolved = thread.questions.some(q => q.question_message_id === entry.message_id && q.receipt.status === "unresolved");
        const source = entry.source;
        const label = source.kind === "owner" || source.kind === "owner_card" ? view?.owners.find(owner => owner.root_id === source.run_id)?.label ?? ownerLabel(source.run_id) : "Owner";
        return <li className="inbox-e inbox-e-q"><span className="inbox-kic" data-tone="decision"><UiIcon name="help" /></span><div className="inbox-asked"><header className="inbox-card-h"><strong>{label} asked</strong><span className="inbox-sp" /><Time value={entry.at} /></header>{unresolved ? <div className="inbox-open-q"><div className="inbox-open-q-text"><InboxFeedback text={entry.text} /></div><span className="inbox-pill">Open</span></div> : <><div className="inbox-md"><InboxFeedback text={entry.text} /></div><InboxImages client={client} sessionId={sessionId} images={entry.images} label="Question images" />{item.answers.map(answer => <div className="inbox-answer" key={answer.message_id}><header className="inbox-card-h"><strong>You</strong><span className="inbox-kind">Answer{gap(entry.at, answer.at) && ` · ${gap(entry.at, answer.at)}`}</span><span className="inbox-sp" /><Time value={answer.at} /></header><div className="inbox-md"><InboxFeedback text={answer.text} /></div><InboxImages client={client} sessionId={sessionId} images={answer.images} label="Answer images" /></div>)}</>}</div></li>;
      })() : entry.kind === "result" ? <li className="inbox-e"><span className="inbox-kic" data-tone={entry.outcome === "failed" ? "blocked" : "done"}><UiIcon name={entry.outcome === "failed" ? "alert" : "check-circle"} /></span><article className="inbox-result" data-tone={entry.outcome === "failed" ? "blocked" : "done"}><header className="inbox-card-h"><strong>Result</strong>{entry.outcome && <span className="inbox-outcome">{entry.outcome === "failed" ? "Failed" : "Succeeded"}</span>}{entry.message_id === thread.unread_result_message_id && <span className="inbox-new-pill">New</span>}<span className="inbox-sp" /><Time value={entry.at} /></header><ResultText text={entry.text} /><InboxImages client={client} sessionId={sessionId} images={entry.images} label="Result images" /></article></li> : <li className="inbox-e"><span className="inbox-kic" data-tone="review"><UiIcon name="user" /></span><article className="inbox-you"><header className="inbox-card-h"><strong>You</strong><span className="inbox-kind">{entry.kind === "request" ? "Request" : "Follow-up"}</span><span className="inbox-sp" /><Time value={entry.at} /></header><div className="inbox-md"><InboxFeedback text={entry.text} /></div><InboxImages client={client} sessionId={sessionId} images={entry.images} label="Your images" />{thread.receipts.some(receipt => receipt.message_id === entry.message_id && receipt.stage === "stored") && <small className="inbox-delivery">waiting to be picked up</small>}</article></li>;
      return <Fragment key={`${entry.message_id}:${index}`}>{showDay && <li className="inbox-e inbox-day"><span>{date.toLocaleDateString(undefined, { month: "short", day: "numeric", ...(date.getFullYear() !== new Date().getFullYear() ? { year: "numeric" } : {}) })}</span></li>}{content}</Fragment>;
    })}
  </ol>;
}
