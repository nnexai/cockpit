export type TextDraft = { value: string; base: string; revision: string; conflict: boolean };
export type DecisionDraft = { title: string; body: string; decided: string; replaces: string | null; revision: string | null };
export type NotesDrafts = {
  scratchpad?: TextDraft;
  unknownOutcome?: string;
  todos: Record<string, TextDraft>;
  decisionEdits: Record<string, { title: TextDraft; body: TextDraft }>;
  newDecision?: DecisionDraft;
  comments: Record<string, TextDraft>;
  commentNew: Record<string, { body: string; author: string }>;
  addTodo: string;
  addCard: string;
};
export function emptyDrafts(): NotesDrafts { return { todos: {}, decisionEdits: {}, comments: {}, commentNew: {}, addTodo: "", addCard: "" }; }
export function reconcileDraft(draft: TextDraft | undefined, value: string, revision: string): TextDraft {
  if (!draft || draft.value === draft.base) return { value, base: value, revision, conflict: false };
  return { ...draft, conflict: draft.conflict || draft.revision !== revision };
}
export function changedDraft(value: string, base: string, revision: string): TextDraft { return { value, base, revision, conflict: false }; }
/** A confirmed save advances the base without erasing text typed during the request. */
export function acknowledgeDraft(draft: TextDraft, content: string, revision: string, submitted: string): TextDraft {
  return { value: draft.value === submitted ? content : draft.value, base: content, revision, conflict: false };
}
const key = (notesId: string) => `cockpit.notes.drafts.v1:${notesId}`;
const memory = new Map<string, NotesDrafts>();
const listeners = new Map<string, Set<(drafts: NotesDrafts) => void>>();
export function subscribeDrafts(notesId: string, listener: (drafts: NotesDrafts) => void): () => void {
  let group = listeners.get(notesId);
  if (!group) { group = new Set(); listeners.set(notesId, group); }
  group.add(listener);
  const subscribers = group;
  return () => { subscribers.delete(listener); if (!subscribers.size) listeners.delete(notesId); };
}
function isTextDraft(value: unknown): value is TextDraft {
  if (!value || typeof value !== "object") return false;
  const draft = value as Record<string, unknown>;
  return typeof draft.value === "string" && typeof draft.base === "string" && typeof draft.revision === "string" && typeof draft.conflict === "boolean";
}
function dictionary<T>(value: unknown, valid: (item: unknown) => item is T): Record<string, T> {
  if (!value || typeof value !== "object" || Array.isArray(value)) return {};
  return Object.fromEntries(Object.entries(value).filter(([, item]) => valid(item))) as Record<string, T>;
}
export function readDrafts(notesId: string): NotesDrafts {
  const cached = memory.get(notesId);
  if (cached) return cached;
  try {
    const raw = localStorage.getItem(key(notesId));
    if (!raw) return emptyDrafts();
    const value: unknown = JSON.parse(raw);
    if (!value || typeof value !== "object") return emptyDrafts();
    const stored = value as Record<string, unknown>;
    const result = emptyDrafts();
    if (isTextDraft(stored.scratchpad)) result.scratchpad = stored.scratchpad;
    result.todos = dictionary(stored.todos, isTextDraft);
    result.comments = dictionary(stored.comments, isTextDraft);
    result.decisionEdits = dictionary(stored.decisionEdits, (item): item is { title: TextDraft; body: TextDraft } => Boolean(item && typeof item === "object" && "title" in item && "body" in item && isTextDraft(item.title) && isTextDraft(item.body)));
    result.commentNew = dictionary(stored.commentNew, (item): item is { body: string; author: string } => Boolean(item && typeof item === "object" && "body" in item && "author" in item && typeof item.body === "string" && typeof item.author === "string"));
    if (stored.newDecision && typeof stored.newDecision === "object") {
      const draft = stored.newDecision as Record<string, unknown>;
      if (typeof draft.title === "string" && typeof draft.body === "string" && typeof draft.decided === "string" && (draft.replaces === null || typeof draft.replaces === "string") && (draft.revision === null || typeof draft.revision === "string")) result.newDecision = draft as DecisionDraft;
    }
    if (typeof stored.addTodo === "string") result.addTodo = stored.addTodo;
    if (typeof stored.addCard === "string") result.addCard = stored.addCard;
    if (typeof stored.unknownOutcome === "string") result.unknownOutcome = stored.unknownOutcome;
    memory.set(notesId, result);
    return result;
  } catch { return emptyDrafts(); }
}
export function storeDrafts(notesId: string, drafts: NotesDrafts): string | null {
  memory.set(notesId, drafts);
  for (const listener of listeners.get(notesId) ?? []) listener(drafts);
  try { localStorage.setItem(key(notesId), JSON.stringify(drafts)); return null; }
  catch { return "Draft retained for this app session, but browser storage is unavailable. Save before restarting Cockpit."; }
}
