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

it("shows a Confluence page's path, version, ancestors and last editor, and lists attachments read-only as not downloaded", async () => {
  const item: LibraryItemSummary = {
    item_id: "source:page-98765", logical_id: "source:confluence:page:98765", kind: "provider_snapshot", provider_id: "cloud", provider_instance: "https://nnexai.atlassian.net/wiki",
    resource_type: "page", canonical_id: "98765", container: { container_id: "SD", label: "SD · Software Development" }, parent_item_id: null,
    ancestors: [{ id: "1", title: "Engineering home" }, { id: "10", title: "Release process" }], order: 1, title: "Release checklist",
    document_path: "pages/sd-98765/document.md", item_path: "pages/sd-98765", source_url: "https://nnexai.atlassian.net/wiki/spaces/SD/pages/98765/Release+checklist", original_url: null,
    source_revision: "7", revision: "r2", state: "changed", partial: null, conflict: [], fetched_at: "2026-09-26T00:00:00Z", checked_at: null, follow_id: null, folder: null, diagnostics: [],
    attachments: [
      { attachment_id: "source:page-98765#attachment:a1", original_name: "release-flow.png", stored_name: "release-flow.png", media_type: "image/png", bytes: 84_000, version: "1", state: "not_downloaded", relative_path: null },
      { attachment_id: "source:page-98765#attachment:a2", original_name: "Q3/plan?.pdf", stored_name: "Q3_plan_.pdf", media_type: "application/pdf", bytes: 1_200_000, version: "3", state: "not_downloaded", relative_path: null },
    ],
  };
  const providers = [{ id: "cloud", base_url: "https://nnexai.atlassian.net/wiki", executable: "confluence", login: "default" }];
  const actions = { open: vi.fn(), refresh: vi.fn(), remove: vi.fn(), copyLink: vi.fn(), canCopyLink: true, refreshBusy: false };
  const host = document.createElement("div");
  document.body.append(host);
  const root = createRoot(host);
  const render = (narrow: boolean, by: string) => <LibraryItemHeader item={item} providers={providers} narrow={narrow} rootCrumb={false} pending={false} actions={actions} onReplace={vi.fn()} details={null}
    pageUpdate={{ at: "2026-09-25T10:00:00Z", by }} />;
  try {
    await act(async () => root.render(render(false, "M. Rossi")));
    expect(host.querySelector(".library-kind-chip")?.textContent).toBe("Confluence page");
    expect(host.querySelector(".library-item-path")?.textContent).toBe("SD / Engineering home / Release process");
    expect(host.querySelector(".library-item-phrase")?.textContent).toMatch(/^updated on last refresh.* · v7 by M\. Rossi$/);
    const metadata = host.querySelector<HTMLDetailsElement>("details.library-metadata")!;
    await act(async () => metadata.querySelector("summary")!.click());
    const values = Object.fromEntries([...metadata.querySelectorAll("dt")].map((term) => [term.textContent, term.nextElementSibling?.textContent]));
    expect(values).toMatchObject({
      Space: "SD · Software Development", "Page id": "98765", Parent: "Release process", Ancestors: "Engineering home / Release process",
      Version: "v7", "Last updated": "2026-09-25T10:00:00Z by M. Rossi",
    });
    expect(values).not.toHaveProperty("Source identity");

    const attachments = host.querySelector(".library-attachments")!;
    expect(attachments.querySelector(".library-attachments-heading")?.textContent).toBe("Attachments 2 · 0 downloaded");
    expect([...attachments.querySelectorAll("tbody tr")].map((row) => [...row.querySelectorAll("td")].map((cell) => cell.textContent)))
      .toEqual([["release-flow.png", "84 KB", "image/png", "not downloaded"], ["Q3_plan_.pdf", "1.2 MB", "application/pdf", "not downloaded"]]);
    // The stored name is shown; the original only in the tooltip.
    expect(attachments.querySelectorAll("td")[4]?.getAttribute("title")).toBe("Q3/plan?.pdf");
    // Read-only until downloads exist: no actions in the table.
    expect(attachments.querySelectorAll("button")).toHaveLength(0);

    // An email in the editor field is never shown.
    await act(async () => root.render(render(true, "m.rossi@example.com")));
    expect(host.textContent).not.toContain("@");
    expect(host.querySelector(".library-item-phrase")?.textContent).toMatch(/ · v7$/);
    expect([...host.querySelectorAll(".library-attachments li")].map((entry) => entry.textContent)).toEqual(["release-flow.png84 KB · not downloaded", "Q3_plan_.pdf1.2 MB · not downloaded"]);
    expect(actions.refresh).not.toHaveBeenCalled();
    expect(actions.remove).not.toHaveBeenCalled();
  } finally {
    await act(async () => root.unmount());
    host.remove();
  }
});
