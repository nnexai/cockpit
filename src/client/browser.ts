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
  parseTerminalOpenRequest,
  parseTerminalStreamMessage,
  parseTerminalCommand,
  validateSessionId,
  type ClosableStream,
  type CockpitClient,
  type TerminalStream,
} from "./CockpitClient";
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

export type BrowserFetch = (input: string, init?: RequestInit) => Promise<Response>;

export interface BrowserWebSocket {
  readonly readyState: number;
  onopen: ((event: Event) => void) | null;
  onmessage: ((event: MessageEvent) => void) | null;
  onerror: ((event: Event) => void) | null;
  onclose: ((event: CloseEvent) => void) | null;
  send(data: string): void;
  close(code?: number, reason?: string): void;
}

export type BrowserWebSocketFactory = (url: string) => BrowserWebSocket;

function defaultFetch(input: string, init?: RequestInit): Promise<Response> {
  return globalThis.fetch(input, init);
}
function defaultWebSocket(url: string): BrowserWebSocket {
  return new WebSocket(url);
}

async function getJson<T>(
  request: BrowserFetch,
  path: string,
  endpoint: string,
  parse: (value: unknown) => T,
  init?: RequestInit,
): Promise<T> {
  let response: Response;
  try {
    response = await request(path, {
      ...init,
      headers: { Accept: "application/json", ...(init?.headers ?? {}) },
    });
  } catch (cause) {
    throw new CockpitClientError("transport_error", `Could not reach the ${endpoint} endpoint`, { cause });
  }
  if (!response.ok) {
    let errorBody: unknown;
    try { errorBody = await response.json(); } catch { /* non-JSON HTTP errors have no envelope */ }
    const envelope = parseErrorEnvelope(errorBody);
    throw new CockpitClientError(
      "http_error",
      envelope?.message ?? `Cockpit ${endpoint} endpoint returned HTTP ${response.status}`,
      { status: response.status, operationCode: envelope?.code },
    );
  }
  let body: unknown;
  try { body = await response.json(); } catch (cause) {
    throw new CockpitClientError("malformed_response", `Cockpit ${endpoint} endpoint returned invalid JSON`, { cause, status: response.status });
  }
  try { return parse(body); } catch (error) {
    if (error instanceof CockpitClientError) throw error;
    throw new CockpitClientError("malformed_response", `Cockpit ${endpoint} endpoint returned an invalid response`, { cause: error, status: response.status });
  }
}

function websocketUrl(path: string): string {
  const location = globalThis.location;
  if (location?.host) {
    const protocol = location.protocol === "https:" ? "wss:" : "ws:";
    return `${protocol}//${location.host}${path}`;
  }
  return path;
}

function streamError(message: string, cause?: unknown): CockpitClientError {
  return new CockpitClientError("stream_error", message, { cause });
}

function sequenceChecker(sessionId: string) {
  let generation: number | undefined;
  let sequence: number | undefined;
  return (message: SessionStreamMessage): CockpitClientError | undefined => {
    if (message.session_id !== sessionId) return streamError("Session stream message belongs to another session");
    if (generation === undefined) {
      if (message.sequence !== 1) return streamError("Session stream must begin at sequence 1");
      generation = message.generation;
      sequence = message.sequence;
      return undefined;
    }
    if (message.generation < generation || message.generation > generation + 1) {
      return streamError("Session stream generation gap detected", { generation, received: message.generation });
    }
    if (message.generation === generation) {
      if (sequence === 0xffffffff || message.sequence !== sequence! + 1) {
        return streamError("Session stream sequence gap detected", { sequence, received: message.sequence });
      }
    }
    generation = message.generation;
    sequence = message.sequence;
    return undefined;
  };
}

function openSessionStream(
  webSocketFactory: BrowserWebSocketFactory,
  sessionId: string,
  onMessage: (message: SessionStreamMessage) => void,
  onError: (error: CockpitClientError) => void,
): Promise<ClosableStream> {
  try { validateSessionId(sessionId); } catch (error) { return Promise.reject(error); }
  return new Promise((resolve, reject) => {
    let socket: BrowserWebSocket;
    let settled = false;
    let closed = false;
    const checkSequence = sequenceChecker(sessionId);
    const fail = (error: CockpitClientError, beforeOpen = false) => {
      if (beforeOpen && !settled) { settled = true; reject(error); }
      else onError(error);
      if (!closed) { closed = true; socket.close(); }
    };
    try {
      socket = webSocketFactory(websocketUrl(`/api/v1/sessions/${encodeURIComponent(sessionId)}/events`));
    } catch (cause) {
      reject(new CockpitClientError("transport_error", "Could not open session stream", { cause }));
      return;
    }
    const handle: ClosableStream = {
      close() {
        if (closed) return;
        closed = true;
        socket.close();
      },
    };
    socket.onopen = () => {
      if (closed) return;
      settled = true;
      resolve(handle);
    };
    socket.onmessage = (event) => {
      if (closed) return;
      if (typeof event.data !== "string") { fail(new CockpitClientError("malformed_response", "Session stream message is not JSON text"), !settled); return; }
      let raw: unknown;
      try { raw = JSON.parse(event.data); } catch (cause) { fail(new CockpitClientError("malformed_response", "Session stream message is invalid JSON", { cause }), !settled); return; }
      let message: SessionStreamMessage;
      try { message = parseSessionStreamMessage(raw); } catch (error) { fail(error instanceof CockpitClientError ? error : streamError("Session stream message is malformed", error), !settled); return; }
      const sequenceError = checkSequence(message);
      if (sequenceError) { fail(sequenceError); return; }
      onMessage(message);
    };
    socket.onerror = (cause) => fail(new CockpitClientError("transport_error", "Session WebSocket failed", { cause }), !settled);
    socket.onclose = (event) => {
      if (closed) return;
      closed = true;
      const error = streamError(`Session WebSocket closed${event.reason ? `: ${event.reason}` : ""}`, event);
      if (!settled) { settled = true; reject(error); } else onError(error);
    };
  });
}

function openTerminalStream(
  webSocketFactory: BrowserWebSocketFactory,
  request: TerminalOpenRequest,
  onMessage: (message: TerminalStreamMessage) => void,
  onError: (error: CockpitClientError) => void,
): Promise<TerminalStream> {
  let validated: TerminalOpenRequest;
  try { validated = parseTerminalOpenRequest(request); } catch (error) { return Promise.reject(error); }
  const path = `/api/v1/sessions/${encodeURIComponent(validated.session_id)}/panes/${encodeURIComponent(validated.pane_id)}/terminal?mode=${validated.mode}&takeover=${validated.takeover ? "true" : "false"}&cols=${validated.cols}&rows=${validated.rows}`;
  return new Promise((resolve, reject) => {
    let socket: BrowserWebSocket;
    let settled = false;
    let closed = false;
    let streamId: string | undefined;
    let lastFrameSequence: bigint | undefined;
    let receivedFrame = false;
    const fail = (error: CockpitClientError, beforeOpen = false) => {
      if (beforeOpen && !settled) { settled = true; reject(error); }
      else onError(error);
      if (!closed) { closed = true; socket.close(); }
    };
    try { socket = webSocketFactory(websocketUrl(path)); } catch (cause) {
      reject(new CockpitClientError("transport_error", "Could not open terminal stream", { cause })); return;
    }
    const handle: TerminalStream = {
      send(command: TerminalCommand) {
        if (closed || !settled || socket.readyState !== 1) throw new CockpitClientError("stream_error", "Terminal stream is not ready");
        const parsed = parseTerminalCommand(command);
        socket.send(JSON.stringify(parsed));
      },
      close() { if (!closed) { closed = true; socket.close(); } },
    };
    socket.onopen = () => { if (!closed) { settled = true; resolve(handle); } };
    socket.onmessage = (event) => {
      if (closed) return;
      if (typeof event.data !== "string") { fail(new CockpitClientError("malformed_response", "Terminal stream message is not JSON text"), !settled); return; }
      let raw: unknown;
      try { raw = JSON.parse(event.data); } catch (cause) { fail(new CockpitClientError("malformed_response", "Terminal stream message is invalid JSON", { cause }), !settled); return; }
      let message: TerminalStreamMessage;
      try { message = parseTerminalStreamMessage(raw); }
      catch (error) { fail(error instanceof CockpitClientError ? error : streamError("Terminal stream message is malformed", error), !settled); return; }
      if (message.session_id !== validated.session_id || message.pane_id !== validated.pane_id) {
        fail(streamError("Terminal stream message belongs to another session or pane"));
        return;
      }
      if (streamId === undefined) streamId = message.stream_id;
      else if (message.stream_id !== streamId) { fail(streamError("Terminal stream message belongs to another stream")); return; }
      if (message.type === "frame") {
        const current = BigInt(message.seq);
        if (!receivedFrame && !message.full) { fail(streamError("Terminal stream must begin with a full frame")); return; }
        if (receivedFrame && !message.full && current !== lastFrameSequence! + 1n) {
          fail(streamError("Terminal frame sequence is not consecutive"));
          return;
        }
        lastFrameSequence = current;
        receivedFrame = true;
      }
      onMessage(message);
    };
    socket.onerror = (cause) => fail(new CockpitClientError("transport_error", "Terminal WebSocket failed", { cause }), !settled);
    socket.onclose = (event) => {
      if (closed) return;
      closed = true;
      const error = streamError(`Terminal WebSocket closed${event.reason ? `: ${event.reason}` : ""}`, event);
      if (!settled) { settled = true; reject(error); } else onError(error);
    };
  });
}

export function createBrowserClient(
  request: BrowserFetch = defaultFetch,
  webSocketFactory: BrowserWebSocketFactory = defaultWebSocket,
): CockpitClient {
  return {
    status(): Promise<StatusResponse> { return getJson(request, "/api/v1/status", "status", parseStatusResponse); },
    sessions(): Promise<SessionListResponse> { return getJson(request, "/api/v1/sessions", "sessions", parseSessionListResponse); },
    sessionSnapshot(sessionId: string): Promise<SessionSnapshotResponse> {
      try { validateSessionId(sessionId); } catch (error) { return Promise.reject(error); }
      return getJson(request, `/api/v1/sessions/${encodeURIComponent(sessionId)}/snapshot`, "session snapshot", parseSessionSnapshotResponse).then((value) => {
        if (value.session_id !== sessionId) throw new CockpitClientError("malformed_response", "Session snapshot belongs to another session");
        return value;
      });
    },
    focus(sessionId: string, focusRequest: FocusRequest): Promise<FocusResponse> {
      try { validateSessionId(sessionId); } catch (error) { return Promise.reject(error); }
      let parsed: FocusRequest;
      try { parsed = parseFocusRequest(focusRequest); } catch (error) { return Promise.reject(error); }
      return getJson(request, `/api/v1/sessions/${encodeURIComponent(sessionId)}/focus`, "focus", parseFocusResponse, {
        method: "POST",
        headers: { "Content-Type": "application/json" },
        body: JSON.stringify(parsed),
      }).then((value) => {
        if (value.session_id !== sessionId) throw new CockpitClientError("malformed_response", "Focus response belongs to another session");
        return value;
      });
    },
    mutate(sessionId: string, mutationRequest: ResourceMutationRequest): Promise<ResourceMutationResponse> {
      try { validateSessionId(sessionId); } catch (error) { return Promise.reject(error); }
      let parsed: ResourceMutationRequest;
      try { parsed = parseResourceMutationRequest(mutationRequest); } catch (error) { return Promise.reject(error); }
      return getJson(
        request,
        `/api/v1/sessions/${encodeURIComponent(sessionId)}/mutations`,
        "mutation",
        parseResourceMutationResponse,
        {
          method: "POST",
          headers: { "Content-Type": "application/json" },
          body: JSON.stringify(parsed),
        },
      ).then((value) => {
        if (value.session_id !== sessionId || value.snapshot.session_id !== sessionId) {
          throw new CockpitClientError("malformed_response", "Mutation response belongs to another session");
        }
        return value;
      });
    },
    subscribeSession(sessionId, onMessage, onError) { return openSessionStream(webSocketFactory, sessionId, onMessage, onError); },
    openTerminal(requestValue, onMessage, onError) { return openTerminalStream(webSocketFactory, requestValue, onMessage, onError); },
  };
}
