import { UiIcon } from "../UiIcon";
import { todoIdentity, type NotesModel } from "./useNotes";

/** Never guess a new task for a revision-qualified reference that disappeared. */
export function RetainedDrafts({ model }: { model: NotesModel }) {
  if (!model.scratchpad) return null;
  const taskKeys = new Set(model.todos.map(todoIdentity));
  const decisionKeys = new Set(model.decisions.map(decision => decision.decision_id));
  const titles = Object.entries(model.drafts.todos).filter(([key, draft]) => !taskKeys.has(key) && draft.value !== draft.base);
  const decisions = Object.entries(model.drafts.decisionEdits).filter(([key, draft]) => !decisionKeys.has(key) && (draft.title.value !== draft.title.base || draft.body.value !== draft.body.base));
  if (!titles.length && !decisions.length) return null;
  const count = titles.length + decisions.length;
  return <details className="notes-retained-drafts">
    <summary className="notes-retained-summary">
      <UiIcon name="info" />
      <span>{count} unsaved {count === 1 ? "edit" : "edits"} kept — source no longer exists</span>
      <span className="notes-disclosure-chevron"><UiIcon name="down" /></span>
    </summary>
    <div className="notes-retained-body">
      <p className="notes-retained-warning" role="status">The source was removed or its imported reference changed. Cockpit did not discard your text or guess a different target. Select the retained text to copy it into the correct source or a new record.</p>
      {titles.map(([key, draft]) => <label key={key}>
        Task source: <code>{key}</code>
        <textarea readOnly aria-label={`Retained task draft ${key}`} value={draft.value} onClick={event => event.currentTarget.select()} />
        <button type="button" onClick={() => model.updateDrafts(current => { const todos = { ...current.todos }; delete todos[key]; return { ...current, todos }; })}>Discard this draft</button>
      </label>)}
      {decisions.map(([key, draft]) => <label key={key}>
        Decision source: <code>{key}</code>
        <input readOnly aria-label={`Retained decision title ${key}`} value={draft.title.value} onClick={event => event.currentTarget.select()} />
        <textarea readOnly aria-label={`Retained decision Markdown ${key}`} value={draft.body.value} onClick={event => event.currentTarget.select()} />
        <button type="button" onClick={() => model.updateDrafts(current => { const edits = { ...current.decisionEdits }; delete edits[key]; return { ...current, decisionEdits: edits }; })}>Discard this draft</button>
      </label>)}
    </div>
  </details>;
}
