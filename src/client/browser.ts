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
  parseSessionStreamMessage,
  parseStatusResponse,
  parseTerminalOpenRequest,
  parseTerminalStreamMessage,
  parseTerminalCommand,
  validateSessionId,
  validateResourceId,
  type BrowserViewFramePacket,
  type BrowserViewOpenRequest,
  type BrowserViewStream,
  type BrowserDraftRecoveryRequest,
  type BrowserViewCommandOutcome,
  type BrowserViewSnapshot,
  type ClosableStream,
  type CockpitClient,
  type TerminalStream,
} from "./CockpitClient";
import type {
  BrowserViewCommandRequest,
  BrowserViewEvent,
  BrowserViewFrameDescriptor,
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
import { transitionSessionStream, type StreamOrderCursor } from "./streamOrder";

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

interface BrowserFrameEnvelope {
  readonly data: ArrayBuffer;
  readonly frameSequence: number;
}

function parseBrowserFrameEnvelope(data: unknown, streamEpoch: number): BrowserFrameEnvelope {
  if (!(data instanceof ArrayBuffer)) throw new CockpitClientError("malformed_response", "Browser frame is not binary");
  if (data.byteLength < 96 || data.byteLength > 96 + 6 * 1024 * 1024) throw new CockpitClientError("malformed_response", "Browser frame exceeds bounds");
  const view = new DataView(data);
  const jpegLength = view.getUint32(80, false);
  if (view.getUint32(0, false) !== 0x49424656 || view.getUint16(4, false) !== 2 || view.getUint16(6, false) !== 96 || view.getBigUint64(8, false) !== BigInt(streamEpoch) || jpegLength === 0 || data.byteLength !== 96 + jpegLength) {
    throw new CockpitClientError("malformed_response", "Browser frame envelope is invalid");
  }
  const jpeg = new Uint8Array(data, 96, 2);
  if (jpeg[0] !== 0xff || jpeg[1] !== 0xd8) throw new CockpitClientError("malformed_response", "Browser frame payload is not JPEG");
  return { data, frameSequence: Number(view.getBigUint64(16, false)) };
}

function parseBrowserFrame(frame: BrowserFrameEnvelope, targetId: string): { descriptor: BrowserViewFrameDescriptor; jpeg: ArrayBuffer } {
  if (targetId.length === 0) throw new CockpitClientError("malformed_response", "Browser frame target identity is unavailable");
  const { data } = frame;
  const view = new DataView(data);
  const jpegLength = view.getUint32(80, false);
  const descriptor = parseBrowserViewFrameDescriptor({
    target_id: targetId,
    stream_epoch: Number(view.getBigUint64(8, false)),
    frame_sequence: frame.frameSequence,
    document_generation: Number(view.getBigUint64(24, false)),
    viewport_revision: Number(view.getBigUint64(32, false)),
    image_width: view.getUint32(40, false),
    image_height: view.getUint32(44, false),
    viewport_css_width: view.getFloat32(48, false),
    viewport_css_height: view.getFloat32(52, false),
    viewport_offset_x: view.getFloat32(56, false),
    viewport_offset_y: view.getFloat32(60, false),
    scroll_x: view.getFloat32(64, false),
    scroll_y: view.getFloat32(68, false),
    capture_timestamp_micros: Number(view.getBigUint64(72, false)),
    jpeg_length: jpegLength,
  });
  return { descriptor, jpeg: data.slice(96) };
}

type BrowserFrameRelease = (kind: "ack" | "discard") => void;
interface PendingBrowserFrame {
  readonly envelope: BrowserFrameEnvelope;
  readonly release: BrowserFrameRelease;
}

function browserFramePacket(frame: PendingBrowserFrame, targetId: string): BrowserViewFramePacket {
  const parsedFrame = parseBrowserFrame(frame.envelope, targetId);
  return {
    descriptor: parsedFrame.descriptor,
    jpeg: parsedFrame.jpeg,
    ack: () => frame.release("ack"),
    discard: () => frame.release("discard"),
  };
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
  return getJson(request, "/api/v1/browser/view/open", "browser view open", parseBrowserViewOpenResponse, { method: "POST", headers: { "Content-Type": "application/json" }, body: JSON.stringify(parsed), signal }).then((opened) => new Promise((resolve, reject) => {
    const identity = opened.snapshot.identity;
    const grant = opened.snapshot.frame_grant;
    if (grant === null) { reject(new CockpitClientError("malformed_response", "Browser view frame grant is missing")); return; }
    let eventSocket: BrowserWebSocket; let frameSocket: BrowserWebSocket;
    let closed = false; let closing = false; let settled = false; let openCount = 0;
    let targetId = opened.snapshot.displayed_target_id ?? "";
    let metadataAttached = false;
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
    const deliverFrame = (frame: PendingBrowserFrame) => {
      let packet: BrowserViewFramePacket;
      try { packet = browserFramePacket(frame, targetId); }
      catch (error) { frame.release("discard"); throw error; }
      try { onFrame(packet); }
      catch (cause) {
        frame.release("discard");
        throw new CockpitClientError("transport_error", "Browser view frame handler failed", { cause });
      }
    };
    const flushPending = () => {
      const pending = pendingFrame;
      pendingFrame = null;
      if (!pending) return;
      if (closed || !metadataAttached) { pending.release("discard"); return; }
      try { deliverFrame(pending); }
      catch (error) { fail(error instanceof CockpitClientError ? error : new CockpitClientError("transport_error", "Browser view frame handler failed", { cause: error })); }
    };
    const packet = (data: unknown) => {
      if (closed) return;
      try {
        const envelope = parseBrowserFrameEnvelope(data, identity.stream_epoch);
        const frame: PendingBrowserFrame = { envelope, release: makeRelease(envelope.frameSequence) };
        if (!metadataAttached) {
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
    const abort = () => fail(new CockpitClientError("stream_error", "Browser view attach was cancelled"));
    if (signal?.aborted) { abort(); return; } signal?.addEventListener("abort", abort, { once: true });
    eventSocket.onopen = maybeReady;
    frameSocket.onopen = () => { frameSocket.send(JSON.stringify({ grant })); maybeReady(); };
    eventSocket.onmessage = (event) => {
      if (closed || typeof event.data !== "string") {
        if (!closed) fail(new CockpitClientError("malformed_response", "Browser view metadata is not JSON text"));
        return;
      }
      try {
        const parsedEvent = matchBrowserViewEvent(parseBrowserViewEvent(JSON.parse(event.data)), identity);
        if (parsedEvent.type === "attached") {
          if (metadataAttached) return;
          targetId = parsedEvent.snapshot.displayed_target_id ?? "";
          metadataAttached = true;
          onEvent(parsedEvent);
          flushPending();
        } else if (metadataAttached) {
          onEvent(parsedEvent);
        }
      } catch (error) {
        fail(error instanceof CockpitClientError ? error : new CockpitClientError("malformed_response", "Browser view metadata is malformed", { cause: error }));
      }
    };
    frameSocket.onmessage = (event) => packet(event.data);
    eventSocket.onerror = (cause) => fail(new CockpitClientError("transport_error", "Browser view metadata WebSocket failed", { cause }));
    frameSocket.onerror = (cause) => fail(new CockpitClientError("transport_error", "Browser view frame WebSocket failed", { cause }));
    eventSocket.onclose = (event) => { if (!closed) fail(streamError(`Browser view metadata WebSocket closed${event.reason ? `: ${event.reason}` : ""}`, event)); };
    frameSocket.onclose = (event) => { if (!closed) fail(streamError(`Browser view frame WebSocket closed${event.reason ? `: ${event.reason}` : ""}`, event)); };
  }));
}

function streamError(message: string, cause?: unknown, operationCode?: string): CockpitClientError {
  return new CockpitClientError("stream_error", message, { cause, operationCode });
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
    let cursor: StreamOrderCursor | null = null;
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
  signal?: AbortSignal,
): Promise<TerminalStream> {
  let validated: TerminalOpenRequest;
  try { validated = parseTerminalOpenRequest(request); } catch (error) { return Promise.reject(error); }
  const path = `/api/v1/sessions/${encodeURIComponent(validated.session_id)}/panes/${encodeURIComponent(validated.pane_id)}/terminal?mode=${validated.mode}&takeover=${validated.takeover ? "true" : "false"}&cols=${validated.cols}&rows=${validated.rows}&cell_width_px=${validated.cell_width_px}&cell_height_px=${validated.cell_height_px}`;
  return new Promise((resolve, reject) => {
    let socket: BrowserWebSocket | undefined;
    let settled = false;
    let closed = false;
    let streamId: string | undefined;
    let lastFrameSequence: bigint | undefined;
    let receivedFrame = false;
    const fail = (error: CockpitClientError, beforeOpen = false) => {
      if (beforeOpen && !settled) { settled = true; reject(error); }
      else onError(error);
      if (!closed) { closed = true; socket?.close(); }
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
    projectConfiguration() { return getJson(request, "/api/v1/project/configuration", "project configuration", parseProjectConfiguration); },
    repositories() { return getJson(request, "/api/v1/project/repositories", "repositories", parseRepositoryList); },
    async resolveWorkspaceDefaults(value) {
      const body = parseWorkspaceDefaultsRequest(value);
      return getJson(request, "/api/v1/project/defaults", "workspace defaults", parseWorkspaceDefaults, { method: "POST", headers: { "content-type": "application/json" }, body: JSON.stringify(body) });
    },
    async planWorkspace(sessionId, value) {
      validateSessionId(sessionId);
      const body = parseWorkspaceSetupRequest(value);
      return matchProjectSession(await getJson(request, `/api/v1/sessions/${encodeURIComponent(sessionId)}/workspace-plans`, "workspace plan", parseWorkspaceSetupPlan, {
        method: "POST", headers: { "Content-Type": "application/json" }, body: JSON.stringify(body),
      }), sessionId);
    },
    async startWorkspace(sessionId, value) {
      validateSessionId(sessionId);
      const body = parseWorkspaceOperationRequest(value);
      return matchProjectSession(await getJson(request, `/api/v1/sessions/${encodeURIComponent(sessionId)}/workspace-operations`, "workspace start", parseWorkspaceOperation, {
        method: "POST", headers: { "Content-Type": "application/json" }, body: JSON.stringify(body),
      }), sessionId, body.operation_id);
    },
    async workspaceOperation(sessionId, operationId) {
      validateSessionId(sessionId);
      validateProjectOperationId(operationId);
      return matchProjectSession(await getJson(request, `/api/v1/sessions/${encodeURIComponent(sessionId)}/workspace-operations/${encodeURIComponent(operationId)}`, "workspace operation", parseWorkspaceOperation), sessionId, operationId);
    },
    async resumeWorkspace(sessionId, value) {
      validateSessionId(sessionId);
      const body = parseWorkspaceOperationRequest(value);
      return matchProjectSession(await getJson(request, `/api/v1/sessions/${encodeURIComponent(sessionId)}/workspace-operations/resume`, "workspace resume", parseWorkspaceOperation, {
        method: "POST", headers: { "Content-Type": "application/json" }, body: JSON.stringify(body),
      }), sessionId, body.operation_id);
    },
    async cancelWorkspace(sessionId, value) {
      validateSessionId(sessionId);
      const body = parseWorkspaceOperationRequest(value);
      return matchProjectSession(await getJson(request, `/api/v1/sessions/${encodeURIComponent(sessionId)}/workspace-operations/cancel`, "workspace cancellation", parseWorkspaceOperation, {
        method: "POST", headers: { "Content-Type": "application/json" }, body: JSON.stringify(body),
      }), sessionId, body.operation_id);
    },
    async reconcileWorkspace(sessionId, value) {
      validateSessionId(sessionId);
      const body = parseWorkspaceReconcileRequest(value);
      return matchProjectSession(await getJson(request, `/api/v1/sessions/${encodeURIComponent(sessionId)}/workspace-operations/reconcile`, "workspace reconciliation", parseWorkspaceOperation, {
        method: "POST", headers: { "Content-Type": "application/json" }, body: JSON.stringify(body),
      }), sessionId, body.operation_id);
    },
    async workspaceTeardownPreview(sessionId, value) {
      validateSessionId(sessionId);
      const body = parseWorkspaceTeardownPreviewRequest(value);
      return matchWorkspaceTeardownPreview(await getJson(request, `/api/v1/sessions/${encodeURIComponent(sessionId)}/workspace-teardown/preview`, "workspace teardown preview", parseWorkspaceTeardownPreview, {
        method: "POST", headers: { "Content-Type": "application/json" }, body: JSON.stringify(body),
      }), body);
    },
    async workspaceTeardownExecute(sessionId, value) {
      validateSessionId(sessionId);
      const body = parseWorkspaceTeardownExecuteRequest(value);
      return matchWorkspaceTeardownResult(await getJson(request, `/api/v1/sessions/${encodeURIComponent(sessionId)}/workspace-teardown/execute`, "workspace teardown execution", parseWorkspaceTeardownResult, {
        method: "POST", headers: { "Content-Type": "application/json" }, body: JSON.stringify(body),
      }), body);
    },
    async workspaceTeardownRecoveries(sessionId) {
      validateSessionId(sessionId);
      return getJson(request, `/api/v1/sessions/${encodeURIComponent(sessionId)}/workspace-teardown/recoveries`, "workspace teardown recoveries", parseWorkspaceTeardownRecoveryList);
    },
    async inspectPane(sessionId, paneId, signal) {
      validateSessionId(sessionId);
      validateResourceId(paneId);
      return matchPanePresentation(await getJson(request, `/api/v1/sessions/${encodeURIComponent(sessionId)}/panes/${encodeURIComponent(paneId)}/presentation`, "pane presentation", parsePanePresentation, { signal }), sessionId, paneId);
    },
    async contextDirectory(sessionId, paneId, value, signal) {
      validateSessionId(sessionId);
      validateResourceId(paneId);
      const body = parseContextDirectoryRequest(value);
      return matchContextResponse(await getJson(request, `/api/v1/sessions/${encodeURIComponent(sessionId)}/panes/${encodeURIComponent(paneId)}/context/directory`, "Context directory", parseContextDirectory, {
        method: "POST", headers: { "Content-Type": "application/json" }, body: JSON.stringify(body), signal,
      }), body);
    },
    async contextDocument(sessionId, paneId, value, signal) {
      validateSessionId(sessionId);
      validateResourceId(paneId);
      const body = parseContextDocumentRequest(value);
      return matchContextResponse(await getJson(request, `/api/v1/sessions/${encodeURIComponent(sessionId)}/panes/${encodeURIComponent(paneId)}/context/document`, "Context document", parseContextDocument, {
        method: "POST", headers: { "Content-Type": "application/json" }, body: JSON.stringify(body), signal,
      }), body);
    },
    async reviewSnapshot(sessionId, paneId, value, signal) {
      validateSessionId(sessionId); validateResourceId(paneId); signal?.throwIfAborted();
      const parsed = parseReviewSnapshotRequest(value);
      const response = await getJson(request, `/api/v1/sessions/${encodeURIComponent(sessionId)}/panes/${encodeURIComponent(paneId)}/review/snapshot`, "Review snapshot", parseReviewSnapshot, { method: "POST", headers: { "Content-Type": "application/json" }, body: JSON.stringify(parsed), signal });
      signal?.throwIfAborted(); return matchReviewSnapshot(response, sessionId, paneId, parsed);
    },
    async reviewFile(sessionId, paneId, value, signal) {
      validateSessionId(sessionId); validateResourceId(paneId); signal?.throwIfAborted();
      const parsed = parseReviewFileRequest(value);
      const response = await getJson(request, `/api/v1/sessions/${encodeURIComponent(sessionId)}/panes/${encodeURIComponent(paneId)}/review/file`, "Review file", parseReviewFile, { method: "POST", headers: { "Content-Type": "application/json" }, body: JSON.stringify(parsed), signal });
      signal?.throwIfAborted(); return matchReviewFile(response, sessionId, paneId, parsed);
    },
    async contextSnapshot(sessionId, paneId, value) {
      validateSessionId(sessionId);
      validateResourceId(paneId);
      const body = parseContextSnapshotRequest(value);
      const response = await getJson(request, `/api/v1/sessions/${encodeURIComponent(sessionId)}/panes/${encodeURIComponent(paneId)}/context/snapshot`, "Context snapshot", parseContextSnapshotResponse, {
        method: "POST", headers: { "Content-Type": "application/json" }, body: JSON.stringify(body),
      });
      return matchContextSnapshot(response, body);
    },
    async contextSearch(sessionId, paneId, value, signal) {
      validateSessionId(sessionId);
      validateResourceId(paneId);
      const body = parseContextSearchRequest(value);
      const response = await getJson(request, `/api/v1/sessions/${encodeURIComponent(sessionId)}/panes/${encodeURIComponent(paneId)}/context/search`, "Context search", parseContextSearchResponse, {
        method: "POST", headers: { "Content-Type": "application/json" }, body: JSON.stringify(body), signal,
      });
      return matchContextSearchResponse(response, body);
    },
    async contextInvalidate(sessionId, paneId, value, signal) {
      validateSessionId(sessionId);
      validateResourceId(paneId);
      const body = parseContextInvalidationRequest(value);
      const response = await getJson(request, `/api/v1/sessions/${encodeURIComponent(sessionId)}/panes/${encodeURIComponent(paneId)}/context/invalidate`, "Context invalidation", parseContextInvalidationResponse, {
        method: "POST", headers: { "Content-Type": "application/json" }, body: JSON.stringify(body), signal,
      });
      return matchContextInvalidationResponse(response, body);
    },
    async contextMedia(sessionId, paneId, value, signal) {
      signal?.throwIfAborted(); validateSessionId(sessionId); validateResourceId(paneId);
      const body = parseContextMediaRequest(value);
      const response = await getJson(request, `/api/v1/sessions/${encodeURIComponent(sessionId)}/panes/${encodeURIComponent(paneId)}/context/media`, "Context image", parseContextMedia, { method: "POST", headers: { "Content-Type": "application/json" }, body: JSON.stringify(body), signal });
      return matchContextMedia(response, body);
    },
    async sourceImport(sessionId, paneId, value, signal) {
      signal?.throwIfAborted(); validateSessionId(sessionId); validateResourceId(paneId);
      const body = parseSourceImport(value);
      const response = await getJson(request, `/api/v1/sessions/${encodeURIComponent(sessionId)}/panes/${encodeURIComponent(paneId)}/sources/import`, "source import", parseSourceResponse, { method: "POST", headers: { "Content-Type": "application/json" }, body: JSON.stringify(body), signal });
      return matchSourceResponse(response, body);
    },
    async sourceRefresh(sessionId, paneId, value, signal) {
      signal?.throwIfAborted(); validateSessionId(sessionId); validateResourceId(paneId);
      const body = parseSourceRefresh(value);
      const response = await getJson(request, `/api/v1/sessions/${encodeURIComponent(sessionId)}/panes/${encodeURIComponent(paneId)}/sources/refresh`, "source refresh", parseSourceResponse, { method: "POST", headers: { "Content-Type": "application/json" }, body: JSON.stringify(body), signal });
      return matchSourceResponse(response, body);
    },
    async sourceList(sessionId, paneId, value, signal) {
      signal?.throwIfAborted(); validateSessionId(sessionId); validateResourceId(paneId);
      const body = parseSourceScope(value);
      const response = await getJson(request, `/api/v1/sessions/${encodeURIComponent(sessionId)}/panes/${encodeURIComponent(paneId)}/sources/list`, "source list", parseSourceResponse, { method: "POST", headers: { "Content-Type": "application/json" }, body: JSON.stringify(body), signal });
      return matchSourceResponse(response, body);
    },
    async openReview(sessionId, value) {
      validateSessionId(sessionId);
      const body = parseReviewLaunchRequest(value);
      return matchPanePresentation(await getJson(request, `/api/v1/sessions/${encodeURIComponent(sessionId)}/review/open`, "Review launch", parsePanePresentation, {
        method: "POST", headers: { "Content-Type": "application/json" }, body: JSON.stringify(body),
      }), sessionId);
    },
    async openContext(sessionId, value) {
      validateSessionId(sessionId);
      const body = parseContextLaunchRequest(value);
      return matchPanePresentation(await getJson(request, `/api/v1/sessions/${encodeURIComponent(sessionId)}/context/open`, "Context launch", parsePanePresentation, {
        method: "POST", headers: { "Content-Type": "application/json" }, body: JSON.stringify(body),
      }), sessionId);
    },
    async commentBatches(sessionId, paneId, value, signal) {
      validateSessionId(sessionId);
      validateResourceId(paneId);
      const body = parseCommentScope(value);
      const response = await getJson(request, `/api/v1/sessions/${encodeURIComponent(sessionId)}/panes/${encodeURIComponent(paneId)}/comments/list`, "comment batches", parseCommentBatchList, {
        method: "POST",
        headers: { "Content-Type": "application/json" },
        body: JSON.stringify(body),
        signal,
      });
      matchCommentAttachment(response.attachment, sessionId, paneId, body);
      return response;
    },
    async commentBatch(sessionId, paneId, value, signal) {
      validateSessionId(sessionId);
      validateResourceId(paneId);
      const body = parseCommentBatchRequest(value);
      const response = await getJson(request, `/api/v1/sessions/${encodeURIComponent(sessionId)}/panes/${encodeURIComponent(paneId)}/comments/batch`, "comment batch", parseCommentBatch, {
        method: "POST",
        headers: { "Content-Type": "application/json" },
        body: JSON.stringify(body),
        signal,
      });
      return matchCommentBatch(response, sessionId, paneId, body.scope, body.batch_id);
    },
    async commentUpsert(sessionId, paneId, value, signal) {
      validateSessionId(sessionId);
      validateResourceId(paneId);
      const body = parseCommentUpsert(value);
      const response = await getJson(request, `/api/v1/sessions/${encodeURIComponent(sessionId)}/panes/${encodeURIComponent(paneId)}/comments/upsert`, "comment upsert", parseCommentBatch, {
        method: "POST",
        headers: { "Content-Type": "application/json" },
        body: JSON.stringify(body),
        signal,
      });
      return matchCommentBatch(response, sessionId, paneId, body.batch.scope, body.batch.batch_id, body.batch.expected_generation, true);
    },
    async commentRemove(sessionId, paneId, value, signal) {
      validateSessionId(sessionId);
      validateResourceId(paneId);
      const body = parseCommentRemove(value);
      const response = await getJson(request, `/api/v1/sessions/${encodeURIComponent(sessionId)}/panes/${encodeURIComponent(paneId)}/comments/remove`, "comment remove", parseCommentBatch, {
        method: "POST",
        headers: { "Content-Type": "application/json" },
        body: JSON.stringify(body),
        signal,
      });
      return matchCommentBatch(response, sessionId, paneId, body.batch.scope, body.batch.batch_id, body.batch.expected_generation, true);
    },
    async commentDiscard(sessionId, paneId, value, signal) {
      validateSessionId(sessionId);
      validateResourceId(paneId);
      const body = parseCommentMutation(value);
      const response = await getJson(request, `/api/v1/sessions/${encodeURIComponent(sessionId)}/panes/${encodeURIComponent(paneId)}/comments/discard`, "comment discard", parseCommentBatchList, {
        method: "POST",
        headers: { "Content-Type": "application/json" },
        body: JSON.stringify(body),
        signal,
      });
      matchCommentAttachment(response.attachment, sessionId, paneId, body.scope);
      return response;
    },    async commentAttach(sessionId, paneId, value, signal) {
      validateSessionId(sessionId);
      validateResourceId(paneId);
      const body = parseCommentMutation(value);
      const response = await getJson(request, `/api/v1/sessions/${encodeURIComponent(sessionId)}/panes/${encodeURIComponent(paneId)}/comments/attach`, "comment attach", parseCommentBatch, {
        method: "POST",
        headers: { "Content-Type": "application/json" },
        body: JSON.stringify(body),
        signal,
      });
      return matchCommentBatch(response, sessionId, paneId, body.scope, body.batch_id, body.expected_generation, true);
    },
    async commentPastePrepare(sessionId, paneId, value, signal) {
      validateSessionId(sessionId); validateResourceId(paneId);
      const parsed = parseCommentPastePrepareRequest(value);
      signal?.throwIfAborted();
      const response = await getJson(request, `/api/v1/sessions/${encodeURIComponent(sessionId)}/panes/${encodeURIComponent(paneId)}/comments/paste-prepare`, "comment paste prepare", parseCommentPastePrepare, { method: "POST", headers: { "Content-Type": "application/json" }, body: JSON.stringify(parsed), signal });
      signal?.throwIfAborted();
      return matchPastePrepare(response, sessionId, parsed);
    },
    async commentPasteSend(sessionId, paneId, value) {
      validateSessionId(sessionId); validateResourceId(paneId);
      const parsed = parseCommentPasteSendRequest(value);
      const response = await getJson(request, `/api/v1/sessions/${encodeURIComponent(sessionId)}/panes/${encodeURIComponent(paneId)}/comments/paste-send`, "comment paste send", parseCommentPasteReceipt, { method: "POST", headers: { "Content-Type": "application/json" }, body: JSON.stringify(parsed) });
      return matchPasteReceipt(response, parsed);
    },
    async commentPasteMarkPasted(sessionId, paneId, value) {
      validateSessionId(sessionId); validateResourceId(paneId);
      const parsed = parseCommentPasteMarkPastedRequest(value);
      const response = await getJson(request, `/api/v1/sessions/${encodeURIComponent(sessionId)}/panes/${encodeURIComponent(paneId)}/comments/paste-mark-pasted`, "comment paste resolution", parseCommentPasteReceipt, { method: "POST", headers: { "Content-Type": "application/json" }, body: JSON.stringify(parsed) });
      return matchMarkedReceipt(response, parsed);
    },
    async commentPreview(sessionId, paneId, value, signal) {
      validateSessionId(sessionId);
      validateResourceId(paneId);
      const body = parseCommentPreviewRequest(value);
      const response = await getJson(request, `/api/v1/sessions/${encodeURIComponent(sessionId)}/panes/${encodeURIComponent(paneId)}/comments/preview`, "comment preview", parseCommentPreview, {
        method: "POST",
        headers: { "Content-Type": "application/json" },
        body: JSON.stringify(body),
        signal,
      });
      return matchCommentPreview(response, body.batch);
    },
    async browserAction(value) {
      const body = parseBrowserRequest(value);
      return getJson(request, "/api/v1/browser/action", "browser action", parseBrowserResponse, {
        method: "POST", headers: { "Content-Type": "application/json" }, body: JSON.stringify(body),
      });
    },
    async browserFeedback(value) {
      const body = parseBrowserFeedbackRequest(value);
      return getJson(request, "/api/v1/browser/feedback", "browser feedback", parseBrowserFeedbackLookup, {
        method: "POST", headers: { "Content-Type": "application/json" }, body: JSON.stringify(body),
      });
    },
    async browserDraftRecovery(value: BrowserDraftRecoveryRequest): Promise<BrowserViewCommandOutcome> {
      const body = parseBrowserDraftRecoveryRequest(value);
      return getJson(request, "/api/v1/browser/drafts/recovery", "browser draft recovery", parseBrowserViewCommandOutcome, {
        method: "POST", headers: { "Content-Type": "application/json" }, body: JSON.stringify(body),
      });
    },
    async acknowledgeBrowserFeedback(value) {
      const body = parseBrowserFeedbackAckRequest(value);
      return getJson(request, "/api/v1/browser/feedback/ack", "browser feedback acknowledgement", parseBrowserFeedbackAck, {
        method: "POST", headers: { "Content-Type": "application/json" }, body: JSON.stringify(body),
      });
    },
    async browserFeedbackImage(value) {
      const body = parseBrowserFeedbackImageRequest(value);
      return getJson(request, "/api/v1/browser/feedback/image", "browser feedback image", parseBrowserFeedbackImage, {
        method: "POST", headers: { "Content-Type": "application/json" }, body: JSON.stringify(body),
      });
    },
    async sendBrowserFeedback(value) {
      const body = parseBrowserFeedbackSendRequest(value);
      return getJson(request, "/api/v1/browser/feedback/send", "browser feedback send", parseBrowserFeedbackSendResponse, {
        method: "POST", headers: { "Content-Type": "application/json" }, body: JSON.stringify(body),
      });
    },
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
    openTerminal(requestValue, onMessage, onError, signal) { return openTerminalStream(webSocketFactory, requestValue, onMessage, onError, signal); },
    openBrowserView(requestValue, onEvent, onFrame, onError, signal) {
      return openBrowserViewStream(request, webSocketFactory, requestValue, onEvent, onFrame, onError, signal);
    },
  };
}
