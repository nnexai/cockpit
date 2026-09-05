import type { CommentReviewRef, ExtensionKind } from "../../protocol/generated/v1";
import { CommentPasteControls } from "./CommentPasteControls";
import { useCallback, useEffect, useMemo, useRef, useState, type ReactNode } from "react";
import type { CockpitClient } from "../../client/CockpitClient";
import type {
  CommentAnchor,
  CommentBatch,
  CommentBatchList,
  CommentCapture,
  CommentDraft,
  CommentPreview,
  CommentRequestScope,
  CommentRemoveRequest,
  CommentUpsertRequest,
  ContextDocument,
  ContextRoot,
  PanePresentation,
} from "../../protocol/generated/v1";
import type { ContextCommentEditorState } from "./ContextViewer";
import "./comments.css";

const windowClientId = (() => {
  try {
    if (typeof crypto !== "undefined" && typeof crypto.randomUUID === "function") return crypto.randomUUID();
  } catch { /* old webviews may not expose crypto */ }
  return `cockpit-${Date.now().toString(36)}-${Math.random().toString(36).slice(2)}`;
})();
type CommentSelection = { start: number; end: number } | null;
type NewDraftTarget = { review?: CommentReviewRef; rootId: string; path: string; revision: string; selection: CommentSelection };
export type CommentDraftActions = {
  edit: (draft: CommentDraft) => void;
  remove: (draft: CommentDraft) => void;
};

type CommentDraftsProps = {
  client: CockpitClient;
  presentation: PanePresentation;
  root: ContextRoot;
  path: string;
  document: ContextDocument | null;
  selection: CommentSelection;
  mode: "source" | "markdown";
  editorState: ContextCommentEditorState | null;
  onEditorStateChange: (state: ContextCommentEditorState | null) => void;
  children?: (drafts: CommentDraft[], actions: CommentDraftActions) => ReactNode;
  onCountChange?: (count: number) => void;
  invalidationGeneration?: number;
  sourceIdentity?: string;
  sourceKind?: ExtensionKind;
  reviewCapture?: CommentReviewRef;
};

function errorText(error: unknown): string {
  if (error instanceof Error && error.message) return error.message;
  if (typeof error === "string" && error) return error;
  if (typeof error === "object" && error !== null && "message" in error && typeof error.message === "string") return error.message;
  return "The comments request could not be completed.";
}

function draftAnchorLabel(anchor: CommentAnchor): string {
  return anchor.kind === "whole_file" ? "Whole file" : `Lines ${anchor.start_line}–${anchor.end_line}`;
}

function sourceLabel(draft: CommentDraft): string {
  return draft.file_ref.absolute_path || draft.file_ref.path;
}
function stale(draft: CommentDraft): boolean {
  return draft.source_state !== "current";
}

export function InlineCommentDrafts({ drafts, line, actions, rootId, path }: { drafts: CommentDraft[]; line: number; actions: CommentDraftActions; rootId?: string; path?: string }) {
  const matches = drafts.filter((draft) => draft.source_state === "current" && (rootId === undefined || draft.file_ref.root_id === rootId) && (path === undefined || draft.file_ref.path === path) && draft.anchor.kind === "lines" && line === draft.anchor.start_line);
  if (matches.length === 0) return null;
  return <div className="comment-inline-drafts" aria-label={`Comments on line ${line}`}>{matches.map((draft) => <article className={`comment-draft comment-inline-draft${stale(draft) ? " is-stale" : ""}`} key={draft.draft_id}>
    <div className="comment-draft-meta"><strong>{draftAnchorLabel(draft.anchor)}</strong><span>{draft.source_state}</span></div>
    <p>{draft.comment_text || <em>Empty comment</em>}</p>
    <div className="comment-draft-actions"><button type="button" onClick={() => actions.edit(draft)}>Edit</button><button type="button" onClick={() => actions.remove(draft)}>Delete</button></div>
  </article>)}</div>;
}

export function CommentDrafts({ client, presentation, root, path, document, selection, mode, editorState, onEditorStateChange, children, onCountChange, invalidationGeneration = 0, sourceIdentity, sourceKind = "context", reviewCapture }: CommentDraftsProps) {
  const currentSourceId = sourceIdentity ?? root.companion_id;
  const scope = useMemo<CommentRequestScope>(() => ({ binding_id: presentation.binding_id, client_id: windowClientId }), [presentation.binding_id]);
  const [batch, setBatch] = useState<CommentBatch | null>(null);
  const [batchList, setBatchList] = useState<CommentBatchList | null>(null);
  const [newDraftTarget, setNewDraftTarget] = useState<NewDraftTarget | null>(null);
  const [loading, setLoading] = useState(true);
  const [overviewOpen, setOverviewOpen] = useState(false);
  const [editor, setEditor] = useState<"whole_file" | "lines" | null>(null);
  const [editing, setEditing] = useState<CommentDraft | null>(null);
  const [text, setText] = useState("");
  const [error, setError] = useState<string | null>(null);
  const identity = `${presentation.session_id}\u0000${presentation.pane_id}\u0000${presentation.binding_id}\u0000${root.root_id}\u0000${currentSourceId ?? ""}`;
  const [pending, setPending] = useState(false);
  const [preview, setPreview] = useState<CommentPreview | null>(null);
  const [retainedStale, setRetainedStale] = useState(false);
  const generationRef = useRef(0);
  const refreshedInvalidation = useRef(invalidationGeneration);
  const identityRef = useRef(identity);
  identityRef.current = identity;

  const clearEditor = useCallback(() => {
    setEditor(null);
    setEditing(null);
    setNewDraftTarget(null);
    setText("");
    onEditorStateChange(null);
  }, [onEditorStateChange]);

  const loadBatch = useCallback(async (batchId: string | null = null) => {
    const generation = ++generationRef.current;
    setLoading(true);
    setError(null);
    try {
      const next = await client.commentBatch(presentation.session_id, presentation.pane_id, { scope, batch_id: batchId });
      if (generation !== generationRef.current || identityRef.current !== identity) return;
      setBatch(next);
      setPreview(null);
      onCountChange?.(next.drafts.length);
    } catch (reason) {
      if (generation === generationRef.current && identityRef.current === identity) setError(errorText(reason));
    } finally {
      if (generation === generationRef.current && identityRef.current === identity) setLoading(false);
    }
  }, [client, identity, onCountChange, presentation.pane_id, presentation.session_id, scope]);

  useEffect(() => {
    if (pending || loading || refreshedInvalidation.current === invalidationGeneration) return;
    refreshedInvalidation.current = invalidationGeneration;
    void loadBatch(batch?.batch_id ?? null);
  }, [batch?.batch_id, invalidationGeneration, loadBatch, loading, pending]);

  const switchBatch = useCallback((batchId: string) => {
    if (batch?.batch_id !== batchId) clearEditor();
    void loadBatch(batchId);
  }, [batch?.batch_id, clearEditor, loadBatch]);

  useEffect(() => {
    setBatch(null);
    setPending(false);
    setError(null);
    setPreview(null);
    setRetainedStale(false);
    setBatchList(null);
    setOverviewOpen(false);
    void loadBatch();
    return () => { generationRef.current += 1; };
  }, [identity, loadBatch]);

  useEffect(() => {
    const matchesCurrentSource = editorState !== null && editorState.rootId === root.root_id;
    if (!matchesCurrentSource || editorState === null) {
      setEditor(null);
      setEditing(null);
      setNewDraftTarget(null);
      setText("");
      return;
    }
    setEditor(editorState.editor);
    setText(editorState.text);
    setNewDraftTarget(editorState.draftId === null ? {
      review: editorState.review,
      rootId: editorState.rootId,
      path: editorState.path,
      revision: editorState.revision,
      selection: editorState.selection,
    } : null);
    setEditing(editorState.draftId === null ? null : batch?.drafts.find((draft) => draft.draft_id === editorState.draftId) ?? null);
  }, [batch, editorState, root.root_id]);

  const removeDraft = useCallback(async (draft: CommentDraft) => {
    if (!batch || pending || !window.confirm(`Delete comment on ${draft.file_ref.path}?`)) return;
    const generation = ++generationRef.current;
    setPending(true);
    setError(null);
    try {
      const request: CommentRemoveRequest = { batch: { scope, batch_id: batch.batch_id, expected_generation: batch.generation }, draft_id: draft.draft_id };
      const next = await client.commentRemove(presentation.session_id, presentation.pane_id, request);
      if (generation !== generationRef.current || identityRef.current !== identity) return;
      setBatch(next);
      onCountChange?.(next.drafts.length);
      if (editing?.draft_id === draft.draft_id) clearEditor();
      setPreview(null);
    } catch (reason) {
      if (generation === generationRef.current && identityRef.current === identity) setError(errorText(reason));
    } finally {
      if (generation === generationRef.current && identityRef.current === identity) setPending(false);
    }
  }, [batch, clearEditor, client, editing?.draft_id, identity, onCountChange, pending, presentation.pane_id, presentation.session_id, scope]);

  const actions = useMemo<CommentDraftActions>(() => ({
    edit: (draft) => {
      const nextEditorState: ContextCommentEditorState = {
        review: draft.file_ref.review,
        rootId: draft.file_ref.root_id,
        path: draft.file_ref.path,
        revision: draft.file_ref.revision,
        draftId: draft.draft_id,
        editor: draft.anchor.kind,
        text: draft.comment_text,
        selection: draft.anchor.kind === "lines" ? { start: draft.anchor.start_line, end: draft.anchor.end_line } : null,
      };
      setEditing(draft);
      setNewDraftTarget(null);
      setText(draft.comment_text);
      setEditor(draft.anchor.kind);
      onEditorStateChange(nextEditorState);
      setOverviewOpen(false);
      setError(null);
    },
    remove: (draft) => { void removeDraft(draft); },
  }), [onEditorStateChange, removeDraft]);

  const selectedRange = selection && document?.text !== null && document?.text !== undefined
    ? { start: Math.min(selection.start, selection.end), end: Math.max(selection.start, selection.end) }
    : null;

  const beginNew = (kind: "whole_file" | "lines") => {
    if (!document || document.text === null || !currentSourceId || !path || (kind === "lines" && !selectedRange)) return;
    const nextEditorState: ContextCommentEditorState = {
      review: reviewCapture,
      rootId: root.root_id,
      path,
      revision: document.revision,
      draftId: null,
      editor: kind,
      text: "",
      selection: selectedRange,
    };
    setEditing(null);
    setNewDraftTarget({ review: reviewCapture, rootId: root.root_id, path, revision: document.revision, selection: selectedRange });
    setText("");
    setEditor(kind);
    onEditorStateChange(nextEditorState);
    setOverviewOpen(false);
    setError(null);
  };

  const staleReviewEditor = Boolean(editorState && editorState.draftId === null && editorState.review && (
    editorState.review.review_id !== reviewCapture?.review_id || editorState.review.generation !== reviewCapture?.generation ||
    editorState.review.file_id !== reviewCapture?.file_id || editorState.review.side !== reviewCapture?.side));
  const recaptureEditor = () => {
    if (!editorState || !document || document.text === null || document.truncated || !reviewCapture) return;
    if (editorState.editor === "lines" && !selection) return;
    onEditorStateChange({ ...editorState, review: reviewCapture, rootId: root.root_id, path,
      revision: document.revision, selection: editorState.editor === "lines" ? selection : null });
  };

  const saveDraft = async () => {
    if (!batch || !editor || pending || staleReviewEditor) return;
    if (!editing && (!newDraftTarget || !currentSourceId)) return;
    if (!text.trim()) { setError("Enter a comment before saving."); return; }
    if (!editing && editor === "lines" && !newDraftTarget?.selection) { setError("Select a contiguous source range first."); return; }
    const capture: CommentCapture | null = editing ? null : { ...(newDraftTarget!.review ? { review: newDraftTarget!.review } : {}), root_id: newDraftTarget!.rootId, path: newDraftTarget!.path, expected_revision: newDraftTarget!.revision, start_line: editor === "lines" ? newDraftTarget!.selection!.start : null, end_line: editor === "lines" ? newDraftTarget!.selection!.end : null };
    const generation = ++generationRef.current;
    setPending(true); setError(null);
    try {
      const request: CommentUpsertRequest = { batch: { scope, batch_id: batch.batch_id, expected_generation: batch.generation }, draft_id: editing?.draft_id ?? null, capture, comment_text: text };
      const next = await client.commentUpsert(presentation.session_id, presentation.pane_id, request);
      if (generation !== generationRef.current || identityRef.current !== identity) return;
      setBatch(next);
      onCountChange?.(next.drafts.length);
      clearEditor();
      setPreview(null);
    } catch (reason) {
      if (generation === generationRef.current && identityRef.current === identity) setError(errorText(reason));
    } finally {
      if (generation === generationRef.current && identityRef.current === identity) setPending(false);
    }
  };

  const openOverview = async () => {
    if (pending || loading) return;
    setOverviewOpen(true); setError(null);
    const generation = ++generationRef.current;
    try {
      const list = await client.commentBatches(presentation.session_id, presentation.pane_id, scope);
      if (generation === generationRef.current && identityRef.current === identity) setBatchList(list);
    } catch (reason) {
      if (generation === generationRef.current && identityRef.current === identity) setError(errorText(reason));
    }
  };

  const attach = async () => {
    if (!batch || pending || !currentSourceId) return;
    const generation = ++generationRef.current;
    setPending(true); setError(null);
    try {
      const next = await client.commentAttach(presentation.session_id, presentation.pane_id, { scope, batch_id: batch.batch_id, expected_generation: batch.generation });
      if (generation === generationRef.current && identityRef.current === identity) { setBatch(next); setPreview(null); }
    } catch (reason) {
      if (generation === generationRef.current && identityRef.current === identity) setError(errorText(reason));
    } finally {
      if (generation === generationRef.current && identityRef.current === identity) setPending(false);
    }
  };

  const makePreview = async (retain: boolean) => {
    if (!batch || pending) return;
    const generation = ++generationRef.current;
    setPending(true); setError(null);
    try {
      const request = { batch: { scope, batch_id: batch.batch_id, expected_generation: batch.generation }, retain_stale_excerpts: retain };
      const next = await client.commentPreview(presentation.session_id, presentation.pane_id, request);
      if (generation === generationRef.current && identityRef.current === identity) {
        setPreview(next);
        setRetainedStale(retain);
        if (next.stale_draft_ids.length > 0) {
          setBatch((current) => current ? {
            ...current,
            drafts: current.drafts.map((draft) => next.stale_draft_ids.includes(draft.draft_id) && draft.source_state === "current" ? { ...draft, source_state: "changed" } : draft),
          } : current);
        }
      }
    } catch (reason) {
      if (generation === generationRef.current && identityRef.current === identity) setError(errorText(reason));
    } finally {
      if (generation === generationRef.current && identityRef.current === identity) setPending(false);
    }
  };

  const currentDrafts = batch?.drafts ?? [];
  const fileDrafts = currentDrafts.filter((draft) => draft.file_ref.root_id === root.root_id && draft.file_ref.path === path && (sourceKind !== "review" || (draft.file_ref.review?.file_id === reviewCapture?.file_id && draft.file_ref.review?.side === reviewCapture?.side)));
  const fileBottomDrafts = fileDrafts.filter((draft) => mode === "markdown" || draft.anchor.kind === "whole_file" || stale(draft));
  const detached = batch?.live_attachment === null;
  const sameSource = Boolean(batch && batch.owner.source_kind === sourceKind && batch.owner.source_id === currentSourceId);
  const content = children ? children(currentDrafts, actions) : null;
  const editorSelection = editing?.anchor.kind === "lines"
    ? { start: editing.anchor.start_line, end: editing.anchor.end_line }
    : newDraftTarget?.selection;
  const editorPath = editing?.file_ref.path ?? newDraftTarget?.path;
  const removedEditorDraft = !loading && editorState?.draftId != null
    && batch !== null && !batch.drafts.some((draft) => draft.draft_id === editorState.draftId);
  const recreateFromCurrentSource = () => {
    if (!document || document.text === null || !path || !editor || (editor === "lines" && !selectedRange)) return;
    const next: ContextCommentEditorState = {
      review: reviewCapture, rootId: root.root_id, path, revision: document.revision, draftId: null,
      editor, text, selection: editor === "lines" ? selectedRange : null,
    };
    setEditing(null);
    setNewDraftTarget({ review: next.review, rootId: next.rootId, path, revision: next.revision, selection: next.selection });
    onEditorStateChange(next);
    setError(null);
  };

  return <>
    <div className="comment-toolbar" aria-label="Reference comments">
      <button type="button" className="comment-count" onClick={() => void openOverview()} disabled={pending || loading} aria-label={`${currentDrafts.length} comments, open overview`}>{currentDrafts.length} comments</button>
      <button type="button" onClick={() => beginNew("whole_file")} disabled={!batch || loading || !currentSourceId || document?.text === null || document?.text === undefined || !path || pending}>Comment whole file</button>
      <button type="button" onClick={() => beginNew("lines")} disabled={!batch || loading || !selectedRange || !currentSourceId || pending}>Comment selected lines</button>
      {detached ? <span className="comment-detached" role="status">Detached recovery</span> : null}
    </div>
    {error ? <div className="comment-notice comment-notice-error" role="alert"><strong>Comments not saved</strong><span>{error}</span><button type="button" onClick={() => void loadBatch(batch?.batch_id ?? null)}>Reload</button></div> : null}
    {loading ? <div className="comment-notice" role="status">Loading comments…</div> : null}
    {content}
    {removedEditorDraft ? <div className="comment-notice" role="status"><span>This comment was deleted in another window. Your unsaved text is retained. Select a file{editor === "lines" ? " and source lines" : ""} to create a new comment.</span><button type="button" onClick={recreateFromCurrentSource} disabled={!document || document.text === null || !path || (editor === "lines" && !selectedRange)}>Use current source</button></div> : null}
    {fileBottomDrafts.length > 0 ? <section className={`comment-file-drafts${mode === "markdown" ? " comment-markdown-drafts" : ""}`} aria-label={`Comments for ${path}`}><h3>{mode === "markdown" ? "File comments" : "File comments"}</h3>{fileBottomDrafts.map((draft) => <article className={`comment-draft${stale(draft) ? " is-stale" : ""}`} key={draft.draft_id}><div className="comment-draft-meta"><strong>{draftAnchorLabel(draft.anchor)}</strong><span>{draft.source_state}</span></div><p>{draft.comment_text}</p><div className="comment-draft-actions"><button type="button" onClick={() => actions.edit(draft)}>Edit</button><button type="button" onClick={() => actions.remove(draft)}>Delete</button></div></article>)}</section> : null}
    {editor ? <section className="comment-editor" aria-label={editing ? "Edit comment" : "New comment"}><h3>{editing ? `Edit ${draftAnchorLabel(editing.anchor).toLowerCase()}` : editor === "whole_file" ? "Comment on whole file" : editorSelection ? `Comment on lines ${editorSelection.start}–${editorSelection.end}` : "Comment on selected lines"}{editorPath && editorPath !== path ? ` · ${editorPath}` : ""}</h3>{staleReviewEditor ? <p role="status">The displayed review source changed. Your text is retained. Select the intended source lines again before using the current source.<button type="button" onClick={recaptureEditor} disabled={!document?.text || document.truncated || (editor === "lines" && !selection)}>Use current source</button></p> : null}<textarea value={text} onChange={(event) => { const nextText = event.target.value; setText(nextText); if (editorState) onEditorStateChange({ ...editorState, text: nextText }); }} rows={4} maxLength={8192} autoFocus aria-label="Comment text" placeholder="Describe what should be changed…" /><div className="comment-editor-actions"><button type="button" onClick={clearEditor}>Cancel</button><button type="button" onClick={() => void saveDraft()} disabled={pending || staleReviewEditor || removedEditorDraft || !text.trim()}>{pending ? "Saving…" : "Save comment"}</button></div></section> : null}
    {overviewOpen ? <div className="comment-overview-backdrop" role="presentation" onPointerDown={(event) => { if (event.target === event.currentTarget) setOverviewOpen(false); }}><section className="comment-overview" role="dialog" aria-modal="true" aria-labelledby="comment-overview-title"><header><h2 id="comment-overview-title">Comments</h2><button type="button" aria-label="Close comments overview" onClick={() => setOverviewOpen(false)}>Close</button></header>{batchList?.batches.length ? <div className="comment-recovery-list"><h3>Saved batches</h3>{batchList.batches.map((summary) => <button type="button" className="comment-recovery-row" key={summary.batch_id} onClick={() => { setOverviewOpen(false); switchBatch(summary.batch_id); }}><span>{summary.draft_count} comments</span><code>{summary.batch_id}</code><small>generation {summary.generation} · {summary.updated_at}</small></button>)}</div> : null}{batch && detached ? <div className="comment-recovery"><strong>Detached batch</strong><span>Reattach only after confirming this {sourceKind === "review" ? "Review" : "Context"} source.</span><button type="button" onClick={() => void attach()} disabled={pending || !sameSource}>Reattach to this source</button></div> : null}<div className="comment-overview-list"><h3>Current batch</h3>{currentDrafts.length === 0 ? <p>No comments yet.</p> : currentDrafts.map((draft) => <article className={`comment-draft${stale(draft) ? " is-stale" : ""}`} key={draft.draft_id}><div className="comment-draft-meta"><strong>{sourceLabel(draft)}</strong><span>{draftAnchorLabel(draft.anchor)} · {draft.source_state}</span></div><p>{draft.comment_text}</p><div className="comment-draft-actions"><button type="button" onClick={() => actions.edit(draft)}>Edit</button><button type="button" onClick={() => actions.remove(draft)}>Delete</button></div></article>)}</div><div className="comment-preview"><div className="comment-preview-heading"><h3>Preview</h3><div><button type="button" onClick={() => void makePreview(false)} disabled={pending}>Refresh preview</button><button type="button" onClick={() => void makePreview(true)} disabled={pending}>Include stale excerpts</button></div></div>{preview ? <><div className="comment-preview-meta"><span>{preview.payload_bytes} payload bytes</span><span>{preview.framed_bytes} framed bytes</span><span>{preview.sanitized_controls} controls sanitized</span><span>{preview.exportable ? "Exportable" : "Not exportable"}</span></div>{preview.reason ? <span className="comment-preview-reason">{preview.reason}</span> : null}<textarea readOnly value={preview.payload} rows={10} aria-label="Comment preview" />{preview.stale_draft_ids.length > 0 ? <span className="comment-preview-blocked">{preview.stale_draft_ids.length} stale comment{preview.stale_draft_ids.length === 1 ? "" : "s"} marked in the file.</span> : null}{preview.exportable && retainedStale ? <span className="comment-preview-ok">Stale excerpts retained explicitly.</span> : null}</> : <span className="comment-preview-meta">No preview generated.</span>}</div>{batch ? <CommentPasteControls client={client} sessionId={presentation.session_id} paneId={presentation.pane_id} scope={scope} batch={batch} retainStale={retainedStale} preview={preview} onAccepted={() => { setPreview(null); void loadBatch(batch.batch_id); }} /> : null}</section></div> : null}
  </>;
}
