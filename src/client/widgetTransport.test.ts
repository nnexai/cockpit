import { afterEach, describe, expect, it, vi } from "vitest";
import { createBrowserClient, type BrowserWebSocket } from "./browser";
import { createNativeClient, type NativeChannel, type NativeInvoke } from "./native";
import type { WidgetEvent, WidgetWindowReport } from "../protocol/generated/v1";

const key = { session_id: "session-1", tab_id: "tab-1", id: "pick" };
const report: WidgetWindowReport = { session_id: "session-1", displayed_tab_id: "tab-1", blocker: null };

class WidgetSocket implements BrowserWebSocket {
  readyState = 1;
  onopen: ((event: Event) => void) | null = null;
  onmessage: ((event: MessageEvent) => void) | null = null;
  onerror: ((event: Event) => void) | null = null;
  onclose: ((event: CloseEvent) => void) | null = null;
  sent: string[] = [];
  closed = false;
  send(data: string | ArrayBuffer) {
    if (typeof data !== "string") throw new Error("Unexpected binary report");
    this.sent.push(data);
  }
  close() { this.closed = true; }
  event(event: WidgetEvent) { this.onmessage?.({ data: JSON.stringify(event) } as MessageEvent); }
}

afterEach(() => vi.useRealTimers());

describe("widget transport security and identity", () => {
  it("retains gateway origin-denial and native stale-selection codes", async () => {
    const browser = createBrowserClient(async () => new Response(JSON.stringify({ code: "request_origin_required", message: "Origin required" }), { status: 400 }));
    await expect(browser.widgetRemove({ key })).rejects.toMatchObject({ code: "http_error", operationCode: "request_origin_required" });
    const native = createNativeClient(async () => { throw { code: "widget_stale", message: "Revision changed" }; });
    await expect(native.widgetSelect({ key, revision: 1, value: { type: "page", value_json: '{"answer":"yes"}' } })).rejects.toMatchObject({ code: "native_error", operationCode: "widget_stale" });
    await expect(native.widgetSelect({ key, revision: 1, value: { type: "choice", choice_id: "yes" } })).rejects.toMatchObject({ code: "native_error", operationCode: "widget_stale" });
  });

  it.each(["browser", "native"])("rejects %s content from another key or revision", async (kind) => {
    const body = { key: { ...key, id: "other" }, revision: 2, sha256: "a".repeat(64), body: { type: "html", document: "<p>active widget</p>" }, selection: null };
    const client = kind === "browser"
      ? createBrowserClient(async () => new Response(JSON.stringify(body)))
      : createNativeClient(async () => body);
    await expect(client.widgetContent({ key, revision: 1 })).rejects.toMatchObject({ code: "malformed_response" });
  });

  it.each(["browser", "native"])("projects %s page selections through the existing transport", async (kind) => {
    const fetch = vi.fn(async (_input: string, _init?: RequestInit) => new Response(JSON.stringify({ at_ms: 100 })));
    const invoke = vi.fn<NativeInvoke>(async () => ({ at_ms: 100 }));
    const client = kind === "browser" ? createBrowserClient(fetch) : createNativeClient(invoke);
    const value_json = '{"answer":{"ids":[1,2]},"__proto__":{"opaque":true}}';
    const request = {
      key, revision: 3,
      value: { type: "page" as const, value_json, key: { ...key, id: "other" }, revision: 999, extra: "ignored" },
    };
    await expect(client.widgetSelect(request)).resolves.toEqual({ at_ms: 100 });
    const projected = { key, revision: 3, value: { type: "page", value_json } };
    if (kind === "browser") {
      expect(fetch).toHaveBeenCalledTimes(1);
      expect(fetch.mock.calls[0][0]).toBe("/api/v1/widgets/select");
      expect(JSON.parse(fetch.mock.calls[0][1]!.body as string)).toEqual(projected);
      expect(invoke).not.toHaveBeenCalled();
    } else {
      expect(invoke).toHaveBeenCalledWith("cockpit_widget_select", { request: projected });
      expect(fetch).not.toHaveBeenCalled();
    }
  });

  it.each(["browser", "native"])("rejects invalid %s page JSON before invoking its transport", async (kind) => {
    const fetch = vi.fn(async () => new Response(JSON.stringify({ at_ms: 100 })));
    const invoke = vi.fn<NativeInvoke>(async () => ({ at_ms: 100 }));
    const client = kind === "browser" ? createBrowserClient(fetch) : createNativeClient(invoke);
    for (const value_json of ["not-json", '{"deep":[1e999]}', JSON.stringify("é".repeat(8192))]) {
      await expect(client.widgetSelect({ key, revision: 3, value: { type: "page", value_json } }))
        .rejects.toMatchObject({ code: "malformed_response" });
    }
    expect(fetch).not.toHaveBeenCalled();
    expect(invoke).not.toHaveBeenCalled();
  });

  it.each(["browser", "native"])("retains %s active scripts and prior selection in content responses", async (kind) => {
    const selection = {
      id: key.id, revision: 1, status: "selected", value_json: '{"filter":"errors"}', at_ms: 50, removed_at_ms: null,
    };
    const body = {
      key, revision: 2, sha256: "a".repeat(64),
      body: { type: "html", document: '<script>window.cockpit.select("clicked")</script>' }, selection,
    };
    const client = kind === "browser"
      ? createBrowserClient(async () => new Response(JSON.stringify(body)))
      : createNativeClient(async () => body);
    await expect(client.widgetContent({ key, revision: 2 })).resolves.toEqual(body);
  });
});

describe("widget browser subscriptions", () => {
  it("isolates concurrent cursors and closes a gap without accepting stale events", async () => {
    const sockets: WidgetSocket[] = [];
    const client = createBrowserClient(undefined, () => {
      const socket = new WidgetSocket(); sockets.push(socket); return socket;
    });
    const first: WidgetEvent[] = [], second: WidgetEvent[] = [];
    const firstError = vi.fn(), secondError = vi.fn();
    const pendingFirst = client.subscribeWidgets((event) => first.push(event), firstError);
    const pendingSecond = client.subscribeWidgets((event) => second.push(event), secondError);
    sockets[0].event({ type: "snapshot", sequence: 10, widgets: [] });
    sockets[1].event({ type: "snapshot", sequence: 2, widgets: [] });
    const [streamFirst, streamSecond] = await Promise.all([pendingFirst, pendingSecond]);
    sockets[0].event({ type: "removed", sequence: 9, key, reason: "user" });
    sockets[1].event({ type: "removed", sequence: 3, key, reason: "user" });
    sockets[0].event({ type: "removed", sequence: 12, key, reason: "user" });
    expect(first.map((event) => event.sequence)).toEqual([10]);
    expect(second.map((event) => event.sequence)).toEqual([2, 3]);
    expect(firstError).toHaveBeenCalledWith(expect.objectContaining({ code: "stream_error" }));
    expect(secondError).not.toHaveBeenCalled();
    expect(sockets[0].closed).toBe(true);
    expect(sockets[1].closed).toBe(false);
    streamFirst.close(); streamSecond.close();
  });

  it("coalesces the newest report by 100ms and cancels pending reports on close", async () => {
    vi.useFakeTimers();
    const socket = new WidgetSocket();
    const client = createBrowserClient(undefined, () => socket);
    const pending = client.subscribeWidgets(() => {}, vi.fn());
    socket.event({ type: "snapshot", sequence: 0, widgets: [] });
    const stream = await pending;
    stream.report(report);
    await vi.advanceTimersByTimeAsync(99);
    expect(socket.sent).toEqual([]);
    stream.report({ ...report, blocker: "library" });
    await vi.advanceTimersByTimeAsync(1);
    expect(socket.sent.map((value) => JSON.parse(value))).toEqual([{ ...report, blocker: "library" }]);
    stream.report(report); stream.close();
    await vi.advanceTimersByTimeAsync(100);
    expect(socket.sent.map((value) => JSON.parse(value))).toEqual([{ ...report, blocker: "library" }]);
  });

  it("rejects a non-snapshot first event and deregisters through websocket close", async () => {
    const socket = new WidgetSocket();
    const pending = createBrowserClient(undefined, () => socket).subscribeWidgets(vi.fn(), vi.fn());
    socket.event({ type: "removed", sequence: 1, key, reason: "user" });
    await expect(pending).rejects.toMatchObject({ code: "malformed_response" });
    expect(socket.closed).toBe(true);
  });
});

describe("widget native subscriptions", () => {
  it("cancels an aborted pending stream once its id arrives and ignores late events", async () => {
    let resolveOpening!: (value: unknown) => void;
    const opening = new Promise<unknown>((resolve) => { resolveOpening = resolve; });
    const calls: string[] = [];
    const invoke: NativeInvoke = async (command) => {
      calls.push(command);
      if (command === "cockpit_widget_subscribe") return opening;
      return null;
    };
    const channels: NativeChannel<unknown>[] = [];
    const client = createNativeClient(invoke, <T,>(handler: (value: T) => void) => {
      const channel = { onmessage: handler }; channels.push(channel as NativeChannel<unknown>); return channel;
    });
    const abort = new AbortController();
    const events = vi.fn();
    const pending = client.subscribeWidgets(events, vi.fn(), abort.signal);
    const rejected = expect(pending).rejects.toMatchObject({ name: "AbortError" });
    abort.abort(); await rejected;
    resolveOpening("late-widget-stream");
    await opening;
    channels[0].onmessage({ type: "snapshot", sequence: 0, widgets: [] });
    expect(events).not.toHaveBeenCalled();
    await vi.waitFor(() => expect(calls).toEqual(["cockpit_widget_subscribe", "cockpit_stream_cancel"]));
  });

  it("maintains independent native windows and reports to the right subscription", async () => {
    vi.useFakeTimers();
    const subscriptions: { id: string; channel: NativeChannel<unknown> }[] = [];
    const reports: { streamId: string; report: WidgetWindowReport }[] = [];
    const cancellations: string[] = [];
    const invoke: NativeInvoke = async (command, args) => {
      if (command === "cockpit_widget_subscribe") {
        const id = `stream-${subscriptions.length}`;
        const channel = args?.channel as NativeChannel<unknown>;
        subscriptions.push({ id, channel });
        channel.onmessage({ type: "snapshot", sequence: subscriptions.length * 10, widgets: [] });
        return id;
      }
      if (command === "cockpit_widget_report") reports.push(args as unknown as { streamId: string; report: WidgetWindowReport });
      if (command === "cockpit_stream_cancel") cancellations.push(args?.streamId as string);
      return null;
    };
    const client = createNativeClient(invoke, <T,>(onmessage: (value: T) => void) => ({ onmessage }));
    const first: WidgetEvent[] = [], second: WidgetEvent[] = [];
    const firstStream = await client.subscribeWidgets((event) => first.push(event), vi.fn());
    const secondStream = await client.subscribeWidgets((event) => second.push(event), vi.fn());
    subscriptions[0].channel.onmessage({ type: "removed", sequence: 11, key, reason: "user" });
    subscriptions[1].channel.onmessage({ type: "removed", sequence: 21, key, reason: "user" });
    firstStream.report(report); secondStream.report({ ...report, blocker: "zoom" });
    await vi.advanceTimersByTimeAsync(100);
    expect(first.map((event) => event.sequence)).toEqual([10, 11]);
    expect(second.map((event) => event.sequence)).toEqual([20, 21]);
    expect(reports).toEqual([{ streamId: "stream-0", report }, { streamId: "stream-1", report: { ...report, blocker: "zoom" } }]);
    firstStream.close();
    subscriptions[0].channel.onmessage({ type: "removed", sequence: 12, key, reason: "user" });
    expect(first.map((event) => event.sequence)).toEqual([10, 11]);
    expect(cancellations).toEqual(["stream-0"]);
    secondStream.close();
  });
});
