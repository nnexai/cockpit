import { Component, Fragment, type ReactNode } from "react";
import type { ContextDocument, ContextRoot, CommentDraft, LibraryItemSummary } from "../../protocol/generated/v1";
import type { SourceLines, ContextFileViewState, ContextViewerProps } from "./ContextViewer";
import { CommentDrafts, type CommentDraftActions } from "./CommentDrafts";
import type { ContextReader } from "./contextSource";
import type { DocumentState } from "./useDocumentLoader";
import { UiIcon } from "../UiIcon";
import { MarkdownView } from "./MarkdownView";
import { HtmlPreview } from "./HtmlPreview";
import { SafeImage } from "./SafeImage";
import { CommentBar } from "./CommentBar";
import { providerFacts, type ProviderFacts } from "./providerDocument";
import { ProviderFactsLine } from "../library/LibraryItemHeader";
import { copyText } from "../library/clipboard";
import { splitSourceLines } from "./sourceLines";
import { documentName, isMarkdown, isHtml, isShownDiagnostic, sourceMetadata, sourceLinesForMarkdown } from "./documentMetadata";
import type { SourceMetadata, SourceSpan } from "./documentMetadata";
export interface DocumentHeaderFacts {
  document: ContextDocument;
  metadata: SourceMetadata;
  facts: ProviderFacts;
  frontmatter: SourceSpan | null;
}
export interface DocumentViewProps extends Pick<ContextViewerProps, "client" | "context" | "value" | "onChange" | "onViewerError"> {
  root: ContextRoot; reader: ContextReader | null;
  sourceLines: typeof SourceLines;
  selectedPath: string | null; selectedKey: string | null; selectedRevision: string | null;
  selectedFileState: ContextFileViewState | undefined;
  documentState: DocumentState | undefined; document: ContextDocument | undefined;
  documentPageLoading: string | null; loadDocumentPage: () => Promise<void>;
  rootEmpty: boolean; refresh: () => void; updateFile: (patch: Partial<ContextFileViewState>) => void;
  openMarkdownLink: (href: string) => Promise<void>;
  commentsEnabled: boolean; invalidationGeneration: number; refreshGeneration: number;
  commentStatus: { count: number; canCreateLines: boolean; canCreateWholeFile: boolean };
  updateCommentStatus: (status: { count: number; canCreateLines: boolean; canCreateWholeFile: boolean }) => void;
  restoreSourceFocus: () => void;
  allowRaster: boolean; hiddenTitle?: string | null; libraryItems?: LibraryItemSummary[];
  renderHeader?: (facts: DocumentHeaderFacts) => ReactNode; emptyContent?: ReactNode; noticeContent?: ReactNode; relativePathAction?: ReactNode;
}
class RenderErrorBoundary extends Component<{ fallback: ReactNode; children: ReactNode }, { error: Error | null }> {
  state = { error: null };
  static getDerivedStateFromError(error: Error) {
    return { error };
  }
  render() {
    return this.state.error ? this.props.fallback : this.props.children;
  }
}
export function DocumentView(props: DocumentViewProps) {
  const { root, selectedPath, selectedFileState, selectedRevision, document, context,
    commentsEnabled, client, onViewerError, value, onChange, invalidationGeneration,
    refreshGeneration, updateCommentStatus, restoreSourceFocus } = props;
    const fileState = selectedPath
      ? selectedFileState ?? { rootId: root.root_id, path: selectedPath, mode: "source" as const, selectionStart: null, selectionEnd: null, scrollTop: 0, revision: selectedRevision }
      : null;
    const renderedMode = document && selectedPath ? (isMarkdown(document, selectedPath) ? "markdown" : isHtml(document, selectedPath) ? "html" : null) : null;
    const effectiveMode = fileState?.mode === "auto" ? renderedMode ?? "source" : fileState?.mode ?? "source";
    const selectedRange = fileState && fileState.selectionStart !== null && fileState.selectionEnd !== null
      ? { start: fileState.selectionStart, end: fileState.selectionEnd }
      : null;
  const renderDocumentBody = (drafts: CommentDraft[] = [], actions?: CommentDraftActions, inlineEditor?: (line: number) => ReactNode) =>
    <DocumentBody {...props} fileState={fileState} effectiveMode={effectiveMode} selectedRange={selectedRange} drafts={drafts} actions={actions} inlineEditor={inlineEditor} />;
    if (!commentsEnabled || !context) return renderDocumentBody();
    return (
      <CommentDrafts
        client={client}
        context={context}
        onViewerError={onViewerError}
        root={root}
        path={selectedPath ?? ""}
        document={document ?? null}
        selection={selectedRange}
        mode={effectiveMode === "markdown" ? "markdown" : "source"}
        editorState={value.commentEditor ?? null}
        onEditorStateChange={(commentEditor) => onChange({ ...value, commentEditor })}
        invalidationGeneration={invalidationGeneration}
        refreshGeneration={refreshGeneration}
        sourceIdentity={root.root_id}
        inlineEditor={effectiveMode !== "markdown"}
        showToolbar={false}
        onCommentStatusChange={updateCommentStatus}
        onEditorDismissed={restoreSourceFocus}
      >
        {(drafts, actions, renderInlineEditor) => renderDocumentBody(drafts, actions, renderInlineEditor)}
      </CommentDrafts>
    );
}

function DocumentBody({ fileState, effectiveMode, selectedRange, drafts, actions, inlineEditor, ...props }: DocumentViewProps & {
  fileState: ContextFileViewState | null; effectiveMode: string;
  selectedRange: { start: number; end: number } | null; drafts: CommentDraft[];
  actions?: CommentDraftActions; inlineEditor?: (line: number) => ReactNode;
}) {
  const { root, selectedPath, selectedKey, documentState, document, rootEmpty, refresh, reader,
    updateFile, documentPageLoading, loadDocumentPage, openMarkdownLink, commentStatus,
    emptyContent, noticeContent, renderHeader, relativePathAction, allowRaster, hiddenTitle, libraryItems, sourceLines: SourceLines } = props;
  if (noticeContent) return noticeContent;
  if (!selectedPath && emptyContent !== undefined) return emptyContent;
      if (!selectedPath && rootEmpty) {
        return <div className="context-empty"><div className="context-empty-message"><strong>No files here yet</strong><span>This directory is empty.</span></div></div>;
      }
      if (!selectedPath) return <div className="context-empty">Select a file to inspect its source.</div>;
      if (!documentState || documentState.status === "loading") return <div className="context-empty">Loading source…</div>;
      if (documentState.status === "error" && !document) {
        return <div className="context-notice context-notice-error"><strong>Unable to read source</strong><span>{documentState.error}</span><button type="button" onClick={refresh}>Retry</button></div>;
      }
      if (!document) return null;
      const metadata = sourceMetadata(document.text ?? "");
      const facts = providerFacts(document.text ?? "");
      const frontmatter = sourceLinesForMarkdown(document.text ?? "").frontmatter;
      const selectedLines = selectedRange
        ? `Lines ${Math.min(selectedRange.start, selectedRange.end)}–${Math.max(selectedRange.start, selectedRange.end)}`
        : null;
      const pageRequestKey = selectedKey && document.next_offset !== undefined
        ? `${selectedKey}\u0000${document.revision}\u0000${document.next_offset}`
        : null;
      const documentDetails = (
        <details className="viewer-details">
          <summary aria-label="Document details" title="Document details"><UiIcon name="info" /></summary>
          <dl>
            <dt>Size</dt><dd>{document.bytes} B</dd><dt>Full path</dt><dd><code>{root.path.replace(/\/$/, "")}/{selectedPath}</code><button type="button" onClick={() => void copyText(`${root.path.replace(/\/$/, "")}/${selectedPath}`)}>Copy full path</button></dd>
            <dt>Relative path</dt><dd><code>{selectedPath}</code>{relativePathAction}</dd>
            <dt>Root identity</dt><dd><code>{root.root_id}</code></dd>
            <dt>Provenance</dt><dd>{root.kind}{root.repository_id ? ` · repository ${root.repository_id}` : ""}</dd>
            <dt>Revision</dt><dd><code>{document.revision}</code></dd>
            <dt>Content hash</dt><dd><code>{document.content_hash ?? "Unavailable"}</code></dd>
            <dt>Media type</dt><dd>{document.media_type || "Unavailable"}</dd>
            {metadata.provider ? <><dt>Provider</dt><dd>{metadata.provider}</dd></> : null}
            {metadata.canonicalId ? <><dt>Source identity</dt><dd><code>{metadata.canonicalId}</code></dd></> : null}
            {metadata.fetchedAt ? <><dt>Freshness</dt><dd>{metadata.fetchedAt}</dd></> : null}
            {frontmatter ? <><dt>Frontmatter</dt><dd><pre>{splitSourceLines(document.text ?? "").slice(frontmatter.start - 1, frontmatter.end).map((line) => line.raw).join("")}</pre></dd></> : null}
            {document.diagnostics.map((diagnostic, index) => <Fragment key={`${diagnostic.code}-${index}`}><dt>Diagnostic</dt><dd><code>{diagnostic.code}</code>{diagnostic.path ? ` · ${diagnostic.path}` : ""} · {diagnostic.message}</dd></Fragment>)}
          </dl>
        </details>
      );
  return (
    <>
      {renderHeader?.({ document, metadata, facts, frontmatter }) ?? <div className="context-document-header">
            {metadata.canonicalId ? <span className="document-source-kind">{metadata.provider ?? "Issue"}</span> : null}<strong title={selectedPath}>{metadata.canonicalId ?? documentName(selectedPath)}</strong>
            {facts.generated ? <ProviderFactsLine facts={facts} now={Date.now()} className="context-document-facts" /> : null}
            {document.truncated ? <span className="context-state-warning">Truncated by preview limit</span> : null}

        {documentDetails}
      </div>}
          {documentState.status === "error" ? <div className="context-notice context-notice-warning" role="status"><strong>Stale source</strong><span>{documentState.error}</span><button type="button" onClick={refresh}>Refresh</button></div> : null}
          {document.text !== null && document.diagnostics.some(isShownDiagnostic) ? <div className="context-notice context-notice-warning" role="status">{document.diagnostics.filter(isShownDiagnostic).map((diagnostic) => <span key={`${diagnostic.code}:${diagnostic.message}`}>{diagnostic.message}</span>)}</div> : null}
          {/\.pdf$/i.test(selectedPath) ? <div className="context-notice"><strong>PDF preview unavailable</strong><span>This file is retained without an active PDF renderer.</span></div> : document.text === null && reader && allowRaster && /\.(png|jpe?g)$/i.test(selectedPath) ? <div className="context-raster-preview"><SafeImage media={reader.media} request={{ root_id: root.root_id, path: selectedPath, expected_revision: document.revision }} alt={selectedPath} className="context-safe-image" /></div> : document.text === null ? <div className="context-notice context-notice-error"><strong>{/\.pdf$/i.test(selectedPath) ? "PDF preview unavailable" : "File refused"}</strong><span>{document.media_type || "Binary or unsupported content"}</span>{document.diagnostics.map((diagnostic) => <span key={`${diagnostic.code}:${diagnostic.message}`}>{diagnostic.message}</span>)}</div> : <>
            {(() => {
              const mode = effectiveMode;
              return <>
                {document.truncated && document.next_offset !== undefined ? <div className="context-notice context-notice-warning" role="status"><span>Showing the start of a large file.</span><button type="button" onClick={() => { updateFile({ mode: "source" }); void loadDocumentPage(); }} disabled={documentPageLoading === pageRequestKey}>{documentPageLoading === pageRequestKey ? "Loading…" : "Load next source page"}</button></div> : null}
                <RenderErrorBoundary fallback={<div className="context-notice context-notice-error"><strong>Markdown rendering failed</strong><span>Showing the canonical source instead.</span><SourceLines text={document.text!} state={{ ...fileState!, mode: "source" }} onSelect={(start, end) => updateFile({ selectionStart: start, selectionEnd: end, mode: "source" })} onScroll={(scrollTop) => updateFile({ scrollTop })} commentDrafts={drafts} commentActions={actions} /></div>}>
                  {mode === "markdown" ? <MarkdownView media={reader?.media ?? null} text={document.text!} hiddenTitle={hiddenTitle ?? null} state={{ ...fileState!, mode: "markdown" }} libraryItems={libraryItems ?? []} onLink={(href) => void openMarkdownLink(href)} onSelect={(start, end) => updateFile({ selectionStart: start, selectionEnd: end })} onScroll={(scrollTop) => updateFile({ scrollTop })} /> : mode === "html" ? <HtmlPreview html={document.text!} title={selectedPath} /> : <SourceLines text={document.text!} state={{ ...fileState!, mode: "source" }} onSelect={(start, end) => updateFile({ selectionStart: start, selectionEnd: end })} onScroll={(scrollTop) => updateFile({ scrollTop })} commentDrafts={drafts} commentActions={actions} inlineEditor={inlineEditor} onCreateLineComment={actions?.createLines} onCreateFileComment={actions?.createWholeFile} />}
                </RenderErrorBoundary>
              </>;
            })()}
            {actions ? <CommentBar actions={actions} selectedLines={selectedLines} status={commentStatus} /> : null}
          </>}
        </>
      );

}
