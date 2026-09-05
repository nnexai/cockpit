import { Channel, invoke as tauriInvoke } from "@tauri-apps/api/core";
import type {
  FocusRequest,
  FocusResponse,
  ResourceMutationRequest,
  ResourceMutationResponse,
  SessionListResponse,
  SessionSnapshotResponse,
  SessionStreamMessage,
  StatusResponse,
  TerminalCommand,
  TerminalOpenRequest,
  TerminalStreamMessage,
} from "../protocol/generated/v1";
import {
  CockpitClientError,
  parseErrorEnvelope,
  parseFocusRequest,
  parseFocusResponse,
  parseResourceMutationRequest,
  parseResourceMutationResponse,
  parseSessionListResponse,
  parseSessionSnapshotResponse,
  parseSessionStreamMessage,
  parseStatusResponse,
  parseTerminalCommand,
  parseTerminalOpenRequest,
  parseTerminalStreamMessage,
  validateSessionId,
  type ClosableStream,
  type CockpitClient,
  type TerminalStream,
} from "./CockpitClient";

export type NativeInvoke = (command: string, args?: Record<string, unknown>) => Promise<unknown>;
export interface NativeChannel<T> { onmessage: (message: T) => void; }
export type NativeChannelFactory = <T>(onMessage: (message: T) => void) => NativeChannel<T>;

function defaultInvoke(command: string, args?: Record<string, unknown>): Promise<unknown> {
  return tauriInvoke<unknown>(command, args);
}
function defaultChannel<T>(onMessage: (message: T) => void): NativeChannel<T> {
  return new Channel<T>(onMessage);
}

async function invokeAndParse<T>(invoke: NativeInvoke, command: string, args: Record<string, unknown> | undefined, operation: string, parse: (value: unknown) => T): Promise<T> {
  let body: unknown;
  try { body = args === undefined ? await invoke(command) : await invoke(command, args); }
  catch (cause) {
    const envelope = parseErrorEnvelope(cause);
    throw new CockpitClientError("native_error", envelope?.message ?? `The native Cockpit ${operation} command failed`, { cause, operationCode: envelope?.code });
  }
  try { return parse(body); }
  catch (error) {
    if (error instanceof CockpitClientError) throw error;
    throw new CockpitClientError("malformed_response", `The native Cockpit ${operation} command returned an invalid response`, { cause: error });
  }
}

function streamFailure(message: string, cause?: unknown): CockpitClientError {
  return new CockpitClientError("stream_error", message, { cause });
}
function streamId(value: unknown): string {
  if (typeof value === "string" && value.length > 0) return value;
  if (typeof value === "object" && value !== null && "stream_id" in value && typeof value.stream_id === "string" && value.stream_id.length > 0) return value.stream_id;
  throw new CockpitClientError("malformed_response", "Native stream command returned no stream id");
}
function sequenceChecker(sessionId: string) {
  let generation: number | undefined;
  let sequence: number | undefined;
  return (message: SessionStreamMessage): CockpitClientError | undefined => {
    if (message.session_id !== sessionId) return streamFailure("Session stream message belongs to another session");
    if (generation === undefined) {
      if (message.sequence !== 1) return streamFailure("Session stream must begin at sequence 1");
      generation = message.generation;
      sequence = message.sequence;
      return undefined;
    }
    if (message.generation < generation || message.generation > generation + 1) return streamFailure("Session stream generation gap detected");
    if (message.generation === generation && (sequence === 0xffffffff || message.sequence !== sequence! + 1)) return streamFailure("Session stream sequence gap detected");
    generation = message.generation;
    sequence = message.sequence;
    return undefined;
  };
}

function sessionSubscription(channelFactory: NativeChannelFactory, invoke: NativeInvoke, sessionId: string, onMessage: (message: SessionStreamMessage) => void, onError: (error: CockpitClientError) => void): Promise<ClosableStream> {
  try { validateSessionId(sessionId); } catch (error) { return Promise.reject(error); }
  let closed = false;
  let cancelled = false;
  let activeStreamId: string | undefined;
  const cancel = (id: string) => {
    if (cancelled) return;
    cancelled = true;
    void invoke("cockpit_stream_cancel", { streamId: id }).catch(() => undefined);
  };
  const checkSequence = sequenceChecker(sessionId);
  const channel = channelFactory<unknown>((raw) => {
    if (closed) return;
    let message: SessionStreamMessage;
    try { message = parseSessionStreamMessage(raw); }
    catch (error) {
      closed = true;
      onError(error instanceof CockpitClientError ? error : streamFailure("Session stream message is malformed", error));
      if (activeStreamId !== undefined) cancel(activeStreamId);
      return;
    }
    const sequenceError = checkSequence(message);
    if (sequenceError) {
      closed = true;
      onError(sequenceError);
      if (activeStreamId !== undefined) cancel(activeStreamId);
      return;
    }
    onMessage(message);
  });
  return invokeAndParse(invoke, "cockpit_session_subscribe", { sessionId, channel }, "session subscription", streamId).then((id) => {
    activeStreamId = id;
    if (closed) cancel(id);
    return {
      close() {
        if (closed) return;
        closed = true;
        cancel(id);
      },
    };
  });
}

function terminalSubscription(channelFactory: NativeChannelFactory, invoke: NativeInvoke, request: TerminalOpenRequest, onMessage: (message: TerminalStreamMessage) => void, onError: (error: CockpitClientError) => void): Promise<TerminalStream> {
  let validated: TerminalOpenRequest;
  try { validated = parseTerminalOpenRequest(request); } catch (error) { return Promise.reject(error); }
  let closed = false;
  let cancelled = false;
  let activeStreamId: string | undefined;
  let lastFrameSequence: bigint | undefined;
  let receivedFrame = false;
  const cancel = (id: string) => {
    if (cancelled) return;
    cancelled = true;
    void invoke("cockpit_stream_cancel", { streamId: id }).catch(() => undefined);
  };
  let ready = false;
  const channel = channelFactory<unknown>((raw) => {
    if (closed) return;
    let message: TerminalStreamMessage;
    try { message = parseTerminalStreamMessage(raw); }
    catch (error) {
      closed = true;
      onError(error instanceof CockpitClientError ? error : streamFailure("Terminal stream message is malformed", error));
      if (activeStreamId !== undefined) cancel(activeStreamId);
      return;
    }
    if (message.session_id !== validated.session_id || message.pane_id !== validated.pane_id) {
      closed = true;
      onError(streamFailure("Terminal stream message belongs to another session or pane"));
      if (activeStreamId !== undefined) cancel(activeStreamId);
      return;
    }
    if (activeStreamId === undefined) activeStreamId = message.stream_id;
    else if (message.stream_id !== activeStreamId) {
      closed = true;
      onError(streamFailure("Terminal stream message belongs to another stream"));
      cancel(activeStreamId);
      return;
    }
    if (message.type === "frame") {
      const current = BigInt(message.seq);
      if (!receivedFrame && !message.full) {
        closed = true;
        onError(streamFailure("Terminal stream must begin with a full frame"));
        if (activeStreamId !== undefined) cancel(activeStreamId);
        return;
      }
      if (receivedFrame && current !== lastFrameSequence! + 1n) {
        closed = true;
        onError(streamFailure("Terminal frame sequence is not consecutive"));
        if (activeStreamId !== undefined) cancel(activeStreamId);
        return;
      }
      lastFrameSequence = current;
      receivedFrame = true;
    }
    onMessage(message);
  });
  return invokeAndParse(invoke, "cockpit_terminal_open", { request: validated, channel }, "terminal open", streamId).then((id) => {
    activeStreamId = id;
    if (closed) cancel(id);
    ready = !closed;
    return {
      send(command: TerminalCommand) {
        if (closed || !ready) throw new CockpitClientError("stream_error", "Terminal stream is not ready");
        const parsed = parseTerminalCommand(command);
        void invoke("cockpit_terminal_command", { streamId: id, command: parsed }).catch((cause) => {
          if (!closed) {
            const envelope = parseErrorEnvelope(cause);
            onError(new CockpitClientError("native_error", envelope?.message ?? "The native terminal command failed", { cause, operationCode: envelope?.code }));
          }
        });
      },
      close() {
        if (closed) return;
        closed = true;
        ready = false;
        cancel(id);
      },
    } satisfies TerminalStream;
  });
}

export function createNativeClient(invoke: NativeInvoke = defaultInvoke, channelFactory: NativeChannelFactory = defaultChannel): CockpitClient {
  return {
    status(): Promise<StatusResponse> { return invokeAndParse(invoke, "cockpit_status", undefined, "status", parseStatusResponse); },
    sessions(): Promise<SessionListResponse> { return invokeAndParse(invoke, "cockpit_sessions", undefined, "sessions", parseSessionListResponse); },
    sessionSnapshot(sessionId: string): Promise<SessionSnapshotResponse> {
      try { validateSessionId(sessionId); } catch (error) { return Promise.reject(error); }
      return invokeAndParse(invoke, "cockpit_session_snapshot", { sessionId }, "session snapshot", parseSessionSnapshotResponse).then((value) => {
        if (value.session_id !== sessionId) throw new CockpitClientError("malformed_response", "Session snapshot belongs to another session");
        return value;
      });
    },
    focus(sessionId: string, request: FocusRequest): Promise<FocusResponse> {
      try { validateSessionId(sessionId); } catch (error) { return Promise.reject(error); }
      let parsed: FocusRequest;
      try { parsed = parseFocusRequest(request); } catch (error) { return Promise.reject(error); }
      return invokeAndParse(invoke, "cockpit_focus", { sessionId, request: parsed }, "focus", parseFocusResponse).then((value) => {
        if (value.session_id !== sessionId) throw new CockpitClientError("malformed_response", "Focus response belongs to another session");
        return value;
      });
    },
    mutate(sessionId: string, mutationRequest: ResourceMutationRequest): Promise<ResourceMutationResponse> {
      try { validateSessionId(sessionId); } catch (error) { return Promise.reject(error); }
      let parsed: ResourceMutationRequest;
      try { parsed = parseResourceMutationRequest(mutationRequest); } catch (error) { return Promise.reject(error); }
      return invokeAndParse(
        invoke,
        "cockpit_mutate",
        { sessionId, request: parsed },
        "mutation",
        parseResourceMutationResponse,
      ).then((value) => {
        if (value.session_id !== sessionId || value.snapshot.session_id !== sessionId) {
          throw new CockpitClientError("malformed_response", "Mutation response belongs to another session");
        }
        return value;
      });
    },
    subscribeSession(sessionId, onMessage, onError) { return sessionSubscription(channelFactory, invoke, sessionId, onMessage, onError); },
    openTerminal(request, onMessage, onError) { return terminalSubscription(channelFactory, invoke, request, onMessage, onError); },
  };
}
