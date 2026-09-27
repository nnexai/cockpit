// @vitest-environment jsdom
import { act } from "react";
import { createRoot } from "react-dom/client";
import { expect, it, vi } from "vitest";
import type { CockpitClient } from "../../client/CockpitClient";
import type { LibraryOperation, SpaceContextListing, SpaceCopyRow, SpaceFollowSummary } from "../../protocol/generated/v1";
import { SpaceContextList } from "./SpaceContextList";
import { useSpaceContextListing } from "./useLibraryOperation";

Object.assign(globalThis, { IS_REACT_ACT_ENVIRONMENT: true });

const target = { session_id: "session", space_id: "space-x" };
const space = { target, label: "X", live: true };

function followRow(followId: string, key: string, name: string, counts: Partial<SpaceFollowSummary>): SpaceCopyRow {
  const follow: SpaceFollowSummary = { follow_id: followId, space_key: key, page_count: 5, new_pages: 0, changed_pages: 0, edited_pages: 0, removed_at_source_pages: 0, ...counts };
  const behind = follow.new_pages + follow.changed_pages > 0;
  return {
    item_id: null, logical_id: followId, title: `${key} · ${name}`, provider_id: "cloud", resource_type: null, kind: "provider_snapshot",
    state: behind ? "library_newer" : "up_to_date", library_newer: behind, paths: [], edited: [], copy_mode: "reflink",
    library_revision_copied: null, current_library_revision: null, follow,
  };
}

async function settle(): Promise<void> {
  await act(async () => { await new Promise((resolve) => window.setTimeout(resolve, 0)); });
}

it("shows one aggregate row per followed space, and each row's Update writes only its own follow", async () => {
  let rows = [
    followRow("follow:sd", "SD", "Software Development", { new_pages: 1, changed_pages: 1 }),
    followRow("follow:ops", "OPS", "Operations", { new_pages: 1, changed_pages: 1, edited_pages: 1 }),
  ];
  const updated: LibraryOperation = {
    operation_id: "op-update-sd", kind: "space_update", item_ids: ["source:n", "source:a1"], report: null, target, cancel_requested: false, finished: true, created_at: "", updated_at: "",
    phases: [{ phase: "space", state: "done", done: 2, total: 2, message: null, error: null }],
    space: { space_id: "space-x", copy_mode: "reflink", written: ["sources/confluence/page/SD/40-n/document.md", "sources/confluence/page/SD/11-a1/document.md"], skipped_edited: [], companion_root_id: "companion:x" },
  };
  const client = {
    projectConfiguration: vi.fn(async () => ({ providers: [{ id: "cloud", base_url: "https://nnexai.atlassian.net/wiki", executable: "confluence" }] })),
    librarySpaceList: vi.fn(async (): Promise<SpaceContextListing> => ({ target, companion: { status: "available", companion_root_id: "companion:x", companion_label: "X" }, attempts: [], rows, behind: rows.filter((row) => row.library_newer).length, diagnostics: [] })),
    librarySpaceUpdate: vi.fn(async () => {
      rows = [followRow("follow:sd", "SD", "Software Development", { page_count: 6 }), rows[1]!];
      return updated;
    }),
  } as unknown as CockpitClient;
  function Harness() {
    const state = useSpaceContextListing(client, target, true);
    return <SpaceContextList client={client} space={space} state={state} onAdd={vi.fn()} />;
  }
  const host = document.createElement("div");
  document.body.append(host);
  const root = createRoot(host);
  const entry = (title: string) => [...host.querySelectorAll<HTMLElement>("[role='listitem']")].find((candidate) => candidate.querySelector(".context-source-title")?.textContent?.startsWith(title))!;
  const chips = (title: string) => [...entry(title).querySelectorAll(".library-state")].map((chip) => chip.textContent);
  try {
    await act(async () => root.render(<Harness />));
    await settle();
    expect([...host.querySelectorAll(".context-source-title")].map((title) => title.textContent)).toEqual(["OPS · Operations · 5 pages", "SD · Software Development · 5 pages"]);
    expect(chips("SD")).toEqual(["↑ Library newer: 1 new, 1 changed pages"]);
    expect(chips("OPS")).toEqual(["↑ Library newer: 1 new, 1 changed pages", "✎ 1 page edited in Space"]);
    expect([...entry("SD").querySelectorAll(".context-source-chip:not(.library-state)")].map((chip) => chip.textContent)).toEqual(["Confluence", "followed space"]);
    // Both follows count toward `Update all`; neither row offers item-only actions.
    expect([...host.querySelectorAll("button")].map((button) => button.textContent)).toContain("Update all (2)");
    expect([...entry("SD").querySelectorAll("button")].map((button) => button.textContent)).toEqual(["Update"]);

    await act(async () => entry("SD").querySelector("button")!.click());
    await settle();
    expect(client.librarySpaceUpdate).toHaveBeenCalledTimes(1);
    expect(client.librarySpaceUpdate).toHaveBeenCalledWith({ target, scope: { scope: "selection", item_ids: [], follow_ids: ["follow:sd"] }, replace_edited: [] });
    // The reread shows SD current; OPS still has its new and changed pages and its own Update.
    expect(chips("SD")).toEqual(["✓ Up to date"]);
    expect(entry("SD").textContent).toContain("SD · Software Development · 6 pages");
    expect(entry("SD").querySelector("button")).toBeNull();
    expect(chips("OPS")).toEqual(["↑ Library newer: 1 new, 1 changed pages", "✎ 1 page edited in Space"]);
    expect([...entry("OPS").querySelectorAll("button")].map((button) => button.textContent)).toEqual(["Update"]);
    expect([...host.querySelectorAll("button")].map((button) => button.textContent)).toContain("Update all (1)");
  } finally {
    await act(async () => root.unmount());
    host.remove();
  }
});
