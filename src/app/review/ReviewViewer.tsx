import { Fragment, useCallback, useRef, useState } from "react";
import type { CockpitClient } from "../../client/CockpitClient";
import type { ContextDocument, ContextRoot, PanePresentation, ReviewFileRequest, ReviewSnapshotRequest } from "../../protocol/generated/v1";
import { CommentDrafts, InlineCommentDrafts, type CommentDraftActions } from "../context/CommentDrafts";
import { SourceLines, type ContextViewState } from "../context/ContextViewer";
import { ReviewPane } from "./ReviewPane";

type Selection = { fileId: string; side: "old" | "new"; start: number; end: number } | null;
type CommentStatus = { count: number; canCreateLines: boolean; canCreateWholeFile: boolean };
export function ReviewViewer({ client, presentation, value, onChange, onTerminalView: _onTerminalView, onRequestControl }: {
  client: CockpitClient; presentation: PanePresentation; value: ContextViewState;
  onChange: (next: ContextViewState) => void; onTerminalView: () => void; onRequestControl: () => void;
}) {
  const [selection, setSelection] = useState<Selection>(null);
  const viewerRef = useRef<HTMLDivElement>(null);
  const [mode, setMode] = useState<"diff" | "source">("diff");
  const [invalidation, setInvalidation] = useState(0);
  const [commentStatus, setCommentStatus] = useState<CommentStatus>({ count: 0, canCreateLines: false, canCreateWholeFile: false });
  const commentActionsRef = useRef<CommentDraftActions | null>(null);
  const changeRef = useRef(onChange); changeRef.current = onChange;
  const valueRef = useRef(value); valueRef.current = value;
  const editorChange = useCallback((commentEditor: ContextViewState["commentEditor"]) => changeRef.current({ ...valueRef.current, commentEditor }), []);
  const updateCommentStatus = useCallback((next: CommentStatus) => setCommentStatus((current) => current.count === next.count && current.canCreateLines === next.canCreateLines && current.canCreateWholeFile === next.canCreateWholeFile ? current : next), []);
  const restoreDiffFocus = useCallback(() => requestAnimationFrame(() => viewerRef.current?.querySelector<HTMLElement>(".review-diff")?.focus()), []);
  const { session_id: session, pane_id: pane, binding_id: binding } = presentation;
  const snapshot = useCallback(async (request: ReviewSnapshotRequest, signal: AbortSignal) => {
    const result = await client.reviewSnapshot(session, pane, request, signal);
    if (!signal.aborted) { setSelection(null); setInvalidation(number => number + 1); }
    return result;
  }, [client, session, pane]);
  const file = useCallback((request: ReviewFileRequest, signal: AbortSignal) => client.reviewFile(session, pane, request, signal), [client, session, pane]);
  const repositoryId = (presentation.roots.find(root => root.root_id === presentation.default_root_id)
    ?? presentation.roots.find(root => root.kind === "repository"))?.repository_id;
  if (!repositoryId) return <div className="review-empty">No Git checkout could be resolved for this Review pane.</div>;
  return <div className="review-viewer" ref={viewerRef} onPointerDown={onRequestControl}>
    <ReviewPane identity={`${session}\0${pane}\0${binding}`} sessionId={session} paneId={pane} bindingId={binding} repositoryId={repositoryId} snapshot={snapshot} file={file} selectedLines={selection}
      onCreateLineComment={() => commentActionsRef.current?.createLines()} onCreateFileComment={() => commentActionsRef.current?.createWholeFile()} onOpenCommentOverview={() => commentActionsRef.current?.openOverview()}
      commentCount={commentStatus.count} canCreateLineComment={commentStatus.canCreateLines} canCreateFileComment={commentStatus.canCreateWholeFile}
      onSelectLines={(file, side, start, end, _lines, shift) => setSelection(previous => ({ fileId: file.file_id, side, start: shift && previous?.fileId === file.file_id && previous.side === side ? previous.start : start, end }))}
      renderFile={(review, diff, content) => {
        const side = selection?.fileId === diff.file.file_id ? selection.side : diff.new_source === null && diff.old_source !== null ? "old" : "new";
        const path = (side === "old" ? diff.file.old_path : diff.file.new_path) ?? "";
        const text = side === "old" ? diff.old_source : diff.new_source;
        const revision = (side === "old" ? diff.file.old_revision : diff.file.new_revision) ?? "unavailable";
        const range = selection?.fileId === diff.file.file_id ? { start: selection.start, end: selection.end } : null;
        const root: ContextRoot = { root_id: review.source_id, kind: "repository", label: "Review", path: review.checkout_path, repository_id: repositoryId, checkout_path: review.checkout_path, companion_id: null };
        const document: ContextDocument = { binding_id: binding, root_id: root.root_id, path, revision, content_hash: side === "old" ? diff.old_source_hash : diff.new_source_hash, bytes: new TextEncoder().encode(text ?? "").length, media_type: "text/plain", text, truncated: side === "old" ? diff.old_source_truncated : diff.new_source_truncated, diagnostics: diff.diagnostics };
        const reference = { review_id: review.review_id, generation: review.generation, file_id: diff.file.file_id, side };
        return <CommentDrafts client={client} presentation={presentation} root={root} sourceIdentity={review.source_id} sourceKind="review" reviewCapture={reference} path={path} document={document} selection={range} mode="source" editorState={value.commentEditor} onEditorStateChange={editorChange} invalidationGeneration={invalidation} inlineEditor showToolbar={false} onCommentStatusChange={updateCommentStatus} onEditorDismissed={restoreDiffFocus}>
          {(drafts, actions, renderInlineEditor) => {
            commentActionsRef.current = actions;
            return <>
            <div className="review-source-controls"><button type="button" onClick={() => setMode(mode === "diff" ? "source" : "diff")}>{mode === "diff" ? "Expand full source" : "Unified diff"}</button><span>{side} side{range ? ` · lines ${Math.min(range.start, range.end)}–${Math.max(range.start, range.end)}` : ""}</span><button type="button" disabled={diff.old_source === null} onClick={() => setSelection({ fileId: diff.file.file_id, side: "old", start: 1, end: 1 })}>Old source</button><button type="button" disabled={diff.new_source === null} onClick={() => setSelection({ fileId: diff.file.file_id, side: "new", start: 1, end: 1 })}>New source</button></div>
            {mode === "source" && text !== null ? <SourceLines text={text} state={{ rootId: root.root_id, path, mode: "source", revision, selectionStart: range?.start ?? null, selectionEnd: range?.end ?? null, scrollTop: 0 }} onSelect={(start, end) => setSelection({ fileId: diff.file.file_id, side, start, end })} onScroll={() => undefined} commentDrafts={drafts.filter(draft => draft.file_ref.review?.file_id === diff.file.file_id && draft.file_ref.review.side === side)} commentActions={actions} inlineEditor={renderInlineEditor} /> : content((shown, oldLine, newLine) => <>{(["old", "new"] as const).map(anchorSide => {
              const line = anchorSide === "old" ? oldLine : newLine;
              return line === null ? null : <Fragment key={anchorSide}><InlineCommentDrafts line={line} drafts={drafts.filter(draft => draft.file_ref.review?.file_id === shown.file.file_id && draft.file_ref.review.side === anchorSide)} actions={actions} />{anchorSide === side ? renderInlineEditor?.(line) : null}</Fragment>;
            })}</>)}
            </>;
          }}
        </CommentDrafts>;
      }} />
  </div>;
}
