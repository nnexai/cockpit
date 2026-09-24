// @vitest-environment jsdom
import "../input/viewerTestLayout";
import { act, useState } from "react";
import { createRoot } from "react-dom/client";
import { expect, it, vi } from "vitest";
import type { CockpitClient } from "../../client/CockpitClient";
import type { CommentBatch, CommentBatchList, ContextDocument, ContextRoot, PanePresentation } from "../../protocol/generated/v1";
import { CommentDrafts } from "./CommentDrafts";
import { SourceLines, type ContextCommentEditorState } from "./ContextViewer";

it("retains prose after a remote deletion and requires explicit source capture to recreate it", async () => {
  const root: ContextRoot = { root_id: "root", kind: "companion", label: "Context", path: "/context", repository_id: "repo", checkout_path: "/repo", companion_id: "source" };
  const presentation = { session_id: "session", pane_id: "pane", binding_id: "binding" } as PanePresentation;
  const document = { text: "new source\n", revision: "new-revision" } as ContextDocument;
  const batch: CommentBatch = { batch_id: "batch", generation: 2, owner: { session_id: "session", pane_id: "pane", terminal_id: "terminal", source_kind: "context", source_id: "source" }, last_known_location: { workspace_id: "space", tab_id: "tab" }, live_attachment: null, drafts: [], updated_at: "now" };
  const upsert = vi.fn(async () => ({ ...batch, generation: 3 }));
  const client = { commentBatch: vi.fn(async () => batch), commentUpsert: upsert } as unknown as CockpitClient;
  function Harness() {
    const [editor, setEditor] = useState<ContextCommentEditorState | null>({ rootId: "root", path: "file.md", revision: "old-revision", draftId: "deleted-draft", editor: "whole_file", text: "My unsaved review", selection: null });
    return <CommentDrafts client={client} presentation={presentation} root={root} path="file.md" document={document} selection={null} mode="source" editorState={editor} onEditorStateChange={setEditor} />;
  }
  const host = window.document.createElement("div");
  window.document.body.append(host);
  const mounted = createRoot(host);
  const button = (label: string) => [...host.querySelectorAll("button")].find((item) => item.textContent === label)!;
  try {
    await act(async () => { mounted.render(<Harness />); });
    expect(host.textContent).toContain("deleted in another window");
    expect(host.querySelector("textarea")?.value).toBe("My unsaved review");
    expect(button("Save comment").disabled).toBe(true);
    await act(async () => button("Use current source").click());
    expect(button("Save comment").disabled).toBe(false);
    await act(async () => button("Save comment").click());
    expect(upsert).toHaveBeenCalledWith("session", "pane", expect.objectContaining({ draft_id: null, capture: { root_id: "root", path: "file.md", expected_revision: "new-revision", start_line: null, end_line: null }, comment_text: "My unsaved review" }));
  } finally {
    await act(async () => mounted.unmount());
    host.remove();
  }
});

it("retains an open Review editor across refresh but requires explicit current-source capture", async () => {
  const root = { root_id: "review-source", kind: "repository", companion_id: null } as ContextRoot;
  const presentation = { session_id: "session", pane_id: "pane", binding_id: "binding" } as PanePresentation;
  const batch: CommentBatch = { batch_id: "batch", generation: 1, drafts: [], owner: { session_id: "session", pane_id: "pane", terminal_id: "terminal", source_kind: "review", source_id: "review-source" }, last_known_location: { workspace_id: "space", tab_id: "tab" }, live_attachment: null, updated_at: "now" };
  const upsert = vi.fn(async () => ({ ...batch, generation: 2 }));
  const client = { commentBatch: vi.fn(async () => batch), commentUpsert: upsert } as unknown as CockpitClient;
  const old = { review_id: "review", generation: 1, file_id: "file", side: "old" as const };
  const current = { ...old, generation: 2, side: "new" as const };
  function Harness() {
    const [editor, setEditor] = useState<ContextCommentEditorState | null>({ rootId: root.root_id, path: "file.ts", revision: "old", review: old, draftId: null, editor: "whole_file", text: "Retained prose", selection: null });
    return <CommentDrafts client={client} presentation={presentation} root={root} sourceKind="review" sourceIdentity="review-source" reviewCapture={current} path="file.ts" document={{ text: "current source", revision: "current" } as ContextDocument} selection={null} mode="source" editorState={editor} onEditorStateChange={setEditor} />;
  }
  const host = window.document.createElement("div"); window.document.body.append(host); const mounted = createRoot(host);
  const button = (label: string) => [...host.querySelectorAll("button")].find(item => item.textContent === label)!;
  try {
    await act(async () => mounted.render(<Harness />));
    expect(host.textContent).toContain("displayed review source changed");
    expect(host.querySelector("textarea")?.value).toBe("Retained prose");
    expect(button("Save comment").disabled).toBe(true);
    await act(async () => button("Use current source").click());
    expect(button("Save comment").disabled).toBe(false);
    await act(async () => button("Save comment").click());
    expect(upsert).toHaveBeenCalledWith("session", "pane", expect.objectContaining({ capture: expect.objectContaining({ review: current, expected_revision: "current" }), comment_text: "Retained prose" }));
  } finally { await act(async () => mounted.unmount()); host.remove(); }
});

it("opens an inline Review editor from C at the current SourceLines selection", async () => {
  const root = { root_id: "review-source", kind: "repository", companion_id: null } as ContextRoot;
  const presentation = { session_id: "session", pane_id: "pane", binding_id: "binding" } as PanePresentation;
  const batch: CommentBatch = { batch_id: "batch", generation: 1, drafts: [], owner: { session_id: "session", pane_id: "pane", terminal_id: "terminal", source_kind: "review", source_id: "review-source" }, last_known_location: { workspace_id: "space", tab_id: "tab" }, live_attachment: null, updated_at: "now" };
  const client = { commentBatch: vi.fn(async () => batch) } as unknown as CockpitClient;
  const onEditorDismissed = vi.fn();
  const host = window.document.createElement("div"); window.document.body.append(host); const mounted = createRoot(host);
  function Harness() {
    const [editor, setEditor] = useState<ContextCommentEditorState | null>(null);
    return <CommentDrafts client={client} presentation={presentation} root={root} sourceKind="review" sourceIdentity="review-source" reviewCapture={{ review_id: "review", generation: 1, file_id: "file", side: "new" }} path="file.ts" document={{ text: "one\ntwo\n", revision: "current" } as ContextDocument} selection={{ start: 2, end: 2 }} mode="source" editorState={editor} onEditorStateChange={setEditor} inlineEditor onEditorDismissed={onEditorDismissed}>
      {(_drafts, actions, inlineEditor) => <div data-review-surface onKeyDown={(event) => { if (event.key.toLowerCase() === "c") actions.createLines(); }}><SourceLines text={`one\ntwo\n`} state={{ rootId: root.root_id, path: "file.ts", mode: "source", selectionStart: 2, selectionEnd: 2, scrollTop: 0 }} onSelect={() => undefined} onScroll={() => undefined} inlineEditor={inlineEditor} /></div>}
    </CommentDrafts>;
  }
  try {
    await act(async () => mounted.render(<Harness />));
    await act(async () => { const line = host.querySelector<HTMLButtonElement>('[data-line="2"]')!; line.focus(); line.dispatchEvent(new KeyboardEvent("keydown", { bubbles: true, key: "c" })); });
    const editor = host.querySelector<HTMLElement>('.comment-inline-editor [aria-label="New comment"]');
    expect(editor).not.toBeNull();
    expect(host.querySelector('[data-line="2"] + .comment-inline-editor textarea')).not.toBeNull();
    expect(host.querySelector('[data-line="1"] + .comment-inline-editor textarea')).toBeNull();
    expect(window.document.activeElement).toBe(host.querySelector("textarea"));
    await act(async () => [...host.querySelectorAll("button")].find((item) => item.textContent === "Cancel")!.click());
    expect(onEditorDismissed).toHaveBeenCalledOnce();
  } finally { await act(async () => mounted.unmount()); host.remove(); }
});

it("saves a comment with Ctrl or Command Enter without leaking the shortcut", async () => {
  const root: ContextRoot = { root_id: "root", kind: "companion", label: "Context", path: "/context", repository_id: "repo", checkout_path: "/repo", companion_id: "source" };
  const presentation = { session_id: "session", pane_id: "pane", binding_id: "binding" } as PanePresentation;
  const batch: CommentBatch = { batch_id: "batch", generation: 1, owner: { session_id: "session", pane_id: "pane", terminal_id: "terminal", source_kind: "context", source_id: "source" }, last_known_location: { workspace_id: "space", tab_id: "tab" }, live_attachment: null, drafts: [], updated_at: "now" };
  const upsert = vi.fn(async () => ({ ...batch, generation: 2 }));
  const onEditorDismissed = vi.fn();
  const client = { commentBatch: vi.fn(async () => batch), commentUpsert: upsert } as unknown as CockpitClient;
  const host = window.document.createElement("div");
  window.document.body.append(host);
  const mounted = createRoot(host);
  const escapedKeys = vi.fn();
  window.document.body.addEventListener("keydown", escapedKeys);
  try {
    await act(async () => mounted.render(<CommentDrafts client={client} presentation={presentation} root={root} path="file.md" document={{ text: "source", revision: "revision" } as ContextDocument} selection={null} mode="source" editorState={{ rootId: "root", path: "file.md", revision: "revision", draftId: null, editor: "whole_file", text: "Save me", selection: null }} onEditorStateChange={() => undefined} onEditorDismissed={onEditorDismissed} />));
    const textarea = host.querySelector<HTMLTextAreaElement>("textarea")!;

    const plainEnter = new KeyboardEvent("keydown", { bubbles: true, cancelable: true, key: "Enter" });
    textarea.dispatchEvent(plainEnter);
    expect(plainEnter.defaultPrevented).toBe(false);
    expect(escapedKeys).toHaveBeenCalledTimes(1);

    const composing = new KeyboardEvent("keydown", { bubbles: true, cancelable: true, key: "Enter", ctrlKey: true, isComposing: true });
    textarea.dispatchEvent(composing);
    const repeated = new KeyboardEvent("keydown", { bubbles: true, cancelable: true, key: "Enter", metaKey: true, repeat: true });
    textarea.dispatchEvent(repeated);
    expect(composing.defaultPrevented).toBe(true);
    expect(repeated.defaultPrevented).toBe(true);
    expect(escapedKeys).toHaveBeenCalledTimes(1);
    expect(upsert).not.toHaveBeenCalled();

    await act(async () => textarea.dispatchEvent(new KeyboardEvent("keydown", { bubbles: true, cancelable: true, key: "Enter", ctrlKey: true })));
    expect(upsert).toHaveBeenCalledTimes(1);
    expect(onEditorDismissed).toHaveBeenCalledTimes(1);
  } finally {
    window.document.body.removeEventListener("keydown", escapedKeys);
    await act(async () => mounted.unmount());
    host.remove();
  }
});

it("confirms saved-batch discard inline and resets the active batch", async () => {
  const root = { root_id: "root", kind: "companion", label: "Context", path: "/context", repository_id: "repo", checkout_path: "/repo", companion_id: "source" } as ContextRoot;
  const presentation = { session_id: "session", pane_id: "pane", binding_id: "binding" } as PanePresentation;
  const active = { batch_id: "active", generation: 2, owner: { session_id: "session", pane_id: "pane", terminal_id: "terminal", source_kind: "context", source_id: "source" }, last_known_location: { workspace_id: "space", tab_id: "tab" }, live_attachment: null, drafts: [], updated_at: "1" } as CommentBatch;
  const list = { attachment: { owner: active.owner, location: active.last_known_location, binding_id: "binding", client_id: "client" }, batches: [{ batch_id: "active", generation: 2, owner: active.owner, last_known_location: active.last_known_location, draft_count: 0, updated_at: "1" }], truncated: false } as CommentBatchList;
  const batchLoad = vi.fn(async () => active);
  const discard = vi.fn(async () => ({ ...list, batches: [] }));
  const client = { commentBatch: batchLoad, commentBatches: vi.fn(async () => list), commentDiscard: discard } as unknown as CockpitClient;
  const host = window.document.createElement("div"); window.document.body.append(host); const mounted = createRoot(host);
  const button = (label: string) => [...host.querySelectorAll("button")].find(item => item.textContent === label)!;
  try {
    await act(async () => mounted.render(<CommentDrafts client={client} presentation={presentation} root={root} path="file.md" document={{ text: "source", revision: "current" } as ContextDocument} selection={null} mode="source" editorState={null} onEditorStateChange={() => undefined} />));
    await act(async () => { await Promise.resolve(); });
    await act(async () => button("0 comments").click());
    await act(async () => { await Promise.resolve(); });
    await act(async () => button("Discard").click());
    expect(discard).not.toHaveBeenCalled();
    expect(host.textContent).toContain("Discard this batch?");
    await act(async () => button("Discard").click());
    expect(discard).toHaveBeenCalledWith("session", "pane", { scope: expect.objectContaining({ binding_id: "binding" }), batch_id: "active", expected_generation: 2 });
    await act(async () => { await Promise.resolve(); });
    expect(batchLoad).toHaveBeenCalledTimes(2);
    expect(host.querySelector<HTMLButtonElement>(".comment-count")?.disabled).toBe(false);
  } finally { await act(async () => mounted.unmount()); host.remove(); }
});

it.each([0, 1])("refreshes generation %i batches using their persistence state", async (generation) => {
  const root: ContextRoot = { root_id: "root", kind: "repository", label: "Review", path: "/repo", repository_id: "repo", checkout_path: "/repo", companion_id: null };
  const presentation = { session_id: "session", pane_id: "pane", binding_id: "binding" } as PanePresentation;
  const batch: CommentBatch = { batch_id: "batch", generation, owner: { session_id: "session", pane_id: "pane", terminal_id: "terminal", source_kind: "review", source_id: "root" }, last_known_location: { workspace_id: "space", tab_id: "tab" }, live_attachment: null, drafts: [], updated_at: "now" };
  const commentBatch = vi.fn(async (_session: string, _pane: string, request: { batch_id: string | null }) => {
    if (generation === 0 && request.batch_id !== null) throw new Error("comment batch does not exist");
    return batch;
  });
  const client = { commentBatch } as unknown as CockpitClient;
  const host = window.document.createElement("div");
  window.document.body.append(host);
  const mounted = createRoot(host);
  const onEditorStateChange = vi.fn();
  const render = (invalidationGeneration: number) => <CommentDrafts client={client} presentation={presentation} root={root} sourceIdentity="root" sourceKind="review" path="file.md" document={null} selection={null} mode="source" editorState={null} onEditorStateChange={onEditorStateChange} invalidationGeneration={invalidationGeneration} />;
  try {
    await act(async () => mounted.render(render(0)));
    await act(async () => mounted.render(render(1)));
    await act(async () => mounted.render(render(2)));
    expect(commentBatch).toHaveBeenCalledTimes(3);
    expect(commentBatch).toHaveBeenLastCalledWith("session", "pane", expect.objectContaining({ batch_id: generation === 0 ? null : "batch" }));
    expect(host.querySelector('[role="alert"]')).toBeNull();
  } finally {
    await act(async () => mounted.unmount());
    host.remove();
  }
});
it("reconciles saved missing Context comments after explicit refresh without changing their identity or text", async () => {
  const root: ContextRoot = { root_id: "root", kind: "companion", label: "Context", path: "/context", repository_id: "repo", checkout_path: "/repo", companion_id: "source" };
  const presentation = { session_id: "session", pane_id: "pane", binding_id: "binding" } as PanePresentation;
  const draft = {
    draft_id: "saved-draft",
    file_ref: { root_id: "root", path: "notes.md", absolute_path: "/context/notes.md", revision: "revision", content_hash: "sha256:captured" },
    anchor: { kind: "whole_file" },
    comment_text: "Keep this note",
    source_state: "missing",
    created_at: "now",
    updated_at: "now",
  } as CommentBatch["drafts"][number];
  const batch: CommentBatch = { batch_id: "saved-batch", generation: 1, drafts: [draft], owner: { session_id: "session", pane_id: "pane", terminal_id: "terminal", source_kind: "context", source_id: "source" }, last_known_location: { workspace_id: "space", tab_id: "tab" }, live_attachment: null, updated_at: "now" };
  const recovered = { ...batch, drafts: [{ ...draft, source_state: "current" }] };
  const commentBatch = vi.fn().mockResolvedValueOnce(batch).mockResolvedValueOnce(recovered);
  const commentUpsert = vi.fn();
  const client = { commentBatch, commentUpsert } as unknown as CockpitClient;
  const host = window.document.createElement("div"); window.document.body.append(host); const mounted = createRoot(host);
  let setRefreshGeneration!: (generation: number) => void;
  function Harness() {
    const [refreshGeneration, updateRefreshGeneration] = useState(0);
    setRefreshGeneration = updateRefreshGeneration;
    return <CommentDrafts client={client} presentation={presentation} root={root} path="notes.md" document={null} selection={null} mode="markdown" editorState={null} onEditorStateChange={() => {}} refreshGeneration={refreshGeneration} />;
  }
  try {
    await act(async () => mounted.render(<Harness />));
    expect(host.querySelector(".comment-file-drafts")?.textContent).toContain("missing");
    expect(host.querySelector(".comment-file-drafts")?.textContent).toContain("Keep this note");
    await act(async () => setRefreshGeneration(1));
    expect(commentBatch).toHaveBeenCalledTimes(2);
    expect(commentBatch).toHaveBeenLastCalledWith("session", "pane", expect.objectContaining({ batch_id: "saved-batch" }));
    expect(host.querySelector(".comment-file-drafts")?.textContent).toContain("current");
    expect(host.querySelector(".comment-file-drafts")?.textContent).toContain("Whole file");
    expect(host.querySelector(".comment-file-drafts")?.textContent).toContain("Keep this note");
    expect(commentUpsert).not.toHaveBeenCalled();
  } finally { await act(async () => mounted.unmount()); host.remove(); }
});

it("ignores an in-flight explicit refresh after the comment identity changes", async () => {
  const root: ContextRoot = { root_id: "root", kind: "companion", label: "Context", path: "/context", repository_id: "repo", checkout_path: "/repo", companion_id: "source" };
  const presentation = { session_id: "session", pane_id: "pane", binding_id: "binding" } as PanePresentation;
  const draft = {
    draft_id: "saved-draft",
    file_ref: { root_id: "root", path: "notes.md", absolute_path: "/context/notes.md", revision: "revision", content_hash: "sha256:captured" },
    anchor: { kind: "whole_file" },
    comment_text: "Keep this note",
    source_state: "current",
    created_at: "now",
    updated_at: "now",
  } as CommentBatch["drafts"][number];
  const batch: CommentBatch = { batch_id: "saved-batch", generation: 1, drafts: [draft], owner: { session_id: "session", pane_id: "pane", terminal_id: "terminal", source_kind: "context", source_id: "source" }, last_known_location: { workspace_id: "space", tab_id: "tab" }, live_attachment: null, updated_at: "now" };
  let resolveRefresh!: (value: CommentBatch) => void;
  const staleRefresh = new Promise<CommentBatch>((resolve) => { resolveRefresh = resolve; });
  const commentBatch = vi.fn(async (_session: string, _pane: string, request: { batch_id: string | null }) => {
    if (commentBatch.mock.calls.length === 1) return batch;
    if (request.batch_id === "saved-batch") return staleRefresh;
    return { ...batch, drafts: [{ ...draft, source_state: "current" }] };
  });
  const client = { commentBatch } as unknown as CockpitClient;
  const host = window.document.createElement("div"); window.document.body.append(host); const mounted = createRoot(host);
  let setRefreshGeneration!: (generation: number) => void;
  let setSourceIdentity!: (identity: string) => void;
  function Harness() {
    const [refreshGeneration, updateRefreshGeneration] = useState(0);
    const [sourceIdentity, updateSourceIdentity] = useState("source");
    setRefreshGeneration = updateRefreshGeneration;
    setSourceIdentity = updateSourceIdentity;
    return <CommentDrafts client={client} presentation={presentation} root={root} sourceIdentity={sourceIdentity} path="notes.md" document={null} selection={null} mode="markdown" editorState={null} onEditorStateChange={() => {}} refreshGeneration={refreshGeneration} />;
  }
  try {
    await act(async () => mounted.render(<Harness />));
    await act(async () => setRefreshGeneration(1));
    expect(commentBatch).toHaveBeenCalledTimes(2);
    await act(async () => setSourceIdentity("recovered-source"));
    expect(commentBatch).toHaveBeenCalledTimes(3);
    expect(host.querySelector(".comment-file-drafts")?.textContent).toContain("current");
    await act(async () => { resolveRefresh({ ...batch, drafts: [{ ...draft, source_state: "missing" }] }); });
    expect(host.querySelector(".comment-file-drafts")?.textContent).toContain("current");
    expect(host.querySelector(".comment-file-drafts")?.textContent).toContain("Keep this note");
  } finally { await act(async () => mounted.unmount()); host.remove(); }
});
