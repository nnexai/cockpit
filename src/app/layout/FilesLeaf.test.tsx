// @vitest-environment jsdom
import "../input/viewerTestLayout";
import { act, useLayoutEffect, useState } from "react";
import { createRoot } from "react-dom/client";
import { expect, it, vi } from "vitest";
import { CockpitClientError, type CockpitClient } from "../../client/CockpitClient";
import type { CommentBatch, ContextDirectory, ContextDocument, ViewerContext } from "../../protocol/generated/v1";
import type { ContextViewState } from "../context/ContextViewer";
import { FilesLeaf } from "./FilesLeaf";
import { createSessionLayoutState, layoutReducer, type LeafCtx, type SessionLayoutState } from "./tabLayoutStore";

it("retains per-source unsaved comments and overview choice through switches and explicit missing-viewer recovery", async () => {
  Object.assign(globalThis, { IS_REACT_ACT_ENVIRONMENT: true });
  const host = document.createElement("div");
  document.body.append(host);
  const mounted = createRoot(host);
  let state: SessionLayoutState = {
    ...createSessionLayoutState("session", "server"),
    activeSpaceId: "space", activeTabId: "tab",
    tabs: { tab: {
      tabId: "tab", spaceId: "space", root: { t: "split", id: "terminals", dir: "row", w: 1, kids: [
        { t: "leaf", id: "pane-a", kind: "terminal", w: 1 }, { t: "leaf", id: "pane-b", kind: "terminal", w: 1 },
      ] },
      terminals: { "pane-a": "terminal-a", "pane-b": "terminal-b" }, selectedLeafId: "pane-a", lastRealLeafId: "pane-a",
      zoomLeafId: null, viewers: {}, widgetShare: 0.4, heldMembers: [], heldTerminalIds: {}, bufferedFocus: null,
      focusedPaneId: "pane-a", revision: 0, selectionRevision: 0,
    } },
  };
  let publish: ((next: SessionLayoutState) => void) | null = null;
  let generation = 0;
  const contexts = new Map<string, ViewerContext>();
  const client = {
    contextDirectory: vi.fn(async (_session: string, _viewer: string, request: { binding_id: string; root_id: string; path: string }): Promise<ContextDirectory> => ({
      binding_id: request.binding_id, root_id: request.root_id, path: request.path, truncated: false, diagnostics: [],
      entries: [{ entry_id: "notes", name: "notes.txt", path: "notes.txt", kind: "file", bytes: 6, revision: "r1", refusal: null }],
    })),
    contextDocument: vi.fn(async (_session: string, _viewer: string, request: { binding_id: string; root_id: string; path: string }): Promise<ContextDocument> => ({
      binding_id: request.binding_id, root_id: request.root_id, path: request.path, revision: "r1", content_hash: "sha256:notes", bytes: 6,
      media_type: "text/plain", text: "notes\n", truncated: false, diagnostics: [],
    })),
    commentBatch: vi.fn(async (_session: string, _viewer: string, request: { scope: { binding_id: string } }): Promise<CommentBatch> => {
      const context = contexts.get(request.scope.binding_id)!;
      return { batch_id: `batch-${context.source_id}`, generation: 1,
        owner: { kind: "viewer", session_id: "session", server_instance: "server", tab_id: "tab", source_kind: "context", source_id: context.source_id },
        last_known_location: { workspace_id: "space", tab_id: "tab" }, live_attachment: null, drafts: [], updated_at: "now" };
    }),
    commentUpsert: vi.fn(),
    viewerOpen: vi.fn(async (): Promise<ViewerContext> => {
      const context = { ...state.tabs.tab.viewers.files!.context!, binding_id: `reopened-${++generation}` };
      contexts.set(context.binding_id, context);
      return context;
    }),
  } as unknown as CockpitClient;
  const ctx: LeafCtx = {
    client, sessionId: "session", serverInstance: "server", clientId: "client",
    getState: () => state,
    dispatch: (action) => { state = layoutReducer(state, action).state; publish?.(state); },
  };
  function switchTo(source: "a" | "b") {
    const rootId = `folder-${source}`;
    const context: ViewerContext = { session_id: "session", viewer_id: "viewer", binding_id: `binding-${++generation}`, tab_id: "tab", space_id: "space",
      kind: "files", source_kind: "context", source_id: rootId, default_root_id: rootId,
      roots: [{ root_id: rootId, kind: "folder", label: `Folder ${source}`, path: `/folder-${source}`, repository_id: "repo", checkout_path: `/folder-${source}`, companion_id: null }], diagnostics: [] };
    contexts.set(context.binding_id, context);
    ctx.dispatch({ type: "viewer/open-begin", tabId: "tab", kind: "files", selector: { kind: "files_folder" }, sourcePaneId: `pane-${source}`, dir: "row" });
    ctx.dispatch({ type: "viewer/opened", tabId: "tab", kind: "files", context });
  }
  function Harness() {
    const [snapshot, setSnapshot] = useState(state);
    useLayoutEffect(() => { publish = setSnapshot; return () => { publish = null; }; }, []);
    return <FilesLeaf ctx={ctx} tabId="tab" slot={snapshot.tabs.tab.viewers.files!} selected onSelect={() => {}} />;
  }
  async function writeComment(text: string) {
    await act(async () => { host.querySelector<HTMLButtonElement>('[data-context-path="notes.txt"]')!.click(); });
    await act(async () => { host.querySelector<HTMLButtonElement>('.context-comment-status button[title="Comment on whole file (Shift+C)"]')!.click(); });
    const textarea = host.querySelector<HTMLTextAreaElement>("textarea")!;
    await act(async () => {
      Object.getOwnPropertyDescriptor(HTMLTextAreaElement.prototype, "value")!.set!.call(textarea, text);
      textarea.dispatchEvent(new Event("input", { bubbles: true }));
    });
    expect(textarea.value).toBe(text);
  }
  try {
    switchTo("a");
    await act(async () => mounted.render(<Harness />));
    await writeComment("Source A: unfinished\nkeep this exact text");
    await act(async () => { host.querySelector<HTMLButtonElement>('[aria-label="Toggle file overview"]')!.click(); });
    await act(async () => switchTo("b"));
    expect(host.querySelector("textarea")).toBeNull();
    await writeComment("Source B: different unfinished text");
    await act(async () => switchTo("a"));
    expect(host.querySelector("textarea")?.value).toBe("Source A: unfinished\nkeep this exact text");
    expect(host.querySelector('[aria-label="Toggle file overview"]')?.getAttribute("aria-expanded")).toBe("false");
    const views = state.tabs.tab.viewers.files!.viewsBySource as Record<string, ContextViewState>;
    expect(views["folder-a"].commentEditor?.text).toBe("Source A: unfinished\nkeep this exact text");
    expect(views["folder-b"].commentEditor?.text).toBe("Source B: different unfinished text");
    expect(views["folder-a"].overviewChoice).toBe(false);
    await act(async () => switchTo("b"));
    expect(host.querySelector("textarea")?.value).toBe("Source B: different unfinished text");
    const missing = new CockpitClientError("http_error", "Viewer expired", { operationCode: "viewer_not_found" });
    vi.mocked(client.contextDirectory).mockRejectedValueOnce(missing);
    await act(async () => switchTo("a"));
    const reopen = [...host.querySelectorAll<HTMLButtonElement>("button")].find((button) => button.textContent === "Reopen");
    expect(reopen).toBeDefined();
    expect(client.viewerOpen).not.toHaveBeenCalled();
    await act(async () => { reopen!.click(); });
    expect(client.viewerOpen).toHaveBeenCalledWith("session", { tab_id: "tab", kind: "files", source_pane_id: "pane-a", source: { kind: "files_folder" }, client_id: "client" });
    expect(host.querySelector("textarea")?.value).toBe("Source A: unfinished\nkeep this exact text");
    expect(host.querySelector('[aria-label="Toggle file overview"]')?.getAttribute("aria-expanded")).toBe("false");
    expect(client.commentUpsert).not.toHaveBeenCalled();
  } finally {
    await act(async () => mounted.unmount());
    host.remove();
  }
});
