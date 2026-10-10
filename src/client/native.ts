import { widgetEventCursor, widgetReports } from "./widgetTransport";
import type { WidgetEventHandler, WidgetStream } from "./CockpitClient";
import { Channel, invoke as tauriInvoke } from "@tauri-apps/api/core";
import type {
  SessionStreamMessage, TerminalCommand, TerminalOpenRequest, TerminalStreamMessage,
} from "../protocol/generated/v1";
import { transitionSessionStream, type StreamOrderCursor } from "./streamOrder";
import {
  CockpitClientError, matchBrowserViewCommandResponse, parseBrowserViewCommandRequest,
  parseBrowserViewFrameDescriptor, parseBrowserViewOpenRequest, parseBrowserViewSnapshot, parseErrorEnvelope,
  parseSessionStreamMessage, parseTerminalCommand, parseTerminalOpenRequest, parseTerminalStreamMessage,
  validateSessionId, type BrowserViewCommandRequest, type BrowserViewEvent, type BrowserViewFrameDescriptor,
  type BrowserViewFramePacket, type BrowserViewOpenRequest, type BrowserViewSnapshot, type BrowserViewStream,
  type ClosableStream, type CockpitClient, type TerminalStream,
} from "./CockpitClient";
import { bindOperations } from "./operations";
import { browserViewDecoder } from "./browserViewDecoder";

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
interface NativeBrowserViewOpenResponse { snapshot: BrowserViewSnapshot; first_frame: BrowserViewFrameDescriptor; }
function parseNativeBrowserViewOpen(value: unknown): NativeBrowserViewOpenResponse {
  if (typeof value !== "object" || value === null) throw new CockpitClientError("malformed_response", "Native browser view open response is malformed");
  const body = value as Record<string, unknown>;
  if (!("snapshot" in body) || !("first_frame" in body)) throw new CockpitClientError("malformed_response", "Native browser view open response is incomplete");
  return { snapshot: parseBrowserViewSnapshot(body.snapshot), first_frame: parseBrowserViewFrameDescriptor(body.first_frame) };
}

interface NativeBrowserViewSubscription {
  stream_id: string;
  endpoint: string;
  grant: string;
}

function parseNativeBrowserViewSubscription(value: unknown): NativeBrowserViewSubscription {
  if (typeof value !== "object" || value === null) throw new CockpitClientError("malformed_response", "Native browser view subscription response is malformed");
  const body = value as Record<string, unknown>;
  if (typeof body.stream_id !== "string" || body.stream_id.length === 0 || body.stream_id.length > 256
    || typeof body.endpoint !== "string" || body.endpoint.length === 0 || body.endpoint.length > 2048
    || typeof body.grant !== "string" || body.grant.length === 0 || body.grant.length > 4096) {
    throw new CockpitClientError("malformed_response", "Native browser view subscription response is incomplete");
  }
  let endpoint: URL;
  try { endpoint = new URL(body.endpoint); } catch (cause) {
    throw new CockpitClientError("malformed_response", "Native browser view WebSocket endpoint is invalid", { cause });
  }
  if (endpoint.protocol !== "ws:" || endpoint.hostname !== "127.0.0.1" || endpoint.port.length === 0
    || endpoint.username !== "" || endpoint.password !== "" || endpoint.hash !== "") {
    throw new CockpitClientError("malformed_response", "Native browser view WebSocket endpoint is not a loopback endpoint");
  }
  return { stream_id: body.stream_id, endpoint: endpoint.href, grant: body.grant };
}

function nativeBrowserViewSubscription(
  invoke: NativeInvoke,
  value: BrowserViewOpenRequest,
  onEvent: (event: BrowserViewEvent) => void,
  onFrame: (packet: BrowserViewFramePacket) => void,
  onError: (error: CockpitClientError) => void,
  signal?: AbortSignal,
): Promise<BrowserViewStream> {
  let request: BrowserViewOpenRequest;
  try { request = parseBrowserViewOpenRequest(value); signal?.throwIfAborted(); } catch (error) { return Promise.reject(error); }
  const opened = invokeAndParse(invoke, "cockpit_browser_view_open", { request }, "browser view open", parseNativeBrowserViewOpen);
  const start = (openedView: NativeBrowserViewOpenResponse) => new Promise<BrowserViewStream>((resolve, reject) => {
    const identity = openedView.snapshot.identity;
    const releaseOpenedView = () => {
      void invoke("cockpit_browser_view_release", { viewId: identity.view_id }).catch(() => undefined);
    };
    let closed = false;
    let settled = false;
    let streamId: string | undefined;
    let socket: WebSocket | undefined;
    let pendingFrame: { descriptor: BrowserViewFrameDescriptor; sequence: number } | undefined;
    const decoder = browserViewDecoder(openedView.snapshot, "native", onEvent, onFrame);
    let cancellationRequested = false;
    let ready = false;
    const cancel = (id: string) => {
      if (cancellationRequested) return;
      cancellationRequested = true;
      void invoke("cockpit_stream_cancel", { streamId: id }).catch(() => undefined);
    };
    const closeSocket = () => {
      if (!socket) return;
      socket.onopen = null;
      socket.onmessage = null;
      socket.onerror = null;
      socket.onclose = null;
      try { socket.close(); } catch { /* Cleanup remains idempotent if the browser already retired it. */ }
    };
    let abort: () => void = () => undefined;
    const fail = (error: CockpitClientError) => {
      if (closed) return;
      const pending = pendingFrame;
      pendingFrame = undefined;
      if (pending) releaseFrame(pending.sequence, "discard");
      closed = true;
      signal?.removeEventListener("abort", abort);
      closeSocket();
      if (streamId !== undefined) cancel(streamId);
      else releaseOpenedView();
      if (!settled) { settled = true; reject(error); }
      else {
        try { onError(error); } catch { /* Stream retirement must survive consumer callback failures. */ }
      }
    };
    const releaseFrame = (sequence: number, kind: "ack" | "discard") => {
      if (closed || !ready || !socket || socket.readyState !== WebSocket.OPEN) return;
      try { socket.send(JSON.stringify({ type: kind, frame_sequence: sequence })); }
      catch (cause) { fail(new CockpitClientError("transport_error", `Could not ${kind} native browser frame`, { cause })); }
    };
    const releasePendingFrame = () => {
      const pending = pendingFrame;
      if (!pending) return;
      pendingFrame = undefined;
      releaseFrame(pending.sequence, "discard");
    };
    const deliverFrame = (raw: unknown) => {
      const pending = pendingFrame;
      if (!pending) throw new CockpitClientError("malformed_response", "Native browser frame payload has no descriptor");
      pendingFrame = undefined;
      let delivered = false;
      const release = (kind: "ack" | "discard") => {
        if (delivered || closed) return;
        delivered = true;
        releaseFrame(pending.sequence, kind);
      };
      decoder.nativeFrame(raw, pending.descriptor, release);
    };
    const message = (raw: unknown) => {
      if (closed) return;
      try {
        if (raw instanceof ArrayBuffer || raw instanceof Uint8Array) {
          deliverFrame(raw);
          return;
        }
        if (typeof raw !== "string") throw new CockpitClientError("malformed_response", "Native browser view message is not JSON text or binary");
        if (raw.length > 1024 * 1024) throw new CockpitClientError("malformed_response", "Native browser view message exceeds bounds");
        if (pendingFrame !== undefined) throw new CockpitClientError("malformed_response", "Native browser frame payload is missing");
        let parsed: unknown;
        try { parsed = JSON.parse(raw); } catch (cause) {
          throw new CockpitClientError("malformed_response", "Native browser view message is invalid JSON", { cause });
        }
        if (typeof parsed !== "object" || parsed === null) throw new CockpitClientError("malformed_response", "Native browser view message is malformed");
        const body = parsed as Record<string, unknown>;
        if (body.kind === "ready") {
          if (ready) throw new CockpitClientError("malformed_response", "Native browser view stream sent ready more than once");
          ready = true;
          settled = true;
          resolve({
            close() {
              if (closed) return;
              releasePendingFrame();
              closed = true;
              signal?.removeEventListener("abort", abort);
              closeSocket();
              if (streamId !== undefined) cancel(streamId);
            },
            command(commandValue) {
              if (closed || !settled) return Promise.reject(new CockpitClientError("stream_error", "Browser view stream is not ready"));
              let command: BrowserViewCommandRequest;
              try { command = parseBrowserViewCommandRequest(commandValue); } catch (error) { return Promise.reject(error); }
              if (command.view_id !== identity.view_id || command.stream_epoch !== identity.stream_epoch) return Promise.reject(new CockpitClientError("malformed_response", "Browser view command identity does not match"));
              return invokeAndParse(invoke, "cockpit_browser_view_command", { request: command }, "browser view command", (response) => matchBrowserViewCommandResponse(response, command));
            },
          });
          return;
        }
        if (!ready && body.kind !== "error") throw new CockpitClientError("malformed_response", "Native browser view stream sent data before ready");
        if (body.kind === "error") {
          throw new CockpitClientError("stream_error", typeof body.message === "string" ? body.message : "Native browser view stream failed", { operationCode: typeof body.code === "string" ? body.code : undefined });
        }
        if (body.kind === "event") {
          decoder.event(body.event);
          return;
        }
        if (body.kind === "frame") {
          const descriptor = decoder.nativeDescriptor(body.descriptor);
          pendingFrame = { descriptor, sequence: descriptor.frame_sequence };
          return;
        }
        throw new CockpitClientError("malformed_response", "Native browser view message kind is unknown");
      } catch (error) {
        fail(error instanceof CockpitClientError ? error : new CockpitClientError("malformed_response", "Native browser view message is malformed", { cause: error }));
      }
    };
    abort = () => fail(new CockpitClientError("stream_error", "Browser view attach was cancelled"));
    if (signal?.aborted) abort(); else signal?.addEventListener("abort", abort, { once: true });
    if (closed) return;
    void invokeAndParse(invoke, "cockpit_browser_view_subscribe", { viewId: identity.view_id, streamEpoch: identity.stream_epoch }, "browser view subscription", parseNativeBrowserViewSubscription).then((subscription) => {
      streamId = subscription.stream_id;
      if (closed) { cancel(streamId); return; }
      try {
        socket = new WebSocket(subscription.endpoint);
        socket.binaryType = "arraybuffer";
        socket.onopen = () => {
          if (closed) return;
          try { socket?.send(JSON.stringify({ grant: subscription.grant })); }
          catch (cause) { fail(new CockpitClientError("transport_error", "Could not authenticate native browser view stream", { cause })); }
        };
        socket.onmessage = (event) => message(event.data);
        socket.onerror = (cause) => fail(new CockpitClientError("transport_error", "Native browser view WebSocket failed", { cause }));
        socket.onclose = (event) => {
          if (!closed) fail(streamFailure(`Native browser view WebSocket closed${event.reason ? `: ${event.reason}` : ""}`, event));
        };
      } catch (cause) {
        fail(new CockpitClientError("transport_error", "Could not open native browser view WebSocket", { cause }));
      }
    }, (error) => { if (!closed) fail(error); });
  });
  if (!signal) return opened.then(start);
  return new Promise((resolve, reject) => {
    let retired = false;
    const abort = () => {
      if (retired) return;
      retired = true;
      reject(new CockpitClientError("stream_error", "Browser view attach was cancelled"));
    };
    if (signal.aborted) abort(); else signal.addEventListener("abort", abort, { once: true });
    void opened.then((openedView) => {
      signal.removeEventListener("abort", abort);
      if (retired) {
        void invoke("cockpit_browser_view_release", { viewId: openedView.snapshot.identity.view_id }).catch(() => undefined);
        return;
      }
      void start(openedView).then(resolve, reject);
    }, (error) => {
      signal.removeEventListener("abort", abort);
      if (!retired) reject(error);
    });
  });
}

function streamFailure(message: string, cause?: unknown, operationCode?: string): CockpitClientError {
  return new CockpitClientError("stream_error", message, { cause, operationCode });
}
function streamId(value: unknown): string {
  if (typeof value === "string" && value.length > 0) return value;
  if (typeof value === "object" && value !== null && "stream_id" in value && typeof value.stream_id === "string" && value.stream_id.length > 0) return value.stream_id;
  throw new CockpitClientError("malformed_response", "Native stream command returned no stream id");
}

function nativeWidgetSubscription(
  factory: NativeChannelFactory, invoke: NativeInvoke, onEvent: WidgetEventHandler,
  onError: (error: CockpitClientError) => void, signal?: AbortSignal,
): Promise<WidgetStream> {
  try { signal?.throwIfAborted(); } catch (error) { return Promise.reject(error); }
  let resolve!: (stream: WidgetStream) => void;
  let reject!: (error: unknown) => void;
  const promise = new Promise<WidgetStream>((accept, fail) => { resolve = accept; reject = fail; });
  const accept = widgetEventCursor();
  let closed = false;
  let ready = false;
  let snapshotSeen = false;
  let activeId: string | undefined;
  const cancel = (id: string) => {
    void invoke("cockpit_stream_cancel", { streamId: id }).catch(() => undefined);
  };
  const reports = widgetReports((report) => {
    if (closed || activeId === undefined) return;
    void invokeAndParse(invoke, "cockpit_widget_report", { streamId: activeId, report }, "widget report", (value) => {
      if (value !== null && value !== undefined) throw new CockpitClientError("malformed_response", "Widget report response is malformed");
    }).catch((error: unknown) => {
      fail(error instanceof CockpitClientError ? error : new CockpitClientError("native_error", "Widget report failed", { cause: error }));
    });
  });
  const close = () => {
    if (closed) return;
    closed = true;
    reports.close();
    signal?.removeEventListener("abort", abort);
    if (activeId !== undefined) cancel(activeId);
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
  const settle = () => {
    if (!closed && !ready && snapshotSeen && activeId !== undefined) {
      ready = true;
      resolve({ close, report: reports.report });
    }
  };
  signal?.addEventListener("abort", abort, { once: true });
  try {
    const channel = factory<unknown>((raw) => {
      if (closed) return;
      try {
        const event = accept(raw);
        if (!event) return;
        onEvent(event);
        snapshotSeen = true;
        settle();
      } catch (cause) {
        fail(cause instanceof CockpitClientError ? cause : new CockpitClientError("malformed_response", "Widget event is malformed", { cause }));
      }
    });
    const errorChannel = factory<unknown>((raw) => {
      const envelope = parseErrorEnvelope(raw);
      fail(new CockpitClientError("stream_error", envelope?.message ?? "Widget stream failed", { operationCode: envelope?.code }));
    });
    void invokeAndParse(invoke, "cockpit_widget_subscribe", { channel, errorChannel }, "widget subscribe", streamId).then((id) => {
      activeId = id;
      if (closed) cancel(id); else settle();
    }, (cause: unknown) => fail(cause instanceof CockpitClientError ? cause : new CockpitClientError("native_error", "Widget subscription failed", { cause })));
  } catch (cause) {
    fail(cause instanceof CockpitClientError ? cause : new CockpitClientError("native_error", "Widget channels failed", { cause }));
  }
  if (signal?.aborted) abort();
  return promise;
}

function sessionSubscription(
  channelFactory: NativeChannelFactory,
  invoke: NativeInvoke,
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
  return new Promise<ClosableStream>((resolve, reject) => {
    let closed = false;
    let settled = false;
    let cancelled = false;
    let activeStreamId: string | undefined;
    let cursor: StreamOrderCursor | null = null;
    const cleanup = () => signal?.removeEventListener("abort", abort);
    const cancel = (id: string) => {
      if (cancelled) return;
      cancelled = true;
      void invoke("cockpit_stream_cancel", { streamId: id }).catch(() => undefined);
    };
    const fail = (error: CockpitClientError) => {
      if (closed) return;
      closed = true;
      cleanup();
      if (activeStreamId !== undefined) cancel(activeStreamId);
      if (!settled) {
        settled = true;
        reject(error);
      } else {
        onError(error);
      }
    };
    const abort = () => {
      if (closed) return;
      closed = true;
      cleanup();
      if (activeStreamId !== undefined) cancel(activeStreamId);
      if (!settled) {
        settled = true;
        reject(streamFailure("Session subscription was cancelled"));
      }
    };
    if (signal?.aborted) {
      abort();
      return;
    }
    signal?.addEventListener("abort", abort, { once: true });
    let channel: NativeChannel<unknown>;
    try {
      channel = channelFactory<unknown>((raw) => {
        if (closed) return;
        let message: SessionStreamMessage;
        try {
          message = parseSessionStreamMessage(raw);
        } catch (error) {
          fail(error instanceof CockpitClientError ? error : streamFailure("Session stream message is malformed", error));
          return;
        }
        const result = transitionSessionStream(sessionId, cursor, message);
        if (result.kind === "ignore") return;
        if (result.kind === "error") {
          fail(streamFailure(result.message, result.classification, result.code));
          return;
        }
        cursor = result.cursor;
        onMessage(message);
      });
    } catch (error) {
      closed = true;
      cleanup();
      settled = true;
      reject(error);
      return;
    }
    void invokeAndParse(invoke, "cockpit_session_subscribe", { sessionId, channel }, "session subscription", streamId).then((id) => {
      activeStreamId = id;
      if (closed) {
        cancel(id);
        return;
      }
      settled = true;
      resolve({
        close() {
          if (closed) return;
          closed = true;
          cleanup();
          cancel(id);
        },
      });
    }, (error: unknown) => {
      if (closed) return;
      closed = true;
      cleanup();
      settled = true;
      reject(error);
    });
  });
}

function terminalSubscription(channelFactory: NativeChannelFactory, invoke: NativeInvoke, request: TerminalOpenRequest, onMessage: (message: TerminalStreamMessage) => void, onError: (error: CockpitClientError) => void, signal?: AbortSignal): Promise<TerminalStream> {
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
  let abort: () => void = () => undefined;
  const removeAbortListener = () => signal?.removeEventListener("abort", abort);
  let ready = false;
  const channel = channelFactory<unknown>((raw) => {
    if (closed) return;
    let message: TerminalStreamMessage;
    try { message = parseTerminalStreamMessage(raw); }
    catch (error) {
      closed = true;
      removeAbortListener();
      onError(error instanceof CockpitClientError ? error : streamFailure("Terminal stream message is malformed", error));
      if (activeStreamId !== undefined) cancel(activeStreamId);
      return;
    }
    if (message.session_id !== validated.session_id || message.pane_id !== validated.pane_id) {
      closed = true;
      removeAbortListener();
      onError(streamFailure("Terminal stream message belongs to another session or pane"));
      if (activeStreamId !== undefined) cancel(activeStreamId);
      return;
    }
    if (activeStreamId === undefined) activeStreamId = message.stream_id;
    else if (message.stream_id !== activeStreamId) {
      closed = true;
      removeAbortListener();
      onError(streamFailure("Terminal stream message belongs to another stream"));
      cancel(activeStreamId);
      return;
    }
    if (message.type === "frame") {
      const current = BigInt(message.seq);
      if (!receivedFrame && !message.full) {
        closed = true;
        removeAbortListener();
        onError(streamFailure("Terminal stream must begin with a full frame"));
        if (activeStreamId !== undefined) cancel(activeStreamId);
        return;
      }
      if (receivedFrame && current !== lastFrameSequence! + 1n) {
        closed = true;
        removeAbortListener();
        onError(streamFailure("Terminal frame sequence is not consecutive"));
        if (activeStreamId !== undefined) cancel(activeStreamId);
        return;
      }
      lastFrameSequence = current;
      receivedFrame = true;
    }
    onMessage(message);
  });
  return new Promise<TerminalStream>((resolve, reject) => {
    let settled = false;
    abort = () => {
      if (closed) return;
      closed = true;
      ready = false;
      if (activeStreamId !== undefined) cancel(activeStreamId);
      if (!settled) { settled = true; reject(streamFailure("Terminal attach was cancelled")); }
    };
    if (signal?.aborted) { abort(); return; }
    signal?.addEventListener("abort", abort, { once: true });
    void invokeAndParse(invoke, "cockpit_terminal_open", { request: validated, channel }, "terminal open", streamId).then((id) => {
      activeStreamId = id;
      if (closed) { cancel(id); return; }
      ready = true;
      settled = true;
      resolve({
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
        signal?.removeEventListener("abort", abort);
        cancel(id);
      },
      } satisfies TerminalStream);
    }, (error) => {
      if (!closed && !settled) { settled = true; signal?.removeEventListener("abort", abort); reject(error); }
    });
  });
}

export function createNativeClient(invoke: NativeInvoke = defaultInvoke, channelFactory: NativeChannelFactory = defaultChannel): CockpitClient {
  return {
    ...bindOperations("native", (operation) => invokeAndParse(invoke, operation.tauri.command, operation.tauri.args(), operation.operation, operation.parse)),
    subscribeWidgets(onEvent, onError, signal) {
      return nativeWidgetSubscription(channelFactory, invoke, onEvent, onError, signal);
    },
    subscribeSession(sessionId, onMessage, onError, signal) { return sessionSubscription(channelFactory, invoke, sessionId, onMessage, onError, signal); },
    openTerminal(request, onMessage, onError, signal) { return terminalSubscription(channelFactory, invoke, request, onMessage, onError, signal); },
    openBrowserView(request, onEvent, onFrame, onError, signal) {
      return nativeBrowserViewSubscription(invoke, request, onEvent, onFrame, onError, signal);
    },
  };
}
