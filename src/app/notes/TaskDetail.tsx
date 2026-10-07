import { useContext, useEffect, useRef, useState } from "react";
import type { NotesComment, NotesTodo } from "../../protocol/generated/v1";
import { UiIcon } from "../UiIcon";
import { MarkdownEditor, MarkdownPreview, NotesEditorScope } from "./MarkdownEditor";
import { acknowledgeDraft, changedDraft, reconcileDraft } from "./drafts"
import { notesError, todoSelector, type NotesModel } from "./useNotes";
import { TodoTitle } from "./TodoTitle";

const threadPositions = new Map<string, number>();
const commentFocusSelectors: Record<"edit" | "delete" | "editor", string> = { edit: ".notes-row-actions button:first-child", delete: ".notes-row-actions button:last-child", editor: ".notes-comment-edit .cm-content" };
export function TaskDetail({ model, todo, todoId, onClose, onCount }: {
  model: NotesModel; todo: NotesTodo | null; todoId: string; onClose(): void; onCount(todoId: string, count: number): void;
}) {
  const [comments, setComments] = useState<NotesComment[]>([]);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState<string | null>(null);
  const [editing, setEditing] = useState<string | null>(null);
  const [deleting, setDeleting] = useState<{ id: string; revision: string } | null>(null);
  const editingRef = useRef(editing); editingRef.current = editing;
  const deletingRef = useRef(deleting); deletingRef.current = deleting;
  const [newBelow, setNewBelow] = useState(0);
  const thread = useRef<HTMLDivElement>(null);
  const heading = useRef<HTMLHeadingElement>(null);
  const commentsRef = useRef(comments);
  commentsRef.current = comments;
  const refreshRef = useRef<() => Promise<void>>(async () => {});
  const countRef = useRef(onCount); countRef.current = onCount;
  const composer = model.drafts.commentNew[todoId] ?? { body: "", author: "Local user" };
  const unknown = model.error?.code === "notes_outcome_unknown";
  const scope = useContext(NotesEditorScope);
  const threadKey = `${scope}/${todoId}`;
  const firstRead = useRef(true);
  useEffect(() => {
    if (!editing) return;
    const frame = requestAnimationFrame(() => { const source = thread.current?.querySelector<HTMLElement>(".notes-comment-edit .cm-content"); if (source && !source.closest("[inert]")) source.focus(); });
    return () => cancelAnimationFrame(frame);
  }, [editing]);
  useEffect(() => {
    let active = true;
    let reading = false;
    heading.current?.focus({ preventScroll: true });
    const load = async () => {
      if (reading) return;
      reading = true;
      try {
        const response = await model.request({ op: "comment_list", todo_id: todoId });
        if (!active) return;
        if (response.result.kind !== "comments") throw new Error("Unexpected comments response");
        const next = response.result.comments;
        const oldIds = new Set(commentsRef.current.map(comment => comment.comment_id));
        const additions = next.filter(comment => !oldIds.has(comment.comment_id)).length;
        const scroller = thread.current;
        const atEnd = !scroller || scroller.scrollHeight - scroller.clientHeight - scroller.scrollTop < 40;
        if (!atEnd && commentsRef.current.length > 0 && additions) setNewBelow(current => current + additions);
        setComments(next); setError(null); countRef.current(todoId, next.length);
        model.updateDrafts(current => {
          const edits = { ...current.comments };
          for (const comment of next) {
            const key = `${todoId}/${comment.comment_id}`;
            if (edits[key]) edits[key] = reconcileDraft(edits[key], comment.body, comment.revision);
          }
          return { ...current, comments: edits };
        });
        if (firstRead.current) {
          firstRead.current = false;
          const savedPosition = threadPositions.get(threadKey);
          requestAnimationFrame(() => { if (thread.current) thread.current.scrollTop = savedPosition ?? thread.current.scrollHeight; });
        } else if (atEnd && additions) requestAnimationFrame(() => { if (thread.current) thread.current.scrollTop = thread.current.scrollHeight; });
      } catch (failure) { if (active) setError(notesError(failure).message); }
      finally { reading = false; if (active) setLoading(false); }
    };
    refreshRef.current = load;
    void load();
    const poll = () => { if (document.visibilityState !== "hidden") void load(); };
    const timer = window.setInterval(poll, 3000);
    window.addEventListener("focus", poll);
    return () => { active = false; window.clearInterval(timer); window.removeEventListener("focus", poll); };
  }, [model.request, model.updateDrafts, todoId, threadKey]);
  const focusCommentAction = (id: string | null, action: "edit" | "delete" | "editor", originId = id) => requestAnimationFrame(() => {
    if (!thread.current?.isConnected || !document.hasFocus() || thread.current.closest("[inert]") || document.querySelector('[role="dialog"][aria-modal="true"]')) return;
    const focused = document.activeElement;
    if (focused instanceof HTMLElement && focused !== document.body) {
      if (!focused.closest(".notes-comment-edit, .notes-delete-confirm, .notes-retained-comment") || focused.closest<HTMLElement>("[data-comment-id]")?.dataset.commentId !== originId) return;
    }
    for (const record of thread.current.querySelectorAll<HTMLElement>("[data-comment-id]")) {
      if (record.dataset.commentId === id) { record.querySelector<HTMLElement>(commentFocusSelectors[action])?.focus({ preventScroll: true }); return; }
    }
    thread.current.closest(".notes-task-detail")?.querySelector<HTMLElement>(".notes-comment-composer .cm-content")?.focus({ preventScroll: true });
  });
  const cancelEdit = () => { const id = editingRef.current; setEditing(null); if (id) focusCommentAction(id, "edit"); };
  const cancelDeletion = () => { const id = deletingRef.current?.id; setDeleting(null); if (id) focusCommentAction(id, "delete"); };
  const removeComment = async (comment: NotesComment) => {
    const index = comments.findIndex(record => record.comment_id === comment.comment_id);
    const nextId = comments[index + 1]?.comment_id ?? comments[index - 1]?.comment_id ?? null;
    const success = await model.mutate({ op: "comment_remove", todo_id: todoId, comment_id: comment.comment_id, expected_revision: comment.revision });
    await refreshRef.current();
    if (success && deletingRef.current?.id === comment.comment_id) { setDeleting(null); focusCommentAction(nextId, "edit", comment.comment_id); }
  };
  const post = async () => {
    if (!todo || !composer.body.trim() || unknown) return;
    const submitted = { ...composer };
    const success = await model.mutate({ op: "comment_add", todo_id: todoId, body: submitted.body, author: submitted.author.trim() || null }, () => {
      model.updateDrafts(current => {
        const currentDraft = current.commentNew[todoId];
        return currentDraft?.body === submitted.body ? { ...current, commentNew: { ...current.commentNew, [todoId]: { ...currentDraft, body: "" } } } : current;
      });
    });
    if (success) { await refreshRef.current(); requestAnimationFrame(() => {
      if (!thread.current?.isConnected || !document.hasFocus() || thread.current.closest("[inert]") || document.querySelector('[role="dialog"][aria-modal="true"]')) return;
      const focused = document.activeElement;
      if (focused instanceof HTMLElement && focused !== document.body && !focused.closest(".notes-comment-composer")) return;
      thread.current.scrollTop = thread.current.scrollHeight;
      thread.current.closest(".notes-task-detail")?.querySelector<HTMLElement>(".notes-comment-composer .cm-content")?.focus();
    }); }
  };
  const save = async (comment: NotesComment, keepMine = false) => {
    const key = `${todoId}/${comment.comment_id}`;
    const draft = model.drafts.comments[key];
    if (!draft || unknown || model.busy || draft.value === draft.base || (!keepMine && draft.conflict)) return;
    const submitted = draft.value;
    await model.mutate({ op: "comment_update", todo_id: todoId, comment_id: comment.comment_id, expected_revision: keepMine ? comment.revision : draft.revision, body: submitted }, response => {
      if (response.result.kind !== "comment") return;
      const saved = response.result.comment;
      let retainedTyping = false;
      model.updateDrafts(current => {
        const edits = { ...current.comments };
        if (edits[key]) {
          const retained = acknowledgeDraft(edits[key], saved.body, saved.revision, submitted);
          retainedTyping = retained.value !== retained.base;
          if (retainedTyping) edits[key] = retained; else delete edits[key];
        }
        return { ...current, comments: edits };
      });
      if (!retainedTyping && editingRef.current === comment.comment_id) {
        setEditing(null);
        focusCommentAction(comment.comment_id, "edit");
      }
    });
    await refreshRef.current();
  };
  const postRetained = async (key: string, submitted: string) => {
    if (!todo || !submitted.trim() || unknown) return;
    const success = await model.mutate({ op: "comment_add", todo_id: todoId, body: submitted, author: null }, response => {
      if (response.result.kind !== "comment") return;
      const saved = response.result.comment;
      const originId = key.slice(todoId.length + 1);
      const focused = document.activeElement;
      const ownsFocus = focused instanceof HTMLElement && focused.closest<HTMLElement>("[data-comment-id]")?.dataset.commentId === originId;
      let retainedTyping = false;
      model.updateDrafts(current => {
        const previous = current.comments[key];
        if (!previous) return current;
        const edits = { ...current.comments };
        delete edits[key];
        const retained = acknowledgeDraft(previous, saved.body, saved.revision, submitted);
        retainedTyping = retained.value !== retained.base;
        if (retainedTyping) edits[`${todoId}/${saved.comment_id}`] = retained;
        return { ...current, comments: edits };
      });
      if (retainedTyping && ownsFocus) { setEditing(saved.comment_id); focusCommentAction(saved.comment_id, "editor", originId); }
    });
    if (success) await refreshRef.current();
  };
  const removedEdits = loading || error ? [] : Object.entries(model.drafts.comments).filter(([key, draft]) => key.startsWith(`${todoId}/`) && draft.value !== draft.base && !comments.some(comment => key === `${todoId}/${comment.comment_id}`));
  return <aside className="notes-task-detail" aria-labelledby="notes-task-detail-title" onKeyDown={event => {
    if (event.nativeEvent.isComposing || event.repeat) return;
    if ((event.ctrlKey || event.metaKey) && event.key === "Enter" && event.target instanceof HTMLElement) {
      const inEdit = event.target.closest(".notes-comment-edit");
      const inComposer = event.target.closest(".notes-comment-composer");
      if (!inEdit && !inComposer) return;
      event.preventDefault(); event.stopPropagation();
      const edit = comments.find(comment => comment.comment_id === editing);
      if (edit && inEdit) void save(edit); else if (inComposer) void post();
    }
    if (event.key === "Escape" && !event.defaultPrevented) {
      event.preventDefault(); event.stopPropagation();
      if (deleting) cancelDeletion(); else if (editing) cancelEdit(); else onClose();
    }
  }}>
    <header className="notes-task-detail-header"><button type="button" className="notes-task-back" onClick={onClose}><UiIcon name="back" />Back to tasks</button><h3 id="notes-task-detail-title" ref={heading} tabIndex={-1}>Task details</h3><button type="button" className="notes-icon-button notes-task-close" onClick={onClose} aria-label="Close task details" title="Close task details"><UiIcon name="close" /></button></header>
    {todo ? <div className="notes-task-summary"><TodoTitle model={model} todo={todo} className="notes-task-title" />
      <div className="notes-task-state"><label className="notes-task-completed"><input type="checkbox" checked={todo.done} disabled={model.busy || unknown} onChange={() => void model.mutate({ op: "todo_set_done", todo: todoSelector(todo), done: !todo.done })} /> Completed</label>
      </div>
      <div className="notes-task-board-actions">{todo.lane ? <><label className="notes-task-lane">Move <select aria-label={`Move ${todo.text}`} value={todo.done ? "done" : todo.lane} disabled={model.busy || unknown} onChange={event => { const to = event.target.value; if (to === "backlog" || to === "doing" || to === "done") void model.mutate({ op: "kanban_move", todo: todoSelector(todo), to }); }}><option value="backlog">Backlog</option><option value="doing">Doing</option><option value="done">Done</option></select></label><button type="button" className="notes-task-unboard" disabled={model.busy || unknown} onClick={() => void model.mutate({ op: "kanban_unboard", todo: todoSelector(todo) })}>Remove from board</button><small className="notes-task-board-hint">Keeps the task and its comments.</small></> : <><span className="notes-task-lane is-unboarded">Not on the board</span><button type="button" disabled={model.busy || unknown} onClick={() => void model.mutate({ op: "kanban_promote", todo: todoSelector(todo) })}><UiIcon name="plus" />Add to board</button></>}</div>
    </div> : <p className="notes-task-removed"><UiIcon name="info" /><span>This task was removed from todos.md. Its comments and your drafts are kept.</span></p>}
    <h4 className="notes-thread-heading"><UiIcon name="comment" /><span>Comments</span> <span className="notes-count">{loading || error ? "" : comments.length}</span></h4>
    <div className={`notes-error-row${error ? " has-error" : ""}`}>{error ? <p role="alert"><UiIcon name="info" /><span>Comments unavailable: {error}</span><button type="button" onClick={() => void refreshRef.current()}>Retry reading</button></p> : null}</div>
    <div className="notes-comment-thread" ref={thread} aria-busy={loading} onScroll={event => {
      threadPositions.set(threadKey, event.currentTarget.scrollTop);
      if (threadPositions.size > 256) { const oldest = threadPositions.keys().next().value; if (oldest !== undefined) threadPositions.delete(oldest); }
      if (event.currentTarget.scrollHeight - event.currentTarget.clientHeight - event.currentTarget.scrollTop < 40) setNewBelow(0);
    }}>
      {loading ? <p className="notes-thread-loading">Loading comments…</p> : !error && !comments.length ? <div className="notes-empty notes-comment-empty"><span className="notes-empty-icon"><UiIcon name="comment" /></span><p className="notes-empty-title">No comments yet.</p><p className="notes-empty-hint">{todo ? "Add the first comment below to keep the discussion with this task." : "This task was removed. Existing drafts are kept."}</p></div> : null}
      {comments.map(comment => {
        const key = `${todoId}/${comment.comment_id}`;
        const draft = model.drafts.comments[key];
        const dirty = draft && draft.value !== draft.base;
        const date = comment.created ? new Date(comment.created) : null;
        const validDate = date && !Number.isNaN(date.getTime());
        return <article className="notes-comment" key={comment.comment_id} data-comment-id={comment.comment_id}>
          <header className="notes-comment-header"><span className="notes-comment-author" title="An explicit label, not verified identity">{comment.author ?? "Unattributed"}</span>{comment.author ? <span className="notes-comment-author-note">· unverified label</span> : null}{validDate ? <time className="notes-comment-time" dateTime={comment.created ?? undefined}>{date.toLocaleString()}</time> : <span className="notes-comment-time">Time not recorded</span>}
            {editing === comment.comment_id && draft ? null : <div className="notes-row-actions notes-comment-actions"><button type="button" className="notes-icon-button" aria-label={dirty ? "Resume edit · draft kept" : "Edit"} title={dirty ? "Resume edit · draft kept" : "Edit"} onClick={() => { model.updateDrafts(current => ({ ...current, comments: { ...current.comments, [key]: current.comments[key] ?? changedDraft(comment.body, comment.body, comment.revision) } })); setEditing(comment.comment_id); }}><UiIcon name="edit" /></button>{dirty ? <button type="button" className="notes-comment-discard" onClick={() => model.updateDrafts(current => { const edits = { ...current.comments }; delete edits[key]; return { ...current, comments: edits }; })}>Discard edit</button> : null}<button type="button" className="notes-icon-button notes-comment-delete" aria-label="Delete" title="Delete" onClick={() => setDeleting({ id: comment.comment_id, revision: comment.revision })}><UiIcon name="trash" /></button></div>}
          </header>
          {editing === comment.comment_id && draft ? <div className="notes-comment-edit"><MarkdownEditor label={`Edit comment ${comment.comment_id}`} draftKey={key} value={draft.value} onChange={value => model.updateDrafts(current => ({ ...current, comments: { ...current.comments, [key]: { ...(current.comments[key] ?? draft), value } } }))} onSave={() => void save(comment)} />
            {draft.conflict ? <div className="notes-conflict" role="alert"><p><UiIcon name="info" /><span>This comment changed elsewhere. Current saved version:</span></p><MarkdownPreview content={comment.body} /><button type="button" disabled={model.busy || unknown} onClick={() => void save(comment, true)}>Keep mine and save</button><button type="button" onClick={() => model.updateDrafts(current => ({ ...current, comments: { ...current.comments, [key]: changedDraft(comment.body, comment.body, comment.revision) } }))}>Reload (discard my edit)</button></div> : <button type="button" className="notes-primary" disabled={model.busy || unknown || !dirty} onClick={() => void save(comment)}><UiIcon name="check" />Save comment</button>}
            <button type="button" onClick={cancelEdit}>Cancel / keep edit</button></div> : <><MarkdownPreview content={comment.body} />{dirty ? <small className="notes-comment-draft">Draft kept</small> : null}</>}
          <details className="notes-comment-reference"><summary><UiIcon name="info" />Record details</summary><span>Author label is not authenticated.</span><span>Created: {comment.created ?? "unknown"}</span><code>{`comments/${todoId}/${comment.comment_id}.md`}</code></details>
          {deleting?.id === comment.comment_id ? <div className="notes-delete-confirm" role="group" aria-label="Delete this comment?"><p>{deleting.revision !== comment.revision ? "Changed elsewhere. Delete the current saved version shown here?" : "Delete this comment? If it changes, deletion will be refused."}</p>{deleting.revision !== comment.revision ? <MarkdownPreview content={comment.body} /> : null}<button type="button" autoFocus onClick={cancelDeletion}>Keep</button><button type="button" disabled={model.busy || unknown} onClick={() => void removeComment(comment)}>{deleting.revision !== comment.revision ? "Delete this version" : "Delete comment"}</button></div> : null}
        </article>;
      })}
      {removedEdits.map(([key, draft]) => <article className="notes-comment notes-retained-comment" key={key} data-comment-id={key.slice(todoId.length + 1)}><p>Deleted elsewhere. Your edit is kept.</p><MarkdownEditor label="Retained deleted comment edit" draftKey={key} value={draft.value} onChange={value => model.updateDrafts(current => ({ ...current, comments: { ...current.comments, [key]: { ...(current.comments[key] ?? draft), value } } }))} /><button type="button" disabled={!todo || model.busy || unknown || !draft.value.trim()} onClick={() => void postRetained(key, draft.value)}>Post as new comment</button><button type="button" onClick={() => model.updateDrafts(current => { const edits = { ...current.comments }; delete edits[key]; return { ...current, comments: edits }; })}>Discard</button></article>)}
    </div>
    {newBelow ? <button type="button" className="notes-new-comments" onClick={() => { if (thread.current) thread.current.scrollTop = thread.current.scrollHeight; setNewBelow(0); }}>{newBelow} new comments below</button> : null}
    <div className="notes-comment-composer">
      <MarkdownEditor label="New comment Markdown" draftKey={`new-comment:${todoId}`} value={composer.body} onChange={body => model.updateDrafts(current => ({ ...current, commentNew: { ...current.commentNew, [todoId]: { ...(current.commentNew[todoId] ?? composer), body } } }))} onSave={() => void post()} hint="Add a comment…" />
      <div className="notes-row-actions notes-composer-footer">
        <label className="notes-composer-author" title="Author label (optional, unverified)"><span>author</span><span className="notes-composer-author-note">unverified</span><input aria-label="Comment author label" title="Author label (optional, unverified)" placeholder="optional · unverified" value={composer.author} maxLength={128} onChange={event => model.updateDrafts(current => ({ ...current, commentNew: { ...current.commentNew, [todoId]: { ...(current.commentNew[todoId] ?? composer), author: event.target.value } } }))} /></label>
        {composer.body ? <><small>Draft kept</small><button type="button" onClick={() => model.updateDrafts(current => ({ ...current, commentNew: { ...current.commentNew, [todoId]: { ...(current.commentNew[todoId] ?? composer), body: "" } } }))}>Discard</button></> : null}
        <button type="button" className="notes-primary" id="postCommentBtn" disabled={!todo || !composer.body.trim() || model.busy || unknown} onClick={() => void post()}><UiIcon name="comment" />{model.busy ? "Saving…" : "Comment"}</button>
      </div>
    </div>
  </aside>;
}
