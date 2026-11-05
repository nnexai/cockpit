import type { InboxQuestion } from "../../protocol/generated/v1";

type QuestionId = Pick<InboxQuestion, "run_id" | "question_message_id">;

/** Exact question identity: the same message id from two owners is two questions. */
export const questionKey = (question: QuestionId) => JSON.stringify([question.run_id, question.question_message_id]);

/** One answer draft per `(thread, run_id, question_message_id)`; follow-ups use the thread id. */
export const answerDraftKey = (threadId: string, question: QuestionId) => JSON.stringify(["answer", threadId, question.run_id, question.question_message_id]);

export function parseAnswerDraftKey(key: string): { threadId: string; runId: string; questionId: string } | null {
  if (!key.startsWith('["answer"')) return null;
  try {
    const value: unknown = JSON.parse(key);
    return Array.isArray(value) && value.length === 4 && value.every(part => typeof part === "string") ? { threadId: value[1], runId: value[2], questionId: value[3] } : null;
  } catch { return null; }
}

/** Plain, single-line rendering of a question for quotes and previews. */
export const plainQuestion = (text: string) => text.replace(/<[^>]*>|[*_`#>\[\]]/g, "").replace(/\s+/g, " ").trim();

/** Non-empty text drafts that belong to a thread: its follow-up and each answer. */
export function threadDraftCount(drafts: Record<string, string>, threadId: string): number {
  return Object.entries(drafts).filter(([key, value]) => value.trim() && (key === threadId || parseAnswerDraftKey(key)?.threadId === threadId)).length;
}

export function draftBadge(textDrafts: number, images: number): string | null {
  if (textDrafts === 0 && images === 0) return null;
  const drafts = textDrafts > 1 ? `Draft ×${textDrafts}` : "Draft";
  return images > 0 ? `${drafts} · ${images} img` : drafts;
}

export type StrandedDraft = { key: string; text: string; runId: string; questionId: string };

/** Answer drafts of a thread whose question is no longer unresolved. */
export function strandedAnswerDrafts(drafts: Record<string, string>, threadId: string, open: readonly QuestionId[]): StrandedDraft[] {
  const openKeys = new Set(open.map(questionKey));
  const stranded: StrandedDraft[] = [];
  for (const [key, text] of Object.entries(drafts)) {
    const parsed = parseAnswerDraftKey(key);
    if (parsed?.threadId === threadId && !openKeys.has(JSON.stringify([parsed.runId, parsed.questionId]))) stranded.push({ key, text, runId: parsed.runId, questionId: parsed.questionId });
  }
  return stranded;
}

/**
 * A replaced question keeps the typed answer: when exactly one open question from the same owner can receive it
 * and has no draft of its own, the text moves there (flagged as carried). Otherwise the draft stays where it is
 * and is surfaced to the user; it is never sent or merged into another target implicitly.
 */
export function carryAnswerDrafts(drafts: Record<string, string>, threadId: string, open: readonly QuestionId[]): { drafts: Record<string, string>; carried: string[] } {
  const next = { ...drafts };
  const carried: string[] = [];
  for (const orphan of strandedAnswerDrafts(drafts, threadId, open)) {
    if (!orphan.text.trim()) { delete next[orphan.key]; continue; }
    const candidates = open.filter(question => question.run_id === orphan.runId);
    if (candidates.length !== 1) continue;
    const target = answerDraftKey(threadId, candidates[0]!);
    if ((next[target] ?? "").trim()) continue;
    next[target] = orphan.text;
    delete next[orphan.key];
    carried.push(target);
  }
  return { drafts: next, carried };
}

/** Choice buttons never overwrite typed text silently. */
export type ChoiceOutcome = { kind: "fill" | "keep" } | { kind: "confirm" };
export function choiceOutcome(draft: string, choice: string): ChoiceOutcome {
  if (!draft.trim()) return { kind: "fill" };
  return draft === choice ? { kind: "keep" } : { kind: "confirm" };
}
export const addBelow = (draft: string, choice: string) => `${draft.replace(/\s+$/, "")}\n${choice}`;
