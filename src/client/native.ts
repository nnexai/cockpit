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
  SessionStreamMessage,
  StatusResponse,
  TerminalCommand,
  TerminalOpenRequest,
  TerminalStreamMessage,
} from "../protocol/generated/v1";
import { transitionSessionStream, type StreamOrderCursor } from "./streamOrder";
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
  validateResourceId,
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
function streamFailure(message: string, cause?: unknown, operationCode?: string): CockpitClientError {
  return new CockpitClientError("stream_error", message, { cause, operationCode });
}
function streamId(value: unknown): string {
  if (typeof value === "string" && value.length > 0) return value;
  if (typeof value === "object" && value !== null && "stream_id" in value && typeof value.stream_id === "string" && value.stream_id.length > 0) return value.stream_id;
  throw new CockpitClientError("malformed_response", "Native stream command returned no stream id");
}

function sessionSubscription(channelFactory: NativeChannelFactory, invoke: NativeInvoke, sessionId: string, onMessage: (message: SessionStreamMessage) => void, onError: (error: CockpitClientError) => void): Promise<ClosableStream> {
  try { validateSessionId(sessionId); } catch (error) { return Promise.reject(error); }
  let closed = false;
  let cancelled = false;
  let activeStreamId: string | undefined;
  let cursor: StreamOrderCursor | null = null;
  const cancel = (id: string) => {
    if (cancelled) return;
    cancelled = true;
    void invoke("cockpit_stream_cancel", { streamId: id }).catch(() => undefined);
  };
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
    const result = transitionSessionStream(sessionId, cursor, message);
    if (result.kind === "ignore") return;
    if (result.kind === "error") {
      closed = true;
      onError(streamFailure(result.message, result.classification, result.code));
      if (activeStreamId !== undefined) cancel(activeStreamId);
      return;
    }
    cursor = result.cursor;
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
  return new Promise<TerminalStream>((resolve, reject) => {
    let settled = false;
    const abort = () => {
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
    openTerminal(request, onMessage, onError, signal) { return terminalSubscription(channelFactory, invoke, request, onMessage, onError, signal); },
  };
}
