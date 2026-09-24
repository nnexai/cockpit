import { UiIcon } from "../UiIcon";
import { useFileOverview } from "../input/useFileOverview";
import { remarkBoundDiagrams } from "./markdownPolicy";
import { SafeImage } from "./SafeImage";
import { MermaidView } from "./MermaidView";
import type { CommentReviewRef } from "../../protocol/generated/v1";
import { HtmlPreview } from "./HtmlPreview";
import { Component, Fragment, useId, type KeyboardEvent as ReactKeyboardEvent, type MouseEvent as ReactMouseEvent, type ReactNode, useCallback, useEffect, useMemo, useRef, useState } from "react";
import ReactMarkdown, { type Components } from "react-markdown";
import remarkGfm from "remark-gfm";
import type { CockpitClient } from "../../client/CockpitClient";
import type {
  ContextDirectory,
  ContextDirectoryRequest,
  ContextDocument,
  ContextDocumentRequest,
  ContextEntry,
  ContextInvalidation,
  ContextKnownRevision,
  ContextRoot,
  PanePresentation,
  CommentDraft,
  ReviewComparison,
} from "../../protocol/generated/v1";
import { CommentDrafts, InlineCommentDrafts, type CommentDraftActions } from "./CommentDrafts";
import { ContextSearch } from "./ContextSearch";
import { ContextResources } from "./ContextResources";
import { splitSourceLines } from "./sourceLines";
import { FilePicker } from "../input/FilePicker";
import { FILE_NAVIGATION_EVENT, fileNavigationAction, type FileNavigationCandidate } from "../input/fileNavigation";
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


export type ContextViewerProps = {
  client: CockpitClient;
  presentation: PanePresentation;
  value: ContextViewState;
  onChange: (next: ContextViewState) => void;
  controlAllowed: boolean;
  onRequestControl: () => void;
  onTerminalView: () => void;
};

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
};

function sourceMetadata(source: string): SourceMetadata {
  const lines = splitSourceLines(source);
  if (lines[0]?.text.trim() !== "---") return { canonicalId: null, provider: null, fetchedAt: null };
  const fields: Record<string, string> = {};
  for (const line of lines.slice(1)) {
    if (line.text.trim() === "---" || line.text.trim() === "...") break;
    const match = /^([A-Za-z0-9_-]+):\s*(.*)$/.exec(line.text);
    if (match && (match[1] === "canonical_id" || match[1] === "fetched_at" || match[1] === "provider")) fields[match[1]] = match[2];
  }
  return { canonicalId: fields.canonical_id ?? null, provider: fields.provider ?? null, fetchedAt: fields.fetched_at ?? null };
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
      className="context-source-scroll"
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
                <code className="context-line-text">{line.text || " "}</code>
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

function MarkdownView({
  text,
  state,
  onSelect,
  onScroll,
  client, presentation,
}: {
  client: CockpitClient; presentation: PanePresentation;
  text: string;
  state: ContextFileViewState;
  onSelect: (start: number, end: number) => void;
  onScroll: (scrollTop: number) => void;
}) {
  const derived = useMemo(() => sourceLinesForMarkdown(text), [text]);
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
      return <span {...blockData(node, derived.sourceLines)}>{path ? <SafeImage client={client} sessionId={presentation.session_id} paneId={presentation.pane_id} request={{ binding_id: presentation.binding_id, root_id: state.rootId, path, expected_revision: null }} alt={alt ?? "Context image"} className="context-safe-image" /> : <span className="context-media-refusal">[Image unavailable: only Context-root PNG/JPEG images are supported]</span>}</span>;
    },
  }), [derived.sourceLines, derived.text, client, presentation.session_id, presentation.pane_id, presentation.binding_id, state.rootId, state.path]);
  return (
    <div className="context-markdown-scroll" ref={scrollRef} onScroll={(event) => onScroll(event.currentTarget.scrollTop)} onClick={(event) => {
      if (event.target instanceof Element && event.target.closest("dialog, button, textarea")) return;
      const span = spanFromClick(event);
      if (span) onSelect(span.start, span.end);
    }}>
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

export function ContextViewer({ client, presentation, value, onChange, controlAllowed, onRequestControl, onTerminalView }: ContextViewerProps) {
  const [directories, setDirectories] = useState<Record<string, DirectoryState>>({});
  const [documents, setDocuments] = useState<Record<string, DocumentState>>({});
  const [documentPageLoading, setDocumentPageLoading] = useState<string | null>(null);
  const [expanded, setExpanded] = useState<Set<string>>(() => new Set());
  const protectedDirectoryKeysRef = useRef<Set<string>>(new Set());
  const [refreshGeneration, setRefreshGeneration] = useState(0);
  const [invalidationGeneration, setInvalidationGeneration] = useState(0);
  const [rootId, setRootId] = useState<string | null>(value.rootId ?? presentation.default_root_id ?? presentation.roots[0]?.root_id ?? null);
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
  const root = presentation.roots.find((candidate) => candidate.root_id === rootId) ?? presentation.roots[0];
  const sessionId = presentation.session_id;
  const paneId = presentation.pane_id;
  const bindingId = presentation.binding_id;
  const activeRootId = root?.root_id ?? "";
  const identityKey = `${sessionId}\u0000${paneId}\u0000${bindingId}\u0000${activeRootId}`;
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
  const selectedRevision = selectedEntry?.revision ?? selectedFileState?.revision ?? null;
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
    if (!root || !selectedPath || !selectedKey || !document || document.next_offset === undefined) return;
    const expectedOffset = document.next_offset;
    const requestKey = `${selectedKey}\u0000${document.revision}\u0000${expectedOffset}`;
    const controller = new AbortController();
    documentController.current?.abort();
    documentController.current = controller;
    setDocumentPageLoading(requestKey);
    try {
      const data = await client.contextDocument(sessionId, paneId, {
        binding_id: bindingId,
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
  }, [bindingId, client, document, paneId, root, selectedKey, selectedPath, sessionId]);
  const knownRevisions = useMemo<ContextKnownRevision[]>(() => Object.values(documents)
    .map((state) => state.document)
    .filter((candidate): candidate is ContextDocument => candidate !== undefined && candidate.root_id === activeRootId)
    .map((candidate) => ({ path: candidate.path, revision: candidate.revision }))
    .sort((left, right) => left.path.localeCompare(right.path)), [activeRootId, documents]);
  const treeRows = useMemo(() => root ? contextTreeRows(root, directories, expanded) : [], [directories, expanded, root]);
  const discoveryDiagnostics = useMemo(() => {
    const seen = new Set<string>();
    return [...presentation.diagnostics, ...(root ? directories[keyFor(root.root_id, "")]?.data?.diagnostics ?? [] : [])].filter((diagnostic) => {
      const key = `${diagnostic.code}\u0000${diagnostic.message}\u0000${diagnostic.path ?? ""}`;
      if (seen.has(key)) return false;
      seen.add(key);
      return true;
    });
  }, [directories, presentation.diagnostics, root]);
  const commentsEnabled = Boolean(root && (root.kind === "companion" || (root.kind === "folder" && root.root_id === presentation.default_root_id)));
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
      const request: ContextDirectoryRequest = {
        binding_id: requestBindingId,
        root_id: requestRootId,
        path,
        offset: force ? undefined : previous?.next_offset,
        revision: force ? undefined : previous?.revision,
      };
      const data = await client.contextDirectory(sessionId, paneId, request, controller.signal);
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
  }, [bindingId, client, identityKey, paneId, sessionId]);
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
    if (root) void loadDirectory(root, "", true);
  }, [identityKey, loadDirectory, onChange, root, value]);

  useEffect(() => {
    if (!root || !selectedPath || !selectedKey) return;
    documentController.current?.abort();
    const requestId = ++documentRequestSequence.current;
    const controller = new AbortController();
    documentController.current = controller;
    const requestIdentity = identityKey;
    const requestBindingId = bindingId;
    const requestRootId = root.root_id;
    setDocumentPageLoading(null);
    setDocuments((current) => retainDocumentState(current, selectedKey, { status: "loading", document: current[selectedKey]?.document }));
    const request: ContextDocumentRequest = { binding_id: requestBindingId, root_id: requestRootId, path: selectedPath, expected_revision: selectedRevision };
    void client.contextDocument(sessionId, paneId, request, controller.signal).then((data) => {
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
  }, [bindingId, client, identityKey, paneId, refreshGeneration, selectedKey, selectedPath, selectedRevision, sessionId, root?.root_id]);

  const chooseRoot = (nextRoot: ContextRoot) => {
    setRootId(nextRoot.root_id);
    setExpanded(new Set());
    onChange({ ...value, rootId: nextRoot.root_id, path: null });
  };
  const chooseEntry = (entry: ContextEntry) => {
    if (entry.kind !== "file" || entry.refusal || !entry.path || !root) return;
    const fileKey = keyFor(root.root_id, entry.path);
    const next = value.files[fileKey] ?? { rootId: root.root_id, path: entry.path, mode: "auto" as const, selectionStart: null, selectionEnd: null, scrollTop: 0, revision: entry.revision };
    const files = { ...value.files };
    delete files[fileKey];
    files[fileKey] = next;
    const keys = Object.keys(files);
    while (keys.length > MAX_RETAINED_FILE_STATES) {
      const oldest = keys.shift();
      if (oldest === undefined) break;
      delete files[oldest];
    }
    onChange({ ...value, rootId: root.root_id, path: entry.path, files });
    overview.select();
  };
  const focusTree = useCallback(() => {
    overview.show();
    requestAnimationFrame(() => {
      const buttons = treeRef.current?.querySelectorAll<HTMLButtonElement>("[data-context-path]");
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
    if (!root) return;
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
            const data = await client.contextDirectory(sessionId, paneId, { binding_id: bindingId, root_id: root.root_id, path, offset, revision }, controller.signal);
            if (controller.signal.aborted || pickerGeneration.current !== generation || requestIdentityRef.current !== requestIdentity || data.binding_id !== bindingId || data.root_id !== root.root_id) return;
            incomplete ||= data.truncated;
            for (const entry of data.entries) {
              if (entry.kind === "directory" && entry.path) pending.push(entry.path);
              if (entry.kind === "file" && entry.path && !entry.refusal) {
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
  }, [bindingId, client, identityKey, paneId, root, sessionId]);
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
  const refresh = () => {
    if (!root) return;
    setRefreshGeneration((generation) => generation + 1);
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
    client.contextSearch(sessionId, paneId, request, signal), [client, paneId, sessionId]);
  const pollContext = useCallback((request: Parameters<CockpitClient["contextInvalidate"]>[2], signal: AbortSignal) =>
    client.contextInvalidate(sessionId, paneId, request, signal), [client, paneId, sessionId]);
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
      return (
        <>
          <div className="context-document-header">
            <span className="document-source-kind">{metadata.canonicalId ? "Issue" : "File"}</span><strong title={selectedPath}>{metadata.canonicalId ?? documentName(selectedPath)}</strong>
            {metadata.canonicalId ? <span className="viewer-secondary-metadata">Imported snapshot</span> : null}
            {document.truncated ? <span className="context-state-warning">Truncated by preview limit</span> : null}

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
          </div>
          {documentState.status === "error" ? <div className="context-notice context-notice-warning" role="status"><strong>Stale source</strong><span>{documentState.error}</span><button type="button" onClick={refresh}>Refresh</button></div> : null}
          {document.text !== null && document.diagnostics.length > 0 ? <div className="context-notice context-notice-warning" role="status">{document.diagnostics.map((diagnostic) => <span key={`${diagnostic.code}:${diagnostic.message}`}>{diagnostic.message}</span>)}</div> : null}
          {/\.pdf$/i.test(selectedPath) ? <div className="context-notice"><strong>PDF preview unavailable</strong><span>This file is retained without an active PDF renderer.</span></div> : document.text === null && (root.kind === "companion" || root.kind === "folder") && /\.(png|jpe?g)$/i.test(selectedPath) ? <div className="context-raster-preview"><SafeImage client={client} sessionId={sessionId} paneId={paneId} request={{ binding_id: bindingId, root_id: root.root_id, path: selectedPath, expected_revision: document.revision }} alt={selectedPath} className="context-safe-image" /></div> : document.text === null ? <div className="context-notice context-notice-error"><strong>{/\.pdf$/i.test(selectedPath) ? "PDF preview unavailable" : "File refused"}</strong><span>{document.media_type || "Binary or unsupported content"}</span>{document.diagnostics.map((diagnostic) => <span key={`${diagnostic.code}:${diagnostic.message}`}>{diagnostic.message}</span>)}</div> : <>
            {(() => {
              const mode = effectiveMode;
              return <>
                {document.truncated && document.next_offset !== undefined ? <div className="context-notice context-notice-warning" role="status"><span>Showing a bounded source window.</span><button type="button" onClick={() => { updateFile({ mode: "source" }); void loadDocumentPage(); }} disabled={documentPageLoading === pageRequestKey}>{documentPageLoading === pageRequestKey ? "Loading…" : "Load next source page"}</button></div> : null}
                <RenderErrorBoundary fallback={<div className="context-notice context-notice-error"><strong>Markdown rendering failed</strong><span>Showing the canonical source instead.</span><SourceLines text={document.text!} state={{ ...fileState!, mode: "source" }} onSelect={(start, end) => updateFile({ selectionStart: start, selectionEnd: end, mode: "source" })} onScroll={(scrollTop) => updateFile({ scrollTop })} commentDrafts={drafts} commentActions={actions} /></div>}>
                  {mode === "markdown" ? <MarkdownView client={client} presentation={presentation} text={document.text!} state={{ ...fileState!, mode: "markdown" }} onSelect={(start, end) => updateFile({ selectionStart: start, selectionEnd: end })} onScroll={(scrollTop) => updateFile({ scrollTop })} /> : mode === "html" ? <HtmlPreview html={document.text!} title={selectedPath} /> : <SourceLines text={document.text!} state={{ ...fileState!, mode: "source" }} onSelect={(start, end) => updateFile({ selectionStart: start, selectionEnd: end })} onScroll={(scrollTop) => updateFile({ scrollTop })} commentDrafts={drafts} commentActions={actions} inlineEditor={inlineEditor} onCreateLineComment={actions?.createLines} onCreateFileComment={actions?.createWholeFile} />}
                </RenderErrorBoundary>
              </>;
            })()}
            {actions ? <footer className="context-comment-status" aria-label="Context comment shortcuts"><span>{selectedLines ?? "Select a line"}</span><span className="context-comment-status-actions"><button type="button" onClick={() => actions.createLines()} disabled={!commentStatus.canCreateLines} title={commentStatus.canCreateLines ? "Comment on selected lines (C)" : "Select source lines before commenting"}><kbd>C</kbd> comment</button><button type="button" onClick={actions.createWholeFile} disabled={!commentStatus.canCreateWholeFile} title={commentStatus.canCreateWholeFile ? "Comment on whole file (Shift+C)" : "Source is not ready"}><kbd>Shift+C</kbd> file</button><button type="button" onClick={() => actions.openOverview()} title="Open comments overview">{commentStatus.count} comments</button></span></footer> : null}
          </>}
        </>
      );
    };
    if (!commentsEnabled) return <>{renderDocumentBody()}<div className="context-notice" role="status">Comments are available from the verified Context root.</div></>;
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
      <div className="context-notice context-notice-error"><strong>Context unavailable</strong><span>{presentation.reason || "This pane is not connected to an authorized Context root."}</span></div>
      <button type="button" onClick={onTerminalView}>Show terminal view</button>
    </div>;
  }

  return (
    <section className="context-viewer" aria-label="Context file viewer" ref={viewerRef} onPointerDown={() => { if (!controlAllowed) onRequestControl(); }} onKeyDownCapture={(event) => {
      if (isEditingTarget(event.target)) return;
      if ((event.ctrlKey || event.metaKey) && !event.altKey && event.key.toLowerCase() === "p") { event.preventDefault(); event.stopPropagation(); openFilePicker(); }
      if (event.altKey && !event.ctrlKey && !event.metaKey && event.key === "1") { event.preventDefault(); focusTree(); }
      if (event.altKey && !event.ctrlKey && !event.metaKey && event.key === "2") { event.preventDefault(); focusContent(); }
    }}>
      <header className="context-toolbar">

        {presentation.roots.length > 1 ? <label className="context-root-select"><span className="sr-only">Context root</span><select value={root.root_id} onChange={(event) => { const next = presentation.roots.find((candidate) => candidate.root_id === event.target.value); if (next) chooseRoot(next); }}>{presentation.roots.map((candidate) => <option value={candidate.root_id} key={candidate.root_id}>{candidate.label}</option>)}</select></label> : null}
        <button type="button" className="viewer-overview-trigger" onClick={overview.toggle} aria-expanded={overview.open} aria-controls={overviewId} aria-label="Toggle file overview"><UiIcon name="sidebar" /> Files</button>
        <button type="button" className="viewer-file-picker-trigger" onClick={openFilePicker} aria-label="Choose Context file" title="Choose Context file"><UiIcon name="search" /></button>
        {root.kind === "companion" ? <button type="button" onClick={() => setResourcesOpen(true)} aria-expanded={resourcesOpen}>Resources</button> : null}
        <span className="context-toolbar-spacer" />
        {document && selectedPath && (isMarkdown(document, selectedPath) || isHtml(document, selectedPath)) ? <div className="viewer-segmented" role="group" aria-label="Document presentation"><button type="button" aria-pressed={selectedFileState?.mode !== "source"} onClick={() => updateFile({ mode: "auto" })}>Preview</button><button type="button" aria-pressed={selectedFileState?.mode === "source"} onClick={() => updateFile({ mode: "source" })}>Source</button></div> : null}

        <button type="button" onClick={refresh} aria-label="Refresh Context files" title="Refresh files"><UiIcon name="refresh" /></button>
        <button type="button" onClick={onTerminalView} aria-label="Show terminal" title="Show terminal"><UiIcon name="terminal" /></button>
      </header>
      {discoveryDiagnostics.length > 0 ? <div className="context-notice context-notice-warning" role="status">{discoveryDiagnostics.map((diagnostic) => <span key={`${diagnostic.code}:${diagnostic.message}`}>{diagnostic.message}</span>)}</div> : null}
      <div className={`context-body${overview.open ? " has-file-overview" : ""}`}>
        {overview.narrow && overview.open ? <button type="button" className="viewer-overview-backdrop" aria-label="Close file overview" onClick={overview.close} /> : null}
        <aside id={overviewId} className={`context-tree${overview.open ? " is-overview-open" : ""}`} aria-label="Context files" ref={treeRef} onKeyDown={(event) => { if (event.key === "Escape" && overview.narrow) { event.preventDefault(); event.stopPropagation(); overview.close(); documentRef.current?.focus(); } else onTreeKeyDown(event); }}>
          <div className="context-tree-header">FILES <span>{root.label}</span></div>
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
          {directories[keyFor(root.root_id, "")]?.status === "loading" ? <div className="context-tree-status">Loading…</div> : null}
          {directories[keyFor(root.root_id, "")]?.status === "error" && !directories[keyFor(root.root_id, "")]?.data ? <div className="context-tree-error">{directories[keyFor(root.root_id, "")]?.error}</div> : null}
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
        <main className="context-document" ref={documentRef} tabIndex={-1}>
          {renderDocument()}
        </main>
      </div>
      {resourcesOpen ? <ContextResources client={client} sessionId={sessionId} paneId={paneId} bindingId={bindingId} root={root} onClose={() => setResourcesOpen(false)} onChanged={files => {
          const paths = files.flatMap(file => { const parts = file.split("/"); return parts.slice(0, -1).map((_, index) => parts.slice(0, index + 1).join("/")); });
          setExpanded(current => new Set([...current, ...paths]));
          void loadDirectory(root, "", true);
          for (const path of new Set(paths)) void loadDirectory(root, path, true);
        }} onImported={(result) => {
          const parts = result.snapshot_path.split("/");
          const paths = parts.map((_, index) => parts.slice(0, index + 1).join("/"));
          setExpanded(current => new Set([...current, ...paths]));
          void loadDirectory(root, "", true);
          for (const path of paths) void loadDirectory(root, path, true);
        }} /> : null}
      {pickerOpen ? <FilePicker candidates={[...pickerIndex.entries.values()].map((entry) => ({ id: entry.entry_id, path: entry.path!, detail: entry.bytes === null ? undefined : `${entry.bytes} B` } satisfies FileNavigationCandidate))} loading={pickerIndex.loading} incomplete={pickerIndex.incomplete} onChoose={(candidate) => { const entry = pickerIndex.entries.get(candidate.path); if (entry) chooseEntry(entry); closeFilePicker(); focusContent(); }} onDismiss={() => { closeFilePicker(); focusContent(); }} /> : null}
    </section>
  );
}
