// @vitest-environment jsdom
import "../input/viewerTestLayout";
import { act, useLayoutEffect, useState } from "react";
import { createRoot } from "react-dom/client";
import { expect, it, vi } from "vitest";
import { CockpitClientError, type CockpitClient } from "../../client/CockpitClient";
import type { CommentBatch, ContextDirectory, ContextDocument, LibraryListing, SpaceContextListing, ViewerContext, ViewerSourceOptions } from "../../protocol/generated/v1";
import type { ContextViewState } from "../context/ContextViewer";
import { FILE_NAVIGATION_EVENT } from "../input/fileNavigation";
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
      zoomLeafId: null, viewers: {}, heldMembers: [], heldTerminalIds: {}, bufferedFocus: null,
      focusedPaneId: "pane-a", revision: 0, selectionRevision: 0,
    } },
  };
  let publish: ((next: SessionLayoutState) => void) | null = null;
  let generation = 0;
  const contexts = new Map<string, ViewerContext>();
  const client = {
    librarySpaceList: vi.fn(async (request: { target: SpaceContextListing["target"] }): Promise<SpaceContextListing> => ({
      target: request.target, space_label: "Test Space", library_root: "/library", checkout_path: null,
      items: [], repository_paths: [], diagnostics: [],
    })),
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
      roots: [{ root_id: rootId, kind: "folder", label: `Folder ${source}`, path: `/folder-${source}`, repository_id: "repo", checkout_path: `/folder-${source}` }], diagnostics: [] };
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

it.each(["authorized", "missing", "stale target"] as const)("opens a Resources repository only from fresh authorized roots: %s", async (scenario) => {
  Object.assign(globalThis, { IS_REACT_ACT_ENVIRONMENT: true });
  const host = document.createElement("div");
  document.body.append(host);
  const mounted = createRoot(host);
  const libraryRoot: LibraryListing["root"] = {
    root_id: "library:fs", kind: "library", label: "Library", path: "/library",
    repository_id: "", checkout_path: "",
  };
  const repositoryRoot: LibraryListing["root"] = {
    root_id: "repository:fresh", kind: "repository", label: "Extra repository", path: "/extra/repository",
    repository_id: "extra", checkout_path: "/extra/repository",
  };
  const context: ViewerContext = {
    session_id: "session", viewer_id: "viewer", binding_id: "binding", tab_id: "tab", space_id: "space",
    kind: "files", source_kind: "context", source_id: "library-context",
    roots: [libraryRoot], default_root_id: libraryRoot.root_id, diagnostics: [],
  };
  const listing: SpaceContextListing = {
    target: { session_id: "session", space_id: "space" }, space_label: "Repository review",
    library_root: "/library", checkout_path: "/checkout", items: [],
    repository_paths: [repositoryRoot.path], diagnostics: [],
  };
  const sources: ViewerSourceOptions = {
    session_id: "session", pane_id: "pane", tab_id: scenario === "stale target" ? "another-tab" : "tab",
    space_id: "space", files_context_root_id: libraryRoot.root_id, files_folder_root_id: null,
    review_repository_ids: [], roots: scenario === "missing" ? [] : [repositoryRoot],
    reason: "", diagnostics: [],
  };
  const client = {
    projectConfiguration: vi.fn(async () => ({ providers: [] })),
    libraryListing: vi.fn(async (): Promise<LibraryListing> => ({ root: libraryRoot, generation: "1", items: [], follows: [], next_offset: null, diagnostics: [] })),
    librarySpaceList: vi.fn(async () => listing),
    viewerSources: vi.fn(async () => sources),
    viewerOpen: vi.fn(async () => ({ ...context, viewer_id: "repository-viewer", binding_id: "repository-binding", source_id: repositoryRoot.root_id, roots: [repositoryRoot], default_root_id: repositoryRoot.root_id })),
    viewerRelease: vi.fn(async () => undefined),
    contextDirectory: vi.fn(async (_session: string, _viewer: string, request: { binding_id: string; root_id: string; path: string }): Promise<ContextDirectory> => ({ ...request, entries: [], truncated: false, diagnostics: [] })),
    commentBatch: vi.fn(async (_session: string, viewer: string, request: { scope: { binding_id: string } }): Promise<CommentBatch> => ({
      batch_id: viewer === "repository-viewer" ? "repository-comments" : "library-comments", generation: 1,
      owner: { kind: "viewer", session_id: "session", server_instance: "server", tab_id: "tab", source_kind: "context", source_id: viewer === "repository-viewer" ? repositoryRoot.root_id : libraryRoot.root_id },
      last_known_location: { workspace_id: "space", tab_id: "tab" },
      live_attachment: { owner: { kind: "viewer", session_id: "session", server_instance: "server", tab_id: "tab", source_kind: "context", source_id: viewer === "repository-viewer" ? repositoryRoot.root_id : libraryRoot.root_id },
        location: { workspace_id: "space", tab_id: "tab" }, binding_id: request.scope.binding_id, client_id: "client" },
      drafts: [], updated_at: "now",
    })),
  } as unknown as CockpitClient;
  let state: SessionLayoutState = {
    ...createSessionLayoutState("session", "server"), activeSpaceId: "space", activeTabId: "tab",
    tabs: { tab: {
      tabId: "tab", spaceId: "space", root: { t: "leaf", id: "pane", kind: "terminal", w: 1 },
      terminals: { pane: "terminal" }, selectedLeafId: "pane", lastRealLeafId: "pane",
      zoomLeafId: null, viewers: {}, heldMembers: [], heldTerminalIds: {}, bufferedFocus: null,
      focusedPaneId: "pane", revision: 0, selectionRevision: 0,
    } },
  };
  let publish: ((next: SessionLayoutState) => void) | null = null;
  const ctx: LeafCtx = {
    client, sessionId: "session", serverInstance: "server", clientId: "client", getState: () => state,
    dispatch: (action) => { state = layoutReducer(state, action).state; publish?.(state); },
  };
  ctx.dispatch({ type: "viewer/open-begin", tabId: "tab", kind: "files", selector: { kind: "files_context" }, sourcePaneId: "pane", dir: "row" });
  ctx.dispatch({ type: "viewer/opened", tabId: "tab", kind: "files", context });
  function Harness() {
    const [snapshot, setSnapshot] = useState(state);
    useLayoutEffect(() => { publish = setSnapshot; return () => { publish = null; }; }, []);
    return <FilesLeaf ctx={ctx} tabId="tab" slot={snapshot.tabs.tab.viewers.files!} selected onSelect={() => {}} />;
  }
  try {
    await act(async () => mounted.render(<Harness />));
    for (let i = 0; i < 8; i++) await act(async () => { await Promise.resolve(); });
    const resources = [...host.querySelectorAll<HTMLButtonElement>("button")].find((button) => button.textContent === "Resources")!;
    expect(resources).toBeDefined();
    await act(async () => resources.click());
    expect(host.textContent).toContain("Library context for Repository review");
    expect(host.querySelector('[aria-label="Remove /extra/repository from Repository review"]')).not.toBeNull();
    await act(async () => host.querySelector<HTMLButtonElement>(".context-repo-path")!.click());
    for (let i = 0; i < 8; i++) await act(async () => { await Promise.resolve(); });
    expect(client.viewerSources).toHaveBeenCalledWith("session", "pane");
    if (scenario === "authorized") {
      expect(client.viewerOpen).toHaveBeenCalledWith("session", {
        tab_id: "tab", kind: "files", source_pane_id: "pane",
        source: { kind: "files_repository", root_id: "repository:fresh" }, client_id: "client",
      });
      expect(state.tabs.tab.viewers.files?.context?.roots).toEqual([repositoryRoot]);
      expect(client.contextDirectory).toHaveBeenCalledWith("session", "repository-viewer", expect.objectContaining({ binding_id: "repository-binding", root_id: repositoryRoot.root_id }), expect.any(AbortSignal));
    } else {
      expect(client.viewerOpen).not.toHaveBeenCalled();
      expect(host.querySelector(".context-resource-error")?.textContent).toBe(scenario === "missing"
        ? "This repository is no longer available in this Space."
        : "The repository source is no longer available in this tab.");
    }
  } finally {
    await act(async () => mounted.unmount());
    host.remove();
  }
});

it("leaves the portalled Add context Source in control through pane selection and restores its opener", async () => {
  Object.assign(globalThis, { IS_REACT_ACT_ENVIRONMENT: true });
  const host = document.createElement("div");
  document.body.append(host);
  const mounted = createRoot(host);
  const libraryRoot: LibraryListing["root"] = {
    root_id: "library:fs", kind: "library", label: "Library", path: "/library", repository_id: "", checkout_path: "",
  };
  const context: ViewerContext = {
    session_id: "session", viewer_id: "viewer", binding_id: "binding", tab_id: "tab", space_id: "space",
    kind: "files", source_kind: "context", source_id: "library-context", roots: [libraryRoot], default_root_id: libraryRoot.root_id, diagnostics: [],
  };
  const owner: CommentBatch["owner"] = { kind: "viewer", session_id: "session", server_instance: "server", tab_id: "tab", source_kind: "context", source_id: context.source_id };
  const client = {
    projectConfiguration: vi.fn(async () => ({ providers: [] })),
    libraryListing: vi.fn(async (): Promise<LibraryListing> => ({ root: libraryRoot, generation: "1", items: [], follows: [], next_offset: null, diagnostics: [] })),
    librarySpaceList: vi.fn(async (): Promise<SpaceContextListing> => ({
      target: { session_id: "session", space_id: "space" }, space_label: "Space", library_root: "/library", checkout_path: "/checkout", items: [], repository_paths: [], diagnostics: [],
    })),
    contextDirectory: vi.fn(async (_session: string, _viewer: string, request: { binding_id: string; root_id: string; path: string }): Promise<ContextDirectory> => ({ ...request, entries: [], truncated: false, diagnostics: [] })),
    contextFileIndex: vi.fn(),
    commentBatch: vi.fn(async (): Promise<CommentBatch> => ({
      batch_id: "comments", generation: 1, owner, last_known_location: { workspace_id: "space", tab_id: "tab" },
      live_attachment: { owner, location: { workspace_id: "space", tab_id: "tab" }, binding_id: "binding", client_id: "client" }, drafts: [], updated_at: "now",
    })),
  } as unknown as CockpitClient;
  let state: SessionLayoutState = {
    ...createSessionLayoutState("session", "server"), activeSpaceId: "space", activeTabId: "tab",
    tabs: { tab: {
      tabId: "tab", spaceId: "space", root: { t: "leaf", id: "pane", kind: "terminal", w: 1 },
      terminals: { pane: "terminal" }, selectedLeafId: "pane", lastRealLeafId: "pane",
      zoomLeafId: null, viewers: {}, heldMembers: [], heldTerminalIds: {}, bufferedFocus: null,
      focusedPaneId: "pane", revision: 0, selectionRevision: 0,
    } },
  };
  let publish: ((next: SessionLayoutState) => void) | null = null;
  const ctx: LeafCtx = {
    client, sessionId: "session", serverInstance: "server", clientId: "client", getState: () => state,
    dispatch: (action) => { state = layoutReducer(state, action).state; publish?.(state); },
  };
  ctx.dispatch({ type: "viewer/open-begin", tabId: "tab", kind: "files", selector: { kind: "files_context" }, sourcePaneId: "pane", dir: "row" });
  ctx.dispatch({ type: "viewer/opened", tabId: "tab", kind: "files", context });
  const onSelect = vi.fn();
  function Harness({ selected }: { selected: boolean }) {
    const [snapshot, setSnapshot] = useState(state);
    useLayoutEffect(() => { publish = setSnapshot; return () => { publish = null; }; }, []);
    return <FilesLeaf ctx={ctx} tabId="tab" slot={snapshot.tabs.tab.viewers.files!} selected={selected} onSelect={onSelect} />;
  }
  try {
    await act(async () => mounted.render(<Harness selected={false} />));
    for (let i = 0; i < 8; i++) await act(async () => { await Promise.resolve(); });
    const pane = host.querySelector<HTMLDivElement>(".viewer-leaf-body")!;
    const add = [...host.querySelectorAll<HTMLButtonElement>(".context-toolbar button")].find((button) => button.textContent === "Add…")!;
    expect(add).toBeDefined();
    await act(async () => add.focus());
    expect(onSelect).toHaveBeenCalledOnce();
    onSelect.mockClear();
    await act(async () => add.dispatchEvent(new MouseEvent("pointerdown", { bubbles: true })));
    expect(onSelect).toHaveBeenCalledOnce();
    onSelect.mockClear();
    await act(async () => add.click());
    const modal = document.body.querySelector<HTMLElement>('[role="dialog"].library-add')!;
    const label = [...modal.querySelectorAll<HTMLLabelElement>("label")].find((candidate) => candidate.textContent === "Source")!;
    const input = document.getElementById(label.htmlFor) as HTMLInputElement;
    expect(pane.contains(modal)).toBe(false);
    expect(document.activeElement).toBe(input);
    expect(onSelect).not.toHaveBeenCalled();

    await act(async () => {
      input.dispatchEvent(new FocusEvent("focusin", { bubbles: true }));
      input.dispatchEvent(new MouseEvent("pointerdown", { bubbles: true }));
    });
    expect(onSelect).not.toHaveBeenCalled();
    await act(async () => mounted.render(<Harness selected />));
    expect(document.activeElement).toBe(input);
    expect(onSelect).not.toHaveBeenCalled();

    for (const key of "/tmp/context") {
      await act(async () => {
        const keydown = new KeyboardEvent("keydown", { key, bubbles: true, cancelable: true });
        input.dispatchEvent(keydown);
        expect(keydown.defaultPrevented).toBe(false);
        Object.getOwnPropertyDescriptor(HTMLInputElement.prototype, "value")!.set!.call(input, input.value + key);
        input.dispatchEvent(new Event("input", { bubbles: true }));
        input.dispatchEvent(new KeyboardEvent("keyup", { key, bubbles: true }));
      });
      expect(document.activeElement).toBe(input);
      expect(onSelect).not.toHaveBeenCalled();
      expect(host.querySelector('[role="dialog"][aria-label="Go to file"]')).toBeNull();
    }
    await act(async () => window.dispatchEvent(new CustomEvent(FILE_NAVIGATION_EVENT, { detail: { action: "open-picker" } })));
    expect(input.value).toBe("/tmp/context");
    expect(document.activeElement).toBe(input);
    expect(onSelect).not.toHaveBeenCalled();
    expect(host.querySelector('[role="dialog"][aria-label="Go to file"]')).toBeNull();

    await act(async () => input.dispatchEvent(new KeyboardEvent("keydown", { key: "Escape", bubbles: true, cancelable: true })));
    expect(document.body.querySelector(".library-add")).toBeNull();
    expect(document.activeElement).toBe(add);
    expect(onSelect).toHaveBeenCalledOnce();
    onSelect.mockClear();
    await act(async () => pane.focus());
    expect(onSelect).toHaveBeenCalledOnce();
  } finally {
    await act(async () => mounted.unmount());
    host.remove();
  }
});
