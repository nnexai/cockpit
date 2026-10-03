import { expect, it, vi } from "vitest";
import { CockpitClientError, type CockpitClient } from "../../client/CockpitClient";
import type { ViewerContext, ViewerOpenRequest } from "../../protocol/generated/v1";
import { leaves } from "./splitTree";
import { createSessionLayoutState, layoutReducer, type LeafCtx, type TabLayoutState } from "./tabLayoutStore";
import { closeViewerLeaf, openViewerLeaf, releaseViewers } from "./viewerLifecycle";

function viewer(sourceId = "source-a", bindingId = "binding-a", tabId = "tab"): ViewerContext {
  return { session_id: "session", viewer_id: `${tabId}:viewer`, binding_id: bindingId, tab_id: tabId, space_id: "space", kind: "files", source_kind: "context", source_id: sourceId, roots: [], default_root_id: null, diagnostics: [] };
}

function fixture(client: CockpitClient, tabIds = ["tab"]) {
  let state = createSessionLayoutState("session", "instance");
  for (const tabId of tabIds) {
    const tab: TabLayoutState = { tabId, spaceId: "space", root: { t: "leaf", id: `${tabId}:terminal`, kind: "terminal", w: 1 }, terminals: { [`${tabId}:terminal`]: "terminal-id" }, selectedLeafId: `${tabId}:terminal`, lastRealLeafId: `${tabId}:terminal`, zoomLeafId: null, viewers: {}, widgetShare: 0.4, heldMembers: [], heldTerminalIds: {}, bufferedFocus: null, focusedPaneId: `${tabId}:terminal`, revision: 0, selectionRevision: 0 };
    state = { ...state, tabs: { ...state.tabs, [tabId]: tab }, activeTabId: tabId, activeSpaceId: "space" };
  }
  const initial = state;
  const ctx: LeafCtx = { client, sessionId: "session", serverInstance: "instance", clientId: "window", getState: () => state, dispatch: action => { state = layoutReducer(state, action).state; } };
  return { ctx, retire: () => { state = createSessionLayoutState("session", "instance"); }, restart: () => { state = { ...initial, serverInstance: "new-instance" }; } };
}

it("switches a single viewer between sources without losing either source's unsaved state", async () => {
  let binding = 0;
  const client = { viewerOpen: vi.fn(async (_session: string, request: ViewerOpenRequest) => viewer(request.source.kind === "files_context" ? "source-a" : "source-b", `binding-${++binding}`)), viewerRelease: vi.fn(async () => undefined) } as unknown as CockpitClient;
  const { ctx } = fixture(client);
  await openViewerLeaf(ctx, "tab", "files", { kind: "files_context" }, "row", "tab:terminal");
  const firstTree = ctx.getState().tabs.tab.root;
  const a = { commentEditor: { text: "Unsaved notes for A" }, path: "a.md" };
  ctx.dispatch({ type: "viewer/source-view", tabId: "tab", kind: "files", sourceId: "source-a", view: a });
  await openViewerLeaf(ctx, "tab", "files", { kind: "files_folder" }, "col", "tab:terminal");
  const b = { commentEditor: { text: "Different unsaved notes for B" }, path: "b.md" };
  ctx.dispatch({ type: "viewer/source-view", tabId: "tab", kind: "files", sourceId: "source-b", view: b });
  await openViewerLeaf(ctx, "tab", "files", { kind: "files_context" }, "row", "tab:terminal");
  const slot = ctx.getState().tabs.tab.viewers.files!;
  expect(slot.context?.source_id).toBe("source-a");
  expect(slot.viewsBySource).toEqual({ "source-a": a, "source-b": b });
  expect(ctx.getState().tabs.tab.root).toEqual(firstTree);
  expect(ctx.getState().tabs.tab.selectedLeafId).toBe("tab:files");
});

it("maps a repository root selector to the core-issued root ID and preserves source drafts", async () => {
  const rootId = "selected-repository-root";
  const repository = { ...viewer("repository-source", "repository-binding"), roots: [] };
  const open = vi.fn(async (_session: string, request: ViewerOpenRequest) => request.source.kind === "files_repository" ? repository : viewer());
  const client = { viewerOpen: open, viewerRelease: vi.fn(async () => undefined) } as unknown as CockpitClient;
  const { ctx } = fixture(client);
  await openViewerLeaf(ctx, "tab", "files", { kind: "files_context" }, "row", "tab:terminal");
  const firstTree = ctx.getState().tabs.tab.root;
  const draft = { commentEditor: { text: "Unsaved Library notes" }, path: "note.md" };
  ctx.dispatch({ type: "viewer/source-view", tabId: "tab", kind: "files", sourceId: "source-a", view: draft });
  await openViewerLeaf(ctx, "tab", "files", { kind: "files_repository", rootId }, "row", "tab:terminal");
  expect(open).toHaveBeenLastCalledWith("session", {
    tab_id: "tab", kind: "files", source_pane_id: "tab:terminal",
    source: { kind: "files_repository", root_id: rootId }, client_id: "window",
  });
  expect(ctx.getState().tabs.tab.viewers.files?.selector).toEqual({ kind: "files_repository", rootId });
  expect(ctx.getState().tabs.tab.viewers.files?.context?.source_id).toBe("repository-source");
  expect(ctx.getState().tabs.tab.root).toEqual(firstTree);
  await openViewerLeaf(ctx, "tab", "files", { kind: "files_context" }, "row", "tab:terminal");
  expect(ctx.getState().tabs.tab.viewers.files?.viewsBySource["source-a"]).toEqual(draft);
});

it("releases a context returned after its opening leaf was closed", async () => {
  let resolve!: (context: ViewerContext) => void;
  const response = new Promise<ViewerContext>(done => { resolve = done; });
  const release = vi.fn(async () => undefined);
  const client = { viewerOpen: vi.fn(() => response), viewerRelease: release } as unknown as CockpitClient;
  const { ctx } = fixture(client);
  const opening = openViewerLeaf(ctx, "tab", "files", { kind: "files_context" }, "row", "tab:terminal");
  await Promise.resolve();
  const closing = closeViewerLeaf(ctx, "tab", "files");
  resolve(viewer());
  await Promise.all([opening, closing]);
  expect(release).toHaveBeenCalledExactlyOnceWith("session", "tab:viewer");
  expect(ctx.getState().tabs.tab.viewers.files).toBeUndefined();
  expect(leaves(ctx.getState().tabs.tab.root).map(leaf => leaf.id)).toEqual(["tab:terminal"]);
  expect(ctx.getState().tabs.tab.selectedLeafId).toBe("tab:terminal");
});

it("keeps a failed release visible and permits an explicit close retry", async () => {
  const release = vi.fn().mockRejectedValueOnce(new Error("Host disconnected")).mockResolvedValue(undefined);
  const client = { viewerOpen: vi.fn(async () => viewer()), viewerRelease: release } as unknown as CockpitClient;
  const { ctx } = fixture(client);
  await openViewerLeaf(ctx, "tab", "files", { kind: "files_context" }, "row", "tab:terminal");
  await expect(closeViewerLeaf(ctx, "tab", "files")).rejects.toThrow("Host disconnected");
  expect(ctx.getState().tabs.tab.viewers.files?.status).toBe("error");
  expect(ctx.getState().tabs.tab.viewers.files?.context?.viewer_id).toBe("tab:viewer");
  await closeViewerLeaf(ctx, "tab", "files");
  expect(ctx.getState().tabs.tab.viewers.files).toBeUndefined();
});

it("does not evict or automatically retry after the viewer registry refuses an open", async () => {
  const open = vi.fn().mockRejectedValueOnce(new CockpitClientError("http_error", "Registry full", { operationCode: "viewer_limit" })).mockResolvedValue(viewer());
  const client = { viewerOpen: open, viewerRelease: vi.fn(async () => undefined) } as unknown as CockpitClient;
  const { ctx } = fixture(client);
  await openViewerLeaf(ctx, "tab", "files", { kind: "files_context" }, "row", "tab:terminal");
  expect(ctx.getState().tabs.tab.viewers.files?.status).toBe("error");
  expect(open).toHaveBeenCalledTimes(1);
  expect(client.viewerRelease).not.toHaveBeenCalled();
  await openViewerLeaf(ctx, "tab", "files", { kind: "files_context" }, "row", "tab:terminal");
  expect(ctx.getState().tabs.tab.viewers.files?.status).toBe("open");
});

it("releases retired contexts even after their tabs have disappeared from local state", async () => {
  const release = vi.fn(async () => undefined);
  const client = { viewerOpen: vi.fn(async (_session: string, request: ViewerOpenRequest) => viewer("source", "binding", request.tab_id)), viewerRelease: release } as unknown as CockpitClient;
  const { ctx, retire } = fixture(client, ["tab-a", "tab-b"]);
  await openViewerLeaf(ctx, "tab-a", "files", { kind: "files_context" }, "row", "tab-a:terminal");
  await openViewerLeaf(ctx, "tab-b", "files", { kind: "files_context" }, "row", "tab-b:terminal");
  retire();
  await releaseViewers(ctx, "all");
  expect(release.mock.calls).toEqual([["session", "tab-a:viewer"], ["session", "tab-b:viewer"]]);
  await releaseViewers(ctx, "all");
  expect(release).toHaveBeenCalledTimes(2);
});

it("does not close a replacement server's viewer when releasing the previous server's contexts", async () => {
  const release = vi.fn(async () => undefined);
  const old = { ...viewer(), viewer_id: "old-viewer" };
  const replacement = { ...viewer(), viewer_id: "new-viewer" };
  const client = { viewerOpen: vi.fn().mockResolvedValueOnce(old).mockResolvedValueOnce(replacement), viewerRelease: release } as unknown as CockpitClient;
  const { ctx, restart } = fixture(client);
  await openViewerLeaf(ctx, "tab", "files", { kind: "files_context" }, "row", "tab:terminal");
  restart();
  const newCtx = { ...ctx, serverInstance: "new-instance" };
  await openViewerLeaf(newCtx, "tab", "files", { kind: "files_context" }, "row", "tab:terminal");
  await releaseViewers(ctx, "all");
  expect(release).toHaveBeenCalledExactlyOnceWith("session", "old-viewer");
  expect(ctx.getState().tabs.tab.viewers.files?.context?.viewer_id).toBe("new-viewer");
  expect(ctx.getState().tabs.tab.selectedLeafId).toBe("tab:files");
});
