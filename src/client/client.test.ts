import { describe, expect, it, vi } from "vitest";
import { createBrowserClient, type BrowserWebSocket } from "./browser";
import { createNativeClient, type NativeChannel } from "./native";
import {
  CockpitClientError,
  parseSessionListResponse,
  parseResourceMutationRequest,
  parseResourceMutationResponse,
  parseSessionSnapshotResponse,
  parseTerminalCommand,
  parseTerminalOpenRequest,
  parseTerminalStreamMessage,
  parseSessionStreamMessage,
  type CockpitClient,
  type CockpitSessionSnapshot,
  type ResourceMutationRequest,
} from "./CockpitClient";

const snapshot: CockpitSessionSnapshot = {
  session_id: "session-1",
  version: "0.8.2",
  protocol: 20,
  focused_space_id: "space-1",
  focused_tab_id: "tab-1",
  focused_pane_id: "pane-1",
  spaces: [{ id: "space-1", label: "Main", number: 1, tab_count: 1, pane_count: 1, focused: true, agent_status: "working", git: { repository_key: "repo-opaque", repository: "cockpit", branch: "main", checkout_path: "/work/cockpit", is_linked_worktree: false } }],
  tabs: [{ id: "tab-1", space_id: "space-1", label: "Shell", number: 1, pane_count: 1, focused: true }],
  panes: [{ id: "pane-1", terminal_id: "terminal-1", space_id: "space-1", tab_id: "tab-1", title: "Terminal", focused: true, agent: null, agent_status: "idle", revision: 1 }],
  layouts: [{ space_id: "space-1", tab_id: "tab-1", area: { x: 0, y: 0, width: 80, height: 24 }, focused_pane_id: "pane-1", panes: [{ pane_id: "pane-1", focused: true, rect: { x: 0, y: 0, width: 80, height: 24 } }], zoomed: false }],
  agents: [],
};
const status = { protocol_version: "v1", cockpit_version: "0.1.0", mode: "normal" as const, herdr: { status: "unavailable" as const, code: "test", message: "test" } };
const sessions = { sessions: [{ id: "session-1", label: "Main", is_default: true, running: true }] };

class FakeSocket implements BrowserWebSocket {
  readyState = 0;
  onopen: ((event: Event) => void) | null = null;
  onmessage: ((event: MessageEvent) => void) | null = null;
  onerror: ((event: Event) => void) | null = null;
  onclose: ((event: CloseEvent) => void) | null = null;
  readonly sent: string[] = [];
  send(data: string): void { this.sent.push(data); }
  open(): void { this.readyState = 1; this.onopen?.(new Event("open")); }
  message(data: unknown): void { this.onmessage?.({ data } as MessageEvent); }
  close(): void { this.readyState = 3; this.onclose?.({ code: 1000, reason: "closed" } as CloseEvent); }
}

function jsonResponse(value: unknown, statusCode = 200): Response {
  return new Response(JSON.stringify(value), { status: statusCode });
}
function streamSnapshot(sequence: number, generation = 1) {
  return { type: "snapshot", session_id: snapshot.session_id, generation, sequence, snapshot } as const;
}
function streamStale(sequence: number, generation = 1) {
  return { type: "stale", session_id: snapshot.session_id, generation, sequence, code: "stale", message: "resync" } as const;
}

function completeClient(overrides: Partial<CockpitClient> = {}): CockpitClient {
  return {
    status: vi.fn(async () => status),
    sessions: vi.fn(async () => sessions),
    sessionSnapshot: vi.fn(async () => snapshot),
    focus: vi.fn(async () => ({ session_id: snapshot.session_id, kind: "pane" as const, target_id: "pane-1", accepted: true })),
    mutate: vi.fn(async () => ({ session_id: snapshot.session_id, snapshot })),
    subscribeSession: vi.fn(async () => ({ close: vi.fn() })),
    openTerminal: vi.fn(async () => ({ send: vi.fn(), close: vi.fn() })),
    ...overrides,
  };
}

describe("client DTO parsers", () => {
  it("strictly validates session and terminal contracts", () => {
    expect(parseSessionListResponse(sessions)).toEqual(sessions);
    const legacyAgent = { pane_id: "pane-1", space_id: "space-1", tab_id: "tab-1", name: "omp", status: "working", title: null, focused: true };
    expect(parseSessionSnapshotResponse({ ...snapshot, agents: [legacyAgent] }).agents[0]?.state_change_seq).toBe(0);
    expect(() => parseSessionSnapshotResponse({ ...snapshot, session_id: undefined })).toThrow(CockpitClientError);
    expect(() => parseSessionSnapshotResponse({
      ...snapshot,
      spaces: [{ ...snapshot.spaces[0], git: { repository: "cockpit", branch: "main", checkout_path: "/work/cockpit" } }],
    })).toThrow(CockpitClientError);
    expect(() => parseSessionSnapshotResponse({
      ...snapshot,
      spaces: [{ ...snapshot.spaces[0], agent_status: undefined }],
    })).toThrow(CockpitClientError);
    expect(() => parseSessionStreamMessage({ type: "snapshot", session_id: "session-1", generation: 1, sequence: 1, snapshot: { ...snapshot, session_id: "other" } })).toThrow(CockpitClientError);
    expect(() => parseTerminalCommand({ type: "terminal.input", text: null, bytes: null })).toThrow(/exactly one/);
    expect(() => parseTerminalCommand({ type: "terminal.input", text: null, bytes: "not base64" })).toThrow(CockpitClientError);
    expect(parseTerminalCommand({ type: "terminal.mouse", kind: "down", button: "left", column: 12, row: 7, modifiers: 0 })).toEqual({ type: "terminal.mouse", kind: "down", button: "left", column: 12, row: 7, modifiers: 0 });
    expect(() => parseTerminalCommand({ type: "terminal.mouse", kind: "moved", button: "left", column: 12, row: 7, modifiers: 0 })).toThrow(CockpitClientError);
    expect(() => parseTerminalOpenRequest({ session_id: "../session", pane_id: "pane-1", mode: "observe", takeover: false, cols: 80, rows: 24 })).toThrow(/invalid target/);
    expect(() => parseTerminalOpenRequest({ session_id: "session-1", pane_id: "pane/child", mode: "observe", takeover: false, cols: 80, rows: 24 })).toThrow(/invalid target/);
  });
  it("validates every resource mutation discriminant and required nullable field", () => {
    const mutations: ResourceMutationRequest[] = [
      { type: "space_create", cwd: null, label: null },
      { type: "space_rename", space_id: "space-1", label: "Work" },
      { type: "space_move_block", space_ids: ["space-1"], before_space_id: null },
      { type: "space_close", space_id: "space-1" },
      { type: "tab_create", space_id: "space-1", label: null },
      { type: "tab_rename", tab_id: "tab-1", label: "Shell" },
      { type: "tab_move", tab_id: "tab-1", insert_index: 2 },
      { type: "tab_close", tab_id: "tab-1" },
      { type: "pane_split", pane_id: "pane-1", direction: "right", ratio: null },
      { type: "pane_resize", pane_id: "pane-1", direction: "left", amount: 0.1 },
      { type: "pane_rename", pane_id: "pane-1", label: null },
      { type: "pane_swap", source_pane_id: "pane-1", target_pane_id: "pane-2" },
      {
        type: "pane_move",
        pane_id: "pane-1",
        destination: { type: "existing_tab", tab_id: "tab-2", direction: "down", target_pane_id: null, ratio: null },
      },
      { type: "pane_zoom", pane_id: "pane-1", mode: "toggle" },
      { type: "pane_close", pane_id: "pane-1" },
    ];
    for (const mutation of mutations) expect(parseResourceMutationRequest(mutation)).toEqual(mutation);
    expect(() => parseResourceMutationRequest({ type: "space_create", label: null })).toThrow(CockpitClientError);
    expect(() => parseResourceMutationRequest({ type: "pane_resize", pane_id: "pane-1", direction: "left", amount: Number.NaN })).toThrow(CockpitClientError);
    expect(() => parseResourceMutationRequest({ type: "pane_resize", pane_id: "pane-1", direction: "left", amount: 0 })).toThrow(CockpitClientError);
    expect(() => parseResourceMutationRequest({ type: "pane_split", pane_id: "pane-1", direction: "right", ratio: 1 })).toThrow(CockpitClientError);
    expect(() => parseResourceMutationRequest({ type: "pane_move", pane_id: "pane-1", destination: { type: "raw", method: "pane.move" } })).toThrow(CockpitClientError);
    expect(() => parseResourceMutationRequest({ type: "raw", method: "layout.apply", params: {} })).toThrow(CockpitClientError);
    expect(() => parseResourceMutationResponse({ session_id: "session-1", snapshot: { ...snapshot, session_id: "other" } })).toThrow(/another session/);
  });
});

describe("browser CockpitClient", () => {
  it("maps named sessions, encoded snapshots, and focus", async () => {
    const request = vi.fn(async (input: string, init?: RequestInit) => {
      if (input === "/api/v1/sessions") return jsonResponse(sessions);
      if (input === "/api/v1/sessions/session_1/snapshot") return jsonResponse({ ...snapshot, session_id: "session_1" });
      if (input === "/api/v1/sessions/session_1/focus") {
        expect(init?.method).toBe("POST");
        expect(init?.body).toBe(JSON.stringify({ kind: "pane", target_id: "pane-1" }));
        return jsonResponse({ session_id: "session_1", kind: "pane", target_id: "pane-1", accepted: true });
      }
      expect(input).toBe("/api/v1/sessions/session_1/mutations");
      expect(init?.method).toBe("POST");
      expect(init?.body).toBe(JSON.stringify({ type: "pane_close", pane_id: "pane-1" }));
      return jsonResponse({ session_id: "session_1", snapshot: { ...snapshot, session_id: "session_1" } });
    });
    const client = createBrowserClient(request);
    await expect(client.sessions()).resolves.toEqual(sessions);
    await expect(client.sessionSnapshot("session_1")).resolves.toMatchObject({ session_id: "session_1" });
    await expect(client.focus("session_1", { kind: "pane", target_id: "pane-1" })).resolves.toMatchObject({ accepted: true });
    expect(request).toHaveBeenCalledTimes(3);
    await expect(client.mutate("session_1", { type: "pane_close", pane_id: "pane-1" })).resolves.toMatchObject({ session_id: "session_1" });
    expect(request).toHaveBeenCalledTimes(4);
  });

  it("validates stream order, reports gaps and malformed messages, and closes idempotently", async () => {
    const socket = new FakeSocket();
    const factory = vi.fn(() => socket);
    const errors: CockpitClientError[] = [];
    const messages: unknown[] = [];
    const client = createBrowserClient(vi.fn(async () => jsonResponse(snapshot)), factory);
    const streamPromise = client.subscribeSession("session-1", (message) => messages.push(message), (error) => errors.push(error));
    expect(factory).toHaveBeenCalledWith("/api/v1/sessions/session-1/events");
    socket.open();
    const stream = await streamPromise;
    socket.message(JSON.stringify(streamSnapshot(1)));
    socket.message(JSON.stringify(streamStale(2)));
    socket.message(JSON.stringify(streamStale(4)));
    expect(messages).toHaveLength(2);
    expect(errors.at(-1)?.message).toMatch(/sequence gap/);
    const errorCount = errors.length;
    socket.message("not json");
    expect(errors).toHaveLength(errorCount);
    stream.close();
    stream.close();
  });
  it("requires a session stream to start at sequence one", async () => {
    const socket = new FakeSocket();
    const errors: CockpitClientError[] = [];
    const open = createBrowserClient(vi.fn(async () => jsonResponse(snapshot)), () => socket).subscribeSession("session-1", vi.fn(), (error) => errors.push(error));
    socket.open();
    await open;
    socket.message(JSON.stringify(streamSnapshot(2)));
    expect(errors.at(-1)?.message).toMatch(/begin at sequence 1/);
    expect(socket.readyState).toBe(3);
  });


  it("opens terminal with encoded ids, sends exact JSON, and suppresses duplicate close", async () => {
    const socket = new FakeSocket();
    const factory = vi.fn(() => socket);
    const client = createBrowserClient(vi.fn(async () => jsonResponse(snapshot)), factory);
    const open = client.openTerminal({ session_id: "s_1", pane_id: "w1A:p1", mode: "control", takeover: true, cols: 80, rows: 24 }, vi.fn(), vi.fn());
    expect(factory).toHaveBeenCalledWith("/api/v1/sessions/s_1/panes/w1A%3Ap1/terminal?mode=control&takeover=true&cols=80&rows=24");
    socket.open();
    const stream = await open;
    socket.message(JSON.stringify({ type: "ownership", session_id: "s_1", pane_id: "w1A:p1", stream_id: "stream-1", state: "owned", message: null }));
    stream.send({ type: "terminal.input", text: "hello", bytes: null });
    stream.send({ type: "terminal.mouse", kind: "down", button: "left", column: 12, row: 7, modifiers: 0 });
    expect(socket.sent).toEqual([
      JSON.stringify({ type: "terminal.input", text: "hello", bytes: null }),
      JSON.stringify({ type: "terminal.mouse", kind: "down", button: "left", column: 12, row: 7, modifiers: 0 }),
    ]);
    stream.close();
    stream.close();
    expect(socket.readyState).toBe(3);
  });

  it("rejects terminal target mismatches and initial incremental frames", async () => {
    const first = new FakeSocket();
    const second = new FakeSocket();
    const sockets = [first, second];
    const factory = vi.fn(() => sockets.shift()!);
    const errors: CockpitClientError[] = [];
    const client = createBrowserClient(vi.fn(async () => jsonResponse(snapshot)), factory);
    const firstOpen = client.openTerminal({ session_id: "session-1", pane_id: "pane-1", mode: "observe", takeover: false, cols: 80, rows: 24 }, vi.fn(), (error) => errors.push(error));
    first.open();
    await firstOpen;
    first.message(JSON.stringify({ type: "ownership", session_id: "other", pane_id: "pane-1", stream_id: "stream-1", state: "observing", message: null }));
    expect(first.readyState).toBe(3);
    const secondOpen = client.openTerminal({ session_id: "session-1", pane_id: "pane-1", mode: "observe", takeover: false, cols: 80, rows: 24 }, vi.fn(), (error) => errors.push(error));
    second.open();
    await secondOpen;
    second.message(JSON.stringify({ type: "frame", session_id: "session-1", pane_id: "pane-1", stream_id: "stream-2", seq: "1", encoding: "ansi", width: 80, height: 24, full: false, bytes: "" }));
    expect(errors).toHaveLength(2);
    expect(second.readyState).toBe(3);

  });
  it("preserves backend HTTP error envelopes", async () => {
    const client = createBrowserClient(vi.fn(async () => jsonResponse({ code: "live_inspection_disabled", message: "disabled" }, 503)));
    await expect(client.sessions()).rejects.toMatchObject({ code: "http_error", status: 503, operationCode: "live_inspection_disabled", message: "disabled" });
  });

  it("rejects mutation response and snapshot session mismatches", async () => {
    const wrongEnvelope = createBrowserClient(vi.fn(async () => jsonResponse({ session_id: "other", snapshot: { ...snapshot, session_id: "other" } })));
    await expect(wrongEnvelope.mutate("session-1", { type: "pane_close", pane_id: "pane-1" })).rejects.toMatchObject({ code: "malformed_response" });
    const wrongSnapshot = createBrowserClient(vi.fn(async () => jsonResponse({ session_id: "session-1", snapshot: { ...snapshot, session_id: "other" } })));
    await expect(wrongSnapshot.mutate("session-1", { type: "pane_close", pane_id: "pane-1" })).rejects.toMatchObject({ code: "malformed_response" });
  });
});

describe("native CockpitClient", () => {
  it("matches browser stream messages and cancellation semantics", async () => {
    let sessionChannel: NativeChannel<unknown> | undefined;
    let terminalChannel: NativeChannel<unknown> | undefined;
    const calls: Array<[string, Record<string, unknown> | undefined]> = [];
    const invoke = vi.fn(async (command: string, args?: Record<string, unknown>) => {
      calls.push([command, args]);
      if (command === "cockpit_sessions") return sessions;
      if (command === "cockpit_session_snapshot") return snapshot;
      if (command === "cockpit_focus") return { session_id: "session-1", kind: "pane", target_id: "pane-1", accepted: true };
      if (command === "cockpit_session_subscribe") return "sub-1";
      if (command === "cockpit_terminal_open") return "term-1";
      if (command === "cockpit_mutate") return { session_id: "session-1", snapshot };
      return undefined;
    });
    const channels = <T,>(onMessage: (message: T) => void): NativeChannel<T> => {
      const channel = { onmessage: onMessage };
      if (sessionChannel === undefined) sessionChannel = channel as NativeChannel<unknown>;
      else terminalChannel = channel as NativeChannel<unknown>;
      return channel;
    };
    const errors: CockpitClientError[] = [];
    const client = createNativeClient(invoke, channels);
    await expect(client.sessionSnapshot("session-1")).resolves.toEqual(snapshot);
    await expect(client.focus("session-1", { kind: "pane", target_id: "pane-1" })).resolves.toMatchObject({ accepted: true });
    const subscription = await client.subscribeSession("session-1", vi.fn(), (error) => errors.push(error));
    await expect(client.mutate("session-1", { type: "pane_close", pane_id: "pane-1" })).resolves.toEqual({ session_id: "session-1", snapshot });
    sessionChannel!.onmessage(streamSnapshot(1));
    sessionChannel!.onmessage(streamStale(2));
    subscription.close();
    subscription.close();
    const terminal = await client.openTerminal({ session_id: "session-1", pane_id: "pane-1", mode: "control", takeover: false, cols: 80, rows: 24 }, vi.fn(), (error) => errors.push(error));
    terminalChannel!.onmessage({ type: "ownership", session_id: "session-1", pane_id: "pane-1", stream_id: "term-1", state: "owned", message: null });
    terminal.send({ type: "terminal.release" });
    terminal.close();
    terminal.close();
    expect(calls.filter(([command]) => command === "cockpit_stream_cancel")).toHaveLength(2);
    expect(calls).toContainEqual(["cockpit_terminal_command", { streamId: "term-1", command: { type: "terminal.release" } }]);
    expect(calls).toContainEqual(["cockpit_mutate", { sessionId: "session-1", request: { type: "pane_close", pane_id: "pane-1" } }]);
    expect(errors).toEqual([]);
  });

  it("rejects native terminal first-frame violations", async () => {
    let channel: NativeChannel<unknown> | undefined;
    const invoke = vi.fn(async (command: string) => command === "cockpit_terminal_open" ? "term-1" : undefined);
    const channelFactory = <T,>(onMessage: (message: T) => void): NativeChannel<T> => {
      const created = { onmessage: onMessage };
      channel = created as NativeChannel<unknown>;
      return created;
    };
    const errors: CockpitClientError[] = [];
    const stream = await createNativeClient(invoke, channelFactory).openTerminal(
      { session_id: "session-1", pane_id: "pane-1", mode: "observe", takeover: false, cols: 80, rows: 24 },
      vi.fn(),
      (error) => errors.push(error),
    );
    channel!.onmessage({ type: "frame", session_id: "session-1", pane_id: "pane-1", stream_id: "term-1", seq: "1", encoding: "ansi", width: 80, height: 24, full: false, bytes: "" });
    expect(errors).toHaveLength(1);
    stream.close();
    expect(invoke).toHaveBeenCalledWith("cockpit_stream_cancel", { streamId: "term-1" });
  });

  it("preserves native backend error envelopes", async () => {
    const invoke = vi.fn(async () => { throw { code: "live_inspection_disabled", message: "disabled" }; });
    await expect(createNativeClient(invoke).sessions()).rejects.toMatchObject({ code: "native_error", operationCode: "live_inspection_disabled", message: "disabled" });
  });

  it("rejects native mutation response and snapshot session mismatches", async () => {
    const wrongEnvelope = createNativeClient(vi.fn(async () => ({ session_id: "other", snapshot: { ...snapshot, session_id: "other" } })));
    await expect(wrongEnvelope.mutate("session-1", { type: "pane_close", pane_id: "pane-1" })).rejects.toMatchObject({ code: "malformed_response" });
    const wrongSnapshot = createNativeClient(vi.fn(async () => ({ session_id: "session-1", snapshot: { ...snapshot, session_id: "other" } })));
    await expect(wrongSnapshot.mutate("session-1", { type: "pane_close", pane_id: "pane-1" })).rejects.toMatchObject({ code: "malformed_response" });
  });
});

describe("client selector mocks", () => {
  it("accepts complete transport-neutral clients", async () => {
    const client = completeClient();
    expect(await client.sessions()).toEqual(sessions);
  });
});
