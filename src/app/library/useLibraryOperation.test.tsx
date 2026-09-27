// @vitest-environment jsdom
import { act } from "react";
import { createRoot } from "react-dom/client";
import { afterEach, expect, it, vi } from "vitest";
import type { CockpitClient } from "../../client/CockpitClient";
import type { LibraryListing, LibraryOperation } from "../../protocol/generated/v1";
import { LIBRARY_CHANGED_EVENT, useLibraryListing, useLibraryOperation } from "./useLibraryOperation";

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
