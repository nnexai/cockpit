// @vitest-environment jsdom
import { act } from "react";
import { createRoot } from "react-dom/client";
import { expect, it, vi } from "vitest";
import type { ReviewFileDiff, ReviewFileRequest, ReviewSnapshot, ReviewSnapshotRequest } from "../../protocol/generated/v1";
import { SourceLines } from "../context/ContextViewer";
import { ReviewPane } from "./ReviewPane";

const changedFile = { file_id: "file", comparison: "all_local", status: "modified", old_path: "src/file.ts", new_path: "src/file.ts", binary: false, additions: 2, deletions: 1, summary: "2 hunks", old_revision: "old", new_revision: "new" } as const;
const snapshot: ReviewSnapshot = {
  binding_id: "binding", session_id: "session", pane_id: "pane", review_id: "review", generation: 1,
  repository_id: "repo", checkout_path: "/repo", source_id: "source", comparison: "all_local", base_revision: null,
  head_revision: "head", index_revision: "index", worktree_revision: "worktree", files: [changedFile], truncated: false, diagnostics: [],
};
const diff: ReviewFileDiff = {
  binding_id: "binding", session_id: "session", pane_id: "pane", review_id: "review", generation: 1,
  file: changedFile, old_source: "one\ntwo\n", new_source: "one\ntwo\n", old_source_hash: "old", new_source_hash: "new",
  old_total_lines: 2, new_total_lines: 2, old_source_truncated: false, new_source_truncated: false, truncated: false, diagnostics: [],
  hunks: [
    { old_path: "src/file.ts", new_path: "src/file.ts", old_start: 1, new_start: 1, lines: [{ kind: "context", old_line: 1, new_line: 1, text: "one" }] },
    { old_path: "src/file.ts", new_path: "src/file.ts", old_start: 2, new_start: 2, lines: [{ kind: "context", old_line: 2, new_line: 2, text: "two" }] },
  ],
};

it("focuses the review diff so local hunk navigation works after entering the surface", async () => {
  const host = window.document.createElement("div");
  window.document.body.append(host);
  const mounted = createRoot(host);
  const scrollIntoView = vi.fn();
  const onSelectLines = vi.fn();
  Object.defineProperty(HTMLElement.prototype, "scrollIntoView", { configurable: true, value: scrollIntoView });
  try {
    await act(async () => {
      mounted.render(<ReviewPane identity="review" sessionId="session" paneId="pane" bindingId="binding" repositoryId="repo"
        snapshot={async () => snapshot} file={async () => diff} selectedLines={{ fileId: "file", side: "new", start: 1, end: 1 }} onSelectLines={onSelectLines} />);
    });
    const surface = host.querySelector<HTMLElement>(".review-diff")!;
    surface.focus();
    expect(window.document.activeElement).toBe(surface);
    expect(host.querySelector(".review-line.is-selected")?.getAttribute("data-new-line")).toBe("1");
    await act(async () => surface.dispatchEvent(new KeyboardEvent("keydown", { bubbles: true, key: "ArrowDown", altKey: true })));
    expect(host.querySelector<HTMLElement>(".review-hunk:focus")).not.toBeNull();
    expect(scrollIntoView).toHaveBeenCalledWith({ block: "nearest" });
    await act(async () => host.querySelector<HTMLElement>(".review-hunk:focus")!.dispatchEvent(new KeyboardEvent("keydown", { bubbles: true, key: "ArrowDown" })));
    expect(onSelectLines).toHaveBeenCalledWith(changedFile, "new", 2, 2, ["two"], false);
  } finally {
    await act(async () => mounted.unmount());
    host.remove();
  }
});

it("groups file paths while retaining separate staged and unstaged entries", async () => {
  const host = window.document.createElement("div");
  window.document.body.append(host);
  const mounted = createRoot(host);
  const staged = { ...changedFile, file_id: "file-staged", comparison: "staged" as const };
  try {
    await act(async () => {
      mounted.render(<ReviewPane identity="review" sessionId="session" paneId="pane" bindingId="binding" repositoryId="repo"
        snapshot={async () => ({ ...snapshot, files: [changedFile, staged] })} file={async () => diff} />);
    });
    expect([...host.querySelectorAll(".review-file-directory summary")].map(item => item.textContent)).toContain("src/");
    expect(host.querySelectorAll(".review-file")).toHaveLength(2);
    expect([...host.querySelectorAll(".review-file small")].map(item => item.textContent)).toEqual(["working", "index"]);
  } finally {
    await act(async () => mounted.unmount());
    host.remove();
  }
});

it("opens the fzf-style picker from a focused review and selects with Ctrl+N then Enter", async () => {
  const host = document.createElement("div");
  document.body.append(host);
  const mounted = createRoot(host);
  const second = { ...changedFile, file_id: "second", old_path: "docs/target.md", new_path: "docs/target.md" };
  const loadFile = vi.fn(async () => diff);
  try {
    await act(async () => mounted.render(<ReviewPane identity="picker" sessionId="session" paneId="pane" bindingId="binding" repositoryId="repo" snapshot={async () => ({ ...snapshot, files: [changedFile, second] })} file={loadFile} />));
    const surface = host.querySelector<HTMLElement>(".review-diff")!;
    surface.focus();
    await act(async () => surface.dispatchEvent(new KeyboardEvent("keydown", { bubbles: true, cancelable: true, key: "p", ctrlKey: true })));
    const input = host.querySelector<HTMLInputElement>(".file-picker input")!;
    expect(input).not.toBeNull();
    await act(async () => input.dispatchEvent(new KeyboardEvent("keydown", { bubbles: true, cancelable: true, key: "n", ctrlKey: true })));
    await act(async () => input.dispatchEvent(new KeyboardEvent("keydown", { bubbles: true, cancelable: true, key: "Enter" })));
    await act(async () => { await Promise.resolve(); });
    expect(loadFile).toHaveBeenLastCalledWith(expect.objectContaining({ file_id: "second" }), expect.any(AbortSignal));
  } finally {
    await act(async () => mounted.unmount());
    host.remove();
  }
});

it("continues hunk navigation and comments from the hunk selected by the mouse", async () => {
  const host = window.document.createElement("div");
  window.document.body.append(host);
  const mounted = createRoot(host);
  const thirdHunk = { old_path: "src/file.ts", new_path: "src/file.ts", old_start: 3, new_start: 3, lines: [{ kind: "context" as const, old_line: 3, new_line: 3, text: "three" }] };
  const diffWithThreeHunks = { ...diff, hunks: [...diff.hunks, thirdHunk] };
  const onSelectLines = vi.fn();
  const onCreateLineComment = vi.fn();
  try {
    await act(async () => {
      mounted.render(<ReviewPane identity="review" sessionId="session" paneId="pane" bindingId="binding" repositoryId="repo"
        snapshot={async () => snapshot} file={async () => diffWithThreeHunks} selectedLines={{ fileId: "file", side: "new", start: 1, end: 1 }} onSelectLines={onSelectLines} onCreateLineComment={onCreateLineComment} />);
    });
    const surface = host.querySelector<HTMLElement>(".review-diff")!;
    const lines = host.querySelectorAll<HTMLButtonElement>(".review-line");
    await act(async () => lines[2].click());
    await act(async () => surface.dispatchEvent(new KeyboardEvent("keydown", { bubbles: true, key: "ArrowUp", altKey: true })));
    expect(onSelectLines).toHaveBeenLastCalledWith(changedFile, "new", 2, 2, ["two"], false);
    await act(async () => surface.dispatchEvent(new KeyboardEvent("keydown", { bubbles: true, key: "c" })));
    expect(onCreateLineComment).toHaveBeenCalledOnce();
  } finally {
    await act(async () => mounted.unmount());
    host.remove();
  }
});


it("uses rendered full-source lines for local arrow navigation", async () => {
  const host = window.document.createElement("div");
  window.document.body.append(host);
  const mounted = createRoot(host);
  const onSelect = vi.fn();
  try {
    await act(async () => {
      mounted.render(<ReviewPane identity="review" sessionId="session" paneId="pane" bindingId="binding" repositoryId="repo"
        snapshot={async () => snapshot} file={async () => diff} selectedLines={{ fileId: "file", side: "new", start: 1, end: 1 }}
        renderFile={() => <SourceLines text={`one\ntwo\nthree\n`} state={{ rootId: "root", path: "file.ts", mode: "source", selectionStart: 1, selectionEnd: 1, scrollTop: 0 }} onSelect={onSelect} onScroll={() => undefined} />} />);
    });
    const firstLine = host.querySelector<HTMLButtonElement>('[data-line="1"]')!;
    await act(async () => { firstLine.focus(); firstLine.dispatchEvent(new KeyboardEvent("keydown", { bubbles: true, key: "ArrowDown" })); });
    expect(onSelect).toHaveBeenCalledWith(2, 2, false);
    await act(async () => firstLine.dispatchEvent(new KeyboardEvent("keydown", { bubbles: true, key: "End" })));
    expect(onSelect).toHaveBeenLastCalledWith(3, 3, false);
  } finally {
    await act(async () => mounted.unmount());
    host.remove();
  }
});

it("compresses directory chains and single leaves while indenting siblings", async () => {
  const host = window.document.createElement("div");
  window.document.body.append(host);
  const mounted = createRoot(host);
  const files = ["crates/core/src/comments/mod.rs", "crates/core/src/comments/paste.rs", "docs/reference/guide.md"].map((path, i) => ({ ...changedFile, file_id: `nested-${i}`, old_path: path, new_path: path }));
  try {
    await act(async () => mounted.render(<ReviewPane identity="nested" sessionId="session" paneId="pane" bindingId="binding" repositoryId="repo" snapshot={async () => ({ ...snapshot, files })} file={async () => diff} />));
    expect([...host.querySelectorAll(".review-file-directory summary")].map(item => item.textContent)).toEqual(["crates/core/src/comments/"]);
    expect(host.querySelectorAll(".review-file")).toHaveLength(3);
    expect(host.querySelector(".review-file")?.getAttribute("style")).toContain("20px");
    expect(host.querySelectorAll(".review-file small")).toHaveLength(0);
    expect(host.textContent).toContain("docs/reference/guide.md");
  } finally { await act(async () => mounted.unmount()); host.remove(); }
});

it("opens a collapsed directory before keyboard focus reaches its file", async () => {
  const host = window.document.createElement("div");
  window.document.body.append(host);
  const mounted = createRoot(host);
  const files = ["docs/reference/guide.md", "crates/core/src/comments/mod.rs", "crates/core/src/comments/paste.rs"].map((path, i) => ({ ...changedFile, file_id: `collapsed-${i}`, old_path: path, new_path: path }));
  try {
    await act(async () => mounted.render(<ReviewPane identity="collapsed" sessionId="session" paneId="pane" bindingId="binding" repositoryId="repo" snapshot={async () => ({ ...snapshot, files })} file={async () => diff} />));
    const directory = host.querySelector<HTMLDetailsElement>(".review-file-directory")!;
    directory.open = false;
    const outsideFile = [...host.querySelectorAll<HTMLButtonElement>(".review-file")].find(item => item.textContent?.includes("docs/reference/guide.md"))!;
    await act(async () => outsideFile.click());
    await act(async () => outsideFile.dispatchEvent(new KeyboardEvent("keydown", { bubbles: true, key: "ArrowUp" })));
    const hiddenFile = [...host.querySelectorAll<HTMLButtonElement>(".review-file")].find(item => item.textContent?.includes("paste.rs"))!;
    expect(directory.open).toBe(true);
    expect(window.document.activeElement).toBe(hiddenFile);
    expect(hiddenFile.dataset.fileId).toBe("collapsed-2");

    await act(async () => outsideFile.click());
    directory.open = false;
    const surface = host.querySelector<HTMLElement>(".review-diff")!;
    surface.focus();
    await act(async () => surface.dispatchEvent(new KeyboardEvent("keydown", { bubbles: true, key: "ArrowLeft", altKey: true })));
    expect(directory.open).toBe(true);
    expect(window.document.activeElement).toBe(surface);
    expect(host.querySelector(".review-file.is-selected")?.getAttribute("data-file-id")).toBe("collapsed-2");
  } finally { await act(async () => mounted.unmount()); host.remove(); }
});

it("starts previous hunk navigation at the last hunk and does not skip a missing selection", async () => {
  const host = window.document.createElement("div");
  window.document.body.append(host);
  const mounted = createRoot(host);
  const onSelectLines = vi.fn();
  try {
    await act(async () => mounted.render(<ReviewPane identity="review" sessionId="session" paneId="pane" bindingId="binding" repositoryId="repo"
      snapshot={async () => snapshot} file={async () => diff} selectedLines={{ fileId: "file", side: "new", start: 99, end: 99 }} onSelectLines={onSelectLines} />));
    const surface = host.querySelector<HTMLElement>(".review-diff")!;
    surface.focus();
    await act(async () => surface.dispatchEvent(new KeyboardEvent("keydown", { bubbles: true, key: "ArrowUp", altKey: true })));
    expect(host.querySelector<HTMLElement>(".review-hunk:focus")?.textContent).toContain("@@ -2 +2 @@");
    expect(onSelectLines).toHaveBeenLastCalledWith(changedFile, "new", 2, 2, ["two"], false);

    surface.focus();
    await act(async () => surface.dispatchEvent(new KeyboardEvent("keydown", { bubbles: true, key: "ArrowDown" })));
    expect(onSelectLines).toHaveBeenLastCalledWith(changedFile, "new", 1, 1, ["one"], false);
    expect(window.document.activeElement).toBe(host.querySelector('[data-new-line="1"]'));
  } finally { await act(async () => mounted.unmount()); host.remove(); }
});

it("waits for an explicit branch base submission without requesting blank or partial refs", async () => {
  const host = document.createElement("div");
  document.body.append(host);
  const mounted = createRoot(host);
  const load = vi.fn(async (_request: unknown, _signal?: AbortSignal) => snapshot);
  try {
    await act(async () => mounted.render(<ReviewPane identity="review" sessionId="session" paneId="pane" bindingId="binding" repositoryId="repo" snapshot={load} file={async () => diff} />));
    load.mockClear();
    const mode = host.querySelector<HTMLSelectElement>("select")!;
    await act(async () => { mode.value = "branch"; mode.dispatchEvent(new Event("change", { bubbles: true })); });
    expect(load).not.toHaveBeenCalled();
    expect(host.querySelector(".review-file")).toBeNull();
    const input = host.querySelector<HTMLInputElement>(".review-base input")!;
    input.focus();
    await act(async () => {
      Object.getOwnPropertyDescriptor(HTMLInputElement.prototype, "value")!.set!.call(input, "main");
      input.dispatchEvent(new Event("input", { bubbles: true }));
    });
    expect(load).not.toHaveBeenCalled();
    expect(document.activeElement).toBe(input);
    await act(async () => input.dispatchEvent(new KeyboardEvent("keydown", { key: "Enter", bubbles: true })));
    expect(load).toHaveBeenCalledOnce();
    expect(load.mock.calls[0][0]).toMatchObject({ comparison: "branch", base_ref: "main" });
    const typeBase = async (value: string) => act(async () => {
      Object.getOwnPropertyDescriptor(HTMLInputElement.prototype, "value")!.set!.call(input, value);
      input.dispatchEvent(new Event("input", { bubbles: true }));
    });
    let finish!: (value: ReviewSnapshot) => void;
    load.mockImplementationOnce(() => new Promise<ReviewSnapshot>((resolve) => { finish = resolve; }));
    await typeBase("release");
    expect(host.querySelector(".review-file")).toBeNull();
    await act(async () => input.dispatchEvent(new KeyboardEvent("keydown", { key: "Enter", bubbles: true })));
    await typeBase("next");
    await act(async () => finish(snapshot));
    expect(host.querySelector(".review-file")).toBeNull();
    await typeBase("");
    await act(async () => { mode.value = "all_local"; mode.dispatchEvent(new Event("change", { bubbles: true })); });
    load.mockClear();
    await act(async () => { mode.value = "branch"; mode.dispatchEvent(new Event("change", { bubbles: true })); });
    expect(load).not.toHaveBeenCalled();
    expect(host.querySelector(".review-file")).toBeNull();

  } finally {
    await act(async () => mounted.unmount());
    host.remove();
  }
});

it("keeps the selected file and arrow navigation across parent updates", async () => {
  const host = document.createElement("div");
  document.body.append(host);
  const mounted = createRoot(host);
  const second = { ...changedFile, file_id: "second", old_path: "docs/second.md", new_path: "docs/second.md" };
  const third = { ...changedFile, file_id: "third", old_path: "docs/third.md", new_path: "docs/third.md" };
  const review = { ...snapshot, files: [changedFile, second, third] };
  const loadSnapshot = vi.fn(async (_request: ReviewSnapshotRequest, _signal: AbortSignal) => structuredClone(review));
  const loadFile = vi.fn(async (request: ReviewFileRequest, _signal: AbortSignal) => ({ ...diff, file: review.files.find((candidate) => candidate.file_id === request.file_id) ?? changedFile }));
  const snapshotCallback = (request: ReviewSnapshotRequest, signal: AbortSignal) => loadSnapshot(request, signal);
  const fileCallback = (request: ReviewFileRequest, signal: AbortSignal) => loadFile(request, signal);
  const render = async () => {
    await act(async () => mounted.render(<ReviewPane identity="stable" sessionId="session" paneId="pane" bindingId="binding" repositoryId="repo"
      snapshot={snapshotCallback} file={fileCallback} />));
  };
  try {
    await render();
    await act(async () => { await Promise.resolve(); });
    const firstVisible = host.querySelector<HTMLButtonElement>(".review-file")!;
    expect(firstVisible.dataset.fileId).toBe("second");
    firstVisible.focus();
    await act(async () => firstVisible.dispatchEvent(new KeyboardEvent("keydown", { bubbles: true, key: "ArrowDown" })));
    expect(host.querySelector(".review-file.is-selected")?.getAttribute("data-file-id")).toBe("third");
    expect(document.activeElement).toBe(host.querySelector('[data-file-id="third"]'));

    const secondButton = host.querySelector<HTMLButtonElement>('[data-file-id="second"]')!;
    await act(async () => secondButton.click());
    await act(async () => { await Promise.resolve(); });
    expect(host.querySelector(".review-file.is-selected")?.getAttribute("data-file-id")).toBe("second");
    expect(loadSnapshot).toHaveBeenCalledTimes(1);

    await render();
    await act(async () => { await Promise.resolve(); });
    expect(loadSnapshot).toHaveBeenCalledTimes(1);
    expect(host.querySelector(".review-file.is-selected")?.getAttribute("data-file-id")).toBe("second");

    const files = host.querySelector<HTMLElement>(".review-files")!;
    await act(async () => files.dispatchEvent(new KeyboardEvent("keydown", { bubbles: true, key: "ArrowDown" })));
    expect(host.querySelector(".review-file.is-selected")?.getAttribute("data-file-id")).toBe("third");
  } finally {
    await act(async () => mounted.unmount());
    host.remove();
  }
});
