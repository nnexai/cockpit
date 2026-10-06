import { useCallback, useEffect, useRef, useState } from "react";
import type { CockpitClient } from "../../client/CockpitClient";
import type { NotesOperation, NotesTargetInfo, NotesTodo, NotesDecisionSummary, NotesResponse, NotesDocument } from "../../protocol/generated/v1";
import { readDrafts, reconcileDraft, storeDrafts, subscribeDrafts, type NotesDrafts } from "./drafts";

const pendingWrites = new Set<string>();
const writeListeners = new Map<string, Set<(busy: boolean) => void>>();

export interface NotesModel {
  drafts: NotesDrafts;
  updateDrafts(update: (current: NotesDrafts) => NotesDrafts): void;
  scratchpad: NotesDocument | null;
  todos: NotesTodo[];
  decisions: NotesDecisionSummary[];
  error: { code: string; message: string } | null;
  setError(error: { code: string; message: string } | null): void;
  storageError: string | null;
  busy: boolean;
  request(operation: NotesOperation): Promise<NotesResponse>;
  mutate(operation: NotesOperation, onSuccess?: (response: NotesResponse) => void): Promise<boolean>;
  refresh(): Promise<void>;
}
export function notesError(error: unknown): { code: string; message: string } {
  if (error instanceof Error) {
    const code = "operationCode" in error && typeof error.operationCode === "string" ? error.operationCode : "code" in error && typeof error.code === "string" ? error.code : "notes_unavailable";
    return { code, message: error.message };
  }
  return { code: "notes_unavailable", message: "Could not access Notes" };
}
export function notesMutationError(error: unknown): { code: string; message: string } {
  const problem = notesError(error);
  const authoritative = problem.code.startsWith("notes_") && error instanceof Error && (("operationCode" in error && typeof error.operationCode === "string") || ("code" in error && typeof error.code === "string"));
  return authoritative ? problem : { code: "notes_outcome_unknown", message: `No confirmed response to the write: ${problem.message}` };
}
export function todoIdentity(todo: NotesTodo): string {
  return todo.id && !todo.problems.includes("duplicate_id") ? todo.id : todo.ref;
}
export function todoSelector(todo: NotesTodo) {
  return todo.id && !todo.problems.includes("duplicate_id")
    ? { by: "id" as const, id: todo.id, expected_revision: todo.revision }
    : { by: "ref" as const, ref: todo.ref };
}
export function useNotes(client: CockpitClient, info: NotesTargetInfo): NotesModel {
  const [drafts, setDrafts] = useState(() => readDrafts(info.notes_id));
  const [storageError, setStorageError] = useState<string | null>(null);
  const [scratchpad, setScratchpad] = useState<NotesDocument | null>(null);
  const [todos, setTodos] = useState<NotesTodo[]>([]);
  const [decisions, setDecisions] = useState<NotesDecisionSummary[]>([]);
  const [error, setError] = useState<{ code: string; message: string } | null>(() => drafts.unknownOutcome ? { code: "notes_outcome_unknown", message: drafts.unknownOutcome } : null);
  const [busy, setBusy] = useState(() => pendingWrites.has(info.notes_id));
  const outcomeUnknown = useRef(Boolean(drafts.unknownOutcome));
  const alive = useRef(true);
  const latest = useRef(0);
  const tokens = useRef(info.change_tokens);
  const updateDrafts = useCallback((update: (current: NotesDrafts) => NotesDrafts) => {
    const next = update(readDrafts(info.notes_id));
    setDrafts(next);
    setStorageError(storeDrafts(info.notes_id, next));
  }, [info.notes_id]);
  useEffect(() => subscribeDrafts(info.notes_id, next => {
    setDrafts(next);
    outcomeUnknown.current = Boolean(next.unknownOutcome);
    if (next.unknownOutcome) setError({ code: "notes_outcome_unknown", message: next.unknownOutcome });
  }), [info.notes_id]);
  useEffect(() => {
    let group = writeListeners.get(info.notes_id);
    if (!group) { group = new Set(); writeListeners.set(info.notes_id, group); }
    group.add(setBusy);
    setBusy(pendingWrites.has(info.notes_id));
    const subscribers = group;
    return () => { subscribers.delete(setBusy); if (!subscribers.size) writeListeners.delete(info.notes_id); };
  }, [info.notes_id]);
  const request = useCallback((operation: NotesOperation) => client.notes({ target: { kind: "notes", notes_id: info.notes_id }, operation }), [client, info.notes_id]);
  const refresh = useCallback(async () => {
    const epoch = ++latest.current;
    const [scratch, tasks, records] = await Promise.all([
      request({ op: "scratchpad_read" }), request({ op: "todo_list", filter: "all" }), request({ op: "decision_list", status: "all", query: null }),
    ]);
    if (!alive.current || epoch !== latest.current) return;
    if (scratch.result.kind !== "scratchpad" || tasks.result.kind !== "todos" || records.result.kind !== "decisions") throw new Error("Notes returned an unexpected result");
    const document = scratch.result.document;
    const currentTodos = tasks.result.todos;
    setScratchpad(document); setTodos(currentTodos); setDecisions(records.result.decisions);
    updateDrafts(current => {
      const editedTodos = { ...current.todos };
      for (const task of currentTodos) {
        const key = todoIdentity(task);
        if (editedTodos[key]) editedTodos[key] = reconcileDraft(editedTodos[key], task.text, task.revision);
      }
      const edits = { ...current.decisionEdits };
      for (const record of records.result.kind === "decisions" ? records.result.decisions : []) {
        const draft = edits[record.decision_id];
        if (draft && draft.title.revision !== record.revision) edits[record.decision_id] = { title: { ...draft.title, conflict: true }, body: { ...draft.body, conflict: true } };
      }
      return { ...current, scratchpad: reconcileDraft(current.scratchpad, document.content, document.revision), todos: editedTodos, decisionEdits: edits };
    });
  }, [request, updateDrafts]);
  const mutate = useCallback(async (operation: NotesOperation, onSuccess?: (response: NotesResponse) => void): Promise<boolean> => {
    if (pendingWrites.has(info.notes_id) || outcomeUnknown.current) return false;
    pendingWrites.add(info.notes_id); setBusy(true); setError(null);
    for (const listener of writeListeners.get(info.notes_id) ?? []) listener(true);
    try {
      let result: NotesResponse;
      try { result = await request(operation); }
      catch (failure) {
        const problem = notesMutationError(failure);
        if (problem.code === "notes_outcome_unknown") {
          outcomeUnknown.current = true;
          updateDrafts(current => ({ ...current, unknownOutcome: problem.message }));
        }
        if (alive.current) setError(problem);
        try { if (alive.current) await refresh(); } catch { /* Keep the mutation's actionable error. */ }
        return false;
      }
      // The captured Notes UUID still owns the response after its surface has closed.
      onSuccess?.(result);
      if ("todo" in operation && result.result.kind === "todo") {
        const saved = result.result.todo;
        const previousKey = operation.todo.by === "id" ? operation.todo.id : operation.todo.ref;
        updateDrafts(current => {
          const previous = current.todos[previousKey];
          if (!previous || previous.base !== saved.text) return current;
          const todos = { ...current.todos };
          delete todos[previousKey];
          todos[todoIdentity(saved)] = { ...previous, revision: saved.revision };
          return { ...current, todos };
        });
      }
      try { if (alive.current) await refresh(); }
      catch (failure) { if (alive.current) setError({ ...notesError(failure), message: `Change saved, but refresh failed: ${notesError(failure).message}` }); }
      return true;
    } finally {
      pendingWrites.delete(info.notes_id);
      for (const listener of writeListeners.get(info.notes_id) ?? []) listener(false);
      if (alive.current) setBusy(false);
    }
  }, [request, refresh, updateDrafts, info.notes_id]);
  useEffect(() => {
    alive.current = true;
    let polling = false;
    const poll = async (force = false) => {
      if (polling || document.visibilityState === "hidden") return;
      polling = true;
      try {
        const response = await request({ op: "target_resolve" });
        if (!alive.current || response.result.kind !== "target") return;
        const next = response.result.info.change_tokens;
        if (force || Object.keys(next).some(key => next[key as keyof typeof next] !== tokens.current[key as keyof typeof next])) {
          await refresh();
          tokens.current = next;
        }
      } catch (failure) { if (alive.current && !outcomeUnknown.current) setError(notesError(failure)); }
      finally { polling = false; }
    };
    void refresh().catch(failure => { if (alive.current && !outcomeUnknown.current) setError(notesError(failure)); });
    const timer = window.setInterval(() => void poll(), 3000);
    const focus = () => { void poll(true); };
    window.addEventListener("focus", focus); document.addEventListener("visibilitychange", focus);
    return () => { alive.current = false; ++latest.current; window.clearInterval(timer); window.removeEventListener("focus", focus); document.removeEventListener("visibilitychange", focus); };
  }, [request, refresh]);
  const acknowledgeError = (problem: { code: string; message: string } | null) => {
    if (problem === null) {
      outcomeUnknown.current = false;
      updateDrafts(current => { const next = { ...current }; delete next.unknownOutcome; return next; });
    }
    setError(problem);
  };
  return { drafts, updateDrafts, scratchpad, todos, decisions, error, setError: acknowledgeError, storageError, busy, request, mutate, refresh };
}
