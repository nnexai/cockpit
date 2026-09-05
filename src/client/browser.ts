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
import { transitionSessionStream, type StreamOrderCursor } from "./streamOrder";

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
): Promise<TerminalStream> {
  let validated: TerminalOpenRequest;
  try { validated = parseTerminalOpenRequest(request); } catch (error) { return Promise.reject(error); }
  const path = `/api/v1/sessions/${encodeURIComponent(validated.session_id)}/panes/${encodeURIComponent(validated.pane_id)}/terminal?mode=${validated.mode}&takeover=${validated.takeover ? "true" : "false"}&cols=${validated.cols}&rows=${validated.rows}&cell_width_px=${validated.cell_width_px}&cell_height_px=${validated.cell_height_px}`;
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
    async commentAttach(sessionId, paneId, value, signal) {
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
