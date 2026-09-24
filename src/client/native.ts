import { parseContextMediaRequest, parseContextMedia, matchContextMedia } from "./contextMediaProtocol";
import { parseSourceScope, parseSourceImport, parseSourceRefresh, parseSourceResponse, matchSourceResponse } from "./sourceProtocol";
import { parseReviewLaunchRequest, parseReviewSnapshotRequest, parseReviewSnapshot, parseReviewFileRequest, parseReviewFile, matchReviewSnapshot, matchReviewFile } from "./reviewProtocol";
import { parseContextSnapshotRequest, parseContextSnapshotResponse, matchContextSnapshot } from "./contextSnapshotProtocol";
import { parseCommentPastePrepareRequest, parseCommentPastePrepare, parseCommentPasteSendRequest, parseCommentPasteReceipt, matchPastePrepare, matchPasteReceipt, parseCommentPasteMarkPastedRequest, matchMarkedReceipt } from "./commentPasteProtocol";
import {
  matchContextResponse, matchPanePresentation, parseContextDirectory,
  parseContextDirectoryRequest, parseContextDocument, parseContextDocumentRequest,
  parseContextLaunchRequest, parsePanePresentation,
} from "./contextProtocol";
import {
  matchContextInvalidationResponse, matchContextSearchResponse,
  parseContextInvalidationRequest, parseContextInvalidationResponse,
  parseContextSearchRequest, parseContextSearchResponse,
} from "./contextSearchProtocol";
import {
  matchCommentAttachment, matchCommentBatch, matchCommentPreview,
  parseCommentBatch, parseCommentBatchList, parseCommentBatchRequest, parseCommentMutation,
  parseCommentPreview, parseCommentPreviewRequest, parseCommentRemove, parseCommentScope,
  parseCommentUpsert,
} from "./commentProtocol";
import {
  matchProjectSession, parseProjectConfiguration, parseRepositoryList,
  parseWorkspaceDefaults, parseWorkspaceDefaultsRequest,
  parseWorkspaceOperation, parseWorkspaceOperationRequest, parseWorkspaceSetupPlan,
  parseWorkspaceReconcileRequest,
  parseWorkspaceSetupRequest, validateProjectOperationId,
} from "./projectProtocol";
import {
  matchWorkspaceTeardownPreview, matchWorkspaceTeardownResult,
  parseWorkspaceTeardownExecuteRequest, parseWorkspaceTeardownPreview,
  parseWorkspaceTeardownPreviewRequest, parseWorkspaceTeardownRecoveryList,
  parseWorkspaceTeardownResult,
} from "./projectTeardownProtocol";
import { Channel, invoke as tauriInvoke } from "@tauri-apps/api/core";
import type {
  FocusRequest,
  FocusResponse,
  ResourceMutationRequest,
  ResourceMutationResponse,
  SessionListResponse,
  SessionSnapshotResponse,
  SpaceGitStatusResponse,
  SessionStreamMessage,
  StatusResponse,
  TerminalCommand,
  TerminalOpenRequest,
  TerminalStreamMessage,
} from "../protocol/generated/v1";
import { transitionSessionStream, type StreamOrderCursor } from "./streamOrder";
import {
  CockpitClientError,
  matchBrowserViewCommandResponse,
  matchBrowserViewEvent,
  parseBrowserViewCommandRequest,
  parseBrowserDraftRecoveryRequest,
  parseBrowserViewCommandOutcome,
  parseBrowserViewCommandResponse,
  parseBrowserViewEvent,
  parseBrowserViewFrameDescriptor,
  parseBrowserViewOpenRequest,
  parseBrowserViewSnapshot,
  parseBrowserFeedbackAck,
  parseBrowserFeedbackAckRequest,
  parseBrowserFeedbackImage,
  parseBrowserFeedbackImageRequest,
  parseBrowserFeedbackLookup,
  parseBrowserFeedbackRequest,
  parseBrowserFeedbackSendRequest,
  parseBrowserFeedbackSendResponse,
  parseBrowserRequest,
  parseBrowserResponse,
  parseErrorEnvelope,
  parseFocusRequest,
  parseFocusResponse,
  parseResourceMutationRequest,
  parseResourceMutationResponse,
  parseSessionListResponse,
  parseSessionSnapshotResponse,
  parseSpaceGitStatusResponse,
  parseSessionStreamMessage,
  parseStatusResponse,
  parseTerminalCommand,
  parseTerminalOpenRequest,
  parseTerminalStreamMessage,
  validateSessionId,
  validateResourceId,
  type BrowserViewCommandRequest,
  type BrowserViewEvent,
  type BrowserViewFrameDescriptor,
  type BrowserViewFramePacket,
  type BrowserViewOpenRequest,
  type BrowserViewSnapshot,
  type BrowserViewStream,
  type BrowserDraftRecoveryRequest,
  type BrowserViewCommandOutcome,
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
interface NativeBrowserViewOpenResponse { snapshot: BrowserViewSnapshot; first_frame: BrowserViewFrameDescriptor; }
function parseNativeBrowserViewOpen(value: unknown): NativeBrowserViewOpenResponse {
  if (typeof value !== "object" || value === null) throw new CockpitClientError("malformed_response", "Native browser view open response is malformed");
  const body = value as Record<string, unknown>;
  if (!("snapshot" in body) || !("first_frame" in body)) throw new CockpitClientError("malformed_response", "Native browser view open response is incomplete");
  return { snapshot: parseBrowserViewSnapshot(body.snapshot), first_frame: parseBrowserViewFrameDescriptor(body.first_frame) };
}
function nativeBinaryFrame(value: unknown, expectedLength: number): ArrayBuffer {
  let bytes: Uint8Array;
  if (value instanceof ArrayBuffer) bytes = new Uint8Array(value);
  else if (value instanceof Uint8Array) bytes = value;
  else throw new CockpitClientError("malformed_response", "Native browser frame payload is not binary");
  if (bytes.byteLength === 0 || bytes.byteLength > 6 * 1024 * 1024 || bytes.byteLength !== expectedLength
    || bytes[0] !== 0xff || bytes[1] !== 0xd8) {
    throw new CockpitClientError("malformed_response", "Native browser frame payload is invalid");
  }
  if (value instanceof ArrayBuffer) return value;
  if (bytes.byteOffset === 0 && bytes.byteLength === bytes.buffer.byteLength && bytes.buffer instanceof ArrayBuffer) return bytes.buffer;
  return bytes.slice().buffer;
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
    let targetId = openedView.snapshot.displayed_target_id ?? undefined;
    let lastFrameSequence = 0;
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
      const jpeg = nativeBinaryFrame(raw, pending.descriptor.jpeg_length);
      lastFrameSequence = pending.sequence;
      let delivered = false;
      const release = (kind: "ack" | "discard") => {
        if (delivered || closed) return;
        delivered = true;
        releaseFrame(pending.sequence, kind);
      };
      try {
        onFrame({ descriptor: pending.descriptor, jpeg, ack: () => release("ack"), discard: () => release("discard") });
      } catch (cause) {
        release("discard");
        throw new CockpitClientError("transport_error", "Native browser frame handler failed", { cause });
      }
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
          const event = matchBrowserViewEvent(parseBrowserViewEvent(body.event), identity);
          if (event.type === "attached") targetId = event.snapshot.displayed_target_id ?? undefined;
          else if (event.type === "targets_changed") targetId = event.displayed_target_id ?? undefined;
          else if (event.type === "document_changed") targetId = event.document?.target_id;
          onEvent(event);
          return;
        }
        if (body.kind === "frame") {
          const descriptor = parseBrowserViewFrameDescriptor(body.descriptor);
          if (targetId !== undefined && descriptor.target_id !== targetId) throw new CockpitClientError("malformed_response", "Native browser frame target identity does not match");
          if (descriptor.stream_epoch !== identity.stream_epoch) throw new CockpitClientError("malformed_response", "Native browser frame stream identity does not match");
          if (descriptor.frame_sequence <= lastFrameSequence) throw new CockpitClientError("malformed_response", "Native browser frame sequence is out of order");
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
    projectConfiguration() { return invokeAndParse(invoke, "cockpit_project_configuration", undefined, "project configuration", parseProjectConfiguration); },
    repositories() { return invokeAndParse(invoke, "cockpit_repositories", undefined, "repositories", parseRepositoryList); },
    resolveWorkspaceDefaults(value) {
      const request = parseWorkspaceDefaultsRequest(value);
      return invokeAndParse(invoke, "cockpit_resolve_workspace_defaults", { request }, "workspace defaults", parseWorkspaceDefaults);
    },
    async planWorkspace(sessionId, value) {
      validateSessionId(sessionId);
      const request = parseWorkspaceSetupRequest(value);
      return matchProjectSession(await invokeAndParse(invoke, "cockpit_workspace_plan", { sessionId, request }, "workspace plan", parseWorkspaceSetupPlan), sessionId);
    },
    async startWorkspace(sessionId, value) {
      validateSessionId(sessionId);
      const request = parseWorkspaceOperationRequest(value);
      return matchProjectSession(await invokeAndParse(invoke, "cockpit_workspace_start", { sessionId, request }, "workspace start", parseWorkspaceOperation), sessionId, request.operation_id);
    },
    async workspaceOperation(sessionId, operationId) {
      validateSessionId(sessionId);
      validateProjectOperationId(operationId);
      return matchProjectSession(await invokeAndParse(invoke, "cockpit_workspace_operation", { sessionId, operationId }, "workspace operation", parseWorkspaceOperation), sessionId, operationId);
    },
    async resumeWorkspace(sessionId, value) {
      validateSessionId(sessionId);
      const request = parseWorkspaceOperationRequest(value);
      return matchProjectSession(await invokeAndParse(invoke, "cockpit_workspace_resume", { sessionId, request }, "workspace resume", parseWorkspaceOperation), sessionId, request.operation_id);
    },
    async cancelWorkspace(sessionId, value) {
      validateSessionId(sessionId);
      const request = parseWorkspaceOperationRequest(value);
      return matchProjectSession(await invokeAndParse(invoke, "cockpit_workspace_cancel", { sessionId, request }, "workspace cancellation", parseWorkspaceOperation), sessionId, request.operation_id);
    },
    async reconcileWorkspace(sessionId, value) {
      validateSessionId(sessionId);
      const request = parseWorkspaceReconcileRequest(value);
      return matchProjectSession(await invokeAndParse(invoke, "cockpit_workspace_reconcile", { sessionId, request }, "workspace reconciliation", parseWorkspaceOperation), sessionId, request.operation_id);
    },
    async workspaceTeardownPreview(sessionId, value) {
      validateSessionId(sessionId);
      const request = parseWorkspaceTeardownPreviewRequest(value);
      return matchWorkspaceTeardownPreview(await invokeAndParse(invoke, "cockpit_workspace_teardown_preview", { sessionId, request }, "workspace teardown preview", parseWorkspaceTeardownPreview), request);
    },
    async workspaceTeardownExecute(sessionId, value) {
      validateSessionId(sessionId);
      const request = parseWorkspaceTeardownExecuteRequest(value);
      return matchWorkspaceTeardownResult(await invokeAndParse(invoke, "cockpit_workspace_teardown_execute", { sessionId, request }, "workspace teardown execution", parseWorkspaceTeardownResult), request);
    },
    async workspaceTeardownRecoveries(sessionId) {
      validateSessionId(sessionId);
      return invokeAndParse(invoke, "cockpit_workspace_teardown_recoveries", { sessionId }, "workspace teardown recoveries", parseWorkspaceTeardownRecoveryList);
    },
    async inspectPane(sessionId, paneId, signal) {
      signal?.throwIfAborted();
      validateSessionId(sessionId);
      validateResourceId(paneId);
      const value = await invokeAndParse(invoke, "cockpit_pane_presentation", { sessionId, paneId }, "pane presentation", parsePanePresentation);
      signal?.throwIfAborted();
      return matchPanePresentation(value, sessionId, paneId);
    },
    async contextDirectory(sessionId, paneId, value, signal) {
      signal?.throwIfAborted();
      validateSessionId(sessionId);
      validateResourceId(paneId);
      const request = parseContextDirectoryRequest(value);
      const response = await invokeAndParse(invoke, "cockpit_context_directory", { sessionId, paneId, request }, "Context directory", parseContextDirectory);
      signal?.throwIfAborted();
      return matchContextResponse(response, request);
    },
    async contextDocument(sessionId, paneId, value, signal) {
      signal?.throwIfAborted();
      validateSessionId(sessionId);
      validateResourceId(paneId);
      const request = parseContextDocumentRequest(value);
      const response = await invokeAndParse(invoke, "cockpit_context_document", { sessionId, paneId, request }, "Context document", parseContextDocument);
      signal?.throwIfAborted();
      return matchContextResponse(response, request);
    },
    async reviewSnapshot(sessionId, paneId, value, signal) {
      validateSessionId(sessionId); validateResourceId(paneId); signal?.throwIfAborted();
      const parsed = parseReviewSnapshotRequest(value);
      const response = await invokeAndParse(invoke, "cockpit_review_snapshot", { sessionId, paneId, request: parsed }, "Review snapshot", parseReviewSnapshot);
      signal?.throwIfAborted(); return matchReviewSnapshot(response, sessionId, paneId, parsed);
    },
    async reviewFile(sessionId, paneId, value, signal) {
      validateSessionId(sessionId); validateResourceId(paneId); signal?.throwIfAborted();
      const parsed = parseReviewFileRequest(value);
      const response = await invokeAndParse(invoke, "cockpit_review_file", { sessionId, paneId, request: parsed }, "Review file", parseReviewFile);
      signal?.throwIfAborted(); return matchReviewFile(response, sessionId, paneId, parsed);
    },
    async contextSnapshot(sessionId, paneId, value) {
      validateSessionId(sessionId);
      validateResourceId(paneId);
      const request = parseContextSnapshotRequest(value);
      const response = await invokeAndParse(invoke, "cockpit_context_snapshot", { sessionId, paneId, request }, "Context snapshot", parseContextSnapshotResponse);
      return matchContextSnapshot(response, request);
    },
    async contextSearch(sessionId, paneId, value, signal) {
      signal?.throwIfAborted();
      validateSessionId(sessionId);
      validateResourceId(paneId);
      const request = parseContextSearchRequest(value);
      const response = await invokeAndParse(invoke, "cockpit_context_search", { sessionId, paneId, request }, "Context search", parseContextSearchResponse);
      signal?.throwIfAborted();
      return matchContextSearchResponse(response, request);
    },
    async contextInvalidate(sessionId, paneId, value, signal) {
      signal?.throwIfAborted();
      validateSessionId(sessionId);
      validateResourceId(paneId);
      const request = parseContextInvalidationRequest(value);
      const response = await invokeAndParse(invoke, "cockpit_context_invalidate", { sessionId, paneId, request }, "Context invalidation", parseContextInvalidationResponse);
      signal?.throwIfAborted();
      return matchContextInvalidationResponse(response, request);
    },
    async contextMedia(sessionId, paneId, value, signal) {
      signal?.throwIfAborted(); validateSessionId(sessionId); validateResourceId(paneId);
      const body = parseContextMediaRequest(value);
      const response = await invokeAndParse(invoke, "cockpit_context_media", { sessionId, paneId, request: body }, "Context image", parseContextMedia);
      signal?.throwIfAborted();
      return matchContextMedia(response, body);
    },
    async sourceImport(sessionId, paneId, value, signal) {
      signal?.throwIfAborted(); validateSessionId(sessionId); validateResourceId(paneId);
      const body = parseSourceImport(value);
      const response = await invokeAndParse(invoke, "cockpit_source_import", { sessionId, paneId, request: body }, "source import", parseSourceResponse);
      signal?.throwIfAborted();
      return matchSourceResponse(response, body);
    },
    async sourceRefresh(sessionId, paneId, value, signal) {
      signal?.throwIfAborted(); validateSessionId(sessionId); validateResourceId(paneId);
      const body = parseSourceRefresh(value);
      const response = await invokeAndParse(invoke, "cockpit_source_refresh", { sessionId, paneId, request: body }, "source refresh", parseSourceResponse);
      signal?.throwIfAborted();
      return matchSourceResponse(response, body);
    },
    async sourceList(sessionId, paneId, value, signal) {
      signal?.throwIfAborted(); validateSessionId(sessionId); validateResourceId(paneId);
      const body = parseSourceScope(value);
      const response = await invokeAndParse(invoke, "cockpit_source_list", { sessionId, paneId, request: body }, "source list", parseSourceResponse);
      signal?.throwIfAborted();
      return matchSourceResponse(response, body);
    },
    async openReview(sessionId, value) {
      validateSessionId(sessionId);
      const request = parseReviewLaunchRequest(value);
      return matchPanePresentation(await invokeAndParse(invoke, "cockpit_review_open", { sessionId, request }, "Review launch", parsePanePresentation), sessionId);
    },
    async openContext(sessionId, value) {
      validateSessionId(sessionId);
      const request = parseContextLaunchRequest(value);
      return matchPanePresentation(await invokeAndParse(invoke, "cockpit_context_open", { sessionId, request }, "Context launch", parsePanePresentation), sessionId);
    },
    async commentBatches(sessionId, paneId, value, signal) {
      signal?.throwIfAborted();
      validateSessionId(sessionId);
      validateResourceId(paneId);
      const request = parseCommentScope(value);
      const response = await invokeAndParse(invoke, "cockpit_comments_list", { sessionId, paneId, request }, "comment batches", parseCommentBatchList);
      signal?.throwIfAborted();
      matchCommentAttachment(response.attachment, sessionId, paneId, request);
      return response;
    },
    async commentBatch(sessionId, paneId, value, signal) {
      signal?.throwIfAborted();
      validateSessionId(sessionId);
      validateResourceId(paneId);
      const request = parseCommentBatchRequest(value);
      const response = await invokeAndParse(invoke, "cockpit_comments_batch", { sessionId, paneId, request }, "comment batch", parseCommentBatch);
      signal?.throwIfAborted();
      return matchCommentBatch(response, sessionId, paneId, request.scope, request.batch_id);
    },
    async commentUpsert(sessionId, paneId, value, signal) {
      signal?.throwIfAborted();
      validateSessionId(sessionId);
      validateResourceId(paneId);
      const request = parseCommentUpsert(value);
      const response = await invokeAndParse(invoke, "cockpit_comments_upsert", { sessionId, paneId, request }, "comment upsert", parseCommentBatch);
      signal?.throwIfAborted();
      return matchCommentBatch(response, sessionId, paneId, request.batch.scope, request.batch.batch_id, request.batch.expected_generation, true);
    },
    async commentRemove(sessionId, paneId, value, signal) {
      signal?.throwIfAborted();
      validateSessionId(sessionId);
      validateResourceId(paneId);
      const request = parseCommentRemove(value);
      const response = await invokeAndParse(invoke, "cockpit_comments_remove", { sessionId, paneId, request }, "comment remove", parseCommentBatch);
      signal?.throwIfAborted();
      return matchCommentBatch(response, sessionId, paneId, request.batch.scope, request.batch.batch_id, request.batch.expected_generation, true);
    },
    async commentDiscard(sessionId, paneId, value, signal) {
      signal?.throwIfAborted();
      validateSessionId(sessionId);
      validateResourceId(paneId);
      const request = parseCommentMutation(value);
      const response = await invokeAndParse(invoke, "cockpit_comments_discard", { sessionId, paneId, request }, "comment discard", parseCommentBatchList);
      signal?.throwIfAborted();
      matchCommentAttachment(response.attachment, sessionId, paneId, request.scope);
      return response;
    },    async commentAttach(sessionId, paneId, value, signal) {
      signal?.throwIfAborted();
      validateSessionId(sessionId);
      validateResourceId(paneId);
      const request = parseCommentMutation(value);
      const response = await invokeAndParse(invoke, "cockpit_comments_attach", { sessionId, paneId, request }, "comment attach", parseCommentBatch);
      signal?.throwIfAborted();
      return matchCommentBatch(response, sessionId, paneId, request.scope, request.batch_id, request.expected_generation, true);
    },
    async commentPastePrepare(sessionId, paneId, value, signal) {
      validateSessionId(sessionId); validateResourceId(paneId);
      const parsed = parseCommentPastePrepareRequest(value);
      signal?.throwIfAborted();
      const response = await invokeAndParse(invoke, "cockpit_comments_paste_prepare", { sessionId, paneId, request: parsed }, "comment paste prepare", parseCommentPastePrepare);
      signal?.throwIfAborted();
      return matchPastePrepare(response, sessionId, parsed);
    },
    async commentPasteSend(sessionId, paneId, value) {
      validateSessionId(sessionId); validateResourceId(paneId);
      const parsed = parseCommentPasteSendRequest(value);
      const response = await invokeAndParse(invoke, "cockpit_comments_paste_send", { sessionId, paneId, request: parsed }, "comment paste send", parseCommentPasteReceipt);
      return matchPasteReceipt(response, parsed);
    },
    async commentPasteMarkPasted(sessionId, paneId, value) {
      validateSessionId(sessionId); validateResourceId(paneId);
      const parsed = parseCommentPasteMarkPastedRequest(value);
      const response = await invokeAndParse(invoke, "cockpit_comments_paste_mark_pasted", { sessionId, paneId, request: parsed }, "comment paste resolution", parseCommentPasteReceipt);
      return matchMarkedReceipt(response, parsed);
    },
    async commentPreview(sessionId, paneId, value, signal) {
      signal?.throwIfAborted();
      validateSessionId(sessionId);
      validateResourceId(paneId);
      const request = parseCommentPreviewRequest(value);
      const response = await invokeAndParse(invoke, "cockpit_comments_preview", { sessionId, paneId, request }, "comment preview", parseCommentPreview);
      signal?.throwIfAborted();
      return matchCommentPreview(response, request.batch);
    },
    browserAction(value) {
      const request = parseBrowserRequest(value);
      return invokeAndParse(invoke, "cockpit_browser_action", { request }, "browser action", parseBrowserResponse);
    },
    async browserFeedback(value) {
      const request = parseBrowserFeedbackRequest(value);
      return invokeAndParse(invoke, "cockpit_browser_feedback", { request }, "browser feedback", parseBrowserFeedbackLookup);
    },
    async browserDraftRecovery(value: BrowserDraftRecoveryRequest): Promise<BrowserViewCommandOutcome> {
      const request = parseBrowserDraftRecoveryRequest(value);
      return invokeAndParse(invoke, "cockpit_browser_draft_recovery", { request }, "browser draft recovery", parseBrowserViewCommandOutcome);
    },
    async acknowledgeBrowserFeedback(value) {
      const request = parseBrowserFeedbackAckRequest(value);
      return invokeAndParse(invoke, "cockpit_browser_feedback_ack", { request }, "browser feedback acknowledgement", parseBrowserFeedbackAck);
    },
    async browserFeedbackImage(value) {
      const request = parseBrowserFeedbackImageRequest(value);
      return invokeAndParse(invoke, "cockpit_browser_feedback_image", { request }, "browser feedback image", parseBrowserFeedbackImage);
    },
    async sendBrowserFeedback(value) {
      const request = parseBrowserFeedbackSendRequest(value);
      return invokeAndParse(invoke, "cockpit_browser_feedback_send", { request }, "browser feedback send", parseBrowserFeedbackSendResponse);
    },
    status(): Promise<StatusResponse> { return invokeAndParse(invoke, "cockpit_status", undefined, "status", parseStatusResponse); },
    sessions(): Promise<SessionListResponse> { return invokeAndParse(invoke, "cockpit_sessions", undefined, "sessions", parseSessionListResponse); },
    spaceGitStatus(sessionId: string, signal?: AbortSignal): Promise<SpaceGitStatusResponse> {
      try { validateSessionId(sessionId); signal?.throwIfAborted(); } catch (error) { return Promise.reject(error); }
      return invokeAndParse(invoke, "cockpit_space_git_status", { sessionId }, "Space Git status", parseSpaceGitStatusResponse).then((value) => {
        signal?.throwIfAborted();
        if (value.session_id !== sessionId) throw new CockpitClientError("malformed_response", "Space Git status belongs to another session");
        return value;
      });
    },
    sessionSnapshot(sessionId: string, signal?: AbortSignal): Promise<SessionSnapshotResponse> {
      try { validateSessionId(sessionId); signal?.throwIfAborted(); } catch (error) { return Promise.reject(error); }
      const pending = invokeAndParse(invoke, "cockpit_session_snapshot", { sessionId }, "session snapshot", parseSessionSnapshotResponse).then((value) => {
        signal?.throwIfAborted();
        if (value.session_id !== sessionId) throw new CockpitClientError("malformed_response", "Session snapshot belongs to another session");
        return value;
      });
      if (!signal) return pending;
      return new Promise<SessionSnapshotResponse>((resolve, reject) => {
        const abort = () => reject(signal.reason);
        signal.addEventListener("abort", abort, { once: true });
        void pending.then((value) => {
          signal.removeEventListener("abort", abort);
          resolve(value);
        }, (error: unknown) => {
          signal.removeEventListener("abort", abort);
          reject(error);
        });
        if (signal.aborted) abort();
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
    subscribeSession(sessionId, onMessage, onError, signal) { return sessionSubscription(channelFactory, invoke, sessionId, onMessage, onError, signal); },
    openTerminal(request, onMessage, onError, signal) { return terminalSubscription(channelFactory, invoke, request, onMessage, onError, signal); },
    openBrowserView(request, onEvent, onFrame, onError, signal) {
      return nativeBrowserViewSubscription(invoke, request, onEvent, onFrame, onError, signal);
    },
  };
}
