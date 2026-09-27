// @vitest-environment jsdom
import { act } from "react";
import { createRoot } from "react-dom/client";
import { expect, it, vi } from "vitest";
import type { LibraryItemSummary } from "../../protocol/generated/v1";
import { LibraryItemHeader } from "./LibraryItemHeader";

Object.assign(globalThis, { IS_REACT_ACT_ENVIRONMENT: true });

it("shows folder lineage, partial and exclusion counts, and explicitly re-copies from both header and narrow menu", async () => {
  const item: LibraryItemSummary = {
    item_id: "folder:notes", logical_id: "folder:notes", kind: "folder_copy", provider_id: null, provider_instance: null, resource_type: null,
    canonical_id: null, container: null, parent_item_id: null, ancestors: [], order: null, title: "Design notes",
    document_path: "folders/notes-12345678/README.md", item_path: "folders/notes-12345678", source_url: null, original_url: null, source_revision: null, revision: "r1",
    state: "partial", partial: { unit: "files", have: 512, total: 600, reason: "file limit" }, conflict: [], fetched_at: "2026-09-26T00:00:00Z", checked_at: null,
    follow_id: null, attachments: [], diagnostics: [],
    folder: { origin_path: "/home/user/notes", git_working_tree: true, files: 512, bytes: 4100000, skipped_symlinks: 3, skipped_special: 1, skipped_ignored: 14, skipped_other: 2 },
  };
  const actions = { open: vi.fn(), refresh: vi.fn(), remove: vi.fn(), copyLink: vi.fn(), canCopyLink: false, refreshBusy: false };
  const host = document.createElement("div");
  document.body.append(host);
  const root = createRoot(host);
  const render = (narrow: boolean) => <LibraryItemHeader item={item} providers={[]} narrow={narrow} rootCrumb pending={false} actions={actions} onReplace={vi.fn()} details={null} />;
  try {
    await act(async () => root.render(render(false)));
    expect(host.querySelector(".library-item-path")?.textContent).toContain("Folders");
    expect(host.querySelector(".library-item-phrase")?.textContent).toContain("from /home/user/notes · 512 files · 4.1 MB · Git working tree");
    expect(host.querySelector('[role="status"]')?.textContent).toContain("512 of 600 files");
    const metadata = host.querySelector<HTMLDetailsElement>("details.library-metadata")!;
    expect(metadata.open).toBe(false);
    await act(async () => metadata.querySelector("summary")!.click());
    const values = Object.fromEntries([...metadata.querySelectorAll("dt")].map((term) => [term.textContent, term.nextElementSibling?.textContent]));
    expect(values).toMatchObject({ "Copied from": "/home/user/notes", "Skipped symlinks": "3", "Skipped special files": "1", "Skipped ignored files": "14", "Skipped other files": "2" });
    await act(async () => [...host.querySelectorAll("button")].find((button) => button.textContent === "Re-copy")!.click());
    expect(actions.refresh).toHaveBeenCalledWith({ scope: "items", item_ids: [item.item_id] }, [item.item_id]);
    actions.refresh.mockClear();
    await act(async () => root.render(render(true)));
    await act(async () => host.querySelector<HTMLButtonElement>("button.library-more")!.click());
    const recopy = [...document.body.querySelectorAll<HTMLButtonElement>('[role="menuitem"]')].find((button) => button.textContent === "Re-copy from /home/user/notes")!;
    await act(async () => recopy.click());
    expect(actions.refresh).toHaveBeenCalledWith({ scope: "items", item_ids: [item.item_id] }, [item.item_id]);
  } finally {
    await act(async () => root.unmount());
    host.remove();
  }
});
