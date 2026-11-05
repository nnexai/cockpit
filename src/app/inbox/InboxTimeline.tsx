import { Fragment, useLayoutEffect, useRef, useState, type ReactNode } from "react";
import type { CockpitClient } from "../../client/CockpitClient";
import type { InboxEntry, InboxQuestion, InboxThread, InboxView } from "../../protocol/generated/v1";
import { UiIcon } from "../UiIcon";
import { Time } from "./InboxTime";
import { InboxFeedback } from "./InboxFeedback";
import { InboxImages } from "./InboxImages";
import { InboxTimelineWidget } from "./InboxLiveWidgets";
import "./inbox-detail.css";
import { answerReceiptLine } from "./inboxDelivery";

export type TimelineItem =
  | { kind: "entry"; entry: InboxEntry; answers: InboxEntry[] }
  | { kind: "updates"; entries: InboxEntry[] };

/** Preserve chronological order while keeping answers with their question. */
export function groupTimeline(entries: InboxEntry[]): TimelineItem[] {
  const answers = new Map<string, InboxEntry[]>();
  const separatedAnswers = new Set<InboxEntry>();
  const questions = new Map<string, number>();
  let lastWidget = -1;
  entries.forEach((entry, index) => {
    if (entry.kind === "widget") lastWidget = index;
    if (entry.kind === "question") questions.set(entry.message_id, index);
    if (entry.kind === "answer" && entry.in_reply_to) {
      const question = questions.get(entry.in_reply_to);
      if (question !== undefined && question < lastWidget) {
        separatedAnswers.add(entry);
      } else {
        const list = answers.get(entry.in_reply_to) ?? [];
        list.push(entry);
        answers.set(entry.in_reply_to, list);
      }
    }
  });
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
    if (entry.kind !== "answer" || separatedAnswers.has(entry)) items.push({ kind: "entry", entry, answers: entry.kind === "question" ? answers.get(entry.message_id) ?? [] : [] });
  }
  flush();
  return items;
}



function gap(from: string, to: string): string | null {
  const minutes = Math.max(0, Math.round((Date.parse(to) - Date.parse(from)) / 60000));
  return Number.isFinite(minutes) ? `${minutes} min later` : null;
}

/** `Today` and `Yesterday` read better than a date for the latest entries. */
function dayLabel(date: Date): string {
  const now = new Date();
  if (date.toDateString() === now.toDateString()) return "Today";
  if (date.toDateString() === new Date(now.getFullYear(), now.getMonth(), now.getDate() - 1).toDateString()) return "Yesterday";
  return date.toLocaleDateString(undefined, { month: "short", day: "numeric", ...(date.getFullYear() !== now.getFullYear() ? { year: "numeric" } : {}) });
}

function Update({ entry, client, sessionId }: { entry: InboxEntry; client: CockpitClient; sessionId: string }) {
  return <details className="inbox-upd"><summary><div className="inbox-upd-t"><InboxFeedback text={entry.text} /></div><Time value={entry.at} /><span className="inbox-chev"><UiIcon name="right" /></span></summary><div className="inbox-md"><InboxFeedback text={entry.text} /><InboxImages client={client} sessionId={sessionId} images={entry.images} label="Update images" /></div></details>;
}

function ClampedText({ text }: { text: string }) {
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

type TimelineProps = {
  thread: InboxThread; view: InboxView | null; client: CockpitClient; sessionId: string; ownerLabel(runId: string): string; visible: boolean;
  /** The one rendering of an open question; the timeline places it where the question was asked. */
  questionCard(question: InboxQuestion): ReactNode;
  /** Result actions are hosted by the unread result's card, or after the last entry when that card is not in this view. */
  resultActions: ReactNode;
  acknowledged: string | null;
  recommendation: ReactNode;
};

export function InboxTimeline({ thread, view, client, sessionId, ownerLabel, visible, questionCard, resultActions, acknowledged, recommendation }: TimelineProps) {
  const items = groupTimeline(thread.timeline);
  const open = thread.questions.filter(question => question.receipt.status === "unresolved");
  const placed = new Set<InboxQuestion>();
  const hostsResult = items.some(item => item.kind === "entry" && item.entry.kind === "result" && item.entry.message_id === thread.unread_result_message_id);
  let lastDay = "";
  return <ol className="inbox-tl" aria-label="Timeline">
    {thread.timeline_omitted > 0 && <li className="inbox-e inbox-day"><span>{thread.timeline_omitted} earlier entries</span></li>}
    {!!thread.earlier_widgets?.length && <li className="inbox-e inbox-day"><span>Earlier live views</span></li>}
    {!!thread.earlier_widgets_omitted && <li className="inbox-e inbox-day"><span>{thread.earlier_widgets_omitted} older live views not shown</span></li>}
    {thread.earlier_widgets?.map(entry => entry.widget && <li className="inbox-e inbox-widget-event" key={JSON.stringify([thread.thread_id, entry.message_id])}><InboxTimelineWidget client={client} sessionId={sessionId} event={entry.widget} visible={visible} /></li>)}
    {items.map((item, index) => {
      const entry = item.kind === "updates" ? item.entries[0] : item.entry;
      if (item.kind === "entry" && entry.kind === "widget" && !entry.widget) return <li className="inbox-e inbox-widget-event" key={JSON.stringify([thread.thread_id, entry.message_id])}><article className="inbox-live-widget"><header><strong>View unavailable</strong></header></article></li>;
      const date = new Date(entry.at);
      const day = Number.isNaN(date.getTime()) ? "" : date.toDateString();
      const showDay = day !== lastDay && !!day;
      lastDay = day;
      const content = item.kind === "entry" && entry.kind === "widget" && entry.widget ? <li className="inbox-e inbox-widget-event"><InboxTimelineWidget client={client} sessionId={sessionId} event={entry.widget} visible={visible} /></li> : item.kind === "updates" ? <li className="inbox-e is-compact"><span className="inbox-dot is-run" /><details className="inbox-run"><summary><span>{item.entries.length} updates</span><Time value={entry.at} /><span>–</span><Time value={item.entries[item.entries.length - 1].at} /><span className="inbox-chev"><UiIcon name="right" /></span></summary><ol className="inbox-run-list">{item.entries.map(update => <li key={update.message_id}><Update entry={update} client={client} sessionId={sessionId} /></li>)}</ol></details></li> : entry.kind === "update" ? <li className="inbox-e is-compact"><span className={`inbox-dot${thread.state === "underway" && index === items.length - 1 ? " is-live" : ""}`} /><Update entry={entry} client={client} sessionId={sessionId} /></li> : entry.kind === "question" ? (() => {
        const owned = entry.source.kind === "owner" || entry.source.kind === "owner_card" || entry.source.kind === "worker" ? entry.source.run_id : null;
        const match = open.find(question => question.question_message_id === entry.message_id && (owned === null || owned === question.run_id));
        if (match && !placed.has(match)) { placed.add(match); return <li className="inbox-e inbox-e-q"><span className="inbox-kic" data-tone="decision"><UiIcon name="help" /></span>{questionCard(match)}</li>; }
        const source = entry.source;
        const label = source.kind === "owner" || source.kind === "owner_card" ? view?.owners.find(owner => owner.root_id === source.run_id)?.label ?? ownerLabel(source.run_id) : "Owner";
        return <li className="inbox-e inbox-e-q"><span className="inbox-kic" data-tone="decision"><UiIcon name="help" /></span><div className="inbox-asked"><header className="inbox-card-h"><strong>{label} asked</strong><span className="inbox-sp" /><Time value={entry.at} /></header><ClampedText text={entry.text} /><InboxImages client={client} sessionId={sessionId} images={entry.images} label="Question images" />{item.answers.map(answer => { const receipt = thread.receipts.find(r => r.message_id === answer.message_id), later = gap(entry.at, answer.at); return <div className="inbox-answer" key={answer.message_id}><header className="inbox-card-h"><strong>You answered</strong>{later && <span className="inbox-kind">{later}</span>}<span className="inbox-sp" /><Time value={answer.at} /></header><div className="inbox-md"><InboxFeedback text={answer.text} /></div><InboxImages client={client} sessionId={sessionId} images={answer.images} label="Answer images" />{receipt && <small className="inbox-receipt"><UiIcon name="check" />{answerReceiptLine(receipt.stage)}</small>}</div>; })}</div></li>;
      })() : entry.kind === "result" ? <li className="inbox-e"><span className="inbox-kic" data-tone={entry.outcome === "failed" ? "blocked" : "done"}><UiIcon name={entry.outcome === "failed" ? "alert" : "check-circle"} /></span><article className="inbox-result" data-tone={entry.outcome === "failed" ? "blocked" : "done"}><header className="inbox-card-h"><strong>Result</strong>{entry.outcome && <span className="inbox-outcome"><UiIcon name={entry.outcome === "failed" ? "alert" : "check"} />{entry.outcome === "failed" ? "Failed" : "Succeeded"}</span>}{entry.message_id === thread.unread_result_message_id && <span className="inbox-new-pill">New</span>}<span className="inbox-sp" /><Time value={entry.at} /></header><ClampedText text={entry.text} /><InboxImages client={client} sessionId={sessionId} images={entry.images} label="Result images" />{entry.message_id === thread.unread_result_message_id ? resultActions : acknowledged === entry.message_id && <small className="inbox-receipt"><UiIcon name="check" />Acknowledged · you can still follow up</small>}</article></li> : <li className="inbox-e"><span className="inbox-kic" data-tone="review"><UiIcon name="user" /></span><article className="inbox-you"><header className="inbox-card-h"><strong>You</strong><span className="inbox-kind">{entry.kind === "request" ? "Request" : entry.kind === "answer" ? "Answer" : "Follow-up"}</span><span className="inbox-sp" /><Time value={entry.at} /></header><div className="inbox-md"><InboxFeedback text={entry.text} /></div><InboxImages client={client} sessionId={sessionId} images={entry.images} label="Your images" />{thread.receipts.some(receipt => receipt.message_id === entry.message_id && receipt.stage === "stored") && <small className="inbox-receipt"><UiIcon name="check" />{answerReceiptLine("stored")}</small>}</article></li>;
      return <Fragment key={JSON.stringify([thread.thread_id, entry.message_id])}>{showDay && <li className="inbox-e inbox-day"><span>{dayLabel(date)}</span></li>}{content}</Fragment>;
    })}
    {open.filter(question => !placed.has(question)).map(question => <li className="inbox-e inbox-e-q" key={JSON.stringify(["open", thread.thread_id, question.run_id, question.question_message_id])}><span className="inbox-kic" data-tone="decision"><UiIcon name="help" /></span>{questionCard(question)}</li>)}
    {!hostsResult && resultActions && <li className="inbox-e"><span className="inbox-kic" data-tone="review"><UiIcon name="eye" /></span><article className="inbox-result" data-tone={thread.timeline.find(entry => entry.message_id === thread.unread_result_message_id)?.outcome === "failed" ? "blocked" : "done"}><header className="inbox-card-h"><strong>Result</strong><span className="inbox-new-pill">New</span></header>{resultActions}</article></li>}
    {recommendation && <li className="inbox-e"><span className="inbox-kic" data-tone="review"><UiIcon name="spark" /></span><div className="inbox-asked">{recommendation}</div></li>}
  </ol>;
}
