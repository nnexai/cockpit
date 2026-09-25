import { UiIcon } from "../UiIcon";
import { useFileOverview } from "../input/useFileOverview";
import { Fragment, type CSSProperties, type KeyboardEvent, type ReactNode, useCallback, useEffect, useId, useMemo, useRef, useState } from "react";
import type {
  ReviewChangedFile,
  ReviewComparison,
  ReviewFileDiff,
  ReviewFileRequest,
  ReviewSnapshot,
  ReviewSnapshotRequest,
} from "../../protocol/generated/v1";
import { FilePicker } from "../input/FilePicker";
import { FILE_NAVIGATION_EVENT, fileNavigationAction, type FileNavigationCandidate } from "../input/fileNavigation";
import { retainReviewScrollPosition, type ReviewViewState } from "../context/ContextViewer";
import { highlightLine } from "../viewer/highlight";
import { TreeSplitter, useTreeWidth, useWrapPreference } from "../viewer/ViewerLayout";
import "./review.css";

export type ReviewLineSelection = { fileId: string; side: "old" | "new"; start: number; end: number } | null;
export type ReviewPaneProps = {
  identity: string;
  sessionId: string;
  paneId: string;
  bindingId: string;
  repositoryId: string;
  snapshot: (request: ReviewSnapshotRequest, signal: AbortSignal) => Promise<ReviewSnapshot>;
  file: (request: ReviewFileRequest, signal: AbortSignal) => Promise<ReviewFileDiff>;
  selectedLines?: ReviewLineSelection;
  onCreateLineComment?: () => void;
  onCreateFileComment?: () => void;
  onOpenCommentOverview?: () => void;
  onOpenSource?: () => void;
  commentCount?: number | null;
  canCreateLineComment?: boolean;
  canCreateFileComment?: boolean;
  onSelectLines?: (file: ReviewChangedFile, side: "old" | "new", start: number, end: number, lines: string[], shift: boolean) => void;
  renderFile?: (snapshot: ReviewSnapshot, diff: ReviewFileDiff, content: (comments?: (diff: ReviewFileDiff, oldLine: number | null, newLine: number | null) => ReactNode) => ReactNode, loadSourcePage: (side: "old" | "new", offset: number) => Promise<ReviewFileDiff | null>) => ReactNode;
  presentationControls?: ReactNode;
  renderLineComments?: (diff: ReviewFileDiff, oldLine: number | null, newLine: number | null) => ReactNode;
  viewState?: ReviewViewState;
  onViewStateChange?: (state: ReviewViewState) => void;
};

const modes: Array<{ value: ReviewComparison; label: string }> = [
  { value: "all_local", label: "All local changes" },
  { value: "staged", label: "Staged" },
  { value: "unstaged", label: "Unstaged" },
  { value: "untracked", label: "Untracked" },
  { value: "branch", label: "Branch" },
];

function errorText(error: unknown): string {
  return error instanceof Error && error.message ? error.message : "Could not refresh local review.";
}

function fileLabel(file: ReviewChangedFile): string {
  if (file.old_path && file.new_path && file.old_path !== file.new_path) return `${file.old_path} → ${file.new_path}`;
  return file.new_path ?? file.old_path ?? "Unnamed change";
}

function filePath(file: ReviewChangedFile): string {
  return file.new_path ?? file.old_path ?? "Unnamed change";
}

function fileName(file: ReviewChangedFile): string {
  const path = filePath(file);
  return path.slice(path.lastIndexOf("/") + 1);
}

/** "modified · unstaged", or just "untracked" when both words agree. */
function fileState(file: ReviewChangedFile): string {
  const status = file.status.replaceAll("_", " ");
  const comparison = file.comparison.replaceAll("_", " ");
  return comparison === status || comparison === "all local" ? status : `${status} · ${comparison}`;
}

function fileStatus(file: ReviewChangedFile): string {
  return { added: "A", copied: "C", deleted: "D", modified: "M", renamed: "R", untracked: "?", binary: "B", mode_only: "M", submodule: "S", unreadable: "!" }[file.status] ?? "M";
}

type FileTree = { files: ReviewChangedFile[]; directories: Map<string, FileTree> };
function buildFileTree(files: ReviewChangedFile[]): FileTree {
  const root: FileTree = { files: [], directories: new Map() };
  for (const file of files) {
    const segments = filePath(file).split("/");
    const leaf = segments.pop();
    if (!leaf) { root.files.push(file); continue; }
    let current = root;
    for (const segment of segments) {
      let next = current.directories.get(segment);
      if (!next) { next = { files: [], directories: new Map() }; current.directories.set(segment, next); }
      current = next;
    }
    current.files.push(file);
  }
  return root;
}

function ReviewFileTree({ files, selected, onSelect }: { files: ReviewChangedFile[]; selected: string | null; onSelect: (fileId: string) => void }) {
  const tree = useMemo(() => buildFileTree(files), [files]);
  const duplicates = new Set(files.filter((item, index) => files.some((other, otherIndex) => index !== otherIndex && filePath(item) === filePath(other))).map(filePath));
  const renderFile = (item: ReviewChangedFile, depth: number, label = fileName(item)) => <button type="button" key={item.file_id} data-file-id={item.file_id} className={`review-file${item.file_id === selected ? " is-selected" : ""}`} style={{ paddingLeft: 8 + depth * 12 }} onClick={() => onSelect(item.file_id)} title={`${fileLabel(item)} · ${item.comparison}`} aria-label={`${filePath(item)}, ${item.status.replaceAll("_", " ")}, ${item.comparison.replaceAll("_", " ")}`}>
    <UiIcon name="file" /><span className="review-file-name">{label}</span>{duplicates.has(filePath(item)) ? <small>{item.comparison === "staged" ? "index" : "working"}</small> : null}<span className="review-file-stats">{item.additions != null && item.additions > 0 ? <em className="is-added">+{item.additions}</em> : null}{item.deletions != null && item.deletions > 0 ? <em className="is-deleted">−{item.deletions}</em> : null}</span>
  </button>;
  const renderNode = (node: FileTree, path: string, depth: number): ReactNode => <>{[...node.directories].sort(([a], [b]) => a.localeCompare(b)).map(([name, initial]) => {
    let label = name;
    let child = initial;
    while (child.files.length === 0 && child.directories.size === 1) {
      const [segment, next] = [...child.directories][0];
      label += `/${segment}`;
      child = next;
    }
    const directoryPath = path ? `${path}/${label}` : label;
    if (child.directories.size === 0 && child.files.length === 1) return renderFile(child.files[0], depth, `${label}/${fileName(child.files[0])}`);
    return <details className="review-file-directory" open key={directoryPath}><summary style={{ paddingLeft: 8 + depth * 12 }} title={directoryPath}>{label}/</summary>{renderNode(child, directoryPath, depth + 1)}</details>;
  })}{[...node.files].sort((a, b) => fileName(a).localeCompare(fileName(b))).map(item => renderFile(item, depth))}</>;
  return <>{renderNode(tree, "", 0)}</>;
}

function highlightedLine(text: string, path: string | null | undefined): ReactNode {
  const html = text ? highlightLine(text, path) : null;
  return html === null ? text : <span dangerouslySetInnerHTML={{ __html: html }} />;
}

function isEditingTarget(target: EventTarget | null): boolean {
  if (!(target instanceof HTMLElement)) return false;
  return target instanceof HTMLInputElement || target instanceof HTMLTextAreaElement || target instanceof HTMLSelectElement || target.isContentEditable;
}
type SourcePageRequest = {
  controller: AbortController;
  token: number;
  identity: string;
  reviewId: string;
  generation: number;
  fileId: string;
  side: "old" | "new";
  revision: string | null;
};

export function reviewScrollIdentity(sessionId: string, paneId: string, bindingId: string, reviewId: string, generation: number, comparison: ReviewComparison, kind: "diff" | "source", fileId: string, side: "old" | "new" | null, revision: string | null): string {
  return [sessionId, paneId, bindingId, reviewId, generation, comparison, kind, fileId, side ?? "", revision ?? ""].join("\u0000");
}


export function ReviewPane({ identity, sessionId, paneId, bindingId, repositoryId, snapshot, file, selectedLines = null, onCreateLineComment, onCreateFileComment, onOpenCommentOverview, onOpenSource, commentCount = null, canCreateLineComment = false, canCreateFileComment = false, onSelectLines, renderFile, presentationControls, renderLineComments, viewState, onViewStateChange }: ReviewPaneProps) {
  const baseId = useId();
  const [comparison, setComparison] = useState<ReviewComparison>(viewState?.comparison ?? "all_local");
  const [baseRef, setBaseRef] = useState(viewState?.baseRef ?? "");
  const [draftBaseRef, setDraftBaseRef] = useState(viewState?.draftBaseRef ?? "");
  const [review, setReview] = useState<ReviewSnapshot | null>(null);
  const [selected, setSelected] = useState<string | null>(viewState?.fileId ?? null);
  const selectedRef = useRef(selected);
  selectedRef.current = selected;
  const viewStateRef = useRef(viewState);
  viewStateRef.current = viewState;
  const [diff, setDiff] = useState<ReviewFileDiff | null>(null);
  const previousSelectedRef = useRef(selected);
  const [pending, setPending] = useState(false);
  const [hunkIndex, setHunkIndex] = useState(viewState?.hunkIndex ?? -1);
  const [scrollPosition, setScrollPosition] = useState(() => ({ identity: viewState?.scrollIdentity ?? null, top: viewState?.scrollTop ?? 0 }));
  const restoredDiff = useRef<string | null>(null);
  const diffRef = useRef<HTMLElement | null>(null);
  const filesRef = useRef<HTMLElement | null>(null);
  const paneRef = useRef<HTMLElement | null>(null);
  const [pickerOpen, setPickerOpen] = useState(false);
  const overview = useFileOverview(paneRef);
  const tree = useTreeWidth();
  const [wrap, toggleWrap] = useWrapPreference();
  const [error, setError] = useState<string | null>(null);
  const generationRef = useRef(0);
  const abortRef = useRef<AbortController | null>(null);
  const identityRef = useRef(identity);
  identityRef.current = identity;
  const comparisonRef = useRef(comparison);
  comparisonRef.current = comparison;
  const reviewRef = useRef(review);
  reviewRef.current = review;
  const diffStateRef = useRef(diff);
  diffStateRef.current = diff;
  const sourcePageRef = useRef<SourcePageRequest | null>(null);
  const sourcePageSequence = useRef(0);
  const invalidateSourcePage = useCallback(() => {
    sourcePageRef.current?.controller.abort();
    sourcePageRef.current = null;
    sourcePageSequence.current += 1;
  }, []);
  const selectedFile = useMemo(() => review?.files.find((item) => item.file_id === selected) ?? null, [review, selected]);
  const reviewMode = viewStateRef.current?.mode ?? "diff";
  const diffRevision = diff ? `${diff.file.old_revision ?? ""}\u0000${diff.file.new_revision ?? ""}` : null;
  const activeScrollIdentity = reviewMode === "diff"
    ? review && diff && diff.file.file_id === selected && diff.review_id === review.review_id && diff.generation === review.generation
      ? reviewScrollIdentity(sessionId, paneId, bindingId, review.review_id, review.generation, review.comparison, "diff", diff.file.file_id, null, diffRevision)
      : null
    : viewStateRef.current?.scrollIdentity ?? null;
  const scrollTop = reviewMode === "diff" && activeScrollIdentity
    ? scrollPosition.identity === activeScrollIdentity ? scrollPosition.top
      : viewStateRef.current?.scrollPositions?.[activeScrollIdentity] ?? (viewStateRef.current?.scrollIdentity === activeScrollIdentity ? viewStateRef.current.scrollTop : 0)
    : viewStateRef.current?.scrollTop ?? 0;
  useEffect(() => {
    const selectionForFile = selectedLines?.fileId === selected ? selectedLines : null;
    onViewStateChange?.({
      comparison,
      baseRef,
      draftBaseRef,
      fileId: selected,
      filePath: selectedFile?.new_path ?? selectedFile?.old_path ?? (selected === null ? null : viewStateRef.current?.filePath ?? null),
      side: selectionForFile?.side ?? null,
      selectionStart: selectionForFile?.start ?? null,
      selectionEnd: selectionForFile?.end ?? null,
      hunkIndex,
      scrollTop,
      scrollPositions: activeScrollIdentity && reviewMode === "diff"
        ? retainReviewScrollPosition(viewStateRef.current?.scrollPositions ?? {}, activeScrollIdentity, scrollTop)
        : viewStateRef.current?.scrollPositions ?? {},
      scrollIdentity: activeScrollIdentity ?? viewStateRef.current?.scrollIdentity ?? null,
      mode: reviewMode,
      commentCount: viewStateRef.current?.commentCount ?? null,
    });
  }, [activeScrollIdentity, baseRef, bindingId, comparison, draftBaseRef, hunkIndex, onViewStateChange, paneId, reviewMode, scrollTop, selected, selectedFile, selectedLines, sessionId]);

  const refresh = useCallback(async () => {
    invalidateSourcePage();
    abortRef.current?.abort();
    const controller = new AbortController();
    abortRef.current = controller;
    const generation = generationRef.current + 1;
    generationRef.current = generation;
    setError(null);
    if (comparison === "branch" && !baseRef) {
      setPending(false);
      setReview(null);
      setSelected(null);
      setDiff(null);
      return;
    }
    setPending(true);
    try {
      const next = await snapshot({ binding_id: bindingId, repository_id: repositoryId, comparison, base_ref: comparison === "branch" ? baseRef.trim() || null : null }, controller.signal);
      if (!controller.signal.aborted && identityRef.current === identity && generation === generationRef.current
        && next.binding_id === bindingId && next.session_id === sessionId && next.pane_id === paneId) {
        setReview(next);
        const retained = next.files.find((item) => item.file_id === selectedRef.current)
          ?? next.files.find((item) => (item.new_path ?? item.old_path) === viewStateRef.current?.filePath && item.comparison === comparison)
          ?? next.files[0];
        setSelected(retained?.file_id ?? null);
      }
    } catch (reason) {
      if (!controller.signal.aborted && identityRef.current === identity && generation === generationRef.current) setError(errorText(reason));
    } finally {
      if (!controller.signal.aborted && generation === generationRef.current) setPending(false);
    }
  }, [baseRef, bindingId, comparison, identity, invalidateSourcePage, paneId, repositoryId, sessionId, snapshot]);
  const editBaseRef = (value: string) => {
    invalidateSourcePage();
    abortRef.current?.abort();
    generationRef.current += 1;
    setDraftBaseRef(value);
    setBaseRef("");
    setPending(false);
    setReview(null);
    setSelected(null);
    setDiff(null);
    setError(null);
  };

  const submitComparison = () => {
    if (comparison === "branch" && draftBaseRef.trim() !== baseRef) setBaseRef(draftBaseRef.trim());
    else void refresh();
  };

  useEffect(() => { void refresh(); }, [refresh]);
  useEffect(() => () => {
    abortRef.current?.abort();
    invalidateSourcePage();
  }, [invalidateSourcePage]);
  useEffect(() => {
    if (!review || !selected) { setDiff(null); return; }
    const controller = new AbortController();
    const generation = generationRef.current;
    if (previousSelectedRef.current !== selected) setDiff(null);
    previousSelectedRef.current = selected;
    void file({ binding_id: bindingId, review_id: review.review_id, generation: review.generation, file_id: selected, source_side: null, source_offset: 0, source_revision: null }, controller.signal)
      .then((next) => {
        if (!controller.signal.aborted && identityRef.current === identity && generation === generationRef.current
          && next.review_id === review.review_id && next.generation === review.generation && next.binding_id === bindingId) setDiff(next);
      })
      .catch((reason) => { if (!controller.signal.aborted && generation === generationRef.current) setError(errorText(reason)); });
    return () => controller.abort();
  }, [bindingId, file, identity, review, selected]);
  useEffect(() => {
    if (reviewMode !== "diff" || !activeScrollIdentity) { restoredDiff.current = null; return; }
    if (!diffRef.current || restoredDiff.current === activeScrollIdentity) return;
    restoredDiff.current = activeScrollIdentity;
    diffRef.current.scrollTop = scrollTop;
    setScrollPosition({ identity: activeScrollIdentity, top: scrollTop });
  }, [activeScrollIdentity, reviewMode, scrollTop]);
  useEffect(() => {
    return () => invalidateSourcePage();
  }, [bindingId, comparison, diff?.file.file_id, diff?.file.new_revision, diff?.file.old_revision, identity, invalidateSourcePage, review?.generation, review?.review_id, selected, selectedLines?.side]);
  const loadSourcePage = useCallback(async (side: "old" | "new", offset: number): Promise<ReviewFileDiff | null> => {
    if (!review || !diff) return null;
    invalidateSourcePage();
    const revision = side === "old" ? diff.file.old_revision ?? null : diff.file.new_revision ?? null;
    const sourceIdentity = reviewScrollIdentity(sessionId, paneId, bindingId, review.review_id, review.generation, review.comparison, "source", diff.file.file_id, side, revision);
    const token = sourcePageSequence.current + 1;
    sourcePageSequence.current = token;
    const controller = new AbortController();
    sourcePageRef.current = { controller, token, identity: sourceIdentity, reviewId: review.review_id, generation: review.generation, fileId: diff.file.file_id, side, revision };
    try {
      const next = await file({
        binding_id: bindingId,
        review_id: review.review_id,
        generation: review.generation,
        file_id: diff.file.file_id,
        source_side: side,
        source_offset: offset,
        source_revision: revision,
      }, controller.signal);
      const active = sourcePageRef.current;
      const currentReview = reviewRef.current;
      const currentDiff = diffStateRef.current;
      if (controller.signal.aborted || !active || active.token !== token || identityRef.current !== identity
        || comparisonRef.current !== comparison || selectedRef.current !== selected
        || currentReview?.review_id !== review.review_id || currentReview.generation !== review.generation
        || currentDiff?.file.file_id !== diff.file.file_id
        || next.binding_id !== bindingId || next.session_id !== sessionId || next.pane_id !== paneId
        || next.review_id !== review.review_id || next.generation !== review.generation
        || next.file.file_id !== diff.file.file_id
        || (revision !== null && (side === "old" ? next.file.old_revision : next.file.new_revision) !== revision)) return null;
      return next;
    } catch (reason) {
      const active = sourcePageRef.current;
      const currentReview = reviewRef.current;
      const currentDiff = diffStateRef.current;
      if (controller.signal.aborted || !active || active.token !== token || identityRef.current !== identity
        || comparisonRef.current !== comparison || selectedRef.current !== selected
        || currentReview?.review_id !== review.review_id || currentReview.generation !== review.generation
        || currentDiff?.file.file_id !== diff.file.file_id) return null;
      setError(errorText(reason));
      return null;
    } finally {
      if (sourcePageRef.current?.token === token) sourcePageRef.current = null;
    }
  }, [bindingId, comparison, diff, file, identity, invalidateSourcePage, paneId, review, selected, sessionId]);

  const fileNavigationOrder = () => {
    if (!review?.files.length) return [];
    const byId = new Map(review.files.map((item) => [item.file_id, item]));
    const rendered = [...(filesRef.current?.querySelectorAll<HTMLButtonElement>(".review-file") ?? [])]
      .map((button) => byId.get(button.dataset.fileId ?? ""))
      .filter((item): item is ReviewChangedFile => item !== undefined);
    return rendered.length === review.files.length ? rendered : review.files;
  };
  const selectFileAt = (index: number, focus = false) => {
    const files = fileNavigationOrder();
    if (!files.length) return;
    const nextIndex = Math.max(0, Math.min(files.length - 1, index));
    const next = files[nextIndex];
    setSelected(next.file_id);
    setHunkIndex(-1);
    const button = [...(filesRef.current?.querySelectorAll<HTMLButtonElement>(".review-file") ?? [])]
      .find((candidate) => candidate.dataset.fileId === next.file_id);
    for (let parent = button?.parentElement; parent; parent = parent.parentElement) {
      if (parent instanceof HTMLDetailsElement) parent.open = true;
    }
    if (focus) button?.focus();
  };
  const focusTree = useCallback(() => {
    overview.show();
    requestAnimationFrame(() => {
    const rows = [...(filesRef.current?.querySelectorAll<HTMLButtonElement>(".review-file") ?? [])];
    const button = rows.find((row) => row.dataset.fileId === selected) ?? rows[0];
    for (let parent = button?.parentElement; parent && parent !== filesRef.current; parent = parent.parentElement) {
      if (parent instanceof HTMLDetailsElement) parent.open = true;
    }
    (button ?? filesRef.current)?.focus();
    });
  }, [selected]);
  const focusContent = useCallback(() => requestAnimationFrame(() => diffRef.current?.focus({ preventScroll: true })), []);
  const moveFile = (delta: number) => {
    const files = fileNavigationOrder();
    if (!files.length) return;
    const index = files.findIndex(item => item.file_id === selected);
    selectFileAt((index < 0 ? 0 : index) + delta);
  };
  const moveHunk = (delta: number) => {
    const hunks = diffRef.current?.querySelectorAll<HTMLElement>(".review-hunk");
    if (!hunks?.length || !diff) return;
    const index = hunkIndex < 0 ? (delta > 0 ? 0 : hunks.length - 1) : Math.max(0, Math.min(hunks.length - 1, hunkIndex + delta));
    const selection = selectedLines?.fileId === diff.file.file_id ? selectedLines : null;
    const side = selection?.side ?? (diff.new_source === null && diff.old_source !== null ? "old" : "new");
    const line = diff.hunks[index]?.lines.find((candidate) => side === "old" ? candidate.old_line !== null : candidate.new_line !== null);
    setHunkIndex(index);
    hunks[index].scrollIntoView({ block: "nearest" });
    hunks[index].focus({ preventScroll: true });
    if (line) {
      const lineNumber = side === "old" ? line.old_line! : line.new_line!;
      onSelectLines?.(diff.file, side, lineNumber, lineNumber, [line.text], false);
    }
  };
  const moveSourceLine = (direction: "next" | "previous" | "first" | "last", extend: boolean, target: EventTarget | null) => {
    const lines = [...(diffRef.current?.querySelectorAll<HTMLButtonElement>(".context-source-line") ?? [])];
    if (!lines.length) return false;
    const sourceTarget = target instanceof HTMLElement ? target.closest<HTMLButtonElement>(".context-source-line") : null;
    const selectedIndex = displayedSelection ? lines.findIndex((line) => Number(line.dataset.line) === displayedSelection.end) : -1;
    const currentIndex = sourceTarget ? lines.indexOf(sourceTarget) : selectedIndex;
    const next = direction === "first" ? lines[0]
      : direction === "last" ? lines[lines.length - 1]
        : currentIndex < 0 ? (direction === "next" ? lines[0] : lines[lines.length - 1])
          : lines[Math.max(0, Math.min(lines.length - 1, currentIndex + (direction === "next" ? 1 : -1)))];
    next.focus();
    next.scrollIntoView({ block: "nearest" });
    next.dispatchEvent(new MouseEvent("click", { bubbles: true, shiftKey: extend }));
    return true;
  };
  const moveLine = (direction: "next" | "previous" | "first" | "last", extend: boolean) => {
    if (!diff) return;
    const selection = selectedLines?.fileId === diff.file.file_id ? selectedLines : null;
    const side = selection?.side ?? (diff.new_source === null && diff.old_source !== null ? "old" : "new");
    const lines = diff.hunks.flatMap((hunk) => hunk.lines.flatMap((line) => {
      const lineNumber = side === "old" ? line.old_line : line.new_line;
      return lineNumber === null ? [] : [{ lineNumber, text: line.text }];
    }));
    if (!lines.length) return;
    const currentIndex = selection ? lines.findIndex((line) => line.lineNumber === selection.end) : -1;
    const target = direction === "first" ? lines[0]
      : direction === "last" ? lines[lines.length - 1]
        : currentIndex < 0 ? (direction === "next" ? lines[0] : lines[lines.length - 1])
          : lines[Math.max(0, Math.min(lines.length - 1, currentIndex + (direction === "next" ? 1 : -1)))];
    onSelectLines?.(diff.file, side, target.lineNumber, target.lineNumber, [target.text], extend);
    const lineElement = diffRef.current?.querySelector<HTMLElement>(`.review-line[data-${side}-line="${target.lineNumber}"]`);
    lineElement?.scrollIntoView({ block: "nearest" });
    lineElement?.focus({ preventScroll: true });
  };
  const onDiffKeyDown = (event: KeyboardEvent<HTMLElement>) => {
    if (event.ctrlKey || event.metaKey || isEditingTarget(event.target)) return;
    if (event.altKey) {
      if (event.key === "ArrowDown" || event.key === "ArrowUp") { event.preventDefault(); moveHunk(event.key === "ArrowDown" ? 1 : -1); }
      if (event.key === "ArrowRight" || event.key === "ArrowLeft") { event.preventDefault(); moveFile(event.key === "ArrowRight" ? 1 : -1); }
      return;
    }
    if (event.key.toLowerCase() === "c") { event.preventDefault(); if (event.shiftKey) onCreateFileComment?.(); else onCreateLineComment?.(); return; }
    if (event.key === "ArrowDown" || event.key === "ArrowUp") { event.preventDefault(); const direction = event.key === "ArrowDown" ? "next" : "previous"; if (!moveSourceLine(direction, event.shiftKey, event.target)) moveLine(direction, event.shiftKey); }
    if (event.key === "Home" || event.key === "End") { event.preventDefault(); const direction = event.key === "Home" ? "first" : "last"; if (!moveSourceLine(direction, event.shiftKey, event.target)) moveLine(direction, event.shiftKey); }
  };
  const onFilesKeyDown = (event: KeyboardEvent<HTMLElement>) => {
    if (event.altKey || event.ctrlKey || event.metaKey || isEditingTarget(event.target)) return;
    const files = fileNavigationOrder();
    if (!files.length) return;
    const target = event.target instanceof HTMLElement ? event.target.closest<HTMLButtonElement>(".review-file") : null;
    const index = files.findIndex(item => item.file_id === (target?.dataset.fileId ?? selected));
    if (event.key === "ArrowDown" || event.key === "ArrowUp") { event.preventDefault(); selectFileAt((index < 0 ? 0 : index) + (event.key === "ArrowDown" ? 1 : -1), true); }
    if (event.key === "Home" || event.key === "End") { event.preventDefault(); selectFileAt(event.key === "Home" ? 0 : files.length - 1, true); }
  };
  useEffect(() => {
    const onNavigation = (event: Event) => {
      if (!paneRef.current?.contains(document.activeElement)) return;
      const action = fileNavigationAction(event);
      if (action === "open-picker") setPickerOpen(true);
      else if (action === "focus-tree") focusTree();
      else if (action === "focus-content") focusContent();
    };
    window.addEventListener(FILE_NAVIGATION_EVENT, onNavigation);
    return () => window.removeEventListener(FILE_NAVIGATION_EVENT, onNavigation);
  }, [focusContent, focusTree]);
  const displayedSelection = selectedLines?.fileId === selected ? selectedLines : null;
  const isSelectedLine = (side: "old" | "new", lineNumber: number | null) => {
    const selection = selectedLines?.fileId === diff?.file.file_id ? selectedLines : null;
    return Boolean(lineNumber !== null && selection && selection.side === side && lineNumber >= Math.min(selection.start, selection.end) && lineNumber <= Math.max(selection.start, selection.end));
  };
  const lineDigits = useMemo(() => String(diff?.hunks.reduce((largest, hunk) => hunk.lines.reduce((inner, line) => Math.max(inner, line.old_line ?? 0, line.new_line ?? 0), largest), 0) ?? 0).length, [diff]);
  const diffContent = (comments = renderLineComments) => {
    const shownFile = diff?.file ?? selectedFile;
    return <>
        {shownFile ? <header><strong title={fileLabel(shownFile)}>{fileName(shownFile)}</strong><span className="viewer-secondary-metadata">{fileState(shownFile)}</span>{review ? <details className="viewer-details"><summary aria-label="Review details" title="Review details"><UiIcon name="info" /></summary><dl><dt>Old path</dt><dd><code>{shownFile.old_path ?? "None"}</code></dd><dt>New path</dt><dd><code>{shownFile.new_path ?? "None"}</code></dd><dt>Checkout</dt><dd><code>{review.checkout_path}</code></dd><dt>Review identity</dt><dd><code>{review.review_id}</code></dd><dt>Source identity</dt><dd><code>{review.source_id}</code></dd><dt>Comparison</dt><dd>{review.comparison.replaceAll("_", " ")}</dd><dt>Base revision</dt><dd><code>{review.base_revision ?? "Unavailable"}</code></dd><dt>Head revision</dt><dd><code>{review.head_revision ?? "Unavailable"}</code></dd><dt>Index revision</dt><dd><code>{review.index_revision}</code></dd><dt>Worktree revision</dt><dd><code>{review.worktree_revision}</code></dd><dt>File identity</dt><dd><code>{shownFile.file_id}</code></dd><dt>Old revision</dt><dd><code>{shownFile.old_revision ?? "Unavailable"}</code></dd><dt>New revision</dt><dd><code>{shownFile.new_revision ?? "Unavailable"}</code></dd>{[...review.diagnostics, ...(diff?.diagnostics ?? [])].map((diagnostic, index) => <Fragment key={`${diagnostic.code}-${index}`}><dt>Diagnostic</dt><dd key={`diagnostic-value-${diagnostic.code}-${index}`}><code>{diagnostic.code}</code>{diagnostic.path ? ` · ${diagnostic.path}` : ""} · {diagnostic.message}</dd></Fragment>)}</dl></details> : null}</header> : null}
        {shownFile?.binary ? <p className="review-empty">Binary file — no text diff.</p> : null}
        {diff?.hunks.map((hunk, index) => <section className={`review-hunk${hunkIndex === index ? " is-current" : ""}`} tabIndex={-1} key={`${index}-${hunk.old_start}-${hunk.new_start}`}>
          <header>@@ -{hunk.old_start} +{hunk.new_start} @@</header>
          {hunk.lines.map((line, lineIndex) => <Fragment key={`${lineIndex}-${line.old_line ?? ""}-${line.new_line ?? ""}`}><button type="button" className={`review-line is-${line.kind}${isSelectedLine("old", line.old_line) || isSelectedLine("new", line.new_line) ? " is-selected" : ""}`} data-old-line={line.old_line ?? undefined} data-new-line={line.new_line ?? undefined} onClick={(event) => {
            setHunkIndex(index);
            const oldGutter = event.target instanceof HTMLElement && event.target.closest("[data-side=old]");
            const side = line.kind === "deleted" || (oldGutter && line.old_line !== null) ? "old" : "new";
            const lineNumber = side === "old" ? line.old_line! : line.new_line!;
            onSelectLines?.(diff.file, side, lineNumber, lineNumber, [line.text], event.shiftKey);
          }}>
            <span data-side="old">{line.old_line ?? ""}</span><span data-side="new">{line.new_line ?? ""}</span><code><span className="review-diff-marker">{line.kind === "added" ? "+" : line.kind === "deleted" ? "-" : " "}</span>{highlightedLine(line.text, diff.file.new_path ?? diff.file.old_path)}</code>
          </button>{comments?.(diff, line.old_line, line.new_line)}</Fragment>)}
        </section>)}
        {diff?.diagnostics.filter(item => !(diff.file.binary && /binary/i.test(item.message))).map((item, index) => <p key={`${item.code}-${index}`} className="review-notice">{item.message}</p>)}
        {diff && diff.hunks.length === 0 && !diff.file.binary ? <p className="review-empty">No textual hunk is available for this change.</p> : null}
        {review && review.files.length === 0 && !pending && !error ? <div className="review-empty" role="status"><UiIcon name="file" /><span>No changes in this comparison.</span><button type="button" onClick={submitComparison}>Refresh</button></div> : null}
        {!review && !pending ? <p className="review-empty">{comparison === "branch" ? "Enter a base ref, then press Enter or Refresh to compare branches." : "Choose a verified Review pane to load a local Git snapshot."}</p> : null}
  </>;
  };

  return <section className="review-pane" aria-label="Local review" ref={paneRef} onKeyDownCapture={(event) => {
    if (isEditingTarget(event.target)) return;
    if ((event.ctrlKey || event.metaKey) && !event.altKey && event.key.toLowerCase() === "p") { event.preventDefault(); event.stopPropagation(); setPickerOpen(true); }
    if (event.altKey && !event.ctrlKey && !event.metaKey && event.key === "1") { event.preventDefault(); focusTree(); }
    if (event.altKey && !event.ctrlKey && !event.metaKey && event.key === "2") { event.preventDefault(); focusContent(); }
    if (event.altKey && !event.ctrlKey && !event.metaKey && event.code === "KeyZ") { event.preventDefault(); toggleWrap(); }
  }}>
    <header className="review-toolbar">
      <label htmlFor={`${baseId}-mode`} className="sr-only">Comparison</label>
      <select id={`${baseId}-mode`} value={comparison} disabled={pending} onChange={(event) => setComparison(event.target.value as ReviewComparison)}>
        {modes.map((mode) => <option key={mode.value} value={mode.value}>{mode.label}</option>)}
      </select>
      {comparison === "branch" ? <label className="review-base" htmlFor={`${baseId}-base`}>Base ref <input id={`${baseId}-base`} value={draftBaseRef} onChange={(event) => editBaseRef(event.target.value)} onKeyDown={(event) => { if (event.key === "Enter" && !event.nativeEvent.isComposing && draftBaseRef.trim()) { event.preventDefault(); submitComparison(); } }} placeholder="main" /></label> : null}
      <button type="button" className="viewer-overview-trigger" onClick={overview.toggle} aria-expanded={overview.open} aria-controls={`${baseId}-file-overview`} aria-label="Toggle file overview"><UiIcon name="sidebar" /><span className="viewer-overview-label">Files</span> <span className="viewer-count">{review?.files.length ?? 0}</span></button>
      <button type="button" className="viewer-file-picker-trigger" onClick={() => setPickerOpen(true)} disabled={!review?.files.length} aria-label="Choose review file" title="Choose review file"><UiIcon name="search" /></button>
      <details className="viewer-navigation"><summary aria-label="Review navigation" title="Review navigation"><UiIcon name="down" /></summary><div className="review-nav" aria-label="Review navigation">
        <button type="button" aria-label="Previous file" title="Previous file (Alt+Left)" onClick={() => moveFile(-1)} disabled={!review?.files.length}>←</button>
        <button type="button" aria-label="Next file" title="Next file (Alt+Right)" onClick={() => moveFile(1)} disabled={!review?.files.length}>→</button>
        <button type="button" aria-label="Previous hunk" title="Previous hunk (Alt+Up)" onClick={() => moveHunk(-1)} disabled={!diff?.hunks.length}>↑</button>
        <button type="button" aria-label="Next hunk" title="Next hunk (Alt+Down)" onClick={() => moveHunk(1)} disabled={!diff?.hunks.length}>↓</button>
      </div></details>
      <span className="review-toolbar-spacer" />{presentationControls}
      <button type="button" className="viewer-wrap-toggle" aria-pressed={wrap} onClick={toggleWrap} title={wrap ? "Long lines wrap (Alt+Z)" : "Long lines scroll (Alt+Z)"}><UiIcon name="wrap" /><span className="viewer-wrap-label">Wrap</span></button>
      <button type="button" onClick={submitComparison} disabled={pending || (comparison === "branch" && !draftBaseRef.trim())} aria-label={pending ? "Refreshing…" : "Refresh"} title="Refresh"><UiIcon name="refresh" /></button>
    </header>
    {error ? <p className="review-notice review-error" role="alert">{error}</p> : null}
    {diff?.truncated ? <p className="review-notice review-warning">This file's unified diff is partial. <button type="button" onClick={onOpenSource}>Open source</button> or narrow the comparison to inspect it safely.</p> : null}
    {review?.truncated ? <p className="review-notice review-warning">Review output reached a configured limit. Refresh with a narrower scope.</p> : null}
    <div className={`review-body${overview.open ? " has-file-overview" : ""}`} style={tree.style}>
      {overview.narrow && overview.open ? <button type="button" className="viewer-overview-backdrop" aria-label="Close file overview" onClick={overview.close} /> : null}
      <nav id={`${baseId}-file-overview`} className={`review-files${overview.open ? " is-overview-open" : ""}`} aria-label="Changed files" ref={filesRef} onKeyDown={(event) => { if (event.key === "Escape" && overview.narrow) { event.preventDefault(); event.stopPropagation(); overview.close(); diffRef.current?.focus({ preventScroll: true }); } else onFilesKeyDown(event); }}>
        {review ? <ReviewFileTree files={review.files} selected={selected} onSelect={(id) => { setSelected(id); overview.select(); }} /> : null}
      </nav>
      {overview.open && !overview.narrow ? <TreeSplitter width={tree.width} onChange={tree.setWidth} /> : null}
      <main className={`review-diff${wrap ? " is-wrapped" : ""}`} style={{ "--diff-digits": `${Math.max(2, lineDigits)}ch` } as CSSProperties} aria-label="Unified diff" tabIndex={0} ref={diffRef} onScroll={(event) => {
        if (reviewMode === "diff" && activeScrollIdentity) setScrollPosition({ identity: activeScrollIdentity, top: event.currentTarget.scrollTop });
      }} onPointerDown={(event) => { if (event.target === event.currentTarget) event.currentTarget.focus({ preventScroll: true }); }} onKeyDown={onDiffKeyDown}>
        {renderFile && review && diff ? renderFile(review, diff, diffContent, loadSourcePage) : diffContent()}
      </main>
    </div>
    <footer className="review-status" aria-label="Review shortcuts"><span>{displayedSelection ? `${displayedSelection.side} · lines ${Math.min(displayedSelection.start, displayedSelection.end)}–${Math.max(displayedSelection.start, displayedSelection.end)}` : ""}</span><span className="review-status-actions"><button type="button" onClick={onCreateLineComment} disabled={!canCreateLineComment} title={canCreateLineComment ? "Comment on selected lines (C)" : "Select review lines before commenting"}><kbd>C</kbd> comment</button><button type="button" onClick={onCreateFileComment} disabled={!canCreateFileComment} title={canCreateFileComment ? "Comment on whole file (Shift+C)" : "Review source is not ready"}><kbd>Shift+C</kbd> file</button><button type="button" onClick={onOpenCommentOverview} title="Open comments overview">{commentCount !== null ? `${commentCount} comments` : review && review.files.length === 0 && !pending ? "Comments" : "Loading comments…"}</button></span></footer>
    {pickerOpen ? <FilePicker candidates={(review?.files ?? []).map((item) => ({ id: item.file_id, path: filePath(item), detail: `${fileStatus(item)} · ${item.comparison.replaceAll("_", " ")}` } satisfies FileNavigationCandidate))} onChoose={(candidate) => { setSelected(candidate.id); setHunkIndex(-1); setPickerOpen(false); focusContent(); }} onDismiss={() => { setPickerOpen(false); focusContent(); }} /> : null}
  </section>;
}
