// @vitest-environment jsdom
import { afterEach, expect, it, vi } from "vitest";
import type { CockpitClient, CockpitClientError, LocalChangeEvent, LocalChangeScope } from "../client/CockpitClient";
import { subscribeLocalChanges } from "./localChanges";

afterEach(() => { vi.useRealTimers(); vi.restoreAllMocks(); });
const scope = { kind: "library_listing" } as const;
const ready: LocalChangeEvent = { stream_id: "one", sequence: 1, kind: "ready", topics: [] };
const flush = async () => { for (let i = 0; i < 10; i++) await Promise.resolve(); };

it("shares a scope, waits for every consumer and releases the last reference", async () => {
  let event!: (event: LocalChangeEvent) => Promise<void>;
  let signal!: AbortSignal;
  const close = vi.fn();
  const subscribe = vi.fn(async (_scope: LocalChangeScope, onEvent: (event: LocalChangeEvent) => Promise<void>, _onError: (error: CockpitClientError) => void, requestSignal?: AbortSignal) => { event = onEvent; signal = requestSignal!; return { close }; });
  const client = { subscribeLocalChanges: subscribe } as unknown as CockpitClient;
  let finish!: () => void;
  const pending = new Promise<void>(resolve => { finish = resolve; });
  const first = vi.fn(async () => {}), second = vi.fn(() => pending);
  const a = subscribeLocalChanges(client, scope, first, vi.fn());
  const b = subscribeLocalChanges(client, scope, second, vi.fn());
  await flush();
  expect(subscribe).toHaveBeenCalledOnce();
  let completed = false;
  void event(ready).then(() => { completed = true; });
  await flush();
  expect(first).toHaveBeenCalledOnce(); expect(second).toHaveBeenCalledOnce();
  expect(completed).toBe(false);
  finish(); await flush(); expect(completed).toBe(true);
  vi.useFakeTimers();
  await vi.advanceTimersByTimeAsync(120_000);
  expect(subscribe).toHaveBeenCalledOnce();
  a.close(); expect(signal.aborted).toBe(false);
  b.close(); expect(signal.aborted).toBe(true);
});

it("stops while hidden and starts a new baseline on restoration", async () => {
  let visible: DocumentVisibilityState = "visible";
  vi.spyOn(document, "visibilityState", "get").mockImplementation(() => visible);
  const signals: AbortSignal[] = [];
  const subscribe = vi.fn(async (_scope: LocalChangeScope, _event: (event: LocalChangeEvent) => Promise<void>, _error: (error: CockpitClientError) => void, signal?: AbortSignal) => { signals.push(signal!); return { close: vi.fn() }; });
  const client = { subscribeLocalChanges: subscribe } as unknown as CockpitClient;
  const handle = subscribeLocalChanges(client, scope, async () => {}, vi.fn());
  await flush();
  visible = "hidden"; document.dispatchEvent(new Event("visibilitychange"));
  expect(signals[0].aborted).toBe(true);
  visible = "visible"; document.dispatchEvent(new Event("visibilitychange")); await flush();
  expect(subscribe).toHaveBeenCalledTimes(2);
  handle.close();
});

it("retries failures four times and stops until explicit retry", async () => {
  const callbacks: ((error: CockpitClientError) => void)[] = [];
  const subscribe = vi.fn(async (_scope: LocalChangeScope, _event: (event: LocalChangeEvent) => Promise<void>, onError: (error: CockpitClientError) => void) => { callbacks.push(onError); return { close: vi.fn() }; });
  const client = { subscribeLocalChanges: subscribe } as unknown as CockpitClient;
  const onError = vi.fn();
  const handle = subscribeLocalChanges(client, scope, async () => {}, onError);
  await flush(); vi.useFakeTimers();
  for (const delay of [1500, 3000, 6000, 12000]) {
    callbacks[callbacks.length - 1](new Error("offline") as CockpitClientError);
    await vi.advanceTimersByTimeAsync(delay);
  }
  callbacks[callbacks.length - 1](new Error("offline") as CockpitClientError);
  await vi.advanceTimersByTimeAsync(120_000);
  expect(subscribe).toHaveBeenCalledTimes(5);
  handle.retry(); await flush();
  expect(subscribe).toHaveBeenCalledTimes(6);
  handle.close();
});

it("renews the retry budget after each reconciled baseline", async () => {
  const events: ((event: LocalChangeEvent) => Promise<void>)[] = [];
  const failures: ((error: CockpitClientError) => void)[] = [];
  const subscribe = vi.fn(async (_scope: LocalChangeScope, event: (event: LocalChangeEvent) => Promise<void>, failure: (error: CockpitClientError) => void) => {
    events.push(event); failures.push(failure); return { close: vi.fn() };
  });
  const onEvent = vi.fn(async (_event: LocalChangeEvent) => {});
  const handle = subscribeLocalChanges({ subscribeLocalChanges: subscribe } as unknown as CockpitClient, scope, onEvent, vi.fn());
  await flush();
  vi.useFakeTimers();
  for (let i = 0; i < 7; i++) {
    await events[i]({ ...ready, stream_id: `stream-${i}` });
    failures[i](new Error("watch epoch ended") as CockpitClientError);
    await vi.advanceTimersByTimeAsync(1500);
    expect(subscribe).toHaveBeenCalledTimes(i + 2);
  }
  await events[7]({ ...ready, stream_id: "stream-7" });
  await events[7]({ stream_id: "stream-7", sequence: 2, kind: "changed", topics: ["listing"] });
  expect(onEvent).toHaveBeenLastCalledWith(expect.objectContaining({ kind: "changed" }));
  handle.close();
});

it("does not reset the retry budget when baseline reconciliation rejects", async () => {
  const events: ((event: LocalChangeEvent) => Promise<void>)[] = [];
  const failures: ((error: CockpitClientError) => void)[] = [];
  const subscribe = vi.fn(async (_scope: LocalChangeScope, event: (event: LocalChangeEvent) => Promise<void>, failure: (error: CockpitClientError) => void) => {
    events.push(event); failures.push(failure); return { close: vi.fn() };
  });
  const handle = subscribeLocalChanges(
    { subscribeLocalChanges: subscribe } as unknown as CockpitClient,
    scope, async () => { throw new Error("read failed"); }, vi.fn(),
  );
  await flush();
  vi.useFakeTimers();
  for (const [i, delay] of [1500, 3000, 6000, 12000].entries()) {
    await events[i](ready);
    failures[i](new Error("watch failed") as CockpitClientError);
    await vi.advanceTimersByTimeAsync(delay);
  }
  await events[4](ready);
  failures[4](new Error("watch failed") as CockpitClientError);
  await vi.advanceTimersByTimeAsync(120_000);
  expect(subscribe).toHaveBeenCalledTimes(5);
  handle.close();
});
