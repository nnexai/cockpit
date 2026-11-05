import { describe, expect, it } from "vitest";
import { addBelow, answerDraftKey, carryAnswerDrafts, choiceOutcome, draftBadge, parseAnswerDraftKey, strandedAnswerDrafts, threadDraftCount } from "./inboxDrafts";

const q = (run_id: string, question_message_id: string) => ({ run_id, question_message_id });

describe("answer draft identity", () => {
  it("keys a draft by thread, owner and question message, so a shared message id never shares text", () => {
    expect(answerDraftKey("t", q("a", "m"))).not.toBe(answerDraftKey("t", q("b", "m")));
    expect(parseAnswerDraftKey(answerDraftKey("t", q("a", "m")))).toEqual({ threadId: "t", runId: "a", questionId: "m" });
  });
  it("does not mistake follow-up, direct or malformed keys for answers", () => {
    expect(parseAnswerDraftKey("thread-id")).toBeNull();
    expect(parseAnswerDraftKey(JSON.stringify(["agent", "pane"]))).toBeNull();
    expect(parseAnswerDraftKey('["answer", 1, 2, 3]')).toBeNull();
    expect(parseAnswerDraftKey('["answer"')).toBeNull();
  });
});

describe("draft badge", () => {
  it("counts only non-empty drafts that belong to the thread", () => {
    const drafts = { t: "follow", [answerDraftKey("t", q("a", "1"))]: "one", [answerDraftKey("t", q("a", "2"))]: "  ", [answerDraftKey("other", q("a", "1"))]: "elsewhere", u: "unrelated" };
    expect(threadDraftCount(drafts, "t")).toBe(2);
  });
  it("names the count and the follow-up images", () => {
    expect(draftBadge(0, 0)).toBeNull();
    expect(draftBadge(1, 0)).toBe("Draft");
    expect(draftBadge(3, 1)).toBe("Draft ×3 · 1 img");
    expect(draftBadge(0, 2)).toBe("Draft · 2 img");
  });
});

describe("choices", () => {
  it("fills an empty draft, keeps an identical one and asks before replacing anything else", () => {
    expect(choiceOutcome("", "Yes").kind).toBe("fill");
    expect(choiceOutcome("  \n", "Yes").kind).toBe("fill");
    expect(choiceOutcome("Yes", "Yes").kind).toBe("keep");
    expect(choiceOutcome("Yes, but", "Yes").kind).toBe("confirm");
  });
  it("adds a choice below the typed text without trailing blank lines", () => {
    expect(addBelow("typed  \n", "No")).toBe("typed\nNo");
  });
});

describe("replaced questions", () => {
  const open = [q("owner", "new")];
  it("moves the draft to the one open question from the same owner and leaves other drafts alone", () => {
    const old = answerDraftKey("t", q("owner", "old")), other = answerDraftKey("t", q("elsewhere", "x"));
    const result = carryAnswerDrafts({ [old]: "Blue", [other]: "kept", t: "follow-up" }, "t", open);
    expect(result.carried).toEqual([answerDraftKey("t", q("owner", "new"))]);
    expect(result.drafts[answerDraftKey("t", q("owner", "new"))]).toBe("Blue");
    expect(result.drafts[old]).toBeUndefined();
    expect(result.drafts[other]).toBe("kept");
    expect(result.drafts.t).toBe("follow-up");
  });
  it("never overwrites text already typed for the successor, nor guesses between several", () => {
    const old = answerDraftKey("t", q("owner", "old")), next = answerDraftKey("t", q("owner", "new"));
    const occupied = carryAnswerDrafts({ [old]: "Blue", [next]: "Already" }, "t", open);
    expect(occupied.carried).toEqual([]);
    expect(occupied.drafts[next]).toBe("Already");
    expect(strandedAnswerDrafts(occupied.drafts, "t", open).map(item => item.text)).toEqual(["Blue"]);
    const ambiguous = carryAnswerDrafts({ [old]: "Blue" }, "t", [q("owner", "a"), q("owner", "b")]);
    expect(ambiguous.carried).toEqual([]);
    expect(ambiguous.drafts[old]).toBe("Blue");
  });
  it("keeps a draft whose question is gone visible instead of sending or merging it, and drops empty ones", () => {
    const gone = answerDraftKey("t", q("owner", "gone")), empty = answerDraftKey("t", q("owner", "empty"));
    const result = carryAnswerDrafts({ [gone]: "Blue", [empty]: "  " }, "t", []);
    expect(result.carried).toEqual([]);
    expect(result.drafts[gone]).toBe("Blue");
    expect(empty in result.drafts).toBe(false);
  });
  it("ignores drafts of other threads", () => {
    const foreign = answerDraftKey("other", q("owner", "old"));
    expect(strandedAnswerDrafts({ [foreign]: "text" }, "t", [])).toEqual([]);
    expect(carryAnswerDrafts({ [foreign]: "text" }, "t", open).drafts[foreign]).toBe("text");
  });
});
