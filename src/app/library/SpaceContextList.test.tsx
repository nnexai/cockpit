// @vitest-environment jsdom
import { act } from "react";
import { createRoot } from "react-dom/client";
import { expect, it, vi } from "vitest";
import type { CockpitClient } from "../../client/CockpitClient";
import type { LibraryItemSummary, SpaceContextListing } from "../../protocol/generated/v1";
import { SpaceContextList } from "./SpaceContextList";
import { useSpaceContextListing } from "./useLibraryOperation";

Object.assign(globalThis, { IS_REACT_ACT_ENVIRONMENT: true });

const target = { session_id: "session", space_id: "space-x" };
const space = { target, label: "X", live: true };
const item: LibraryItemSummary = {
  item_id: "source:ops-311", logical_id: "source:jira:ops-311", kind: "provider_snapshot", provider_id: "jira", provider_instance: "https://jira.test", resource_type: "issue",
  canonical_id: "OPS-311", container: null, parent_item_id: null, ancestors: [], order: null, title: "Rotate signing keys",
  document_path: "jira/OPS-311.md", item_path: "jira/OPS-311", source_url: null, original_url: null, source_revision: null, revision: "r1",
  state: "fresh", partial: null, conflict: [], fetched_at: null, checked_at: null, refs: [{ kind: "manual" }], purge_after: null, issue: null, attachments: [], folder: null, diagnostics: [],
};

async function settle(): Promise<void> {
  let resolve!: () => void;
  const promise = new Promise<void>((nextResolve) => { resolve = nextResolve; });
  await act(async () => { window.setTimeout(resolve, 0); await promise; });
}

function fixture() {
  let listing: SpaceContextListing = { target, space_label: "api-review", library_root: "/data/library", checkout_path: "/repo", items: [item], repository_paths: [], diagnostics: [] };
  const client = {
    projectConfiguration: vi.fn(async () => ({ providers: [{ id: "jira", kind: "jira" as const, base_url: "https://jira.test", deployment: "data_center" as const }] })),
    librarySpaceList: vi.fn(async () => listing),
    librarySpaceRemove: vi.fn(async () => { listing = { ...listing, items: [] }; return listing; }),
  } as unknown as CockpitClient;
  function Harness() {
    const state = useSpaceContextListing(client, target, true);
    return <SpaceContextList client={client} space={space} state={state} onAdd={vi.fn()} />;
  }
  return { client, Harness };
}

it("removes only the selected item IDs without a copy confirmation", async () => {
  const { client, Harness } = fixture();
  const host = document.createElement("div");
  document.body.append(host);
  const root = createRoot(host);
  try {
    await act(async () => root.render(<Harness />));
    await settle();
    const remove = host.querySelector<HTMLButtonElement>('button[aria-label="Remove Rotate signing keys from api-review"]')!;
    expect(remove.textContent).toBe("Remove from Space");
    expect(host.querySelector('[role="status"]')?.textContent).toBe("Up to date");
    await act(async () => remove.click());
    await settle();
    expect(host.querySelector("[data-space-item]")).toBeNull();
    expect(client.librarySpaceRemove).toHaveBeenCalledTimes(1);
    expect(client.librarySpaceRemove).toHaveBeenCalledWith({ target, item_ids: [item.item_id] });
    expect(host.textContent).toContain("Nothing selected for api-review yet.");
    expect(document.body.querySelector(".library-confirm")).toBeNull();
  } finally {
    await act(async () => root.unmount());
    host.remove();
  }
});

it("keeps a selected row when removal fails and retries the same selection request", async () => {
  const { client, Harness } = fixture();
  vi.mocked(client.librarySpaceRemove).mockRejectedValueOnce(new Error("Disk error"));
  const host = document.createElement("div");
  document.body.append(host);
  const root = createRoot(host);
  try {
    await act(async () => root.render(<Harness />));
    await settle();
    await act(async () => host.querySelector<HTMLButtonElement>('button[aria-label="Remove Rotate signing keys from api-review"]')!.click());
    await settle();
    const row = host.querySelector<HTMLElement>("[data-space-item]")!;
    expect(row.textContent).toContain(item.title);
    expect(row.querySelector('[role="alert"]')?.textContent).toContain("Couldn't remove Rotate signing keys from api-review. Disk error");
    const retry = [...row.querySelectorAll<HTMLButtonElement>("button")].find((button) => button.textContent === "Retry")!;
    await act(async () => retry.click());
    await settle();
    expect(client.librarySpaceRemove).toHaveBeenCalledTimes(2);
    expect(client.librarySpaceRemove).toHaveBeenLastCalledWith({ target, item_ids: [item.item_id] });
    expect(host.querySelector("[data-space-item]")).toBeNull();
  } finally {
    await act(async () => root.unmount());
    host.remove();
  }
});
