// @vitest-environment jsdom
import { act } from "react";
import { createRoot } from "react-dom/client";
import { expect, it, vi } from "vitest";
import type { ContextFileIndex, ContextRoot, LocalChangeEvent } from "../../protocol/generated/v1";
import type { ContextReader } from "./contextSource";
import { useFilePicker } from "./useFilePicker";

it("reconciles a change during a slow index read without periodic revalidation and releases the watch", async () => {
  Object.assign(globalThis, { IS_REACT_ACT_ENVIRONMENT: true });
  vi.useFakeTimers();
  const host = document.createElement("div"); document.body.append(host);
  const mounted = createRoot(host);
  const root = { root_id: "picker-test-root" } as ContextRoot;
  const index = (path: string) => ({ root_id: root.root_id, state: "fresh", files: [{ path, bytes: null }], truncated: false } as ContextFileIndex);
  let resolve!: (value: ContextFileIndex) => void;
  const pending = new Promise<ContextFileIndex>((nextResolve) => { resolve = nextResolve; });
  let fresh = 0;
  let emit!: (event: LocalChangeEvent) => Promise<void>;
  const close = vi.fn();
  const reader = {
    identity: "picker",
    fileIndex: vi.fn(async (_id: string, mode: string) => {
      if (mode === "cached") return { ...index("cached.md"), state: "cached" };
      fresh++;
      if (fresh === 1) return index("warm.md");
      if (fresh === 2) return pending;
      return index("new.md");
    }),
    watchIndex: vi.fn((_root: string, callback: typeof emit) => { emit = callback; return { close, retry: vi.fn() }; }),
  } as unknown as ContextReader;
  let picker!: { openFilePicker(): void; closeFilePicker(): void; pickerIndex: { files: readonly { path: string }[] } };
  function Harness() { picker = useFilePicker({ reader, root, bindingId: "binding", identityKey: "picker" }); return <div>{picker.pickerIndex.files.map((f) => f.path).join(",")}</div>; }
  const event = (kind: "ready" | "changed", sequence: number): LocalChangeEvent => ({ stream_id: "s", sequence, kind, topics: ["index"] });
  try {
    await act(async () => { mounted.render(<Harness />); });
    await act(async () => { picker.openFilePicker(); });
    let ready!: Promise<void>;
    await act(async () => { ready = emit(event("ready", 1)); await Promise.resolve(); });
    await act(async () => { void emit(event("changed", 2)); await vi.advanceTimersByTimeAsync(11_000); });
    expect(fresh).toBe(2);
    await act(async () => { resolve(index("old.md")); await ready; });
    expect(host.textContent).toContain("new.md");
    await act(async () => { await vi.advanceTimersByTimeAsync(120_000); });
    expect(fresh).toBe(3);
    await act(async () => picker.closeFilePicker());
    expect(close).toHaveBeenCalledOnce();
  } finally { await act(async () => mounted.unmount()); host.remove(); vi.useRealTimers(); }
});
