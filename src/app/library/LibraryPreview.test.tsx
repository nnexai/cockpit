// @vitest-environment jsdom
import "../input/viewerTestLayout";
import { act, useState } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, expect, it, vi } from "vitest";
import type { CockpitClient } from "../../client/CockpitClient";
import type { ContextDocument, LibraryItemSummary, LibraryListing } from "../../protocol/generated/v1";
import { ContextViewer, createContextViewState, type ContextViewState } from "../context/ContextViewer";
import { LibraryView } from "./LibraryView";

import { announceLibraryChanged } from "./useLibraryOperation";
Object.assign(globalThis, { IS_REACT_ACT_ENVIRONMENT: true });
const originalVisibility = Object.getOwnPropertyDescriptor(document, "visibilityState");
const mounted: Array<{ root: Root; host: HTMLDivElement }> = [];
const page: LibraryItemSummary = {
  item_id: "page:7", logical_id: "page:7", kind: "provider_snapshot", provider_id: "confluence", provider_instance: "https://wiki.test", resource_type: "page", canonical_id: "7",
  container: { container_id: "SD", label: "SD" }, parent_item_id: null, ancestors: [], order: null, title: "Release", document_path: "confluence/wiki.test/SD/Release/Release.md", item_path: "confluence/wiki.test/SD/Release", source_url: null, original_url: null, source_revision: "1", revision: "snapshot-1", state: "fresh", partial: null, conflict: [], fetched_at: null, checked_at: null, refs: [{ kind: "manual" }], purge_after: null, issue: null, folder: null, attachments: [], diagnostics: [],
};
const other: LibraryItemSummary = { ...page, item_id: "page:8", logical_id: "page:8", canonical_id: "8", title: "Other", document_path: "confluence/wiki.test/SD/Other/Other.md", item_path: "confluence/wiki.test/SD/Other" };
const renamed: LibraryItemSummary = { ...page, title: "Renamed release", document_path: "confluence/wiki.test/SD/Renamed release/Renamed release.md", item_path: "confluence/wiki.test/SD/Renamed release", source_revision: "2", revision: "snapshot-2" };

beforeEach(() => {
  vi.useFakeTimers();
  Object.defineProperty(document, "visibilityState", { value: "visible", configurable: true });
});
afterEach(async () => {
  for (const { root, host } of mounted.splice(0)) { await act(async () => root.unmount()); host.remove(); }
  vi.useRealTimers();
  if (originalVisibility) Object.defineProperty(document, "visibilityState", originalVisibility);
  else Reflect.deleteProperty(document, "visibilityState");
});

function savedDocument(path: string, revision: string, text: string): ContextDocument {
  return { binding_id: "library", root_id: "library:fs", path, revision, text, bytes: text.length, content_hash: null, media_type: "text/markdown", truncated: false, diagnostics: [] };
}
function deferredDocument() {
  let resolve!: (document: ContextDocument) => void;
  const promise = new Promise<ContextDocument>((accept) => { resolve = accept; });
  return { promise, resolve };
}
async function flush() { for (let i = 0; i < 8; i++) await act(async () => { await Promise.resolve(); }); }
async function fixture(initial: LibraryItemSummary[] = [page, other], path?: string) {
  let items = initial;
  let generation = 1;
  const bodies = new Map(initial.map((item) => [item.document_path!, `${item.title} body 1`]));
  const libraryListing = vi.fn(async (): Promise<LibraryListing> => ({ root: { root_id: "library:fs", kind: "library", label: "Library", path: "/library", repository_id: "", checkout_path: "" }, generation: `g${generation}`, items, follows: [], next_offset: null, diagnostics: [] }));
  const libraryDocument = vi.fn(async (request: { path: string }, _signal: AbortSignal) => {
    const item = items.find((candidate) => candidate.document_path === request.path || request.path.startsWith(`${candidate.item_path}/`));
    if (!item) throw new Error("Requested an obsolete owned path");
    return savedDocument(request.path, item.revision, bodies.get(request.path) ?? "Extra file body");
  });
  const client = { libraryListing, libraryDocument, libraryRefresh: vi.fn(), projectConfiguration: vi.fn(async () => ({ providers: [{ id: "confluence", kind: "confluence", base_url: "https://wiki.test", deployment: "data_center" }] })) } as unknown as CockpitClient;
  const host = document.createElement("div"); document.body.append(host);
  const root = createRoot(host); mounted.push({ root, host });
  function FileSelection() {
    const [view, onChange] = useState<ContextViewState>(() => ({ ...createContextViewState(), rootId: "library", path: path! }));
    return <ContextViewer client={client} context={null} value={view} onChange={onChange} />;
  }
  await act(async () => root.render(path ? <FileSelection /> : <LibraryView client={client} onClose={vi.fn()} />));
  await flush();
  return {
    host, client, libraryListing, libraryDocument,
    async select(item: LibraryItemSummary) {
      await act(async () => host.querySelector<HTMLButtonElement>(`[data-library-row="${item.item_id}"]`)!.click());
      await flush();
    },
    async publish(nextItems: LibraryItemSummary[], updatedBodies: Array<[string, string]> = []) {
      items = nextItems; generation += 1;
      for (const [bodyPath, body] of updatedBodies) bodies.set(bodyPath, body);
      await act(async () => { await vi.advanceTimersByTimeAsync(60_000); });
      await flush();
    },
    replaceBytes(bodyPath: string, body: string) { bodies.set(bodyPath, body); },
    body: () => host.querySelector(".context-document")?.textContent ?? "",
    selectedId: () => host.querySelector<HTMLElement>(".library-tree-row.is-selected")?.dataset.libraryRow,
  };
}

it("keeps the selected primary document through an automatic owned-path rename and then a same-path body snapshot", async () => {
  const display = await fixture();
  await display.select(page);
  expect(display.body()).toContain("Release body 1");
  expect(display.libraryDocument).toHaveBeenCalledTimes(1);

  await display.publish([renamed, other], [[renamed.document_path!, "Published renamed body 2"]]);
  expect(display.selectedId()).toBe(page.item_id);
  expect(display.body()).toContain("Published renamed body 2");
  expect(display.body()).not.toContain("Select an item to read it");
  expect(display.libraryDocument.mock.calls.map(([request]) => request.path)).toEqual([page.document_path, renamed.document_path]);

  const bodyOnly = { ...renamed, source_revision: "3", revision: "snapshot-3" };
  await display.publish([bodyOnly, other], [[bodyOnly.document_path!, "Published same-path body 3"]]);
  expect(display.selectedId()).toBe(page.item_id);
  expect(display.body()).toContain("Published same-path body 3");
  expect(display.libraryDocument).toHaveBeenCalledTimes(3);

  // A new index generation and another item's snapshot are not a body invalidation.
  await display.publish([bodyOnly, { ...other, revision: "other-snapshot-2" }]);
  expect(display.libraryDocument).toHaveBeenCalledTimes(3);
  expect(display.body()).toContain("Published same-path body 3");
  expect(display.client.libraryRefresh).not.toHaveBeenCalled();
});

it("ignores a late document response from an older selected snapshot even when the path is unchanged", async () => {
  const display = await fixture();
  await display.select(page);
  const older = deferredDocument();
  display.libraryDocument.mockReturnValueOnce(older.promise);
  await display.publish([{ ...page, revision: "snapshot-2" }, other]);
  const olderSignal = display.libraryDocument.mock.calls.at(-1)![1];
  expect(olderSignal.aborted).toBe(false);

  await display.publish([{ ...page, revision: "snapshot-3" }, other], [[page.document_path!, "Newest published body"]]);
  expect(olderSignal.aborted).toBe(true);
  expect(display.body()).toContain("Newest published body");
  await act(async () => older.resolve(savedDocument(page.document_path!, "snapshot-2", "Obsolete response body")));
  expect(display.body()).toContain("Newest published body");
  expect(display.body()).not.toContain("Obsolete response body");
});

it("rereads bytes on a legacy manual change event even when the indexed snapshot revision did not change", async () => {
  const display = await fixture();
  await display.select(page);
  display.replaceBytes(page.document_path!, "Restored source bytes after replacing an external edit");
  await act(async () => announceLibraryChanged());
  await flush();
  expect(display.body()).toContain("Restored source bytes after replacing an external edit");
  expect(display.libraryDocument).toHaveBeenCalledTimes(2);
  expect(display.libraryDocument.mock.calls.map(([request]) => request.path)).toEqual([page.document_path, page.document_path]);
});

it("preserves a newer manual item selection rather than following the previously selected item's rename", async () => {
  const display = await fixture();
  const pending = deferredDocument();
  display.libraryDocument.mockReturnValueOnce(pending.promise);
  await display.select(page);
  const oldSignal = display.libraryDocument.mock.calls[0]![1];
  await display.select(other);
  expect(oldSignal.aborted).toBe(true);
  expect(display.selectedId()).toBe(other.item_id);
  await display.publish([renamed, other], [[renamed.document_path!, "Unselected renamed body"]]);
  expect(display.selectedId()).toBe(other.item_id);
  expect(display.body()).toContain("Other body 1");
  expect(display.libraryDocument.mock.calls.map(([request]) => request.path)).toEqual([page.document_path, other.document_path]);
  await act(async () => pending.resolve(savedDocument(page.document_path!, "snapshot-1", "Late previous item body")));
  expect(display.body()).not.toContain("Late previous item body");
});

it("clears a removed primary item instead of selecting a different item that reuses its old path", async () => {
  const display = await fixture();
  await display.select(page);
  await display.publish([{ ...other, document_path: page.document_path, item_path: page.item_path }], [[page.document_path!, "Different item at old path"]]);
  expect(display.selectedId()).toBeUndefined();
  expect(display.body()).toContain("Select an item to read it");
  expect(display.body()).not.toContain("Different item at old path");
  expect(display.libraryDocument).toHaveBeenCalledTimes(1);
});

it.each(["attachment", "arbitrary file"])("does not retarget a missing nonprimary %s to the renamed primary document", async (kind) => {
  const extraPath = `${page.item_path}/_files/readme.md`;
  const item = kind === "attachment" ? { ...page, attachments: [{ attachment_id: "readme", original_name: "readme.md", stored_name: "readme.md", media_type: "text/markdown", bytes: 15, version: "1", state: "downloaded" as const, relative_path: "_files/readme.md" }] } : page;
  const display = await fixture([item], extraPath);
  expect(display.body()).toContain("Extra file body");
  await display.publish([renamed], [[renamed.document_path!, "Primary document must not be selected"]]);
  expect(display.body()).toContain("Select an item to read it");
  expect(display.body()).not.toContain("Primary document must not be selected");
  expect(display.libraryDocument.mock.calls.map(([request]) => request.path)).toEqual([extraPath]);
});
