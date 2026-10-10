import { widgetEventCursor, widgetReports } from "./widgetTransport";
import type { WidgetEventHandler, WidgetStream } from "./CockpitClient";
import {
  CockpitClientError, matchBrowserViewCommandResponse, parseBrowserViewCommandRequest,
  parseBrowserViewFrameDescriptor, parseBrowserViewOpenRequest, parseBrowserViewSnapshot, parseErrorEnvelope,
  parseSessionStreamMessage, parseTerminalOpenRequest, parseTerminalStreamMessage, parseTerminalCommand,
  validateSessionId, type BrowserViewFramePacket, type BrowserViewOpenRequest, type BrowserViewStream,
  type BrowserViewSnapshot, type ClosableStream, type CockpitClient, type TerminalStream,
} from "./CockpitClient";
import type {
  BrowserViewCommandRequest, BrowserViewEvent, BrowserViewFrameDescriptor, SessionStreamMessage,
  TerminalCommand, TerminalOpenRequest, TerminalStreamMessage,
} from "../protocol/generated/v1";
import { transitionSessionStream, type StreamOrderCursor } from "./streamOrder";
import { bindOperations } from "./operations";
import { browserViewDecoder, type BrowserFrameEnvelope, type BrowserFrameRelease } from "./browserViewDecoder";

export type BrowserFetch = (input: string, init?: RequestInit) => Promise<Response>;

export interface BrowserWebSocket {
  readonly readyState: number;
  /**
   * Browser WebSockets default to Blob delivery. The default factory changes
   * this to "arraybuffer"; custom factories may omit the property, so frame
   * parsing still validates the runtime message type.
   */
  binaryType?: BinaryType;
  onopen: ((event: Event) => void) | null;
  onmessage: ((event: MessageEvent) => void) | null;
  onerror: ((event: Event) => void) | null;
  onclose: ((event: CloseEvent) => void) | null;
  send(data: string | ArrayBuffer): void;
  close(code?: number, reason?: string): void;
}

export type BrowserWebSocketFactory = (url: string) => BrowserWebSocket;

function defaultFetch(input: string, init?: RequestInit): Promise<Response> {
  return globalThis.fetch(input, init);
}
function defaultWebSocket(url: string): BrowserWebSocket {
  const socket = new WebSocket(url);
  socket.binaryType = "arraybuffer";
  return socket;
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
    const abortError = typeof cause === "object" && cause !== null
      && "name" in cause && cause.name === "AbortError";
    if (init?.signal?.aborted || abortError) {
      throw cause;
    }
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

function openWidgetStream(
  factory: BrowserWebSocketFactory, onEvent: WidgetEventHandler,
  onError: (error: CockpitClientError) => void, signal?: AbortSignal,
): Promise<WidgetStream> {
  try { signal?.throwIfAborted(); } catch (error) { return Promise.reject(error); }
  let socket: BrowserWebSocket;
  try { socket = factory(websocketUrl("/api/v1/widgets/events")); }
  catch (cause) { return Promise.reject(new CockpitClientError("transport_error", "Widget stream could not open", { cause })); }
  let resolve!: (stream: WidgetStream) => void;
  let reject!: (error: unknown) => void;
  // The shipped frontend targets ES2022, before Promise.withResolvers.
  const promise = new Promise<WidgetStream>((accept, fail) => { resolve = accept; reject = fail; });
  const accept = widgetEventCursor();
  let closed = false;
  let ready = false;
  const reports = widgetReports((report) => {
    try { socket.send(JSON.stringify(report)); }
    catch (cause) { fail(new CockpitClientError("stream_error", "Widget report failed", { cause })); }
  });
  const close = () => {
    if (closed) return;
    closed = true;
    reports.close();
    signal?.removeEventListener("abort", abort);
    socket.onopen = socket.onmessage = socket.onerror = socket.onclose = null;
    socket.close();
  };
  const fail = (error: CockpitClientError) => {
    if (closed) return;
    close();
    if (ready) onError(error); else reject(error);
  };
  const abort = () => {
    close();
    if (!ready) reject(signal?.reason ?? new DOMException("Aborted", "AbortError"));
  };
  signal?.addEventListener("abort", abort, { once: true });
  socket.onmessage = ({ data }) => {
    if (closed) return;
    try {
      if (typeof data !== "string" || new TextEncoder().encode(data).byteLength > 8 * 1024 * 1024) {
        throw new CockpitClientError("malformed_response", "Widget event exceeds transport bounds");
      }
      const event = accept(JSON.parse(data));
      if (!event) return;
      onEvent(event);
      if (!ready) {
        ready = true;
        resolve({ close, report: reports.report });
      }
    } catch (cause) {
      fail(cause instanceof CockpitClientError ? cause : new CockpitClientError("malformed_response", "Widget event is malformed", { cause }));
    }
  };
  socket.onerror = () => fail(new CockpitClientError("transport_error", "Widget stream failed"));
  socket.onclose = () => fail(new CockpitClientError("stream_error", "Widget stream disconnected"));
  if (signal?.aborted) abort();
  return promise;
}

function websocketUrl(path: string): string {
  if (/^wss?:\/\//.test(path)) return path;
  const location = globalThis.location;
  if (location?.host) {
    const protocol = location.protocol === "https:" ? "wss:" : "ws:";
    return `${protocol}//${location.host}${path}`;
  }
  return path;
}
interface BrowserViewOpenResponse {
  snapshot: BrowserViewSnapshot;
  first_frame: BrowserViewFrameDescriptor;
  frame_endpoint: string;
}

function parseBrowserViewOpenResponse(value: unknown): BrowserViewOpenResponse {
  if (typeof value !== "object" || value === null) throw new CockpitClientError("malformed_response", "Browser view open response is malformed");
  const body = value as Record<string, unknown>;
  if (typeof body.frame_endpoint !== "string" || body.frame_endpoint.length === 0) throw new CockpitClientError("malformed_response", "Browser view frame endpoint is missing");
  return {
    snapshot: parseBrowserViewSnapshot(body.snapshot),
    first_frame: parseBrowserViewFrameDescriptor(body.first_frame),
    frame_endpoint: body.frame_endpoint,
  };
}


interface PendingBrowserFrame {
  readonly envelope: BrowserFrameEnvelope;
  readonly release: BrowserFrameRelease;
}


function openBrowserViewStream(
  request: BrowserFetch,
  webSocketFactory: BrowserWebSocketFactory,
  value: BrowserViewOpenRequest,
  onEvent: (event: BrowserViewEvent) => void,
  onFrame: (packet: BrowserViewFramePacket) => void,
  onError: (error: CockpitClientError) => void,
  signal?: AbortSignal,
): Promise<BrowserViewStream> {
  let parsed: BrowserViewOpenRequest;
  try { parsed = parseBrowserViewOpenRequest(value); signal?.throwIfAborted(); } catch (error) { return Promise.reject(error); }
  const opened = getJson(request, "/api/v1/browser/view/open", "browser view open", parseBrowserViewOpenResponse, { method: "POST", headers: { "Content-Type": "application/json" }, body: JSON.stringify(parsed), signal });
  const detachOpened = (opened: BrowserViewOpenResponse) => {
    const identity = opened.snapshot.identity;
    const command: BrowserViewCommandRequest = {
      view_id: identity.view_id,
      stream_epoch: identity.stream_epoch,
      request_id: `browser-abort-${Date.now().toString(36)}-${Math.random().toString(36).slice(2)}`,
      command: { type: "detach" },
    };
    void getJson(
      request,
      "/api/v1/browser/view/command",
      "browser view detach",
      (response) => matchBrowserViewCommandResponse(response, command),
      { method: "POST", headers: { "Content-Type": "application/json" }, body: JSON.stringify(command) },
    ).catch(() => undefined);
  };
  const start = (opened: BrowserViewOpenResponse) => new Promise<BrowserViewStream>((resolve, reject) => {
    const identity = opened.snapshot.identity;
    const detach = () => detachOpened(opened);
    const grant = opened.snapshot.frame_grant;
    if (grant === null) { reject(new CockpitClientError("malformed_response", "Browser view frame grant is missing")); return; }
    let eventSocket: BrowserWebSocket; let frameSocket: BrowserWebSocket;
    let closed = false; let closing = false; let settled = false; let openCount = 0;
    let abort: () => void = () => undefined;
    const decoder = browserViewDecoder(opened.snapshot, "browser", onEvent, onFrame);
    // Keep one raw frame until the attached snapshot establishes its target identity.
    let pendingFrame: PendingBrowserFrame | null = null;
    const settlePending = () => {
      const pending = pendingFrame;
      pendingFrame = null;
      pending?.release("discard");
    };
    const fail = (error: CockpitClientError) => {
      if (closed || closing) return;
      closing = true;
      settlePending();
      closed = true;
      signal?.removeEventListener("abort", abort);
      eventSocket?.close();
      frameSocket?.close();
      if (!settled) { settled = true; reject(error); }
      else onError(error);
    };
    const makeRelease = (frameSequence: number): BrowserFrameRelease => {
      let released = false;
      return (kind) => {
        if (released) return;
        released = true;
        if (closed) return;
        try { frameSocket.send(JSON.stringify({ type: kind, frame_sequence: frameSequence })); }
        catch (cause) { fail(new CockpitClientError("transport_error", `Could not ${kind} browser frame`, { cause })); }
      };
    };
    const deliverFrame = (frame: PendingBrowserFrame) => decoder.browserFrame(frame.envelope, frame.release);
    const flushPending = () => {
      const pending = pendingFrame;
      pendingFrame = null;
      if (!pending) return;
      if (closed || !decoder.attached) { pending.release("discard"); return; }
      try { deliverFrame(pending); }
      catch (error) { fail(error instanceof CockpitClientError ? error : new CockpitClientError("transport_error", "Browser view frame handler failed", { cause: error })); }
    };
    const packet = (data: unknown) => {
      if (closed) return;
      try {
        const envelope = decoder.envelope(data);
        const frame: PendingBrowserFrame = { envelope, release: makeRelease(envelope.frameSequence) };
        if (!decoder.attached) {
          const superseded = pendingFrame;
          pendingFrame = null;
          superseded?.release("discard");
          if (closed) { frame.release("discard"); return; }
          pendingFrame = frame;
          return;
        }
        deliverFrame(frame);
      } catch (error) {
        fail(error instanceof CockpitClientError ? error : new CockpitClientError("malformed_response", "Browser frame is malformed", { cause: error }));
      }
    };
    try {
      eventSocket = webSocketFactory(websocketUrl(`/api/v1/browser/view/events/${encodeURIComponent(identity.view_id)}`));
      frameSocket = webSocketFactory(websocketUrl(opened.frame_endpoint));
    } catch (cause) { fail(new CockpitClientError("transport_error", "Could not open browser view streams", { cause })); return; }
    let stream: BrowserViewStream;
    const maybeReady = () => { if (!closed && ++openCount === 2) { settled = true; resolve(stream); } };
    stream = {
      close() {
        if (closed || closing) return;
        closing = true;
        settlePending();
        closed = true;
        signal?.removeEventListener("abort", abort);
        eventSocket.close();
        frameSocket.close();
      },
      command(commandValue) {
        if (closed || !settled) return Promise.reject(new CockpitClientError("stream_error", "Browser view stream is not ready"));
        let command: BrowserViewCommandRequest;
        try { command = parseBrowserViewCommandRequest(commandValue); } catch (error) { return Promise.reject(error); }
        if (command.view_id !== identity.view_id || command.stream_epoch !== identity.stream_epoch) return Promise.reject(new CockpitClientError("malformed_response", "Browser view command identity does not match"));
        return getJson(request, "/api/v1/browser/view/command", "browser view command", (response) => matchBrowserViewCommandResponse(response, command), { method: "POST", headers: { "Content-Type": "application/json" }, body: JSON.stringify(command), signal });
      },
    };
    abort = () => { detach(); fail(new CockpitClientError("stream_error", "Browser view attach was cancelled")); };
    if (signal?.aborted) { abort(); return; }
    signal?.addEventListener("abort", abort, { once: true });
    eventSocket.onopen = maybeReady;
    frameSocket.onopen = () => { frameSocket.send(JSON.stringify({ grant })); maybeReady(); };
    eventSocket.onmessage = (event) => {
      if (closed || typeof event.data !== "string") {
        if (!closed) fail(new CockpitClientError("malformed_response", "Browser view metadata is not JSON text"));
        return;
      }
      try {
        if (decoder.event(JSON.parse(event.data))) flushPending();
      } catch (error) {
        fail(error instanceof CockpitClientError ? error : new CockpitClientError("malformed_response", "Browser view metadata is malformed", { cause: error }));
      }
    };
    frameSocket.onmessage = (event) => packet(event.data);
    eventSocket.onerror = (cause) => fail(new CockpitClientError("transport_error", "Browser view metadata WebSocket failed", { cause }));
    frameSocket.onerror = (cause) => fail(new CockpitClientError("transport_error", "Browser view frame WebSocket failed", { cause }));
    eventSocket.onclose = (event) => { if (!closed) fail(streamError(`Browser view metadata WebSocket closed${event.reason ? `: ${event.reason}` : ""}`, event)); };
    frameSocket.onclose = (event) => { if (!closed) fail(streamError(`Browser view frame WebSocket closed${event.reason ? `: ${event.reason}` : ""}`, event)); };
  });
  if (!signal) return opened.then(start);
  return new Promise((resolve, reject) => {
    let retired = false;
    const abort = () => {
      if (retired) return;
      retired = true;
      reject(new CockpitClientError("stream_error", "Browser view attach was cancelled"));
    };
    if (signal.aborted) {
      abort();
    } else {
      signal.addEventListener("abort", abort, { once: true });
    }
    void opened.then((value) => {
      signal.removeEventListener("abort", abort);
      if (retired) {
        detachOpened(value);
        return;
      }
      void start(value).then(resolve, reject);
    }, (error) => {
      signal.removeEventListener("abort", abort);
      if (!retired) reject(error);
    });
  });
}

function streamError(message: string, cause?: unknown, operationCode?: string): CockpitClientError {
  return new CockpitClientError("stream_error", message, { cause, operationCode });
}
function openSessionStream(
  webSocketFactory: BrowserWebSocketFactory,
  sessionId: string,
  onMessage: (message: SessionStreamMessage) => void,
  onError: (error: CockpitClientError) => void,
  signal?: AbortSignal,
): Promise<ClosableStream> {
  try {
    validateSessionId(sessionId);
    signal?.throwIfAborted();
  } catch (error) {
    return Promise.reject(error);
  }
  return new Promise((resolve, reject) => {
    let socket: BrowserWebSocket | undefined;
    let settled = false;
    let closed = false;
    let cursor: StreamOrderCursor | null = null;
    const cleanup = () => signal?.removeEventListener("abort", abort);
    const abort = () => {
      if (closed) return;
      closed = true;
      cleanup();
      socket?.close();
      if (!settled) {
        settled = true;
        reject(streamError("Session subscription was cancelled"));
      }
    };
    const fail = (error: CockpitClientError, beforeOpen = false) => {
      if (closed) return;
      closed = true;
      cleanup();
      socket?.close();
      if (beforeOpen && !settled) {
        settled = true;
        reject(error);
      } else {
        onError(error);
      }
    };
    if (signal?.aborted) {
      abort();
      return;
    }
    signal?.addEventListener("abort", abort, { once: true });
    try {
      socket = webSocketFactory(websocketUrl(`/api/v1/sessions/${encodeURIComponent(sessionId)}/events`));
    } catch (cause) {
      cleanup();
      reject(new CockpitClientError("transport_error", "Could not open session stream", { cause }));
      return;
    }
    const handle: ClosableStream = {
      close() {
        if (closed) return;
        closed = true;
        cleanup();
        socket?.close();
      },
    };
    socket.onopen = () => {
      if (closed) return;
      settled = true;
      resolve(handle);
    };
    socket.onmessage = (event) => {
      if (closed) return;
      if (typeof event.data !== "string") {
        fail(new CockpitClientError("malformed_response", "Session stream message is not JSON text"), !settled);
        return;
      }
      let raw: unknown;
      try {
        raw = JSON.parse(event.data);
      } catch (cause) {
        fail(new CockpitClientError("malformed_response", "Session stream message is invalid JSON", { cause }), !settled);
        return;
      }
      let message: SessionStreamMessage;
      try {
        message = parseSessionStreamMessage(raw);
      } catch (error) {
        fail(error instanceof CockpitClientError ? error : streamError("Session stream message is malformed", error), !settled);
        return;
      }
      const result = transitionSessionStream(sessionId, cursor, message);
      if (result.kind === "ignore") return;
      if (result.kind === "error") {
        fail(streamError(result.message, result.classification, result.code));
        return;
      }
      cursor = result.cursor;
      onMessage(message);
    };
    socket.onerror = (cause) => fail(new CockpitClientError("transport_error", "Session WebSocket failed", { cause }), !settled);
    socket.onclose = (event) => {
      if (closed) return;
      closed = true;
      cleanup();
      const error = streamError(`Session WebSocket closed${event.reason ? `: ${event.reason}` : ""}`, event);
      if (!settled) {
        settled = true;
        reject(error);
      } else {
        onError(error);
      }
    };
  });
}

function openTerminalStream(
  webSocketFactory: BrowserWebSocketFactory,
  request: TerminalOpenRequest,
  onMessage: (message: TerminalStreamMessage) => void,
  onError: (error: CockpitClientError) => void,
  signal?: AbortSignal,
): Promise<TerminalStream> {
  let validated: TerminalOpenRequest;
  try { validated = parseTerminalOpenRequest(request); } catch (error) { return Promise.reject(error); }
  const path = `/api/v1/sessions/${encodeURIComponent(validated.session_id)}/panes/${encodeURIComponent(validated.pane_id)}/terminal?mode=${validated.mode}&takeover=${validated.takeover ? "true" : "false"}&cols=${validated.cols}&rows=${validated.rows}&cell_width_px=${validated.cell_width_px}&cell_height_px=${validated.cell_height_px}${validated.target_kind === "popup" ? "&target_kind=popup" : ""}`;
  return new Promise((resolve, reject) => {
    let socket: BrowserWebSocket | undefined;
    let settled = false;
    let closed = false;
    let streamId: string | undefined;
    let lastFrameSequence: bigint | undefined;
    let receivedFrame = false;
    const fail = (error: CockpitClientError, beforeOpen = false) => {
      if (closed) return;
      closed = true;
      signal?.removeEventListener("abort", abort);
      socket?.close();
      if (beforeOpen && !settled) { settled = true; reject(error); }
      else onError(error);
    };
    const abort = () => {
      if (closed) return;
      closed = true;
      socket?.close();
      if (!settled) { settled = true; reject(new CockpitClientError("stream_error", "Terminal attach was cancelled")); }
    };
    if (signal?.aborted) { abort(); return; }
    signal?.addEventListener("abort", abort, { once: true });
    try { socket = webSocketFactory(websocketUrl(path)); } catch (cause) {
      signal?.removeEventListener("abort", abort);
      reject(new CockpitClientError("transport_error", "Could not open terminal stream", { cause })); return;
    }
    const handle: TerminalStream = {
      send(command: TerminalCommand) {
        if (closed || !settled || socket.readyState !== 1) throw new CockpitClientError("stream_error", "Terminal stream is not ready");
        const parsed = parseTerminalCommand(command);
        socket.send(JSON.stringify(parsed));
      },
      close() { if (!closed) { closed = true; signal?.removeEventListener("abort", abort); socket?.close(); } },
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
        if (receivedFrame && current !== lastFrameSequence! + 1n) {
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
      signal?.removeEventListener("abort", abort);
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
    ...bindOperations("browser", (operation) => {
      const http = operation.http;
      const path = http.route();
      let init: RequestInit | undefined;
      if (http.method === "POST") {
        init = { method: http.method };
        if (http.json) {
          init.headers = { [http.contentTypeHeader ?? "Content-Type"]: "application/json" };
          init.body = JSON.stringify(http.json());
        }
      }
      if (http.signal) { init ??= {}; init.signal = http.signal(); }
      return getJson(request, path, operation.operation, operation.parse, init);
    }),
    subscribeWidgets(onEvent, onError, signal) {
      return openWidgetStream(webSocketFactory, onEvent, onError, signal);
    },
    subscribeSession(sessionId, onMessage, onError, signal) { return openSessionStream(webSocketFactory, sessionId, onMessage, onError, signal); },
    openTerminal(requestValue, onMessage, onError, signal) { return openTerminalStream(webSocketFactory, requestValue, onMessage, onError, signal); },
    openBrowserView(requestValue, onEvent, onFrame, onError, signal) {
      return openBrowserViewStream(request, webSocketFactory, requestValue, onEvent, onFrame, onError, signal);
    },
  };
}
