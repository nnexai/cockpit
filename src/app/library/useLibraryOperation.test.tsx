// @vitest-environment jsdom
import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import type { CockpitClient } from "../../client/CockpitClient";
import type { LibraryItemSummary, LibraryListing, LibraryOperation, SpaceContextListing } from "../../protocol/generated/v1";
import { LIBRARY_CHANGED_EVENT, useLibraryListing, useLibraryOperation, useSpaceContextListing, type LibraryChangeDetail, type LibraryListingState } from "./useLibraryOperation";

const watches = vi.hoisted(() => new Map<object, { event: (event: { kind: string }) => Promise<void>; error: () => void }>());
vi.mock("../localChanges", () => ({
  subscribeLocalChanges: (client: object, _scope: unknown, event: (event: { kind: string }) => Promise<void>, error: () => void) => {
    const watch = { event, error };
    watches.set(client, watch);
    Promise.resolve().then(() => { if (watches.get(client) === watch) void event({ kind: "ready" }); });
    return { close: () => { if (watches.get(client) === watch) watches.delete(client); }, retry: () => {} };
  },
}));

Object.assign(globalThis, { IS_REACT_ACT_ENVIRONMENT: true });

afterEach(() => { vi.useRealTimers(); });

const operation = (finished: boolean): LibraryOperation => ({
  operation_id: "lifecycle-op", kind: "add", phases: [{ phase: "library", state: finished ? "done" : "running", done: finished ? 1 : 0, total: 1, message: null, error: null }],
  item_ids: ["source:item"], report: null, space: null, target: null, cancel_requested: false, finished, created_at: "", updated_at: "",
});

it("keeps polling after the initiating view unmounts and exposes pending IDs to a remounted Library", async () => {
  vi.useFakeTimers();
  const changed = vi.fn();
  window.addEventListener(LIBRARY_CHANGED_EVENT, changed);
  const client = {
    libraryOperation: vi.fn().mockResolvedValueOnce(operation(false)).mockResolvedValueOnce(operation(true)),
    libraryOperationCancel: vi.fn(),
  } as unknown as CockpitClient;
  let startRequest: Promise<LibraryOperation | null> | null = null;
  function View() {
    const state = useLibraryOperation(client);
    return <div><span data-pending={state.pendingItemIds.has("source:item")} />
      <button type="button" onClick={() => { startRequest = state.start(async () => operation(false)); }}>Start</button></div>;
  }
  const host = document.createElement("div");
  document.body.append(host);
  const root = createRoot(host);
  try {
    await act(async () => root.render(<View />));
    await act(async () => { host.querySelector("button")!.click(); await startRequest; });
    expect(host.querySelector("span")?.getAttribute("data-pending")).toBe("true");
    await act(async () => root.render(null));
    await act(async () => { await vi.advanceTimersByTimeAsync(750); });
    expect(client.libraryOperation).toHaveBeenCalledWith("lifecycle-op");
    await act(async () => root.render(<View />));
    expect(host.querySelector("span")?.getAttribute("data-pending")).toBe("true");
    await act(async () => { await vi.advanceTimersByTimeAsync(750); });
    expect(host.querySelector("span")?.getAttribute("data-pending")).toBe("false");
    expect(changed).toHaveBeenCalledOnce();
  } finally {
    window.removeEventListener(LIBRARY_CHANGED_EVENT, changed);
    await act(async () => root.unmount());
    host.remove();
  }
});

it("still polls and announces an accepted operation after a newer start takes over the surface", async () => {
  vi.useFakeTimers();
  const announced: string[] = [];
  const changed = (event: Event) => { const detail = (event as CustomEvent<LibraryChangeDetail>).detail; if (detail && detail.kind !== "snapshot_update") announced.push(detail.operation_id); };
  window.addEventListener(LIBRARY_CHANGED_EVENT, changed);
  const older = (finished: boolean): LibraryOperation => ({ ...operation(finished), operation_id: "older-op", item_ids: ["source:older"] });
  const newer = (finished: boolean): LibraryOperation => ({ ...operation(finished), operation_id: "newer-op", item_ids: ["source:newer"] });
  const client = {
    libraryOperation: vi.fn(async (id: string) => id === "older-op" ? older(true) : newer(true)),
    libraryOperationCancel: vi.fn(),
  } as unknown as CockpitClient;
  let acceptOlder: (value: LibraryOperation) => void = () => undefined;
  let olderRequest: Promise<LibraryOperation | null> | null = null;
  let newerRequest: Promise<LibraryOperation | null> | null = null;
  function View() {
    const state = useLibraryOperation(client);
    return <div><span data-shown={state.operation?.operation_id ?? ""} />
      <button type="button" data-start="older" onClick={() => { olderRequest = state.start(() => new Promise((resolve) => { acceptOlder = resolve; })); }}>Older</button>
      <button type="button" data-start="newer" onClick={() => { newerRequest = state.start(async () => newer(false)); }}>Newer</button></div>;
  }
  const host = document.createElement("div");
  document.body.append(host);
  const root = createRoot(host);
  try {
    await act(async () => root.render(<View />));
    await act(async () => { host.querySelector<HTMLButtonElement>("[data-start='older']")!.click(); });
    await act(async () => { host.querySelector<HTMLButtonElement>("[data-start='newer']")!.click(); await newerRequest; });
    // The older request is accepted only after the newer one started.
    await act(async () => { acceptOlder(older(false)); await olderRequest; });
    expect(host.querySelector("span")?.getAttribute("data-shown")).toBe("newer-op");
    await act(async () => { await vi.advanceTimersByTimeAsync(750); });
    expect(client.libraryOperation).toHaveBeenCalledWith("older-op");
    expect(announced).toContain("older-op");
    expect(announced).toContain("newer-op");
    expect(host.querySelector("span")?.getAttribute("data-shown")).toBe("newer-op");
  } finally {
    window.removeEventListener(LIBRARY_CHANGED_EVENT, changed);
    await act(async () => root.unmount());
    host.remove();
  }
});

it("retains the last good listing when reload fails so the view can show its retry notice", async () => {
  const listing = { items: [{ item_id: "source:item" }], next_offset: null, generation: 1 } as unknown as LibraryListing;
  const client = {
    libraryListing: vi.fn().mockResolvedValueOnce(listing).mockRejectedValueOnce(new Error("offline")),
    projectConfiguration: vi.fn(async () => ({ providers: [] })),
  } as unknown as CockpitClient;
  function View() {
    const state = useLibraryListing(client, true);
    return <div><span data-status={state.status} data-items={state.listing?.items.length ?? 0}>{state.error}</span>
      <button type="button" onClick={state.reload}>Retry</button></div>;
  }
  const host = document.createElement("div");
  document.body.append(host);
  const root = createRoot(host);
  try {
    await act(async () => root.render(<View />));
    expect(host.querySelector("span")?.getAttribute("data-status")).toBe("ready");
    await act(async () => { host.querySelector("button")!.click(); });
    expect(host.querySelector("span")?.getAttribute("data-status")).toBe("error");
    expect(host.querySelector("span")?.getAttribute("data-items")).toBe("1");
    expect(host.textContent).toContain("offline");
  } finally {
    await act(async () => root.unmount());
    host.remove();
  }
});

it("reads direct Space selections on demand and on Library changes, without periodic polling", async () => {
  vi.useFakeTimers();
  const target = { session_id: "session", space_id: "space" };
  const listing: SpaceContextListing = { target, space_label: "Review", library_root: "/data/library", checkout_path: "/repo", items: [], repository_paths: ["/extra"], diagnostics: [] };
  const client = { librarySpaceList: vi.fn(async () => listing) } as unknown as CockpitClient;
  function View() {
    const state = useSpaceContextListing(client, target, true);
    return <div><span data-status={state.status}>{state.listing?.repository_paths.join(",")}</span>
      <button type="button" onClick={state.reload}>Reload</button></div>;
  }
  const host = document.createElement("div");
  document.body.append(host);
  const root = createRoot(host);
  try {
    await act(async () => root.render(<View />));
    expect(client.librarySpaceList).toHaveBeenCalledWith({ target }, expect.any(AbortSignal));
    expect(host.querySelector("span")?.getAttribute("data-status")).toBe("ready");
    expect(host.textContent).toContain("/extra");
    await act(async () => { await vi.advanceTimersByTimeAsync(10_000); });
    expect(client.librarySpaceList).toHaveBeenCalledTimes(1);
    await act(async () => host.querySelector<HTMLButtonElement>("button")!.click());
    expect(client.librarySpaceList).toHaveBeenCalledTimes(2);
    await act(async () => window.dispatchEvent(new Event(LIBRARY_CHANGED_EVENT)));
    expect(client.librarySpaceList).toHaveBeenCalledTimes(3);
  } finally {
    await act(async () => root.unmount());
    host.remove();
  }
});

describe("background Library listing visibility", () => {
  const originalVisibility = Object.getOwnPropertyDescriptor(document, "visibilityState");
  const mounted: Array<{ root: Root; host: HTMLDivElement }> = [];
  const listeners: Array<(event: Event) => void> = [];

  function visibility(value: DocumentVisibilityState) {
    Object.defineProperty(document, "visibilityState", { value, configurable: true });
  }

  beforeEach(() => {
    vi.useFakeTimers();
    visibility("visible");
  });

  afterEach(async () => {
    for (const { root, host } of mounted.splice(0)) {
      await act(async () => root.unmount());
      host.remove();
    }
    for (const listener of listeners.splice(0)) window.removeEventListener(LIBRARY_CHANGED_EVENT, listener);
    if (originalVisibility) Object.defineProperty(document, "visibilityState", originalVisibility);
    else Reflect.deleteProperty(document, "visibilityState");
  });

  function item(item_id: string): LibraryItemSummary {
    return {
      item_id, logical_id: item_id, kind: "provider_snapshot", provider_id: "gitlab", provider_instance: "https://gitlab.test", resource_type: "issue",
      canonical_id: item_id, container: null, parent_item_id: null, ancestors: [], order: null, title: item_id,
      document_path: null, item_path: item_id, source_url: null, original_url: null, source_revision: null, revision: "r",
      state: "fresh", partial: null, conflict: [], fetched_at: null, checked_at: null, refs: [{ kind: "manual" }], purge_after: null,
      issue: null, attachments: [], folder: null, diagnostics: [],
    };
  }

  function page(generation: string, ids: string[], next_offset: number | null = null): LibraryListing {
    return {
      root: { root_id: "library:fs", kind: "library", label: "Library", path: "/library", repository_id: "", checkout_path: "" },
      generation, items: ids.map(item), next_offset, follows: [], diagnostics: [],
    };
  }

  function deferred() {
    let resolve!: (value: LibraryListing) => void;
    let reject!: (cause: Error) => void;
    const promise = new Promise<LibraryListing>((done, fail) => { resolve = done; reject = fail; });
    return { promise, resolve, reject };
  }

  function fixture() {
    const libraryListing = vi.fn<CockpitClient["libraryListing"]>();
    const client = { libraryListing, projectConfiguration: vi.fn(async () => ({ providers: [] })) } as unknown as CockpitClient;
    return { client, libraryListing };
  }

  function observeChanges() {
    const changed = vi.fn();
    listeners.push(changed);
    window.addEventListener(LIBRARY_CHANGED_EVENT, changed);
    return changed;
  }

  async function view(client: CockpitClient, active = true) {
    const host = document.createElement("div");
    document.body.append(host);
    const root = createRoot(host);
    mounted.push({ root, host });
    const renders: Array<LibraryListing | null> = [];
    let latest!: LibraryListingState;
    function View({ source, enabled }: { source: CockpitClient; enabled: boolean }) {
      latest = useLibraryListing(source, enabled);
      renders.push(latest.listing);
      return <div><span data-status={latest.status} data-items={latest.listing?.items.map((item) => item.item_id).join(",") ?? ""}>{latest.error}</span>
        <button type="button" onClick={latest.reload}>Reload</button></div>;
    }
    await act(async () => root.render(<View source={client} enabled={active} />));
    return {
      host, renders,
      get state() { return latest; },
      render: async (source: CockpitClient, enabled: boolean) => {
        await act(async () => root.render(<View source={source} enabled={enabled} />));
      },
      unmount: async () => { await act(async () => root.render(null)); },
    };
  }

  const emit = async (client: CockpitClient, kind = "changed") => {
    await act(async () => { await watches.get(client)!.event({ kind }); });
  };

  it("does no work while inactive and no recurring reads after its baseline", async () => {
    const { client, libraryListing } = fixture();
    libraryListing.mockResolvedValue(page("g1", ["a"]));
    const display = await view(client, false);
    expect(watches.has(client)).toBe(false);
    expect(client.projectConfiguration).not.toHaveBeenCalled();
    await display.render(client, true);
    await act(async () => { await vi.advanceTimersByTimeAsync(120_000); });
    expect(libraryListing.mock.calls).toEqual([[null]]);
  });

  it("reads all changed pages and announces once without feedback", async () => {
    const { client, libraryListing } = fixture();
    libraryListing.mockResolvedValueOnce(page("g1", ["old"])).mockResolvedValueOnce(page("g2", ["new"], 256)).mockResolvedValueOnce(page("g2", ["second"]));
    const changed = observeChanges();
    const display = await view(client);
    await emit(client);
    expect(display.state.listing?.items.map((item) => item.item_id)).toEqual(["new", "second"]);
    expect(libraryListing.mock.calls).toEqual([[null], [null], [256]]);
    expect(changed).toHaveBeenCalledOnce();
  });

  it("updates follow status without rereading remaining pages", async () => {
    const { client, libraryListing } = fixture();
    const initial = page("g1", ["one"], 256);
    initial.follows = [{
      follow_id: "follow:j", provider_id: "jira", provider_instance: "https://jira.test",
      source: { kind: "jira_query", jql: "project = OPS", mode: "live" },
      include_attachments: false, item_count: 2, partial: null, excluded_ids: [], last_refreshed_at: null, state: "fresh",
    }];
    const next = { ...initial, follows: initial.follows.map((follow) => ({ ...follow, sync: { last_success_at: "now", reason: null } })) };
    libraryListing.mockResolvedValueOnce(initial).mockResolvedValueOnce(page("g1", ["two"])).mockResolvedValueOnce(next);
    const display = await view(client);
    const items = display.state.listing!.items;
    await emit(client);
    expect(libraryListing.mock.calls).toEqual([[null], [256], [null]]);
    expect(display.state.listing?.items).toBe(items);
    expect(display.state.listing?.follows).toBe(next.follows);
  });

  it("coalesces changes during pagination and keeps the final snapshot", async () => {
    const { client, libraryListing } = fixture();
    const pending = deferred();
    libraryListing.mockResolvedValueOnce(page("g1", ["old"])).mockResolvedValueOnce(page("g2", ["a"], 256))
      .mockReturnValueOnce(pending.promise).mockResolvedValueOnce(page("g3", ["final"]));
    const display = await view(client);
    let first!: Promise<void>;
    await act(async () => { first = watches.get(client)!.event({ kind: "changed" }); });
    await act(async () => { void watches.get(client)!.event({ kind: "changed" }); });
    await act(async () => { pending.resolve(page("g2", ["b"])); await first; });
    expect(display.state.listing?.items.map((item) => item.item_id)).toEqual(["final"]);
    expect(libraryListing.mock.calls).toEqual([[null], [null], [256], [null]]);
  });

  it("restarts pagination when generation moves between pages", async () => {
    const { client, libraryListing } = fixture();
    libraryListing.mockResolvedValueOnce(page("g1", ["old"])).mockResolvedValueOnce(page("g2", ["a"], 256))
      .mockResolvedValueOnce(page("g3", ["discard"])).mockResolvedValueOnce(page("g3", ["final"]));
    const display = await view(client);
    await emit(client);
    expect(display.state.listing?.generation).toBe("g3");
    expect(libraryListing.mock.calls).toEqual([[null], [null], [256], [null]]);
  });

  it("preserves data and watch failure through manual reload until ready", async () => {
    const { client, libraryListing } = fixture();
    libraryListing.mockResolvedValue(page("g1", ["old"]));
    const display = await view(client);
    await act(async () => watches.get(client)!.error());
    expect(display.state.watchUnavailable).toBe(true);
    await act(async () => display.state.reload());
    expect(display.state.listing?.items[0].item_id).toBe("old");
    expect(display.state.watchUnavailable).toBe(true);
    await emit(client, "ready");
    expect(display.state.watchUnavailable).toBe(false);
  });

  it("refreshes during a watch outage and stops on recovery", async () => {
    vi.useFakeTimers();
    const { client, libraryListing } = fixture();
    libraryListing.mockResolvedValue(page("g1", ["old"]));
    const display = await view(client);
    await act(async () => watches.get(client)!.error());
    libraryListing.mockResolvedValue(page("g2", ["new"]));
    await act(async () => { await vi.advanceTimersByTimeAsync(30_000); });
    expect(display.state.listing?.items[0].item_id).toBe("new");
    expect(display.state.watchUnavailable).toBe(true);
    await emit(client, "ready");
    const reads = libraryListing.mock.calls.length;
    await act(async () => { await vi.advanceTimersByTimeAsync(120_000); });
    expect(libraryListing).toHaveBeenCalledTimes(reads);
  });

  it("ignores late responses after client or visibility changes", async () => {
    const old = fixture();
    old.libraryListing.mockResolvedValueOnce(page("old", ["old"]));
    const display = await view(old.client);
    const pending = deferred();
    old.libraryListing.mockReturnValueOnce(pending.promise);
    await act(async () => { void watches.get(old.client)!.event({ kind: "changed" }); });
    const next = fixture();
    next.libraryListing.mockResolvedValue(page("new", ["new"]));
    await display.render(next.client, true);
    await act(async () => pending.resolve(page("stale", ["stale"], 256)));
    expect(display.state.listing?.generation).toBe("new");
    expect(old.libraryListing).toHaveBeenCalledTimes(2);
    const hidden = deferred();
    next.libraryListing.mockReturnValueOnce(hidden.promise);
    await act(async () => { void watches.get(next.client)!.event({ kind: "changed" }); });
    visibility("hidden");
    await act(async () => document.dispatchEvent(new Event("visibilitychange")));
    await act(async () => hidden.resolve(page("hidden", ["hidden"])));
    expect(display.state.listing?.generation).toBe("new");
  });
});
