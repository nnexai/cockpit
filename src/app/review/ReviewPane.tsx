import { Fragment, type KeyboardEvent, type ReactNode, useCallback, useEffect, useId, useMemo, useRef, useState } from "react";
import type {
  ReviewChangedFile,
  ReviewComparison,
  ReviewFileDiff,
  ReviewSnapshot,
  ReviewSnapshotRequest,
} from "../../protocol/generated/v1";
import "./review.css";

export type ReviewLineSelection = { fileId: string; side: "old" | "new"; start: number; end: number } | null;
export type ReviewPaneProps = {
  identity: string;
  sessionId: string;
  paneId: string;
  bindingId: string;
  repositoryId: string;
  snapshot: (request: ReviewSnapshotRequest, signal: AbortSignal) => Promise<ReviewSnapshot>;
  file: (request: { binding_id: string; review_id: string; generation: number; file_id: string }, signal: AbortSignal) => Promise<ReviewFileDiff>;
  selectedLines?: ReviewLineSelection;
  onCreateLineComment?: () => void;
  onCreateFileComment?: () => void;
  onOpenCommentOverview?: () => void;
  commentCount?: number;
  canCreateLineComment?: boolean;
  canCreateFileComment?: boolean;
  onSelectLines?: (file: ReviewChangedFile, side: "old" | "new", start: number, end: number, lines: string[], shift: boolean) => void;
  renderFile?: (snapshot: ReviewSnapshot, diff: ReviewFileDiff, content: (comments?: (diff: ReviewFileDiff, oldLine: number | null, newLine: number | null) => ReactNode) => ReactNode) => ReactNode;
  renderLineComments?: (diff: ReviewFileDiff, oldLine: number | null, newLine: number | null) => ReactNode;
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
    <b>{fileStatus(item)}</b><span className="review-file-name">{label}</span>{duplicates.has(filePath(item)) ? <small>{item.comparison === "staged" ? "index" : "working"}</small> : null}<span className="review-file-stats">{item.additions != null && item.additions > 0 ? <em className="is-added">+{item.additions}</em> : null}{item.deletions != null && item.deletions > 0 ? <em className="is-deleted">−{item.deletions}</em> : null}</span>
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
  const codeLike = /\.(?:[cm]?[jt]sx?|rs|py|go|java|json|ya?ml|toml|md)$/i.test(path ?? "");
  if (!codeLike || !text) return text;
  const tokens = text.split(/(\/\/.*|#.*|"[^"]*"|'[^']*'|`[^`]*`|\b(?:const|let|var|function|return|if|else|for|while|match|pub|fn|struct|impl|use|import|from|export|type|interface|async|await|true|false|null|None)\b|\b\d+(?:\.\d+)?\b)/g);
  return tokens.map((token, index) => {
    const kind = token.startsWith("//") || token.startsWith("#") ? "comment"
      : /^("|'|`)/.test(token) ? "string"
        : /^(const|let|var|function|return|if|else|for|while|match|pub|fn|struct|impl|use|import|from|export|type|interface|async|await|true|false|null|None)$/.test(token) ? "keyword"
          : /^\d/.test(token) ? "number" : null;
    return kind ? <span className={`review-token is-${kind}`} key={index}>{token}</span> : token;
  });
}

function isEditingTarget(target: EventTarget | null): boolean {
  if (!(target instanceof HTMLElement)) return false;
  return target instanceof HTMLInputElement || target instanceof HTMLTextAreaElement || target instanceof HTMLSelectElement || target.isContentEditable;
}

export function ReviewPane({ identity, sessionId, paneId, bindingId, repositoryId, snapshot, file, selectedLines = null, onCreateLineComment, onCreateFileComment, onOpenCommentOverview, commentCount = 0, canCreateLineComment = false, canCreateFileComment = false, onSelectLines, renderFile, renderLineComments }: ReviewPaneProps) {
  const baseId = useId();
  const [comparison, setComparison] = useState<ReviewComparison>("all_local");
  const [baseRef, setBaseRef] = useState("");
  const [review, setReview] = useState<ReviewSnapshot | null>(null);
  const [selected, setSelected] = useState<string | null>(null);
  const [diff, setDiff] = useState<ReviewFileDiff | null>(null);
  const [pending, setPending] = useState(false);
  const [hunkIndex, setHunkIndex] = useState(-1);
  const diffRef = useRef<HTMLElement | null>(null);
  const filesRef = useRef<HTMLElement | null>(null);
  const [error, setError] = useState<string | null>(null);
  const generationRef = useRef(0);
  const abortRef = useRef<AbortController | null>(null);
  const identityRef = useRef(identity);
  identityRef.current = identity;

  const selectedFile = useMemo(() => review?.files.find((item) => item.file_id === selected) ?? null, [review, selected]);
  const refresh = useCallback(async () => {
    abortRef.current?.abort();
    const controller = new AbortController();
    abortRef.current = controller;
    const generation = generationRef.current + 1;
    generationRef.current = generation;
    setPending(true);
    setError(null);
    try {
      const next = await snapshot({ binding_id: bindingId, repository_id: repositoryId, comparison, base_ref: comparison === "branch" ? baseRef.trim() || null : null }, controller.signal);
      if (!controller.signal.aborted && identityRef.current === identity && generation === generationRef.current
        && next.binding_id === bindingId && next.session_id === sessionId && next.pane_id === paneId) {
        setReview(next);
        setSelected(next.files[0]?.file_id ?? null);
        setDiff(null);
      }
    } catch (reason) {
      if (!controller.signal.aborted && identityRef.current === identity && generation === generationRef.current) setError(errorText(reason));
    } finally {
      if (!controller.signal.aborted && generation === generationRef.current) setPending(false);
    }
  }, [baseRef, bindingId, comparison, identity, paneId, repositoryId, sessionId, snapshot]);

  useEffect(() => { void refresh(); }, [refresh]);
  useEffect(() => () => abortRef.current?.abort(), []);
  useEffect(() => {
    if (!review || !selected) { setDiff(null); return; }
    const controller = new AbortController();
    const generation = generationRef.current;
    setDiff(null);
    void file({ binding_id: bindingId, review_id: review.review_id, generation: review.generation, file_id: selected }, controller.signal)
      .then((next) => {
        if (!controller.signal.aborted && identityRef.current === identity && generation === generationRef.current
          && next.review_id === review.review_id && next.generation === review.generation && next.binding_id === bindingId) setDiff(next);
      })
      .catch((reason) => { if (!controller.signal.aborted && generation === generationRef.current) setError(errorText(reason)); });
    return () => controller.abort();
  }, [bindingId, file, identity, review, selected]);
  useEffect(() => { setHunkIndex(-1); }, [diff?.file.file_id]);

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
  const moveFile = (delta: number) => {
    const files = fileNavigationOrder();
    if (!files.length) return;
    const index = files.findIndex(item => item.file_id === selected);
    selectFileAt((index < 0 ? 0 : index) + delta);
  };
  const moveHunk = (delta: number) => {
    const hunks = diffRef.current?.querySelectorAll<HTMLElement>(".review-hunk");
    if (!hunks?.length || !diff) return;
    const index = Math.max(0, Math.min(hunks.length - 1, hunkIndex + delta));
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
    const currentIndex = selection ? Math.max(0, lines.findIndex((line) => line.lineNumber === selection.end)) : -1;
    const target = direction === "first" ? lines[0]
      : direction === "last" ? lines[lines.length - 1]
        : currentIndex < 0 ? (direction === "next" ? lines[0] : lines[lines.length - 1])
          : lines[Math.max(0, Math.min(lines.length - 1, currentIndex + (direction === "next" ? 1 : -1)))];
    onSelectLines?.(diff.file, side, target.lineNumber, target.lineNumber, [target.text], extend);
    diffRef.current?.querySelector<HTMLElement>(`.review-line[data-${side}-line="${target.lineNumber}"]`)?.scrollIntoView({ block: "nearest" });
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
    const index = files.findIndex(item => item.file_id === selected);
    if (event.key === "ArrowDown" || event.key === "ArrowUp") { event.preventDefault(); selectFileAt((index < 0 ? 0 : index) + (event.key === "ArrowDown" ? 1 : -1), true); }
    if (event.key === "Home" || event.key === "End") { event.preventDefault(); selectFileAt(event.key === "Home" ? 0 : files.length - 1, true); }
  };
  const displayedSelection = selectedLines?.fileId === selected ? selectedLines : null;
  const isSelectedLine = (side: "old" | "new", lineNumber: number | null) => {
    const selection = selectedLines?.fileId === diff?.file.file_id ? selectedLines : null;
    return Boolean(lineNumber !== null && selection && selection.side === side && lineNumber >= Math.min(selection.start, selection.end) && lineNumber <= Math.max(selection.start, selection.end));
  };
  const diffContent = (comments = renderLineComments) => <>
        {selectedFile ? <header><code>{fileLabel(selectedFile)}</code><span>{selectedFile.summary}</span></header> : null}
        {selectedFile?.binary ? <p className="review-empty">Binary content has no text anchors.</p> : null}
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
        {diff?.diagnostics.map((item, index) => <p key={`${item.code}-${index}`} className="review-notice">{item.message}</p>)}
        {diff && diff.hunks.length === 0 && !selectedFile?.binary ? <p className="review-empty">No textual hunk is available for this change.</p> : null}
        {!review && !pending ? <p className="review-empty">Choose a verified Review pane to load a local Git snapshot.</p> : null}
  </>;

  return <section className="review-pane" aria-label="Local review">
    <header className="review-toolbar">
      <label htmlFor={`${baseId}-mode`} className="sr-only">Comparison</label>
      <select id={`${baseId}-mode`} value={comparison} disabled={pending} onChange={(event) => setComparison(event.target.value as ReviewComparison)}>
        {modes.map((mode) => <option key={mode.value} value={mode.value}>{mode.label}</option>)}
      </select>
      {comparison === "branch" ? <label className="review-base" htmlFor={`${baseId}-base`}>Base ref <input id={`${baseId}-base`} value={baseRef} onChange={(event) => setBaseRef(event.target.value)} placeholder="main" disabled={pending} /></label> : null}
      <div className="review-nav" aria-label="Review navigation">
        <button type="button" aria-label="Previous file" title="Previous file (Alt+Left)" onClick={() => moveFile(-1)} disabled={!review?.files.length}>←</button>
        <button type="button" aria-label="Next file" title="Next file (Alt+Right)" onClick={() => moveFile(1)} disabled={!review?.files.length}>→</button>
        <button type="button" aria-label="Previous hunk" title="Previous hunk (Alt+Up)" onClick={() => moveHunk(-1)} disabled={!diff?.hunks.length}>↑</button>
        <button type="button" aria-label="Next hunk" title="Next hunk (Alt+Down)" onClick={() => moveHunk(1)} disabled={!diff?.hunks.length}>↓</button>
      </div>
      <span className="review-toolbar-spacer" />
      <button type="button" onClick={() => void refresh()} disabled={pending || (comparison === "branch" && !baseRef.trim())}>{pending ? "Refreshing…" : "Refresh"}</button>
    </header>
    {error ? <p className="review-notice review-error" role="alert">{error}</p> : null}
    {diff?.truncated ? <p className="review-notice review-warning">This file reached the preview limit. Comments require an available complete source.</p> : null}
    {review?.truncated ? <p className="review-notice review-warning">Review output reached a configured limit. Refresh with a narrower scope.</p> : null}
    <div className="review-body">
      <nav className="review-files" aria-label="Changed files" ref={filesRef} onKeyDown={onFilesKeyDown}>
        <header><span>Changed files</span><small>{review?.files.length ?? 0}</small></header>
        {review ? <ReviewFileTree files={review.files} selected={selected} onSelect={setSelected} /> : null}
        {review && review.files.length === 0 ? <p>No changes in this scope.</p> : null}
      </nav>
      <main className="review-diff" aria-label="Unified diff" tabIndex={0} ref={diffRef} onPointerDown={(event) => { if (event.target === event.currentTarget) event.currentTarget.focus(); }} onKeyDown={onDiffKeyDown}>
        {renderFile && review && diff ? renderFile(review, diff, diffContent) : diffContent()}
      </main>
    </div>
    <footer className="review-status" aria-label="Review shortcuts"><span>{displayedSelection ? `${displayedSelection.side} · lines ${Math.min(displayedSelection.start, displayedSelection.end)}–${Math.max(displayedSelection.start, displayedSelection.end)}` : "Select a line"}</span><span className="review-status-actions"><button type="button" onClick={onCreateLineComment} disabled={!canCreateLineComment} title={canCreateLineComment ? "Comment on selected lines (C)" : "Select review lines before commenting"}><kbd>C</kbd> comment</button><button type="button" onClick={onCreateFileComment} disabled={!canCreateFileComment} title={canCreateFileComment ? "Comment on whole file (Shift+C)" : "Review source is not ready"}><kbd>Shift+C</kbd> file</button><button type="button" onClick={onOpenCommentOverview} title="Open comments overview">{commentCount} comments</button><span><kbd>Alt+↑↓</kbd> hunk</span></span></footer>
  </section>;
}
