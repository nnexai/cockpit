import { useContext, useLayoutEffect, useRef } from "react";
import { NotesEditorScope } from "./MarkdownEditor";
import type { NotesTodo } from "../../protocol/generated/v1";
import { acknowledgeDraft, changedDraft } from "./drafts"
import { todoIdentity, todoSelector, type NotesModel } from "./useNotes"

const titleSelections = new Map<string, { start: number; end: number }>();
function resizeTitle(textarea: HTMLTextAreaElement) {
  textarea.style.height = "auto";
  const border = textarea.offsetHeight - textarea.clientHeight;
  textarea.style.height = `${Math.max(31, textarea.scrollHeight + border)}px`;
}
export function TodoTitle({ model, todo, className = "" }: { model: NotesModel; todo: NotesTodo; className?: string }) {
  const key = todoIdentity(todo);
  const draft = model.drafts.todos[key];
  const value = draft?.value ?? todo.text;
  const dirty = draft && draft.value !== draft.base;
  const scope = useContext(NotesEditorScope);
  const field = useRef<HTMLTextAreaElement>(null);
  const selectionKey = `${scope}/${key}`;
  useLayoutEffect(() => {
    const textarea = field.current;
    if (!textarea) return;
    resizeTitle(textarea);
    const selection = titleSelections.get(selectionKey);
    if (selection && document.activeElement !== textarea) textarea.setSelectionRange(selection.start, selection.end);
  }, [value, selectionKey]);
  useLayoutEffect(() => {
    const textarea = field.current;
    if (!textarea) return;
    let width = textarea.getBoundingClientRect().width;
    const observer = new ResizeObserver(() => {
      const nextWidth = textarea.getBoundingClientRect().width;
      if (nextWidth === width) return;
      width = nextWidth;
      resizeTitle(textarea);
    });
    observer.observe(textarea);
    return () => observer.disconnect();
  }, []);
  const blocked = todo.problems.includes("metadata_malformed") || todo.problems.includes("lazy_continuation");
  const save = async (keepMine = false) => {
    if (!draft || !dirty || blocked || model.busy || model.error?.code === "notes_outcome_unknown" || (!keepMine && draft.conflict)) return;
    const selector = keepMine ? todoSelector(todo) : todo.id && !todo.problems.includes("duplicate_id") ? { by: "id" as const, id: todo.id, expected_revision: draft.revision } : { by: "ref" as const, ref: todo.ref };
    const submitted = draft.value;
    await model.mutate({ op: "todo_update", todo: selector, text: submitted }, response => {
      if (response.result.kind !== "todo") return;
      const saved = response.result.todo;
      model.updateDrafts(current => {
        const edits = { ...current.todos };
        if (edits[key]) {
          const nextKey = todoIdentity(saved);
          const retained = acknowledgeDraft(edits[key], saved.text, saved.revision, submitted);
          delete edits[key];
          if (retained.value !== retained.base) edits[nextKey] = retained;
        }
        return { ...current, todos: edits };
      });
    });
  };
  return <div className={`notes-todo-title ${className}`}>
    <textarea ref={field} rows={1} onSelect={event => {
      titleSelections.set(selectionKey, { start: event.currentTarget.selectionStart, end: event.currentTarget.selectionEnd });
      if (titleSelections.size > 256) { const oldest = titleSelections.keys().next().value; if (oldest !== undefined) titleSelections.delete(oldest); }
    }} aria-label={`Task title: ${todo.text}`} value={value} disabled={blocked} onChange={event => {
      const text = event.target.value;
      model.updateDrafts(current => ({ ...current, todos: { ...current.todos, [key]: { ...(current.todos[key] ?? changedDraft(todo.text, todo.text, todo.revision)), value: text } } }));
    }} onKeyDown={event => {
      if (event.key === "Enter" && !event.shiftKey && !event.nativeEvent.isComposing) { event.preventDefault(); if (!event.repeat && !draft?.conflict) void save(); }
    }} />
    {dirty ? draft.conflict ? <div className="notes-conflict" role="alert"><span>Changed elsewhere. Saved title: {todo.text}</span><button type="button" disabled={model.busy || blocked} onClick={() => void save(true)}>Keep mine and save</button><button type="button" onClick={() => model.updateDrafts(current => ({ ...current, todos: { ...current.todos, [key]: changedDraft(todo.text, todo.text, todo.revision) } }))}>Reload (discard)</button></div> : <small>Draft kept <button type="button" disabled={model.busy || blocked} onClick={() => void save()}>Save title</button></small> : null}
    {todo.problems.length ? <small className="notes-item-problems">Source issue: {todo.problems.join(", ")}. {blocked ? "Repair this item in todos.md; Cockpit will not guess its boundaries." : "An explicit edit adopts or repairs this item."}</small> : null}
  </div>;
}
