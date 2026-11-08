import { createElement } from "react";
import { renderToStaticMarkup } from "react-dom/server";
import { describe, expect, it } from "vitest";
import type { InboxEntry, InboxEntryKind, InboxQuestion, InboxReceipt, InboxThread } from "../../protocol/generated/v1";
import type { CockpitClient } from "../../client/CockpitClient";
import { answerContext, groupTimeline, InboxTimeline } from "./InboxTimeline";

function entry(id: string, kind: InboxEntryKind, inReplyTo: string | null = null): InboxEntry {
  return { message_id: id, kind, in_reply_to: inReplyTo, at: "2026-11-03T12:00:00Z", text: id, source: { kind: "user" }, outcome: null, images: [], artifacts: [] };
}

describe("groupTimeline", () => {
  it("keeps directives as separate entries between update groups", () => {
    const directive = { ...entry("directive", "directive"), task_title: "Build" };
    const updates = [entry("1", "update"), entry("2", "update"), entry("3", "update")];
    expect(groupTimeline([...updates, directive, ...updates])).toEqual([
      { kind: "updates", entries: updates },
      { kind: "entry", entry: directive, answers: [] },
      { kind: "updates", entries: updates },
    ]);
  });
  it("collapses three or more consecutive updates", () => {
    const updates = [entry("1", "update"), entry("2", "update"), entry("3", "update"), entry("4", "update")];
    expect(groupTimeline(updates)).toEqual([{ kind: "updates", entries: updates }]);
  });

  it("keeps one or two updates as separate entries", () => {
    const updates = [entry("1", "update"), entry("2", "update")];
    expect(groupTimeline(updates)).toEqual(updates.map(update => ({ kind: "entry", entry: update, answers: [] })));
    expect(groupTimeline(updates.slice(0, 1))).toHaveLength(1);
  });

  it("preserves answer and orphan positions across intervening entries", () => {
    const question = entry("q", "question");
    const answer = entry("a", "answer", "q");
    const orphan = entry("orphan", "answer", "unknown");
    const update = entry("u", "update");
    expect(groupTimeline([question, update, answer, orphan])).toEqual(
      [question, update, answer, orphan].map(item => ({ kind: "entry", entry: item, answers: [] })),
    );
  });

  it("preserves request, result and follow-up order and separates update runs", () => {
    const request = entry("request", "request");
    const result = entry("result", "result");
    const followUp = entry("follow-up", "follow_up");
    const updates = [entry("1", "update"), entry("2", "update"), entry("3", "update")];
    expect(groupTimeline([request, ...updates, result, followUp])).toEqual([
      { kind: "entry", entry: request, answers: [] },
      { kind: "updates", entries: updates },
      { kind: "entry", entry: result, answers: [] },
      { kind: "entry", entry: followUp, answers: [] },
    ]);
  });

  it("keeps an interim in place between update groups and the result", () => {
    const before = [entry("1", "update"), entry("2", "update"), entry("3", "update")];
    const interim = { ...entry("interim", "interim"), source: { kind: "worker" as const, run_id: "worker-1" } };
    const after = [entry("4", "update"), entry("5", "update"), entry("6", "update")];
    const result = entry("result", "result");
    expect(groupTimeline([...before, interim, ...after, result])).toEqual([
      { kind: "updates", entries: before },
      { kind: "entry", entry: interim, answers: [] },
      { kind: "updates", entries: after },
      { kind: "entry", entry: result, answers: [] },
    ]);
  });

  it("keeps a publication between updates rather than hiding it in a collapsed group", () => {
    const before = [entry("1", "update"), entry("2", "update"), entry("3", "update")];
    const publication = entry("publication", "widget");
    const after = [entry("4", "update"), entry("5", "update"), entry("6", "update")];
    expect(groupTimeline([...before, publication, ...after])).toEqual([
      { kind: "updates", entries: before },
      { kind: "entry", entry: publication, answers: [] },
      { kind: "updates", entries: after },
    ]);
  });

  it("does not move an answer across a widget publication", () => {
    const question = entry("q", "question");
    const publication = entry("w", "widget");
    const answer = entry("a", "answer", "q");
    expect(groupTimeline([question, publication, answer])).toEqual([
      { kind: "entry", entry: question, answers: [] },
      { kind: "entry", entry: publication, answers: [] },
      { kind: "entry", entry: answer, answers: [] },
    ]);
  });
});

function receipt(to: string, stage: InboxReceipt["stage"] = "acked"): InboxReceipt {
  return { message_id: "a", kind: "answer", to_run_id: to, attempt_run_id: null, stage, created_at: "2026-11-03T12:00:00Z", acked_at: null, stale: false };
}

function ownedQuestion(run: string, text: string): InboxEntry {
  return { ...entry("q", "question"), text, source: { kind: "owner", run_id: run, harness: "omp" } };
}

describe("answer context", () => {
  it("uses the receipt recipient to disambiguate matching question IDs", () => {
    const one = ownedQuestion("one", "Question one");
    const two = ownedQuestion("two", "Question two");
    const answer = entry("a", "answer", "q");
    const delivered = receipt("two", "read");
    expect(answerContext(answer, [one, two], [delivered])).toEqual({ receipt: delivered, question: two });
    expect(answerContext(answer, [one], [delivered])).toEqual({ receipt: delivered, question: undefined });
    expect(answerContext(answer, [one, two], [])).toEqual({ receipt: undefined, question: undefined });
    expect(answerContext(answer, [one, two], [receipt("one"), delivered])).toEqual({ receipt: undefined, question: undefined });
    expect(answerContext(answer, [two, two], [delivered]).question).toBeUndefined();
  });
});

describe("InboxTimeline rendering", () => {
  it("renders answers in append order with the correct question and receipt", () => {
    const one = ownedQuestion("one", "First question");
    const two = ownedQuestion("two", "Second question");
    const update = { ...entry("u", "update"), text: "Intervening update" };
    const answer = { ...entry("a", "answer", "q"), text: "Answer text" };
    const orphan = { ...entry("orphan", "answer", "missing"), text: "Orphan text" };
    const thread = { thread_id: "thread", state: "underway", questions: [], receipts: [receipt("two", "read")], timeline: [one, two, update, answer, orphan], timeline_omitted: 0, unread_interims: [] } as unknown as InboxThread;
    const html = renderToStaticMarkup(createElement(InboxTimeline, { thread, view: null, client: {} as CockpitClient, sessionId: "session", ownerLabel: run => run, visible: true, questionCard: () => null, resultActions: null, interimActions: () => null, acknowledged: null, recommendation: null }));
    expect(html.indexOf("Intervening update")).toBeLessThan(html.indexOf("Answer text"));
    expect(html.indexOf("Answer text")).toBeLessThan(html.indexOf("Orphan text"));
    expect(html).toContain("In reply to: Second question");
    expect(html).not.toContain("In reply to: First question");
    expect(html).toContain("Delivered · picked up");
  });

  it("attributes owner-recorded answers without borrowing a user receipt", () => {
    const answer = { ...entry("a", "answer", "q"), source: { kind: "owner" as const, run_id: "one", harness: "omp" }, text: "Recorded answer" };
    const thread = { thread_id: "thread", state: "underway", questions: [], receipts: [receipt("one")], timeline: [ownedQuestion("one", "Question"), answer], timeline_omitted: 0, unread_interims: [] } as unknown as InboxThread;
    const html = renderToStaticMarkup(createElement(InboxTimeline, { thread, view: null, client: {} as CockpitClient, sessionId: "session", ownerLabel: run => run, visible: true, questionCard: () => null, resultActions: null, interimActions: () => null, acknowledged: null, recommendation: null }));
    expect(html).toContain("Owner recorded");
    expect(html).toContain("Recorded answer");
    expect(html).toContain("In reply to: Question");
    expect(html).not.toContain("Delivered · received");
  });

  it("renders worker interims independently of updates and final results", () => {
    const updates = [entry("1", "update"), entry("2", "update"), entry("3", "update")];
    const interim = { ...entry("interim", "interim"), text: "Partial work", task_title: "Build task", source: { kind: "worker" as const, run_id: "worker-1" }, interim: { run_id: "worker-1", message_id: "interim", task_id: "task-1", attempt: 1 } };
    const result = { ...entry("result", "result"), text: "Final work" };
    const thread = { thread_id: "thread", state: "underway", questions: [], receipts: [], timeline: [...updates, interim, result], timeline_omitted: 0, unread_interims: [{ run_id: "worker-1", message_id: "interim", task_id: "task-1", attempt: 1 }] } as unknown as InboxThread;
    const html = renderToStaticMarkup(createElement(InboxTimeline, { thread, view: null, client: {} as CockpitClient, sessionId: "session", ownerLabel: run => run, visible: true, questionCard: () => null, resultActions: "Result action", interimActions: item => `Read ${item.message_id}`, acknowledged: null, recommendation: null }));
    expect(html).toContain("3 updates");
    expect(html).toContain("Interim deliverable");
    expect(html).toContain("Worker claim · Build task · attempt 1 · Run worker-1");
    expect(html).toContain("Work continues. Not a final result.");
    expect(html).toContain("Read interim");
    expect(html.indexOf("3 updates")).toBeLessThan(html.indexOf("Partial work"));
    expect(html.indexOf("Partial work")).toBeLessThan(html.indexOf("Final work"));
    expect(html).toContain("New");
  });

  it("matches unread interims by worker as well as message ID", () => {
    const interim = { ...entry("shared", "interim"), source: { kind: "worker" as const, run_id: "worker-1" }, interim: { run_id: "worker-1", message_id: "shared", task_id: "task-1", attempt: 1 } };
    const thread = { thread_id: "thread", state: "underway", questions: [], receipts: [], timeline: [interim], timeline_omitted: 0, unread_interims: [{ run_id: "worker-2", message_id: "shared", task_id: "task-2", attempt: 1 }] } as unknown as InboxThread;
    const html = renderToStaticMarkup(createElement(InboxTimeline, { thread, view: null, client: {} as CockpitClient, sessionId: "session", ownerLabel: run => run, visible: true, questionCard: () => null, resultActions: null, interimActions: () => null, acknowledged: null, recommendation: null }));
    expect(html).toContain("Run worker-1");
    expect(html).not.toContain("inbox-new-pill");
    expect(html).toContain("1 unread interim deliverable is not available in this view");
  });

  it("places unretained open questions in a labelled section before timeline entries", () => {
    const question = { run_id: "one", question_message_id: "old", receipt: { status: "unresolved" } } as InboxQuestion;
    const thread = { thread_id: "thread", state: "underway", questions: [question], receipts: [], timeline: [entry("new", "update")], timeline_omitted: 1, unread_interims: [] } as unknown as InboxThread;
    const html = renderToStaticMarkup(createElement(InboxTimeline, { thread, view: null, client: {} as CockpitClient, sessionId: "session", ownerLabel: run => run, visible: true, questionCard: () => "Open question card", resultActions: null, interimActions: () => null, acknowledged: null, recommendation: null }));
    expect(html.indexOf("Open questions outside the timeline")).toBeLessThan(html.indexOf("Open question card"));
    expect(html.indexOf("Open question card")).toBeLessThan(html.indexOf("new"));
  });
});
