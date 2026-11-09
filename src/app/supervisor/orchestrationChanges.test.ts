// @vitest-environment jsdom
import { afterEach, expect, it, vi } from "vitest";
import type { CockpitClient, LocalChangeEvent } from "../../client/CockpitClient";
import { orchestrationChanges } from "./orchestrationChanges";

const scope = { kind: "orchestration", session_id: "session", server_instance: "server", state_store_id: "store", root_id: null } as const;
const ready: LocalChangeEvent = { stream_id: "stream", sequence: 1, kind: "ready", topics: ["orchestration"] };
const flush = async () => { for (let i = 0; i < 12; i++) await Promise.resolve(); };
afterEach(() => { vi.useRealTimers(); vi.restoreAllMocks(); });

it("keeps a healthy stream quiet and serializes a change during a read", async () => {
  vi.useFakeTimers();
  let emit!: Parameters<CockpitClient["subscribeLocalChanges"]>[1];
  let finish!: () => void;
  const pending = new Promise<void>(resolve => { finish = resolve; });
  const read = vi.fn().mockResolvedValueOnce(undefined).mockReturnValueOnce(pending).mockResolvedValue(undefined);
  const subscribe = vi.fn<CockpitClient["subscribeLocalChanges"]>(async (_scope, onEvent) => { emit = onEvent; return { close: vi.fn() }; });
  const changes = orchestrationChanges({ subscribeLocalChanges: subscribe }, async () => scope, read, vi.fn());
  await flush(); await emit(ready); await vi.advanceTimersByTimeAsync(100_000);
  expect(subscribe).toHaveBeenCalledTimes(1); expect(read).toHaveBeenCalledTimes(1);
  const first = emit({ ...ready, sequence: 2, kind: "changed" }); await flush();
  const second = changes.refresh(); expect(read).toHaveBeenCalledTimes(2);
  finish(); await Promise.all([first, second]); expect(read).toHaveBeenCalledTimes(3);
  expect(subscribe).toHaveBeenCalledTimes(1); changes.close();
});

it("retries failed reads without crediting them and reconnects with a fresh ready", async () => {
  vi.useFakeTimers();
  let emit!: Parameters<CockpitClient["subscribeLocalChanges"]>[1];
  const close = vi.fn(), unavailable = vi.fn();
  const subscribe = vi.fn<CockpitClient["subscribeLocalChanges"]>(async (_scope, onEvent) => { emit = onEvent; return { close }; });
  const read = vi.fn().mockRejectedValueOnce(new Error("offline")).mockResolvedValue(undefined);
  const changes = orchestrationChanges({ subscribeLocalChanges: subscribe }, async () => scope, read, unavailable);
  await flush(); await expect(emit(ready)).rejects.toThrow("offline");
  expect(unavailable).toHaveBeenCalledOnce(); expect(close).toHaveBeenCalledOnce();
  await vi.advanceTimersByTimeAsync(1500); await flush(); expect(subscribe).toHaveBeenCalledTimes(2);
  await emit(ready); expect(read).toHaveBeenCalledTimes(2); changes.close();
});

it("closes a late opening and ignores its events", async () => {
  let finish!: (stream: { close(): void }) => void;
  let emit!: Parameters<CockpitClient["subscribeLocalChanges"]>[1];
  const close = vi.fn(), read = vi.fn();
  const subscribe = vi.fn<CockpitClient["subscribeLocalChanges"]>((_scope, onEvent) => { emit = onEvent; return new Promise<{ close(): void }>(resolve => { finish = resolve; }); });
  const changes = orchestrationChanges({ subscribeLocalChanges: subscribe }, async () => scope, read, vi.fn());
  await flush(); changes.close(); finish({ close }); await flush(); await emit(ready);
  expect(close).toHaveBeenCalledOnce(); expect(read).not.toHaveBeenCalled();
});

it("keeps healthy hidden subscriptions and pauses error retries while hidden", async () => {
  vi.useFakeTimers();
  const visibility = Object.getOwnPropertyDescriptor(document, "visibilityState");
  let fail!: Parameters<CockpitClient["subscribeLocalChanges"]>[2];
  const close = vi.fn();
  const subscribe = vi.fn<CockpitClient["subscribeLocalChanges"]>(async (_scope, onEvent, onError) => { fail = onError; void onEvent(ready); return { close }; });
  const changes = orchestrationChanges({ subscribeLocalChanges: subscribe }, async () => scope, vi.fn(async () => {}), vi.fn());
  try {
    Object.defineProperty(document, "visibilityState", { configurable: true, value: "hidden" });
    await flush(); await vi.advanceTimersByTimeAsync(100_000);
    expect(subscribe).toHaveBeenCalledTimes(1); expect(close).not.toHaveBeenCalled();
    fail(new Error("offline") as Parameters<typeof fail>[0]); await vi.advanceTimersByTimeAsync(100_000);
    expect(subscribe).toHaveBeenCalledTimes(1);
    Object.defineProperty(document, "visibilityState", { configurable: true, value: "visible" });
    document.dispatchEvent(new Event("visibilitychange"));
    await vi.advanceTimersByTimeAsync(1500); await flush();
    expect(subscribe).toHaveBeenCalledTimes(2);
  } finally {
    changes.close();
    if (visibility) Object.defineProperty(document, "visibilityState", visibility); else Reflect.deleteProperty(document, "visibilityState");
  }
});
