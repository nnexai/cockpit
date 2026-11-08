import { describe, expect, it, vi } from "vitest";
import type { VoiceTransportEvent } from "../app/voice/useVoiceSession";
import type { VoiceConnectResponse, VoiceEventsRequest, VoiceEventsResponse } from "./voiceProtocol";
import { createVoiceTransport } from "./voiceTransport";

// The project's ES2022 library does not include Promise.withResolvers.
function deferred<T>() {
  let resolve!: (value: T) => void;
  const promise = new Promise<T>((done) => { resolve = done; });
  return { promise, resolve };
}

const connected: VoiceConnectResponse = {
  call_id: "call-1", generation: 4, sdp_answer: "answer", voice: "voice", liaison_root_id: "root",
};
const identity = { call_id: "call-1", generation: 4 };

describe("voice transport lifecycle", () => {
  it("delivers ordered transcript updates once and preserves the end outcome", async () => {
    const ended = deferred<VoiceTransportEvent>();
    const observed: VoiceTransportEvent[] = [];
    const events = vi.fn<(_request: VoiceEventsRequest, _signal: AbortSignal) => Promise<VoiceEventsResponse>>()
      .mockResolvedValueOnce({
        events: [
          { ...identity, seq: 1, kind: "phase", phase: "listening" },
          { ...identity, seq: 2, kind: "transcript", role: "user", turn: 1, text: "hello", final: false },
        ], next_seq: 2, ended: false,
      })
      .mockResolvedValueOnce({
        events: [
          { ...identity, seq: 2, kind: "transcript", role: "user", turn: 1, text: "hello", final: false },
          { ...identity, seq: 3, kind: "transcript", role: "user", turn: 1, text: "hello there", final: true },
          { ...identity, seq: 4, kind: "ended", reason: "interrupted", error: null },
        ], next_seq: 4, ended: true,
      });
    const transport = createVoiceTransport("session", {
      connect: async () => connected, events, control: async () => {},
    });
    transport.subscribe((event) => {
      observed.push(event);
      if (event.type === "ended") ended.resolve(event);
    });

    await transport.connect("offer", new AbortController().signal);
    await ended.promise;
    expect(observed).toEqual([
      { type: "phase", phase: "listening" },
      { type: "transcript", role: "user", turn: 1, text: "hello", final: false },
      { type: "transcript", role: "user", turn: 1, text: "hello there", final: true },
      { type: "ended", reason: "interrupted", error: undefined },
    ]);
    expect(events).toHaveBeenCalledTimes(2);
    expect(events.mock.calls[1][0].after_seq).toBe(2);
    await transport.stop();
  });

  it.each([
    {
      name: "stale generation",
      response: {
        events: [
          { ...identity, seq: 1, kind: "phase" as const, phase: "listening" },
          { ...identity, generation: 3, seq: 2, kind: "transcript" as const, role: "assistant" as const, turn: 1, text: "stale", final: true },
        ], next_seq: 2, ended: false,
      },
      error: "Stale voice event identity",
    },
    {
      name: "out-of-order sequence",
      response: {
        events: [
          { ...identity, seq: 2, kind: "phase" as const, phase: "listening" },
          { ...identity, seq: 1, kind: "phase" as const, phase: "speaking" },
        ], next_seq: 2, ended: false,
      },
      error: "Voice event sequence lost or out of order",
    },
  ])("ends without delivering any part of a $name batch", async ({ response, error }) => {
    const ended = deferred<VoiceTransportEvent>();
    const observed: VoiceTransportEvent[] = [];
    const events = vi.fn(async (): Promise<VoiceEventsResponse> => response);
    const control = vi.fn(async () => {});
    const transport = createVoiceTransport("session", { connect: async () => connected, events, control });
    transport.subscribe((event) => {
      observed.push(event);
      if (event.type === "ended") ended.resolve(event);
    });

    await transport.connect("offer", new AbortController().signal);
    await ended.promise;
    expect(observed).toEqual([{ type: "ended", error: `Voice event stream failed: ${error}` }]);
    expect(events).toHaveBeenCalledOnce();
    await transport.stop();
    expect(control).toHaveBeenCalledOnce();
  });

  it("stops a call acquired after stop was requested before connect completed", async () => {
    const pending = deferred<VoiceConnectResponse>();
    const started = deferred<void>();
    const events = vi.fn(async (): Promise<VoiceEventsResponse> => ({ events: [], next_seq: 0, ended: true }));
    const control = vi.fn(async () => {});
    const transport = createVoiceTransport("session", {
      connect: () => { started.resolve(); return pending.promise; }, events, control,
    });
    const connecting = transport.connect("offer", new AbortController().signal);
    const rejected = expect(connecting).rejects.toMatchObject({ name: "AbortError" });
    await started.promise;
    const stopping = transport.stop();
    pending.resolve(connected);
    await rejected;
    await stopping;

    expect(events).not.toHaveBeenCalled();
    expect(control).toHaveBeenCalledOnce();
    expect(control).toHaveBeenCalledWith({ ...identity, control: { type: "stop" } });
  });

  it("does not retry a stop whose outcome is unknown", async () => {
    const control = vi.fn(async () => { throw new Error("connection lost after sending"); });
    const transport = createVoiceTransport("session", {
      connect: async () => connected,
      events: async () => ({ events: [], next_seq: 0, ended: true }),
      control,
    });
    await transport.connect("offer", new AbortController().signal);
    const first = transport.stop();
    const second = transport.stop();
    await expect(first).rejects.toThrow("connection lost after sending");
    await expect(second).rejects.toThrow("connection lost after sending");
    await transport.stop();
    expect(control).toHaveBeenCalledOnce();
  });

  it("does not retry a connect whose outcome is unknown", async () => {
    const connect = vi.fn(async (): Promise<VoiceConnectResponse> => { throw new Error("connection lost after sending"); });
    const control = vi.fn(async () => {});
    const transport = createVoiceTransport("session", {
      connect,
      events: async () => ({ events: [], next_seq: 0, ended: true }),
      control,
    });
    await expect(transport.connect("offer", new AbortController().signal)).rejects.toThrow("connection lost after sending");
    await transport.stop();
    expect(connect).toHaveBeenCalledOnce();
    expect(control).not.toHaveBeenCalled();
  });
});
