// @vitest-environment jsdom

import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, describe, expect, it, vi } from "vitest";
import type { ContextInvalidationResponse, ContextSearchResponse } from "../../protocol/generated/v1";
import { ContextSearch } from "./ContextSearch";

type Deferred<T> = { promise: Promise<T>; resolve: (value: T) => void };

function deferred<T>(): Deferred<T> {
  let resolve!: (value: T) => void;
  const promise = new Promise<T>((nextResolve) => { resolve = nextResolve; });
  return { promise, resolve };
}

let container: HTMLDivElement;
let root: Root;

afterEach(() => {
  act(() => root.unmount());
  container.remove();
  vi.useRealTimers();
});

async function settle(): Promise<void> {
  await act(async () => { await Promise.resolve(); });
}

describe("ContextSearch", () => {
  it("ignores an aborted late search response", async () => {
    Object.assign(globalThis, { IS_REACT_ACT_ENVIRONMENT: true });
    container = document.createElement("div");
    document.body.append(container);
    root = createRoot(container);
    const first = deferred<ContextSearchResponse>();
    const second = deferred<ContextSearchResponse>();
    const search = vi.fn()
      .mockReturnValueOnce(first.promise)
      .mockReturnValueOnce(second.promise);
    act(() => {
      root.render(<ContextSearch
        identity="session\u0000pane"
        bindingId="binding"
        rootId="root"
        known={[]}
        search={search}
        poll={vi.fn()}
        onSelect={vi.fn()}
        onInvalidate={vi.fn()}
      />);
    });
    const input = container.querySelector<HTMLInputElement>('input[type="search"]')!;
    const form = container.querySelector("form")!;
    act(() => {
      Object.getOwnPropertyDescriptor(HTMLInputElement.prototype, "value")?.set?.call(input, "first");
      input.dispatchEvent(new Event("input", { bubbles: true }));
    });
    act(() => {
      form.dispatchEvent(new Event("submit", { bubbles: true, cancelable: true }));
    });
    await settle();
    act(() => {
      Object.getOwnPropertyDescriptor(HTMLInputElement.prototype, "value")?.set?.call(input, "second");
      input.dispatchEvent(new Event("input", { bubbles: true }));
    });
    act(() => {
      form.dispatchEvent(new Event("submit", { bubbles: true, cancelable: true }));
    });
    await settle();
    const firstRequest = search.mock.calls[0]?.[0] as { request_generation: number };
    const secondRequest = search.mock.calls[1]?.[0] as { request_generation: number };
    second.resolve({ binding_id: "binding", root_id: "root", query: "second", request_generation: secondRequest.request_generation, results: [{ path: "second.md", line: 2, excerpt: "second", revision: "r2" }], scanned_files: 1, truncated: false });
    await settle();
    first.resolve({ binding_id: "binding", root_id: "root", query: "first", request_generation: firstRequest.request_generation, results: [{ path: "first.md", line: 1, excerpt: "first", revision: "r1" }], scanned_files: 1, truncated: false });
    await settle();
    expect(container.textContent).toContain("second.md:2");
    expect(container.textContent).not.toContain("first.md:1");
  });

  it("keeps one visible-file poll pending and ignores an old identity response", async () => {
    Object.assign(globalThis, { IS_REACT_ACT_ENVIRONMENT: true });
    vi.useFakeTimers();
    container = document.createElement("div");
    document.body.append(container);
    root = createRoot(container);
    const first = deferred<ContextInvalidationResponse>();
    const second = deferred<ContextInvalidationResponse>();
    const poll = vi.fn()
      .mockReturnValueOnce(first.promise)
      .mockReturnValueOnce(second.promise);
    const onInvalidate = vi.fn();
    const known = [{ path: "visible.md", revision: "r1" }];
    act(() => {
      root.render(<ContextSearch
        identity="session\u0000pane-a"
        bindingId="binding"
        rootId="root"
        known={known}
        search={vi.fn()}
        poll={poll}
        onSelect={vi.fn()}
        onInvalidate={onInvalidate}
      />);
    });
    await settle();
    expect(poll).toHaveBeenCalledTimes(1);
    await act(async () => { await vi.advanceTimersByTimeAsync(9_000); });
    expect(poll).toHaveBeenCalledTimes(1);

    act(() => {
      root.render(<ContextSearch
        identity="session\u0000pane-b"
        bindingId="binding"
        rootId="root"
        known={known}
        search={vi.fn()}
        poll={poll}
        onSelect={vi.fn()}
        onInvalidate={onInvalidate}
      />);
    });
    await settle();
    expect(poll).toHaveBeenCalledTimes(2);
    const firstRequest = poll.mock.calls[0]?.[0] as { request_generation: number };
    const secondRequest = poll.mock.calls[1]?.[0] as { request_generation: number };
    first.resolve({
      binding_id: "binding",
      root_id: "root",
      request_generation: firstRequest.request_generation,
      invalidations: [{ path: "visible.md", state: "changed", revision: "r2" }],
      truncated: false,
    });
    await settle();
    expect(onInvalidate).not.toHaveBeenCalled();
    second.resolve({
      binding_id: "binding",
      root_id: "root",
      request_generation: secondRequest.request_generation,
      invalidations: [{ path: "visible.md", state: "changed", revision: "r2" }],
      truncated: false,
    });
    await settle();
    expect(onInvalidate).toHaveBeenCalledTimes(1);
  });
});
