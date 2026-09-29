// @vitest-environment jsdom
import "../input/viewerTestLayout";
import { act, useState } from "react";
import { createRoot } from "react-dom/client";
import { expect, it, vi } from "vitest";
import type { CockpitClient } from "../../client/CockpitClient";
import type { CommentBatch, ViewerContext, ReviewFileDiff, ReviewSnapshot } from "../../protocol/generated/v1";
import { createReviewViewState, retainReviewScrollPosition, type ContextViewState } from "../context/ContextViewer";
import { ReviewViewer, reviewRepositoryId } from "./ReviewViewer";
import { reviewScrollIdentity } from "./ReviewPane";

const context: ViewerContext = { session_id: "session", viewer_id: "viewer", binding_id: "binding", tab_id: "tab", space_id: "space", kind: "review", source_kind: "review", source_id: "source", default_root_id: "repo-root", roots: [{ root_id: "repo-root", kind: "repository", label: "Repository", path: "/repo", repository_id: "repo", checkout_path: "/repo", companion_id: null }], diagnostics: [] };

it("uses the default repository root when nested repositories share a context", () => {
  const roots = [
    { root_id: "outer", kind: "repository", repository_id: "outer-repo" },
    { root_id: "nested", kind: "repository", repository_id: "nested-repo" },
  ] as ViewerContext["roots"];
  expect(reviewRepositoryId({ roots, default_root_id: "nested" })).toBe("nested-repo");
});

it("keeps a saved review editor cleared when its comment status updates", async () => {
  const file = { file_id: "file", comparison: "all_local", status: "modified", old_path: "src/file.ts", new_path: "src/file.ts", binary: false, additions: 0, deletions: 1, summary: "one line", old_revision: "old", new_revision: "new" } as const;
  const snapshot: ReviewSnapshot = { binding_id: "binding", session_id: "session", viewer_id: "viewer", review_id: "review", generation: 1, repository_id: "repo", checkout_path: "/repo", source_id: "source", comparison: "all_local", base_revision: null, head_revision: "head", index_revision: "index", worktree_revision: "worktree", files: [file], truncated: false, diagnostics: [] };
  const diff: ReviewFileDiff = { binding_id: "binding", session_id: "session", viewer_id: "viewer", review_id: "review", generation: 1, file, old_source: "one\ntwo\n", new_source: "one\ntwo\n", old_source_hash: "old", new_source_hash: "new", old_source_offset: 0, new_source_offset: 0, old_source_total_bytes: 8, new_source_total_bytes: 8, old_total_lines: 2, new_total_lines: 2, old_source_truncated: false, new_source_truncated: false, truncated: false, diagnostics: [], hunks: [{ old_path: "src/file.ts", new_path: "src/file.ts", old_start: 1, new_start: 1, lines: [{ kind: "deleted", old_line: 2, new_line: null, text: "two" }] }] };
  const batch: CommentBatch = { batch_id: "batch", generation: 1, owner: { kind: "viewer", session_id: "session", server_instance: "0123456789abcdef", tab_id: "tab", source_kind: "review", source_id: "source" }, last_known_location: { workspace_id: "space", tab_id: "tab" }, live_attachment: null, drafts: [], updated_at: "now" };
  const upsert = vi.fn(async () => ({ ...batch, generation: 2, drafts: [{ draft_id: "draft", file_ref: { root_id: "source", path: "src/file.ts", revision: "old", review: { review_id: "review", generation: 1, file_id: "file", side: "old" } }, anchor: { kind: "lines" as const, start_line: 2, end_line: 2 }, comment_text: "Save me", source_state: "current" as const }] }));
  const client = { reviewSnapshot: vi.fn(async () => snapshot), reviewFile: vi.fn(async () => diff), commentBatch: vi.fn(async () => batch), commentUpsert: upsert } as unknown as CockpitClient;
  const value: ContextViewState = { rootId: null, path: null, files: {}, commentEditor: { rootId: "source", path: "src/file.ts", revision: "old", review: { review_id: "review", generation: 1, file_id: "file", side: "old" }, draftId: null, editor: "lines", text: "Save me", selection: { start: 2, end: 2 } }, review: { ...createReviewViewState(), fileId: "file", side: "old", selectionStart: 2, selectionEnd: 2 } };
  const host = document.createElement("div");
  document.body.append(host);
  const mounted = createRoot(host);
  function Harness() {
    const [current, setCurrent] = useState(value);
    return <ReviewViewer client={client} context={context} value={current} onChange={setCurrent} />;
  }
  try {
    await act(async () => { mounted.render(<Harness />); await Promise.resolve(); await Promise.resolve(); });
    const save = [...host.querySelectorAll("button")].find((button) => button.textContent === "Save comment")!;
    await act(async () => { save.click(); await Promise.resolve(); await Promise.resolve(); });
    expect(upsert).toHaveBeenCalledOnce();
    expect(host.querySelector("textarea")).toBeNull();
  } finally {
    await act(async () => mounted.unmount());
    host.remove();
  }
});
it("bounds full review scroll identities and separates generation and revision", () => {
  const first = reviewScrollIdentity("session", "viewer", "binding", "review", 1, "all_local", "source", "file", "new", "r1");
  const second = reviewScrollIdentity("session", "viewer", "binding", "review", 2, "all_local", "source", "file", "new", "r2");
  expect(first).not.toBe(second);
  let positions: Record<string, number> = {};
  for (let index = 0; index < 64; index += 1) positions = retainReviewScrollPosition(positions, `identity-${index}`, index);
  positions = retainReviewScrollPosition(positions, first, 240);
  positions = retainReviewScrollPosition(positions, second, 480);
  expect(Object.keys(positions)).toHaveLength(64);
  expect(positions[first]).toBe(240);
  expect(positions[second]).toBe(480);
  expect(positions["identity-0"]).toBeUndefined();
});

it("isolates deferred source pages and scroll across file and generation changes", async () => {
  const firstFile = { file_id: "first", comparison: "all_local", status: "modified", old_path: "first.ts", new_path: "first.ts", binary: false, additions: 1, deletions: 0, summary: "first", old_revision: "old-first", new_revision: "new-first" } as const;
  const secondFile = { ...firstFile, file_id: "second", old_path: "second.ts", new_path: "second.ts", old_revision: "old-second", new_revision: "new-second" } as const;
  const snapshot: ReviewSnapshot = { binding_id: "binding", session_id: "session", viewer_id: "viewer", review_id: "review", generation: 1, repository_id: "repo", checkout_path: "/repo", source_id: "source", comparison: "all_local", base_revision: null, head_revision: "head", index_revision: "index", worktree_revision: "worktree", files: [firstFile, secondFile], truncated: false, diagnostics: [] };
  let generation = 1;
  const diffFor = (file: ReviewFileDiff["file"], text: string): ReviewFileDiff => ({ binding_id: "binding", session_id: "session", viewer_id: "viewer", review_id: "review", generation, file, old_source: text, new_source: text, old_source_hash: "old-hash", new_source_hash: "new-hash", old_source_offset: 0, new_source_offset: 0, old_source_total_bytes: 80, new_source_total_bytes: 80, old_total_lines: 2, new_total_lines: 2, old_source_truncated: true, new_source_truncated: true, truncated: false, diagnostics: [], hunks: [] });
  let resolvePage!: (value: ReviewFileDiff) => void;
  const deferredPage = new Promise<ReviewFileDiff>((resolve) => { resolvePage = resolve; });
  const client = {
    reviewSnapshot: vi.fn(async () => ({ ...snapshot, generation })),
    reviewFile: vi.fn(async (_session: string, _viewer: string, request: { file_id: string; source_offset?: number; source_side?: string | null }) => {
      if (request.file_id === "first" && request.source_side === "new") return deferredPage;
      return diffFor(request.file_id === "second" ? secondFile : firstFile, request.file_id === "second" ? "second-current" : "first-current");
    }),
    commentBatch: vi.fn(async () => ({ batch_id: "batch", generation: 1, owner: { kind: "viewer", session_id: "session", server_instance: "0123456789abcdef", tab_id: "tab", source_kind: "review", source_id: "source" }, last_known_location: { workspace_id: "space", tab_id: "tab" }, live_attachment: null, drafts: [], updated_at: "now" } satisfies CommentBatch)),
  } as unknown as CockpitClient;
  const value: ContextViewState = { rootId: null, path: null, files: {}, commentEditor: null, review: { ...createReviewViewState(), mode: "source", fileId: "first", side: "new" } };
  const host = document.createElement("div");
  document.body.append(host);
  const mounted = createRoot(host);
  function Harness() {
    const [current, setCurrent] = useState(value);
    return <ReviewViewer client={client} context={context} value={current} onChange={setCurrent} />;
  }
  try {
    await act(async () => { mounted.render(<Harness />); await Promise.resolve(); await Promise.resolve(); });
    const pageButton = [...host.querySelectorAll<HTMLButtonElement>("button")].find((button) => button.textContent?.includes("Load next source page"));
    expect(pageButton).toBeDefined();
    await act(async () => pageButton?.click());
    await act(async () => host.querySelector<HTMLButtonElement>('[data-file-id="second"]')?.click());
    await act(async () => { await Promise.resolve(); await Promise.resolve(); });
    resolvePage(diffFor(firstFile, "first-current\nlate-stale"));
    await act(async () => { await Promise.resolve(); await Promise.resolve(); });
    expect(host.textContent).toContain("second-current");
    expect(host.textContent).not.toContain("late-stale");
    await act(async () => host.querySelector<HTMLButtonElement>('[data-file-id="first"]')?.click());
    await act(async () => { await Promise.resolve(); await Promise.resolve(); });
    expect(host.textContent).toContain("first-current");
    expect(host.textContent).not.toContain("late-stale");
    const source = host.querySelector<HTMLElement>(".context-source-scroll")!;
    source.scrollTop = 320;
    await act(async () => source.dispatchEvent(new Event("scroll", { bubbles: true })));
    generation += 1;
    await act(async () => host.querySelector<HTMLButtonElement>('button[aria-label="Refresh"]')!.click());
    await act(async () => { await Promise.resolve(); await Promise.resolve(); });
    expect(host.textContent).toContain("first-current");
    expect(host.querySelector<HTMLElement>(".context-source-scroll")!.scrollTop).toBe(0);
  } finally {
    await act(async () => mounted.unmount());
    host.remove();
  }
});

it("restores unsaved comment text, line selection and file overview choice when revisiting a source", async () => {
  const changedFile = { file_id: "file", comparison: "all_local", status: "modified", old_path: "src/file.ts", new_path: "src/file.ts", binary: false, additions: 0, deletions: 1, summary: "one line", old_revision: "old", new_revision: "new" } as const;
  let activeContext = context;
  const client = {
    reviewSnapshot: async () => ({ binding_id: activeContext.binding_id, session_id: "session", viewer_id: "viewer", review_id: activeContext.source_id, generation: 1, repository_id: "repo", checkout_path: "/repo", source_id: activeContext.source_id, comparison: "all_local", base_revision: null, head_revision: "head", index_revision: "index", worktree_revision: "worktree", files: [changedFile], truncated: false, diagnostics: [] } satisfies ReviewSnapshot),
    reviewFile: async () => ({ binding_id: activeContext.binding_id, session_id: "session", viewer_id: "viewer", review_id: activeContext.source_id, generation: 1, file: changedFile, old_source: "one\ntwo\n", new_source: "one\n", old_source_hash: "old", new_source_hash: "new", old_source_offset: 0, new_source_offset: 0, old_source_total_bytes: 8, new_source_total_bytes: 4, old_total_lines: 2, new_total_lines: 1, old_source_truncated: false, new_source_truncated: false, truncated: false, diagnostics: [], hunks: [{ old_path: "src/file.ts", new_path: "src/file.ts", old_start: 1, new_start: 1, lines: [{ kind: "deleted", old_line: 2, new_line: null, text: "two" }] }] } satisfies ReviewFileDiff),
    commentBatch: async () => ({ batch_id: activeContext.source_id, generation: 1, owner: { kind: "viewer", session_id: "session", server_instance: "0123456789abcdef", tab_id: "tab", source_kind: "review", source_id: activeContext.source_id }, last_known_location: { workspace_id: "space", tab_id: "tab" }, live_attachment: null, drafts: [], updated_at: "now" } satisfies CommentBatch),
  } as unknown as CockpitClient;
  const views: Record<string, ContextViewState> = {
    source: { rootId: null, path: null, files: {}, commentEditor: { rootId: "source", path: "src/file.ts", revision: "old", review: { review_id: "source", generation: 1, file_id: "file", side: "old" }, draftId: null, editor: "lines", text: "", selection: { start: 2, end: 2 } }, review: { ...createReviewViewState(), fileId: "file", side: "old", selectionStart: 2, selectionEnd: 2 } },
    other: { rootId: null, path: null, files: {}, commentEditor: null, review: createReviewViewState() },
  };
  const host = document.createElement("div");
  document.body.append(host);
  const mounted = createRoot(host);
  function Harness({ viewerContext }: { viewerContext: ViewerContext }) {
    const [current, setCurrent] = useState(views[viewerContext.source_id]);
    return <ReviewViewer client={client} context={viewerContext} value={current} onChange={(next) => { views[viewerContext.source_id] = next; setCurrent(next); }} />;
  }
  const show = async (sourceId: string, bindingId: string) => {
    activeContext = { ...context, source_id: sourceId, binding_id: bindingId };
    await act(async () => mounted.render(<Harness key={`${activeContext.viewer_id}:${bindingId}`} viewerContext={activeContext} />));
  };
  try {
    await show("source", "first-binding");
    const editor = host.querySelector<HTMLTextAreaElement>("textarea")!;
    await act(async () => {
      Object.getOwnPropertyDescriptor(HTMLTextAreaElement.prototype, "value")!.set!.call(editor, "Still unsaved");
      editor.dispatchEvent(new Event("input", { bubbles: true }));
    });
    const overviewToggle = host.querySelector<HTMLButtonElement>('button[aria-label="Toggle file overview"]')!;
    const savedOverviewOpen = overviewToggle.getAttribute("aria-expanded") !== "true";
    await act(async () => overviewToggle.click());
    await show("other", "second-binding");
    expect(host.querySelector("textarea")).toBeNull();
    await show("source", "revisited-binding");
    expect(host.querySelector<HTMLTextAreaElement>("textarea")?.value).toBe("Still unsaved");
    expect(host.querySelector(".review-line.is-selected")?.getAttribute("data-old-line")).toBe("2");
    expect(host.querySelector('button[aria-label="Toggle file overview"]')?.getAttribute("aria-expanded")).toBe(String(savedOverviewOpen));
  } finally {
    await act(async () => mounted.unmount());
    host.remove();
  }
});
