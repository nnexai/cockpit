// @vitest-environment jsdom
import "../input/viewerTestLayout";
import { act, createElement, useState } from "react";
import { createRoot } from "react-dom/client";
import { expect, it, vi } from "vitest";
import { CockpitClientError, type CockpitClient } from "../../client/CockpitClient";
import type { CommentBatch, CommentUpsertRequest, ContextDirectory, ContextFileIndex, LibraryItemSummary, LibraryListing, LibraryOperation, ViewerContext, SpaceContextListing } from "../../protocol/generated/v1";
import { ContextViewer, createContextViewState, SourceLines, type ContextViewState } from "./ContextViewer";


async function settle(): Promise<void> {
  await act(async () => { await Promise.resolve(); });
}

async function settleFrame(): Promise<void> {
  await act(async () => { await new Promise<void>((resolve) => requestAnimationFrame(() => resolve())); });
}

it("anchors an initial Shift+Arrow range at the focused source line without bubbling", async () => {
  Object.assign(globalThis, { IS_REACT_ACT_ENVIRONMENT: true });
  const host = document.createElement("div");
  document.body.append(host);
  const mounted = createRoot(host);
  const selected = vi.fn();
  const bubbled = vi.fn();
  function Harness() {
    const [selection, setSelection] = useState<{ start: number | null; end: number | null }>({ start: null, end: null });
    return <div onKeyDown={bubbled}><ContextViewerSource selection={selection} onSelect={(start, end, extend) => { selected(start, end, extend); setSelection({ start, end }); }} /></div>;
  }
  try {
    await act(async () => mounted.render(<Harness />));
    const line = (number: number) => host.querySelector<HTMLButtonElement>(`[data-line="${number}"]`)!;
    await act(async () => line(1).dispatchEvent(new KeyboardEvent("keydown", { bubbles: true, cancelable: true, key: "ArrowDown", shiftKey: true })));
    expect(selected).toHaveBeenLastCalledWith(1, 2, true);
    await act(async () => line(2).dispatchEvent(new KeyboardEvent("keydown", { bubbles: true, cancelable: true, key: "ArrowUp", shiftKey: true })));
    expect(selected).toHaveBeenLastCalledWith(1, 1, true);
    expect(bubbled).not.toHaveBeenCalled();
  } finally {
    await act(async () => mounted.unmount());
    host.remove();
  }
});

function ContextViewerSource({ selection, onSelect }: { selection: { start: number | null; end: number | null }; onSelect: (start: number, end: number, extend: boolean) => void }) {
  return <SourceLines text={"one\ntwo\nthree"} state={{ rootId: "root", path: "file.txt", mode: "source", revision: "r1", selectionStart: selection.start, selectionEnd: selection.end, scrollTop: 0 }} onSelect={onSelect} onScroll={vi.fn()} />;
}

it("opens only requested directories, compresses loaded single-child paths, and opens files from the tree keyboard", async () => {
  const host = document.createElement("div");
  document.body.append(host);
  const mounted = createRoot(host);
  let resolveSrc: ((directory: ContextDirectory) => void) | undefined;
  const directory = vi.fn((_session: string, _pane: string, request: { path: string }): Promise<ContextDirectory> => {
    const response = (entries: ContextDirectory["entries"]): ContextDirectory => ({ binding_id: "binding", root_id: "folder", path: request.path, truncated: false, diagnostics: [], entries });
    if (request.path === "") return Promise.resolve(response([
      { entry_id: "src", name: "src", path: "src", kind: "directory", bytes: null, revision: "r1", refusal: null },
      { entry_id: "refused", name: "unsafe.bin", path: "unsafe.bin", kind: "file", bytes: 12, revision: "r1", refusal: "Unsafe file" },
      { entry_id: "after", name: "after.html", path: "after.html", kind: "file", bytes: 12, revision: "r1", refusal: null },
    ]));
    if (request.path === "src") return new Promise((resolve) => { resolveSrc = resolve; });
    return Promise.resolve(response([{ entry_id: "file", name: "example.html", path: "src/deep/example.html", kind: "file", bytes: 12, revision: "r3", refusal: null }]));
  });
  const documentRead = vi.fn(async (_session: string, _pane: string, request: { path: string }) => ({ binding_id: "binding", root_id: "folder", path: request.path, revision: "r3", content_hash: null, bytes: 12, media_type: "text/html", text: "<p>ok</p>", truncated: false, diagnostics: [] }));
  const client = { contextDirectory: directory, contextDocument: documentRead } as unknown as CockpitClient;
  const context: ViewerContext = { session_id: "session", viewer_id: "viewer", binding_id: "binding", tab_id: "tab", space_id: "space", kind: "files", source_kind: "context", source_id: "source", default_root_id: "folder", roots: [{ root_id: "folder", kind: "folder", label: "Folder", path: "/folder", repository_id: "folder", checkout_path: "/folder" }], diagnostics: [] }
  function Harness() {
    const [view, setView] = useState(createContextViewState());
    return createElement(ContextViewer, { client, context, value: view, onChange: setView });
  }
  const press = async (button: HTMLButtonElement, key: string) => {
    await act(async () => button.dispatchEvent(new KeyboardEvent("keydown", { bubbles: true, key })));
    await settle();
  };
  try {
    await act(async () => mounted.render(<Harness />));
    await settle();
    const row = (path: string) => host.querySelector<HTMLButtonElement>(`[data-context-path="${path}"]`)!;
    row("src").focus();
    await press(row("src"), "ArrowRight");
    await act(async () => resolveSrc?.({
      binding_id: "binding", root_id: "folder", path: "src", truncated: false, diagnostics: [],
      entries: [{ entry_id: "deep", name: "deep", path: "src/deep", kind: "directory", bytes: null, revision: "r2", refusal: null }],
    }));
    await settle();
    expect(document.activeElement).toBe(row("src/deep"));
    await press(document.activeElement as HTMLButtonElement, "ArrowDown");
    await settleFrame();
    expect(document.activeElement).toBe(row("after.html"));
    await press(row("src/deep"), "ArrowRight");
    expect(directory).toHaveBeenCalledTimes(3);
    expect(host.querySelector('[data-context-path="src/deep"]')?.textContent).toContain("src/deep");
    await press(row("src/deep/example.html"), "Enter");
    expect(documentRead).toHaveBeenCalledWith("session", "viewer", expect.objectContaining({ path: "src/deep/example.html" }), expect.any(AbortSignal));
    await settle();
    expect(host.querySelector("iframe[title='src/deep/example.html preview']")).not.toBeNull();
    expect(host.textContent).not.toContain("Read-only");
  } finally {
    await act(async () => mounted.unmount());
    host.remove();
  }
});

it("does not reclaim tree focus after async expansion when the user focuses content", async () => {
  const host = document.createElement("div");
  document.body.append(host);
  const mounted = createRoot(host);
  let resolveSrc: ((directory: ContextDirectory) => void) | undefined;
  const directory = vi.fn((_session: string, _pane: string, request: { path: string }): Promise<ContextDirectory> => {
    if (request.path === "") return Promise.resolve({
      binding_id: "binding", root_id: "folder", path: "", truncated: false, diagnostics: [],
      entries: [{ entry_id: "src", name: "src", path: "src", kind: "directory", bytes: null, revision: "r1", refusal: null }],
    });
    return new Promise((resolve) => { resolveSrc = resolve; });
  });
  const client = { contextDirectory: directory } as unknown as CockpitClient;
  const context: ViewerContext = { session_id: "session", viewer_id: "viewer", binding_id: "binding", tab_id: "tab", space_id: "space", kind: "files", source_kind: "context", source_id: "source", default_root_id: "folder", roots: [{ root_id: "folder", kind: "folder", label: "Folder", path: "/folder", repository_id: "folder", checkout_path: "/folder" }], diagnostics: [] }
  function Harness() {
    const [view, setView] = useState(createContextViewState());
    return <ContextViewer client={client} context={context} value={view} onChange={setView} />;
  }
  try {
    await act(async () => mounted.render(<Harness />));
    await settle();
    const src = host.querySelector<HTMLButtonElement>('[data-context-path="src"]')!;
    src.focus();
    await act(async () => src.dispatchEvent(new KeyboardEvent("keydown", { bubbles: true, cancelable: true, key: "ArrowRight" })));
    const content = host.querySelector<HTMLElement>(".context-document")!;
    content.focus();
    await act(async () => resolveSrc?.({
      binding_id: "binding", root_id: "folder", path: "src", truncated: false, diagnostics: [],
      entries: [{ entry_id: "deep", name: "deep", path: "src/deep", kind: "directory", bytes: null, revision: "r2", refusal: null }],
    }));
    await settle();
    expect(document.activeElement).toBe(content);
  } finally {
    await act(async () => mounted.unmount());
    host.remove();
  }
});

it("indexes unopened nested files for the picker and opens the selected result", async () => {
  const host = document.createElement("div");
  document.body.append(host);
  const mounted = createRoot(host);
  const directory = vi.fn(async (_session: string, _pane: string, request: { path: string }): Promise<ContextDirectory> => ({
    binding_id: "binding", root_id: "folder", path: request.path, truncated: false, diagnostics: [],
    entries: request.path === "" ? [{ entry_id: "nested", name: "nested", path: "nested", kind: "directory", bytes: null, revision: "r1", refusal: null }] : [],
  }));
  const index = vi.fn(async (_session: string, _pane: string, request: { mode: "cached" | "fresh" }): Promise<ContextFileIndex> => ({
    binding_id: "binding", root_id: "folder", files: [{ path: "nested/target.md", bytes: 12 }], truncated: false, source: "walk", state: request.mode === "cached" ? "miss" : "fresh", diagnostics: [],
  }));
  const documentRead = vi.fn(async (_session: string, _pane: string, request: { path: string }) => ({ binding_id: "binding", root_id: "folder", path: request.path, revision: "r2", content_hash: null, bytes: 12, media_type: "text/markdown", text: "# Target", truncated: false, diagnostics: [] }));
  const client = { contextDirectory: directory, contextFileIndex: index, contextDocument: documentRead } as unknown as CockpitClient;
  const context: ViewerContext = { session_id: "session", viewer_id: "viewer", binding_id: "binding", tab_id: "tab", space_id: "space", kind: "files", source_kind: "context", source_id: "source", default_root_id: "folder", roots: [{ root_id: "folder", kind: "folder", label: "Folder", path: "/folder", repository_id: "folder", checkout_path: "/folder" }], diagnostics: [] }
  function Harness() {
    const [view, setView] = useState(createContextViewState());
    return <ContextViewer client={client} context={context} value={view} onChange={setView} />;
  }
  try {
    await act(async () => mounted.render(<Harness />));
    await settle();
    const treeFile = host.querySelector<HTMLButtonElement>("[data-context-path='nested']")!;
    treeFile.focus();
    await act(async () => treeFile.dispatchEvent(new KeyboardEvent("keydown", { bubbles: true, cancelable: true, key: "p", ctrlKey: true })));
    await settle(); await settle(); await settle();
    expect(directory.mock.calls.map((call) => call[2].path)).toEqual([""]);
    // One background warm-up on mount, then cached + fresh when the picker opens.
    expect(index).toHaveBeenCalledTimes(3);
    expect(host.querySelector(".file-picker-results")?.textContent).not.toContain(".cockpit");
    const result = [...host.querySelectorAll<HTMLButtonElement>(".file-picker-results button")].find((button) => button.title.includes("nested/target.md"));
    expect(result).toBeDefined();
    await act(async () => result?.click());
    await settle();
    expect(documentRead).toHaveBeenCalledWith("session", "viewer", expect.objectContaining({ path: "nested/target.md" }), expect.any(AbortSignal));
    await act(async () => { await new Promise<void>(resolve => requestAnimationFrame(() => resolve())); });
    expect(document.activeElement).toBe(host.querySelector(".context-document"));
  } finally {
    await act(async () => mounted.unmount());
    host.remove();
  }
});

it("cancels fresh picker indexing when the picker is dismissed", async () => {
  const host = document.createElement("div");
  document.body.append(host);
  const mounted = createRoot(host);
  let pickerSignal: AbortSignal | undefined;
  const directory = vi.fn(async (_session: string, _pane: string, request: { path: string }): Promise<ContextDirectory> => ({
    binding_id: "binding", root_id: "folder", path: request.path, truncated: false, diagnostics: [],
    entries: [{ entry_id: "first", name: "first.ts", path: "first.ts", kind: "file", bytes: 1, revision: "r1", refusal: null }],
  }));
  const fileIndex = vi.fn((_session: string, _pane: string, request: { mode: "cached" | "fresh" }, signal?: AbortSignal): Promise<ContextFileIndex> => {
    if (request.mode === "cached") return Promise.resolve({ binding_id: "binding", root_id: "folder", files: [], truncated: false, source: "walk", state: "miss", diagnostics: [] });
    pickerSignal = signal;
    return new Promise<ContextFileIndex>(() => undefined);
  });
  const client = { contextDirectory: directory, contextFileIndex: fileIndex } as unknown as CockpitClient;
  const context: ViewerContext = { session_id: "session", viewer_id: "viewer", binding_id: "binding", tab_id: "tab", space_id: "space", kind: "files", source_kind: "context", source_id: "source", default_root_id: "folder", roots: [{ root_id: "folder", kind: "folder", label: "Folder", path: "/folder", repository_id: "folder", checkout_path: "/folder" }], diagnostics: [] }
  function Harness() {
    const [view, setView] = useState(createContextViewState());
    return <ContextViewer client={client} context={context} value={view} onChange={setView} />;
  }
  try {
    await act(async () => mounted.render(<Harness />));
    await settle();
    const treeFile = host.querySelector<HTMLButtonElement>("[data-context-path='first.ts']")!;
    treeFile.focus();
    await act(async () => treeFile.dispatchEvent(new KeyboardEvent("keydown", { bubbles: true, cancelable: true, key: "p", ctrlKey: true })));
    await settle();
    expect(pickerSignal).toBeDefined();
    const input = host.querySelector<HTMLInputElement>(".file-picker input")!;
    await act(async () => input.dispatchEvent(new KeyboardEvent("keydown", { bubbles: true, cancelable: true, key: "Escape" })));
    expect(pickerSignal?.aborted).toBe(true);
    expect(host.querySelector(".file-picker")).toBeNull();
  } finally {
    await act(async () => mounted.unmount());
    host.remove();
  }
});

it("recovers a stale or failed picker list on its own, without reopening", async () => {
  const host = document.createElement("div");
  document.body.append(host);
  const mounted = createRoot(host);
  const directory = vi.fn(async (_session: string, _pane: string, request: { path: string }): Promise<ContextDirectory> => ({
    binding_id: "heal-binding", root_id: "heal-folder", path: request.path, truncated: false, diagnostics: [],
    entries: [{ entry_id: "first", name: "first.ts", path: "first.ts", kind: "file", bytes: 1, revision: "r1", refusal: null }],
  }));
  let freshCalls = 0;
  const fileIndex = vi.fn(async (_session: string, _pane: string, request: { mode: "cached" | "fresh" }): Promise<ContextFileIndex> => {
    const base = { binding_id: "heal-binding", root_id: "heal-folder", truncated: false as const, source: "walk" as const, diagnostics: [] };
    if (request.mode === "cached") return { ...base, files: [{ path: "old.md", bytes: null }], state: "cached" };
    freshCalls += 1;
    // Call 1 is the mount warm-up, call 2 the first picker fetch (fails), call 3 the automatic retry.
    if (freshCalls === 2) throw new Error("transient");
    return { ...base, files: [{ path: "new.md", bytes: null }], state: "fresh" };
  });
  const client = { contextDirectory: directory, contextFileIndex: fileIndex } as unknown as CockpitClient;
  const context: ViewerContext = { session_id: "session", viewer_id: "viewer", binding_id: "heal-binding", tab_id: "tab", space_id: "space", kind: "files", source_kind: "context", source_id: "source", default_root_id: "heal-folder", roots: [{ root_id: "heal-folder", kind: "folder", label: "Folder", path: "/folder", repository_id: "folder", checkout_path: "/folder" }], diagnostics: [] }
  function Harness() {
    const [view, setView] = useState(createContextViewState());
    return <ContextViewer client={client} context={context} value={view} onChange={setView} />;
  }
  try {
    await act(async () => mounted.render(<Harness />));
    await settle();
    const treeFile = host.querySelector<HTMLButtonElement>("[data-context-path='first.ts']")!;
    treeFile.focus();
    await act(async () => treeFile.dispatchEvent(new KeyboardEvent("keydown", { bubbles: true, cancelable: true, key: "p", ctrlKey: true })));
    await settle();
    expect(host.querySelector(".file-picker-status")?.textContent).toBe("May be out of date");
    await act(async () => { await new Promise((resolve) => setTimeout(resolve, 1_800)); });
    await settle();
    expect(host.querySelector(".file-picker-status")?.textContent).toBe("1 files");
    const titles = [...host.querySelectorAll<HTMLButtonElement>(".file-picker-results button")].map((button) => button.title);
    expect(titles.some((title) => title.includes("new.md"))).toBe(true);
    expect(titles.some((title) => title.includes("old.md"))).toBe(false);
  } finally {
    await act(async () => mounted.unmount());
    host.remove();
  }
});

it("keeps a late cached picker list visible after fresh indexing fails", async () => {
  const host = document.createElement("div");
  document.body.append(host);
  const mounted = createRoot(host);
  let resolveCached: ((result: ContextFileIndex) => void) | undefined;
  const directory = vi.fn(async (_session: string, _pane: string, request: { path: string }): Promise<ContextDirectory> => ({
    binding_id: "late-cache-binding", root_id: "late-cache-folder", path: request.path, truncated: false, diagnostics: [],
    entries: [{ entry_id: "first", name: "first.ts", path: "first.ts", kind: "file", bytes: 1, revision: "r1", refusal: null }],
  }));
  const fileIndex = vi.fn((_session: string, _pane: string, request: { mode: "cached" | "fresh" }): Promise<ContextFileIndex> => {
    if (request.mode === "fresh") return Promise.reject(new Error("fresh index failed"));
    return new Promise((resolve) => { resolveCached = resolve; });
  });
  const client = { contextDirectory: directory, contextFileIndex: fileIndex } as unknown as CockpitClient;
  const context: ViewerContext = { session_id: "session", viewer_id: "viewer", binding_id: "late-cache-binding", tab_id: "tab", space_id: "space", kind: "files", source_kind: "context", source_id: "source", default_root_id: "late-cache-folder", roots: [{ root_id: "late-cache-folder", kind: "folder", label: "Folder", path: "/folder", repository_id: "folder", checkout_path: "/folder" }], diagnostics: [] }
  function Harness() {
    const [view, setView] = useState(createContextViewState());
    return <ContextViewer client={client} context={context} value={view} onChange={setView} />;
  }
  try {
    await act(async () => mounted.render(<Harness />));
    await settle();
    const treeFile = host.querySelector<HTMLButtonElement>("[data-context-path='first.ts']")!;
    treeFile.focus();
    await act(async () => treeFile.dispatchEvent(new KeyboardEvent("keydown", { bubbles: true, cancelable: true, key: "p", ctrlKey: true })));
    await settle();
    expect(host.querySelector(".file-picker-status")?.textContent).toBe("Could not load files");
    await act(async () => resolveCached?.({
      binding_id: "late-cache-binding", root_id: "late-cache-folder", files: [{ path: "nested/target.md", bytes: null }],
      truncated: false, source: "walk", state: "cached", diagnostics: [],
    }));
    await settle();
    expect(host.querySelector(".file-picker-status")?.textContent).toBe("May be out of date");
    expect([...host.querySelectorAll<HTMLButtonElement>(".file-picker-results button")].some((button) => button.title.includes("nested/target.md"))).toBe(true);
  } finally {
    await act(async () => mounted.unmount());
    host.remove();
  }
});

it("shows the incomplete file-index status for a capped index", async () => {
  const host = document.createElement("div");
  document.body.append(host);
  const mounted = createRoot(host);
  const files = Array.from({ length: 10_000 }, (_, index) => ({ path: `file-${index}.ts`, bytes: 1 }));
  const directory = vi.fn(async (_session: string, _pane: string, request: { path: string }): Promise<ContextDirectory> => ({ binding_id: "binding", root_id: "folder", path: request.path, truncated: false, diagnostics: [], entries: [] }));
  const fileIndex = vi.fn(async (_session: string, _pane: string, request: { mode: "cached" | "fresh" }): Promise<ContextFileIndex> => ({
    binding_id: "binding", root_id: "folder", files, truncated: true, source: "walk", state: request.mode === "cached" ? "cached" : "fresh", diagnostics: [],
  }));
  const client = { contextDirectory: directory, contextFileIndex: fileIndex } as unknown as CockpitClient;
  const context: ViewerContext = { session_id: "session", viewer_id: "viewer", binding_id: "binding", tab_id: "tab", space_id: "space", kind: "files", source_kind: "context", source_id: "source", default_root_id: "folder", roots: [{ root_id: "folder", kind: "folder", label: "Folder", path: "/folder", repository_id: "folder", checkout_path: "/folder" }], diagnostics: [] }
  function Harness() {
    const [view, setView] = useState(createContextViewState());
    return <ContextViewer client={client} context={context} value={view} onChange={setView} />;
  }
  try {
    await act(async () => mounted.render(<Harness />));
    await settle();
    const viewer = host.querySelector<HTMLElement>(".context-viewer")!;
    await act(async () => viewer.dispatchEvent(new KeyboardEvent("keydown", { bubbles: true, cancelable: true, key: "p", ctrlKey: true })));
    await settle(); await settle(); await settle();
    expect(host.querySelector(".file-picker-status")?.textContent).toBe("10000 files · index incomplete");
    expect(fileIndex).toHaveBeenCalledTimes(2);
  } finally {
    await act(async () => mounted.unmount());
    host.remove();
  }
});

it("renders a PNG from a verified Folder root through the safe media command", async () => {
  Object.assign(globalThis, { IS_REACT_ACT_ENVIRONMENT: true });
  const host = document.createElement("div");
  document.body.append(host);
  const mounted = createRoot(host);
  const directory = vi.fn(async (_session: string, _pane: string, request: { path: string }): Promise<ContextDirectory> => ({
    binding_id: "binding", root_id: "folder", path: request.path, truncated: false, diagnostics: [], entries: [
      { entry_id: "png", name: "palette.png", path: "palette.png", kind: "file", bytes: 68, revision: "r1", refusal: null },
    ],
  }));
  const documentRead = vi.fn(async (_session: string, _pane: string, request: { path: string }) => ({ binding_id: "binding", root_id: "folder", path: request.path, revision: "r1", content_hash: null, bytes: 68, media_type: "application/octet-stream", text: null, truncated: false, diagnostics: [] }));
  const mediaRead = vi.fn(async () => ({ binding_id: "binding", root_id: "folder", path: "palette.png", revision: "r1", content_hash: "sha256:fixture", bytes: 68, mime_type: "image/png", width: 1, height: 1, data_base64: "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVQIHWP4z8DwHwAFgAI/ScLq9wAAAABJRU5ErkJggg==" }));
  const client = { contextDirectory: directory, contextDocument: documentRead, contextMedia: mediaRead } as unknown as CockpitClient;
  const context: ViewerContext = { session_id: "session", viewer_id: "viewer", binding_id: "binding", tab_id: "tab", space_id: "space", kind: "files", source_kind: "context", source_id: "source", default_root_id: "folder", roots: [{ root_id: "folder", kind: "folder", label: "Folder", path: "/folder", repository_id: "folder", checkout_path: "/folder" }], diagnostics: [] }
  vi.stubGlobal("URL", { ...URL, createObjectURL: vi.fn(() => "blob:folder-png"), revokeObjectURL: vi.fn() });
  function Harness() {
    const [view, setView] = useState(createContextViewState());
    return createElement(ContextViewer, { client, context, value: view, onChange: setView });
  }
  try {
    await act(async () => mounted.render(<Harness />));
    await settle();
    const row = host.querySelector<HTMLButtonElement>('[data-context-path="palette.png"]')!;
    await act(async () => row.dispatchEvent(new KeyboardEvent("keydown", { bubbles: true, key: "Enter" })));
    await settle();
    await settle();
    expect(mediaRead).toHaveBeenCalledWith("session", "viewer", { binding_id: "binding", root_id: "folder", path: "palette.png", expected_revision: "r1" }, expect.any(AbortSignal));
    expect(host.querySelector<HTMLImageElement>('img[alt="palette.png"]')?.src).toBe("blob:folder-png");
  } finally {
    await act(async () => mounted.unmount());
    host.remove();
    vi.unstubAllGlobals();
  }
});

it("retains the visible source when a continuation crosses a revision change", async () => {
  const host = document.createElement("div");
  document.body.append(host);
  const mounted = createRoot(host);
  const directory = vi.fn(async (): Promise<ContextDirectory> => ({
    binding_id: "binding", root_id: "folder", path: "", truncated: false, diagnostics: [],
    entries: [{ entry_id: "large", name: "large.txt", path: "large.txt", kind: "file", bytes: 6, revision: "r1", refusal: null }],
  }));
  const documentRead = vi.fn()
    .mockResolvedValueOnce({ binding_id: "binding", root_id: "folder", path: "large.txt", revision: "r1", content_hash: null, bytes: 6, media_type: "text/plain", text: "old\n", truncated: true, offset: 0, next_offset: 4, total_bytes: 6, line_offset: 0, diagnostics: [] })
    .mockResolvedValueOnce({ binding_id: "binding", root_id: "folder", path: "large.txt", revision: "r2", content_hash: null, bytes: 6, media_type: "text/plain", text: "new\n", truncated: false, offset: 4, next_offset: undefined, total_bytes: 6, line_offset: undefined, diagnostics: [] });
  const client = { contextDirectory: directory, contextDocument: documentRead } as unknown as CockpitClient;
  const context: ViewerContext = { session_id: "session", viewer_id: "viewer", binding_id: "binding", tab_id: "tab", space_id: "space", kind: "files", source_kind: "context", source_id: "source", default_root_id: "folder", roots: [{ root_id: "folder", kind: "folder", label: "Folder", path: "/folder", repository_id: "folder", checkout_path: "/folder" }], diagnostics: [] }
  function Harness() {
    const [view, setView] = useState(createContextViewState());
    return <ContextViewer client={client} context={context} value={view} onChange={setView} />;
  }
  try {
    await act(async () => mounted.render(<Harness />));
    await settle();
    await act(async () => host.querySelector<HTMLButtonElement>('[data-context-path="large.txt"]')?.click());
    await settle();
    expect(host.textContent).toContain("old");
    const continuation = [...host.querySelectorAll<HTMLButtonElement>("button")].find((button) => button.textContent?.includes("Load next source page"));
    expect(continuation).toBeDefined();
    await act(async () => continuation?.click());
    await settle();
    expect(host.textContent).toContain("old");
    expect(host.textContent).toContain("Stale source");
    expect(documentRead).toHaveBeenLastCalledWith("session", "viewer", expect.objectContaining({ path: "large.txt", offset: 4, expected_revision: "r1" }), expect.any(AbortSignal));
  } finally {
    await act(async () => mounted.unmount());
    host.remove();
  }
});

it("releases continuation loading when navigation aborts the pending page", async () => {
  const host = document.createElement("div");
  document.body.append(host);
  const mounted = createRoot(host);
  let resolvePage: (() => void) | undefined;
  const pendingPage = new Promise<void>((resolve) => { resolvePage = resolve; });
  const directory = vi.fn(async (): Promise<ContextDirectory> => ({
    binding_id: "binding", root_id: "folder", path: "", truncated: false, diagnostics: [], entries: [
      { entry_id: "one", name: "one.txt", path: "one.txt", kind: "file", bytes: 6, revision: "r1", refusal: null },
      { entry_id: "two", name: "two.txt", path: "two.txt", kind: "file", bytes: 6, revision: "r2", refusal: null },
    ],
  }));
  const documentRead = vi.fn(async (_session: string, _pane: string, request: { path: string; offset?: number }, signal?: AbortSignal) => {
    if (request.path === "one.txt" && request.offset !== undefined) {
      await pendingPage;
      if (signal?.aborted) throw new DOMException("aborted", "AbortError");
      return { binding_id: "binding", root_id: "folder", path: "one.txt", revision: "r1", content_hash: null, bytes: 6, media_type: "text/plain", text: "tail", truncated: false, offset: 4, next_offset: undefined, total_bytes: 6, line_offset: undefined, diagnostics: [] };
    }
    const path = request.path;
    return { binding_id: "binding", root_id: "folder", path, revision: path === "one.txt" ? "r1" : "r2", content_hash: null, bytes: 6, media_type: "text/plain", text: path === "one.txt" ? "head" : "two", truncated: true, offset: 0, next_offset: 4, total_bytes: 6, line_offset: 0, diagnostics: [] };
  });
  const client = { contextDirectory: directory, contextDocument: documentRead } as unknown as CockpitClient;
  const context: ViewerContext = { session_id: "session", viewer_id: "viewer", binding_id: "binding", tab_id: "tab", space_id: "space", kind: "files", source_kind: "context", source_id: "source", default_root_id: "folder", roots: [{ root_id: "folder", kind: "folder", label: "Folder", path: "/folder", repository_id: "folder", checkout_path: "/folder" }], diagnostics: [] }
  function Harness() {
    const [view, setView] = useState(createContextViewState());
    return <ContextViewer client={client} context={context} value={view} onChange={setView} />;
  }
  try {
    await act(async () => mounted.render(<Harness />));
    await settle();
    await act(async () => host.querySelector<HTMLButtonElement>('[data-context-path="one.txt"]')?.click());
    await settle();
    const continuation = [...host.querySelectorAll<HTMLButtonElement>("button")].find((button) => button.textContent?.includes("Load next source page"));
    expect(continuation).toBeDefined();
    await act(async () => continuation?.click());
    await act(async () => host.querySelector<HTMLButtonElement>('[data-context-path="two.txt"]')?.click());
    await settle();
    const next = [...host.querySelectorAll<HTMLButtonElement>("button")].find((button) => button.textContent?.includes("Load next source page"));
    expect(next).toBeDefined();
    expect(next?.disabled).toBe(false);
    resolvePage?.();
  } finally {
    await act(async () => mounted.unmount());
    host.remove();
  }
});


it("shows the Library as a pane root with Add… and Refresh all instead of Resources, keeping file reread distinct from provider refresh", async () => {
  Object.assign(globalThis, { IS_REACT_ACT_ENVIRONMENT: true });
  const host = document.createElement("div");
  document.body.append(host);
  const mounted = createRoot(host);
  const item: LibraryItemSummary = {
    item_id: "source:mr", logical_id: "source:gitlab:https://gitlab.test:review:platform/api!482", kind: "provider_snapshot", provider_id: "gitlab", provider_instance: "https://gitlab.test", resource_type: "review",
    canonical_id: "platform/api!482", container: { container_id: "platform/api", label: "platform/api" }, parent_item_id: null, ancestors: [], order: null, title: "Fix token refresh race",
    document_path: "gitlab/gitlab.test/platform/api/merge-requests/482/Fix token refresh race.md", item_path: "gitlab/gitlab.test/platform/api/merge-requests/482", source_url: "https://gitlab.test/platform/api/-/merge_requests/482", original_url: null, source_revision: "abc", revision: "sha256:r1",
    state: "fresh", partial: null, conflict: [], fetched_at: null, checked_at: null, refs: [{ kind: "manual" }], purge_after: null, issue: null, attachments: [], folder: null, diagnostics: [],
  };
  const listing: LibraryListing = { root: { root_id: "library:fs", kind: "library", label: "Library", path: "/data/library", repository_id: "", checkout_path: "" }, generation: "1", items: [item], follows: [], next_offset: null, diagnostics: [] };
  const operation: LibraryOperation = { operation_id: "op-1", kind: "refresh", phases: [{ phase: "library", state: "done", done: 1, total: 1, message: null, error: null }], item_ids: ["source:mr"], report: { new: 0, updated: 1, unchanged: 0, removed_at_source: 0, dropped: 0, partial: 0, failed: 0, conflict: 0, rows: [], truncated_rows: false }, space: null, target: null, cancel_requested: false, finished: true, created_at: "", updated_at: "" };
  let documentText = "# Fix it";
  let currentListing = listing;
  const client = {
    contextDirectory: vi.fn(async (): Promise<ContextDirectory> => ({ binding_id: "binding", root_id: "folder", path: "", truncated: false, diagnostics: [], entries: [] })),
    libraryListing: vi.fn(async () => currentListing),
    projectConfiguration: vi.fn(async () => ({ providers: [{ id: "gitlab", kind: "gitlab" as const, base_url: "https://gitlab.test", executable: "/usr/bin/glab" }] })),
    libraryDocument: vi.fn(async (request: { path: string }) => ({ binding_id: "library", root_id: "library:fs", path: request.path, revision: "r1", content_hash: null, bytes: 7, media_type: "text/markdown", text: documentText, truncated: false, diagnostics: [] })),
    contextDocument: vi.fn(),
    commentBatch: vi.fn(),
    libraryRefresh: vi.fn(async () => operation),
  } as unknown as CockpitClient;
  const context: ViewerContext = { session_id: "session", viewer_id: "viewer", binding_id: "binding", tab_id: "tab", space_id: "space", kind: "files", source_kind: "context", source_id: "source", default_root_id: "folder", roots: [{ root_id: "folder", kind: "folder", label: "Folder", path: "/folder", repository_id: "repo", checkout_path: "/repo" }], diagnostics: [] }
  function Harness() {
    const [view, setView] = useState(createContextViewState());
    return createElement(ContextViewer, { client, context, value: view, onChange: setView });
  }
  const flush = async () => { for (let index = 0; index < 6; index += 1) await settle(); };
  const toolbarButton = (label: string) => [...host.querySelectorAll<HTMLButtonElement>(".context-toolbar button")].find((candidate) => candidate.textContent === label || candidate.getAttribute("aria-label") === label);
  // Rereading the Library directory is local; it lives in the `⋯` menu, not a toolbar button.
  const reloadListing = async () => {
    await act(async () => toolbarButton("Library actions")!.click());
    await act(async () => [...document.body.querySelectorAll<HTMLButtonElement>('[role="menuitem"]')].find((entry) => entry.textContent?.startsWith("Reload listing"))!.click());
  };
  try {
    await act(async () => mounted.render(<Harness />));
    await flush();
    expect(toolbarButton("Resources")).toBeDefined();
    expect(client.libraryListing).not.toHaveBeenCalled();

    const select = host.querySelector<HTMLSelectElement>(".context-root-select select")!;
    await act(async () => { select.value = "library"; select.dispatchEvent(new Event("change", { bubbles: true })); });
    await flush();
    expect(toolbarButton("Show file tree")).toBeUndefined();
    expect(toolbarButton("Add…")).toBeDefined();
    expect(toolbarButton("Hide file tree")?.getAttribute("aria-keyshortcuts")).toBe("Alt+1");
    expect(toolbarButton("Find in Library")?.getAttribute("aria-keyshortcuts")).toBe("Control+p /");
    expect(toolbarButton("Refresh all")?.disabled).toBe(false);
    const row = host.querySelector<HTMLButtonElement>('[data-library-row="source:mr"]')!;
    expect(row.textContent).toContain("!482 Fix token refresh race");
    expect(host.querySelector('[data-library-row^="instance:"]')?.textContent).toContain("GitLab · gitlab.test");

    await act(async () => row.click());
    await flush();
    expect(client.libraryDocument).toHaveBeenCalledWith({ path: "gitlab/gitlab.test/platform/api/merge-requests/482/Fix token refresh race.md", expected_revision: null, offset: null }, expect.any(AbortSignal));
    expect(client.contextDocument).not.toHaveBeenCalled();
    expect(host.querySelector(".context-comment-status")).toBeNull();
    expect(host.querySelector(".library-kind-chip")?.textContent).toBe("GitLab MR");
    // Wrap only applies to source lines: it appears with Source and is not offered over the rendered preview.
    expect(toolbarButton("Refresh Context files")).toBeUndefined();
    expect(toolbarButton("Wrap long lines")).toBeUndefined();
    const segment = (label: string) => [...host.querySelectorAll<HTMLButtonElement>(".viewer-segmented button")].find((button) => button.textContent === label)!;
    await act(async () => segment("Source").click());
    expect(segment("Source").getAttribute("aria-keyshortcuts")).toBe("Alt+M");
    expect(toolbarButton("Wrap long lines")?.getAttribute("aria-keyshortcuts")).toBe("Alt+Z");
    await act(async () => segment("Preview").click());
    expect(toolbarButton("Wrap long lines")).toBeUndefined();

    const listingReads = vi.mocked(client.libraryListing).mock.calls.length;
    await reloadListing();
    await flush();
    expect(client.libraryRefresh).not.toHaveBeenCalled();
    expect(vi.mocked(client.libraryListing).mock.calls.length).toBeGreaterThan(listingReads);

    await act(async () => toolbarButton("Refresh all")!.click());
    await flush();
    expect(client.libraryRefresh).toHaveBeenCalledWith({ scope: "all" });
    expect(host.querySelector(".library-report")?.textContent).toContain("1 updated");
    // A failure before enumeration has no item IDs: Retry must preserve Refresh all, not issue an empty item refresh.
    vi.mocked(client.libraryRefresh).mockResolvedValueOnce({
      ...operation, operation_id: "refresh-before-enumeration", item_ids: [], report: null,
      phases: [{ phase: "library", state: "failed", done: 0, total: null, message: null, error: { code: "source_cli_failed", message: "Provider offline" } }],
    });
    await act(async () => toolbarButton("Refresh all")!.click());
    await flush();
    const retryRefresh = [...host.querySelectorAll<HTMLButtonElement>(".library-status-area button")].find((button) => button.textContent === "Retry")!;
    await act(async () => retryRefresh.click());
    await flush();
    expect(client.libraryRefresh).toHaveBeenLastCalledWith({ scope: "all" });
    documentText = "# Updated by another source";
    await act(async () => window.dispatchEvent(new Event("cockpit:library-changed")));
    await flush();
    expect(vi.mocked(client.libraryDocument).mock.calls.length).toBeGreaterThan(1);
    expect(host.textContent).toContain("Updated by another source");

    vi.mocked(client.libraryListing).mockRejectedValueOnce(new Error("listing offline"));
    await reloadListing();
    await flush();
    expect(host.querySelector('[role="alert"]')?.textContent).toContain("Library unavailable: listing offline. Space context is unaffected.");
    expect(host.querySelector('[data-library-row="source:mr"]')).not.toBeNull();
    currentListing = { ...listing, generation: "2", items: [] };
    await act(async () => window.dispatchEvent(new Event("cockpit:library-changed")));
    await flush();
    expect(host.querySelector(".library-kind-chip")).toBeNull();
  } finally {
    await act(async () => mounted.unmount());
    host.remove();
  }
});

it.each(["bound", "global"] as const)("opens every copied folder file and safe media through the %s Library authority", async (authority) => {
  Object.assign(globalThis, { IS_REACT_ACT_ENVIRONMENT: true });
  const host = document.createElement("div");
  document.body.append(host);
  const mounted = createRoot(host);
  const item: LibraryItemSummary = {
    item_id: "folder:notes", logical_id: "folder:notes", kind: "folder_copy", provider_id: null, provider_instance: null, resource_type: null,
    canonical_id: null, container: null, parent_item_id: null, ancestors: [], order: null, title: "Design notes",
    document_path: "folders/Design notes/README.md", item_path: "folders/Design notes", source_url: null, original_url: null, source_revision: null, revision: "sha256:r1",
    state: "fresh", partial: null, conflict: [], fetched_at: null, checked_at: null, refs: [{ kind: "manual" }], purge_after: null, issue: null, attachments: [], diagnostics: [],
    folder: { origin_path: "/home/user/notes", git_working_tree: false, files: 3, bytes: 88, skipped_symlinks: 0, skipped_special: 0, skipped_ignored: 0, skipped_other: 0 },
  };
  const library: LibraryListing = { root: { root_id: "library:fs", kind: "library", label: "Library", path: "/data/library", repository_id: "", checkout_path: "" }, generation: "1", items: [item], follows: [], next_offset: null, diagnostics: [] };
  const entry = (path: string, kind: "file" | "directory") => ({ entry_id: path, name: path.slice(path.lastIndexOf("/") + 1), path, kind, bytes: kind === "file" ? path.endsWith(".png") ? 68 : 10 : null, revision: "r1", refusal: null });
  const files = [{ path: item.document_path!, bytes: 10 }, { path: "folders/Design notes/docs/guide.md", bytes: 10 }, { path: "folders/Design notes/palette.png", bytes: 68 }];
  const documentResponse = (bindingId: string, path: string) => ({ binding_id: bindingId, root_id: library.root.root_id, path, revision: "r1", content_hash: null, bytes: path.endsWith(".png") ? 68 : 10, media_type: path.endsWith(".png") ? "application/octet-stream" : "text/markdown", text: path.endsWith(".png") ? null : path.endsWith("guide.md") ? "# Guide body" : "# Readme body", truncated: false, diagnostics: [] });
  const mediaResponse = (bindingId: string, path: string) => ({ binding_id: bindingId, root_id: library.root.root_id, path, revision: "r1", content_hash: "sha256:fixture", bytes: 68, mime_type: "image/png", width: 1, height: 1, data_base64: "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVQIHWP4z8DwHwAFgAI/ScLq9wAAAABJRU5ErkJggg==" });
  const client = {
    libraryListing: vi.fn(async () => library),
    projectConfiguration: vi.fn(async () => ({ providers: [] })),
    libraryDirectory: vi.fn(async (request: { path: string }): Promise<ContextDirectory> => ({ binding_id: "library", root_id: "library:fs", path: request.path, truncated: false, diagnostics: [],
      entries: request.path === "" ? [entry("folders/Design notes", "directory")] : files.map((file) => entry(file.path, "file")) })),
    libraryFileIndex: vi.fn(async (request: { mode: "cached" | "fresh" }): Promise<ContextFileIndex> => ({ binding_id: "library", root_id: library.root.root_id, files, truncated: false, source: "walk", state: request.mode === "cached" ? "cached" : "fresh", diagnostics: [] })),
    libraryDocument: vi.fn(async (request: { path: string }) => documentResponse("library", request.path)),
    libraryMedia: vi.fn(async (request: { path: string }) => mediaResponse("library", request.path)),
    contextFileIndex: vi.fn(async (_session: string, _viewer: string, request: { binding_id: string; root_id: string; mode: "cached" | "fresh" }): Promise<ContextFileIndex> => ({ ...request, files, truncated: false, source: "walk", state: request.mode === "cached" ? "cached" : "fresh", diagnostics: [] })),
    contextDocument: vi.fn(async (_session: string, _viewer: string, request: { binding_id: string; path: string }) => documentResponse(request.binding_id, request.path)),
    contextMedia: vi.fn(async (_session: string, _viewer: string, request: { binding_id: string; path: string }) => mediaResponse(request.binding_id, request.path)),
    commentBatch: vi.fn(async (): Promise<CommentBatch> => ({
      batch_id: "folder-comments", generation: 1,
      owner: { kind: "viewer", session_id: "session", server_instance: "server", tab_id: "tab", source_kind: "context", source_id: library.root.root_id },
      last_known_location: { workspace_id: "space", tab_id: "tab" }, live_attachment: null, drafts: [], updated_at: "now",
    })),
  } as unknown as CockpitClient;
  const context: ViewerContext | null = authority === "global" ? null : {
    session_id: "session", viewer_id: "viewer", binding_id: "bound-library-binding", tab_id: "tab", space_id: "space", kind: "files",
    source_kind: "context", source_id: library.root.root_id, roots: [library.root], default_root_id: library.root.root_id, diagnostics: [],
  };
  vi.stubGlobal("URL", { ...URL, createObjectURL: vi.fn(() => "blob:library-png"), revokeObjectURL: vi.fn() });
  function Harness() {
    const [view, setView] = useState(createContextViewState());
    return createElement(ContextViewer, { client, context, value: view, onChange: setView });
  }
  const flush = async () => { for (let index = 0; index < 8; index += 1) await settle(); };
  try {
    await act(async () => mounted.render(<Harness />));
    await flush();
    const row = host.querySelector<HTMLButtonElement>('[data-library-row="folder:notes"]')!;
    row.focus();
    await act(async () => row.dispatchEvent(new KeyboardEvent("keydown", { bubbles: true, cancelable: true, key: "p", ctrlKey: true })));
    await flush();
    const result = [...host.querySelectorAll<HTMLButtonElement>(".file-picker-results button")].find((button) => button.title.includes("docs/guide.md"));
    await act(async () => result!.click());
    await flush();
    if (authority === "bound") {
      expect(client.contextDocument).toHaveBeenCalledWith("session", "viewer", { binding_id: "bound-library-binding", root_id: library.root.root_id, path: "folders/Design notes/docs/guide.md", expected_revision: null }, expect.any(AbortSignal));
      expect(client.contextFileIndex).toHaveBeenCalledWith("session", "viewer", { binding_id: "bound-library-binding", root_id: library.root.root_id, mode: "cached" }, expect.any(AbortSignal));
      expect(client.libraryDocument).not.toHaveBeenCalled();
      expect(client.libraryFileIndex).not.toHaveBeenCalled();
      expect(client.commentBatch).toHaveBeenCalledWith("session", "viewer", { scope: { binding_id: "bound-library-binding", client_id: expect.any(String) }, batch_id: null });
      expect(host.querySelector<HTMLButtonElement>('[title="Comment on whole file (Shift+C)"]')?.disabled).toBe(false);
    } else {
      expect(client.libraryDocument).toHaveBeenCalledWith(expect.objectContaining({ path: "folders/Design notes/docs/guide.md" }), expect.any(AbortSignal));
      expect(client.contextDocument).not.toHaveBeenCalled();
      expect(client.contextFileIndex).not.toHaveBeenCalled();
      expect(client.commentBatch).not.toHaveBeenCalled();
      expect(host.querySelector(".context-comment-status")).toBeNull();
    }
    // The listing doesn't name this file, but it is part of the folder copy: it stays open.
    expect(host.textContent).toContain("Guide body");
    await act(async () => row.dispatchEvent(new KeyboardEvent("keydown", { bubbles: true, cancelable: true, key: "p", ctrlKey: true })));
    await flush();
    const imageResult = [...host.querySelectorAll<HTMLButtonElement>(".file-picker-results button")].find((button) => button.title.includes("palette.png"))!;
    await act(async () => imageResult.click());
    await flush();
    if (authority === "bound") {
      expect(client.contextMedia).toHaveBeenCalledWith("session", "viewer", { binding_id: "bound-library-binding", root_id: library.root.root_id, path: "folders/Design notes/palette.png", expected_revision: "r1" }, expect.any(AbortSignal));
      expect(client.libraryMedia).not.toHaveBeenCalled();
    } else {
      expect(client.libraryMedia).toHaveBeenCalledWith({ path: "folders/Design notes/palette.png", expected_revision: "r1" }, expect.any(AbortSignal));
      expect(client.contextMedia).not.toHaveBeenCalled();
      expect(client.commentBatch).not.toHaveBeenCalled();
    }
    expect(host.querySelector<HTMLImageElement>('img[alt="folders/Design notes/palette.png"]')?.src).toBe("blob:library-png");
  } finally {
    await act(async () => mounted.unmount());
    host.remove();
    vi.unstubAllGlobals();
  }
});

it("keeps an issued Library root's viewer authority and item selection through comment capture and saving", async () => {
  const host = document.createElement("div");
  document.body.append(host);
  const mounted = createRoot(host);
  const target = { session_id: "session", space_id: "space-1" };
  const item: LibraryItemSummary = {
    item_id: "source:ops-311", logical_id: "source:jira:ops-311", kind: "provider_snapshot", provider_id: "jira", provider_instance: "https://jira.test", resource_type: "issue",
    canonical_id: "OPS-311", container: { container_id: "OPS", label: "OPS" }, parent_item_id: null, ancestors: [], order: null, title: "Rotate signing keys",
    document_path: "jira/OPS-311/Rotate signing keys.md", item_path: "jira/OPS-311", source_url: null, original_url: null, source_revision: null, revision: "r1",
    state: "fresh", partial: null, conflict: [], fetched_at: null, checked_at: null, refs: [{ kind: "manual" }], purge_after: null, issue: null, attachments: [], folder: null, diagnostics: [],
  };
  const library: LibraryListing = {
    root: { root_id: "library:realpath-identity", kind: "library", label: "Library", path: "/real/data/library", repository_id: "", checkout_path: "" },
    generation: "1", items: [item], follows: [], next_offset: null, diagnostics: [],
  };
  let listing: SpaceContextListing = { target, space_label: "api-review", library_root: library.root.path, checkout_path: "/repo", items: [], repository_paths: [], diagnostics: [] };
  const selected: LibraryOperation = {
    operation_id: "op-header-select", kind: "space_add", phases: [{ phase: "space", state: "done", done: 1, total: 1, message: null, error: null }],
    item_ids: [item.item_id], report: null, space: { space_id: target.space_id, item_ids: [item.item_id] },
    target, cancel_requested: false, finished: true, created_at: "", updated_at: "",
  };
  let batch: CommentBatch = {
    batch_id: "bound-library-comments", generation: 1,
    owner: { kind: "viewer", session_id: target.session_id, server_instance: "server", tab_id: "tab", source_kind: "context", source_id: library.root.root_id },
    last_known_location: { workspace_id: target.space_id, tab_id: "tab" }, live_attachment: null, drafts: [], updated_at: "now",
  };
  const changes: ContextViewState[] = [];
  const client = {
    contextDirectory: vi.fn(),
    contextDocument: vi.fn(async (_session: string, _viewer: string, request: { binding_id: string; root_id: string; path: string }) => ({ ...request, revision: "r1", content_hash: "sha256:keys", bytes: 6, media_type: "text/markdown", text: "# Keys", truncated: false, diagnostics: [] })),
    contextFileIndex: vi.fn(async (_session: string, _viewer: string, request: { binding_id: string; root_id: string }): Promise<ContextFileIndex> => ({ ...request, files: [{ path: item.document_path!, bytes: 6 }], truncated: false, source: "walk", state: "cached", diagnostics: [] })),
    libraryListing: vi.fn(async () => library),
    projectConfiguration: vi.fn(async () => ({ providers: [{ id: "jira", kind: "jira" as const, base_url: "https://jira.test", deployment: "data_center" as const }] })),
    libraryDocument: vi.fn(),
    libraryFileIndex: vi.fn(),
    commentBatch: vi.fn(async () => batch),
    commentBatches: vi.fn(async (_session: string, _viewer: string, scope: { binding_id: string; client_id: string }) => ({
      attachment: { owner: batch.owner, location: batch.last_known_location, ...scope }, batches: [], truncated: false,
    })),
    commentUpsert: vi.fn(async (_session: string, _viewer: string, request: CommentUpsertRequest): Promise<CommentBatch> => {
      const capture = request.capture!;
      batch = { ...batch, generation: batch.generation + 1, drafts: [{
        draft_id: "library-note", file_ref: { root_id: capture.root_id, path: capture.path, absolute_path: `${library.root.path}/${capture.path}`, revision: capture.expected_revision, content_hash: "sha256:keys" },
        anchor: { kind: "whole_file" }, comment_text: request.comment_text, source_state: "current", updated_at: "now",
      }] };
      return batch;
    }),
    librarySpaceList: vi.fn(async () => listing),
    librarySpaceAdd: vi.fn(async () => { listing = { ...listing, items: [item] }; return selected; }),
    librarySpaceRemove: vi.fn(async () => { listing = { ...listing, items: [] }; return listing; }),
  } as unknown as CockpitClient;
  const context: ViewerContext = {
    session_id: target.session_id, viewer_id: "viewer", binding_id: "binding", tab_id: "tab", space_id: target.space_id,
    kind: "files", source_kind: "context", source_id: "source", default_root_id: library.root.root_id, roots: [library.root], diagnostics: [],
  };
  function Harness() {
    const [view, setView] = useState(createContextViewState());
    return <ContextViewer client={client} context={context} value={view} onChange={(next) => { changes.push(next); setView(next); }} space={{ target, label: "api-review", live: true }} />;
  }
  const flush = async () => { for (let index = 0; index < 8; index += 1) await settle(); };
  const headerButton = (label: string) => [...host.querySelectorAll<HTMLButtonElement>(".library-item-header button")].find((button) => button.textContent === label);
  try {
    await act(async () => mounted.render(<Harness />));
    await flush();
    await act(async () => host.querySelector<HTMLButtonElement>('[data-library-row="source:ops-311"]')!.click());
    await flush();
    expect(client.contextDocument).toHaveBeenCalledWith("session", "viewer", {
      binding_id: "binding", root_id: library.root.root_id, path: item.document_path, expected_revision: null,
    }, expect.any(AbortSignal));
    expect(client.contextDirectory).not.toHaveBeenCalled();
    expect(client.libraryDocument).not.toHaveBeenCalled();
    expect(client.contextFileIndex).toHaveBeenCalledWith("session", "viewer", { binding_id: "binding", root_id: library.root.root_id, mode: "fresh" }, expect.any(AbortSignal));
    expect(client.libraryFileIndex).not.toHaveBeenCalled();
    expect(client.commentBatch).toHaveBeenCalledWith("session", "viewer", {
      scope: { binding_id: "binding", client_id: expect.any(String) }, batch_id: null,
    });
    expect(changes.at(-1)?.rootId).toBe(library.root.root_id);
    expect(changes.at(-1)?.path).toBe(item.document_path);
    expect(host.querySelector(".library-kind-chip")?.textContent).toBe("Jira issue");
    expect(host.querySelector(".context-markdown-body")).not.toBeNull();
    // The detached batch belongs to the issued root, not context.source_id.
    // An enabled Reattach proves CommentDrafts retains that root as sourceIdentity.
    await act(async () => host.querySelector<HTMLButtonElement>('[title="Open comments overview"]')!.click());
    await flush();
    expect([...document.body.querySelectorAll<HTMLButtonElement>("button")].find((button) => button.textContent === "Reattach")?.disabled).toBe(false);
    await act(async () => document.body.querySelector<HTMLButtonElement>('[aria-label="Close comments overview"]')!.click());
    const wholeFile = host.querySelector<HTMLButtonElement>('[title="Comment on whole file (Shift+C)"]')!;
    expect(wholeFile.disabled).toBe(false);
    await act(async () => wholeFile.click());
    expect(changes.at(-1)?.commentEditor).toEqual(expect.objectContaining({ rootId: library.root.root_id, path: item.document_path, revision: "r1", editor: "whole_file" }));
    const textarea = document.body.querySelector<HTMLTextAreaElement>("textarea")!;
    await act(async () => {
      Object.getOwnPropertyDescriptor(HTMLTextAreaElement.prototype, "value")!.set!.call(textarea, "Verify key rotation before deployment.");
      textarea.dispatchEvent(new Event("input", { bubbles: true }));
    });
    await act(async () => [...document.body.querySelectorAll<HTMLButtonElement>("button")].find((button) => button.textContent === "Save comment")!.click());
    await flush();
    expect(client.commentUpsert).toHaveBeenCalledWith("session", "viewer", {
      batch: { scope: { binding_id: "binding", client_id: expect.any(String) }, batch_id: "bound-library-comments", expected_generation: 1 },
      draft_id: null, capture: { root_id: library.root.root_id, path: item.document_path, expected_revision: "r1", start_line: null, end_line: null },
      comment_text: "Verify key rotation before deployment.",
    });
    expect(changes.at(-1)?.commentEditor).toBeNull();
    expect(host.querySelector(".comment-file-drafts")?.textContent).toContain("Verify key rotation before deployment.");
    expect(host.querySelector(".library-item-header")?.textContent).toContain(item.title);
    expect(host.querySelector(".library-kind-chip")?.textContent).toBe("Jira issue");
    expect(host.querySelector(".context-markdown-body")).not.toBeNull();
    await act(async () => headerButton("Add to Space")!.click());
    await flush();
    expect(client.librarySpaceAdd).toHaveBeenCalledWith({ target, item_ids: [item.item_id] });
    expect(headerButton("Remove from Space")).toBeDefined();
    await act(async () => headerButton("Remove from Space")!.click());
    await flush();
    expect(client.librarySpaceRemove).toHaveBeenCalledWith({ target, item_ids: [item.item_id] });
    expect(headerButton("Add to Space")).toBeDefined();
    expect(host.textContent).toContain("Keys");
    expect(document.body.querySelector(".library-confirm")).toBeNull();
  } finally {
    await act(async () => mounted.unmount());
    host.remove();
  }
});


it("keeps the rendered Markdown while scrolling and records the position once scrolling settles", async () => {
  Object.assign(globalThis, { IS_REACT_ACT_ENVIRONMENT: true });
  const host = document.createElement("div");
  document.body.append(host);
  const mounted = createRoot(host);
  const text = "# Notes\n\nFirst paragraph.\n\n| a | b |\n| --- | --- |\n| 1 | 2 |\n";
  const client = {
    contextDirectory: vi.fn(async (): Promise<ContextDirectory> => ({
      binding_id: "binding", root_id: "folder", path: "", truncated: false, diagnostics: [],
      entries: [{ entry_id: "notes", name: "notes.md", path: "notes.md", kind: "file", bytes: text.length, revision: "r1", refusal: null }],
    })),
    contextDocument: vi.fn(async () => ({ binding_id: "binding", root_id: "folder", path: "notes.md", revision: "r1", content_hash: null, bytes: text.length, media_type: "text/markdown", text, truncated: false, offset: 0, next_offset: undefined, total_bytes: text.length, line_offset: 0, diagnostics: [] })),
  } as unknown as CockpitClient;
  const context: ViewerContext = { session_id: "session", viewer_id: "viewer", binding_id: "binding", tab_id: "tab", space_id: "space", kind: "files", source_kind: "context", source_id: "source", default_root_id: "folder", roots: [{ root_id: "folder", kind: "folder", label: "Folder", path: "/folder", repository_id: "folder", checkout_path: "/folder" }], diagnostics: [] }
  const changes: ContextViewState[] = [];
  function Harness() {
    const [view, setView] = useState(createContextViewState());
    return <ContextViewer client={client} context={context} value={view} onChange={(next) => { changes.push(next); setView(next); }} />;
  }
  try {
    await act(async () => mounted.render(<Harness />));
    await settle();
    await act(async () => host.querySelector<HTMLButtonElement>('[data-context-path="notes.md"]')?.click());
    await settle();
    const scroller = host.querySelector<HTMLElement>(".context-markdown-scroll")!;
    const paragraph = scroller.querySelector("p[data-source-start]");
    expect(paragraph?.textContent).toBe("First paragraph.");
    const before = changes.length;
    for (const top of [40, 80, 120]) {
      Object.defineProperty(scroller, "scrollTop", { configurable: true, value: top });
      await act(async () => { scroller.dispatchEvent(new Event("scroll")); });
    }
    expect(changes.length).toBe(before);
    await act(async () => { await new Promise<void>((resolve) => { setTimeout(resolve, 200); }); });
    expect(changes.length).toBe(before + 1);
    expect(changes.at(-1)?.files["folder\u0000notes.md"]?.scrollTop).toBe(120);
    expect(scroller.querySelector("p[data-source-start]")).toBe(paragraph);
  } finally {
    await act(async () => mounted.unmount());
    host.remove();
  }
});

it("reports an expired reader to its owning leaf without replacing the selected source state", async () => {
  Object.assign(globalThis, { IS_REACT_ACT_ENVIRONMENT: true });
  const host = document.createElement("div");
  document.body.append(host);
  const mounted = createRoot(host);
  const missing = new CockpitClientError("http_error", "Viewer expired", { operationCode: "viewer_not_found" });
  const context: ViewerContext = {
    session_id: "session", viewer_id: "viewer", binding_id: "binding", tab_id: "tab", space_id: "space",
    kind: "files", source_kind: "context", source_id: "source", default_root_id: "folder",
    roots: [{ root_id: "folder", kind: "folder", label: "Folder", path: "/folder", repository_id: "repo", checkout_path: "/folder" }],
    diagnostics: [],
  };
  const client = { contextDirectory: vi.fn().mockRejectedValue(missing) } as unknown as CockpitClient;
  const value: ContextViewState = { ...createContextViewState(), rootId: "folder" };
  const onChange = vi.fn();
  const onViewerError = vi.fn();
  try {
    await act(async () => mounted.render(<ContextViewer client={client} context={context} value={value} onChange={onChange} onViewerError={onViewerError} />));
    await settle();
    expect(onViewerError).toHaveBeenCalledWith(missing);
    expect(host.querySelector(".context-tree-error")?.textContent).toContain("Viewer expired");
    expect(onChange).not.toHaveBeenCalled();
    expect(client.contextDirectory).toHaveBeenCalledOnce();
  } finally {
    await act(async () => mounted.unmount());
    host.remove();
  }
});
