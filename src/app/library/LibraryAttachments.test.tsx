// @vitest-environment jsdom
import "../input/viewerTestLayout";
import { act, useState } from "react";
import { createRoot } from "react-dom/client";
import { expect, it, vi } from "vitest";
import type { CockpitClient } from "../../client/CockpitClient";
import type { LibraryItemSummary, LibraryListing, LibraryOperation } from "../../protocol/generated/v1";
import { ContextViewer, createContextViewState, type ContextViewState } from "../context/ContextViewer";
import { LibraryItemHeader } from "./LibraryItemHeader";

Object.assign(globalThis, { IS_REACT_ACT_ENVIRONMENT: true });
const page: LibraryItemSummary = {
  item_id: "page:s7", logical_id: "page:s7", kind: "provider_snapshot", provider_id: "confluence", provider_instance: "https://wiki.test", resource_type: "page", canonical_id: "7",
  container: { container_id: "SD", label: "SD" }, parent_item_id: null, ancestors: [], order: null, title: "Release", document_path: "confluence/wiki.test/SD - Software Development/Release/Release.md", item_path: "confluence/wiki.test/SD - Software Development/Release", source_url: null, original_url: null, source_revision: "1", revision: "r1", state: "fresh", partial: null, conflict: [], fetched_at: null, checked_at: null, refs: [{ kind: "manual" }], purge_after: null, issue: null, folder: null, diagnostics: [],
  attachments: [
    { attachment_id: "png", original_name: "../flow.png", stored_name: "flow.png", media_type: "image/png", bytes: 68, version: "1", state: "not_downloaded", relative_path: null },
    { attachment_id: "pdf", original_name: "report.pdf", stored_name: "report.pdf", media_type: "application/pdf", bytes: 100, version: "1", state: "failed", relative_path: null },
    { attachment_id: "large", original_name: "large.bin", stored_name: "large.bin", media_type: null, bytes: 100_000_000, version: "1", state: "over_limit", relative_path: null },
  ],
};
const providers = [{ id: "confluence", kind: "confluence" as const, base_url: "https://wiki.test", deployment: "data_center" as const }];
async function flush() { for (let i = 0; i < 8; i++) await act(async () => { await Promise.resolve(); }); }

it("downloads only chosen rows, keeps over-limit rows inert, and removes only downloaded bytes", async () => {
  const host = document.createElement("div"); document.body.append(host);
  const root = createRoot(host);
  const attachments = { start: vi.fn(), open: vi.fn(), busy: false, active: null };
  const actions = { open: vi.fn(), refresh: vi.fn(), remove: vi.fn(), copyLink: vi.fn(), canCopyLink: false, refreshBusy: false, attachments };
  const render = (item: LibraryItemSummary) => root.render(<LibraryItemHeader item={item} providers={providers} narrow={false} rootCrumb pending={false} actions={actions} onReplace={vi.fn()} details={null} />);
  const button = (name: string) => [...host.querySelectorAll<HTMLButtonElement>("button")].find((node) => node.textContent === name)!;
  try {
    await act(async () => render(page));
    // Bulk actions live in the panel header: folded, the state line offers only the toggle.
    expect(host.querySelectorAll(".library-attachments-toggle")).toHaveLength(1);
    expect(button("Download all")).toBeUndefined();
    expect(button("Remove downloaded")).toBeUndefined();
    await act(async () => host.querySelector<HTMLButtonElement>(".library-attachments-toggle")!.click());
    expect(attachments.start).not.toHaveBeenCalled();
    expect(host.querySelector(".library-attachments-head")?.contains(button("Download all"))).toBe(true);
    await act(async () => button("Download all").click());
    expect(attachments.start).toHaveBeenLastCalledWith(page, "download", ["png", "pdf"]);
    attachments.start.mockClear();
    expect(host.textContent).toContain("not downloaded: over limit");
    expect(host.querySelector('[aria-label="Select large.bin"]')).toBeNull();
    await act(async () => host.querySelector<HTMLInputElement>('[aria-label="Select flow.png"]')!.click());
    await act(async () => button("Download selected (1)").click());
    expect(attachments.start).toHaveBeenLastCalledWith(page, "download", ["png"]);
    await act(async () => button("Download all").click());
    expect(attachments.start).toHaveBeenLastCalledWith(page, "download", ["png", "pdf"]);
    const downloaded = { ...page, attachments: page.attachments.map((attachment) => attachment.attachment_id === "png" ? { ...attachment, state: "downloaded" as const, relative_path: "_files/flow.png" } : attachment) };
    await act(async () => render(downloaded));
    await act(async () => button("Remove downloaded").click());
    expect(attachments.start).toHaveBeenLastCalledWith(downloaded, "remove_downloaded", ["png"]);
    expect(host.querySelector('[title="../flow.png"]')?.textContent).toBe("flow.png");
  } finally { await act(async () => root.unmount()); host.remove(); }
});

it("lists a Jira issue's attachments read-only, with no download controls", async () => {
  const host = document.createElement("div"); document.body.append(host);
  const root = createRoot(host);
  const attachments = { start: vi.fn(), open: vi.fn(), busy: false, active: null };
  const actions = { open: vi.fn(), refresh: vi.fn(), remove: vi.fn(), copyLink: vi.fn(), canCopyLink: false, refreshBusy: false, attachments };
  const issue: LibraryItemSummary = { ...page, item_id: "issue:ops1", provider_id: "jira", provider_instance: "https://jira.test", resource_type: "issue", canonical_id: "OPS-1", container: { container_id: "OPS", label: "OPS" }, title: "Crash", document_path: "jira/jira.test/OPS/OPS-1/Crash.md", item_path: "jira/jira.test/OPS/OPS-1", attachments: [{ attachment_id: "10100", original_name: "trace.log", stored_name: "trace.log", media_type: "text/plain", bytes: 2048, version: null, state: "not_downloaded", relative_path: null }] };
  try {
    await act(async () => root.render(<LibraryItemHeader item={issue} providers={[{ id: "jira", kind: "jira" as const, base_url: "https://jira.test", deployment: "data_center" as const }]} narrow={false} rootCrumb pending={false} actions={actions} onReplace={vi.fn()} details={null} />));
    await act(async () => host.querySelector<HTMLButtonElement>(".library-attachments-toggle")!.click());
    expect(host.querySelector('[aria-label="Attachments"]')?.textContent).toContain("trace.log");
    expect(host.querySelector(".library-attachments")?.textContent).toContain("2 KB");
    expect([...host.querySelectorAll(".library-attachments button")]).toHaveLength(0);
    expect(host.querySelector('[aria-label="Select trace.log"]')).toBeNull();
  } finally { await act(async () => root.unmount()); host.remove(); }
});

it.each(["loading", "needs_token", "stored"] as const)("gates a Jira issue's attachment downloads on the stored token (%s)", async (access) => {
  const host = document.createElement("div"); document.body.append(host);
  const root = createRoot(host);
  const attachments = { start: vi.fn(), open: vi.fn(), busy: false, active: null };
  const credentials = { statuses: null, ensure: vi.fn(), open: vi.fn(), attachmentAccess: vi.fn(() => access) };
  const actions = { open: vi.fn(), refresh: vi.fn(), remove: vi.fn(), copyLink: vi.fn(), canCopyLink: false, refreshBusy: false, attachments, credentials };
  const issue: LibraryItemSummary = { ...page, item_id: "issue:ops1", provider_id: "jira", provider_instance: "https://jira.test", resource_type: "issue", canonical_id: "OPS-1", container: { container_id: "OPS", label: "OPS" }, title: "Crash", document_path: "jira/jira.test/OPS/OPS-1/Crash.md", item_path: "jira/jira.test/OPS/OPS-1", attachments: [{ attachment_id: "10100", original_name: "trace.log", stored_name: "trace.log", media_type: "text/plain", bytes: 2048, version: null, state: "not_downloaded", relative_path: null }] };
  const button = (name: string) => [...host.querySelectorAll<HTMLButtonElement>(".library-attachments button")].find((node) => node.textContent === name);
  try {
    await act(async () => root.render(<LibraryItemHeader item={issue} providers={[{ id: "jira", kind: "jira" as const, base_url: "https://jira.test", deployment: "data_center" as const }]} narrow={false} rootCrumb pending={false} actions={actions} onReplace={vi.fn()} details={null} />));
    // Token states are read once the item's attachments are in view, only while they are unknown.
    expect(credentials.ensure).toHaveBeenCalledTimes(access === "loading" ? 1 : 0);
    await act(async () => host.querySelector<HTMLButtonElement>(".library-attachments-toggle")!.click());
    if (access === "stored") {
      expect(host.querySelector(".library-attachments-head")?.textContent).toContain("0 of 1 downloaded");
      await act(async () => button("Download all")!.click());
      expect(attachments.start).toHaveBeenCalledWith(issue, "download", ["10100"]);
    } else {
      expect(button("Download all")).toBeUndefined();
      expect(host.querySelector('[aria-label="Select trace.log"]')).toBeNull();
    }
    // Until token state is known, the head claims nothing: no "not downloaded" line that a stored token would contradict.
    if (access === "loading") expect(host.querySelector(".library-attachments-head")?.textContent).toBe("");
    if (access === "needs_token") {
      await act(async () => button("Provider token…")!.click());
      expect(credentials.open).toHaveBeenCalledWith("jira");
    } else {
      expect(button("Provider token…")).toBeUndefined();
    }
  } finally { await act(async () => root.unmount()); host.remove(); }
});

it.each([
  { access: "needs_token" as const, provider: "jira" as const, cue: true },
  { access: "stored" as const, provider: "jira" as const, cue: false },
  { access: null, provider: "confluence" as const, cue: false },
  { access: null, provider: "gitlab" as const, cue: false },
])("opens credentials only from blocked attachment prompts for $provider (token $access)", async ({ access, provider, cue }) => {
  const host = document.createElement("div"); document.body.append(host);
  const root = createRoot(host);
  const credentials = { statuses: null, ensure: vi.fn(), open: vi.fn(), attachmentAccess: vi.fn(() => access) };
  const actions = { open: vi.fn(), refresh: vi.fn(), remove: vi.fn(), copyLink: vi.fn(), canCopyLink: false, refreshBusy: false, attachments: { start: vi.fn(), open: vi.fn(), busy: false, active: null }, credentials };
  const item: LibraryItemSummary = { ...page, item_id: `${provider}:1`, provider_id: provider, provider_instance: "https://x.test", resource_type: "issue", canonical_id: "OPS-1", container: null, title: "Crash", attachments: [{ ...page.attachments[0]! }] };
  try {
    await act(async () => root.render(<LibraryItemHeader item={item} providers={[{ id: provider, kind: provider, base_url: "https://x.test", ...(provider === "gitlab" ? { executable: "glab" } : { deployment: "data_center" as const }) }]} narrow={false} rootCrumb pending={false} actions={actions} onReplace={vi.fn()} details={null} />));
    const cueButton = host.querySelector<HTMLButtonElement>(".library-token-cue button");
    expect(cueButton !== null).toBe(cue);
    if (cueButton) { await act(async () => cueButton.click()); expect(credentials.open).toHaveBeenCalledWith(provider); }
    await act(async () => host.querySelector<HTMLButtonElement>("button.library-more")!.click());
    const entries = [...document.body.querySelectorAll<HTMLButtonElement>('[role="menuitem"]')].filter((node) => node.textContent === "Provider token…");
    expect(entries).toHaveLength(cue ? 1 : 0);
    if (cue) {
      credentials.open.mockClear();
      await act(async () => entries[0]!.click());
      expect(credentials.open).toHaveBeenCalledWith(provider);
      expect(actions.attachments.start).not.toHaveBeenCalled();
    }
  } finally { await act(async () => root.unmount()); host.remove(); }
});

it("keeps metadata-only selection local, polls an explicit download, then opens only saved bytes through SafeImage", async () => {
  const host = document.createElement("div"); document.body.append(host);
  const root = createRoot(host);
  let item = page;
  const listing = (): LibraryListing => ({ root: { root_id: "library:fs", kind: "library", label: "Library", path: "/library", repository_id: "", checkout_path: "" }, generation: item.revision, items: [item], follows: [], next_offset: null, diagnostics: [] });
  const operation: LibraryOperation = { operation_id: "s7-ui-download", kind: "attachments", item_ids: [page.item_id], phases: [{ phase: "library", state: "running", done: 0, total: 1, message: null, error: null }], report: null, space: null, target: null, cancel_requested: false, finished: false, created_at: "", updated_at: "" };
  const client = {
    projectConfiguration: vi.fn(async () => ({ providers })), libraryListing: vi.fn(async () => listing()),
    libraryDocument: vi.fn(async (request: { path: string }) => ({ binding_id: "library", root_id: "library:fs", path: request.path, revision: "r2", content_hash: null, bytes: 68, media_type: "application/octet-stream", text: null, truncated: false, diagnostics: [] })),
    libraryMedia: vi.fn(async (request: { path: string }) => ({ binding_id: "library", root_id: "library:fs", path: request.path, revision: "r2", content_hash: null, bytes: 68, mime_type: "image/png", width: 1, height: 1, data_base64: "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVQIHWP4z8DwHwAFgAI/ScLq9wAAAABJRU5ErkJggg==" })),
    libraryAttachments: vi.fn(async () => operation),
    libraryOperation: vi.fn(async () => {
      item = { ...page, revision: "r2", attachments: page.attachments.map((attachment) => attachment.attachment_id === "png" ? { ...attachment, state: "downloaded", relative_path: "_files/flow.png" } : attachment) };
      return { ...operation, finished: true, phases: [{ phase: "library", state: "done", done: 1, total: 1, message: null, error: null }] };
    }),
  } as unknown as CockpitClient;
  function Harness() { const [view, onChange] = useState(createContextViewState()); return <ContextViewer client={client} context={null} value={view} onChange={onChange} />; }
  vi.stubGlobal("URL", { ...URL, createObjectURL: vi.fn(() => "blob:s7-png"), revokeObjectURL: vi.fn() });
  try {
    await act(async () => root.render(<Harness />)); await flush();
    await act(async () => host.querySelector<HTMLButtonElement>(".library-tree-group.is-attachments")!.click());
    await act(async () => host.querySelector<HTMLButtonElement>('[data-library-row="png"]')!.click()); await flush();
    expect(host.textContent).toContain("Not downloaded. 68 bytes · image/png · version 1.");
    expect(client.libraryMedia).not.toHaveBeenCalled();
    expect(client.libraryAttachments).not.toHaveBeenCalled();
    vi.useFakeTimers();
    await act(async () => host.querySelector<HTMLButtonElement>(".library-attachment-notice button")!.click());
    expect(client.libraryAttachments).toHaveBeenCalledWith({ item_id: page.item_id, attachment_ids: ["png"], action: "download" });
    await act(async () => { await vi.advanceTimersByTimeAsync(750); }); await flush();
    expect(host.textContent).toContain("Downloaded 1 of 1 attachment.");
    await act(async () => host.querySelector<HTMLButtonElement>('[data-library-row="png"]')!.click()); await flush();
    expect(client.libraryMedia).toHaveBeenCalledWith({ path: "confluence/wiki.test/SD - Software Development/Release/_files/flow.png", expected_revision: "r2" }, expect.any(AbortSignal));
    expect(host.querySelector<HTMLImageElement>(".context-raster-preview img")?.src).toBe("blob:s7-png");
  } finally { await act(async () => root.unmount()); host.remove(); vi.useRealTimers(); vi.unstubAllGlobals(); }
});

it.each(["library", "folder"] as const)("resolves a page-relative image through the %s reader with the shared attachment layout", async (kind) => {
  const host = document.createElement("div"); document.body.append(host);
  const mounted = createRoot(host);
  const rootId = kind === "library" ? "library" : "folder";
  const serverRootId = kind === "library" ? "library:fs" : rootId;
  const prefix = kind === "library" ? page.item_path : "confluence/wiki.test/SD - Software Development/Release";
  const path = `${prefix}/Release.md`;
  const root = { root_id: serverRootId, kind, label: kind, path: `/${kind}`, repository_id: "", checkout_path: "" };
  const documentRead = async () => ({ binding_id: "binding", root_id: serverRootId, path, revision: "r1", content_hash: null, bytes: 100, media_type: "text/markdown", text: "# Release\n\n![Flow](_files/flow.png)", truncated: false, diagnostics: [] });
  const media = { binding_id: "binding", root_id: serverRootId, path: `${prefix}/_files/flow.png`, revision: "r1", content_hash: null, bytes: 68, mime_type: "image/png", width: 1, height: 1, data_base64: "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVQIHWP4z8DwHwAFgAI/ScLq9wAAAABJRU5ErkJggg==" };
  const client = {
    projectConfiguration: vi.fn(async () => ({ providers })),
    libraryListing: vi.fn(async () => ({ root: { root_id: "library:fs", kind: "library", label: "Library", path: "/library", repository_id: "", checkout_path: "" }, generation: "1", items: [page], follows: [], next_offset: null, diagnostics: [] })),
    libraryDocument: vi.fn(documentRead), contextDocument: vi.fn(documentRead),
    contextDirectory: vi.fn(async (_session: string, _viewer: string, request: { path: string }) => ({ binding_id: "binding", root_id: rootId, path: request.path, entries: [], truncated: false, diagnostics: [] })),
    libraryMedia: vi.fn(async () => media), contextMedia: vi.fn(async () => media),
  } as unknown as CockpitClient;
  function Harness() {
    const [view, onChange] = useState<ContextViewState>(() => ({ ...createContextViewState(), rootId, path, files: { [`${rootId}\u0000${path}`]: { rootId, path, mode: "auto", selectionStart: null, selectionEnd: null, scrollTop: 0 } } }));
    return <ContextViewer client={client} context={kind === "library" ? null : { session_id: "session", viewer_id: "viewer", binding_id: "binding", tab_id: "tab", space_id: "space", kind: "files", source_kind: "context", source_id: "folder-source", default_root_id: rootId, roots: [root], diagnostics: [] }} value={view} onChange={onChange} />;
  }
  vi.stubGlobal("URL", { ...URL, createObjectURL: vi.fn(() => "blob:shared-layout"), revokeObjectURL: vi.fn() });
  try {
    await act(async () => mounted.render(<Harness />)); await flush();
    const request = { path: `${prefix}/_files/flow.png`, expected_revision: null };
    if (kind === "library") expect(client.libraryMedia).toHaveBeenCalledWith(request, expect.any(AbortSignal));
    else expect(client.contextMedia).toHaveBeenCalledWith("session", "viewer", { ...request, binding_id: "binding", root_id: rootId }, expect.any(AbortSignal));
    expect(host.querySelector<HTMLImageElement>('img[alt="Flow"]')?.src).toBe("blob:shared-layout");
  } finally { await act(async () => mounted.unmount()); host.remove(); vi.unstubAllGlobals(); }
});

it.each(["pdf", "svg", "html"])("never activates a downloaded %s attachment as a raster image", async (extension) => {
  const host = document.createElement("div"); document.body.append(host);
  const mounted = createRoot(host);
  const name = `hostile.${extension}`;
  const path = `${page.item_path}/_files/${name}`;
  const item = { ...page, attachments: [{ ...page.attachments[0]!, stored_name: name, state: "downloaded" as const, relative_path: `_files/${name}` }] };
  const client = {
    projectConfiguration: vi.fn(async () => ({ providers })),
    libraryListing: vi.fn(async () => ({ root: { root_id: "library:fs", kind: "library", label: "Library", path: "/library", repository_id: "", checkout_path: "" }, generation: "1", items: [item], follows: [], next_offset: null, diagnostics: [] })),
    libraryDocument: vi.fn(async () => ({ binding_id: "library", root_id: "library:fs", path, revision: "r1", content_hash: null, bytes: 100, media_type: extension === "html" ? "text/html" : extension === "svg" ? "image/svg+xml" : "application/pdf", text: extension === "pdf" ? null : '<svg onload="window.attachmentExecuted=true"><script>window.attachmentExecuted=true</script></svg>', truncated: false, diagnostics: [] })),
    libraryMedia: vi.fn(),
  } as unknown as CockpitClient;
  function Harness() {
    const [view, onChange] = useState<ContextViewState>(() => ({ ...createContextViewState(), rootId: "library", path }));
    return <ContextViewer client={client} context={null} value={view} onChange={onChange} />;
  }
  try {
    await act(async () => mounted.render(<Harness />)); await flush();
    expect(client.libraryMedia).not.toHaveBeenCalled();
    expect(host.querySelector("svg[onload],script")).toBeNull();
    if (extension === "pdf") expect(host.textContent).toContain("PDF preview unavailable");
    if (extension === "svg") expect(host.textContent).toContain("window.attachmentExecuted=true");
    for (const iframe of host.querySelectorAll("iframe")) expect(iframe.getAttribute("sandbox")?.split(" ")).not.toContain("allow-same-origin");
  } finally { await act(async () => mounted.unmount()); host.remove(); }
});
