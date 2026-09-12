// @vitest-environment jsdom
import { act, createElement, useState } from "react";
import { createRoot } from "react-dom/client";
import { expect, it, vi } from "vitest";
import type { CockpitClient } from "../../client/CockpitClient";
import type { ContextDirectory, PanePresentation } from "../../protocol/generated/v1";
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
