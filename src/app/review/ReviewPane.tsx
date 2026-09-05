import { Fragment, type ReactNode, useCallback, useEffect, useId, useMemo, useRef, useState } from "react";
import type {
  ReviewChangedFile,
  ReviewComparison,
  ReviewFileDiff,
  ReviewSnapshot,
  ReviewSnapshotRequest,
} from "../../protocol/generated/v1";
import "./review.css";

export type ReviewPaneProps = {
  identity: string;
  sessionId: string;
  paneId: string;
  bindingId: string;
  repositoryId: string;
  snapshot: (request: ReviewSnapshotRequest, signal: AbortSignal) => Promise<ReviewSnapshot>;
  file: (request: { binding_id: string; review_id: string; generation: number; file_id: string }, signal: AbortSignal) => Promise<ReviewFileDiff>;
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

export function ReviewPane({ identity, sessionId, paneId, bindingId, repositoryId, snapshot, file, onSelectLines, renderFile, renderLineComments }: ReviewPaneProps) {
  const baseId = useId();
  const [comparison, setComparison] = useState<ReviewComparison>("all_local");
  const [baseRef, setBaseRef] = useState("");
  const [review, setReview] = useState<ReviewSnapshot | null>(null);
  const [selected, setSelected] = useState<string | null>(null);
  const [diff, setDiff] = useState<ReviewFileDiff | null>(null);
  const [pending, setPending] = useState(false);
  const [hunkIndex, setHunkIndex] = useState(-1);
  const surfaceRef = useRef<HTMLElement | null>(null);
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

  const moveFile = (delta: number) => {
    if (!review?.files.length) return;
    const index = review.files.findIndex(item => item.file_id === selected);
    setSelected(review.files[Math.max(0, Math.min(review.files.length - 1, index + delta))].file_id);
    setHunkIndex(-1);
  };
  const moveHunk = (delta: number) => {
    const hunks = surfaceRef.current?.querySelectorAll<HTMLElement>(".review-hunk");
    if (!hunks?.length) return;
    const index = Math.max(0, Math.min(hunks.length - 1, hunkIndex + delta));
    setHunkIndex(index); hunks[index].scrollIntoView({ block: "nearest" }); hunks[index].focus({ preventScroll: true });
  };
  const diffContent = (comments = renderLineComments) => <>
        {selectedFile ? <header><code>{fileLabel(selectedFile)}</code><span>{selectedFile.summary}</span></header> : null}
        {selectedFile?.binary ? <p className="review-empty">Binary content has no text anchors.</p> : null}
        {diff?.hunks.map((hunk, hunkIndex) => <section className="review-hunk" tabIndex={-1} key={`${hunkIndex}-${hunk.old_start}-${hunk.new_start}`}>
          <header>@@ -{hunk.old_start} +{hunk.new_start} @@</header>
          {hunk.lines.map((line, index) => <Fragment key={`${index}-${line.old_line ?? ""}-${line.new_line ?? ""}`}><button type="button" key={`${index}-${line.old_line ?? ""}-${line.new_line ?? ""}`} className={`review-line is-${line.kind}`} onClick={(event) => {
            const oldGutter = event.target instanceof HTMLElement && event.target.closest("[data-side=old]");
            const side = line.kind === "deleted" || (oldGutter && line.old_line !== null) ? "old" : "new";
            const lineNumber = side === "old" ? line.old_line! : line.new_line!;
            onSelectLines?.(diff.file, side, lineNumber, lineNumber, [line.text], event.shiftKey);
          }}>
            <span data-side="old">{line.old_line ?? ""}</span><span data-side="new">{line.new_line ?? ""}</span><code>{line.kind === "added" ? "+" : line.kind === "deleted" ? "-" : " "}{line.text}</code>
          </button>{comments?.(diff, line.old_line, line.new_line)}</Fragment>)}
        </section>)}
        {diff?.diagnostics.map((item, index) => <p key={`${item.code}-${index}`} className="review-notice">{item.message}</p>)}
        {diff && diff.hunks.length === 0 && !selectedFile?.binary ? <p className="review-empty">No textual hunk is available for this change.</p> : null}
        {!review && !pending ? <p className="review-empty">Choose a verified Review pane to load a local Git snapshot.</p> : null}
  </>;

  return <section className="review-pane" aria-label="Local review" ref={surfaceRef} onKeyDown={event => {
    if (!event.altKey || event.ctrlKey || event.metaKey || event.target instanceof HTMLInputElement || event.target instanceof HTMLTextAreaElement || event.target instanceof HTMLSelectElement) return;
    if (event.key === "ArrowDown" || event.key === "ArrowUp") { event.preventDefault(); moveHunk(event.key === "ArrowDown" ? 1 : -1); }
    if (event.key === "ArrowRight" || event.key === "ArrowLeft") { event.preventDefault(); moveFile(event.key === "ArrowRight" ? 1 : -1); }
  }}>
    <header className="review-toolbar">
      <strong>Review</strong>
      <label htmlFor={`${baseId}-mode`} className="sr-only">Comparison</label>
      <select id={`${baseId}-mode`} value={comparison} disabled={pending} onChange={(event) => setComparison(event.target.value as ReviewComparison)}>
        {modes.map((mode) => <option key={mode.value} value={mode.value}>{mode.label}</option>)}
      </select>
      {comparison === "branch" ? <label className="review-base" htmlFor={`${baseId}-base`}>Base ref <input id={`${baseId}-base`} value={baseRef} onChange={(event) => setBaseRef(event.target.value)} placeholder="main" disabled={pending} /></label> : null}
      <button type="button" aria-label="Previous file" title="Previous file (Alt+Left)" onClick={() => moveFile(-1)} disabled={!review?.files.length}>←</button>
      <button type="button" aria-label="Next file" title="Next file (Alt+Right)" onClick={() => moveFile(1)} disabled={!review?.files.length}>→</button>
      <button type="button" aria-label="Previous hunk" title="Previous hunk (Alt+Up)" onClick={() => moveHunk(-1)} disabled={!diff?.hunks.length}>↑</button>
      <button type="button" aria-label="Next hunk" title="Next hunk (Alt+Down)" onClick={() => moveHunk(1)} disabled={!diff?.hunks.length}>↓</button>
      <span className="review-toolbar-spacer" />
      <button type="button" onClick={() => void refresh()} disabled={pending || (comparison === "branch" && !baseRef.trim())}>{pending ? "Refreshing…" : "Refresh"}</button>
    </header>
    {error ? <p className="review-notice review-error" role="alert">{error}</p> : null}
    {diff?.truncated ? <p className="review-notice review-warning">This file reached the preview limit. Comments require an available complete source.</p> : null}
    {review?.truncated ? <p className="review-notice review-warning">Review output reached a configured limit. Refresh with a narrower scope.</p> : null}
    <div className="review-body">
      <nav className="review-files" aria-label="Changed files">
        <header><span>Changed files</span><small>{review?.files.length ?? 0}</small></header>
        {review?.files.map((item) => <button type="button" key={item.file_id} className={item.file_id === selected ? "is-selected" : ""} onClick={() => setSelected(item.file_id)}>
          <b>{item.status.replaceAll("_", " ")}</b><span>{fileLabel(item)}</span><small>{item.comparison.replaceAll("_", " ")}</small>
        </button>)}
        {review && review.files.length === 0 ? <p>No changes in this scope.</p> : null}
      </nav>
      <main className="review-diff" aria-label="Unified diff">
        {renderFile && review && diff ? renderFile(review, diff, diffContent) : diffContent()}
      </main>
    </div>
  </section>;
}
