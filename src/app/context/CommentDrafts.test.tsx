// @vitest-environment jsdom
import { act, useState } from "react";
import { createRoot } from "react-dom/client";
import { expect, it, vi } from "vitest";
import type { CockpitClient } from "../../client/CockpitClient";
import type { CommentBatch, ContextDocument, ContextRoot, PanePresentation } from "../../protocol/generated/v1";
import { CommentDrafts } from "./CommentDrafts";
import type { ContextCommentEditorState } from "./ContextViewer";

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
