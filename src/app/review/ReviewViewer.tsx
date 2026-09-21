import { Fragment, useCallback, useEffect, useRef, useState } from "react";
import type { CockpitClient } from "../../client/CockpitClient";
import type { ContextDocument, ContextRoot, PanePresentation, ReviewComparison, ReviewFileDiff, ReviewFileRequest, ReviewSnapshotRequest } from "../../protocol/generated/v1";
import { CommentDrafts, InlineCommentDrafts, type CommentDraftActions } from "../context/CommentDrafts";
import { createReviewViewState, retainReviewScrollPosition, SourceLines, type ContextViewState, type ReviewViewState } from "../context/ContextViewer";
import { ReviewPane, reviewScrollIdentity } from "./ReviewPane";

type Selection = { fileId: string; side: "old" | "new"; start: number; end: number } | null;
type CommentStatus = { count: number | null; canCreateLines: boolean; canCreateWholeFile: boolean };
type SourcePageLoad = { key: string; token: number };

/** CommentDrafts keys its batch by this source identity; comparison scopes its review load. */
export function reviewCommentBatchIdentity(presentation: Pick<PanePresentation, "session_id" | "pane_id" | "binding_id">, sourceId: string, comparison: ReviewComparison): string {
  const draftsIdentity = `${presentation.session_id}\u0000${presentation.pane_id}\u0000${presentation.binding_id}\u0000${sourceId}\u0000${sourceId}`;
  return `${draftsIdentity}\u0000${comparison}`;
}
export function reviewRepositoryId(presentation: Pick<PanePresentation, "roots" | "default_root_id">): string | undefined {
  return (presentation.roots.find(root => root.root_id === presentation.default_root_id && root.kind === "repository")
    ?? presentation.roots.find(root => root.kind === "repository"))?.repository_id;
}

export function ReviewViewer({ client, presentation, value, onChange, onTerminalView: _onTerminalView, onRequestControl }: {
  client: CockpitClient; presentation: PanePresentation; value: ContextViewState;
  onChange: (next: ContextViewState) => void; onTerminalView: () => void; onRequestControl: () => void;
}) {
  const reviewView = value.review ?? createReviewViewState();
  const [selection, setSelection] = useState<Selection>(() => reviewView.fileId && reviewView.side && reviewView.selectionStart !== null && reviewView.selectionEnd !== null
    ? { fileId: reviewView.fileId, side: reviewView.side, start: reviewView.selectionStart, end: reviewView.selectionEnd } : null);
  const viewerRef = useRef<HTMLDivElement>(null);
  const [mode, setMode] = useState<"diff" | "source">(reviewView.mode);
  const [invalidation, setInvalidation] = useState(0);
  const [commentStatus, setCommentStatus] = useState<CommentStatus>({ count: reviewView.commentCount, canCreateLines: false, canCreateWholeFile: false });
  const [sourcePages, setSourcePages] = useState<Record<string, { text: string; nextOffset: number; totalBytes: number }>>({});
  const [sourceLoading, setSourceLoading] = useState<string | null>(null);
  const sourceLoadRef = useRef<SourcePageLoad | null>(null);
  const sourceLoadSequence = useRef(0);
  const activeSourceKeyRef = useRef<string | null>(null);
  const activeSourceMetadataRef = useRef<{ reviewId: string; generation: number; fileId: string } | null>(null);
  const commentActionsRef = useRef<CommentDraftActions | null>(null);
  const activeCommentIdentityRef = useRef<string | null>(null);
  const activeComparisonRef = useRef<ReviewComparison>(reviewView.comparison);
  const changeRef = useRef(onChange); changeRef.current = onChange;
  const valueRef = useRef(value); valueRef.current = value;
  const reviewViewRef = useRef(reviewView); reviewViewRef.current = reviewView;
  const publishView = useCallback((next: ContextViewState) => {
    valueRef.current = next;
    changeRef.current(next);
  }, []);
  const updateReviewView = useCallback((patch: Partial<ReviewViewState>) => {
    const current = valueRef.current.review ?? createReviewViewState();
    publishView({ ...valueRef.current, review: { ...current, ...patch } });
  }, [publishView]);
  const appendSourcePage = useCallback((sourceKey: string, side: "old" | "new", requestedOffset: number, expectedRevision: string | null, token: number, next: ReviewFileDiff) => {
    const metadata = activeSourceMetadataRef.current;
    if (sourceLoadRef.current?.token !== token || activeSourceKeyRef.current !== sourceKey || !metadata
      || next.session_id !== presentation.session_id || next.pane_id !== presentation.pane_id || next.binding_id !== presentation.binding_id
      || next.review_id !== metadata.reviewId || next.generation !== metadata.generation || next.file.file_id !== metadata.fileId) return;
    const chunk = side === "old" ? next.old_source : next.new_source;
    const offset = side === "old" ? next.old_source_offset : next.new_source_offset;
    const total = side === "old" ? next.old_source_total_bytes : next.new_source_total_bytes;
    const revision = side === "old" ? next.file.old_revision : next.file.new_revision;
    if (chunk === null || offset !== requestedOffset || (expectedRevision !== null && revision !== expectedRevision)) return;
    const bytes = new TextEncoder().encode(chunk).byteLength;
    setSourcePages((current) => {
      if (requestedOffset > 0 && current[sourceKey]?.nextOffset !== requestedOffset) return current;
      return {
        ...current,
        [sourceKey]: {
          text: requestedOffset === 0 ? chunk : `${current[sourceKey]?.text ?? ""}${chunk}`,
          nextOffset: offset + bytes,
          totalBytes: total ?? offset + bytes,
        },
      };
    });
  }, [presentation.binding_id, presentation.pane_id, presentation.session_id]);
  const handleReviewViewChange = useCallback((next: ReviewViewState) => {
    const comparisonChanged = activeComparisonRef.current !== next.comparison;
    activeComparisonRef.current = next.comparison;
    if (comparisonChanged) {
      activeCommentIdentityRef.current = null;
      setCommentStatus((current) => current.count === null && !current.canCreateLines && !current.canCreateWholeFile
        ? current : { count: null, canCreateLines: false, canCreateWholeFile: false });
      next = { ...next, commentCount: null };
    }
    publishView({ ...valueRef.current, review: next });
  }, [publishView]);
  useEffect(() => {
    sourceLoadSequence.current += 1;
    sourceLoadRef.current = null;
    setSourcePages({});
    setSourceLoading(null);
  }, [invalidation]);
  useEffect(() => {
    updateReviewView({
      fileId: selection?.fileId ?? reviewViewRef.current.fileId,
      side: selection?.side ?? reviewViewRef.current.side,
      selectionStart: selection?.start ?? reviewViewRef.current.selectionStart,
      selectionEnd: selection?.end ?? reviewViewRef.current.selectionEnd,
    });
  }, [selection, updateReviewView]);
  const editorChange = useCallback((commentEditor: ContextViewState["commentEditor"]) => publishView({ ...valueRef.current, commentEditor }), [publishView]);
  const updateCommentStatus = useCallback((identity: string, comparison: ReviewComparison, next: CommentStatus) => {
    if (comparison !== activeComparisonRef.current) return;
    if (identity !== activeCommentIdentityRef.current && activeCommentIdentityRef.current !== null) {
      activeCommentIdentityRef.current = identity;
      setCommentStatus({ count: null, canCreateLines: false, canCreateWholeFile: false });
      updateReviewView({ commentCount: null });
    } else if (activeCommentIdentityRef.current === null) {
      activeCommentIdentityRef.current = identity;
    }
    const commentsLoading = viewerRef.current?.querySelector<HTMLElement>('.comment-notice[role="status"]')?.textContent === "Loading comments…";
    if (commentsLoading) return;
    setCommentStatus((current) => current.count === next.count && current.canCreateLines === next.canCreateLines && current.canCreateWholeFile === next.canCreateWholeFile ? current : next);
    if (reviewViewRef.current.commentCount !== next.count) updateReviewView({ commentCount: next.count });
  }, [updateReviewView]);
  const restoreDiffFocus = useCallback(() => requestAnimationFrame(() => viewerRef.current?.querySelector<HTMLElement>(".review-diff")?.focus({ preventScroll: true })), []);
  const { session_id: session, pane_id: pane, binding_id: binding } = presentation;
  const snapshot = useCallback(async (request: ReviewSnapshotRequest, signal: AbortSignal) => {
    const result = await client.reviewSnapshot(session, pane, request, signal);
    if (!signal.aborted) setInvalidation(number => number + 1);
    return result;
  }, [client, session, pane]);
  const file = useCallback((request: ReviewFileRequest, signal: AbortSignal) => client.reviewFile(session, pane, request, signal), [client, session, pane]);
  const repositoryId = reviewRepositoryId(presentation);
  if (!repositoryId) return <div className="review-empty">No Git checkout could be resolved for this Review pane.</div>;
  return <div className="review-viewer" ref={viewerRef} onPointerDown={onRequestControl}>
    <ReviewPane identity={`${session}\0${pane}\0${binding}`} sessionId={session} paneId={pane} bindingId={binding} repositoryId={repositoryId} snapshot={snapshot} file={file} selectedLines={selection}
      viewState={reviewView.mode === mode ? reviewView : { ...reviewView, mode }} onViewStateChange={handleReviewViewChange}
      onOpenSource={() => setMode("source")}
      onCreateLineComment={() => commentActionsRef.current?.createLines()} onCreateFileComment={() => commentActionsRef.current?.createWholeFile()} onOpenCommentOverview={() => commentActionsRef.current?.openOverview()}
      commentCount={commentStatus.count} canCreateLineComment={commentStatus.canCreateLines} canCreateFileComment={commentStatus.canCreateWholeFile}
      onSelectLines={(file, side, start, end, _lines, shift) => setSelection(previous => ({ fileId: file.file_id, side, start: shift && previous?.fileId === file.file_id && previous.side === side ? previous.start : start, end }))}
      presentationControls={<div className="viewer-segmented" role="group" aria-label="Review presentation"><button type="button" aria-pressed={mode === "diff"} onClick={() => setMode("diff")}>Diff</button><button type="button" aria-pressed={mode === "source"} onClick={() => setMode("source")}>Full source</button></div>}
      renderFile={(review, diff, content, loadSourcePage) => {
        const side = selection?.fileId === diff.file.file_id ? selection.side : diff.new_source === null && diff.old_source !== null ? "old" : "new";
        const path = (side === "old" ? diff.file.old_path : diff.file.new_path) ?? "";
        const revision = side === "old" ? diff.file.old_revision ?? null : diff.file.new_revision ?? null;
        const sourceKey = reviewScrollIdentity(session, pane, binding, review.review_id, review.generation, review.comparison, "source", diff.file.file_id, side, revision);
        activeSourceKeyRef.current = sourceKey;
        activeSourceMetadataRef.current = { reviewId: review.review_id, generation: review.generation, fileId: diff.file.file_id };
        const page = sourcePages[sourceKey];
        const source = side === "old" ? diff.old_source : diff.new_source;
        const text = page?.text ?? source;
        const totalBytes = page?.totalBytes ?? (side === "old" ? diff.old_source_total_bytes : diff.new_source_total_bytes);
        const sourceTruncated = page ? page.nextOffset < page.totalBytes : (side === "old" ? diff.old_source_truncated : diff.new_source_truncated);
        const documentRevision = revision ?? "unavailable";
        const range = selection?.fileId === diff.file.file_id ? { start: selection.start, end: selection.end } : null;
        const root: ContextRoot = { root_id: review.source_id, kind: "repository", label: "Review", path: review.checkout_path, repository_id: repositoryId, checkout_path: review.checkout_path, companion_id: null };
        const document: ContextDocument = { binding_id: binding, root_id: root.root_id, path, revision: documentRevision, content_hash: side === "old" ? diff.old_source_hash : diff.new_source_hash, bytes: new TextEncoder().encode(text ?? "").length, media_type: "text/plain", text, truncated: sourceTruncated, diagnostics: diff.diagnostics };
        const reference = { review_id: review.review_id, generation: review.generation, file_id: diff.file.file_id, side };
        const commentIdentity = reviewCommentBatchIdentity(presentation, review.source_id, review.comparison);
        return <CommentDrafts key={commentIdentity} client={client} presentation={presentation} root={root} sourceIdentity={review.source_id} sourceKind="review" reviewCapture={reference} path={path} document={document} selection={range} mode="source" editorState={value.commentEditor} onEditorStateChange={editorChange} invalidationGeneration={invalidation} inlineEditor showToolbar={false} onCommentStatusChange={(next) => updateCommentStatus(commentIdentity, review.comparison, next)} onEditorDismissed={restoreDiffFocus}>
          {(drafts, actions, renderInlineEditor) => {
            commentActionsRef.current = actions;
            return <>
            {mode === "source" ? <div className="review-source-controls"><span>{side} side{range ? ` · lines ${Math.min(range.start, range.end)}–${Math.max(range.start, range.end)}` : ""}</span><button type="button" disabled={diff.old_source === null && !diff.old_source_truncated} onClick={() => setSelection({ fileId: diff.file.file_id, side: "old", start: 1, end: 1 })}>Old source</button><button type="button" disabled={diff.new_source === null && !diff.new_source_truncated} onClick={() => setSelection({ fileId: diff.file.file_id, side: "new", start: 1, end: 1 })}>New source</button></div> : null}
            {mode === "source" && text !== null ? <><SourceLines key={sourceKey} text={text} state={{ rootId: root.root_id, path, mode: "source", revision: documentRevision, selectionStart: range?.start ?? null, selectionEnd: range?.end ?? null, scrollTop: reviewView.scrollPositions?.[sourceKey] ?? (reviewView.scrollIdentity === sourceKey ? reviewView.scrollTop : 0) }} onSelect={(start, end) => setSelection({ fileId: diff.file.file_id, side, start, end })} onScroll={(scrollTop) => updateReviewView({ scrollTop, scrollIdentity: sourceKey, scrollPositions: retainReviewScrollPosition(reviewViewRef.current.scrollPositions ?? {}, sourceKey, scrollTop) })} commentDrafts={drafts.filter(draft => draft.file_ref.review?.file_id === diff.file.file_id && draft.file_ref.review.side === side)} commentActions={actions} inlineEditor={renderInlineEditor} />{sourceTruncated ? <button type="button" disabled={sourceLoading === sourceKey} onClick={() => { const token = sourceLoadSequence.current + 1; sourceLoadSequence.current = token; sourceLoadRef.current = { key: sourceKey, token }; const offset = page?.nextOffset ?? 0; setSourceLoading(sourceKey); void loadSourcePage(side, offset).then((next) => { if (next) appendSourcePage(sourceKey, side, offset, revision, token, next); }).finally(() => { if (sourceLoadRef.current?.token === token) { sourceLoadRef.current = null; setSourceLoading((current) => current === sourceKey ? null : current); } }); }}>{sourceLoading === sourceKey ? "Loading source…" : `Load next source page${totalBytes ? ` (${Math.min((page?.nextOffset ?? 0) + 1, totalBytes)}–${totalBytes} bytes)` : ""}`}</button> : null}</> : mode === "source" ? <p className="review-notice">This source is larger than the initial preview. <button type="button" disabled={sourceLoading === sourceKey} onClick={() => { const token = sourceLoadSequence.current + 1; sourceLoadSequence.current = token; sourceLoadRef.current = { key: sourceKey, token }; setSourceLoading(sourceKey); void loadSourcePage(side, 0).then((next) => { if (next) appendSourcePage(sourceKey, side, 0, revision, token, next); }).finally(() => { if (sourceLoadRef.current?.token === token) { sourceLoadRef.current = null; setSourceLoading((current) => current === sourceKey ? null : current); } }); }}>{sourceLoading === sourceKey ? "Loading…" : "Load source"}</button></p> : content((shown, oldLine, newLine) => <>{(["old", "new"] as const).map(anchorSide => {
              const line = anchorSide === "old" ? oldLine : newLine;
              return line === null ? null : <Fragment key={anchorSide}><InlineCommentDrafts line={line} drafts={drafts.filter(draft => draft.file_ref.review?.file_id === shown.file.file_id && draft.file_ref.review.side === anchorSide)} actions={actions} />{anchorSide === side ? renderInlineEditor?.(line) : null}</Fragment>;
            })}</>)}
            </>;
          }}
        </CommentDrafts>;
      }} />
  </div>;
}
