// @vitest-environment jsdom
import "../input/viewerTestLayout";
import { act, createElement, useState } from "react";
import { createRoot } from "react-dom/client";
import { expect, it, vi } from "vitest";
import { CockpitClientError, type CockpitClient } from "../../client/CockpitClient";
import type { ContextDirectory, LibraryItemSummary, LibraryListing, LibraryOperation, PanePresentation, SpaceAddAttempt, SpaceContextListing, SpaceCopyRow, SpaceUpdateRequest } from "../../protocol/generated/v1";
import { ContextViewer, createContextViewState, SourceLines } from "./ContextViewer";

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
  const presentation = { session_id: "session", pane_id: "pane", binding_id: "binding", default_root_id: "folder", roots: [{ root_id: "folder", kind: "folder", label: "Folder", path: "/folder", repository_id: "folder", checkout_path: "/folder", companion_id: null }], diagnostics: [] } as unknown as PanePresentation;
  function Harness() {
    const [view, setView] = useState(createContextViewState());
    return createElement(ContextViewer, { client, presentation, value: view, onChange: setView, controlAllowed: true, onRequestControl: vi.fn(), onTerminalView: vi.fn() });
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
    expect(documentRead).toHaveBeenCalledWith("session", "pane", expect.objectContaining({ path: "src/deep/example.html" }), expect.any(AbortSignal));
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
  const presentation = { session_id: "session", pane_id: "pane", binding_id: "binding", default_root_id: "folder", roots: [{ root_id: "folder", kind: "folder", label: "Folder", path: "/folder", repository_id: "folder", checkout_path: "/folder", companion_id: null }], diagnostics: [] } as unknown as PanePresentation;
  function Harness() {
    const [view, setView] = useState(createContextViewState());
    return <ContextViewer client={client} presentation={presentation} value={view} onChange={setView} controlAllowed onRequestControl={vi.fn()} onTerminalView={vi.fn()} />;
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
    binding_id: "binding", root_id: "folder", path: request.path, truncated: false, diagnostics: [], entries: request.path === ""
      ? [{ entry_id: "nested", name: "nested", path: "nested", kind: "directory", bytes: null, revision: "r1", refusal: null }]
      : [{ entry_id: "target", name: "target.md", path: "nested/target.md", kind: "file", bytes: 12, revision: "r2", refusal: null }],
  }));
  const documentRead = vi.fn(async (_session: string, _pane: string, request: { path: string }) => ({ binding_id: "binding", root_id: "folder", path: request.path, revision: "r2", content_hash: null, bytes: 12, media_type: "text/markdown", text: "# Target", truncated: false, diagnostics: [] }));
  const client = { contextDirectory: directory, contextDocument: documentRead } as unknown as CockpitClient;
  const presentation = { session_id: "session", pane_id: "pane", binding_id: "binding", default_root_id: "folder", roots: [{ root_id: "folder", kind: "folder", label: "Folder", path: "/folder", repository_id: "folder", checkout_path: "/folder", companion_id: null }], diagnostics: [] } as unknown as PanePresentation;
  function Harness() {
    const [view, setView] = useState(createContextViewState());
    return <ContextViewer client={client} presentation={presentation} value={view} onChange={setView} controlAllowed onRequestControl={vi.fn()} onTerminalView={vi.fn()} />;
  }
  try {
    await act(async () => mounted.render(<Harness />));
    await settle();
    const treeFile = host.querySelector<HTMLButtonElement>("[data-context-path='nested']")!;
    treeFile.focus();
    await act(async () => treeFile.dispatchEvent(new KeyboardEvent("keydown", { bubbles: true, cancelable: true, key: "p", ctrlKey: true })));
    await settle(); await settle(); await settle();
    expect(directory.mock.calls.map((call) => call[2].path)).toContain("nested");
    const result = [...host.querySelectorAll<HTMLButtonElement>(".file-picker-results button")].find((button) => button.textContent?.includes("nested/target.md"));
    expect(result).toBeDefined();
    await act(async () => result?.click());
    await settle();
    expect(documentRead).toHaveBeenCalledWith("session", "pane", expect.objectContaining({ path: "nested/target.md" }), expect.any(AbortSignal));
    await act(async () => { await new Promise<void>(resolve => requestAnimationFrame(() => resolve())); });
    expect(document.activeElement).toBe(host.querySelector(".context-document"));
  } finally {
    await act(async () => mounted.unmount());
    host.remove();
  }
});

it("cancels recursive picker indexing when the picker is dismissed", async () => {
  const host = document.createElement("div");
  document.body.append(host);
  const mounted = createRoot(host);
  let requestCount = 0;
  let pickerSignal: AbortSignal | undefined;
  const directory = vi.fn((_session: string, _pane: string, request: { path: string }, signal?: AbortSignal): Promise<ContextDirectory> => {
    requestCount += 1;
    if (requestCount === 1) return Promise.resolve({ binding_id: "binding", root_id: "folder", path: request.path, truncated: false, diagnostics: [], entries: [{ entry_id: "first", name: "first.ts", path: "first.ts", kind: "file", bytes: 1, revision: "r1", refusal: null }] });
    pickerSignal = signal;
    return new Promise(() => undefined);
  });
  const client = { contextDirectory: directory } as unknown as CockpitClient;
  const presentation = { session_id: "session", pane_id: "pane", binding_id: "binding", default_root_id: "folder", roots: [{ root_id: "folder", kind: "folder", label: "Folder", path: "/folder", repository_id: "folder", checkout_path: "/folder", companion_id: null }], diagnostics: [] } as unknown as PanePresentation;
  function Harness() {
    const [view, setView] = useState(createContextViewState());
    return <ContextViewer client={client} presentation={presentation} value={view} onChange={setView} controlAllowed onRequestControl={vi.fn()} onTerminalView={vi.fn()} />;
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

it("caps picker results when one directory exceeds the picker file limit", async () => {
  const host = document.createElement("div");
  document.body.append(host);
  const mounted = createRoot(host);
  let requestCount = 0;
  const entries = Array.from({ length: 10_001 }, (_, index) => ({ entry_id: `entry-${index}`, name: `file-${index}.ts`, path: `file-${index}.ts`, kind: "file" as const, bytes: 1, revision: "r1", refusal: null }));
  const directory = vi.fn(async (_session: string, _pane: string, request: { path: string }): Promise<ContextDirectory> => ({ binding_id: "binding", root_id: "folder", path: request.path, truncated: false, diagnostics: [], entries: requestCount++ === 0 ? [] : entries }));
  const client = { contextDirectory: directory } as unknown as CockpitClient;
  const presentation = { session_id: "session", pane_id: "pane", binding_id: "binding", default_root_id: "folder", roots: [{ root_id: "folder", kind: "folder", label: "Folder", path: "/folder", repository_id: "folder", checkout_path: "/folder", companion_id: null }], diagnostics: [] } as unknown as PanePresentation;
  function Harness() {
    const [view, setView] = useState(createContextViewState());
    return <ContextViewer client={client} presentation={presentation} value={view} onChange={setView} controlAllowed onRequestControl={vi.fn()} onTerminalView={vi.fn()} />;
  }
  try {
    await act(async () => mounted.render(<Harness />));
    await settle();
    const viewer = host.querySelector<HTMLElement>(".context-viewer")!;
    await act(async () => viewer.dispatchEvent(new KeyboardEvent("keydown", { bubbles: true, cancelable: true, key: "p", ctrlKey: true })));
    await settle(); await settle(); await settle();
    expect(host.querySelector(".file-picker-status")?.textContent).toBe("10000 files · index incomplete");
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
  const presentation = { session_id: "session", pane_id: "pane", binding_id: "binding", default_root_id: "folder", roots: [{ root_id: "folder", kind: "folder", label: "Folder", path: "/folder", repository_id: "folder", checkout_path: "/folder", companion_id: null }], diagnostics: [] } as unknown as PanePresentation;
  vi.stubGlobal("URL", { ...URL, createObjectURL: vi.fn(() => "blob:folder-png"), revokeObjectURL: vi.fn() });
  function Harness() {
    const [view, setView] = useState(createContextViewState());
    return createElement(ContextViewer, { client, presentation, value: view, onChange: setView, controlAllowed: true, onRequestControl: vi.fn(), onTerminalView: vi.fn() });
  }
  try {
    await act(async () => mounted.render(<Harness />));
    await settle();
    const row = host.querySelector<HTMLButtonElement>('[data-context-path="palette.png"]')!;
    await act(async () => row.dispatchEvent(new KeyboardEvent("keydown", { bubbles: true, key: "Enter" })));
    await settle();
    await settle();
    expect(mediaRead).toHaveBeenCalledWith("session", "pane", { binding_id: "binding", root_id: "folder", path: "palette.png", expected_revision: "r1" }, expect.any(AbortSignal));
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
  const presentation = { session_id: "session", pane_id: "pane", binding_id: "binding", default_root_id: "folder", roots: [{ root_id: "folder", kind: "folder", label: "Folder", path: "/folder", repository_id: "folder", checkout_path: "/folder", companion_id: null }], diagnostics: [] } as unknown as PanePresentation;
  function Harness() {
    const [view, setView] = useState(createContextViewState());
    return <ContextViewer client={client} presentation={presentation} value={view} onChange={setView} controlAllowed onRequestControl={vi.fn()} onTerminalView={vi.fn()} />;
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
    expect(documentRead).toHaveBeenLastCalledWith("session", "pane", expect.objectContaining({ path: "large.txt", offset: 4, expected_revision: "r1" }), expect.any(AbortSignal));
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
  const presentation = { session_id: "session", pane_id: "pane", binding_id: "binding", default_root_id: "folder", roots: [{ root_id: "folder", kind: "folder", label: "Folder", path: "/folder", repository_id: "folder", checkout_path: "/folder", companion_id: null }], diagnostics: [] } as unknown as PanePresentation;
  function Harness() {
    const [view, setView] = useState(createContextViewState());
    return <ContextViewer client={client} presentation={presentation} value={view} onChange={setView} controlAllowed onRequestControl={vi.fn()} onTerminalView={vi.fn()} />;
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

it("explains an empty root instead of asking for a file selection", async () => {
  Object.assign(globalThis, { IS_REACT_ACT_ENVIRONMENT: true });
  const host = document.createElement("div");
  document.body.append(host);
  const mounted = createRoot(host);
  const directory = vi.fn(async (_session: string, _pane: string, request: { path: string }): Promise<ContextDirectory> => ({
    binding_id: "binding", root_id: "folder", path: request.path, truncated: false, diagnostics: [], entries: [],
  }));
  const client = { contextDirectory: directory } as unknown as CockpitClient;
  const presentation = { session_id: "session", pane_id: "pane", binding_id: "binding", default_root_id: "folder", roots: [{ root_id: "folder", kind: "folder", label: "Folder", path: "/folder", repository_id: "folder", checkout_path: "/folder", companion_id: null }], diagnostics: [] } as unknown as PanePresentation;
  function Harness() {
    const [view, setView] = useState(createContextViewState());
    return createElement(ContextViewer, { client, presentation, value: view, onChange: setView, controlAllowed: true, onRequestControl: vi.fn(), onTerminalView: vi.fn() });
  }
  try {
    await act(async () => mounted.render(<Harness />));
    await settle();
    expect(host.querySelector(".context-tree-status")?.textContent).toBe("Empty");
    expect(host.querySelector(".context-empty-message")?.textContent).toContain("This directory is empty.");
    expect(host.textContent).not.toContain("Select a file to inspect its source.");
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
    document_path: "gitlab/mr-482/document.md", item_path: "gitlab/mr-482", source_url: "https://gitlab.test/platform/api/-/merge_requests/482", original_url: null, source_revision: "abc", revision: "sha256:r1",
    state: "fresh", partial: null, conflict: [], fetched_at: null, checked_at: null, follow_id: null, attachments: [], folder: null, diagnostics: [],
  };
  const listing: LibraryListing = { root: { root_id: "library:fs", kind: "library", label: "Library", path: "/data/library", repository_id: "", checkout_path: "", companion_id: null }, generation: "1", items: [item], follows: [], next_offset: null, diagnostics: [] };
  const operation: LibraryOperation = { operation_id: "op-1", kind: "refresh", phases: [{ phase: "library", state: "done", done: 1, total: 1, message: null, error: null }], item_ids: ["source:mr"], report: { new: 0, updated: 1, unchanged: 0, removed_at_source: 0, partial: 0, failed: 0, conflict: 0, rows: [], truncated_rows: false }, space: null, target: null, cancel_requested: false, finished: true, created_at: "", updated_at: "" };
  let documentText = "# Fix it";
  let currentListing = listing;
  const client = {
    contextDirectory: vi.fn(async (): Promise<ContextDirectory> => ({ binding_id: "binding", root_id: "companion", path: "", truncated: false, diagnostics: [], entries: [] })),
    libraryListing: vi.fn(async () => currentListing),
    projectConfiguration: vi.fn(async () => ({ providers: [{ id: "gitlab", base_url: "https://gitlab.test", executable: "/usr/bin/glab" }] })),
    libraryDocument: vi.fn(async (request: { path: string }) => ({ binding_id: "library", root_id: "library:fs", path: request.path, revision: "r1", content_hash: null, bytes: 7, media_type: "text/markdown", text: documentText, truncated: false, diagnostics: [] })),
    libraryRefresh: vi.fn(async () => operation),
  } as unknown as CockpitClient;
  const presentation = { session_id: "session", pane_id: "pane", binding_id: "binding", default_root_id: "companion", roots: [{ root_id: "companion", kind: "companion", label: "Context", path: "/companion", repository_id: "repo", checkout_path: "/repo", companion_id: "c1" }], diagnostics: [] } as unknown as PanePresentation;
  function Harness() {
    const [view, setView] = useState(createContextViewState());
    return createElement(ContextViewer, { client, presentation, value: view, onChange: setView, controlAllowed: true, onRequestControl: vi.fn(), onTerminalView: vi.fn() });
  }
  const flush = async () => { for (let index = 0; index < 6; index += 1) await settle(); };
  const toolbarButton = (label: string) => [...host.querySelectorAll<HTMLButtonElement>(".context-toolbar button")].find((candidate) => candidate.textContent === label || candidate.getAttribute("aria-label") === label);
  try {
    await act(async () => mounted.render(<Harness />));
    await flush();
    expect(toolbarButton("Resources")).toBeDefined();
    expect(client.libraryListing).not.toHaveBeenCalled();

    const select = host.querySelector<HTMLSelectElement>(".context-root-select select")!;
    await act(async () => { select.value = "library"; select.dispatchEvent(new Event("change", { bubbles: true })); });
    await flush();
    expect(toolbarButton("Resources")).toBeUndefined();
    expect(toolbarButton("Add…")).toBeDefined();
    expect(toolbarButton("Refresh all")?.disabled).toBe(false);
    const row = host.querySelector<HTMLButtonElement>('[data-library-row="source:mr"]')!;
    expect(row.textContent).toContain("!482 Fix token refresh race");
    expect(host.querySelector('[data-library-row^="instance:"]')?.textContent).toContain("GitLab · gitlab.test");

    await act(async () => row.click());
    await flush();
    expect(client.libraryDocument).toHaveBeenCalledWith({ path: "gitlab/mr-482/document.md", expected_revision: null, offset: null }, expect.any(AbortSignal));
    expect(host.querySelector(".library-kind-chip")?.textContent).toBe("GitLab MR");

    const listingReads = vi.mocked(client.libraryListing).mock.calls.length;
    await act(async () => toolbarButton("Refresh Context files")!.click());
    await flush();
    expect(client.libraryRefresh).not.toHaveBeenCalled();
    expect(vi.mocked(client.libraryListing).mock.calls.length).toBeGreaterThan(listingReads);

    await act(async () => toolbarButton("Refresh all")!.click());
    await flush();
    expect(client.libraryRefresh).toHaveBeenCalledWith({ scope: "all" });
    expect(host.querySelector(".library-report")?.textContent).toContain("1 updated");
    documentText = "# Updated by another source";
    await act(async () => window.dispatchEvent(new Event("cockpit:library-changed")));
    await flush();
    expect(vi.mocked(client.libraryDocument).mock.calls.length).toBeGreaterThan(1);
    expect(host.textContent).toContain("Updated by another source");

    vi.mocked(client.libraryListing).mockRejectedValueOnce(new Error("listing offline"));
    await act(async () => toolbarButton("Refresh Context files")!.click());
    await flush();
    expect(host.querySelector('[role="alert"]')?.textContent).toContain("Library unavailable: listing offline. Space context is unaffected.");
    expect(host.querySelector(".context-tree-error button")?.textContent).toBe("Retry");
    expect(host.querySelector('[data-library-row="source:mr"]')).not.toBeNull();
    currentListing = { ...listing, generation: "2", items: [] };
    await act(async () => window.dispatchEvent(new Event("cockpit:library-changed")));
    await flush();
    expect(host.querySelector(".library-kind-chip")).toBeNull();
    expect(host.textContent).toContain("Jira issue, or a folder.");
  } finally {
    await act(async () => mounted.unmount());
    host.remove();
  }
});

it("lists this Space's Library context in Resources, failed adds first, and retries from the saved item into the companion", async () => {
  Object.assign(globalThis, { IS_REACT_ACT_ENVIRONMENT: true });
  const host = document.createElement("div");
  document.body.append(host);
  const mounted = createRoot(host);
  const target = { session_id: "session", space_id: "space-1" };
  const row = (title: string, state: SpaceCopyRow["state"]): SpaceCopyRow => ({
    item_id: `source:${title}`, logical_id: `logical:${title}`, title, provider_id: "github", resource_type: "issue", kind: "provider_snapshot", state, library_newer: state === "library_newer",
    paths: [`sources/github/issue/${title}.md`], edited: [], copy_mode: "reflink", library_revision_copied: "r1", current_library_revision: state === "library_newer" ? "r2" : "r1", follow: null,
  });
  const failed: SpaceAddAttempt = {
    target, space_label: null, item_id: "source:pr-7", follow_id: null, title: "Fix token refresh race", state: "failed",
    error: { code: "source_companion_unavailable", message: "Exactly one verified companion is required" }, operation_id: "op-failed", updated_at: "",
  };
  const companion = { status: "available" as const, companion_root_id: "companion:c1", companion_label: "Context" };
  let listing: SpaceContextListing = { target, companion, attempts: [failed], rows: [row("alpha", "up_to_date"), row("zeta", "library_newer")], behind: 1, diagnostics: [] };
  const copied: LibraryOperation = {
    operation_id: "op-resources-retry", kind: "space_add", phases: [{ phase: "space", state: "done", done: 1, total: 1, message: null, error: null }], item_ids: ["source:pr-7"],
    report: null, space: { space_id: "space-1", copy_mode: "reflink", written: ["sources/github/review/acme-api-7.md"], skipped_edited: [], companion_root_id: "companion:c1" },
    target, cancel_requested: false, finished: true, created_at: "", updated_at: "",
  };
  const client = {
    contextDirectory: vi.fn(async (_session: string, _pane: string, request: { root_id: string; path: string }): Promise<ContextDirectory> => ({ binding_id: "binding", root_id: request.root_id, path: request.path, truncated: false, diagnostics: [], entries: [] })),
    projectConfiguration: vi.fn(async () => ({ providers: [{ id: "github", base_url: "https://github.com", executable: "gh" }] })),
    repositories: vi.fn(async () => ({ repositories: [], diagnostics: [] })),
    librarySpaceList: vi.fn(async () => listing),
    librarySpaceAdd: vi.fn(async () => {
      listing = { ...listing, attempts: [], rows: [...listing.rows, { ...row("Fix token refresh race", "up_to_date"), item_id: "source:pr-7" }] };
      return copied;
    }),
  } as unknown as CockpitClient;
  const presentation = { session_id: "session", pane_id: "pane", binding_id: "binding", default_root_id: "companion:c1", roots: [{ root_id: "companion:c1", kind: "companion", label: "Context", path: "/companion", repository_id: "repo", checkout_path: "/repo", companion_id: "c1" }], diagnostics: [] } as unknown as PanePresentation;
  const space = { target, label: "api-review", live: true };
  function Harness() {
    const [view, setView] = useState(createContextViewState());
    return createElement(ContextViewer, { client, presentation, value: view, onChange: setView, controlAllowed: true, onRequestControl: vi.fn(), onTerminalView: vi.fn(), space });
  }
  const flush = async () => { for (let index = 0; index < 8; index += 1) await settle(); };
  try {
    await act(async () => mounted.render(<Harness />));
    await flush();
    expect(client.librarySpaceList).toHaveBeenCalledWith({ target }, expect.any(AbortSignal));
    const resources = [...host.querySelectorAll<HTMLButtonElement>(".context-toolbar button")].find((candidate) => candidate.textContent?.startsWith("Resources"))!;
    expect(resources.textContent).toBe("Resources · 1 behind");
    await act(async () => resources.click());
    await flush();
    const dialog = host.querySelector<HTMLElement>(".context-resources")!;
    expect(dialog.querySelector(".space-context-summary")?.textContent).toBe("In api-review · 2 items · 1 behind");
    expect([...dialog.querySelectorAll("[role='listitem'] .context-source-title")].map((title) => title.textContent)).toEqual(["Fix token refresh race", "zeta", "alpha"]);
    const attempt = dialog.querySelector<HTMLElement>("[role='listitem']")!;
    expect(attempt.textContent).toContain("✕ Not added — Retry");
    expect(attempt.querySelector("[role='alert']")?.textContent).toBe("Saved to the Library, but api-review's context folder couldn't be verified. Nothing was written to api-review.");
    expect(dialog.textContent).toContain("↑ Library newer");
    expect(dialog.textContent).toContain("✓ Up to date");

    const retry = [...attempt.querySelectorAll<HTMLButtonElement>("button")].find((button) => button.textContent === "Retry adding to api-review")!;
    retry.focus();
    await act(async () => retry.click());
    await flush();
    expect(client.librarySpaceAdd).toHaveBeenCalledWith({ target, item_ids: ["source:pr-7"], follow_ids: [] });
    const paths = vi.mocked(client.contextDirectory).mock.calls.map(([, , request]) => request.path);
    expect(paths).toEqual(expect.arrayContaining(["sources", "sources/github", "sources/github/review"]));
    expect(dialog.textContent).not.toContain("Not added");
    expect([...dialog.querySelectorAll("[role='listitem'] .context-source-title")].map((title) => title.textContent)).toEqual(["zeta", "alpha", "Fix token refresh race"]);
    expect(document.activeElement?.textContent).toBe("Add…");
  } finally {
    await act(async () => mounted.unmount());
    host.remove();
  }
});

function spaceResourcesFixture(initialRows: SpaceCopyRow[]) {
  const target = { session_id: "session", space_id: "space-1" };
  const companion = { status: "available" as const, companion_root_id: "companion:c1", companion_label: "Context" };
  const state = { rows: initialRows };
  const client = {
    contextDirectory: vi.fn(async (_session: string, _pane: string, request: { root_id: string; path: string }): Promise<ContextDirectory> => ({ binding_id: "binding", root_id: request.root_id, path: request.path, truncated: false, diagnostics: [], entries: [] })),
    projectConfiguration: vi.fn(async () => ({ providers: [{ id: "github", base_url: "https://github.com", executable: "gh" }] })),
    repositories: vi.fn(async () => ({ repositories: [], diagnostics: [] })),
    librarySpaceList: vi.fn(async (): Promise<SpaceContextListing> => ({ target, companion, attempts: [], rows: state.rows, behind: state.rows.filter((row) => row.library_newer).length, diagnostics: [] })),
    librarySpaceUpdate: vi.fn(),
    librarySpaceRemove: vi.fn(),
  } as unknown as CockpitClient;
  const presentation = { session_id: "session", pane_id: "pane", binding_id: "binding", default_root_id: "companion:c1", roots: [{ root_id: "companion:c1", kind: "companion", label: "Context", path: "/companion", repository_id: "repo", checkout_path: "/repo", companion_id: "c1" }], diagnostics: [] } as unknown as PanePresentation;
  const space = { target, label: "api-review", live: true };
  function Harness() {
    const [view, setView] = useState(createContextViewState());
    return createElement(ContextViewer, { client, presentation, value: view, onChange: setView, controlAllowed: true, onRequestControl: vi.fn(), onTerminalView: vi.fn(), space });
  }
  return { target, state, client, Harness };
}

function spaceRow(title: string, state: SpaceCopyRow["state"], overrides: Partial<SpaceCopyRow> = {}): SpaceCopyRow {
  return {
    item_id: `source:${title}`, logical_id: `logical:${title}`, title, provider_id: "github", resource_type: "issue", kind: "provider_snapshot", state, library_newer: state === "library_newer",
    paths: [`sources/github/issue/${title}.md`], edited: [], copy_mode: "reflink", library_revision_copied: "r1", current_library_revision: "r1", follow: null, ...overrides,
  };
}

function spaceUpdated(operationId: string, target: { session_id: string; space_id: string }, itemIds: string[], written: string[], skipped: string[] = []): LibraryOperation {
  return {
    operation_id: operationId, kind: "space_update", phases: [{ phase: "space", state: "done", done: 1, total: 1, message: null, error: null }], item_ids: itemIds,
    report: null, space: { space_id: "space-1", copy_mode: "reflink", written, skipped_edited: skipped, companion_root_id: "companion:c1" },
    target, cancel_requested: false, finished: true, created_at: "", updated_at: "",
  };
}

it("updates only the selected copy in this Space, counts Update all from behind and missing copies, reports skipped edits, and never offers Update on kept copies", async () => {
  Object.assign(globalThis, { IS_REACT_ACT_ENVIRONMENT: true });
  const host = document.createElement("div");
  document.body.append(host);
  const mounted = createRoot(host);
  const edited = { path: "sources/github/issue/delta.md", current_hash: "sha256:edited" };
  const fixture = spaceResourcesFixture([
    spaceRow("alpha", "library_newer"),
    spaceRow("beta", "library_newer"),
    spaceRow("delta", "edited_in_space", { library_newer: true, edited: [edited] }),
    spaceRow("gamma", "missing_in_space"),
    spaceRow("kept", "removed_at_source"),
    spaceRow("orphan", "not_in_library", { item_id: null }),
    spaceRow("legacy", "not_linked", { item_id: null }),
    spaceRow("current", "up_to_date"),
  ]);
  const { target, state, client, Harness } = fixture;
  const flush = async () => { for (let index = 0; index < 8; index += 1) await settle(); };
  const actions = (title: string) => {
    const entry = [...host.querySelectorAll<HTMLElement>(".context-resources [role='listitem']")].find((candidate) => candidate.querySelector(".context-source-title")?.textContent === title)!;
    return [...entry.querySelectorAll<HTMLButtonElement>(".space-context-actions button")];
  };
  const updateAll = () => [...host.querySelectorAll<HTMLButtonElement>(".space-context-bar button")].find((button) => button.textContent?.startsWith("Update all"))!;
  try {
    await act(async () => mounted.render(<Harness />));
    await flush();
    const resources = [...host.querySelectorAll<HTMLButtonElement>(".context-toolbar button")].find((candidate) => candidate.textContent?.startsWith("Resources"))!;
    await act(async () => resources.click());
    await flush();
    // Rows needing action first, by title; then up-to-date and unlinked copies.
    expect([...host.querySelectorAll(".context-resources [role='listitem'] .context-source-title")].map((title) => title.textContent))
      .toEqual(["alpha", "beta", "delta", "gamma", "kept", "orphan", "current", "legacy"]);
    expect(updateAll().textContent).toBe("Update all (3)");
    expect(actions("alpha").map((button) => button.textContent)).toEqual(["Update"]);
    expect(actions("gamma").map((button) => button.textContent)).toEqual(["Restore from Library"]);
    expect(actions("delta").map((button) => button.textContent)).toEqual(["Replace with Library version…"]);
    expect(actions("kept").map((button) => button.textContent)).toEqual(["Remove from this Space…"]);
    expect(actions("orphan").map((button) => button.textContent)).toEqual(["Remove from this Space…"]);
    expect(actions("legacy")).toEqual([]);
    expect(actions("current").map((button) => button.textContent)).toEqual(["Remove from this Space…"]);

    vi.mocked(client.librarySpaceUpdate).mockImplementationOnce(async () => {
      state.rows = state.rows.map((row): SpaceCopyRow => row.title === "alpha" ? { ...row, state: "up_to_date", library_newer: false } : row);
      return spaceUpdated("op-s3-selected", target, ["source:alpha"], ["sources/github/issue/alpha.md"]);
    });
    const update = actions("alpha")[0];
    update.focus();
    await act(async () => update.click());
    await flush();
    expect(client.librarySpaceUpdate).toHaveBeenCalledWith({ target, scope: { scope: "selection", item_ids: ["source:alpha"], follow_ids: [] }, replace_edited: [] });
    // The reread listing, not the click, decides the row: alpha is current, beta is still behind, and the row kept its place.
    expect(host.querySelector(".context-resources [role='listitem']")?.textContent).toContain("✓ Up to date");
    expect(actions("beta").map((button) => button.textContent)).toEqual(["Update"]);
    expect(updateAll().textContent).toBe("Update all (2)");
    expect(document.activeElement).toBe(actions("alpha")[0]);
    expect(document.activeElement?.textContent).toBe("Remove from this Space…");

    vi.mocked(client.librarySpaceUpdate).mockImplementationOnce(async () => {
      state.rows = state.rows.map((row): SpaceCopyRow => row.title === "beta" || row.title === "gamma" ? { ...row, state: "up_to_date", library_newer: false } : row);
      return spaceUpdated("op-s3-all", target, ["source:beta", "source:delta", "source:gamma"], ["sources/github/issue/beta.md", "sources/github/issue/gamma.md"], [edited.path]);
    });
    await act(async () => updateAll().click());
    await flush();
    expect(client.librarySpaceUpdate).toHaveBeenLastCalledWith({ target, scope: { scope: "all" }, replace_edited: [] });
    expect(host.querySelector(".space-context [role='status']")?.textContent).toBe("Updated 2 items in api-review. Skipped 1 edited copy.");
    // Only the edited copy is left: N stays 0, but Update all still runs so it can report the skipped edit.
    expect(updateAll().textContent).toBe("Update all (0)");
    expect(updateAll().getAttribute("aria-disabled")).toBe("false");
    vi.mocked(client.librarySpaceUpdate).mockImplementationOnce(async () => spaceUpdated("op-s3-all-edited", target, ["source:delta"], [], [edited.path]));
    await act(async () => updateAll().click());
    await flush();
    expect(client.librarySpaceUpdate).toHaveBeenCalledTimes(3);
    expect(client.librarySpaceUpdate).toHaveBeenLastCalledWith({ target, scope: { scope: "all" }, replace_edited: [] });
    expect(host.querySelector(".space-context [role='status']")?.textContent).toBe("Nothing updated in api-review. Skipped 1 edited copy.");
    expect(host.querySelector(".context-resources")?.textContent).toContain("✎ Edited in Space · Library newer");
  } finally {
    await act(async () => mounted.unmount());
    host.remove();
  }
});

it("replaces an edited copy only after confirmation with the listed hashes, keeps it on a conflict, and removes a copy from this Space", async () => {
  Object.assign(globalThis, { IS_REACT_ACT_ENVIRONMENT: true });
  const host = document.createElement("div");
  document.body.append(host);
  const mounted = createRoot(host);
  const path = "sources/github/issue/delta.md";
  const { target, state, client, Harness } = spaceResourcesFixture([
    spaceRow("delta", "edited_in_space", { library_newer: true, edited: [{ path, current_hash: "sha256:first" }], paths: [path] }),
    spaceRow("kept", "removed_at_source"),
  ]);
  const flush = async () => { for (let index = 0; index < 8; index += 1) await settle(); };
  const entry = (title: string) => [...host.querySelectorAll<HTMLElement>(".context-resources [role='listitem']")].find((candidate) => candidate.querySelector(".context-source-title")?.textContent === title);
  const button = (root: ParentNode | undefined, label: string) => [...(root?.querySelectorAll<HTMLButtonElement>("button") ?? [])].find((candidate) => candidate.textContent === label);
  const confirmDialog = () => document.body.querySelector<HTMLElement>(".library-confirm");
  try {
    await act(async () => mounted.render(<Harness />));
    await flush();
    await act(async () => [...host.querySelectorAll<HTMLButtonElement>(".context-toolbar button")].find((candidate) => candidate.textContent?.startsWith("Resources"))!.click());
    await flush();

    const replace = button(entry("delta"), "Replace with Library version…")!;
    replace.focus();
    await act(async () => replace.click());
    expect(confirmDialog()?.querySelector("h2")?.textContent).toBe('Replace your edited copy of "delta"?');
    expect(confirmDialog()?.textContent).toContain(path);
    expect(document.activeElement?.textContent).toBe("Keep my copy");
    await act(async () => button(confirmDialog()!, "Keep my copy")!.click());
    expect(confirmDialog()).toBeNull();
    expect(client.librarySpaceUpdate).not.toHaveBeenCalled();
    expect(document.activeElement).toBe(replace);

    // The copy was edited again after the listing was read: the confirmed hash no longer matches.
    vi.mocked(client.librarySpaceUpdate).mockImplementationOnce(async () => {
      state.rows = state.rows.map((row) => row.title === "delta" ? { ...row, edited: [{ path, current_hash: "sha256:second" }] } : row);
      throw new CockpitClientError("http_error", "The Space copy changed", { status: 409, operationCode: "space_copy_conflict" });
    });
    const listReads = vi.mocked(client.librarySpaceList).mock.calls.length;
    await act(async () => replace.click());
    await act(async () => button(confirmDialog()!, "Replace with Library version")!.click());
    await flush();
    const firstReplace: SpaceUpdateRequest = { target, scope: { scope: "selection", item_ids: ["source:delta"], follow_ids: [] }, replace_edited: [{ path, current_hash: "sha256:first" }] };
    expect(client.librarySpaceUpdate).toHaveBeenCalledWith(firstReplace);
    expect(confirmDialog()).toBeNull();
    expect(vi.mocked(client.librarySpaceList).mock.calls.length).toBeGreaterThan(listReads);
    expect(entry("delta")?.querySelector("[role='alert']")?.textContent).toBe("api-review's copy changed since it was checked, so nothing was changed. Review it and try again.");
    expect(entry("delta")?.textContent).toContain("✎ Edited in Space · Library newer");

    // Retrying confirms the reread hash, never the stale one.
    vi.mocked(client.librarySpaceUpdate).mockImplementationOnce(async () => {
      state.rows = state.rows.map((row): SpaceCopyRow => row.title === "delta" ? { ...row, state: "up_to_date", library_newer: false, edited: [] } : row);
      return spaceUpdated("op-s3-replace", target, ["source:delta"], [path]);
    });
    await act(async () => button(entry("delta"), "Replace with Library version…")!.click());
    await act(async () => button(confirmDialog()!, "Replace with Library version")!.click());
    await flush();
    expect(client.librarySpaceUpdate).toHaveBeenLastCalledWith({ ...firstReplace, replace_edited: [{ path, current_hash: "sha256:second" }] });
    expect(entry("delta")?.textContent).toContain("✓ Up to date");
    expect(entry("delta")?.querySelector("[role='alert']")).toBeNull();

    const remove = button(entry("kept"), "Remove from this Space…")!;
    remove.focus();
    await act(async () => remove.click());
    expect(confirmDialog()?.querySelector("h2")?.textContent).toBe('Remove "kept" from api-review?');
    expect(confirmDialog()?.textContent).toContain("Deletes api-review's copy. The Library item stays.");
    expect(document.activeElement?.textContent).toBe("Cancel");
    vi.mocked(client.librarySpaceRemove).mockImplementationOnce(async () => {
      state.rows = state.rows.filter((row) => row.title !== "kept");
      return { target, companion: { status: "available", companion_root_id: "companion:c1", companion_label: "Context" }, attempts: [], rows: state.rows, behind: 0, diagnostics: [] };
    });
    await act(async () => button(confirmDialog()!, "Remove from api-review")!.click());
    await flush();
    expect(client.librarySpaceRemove).toHaveBeenCalledWith({ target, logical_id: "logical:kept", confirmed: [] });
    expect(entry("kept")).toBeUndefined();
    expect(document.activeElement?.textContent).toBe("Add…");
    // Nothing behind, missing or edited is left: Update all has nothing to do.
    const updateAll = [...host.querySelectorAll<HTMLButtonElement>(".space-context-bar button")].find((candidate) => candidate.textContent?.startsWith("Update all"))!;
    expect(updateAll.getAttribute("aria-disabled")).toBe("true");
    await act(async () => updateAll.click());
    expect(client.librarySpaceUpdate).toHaveBeenCalledTimes(2);
  } finally {
    await act(async () => mounted.unmount());
    host.remove();
  }
});

it("keeps Tab and Shift+Tab inside a Space copy confirmation opened over Resources, and Escape still cancels it", async () => {
  Object.assign(globalThis, { IS_REACT_ACT_ENVIRONMENT: true });
  const host = document.createElement("div");
  document.body.append(host);
  const mounted = createRoot(host);
  const path = "sources/github/issue/delta.md";
  const { client, Harness } = spaceResourcesFixture([spaceRow("delta", "edited_in_space", { library_newer: true, edited: [{ path, current_hash: "sha256:first" }], paths: [path] })]);
  const flush = async () => { for (let index = 0; index < 8; index += 1) await settle(); };
  const confirmDialog = () => document.body.querySelector<HTMLElement>(".library-confirm");
  const press = async (target: Element, key: string, shiftKey = false) => {
    await act(async () => target.dispatchEvent(new KeyboardEvent("keydown", { bubbles: true, cancelable: true, key, shiftKey })));
  };
  try {
    await act(async () => mounted.render(<Harness />));
    await flush();
    await act(async () => [...host.querySelectorAll<HTMLButtonElement>(".context-toolbar button")].find((candidate) => candidate.textContent?.startsWith("Resources"))!.click());
    await flush();
    const replace = [...host.querySelectorAll<HTMLButtonElement>(".context-resources button")].find((candidate) => candidate.textContent === "Replace with Library version…")!;
    replace.focus();
    await act(async () => replace.click());
    expect(document.activeElement?.textContent).toBe("Keep my copy");
    const buttons = [...confirmDialog()!.querySelectorAll<HTMLButtonElement>("button")];
    const first = buttons[0];
    const last = buttons.at(-1)!;
    first.focus();
    await press(first, "Tab", true);
    expect(document.activeElement).toBe(last);
    expect(confirmDialog()?.contains(document.activeElement)).toBe(true);
    await press(last, "Tab");
    expect(document.activeElement).toBe(first);
    await press(first, "Escape");
    expect(confirmDialog()).toBeNull();
    expect(host.querySelector(".context-resources")).not.toBeNull();
    expect(document.activeElement).toBe(replace);
    expect(client.librarySpaceUpdate).not.toHaveBeenCalled();
  } finally {
    await act(async () => mounted.unmount());
    host.remove();
  }
});

it("holds an update's result and its conflicting actions until the Space is reread, and offers a reread when that fails", async () => {
  Object.assign(globalThis, { IS_REACT_ACT_ENVIRONMENT: true });
  const host = document.createElement("div");
  document.body.append(host);
  const mounted = createRoot(host);
  const edited = { path: "sources/github/issue/delta.md", current_hash: "sha256:edited" };
  const { target, state, client, Harness } = spaceResourcesFixture([
    spaceRow("alpha", "library_newer"),
    spaceRow("delta", "edited_in_space", { library_newer: true, edited: [edited], paths: [edited.path] }),
  ]);
  const flush = async () => { for (let index = 0; index < 8; index += 1) await settle(); };
  const updateAll = () => [...host.querySelectorAll<HTMLButtonElement>(".space-context-bar button")].find((button) => button.textContent?.startsWith("Update all"))!;
  const report = () => host.querySelector(".space-context > p[role='status']");
  const listingError = () => host.querySelector<HTMLElement>(".space-context > .context-resource-error");
  try {
    await act(async () => mounted.render(<Harness />));
    await flush();
    await act(async () => [...host.querySelectorAll<HTMLButtonElement>(".context-toolbar button")].find((candidate) => candidate.textContent?.startsWith("Resources"))!.click());
    await flush();
    vi.mocked(client.librarySpaceUpdate).mockImplementationOnce(async () => {
      state.rows = state.rows.map((row): SpaceCopyRow => row.title === "alpha" ? { ...row, state: "up_to_date", library_newer: false } : row);
      return spaceUpdated("op-s3-unconfirmed", target, ["source:alpha", "source:delta"], ["sources/github/issue/alpha.md"], [edited.path]);
    });
    vi.mocked(client.librarySpaceList).mockRejectedValueOnce(new Error("Space offline"));
    await act(async () => updateAll().click());
    await flush();
    expect(client.librarySpaceUpdate).toHaveBeenCalledTimes(1);
    // The update finished but nothing confirmed it: no result, no spinner, and Update all stays held.
    expect(host.textContent).not.toMatch(/Updated \d/);
    expect(report()).toBeNull();
    expect(host.querySelector(".context-resources .library-spinner")).toBeNull();
    expect(listingError()?.querySelector("strong")?.textContent).toBe("The update finished, but api-review's copies couldn't be reread, so its result isn't shown yet.");
    expect(listingError()?.textContent).toContain("Space offline");
    expect(updateAll().getAttribute("aria-disabled")).toBe("true");
    const alphaUpdate = [...host.querySelectorAll<HTMLButtonElement>(".context-resources [role='listitem'] .space-context-actions button")].find((button) => button.textContent === "Update")!;
    expect(alphaUpdate.getAttribute("aria-disabled")).toBe("true");
    await act(async () => updateAll().click());
    expect(client.librarySpaceUpdate).toHaveBeenCalledTimes(1);

    await act(async () => [...listingError()!.querySelectorAll<HTMLButtonElement>("button")].find((button) => button.textContent === "Retry")!.click());
    await flush();
    expect(listingError()).toBeNull();
    expect(report()?.textContent).toBe("Updated 1 item in api-review. Skipped 1 edited copy.");
    expect(updateAll().getAttribute("aria-disabled")).toBe("false");
  } finally {
    await act(async () => mounted.unmount());
    host.remove();
  }
});

it("keeps an Update all report as it finished after the skipped copy is replaced and then removed", async () => {
  Object.assign(globalThis, { IS_REACT_ACT_ENVIRONMENT: true });
  const host = document.createElement("div");
  document.body.append(host);
  const mounted = createRoot(host);
  const edited = { path: "sources/github/issue/delta.md", current_hash: "sha256:edited" };
  const { target, state, client, Harness } = spaceResourcesFixture([
    spaceRow("alpha", "library_newer"),
    spaceRow("delta", "edited_in_space", { library_newer: true, edited: [edited], paths: [edited.path] }),
  ]);
  const flush = async () => { for (let index = 0; index < 8; index += 1) await settle(); };
  const entry = (title: string) => [...host.querySelectorAll<HTMLElement>(".context-resources [role='listitem']")].find((candidate) => candidate.querySelector(".context-source-title")?.textContent === title);
  const button = (root: ParentNode | undefined, label: string) => [...(root?.querySelectorAll<HTMLButtonElement>("button") ?? [])].find((candidate) => candidate.textContent === label);
  const confirmDialog = () => document.body.querySelector<HTMLElement>(".library-confirm");
  const report = () => host.querySelector(".space-context > p[role='status']")?.textContent;
  try {
    await act(async () => mounted.render(<Harness />));
    await flush();
    await act(async () => [...host.querySelectorAll<HTMLButtonElement>(".context-toolbar button")].find((candidate) => candidate.textContent?.startsWith("Resources"))!.click());
    await flush();
    vi.mocked(client.librarySpaceUpdate).mockImplementationOnce(async () => {
      state.rows = state.rows.map((row): SpaceCopyRow => row.title === "alpha" ? { ...row, state: "up_to_date", library_newer: false } : row);
      return spaceUpdated("op-s3-all-history", target, ["source:alpha", "source:delta"], ["sources/github/issue/alpha.md"], [edited.path]);
    });
    await act(async () => button(host.querySelector(".space-context-bar") ?? undefined, "Update all (1)")!.click());
    await flush();
    expect(report()).toBe("Updated 1 item in api-review. Skipped 1 edited copy.");

    vi.mocked(client.librarySpaceUpdate).mockImplementationOnce(async () => {
      state.rows = state.rows.map((row): SpaceCopyRow => row.title === "delta" ? { ...row, state: "up_to_date", library_newer: false, edited: [] } : row);
      return spaceUpdated("op-s3-replace-history", target, ["source:delta"], [edited.path]);
    });
    await act(async () => button(entry("delta"), "Replace with Library version…")!.click());
    await act(async () => button(confirmDialog()!, "Replace with Library version")!.click());
    await flush();
    expect(entry("delta")?.textContent).toContain("✓ Up to date");
    expect(report()).toBe("Updated 1 item in api-review. Skipped 1 edited copy.");

    vi.mocked(client.librarySpaceRemove).mockImplementationOnce(async () => {
      state.rows = state.rows.filter((row) => row.title !== "delta");
      return { target, companion: { status: "available", companion_root_id: "companion:c1", companion_label: "Context" }, attempts: [], rows: state.rows, behind: 0, diagnostics: [] };
    });
    await act(async () => button(entry("delta"), "Remove from this Space…")!.click());
    await act(async () => button(confirmDialog()!, "Remove from api-review")!.click());
    await flush();
    expect(entry("delta")).toBeUndefined();
    expect(report()).toBe("Updated 1 item in api-review. Skipped 1 edited copy.");
  } finally {
    await act(async () => mounted.unmount());
    host.remove();
  }
});

it("adds a Library item to the Space and rereads its standing after provider refresh without updating its copy", async () => {
  Object.assign(globalThis, { IS_REACT_ACT_ENVIRONMENT: true });
  const host = document.createElement("div");
  document.body.append(host);
  const mounted = createRoot(host);
  const target = { session_id: "session", space_id: "space-1" };
  const item: LibraryItemSummary = {
    item_id: "source:ops-311", logical_id: "source:jira:ops-311", kind: "provider_snapshot", provider_id: "jira", provider_instance: "https://jira.test", resource_type: "issue",
    canonical_id: "OPS-311", container: { container_id: "OPS", label: "OPS" }, parent_item_id: null, ancestors: [], order: null, title: "Rotate signing keys",
    document_path: "jira/ops-311/document.md", item_path: "jira/ops-311", source_url: null, original_url: null, source_revision: null, revision: "sha256:r1",
    state: "fresh", partial: null, conflict: [], fetched_at: null, checked_at: null, follow_id: null, attachments: [], folder: null, diagnostics: [],
  };
  const library: LibraryListing = { root: { root_id: "library:fs", kind: "library", label: "Library", path: "/data/library", repository_id: "", checkout_path: "", companion_id: null }, generation: "1", items: [item], follows: [], next_offset: null, diagnostics: [] };
  const companion = { status: "available" as const, companion_root_id: "companion:c1", companion_label: "Context" };
  let rows: SpaceCopyRow[] = [];
  const copied: LibraryOperation = {
    operation_id: "op-header-add", kind: "space_add", phases: [{ phase: "space", state: "done", done: 1, total: 1, message: null, error: null }], item_ids: ["source:ops-311"],
    report: null, space: { space_id: "space-1", copy_mode: "copy", written: ["sources/jira/issue/ops-311.md"], skipped_edited: [], companion_root_id: "companion:c1" },
    target, cancel_requested: false, finished: true, created_at: "", updated_at: "",
  };
  const client = {
    libraryListing: vi.fn(async () => library),
    projectConfiguration: vi.fn(async () => ({ providers: [{ id: "jira", base_url: "https://jira.test", executable: "jira" }] })),
    libraryDocument: vi.fn(async (request: { path: string }) => ({ binding_id: "library", root_id: "library:fs", path: request.path, revision: library.items[0].revision, content_hash: null, bytes: 7, media_type: "text/markdown", text: library.generation === "1" ? "# Keys" : "# Refreshed keys", truncated: false, diagnostics: [] })),
    librarySpaceList: vi.fn(async (): Promise<SpaceContextListing> => ({ target, companion, attempts: [], rows, behind: rows.filter((row) => row.library_newer).length, diagnostics: [] })),
    librarySpaceAdd: vi.fn(async () => {
      rows = [{ item_id: "source:ops-311", logical_id: "source:jira:ops-311", title: "Rotate signing keys", provider_id: "jira", resource_type: "issue", kind: "provider_snapshot", state: "up_to_date", library_newer: false, paths: ["sources/jira/issue/ops-311.md"], edited: [], copy_mode: "copy", library_revision_copied: "sha256:r1", current_library_revision: "sha256:r1", follow: null }];
      return copied;
    }),
    libraryRefresh: vi.fn(async (): Promise<LibraryOperation> => ({
      ...copied, operation_id: "op-header-source-refresh", kind: "refresh", target: null, space: null, finished: false,
      phases: [{ phase: "library", state: "running", done: 0, total: 1, message: null, error: null }],
    })),
    libraryOperation: vi.fn(async (): Promise<LibraryOperation> => {
      library.generation = "2";
      library.items = [{ ...item, revision: "sha256:r2" }];
      rows = rows.map((row) => ({ ...row, state: "library_newer", library_newer: true, current_library_revision: "sha256:r2" }));
      return {
        ...copied, operation_id: "op-header-source-refresh", kind: "refresh", target: null, space: null,
        phases: [{ phase: "library", state: "done", done: 1, total: 1, message: null, error: null }],
      };
    }),
    librarySpaceUpdate: vi.fn(),
  } as unknown as CockpitClient;
  function Harness() {
    const [view, setView] = useState(createContextViewState());
    return createElement(ContextViewer, { client, presentation: null, value: view, onChange: setView, controlAllowed: true, onRequestControl: vi.fn(), space: { target, label: "api-review", live: true } });
  }
  const flush = async () => { for (let index = 0; index < 8; index += 1) await settle(); };
  try {
    await act(async () => mounted.render(<Harness />));
    await flush();
    await act(async () => host.querySelector<HTMLButtonElement>('[data-library-row="source:ops-311"]')!.click());
    await flush();
    const header = host.querySelector<HTMLElement>(".library-item-header")!;
    const add = [...header.querySelectorAll<HTMLButtonElement>("button")].find((button) => button.textContent === "Add to api-review")!;
    const documentReads = vi.mocked(client.libraryDocument).mock.calls.length;
    add.focus();
    await act(async () => add.click());
    await flush();
    expect(client.librarySpaceAdd).toHaveBeenCalledWith({ target, item_ids: ["source:ops-311"], follow_ids: [] });
    // Copying into the Space leaves the Library item alone: the open document and its header stay mounted.
    expect(vi.mocked(client.libraryDocument).mock.calls.length).toBe(documentReads);
    expect(header.isConnected).toBe(true);
    expect(header.querySelector(".library-space-state")?.textContent).toBe("In api-review · ✓ Up to date");
    expect([...header.querySelectorAll("button")].map((button) => button.textContent)).not.toContain("Add to api-review");
    expect(document.activeElement).toBe(header.querySelector(".library-space-slot"));

    vi.useFakeTimers();
    await act(async () => [...header.querySelectorAll<HTMLButtonElement>("button")].find((button) => button.textContent === "Refresh")!.click());
    await act(async () => { await vi.advanceTimersByTimeAsync(750); });
    await flush();
    expect(client.libraryRefresh).toHaveBeenCalledWith({ scope: "items", item_ids: [item.item_id] });
    expect(host.textContent).toContain("Refreshed keys");
    expect(host.querySelector(".library-space-state")?.textContent).toBe("In api-review · ↑ Library newer");
    expect(client.librarySpaceAdd).toHaveBeenCalledTimes(1);
    expect(client.librarySpaceUpdate).not.toHaveBeenCalled();
  } finally {
    await act(async () => mounted.unmount());
    host.remove();
    vi.useRealTimers();
  }
});

it("rereads the Space's copies when Context files are refreshed and when Resources reopens", async () => {
  Object.assign(globalThis, { IS_REACT_ACT_ENVIRONMENT: true });
  const host = document.createElement("div");
  document.body.append(host);
  const mounted = createRoot(host);
  const target = { session_id: "session", space_id: "space-1" };
  const companion = { status: "available" as const, companion_root_id: "companion:c1", companion_label: "Context" };
  const copy = (state: SpaceCopyRow["state"], libraryNewer = false): SpaceCopyRow => ({
    item_id: "source:ops-311", logical_id: "source:jira:ops-311", title: "Rotate signing keys", provider_id: "jira", resource_type: "issue", kind: "provider_snapshot", state, library_newer: libraryNewer,
    paths: ["sources/jira/issue/ops-311.md"], edited: [], copy_mode: "copy", library_revision_copied: "r1", current_library_revision: libraryNewer ? "r2" : "r1", follow: null,
  });
  let listing: SpaceContextListing = { target, companion, attempts: [], rows: [copy("up_to_date")], behind: 0, diagnostics: [] };
  const client = {
    contextDirectory: vi.fn(async (_session: string, _pane: string, request: { root_id: string; path: string }): Promise<ContextDirectory> => ({ binding_id: "binding", root_id: request.root_id, path: request.path, truncated: false, diagnostics: [], entries: [] })),
    projectConfiguration: vi.fn(async () => ({ providers: [{ id: "jira", base_url: "https://jira.test", executable: "jira" }] })),
    repositories: vi.fn(async () => ({ repositories: [], diagnostics: [] })),
    librarySpaceList: vi.fn(async () => ({ ...listing })),
  } as unknown as CockpitClient;
  const presentation = { session_id: "session", pane_id: "pane", binding_id: "binding", default_root_id: "companion:c1", roots: [{ root_id: "companion:c1", kind: "companion", label: "Context", path: "/companion", repository_id: "repo", checkout_path: "/repo", companion_id: "c1" }], diagnostics: [] } as unknown as PanePresentation;
  function Harness() {
    const [view, setView] = useState(createContextViewState());
    return createElement(ContextViewer, { client, presentation, value: view, onChange: setView, controlAllowed: true, onRequestControl: vi.fn(), onTerminalView: vi.fn(), space: { target, label: "api-review", live: true } });
  }
  const flush = async () => { for (let index = 0; index < 8; index += 1) await settle(); };
  const toolbarButton = (label: string) => [...host.querySelectorAll<HTMLButtonElement>(".context-toolbar button")].find((candidate) => candidate.textContent?.startsWith(label) || candidate.getAttribute("aria-label") === label);
  const openResources = async () => { await act(async () => toolbarButton("Resources")!.click()); await flush(); };
  const closeResources = async () => { await act(async () => host.querySelector<HTMLButtonElement>(".context-resources-close")!.click()); await flush(); };
  try {
    await act(async () => mounted.render(<Harness />));
    await flush();
    expect(toolbarButton("Resources")?.textContent).toBe("Resources");

    // The user edits the copy outside Cockpit, then refreshes the files.
    listing = { ...listing, rows: [copy("edited_in_space", true)], behind: 1 };
    await act(async () => toolbarButton("Refresh Context files")!.click());
    await flush();
    expect(toolbarButton("Resources")?.textContent).toBe("Resources · 1 behind");
    await openResources();
    expect(host.querySelector(".context-resources")?.textContent).toContain("✎ Edited in Space · Library newer");
    await closeResources();

    // Deleted on disk while Resources is closed: reopening shows it without another refresh.
    listing = { ...listing, rows: [copy("missing_in_space")], behind: 0 };
    await openResources();
    const resources = host.querySelector(".context-resources")!;
    expect(resources.textContent).toContain("○ Missing in Space");
    expect(resources.textContent).not.toContain("Edited in Space");
  } finally {
    await act(async () => mounted.unmount());
    host.remove();
  }
});

it("keeps a failed Add to <Space> visible with its retry when no durable attempt records it", async () => {
  Object.assign(globalThis, { IS_REACT_ACT_ENVIRONMENT: true });
  const host = document.createElement("div");
  document.body.append(host);
  const mounted = createRoot(host);
  const target = { session_id: "session", space_id: "space-1" };
  const item: LibraryItemSummary = {
    item_id: "source:ops-311", logical_id: "source:jira:ops-311", kind: "provider_snapshot", provider_id: "jira", provider_instance: "https://jira.test", resource_type: "issue",
    canonical_id: "OPS-311", container: { container_id: "OPS", label: "OPS" }, parent_item_id: null, ancestors: [], order: null, title: "Rotate signing keys",
    document_path: "jira/ops-311/document.md", item_path: "jira/ops-311", source_url: null, original_url: null, source_revision: null, revision: "sha256:r1",
    state: "fresh", partial: null, conflict: [], fetched_at: null, checked_at: null, follow_id: null, attachments: [], folder: null, diagnostics: [],
  };
  const library: LibraryListing = { root: { root_id: "library:fs", kind: "library", label: "Library", path: "/data/library", repository_id: "", checkout_path: "", companion_id: null }, generation: "1", items: [item], follows: [], next_offset: null, diagnostics: [] };
  // The copy stopped before its attempt was written: the reread listing has no attempt for it.
  const stopped: LibraryOperation = {
    operation_id: "op-header-stopped", kind: "space_add", item_ids: [], report: null, space: null, target, cancel_requested: false, finished: true, created_at: "", updated_at: "",
    phases: [{ phase: "space", state: "failed", done: 0, total: 1, message: null, error: { code: "library_item_busy", message: "Too many pending Space adds" } }],
  };
  const client = {
    libraryListing: vi.fn(async () => library),
    projectConfiguration: vi.fn(async () => ({ providers: [{ id: "jira", base_url: "https://jira.test", executable: "jira" }] })),
    libraryDocument: vi.fn(async (request: { path: string }) => ({ binding_id: "library", root_id: "library:fs", path: request.path, revision: "r1", content_hash: null, bytes: 7, media_type: "text/markdown", text: "# Keys", truncated: false, diagnostics: [] })),
    librarySpaceList: vi.fn(async (): Promise<SpaceContextListing> => ({ target, companion: { status: "available", companion_root_id: "companion:c1", companion_label: "Context" }, attempts: [], rows: [], behind: 0, diagnostics: [] })),
    librarySpaceAdd: vi.fn(async () => stopped),
  } as unknown as CockpitClient;
  function Harness() {
    const [view, setView] = useState(createContextViewState());
    return createElement(ContextViewer, { client, presentation: null, value: view, onChange: setView, controlAllowed: true, onRequestControl: vi.fn(), space: { target, label: "api-review", live: true } });
  }
  const flush = async () => { for (let index = 0; index < 8; index += 1) await settle(); };
  const headerButton = (label: string) => [...host.querySelectorAll<HTMLButtonElement>(".library-item-header button")].find((button) => button.textContent === label);
  try {
    await act(async () => mounted.render(<Harness />));
    await flush();
    await act(async () => host.querySelector<HTMLButtonElement>('[data-library-row="source:ops-311"]')!.click());
    await flush();
    const listReads = vi.mocked(client.librarySpaceList).mock.calls.length;
    await act(async () => headerButton("Add to api-review")!.click());
    await flush();
    expect(vi.mocked(client.librarySpaceList).mock.calls.length).toBeGreaterThan(listReads);
    const notice = host.querySelector(".library-item-header [role='alert']");
    expect(notice?.textContent).toContain("Saved to the Library, but not added to api-review. Too many pending Space adds");
    expect(headerButton("Add to api-review")).toBeUndefined();
    await act(async () => headerButton("Retry adding to api-review")!.click());
    await flush();
    expect(client.librarySpaceAdd).toHaveBeenCalledTimes(2);
    expect(client.librarySpaceAdd).toHaveBeenLastCalledWith({ target, item_ids: ["source:ops-311"], follow_ids: [] });
  } finally {
    await act(async () => mounted.unmount());
    host.remove();
  }
});

it("clears a header's failed Add to <Space> once another surface copies the item into that Space", async () => {
  Object.assign(globalThis, { IS_REACT_ACT_ENVIRONMENT: true });
  const host = document.createElement("div");
  document.body.append(host);
  const mounted = createRoot(host);
  const target = { session_id: "session", space_id: "space-1" };
  const item: LibraryItemSummary = {
    item_id: "source:ops-311", logical_id: "source:jira:ops-311", kind: "provider_snapshot", provider_id: "jira", provider_instance: "https://jira.test", resource_type: "issue",
    canonical_id: "OPS-311", container: { container_id: "OPS", label: "OPS" }, parent_item_id: null, ancestors: [], order: null, title: "Rotate signing keys",
    document_path: "jira/ops-311/document.md", item_path: "jira/ops-311", source_url: null, original_url: null, source_revision: null, revision: "sha256:r1",
    state: "fresh", partial: null, conflict: [], fetched_at: null, checked_at: null, follow_id: null, attachments: [], folder: null, diagnostics: [],
  };
  const library: LibraryListing = { root: { root_id: "library:fs", kind: "library", label: "Library", path: "/data/library", repository_id: "", checkout_path: "", companion_id: null }, generation: "1", items: [item], follows: [], next_offset: null, diagnostics: [] };
  const stopped: LibraryOperation = {
    operation_id: "op-header-stopped-then-added", kind: "space_add", item_ids: [], report: null, space: null, target, cancel_requested: false, finished: true, created_at: "", updated_at: "",
    phases: [{ phase: "space", state: "failed", done: 0, total: 1, message: null, error: { code: "library_item_busy", message: "Too many pending Space adds" } }],
  };
  let rows: SpaceCopyRow[] = [];
  const client = {
    libraryListing: vi.fn(async () => library),
    projectConfiguration: vi.fn(async () => ({ providers: [{ id: "jira", base_url: "https://jira.test", executable: "jira" }] })),
    libraryDocument: vi.fn(async (request: { path: string }) => ({ binding_id: "library", root_id: "library:fs", path: request.path, revision: "r1", content_hash: null, bytes: 7, media_type: "text/markdown", text: "# Keys", truncated: false, diagnostics: [] })),
    librarySpaceList: vi.fn(async (): Promise<SpaceContextListing> => ({ target, companion: { status: "available", companion_root_id: "companion:c1", companion_label: "Context" }, attempts: [], rows, behind: 0, diagnostics: [] })),
    librarySpaceAdd: vi.fn(async () => stopped),
  } as unknown as CockpitClient;
  function Harness() {
    const [view, setView] = useState(createContextViewState());
    return createElement(ContextViewer, { client, presentation: null, value: view, onChange: setView, controlAllowed: true, onRequestControl: vi.fn(), space: { target, label: "api-review", live: true } });
  }
  const flush = async () => { for (let index = 0; index < 8; index += 1) await settle(); };
  const headerButton = (label: string) => [...host.querySelectorAll<HTMLButtonElement>(".library-item-header button")].find((button) => button.textContent === label);
  try {
    await act(async () => mounted.render(<Harness />));
    await flush();
    await act(async () => host.querySelector<HTMLButtonElement>('[data-library-row="source:ops-311"]')!.click());
    await flush();
    await act(async () => headerButton("Add to api-review")!.click());
    await flush();
    expect(headerButton("Retry adding to api-review")).toBeDefined();

    // The Add dialog (another surface) copies the item; its finished operation rereads the Space.
    rows = [{ item_id: "source:ops-311", logical_id: "source:jira:ops-311", title: "Rotate signing keys", provider_id: "jira", resource_type: "issue", kind: "provider_snapshot", state: "up_to_date", library_newer: false, paths: ["sources/jira/issue/ops-311.md"], edited: [], copy_mode: "copy", library_revision_copied: "sha256:r1", current_library_revision: "sha256:r1", follow: null }];
    const addedElsewhere: LibraryOperation = { ...stopped, operation_id: "op-added-elsewhere", item_ids: ["source:ops-311"], phases: [{ phase: "space", state: "done", done: 1, total: 1, message: null, error: null }], space: { space_id: "space-1", copy_mode: "copy", written: ["sources/jira/issue/ops-311.md"], skipped_edited: [], companion_root_id: "companion:c1" } };
    await act(async () => window.dispatchEvent(new CustomEvent("cockpit:library-changed", { detail: addedElsewhere })));
    await flush();
    const header = host.querySelector<HTMLElement>(".library-item-header")!;
    expect(header.querySelector(".library-space-state")?.textContent).toBe("In api-review · ✓ Up to date");
    expect(header.querySelector("[role='alert']")).toBeNull();
    expect(headerButton("Retry adding to api-review")).toBeUndefined();
    expect(header.textContent).not.toContain("Too many pending Space adds");
  } finally {
    await act(async () => mounted.unmount());
    host.remove();
  }
});
