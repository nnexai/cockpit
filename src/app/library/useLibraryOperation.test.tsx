// @vitest-environment jsdom
import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import type { CockpitClient } from "../../client/CockpitClient";
import type { LibraryItemSummary, LibraryListing, LibraryOperation, SpaceContextListing } from "../../protocol/generated/v1";
import { LIBRARY_CHANGED_EVENT, useLibraryListing, useLibraryOperation, useSpaceContextListing, type LibraryChangeDetail, type LibraryListingState } from "./useLibraryOperation";

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

  async function advance(milliseconds = 60_000) {
    await act(async () => { await vi.advanceTimersByTimeAsync(milliseconds); });
  }

  it("probes only the first page of an unchanged generation without replacing or rerendering the listing", async () => {
    const { client, libraryListing } = fixture();
    const first = page("g1", ["a"], 256);
    libraryListing.mockResolvedValueOnce(first).mockResolvedValueOnce(page("g1", ["b"])).mockResolvedValue(first);
    const changed = observeChanges();
    const display = await view(client);
    const before = display.state.listing;
    const renders = display.renders.length;
    expect(libraryListing.mock.calls).toEqual([[null], [256]]);
    await advance(59_999);
    expect(libraryListing).toHaveBeenCalledTimes(2);
    await advance(1);
    await advance();
    expect(libraryListing.mock.calls).toEqual([[null], [256], [null], [null]]);
    expect(display.state.listing).toBe(before);
    expect(display.renders).toHaveLength(renders);
    expect(changed).not.toHaveBeenCalled();
  });

  it("reuses the changed probe page and atomically publishes all pages with one non-looping notification", async () => {
    const { client, libraryListing } = fixture();
    libraryListing.mockResolvedValueOnce(page("g1", ["old", "removed"]));
    const changed = observeChanges();
    const display = await view(client);
    const second = deferred();
    libraryListing.mockResolvedValueOnce(page("g2", ["new-a"], 256)).mockReturnValueOnce(second.promise)
      .mockResolvedValueOnce(page("g2", ["new-c"]));
    await advance();
    expect(display.state.status).toBe("ready");
    expect(display.host.querySelector("span")?.getAttribute("data-items")).toBe("old,removed");
    expect(changed).not.toHaveBeenCalled();
    await act(async () => second.resolve(page("g2", ["new-b"], 512)));
    expect(display.host.querySelector("span")?.getAttribute("data-items")).toBe("new-a,new-b,new-c");
    expect(display.state.listing?.next_offset).toBeNull();
    expect(libraryListing.mock.calls).toEqual([[null], [null], [256], [512]]);
    expect(changed).toHaveBeenCalledOnce();
  });

  it.each([
    { stage: "probe", failure: false },
    { stage: "probe", failure: true },
    { stage: "remaining pages", failure: false },
    { stage: "remaining pages", failure: true },
  ])("ignores an older $stage response or error after manual reload (failure=$failure)", async ({ stage, failure }) => {
    const { client, libraryListing } = fixture();
    libraryListing.mockResolvedValueOnce(page("g1", ["initial"]));
    const changed = observeChanges();
    const display = await view(client);
    const older = deferred();
    if (stage === "remaining pages") libraryListing.mockResolvedValueOnce(page("g2", ["stale-a"], 256));
    libraryListing.mockReturnValueOnce(older.promise);
    await advance();
    libraryListing.mockResolvedValueOnce(page("g3", ["current"]));
    await act(async () => display.host.querySelector("button")!.click());
    const before = display.state.listing;
    await act(async () => {
      if (failure) older.reject(new Error("stale failure"));
      else older.resolve(page("g2", ["stale-b"], 512));
    });
    expect(display.state.listing).toBe(before);
    expect(display.state.listing?.generation).toBe("g3");
    expect(display.state.status).toBe("ready");
    expect(display.state.error).toBeNull();
    expect(libraryListing.mock.calls).toEqual(stage === "probe" ? [[null], [null], [null]] : [[null], [null], [256], [null]]);
    expect(changed).not.toHaveBeenCalled();
    // The comparison baseline must also be the newly applied manual listing.
    libraryListing.mockResolvedValueOnce(page("g3", ["current"], 256));
    await advance();
    expect(display.state.listing).toBe(before);
    expect(libraryListing.mock.calls.at(-1)).toEqual([null]);
    expect(libraryListing).toHaveBeenCalledTimes(stage === "probe" ? 4 : 5);
  });

  it("stops an obsolete initial paginated read after a newer manual reload", async () => {
    const { client, libraryListing } = fixture();
    const older = deferred();
    libraryListing.mockResolvedValueOnce(page("g1", ["old-a"], 256)).mockReturnValueOnce(older.promise);
    const display = await view(client);
    libraryListing.mockResolvedValueOnce(page("g2", ["current"]));
    await act(async () => display.host.querySelector("button")!.click());
    await act(async () => older.resolve(page("g1", ["old-b"], 512)));
    expect(display.state.listing?.items.map((item) => item.item_id)).toEqual(["current"]);
    expect(libraryListing.mock.calls).toEqual([[null], [256], [null]]);
  });

  it("restarts once when the generation moves between pages without publishing mixed generations", async () => {
    const { client, libraryListing } = fixture();
    libraryListing.mockResolvedValueOnce(page("g1", ["old"]));
    const display = await view(client);
    const changed = observeChanges();
    const final = deferred();
    libraryListing.mockResolvedValueOnce(page("g2", ["discard-a"], 256))
      .mockResolvedValueOnce(page("g3", ["discard-b"]))
      .mockResolvedValueOnce(page("g3", ["current-a"], 256))
      .mockReturnValueOnce(final.promise);
    await advance();
    expect(display.host.querySelector("span")?.getAttribute("data-items")).toBe("old");
    expect(changed).not.toHaveBeenCalled();
    await act(async () => final.resolve(page("g3", ["current-b"])));
    expect(display.state.listing?.generation).toBe("g3");
    expect(display.host.querySelector("span")?.getAttribute("data-items")).toBe("current-a,current-b");
    expect(libraryListing.mock.calls).toEqual([[null], [null], [256], [null], [256]]);
    expect(changed).toHaveBeenCalledOnce();
  });

  it("retains the last coherent listing and reports an error when both pagination attempts move", async () => {
    const { client, libraryListing } = fixture();
    libraryListing.mockResolvedValueOnce(page("g1", ["old"]));
    const display = await view(client);
    const before = display.state.listing;
    const changed = observeChanges();
    libraryListing.mockResolvedValueOnce(page("g2", ["discard-a"], 256))
      .mockResolvedValueOnce(page("g3", ["discard-b"]))
      .mockResolvedValueOnce(page("g3", ["discard-c"], 256))
      .mockResolvedValueOnce(page("g4", ["discard-d"]));
    await advance();
    expect(display.state.listing).toBe(before);
    expect(display.state.status).toBe("error");
    expect(display.state.error).not.toBeNull();
    expect(libraryListing.mock.calls).toEqual([[null], [null], [256], [null], [256]]);
    expect(changed).not.toHaveBeenCalled();
  });

  it("does not overlap an unfinished initial read or an unfinished probe", async () => {
    const { client, libraryListing } = fixture();
    const initial = deferred();
    libraryListing.mockReturnValueOnce(initial.promise);
    const display = await view(client);
    await advance(120_000);
    expect(libraryListing.mock.calls).toEqual([[null]]);
    await act(async () => initial.resolve(page("g1", ["old"])));
    const probe = deferred();
    libraryListing.mockReturnValueOnce(probe.promise);
    await advance();
    await advance(120_000);
    expect(libraryListing.mock.calls).toEqual([[null], [null]]);
    await act(async () => probe.resolve(page("g1", ["old"])));
    expect(display.state.status).toBe("ready");
    libraryListing.mockResolvedValueOnce(page("g1", ["old"]));
    await advance();
    expect(libraryListing.mock.calls).toEqual([[null], [null], [null]]);
  });

  it("defers a due hidden-window probe until visibility resumes and removes the deferred listener on unmount", async () => {
    const { client, libraryListing } = fixture();
    libraryListing.mockResolvedValue(page("g1", ["old"]));
    const display = await view(client);
    visibility("hidden");
    await advance(120_000);
    await act(async () => document.dispatchEvent(new Event("visibilitychange")));
    expect(libraryListing.mock.calls).toEqual([[null]]);
    visibility("visible");
    await act(async () => document.dispatchEvent(new Event("visibilitychange")));
    expect(libraryListing.mock.calls).toEqual([[null], [null]]);
    await act(async () => document.dispatchEvent(new Event("visibilitychange")));
    expect(libraryListing).toHaveBeenCalledTimes(2);
    visibility("hidden");
    await advance();
    await display.unmount();
    visibility("visible");
    await act(async () => document.dispatchEvent(new Event("visibilitychange")));
    await advance(120_000);
    expect(libraryListing).toHaveBeenCalledTimes(2);
  });

  it("does not probe early on visibility changes before its minute is due", async () => {
    const { client, libraryListing } = fixture();
    libraryListing.mockResolvedValue(page("g1", ["old"]));
    await view(client);
    await advance(30_000);
    visibility("hidden");
    await act(async () => document.dispatchEvent(new Event("visibilitychange")));
    visibility("visible");
    await act(async () => document.dispatchEvent(new Event("visibilitychange")));
    expect(libraryListing).toHaveBeenCalledTimes(1);
    await advance(30_000);
    expect(libraryListing).toHaveBeenCalledTimes(2);
  });

  it.each([
    { boundary: "inactive", stage: "probe" },
    { boundary: "inactive", stage: "remaining pages" },
    { boundary: "unmount", stage: "probe" },
    { boundary: "unmount", stage: "remaining pages" },
  ])("stops a pending $stage at the $boundary boundary without requesting further pages", async ({ boundary, stage }) => {
    const { client, libraryListing } = fixture();
    libraryListing.mockResolvedValueOnce(page("g1", ["old"]));
    const display = await view(client);
    const changed = observeChanges();
    const pending = deferred();
    if (stage === "remaining pages") libraryListing.mockResolvedValueOnce(page("g2", ["new-a"], 256));
    libraryListing.mockReturnValueOnce(pending.promise);
    await advance();
    if (boundary === "inactive") await display.render(client, false);
    else await display.unmount();
    await act(async () => pending.resolve(page("g2", ["new-b"], 512)));
    await act(async () => window.dispatchEvent(new Event(LIBRARY_CHANGED_EVENT)));
    await advance(120_000);
    expect(libraryListing.mock.calls).toEqual(stage === "probe" ? [[null], [null]] : [[null], [null], [256]]);
    // Only the externally dispatched event was observed, not a stale poll announcement.
    expect(changed).toHaveBeenCalledOnce();
    if (boundary === "inactive") {
      expect(display.state.listing?.generation).toBe("g1");
      libraryListing.mockResolvedValueOnce(page("g3", ["reactivated"]));
      await display.render(client, true);
      expect(display.state.listing?.generation).toBe("g3");
      libraryListing.mockResolvedValueOnce(page("g3", ["reactivated"]));
      await advance();
      expect(libraryListing.mock.calls.at(-1)).toEqual([null]);
    }
  });

  it("does no work while initially inactive, then loads and probes on activation", async () => {
    const { client, libraryListing } = fixture();
    const display = await view(client, false);
    await advance(120_000);
    await act(async () => window.dispatchEvent(new Event(LIBRARY_CHANGED_EVENT)));
    expect(libraryListing).not.toHaveBeenCalled();
    expect(client.projectConfiguration).not.toHaveBeenCalled();
    libraryListing.mockResolvedValue(page("g1", ["active"]));
    await display.render(client, true);
    expect(display.state.status).toBe("ready");
    await advance();
    expect(libraryListing.mock.calls).toEqual([[null], [null]]);
  });

  it("drops an old client's pending probe and recovers a new client even if its generation matches the old listing", async () => {
    const old = fixture();
    old.libraryListing.mockResolvedValueOnce(page("same-generation", ["old-client"]));
    const display = await view(old.client);
    const pending = deferred();
    old.libraryListing.mockReturnValueOnce(pending.promise);
    await advance();
    const next = fixture();
    next.libraryListing.mockRejectedValueOnce(new Error("new client offline"));
    await display.render(next.client, true);
    expect(display.state.status).toBe("error");
    await act(async () => pending.resolve(page("stale-generation", ["stale"], 256)));
    expect(old.libraryListing.mock.calls).toEqual([[null], [null]]);
    expect(display.state.error).not.toBeNull();
    next.libraryListing.mockResolvedValueOnce(page("same-generation", ["new-client"]));
    await advance();
    expect(display.state.status).toBe("ready");
    expect(display.state.error).toBeNull();
    expect(display.state.listing?.items.map((item) => item.item_id)).toEqual(["new-client"]);
    expect(next.libraryListing.mock.calls).toEqual([[null], [null]]);
  });

  it.each(["probe", "remaining pages"])("reports a current $stage failure without erasing good data and recovers on a changed generation", async (stage) => {
    const { client, libraryListing } = fixture();
    libraryListing.mockResolvedValueOnce(page("g1", ["old"]));
    const display = await view(client);
    const before = display.state.listing;
    const changed = observeChanges();
    if (stage === "remaining pages") libraryListing.mockResolvedValueOnce(page("g2", ["incomplete"], 256));
    libraryListing.mockRejectedValueOnce(new Error("offline"));
    await advance();
    expect(display.state.listing).toBe(before);
    expect(display.state.status).toBe("error");
    expect(display.host.textContent).toContain("offline");
    expect(changed).not.toHaveBeenCalled();
    // An unchanged generation must not erase the error or churn the existing listing.
    libraryListing.mockResolvedValueOnce(page("g1", ["old"]));
    const renders = display.renders.length;
    await advance();
    expect(display.state.status).toBe("error");
    expect(display.state.listing).toBe(before);
    expect(display.renders).toHaveLength(renders);
    libraryListing.mockResolvedValueOnce(page("g2", ["recovered"], 256)).mockResolvedValueOnce(page("g2", ["rest"]));
    await advance();
    expect(display.state.status).toBe("ready");
    expect(display.state.error).toBeNull();
    expect(display.host.querySelector("span")?.getAttribute("data-items")).toBe("recovered,rest");
    expect(changed).toHaveBeenCalledOnce();
  });

  it("recovers an initial load failure through the generation probe", async () => {
    const { client, libraryListing } = fixture();
    libraryListing.mockRejectedValueOnce(new Error("initially offline"));
    const display = await view(client);
    expect(display.state.status).toBe("error");
    expect(display.state.listing).toBeNull();
    libraryListing.mockResolvedValueOnce(page("g1", ["recovered"], 256)).mockResolvedValueOnce(page("g1", ["rest"]));
    await advance();
    expect(display.state.status).toBe("ready");
    expect(display.state.error).toBeNull();
    expect(display.host.querySelector("span")?.getAttribute("data-items")).toBe("recovered,rest");
    expect(libraryListing.mock.calls).toEqual([[null], [null], [256]]);
  });

  it("keeps external event reloads intact and clears an earlier current error on successful manual reload", async () => {
    const { client, libraryListing } = fixture();
    libraryListing.mockResolvedValueOnce(page("g1", ["old"]));
    const display = await view(client);
    libraryListing.mockRejectedValueOnce(new Error("offline"));
    await advance();
    expect(display.state.status).toBe("error");
    libraryListing.mockResolvedValueOnce(page("g2", ["manual"]));
    await act(async () => display.host.querySelector("button")!.click());
    expect(display.state.status).toBe("ready");
    expect(display.state.error).toBeNull();
    libraryListing.mockResolvedValueOnce(page("g3", ["external-a"], 256)).mockResolvedValueOnce(page("g3", ["external-b"]));
    await act(async () => window.dispatchEvent(new Event(LIBRARY_CHANGED_EVENT)));
    expect(display.host.querySelector("span")?.getAttribute("data-items")).toBe("external-a,external-b");
    expect(libraryListing.mock.calls).toEqual([[null], [null], [null], [null], [256]]);
  });

  it("refreshes another mounted listing once and supersedes its pending probe without a notification loop", async () => {
    const first = fixture();
    const second = fixture();
    first.libraryListing.mockResolvedValueOnce(page("g1", ["old"]));
    second.libraryListing.mockResolvedValueOnce(page("g1", ["old"]));
    const firstDisplay = await view(first.client);
    const secondDisplay = await view(second.client);
    const firstProbe = deferred();
    const secondProbe = deferred();
    first.libraryListing.mockReturnValueOnce(firstProbe.promise);
    second.libraryListing.mockReturnValueOnce(secondProbe.promise);
    await advance();
    const changed = observeChanges();
    second.libraryListing.mockResolvedValueOnce(page("g2", ["updated"]));
    await act(async () => firstProbe.resolve(page("g2", ["updated"])));
    expect(firstDisplay.state.listing?.generation).toBe("g2");
    expect(secondDisplay.state.listing?.generation).toBe("g2");
    await act(async () => secondProbe.resolve(page("stale", ["discard"], 256)));
    expect(secondDisplay.state.listing?.generation).toBe("g2");
    expect(first.libraryListing.mock.calls).toEqual([[null], [null]]);
    expect(second.libraryListing.mock.calls).toEqual([[null], [null], [null]]);
    expect(changed).toHaveBeenCalledOnce();
    first.libraryListing.mockResolvedValueOnce(page("g2", ["updated"]));
    second.libraryListing.mockResolvedValueOnce(page("g2", ["updated"]));
    await advance();
    expect(first.libraryListing.mock.calls).toEqual([[null], [null], [null]]);
    expect(second.libraryListing.mock.calls).toEqual([[null], [null], [null], [null]]);
    expect(changed).toHaveBeenCalledOnce();
  });
});
