import type { ContextMedia, ContextMediaRequest } from "../protocol/generated/v1";
import type { SourceImportRequest, SourceRefreshRequest, SourceImportResponse } from "../protocol/generated/v1";
import type { ReviewLaunchRequest, ReviewSnapshotRequest, ReviewSnapshot, ReviewFileRequest, ReviewFileDiff } from "../protocol/generated/v1";
import type { ContextSnapshotRequest, ContextSnapshotResponse } from "../protocol/generated/v1";
import type { CommentPastePrepareRequest, CommentPastePrepareResponse, CommentPasteReceipt, CommentPasteMarkPastedRequest, CommentPasteSendRequest } from "../protocol/generated/v1";
import type {
  AgentSummary,
  BrowserAction,
  BrowserAssociation,
  BrowserConnectionState,
  BrowserRequest,
  BrowserResponse,
  BrowserTarget,
  BrowserFeedbackRequest,
  BrowserFeedbackAckRequest,
  BrowserFeedbackLookup,
  BrowserFeedbackImageRequest,
  BrowserFeedbackImage,
  BrowserFeedbackSendRequest,
  BrowserFeedbackSendResponse,
  BrowserFeedbackAck,
  BrowserFeedbackCapture,
  BrowserFeedbackResponse,
  BrowserAnnotation,
  BrowserElementEvidence,
  BrowserPageEvidence,
  BrowserCaptureContext,
  BrowserPoint,
  BrowserRect,
  CommentPasteState,
  CommentPasteTarget,
  CockpitCapabilities,
  ErrorResponse,
  FocusRequest,
  FocusResponse,
  LayoutPane,
  LayoutRect,
  PaneMoveDestination,
  ResourceMutationRequest,
  ResourceMutationResponse,
  PaneSummary,
  SessionListResponse,
  SessionSnapshotResponse,
  SessionStreamMessage,
  SessionSummary,
  SpaceGitSummary,
  SpaceSummary,
  TabLayout,
  TabSummary,
  StatusResponse,
  TerminalCommand,
  TerminalOpenRequest,
  TerminalOwnershipState,
  TerminalStreamMessage,
  ProjectConfiguration,
  RepositoryListResponse,
  WorkspaceSetupRequest,
  WorkspaceSetupPlan,
  WorkspaceOperationRequest,
  WorkspaceOperation,
  WorkspaceReconcileRequest,
  WorkspaceTeardownExecuteRequest,
  WorkspaceTeardownPreview,
  WorkspaceTeardownPreviewRequest,
  WorkspaceTeardownRecoveryList,
  WorkspaceTeardownResult,
  PanePresentation,
  ContextDirectoryRequest,
  ContextDirectory,
  ContextDocumentRequest,
  ContextDocument,
  ContextLaunchRequest,
  ContextSearchRequest,
  ContextSearchResponse,
  ContextInvalidationRequest,
  ContextInvalidationResponse,
  CommentBatch,
  CommentBatchList,
  CommentBatchMutation,
  CommentBatchRequest,
  CommentPreview,
  CommentPreviewRequest,
  CommentRemoveRequest,
  CommentRequestScope,
  CommentUpsertRequest,
} from "../protocol/generated/v1";

export type CockpitStatus = StatusResponse;
export type CockpitHerdrIdentity = Extract<
  StatusResponse["herdr"],
  { status: "compatible" }
>["identity"];
export type CockpitSessionSnapshot = SessionSnapshotResponse;
export type {
  AgentSummary,
  FocusRequest,
  PaneMoveDestination,
  ResourceMutationRequest,
  ResourceMutationResponse,
  FocusResponse,
  SessionListResponse,
  SessionSnapshotResponse,
  SessionStreamMessage,
  SessionSummary,
  TabSummary,
  TerminalCommand,
  TerminalOpenRequest,
  TerminalStreamMessage,
};

export interface ClosableStream {
  close(): void;
}

export interface TerminalStream extends ClosableStream {
  send(command: TerminalCommand): void;
}

export interface CockpitClient {
  status(): Promise<StatusResponse>;
  browserAction(request: BrowserRequest): Promise<BrowserResponse>;
  browserFeedback(request: BrowserFeedbackRequest): Promise<BrowserFeedbackLookup>;
  acknowledgeBrowserFeedback(request: BrowserFeedbackAckRequest): Promise<BrowserFeedbackAck>;
  browserFeedbackImage(request: BrowserFeedbackImageRequest): Promise<BrowserFeedbackImage>;
  sendBrowserFeedback(request: BrowserFeedbackSendRequest): Promise<BrowserFeedbackSendResponse>;
  projectConfiguration(): Promise<ProjectConfiguration>;
  repositories(): Promise<RepositoryListResponse>;
  planWorkspace(sessionId: string, request: WorkspaceSetupRequest): Promise<WorkspaceSetupPlan>;
  startWorkspace(sessionId: string, request: WorkspaceOperationRequest): Promise<WorkspaceOperation>;
  workspaceOperation(sessionId: string, operationId: string): Promise<WorkspaceOperation>;
  resumeWorkspace(sessionId: string, request: WorkspaceOperationRequest): Promise<WorkspaceOperation>;
  cancelWorkspace(sessionId: string, request: WorkspaceOperationRequest): Promise<WorkspaceOperation>;
  reconcileWorkspace(sessionId: string, request: WorkspaceReconcileRequest): Promise<WorkspaceOperation>;
  workspaceTeardownPreview(sessionId: string, request: WorkspaceTeardownPreviewRequest): Promise<WorkspaceTeardownPreview>;
  workspaceTeardownExecute(sessionId: string, request: WorkspaceTeardownExecuteRequest): Promise<WorkspaceTeardownResult>;
  workspaceTeardownRecoveries(sessionId: string): Promise<WorkspaceTeardownRecoveryList>;
  inspectPane(sessionId: string, paneId: string, signal?: AbortSignal): Promise<PanePresentation>;
  contextDirectory(sessionId: string, paneId: string, request: ContextDirectoryRequest, signal?: AbortSignal): Promise<ContextDirectory>;
  contextDocument(sessionId: string, paneId: string, request: ContextDocumentRequest, signal?: AbortSignal): Promise<ContextDocument>;
  reviewSnapshot(sessionId: string, paneId: string, request: ReviewSnapshotRequest, signal?: AbortSignal): Promise<ReviewSnapshot>;
  reviewFile(sessionId: string, paneId: string, request: ReviewFileRequest, signal?: AbortSignal): Promise<ReviewFileDiff>;
  contextSnapshot(sessionId: string, paneId: string, request: ContextSnapshotRequest): Promise<ContextSnapshotResponse>;
  contextSearch(sessionId: string, paneId: string, request: ContextSearchRequest, signal?: AbortSignal): Promise<ContextSearchResponse>;
  contextInvalidate(sessionId: string, paneId: string, request: ContextInvalidationRequest, signal?: AbortSignal): Promise<ContextInvalidationResponse>;
  contextMedia(sessionId: string, paneId: string, request: ContextMediaRequest, signal?: AbortSignal): Promise<ContextMedia>;
  sourceImport(sessionId: string, paneId: string, request: SourceImportRequest, signal?: AbortSignal): Promise<SourceImportResponse>;
  sourceRefresh(sessionId: string, paneId: string, request: SourceRefreshRequest, signal?: AbortSignal): Promise<SourceImportResponse>;
  sourceList(sessionId: string, paneId: string, request: { binding_id: string; root_id: string }, signal?: AbortSignal): Promise<SourceImportResponse>;
  openReview(sessionId: string, request: ReviewLaunchRequest): Promise<PanePresentation>;
  openContext(sessionId: string, request: ContextLaunchRequest): Promise<PanePresentation>;
  commentBatches(sessionId: string, paneId: string, request: CommentRequestScope, signal?: AbortSignal): Promise<CommentBatchList>;
  commentBatch(sessionId: string, paneId: string, request: CommentBatchRequest, signal?: AbortSignal): Promise<CommentBatch>;
  commentUpsert(sessionId: string, paneId: string, request: CommentUpsertRequest, signal?: AbortSignal): Promise<CommentBatch>;
  commentRemove(sessionId: string, paneId: string, request: CommentRemoveRequest, signal?: AbortSignal): Promise<CommentBatch>;
  commentDiscard(sessionId: string, paneId: string, request: CommentBatchMutation, signal?: AbortSignal): Promise<CommentBatchList>;
  commentAttach(sessionId: string, paneId: string, request: CommentBatchMutation, signal?: AbortSignal): Promise<CommentBatch>;
  commentPastePrepare(sessionId: string, paneId: string, request: CommentPastePrepareRequest, signal?: AbortSignal): Promise<CommentPastePrepareResponse>;
  commentPasteMarkPasted(sessionId: string, paneId: string, request: CommentPasteMarkPastedRequest): Promise<CommentPasteReceipt>;
  commentPasteSend(sessionId: string, paneId: string, request: CommentPasteSendRequest): Promise<CommentPasteReceipt>;
  commentPreview(sessionId: string, paneId: string, request: CommentPreviewRequest, signal?: AbortSignal): Promise<CommentPreview>;
  sessions(): Promise<SessionListResponse>;
  sessionSnapshot(sessionId: string): Promise<CockpitSessionSnapshot>;
  focus(sessionId: string, request: FocusRequest): Promise<FocusResponse>;
  mutate(
    sessionId: string,
    request: ResourceMutationRequest,
  ): Promise<ResourceMutationResponse>;
  subscribeSession(
    sessionId: string,
    onMessage: (message: SessionStreamMessage) => void,
    onError: (error: CockpitClientError) => void,
  ): Promise<ClosableStream>;
  openTerminal(
    request: TerminalOpenRequest,
    onMessage: (message: TerminalStreamMessage) => void,
    onError: (error: CockpitClientError) => void,
    signal?: AbortSignal,
  ): Promise<TerminalStream>;
}

export type CockpitClientErrorCode =
  | "transport_error"
  | "http_error"
  | "malformed_response"
  | "native_error"
  | "stream_error";

export interface CockpitClientErrorOptions {
  status?: number;
  cause?: unknown;
  operationCode?: string;
}

/** A stable, transport-independent error returned by a client adapter. */
export class CockpitClientError extends Error {
  readonly code: CockpitClientErrorCode;
  readonly status?: number;
  readonly cause?: unknown;
  readonly operationCode?: string;

  constructor(
    code: CockpitClientErrorCode,
    message: string,
    options: CockpitClientErrorOptions = {},
  ) {
    super(message);
    this.name = "CockpitClientError";
    this.code = code;
    this.status = options.status;
    this.cause = options.cause;
    this.operationCode = options.operationCode;
  }
}

function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === "object" && value !== null;
}

export function parseErrorEnvelope(value: unknown): ErrorResponse | undefined {
  if (
    !isRecord(value) ||
    typeof value.code !== "string" ||
    typeof value.message !== "string"
  ) {
    return undefined;
  }
  return { code: value.code, message: value.message };
}

function malformed(message: string): never {
  throw new CockpitClientError("malformed_response", message);
}

function isInteger(value: unknown, max: number): value is number {
  return (
    typeof value === "number" &&
    Number.isFinite(value) &&
    Number.isSafeInteger(value) &&
    value >= 0 &&
    value <= max
  );
}

const isU8 = (value: unknown): value is number => isInteger(value, 0xff);
const isU16 = (value: unknown): value is number => isInteger(value, 0xffff);
const isU32 = (value: unknown): value is number => isInteger(value, 0xffffffff);
const isU64 = (value: unknown): value is number => isInteger(value, Number.MAX_SAFE_INTEGER);
const isString = (value: unknown): value is string => typeof value === "string";
const isNullableString = (value: unknown): value is string | null =>
  value === null || isString(value);
const isBoolean = (value: unknown): value is boolean => typeof value === "boolean";

function isIdentity(value: unknown): value is CockpitHerdrIdentity {
  return (
    isRecord(value) &&
    isString(value.version) &&
    isU32(value.protocol) &&
    isU32(value.schema_version)
  );
}

function isSpaceGitSummary(value: unknown): value is SpaceGitSummary {
  return (
    isRecord(value) &&
    isString(value.repository_key) &&
    isString(value.repository) &&
    isNullableString(value.branch) &&
    isString(value.checkout_path) &&
    isBoolean(value.is_linked_worktree)
  );
}

function isSpaceSummary(value: unknown): value is SpaceSummary {
  return (
    isRecord(value) &&
    isString(value.id) &&
    isString(value.label) &&
    isU32(value.number) &&
    isU32(value.tab_count) &&
    isU32(value.pane_count) &&
    isBoolean(value.focused) &&
    isString(value.agent_status) &&
    (value.git === null || isSpaceGitSummary(value.git))
  );
}
function isTabSummary(value: unknown): value is TabSummary {
  return (
    isRecord(value) &&
    isString(value.id) &&
    isString(value.space_id) &&
    isString(value.label) &&
    isU32(value.number) &&
    isU32(value.pane_count) &&
    isBoolean(value.focused)
  );
}
function isPaneSummary(value: unknown): value is PaneSummary {
  return (
    isRecord(value) &&
    isString(value.id) &&
    isString(value.terminal_id) &&
    isString(value.space_id) &&
    isString(value.tab_id) &&
    isNullableString(value.title) &&
    isBoolean(value.focused) &&
    isNullableString(value.agent) &&
    isString(value.agent_status) &&
    isU64(value.revision)
  );
}
function isLayoutRect(value: unknown): value is LayoutRect {
  return (
    isRecord(value) &&
    isU32(value.x) &&
    isU32(value.y) &&
    isU32(value.width) &&
    isU32(value.height)
  );
}
function isLayoutPane(value: unknown): value is LayoutPane {
  return isRecord(value) && isString(value.pane_id) && isBoolean(value.focused) && isLayoutRect(value.rect);
}
function isTabLayout(value: unknown): value is TabLayout {
  return (
    isRecord(value) &&
    isString(value.space_id) &&
    isString(value.tab_id) &&
    isLayoutRect(value.area) &&
    isNullableString(value.focused_pane_id) &&
    Array.isArray(value.panes) &&
    value.panes.every(isLayoutPane) &&
    isBoolean(value.zoomed)
  );
}
type AgentSummaryWire = Omit<AgentSummary, "state_change_seq"> & { state_change_seq?: number };
function isAgentSummary(value: unknown): value is AgentSummaryWire {
  return (
    isRecord(value) &&
    isString(value.pane_id) &&
    isString(value.space_id) &&
    isString(value.tab_id) &&
    isString(value.name) &&
    isString(value.status) &&
    isNullableString(value.title) &&
    isBoolean(value.focused) &&
    (value.state_change_seq === undefined || isU64(value.state_change_seq))
  );
}
function parseHerdrCompatibility(value: unknown): StatusResponse["herdr"] {
  if (!isRecord(value) || !isString(value.status)) return malformed("Status response is missing required fields");
  if (value.status === "compatible") {
    if (!isIdentity(value.identity)) return malformed("Compatible Herdr status has no valid identity");
    return { status: "compatible", identity: value.identity };
  }
  if (value.status === "incompatible") {
    if (!isString(value.code) || !isString(value.message) || !(value.identity === null || isIdentity(value.identity))) {
      return malformed("Incompatible Herdr status is missing required fields");
    }
    return { status: "incompatible", identity: value.identity, code: value.code, message: value.message };
  }
  if (value.status === "unavailable") {
    if (!isString(value.code) || !isString(value.message)) return malformed("Unavailable Herdr status is missing required fields");
    return { status: "unavailable", code: value.code, message: value.message };
  }
  return malformed(`Unknown Herdr status: ${value.status}`);
}

/** Validate untrusted transport data before it crosses the client seam. */
export function parseStatusResponse(value: unknown): StatusResponse {
  if (!isRecord(value) || !isString(value.protocol_version) || !isString(value.cockpit_version)) {
    return malformed("Status response is missing required fields");
  }
  const hasCapabilities = Object.prototype.hasOwnProperty.call(value, "capabilities");
  const capabilities: CockpitCapabilities = !hasCapabilities
    ? { terminal_mouse_input: false }
    : isRecord(value.capabilities) && isBoolean(value.capabilities.terminal_mouse_input)
      ? { terminal_mouse_input: value.capabilities.terminal_mouse_input }
      : malformed("Status response capabilities are invalid");
  const herdr = parseHerdrCompatibility(value.herdr);
  if (value.mode !== "normal" && value.mode !== "test") return malformed("Status response is missing required fields");
  return {
    protocol_version: value.protocol_version,
    cockpit_version: value.cockpit_version,
    mode: value.mode,
    capabilities,
    herdr,
  };
}
const browserConnections: readonly BrowserConnectionState[] = ["absent", "open", "closed", "disconnected", "outcome_unknown"];
function isBrowserConnection(value: unknown): value is BrowserConnectionState {
  return isString(value) && browserConnections.includes(value as BrowserConnectionState);
}
function parseBrowserTarget(value: unknown): BrowserTarget {
  if (!isRecord(value) || !isString(value.session_id) || value.session_id.length === 0
    || !(value.space_id === null || isString(value.space_id))
    || !(value.pane_id === null || isString(value.pane_id))
    || !(value.endpoint_path === null || isString(value.endpoint_path))
    || (value.space_id === null) === (value.pane_id === null)) {
    return malformed("Browser target is malformed");
  }
  return value as BrowserTarget;
}
function parseBrowserAction(value: unknown): BrowserAction {
  if (!isRecord(value) || !isString(value.kind)) return malformed("Browser action is malformed");
  if (value.kind === "open") {
    if (!(value.url === null || isString(value.url))) return malformed("Browser open URL is malformed");
    return value as BrowserAction;
  }
  if (value.kind === "status" || value.kind === "show" || value.kind === "close") return value as BrowserAction;
  return malformed("Unknown browser action");
}
function parseBrowserAssociation(value: unknown): BrowserAssociation {
  if (!isRecord(value) || !isString(value.association_key) || !isString(value.owner_id)
    || !isString(value.session_id) || !isString(value.space_id) || !isString(value.space_label)
    || !isString(value.playwright_session) || !isString(value.working_directory)
    || !isString(value.profile_path) || !isString(value.invocation) || !isBrowserConnection(value.connection)
    || !(value.incarnation === null || isString(value.incarnation))
    || !(value.opened_tab === null || isString(value.opened_tab))) {
    return malformed("Browser association is malformed");
  }
  return value as BrowserAssociation;
}
export function parseBrowserRequest(value: unknown): BrowserRequest {
  if (!isRecord(value)) return malformed("Browser request is malformed");
  return { target: parseBrowserTarget(value.target), action: parseBrowserAction(value.action) };
}
export function parseBrowserResponse(value: unknown): BrowserResponse {
  if (!isRecord(value) || !(value.association === null || isRecord(value.association))
    || !isBrowserConnection(value.connection) || !isString(value.message)) return malformed("Browser response is malformed");
  return { association: value.association === null ? null : parseBrowserAssociation(value.association), connection: value.connection, message: value.message };
}
function parseFeedbackPoint(value: unknown): BrowserPoint {
  if (!isRecord(value) || typeof value.x !== "number" || !Number.isFinite(value.x) || typeof value.y !== "number" || !Number.isFinite(value.y)) {
    return malformed("Browser feedback point is malformed");
  }
  return { x: value.x, y: value.y };
}
function parseFeedbackRect(value: unknown): BrowserRect {
  if (!isRecord(value) || typeof value.x !== "number" || !Number.isFinite(value.x) || typeof value.y !== "number" || !Number.isFinite(value.y)
    || typeof value.width !== "number" || !Number.isFinite(value.width) || value.width < 0
    || typeof value.height !== "number" || !Number.isFinite(value.height) || value.height < 0) {
    return malformed("Browser feedback bounds are malformed");
  }
  return { x: value.x, y: value.y, width: value.width, height: value.height };
}
function parseFeedbackElement(value: unknown): BrowserElementEvidence {
  if (!isRecord(value) || !isString(value.tag) || !isString(value.text)
    || !(value.role === null || isString(value.role)) || !(value.name === null || isString(value.name))
    || !Array.isArray(value.locators) || value.locators.length > 256 || !value.locators.every(isString)
    || !isString(value.excerpt)) return malformed("Browser element evidence is malformed");
  return value as BrowserElementEvidence;
}
function parseFeedbackPage(value: unknown): BrowserPageEvidence {
  if (!isRecord(value) || !isString(value.url) || !isString(value.title)
    || !isU64(value.tab_id) || !isString(value.document_id) || !isString(value.captured_at)
    || !isRecord(value.viewport) || typeof value.viewport.width !== "number" || !Number.isFinite(value.viewport.width)
    || typeof value.viewport.height !== "number" || !Number.isFinite(value.viewport.height)
    || typeof value.viewport.scroll_x !== "number" || !Number.isFinite(value.viewport.scroll_x)
    || typeof value.viewport.scroll_y !== "number" || !Number.isFinite(value.viewport.scroll_y)
    || typeof value.viewport.device_pixel_ratio !== "number" || !Number.isFinite(value.viewport.device_pixel_ratio)
    || typeof value.viewport.visual_scale !== "number" || !Number.isFinite(value.viewport.visual_scale)
    || !isU32(value.image_width) || !isU32(value.image_height)) return malformed("Browser page evidence is malformed");
  return {
    url: value.url, title: value.title, tab_id: value.tab_id as BrowserPageEvidence["tab_id"], document_id: value.document_id, captured_at: value.captured_at,
    viewport: value.viewport as BrowserPageEvidence["viewport"], image_width: value.image_width, image_height: value.image_height,
  };
}
function parseFeedbackAnnotation(value: unknown): BrowserAnnotation {
  if (!isRecord(value) || !isString(value.id) || !isString(value.kind) || !["freehand", "element", "region"].includes(value.kind)
    || !isString(value.comment) || !isString(value.color) || !Array.isArray(value.points) || value.points.length > 10000
    || !(value.bounds === null || isRecord(value.bounds)) || !(value.element === null || isRecord(value.element))) {
    return malformed("Browser annotation is malformed");
  }
  return {
    id: value.id,
    kind: value.kind as BrowserAnnotation["kind"],
    comment: value.comment,
    color: value.color,
    points: value.points.map(parseFeedbackPoint),
    bounds: value.bounds === null ? null : parseFeedbackRect(value.bounds),
    element: value.element === null ? null : parseFeedbackElement(value.element),
  };
}
function parseFeedbackContext(value: unknown): BrowserCaptureContext {
  if (!isRecord(value) || !isString(value.association_key) || !isString(value.session_id)
    || !isString(value.space_id) || !isString(value.space_label) || !isString(value.playwright_session)
    || !isString(value.working_directory) || !isString(value.invocation) || !isString(value.browser_instance)) {
    return malformed("Browser capture context is malformed");
  }
  return value as BrowserCaptureContext;
}
function parseFeedbackCapture(value: unknown): BrowserFeedbackCapture {
  if (!isRecord(value) || !isString(value.id) || !isRecord(value.context) || !isRecord(value.page)
    || !Array.isArray(value.annotations) || value.annotations.length > 10000 || !Array.isArray(value.pending_ids)
    || value.pending_ids.length > 10000 || !value.pending_ids.every(isString) || !isString(value.image_path)) {
    return malformed("Browser feedback capture is malformed");
  }
  return {
    id: value.id,
    context: parseFeedbackContext(value.context),
    page: parseFeedbackPage(value.page),
    annotations: value.annotations.map(parseFeedbackAnnotation),
    pending_ids: value.pending_ids,
    image_path: value.image_path,
  };
}
function parseBrowserFeedbackResponse(value: unknown): BrowserFeedbackResponse {
  if (!isRecord(value) || !Array.isArray(value.captures) || value.captures.length > 256
    || !isU32(value.pending_count) || !isU64(value.retention_seconds)) return malformed("Browser feedback response is malformed");
  return { captures: value.captures.map(parseFeedbackCapture), pending_count: value.pending_count, retention_seconds: value.retention_seconds as BrowserFeedbackResponse["retention_seconds"] };
}
export function parseBrowserFeedbackRequest(value: unknown): BrowserFeedbackRequest {
  if (!isRecord(value)) return malformed("Browser feedback request is malformed");
  return { target: parseBrowserTarget(value.target) };
}
export function parseBrowserFeedbackAckRequest(value: unknown): BrowserFeedbackAckRequest {
  if (!isRecord(value) || !Array.isArray(value.ids) || value.ids.length > 10000 || !value.ids.every((id) => isString(id) && id.length > 0)) {
    return malformed("Browser feedback acknowledgement request is malformed");
  }
  return { target: parseBrowserTarget(value.target), ids: value.ids };
}
export function parseBrowserFeedbackImageRequest(value: unknown): BrowserFeedbackImageRequest {
  if (!isRecord(value) || !isString(value.capture_id) || value.capture_id.length === 0) {
    return malformed("Browser feedback image request is malformed");
  }
  return { target: parseBrowserTarget(value.target), capture_id: value.capture_id };
}
export function parseBrowserFeedbackSendRequest(value: unknown): BrowserFeedbackSendRequest {
  if (!isRecord(value) || !Array.isArray(value.ids) || value.ids.length > 10000
    || !value.ids.every((id) => isString(id) && id.length > 0) || !isString(value.operation_id)
    || value.operation_id.length === 0 || !isBoolean(value.acknowledge_duplicate_risk)) {
    return malformed("Browser feedback send request is malformed");
  }
  return { target: parseBrowserTarget(value.target), ids: value.ids, operation_id: value.operation_id, acknowledge_duplicate_risk: value.acknowledge_duplicate_risk };
}
export function parseBrowserFeedbackLookup(value: unknown): BrowserFeedbackLookup {
  if (!isRecord(value)) return malformed("Browser feedback lookup is malformed");
  return { browser: parseBrowserResponse(value.browser), feedback: parseBrowserFeedbackResponse(value.feedback) };
}
export function parseBrowserFeedbackImage(value: unknown): BrowserFeedbackImage {
  if (!isRecord(value) || !isString(value.mime_type) || value.mime_type.length === 0 || !isString(value.data_base64) || !isBase64(value.data_base64)) {
    return malformed("Browser feedback image is malformed");
  }
  return { mime_type: value.mime_type, data_base64: value.data_base64 };
}
function parseCommentPasteTargetForFeedback(value: unknown): CommentPasteTarget {
  if (!isRecord(value) || !isString(value.endpoint_identity) || !isString(value.session_id) || !isString(value.workspace_id)
    || !isString(value.tab_id) || !isString(value.pane_id) || !isString(value.terminal_id)
    || !isString(value.agent_fingerprint) || !isString(value.agent_label)) return malformed("Browser feedback target is malformed");
  return value as CommentPasteTarget;
}
export function parseBrowserFeedbackSendResponse(value: unknown): BrowserFeedbackSendResponse {
  if (!isRecord(value) || !isString(value.operation_id) || !isString(value.state)
    || !(["pending", "accepted", "rejected", "outcome_unknown"] as readonly string[]).includes(value.state)
    || !(value.target === null || isRecord(value.target)) || !Array.isArray(value.acknowledged_ids)
    || !value.acknowledged_ids.every((id) => isString(id)) || !isU32(value.pending_count) || !isString(value.message)) {
    return malformed("Browser feedback send response is malformed");
  }
  return {
    operation_id: value.operation_id,
    state: value.state as CommentPasteState,
    target: value.target === null ? null : parseCommentPasteTargetForFeedback(value.target),
    acknowledged_ids: value.acknowledged_ids,
    pending_count: value.pending_count,
    message: value.message,
  };
}
export function parseBrowserFeedbackAck(value: unknown): BrowserFeedbackAck {
  if (!isRecord(value) || !Array.isArray(value.acknowledged_ids) || !value.acknowledged_ids.every((id) => isString(id)) || !isU32(value.remaining)) {
    return malformed("Browser feedback acknowledgement response is malformed");
  }
  return { acknowledged_ids: value.acknowledged_ids, remaining: value.remaining };
}

export function parseSessionSnapshotResponse(value: unknown): SessionSnapshotResponse {
  if (
    !isRecord(value) || !isString(value.session_id) || !isString(value.version) || !isU32(value.protocol) ||
    !isNullableString(value.focused_space_id) || !isNullableString(value.focused_tab_id) || !isNullableString(value.focused_pane_id) ||
    !Array.isArray(value.spaces) || !value.spaces.every(isSpaceSummary) || !Array.isArray(value.tabs) || !value.tabs.every(isTabSummary) ||
    !Array.isArray(value.panes) || !value.panes.every(isPaneSummary) || !Array.isArray(value.layouts) || !value.layouts.every(isTabLayout) ||
    !Array.isArray(value.agents) || !value.agents.every(isAgentSummary)
  ) return malformed("Session snapshot response is missing required fields");
  return {
    session_id: value.session_id, version: value.version, protocol: value.protocol,
    focused_space_id: value.focused_space_id, focused_tab_id: value.focused_tab_id, focused_pane_id: value.focused_pane_id,
    spaces: value.spaces, tabs: value.tabs, panes: value.panes, layouts: value.layouts,
    agents: value.agents.map((agent) => ({ ...agent, state_change_seq: agent.state_change_seq ?? 0 })),
  };
}

export function parseSessionSummary(value: unknown): SessionSummary {
  if (!isRecord(value) || !isString(value.id) || !isString(value.label) || !isBoolean(value.is_default) || !isBoolean(value.running)) {
    return malformed("Session summary is missing required fields");
  }
  return { id: value.id, label: value.label, is_default: value.is_default, running: value.running };
}
export function parseSessionListResponse(value: unknown): SessionListResponse {
  if (!isRecord(value) || !Array.isArray(value.sessions) || !value.sessions.every((s) => {
    try { parseSessionSummary(s); return true; } catch { return false; }
  })) return malformed("Session list response is missing required fields");
  return { sessions: value.sessions.map(parseSessionSummary) };
}

const focusKinds = ["space", "tab", "pane", "agent"] as const;
function isFocusKind(value: unknown): value is FocusRequest["kind"] {
  return typeof value === "string" && (focusKinds as readonly string[]).includes(value);
}
export function parseFocusRequest(value: unknown): FocusRequest {
  if (!isRecord(value) || !isFocusKind(value.kind) || !isString(value.target_id)) return malformed("Focus request is missing required fields");
  return { kind: value.kind, target_id: value.target_id };
}
export function parseFocusResponse(value: unknown): FocusResponse {
  if (!isRecord(value) || !isString(value.session_id) || !isFocusKind(value.kind) || !isString(value.target_id) || !isBoolean(value.accepted)) {
    return malformed("Focus response is missing required fields");
  }
  return { session_id: value.session_id, kind: value.kind, target_id: value.target_id, accepted: value.accepted };
}

const paneSplitDirections = ["right", "down"] as const;
const paneResizeDirections = ["left", "right", "up", "down"] as const;
const paneZoomModes = ["toggle", "on", "off"] as const;
function isOneOf<T extends string>(value: unknown, choices: readonly T[]): value is T {
  return typeof value === "string" && choices.includes(value as T);
}
function isNullableRatio(value: unknown): value is number | null {
  return value === null || (typeof value === "number" && Number.isFinite(value) && value > 0 && value < 1);
}
function parsePaneMoveDestination(value: unknown): PaneMoveDestination {
  if (!isRecord(value) || !isString(value.type)) return malformed("Pane move destination is malformed");
  if (
    value.type === "existing_tab" &&
    isString(value.tab_id) &&
    isOneOf(value.direction, paneSplitDirections) &&
    isNullableString(value.target_pane_id) &&
    isNullableRatio(value.ratio)
  ) {
    return {
      type: "existing_tab",
      tab_id: value.tab_id,
      direction: value.direction,
      target_pane_id: value.target_pane_id,
      ratio: value.ratio,
    };
  }
  if (value.type === "new_tab" && isNullableString(value.space_id) && isNullableString(value.label)) {
    return { type: "new_tab", space_id: value.space_id, label: value.label };
  }
  if (value.type === "new_space" && isNullableString(value.label) && isNullableString(value.tab_label)) {
    return { type: "new_space", label: value.label, tab_label: value.tab_label };
  }
  return malformed(`Unknown or malformed pane move destination: ${value.type}`);
}

export function parseResourceMutationRequest(value: unknown): ResourceMutationRequest {
  if (!isRecord(value) || !isString(value.type)) return malformed("Resource mutation request is malformed");
  switch (value.type) {
    case "space_create":
      if (isNullableString(value.cwd) && isNullableString(value.label)) return { type: value.type, cwd: value.cwd, label: value.label };
      break;
    case "space_rename":
      if (isString(value.space_id) && isString(value.label)) return { type: value.type, space_id: value.space_id, label: value.label };
      break;
    case "space_move_block":
      if (Array.isArray(value.space_ids) && value.space_ids.every(isString) && isNullableString(value.before_space_id)) {
        return { type: value.type, space_ids: value.space_ids, before_space_id: value.before_space_id };
      }
      break;
    case "space_close":
      if (isString(value.space_id)) return { type: value.type, space_id: value.space_id };
      break;
    case "tab_create":
      if (isString(value.space_id) && isNullableString(value.label)) return { type: value.type, space_id: value.space_id, label: value.label };
      break;
    case "tab_rename":
      if (isString(value.tab_id) && isString(value.label)) return { type: value.type, tab_id: value.tab_id, label: value.label };
      break;
    case "tab_move":
      if (isString(value.tab_id) && isU32(value.insert_index)) return { type: value.type, tab_id: value.tab_id, insert_index: value.insert_index };
      break;
    case "tab_close":
      if (isString(value.tab_id)) return { type: value.type, tab_id: value.tab_id };
      break;
    case "pane_split":
      if (isString(value.pane_id) && isOneOf(value.direction, paneSplitDirections) && isNullableRatio(value.ratio)) {
        return { type: value.type, pane_id: value.pane_id, direction: value.direction, ratio: value.ratio };
      }
      break;
    case "pane_resize":
      if (isString(value.pane_id) && isOneOf(value.direction, paneResizeDirections) && typeof value.amount === "number" && Number.isFinite(value.amount) && value.amount > 0) {
        return { type: value.type, pane_id: value.pane_id, direction: value.direction, amount: value.amount };
      }
      break;
    case "pane_rename":
      if (isString(value.pane_id) && isNullableString(value.label)) return { type: value.type, pane_id: value.pane_id, label: value.label };
      break;
    case "pane_swap":
      if (isString(value.source_pane_id) && isString(value.target_pane_id)) {
        return { type: value.type, source_pane_id: value.source_pane_id, target_pane_id: value.target_pane_id };
      }
      break;
    case "pane_move":
      if (isString(value.pane_id)) return { type: value.type, pane_id: value.pane_id, destination: parsePaneMoveDestination(value.destination) };
      break;
    case "pane_zoom":
      if (isString(value.pane_id) && isOneOf(value.mode, paneZoomModes)) return { type: value.type, pane_id: value.pane_id, mode: value.mode };
      break;
    case "pane_close":
      if (isString(value.pane_id)) return { type: value.type, pane_id: value.pane_id };
      break;
    default:
      return malformed(`Unknown resource mutation type: ${value.type}`);
  }
  return malformed(`Resource mutation ${value.type} is missing required fields`);
}

export function parseResourceMutationResponse(value: unknown): ResourceMutationResponse {
  if (!isRecord(value) || !isString(value.session_id)) return malformed("Resource mutation response is missing required fields");
  const snapshot = parseSessionSnapshotResponse(value.snapshot);
  if (snapshot.session_id !== value.session_id) return malformed("Resource mutation snapshot belongs to another session");
  return { session_id: value.session_id, snapshot };
}

function isSessionStreamMessage(value: unknown): value is SessionStreamMessage {
  if (!isRecord(value) || !isString(value.type) || !isString(value.session_id) || !isU32(value.generation) || !isU32(value.sequence)) return false;
  if (value.type === "snapshot") {
    try {
      const nested = parseSessionSnapshotResponse(value.snapshot);
      return nested.session_id === value.session_id;
    } catch {
      return false;
    }
  }
  return (value.type === "stale" || value.type === "disconnected") && isString(value.code) && isString(value.message);
}
export function parseSessionStreamMessage(value: unknown): SessionStreamMessage {
  if (!isSessionStreamMessage(value)) return malformed("Session stream message is malformed");
  if (value.type === "snapshot") {
    const nested = parseSessionSnapshotResponse(value.snapshot);
    if (nested.session_id !== value.session_id) return malformed("Session stream snapshot belongs to another session");
    return { type: "snapshot", session_id: value.session_id, generation: value.generation, sequence: value.sequence, snapshot: nested };
  }
  return { type: value.type, session_id: value.session_id, generation: value.generation, sequence: value.sequence, code: value.code, message: value.message };
}

function isBase64(value: unknown): value is string {
  return isString(value) && value.length % 4 === 0 && /^(?:[A-Za-z0-9+/]{4})*(?:[A-Za-z0-9+/]{2}==|[A-Za-z0-9+/]{3}=)?$/.test(value);
}
const ownershipStates: readonly TerminalOwnershipState[] = ["pending", "observing", "owned", "conflict", "released", "lost"];
function isOwnershipState(value: unknown): value is TerminalOwnershipState {
  return typeof value === "string" && ownershipStates.includes(value as TerminalOwnershipState);
}
export function parseTerminalOpenRequest(value: unknown): TerminalOpenRequest {
  if (
    !isRecord(value) ||
    !isString(value.session_id) ||
    !isString(value.pane_id) ||
    (value.mode !== "observe" && value.mode !== "control") ||
    !isBoolean(value.takeover) ||
    !isU16(value.cols) ||
    !isU16(value.rows) ||
    value.cols === 0 ||
    value.rows === 0 ||
    !isU32(value.cell_width_px) ||
    !isU32(value.cell_height_px)
  ) {
    return malformed("Terminal open request is malformed");
  }
  try {
    validateSessionId(value.session_id);
    validateResourceId(value.pane_id);
  } catch {
    return malformed("Terminal open request has an invalid target");
  }
  return {
    session_id: value.session_id,
    pane_id: value.pane_id,
    mode: value.mode,
    takeover: value.takeover,
    cols: value.cols,
    rows: value.rows,
    cell_width_px: value.cell_width_px,
    cell_height_px: value.cell_height_px,
  };
}
export function parseTerminalCommand(value: unknown): TerminalCommand {
  if (!isRecord(value) || !isString(value.type)) return malformed("Terminal command is malformed");
  if (value.type === "terminal.input") {
    if (!((value.text === null || isString(value.text)) && (value.bytes === null || isBase64(value.bytes)) && (value.text !== null) !== (value.bytes !== null))) return malformed("Terminal input requires exactly one of text or bytes");
    const text = value.text as string | null;
    const bytes = value.bytes as string | null;
    return { type: "terminal.input", text, bytes };
  }
  if (value.type === "terminal.resize") {
    if (!isU16(value.cols) || !isU16(value.rows) || !isU32(value.cell_width_px) || !isU32(value.cell_height_px)) return malformed("Terminal resize command is malformed");
    return { type: "terminal.resize", cols: value.cols, rows: value.rows, cell_width_px: value.cell_width_px, cell_height_px: value.cell_height_px };
  }
  if (value.type === "terminal.scroll") {
    if ((value.direction !== "up" && value.direction !== "down") || !isU32(value.lines) || (value.source !== "wheel" && value.source !== "page_key") || !(value.column === null || isU16(value.column)) || !(value.row === null || isU16(value.row)) || !isU8(value.modifiers)) return malformed("Terminal scroll command is malformed");
    return { type: "terminal.scroll", direction: value.direction, lines: value.lines, source: value.source, column: value.column, row: value.row, modifiers: value.modifiers };
  }
  if (value.type === "terminal.mouse") {
    const kind = value.kind;
    const button = value.button;
    if ((kind !== "down" && kind !== "up" && kind !== "drag" && kind !== "moved") || !isU16(value.column) || !isU16(value.row) || !isU8(value.modifiers)) return malformed("Terminal mouse command is malformed");
    if (kind === "moved") {
      if (button !== null) return malformed("Terminal mouse command is malformed");
      return { type: "terminal.mouse", kind, button: null, column: value.column, row: value.row, modifiers: value.modifiers };
    }
    if (button !== "left" && button !== "right" && button !== "middle") return malformed("Terminal mouse command is malformed");
    return { type: "terminal.mouse", kind, button, column: value.column, row: value.row, modifiers: value.modifiers };
  }
  if (value.type === "terminal.release") return { type: "terminal.release" };
  return malformed("Terminal command has an unknown type");
}
export function parseTerminalStreamMessage(value: unknown): TerminalStreamMessage {
  if (!isRecord(value) || !isString(value.type) || !isString(value.session_id) || !isString(value.pane_id) || !isString(value.stream_id)) return malformed("Terminal stream message is malformed");
  if (value.type === "ownership") {
    if (!isOwnershipState(value.state) || !isNullableString(value.message)) return malformed("Terminal ownership message is malformed");
    return { type: "ownership", session_id: value.session_id, pane_id: value.pane_id, stream_id: value.stream_id, state: value.state, message: value.message };
  }
  if (value.type === "mouse_mode") {
    if (!isBoolean(value.enabled)) return malformed("Terminal mouse mode message is malformed");
    return { type: "mouse_mode", session_id: value.session_id, pane_id: value.pane_id, stream_id: value.stream_id, enabled: value.enabled };
  }
  if (value.type === "frame") {
    if (!isString(value.seq) || !/^[0-9]+$/.test(value.seq) || !isString(value.encoding) || value.encoding !== "ansi" || !isU16(value.width) || !isU16(value.height) || !isBoolean(value.full) || !isBase64(value.bytes)) return malformed("Terminal frame message is malformed");
    try { BigInt(value.seq); } catch { return malformed("Terminal frame sequence is malformed"); }
    return { type: "frame", session_id: value.session_id, pane_id: value.pane_id, stream_id: value.stream_id, seq: value.seq, encoding: value.encoding, width: value.width, height: value.height, full: value.full, bytes: value.bytes };
  }
  if (value.type === "closed") {
    if (!isString(value.reason)) return malformed("Terminal closed message is malformed");
    return { type: "closed", session_id: value.session_id, pane_id: value.pane_id, stream_id: value.stream_id, reason: value.reason };
  }
  if (value.type === "disconnected" || value.type === "error") {
    if (!isString(value.code) || !isString(value.message)) return malformed("Terminal failure message is malformed");
    return { type: value.type, session_id: value.session_id, pane_id: value.pane_id, stream_id: value.stream_id, code: value.code, message: value.message };
  }
  return malformed("Terminal stream message has an unknown type");
}

export function validateSessionId(sessionId: string): string {
  if (!isString(sessionId) || sessionId.length === 0 || sessionId.length > 96 || !/^[A-Za-z0-9_-]+$/.test(sessionId)) malformed("Session id must be a valid name");
  return sessionId;
}

export function validateResourceId(resourceId: string): string {
  if (!isString(resourceId) || resourceId.length === 0 || resourceId.length > 128 || !/^[A-Za-z0-9:_-]+$/.test(resourceId)) malformed("Resource id must be a valid Herdr identifier");
  return resourceId;
}
