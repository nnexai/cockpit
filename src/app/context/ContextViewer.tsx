import { UiIcon } from "../UiIcon";
import { useFileOverview } from "../input/useFileOverview";
import { remarkBoundDiagrams } from "./markdownPolicy";
import { SafeImage } from "./SafeImage";
import { MermaidView } from "./MermaidView";
import type { CommentReviewRef } from "../../protocol/generated/v1";
import { HtmlPreview } from "./HtmlPreview";
import { Component, Fragment, useId, useLayoutEffect, type CSSProperties, type KeyboardEvent as ReactKeyboardEvent, type MouseEvent as ReactMouseEvent, type ReactNode, useCallback, useEffect, useMemo, useRef, useState } from "react";
import ReactMarkdown, { type Components } from "react-markdown";
import remarkGfm from "remark-gfm";
import type { CockpitClient } from "../../client/CockpitClient";
import type {
  ContextDirectory,
  ContextDocument,
  ContextEntry,
  ContextInvalidation,
  ContextKnownRevision,
  ContextRoot,
  PanePresentation,
  CommentDraft,
  LibraryAttachmentRequest,
  LibraryItemSummary,
  LibraryOperation,
  LibraryRefreshRequest,
  ReviewComparison,
  SpaceContextListing,
  SpaceCopyRow,
  SpaceTarget,
} from "../../protocol/generated/v1";
import { CommentDrafts, InlineCommentDrafts, type CommentDraftActions } from "./CommentDrafts";
import { ContextSearch } from "./ContextSearch";
import { ContextResources } from "./ContextResources";
import { splitSourceLines } from "./sourceLines";
import { FilePicker } from "../input/FilePicker";
import { FILE_NAVIGATION_EVENT, fileNavigationAction, type FileNavigationCandidate } from "../input/fileNavigation";
import { highlightLines } from "../viewer/highlight";
import { TreeSplitter, useTreeWidth, useWrapPreference } from "../viewer/ViewerLayout";
import { LIBRARY_ROOT_ID, libraryReader, paneReader, type ContextDirectoryRead, type ContextDocumentRead, type ContextReader } from "./contextSource";
import { AddContextDialog } from "../library/AddContextDialog";
import { LibraryConfirmDialog, SpaceCopyConfirmDialog, spaceCopyConflict, type SpaceCopyConfirmation } from "../library/LibraryConfirmDialog";
import { AttachmentReport, LibraryAttachmentNotice, LibraryItemHeader, type ItemSpaceState } from "../library/LibraryItemHeader";
import { LibraryMenu, LibraryTree, attachmentPath, menuAnchor, type LibraryAttachmentActions, type LibraryItemActions } from "../library/LibraryTree";
import { RefreshReport } from "../library/RefreshReport";
import { providerFamily, sameSpaceTarget, type LibrarySpace } from "../library/libraryState";
import { headerSpaceAction, spaceCopyChip, type SpaceCopyActionKind } from "../library/spaceCopyPresentation";
import { spaceAddFailure, spaceCopyActions, spaceUpdateOutcome, spaceUpdateUnconfirmed, useSpaceUpdate } from "../library/SpaceContextList";
import { announceLibraryChanged, LIBRARY_CHANGED_EVENT, useLibraryListing, useLibraryOperation, useSpaceContextListing, type LibraryListingState } from "../library/useLibraryOperation";
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
}

export interface ContextViewState {
  rootId: string | null;
  path: string | null;
  files: Record<string, ContextFileViewState>;
  commentEditor: ContextCommentEditorState | null;
  review: ReviewViewState | null;
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
export type LibraryCommand = { token: number; kind: "refresh" } | { token: number; kind: "open"; itemId: string };

export type ContextViewerProps = {
  client: CockpitClient;
  /** The pane's roots and read authority; null in the Library view, which has only the Library root. */
  presentation: PanePresentation | null;
  value: ContextViewState;
  onChange: (next: ContextViewState) => void;
  controlAllowed: boolean;
  onRequestControl: () => void;
  /** Absent in the Library view, which has no terminal to return to. */
  onTerminalView?: () => void;
  /** The Library view's listing; a pane reads its own only while its Library root is shown. */
  library?: LibraryListingState;
  libraryCommand?: LibraryCommand | null;
  /**
   * The Space `Add to <Space>` and `Resources` act on: the pane's own Space, or
   * the selected Space in the Library view. Null with no session or Space.
   */
  space?: LibrarySpace | null;
};

const NO_PENDING_ITEMS: ReadonlySet<string> = new Set();

type DirectoryState = {
  status: "idle" | "loading" | "ready" | "error";
  data?: ContextDirectory;
  error?: string;
};

type DocumentState = {
  status: "loading" | "ready" | "error";
  document?: ContextDocument;
  error?: string;
};
const MAX_RETAINED_FILE_STATES = 64;
const MAX_RETAINED_DIRECTORY_STATES = 128;
const MAX_RETAINED_EXPANDED_DIRECTORIES = 64;
const MAX_RETAINED_DOCUMENT_BYTES = 8 * 1024 * 1024;
const MAX_PICKER_DIRECTORIES = 512;
const MAX_PICKER_FILES = 10_000;

function retainedDocumentBytes(state: DocumentState | undefined): number {
  const text = state?.document?.text;
  if (text === null || text === undefined) return 0;
  return Math.max(text.length, state?.document?.bytes ?? 0);
}
function retainDirectoryState(current: Record<string, DirectoryState>, key: string, state: DirectoryState, protectedKeys: Set<string>): Record<string, DirectoryState> {
  const next = { ...current };
  delete next[key];
  next[key] = state;
  const keys = Object.keys(next);
  if (keys.length > MAX_RETAINED_DIRECTORY_STATES) {
    const oldest = keys.find((candidate) => candidate !== key && !protectedKeys.has(candidate)) ?? keys.find((candidate) => candidate !== key) ?? keys[0];
    if (oldest !== undefined) delete next[oldest];
  }
  return next;
}

function retainDocumentState(current: Record<string, DocumentState>, key: string, state: DocumentState): Record<string, DocumentState> {
  const next = { ...current };
  delete next[key];
  next[key] = state;
  const keys = Object.keys(next);
  while (keys.length > MAX_RETAINED_FILE_STATES) {
    const oldest = keys.shift();
    if (oldest === undefined) break;
    delete next[oldest];
  }
  let bytes = Object.keys(next).reduce((total, candidate) => total + retainedDocumentBytes(next[candidate]), 0);
  if (bytes > MAX_RETAINED_DOCUMENT_BYTES) {
    for (const candidate of Object.keys(next)) {
      if (candidate === key) continue;
      bytes -= retainedDocumentBytes(next[candidate]);
      delete next[candidate];
      if (bytes <= MAX_RETAINED_DOCUMENT_BYTES) break;
    }
  }
  return next;
}


type SourceSpan = { start: number; end: number };

type DerivedMarkdown = {
  text: string;
  sourceLines: number[];
  frontmatter: SourceSpan | null;
};

function keyFor(rootId: string, path: string): string {
  return `${rootId}\u0000${path}`;
}

/** Provider items own their document and `_files/*`, not child items in their directory. */
function libraryItemHolds(item: LibraryItemSummary, path: string): boolean {
  const itemRoot = item.item_path.replace(/\/+$/, "");
  const attachmentPrefix = `${itemRoot}/_files/`;
  const relativeAttachment = path.startsWith(attachmentPrefix) ? path.slice(attachmentPrefix.length) : "";
  return item.document_path === path
    || (item.folder !== null && path.startsWith(`${itemRoot}/`))
    || (item.folder === null && relativeAttachment !== "" && !relativeAttachment.includes("/"));
}

function readableError(error: unknown): string {
  if (error instanceof Error && error.message) return error.message;
  if (typeof error === "string" && error) return error;
  if (typeof error === "object" && error !== null && "message" in error) {
    const message = error.message;
    if (typeof message === "string" && message) return message;
  }
  return "The Context request could not be completed.";
}

function isEditingTarget(target: EventTarget | null): boolean {
  return target instanceof HTMLElement && (target instanceof HTMLInputElement || target instanceof HTMLTextAreaElement || target instanceof HTMLSelectElement || target.isContentEditable);
}

function sourceLinesForMarkdown(source: string): DerivedMarkdown {
  const lines = splitSourceLines(source);
  let frontmatter: SourceSpan | null = null;
  let bodyStart = 0;
  if (lines[0]?.text.trim() === "---") {
    for (let index = 1; index < lines.length; index += 1) {
      if (lines[index].text.trim() === "---" || lines[index].text.trim() === "...") {
        frontmatter = { start: 1, end: index + 1 };
        bodyStart = index + 1;
        break;
      }
    }
  }
  const body = lines.slice(bodyStart);
  return {
    text: body.map((line) => line.text).join("\n"),
    sourceLines: body.map((_, index) => bodyStart + index + 1),
    frontmatter,
  };
}
type SourceMetadata = {
  canonicalId: string | null;
  provider: string | null;
  fetchedAt: string | null;
  /** A Confluence page's last edit (P10 `last_modified`, `last_modified_by`). */
  lastModified: string | null;
  lastModifiedBy: string | null;
};

const SOURCE_METADATA_KEYS: Record<string, true> = { canonical_id: true, fetched_at: true, provider: true, last_modified: true, last_modified_by: true };

function sourceMetadata(source: string): SourceMetadata {
  const lines = splitSourceLines(source);
  const fields: Record<string, string> = {};
  if (lines[0]?.text.trim() === "---") for (const line of lines.slice(1)) {
    if (line.text.trim() === "---" || line.text.trim() === "...") break;
    const match = /^([A-Za-z0-9_-]+):\s*(.*)$/.exec(line.text);
    if (match && SOURCE_METADATA_KEYS[match[1]]) fields[match[1]] = match[2];
  }
  // Page fields are written as JSON-quoted scalars.
  const scalar = (value: string | undefined): string | null => {
    if (!value) return null;
    try { return value.startsWith("\"") ? String(JSON.parse(value)) : value; } catch { return value; }
  };
  return { canonicalId: fields.canonical_id ?? null, provider: fields.provider ?? null, fetchedAt: fields.fetched_at ?? null, lastModified: scalar(fields.last_modified), lastModifiedBy: scalar(fields.last_modified_by) };
}

/** Preview-limit diagnostics are already explained by the bounded-window notice. */
function isShownDiagnostic(diagnostic: { code: string }): boolean {
  return diagnostic.code !== "context_preview_lines" && diagnostic.code !== "context_preview_bytes";
}

function documentName(path: string): string {
  const name = path.slice(path.lastIndexOf("/") + 1);
  return name || path;
}

function isMarkdown(document: ContextDocument, path: string): boolean {
  return /(?:^|\.)md(?:own)?$/i.test(path) || document.media_type.toLowerCase().includes("markdown");
}

function isHtml(document: ContextDocument, path: string): boolean {
  return /(?:^|\.)x?html?$/i.test(path) || document.media_type.toLowerCase().includes("html");
}

function isSafeHref(href: string | undefined): boolean {
  if (!href) return false;
  try {
    const parsed = new URL(href, "https://context.invalid");
    if (parsed.origin === "https://context.invalid" && !/^(?:\/|#|\?)/.test(href)) return false;
    return parsed.protocol === "http:" || parsed.protocol === "https:" || parsed.protocol === "mailto:";
  } catch {
    return false;
  }
}

function nodePosition(node: unknown): { start: number; end: number } | null {
  if (typeof node !== "object" || node === null || !("position" in node)) return null;
  const position = node.position;
  if (typeof position !== "object" || position === null || !("start" in position) || !("end" in position)) return null;
  const startValue = position.start;
  const endValue = position.end;
  if (typeof startValue !== "object" || startValue === null || !("line" in startValue)) return null;
  if (typeof endValue !== "object" || endValue === null || !("line" in endValue)) return null;
  const start = startValue.line;
  const end = endValue.line;
  return typeof start === "number" && typeof end === "number" ? { start, end } : null;
}

function blockData(node: unknown, mapping: number[]): { "data-source-start": number; "data-source-end": number } | undefined {
  const position = nodePosition(node);
  if (!position || mapping.length === 0) return undefined;
  const start = mapping[Math.max(0, Math.min(mapping.length - 1, position.start - 1))];
  const end = mapping[Math.max(0, Math.min(mapping.length - 1, position.end - 1))];
  return start === undefined || end === undefined ? undefined : { "data-source-start": start, "data-source-end": end };
}

function spanFromClick(event: ReactMouseEvent<HTMLDivElement>): SourceSpan | null {
  const target = event.target;
  if (!(target instanceof HTMLElement)) return null;
  const block = target.closest<HTMLElement>("[data-source-start]");
  if (!block) return null;
  const start = Number(block.dataset.sourceStart);
  const end = Number(block.dataset.sourceEnd);
  return Number.isSafeInteger(start) && Number.isSafeInteger(end) ? { start, end } : null;
}

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

export function localImagePath(documentPath: string, value: string | undefined): string | null {
  if (!value || /^(?:[A-Za-z][A-Za-z0-9+.-]*:|[/\\])/.test(value) || /[?#]/.test(value)) return null;
  let decoded: string;
  try { decoded = decodeURIComponent(value); } catch { return null; }
  if (/[\x00-\x1f\x7f\\]/.test(decoded) || decoded.startsWith("/")) return null;
  const segments = documentPath.split("/").slice(0, -1);
  for (const segment of decoded.split("/")) {
    if (!segment || segment === ".") continue;
    if (segment === "..") { if (!segments.length) return null; segments.pop(); }
    else segments.push(segment);
  }
  return segments.length ? segments.join("/") : null;
}

/** An absolute http(s) image address; other schemes are never loaded. */
function remoteImageUrl(value: string | undefined): string | null {
  if (!value) return null;
  try {
    const url = new URL(value);
    return url.protocol === "https:" || url.protocol === "http:" ? url.href : null;
  } catch {
    return null;
  }
}

/** Markdown image references to web addresses, e.g. `![badge](https://…)`. */
function countExternalImages(markdown: string): number {
  return markdown.match(/!\[[^\]]*\]\(\s*<?https?:\/\//gi)?.length ?? 0;
}

/**
 * A remote image stays blocked until the reader loads external images for the
 * document, as a mail client does. It loads without a referrer; a failure
 * leaves its alt text.
 */
function RemoteImage({ src, alt, allowed }: { src: string; alt: string; allowed: boolean }) {
  const [failed, setFailed] = useState(false);
  if (!allowed || failed) return <span className="context-media-refusal" title={src}>{alt || "image"}</span>;
  return <img className="context-safe-image" src={src} alt={alt} title={alt || undefined} referrerPolicy="no-referrer" loading="lazy" decoding="async" onError={() => setFailed(true)} />;
}

function MarkdownView({
  text,
  state,
  onSelect,
  onScroll,
  media,
}: {
  media: ContextReader["media"] | null;
  text: string;
  state: ContextFileViewState;
  onSelect: (start: number, end: number) => void;
  onScroll: (scrollTop: number) => void;
}) {
  const derived = useMemo(() => sourceLinesForMarkdown(text), [text]);
  const externalImages = useMemo(() => countExternalImages(derived.text), [derived.text]);
  const documentKey = `${state.rootId}\u0000${state.path}`;
  const [externalAllowedFor, setExternalAllowedFor] = useState<string | null>(null);
  const externalAllowed = externalAllowedFor === documentKey;
  const scrollRef = useRef<HTMLDivElement>(null);
  const restoredScrollIdentity = useRef<string | null>(null);
  const scrollIdentity = `${state.rootId}\u0000${state.path}\u0000${state.revision ?? ""}\u0000${text}`;
  useEffect(() => {
    if (restoredScrollIdentity.current === scrollIdentity) return;
    restoredScrollIdentity.current = scrollIdentity;
    if (scrollRef.current) scrollRef.current.scrollTop = state.scrollTop;
  }, [scrollIdentity, state.scrollTop]);
  useEffect(() => {
    for (const block of scrollRef.current?.querySelectorAll<HTMLElement>("[data-source-start]") ?? []) {
      const selected = Number(block.dataset.sourceStart) === state.selectionStart && Number(block.dataset.sourceEnd) === state.selectionEnd;
      block.classList.toggle("is-selected-block", selected);
    }
  }, [state.selectionStart, state.selectionEnd]);
  const components: Components = useMemo(() => ({
    p: ({ node, children, ...props }) => <p {...props} {...blockData(node, derived.sourceLines)}>{children}</p>,
    h1: ({ node, children, ...props }) => <h1 {...props} {...blockData(node, derived.sourceLines)}>{children}</h1>,
    h2: ({ node, children, ...props }) => <h2 {...props} {...blockData(node, derived.sourceLines)}>{children}</h2>,
    h3: ({ node, children, ...props }) => <h3 {...props} {...blockData(node, derived.sourceLines)}>{children}</h3>,
    h4: ({ node, children, ...props }) => <h4 {...props} {...blockData(node, derived.sourceLines)}>{children}</h4>,
    h5: ({ node, children, ...props }) => <h5 {...props} {...blockData(node, derived.sourceLines)}>{children}</h5>,
    h6: ({ node, children, ...props }) => <h6 {...props} {...blockData(node, derived.sourceLines)}>{children}</h6>,
    blockquote: ({ node, children, ...props }) => <blockquote {...props} {...blockData(node, derived.sourceLines)}>{children}</blockquote>,
    ul: ({ node, children, ...props }) => <ul {...props} {...blockData(node, derived.sourceLines)}>{children}</ul>,
    ol: ({ node, children, ...props }) => <ol {...props} {...blockData(node, derived.sourceLines)}>{children}</ol>,
    pre: ({ node, children, ...props }) => {
      const code = node?.children.find(child => child.type === "element" && child.tagName === "code");
      if (code?.type === "element" && Array.isArray(code.properties.className) && code.properties.className.includes("language-mermaid")) {

        const source = code.children.filter(child => child.type === "text").map(child => child.type === "text" ? child.value : "").join("");
        return <div {...blockData(node, derived.sourceLines)}>{code.properties.dataMermaidPreview === true ? <MermaidView source={source} /> : <><p>Only the first four diagrams are previewed.</p><pre>{children}</pre></>}</div>;
      }
      return <pre {...props} {...blockData(node, derived.sourceLines)}>{children}</pre>;
    },
    a: ({ node, href, children, ...props }) => isSafeHref(href)
      ? <span {...props} title={href} className="context-link-reference" {...blockData(node, derived.sourceLines)}>{children}</span>
      : <span {...props} {...blockData(node, derived.sourceLines)}>{children}</span>,
    img: ({ node, alt, src }) => {
      const path = localImagePath(state.path, src);
      const remote = !path && remoteImageUrl(src);
      return <span {...blockData(node, derived.sourceLines)}>{path && media ? <SafeImage media={media} request={{ root_id: state.rootId, path, expected_revision: null }} alt={alt ?? "Context image"} className="context-safe-image" /> : remote ? <RemoteImage src={remote} alt={alt ?? ""} allowed={externalAllowed} /> : <span className="context-media-refusal">{alt || "image"}</span>}</span>;
    },
  }), [derived.sourceLines, derived.text, externalAllowed, media, state.rootId, state.path]);
  return (
    <div className="context-markdown-scroll" ref={scrollRef} onScroll={(event) => onScroll(event.currentTarget.scrollTop)} onClick={(event) => {
      if (event.target instanceof Element && event.target.closest("dialog, button, textarea")) return;
      const span = spanFromClick(event);
      if (span) onSelect(span.start, span.end);
    }}>
      {externalImages > 0 && !externalAllowed ? <div className="context-external-images" role="status"><span>{externalImages === 1 ? "1 external image" : `${externalImages} external images`} not loaded</span><button type="button" onClick={(event) => { event.stopPropagation(); setExternalAllowedFor(documentKey); }}>Load external images</button></div> : null}
      <article className="context-markdown-body">
        <ReactMarkdown skipHtml remarkPlugins={[remarkGfm, remarkBoundDiagrams]} components={components}>{derived.text}</ReactMarkdown>
      </article>
    </div>
  );
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
type ContextTreeRow = {
  entry: ContextEntry;
  path: string;
  depth: number;
  label: string;
  open: boolean;
};

function isTreeRowEnabled(row: ContextTreeRow): boolean {
  return (row.entry.kind === "file" || row.entry.kind === "directory") && !row.entry.refusal;
}

function contextTreeRows(root: ContextRoot, directories: Record<string, DirectoryState>, expanded: Set<string>): ContextTreeRow[] {
  const rows: ContextTreeRow[] = [];
  const visit = (path: string, depth: number) => {
    const state = directories[keyFor(root.root_id, path)];
    if (!state?.data) return;
    for (const initial of state.data.entries) {
      let entry = initial;
      let entryPath = entry.path ?? (path ? `${path}/${entry.name}` : entry.name);
      let label = entry.name;
      // Compress only directories whose next segment is already loaded and open.
      // This never reads descendants merely to improve presentation.
      while (entry.kind === "directory" && expanded.has(entryPath)) {
        const child = directories[keyFor(root.root_id, entryPath)]?.data;
        if (!child || child.entries.length !== 1 || child.entries[0]?.kind !== "directory") break;
        entry = child.entries[0];
        entryPath = entry.path ?? `${entryPath}/${entry.name}`;
        label += `/${entry.name}`;
      }
      const open = entry.kind === "directory" && expanded.has(entryPath);
      rows.push({ entry, path: entryPath, depth, label, open });
      if (open) visit(entryPath, depth + 1);
    }
  };
  visit("", 0);
  return rows;
}

export function ContextViewer({ client, presentation, value, onChange, controlAllowed, onRequestControl, onTerminalView, library: viewLibrary, libraryCommand = null, space = null }: ContextViewerProps) {
  const [directories, setDirectories] = useState<Record<string, DirectoryState>>({});
  const [documents, setDocuments] = useState<Record<string, DocumentState>>({});
  const [documentPageLoading, setDocumentPageLoading] = useState<string | null>(null);
  const [expanded, setExpanded] = useState<Set<string>>(() => new Set());
  const expandedRef = useRef(expanded);
  expandedRef.current = expanded;
  const protectedDirectoryKeysRef = useRef<Set<string>>(new Set());
  const [refreshGeneration, setRefreshGeneration] = useState(0);
  const [invalidationGeneration, setInvalidationGeneration] = useState(0);
  const [rootId, setRootId] = useState<string | null>(value.rootId ?? (presentation ? presentation.default_root_id ?? presentation.roots[0]?.root_id ?? null : LIBRARY_ROOT_ID));
  const [commentStatus, setCommentStatus] = useState({ count: 0, canCreateLines: false, canCreateWholeFile: false });
  const [pickerOpen, setPickerOpen] = useState(false);
  const [resourcesOpen, setResourcesOpen] = useState(false);
  const [pickerIndex, setPickerIndex] = useState({ loading: false, incomplete: false, entries: new Map<string, ContextEntry>() });
  const directoriesRef = useRef(directories);
  directoriesRef.current = directories;
  const directoryRequests = useRef<Record<string, number>>({});
  const directoryControllers = useRef<Record<string, AbortController>>({});
  const directoryRequestSequence = useRef(0);
  const documentRequestSequence = useRef(0);
  const documentController = useRef<AbortController | null>(null);
  const pickerController = useRef<AbortController | null>(null);
  const pickerGeneration = useRef(0);
  const viewerRef = useRef<HTMLElement>(null);
  const overview = useFileOverview(viewerRef);
  const overviewId = useId();
  const tree = useTreeWidth();
  const [wrap, toggleWrap] = useWrapPreference();
  const documentRef = useRef<HTMLElement>(null);
  const treeRef = useRef<HTMLElement>(null);
  const treeFocusPathRef = useRef<{ path: string; restoreAfterLoad: boolean } | null>(null);
  const mountedRef = useRef(true);
  useEffect(() => {
    mountedRef.current = true;
    return () => {
      mountedRef.current = false;
      for (const controller of Object.values(directoryControllers.current)) controller.abort();
      directoryControllers.current = {};
      directoryRequests.current = {};
      documentController.current?.abort();
      documentController.current = null;
      documentRequestSequence.current += 1;
      pickerController.current?.abort();
      pickerGeneration.current += 1;
    };
  }, []);
  // The Library is a client-side root in every pane and the only root of the Library view.
  const libraryChosen = presentation === null || rootId === LIBRARY_ROOT_ID;
  const paneLibrary = useLibraryListing(client, viewLibrary === undefined && libraryChosen);
  const library = viewLibrary ?? paneLibrary;
  const serverLibraryRoot = library.listing?.root ?? null;
  const libraryPath = serverLibraryRoot?.path ?? "";
  const libraryRoot = useMemo<ContextRoot>(() => ({ root_id: LIBRARY_ROOT_ID, kind: "library", label: "Library", path: libraryPath, repository_id: "", checkout_path: "", companion_id: null }), [libraryPath]);
  const roots = useMemo(() => presentation ? [...presentation.roots, libraryRoot] : [libraryRoot], [libraryRoot, presentation]);
  const root: ContextRoot = libraryChosen || !presentation ? libraryRoot : presentation.roots.find((candidate) => candidate.root_id === rootId) ?? presentation.roots[0];
  const isLibrary = root?.kind === "library";
  const serverLibraryRootId = serverLibraryRoot?.root_id ?? null;
  const sessionId = presentation?.session_id ?? null;
  const paneId = presentation?.pane_id ?? null;
  const paneBindingId = presentation?.binding_id ?? null;
  // Readers follow authority identity, not each refreshed listing or presentation object.
  const reader = useMemo<ContextReader | null>(() => {
    if (isLibrary) return serverLibraryRoot ? libraryReader(client, serverLibraryRoot) : null;
    return presentation ? paneReader(client, presentation) : null;
  }, [client, isLibrary, serverLibraryRootId, sessionId, paneId, paneBindingId]);
  const bindingId = isLibrary ? "library" : paneBindingId ?? "";
  const activeRootId = root?.root_id ?? "";
  const identityKey = `${reader?.identity ?? "pending"}\u0000${activeRootId}`;
  const requestIdentityRef = useRef(identityKey);
  requestIdentityRef.current = identityKey;
  const currentBindingRef = useRef(bindingId);
  currentBindingRef.current = bindingId;
  const currentRootRef = useRef(activeRootId);
  currentRootRef.current = activeRootId;
  const selectedPath = value.rootId === root?.root_id ? value.path : null;
  const selectedKey = root && selectedPath ? keyFor(root.root_id, selectedPath) : null;
  const selectedFileState = selectedKey ? value.files[selectedKey] : undefined;
  const directoryPathForFile = selectedPath?.includes("/") ? selectedPath.slice(0, selectedPath.lastIndexOf("/")) : "";
  const selectedDirectory = root ? directories[keyFor(root.root_id, directoryPathForFile)] : undefined;
  const selectedEntry = selectedDirectory?.data?.entries.find((entry) => (entry.path ?? (directoryPathForFile ? `${directoryPathForFile}/${entry.name}` : entry.name)) === selectedPath);
  // Library files are replaced by provider refreshes; always read the current revision.
  const selectedRevision = isLibrary ? null : selectedEntry?.revision ?? selectedFileState?.revision ?? null;
  const documentState = selectedKey ? documents[selectedKey] : undefined;
  const protectedDirectoryKeys = new Set<string>();
  if (root) {
    protectedDirectoryKeys.add(keyFor(root.root_id, ""));
    protectedDirectoryKeys.add(keyFor(root.root_id, directoryPathForFile));
    for (const path of expanded) protectedDirectoryKeys.add(keyFor(root.root_id, path));
  }
  protectedDirectoryKeysRef.current = protectedDirectoryKeys;
  const document = documentState?.document;
  const loadDocumentPage = useCallback(async () => {
    if (!reader || !root || !selectedPath || !selectedKey || !document || document.next_offset === undefined) return;
    const expectedOffset = document.next_offset;
    const requestKey = `${selectedKey}\u0000${document.revision}\u0000${expectedOffset}`;
    const controller = new AbortController();
    documentController.current?.abort();
    documentController.current = controller;
    setDocumentPageLoading(requestKey);
    try {
      const data = await reader.document({
        root_id: root.root_id,
        path: selectedPath,
        expected_revision: document.revision,
        offset: expectedOffset,
      }, controller.signal);
      if (controller.signal.aborted) return;
      const offset = data.offset ?? 0;
      if (data.revision !== document.revision || offset !== expectedOffset || data.text === null) throw new Error("Source changed while loading the next page; refresh to revalidate it.");
      setDocuments((current) => retainDocumentState(current, selectedKey, {
        status: "ready",
        document: {
          ...data,
          text: `${document.text ?? ""}${data.text}`,
          offset: 0,
          next_offset: data.next_offset,
          truncated: data.truncated,
          content_hash: data.truncated ? null : data.content_hash,
        },
      }));
    } catch (error) {
      if (!controller.signal.aborted) setDocuments((current) => retainDocumentState(current, selectedKey, { status: "error", document: current[selectedKey]?.document ?? document, error: readableError(error) }));
    } finally {
      if (documentController.current === controller) documentController.current = null;
      setDocumentPageLoading((current) => current === requestKey ? null : current);
    }
  }, [document, reader, root, selectedKey, selectedPath]);
  const knownRevisions = useMemo<ContextKnownRevision[]>(() => Object.values(documents)
    .map((state) => state.document)
    .filter((candidate): candidate is ContextDocument => candidate !== undefined && candidate.root_id === activeRootId)
    .map((candidate) => ({ path: candidate.path, revision: candidate.revision }))
    .sort((left, right) => left.path.localeCompare(right.path)), [activeRootId, documents]);
  const treeRows = useMemo(() => root ? contextTreeRows(root, directories, expanded) : [], [directories, expanded, root]);
  const discoveryDiagnostics = useMemo(() => {
    const seen = new Set<string>();
    const rootDiagnostics = isLibrary ? library.listing?.diagnostics ?? [] : presentation?.diagnostics ?? [];
    return [...rootDiagnostics, ...(root ? directories[keyFor(root.root_id, "")]?.data?.diagnostics ?? [] : [])].filter((diagnostic) => {
      const key = `${diagnostic.code}\u0000${diagnostic.message}\u0000${diagnostic.path ?? ""}`;
      if (seen.has(key)) return false;
      seen.add(key);
      return true;
    });
  }, [directories, isLibrary, library.listing?.diagnostics, presentation?.diagnostics, root]);
  // The Library root has no comments: a Library path is outside every Space.
  const commentsEnabled = Boolean(presentation && root && (root.kind === "companion" || (root.kind === "folder" && root.root_id === presentation.default_root_id)));
  const restoreSourceFocus = useCallback(() => {
    requestAnimationFrame(() => {
      const line = selectedFileState?.selectionEnd ?? selectedFileState?.selectionStart ?? 1;
      documentRef.current?.querySelector<HTMLButtonElement>(`[data-line="${line}"]`)?.focus();
    });
  }, [selectedFileState?.selectionEnd, selectedFileState?.selectionStart]);
  const updateCommentStatus = useCallback((next: { count: number; canCreateLines: boolean; canCreateWholeFile: boolean }) => {
    setCommentStatus((current) => current.count === next.count && current.canCreateLines === next.canCreateLines && current.canCreateWholeFile === next.canCreateWholeFile ? current : next);
  }, []);

  const updateFile = useCallback((patch: Partial<ContextFileViewState>) => {
    if (!root || !selectedPath || !selectedKey) return;
    const previous = value.files[selectedKey] ?? { rootId: root.root_id, path: selectedPath, mode: "source" as const, selectionStart: null, selectionEnd: null, scrollTop: 0, revision: selectedRevision };
    const files = { ...value.files };
    delete files[selectedKey];
    files[selectedKey] = { ...previous, ...patch };
    const keys = Object.keys(files);
    while (keys.length > MAX_RETAINED_FILE_STATES) {
      const oldest = keys.shift();
      if (oldest === undefined) break;
      delete files[oldest];
    }
    onChange({ ...value, rootId: root.root_id, path: selectedPath, files });
  }, [onChange, root, selectedKey, selectedPath, value]);
  useEffect(() => {
    if (!selectedKey || !document || !selectedFileState || selectedFileState.revision === document.revision) return;
    updateFile({ revision: document.revision, selectionStart: null, selectionEnd: null, scrollTop: 0 });
  }, [document?.revision, selectedFileState, selectedKey, updateFile]);
  useEffect(() => {
    const editor = value.commentEditor;
    if (!root || !editor || editor.editor !== "lines" || editor.rootId !== root.root_id) return;
    if (selectedPath !== editor.path) {
      const editorKey = keyFor(root.root_id, editor.path);
      const current = value.files[editorKey] ?? { rootId: root.root_id, path: editor.path, mode: "source" as const, selectionStart: null, selectionEnd: null, scrollTop: 0, revision: editor.revision };
      onChange({ ...value, rootId: root.root_id, path: editor.path, files: { ...value.files, [editorKey]: { ...current, mode: "source", selectionStart: editor.selection?.start ?? current.selectionStart, selectionEnd: editor.selection?.end ?? current.selectionEnd } } });
      return;
    }
    if (selectedFileState?.mode !== "source") updateFile({ mode: "source" });
  }, [onChange, root, selectedFileState?.mode, selectedPath, updateFile, value]);

  const loadDirectory = useCallback(async (directoryRoot: ContextRoot, path: string, force = false) => {
    if (!reader) return;
    const key = keyFor(directoryRoot.root_id, path);
    const previous = directoriesRef.current[key]?.data;
    if (!force && directoriesRef.current[key]?.status === "ready" && previous?.next_offset === undefined) return;
    directoryControllers.current[key]?.abort();
    const requestId = ++directoryRequestSequence.current;
    directoryRequests.current[key] = requestId;
    setDirectories((current) => retainDirectoryState(current, key, { status: "loading", data: current[key]?.data }, protectedDirectoryKeysRef.current));
    const controller = new AbortController();
    directoryControllers.current[key] = controller;
    const requestIdentity = identityKey;
    const requestBindingId = bindingId;
    const requestRootId = directoryRoot.root_id;
    try {
      const request: ContextDirectoryRead = {
        root_id: requestRootId,
        path,
        offset: force ? undefined : previous?.next_offset,
        revision: force ? undefined : previous?.revision,
      };
      const data = await reader.directory(request, controller.signal);
      if (!mountedRef.current || controller.signal.aborted || directoryRequests.current[key] !== requestId
        || requestIdentityRef.current !== requestIdentity || currentBindingRef.current !== requestBindingId || currentRootRef.current !== requestRootId) return;
      const merged = !force && previous?.revision !== undefined && data.revision === previous.revision
        ? { ...data, entries: [...previous.entries, ...data.entries] }
        : data;
      setDirectories((current) => retainDirectoryState(current, key, { status: "ready", data: merged }, protectedDirectoryKeysRef.current));
    } catch (error) {
      if (!mountedRef.current || controller.signal.aborted || directoryRequests.current[key] !== requestId
        || requestIdentityRef.current !== requestIdentity || currentBindingRef.current !== requestBindingId || currentRootRef.current !== requestRootId) return;
      setDirectories((current) => retainDirectoryState(current, key, { status: "error", data: current[key]?.data, error: readableError(error) }, protectedDirectoryKeysRef.current));
    } finally {
      if (directoryControllers.current[key] === controller) delete directoryControllers.current[key];
    }
  }, [bindingId, identityKey, reader]);
  const identityRef = useRef<string | null>(null);
  useEffect(() => {
    if (identityRef.current === identityKey) return;
    identityRef.current = identityKey;
    for (const controller of Object.values(directoryControllers.current)) controller.abort();
    directoryControllers.current = {};
    directoryRequests.current = {};
    directoriesRef.current = {};
    setDirectories({});
    documentController.current?.abort();
    documentController.current = null;
    setDocuments({});
    setExpanded(new Set());
    documentRequestSequence.current += 1;
    pickerController.current?.abort();
    pickerGeneration.current += 1;
    setResourcesOpen(false);
    setPickerOpen(false);
    overview.reset();
    setPickerIndex({ loading: false, incomplete: false, entries: new Map() });
    if (root && value.rootId !== root.root_id) {
      onChange({ ...value, rootId: root.root_id, path: null });
    }
    // The Library tree comes from its listing, not from directory reads.
    if (root && !isLibrary) void loadDirectory(root, "", true);
  }, [identityKey, isLibrary, loadDirectory, onChange, root, value]);

  useEffect(() => {
    if (!reader || !root || !selectedPath || !selectedKey) return;
    documentController.current?.abort();
    const requestId = ++documentRequestSequence.current;
    const controller = new AbortController();
    documentController.current = controller;
    const requestIdentity = identityKey;
    const requestBindingId = bindingId;
    const requestRootId = root.root_id;
    setDocumentPageLoading(null);
    setDocuments((current) => retainDocumentState(current, selectedKey, { status: "loading", document: current[selectedKey]?.document }));
    const request: ContextDocumentRead = { root_id: requestRootId, path: selectedPath, expected_revision: selectedRevision };
    void reader.document(request, controller.signal).then((data) => {
      if (!mountedRef.current || controller.signal.aborted || requestId !== documentRequestSequence.current
        || requestIdentityRef.current !== requestIdentity || currentBindingRef.current !== requestBindingId || currentRootRef.current !== requestRootId) return;
      setDocuments((current) => retainDocumentState(current, selectedKey, { status: "ready", document: data }));
    }).catch((error: unknown) => {
      if (!mountedRef.current || controller.signal.aborted || requestId !== documentRequestSequence.current
        || requestIdentityRef.current !== requestIdentity || currentBindingRef.current !== requestBindingId || currentRootRef.current !== requestRootId) return;
      setDocuments((current) => retainDocumentState(current, selectedKey, { status: "error", document: current[selectedKey]?.document, error: readableError(error) }));
    });
    return () => {
      controller.abort();
      if (documentController.current === controller) documentController.current = null;
      documentRequestSequence.current += 1;
    };
  }, [bindingId, identityKey, reader, refreshGeneration, selectedKey, selectedPath, selectedRevision, root?.root_id]);

  const chooseRoot = (nextRoot: ContextRoot) => {
    setRootId(nextRoot.root_id);
    setExpanded(new Set());
    onChange({ ...value, rootId: nextRoot.root_id, path: null });
  };
  const chooseEntry = (entry: ContextEntry) => {
    if (entry.kind !== "file" || entry.refusal || !entry.path) return;
    openFile(entry.path, entry.revision);
  };
  const openFile = (path: string, revision: string | null) => {
    if (!root) return;
    const fileKey = keyFor(root.root_id, path);
    const next = value.files[fileKey] ?? { rootId: root.root_id, path, mode: "auto" as const, selectionStart: null, selectionEnd: null, scrollTop: 0, revision };
    const files = { ...value.files };
    delete files[fileKey];
    files[fileKey] = next;
    const keys = Object.keys(files);
    while (keys.length > MAX_RETAINED_FILE_STATES) {
      const oldest = keys.shift();
      if (oldest === undefined) break;
      delete files[oldest];
    }
    onChange({ ...value, rootId: root.root_id, path, files });
    overview.select();
  };
  const focusTree = useCallback(() => {
    overview.show();
    requestAnimationFrame(() => {
      const buttons = treeRef.current?.querySelectorAll<HTMLButtonElement>("[data-context-path], [data-library-row]");
      const selected = selectedPath ? [...(buttons ?? [])].find((button) => button.dataset.contextPath === selectedPath) : null;
      (selected ?? buttons?.[0] ?? treeRef.current)?.focus();
    });
  }, [selectedPath]);
  const focusContent = useCallback(() => {
    requestAnimationFrame(() => documentRef.current?.focus());
  }, []);
  const closeFilePicker = useCallback(() => {
    pickerController.current?.abort();
    pickerController.current = null;
    pickerGeneration.current += 1;
    setPickerOpen(false);
    setPickerIndex({ loading: false, incomplete: false, entries: new Map() });
  }, []);
  const openFilePicker = useCallback(() => {
    if (!root || !reader) return;
    pickerController.current?.abort();
    const controller = new AbortController();
    pickerController.current = controller;
    const generation = ++pickerGeneration.current;
    const requestIdentity = identityKey;
    setPickerOpen(true);
    setPickerIndex({ loading: true, incomplete: false, entries: new Map() });
    void (async () => {
      const pending = [""];
      const visited = new Set<string>();
      const entries = new Map<string, ContextEntry>();
      let incomplete = false;
      while (pending.length > 0 && visited.size < MAX_PICKER_DIRECTORIES && entries.size < MAX_PICKER_FILES && !controller.signal.aborted) {
        const path = pending.shift()!;
        if (visited.has(path)) continue;
        visited.add(path);
        try {
          let offset: number | undefined;
          let revision: string | undefined;
          do {
            const data = await reader.directory({ root_id: root.root_id, path, offset, revision }, controller.signal);
            if (controller.signal.aborted || pickerGeneration.current !== generation || requestIdentityRef.current !== requestIdentity || data.binding_id !== bindingId || data.root_id !== root.root_id) return;
            incomplete ||= data.truncated;
            for (const entry of data.entries) {
              if (!entry.path || entry.path.split("/").includes(".cockpit")) continue;
              if (entry.kind === "directory") pending.push(entry.path);
              if (entry.kind === "file" && !entry.refusal) {
                if (entries.has(entry.path) || entries.size < MAX_PICKER_FILES) entries.set(entry.path, entry);
                else incomplete = true;
              }
            }
            offset = data.next_offset;
            revision = data.revision;
            setPickerIndex({ loading: true, incomplete, entries: new Map(entries) });
          } while (offset !== undefined && !controller.signal.aborted && entries.size < MAX_PICKER_FILES);
        } catch {
          if (controller.signal.aborted || pickerGeneration.current !== generation) return;
          incomplete = true;
        }
      }
      if (pending.length > 0 || visited.size >= MAX_PICKER_DIRECTORIES || entries.size >= MAX_PICKER_FILES) incomplete = true;
      if (!controller.signal.aborted && pickerGeneration.current === generation && requestIdentityRef.current === requestIdentity) setPickerIndex({ loading: false, incomplete, entries });
    })();
  }, [bindingId, identityKey, reader, root]);
  const toggleDirectory = (entry: ContextEntry) => {
    if (!root || !entry.path || entry.kind !== "directory") return;
    const next = new Set(expanded);
    if (next.has(entry.path)) next.delete(entry.path); else {
      next.add(entry.path);
      if (next.size > MAX_RETAINED_EXPANDED_DIRECTORIES) {
        const oldest = next.values().next().value;
        if (typeof oldest === "string") next.delete(oldest);
      }
      void loadDirectory(root, entry.path);
    }
    setExpanded(next);
  };
  // Rereads files on disk (and the Library index); never a provider refresh.
  const refresh = () => {
    if (!root) return;
    setRefreshGeneration((generation) => generation + 1);
    // Space copies edited or deleted on disk change their state; reread it with the files.
    spaceListing.reload();
    if (isLibrary) { library.reload(); return; }
    const path = selectedPath ? directoryPathForFile : "";
    void loadDirectory(root, path, true);
  };
  const focusTreePath = (path: string, restoreAfterLoad = false) => {
    treeFocusPathRef.current = { path, restoreAfterLoad };
    requestAnimationFrame(() => {
      const request = treeFocusPathRef.current;
      if (!request || request.path !== path) return;
      const target = [...(treeRef.current?.querySelectorAll<HTMLButtonElement>("[data-context-path]") ?? [])]
        .find((button) => button.dataset.contextPath === request.path);
      target?.focus();
      if (!request.restoreAfterLoad) treeFocusPathRef.current = null;
    });
  };
  useEffect(() => {
    const request = treeFocusPathRef.current;
    if (!request?.restoreAfterLoad || !root) return;
    const state = directories[keyFor(root.root_id, request.path)];
    if (!state || state.status === "loading") return;
    const activeElement = globalThis.document.activeElement;
    if (activeElement !== globalThis.document.body && !treeRef.current?.contains(activeElement)) {
      treeFocusPathRef.current = null;
      return;
    }
    const buttons = [...(treeRef.current?.querySelectorAll<HTMLButtonElement>("[data-context-path]") ?? [])];
    const target = buttons.find((button) => button.dataset.contextPath === request.path)
      ?? buttons.find((button) => button.dataset.contextPath?.startsWith(`${request.path}/`));
    target?.focus();
    treeFocusPathRef.current = null;
  }, [directories, root, treeRows]);
  const onTreeKeyDown = (event: ReactKeyboardEvent<HTMLElement>) => {
    if (event.ctrlKey || event.metaKey || event.altKey || !(event.target instanceof HTMLElement)) return;
    const current = event.target.closest<HTMLButtonElement>("[data-context-path]");
    if (!current) return;
    const index = treeRows.findIndex((row) => row.path === current.dataset.contextPath);
    const row = treeRows[index];
    if (!row) return;
    if (event.key === "ArrowDown" || event.key === "ArrowUp" || event.key === "Home" || event.key === "End") {
      event.preventDefault();
      const enabledRows = treeRows.filter(isTreeRowEnabled);
      const currentIndex = enabledRows.findIndex((candidate) => candidate.path === row.path);
      const next = event.key === "Home" ? enabledRows[0] : event.key === "End" ? enabledRows.at(-1)
        : enabledRows[Math.max(0, Math.min(enabledRows.length - 1, currentIndex + (event.key === "ArrowDown" ? 1 : -1)))];
      focusTreePath(next?.path ?? row.path);
      return;
    }
    if (row.entry.kind === "directory" && event.key === "ArrowRight") {
      event.preventDefault();
      if (!row.open) {
        focusTreePath(row.path, true);
        toggleDirectory(row.entry);
      } else if (treeRows[index + 1]?.depth > row.depth) {
        focusTreePath(treeRows[index + 1]!.path);
      }
      return;
    }
    if (event.key === "ArrowLeft") {
      event.preventDefault();
      if (row.entry.kind === "directory" && row.open) {
        focusTreePath(row.path);
        toggleDirectory(row.entry);
      } else {
        const parent = [...treeRows.slice(0, index)].reverse().find((candidate) => candidate.entry.kind === "directory" && candidate.depth < row.depth);
        focusTreePath(parent?.path ?? row.path);
      }
      return;
    }
    if (event.key === "Enter") {
      event.preventDefault();
      if (row.entry.kind === "directory") toggleDirectory(row.entry); else chooseEntry(row.entry);
    }
  };
  const searchContext = useCallback((request: Parameters<CockpitClient["contextSearch"]>[2], signal: AbortSignal) =>
    reader?.search ? reader.search(request, signal) : Promise.reject(new Error("Search is unavailable for this root.")), [reader]);
  const pollContext = useCallback((request: Parameters<CockpitClient["contextInvalidate"]>[2], signal: AbortSignal) =>
    reader?.invalidate ? reader.invalidate(request, signal) : Promise.reject(new Error("Change polling is unavailable for this root.")), [reader]);
  const [libraryPendingIds, setLibraryPendingIds] = useState<ReadonlySet<string>>(NO_PENDING_ITEMS);
  const [libraryReportVerb, setLibraryReportVerb] = useState<"Refresh" | "Replace">("Refresh");
  const [libraryReportDismissed, setLibraryReportDismissed] = useState(false);
  const [libraryAdd, setLibraryAdd] = useState<"library" | "space" | null>(null);
  const [libraryConfirm, setLibraryConfirm] = useState<{ kind: "remove" | "replace"; item: LibraryItemSummary } | null>(null);
  const [libraryToolbarMenu, setLibraryToolbarMenu] = useState<{ x: number; y: number } | null>(null);
  const [libraryOpenRequest, setLibraryOpenRequest] = useState<string | null>(null);
  const handledLibraryCommand = useRef<number | null>(null);
  const libraryOperation = useLibraryOperation(client, () => {
    setRefreshGeneration((generation) => generation + 1);
  });
  const startLibraryOperation = libraryOperation.start;
  const [attachmentRequest, setAttachmentRequest] = useState<LibraryAttachmentRequest | null>(null);
  const listingRef = useRef(library.listing);
  listingRef.current = library.listing;
  const [beforeAttachments, setBeforeAttachments] = useState<LibraryListingState["listing"] | undefined>(undefined);
  const attachmentOperation = useLibraryOperation(client, () => setBeforeAttachments(listingRef.current));
  const attachmentBusy = attachmentOperation.running || attachmentOperation.starting;
  const libraryBusy = libraryOperation.running || libraryOperation.starting || attachmentBusy;
  const pendingItemIds = useMemo(() => new Set([...libraryOperation.pendingItemIds, ...(libraryOperation.running || libraryOperation.starting ? libraryPendingIds : []), ...(attachmentBusy && attachmentRequest ? [attachmentRequest.item_id] : [])]), [libraryOperation.running, libraryOperation.starting, libraryOperation.pendingItemIds, libraryPendingIds, attachmentBusy, attachmentRequest]);
  const libraryItems = library.listing?.items;
  const [attachmentNotice, setAttachmentNotice] = useState<{ itemId: string; attachmentId: string } | null>(null);
  const noticeItem = isLibrary ? libraryItems?.find((item) => item.item_id === attachmentNotice?.itemId) : null;
  const noticeAttachment = noticeItem?.attachments.find((attachment) => attachment.attachment_id === attachmentNotice?.attachmentId);
  const selectedAttachmentId = noticeAttachment?.attachment_id ?? (isLibrary && selectedPath ? libraryItems?.flatMap((item) => item.attachments.filter((attachment) => attachmentPath(item, attachment) === selectedPath)).at(0)?.attachment_id : null);
  const selectedLibraryItem = isLibrary && selectedPath && !noticeAttachment ? libraryItems?.find((item) => item.document_path === selectedPath) ?? null : null;
  useEffect(() => { setAttachmentNotice(null); }, [selectedPath, isLibrary]);
  useEffect(() => {
    if (!isLibrary) return;
    // A Space add or update copies saved items out of the Library without changing them; rereading
    // the open document would replace its header, and the Space action's focus, with `Loading source…`.
    const changed = (event: Event) => {
      const kind = (event as CustomEvent<LibraryOperation | null>).detail?.kind;
      if (kind === "space_add" || kind === "space_update") return;
      setRefreshGeneration((generation) => generation + 1);
    };
    window.addEventListener(LIBRARY_CHANGED_EVENT, changed);
    return () => window.removeEventListener(LIBRARY_CHANGED_EVENT, changed);
  }, [isLibrary]);
  // A Space copy into this companion root expands the folders it wrote;
  // an update rereads the folders on show and leaves navigation as it was.
  const companionRoot = root?.kind === "companion" ? root : null;
  useEffect(() => {
    if (!companionRoot) return;
    const changed = (event: Event) => {
      const operation = (event as CustomEvent<LibraryOperation | null>).detail;
      // A removal (a Space copy or a Library item) may have deleted files shown here.
      if (!operation) {
        void loadDirectory(companionRoot, "", true);
        for (const path of expandedRef.current) void loadDirectory(companionRoot, path, true);
        return;
      }
      const result = operation.space;
      if (!result || result.companion_root_id !== companionRoot.root_id || result.written.length === 0) return;
      const paths = result.written.flatMap((file) => { const parts = file.split("/"); return parts.slice(0, -1).map((_, index) => parts.slice(0, index + 1).join("/")); });
      const updating = operation.kind === "space_update";
      if (!updating) setExpanded((current) => new Set([...current, ...paths]));
      void loadDirectory(companionRoot, "", true);
      for (const path of new Set(paths)) if (!updating || expandedRef.current.has(path)) void loadDirectory(companionRoot, path, true);
    };
    window.addEventListener(LIBRARY_CHANGED_EVENT, changed);
    return () => window.removeEventListener(LIBRARY_CHANGED_EVENT, changed);
  }, [companionRoot, loadDirectory]);
  // Space-targeted actions need a live Herdr session (design §4.1).
  const spaceLive = space?.live ? space : null;
  const spaceListing = useSpaceContextListing(client, spaceLive?.target ?? null, spaceLive !== null && (isLibrary || companionRoot !== null));
  // A finished add reads as adding until the listing reread after it arrives.
  const spaceListingRef = useRef(spaceListing.listing);
  spaceListingRef.current = spaceListing.listing;
  const [listingBeforeSpaceAdd, setListingBeforeSpaceAdd] = useState<SpaceContextListing | null | undefined>(undefined);
  const spaceAdd = useLibraryOperation(client, () => setListingBeforeSpaceAdd(spaceListingRef.current));
  const startSpaceOperation = spaceAdd.start;
  const [spaceAddRequest, setSpaceAddRequest] = useState<{ itemId: string; target: SpaceTarget } | null>(null);
  const startSpaceAdd = (item: LibraryItemSummary) => {
    if (!spaceLive) return;
    const target = spaceLive.target;
    setSpaceAddRequest({ itemId: item.item_id, target });
    void startSpaceOperation(() => client.librarySpaceAdd({ target, item_ids: [item.item_id], follow_ids: [] }));
  };
  // A copy can fail before its durable attempt is written (attempt limit, interrupted worker).
  const stoppedPhase = spaceAdd.operation?.finished ? spaceAdd.operation.phases.find((phase) => phase.phase === "space" && phase.state === "failed") : undefined;
  const spaceAddStopped = stoppedPhase && spaceLive ? spaceAddFailure(stoppedPhase.error, spaceLive.label) : null;
  const itemSpace = (item: LibraryItemSummary): ItemSpaceState | null => {
    const listing = spaceListing.listing;
    if (!spaceLive || !listing) return null;
    const mine = spaceAddRequest?.itemId === item.item_id && sameSpaceTarget(spaceAddRequest.target, spaceLive.target);
    const settling = listingBeforeSpaceAdd !== undefined && listing === listingBeforeSpaceAdd && spaceListing.status !== "error";
    const row = listing.rows.find((candidate) => candidate.item_id === item.item_id);
    // A local failure stands only while the Space still lacks the copy; another surface may have added it since.
    const stillMissing = headerSpaceAction(row, spaceLive.label).actions.some((action) => action.kind === "add");
    // `Update`, a confirmed replace or removal, and the Library version (D23); `Add to Library again` has no flow here.
    const copyActions = row ? headerSpaceAction(row, spaceLive.label).actions.filter((action) => action.kind === "view_library" || spaceCopyActions(row).some((allowed) => allowed.kind === action.kind)) : [];
    const copyStatus = row ? spaceCopyStatus(row, spaceLive.label) : null;
    return {
      label: spaceLive.label,
      row,
      attempt: listing.attempts.find((attempt) => attempt.item_id === item.item_id),
      adding: mine && (spaceAdd.starting || spaceAdd.running || settling),
      error: mine && stillMissing ? spaceAdd.error ?? spaceAddStopped : null,
      onAdd: () => startSpaceAdd(item),
      actions: copyActions,
      updating: copyStatus?.working ?? false,
      busy: copyStatus?.busy ?? false,
      copyError: copyStatus?.failure ?? null,
      onAction: (action) => { if (row) startSpaceCopyAction(row, action.kind); },
    };
  };
  // The open companion file's Space copy (design §4.6): its notice offers `Update`, a confirmed replace, and the Library version.
  const spaceCompanion = spaceListing.listing?.companion;
  const openSpaceCopy = companionRoot && selectedPath && spaceCompanion?.status === "available" && spaceCompanion.companion_root_id === companionRoot.root_id
    ? spaceListing.listing?.rows.find((row) => row.paths.includes(selectedPath)) ?? null
    : null;
  const spaceCopyUpdate = useSpaceUpdate(client, spaceListing);
  const [spaceCopyConfirm, setSpaceCopyConfirm] = useState<SpaceCopyConfirmation | null>(null);
  // The copy the notice or the Library header last acted on, so a result or conflict never shows on another copy.
  const [spaceCopyAction, setSpaceCopyAction] = useState<{ logicalId: string; conflict: boolean } | null>(null);
  const spaceCopyStatus = (row: SpaceCopyRow, label: string) => {
    const mine = spaceCopyAction?.logicalId === row.logical_id;
    const working = mine && spaceCopyUpdate.working;
    const unconfirmed = mine && spaceCopyUpdate.unconfirmed;
    const outcome = mine && spaceCopyUpdate.operation && !spaceCopyUpdate.busy ? spaceUpdateOutcome(spaceCopyUpdate.operation, label, spaceCopyUpdate.itemPaths) : null;
    const failure = !mine || working ? null : spaceCopyAction?.conflict ? spaceCopyConflict(label) : spaceCopyUpdate.error ?? (unconfirmed ? `${spaceUpdateUnconfirmed(label)} ${spaceListing.error ?? ""}`.trim() : outcome?.failed ? outcome.text : null);
    return { busy: mine && spaceCopyUpdate.busy, working, unconfirmed, outcome, failure };
  };
  const startSpaceCopyAction = (row: SpaceCopyRow, kind: SpaceCopyActionKind) => {
    if (!spaceLive || spaceCopyStatus(row, spaceLive.label).busy) return;
    if (kind === "view_library") {
      if (row.item_id) { setLibraryOpenRequest(row.item_id); if (!isLibrary) chooseRoot(libraryRoot); }
      return;
    }
    setSpaceCopyAction({ logicalId: row.logical_id, conflict: false });
    if (kind === "replace" || kind === "remove") { setSpaceCopyConfirm({ kind, row }); return; }
    const itemId = row.item_id;
    if (kind === "update" && itemId) void spaceCopyUpdate.start(() => client.librarySpaceUpdate({ target: spaceLive.target, scope: { scope: "selection", item_ids: [itemId], follow_ids: [] }, replace_edited: [] }));
  };
  const spaceCopyNoticeRef = useRef<HTMLDivElement>(null);
  const spaceCopyNoticeFocused = useRef(false);
  // Once the reread copy needs no notice, focus moves to the document rather than the page.
  useLayoutEffect(() => {
    const active = window.document.activeElement;
    if (!spaceCopyNoticeFocused.current || (active !== null && active !== window.document.body)) return;
    const next = spaceCopyNoticeRef.current?.querySelector("button");
    if (next) next.focus({ preventScroll: true });
    else { spaceCopyNoticeFocused.current = false; documentRef.current?.focus({ preventScroll: true }); }
  });
  const renderSpaceCopyNotice = (): ReactNode => {
    if (!openSpaceCopy || !spaceLive) return null;
    const chip = spaceCopyChip(openSpaceCopy);
    const actions = spaceCopyActions(openSpaceCopy).filter((action) => action.kind === "update" || action.kind === "replace");
    const itemId = openSpaceCopy.item_id;
    const viewLibrary = itemId && chip.actions.some((action) => action.kind === "view_library") ? itemId : null;
    const { busy, working, unconfirmed, outcome, failure } = spaceCopyStatus(openSpaceCopy, spaceLive.label);
    const skipped = outcome && !outcome.failed && outcome.skipped > 0 ? outcome.text : null;
    const text = working ? `Updating ${spaceLive.label}…` : failure ?? skipped ?? chip.notice;
    if (!text) return null;
    return <div ref={spaceCopyNoticeRef} className={`context-notice${failure ? " context-notice-error" : chip.tone === "working" ? " context-notice-warning" : ""}`} role={failure ? "alert" : "status"}
      onFocus={() => { spaceCopyNoticeFocused.current = true; }} onBlur={(event) => { if (event.relatedTarget) spaceCopyNoticeFocused.current = false; }}>
      <strong>{working ? <span className="library-spinner" aria-hidden="true" /> : <span aria-hidden="true">{chip.glyph} </span>}{chip.word}</strong>
      <span>{text}</span>
      {/* aria-disabled keeps focus on the pressed button while the update runs. */}
      {actions.map((action) => <button key={action.kind} type="button" aria-disabled={busy} onClick={() => startSpaceCopyAction(openSpaceCopy, action.kind)}>{action.label}</button>)}
      {unconfirmed ? <button type="button" onClick={spaceListing.reload}>Retry</button> : null}
      {viewLibrary ? <button type="button" onClick={() => { setLibraryOpenRequest(viewLibrary); chooseRoot(libraryRoot); }}>View Library version</button> : null}
    </div>;
  };
  useEffect(() => {
    if (!isLibrary || library.status !== "ready" || !library.listing || !selectedPath) return;
    if (library.listing.items.some((item) => libraryItemHolds(item, selectedPath))) return;
    const key = keyFor(LIBRARY_ROOT_ID, selectedPath);
    setDocuments((current) => {
      const next = { ...current };
      delete next[key];
      return next;
    });
    onChange({ ...value, path: null });
  }, [isLibrary, library.listing, library.status, onChange, selectedPath, value]);
  const startLibraryRefresh = useCallback((request: LibraryRefreshRequest, itemIds: string[]) => {
    setLibraryReportVerb("Refresh");
    setLibraryReportDismissed(false);
    setLibraryPendingIds(new Set(itemIds));
    void startLibraryOperation(() => client.libraryRefresh(request));
  }, [client, startLibraryOperation]);
  const refreshLibrary = useCallback(() => {
    startLibraryRefresh({ scope: "all" }, (libraryItems ?? []).map((item) => item.item_id));
  }, [libraryItems, startLibraryRefresh]);
  const openLibraryItem = (item: LibraryItemSummary) => {
    setAttachmentNotice(null);
    if (item.document_path) openFile(item.document_path, null);
  };
  const attachmentActions: LibraryAttachmentActions = {
    busy: libraryBusy,
    active: attachmentBusy ? attachmentRequest : null,
    start: (item, action, attachmentIds) => {
      if (libraryBusy || attachmentIds.length === 0) return;
      const request = { item_id: item.item_id, attachment_ids: attachmentIds, action };
      setBeforeAttachments(undefined);
      setAttachmentRequest(request);
      void attachmentOperation.start(() => client.libraryAttachments(request));
    },
    open: (item, attachment) => {
      const path = attachmentPath(item, attachment);
      if (path) { setAttachmentNotice(null); openFile(path, null); }
      else setAttachmentNotice({ itemId: item.item_id, attachmentId: attachment.attachment_id });
    },
  };
  const libraryActions: LibraryItemActions = {
    attachments: attachmentActions,
    open: openLibraryItem,
    refresh: startLibraryRefresh,
    remove: (item) => setLibraryConfirm({ kind: "remove", item }),
    copyLink: (item) => {
      const link = item.source_url ?? item.original_url;
      if (link) void navigator.clipboard?.writeText(link).catch(() => undefined);
    },
    canCopyLink: typeof navigator !== "undefined" && Boolean(navigator.clipboard),
    refreshBusy: libraryBusy,
    spaceEntries: (item) => {
      const state = itemSpace(item);
      if (!state) return [];
      const add = headerSpaceAction(state.row, state.label).actions.find((action) => action.kind === "add");
      if (add) return [{ label: add.label, onSelect: state.onAdd, disabled: state.adding || state.attempt?.state === "pending" }];
      return state.actions.map((action) => ({ label: action.label, onSelect: () => state.onAction(action), disabled: state.busy, destructive: action.kind === "remove" }));
    },
    removeFollow: async (follow, mode) => {
      await client.libraryRemove({ mode, follow_id: follow.follow_id });
      if (mode === "follow" && selectedLibraryItem?.follow_id === follow.follow_id) onChange({ ...value, path: null });
      announceLibraryChanged();
    },
  };
  // Palette commands and `Open in Library` wait until the listing can serve them.
  useEffect(() => {
    if (!isLibrary || !library.listing) return;
    if (libraryCommand && handledLibraryCommand.current !== libraryCommand.token) {
      if (libraryCommand.kind === "refresh") {
        handledLibraryCommand.current = libraryCommand.token;
        if (!libraryBusy) refreshLibrary();
      } else {
        const item = library.listing.items.find((candidate) => candidate.item_id === libraryCommand.itemId);
        if (item) { handledLibraryCommand.current = libraryCommand.token; openLibraryItem(item); }
      }
    }
    if (libraryOpenRequest) {
      const item = library.listing.items.find((candidate) => candidate.item_id === libraryOpenRequest);
      if (item) { setLibraryOpenRequest(null); openLibraryItem(item); }
    }
  });
  // At pane width ≤ 420 px `Add…` and `Refresh all` move into the toolbar `⋯` menu.
  const [compactToolbar, setCompactToolbar] = useState(false);
  useLayoutEffect(() => {
    const viewer = viewerRef.current;
    if (!viewer || typeof ResizeObserver === "undefined") return;
    const measure = () => { const width = viewer.getBoundingClientRect().width; if (width > 0) setCompactToolbar(width <= 420); };
    measure();
    const observer = new ResizeObserver(measure);
    observer.observe(viewer);
    return () => observer.disconnect();
  }, []);
  const selectSearchResult = useCallback((result: { path: string; line: number; revision: string }) => {
    if (!root) return;
    const fileKey = keyFor(root.root_id, result.path);
    const files = { ...value.files, [fileKey]: {
      ...(value.files[fileKey] ?? {
        rootId: root.root_id,
        path: result.path,
        mode: "auto" as const,
        selectionStart: null,
        selectionEnd: null,
        scrollTop: 0,
      }),
      revision: result.revision,
      selectionStart: result.line,
      selectionEnd: result.line,
    } };
    onChange({ ...value, rootId: root.root_id, path: result.path, files });
  }, [onChange, root, value]);
  const invalidateVisibleFiles = useCallback((invalidations: ContextInvalidation[]) => {
    if (!root || invalidations.length === 0) return;
    const invalidated = new Set(invalidations.map((item) => item.path));
    const files = { ...value.files };
    let changed = false;
    for (const path of invalidated) {
      const key = keyFor(root.root_id, path);
      const state = files[key];
      if (!state) continue;
      const update = invalidations.find((item) => item.path === path);
      files[key] = {
        ...state,
        revision: update?.revision ?? state.revision,
        selectionStart: null,
        selectionEnd: null,
      };
      changed = true;
    }
    setDocuments((current) => {
      const next = { ...current };
      for (const path of invalidated) {
        const key = keyFor(root.root_id, path);
        const state = next[key];
        if (state?.document && state.status !== "error") {
          // A late invalidation must not hide the source read's actionable error.
          next[key] = {
            status: "error",
            document: state.document,
            error: "Source changed; the previous view is retained until it is refreshed.",
          };
        }
      }
      return next;
    });
    if (selectedPath && invalidated.has(selectedPath)) {
      void loadDirectory(root, directoryPathForFile, true);
    }
    if (changed) onChange({ ...value, files });
    setInvalidationGeneration((generation) => generation + 1);
  }, [directoryPathForFile, loadDirectory, onChange, root, selectedPath, value]);
  useEffect(() => {
    const onNavigation = (event: Event) => {
      if (!viewerRef.current?.contains(globalThis.document.activeElement)) return;
      const action = fileNavigationAction(event);
      if (action === "open-picker") openFilePicker();
      else if (action === "focus-tree") focusTree();
      else if (action === "focus-content") focusContent();
    };
    window.addEventListener(FILE_NAVIGATION_EVENT, onNavigation);
    return () => window.removeEventListener(FILE_NAVIGATION_EVENT, onNavigation);
  }, [focusContent, focusTree, openFilePicker]);
  const rootDirectory = directories[keyFor(root.root_id, "")];
  const rootEmpty = rootDirectory?.status === "ready" && rootDirectory.data?.entries.length === 0 && rootDirectory.data.next_offset === undefined;
  // Open the folder's guide instead of an empty document area.
  const openedDefaultRef = useRef<string | null>(null);
  useEffect(() => {
    if (value.path || !rootDirectory?.data || openedDefaultRef.current === identityKey) return;
    openedDefaultRef.current = identityKey;
    const files = rootDirectory.data.entries.filter((entry) => entry.kind === "file" && !entry.refusal && entry.path);
    const named = (name: string) => files.find((entry) => entry.name.toLowerCase() === name);
    const guide = named("task.md") ?? named("readme.md") ?? files.find((entry) => /\.md$/i.test(entry.name));
    if (guide) chooseEntry(guide);
  });
  const renderDocument = (): ReactNode => {
    const fileState = selectedPath
      ? selectedFileState ?? { rootId: root.root_id, path: selectedPath, mode: "source" as const, selectionStart: null, selectionEnd: null, scrollTop: 0, revision: selectedRevision }
      : null;
    const renderedMode = document && selectedPath ? (isMarkdown(document, selectedPath) ? "markdown" : isHtml(document, selectedPath) ? "html" : null) : null;
    const effectiveMode = fileState?.mode === "auto" ? renderedMode ?? "source" : fileState?.mode ?? "source";
    const selectedRange = fileState && fileState.selectionStart !== null && fileState.selectionEnd !== null
      ? { start: fileState.selectionStart, end: fileState.selectionEnd }
      : null;
    const renderDocumentBody = (drafts: CommentDraft[] = [], actions?: CommentDraftActions, inlineEditor?: (line: number) => ReactNode): ReactNode => {
      if (noticeItem && noticeAttachment) return attachmentPath(noticeItem, noticeAttachment)
        ? <div className="context-notice library-attachment-notice"><span>Downloaded.</span><button type="button" onClick={() => attachmentActions.open(noticeItem, noticeAttachment)}>Open attachment</button></div>
        : <LibraryAttachmentNotice item={noticeItem} attachment={noticeAttachment} attachments={attachmentActions} />;
      if (isLibrary && !selectedPath) {
        if (!library.listing && library.status === "error") {
          return <div className="context-notice context-notice-error" role="alert"><strong>Library unavailable:</strong><span>{library.error}</span><span>Space context is unaffected.</span><button type="button" onClick={library.reload}>Retry</button></div>;
        }
        if (!library.listing) return <div className="context-empty">Loading…</div>;
        if (library.listing.items.length === 0) {
          return <div className="context-empty"><div className="context-empty-message"><strong>The Library is empty</strong><span>Add an issue, merge request, pull request, Jira issue, Confluence page, or a folder. The Library keeps it without a Space or session.</span><button type="button" onClick={() => setLibraryAdd("library")}>Add context…</button></div></div>;
        }
        return <div className="context-empty">Select a Library item to read it.</div>;
      }
      if (!selectedPath && rootEmpty) {
        return <div className="context-empty"><div className="context-empty-message"><strong>No files here yet</strong><span>{root.kind === "companion" ? "Add Library context from Resources." : "This directory is empty."}</span>{root.kind === "companion" ? <button type="button" onClick={() => setResourcesOpen(true)}>Open Resources</button> : null}</div></div>;
      }
      if (!selectedPath) return <div className="context-empty">Select a file to inspect its source.</div>;
      if (!documentState || documentState.status === "loading") return <div className="context-empty">Loading source…</div>;
      if (documentState.status === "error" && !document) {
        return <div className="context-notice context-notice-error"><strong>Unable to read source</strong><span>{documentState.error}</span><button type="button" onClick={refresh}>Retry</button></div>;
      }
      if (!document) return null;
      const metadata = sourceMetadata(document.text ?? "");
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
            <dt>Size</dt><dd>{document.bytes} B</dd><dt>Full path</dt><dd><code>{root.path.replace(/\/$/, "")}/{selectedPath}</code></dd>
            <dt>Relative path</dt><dd><code>{selectedPath}</code></dd>
            <dt>Root identity</dt><dd><code>{root.root_id}</code></dd>
            <dt>Provenance</dt><dd>{root.kind}{root.repository_id ? ` · repository ${root.repository_id}` : ""}{root.companion_id ? ` · companion ${root.companion_id}` : ""}</dd>
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
          {selectedLibraryItem ? <LibraryItemHeader item={selectedLibraryItem} providers={library.providers} narrow={overview.narrow} rootCrumb={presentation !== null} pending={pendingItemIds.has(selectedLibraryItem.item_id)} actions={libraryActions} onReplace={(item) => setLibraryConfirm({ kind: "replace", item })} details={documentDetails} space={itemSpace(selectedLibraryItem)} pageUpdate={{ at: metadata.lastModified, by: metadata.lastModifiedBy }} /> : <div className="context-document-header">
            {metadata.canonicalId ? <span className="document-source-kind">{metadata.provider ?? "Issue"}</span> : null}<strong title={selectedPath}>{metadata.canonicalId ?? documentName(selectedPath)}</strong>
            {document.truncated ? <span className="context-state-warning">Truncated by preview limit</span> : null}

            {documentDetails}
          </div>}
          {renderSpaceCopyNotice()}
          {documentState.status === "error" ? <div className="context-notice context-notice-warning" role="status"><strong>Stale source</strong><span>{documentState.error}</span><button type="button" onClick={refresh}>Refresh</button></div> : null}
          {document.text !== null && document.diagnostics.some(isShownDiagnostic) ? <div className="context-notice context-notice-warning" role="status">{document.diagnostics.filter(isShownDiagnostic).map((diagnostic) => <span key={`${diagnostic.code}:${diagnostic.message}`}>{diagnostic.message}</span>)}</div> : null}
          {/\.pdf$/i.test(selectedPath) ? <div className="context-notice"><strong>PDF preview unavailable</strong><span>This file is retained without an active PDF renderer.</span></div> : document.text === null && reader && (root.kind === "companion" || root.kind === "folder" || root.kind === "library") && /\.(png|jpe?g)$/i.test(selectedPath) ? <div className="context-raster-preview"><SafeImage media={reader.media} request={{ root_id: root.root_id, path: selectedPath, expected_revision: document.revision }} alt={selectedPath} className="context-safe-image" /></div> : document.text === null ? <div className="context-notice context-notice-error"><strong>{/\.pdf$/i.test(selectedPath) ? "PDF preview unavailable" : "File refused"}</strong><span>{document.media_type || "Binary or unsupported content"}</span>{document.diagnostics.map((diagnostic) => <span key={`${diagnostic.code}:${diagnostic.message}`}>{diagnostic.message}</span>)}</div> : <>
            {(() => {
              const mode = effectiveMode;
              return <>
                {document.truncated && document.next_offset !== undefined ? <div className="context-notice context-notice-warning" role="status"><span>Showing the start of a large file.</span><button type="button" onClick={() => { updateFile({ mode: "source" }); void loadDocumentPage(); }} disabled={documentPageLoading === pageRequestKey}>{documentPageLoading === pageRequestKey ? "Loading…" : "Load next source page"}</button></div> : null}
                <RenderErrorBoundary fallback={<div className="context-notice context-notice-error"><strong>Markdown rendering failed</strong><span>Showing the canonical source instead.</span><SourceLines text={document.text!} state={{ ...fileState!, mode: "source" }} onSelect={(start, end) => updateFile({ selectionStart: start, selectionEnd: end, mode: "source" })} onScroll={(scrollTop) => updateFile({ scrollTop })} commentDrafts={drafts} commentActions={actions} /></div>}>
                  {mode === "markdown" ? <MarkdownView media={reader?.media ?? null} text={document.text!} state={{ ...fileState!, mode: "markdown" }} onSelect={(start, end) => updateFile({ selectionStart: start, selectionEnd: end })} onScroll={(scrollTop) => updateFile({ scrollTop })} /> : mode === "html" ? <HtmlPreview html={document.text!} title={selectedPath} /> : <SourceLines text={document.text!} state={{ ...fileState!, mode: "source" }} onSelect={(start, end) => updateFile({ selectionStart: start, selectionEnd: end })} onScroll={(scrollTop) => updateFile({ scrollTop })} commentDrafts={drafts} commentActions={actions} inlineEditor={inlineEditor} onCreateLineComment={actions?.createLines} onCreateFileComment={actions?.createWholeFile} />}
                </RenderErrorBoundary>
              </>;
            })()}
            {actions ? <footer className="context-comment-status" aria-label="Context comment shortcuts"><span>{selectedLines ?? ""}</span><span className="context-comment-status-actions"><button type="button" onClick={() => actions.createLines()} disabled={!commentStatus.canCreateLines} title={commentStatus.canCreateLines ? "Comment on selected lines (C)" : "Select source lines before commenting"}><kbd>C</kbd> comment</button><button type="button" onClick={actions.createWholeFile} disabled={!commentStatus.canCreateWholeFile} title={commentStatus.canCreateWholeFile ? "Comment on whole file (Shift+C)" : "Source is not ready"}><kbd>Shift+C</kbd> file</button><button type="button" onClick={() => actions.openOverview()} title="Open comments overview">{commentStatus.count} comments</button></span></footer> : null}
          </>}
        </>
      );
    };
    if (!commentsEnabled || !presentation) return renderDocumentBody();
    return (
      <CommentDrafts
        client={client}
        presentation={presentation}
        root={root}
        path={selectedPath ?? ""}
        document={document ?? null}
        selection={selectedRange}
        mode={effectiveMode === "markdown" ? "markdown" : "source"}
        editorState={value.commentEditor ?? null}
        onEditorStateChange={(commentEditor) => onChange({ ...value, commentEditor })}
        invalidationGeneration={invalidationGeneration}
        refreshGeneration={refreshGeneration}
        sourceIdentity={root.companion_id ?? root.root_id}
        inlineEditor={effectiveMode !== "markdown"}
        showToolbar={false}
        onCommentStatusChange={updateCommentStatus}
        onEditorDismissed={restoreSourceFocus}
      >
        {(drafts, actions, renderInlineEditor) => renderDocumentBody(drafts, actions, renderInlineEditor)}
      </CommentDrafts>
    );
  };

  if (!root) {
    return <div className="context-viewer context-viewer-empty" onPointerDown={() => { if (!controlAllowed) onRequestControl(); }}>
      <div className="context-notice context-notice-error"><strong>Context unavailable</strong><span>{presentation?.reason || "This pane is not connected to an authorized Context root."}</span></div>
      {onTerminalView ? <button type="button" onClick={onTerminalView}>Show terminal view</button> : null}
    </div>;
  }

  return (
    <section className="context-viewer" aria-label="Context file viewer" ref={viewerRef} onPointerDown={() => { if (!controlAllowed) onRequestControl(); }} onKeyDownCapture={(event) => {
      if (isEditingTarget(event.target)) return;
      if ((event.ctrlKey || event.metaKey) && !event.altKey && event.key.toLowerCase() === "p") { event.preventDefault(); event.stopPropagation(); openFilePicker(); }
      if (event.altKey && !event.ctrlKey && !event.metaKey && event.key === "1") { event.preventDefault(); focusTree(); }
      if (event.altKey && !event.ctrlKey && !event.metaKey && event.key === "2") { event.preventDefault(); focusContent(); }
      if (event.altKey && !event.ctrlKey && !event.metaKey && event.code === "KeyZ") { event.preventDefault(); toggleWrap(); }
    }}>
      <header className="context-toolbar">

        {roots.length > 1 ? <label className="context-root-select"><span className="sr-only">Context root</span><select value={root.root_id} onChange={(event) => { const next = roots.find((candidate) => candidate.root_id === event.target.value); if (next) chooseRoot(next); }}>{roots.map((candidate) => <option value={candidate.root_id} key={candidate.root_id}>{candidate.label}</option>)}</select></label> : null}
        <button type="button" className="viewer-overview-trigger" onClick={overview.toggle} aria-expanded={overview.open} aria-controls={overviewId} aria-label="Toggle file overview"><UiIcon name="sidebar" /> Files</button>
        <button type="button" className="viewer-file-picker-trigger" onClick={openFilePicker} aria-label="Choose Context file" title="Choose Context file"><UiIcon name="search" /></button>
        {root.kind === "companion" ? <button type="button" onClick={() => setResourcesOpen(true)} aria-expanded={resourcesOpen}>{spaceListing.listing && spaceListing.listing.behind > 0 ? `Resources · ${spaceListing.listing.behind} behind` : "Resources"}</button> : null}
        {isLibrary && !compactToolbar ? <>
          <button type="button" onClick={() => setLibraryAdd("library")}>Add…</button>
          <button type="button" onClick={refreshLibrary} disabled={libraryBusy || !library.listing || library.listing.items.length === 0} title="Refresh every item from its source">Refresh all</button>
        </> : null}
        {isLibrary && compactToolbar ? <button type="button" aria-label="More Library actions" aria-haspopup="menu" aria-expanded={libraryToolbarMenu !== null} onClick={(event) => setLibraryToolbarMenu(menuAnchor(event.currentTarget))}><UiIcon name="more" /></button> : null}
        <span className="context-toolbar-spacer" />
        {document && selectedPath && (isMarkdown(document, selectedPath) || isHtml(document, selectedPath)) ? <div className="viewer-segmented" role="group" aria-label="Document presentation"><button type="button" aria-pressed={selectedFileState?.mode !== "source"} onClick={() => updateFile({ mode: "auto" })}>Preview</button><button type="button" aria-pressed={selectedFileState?.mode === "source"} onClick={() => updateFile({ mode: "source" })}>Source</button></div> : null}

        <button type="button" className="viewer-wrap-toggle" aria-pressed={wrap} onClick={toggleWrap} title={wrap ? "Long lines wrap (Alt+Z)" : "Long lines scroll (Alt+Z)"}><UiIcon name="wrap" /><span className="viewer-wrap-label">Wrap</span></button>
        <button type="button" onClick={refresh} aria-label="Refresh Context files" title="Refresh files"><UiIcon name="refresh" /></button>
        {onTerminalView ? <button type="button" onClick={onTerminalView} aria-label="Show terminal" title="Show terminal"><UiIcon name="terminal" /></button> : null}
      </header>
      {isLibrary && attachmentRequest ? <AttachmentReport request={attachmentRequest} operation={attachmentOperation.operation} starting={attachmentOperation.starting} error={attachmentOperation.error}
        item={library.status === "error" ? null : libraryItems?.find((item) => item.item_id === attachmentRequest.item_id) ?? null}
        settled={beforeAttachments !== undefined && (library.status === "error" || (library.status === "ready" && library.listing !== beforeAttachments))}
        onCancel={attachmentOperation.cancel} onDismiss={() => setAttachmentRequest(null)}
        onRetry={(ids) => { const item = libraryItems?.find((item) => item.item_id === attachmentRequest.item_id); if (item) attachmentActions.start(item, attachmentRequest.action, ids); }} /> : null}
      {isLibrary && libraryOperation.operation && !libraryReportDismissed ? <RefreshReport operation={libraryOperation.operation} verb={libraryReportVerb} error={libraryOperation.error}
        onCancel={libraryOperation.cancel} onDismiss={() => setLibraryReportDismissed(true)}
        onOpenItem={(itemId) => { const item = libraryItems?.find((candidate) => candidate.item_id === itemId); if (item) openLibraryItem(item); }}
        onRetry={(itemIds) => startLibraryRefresh({ scope: "items", item_ids: itemIds }, itemIds)}
        onRetryFollow={(followId) => startLibraryRefresh({ scope: "follow", follow_id: followId }, [])} /> : null}
      {isLibrary && !libraryOperation.operation && libraryOperation.error ? <div className="context-notice context-notice-error" role="alert"><strong>Library operation failed:</strong><span>{libraryOperation.error}</span></div> : null}
      {discoveryDiagnostics.length > 0 ? <div className="context-notice context-notice-warning" role="status">{discoveryDiagnostics.map((diagnostic) => <span key={`${diagnostic.code}:${diagnostic.message}`}>{diagnostic.message}</span>)}</div> : null}
      <div className={`context-body${overview.open ? " has-file-overview" : ""}`} style={tree.style}>
        {overview.narrow && overview.open ? <button type="button" className="viewer-overview-backdrop" aria-label="Close file overview" onClick={overview.close} /> : null}
        <aside id={overviewId} className={`context-tree${overview.open ? " is-overview-open" : ""}`} aria-label={isLibrary ? "Library items" : "Context files"} ref={treeRef} onKeyDown={(event) => { if (event.key === "Escape" && overview.narrow) { event.preventDefault(); event.stopPropagation(); overview.close(); documentRef.current?.focus(); } else if (!isLibrary) onTreeKeyDown(event); }}>
        {root.kind === "companion" ? <details className="viewer-tree-search"><summary><UiIcon name="search" /> Search contents</summary><ContextSearch
          identity={identityKey}
          bindingId={bindingId}
          rootId={root.root_id}
          known={knownRevisions}
          search={searchContext}
          poll={pollContext}
          onSelect={selectSearchResult}
          onInvalidate={invalidateVisibleFiles}
          disabled={!controlAllowed}
        /></details> : null}
          {isLibrary ? <>
            {!library.listing && library.status !== "error" ? <div className="context-tree-status">Loading…</div> : null}
            {library.status === "error" ? <div className="context-tree-error" role="alert">
              <span>Library unavailable: {library.error}. Space context is unaffected.</span>
              <button type="button" onClick={library.reload}>Retry</button>
            </div> : null}
            {library.listing?.items.length === 0 ? <div className="context-tree-status is-empty">Empty</div> : null}
            {library.listing ? <LibraryTree items={library.listing.items} follows={library.listing.follows} providers={library.providers} selectedItemId={selectedLibraryItem?.item_id ?? null} selectedAttachmentId={selectedAttachmentId} pendingItemIds={pendingItemIds} actions={libraryActions} /> : null}
          </> : null}
          {directories[keyFor(root.root_id, "")]?.status === "loading" ? <div className="context-tree-status">Loading…</div> : null}
          {directories[keyFor(root.root_id, "")]?.status === "error" && !directories[keyFor(root.root_id, "")]?.data ? <div className="context-tree-error">{directories[keyFor(root.root_id, "")]?.error}</div> : null}
          {rootEmpty ? <div className="context-tree-status is-empty">Empty</div> : null}
          {treeRows.map((row) => <div className="context-tree-node" key={row.entry.entry_id}>
            <button type="button" data-context-path={row.path} className={`context-tree-row${selectedPath === row.path ? " is-selected" : ""}`} style={{ paddingLeft: `${8 + row.depth * 16}px` }} disabled={!isTreeRowEnabled(row)} onClick={() => row.entry.kind === "directory" ? toggleDirectory(row.entry) : chooseEntry(row.entry)} aria-label={`${row.label}${row.entry.refusal ? `, refused: ${row.entry.refusal}` : ""}`}>
              <span className="context-tree-disclosure">{row.entry.kind === "directory" ? <UiIcon name={row.open ? "down" : "right"} /> : null}</span>
              <span className="context-tree-icon" aria-hidden="true">{row.entry.kind === "directory" ? null : <UiIcon name="file" />}</span>
              <span className="context-tree-name" title={row.path}>{row.label}</span>
              <span className="context-tree-meta">{row.entry.refusal ?? ""}</span>
            </button>{row.entry.kind === "directory" && directories[keyFor(root.root_id, row.path)]?.data?.next_offset !== undefined ? <button type="button" className="context-tree-more" onClick={() => void loadDirectory(root, row.path)} aria-label={`Load more entries in ${row.path}`}>more</button> : null}
            {row.entry.refusal ? <div className="context-tree-refusal">{row.entry.refusal}</div> : null}
          </div>)}
        </aside>
        {overview.open && !overview.narrow ? <TreeSplitter width={tree.width} onChange={tree.setWidth} /> : null}
        <main className="context-document" ref={documentRef} tabIndex={-1}>
          {renderDocument()}
        </main>
      </div>
      {resourcesOpen && presentation ? <ContextResources client={client} root={root} space={space} spaceListing={spaceListing} onAdd={() => setLibraryAdd("space")} onClose={() => setResourcesOpen(false)} /> : null}
      {libraryToolbarMenu ? <LibraryMenu x={libraryToolbarMenu.x} y={libraryToolbarMenu.y} label="Library actions" onDismiss={() => setLibraryToolbarMenu(null)} entries={[
        { label: "Add…", onSelect: () => setLibraryAdd("library") },
        { label: "Refresh all", onSelect: refreshLibrary, disabled: libraryBusy || !library.listing || library.listing.items.length === 0 },
      ]} /> : null}
      {libraryAdd ? <AddContextDialog client={client} onClose={() => setLibraryAdd(null)} space={space}
        onOpenItem={(itemId) => { setLibraryOpenRequest(itemId); if (!isLibrary) chooseRoot(libraryRoot); }} defaultDestination={libraryAdd}
        openInSpace={companionRoot ? { companionRootId: companionRoot.root_id, open: (path) => { setResourcesOpen(false); openFile(path, null); } } : null} /> : null}
      {libraryConfirm?.kind === "remove" ? <LibraryConfirmDialog title={`Remove "${libraryConfirm.item.title}" from the Library?`} safeLabel="Cancel" confirmLabel="Remove from Library" destructive
        body={<p>Deletes the Library copy. Copies already in Spaces stay as they are and stop receiving updates. {providerFamily(library.providers, libraryConfirm.item.provider_id).name} isn't changed. You can add it again from its link.</p>}
        onClose={() => setLibraryConfirm(null)}
        onConfirm={async () => {
          const item = libraryConfirm.item;
          await client.libraryRemove({ mode: "item", item_id: item.item_id, expected_revision: item.revision });
          if (selectedPath === item.document_path) onChange({ ...value, path: null });
          setLibraryConfirm(null);
          announceLibraryChanged();
        }} /> : null}
      {libraryConfirm?.kind === "replace" ? <LibraryConfirmDialog title="Replace the edited Library file?" safeLabel="Keep file" confirmLabel="Replace with source version" destructive
        body={<p>This Library file was changed outside Cockpit. Replacing it fetches the source version and discards those changes. Spaces aren't changed.</p>}
        onClose={() => setLibraryConfirm(null)}
        onConfirm={async () => {
          const item = libraryConfirm.item;
          setLibraryConfirm(null);
          setLibraryReportVerb("Replace");
          setLibraryReportDismissed(false);
          setLibraryPendingIds(new Set([item.item_id]));
          await startLibraryOperation(() => client.libraryReplace({ item_id: item.item_id, confirmed: item.conflict }));
        }} /> : null}
      {spaceCopyConfirm && spaceLive ? <SpaceCopyConfirmDialog client={client} space={spaceLive} confirmation={spaceCopyConfirm}
        onReplacing={(operation) => void spaceCopyUpdate.start(async () => operation)}
        onConflict={() => { setSpaceCopyAction({ logicalId: spaceCopyConfirm.row.logical_id, conflict: true }); spaceListing.reload(); }}
        onClose={() => setSpaceCopyConfirm(null)} /> : null}
      {pickerOpen ? <FilePicker candidates={[...pickerIndex.entries.values()].map((entry) => ({ id: entry.entry_id, path: entry.path!, detail: entry.bytes === null ? undefined : `${entry.bytes} B` } satisfies FileNavigationCandidate))} loading={pickerIndex.loading} incomplete={pickerIndex.incomplete} onChoose={(candidate) => { const entry = pickerIndex.entries.get(candidate.path); if (entry) chooseEntry(entry); closeFilePicker(); focusContent(); }} onDismiss={() => { closeFilePicker(); focusContent(); }} /> : null}
    </section>
  );
}
