import { useCallback, useEffect, useId, useMemo, useRef, useState, type CSSProperties, type ReactNode } from "react";
import type { CockpitClient } from "../../client/CockpitClient";
import type { CommentReviewRef, ViewerContext, CommentDraft, ContextRoot, ReviewComparison } from "../../protocol/generated/v1";
import { useFileOverview } from "../input/useFileOverview";
import { FilePicker } from "../input/FilePicker";
import { FILE_NAVIGATION_EVENT, fileNavigationAction } from "../input/fileNavigation";
import { useTreeWidth, useWrapPreference } from "../viewer/ViewerLayout";
import { highlightLines } from "../viewer/highlight";
import { InlineCommentDrafts, type CommentDraftActions } from "./CommentDrafts";
import { ContextSearch } from "./ContextSearch";
import { UiIcon } from "../UiIcon";
import { splitSourceLines } from "./sourceLines";
import { isEditingTarget } from "./viewerState";
import { isMarkdown, isHtml } from "./documentMetadata";
import { useDefaultGuide, useDirectoryDiagnostics, useDirectoryTree } from "./useDirectoryTree";
import { useDocumentCache, useDocumentLoader } from "./useDocumentLoader";
import { useFilePicker } from "./useFilePicker";
import { useViewerFiles, useSelectedFile } from "./useViewerFiles";
import { useViewerComments } from "./useViewerComments";
import { useViewerSearch } from "./useViewerSearch";
import { useDocumentLinks } from "./useDocumentLinks";
import { DocumentView, type DocumentViewProps } from "./DocumentView";
import { ContextViewerFrame, type ViewerToolbarProps } from "./ContextViewerFrame";
import { ContextDirectoryRows } from "./ContextDirectoryRows";
import { filesViewSlots } from "./ContextFilesView";
import { useLibraryPrimarySelection, useLibrarySelectionFreshness, useLibraryViewerSource } from "../library/useLibraryViewerSource";
import { useLibraryViewerController } from "../library/useLibraryViewerController";
import { libraryViewSlots } from "../library/LibraryViewerView";
import { LibraryViewerDialogs } from "../library/LibraryViewerDialogs";
import type { LibrarySpace } from "../library/libraryState";
import type { LibraryListingState } from "../library/useLibraryOperation";
import "./context.css";

export type ContextViewMode = "auto" | "source" | "markdown" | "html";

export interface ContextFileViewState {
  rootId: string;
  path: string;
  mode: ContextViewMode;
  selectionStart: number | null;
  selectionEnd: number | null;
  scrollTop: number;
  revision?: string | null;
}

export interface ContextCommentEditorState {
  review?: CommentReviewRef;
  rootId: string;
  path: string;
  revision: string;
  draftId: string | null;
  editor: "whole_file" | "lines";
  text: string;
  selection: { start: number; end: number } | null;
}

export interface ReviewViewState {
  comparison: ReviewComparison;
  baseRef: string;
  draftBaseRef: string;
  fileId: string | null;
  filePath: string | null;
  side: "old" | "new" | null;
  selectionStart: number | null;
  selectionEnd: number | null;
  hunkIndex: number;
  /** Current view scroll, retained separately from the bounded cross-identity cache. */
  scrollTop: number;
  /** At most 64 full review identities are retained for revisit restoration. */
  scrollPositions: Record<string, number>;
  /** Identity represented by scrollTop and the current rendered scroll surface. */
  scrollIdentity: string | null;
  mode: "diff" | "source";
  commentCount: number | null;
  overviewChoice?: boolean | null;
}

export interface ContextViewState {
  rootId: string | null;
  path: string | null;
  files: Record<string, ContextFileViewState>;
  commentEditor: ContextCommentEditorState | null;
  review: ReviewViewState | null;
  overviewChoice?: boolean | null;
}

export function createContextViewState(): ContextViewState {
  return { rootId: null, path: null, files: {}, commentEditor: null, review: null };
}

export function createReviewViewState(): ReviewViewState {
  return {
    comparison: "all_local",
    baseRef: "",
    draftBaseRef: "",
    fileId: null,
    filePath: null,
    side: null,
    selectionStart: null,
    selectionEnd: null,
    hunkIndex: -1,
    scrollTop: 0,
    scrollPositions: {},
    scrollIdentity: null,
    mode: "diff",
    commentCount: null,
  };
}

export function retainReviewScrollPosition(current: Record<string, number>, identity: string, scrollTop: number): Record<string, number> {
  const next = { ...current };
  delete next[identity];
  next[identity] = scrollTop;
  const keys = Object.keys(next);
  while (keys.length > 64) {
    const oldest = keys.shift();
    if (oldest !== undefined) delete next[oldest];
  }
  return next;
}


/** A request from outside the viewer to act on its Library root once the listing can serve it. */
export type LibraryCommand = { token: number; kind: "refresh" } | { token: number; kind: "tokens" } | { token: number; kind: "open"; itemId: string };

export type ContextViewerProps = {
  client: CockpitClient;
  /** The viewer's roots and read authority; null in the session-independent Library view. */
  context: ViewerContext | null;
  value: ContextViewState;
  onChange: (next: ContextViewState) => void;
  /** Reports a missing viewer context so its owning leaf can offer explicit Reopen. */
  onViewerError?: (error: unknown) => void;
  /** Opens only a freshly issued repository root through the owning viewer. */
  onOpenRepository?: (path: string) => Promise<void>;
  /** The Library view's listing; a viewer reads its own only while its Library root is shown. */
  library?: LibraryListingState;
  libraryCommand?: LibraryCommand | null;
  /**
   * The Space `Add to <Space>` and `Resources` act on: the viewer's own Space, or
   * the selected Space in the Library view. Null with no session or Space.
   */
  space?: LibrarySpace | null;
};

export function SourceLines({
  text,
  state,
  onSelect,
  onScroll,
  commentDrafts = [],
  commentActions,
  inlineEditor,
  onCreateLineComment,
  onCreateFileComment,
}: {
  text: string;
  state: ContextFileViewState;
  onSelect: (start: number, end: number, extend: boolean) => void;
  onScroll: (scrollTop: number) => void;
  commentDrafts?: CommentDraft[];
  commentActions?: CommentDraftActions;
  inlineEditor?: (line: number) => ReactNode;
  onCreateLineComment?: () => void;
  onCreateFileComment?: () => void;
}) {
  const lines = useMemo(() => splitSourceLines(text), [text]);
  const highlighted = useMemo(() => highlightLines(text, state.path), [text, state.path]);
  const [wrap] = useWrapPreference();
  const scrollRef = useRef<HTMLDivElement>(null);
  const anchorRef = useRef<number | null>(null);
  const restoredScrollIdentity = useRef<string | null>(null);
  useEffect(() => {
    anchorRef.current = null;
  }, [state.rootId, state.path, state.revision, text]);
  const scrollIdentity = `${state.rootId}\u0000${state.path}\u0000${state.revision ?? ""}\u0000${text}`;
  useEffect(() => {
    if (restoredScrollIdentity.current === scrollIdentity) return;
    restoredScrollIdentity.current = scrollIdentity;
    if (scrollRef.current) scrollRef.current.scrollTop = state.scrollTop;
  }, [scrollIdentity, state.scrollTop]);
  const selectLine = (lineNumber: number, extend: boolean) => {
    const anchor = anchorRef.current;
    const start = extend && anchor !== null ? Math.min(anchor, lineNumber) : lineNumber;
    const end = extend && anchor !== null ? Math.max(anchor, lineNumber) : lineNumber;
    if (!extend) anchorRef.current = lineNumber;
    onSelect(start, end, extend);
  };
  return (
    <div
      className={`context-source-scroll${wrap ? " is-wrapped" : ""}`}
      style={{ "--source-digits": `${Math.max(2, String(lines.length).length)}ch` } as CSSProperties}
      ref={scrollRef}
      onScroll={(event) => onScroll(event.currentTarget.scrollTop)}
      role="grid"
      aria-label="Source lines"
    >
      <div className="context-source-lines">
        {lines.map((line, index) => {
          const lineNumber = index + 1;
          const selected = state.selectionStart !== null && state.selectionEnd !== null && lineNumber >= Math.min(state.selectionStart, state.selectionEnd) && lineNumber <= Math.max(state.selectionStart, state.selectionEnd);
          return (
            <span className="context-source-line-wrap" key={lineNumber}>
              <button
                type="button"
                role="row"
                className={`context-source-line${selected ? " is-selected" : ""}`}
                aria-selected={selected}
                data-line={lineNumber}
                onClick={(event) => {
                  selectLine(lineNumber, event.shiftKey);
                }}
                onKeyDown={(event) => {
                  if (event.ctrlKey || event.metaKey || event.altKey) return;
                  if (event.key.toLowerCase() === "c" && (onCreateLineComment || onCreateFileComment)) {
                    event.preventDefault();
                    event.stopPropagation();
                    if (event.shiftKey) onCreateFileComment?.(); else onCreateLineComment?.();
                    return;
                  }
                  const next = event.key === "ArrowUp" ? Math.max(1, lineNumber - 1)
                    : event.key === "ArrowDown" ? Math.min(lines.length, lineNumber + 1)
                      : event.key === "Home" ? 1
                        : event.key === "End" ? lines.length : null;
                  if (next === null) return;
                  event.preventDefault();
                  event.stopPropagation();
                  if (event.shiftKey && anchorRef.current === null) anchorRef.current = state.selectionStart ?? lineNumber;
                  selectLine(next, event.shiftKey);
                  requestAnimationFrame(() => scrollRef.current?.querySelector<HTMLButtonElement>(`[data-line="${next}"]`)?.focus());
                }}
              >
                <span className="context-line-number" aria-hidden="true">{lineNumber}</span>
                {highlighted?.[index] ? <code className="context-line-text" dangerouslySetInnerHTML={{ __html: highlighted[index] }} /> : <code className="context-line-text">{line.text || " "}</code>}
              </button>
              {commentActions ? <InlineCommentDrafts drafts={commentDrafts} line={lineNumber} actions={commentActions} rootId={state.rootId} path={state.path} /> : null}
              {inlineEditor?.(lineNumber)}
            </span>
          );
        })}
      </div>
    </div>
  );
}

export function ContextViewer(props: ContextViewerProps) {
  const { client, context, value, onChange, onViewerError, libraryCommand = null, space = null } = props;
  const source = useLibraryViewerSource(props);
  const { root, roots, reader, bindingId, identityKey } = source;
  const viewerRef = useRef<HTMLElement>(null);
  const documentRef = useRef<HTMLElement>(null);
  const treeRef = useRef<HTMLElement>(null);
  const overview = useFileOverview(viewerRef, value.overviewChoice ?? null, (overviewChoice) => onChange({ ...value, overviewChoice }));
  const overviewId = useId();
  const [wrap, toggleWrap] = useWrapPreference();
  const tree = useTreeWidth(source.layout);
  const cache = useDocumentCache();
  const selection = useLibraryPrimarySelection(source, value);
  const { selectedPath, selectedSnapshotRevision } = selection;
  useLibrarySelectionFreshness(source, selection, value, onChange, cache.setDocuments);
  const files = useViewerFiles({ root, selectedPath, value, onChange, narrow: overview.narrow,
    clearPrimarySelection: () => { selection.primarySelectionRef.current = null; } });
  const { selectedKey, selectedFileState, openFile } = files;
  const directory = useDirectoryTree({ reader, root, bindingId, identityKey, selectedPath,
    treeRef, enabled: source.directoryEnabled, openFile });
  const { directoryPathForFile, loadDirectory } = directory;
  const selectedRevision = source.latestRevisionOnly || Boolean(selectedKey && files.latestRevisionKeysRef.current.has(selectedKey))
    ? null : directory.selectedEntry?.revision ?? selectedFileState?.revision ?? null;
  const updateFile = useSelectedFile({ root, selectedPath, selectedKey, selectedRevision, value, onChange });
  const comments = useViewerComments({ root, document: selectedKey ? cache.documents[selectedKey]?.document : undefined,
    selectedPath, selectedKey, selectedFileState, value, onChange, updateFile, documentRef });
  const identityRef = useRef<string | null>(null);
  useEffect(() => {
    if (identityRef.current === identityKey) return;
    identityRef.current = identityKey;
    if (root && value.rootId !== root.root_id) onChange({ ...value, rootId: root.root_id, path: null });
  }, [identityKey, onChange, root, value]);
  const [refreshGeneration, setRefreshGeneration] = useState(0);
  const revalidate = useCallback(() => setRefreshGeneration((generation) => generation + 1), []);
  const loader = useDocumentLoader({ cache, reader, root, bindingId, identityKey, selectedPath,
    selectedKey, selectedRevision, selectedSnapshotRevision, refreshGeneration });
  const { document } = loader;
  const picker = useFilePicker({ reader, root, bindingId, identityKey });
  const { openFilePicker, closeFilePicker } = picker;
  const chooseRoot = (nextRoot: ContextRoot) => {
    source.setRootId(nextRoot.root_id);
    directory.setExpanded(new Set());
    onChange({ ...value, rootId: nextRoot.root_id, path: null });
  };
  const library = useLibraryViewerController({ client, source, selectedPath, value, onChange,
    space, libraryCommand, revalidate, openFile, chooseRoot, viewerRef, setDocuments: cache.setDocuments });
  const links = useDocumentLinks({ root, reader, selectedPath, libraryItems: library.libraryItems,
    onLibraryLink: library.onLibraryLink, openFile, setExpanded: directory.setExpanded, loadDirectory });
  const [resourcesOpen, setResourcesOpen] = useState(false);
  useEffect(() => { setResourcesOpen(false); }, [identityKey]);
  const refresh = () => {
    if (!root) return;
    revalidate();
    library.reload(selectedPath ? directoryPathForFile : "", loadDirectory);
  };
  const search = useViewerSearch({ root, reader, value, onChange, selectedPath,
    directoryPathForFile, loadDirectory, setDocuments: cache.setDocuments });
  const discoveryDiagnostics = useDirectoryDiagnostics(root, directory.directories, source.rootDiagnostics);
  const focusTree = useCallback(() => {
    overview.show();
    requestAnimationFrame(() => {
      const buttons = treeRef.current?.querySelectorAll<HTMLButtonElement>("[data-context-path], [data-library-row]");
      const selected = selectedPath ? [...(buttons ?? [])].find((button) => button.dataset.contextPath === selectedPath) : null;
      (selected ?? buttons?.[0] ?? treeRef.current)?.focus();
    });
  }, [overview.show, selectedPath]);
  const focusContent = useCallback(() => { requestAnimationFrame(() => documentRef.current?.focus()); }, []);
  useEffect(() => {
    const onNavigation = (event: Event) => {
      const active = globalThis.document.activeElement;
      if (!viewerRef.current?.contains(active) || isEditingTarget(active) || active?.closest('dialog[open], [role="dialog"][aria-modal="true"]')) return;
      if (fileNavigationAction(event) === "open-picker") openFilePicker();
    };
    window.addEventListener(FILE_NAVIGATION_EVENT, onNavigation);
    return () => window.removeEventListener(FILE_NAVIGATION_EVENT, onNavigation);
  }, [openFilePicker]);
  useDefaultGuide({ identityKey, hasPath: Boolean(value.path), rootDirectory: directory.rootDirectory, chooseEntry: directory.chooseEntry });
  if (!root) return <div className="context-viewer context-viewer-empty"><div className="context-notice context-notice-error"><strong>Context unavailable</strong><span>This viewer is not connected to an authorized Context root.</span></div></div>;
  const presentable = Boolean(document && selectedPath && (isMarkdown(document, selectedPath) || isHtml(document, selectedPath)));
  const fileMode = selectedFileState?.mode ?? "source";
  const sourceShown = Boolean(document && selectedPath) && (fileMode === "source" || (fileMode === "auto" && !presentable));
  const toolbar: ViewerToolbarProps = { overview, overviewId, openFilePicker, context: context !== null,
    resourcesOpen, setResourcesOpen, presentable, selectedFileState, updateFile, wrap, toggleWrap, refresh, sourceShown };
  const documentProps: DocumentViewProps = { client, context, value, onChange, onViewerError, root, reader, sourceLines: SourceLines,
    selectedPath, selectedKey, selectedRevision, selectedFileState, documentState: loader.documentState,
    document, documentPageLoading: loader.documentPageLoading, loadDocumentPage: loader.loadDocumentPage,
    rootEmpty: directory.rootEmpty, refresh, updateFile, openMarkdownLink: links.openMarkdownLink,
    commentsEnabled: source.commentsEnabled, invalidationGeneration: search.invalidationGeneration,
    refreshGeneration, ...comments, allowRaster: source.allowRaster, libraryItems: library.libraryItems };
  const view = source.isLibrary
    ? libraryViewSlots({ source, controller: library, toolbar, document: documentProps, narrow: overview.narrow })
    : filesViewSlots({ toolbar, document: <DocumentView {...documentProps} />, onTreeKeyDown: directory.onTreeKeyDown });
  return <ContextViewerFrame view={view} root={root} roots={roots} chooseRoot={chooseRoot}
    viewerRef={viewerRef} treeRef={treeRef} documentRef={documentRef} overview={overview} overviewId={overviewId}
    tree={tree} layout={source.layout} discoveryDiagnostics={discoveryDiagnostics} linkNotice={links.linkNotice}
    search={reader?.search ? <details className="viewer-tree-search"><summary><UiIcon name="search" /> Search contents</summary><ContextSearch
      identity={identityKey} bindingId={bindingId} rootId={root.root_id} known={loader.knownRevisions}
      search={search.searchContext} poll={search.pollContext} onSelect={files.selectSearchResult}
      onInvalidate={search.invalidateVisibleFiles} /></details> : null}
    directoryRows={<ContextDirectoryRows root={root} selectedPath={selectedPath} {...directory} />}
    onShortcut={(action) => {
      if (action === "open-file-picker") openFilePicker();
      else if (action === "focus-file-tree") focusTree();
      else if (action === "focus-file-content") focusContent();
      else if (action === "toggle-wrap") toggleWrap();
      else if (action === "reload-listing") refresh();
      else if (action === "toggle-preview" && presentable) updateFile({ mode: selectedFileState?.mode === "source" ? "auto" : "source" });
    }}>
    <LibraryViewerDialogs {...props} source={source} controller={library} refresh={refresh}
      resourcesOpen={resourcesOpen} setResourcesOpen={setResourcesOpen} selectedPath={selectedPath} />
    {picker.pickerOpen ? <FilePicker candidates={picker.pickerCandidates} preparedCandidates={picker.preparedPickerCandidates}
      loading={picker.pickerIndex.loading} incomplete={picker.pickerIndex.incomplete}
      mayBeOutOfDate={picker.pickerIndex.mayBeOutOfDate} failed={picker.pickerIndex.failed}
      onChoose={(candidate) => { openFile(candidate.path, null); closeFilePicker(); focusContent(); }}
      onDismiss={() => { closeFilePicker(); focusContent(); }} /> : null}
  </ContextViewerFrame>;
}
