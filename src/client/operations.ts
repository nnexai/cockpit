import { parseNotesRequest, parseNotesResponse, matchNotesResponse } from "./notesProtocol";
import {
  matchOrchestrationSnapshot, parseOrchestrationSnapshotRequest, parseOrchestrationMutationRequest, 
  parseOrchestrationMutationResponse, parseOrchestrationWaitRequest, parseOrchestrationWaitResponse,
} from "./orchestrationProtocol";
import {
  matchWidgetContent, parseWidgetContentRequest, parseWidgetRemoveRequest, parseWidgetRemoveResponse, 
  parseWidgetSelectRequest, parseWidgetSelectResponse,
} from "./widgetProtocol";
import { parseContextMediaRequest, parseContextMedia, matchContextMedia } from "./contextMediaProtocol";
import {
  matchLibraryAttachmentsOperation, matchLibraryDirectory, matchLibraryDocument, matchLibraryMedia, 
  matchLibraryOperation, parseLibraryAddRequest, parseLibraryAttachmentRequest, parseLibraryDirectory, 
  parseLibraryDirectoryRequest, parseLibraryDocument, parseLibraryDocumentRequest, parseLibraryFileIndex, 
  parseLibraryFileIndexRequest, parseLibraryListing, parseLibraryMedia, parseLibraryMediaRequest, 
  parseLibraryResolveRequest, parseLibraryResolution, parseLibraryOperation, parseLibraryOperationId, 
  parseLibraryRefreshRequest, parseLibraryRemoveRequest, parseLibraryReplaceRequest, 
  parseLibraryConfluenceSpacesRequest, parseLibraryConfluenceSpaces, parseSpaceContextRequest, 
  parseSpaceContextListing, matchSpaceContextListing, parseSpaceAddRequest, matchSpaceOperation, 
  parseSpaceRepositoriesRequest, parseSpaceRemoveRequest,
} from "./libraryProtocol";
import {
  parseReviewSnapshotRequest, parseReviewSnapshot, parseReviewFileRequest, parseReviewFile, 
  matchReviewSnapshot, matchReviewFile,
} from "./reviewProtocol";
import {
  parseCommentPastePrepareRequest, parseCommentPastePrepare, parseCommentPasteSendRequest, 
  parseCommentPasteReceipt, matchPastePrepare, matchPasteReceipt, parseCommentPasteMarkPastedRequest, 
  matchMarkedReceipt,
} from "./commentPasteProtocol";
import {
  matchContextResponse, parseContextDirectory, parseContextFileIndex, parseContextDirectoryRequest, 
  parseContextDocument, parseContextDocumentRequest, parseContextFileIndexRequest, parseViewerSourceOptions, 
  matchViewerSourceOptions, parseViewerOpenRequest, parseViewerContext, matchViewerContext,
} from "./contextProtocol";
import {
  matchContextInvalidationResponse, matchContextSearchResponse, parseContextInvalidationRequest, 
  parseContextInvalidationResponse, parseContextSearchRequest, parseContextSearchResponse,
} from "./contextSearchProtocol";
import {
  matchCommentAttachment, matchCommentBatch, matchCommentPreview, parseCommentBatch, parseCommentBatchList, 
  parseCommentBatchRequest, parseCommentMutation, parseCommentPreview, parseCommentPreviewRequest, 
  parseCommentRemove, parseCommentScope, parseCommentUpsert,
} from "./commentProtocol";
import {
  matchProviderCredential, parseProviderCredentialClearRequest, parseProviderCredentialSetRequest, 
  parseProviderCredentialStatus, parseProviderCredentialStatusList,
} from "./credentialProtocol";
import { parseQuotaStatusResponse } from "./quotaProtocol";
import {
  matchProjectSession, parseProjectConfiguration, parseRepositoryList, parseWorkspaceDefaults, 
  parseWorkspaceDefaultsRequest, parseWorkspaceOperation, parseWorkspaceOperationRequest, 
  parseWorkspaceSetupPlan, parseWorkspaceReconcileRequest, parseWorkspaceSetupRequest, 
  validateProjectOperationId,
} from "./projectProtocol";
import {
  matchWorkspaceTeardownPreview, matchWorkspaceTeardownResult, parseWorkspaceTeardownExecuteRequest, 
  parseWorkspaceTeardownPreview, parseWorkspaceTeardownPreviewRequest, parseWorkspaceTeardownRecoveryList, 
  parseWorkspaceTeardownResult,
} from "./projectTeardownProtocol";
import {
  CockpitClientError, parseBrowserDraftRecoveryRequest, parseBrowserViewCommandOutcome, 
  parseBrowserFeedbackAck, parseBrowserFeedbackAckRequest, parseBrowserFeedbackImage, 
  parseBrowserFeedbackImageRequest, parseBrowserFeedbackLookup, parseBrowserFeedbackRequest, 
  parseBrowserFeedbackSendRequest, parseBrowserFeedbackSendResponse, parseBrowserRequest, parseBrowserResponse, 
  parseBrowserCleanupStatus, parseBrowserCleanupRetryRequest, parseFocusRequest, parseFocusResponse, 
  parseResourceMutationRequest, parseResourceMutationResponse, parseSessionListResponse, 
  parseSessionSnapshotResponse, parseSpaceGitStatusResponse, matchSpaceGitActionResponse, parseStatusResponse, 
  validateSessionId, validateResourceId, type CockpitClient,
} from "./CockpitClient";
import type { FocusRequest, ResourceMutationRequest, SpaceGitActionRequest } from "../protocol/generated/v1";
import { widgetAbortable } from "./widgetTransport";

type Backend = "browser" | "native";
type StreamOperation = "subscribeWidgets" | "subscribeSession" | "openTerminal" | "openBrowserView";
type OperationName = Exclude<keyof CockpitClient, StreamOperation>;
export type RequestOperations = Pick<CockpitClient, OperationName>;

export interface EncodedOperation<R> {
  operation: string;
  http: {
    method: "GET" | "POST"; route: () => string;
    json?: () => unknown; signal?: () => AbortSignal | undefined;
    contentTypeHeader?: "content-type";
  };
  tauri: { command: string; args: () => Record<string, unknown> | undefined };
  parse: (value: unknown) => R;
  matchIdentity?: (response: R) => R;
}
export type OperationTransport = <R>(operation: EncodedOperation<R>) => Promise<R>;
type OperationRow<K extends OperationName> = {
  encode: (backend: Backend, ...args: Parameters<CockpitClient[K]>) => EncodedOperation<Awaited<ReturnType<CockpitClient[K]>>>;
  direct?: "both" | "native";
  throwNativeValidation?: true;
  nativeAbort?: "snapshot" | "widget";
};
type OperationTable = { [K in OperationName]: OperationRow<K> };

// Encoding, parser normalization, and identity matching are separate phases:
// matches outside a transport parser must not acquire its normalization/status.
const operations: OperationTable = {
  orchestrationSnapshot: {
    encode(backend, value) {
      const body = parseOrchestrationSnapshotRequest(value); const query = backend === "native" || body.root_id === null ? "" : `?root_id=${encodeURIComponent(body.root_id)}`;
      return {
        operation: "orchestration snapshot",
        http: { method: "GET", route: () => `/api/v1/sessions/${encodeURIComponent(body.session_id)}/orchestration${query}` },
        tauri: { command: "orchestration_snapshot", args: () => ({ request: body }) },
        parse: response => matchOrchestrationSnapshot(response, body),
      };
    },
  },
  orchestrationMutate: {
    encode(_backend, value) {
      const body = parseOrchestrationMutationRequest(value);
      return {
        operation: "orchestration mutation",
        http: { method: "POST", route: () => `/api/v1/sessions/${encodeURIComponent(body.session_id)}/orchestration/mutations`, json: () => body },
        tauri: { command: "orchestration_mutate", args: () => ({ request: body }) },
        parse: parseOrchestrationMutationResponse,
      };
    },
  },
  orchestrationWait: {
    encode(_backend, value) {
      const body = parseOrchestrationWaitRequest(value);
      return {
        operation: "orchestration wait",
        http: { method: "POST", route: () => "/api/v1/orchestration/wait", json: () => body },
        tauri: { command: "orchestration_wait", args: () => ({ request: body }) },
        parse: parseOrchestrationWaitResponse,
      };
    },
  },
  projectConfiguration: { direct: "both",
    encode(_backend) {
      return {
        operation: "project configuration",
        http: { method: "GET", route: () => "/api/v1/project/configuration" },
        tauri: { command: "cockpit_project_configuration", args: () => (undefined) },
        parse: parseProjectConfiguration,
      };
    },
  },
  providerCredentials: { direct: "both",
    encode(_backend) {
      return {
        operation: "provider credentials",
        http: { method: "GET", route: () => "/api/v1/provider-credentials" },
        tauri: { command: "cockpit_provider_credentials", args: () => (undefined) },
        parse: parseProviderCredentialStatusList,
      };
    },
  },
  setProviderCredential: {
    encode(_backend, value) {
      const body = parseProviderCredentialSetRequest(value);
      return {
        operation: "provider credential",
        http: { method: "POST", route: () => "/api/v1/provider-credentials/set", json: () => body },
        tauri: { command: "cockpit_provider_credential_set", args: () => ({ request: body }) },
        parse: parseProviderCredentialStatus,
        matchIdentity: (response) => { return matchProviderCredential(response, body); },
      };
    },
  },
  clearProviderCredential: {
    encode(_backend, value) {
      const body = parseProviderCredentialClearRequest(value);
      return {
        operation: "provider credential",
        http: { method: "POST", route: () => "/api/v1/provider-credentials/clear", json: () => body },
        tauri: { command: "cockpit_provider_credential_clear", args: () => ({ request: body }) },
        parse: parseProviderCredentialStatus,
        matchIdentity: (response) => { return matchProviderCredential(response, body); },
      };
    },
  },
  repositories: { direct: "both",
    encode(_backend) {
      return {
        operation: "repositories",
        http: { method: "GET", route: () => "/api/v1/project/repositories" },
        tauri: { command: "cockpit_repositories", args: () => (undefined) },
        parse: parseRepositoryList,
      };
    },
  },
  resolveWorkspaceDefaults: { direct: "native", throwNativeValidation: true,
    encode(_backend, value) {
      const body = parseWorkspaceDefaultsRequest(value);
      return {
        operation: "workspace defaults",
        http: { method: "POST", route: () => "/api/v1/project/defaults", json: () => body, contentTypeHeader: "content-type" },
        tauri: { command: "cockpit_resolve_workspace_defaults", args: () => ({ request: body }) },
        parse: parseWorkspaceDefaults,
      };
    },
  },
  planWorkspace: {
    encode(_backend, sessionId, value) {
      validateSessionId(sessionId); const body = parseWorkspaceSetupRequest(value);
      return {
        operation: "workspace plan",
        http: { method: "POST", route: () => `/api/v1/sessions/${encodeURIComponent(sessionId)}/workspace-plans`, json: () => body },
        tauri: { command: "cockpit_workspace_plan", args: () => ({ sessionId, request: body }) },
        parse: parseWorkspaceSetupPlan,
        matchIdentity: (response) => { return matchProjectSession(response, sessionId); },
      };
    },
  },
  startWorkspace: {
    encode(_backend, sessionId, value) {
      validateSessionId(sessionId); const body = parseWorkspaceOperationRequest(value);
      return {
        operation: "workspace start",
        http: { method: "POST", route: () => `/api/v1/sessions/${encodeURIComponent(sessionId)}/workspace-operations`, json: () => body },
        tauri: { command: "cockpit_workspace_start", args: () => ({ sessionId, request: body }) },
        parse: parseWorkspaceOperation,
        matchIdentity: (response) => { return matchProjectSession(response, sessionId, body.operation_id); },
      };
    },
  },
  workspaceOperation: {
    encode(_backend, sessionId, operationId) {
      validateSessionId(sessionId); validateProjectOperationId(operationId);
      return {
        operation: "workspace operation",
        http: { method: "GET", route: () => `/api/v1/sessions/${encodeURIComponent(sessionId)}/workspace-operations/${encodeURIComponent(operationId)}` },
        tauri: { command: "cockpit_workspace_operation", args: () => ({ sessionId, operationId }) },
        parse: parseWorkspaceOperation,
        matchIdentity: (response) => { return matchProjectSession(response, sessionId, operationId); },
      };
    },
  },
  resumeWorkspace: {
    encode(_backend, sessionId, value) {
      validateSessionId(sessionId); const body = parseWorkspaceOperationRequest(value);
      return {
        operation: "workspace resume",
        http: { method: "POST", route: () => `/api/v1/sessions/${encodeURIComponent(sessionId)}/workspace-operations/resume`, json: () => body },
        tauri: { command: "cockpit_workspace_resume", args: () => ({ sessionId, request: body }) },
        parse: parseWorkspaceOperation,
        matchIdentity: (response) => { return matchProjectSession(response, sessionId, body.operation_id); },
      };
    },
  },
  cancelWorkspace: {
    encode(_backend, sessionId, value) {
      validateSessionId(sessionId); const body = parseWorkspaceOperationRequest(value);
      return {
        operation: "workspace cancellation",
        http: { method: "POST", route: () => `/api/v1/sessions/${encodeURIComponent(sessionId)}/workspace-operations/cancel`, json: () => body },
        tauri: { command: "cockpit_workspace_cancel", args: () => ({ sessionId, request: body }) },
        parse: parseWorkspaceOperation,
        matchIdentity: (response) => { return matchProjectSession(response, sessionId, body.operation_id); },
      };
    },
  },
  reconcileWorkspace: {
    encode(_backend, sessionId, value) {
      validateSessionId(sessionId); const body = parseWorkspaceReconcileRequest(value);
      return {
        operation: "workspace reconciliation",
        http: { method: "POST", route: () => `/api/v1/sessions/${encodeURIComponent(sessionId)}/workspace-operations/reconcile`, json: () => body },
        tauri: { command: "cockpit_workspace_reconcile", args: () => ({ sessionId, request: body }) },
        parse: parseWorkspaceOperation,
        matchIdentity: (response) => { return matchProjectSession(response, sessionId, body.operation_id); },
      };
    },
  },
  workspaceTeardownPreview: {
    encode(_backend, sessionId, value) {
      validateSessionId(sessionId); const body = parseWorkspaceTeardownPreviewRequest(value);
      return {
        operation: "workspace teardown preview",
        http: { method: "POST", route: () => `/api/v1/sessions/${encodeURIComponent(sessionId)}/workspace-teardown/preview`, json: () => body },
        tauri: { command: "cockpit_workspace_teardown_preview", args: () => ({ sessionId, request: body }) },
        parse: parseWorkspaceTeardownPreview,
        matchIdentity: (response) => { return matchWorkspaceTeardownPreview(response, body); },
      };
    },
  },
  workspaceTeardownExecute: {
    encode(_backend, sessionId, value) {
      validateSessionId(sessionId); const body = parseWorkspaceTeardownExecuteRequest(value);
      return {
        operation: "workspace teardown execution",
        http: { method: "POST", route: () => `/api/v1/sessions/${encodeURIComponent(sessionId)}/workspace-teardown/execute`, json: () => body },
        tauri: { command: "cockpit_workspace_teardown_execute", args: () => ({ sessionId, request: body }) },
        parse: parseWorkspaceTeardownResult,
        matchIdentity: (response) => { return matchWorkspaceTeardownResult(response, body); },
      };
    },
  },
  workspaceTeardownRecoveries: {
    encode(_backend, sessionId) {
      validateSessionId(sessionId);
      return {
        operation: "workspace teardown recoveries",
        http: { method: "GET", route: () => `/api/v1/sessions/${encodeURIComponent(sessionId)}/workspace-teardown/recoveries` },
        tauri: { command: "cockpit_workspace_teardown_recoveries", args: () => ({ sessionId }) },
        parse: parseWorkspaceTeardownRecoveryList,
      };
    },
  },
  viewerSources: {
    encode(_backend, sessionId, paneId, signal) {
      signal?.throwIfAborted(); validateSessionId(sessionId); validateResourceId(paneId);
      return {
        operation: "viewer sources",
        http: { method: "GET", route: () => `/api/v1/sessions/${encodeURIComponent(sessionId)}/panes/${encodeURIComponent(paneId)}/viewer-sources`, signal: () => signal },
        tauri: { command: "cockpit_viewer_sources", args: () => ({ sessionId, paneId }) },
        parse: parseViewerSourceOptions,
        matchIdentity: (response) => { signal?.throwIfAborted(); return matchViewerSourceOptions(response, sessionId, paneId); },
      };
    },
  },
  viewerOpen: {
    encode(_backend, sessionId, value) {
      validateSessionId(sessionId); const body = parseViewerOpenRequest(value);
      return {
        operation: "viewer open",
        http: { method: "POST", route: () => `/api/v1/sessions/${encodeURIComponent(sessionId)}/viewers/open`, json: () => body },
        tauri: { command: "cockpit_viewer_open", args: () => ({ sessionId, request: body }) },
        parse: parseViewerContext,
        matchIdentity: (response) => { return matchViewerContext(response, sessionId, body); },
      };
    },
  },
  viewerRelease: {
    encode(_backend, sessionId, viewerId) {
      validateSessionId(sessionId); validateResourceId(viewerId);
      return {
        operation: "viewer release",
        http: { method: "POST", route: () => `/api/v1/sessions/${encodeURIComponent(sessionId)}/viewers/${encodeURIComponent(viewerId)}/release` },
        tauri: { command: "cockpit_viewer_release", args: () => ({ sessionId, viewerId }) },
        parse: (value) => { if (value !== null) throw new CockpitClientError("malformed_response", "Viewer release response is malformed"); },
        matchIdentity: (response) => { return response; },
      };
    },
  },
  contextDirectory: {
    encode(backend, sessionId, viewerId, value, signal) {
      if (backend === "native") signal?.throwIfAborted(); validateSessionId(sessionId); validateResourceId(viewerId); const body = parseContextDirectoryRequest(value);
      return {
        operation: "Context directory",
        http: { method: "POST", route: () => `/api/v1/sessions/${encodeURIComponent(sessionId)}/viewers/${encodeURIComponent(viewerId)}/context/directory`, json: () => body, signal: () => signal },
        tauri: { command: "cockpit_context_directory", args: () => ({ sessionId, viewerId, request: body }) },
        parse: parseContextDirectory,
        matchIdentity: (response) => { if (backend === "native") signal?.throwIfAborted(); return matchContextResponse(response, body); },
      };
    },
  },
  contextFileIndex: {
    encode(backend, sessionId, viewerId, value, signal) {
      if (backend === "native") signal?.throwIfAborted(); validateSessionId(sessionId); validateResourceId(viewerId); const body = parseContextFileIndexRequest(value);
      return {
        operation: "Context file index",
        http: { method: "POST", route: () => `/api/v1/sessions/${encodeURIComponent(sessionId)}/viewers/${encodeURIComponent(viewerId)}/context/files`, json: () => body, signal: () => signal },
        tauri: { command: "cockpit_context_file_index", args: () => ({ sessionId, viewerId, request: body }) },
        parse: parseContextFileIndex,
        matchIdentity: (response) => { signal?.throwIfAborted(); if (response.binding_id !== body.binding_id || response.root_id !== body.root_id) throw new CockpitClientError("malformed_response", "Context file index belongs to another root"); return response; },
      };
    },
  },
  contextDocument: {
    encode(backend, sessionId, viewerId, value, signal) {
      if (backend === "native") signal?.throwIfAborted(); validateSessionId(sessionId); validateResourceId(viewerId); const body = parseContextDocumentRequest(value);
      return {
        operation: "Context document",
        http: { method: "POST", route: () => `/api/v1/sessions/${encodeURIComponent(sessionId)}/viewers/${encodeURIComponent(viewerId)}/context/document`, json: () => body, signal: () => signal },
        tauri: { command: "cockpit_context_document", args: () => ({ sessionId, viewerId, request: body }) },
        parse: parseContextDocument,
        matchIdentity: (response) => { if (backend === "native") signal?.throwIfAborted(); return matchContextResponse(response, body); },
      };
    },
  },
  reviewSnapshot: {
    encode(_backend, sessionId, viewerId, value, signal) {
      validateSessionId(sessionId); validateResourceId(viewerId); signal?.throwIfAborted(); const parsed = parseReviewSnapshotRequest(value);
      return {
        operation: "Review snapshot",
        http: { method: "POST", route: () => `/api/v1/sessions/${encodeURIComponent(sessionId)}/viewers/${encodeURIComponent(viewerId)}/review/snapshot`, json: () => parsed, signal: () => signal },
        tauri: { command: "cockpit_review_snapshot", args: () => ({ sessionId, viewerId, request: parsed }) },
        parse: parseReviewSnapshot,
        matchIdentity: (response) => { signal?.throwIfAborted(); return matchReviewSnapshot(response, sessionId, viewerId, parsed); },
      };
    },
  },
  reviewFile: {
    encode(_backend, sessionId, viewerId, value, signal) {
      validateSessionId(sessionId); validateResourceId(viewerId); signal?.throwIfAborted(); const parsed = parseReviewFileRequest(value);
      return {
        operation: "Review file",
        http: { method: "POST", route: () => `/api/v1/sessions/${encodeURIComponent(sessionId)}/viewers/${encodeURIComponent(viewerId)}/review/file`, json: () => parsed, signal: () => signal },
        tauri: { command: "cockpit_review_file", args: () => ({ sessionId, viewerId, request: parsed }) },
        parse: parseReviewFile,
        matchIdentity: (response) => { signal?.throwIfAborted(); return matchReviewFile(response, sessionId, viewerId, parsed); },
      };
    },
  },
  contextSearch: {
    encode(backend, sessionId, viewerId, value, signal) {
      if (backend === "native") signal?.throwIfAborted(); validateSessionId(sessionId); validateResourceId(viewerId); const body = parseContextSearchRequest(value);
      return {
        operation: "Context search",
        http: { method: "POST", route: () => `/api/v1/sessions/${encodeURIComponent(sessionId)}/viewers/${encodeURIComponent(viewerId)}/context/search`, json: () => body, signal: () => signal },
        tauri: { command: "cockpit_context_search", args: () => ({ sessionId, viewerId, request: body }) },
        parse: parseContextSearchResponse,
        matchIdentity: (response) => { if (backend === "native") signal?.throwIfAborted(); return matchContextSearchResponse(response, body); },
      };
    },
  },
  contextInvalidate: {
    encode(backend, sessionId, viewerId, value, signal) {
      if (backend === "native") signal?.throwIfAborted(); validateSessionId(sessionId); validateResourceId(viewerId); const body = parseContextInvalidationRequest(value);
      return {
        operation: "Context invalidation",
        http: { method: "POST", route: () => `/api/v1/sessions/${encodeURIComponent(sessionId)}/viewers/${encodeURIComponent(viewerId)}/context/invalidate`, json: () => body, signal: () => signal },
        tauri: { command: "cockpit_context_invalidate", args: () => ({ sessionId, viewerId, request: body }) },
        parse: parseContextInvalidationResponse,
        matchIdentity: (response) => { if (backend === "native") signal?.throwIfAborted(); return matchContextInvalidationResponse(response, body); },
      };
    },
  },
  contextMedia: {
    encode(backend, sessionId, viewerId, value, signal) {
      signal?.throwIfAborted(); validateSessionId(sessionId); validateResourceId(viewerId); const body = parseContextMediaRequest(value);
      return {
        operation: "Context image",
        http: { method: "POST", route: () => `/api/v1/sessions/${encodeURIComponent(sessionId)}/viewers/${encodeURIComponent(viewerId)}/context/media`, json: () => body, signal: () => signal },
        tauri: { command: "cockpit_context_media", args: () => ({ sessionId, viewerId, request: body }) },
        parse: parseContextMedia,
        matchIdentity: (response) => { if (backend === "native") signal?.throwIfAborted(); return matchContextMedia(response, body); },
      };
    },
  },
  libraryListing: {
    encode(backend, offset) {
      if (offset !== undefined && offset !== null && (!Number.isInteger(offset) || offset < 0 || offset > 0xffffffff)) throw new CockpitClientError("malformed_response", "Invalid Library listing offset"); const query = backend === "native" || offset == null ? "" : `?offset=${offset}`;
      return {
        operation: "Library listing",
        http: { method: "GET", route: () => `/api/v1/library${query}` },
        tauri: { command: "cockpit_library_listing", args: () => (offset == null ? undefined : { offset }) },
        parse: parseLibraryListing,
      };
    },
  },
  libraryResolve: {
    encode(_backend, value) {
      const body = parseLibraryResolveRequest(value);
      return {
        operation: "Library resolve",
        http: { method: "POST", route: () => "/api/v1/library/resolve", json: () => body },
        tauri: { command: "cockpit_library_resolve", args: () => ({ request: body }) },
        parse: parseLibraryResolution,
      };
    },
  },
  libraryConfluenceSpaces: {
    encode(_backend, value) {
      const body = parseLibraryConfluenceSpacesRequest(value);
      return {
        operation: "Confluence spaces",
        http: { method: "POST", route: () => "/api/v1/library/confluence/spaces", json: () => body },
        tauri: { command: "cockpit_library_confluence_spaces", args: () => ({ request: body }) },
        parse: (response) => parseLibraryConfluenceSpaces(response, body),
      };
    },
  },
  libraryAdd: {
    encode(_backend, value) {
      const body = parseLibraryAddRequest(value);
      return {
        operation: "Library add",
        http: { method: "POST", route: () => "/api/v1/library/add", json: () => body },
        tauri: { command: "cockpit_library_add", args: () => ({ request: body }) },
        parse: parseLibraryOperation,
        matchIdentity: (response) => { return body.target ? matchSpaceOperation(response, body.target) : response; },
      };
    },
  },
  libraryAttachments: {
    encode(_backend, value) {
      const body = parseLibraryAttachmentRequest(value);
      return {
        operation: "Library attachments",
        http: { method: "POST", route: () => "/api/v1/library/attachments", json: () => body },
        tauri: { command: "cockpit_library_attachments", args: () => ({ request: body }) },
        parse: parseLibraryOperation,
        matchIdentity: (response) => { return matchLibraryAttachmentsOperation(response, body); },
      };
    },
  },
  libraryRefresh: {
    encode(_backend, value) {
      const body = parseLibraryRefreshRequest(value);
      return {
        operation: "Library refresh",
        http: { method: "POST", route: () => "/api/v1/library/refresh", json: () => body },
        tauri: { command: "cockpit_library_refresh", args: () => ({ request: body }) },
        parse: parseLibraryOperation,
      };
    },
  },
  libraryOperation: {
    encode(_backend, operationId) {
      const id = parseLibraryOperationId(operationId);
      return {
        operation: "Library operation",
        http: { method: "GET", route: () => `/api/v1/library/operations/${encodeURIComponent(id)}` },
        tauri: { command: "cockpit_library_operation", args: () => ({ operationId: id }) },
        parse: parseLibraryOperation,
        matchIdentity: (response) => { return matchLibraryOperation(response, id); },
      };
    },
  },
  libraryOperationCancel: {
    encode(_backend, operationId) {
      const id = parseLibraryOperationId(operationId);
      return {
        operation: "Library operation cancellation",
        http: { method: "POST", route: () => `/api/v1/library/operations/${encodeURIComponent(id)}/cancel` },
        tauri: { command: "cockpit_library_operation_cancel", args: () => ({ operationId: id }) },
        parse: parseLibraryOperation,
        matchIdentity: (response) => { return matchLibraryOperation(response, id); },
      };
    },
  },
  libraryReplace: {
    encode(_backend, value) {
      const body = parseLibraryReplaceRequest(value);
      return {
        operation: "Library replace",
        http: { method: "POST", route: () => "/api/v1/library/replace", json: () => body },
        tauri: { command: "cockpit_library_replace", args: () => ({ request: body }) },
        parse: parseLibraryOperation,
      };
    },
  },
  libraryRemove: {
    encode(_backend, value) {
      const body = parseLibraryRemoveRequest(value);
      return {
        operation: "Library remove",
        http: { method: "POST", route: () => "/api/v1/library/remove", json: () => body },
        tauri: { command: "cockpit_library_remove", args: () => ({ request: body }) },
        parse: parseLibraryListing,
      };
    },
  },
  libraryDirectory: {
    encode(_backend, value, signal) {
      signal?.throwIfAborted(); const body = parseLibraryDirectoryRequest(value);
      return {
        operation: "Library directory",
        http: { method: "POST", route: () => "/api/v1/library/directory", json: () => body, signal: () => signal },
        tauri: { command: "cockpit_library_directory", args: () => ({ request: body }) },
        parse: parseLibraryDirectory,
        matchIdentity: (response) => { signal?.throwIfAborted(); return matchLibraryDirectory(response, body); },
      };
    },
  },
  libraryFileIndex: {
    encode(_backend, value, signal) {
      signal?.throwIfAborted(); const body = parseLibraryFileIndexRequest(value);
      return {
        operation: "Library file index",
        http: { method: "POST", route: () => "/api/v1/library/files", json: () => body, signal: () => signal },
        tauri: { command: "cockpit_library_file_index", args: () => ({ request: body }) },
        parse: parseLibraryFileIndex,
        matchIdentity: (response) => { signal?.throwIfAborted(); return response; },
      };
    },
  },
  libraryDocument: {
    encode(_backend, value, signal) {
      signal?.throwIfAborted(); const body = parseLibraryDocumentRequest(value);
      return {
        operation: "Library document",
        http: { method: "POST", route: () => "/api/v1/library/document", json: () => body, signal: () => signal },
        tauri: { command: "cockpit_library_document", args: () => ({ request: body }) },
        parse: parseLibraryDocument,
        matchIdentity: (response) => { signal?.throwIfAborted(); return matchLibraryDocument(response, body); },
      };
    },
  },
  libraryMedia: {
    encode(_backend, value, signal) {
      signal?.throwIfAborted(); const body = parseLibraryMediaRequest(value);
      return {
        operation: "Library media",
        http: { method: "POST", route: () => "/api/v1/library/media", json: () => body, signal: () => signal },
        tauri: { command: "cockpit_library_media", args: () => ({ request: body }) },
        parse: parseLibraryMedia,
        matchIdentity: (response) => { signal?.throwIfAborted(); return matchLibraryMedia(response, body); },
      };
    },
  },
  notes: {
    encode(_backend, value, signal) {
      signal?.throwIfAborted(); const body = parseNotesRequest(value);
      return {
        operation: "Notes",
        http: { method: "POST", route: () => "/api/v1/notes", json: () => body, signal: () => signal },
        tauri: { command: "cockpit_notes_execute", args: () => ({ request: body }) },
        parse: parseNotesResponse,
        matchIdentity: (response) => { signal?.throwIfAborted(); return matchNotesResponse(response, body); },
      };
    },
  },
  librarySpaceList: {
    encode(_backend, value, signal) {
      signal?.throwIfAborted(); const body = parseSpaceContextRequest(value);
      return {
        operation: "Space context",
        http: { method: "POST", route: () => "/api/v1/library/space/list", json: () => body, signal: () => signal },
        tauri: { command: "cockpit_library_space_list", args: () => ({ request: body }) },
        parse: parseSpaceContextListing,
        matchIdentity: (response) => { signal?.throwIfAborted(); return matchSpaceContextListing(response, body); },
      };
    },
  },
  librarySpaceAdd: {
    encode(_backend, value) {
      const body = parseSpaceAddRequest(value);
      return {
        operation: "Space add",
        http: { method: "POST", route: () => "/api/v1/library/space/add", json: () => body },
        tauri: { command: "cockpit_library_space_add", args: () => ({ request: body }) },
        parse: parseLibraryOperation,
        matchIdentity: (response) => { return matchSpaceOperation(response, body.target); },
      };
    },
  },
  librarySpaceRepositories: {
    encode(_backend, value) {
      const body = parseSpaceRepositoriesRequest(value);
      return {
        operation: "Space repositories",
        http: { method: "POST", route: () => "/api/v1/library/space/repositories", json: () => body },
        tauri: { command: "cockpit_library_space_repositories", args: () => ({ request: body }) },
        parse: parseSpaceContextListing,
        matchIdentity: (response) => { return matchSpaceContextListing(response, body); },
      };
    },
  },
  librarySpaceRemove: {
    encode(_backend, value) {
      const body = parseSpaceRemoveRequest(value);
      return {
        operation: "Space removal",
        http: { method: "POST", route: () => "/api/v1/library/space/remove", json: () => body },
        tauri: { command: "cockpit_library_space_remove", args: () => ({ request: body }) },
        parse: parseSpaceContextListing,
        matchIdentity: (response) => { return matchSpaceContextListing(response, body); },
      };
    },
  },
  commentBatches: {
    encode(backend, sessionId, viewerId, value, signal) {
      if (backend === "native") signal?.throwIfAborted(); validateSessionId(sessionId); validateResourceId(viewerId); const body = parseCommentScope(value);
      return {
        operation: "comment batches",
        http: { method: "POST", route: () => `/api/v1/sessions/${encodeURIComponent(sessionId)}/viewers/${encodeURIComponent(viewerId)}/comments/list`, json: () => body, signal: () => signal },
        tauri: { command: "cockpit_comments_list", args: () => ({ sessionId, viewerId, request: body }) },
        parse: parseCommentBatchList,
        matchIdentity: (response) => { if (backend === "native") signal?.throwIfAborted(); matchCommentAttachment(response.attachment, sessionId, body); return response; },
      };
    },
  },
  commentBatch: {
    encode(backend, sessionId, viewerId, value, signal) {
      if (backend === "native") signal?.throwIfAborted(); validateSessionId(sessionId); validateResourceId(viewerId); const body = parseCommentBatchRequest(value);
      return {
        operation: "comment batch",
        http: { method: "POST", route: () => `/api/v1/sessions/${encodeURIComponent(sessionId)}/viewers/${encodeURIComponent(viewerId)}/comments/batch`, json: () => body, signal: () => signal },
        tauri: { command: "cockpit_comments_batch", args: () => ({ sessionId, viewerId, request: body }) },
        parse: parseCommentBatch,
        matchIdentity: (response) => { if (backend === "native") signal?.throwIfAborted(); return matchCommentBatch(response, sessionId, body.scope, body.batch_id); },
      };
    },
  },
  commentUpsert: {
    encode(backend, sessionId, viewerId, value, signal) {
      if (backend === "native") signal?.throwIfAborted(); validateSessionId(sessionId); validateResourceId(viewerId); const body = parseCommentUpsert(value);
      return {
        operation: "comment upsert",
        http: { method: "POST", route: () => `/api/v1/sessions/${encodeURIComponent(sessionId)}/viewers/${encodeURIComponent(viewerId)}/comments/upsert`, json: () => body, signal: () => signal },
        tauri: { command: "cockpit_comments_upsert", args: () => ({ sessionId, viewerId, request: body }) },
        parse: parseCommentBatch,
        matchIdentity: (response) => { if (backend === "native") signal?.throwIfAborted(); return matchCommentBatch(response, sessionId, body.batch.scope, body.batch.batch_id, body.batch.expected_generation, true); },
      };
    },
  },
  commentRemove: {
    encode(backend, sessionId, viewerId, value, signal) {
      if (backend === "native") signal?.throwIfAborted(); validateSessionId(sessionId); validateResourceId(viewerId); const body = parseCommentRemove(value);
      return {
        operation: "comment remove",
        http: { method: "POST", route: () => `/api/v1/sessions/${encodeURIComponent(sessionId)}/viewers/${encodeURIComponent(viewerId)}/comments/remove`, json: () => body, signal: () => signal },
        tauri: { command: "cockpit_comments_remove", args: () => ({ sessionId, viewerId, request: body }) },
        parse: parseCommentBatch,
        matchIdentity: (response) => { if (backend === "native") signal?.throwIfAborted(); return matchCommentBatch(response, sessionId, body.batch.scope, body.batch.batch_id, body.batch.expected_generation, true); },
      };
    },
  },
  commentDiscard: {
    encode(backend, sessionId, viewerId, value, signal) {
      if (backend === "native") signal?.throwIfAborted(); validateSessionId(sessionId); validateResourceId(viewerId); const body = parseCommentMutation(value);
      return {
        operation: "comment discard",
        http: { method: "POST", route: () => `/api/v1/sessions/${encodeURIComponent(sessionId)}/viewers/${encodeURIComponent(viewerId)}/comments/discard`, json: () => body, signal: () => signal },
        tauri: { command: "cockpit_comments_discard", args: () => ({ sessionId, viewerId, request: body }) },
        parse: parseCommentBatchList,
        matchIdentity: (response) => { if (backend === "native") signal?.throwIfAborted(); matchCommentAttachment(response.attachment, sessionId, body.scope); return response; },
      };
    },
  },
  commentAttach: {
    encode(backend, sessionId, viewerId, value, signal) {
      if (backend === "native") signal?.throwIfAborted(); validateSessionId(sessionId); validateResourceId(viewerId); const body = parseCommentMutation(value);
      return {
        operation: "comment attach",
        http: { method: "POST", route: () => `/api/v1/sessions/${encodeURIComponent(sessionId)}/viewers/${encodeURIComponent(viewerId)}/comments/attach`, json: () => body, signal: () => signal },
        tauri: { command: "cockpit_comments_attach", args: () => ({ sessionId, viewerId, request: body }) },
        parse: parseCommentBatch,
        matchIdentity: (response) => { if (backend === "native") signal?.throwIfAborted(); return matchCommentBatch(response, sessionId, body.scope, body.batch_id, body.expected_generation, true); },
      };
    },
  },
  commentPastePrepare: {
    encode(_backend, sessionId, viewerId, value, signal) {
      validateSessionId(sessionId); validateResourceId(viewerId); const parsed = parseCommentPastePrepareRequest(value); signal?.throwIfAborted();
      return {
        operation: "comment paste prepare",
        http: { method: "POST", route: () => `/api/v1/sessions/${encodeURIComponent(sessionId)}/viewers/${encodeURIComponent(viewerId)}/comments/paste-prepare`, json: () => parsed, signal: () => signal },
        tauri: { command: "cockpit_comments_paste_prepare", args: () => ({ sessionId, viewerId, request: parsed }) },
        parse: parseCommentPastePrepare,
        matchIdentity: (response) => { signal?.throwIfAborted(); return matchPastePrepare(response, sessionId, parsed); },
      };
    },
  },
  commentPasteSend: {
    encode(_backend, sessionId, viewerId, value) {
      validateSessionId(sessionId); validateResourceId(viewerId); const parsed = parseCommentPasteSendRequest(value);
      return {
        operation: "comment paste send",
        http: { method: "POST", route: () => `/api/v1/sessions/${encodeURIComponent(sessionId)}/viewers/${encodeURIComponent(viewerId)}/comments/paste-send`, json: () => parsed },
        tauri: { command: "cockpit_comments_paste_send", args: () => ({ sessionId, viewerId, request: parsed }) },
        parse: parseCommentPasteReceipt,
        matchIdentity: (response) => { return matchPasteReceipt(response, parsed); },
      };
    },
  },
  commentPasteMarkPasted: {
    encode(_backend, sessionId, viewerId, value) {
      validateSessionId(sessionId); validateResourceId(viewerId); const parsed = parseCommentPasteMarkPastedRequest(value);
      return {
        operation: "comment paste resolution",
        http: { method: "POST", route: () => `/api/v1/sessions/${encodeURIComponent(sessionId)}/viewers/${encodeURIComponent(viewerId)}/comments/paste-mark-pasted`, json: () => parsed },
        tauri: { command: "cockpit_comments_paste_mark_pasted", args: () => ({ sessionId, viewerId, request: parsed }) },
        parse: parseCommentPasteReceipt,
        matchIdentity: (response) => { return matchMarkedReceipt(response, parsed); },
      };
    },
  },
  commentPreview: {
    encode(backend, sessionId, viewerId, value, signal) {
      if (backend === "native") signal?.throwIfAborted(); validateSessionId(sessionId); validateResourceId(viewerId); const body = parseCommentPreviewRequest(value);
      return {
        operation: "comment preview",
        http: { method: "POST", route: () => `/api/v1/sessions/${encodeURIComponent(sessionId)}/viewers/${encodeURIComponent(viewerId)}/comments/preview`, json: () => body, signal: () => signal },
        tauri: { command: "cockpit_comments_preview", args: () => ({ sessionId, viewerId, request: body }) },
        parse: parseCommentPreview,
        matchIdentity: (response) => { if (backend === "native") signal?.throwIfAborted(); return matchCommentPreview(response, body.batch); },
      };
    },
  },
  browserCleanupStatus: { direct: "both",
    encode(_backend) {
      return {
        operation: "browser cleanup",
        http: { method: "GET", route: () => "/api/v1/browser/cleanup" },
        tauri: { command: "cockpit_browser_cleanup_status", args: () => (undefined) },
        parse: parseBrowserCleanupStatus,
      };
    },
  },
  browserCleanupRetry: { direct: "native", throwNativeValidation: true,
    encode(_backend, value) {
      const body = parseBrowserCleanupRetryRequest(value);
      return {
        operation: "browser cleanup retry",
        http: { method: "POST", route: () => "/api/v1/browser/cleanup/retry", json: () => body },
        tauri: { command: "cockpit_browser_cleanup_retry", args: () => ({ request: body }) },
        parse: parseBrowserCleanupStatus,
      };
    },
  },
  browserAction: { direct: "native", throwNativeValidation: true,
    encode(_backend, value) {
      const body = parseBrowserRequest(value);
      return {
        operation: "browser action",
        http: { method: "POST", route: () => "/api/v1/browser/action", json: () => body },
        tauri: { command: "cockpit_browser_action", args: () => ({ request: body }) },
        parse: parseBrowserResponse,
      };
    },
  },
  browserFeedback: {
    encode(_backend, value) {
      const body = parseBrowserFeedbackRequest(value);
      return {
        operation: "browser feedback",
        http: { method: "POST", route: () => "/api/v1/browser/feedback", json: () => body },
        tauri: { command: "cockpit_browser_feedback", args: () => ({ request: body }) },
        parse: parseBrowserFeedbackLookup,
      };
    },
  },
  browserDraftRecovery: {
    encode(_backend, value) {
      const body = parseBrowserDraftRecoveryRequest(value);
      return {
        operation: "browser draft recovery",
        http: { method: "POST", route: () => "/api/v1/browser/drafts/recovery", json: () => body },
        tauri: { command: "cockpit_browser_draft_recovery", args: () => ({ request: body }) },
        parse: parseBrowserViewCommandOutcome,
      };
    },
  },
  acknowledgeBrowserFeedback: {
    encode(_backend, value) {
      const body = parseBrowserFeedbackAckRequest(value);
      return {
        operation: "browser feedback acknowledgement",
        http: { method: "POST", route: () => "/api/v1/browser/feedback/ack", json: () => body },
        tauri: { command: "cockpit_browser_feedback_ack", args: () => ({ request: body }) },
        parse: parseBrowserFeedbackAck,
      };
    },
  },
  browserFeedbackImage: {
    encode(_backend, value) {
      const body = parseBrowserFeedbackImageRequest(value);
      return {
        operation: "browser feedback image",
        http: { method: "POST", route: () => "/api/v1/browser/feedback/image", json: () => body },
        tauri: { command: "cockpit_browser_feedback_image", args: () => ({ request: body }) },
        parse: parseBrowserFeedbackImage,
      };
    },
  },
  sendBrowserFeedback: {
    encode(_backend, value) {
      const body = parseBrowserFeedbackSendRequest(value);
      return {
        operation: "browser feedback send",
        http: { method: "POST", route: () => "/api/v1/browser/feedback/send", json: () => body },
        tauri: { command: "cockpit_browser_feedback_send", args: () => ({ request: body }) },
        parse: parseBrowserFeedbackSendResponse,
      };
    },
  },
  status: { direct: "both",
    encode(_backend) {
      return {
        operation: "status",
        http: { method: "GET", route: () => "/api/v1/status" },
        tauri: { command: "cockpit_status", args: () => (undefined) },
        parse: parseStatusResponse,
      };
    },
  },
  quotaStatus: { direct: "both",
    encode(_backend, quotaRequest, signal) {
      signal?.throwIfAborted();
      return {
        operation: "subscription quota",
        http: { method: "GET", route: () => `/api/v1/quota?agents_working=${quotaRequest.agents_working ? "true" : "false"}`, signal: () => signal },
        tauri: { command: "cockpit_quota_status", args: () => ({ request: { agents_working: quotaRequest.agents_working } }) },
        parse: parseQuotaStatusResponse,
        matchIdentity: (response) => { signal?.throwIfAborted(); return response; },
      };
    },
  },
  sessions: { direct: "both",
    encode(_backend) {
      return {
        operation: "sessions",
        http: { method: "GET", route: () => "/api/v1/sessions" },
        tauri: { command: "cockpit_sessions", args: () => (undefined) },
        parse: parseSessionListResponse,
      };
    },
  },
  sessionSnapshot: { direct: "both", nativeAbort: "snapshot",
    encode(_backend, sessionId, signal) {
      validateSessionId(sessionId); signal?.throwIfAborted();
      return {
        operation: "session snapshot",
        http: { method: "GET", route: () => `/api/v1/sessions/${encodeURIComponent(sessionId)}/snapshot`, signal: () => signal },
        tauri: { command: "cockpit_session_snapshot", args: () => ({ sessionId }) },
        parse: parseSessionSnapshotResponse,
        matchIdentity: (response) => { signal?.throwIfAborted(); if (response.session_id !== sessionId) throw new CockpitClientError("malformed_response", "Session snapshot belongs to another session"); return response; },
      };
    },
  },
  spaceGitStatus: { direct: "both",
    encode(backend, sessionId, signal) {
      validateSessionId(sessionId); signal?.throwIfAborted();
      return {
        operation: "Space Git status",
        http: { method: "GET", route: () => `/api/v1/sessions/${encodeURIComponent(sessionId)}/space-git`, signal: () => signal },
        tauri: { command: "cockpit_space_git_status", args: () => ({ sessionId }) },
        parse: parseSpaceGitStatusResponse,
        matchIdentity: (response) => { if (backend === "native") signal?.throwIfAborted(); if (response.session_id !== sessionId) throw new CockpitClientError("malformed_response", "Space Git status belongs to another session"); return response; },
      };
    },
  },
  spaceGitAction: { direct: "both",
    encode(_backend, sessionId, actionRequest) {
      validateSessionId(sessionId);
      let submitted: SpaceGitActionRequest;
      const submit = () => { submitted = { ...actionRequest }; return submitted; };
      return {
        operation: "Space Git action",
        http: {
          method: "POST",
          route: () => { submit(); return `/api/v1/sessions/${encodeURIComponent(sessionId)}/space-git/actions`; },
          json: () => submitted,
        },
        tauri: { command: "cockpit_space_git_action", args: () => ({ sessionId, request: submit() }) },
        parse: (value) => matchSpaceGitActionResponse(value, sessionId, submitted),
      };
    },
  },
  focus: { direct: "both",
    encode(_backend, sessionId, focusRequest) {
      validateSessionId(sessionId); let parsed: FocusRequest; parsed = parseFocusRequest(focusRequest);
      return {
        operation: "focus",
        http: { method: "POST", route: () => `/api/v1/sessions/${encodeURIComponent(sessionId)}/focus`, json: () => parsed },
        tauri: { command: "cockpit_focus", args: () => ({ sessionId, request: parsed }) },
        parse: parseFocusResponse,
        matchIdentity: (response) => { if (response.session_id !== sessionId) throw new CockpitClientError("malformed_response", "Focus response belongs to another session"); return response; },
      };
    },
  },
  mutate: { direct: "both",
    encode(_backend, sessionId, mutationRequest) {
      validateSessionId(sessionId); let parsed: ResourceMutationRequest; parsed = parseResourceMutationRequest(mutationRequest);
      return {
        operation: "mutation",
        http: { method: "POST", route: () => `/api/v1/sessions/${encodeURIComponent(sessionId)}/mutations`, json: () => parsed },
        tauri: { command: "cockpit_mutate", args: () => ({ sessionId, request: parsed }) },
        parse: parseResourceMutationResponse,
        matchIdentity: (response) => { if (response.session_id !== sessionId || response.snapshot.session_id !== sessionId) { throw new CockpitClientError("malformed_response", "Mutation response belongs to another session"); } return response; },
      };
    },
  },
  widgetContent: { nativeAbort: "widget",
    encode(_backend, value, signal) {
      const parsed = parseWidgetContentRequest(value);
      return {
        operation: "widget content",
        http: { method: "POST", route: () => "/api/v1/widgets/content", json: () => parsed, signal: () => signal },
        tauri: { command: "cockpit_widget_content", args: () => ({ request: parsed }) },
        parse: (body) => matchWidgetContent(body, parsed),
      };
    },
  },
  widgetRemove: { nativeAbort: "widget",
    encode(_backend, value, signal) {
      const parsed = parseWidgetRemoveRequest(value);
      return {
        operation: "widget remove",
        http: { method: "POST", route: () => "/api/v1/widgets/remove", json: () => parsed, signal: () => signal },
        tauri: { command: "cockpit_widget_remove", args: () => ({ request: parsed }) },
        parse: parseWidgetRemoveResponse,
      };
    },
  },
  widgetSelect: { nativeAbort: "widget",
    encode(_backend, value, signal) {
      const parsed = parseWidgetSelectRequest(value);
      return {
        operation: "widget select",
        http: { method: "POST", route: () => "/api/v1/widgets/select", json: () => parsed, signal: () => signal },
        tauri: { command: "cockpit_widget_select", args: () => ({ request: parsed }) },
        parse: parseWidgetSelectResponse,
      };
    },
  },
};

function snapshotAbortable<R>(pending: Promise<R>, signal?: AbortSignal): Promise<R> {
  if (!signal) return pending;
  return new Promise<R>((resolve, reject) => {
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
}

export function bindOperations(backend: Backend, transport: OperationTransport): RequestOperations {
  const client = {} as RequestOperations;
  function bind<K extends OperationName>(name: K): CockpitClient[K] {
    const row = operations[name];
    const method = (...args: Parameters<CockpitClient[K]>) => {
      const run = () => {
        let encoded: EncodedOperation<Awaited<ReturnType<CockpitClient[K]>>>;
        try { encoded = row.encode(backend, ...args); }
        catch (error) {
          if (backend === "native" && row.throwNativeValidation) throw error;
          return Promise.reject(error);
        }
        const dispatch = () => {
          let pending = transport(encoded);
          if (encoded.matchIdentity) pending = pending.then(encoded.matchIdentity);
          if (backend === "native" && row.nativeAbort === "snapshot") {
            pending = snapshotAbortable(pending, args[1] as AbortSignal | undefined);
          }
          return pending;
        };
        if (row.direct === "both" || (backend === "native" && row.direct === "native")) return dispatch();
        try { return dispatch(); } catch (error) { return Promise.reject(error); }
      };
      if (backend === "native" && row.nativeAbort === "widget") {
        return widgetAbortable(run, args[1] as AbortSignal | undefined);
      }
      return run();
    };
    // The mapped table checks each row's arguments/result; only iteration erases K.
    return method as CockpitClient[K];
  }
  for (const name of Object.keys(operations) as OperationName[]) client[name] = bind(name) as never;
  return client;
}
