// @vitest-environment jsdom
import { describe, expect, it, vi } from "vitest";
import { limitLocalChangeSubscriptions, localChangeReceiver, parseLocalChangeEvent } from "./localChanges";
import { createBrowserClient, type BrowserWebSocket } from "./browser";
import { createNativeClient, type NativeChannel } from "./native";

const ready = { stream_id: "stream", sequence: 1, kind: "ready", topics: ["orchestration"] } as const;
const scope = { kind: "orchestration", session_id: "session", server_instance: "server", state_store_id: "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef", root_id: null } as const;
const flush = async () => { for (let i = 0; i < 8; i++) await Promise.resolve(); };

describe("local changes transport", () => {
  it("rejects malformed, excessive and prototype topics", () => {
    expect(() => parseLocalChangeEvent({ ...ready, topics: ["__proto__"] })).toThrow();
    expect(() => parseLocalChangeEvent({ ...ready, topics: ["index", "index"] })).toThrow();
    expect(() => parseLocalChangeEvent({ ...ready, sequence: 0 })).toThrow();
    expect(() => parseLocalChangeEvent({ ...ready, reason: null })).toThrow();
    expect(parseLocalChangeEvent({ ...ready, topics: ["orchestration"] }).topics).toEqual(["orchestration"]);
  });
  it("credits only after the consumer and rejects sequence jumps", async () => {
    let finish!: () => void;
    const pending = new Promise<void>(resolve => { finish = resolve; });
    const ack = vi.fn(async () => {}), fail = vi.fn(), opened = vi.fn();
    const receive = localChangeReceiver(() => pending, ack, fail, opened);
    receive(ready);
    await flush();
    expect(ack).not.toHaveBeenCalled();
    finish(); await flush();
    expect(ack).toHaveBeenCalledWith({ stream_id: "stream", sequence: 1 });
    receive({ ...ready, sequence: 3, kind: "changed" });
    expect(fail).toHaveBeenCalledOnce();
  });
  it("bounds simultaneous subscriptions and releases admission on close", async () => {
    const open = vi.fn(async () => ({ close: vi.fn() }));
    const subscribe = limitLocalChangeSubscriptions(open);
    const streams = await Promise.all(Array.from({ length: 32 }, () => subscribe(scope, async () => {}, vi.fn())));
    await expect(subscribe(scope, async () => {}, vi.fn())).rejects.toMatchObject({ code: "stream_error" });
    streams[0].close();
    const replacement = await subscribe(scope, async () => {}, vi.fn());
    replacement.close();
    streams.slice(1).forEach(stream => stream.close());
  });
  it("sends browser scope and credits after read", async () => {
    const socket: BrowserWebSocket = { readyState: 1, onopen: null, onmessage: null, onerror: null, onclose: null, send: vi.fn(), close: vi.fn() };
    let finish!: () => void;
    const read = new Promise<void>(resolve => { finish = resolve; });
    const opening = createBrowserClient(undefined, () => socket).subscribeLocalChanges(scope, () => read, vi.fn());
    socket.onopen!(new Event("open"));
    expect(socket.send).toHaveBeenCalledWith(JSON.stringify(scope));
    socket.onmessage!({ data: JSON.stringify(ready) } as MessageEvent);
    const stream = await opening; await flush();
    expect(socket.send).toHaveBeenCalledTimes(1);
    finish(); await flush();
    expect(socket.send).toHaveBeenLastCalledWith(JSON.stringify({ stream_id: "stream", sequence: 1 }));
    stream.close();
    expect(socket.close).toHaveBeenCalledOnce();
  });
  it("credits native events after the consumer completes", async () => {
    let channel!: NativeChannel<unknown>;
    let finish!: () => void;
    const pending = new Promise<void>(resolve => { finish = resolve; });
    const invoke = vi.fn(async (command: string, args?: Record<string, unknown>) => {
      if (command === "cockpit_local_changes_subscribe") {
        channel = args!.channel as NativeChannel<unknown>;
        return "registry-id";
      }
      return undefined;
    });
    const opening = createNativeClient(invoke, <T,>(onmessage: (message: T) => void) => ({ onmessage })).subscribeLocalChanges(scope, () => pending, vi.fn());
    await flush();
    channel.onmessage(ready);
    const stream = await opening; await flush();
    expect(invoke).not.toHaveBeenCalledWith("cockpit_local_changes_ack", expect.anything());
    finish(); await flush();
    expect(invoke).toHaveBeenCalledWith("cockpit_local_changes_ack", { ack: { stream_id: "stream", sequence: 1 } });
    stream.close();
    expect(invoke).toHaveBeenCalledWith("cockpit_stream_cancel", { streamId: "registry-id" });
  });
  it("cancels a native subscription that completes after abort", async () => {
    let finish!: (id: string) => void;
    const calls: string[] = [];
    const invoke = vi.fn((command: string) => {
      calls.push(command);
      return command === "cockpit_local_changes_subscribe" ? new Promise<unknown>(resolve => { finish = resolve; }) : Promise.resolve(undefined);
    });
    const controller = new AbortController();
    const opening = createNativeClient(invoke, <T,>(onmessage: (message: T) => void): NativeChannel<T> => ({ onmessage })).subscribeLocalChanges(scope, async () => {}, vi.fn(), controller.signal);
    controller.abort();
    await expect(opening).rejects.toMatchObject({ name: "AbortError" });
    finish("late"); await flush();
    expect(calls).toEqual(["cockpit_local_changes_subscribe", "cockpit_stream_cancel"]);
  });
});
