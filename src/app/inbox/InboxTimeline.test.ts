import { describe, expect, it } from "vitest";
import type { InboxEntry, InboxEntryKind } from "../../protocol/generated/v1";
import { groupTimeline } from "./InboxTimeline";

function entry(id: string, kind: InboxEntryKind, inReplyTo: string | null = null): InboxEntry {
  return { message_id: id, kind, in_reply_to: inReplyTo, at: "2026-11-03T12:00:00Z", text: id, source: { kind: "user" }, outcome: null, images: [] };
}

describe("groupTimeline", () => {
  it("collapses three or more consecutive updates", () => {
    const updates = [entry("1", "update"), entry("2", "update"), entry("3", "update"), entry("4", "update")];
    expect(groupTimeline(updates)).toEqual([{ kind: "updates", entries: updates }]);
  });

  it("keeps one or two updates as separate entries", () => {
    const updates = [entry("1", "update"), entry("2", "update")];
    expect(groupTimeline(updates)).toEqual(updates.map(update => ({ kind: "entry", entry: update, answers: [] })));
    expect(groupTimeline(updates.slice(0, 1))).toHaveLength(1);
  });

  it("attaches answers to their question and does not render answers independently", () => {
    const question = entry("q", "question");
    const answer = entry("a", "answer", "q");
    const orphan = entry("orphan", "answer", "unknown");
    const update = entry("u", "update");
    expect(groupTimeline([question, update, answer, orphan])).toEqual([
      { kind: "entry", entry: question, answers: [answer] },
      { kind: "entry", entry: update, answers: [] },
    ]);
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
