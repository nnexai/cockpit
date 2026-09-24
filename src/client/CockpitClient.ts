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
  BrowserFeedbackDeliveryStatus,
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
  BrowserViewBlocker,
  BrowserViewBlockerKind,
  BrowserViewCapabilities,
  BrowserViewCapability,
  BrowserViewCaptureCommand,
  BrowserViewCaptureOutcome,
  BrowserViewClipboardCommand,
  BrowserViewCommand,
  BrowserViewCommandOutcome,
  BrowserViewCommandRequest,
  BrowserViewCommandResponse,
  BrowserViewCompositionInput,
  BrowserViewCompositionKind,
  BrowserViewControlState,
  BrowserViewControlStatus,
  BrowserViewCursor,
  BrowserViewCursorState,
  BrowserViewDialogCommand,
  BrowserViewDocumentCommandContext,
  BrowserViewDocumentState,
  BrowserViewDownloadCommand,
  BrowserViewDraftAnnotation,
  BrowserViewDraftCommand,
  BrowserViewDraftState,
  BrowserViewDraftInventory,
  BrowserDraftRecoveryAction,
  BrowserDraftRecoveryRequest,
  BrowserViewEvent,
  BrowserViewEventMetadata,
  BrowserViewFileCommand,
  BrowserViewFocusState,
  BrowserViewFrameDescriptor,
  BrowserViewFrameEnvelopeV2,
  BrowserViewFrameGrant,
  BrowserViewIdentity,
  BrowserViewInspectCommand,
  BrowserViewInspectResult,
  BrowserViewInspectionFreshness,
  BrowserViewKeyKind,
  BrowserViewKeyboardInput,
  BrowserViewLocation,
  BrowserViewNavigationCommand,
  BrowserViewNavigationState,
  BrowserViewOpenRequest,
  BrowserViewPermissionCommand,
  BrowserViewPermissionDecision,
  BrowserViewPointerButton,
  BrowserViewPointerInput,
  BrowserViewPointerKind,
  BrowserViewPresentation,
  BrowserViewSnapshot,
  BrowserViewTabCommand,
  BrowserViewTargetKind,
  BrowserViewTargetSummary,
  BrowserViewTextInput,
  BrowserViewViewportRequest,
  BrowserViewViewportState,
  BrowserViewWheelInput,
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
  WorkspaceDefaults, WorkspaceDefaultsRequest,
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

export type {
  BrowserViewBlocker,
  BrowserViewBlockerKind,
  BrowserViewCapabilities,
  BrowserViewCapability,
  BrowserViewCaptureCommand,
  BrowserViewCaptureOutcome,
  BrowserViewClipboardCommand,
  BrowserViewCommand,
  BrowserViewCommandOutcome,
  BrowserViewCommandRequest,
  BrowserViewCommandResponse,
  BrowserViewCompositionInput,
  BrowserViewCompositionKind,
  BrowserViewControlState,
  BrowserViewControlStatus,
  BrowserViewCursor,
  BrowserViewCursorState,
  BrowserViewDialogCommand,
  BrowserViewDocumentCommandContext,
  BrowserViewDocumentState,
  BrowserViewDownloadCommand,
  BrowserViewDraftAnnotation,
  BrowserViewDraftCommand,
  BrowserViewDraftState,
  BrowserDraftRecoveryAction,
  BrowserDraftRecoveryRequest,
  BrowserViewEvent,
  BrowserViewEventMetadata,
  BrowserViewFileCommand,
  BrowserViewFocusState,
  BrowserViewFrameDescriptor,
  BrowserViewFrameEnvelopeV2,
  BrowserViewFrameGrant,
  BrowserViewIdentity,
  BrowserViewInspectCommand,
  BrowserViewInspectResult,
  BrowserViewInspectionFreshness,
  BrowserViewKeyKind,
  BrowserViewKeyboardInput,
  BrowserViewLocation,
  BrowserViewNavigationCommand,
  BrowserViewNavigationState,
  BrowserViewOpenRequest,
  BrowserViewPermissionCommand,
  BrowserViewPermissionDecision,
  BrowserViewPointerButton,
  BrowserViewPointerInput,
  BrowserViewPointerKind,
  BrowserViewPresentation,
  BrowserViewSnapshot,
  BrowserViewTabCommand,
  BrowserViewTargetKind,
  BrowserViewTargetSummary,
  BrowserViewTextInput,
  BrowserViewViewportRequest,
  BrowserViewViewportState,
  BrowserViewWheelInput,
};
export interface BrowserViewFramePacket {
  readonly descriptor: BrowserViewFrameDescriptor;
  readonly jpeg: ArrayBuffer;
  /** Release the transport's delivery credit after the frame is presented. */
  ack(): void;
  /** Release the transport's delivery credit without presenting the frame. */
  discard(): void;
}

export interface BrowserViewStream extends ClosableStream {
  command(request: BrowserViewCommandRequest): Promise<BrowserViewCommandResponse>;
}

export type BrowserViewEventHandler = (event: BrowserViewEvent) => void;
export type BrowserViewFrameHandler = (packet: BrowserViewFramePacket) => void;


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
  browserDraftRecovery(request: BrowserDraftRecoveryRequest): Promise<BrowserViewCommandOutcome>;
  acknowledgeBrowserFeedback(request: BrowserFeedbackAckRequest): Promise<BrowserFeedbackAck>;
  browserFeedbackImage(request: BrowserFeedbackImageRequest): Promise<BrowserFeedbackImage>;
  sendBrowserFeedback(request: BrowserFeedbackSendRequest): Promise<BrowserFeedbackSendResponse>;
  projectConfiguration(): Promise<ProjectConfiguration>;
  resolveWorkspaceDefaults(request: WorkspaceDefaultsRequest): Promise<WorkspaceDefaults>;
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
  sessionSnapshot(sessionId: string, signal?: AbortSignal): Promise<CockpitSessionSnapshot>;
  focus(sessionId: string, request: FocusRequest): Promise<FocusResponse>;
  mutate(
    sessionId: string,
    request: ResourceMutationRequest,
  ): Promise<ResourceMutationResponse>;
  subscribeSession(
    sessionId: string,
    onMessage: (message: SessionStreamMessage) => void,
    onError: (error: CockpitClientError) => void,
    signal?: AbortSignal,
  ): Promise<ClosableStream>;
  openTerminal(
    request: TerminalOpenRequest,
    onMessage: (message: TerminalStreamMessage) => void,
    onError: (error: CockpitClientError) => void,
    signal?: AbortSignal,
  ): Promise<TerminalStream>;
  openBrowserView(
    request: BrowserViewOpenRequest,
    onEvent: BrowserViewEventHandler,
    onFrame: BrowserViewFrameHandler,
    onError: (error: CockpitClientError) => void,
    signal?: AbortSignal,
  ): Promise<BrowserViewStream>;
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
  if (value.kind === "status" || value.kind === "close") return value as BrowserAction;
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
const browserViewMaxId = 512;
const browserViewMaxText = 64 * 1024;
const browserViewMaxFileSelections = 64;
const browserViewMaxCaptureAnnotations = 64;
const browserViewMaxAnnotationPoints = 8192;
const browserViewMaxNoteTextCodeUnits = 4_000;
const browserViewMaxWidth = 2560;
const browserViewMaxHeight = 1600;
const browserViewMaxPixels = browserViewMaxWidth * browserViewMaxHeight;
const browserViewMaxJpegBytes = 6 * 1024 * 1024;
const isFiniteNumber = (value: unknown): value is number =>
  typeof value === "number" && Number.isFinite(value);
const isBoundedId = (value: unknown): value is string =>
  isString(value) && value.length > 0 && value.length <= browserViewMaxId;
const isBoundedText = (value: unknown): value is string =>
  isString(value) && value.length <= browserViewMaxText;
const parseNullable = <T>(value: unknown, parser: (value: unknown) => T): T | null =>
  value === null ? null : parser(value);
function parseBrowserViewEnum<T extends string>(value: unknown, values: readonly T[], label: string): T {
  if (!isOneOf(value, values)) return malformed(`${label} is malformed`);
  return value;
}

export function parseBrowserViewViewportRequest(value: unknown): BrowserViewViewportRequest {
  if (!isRecord(value) || !isU32(value.css_width) || value.css_width === 0 || value.css_width > browserViewMaxWidth
    || !isU32(value.css_height) || value.css_height === 0 || value.css_height > browserViewMaxHeight
    || !isFiniteNumber(value.device_pixel_ratio) || value.device_pixel_ratio <= 0) {
    return malformed("Browser view viewport request is malformed");
  }
  return { css_width: value.css_width, css_height: value.css_height, device_pixel_ratio: value.device_pixel_ratio };
}

export function parseBrowserViewOpenRequest(value: unknown): BrowserViewOpenRequest {
  if (!isRecord(value) || !isBoundedId(value.client_id)
    || (value.presentation !== "split" && value.presentation !== "browser_only")
    || !isBoolean(value.takeover)) return malformed("Browser view open request is malformed");
  return {
    target: parseBrowserTarget(value.target),
    client_id: value.client_id,
    presentation: value.presentation,
    viewport: parseBrowserViewViewportRequest(value.viewport),
    takeover: value.takeover,
  };
}

export function parseBrowserViewIdentity(value: unknown): BrowserViewIdentity {
  if (!isRecord(value) || !isBoundedId(value.association_key) || !isBoundedId(value.browser_incarnation)
    || !isBoundedId(value.view_id) || !isU64(value.stream_epoch)) return malformed("Browser view identity is malformed");
  return {
    association_key: value.association_key,
    browser_incarnation: value.browser_incarnation,
    view_id: value.view_id,
    stream_epoch: value.stream_epoch,
  };
}

export function parseBrowserViewTargetSummary(value: unknown): BrowserViewTargetSummary {
  if (!isRecord(value) || !isBoundedId(value.target_id) || !isString(value.title) || !isString(value.url)
    || !isU32(value.order) || !(value.opener_target_id === null || isBoundedId(value.opener_target_id))
    || !isBoolean(value.can_close)) return malformed("Browser view target summary is malformed");
  return {
    target_id: value.target_id,
    kind: parseBrowserViewEnum(value.kind, ["page", "popup", "background", "internal"] as const, "Browser view target kind"),
    title: value.title,
    url: value.url,
    order: value.order,
    opener_target_id: value.opener_target_id,
    can_close: value.can_close,
  };
}

export function parseBrowserViewDocumentState(value: unknown): BrowserViewDocumentState {
  if (!isRecord(value) || !isBoundedId(value.target_id) || !isBoundedId(value.frame_id)
    || !isU64(value.document_generation) || !isU64(value.frame_generation)) {
    return malformed("Browser view document state is malformed");
  }
  return {
    target_id: value.target_id,
    frame_id: value.frame_id,
    document_generation: value.document_generation,
    frame_generation: value.frame_generation,
  };
}

export function parseBrowserViewViewportState(value: unknown): BrowserViewViewportState {
  if (!isRecord(value) || !isU64(value.viewport_revision)
    || !isFiniteNumber(value.css_width) || value.css_width <= 0
    || !isFiniteNumber(value.css_height) || value.css_height <= 0
    || !isFiniteNumber(value.visual_offset_x) || !isFiniteNumber(value.visual_offset_y)
    || !isFiniteNumber(value.scroll_x) || !isFiniteNumber(value.scroll_y)
    || !isFiniteNumber(value.visual_scale) || value.visual_scale <= 0
    || !isFiniteNumber(value.page_scale) || value.page_scale <= 0
    || !isFiniteNumber(value.device_pixel_ratio) || value.device_pixel_ratio <= 0
    || !isBoolean(value.geometry_fresh)) return malformed("Browser view viewport state is malformed");
  return {
    viewport_revision: value.viewport_revision, css_width: value.css_width, css_height: value.css_height,
    visual_offset_x: value.visual_offset_x, visual_offset_y: value.visual_offset_y, scroll_x: value.scroll_x,
    scroll_y: value.scroll_y, visual_scale: value.visual_scale, page_scale: value.page_scale,
    device_pixel_ratio: value.device_pixel_ratio, geometry_fresh: value.geometry_fresh,
  };
}

export function parseBrowserViewNavigationState(value: unknown): BrowserViewNavigationState {
  if (!isRecord(value) || !isString(value.url) || !isString(value.title) || !isBoolean(value.loading)
    || !isBoolean(value.can_go_back) || !isBoolean(value.can_go_forward)
    || !(value.requested_url === null || isString(value.requested_url))) return malformed("Browser view navigation state is malformed");
  return {
    url: value.url, title: value.title, loading: value.loading, can_go_back: value.can_go_back,
    can_go_forward: value.can_go_forward, requested_url: value.requested_url,
  };
}

const browserViewCursors = [
  "default", "pointer", "text", "crosshair", "move", "not_allowed", "wait", "grab", "grabbing", "cell",
  "help", "progress", "zoom_in", "zoom_out", "column_resize", "row_resize", "east_resize", "west_resize",
  "north_resize", "south_resize", "northeast_resize", "northwest_resize", "southeast_resize", "southwest_resize",
] as const;
export function parseBrowserViewCursorState(value: unknown): BrowserViewCursorState {
  if (!isRecord(value) || !isBoundedId(value.target_id) || !isU64(value.pointer_sample_sequence)
    || !isU64(value.document_generation) || !isU64(value.viewport_revision)) return malformed("Browser view cursor state is malformed");
  return {
    cursor: parseBrowserViewEnum(value.cursor, browserViewCursors, "Browser view cursor"),
    pointer_sample_sequence: value.pointer_sample_sequence, target_id: value.target_id,
    document_generation: value.document_generation, viewport_revision: value.viewport_revision,
  };
}

export function parseBrowserViewFocusState(value: unknown): BrowserViewFocusState {
  if (!isRecord(value) || !isBoolean(value.page_focused) || !isBoolean(value.editable)
    || !isBoolean(value.selection_available) || !isBoolean(value.composition_active)) {
    return malformed("Browser view focus state is malformed");
  }
  return {
    page_focused: value.page_focused, editable: value.editable,
    selection_available: value.selection_available, composition_active: value.composition_active,
  };
}

export function parseBrowserViewBlocker(value: unknown): BrowserViewBlocker {
  if (!isRecord(value) || !isBoundedId(value.blocker_id) || !isString(value.message)
    || !(value.default_prompt === null || isString(value.default_prompt))
    || !isBoundedId(value.target_id) || !isU64(value.document_generation) || !isBoolean(value.cancellable)) {
    return malformed("Browser view blocker is malformed");
  }
  return {
    blocker_id: value.blocker_id,
    kind: parseBrowserViewEnum(value.kind, ["dialog", "file_chooser", "download", "permission", "unsupported"] as const, "Browser view blocker kind"),
    message: value.message, default_prompt: value.default_prompt, target_id: value.target_id,
    document_generation: value.document_generation, cancellable: value.cancellable,
  };
}

const browserViewCapabilities = ["supported", "unsupported", "unavailable"] as const;
export function parseBrowserViewCapabilities(value: unknown): BrowserViewCapabilities {
  const fields = ["pointer_input", "keyboard_input", "text_input", "composition_input", "clipboard_read",
    "clipboard_write", "dialogs", "file_chooser", "downloads", "permissions", "inspection", "capture", "drafts", "audio"] as const;
  if (!isRecord(value) || fields.some((field) => !isOneOf(value[field], browserViewCapabilities))) {
    return malformed("Browser view capabilities are malformed");
  }
  return Object.fromEntries(fields.map((field) => [field, value[field]])) as BrowserViewCapabilities;
}

export function parseBrowserViewControlState(value: unknown): BrowserViewControlState {
  if (!isRecord(value) || !(value.status === "observing" || value.status === "pending" || value.status === "controlled"
    || value.status === "revoked" || value.status === "lost")
    || !(value.controller_view_id === null || isBoundedId(value.controller_view_id))
    || !isU64(value.lease_generation) || !isU64(value.next_input_sequence) || !isBoolean(value.can_take_control)) {
    return malformed("Browser view control state is malformed");
  }
  return {
    status: value.status, controller_view_id: value.controller_view_id, lease_generation: value.lease_generation,
    next_input_sequence: value.next_input_sequence, can_take_control: value.can_take_control,
  };
}

export function parseBrowserViewFrameEnvelope(value: unknown): BrowserViewFrameEnvelopeV2 {
  if (!isRecord(value) || !isU32(value.magic) || !isU16(value.version) || !isU16(value.header_bytes)
    || !isU32(value.max_width) || !isU32(value.max_height) || !isU32(value.max_pixels) || !isU32(value.max_jpeg_bytes)) {
    return malformed("Browser view frame envelope is malformed");
  }
  return {
    magic: value.magic, version: value.version, header_bytes: value.header_bytes, max_width: value.max_width,
    max_height: value.max_height, max_pixels: value.max_pixels, max_jpeg_bytes: value.max_jpeg_bytes,
  };
}

export function parseBrowserViewFrameGrant(value: unknown): BrowserViewFrameGrant {
  if (!isRecord(value) || !isBoundedId(value.view_id) || !isU64(value.stream_epoch)
    || !isBoundedText(value.grant) || value.grant.length === 0 || !isString(value.expires_at)) {
    return malformed("Browser view frame grant is malformed");
  }
  return {
    view_id: value.view_id, stream_epoch: value.stream_epoch, grant: value.grant,
    expires_at: value.expires_at, envelope: parseBrowserViewFrameEnvelope(value.envelope),
  };
}

export function parseBrowserViewFrameDescriptor(value: unknown): BrowserViewFrameDescriptor {
  if (!isRecord(value) || !isBoundedId(value.target_id) || !isU64(value.stream_epoch)
    || !isU64(value.frame_sequence) || !isU64(value.document_generation) || !isU64(value.viewport_revision)
    || !isU32(value.image_width) || value.image_width === 0 || value.image_width > browserViewMaxWidth
    || !isU32(value.image_height) || value.image_height === 0 || value.image_height > browserViewMaxHeight
    || value.image_width * value.image_height > browserViewMaxPixels
    || !isFiniteNumber(value.viewport_css_width) || value.viewport_css_width <= 0
    || !isFiniteNumber(value.viewport_css_height) || value.viewport_css_height <= 0
    || !isFiniteNumber(value.viewport_offset_x) || !isFiniteNumber(value.viewport_offset_y)
    || !isFiniteNumber(value.scroll_x) || !isFiniteNumber(value.scroll_y)
    || !isU64(value.capture_timestamp_micros) || !isU32(value.jpeg_length)
    || value.jpeg_length === 0 || value.jpeg_length > browserViewMaxJpegBytes) {
    return malformed("Browser view frame descriptor is malformed");
  }
  return {
    target_id: value.target_id, stream_epoch: value.stream_epoch, frame_sequence: value.frame_sequence,
    document_generation: value.document_generation, viewport_revision: value.viewport_revision,
    image_width: value.image_width, image_height: value.image_height,
    viewport_css_width: value.viewport_css_width, viewport_css_height: value.viewport_css_height,
    viewport_offset_x: value.viewport_offset_x, viewport_offset_y: value.viewport_offset_y,
    scroll_x: value.scroll_x, scroll_y: value.scroll_y, capture_timestamp_micros: value.capture_timestamp_micros,
    jpeg_length: value.jpeg_length,
  };
}

export function parseBrowserViewSnapshot(value: unknown): BrowserViewSnapshot {
  if (!isRecord(value) || !isU64(value.metadata_sequence) || !Array.isArray(value.targets)
    || value.targets.length > 10000 || !(value.displayed_target_id === null || isBoundedId(value.displayed_target_id))) {
    return malformed("Browser view snapshot is malformed");
  }
  const identity = parseBrowserViewIdentity(value.identity);
  const targets = value.targets.map(parseBrowserViewTargetSummary);
  const document = parseNullable(value.document, parseBrowserViewDocumentState);
  const viewport = parseNullable(value.viewport, parseBrowserViewViewportState);
  const navigation = parseNullable(value.navigation, parseBrowserViewNavigationState);
  const cursor = parseNullable(value.cursor, parseBrowserViewCursorState);
  const blocker = parseNullable(value.blocker, parseBrowserViewBlocker);
  const frameGrant = parseNullable(value.frame_grant, parseBrowserViewFrameGrant);
  if (frameGrant !== null && (frameGrant.view_id !== identity.view_id || frameGrant.stream_epoch !== identity.stream_epoch)) {
    return malformed("Browser view frame grant belongs to another stream");
  }
  return {
    identity, metadata_sequence: value.metadata_sequence, targets,
    displayed_target_id: value.displayed_target_id, document, viewport, navigation, cursor,
    focus: parseBrowserViewFocusState(value.focus), blocker, capabilities: parseBrowserViewCapabilities(value.capabilities),
    control: parseBrowserViewControlState(value.control), frame_grant: frameGrant,
  };
}

export function parseBrowserViewEventMetadata(value: unknown): BrowserViewEventMetadata {
  if (!isRecord(value) || !isBoundedId(value.view_id) || !isU64(value.stream_epoch) || !isU64(value.metadata_sequence)) {
    return malformed("Browser view event metadata is malformed");
  }
  return { view_id: value.view_id, stream_epoch: value.stream_epoch, metadata_sequence: value.metadata_sequence };
}

function parseBrowserViewMetadataEvent(value: Record<string, unknown>): BrowserViewEventMetadata {
  return parseBrowserViewEventMetadata(value.metadata);
}
export function parseBrowserViewEvent(value: unknown): BrowserViewEvent {
  if (!isRecord(value) || !isString(value.type)) return malformed("Browser view event is malformed");
  const metadata = parseBrowserViewMetadataEvent(value);
  switch (value.type) {
    case "attached": {
      const snapshot = parseBrowserViewSnapshot(value.snapshot);
      if (snapshot.identity.view_id !== metadata.view_id || snapshot.identity.stream_epoch !== metadata.stream_epoch
        || snapshot.metadata_sequence !== metadata.metadata_sequence) return malformed("Attached browser view snapshot identity is malformed");
      return { type: "attached", metadata, snapshot };
    }
    case "targets_changed":
      if (!Array.isArray(value.targets) || !value.targets.every((item) => { try { parseBrowserViewTargetSummary(item); return true; } catch { return false; } })
        || !(value.displayed_target_id === null || isBoundedId(value.displayed_target_id))) return malformed("Browser view targets event is malformed");
      return { type: "targets_changed", metadata, targets: value.targets.map(parseBrowserViewTargetSummary), displayed_target_id: value.displayed_target_id };
    case "document_changed":
      return { type: "document_changed", metadata, document: parseNullable(value.document, parseBrowserViewDocumentState) };
    case "viewport_changed":
      return { type: "viewport_changed", metadata, viewport: parseNullable(value.viewport, parseBrowserViewViewportState) };
    case "navigation_changed":
      return { type: "navigation_changed", metadata, navigation: parseNullable(value.navigation, parseBrowserViewNavigationState) };
    case "cursor_changed":
      return { type: "cursor_changed", metadata, cursor: parseNullable(value.cursor, parseBrowserViewCursorState) };
    case "focus_changed":
      return { type: "focus_changed", metadata, focus: parseBrowserViewFocusState(value.focus) };
    case "blocker_changed":
      return { type: "blocker_changed", metadata, blocker: parseNullable(value.blocker, parseBrowserViewBlocker) };
    case "capabilities_changed":
      return { type: "capabilities_changed", metadata, capabilities: parseBrowserViewCapabilities(value.capabilities) };
    case "control_changed":
      return { type: "control_changed", metadata, control: parseBrowserViewControlState(value.control) };
    case "frame_descriptor":
      return { type: "frame_descriptor", metadata, descriptor: parseBrowserViewFrameDescriptor(value.descriptor) };
    case "frame_transport_revoked":
      if (!isString(value.code) || !isString(value.message)) return malformed("Browser view frame transport event is malformed");
      return { type: "frame_transport_revoked", metadata, code: value.code, message: value.message };
    case "failed":
      if (!isString(value.code) || !isString(value.message)) return malformed("Browser view failure event is malformed");
      return { type: "failed", metadata, code: value.code, message: value.message };
    case "closed":
      if (!isString(value.reason)) return malformed("Browser view closed event is malformed");
      return { type: "closed", metadata, reason: value.reason };
    default:
      return malformed(`Unknown browser view event type: ${value.type}`);
  }
}

function parseBrowserViewLocation(value: unknown): BrowserViewLocation {
  if (!isRecord(value) || !isBoundedId(value.target_id) || !isU64(value.document_generation)
    || !isU64(value.viewport_revision) || !isU64(value.presented_frame_sequence) || !isU64(value.lease_generation)) {
    return malformed("Browser view location is malformed");
  }
  return {
    target_id: value.target_id, document_generation: value.document_generation,
    viewport_revision: value.viewport_revision, presented_frame_sequence: value.presented_frame_sequence,
    lease_generation: value.lease_generation,
  };
}
function parseBrowserViewDocumentCommandContext(value: unknown): BrowserViewDocumentCommandContext {
  if (!isRecord(value) || !isBoundedId(value.target_id) || !isU64(value.document_generation) || !isU64(value.lease_generation)) {
    return malformed("Browser view document context is malformed");
  }
  return { target_id: value.target_id, document_generation: value.document_generation, lease_generation: value.lease_generation };
}
function parseBrowserViewPointerInput(value: unknown): BrowserViewPointerInput {
  if (!isRecord(value) || !isOneOf(value.kind, ["move", "down", "up", "cancel"] as const)
    || !(value.button === null || isOneOf(value.button, ["left", "middle", "right"] as const))
    || !isFiniteNumber(value.x) || !isFiniteNumber(value.y) || !isU8(value.buttons) || !isU8(value.modifiers)
    || !isU8(value.click_count) || !isU64(value.input_sequence)) return malformed("Browser view pointer input is malformed");
  return {
    kind: value.kind, button: value.button, x: value.x, y: value.y, buttons: value.buttons,
    modifiers: value.modifiers, click_count: value.click_count, input_sequence: value.input_sequence,
  };
}
function parseBrowserViewWheelInput(value: unknown): BrowserViewWheelInput {
  if (!isRecord(value) || !isFiniteNumber(value.x) || !isFiniteNumber(value.y)
    || !isFiniteNumber(value.delta_x_css) || !isFiniteNumber(value.delta_y_css)
    || !isU8(value.modifiers) || !isU64(value.input_sequence)) return malformed("Browser view wheel input is malformed");
  return { x: value.x, y: value.y, delta_x_css: value.delta_x_css, delta_y_css: value.delta_y_css, modifiers: value.modifiers, input_sequence: value.input_sequence };
}
function parseBrowserViewKeyboardInput(value: unknown): BrowserViewKeyboardInput {
  if (!isRecord(value) || !isOneOf(value.kind, ["down", "up"] as const) || !isString(value.key) || !isString(value.code)
    || !isU8(value.location) || !isU8(value.modifiers) || !isBoolean(value.repeat) || !isU64(value.input_sequence)) {
    return malformed("Browser view keyboard input is malformed");
  }
  return { kind: value.kind, key: value.key, code: value.code, location: value.location, modifiers: value.modifiers, repeat: value.repeat, input_sequence: value.input_sequence };
}
function parseBrowserViewTextInput(value: unknown): BrowserViewTextInput {
  if (!isRecord(value) || !isBoundedText(value.text) || !isU64(value.input_sequence)) return malformed("Browser view text input is malformed");
  return { text: value.text, input_sequence: value.input_sequence };
}
function parseBrowserViewCompositionInput(value: unknown): BrowserViewCompositionInput {
  if (!isRecord(value) || !isOneOf(value.kind, ["start", "update", "commit", "cancel"] as const)
    || !isBoundedText(value.text) || !isU64(value.input_sequence)) return malformed("Browser view composition input is malformed");
  return { kind: value.kind, text: value.text, input_sequence: value.input_sequence };
}
function parseBrowserViewClipboardCommand(value: unknown): BrowserViewClipboardCommand {
  if (!isRecord(value) || !isString(value.type)) return malformed("Browser view clipboard command is malformed");
  if (value.type === "copy") return { type: "copy" };
  if (value.type === "paste" && isBoundedText(value.text)) return { type: "paste", text: value.text };
  return malformed("Browser view clipboard command is malformed");
}
function parseBrowserViewNavigationCommand(value: unknown): BrowserViewNavigationCommand {
  if (!isRecord(value) || !isString(value.type)) return malformed("Browser view navigation command is malformed");
  if (value.type === "navigate" && isString(value.url)) return { type: "navigate", url: value.url };
  if (value.type === "back" || value.type === "forward" || value.type === "reload" || value.type === "stop") return { type: value.type };
  return malformed("Browser view navigation command is malformed");
}
function parseBrowserViewTabCommand(value: unknown): BrowserViewTabCommand {
  if (!isRecord(value) || !isString(value.type)) return malformed("Browser view tab command is malformed");
  if (value.type === "select" && isBoundedId(value.target_id)) return { type: "select", target_id: value.target_id };
  if (value.type === "create" && (value.url === null || isString(value.url))) return { type: "create", url: value.url };
  if (value.type === "close" && isBoundedId(value.target_id)) return { type: "close", target_id: value.target_id };
  return malformed("Browser view tab command is malformed");
}
function parseBrowserViewDialogCommand(value: unknown): BrowserViewDialogCommand {
  if (!isRecord(value) || !isString(value.type)) return malformed("Browser view dialog command is malformed");
  if (value.type === "dismiss") return { type: "dismiss" };
  if (value.type === "accept" && (value.text === null || isBoundedText(value.text))) return { type: "accept", text: value.text };
  return malformed("Browser view dialog command is malformed");
}
function parseBrowserViewFileCommand(value: unknown): BrowserViewFileCommand {
  if (!isRecord(value) || !isString(value.type)) return malformed("Browser view file command is malformed");
  if (value.type === "cancel") return { type: "cancel" };
  if (value.type === "choose" && Array.isArray(value.selection_ids) && value.selection_ids.length <= browserViewMaxFileSelections
    && value.selection_ids.every(isBoundedId)) return { type: "choose", selection_ids: value.selection_ids };
  return malformed("Browser view file command is malformed");
}
function parseBrowserViewDownloadCommand(value: unknown): BrowserViewDownloadCommand {
  if (!isRecord(value) || (value.type !== "accept" && value.type !== "cancel")) return malformed("Browser view download command is malformed");
  return { type: value.type };
}
function parseBrowserViewPermissionCommand(value: unknown): BrowserViewPermissionCommand {
  if (!isRecord(value) || !isOneOf(value.decision, ["allow", "deny", "cancel"] as const)) return malformed("Browser view permission command is malformed");
  return { decision: value.decision };
}
function parseBrowserViewInspectResult(value: unknown): BrowserViewInspectResult {
  if (!isRecord(value) || !isBoundedId(value.frame_id) || !isU64(value.frame_generation)
    || !(value.pointer_sample_sequence === null || isU64(value.pointer_sample_sequence)) || !isBoolean(value.inspectable)
    || !isOneOf(value.freshness, ["fresh", "review_required", "stale", "unavailable"] as const)
    || !(value.limitation === null || isString(value.limitation))) return malformed("Browser view inspection result is malformed");
  return {
    location: parseBrowserViewLocation(value.location), frame_id: value.frame_id,
    frame_generation: value.frame_generation, pointer_sample_sequence: value.pointer_sample_sequence,
    bounds: value.bounds === null ? null : parseFeedbackRect(value.bounds),
    evidence: value.evidence === null ? null : parseFeedbackElement(value.evidence),
    inspectable: value.inspectable, freshness: value.freshness, limitation: value.limitation,
  };
}
function parseBrowserViewInspectCommand(value: unknown): BrowserViewInspectCommand {
  if (!isRecord(value) || !(value.pointer_sample_sequence === null || isU64(value.pointer_sample_sequence)) || !isFiniteNumber(value.x) || !isFiniteNumber(value.y)) {
    return malformed("Browser view inspection command is malformed");
  }
  return { location: parseBrowserViewLocation(value.location), pointer_sample_sequence: value.pointer_sample_sequence, x: value.x, y: value.y };
}
function parseBrowserViewCaptureCommand(value: unknown): BrowserViewCaptureCommand {
  if (!isRecord(value) || !isBoundedId(value.draft_id) || !isU64(value.draft_revision)
    || !Array.isArray(value.annotation_ids) || value.annotation_ids.length > browserViewMaxCaptureAnnotations
    || !value.annotation_ids.every(isBoundedId) || !isBoolean(value.capture_as_shown)) return malformed("Browser view capture command is malformed");
  return {
    location: parseBrowserViewLocation(value.location), draft_id: value.draft_id, draft_revision: value.draft_revision,
    annotation_ids: value.annotation_ids, capture_as_shown: value.capture_as_shown,
  };
}
function parseBrowserViewDraftAnnotation(value: unknown): BrowserViewDraftAnnotation {
  if (!isRecord(value) || !isBoundedId(value.id) || !isOneOf(value.kind, ["freehand", "element", "region"] as const) || !isString(value.color) || !Array.isArray(value.points)
    || value.points.length > browserViewMaxAnnotationPoints || !(value.comment === null || isString(value.comment))) return malformed("Browser view draft annotation is malformed");
  return {
    id: value.id, kind: value.kind, color: value.color, points: value.points.map(parseFeedbackPoint),
    bounds: value.bounds === null ? null : parseFeedbackRect(value.bounds),
    evidence: value.evidence === null ? null : parseFeedbackElement(value.evidence), comment: value.comment,
  };
}
function parseBrowserViewDraftState(value: unknown): BrowserViewDraftState {
  if (!isRecord(value) || !isBoundedId(value.draft_id) || !isBoundedId(value.target_id)
    || !isU64(value.document_generation) || !isU64(value.revision) || !Array.isArray(value.annotations)
    || !value.annotations.every((item) => { try { parseBrowserViewDraftAnnotation(item); return true; } catch { return false; } })
    || !isOneOf(value.freshness, ["fresh", "review_required", "stale", "unavailable"] as const)
    || !isBoolean(value.stale) || !isRecord(value.editor)
    || !isNullableString(value.editor.selected_annotation_id) || !isBoolean(value.editor.notes_open)
    || !(value.editor.note_annotation_id === undefined || isNullableString(value.editor.note_annotation_id))
    || !(value.editor.note_text === undefined || (isString(value.editor.note_text) && value.editor.note_text.length <= browserViewMaxNoteTextCodeUnits))) {
    return malformed("Browser view draft state is malformed");
  }
  return {
    draft_id: value.draft_id,
    target_id: value.target_id,
    document_generation: value.document_generation,
    revision: value.revision,
    annotations: value.annotations.map(parseBrowserViewDraftAnnotation),
    freshness: value.freshness,
    stale: value.stale,
    editor: {
      selected_annotation_id: value.editor.selected_annotation_id,
      notes_open: value.editor.notes_open,
      note_annotation_id: value.editor.note_annotation_id === undefined ? null : value.editor.note_annotation_id,
      note_text: value.editor.note_text === undefined ? "" : value.editor.note_text,
    },
  };
}
function parseBrowserViewPendingCapture(value: unknown) {
  if (!isRecord(value) || !isBoundedId(value.association_key) || !isBoundedId(value.browser_incarnation)
    || !isBoundedId(value.capture_id) || !isBoundedId(value.draft_id) || !isU64(value.draft_revision)
    || !Array.isArray(value.annotation_ids) || !value.annotation_ids.every(isBoundedId)
    || !(value.last_error === undefined || value.last_error === null || isBoundedText(value.last_error))) {
    return malformed("Browser pending capture is malformed");
  }
  return {
    association_key: value.association_key,
    browser_incarnation: value.browser_incarnation,
    capture_id: value.capture_id,
    draft_id: value.draft_id,
    draft_revision: value.draft_revision,
    annotation_ids: value.annotation_ids,
    last_error: value.last_error === undefined ? null : value.last_error,
  };
}
function parseBrowserViewDraftInventory(value: unknown): BrowserViewDraftInventory {
  if (!isRecord(value) || !Array.isArray(value.drafts) || value.drafts.length > 8
    || !value.drafts.every((item) => { try { parseBrowserViewDraftState(item); return true; } catch { return false; } })
    || !isU64(value.active_draft_limit)
    || !(value.pending_capture === null || isRecord(value.pending_capture))) {
    return malformed("Browser draft inventory outcome is malformed");
  }
  return {
    drafts: value.drafts.map(parseBrowserViewDraftState),
    active_draft_limit: value.active_draft_limit,
    pending_capture: value.pending_capture === null ? null : parseBrowserViewPendingCapture(value.pending_capture),
  };
}
function parseBrowserViewCaptureOutcome(value: unknown): BrowserViewCaptureOutcome {
  if (!isRecord(value) || !isString(value.state)) return malformed("Browser capture outcome is malformed");
  if (value.state === "absent") return { state: "absent" };
  if (value.state === "pending") return { state: "pending", pending: parseBrowserViewPendingCapture(value.pending) };
  if (value.state === "saved") {
    if (!isRecord(value.saved) || !isBoundedId(value.saved.capture_id) || !Array.isArray(value.saved.annotation_ids)
      || !value.saved.annotation_ids.every(isBoundedId) || !isString(value.saved.image_path) || !isU64(value.saved.pending_count)) return malformed("Browser saved capture is malformed");
    return { state: "saved", saved: { capture_id: value.saved.capture_id, annotation_ids: value.saved.annotation_ids, image_path: value.saved.image_path, pending_count: value.saved.pending_count } };
  }
  return malformed("Browser capture outcome is malformed");
}
function parseBrowserViewDraftCommand(value: unknown): BrowserViewDraftCommand {
  if (!isRecord(value) || !isString(value.type)) return malformed("Browser view draft command is malformed");
  if (value.type === "open" && (value.draft_id === null || isBoundedId(value.draft_id))) return { type: "open", draft_id: value.draft_id };
  if (value.type === "upsert_annotation") return { type: "upsert_annotation", annotation: parseBrowserViewDraftAnnotation(value.annotation) };
  if (value.type === "remove_annotation" && isBoundedId(value.annotation_id)) return { type: "remove_annotation", annotation_id: value.annotation_id };
  if (value.type === "list" || value.type === "clear" || value.type === "discard" || value.type === "retry_pending" || value.type === "discard_pending") return { type: value.type };
  if (value.type === "save_capture" && isRecord(value.submission) && Array.isArray(value.annotation_ids) && value.annotation_ids.every(isBoundedId) && isRecord(value.provenance)) {
    const submission = value.submission;
    const provenance = value.provenance;
    if (!isBoundedId(submission.association_key) || !isBoundedId(submission.browser_instance) || !isBoundedId(submission.capture_id)
      || !Array.isArray(submission.annotations) || !isString(submission.png_base64)
      || !isBoundedId(provenance.target_id) || !isBoundedId(provenance.frame_id)
      || !isU64(provenance.document_generation) || !isU64(provenance.frame_generation) || !isU64(provenance.stream_epoch)
      || !isU64(provenance.frame_sequence) || !isU64(provenance.viewport_revision) || !isU64(provenance.pixel_captured_at_micros)
      || !isBoolean(provenance.capture_as_shown)) return malformed("Browser capture save command is malformed");
    return {
      type: "save_capture",
      submission: {
        association_key: submission.association_key, browser_instance: submission.browser_instance, capture_id: submission.capture_id,
        page: parseFeedbackPage(submission.page), annotations: submission.annotations.map(parseFeedbackAnnotation), png_base64: submission.png_base64,
      },
      annotation_ids: value.annotation_ids,
      provenance: {
        target_id: provenance.target_id, frame_id: provenance.frame_id, document_generation: provenance.document_generation,
        frame_generation: provenance.frame_generation, stream_epoch: provenance.stream_epoch, frame_sequence: provenance.frame_sequence,
        viewport_revision: provenance.viewport_revision, pixel_captured_at_micros: provenance.pixel_captured_at_micros, capture_as_shown: provenance.capture_as_shown,
      },
    };
  }
  return malformed("Browser view draft command is malformed");
}
export function parseBrowserDraftRecoveryRequest(value: unknown): BrowserDraftRecoveryRequest {
  if (!isRecord(value) || !isRecord(value.action) || !isString(value.action.type)) return malformed("Browser draft recovery request is malformed");
  const action: BrowserDraftRecoveryAction = value.action.type === "list" || value.action.type === "retry_pending" || value.action.type === "discard_pending"
    ? { type: value.action.type }
    : value.action.type === "set_editor"
      && isBoundedId(value.action.draft_id)
      && isU64(value.action.expected_revision)
      && isRecord(value.action.editor)
      && isNullableString(value.action.editor.selected_annotation_id)
      && isBoolean(value.action.editor.notes_open)
      && (value.action.editor.note_annotation_id === undefined || isNullableString(value.action.editor.note_annotation_id))
      && (value.action.editor.note_text === undefined || (isString(value.action.editor.note_text) && value.action.editor.note_text.length <= browserViewMaxNoteTextCodeUnits))
      ? {
        type: "set_editor",
        draft_id: value.action.draft_id,
        expected_revision: value.action.expected_revision,
        editor: {
          selected_annotation_id: value.action.editor.selected_annotation_id,
          notes_open: value.action.editor.notes_open,
          note_annotation_id: value.action.editor.note_annotation_id === undefined ? null : value.action.editor.note_annotation_id,
          note_text: value.action.editor.note_text === undefined ? "" : value.action.editor.note_text,
        },
      }
      : value.action.type === "upsert_annotation"
        && isBoundedId(value.action.draft_id)
        && isU64(value.action.expected_revision)
        && isRecord(value.action.annotation)
        ? {
          type: "upsert_annotation",
          draft_id: value.action.draft_id,
          expected_revision: value.action.expected_revision,
          annotation: parseBrowserViewDraftAnnotation(value.action.annotation),
        }
        : value.action.type === "remove_annotation"
          && isBoundedId(value.action.draft_id)
          && isU64(value.action.expected_revision)
          && isBoundedId(value.action.annotation_id)
          ? {
            type: "remove_annotation",
            draft_id: value.action.draft_id,
            expected_revision: value.action.expected_revision,
            annotation_id: value.action.annotation_id,
          }
          : value.action.type === "discard_draft" && isBoundedId(value.action.draft_id) && isU64(value.action.expected_revision)
            ? { type: "discard_draft", draft_id: value.action.draft_id, expected_revision: value.action.expected_revision }
            : malformed("Browser draft recovery action is malformed");
  return { target: parseBrowserTarget(value.target), action };
}

export function parseBrowserViewCommand(value: unknown): BrowserViewCommand {
  if (!isRecord(value) || !isString(value.type)) return malformed("Browser view command is malformed");
  switch (value.type) {
    case "take_control": return { type: "take_control", viewport: parseBrowserViewViewportRequest(value.viewport) };
    case "release_control":
      if (!isU64(value.lease_generation)) return malformed("Browser view release command is malformed");
      return { type: "release_control", lease_generation: value.lease_generation };
    case "detach": return { type: "detach" };
    case "resize": return { type: "resize", context: parseBrowserViewDocumentCommandContext(value.context), viewport: parseBrowserViewViewportRequest(value.viewport) };
    case "pointer": return { type: "pointer", location: parseBrowserViewLocation(value.location), input: parseBrowserViewPointerInput(value.input) };
    case "wheel": return { type: "wheel", location: parseBrowserViewLocation(value.location), input: parseBrowserViewWheelInput(value.input) };
    case "keyboard": return { type: "keyboard", context: parseBrowserViewDocumentCommandContext(value.context), input: parseBrowserViewKeyboardInput(value.input) };
    case "text": return { type: "text", context: parseBrowserViewDocumentCommandContext(value.context), input: parseBrowserViewTextInput(value.input) };
    case "composition": return { type: "composition", context: parseBrowserViewDocumentCommandContext(value.context), input: parseBrowserViewCompositionInput(value.input) };
    case "clipboard": return { type: "clipboard", context: parseBrowserViewDocumentCommandContext(value.context), command: parseBrowserViewClipboardCommand(value.command) };
    case "navigation": return { type: "navigation", context: parseBrowserViewDocumentCommandContext(value.context), command: parseBrowserViewNavigationCommand(value.command) };
    case "tab": return { type: "tab", command: parseBrowserViewTabCommand(value.command) };
    case "dialog":
      if (!isBoundedId(value.blocker_id)) return malformed("Browser view dialog command is malformed");
      return { type: "dialog", blocker_id: value.blocker_id, command: parseBrowserViewDialogCommand(value.command) };
    case "file":
      if (!isBoundedId(value.blocker_id)) return malformed("Browser view file command is malformed");
      return { type: "file", blocker_id: value.blocker_id, command: parseBrowserViewFileCommand(value.command) };
    case "download":
      if (!isBoundedId(value.blocker_id)) return malformed("Browser view download command is malformed");
      return { type: "download", blocker_id: value.blocker_id, command: parseBrowserViewDownloadCommand(value.command) };
    case "permission":
      if (!isBoundedId(value.blocker_id)) return malformed("Browser view permission command is malformed");
      return { type: "permission", blocker_id: value.blocker_id, command: parseBrowserViewPermissionCommand(value.command) };
    case "inspect": return { type: "inspect", command: parseBrowserViewInspectCommand(value.command) };
    case "capture": return { type: "capture", command: parseBrowserViewCaptureCommand(value.command) };
    case "draft":
      if (!isBoundedId(value.context && isRecord(value.context) ? value.context.target_id : undefined)
        || !(value.draft_id === null || isBoundedId(value.draft_id))
        || !(value.expected_revision === null || isU64(value.expected_revision))) return malformed("Browser view draft command is malformed");
      return {
        type: "draft", context: parseBrowserViewDocumentCommandContext(value.context), draft_id: value.draft_id,
        expected_revision: value.expected_revision, command: parseBrowserViewDraftCommand(value.command),
      };
    default: return malformed(`Unknown browser view command type: ${value.type}`);
  }
}

export function parseBrowserViewCommandRequest(value: unknown): BrowserViewCommandRequest {
  if (!isRecord(value) || !isBoundedId(value.view_id) || !isU64(value.stream_epoch) || !isBoundedId(value.request_id)) {
    return malformed("Browser view command request is malformed");
  }
  return { view_id: value.view_id, stream_epoch: value.stream_epoch, request_id: value.request_id, command: parseBrowserViewCommand(value.command) };
}
export function parseBrowserViewCommandOutcome(value: unknown): BrowserViewCommandOutcome {
  if (!isRecord(value) || !isString(value.type)) return malformed("Browser view command outcome is malformed");
  switch (value.type) {
    case "none": return { type: "none" };
    case "snapshot": return { type: "snapshot", snapshot: parseBrowserViewSnapshot(value.snapshot) };
    case "control": return { type: "control", control: parseBrowserViewControlState(value.control) };
    case "inspection": return { type: "inspection", inspection: parseBrowserViewInspectResult(value.inspection) };
    case "capture_prepared":
      if (!isBoundedId(value.capture_id)) return malformed("Browser view capture outcome is malformed");
      return { type: "capture_prepared", capture_id: value.capture_id, descriptor: parseBrowserViewFrameDescriptor(value.descriptor) };
    case "draft": return { type: "draft", draft: parseBrowserViewDraftState(value.draft) };
    case "draft_inventory":
      if (!isRecord(value.inventory) || !Array.isArray(value.inventory.drafts) || !value.inventory.drafts.every((item) => { try { parseBrowserViewDraftState(item); return true; } catch { return false; } })
        || !isU64(value.inventory.active_draft_limit)) return malformed("Browser draft inventory outcome is malformed");
      return { type: "draft_inventory", inventory: { drafts: value.inventory.drafts.map(parseBrowserViewDraftState), active_draft_limit: value.inventory.active_draft_limit, pending_capture: value.inventory.pending_capture === null ? null : parseBrowserViewPendingCapture(value.inventory.pending_capture) } };
    case "capture":
      return { type: "capture", capture: parseBrowserViewCaptureOutcome(value.capture) };
    case "clipboard":
      if (!(value.text === null || isString(value.text))) return malformed("Browser view clipboard outcome is malformed");
      return { type: "clipboard", text: value.text };
    default: return malformed(`Unknown browser view command outcome type: ${value.type}`);
  }
}
export function parseBrowserViewCommandResponse(value: unknown): BrowserViewCommandResponse {
  if (!isRecord(value) || !isString(value.status) || !isBoundedId(value.view_id)
    || !isU64(value.stream_epoch) || !isBoundedId(value.request_id)) return malformed("Browser view command response is malformed");
  if (value.status === "accepted") return { status: "accepted", view_id: value.view_id, stream_epoch: value.stream_epoch, request_id: value.request_id, outcome: parseBrowserViewCommandOutcome(value.outcome) };
  if (value.status === "rejected" || value.status === "outcome_unknown") {
    if (!isString(value.code) || !isString(value.message)) return malformed("Browser view command failure is malformed");
    return { status: value.status, view_id: value.view_id, stream_epoch: value.stream_epoch, request_id: value.request_id, code: value.code, message: value.message };
  }
  if (value.status === "unsupported") {
    if (!isString(value.capability) || !isString(value.message)) return malformed("Browser view unsupported response is malformed");
    return { status: "unsupported", view_id: value.view_id, stream_epoch: value.stream_epoch, request_id: value.request_id, capability: value.capability, message: value.message };
  }
  if (value.status === "stale") {
    if (!isU64(value.current_stream_epoch) || !isU64(value.current_metadata_sequence)
      || !isString(value.code) || !isString(value.message)) return malformed("Browser view stale response is malformed");
    return { status: "stale", view_id: value.view_id, stream_epoch: value.stream_epoch, request_id: value.request_id, current_stream_epoch: value.current_stream_epoch, current_metadata_sequence: value.current_metadata_sequence, code: value.code, message: value.message };
  }
  return malformed(`Unknown browser view command response status: ${value.status}`);
}

type BrowserViewIdentityMatch = Pick<BrowserViewIdentity, "view_id" | "stream_epoch">;
function matchBrowserViewIdentity(value: BrowserViewIdentityMatch, expected: BrowserViewIdentityMatch): void {
  if (value.view_id !== expected.view_id || value.stream_epoch !== expected.stream_epoch) malformed("Browser view stream identity does not match");
}
export function matchBrowserViewSnapshot(value: unknown, expected: BrowserViewIdentityMatch): BrowserViewSnapshot;
export function matchBrowserViewSnapshot(value: unknown, viewId: string, streamEpoch: number): BrowserViewSnapshot;
export function matchBrowserViewSnapshot(value: unknown, expectedOrViewId: BrowserViewIdentityMatch | string, streamEpoch?: number): BrowserViewSnapshot {
  const snapshot = parseBrowserViewSnapshot(value);
  const expected = typeof expectedOrViewId === "string"
    ? streamEpoch === undefined ? malformed("Browser view stream identity is malformed") : { view_id: expectedOrViewId, stream_epoch: streamEpoch }
    : expectedOrViewId;
  if (!isU64(expected.stream_epoch)) return malformed("Browser view stream identity is malformed");
  matchBrowserViewIdentity(snapshot.identity, expected);
  return snapshot;
}
export function matchBrowserViewEvent(value: unknown, expected: BrowserViewIdentityMatch): BrowserViewEvent;
export function matchBrowserViewEvent(value: unknown, viewId: string, streamEpoch: number): BrowserViewEvent;
export function matchBrowserViewEvent(value: unknown, expectedOrViewId: BrowserViewIdentityMatch | string, streamEpoch?: number): BrowserViewEvent {
  const event = parseBrowserViewEvent(value);
  const expected = typeof expectedOrViewId === "string"
    ? streamEpoch === undefined ? malformed("Browser view stream identity is malformed") : { view_id: expectedOrViewId, stream_epoch: streamEpoch }
    : expectedOrViewId;
  if (!isU64(expected.stream_epoch)) return malformed("Browser view stream identity is malformed");
  matchBrowserViewIdentity(event.metadata, expected);
  return event;
}
export function matchBrowserViewCommandResponse(value: unknown, request: BrowserViewCommandRequest): BrowserViewCommandResponse {
  const response = parseBrowserViewCommandResponse(value);
  if (response.view_id !== request.view_id || response.stream_epoch !== request.stream_epoch || response.request_id !== request.request_id) {
    return malformed("Browser view command response identity does not match request");
  }
  return response;
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
    || !(value.tab_id === undefined || value.tab_id === null || isU64(value.tab_id))
    || !isString(value.document_id) || !isString(value.captured_at)
    || !isRecord(value.viewport) || typeof value.viewport.width !== "number" || !Number.isFinite(value.viewport.width)
    || typeof value.viewport.height !== "number" || !Number.isFinite(value.viewport.height)
    || typeof value.viewport.scroll_x !== "number" || !Number.isFinite(value.viewport.scroll_x)
    || typeof value.viewport.scroll_y !== "number" || !Number.isFinite(value.viewport.scroll_y)
    || typeof value.viewport.device_pixel_ratio !== "number" || !Number.isFinite(value.viewport.device_pixel_ratio)
    || typeof value.viewport.visual_scale !== "number" || !Number.isFinite(value.viewport.visual_scale)
    || !isU32(value.image_width) || !isU32(value.image_height)) return malformed("Browser page evidence is malformed");
  return {
    url: value.url, title: value.title, tab_id: value.tab_id === undefined ? null : value.tab_id as BrowserPageEvidence["tab_id"], document_id: value.document_id, captured_at: value.captured_at,
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
    id: value.id, kind: value.kind as BrowserAnnotation["kind"], comment: value.comment, color: value.color,
    points: value.points.map(parseFeedbackPoint), bounds: value.bounds === null ? null : parseFeedbackRect(value.bounds),
    element: value.element === null ? null : parseFeedbackElement(value.element),
  };
}
function parseFeedbackContext(value: unknown): BrowserCaptureContext {
  if (!isRecord(value) || !isString(value.association_key) || !isString(value.session_id)
    || !isString(value.space_id) || !isString(value.space_label) || !isString(value.playwright_session)
    || !isString(value.working_directory) || !isString(value.invocation) || !isString(value.browser_instance)) {
    return malformed("Browser capture context is malformed");
  }
  return {
    association_key: value.association_key, session_id: value.session_id, space_id: value.space_id,
    space_label: value.space_label, playwright_session: value.playwright_session, working_directory: value.working_directory,
    invocation: value.invocation, browser_instance: value.browser_instance,
    inline_provenance: value.inline_provenance === undefined ? null : value.inline_provenance === null ? null : value.inline_provenance as BrowserCaptureContext["inline_provenance"],
  };
}
function parseFeedbackCapture(value: unknown): BrowserFeedbackCapture {
  if (!isRecord(value) || !isString(value.id) || !isRecord(value.context) || !isRecord(value.page)
    || !Array.isArray(value.annotations) || value.annotations.length > 10000 || !Array.isArray(value.pending_ids)
    || value.pending_ids.length > 10000 || !value.pending_ids.every(isString) || !isString(value.image_path)) {
    return malformed("Browser feedback capture is malformed");
  }
  return {
    id: value.id, context: parseFeedbackContext(value.context), page: parseFeedbackPage(value.page),
    annotations: value.annotations.map(parseFeedbackAnnotation), pending_ids: value.pending_ids, image_path: value.image_path,
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
  if (!isRecord(value) || !isString(value.capture_id) || value.capture_id.length === 0) return malformed("Browser feedback image request is malformed");
  return { target: parseBrowserTarget(value.target), capture_id: value.capture_id };
}
export function parseBrowserFeedbackSendRequest(value: unknown): BrowserFeedbackSendRequest {
  if (!isRecord(value) || !Array.isArray(value.ids) || value.ids.length > 10000
    || !value.ids.every((id) => isString(id) && id.length > 0) || !isString(value.operation_id)
    || value.operation_id.length === 0 || !isBoolean(value.acknowledge_duplicate_risk)) return malformed("Browser feedback send request is malformed");
  return { target: parseBrowserTarget(value.target), ids: value.ids, operation_id: value.operation_id, acknowledge_duplicate_risk: value.acknowledge_duplicate_risk };
}
function parseBrowserFeedbackDeliveryStatus(value: unknown): BrowserFeedbackDeliveryStatus {
  if (!isRecord(value) || !isString(value.capture_id) || value.capture_id.length === 0
    || !isString(value.operation_id) || value.operation_id.length === 0
    || !Array.isArray(value.selected_ids) || value.selected_ids.length > 64
    || !value.selected_ids.every((id) => isString(id) && id.length > 0)
    || !isString(value.state) || !(["pending", "accepted", "rejected", "outcome_unknown"] as readonly string[]).includes(value.state)
    || !isString(value.message) || value.message.length > 4096) return malformed("Browser feedback delivery status is malformed");
  return {
    capture_id: value.capture_id,
    operation_id: value.operation_id,
    selected_ids: value.selected_ids,
    state: value.state as BrowserFeedbackDeliveryStatus["state"],
    message: value.message,
  };
}
export function parseBrowserFeedbackLookup(value: unknown): BrowserFeedbackLookup {
  if (!isRecord(value)) return malformed("Browser feedback lookup is malformed");
  const deliveries = value.deliveries === undefined ? [] : value.deliveries;
  if (!Array.isArray(deliveries) || deliveries.length > 64) return malformed("Browser feedback deliveries are malformed");
  return {
    browser: parseBrowserResponse(value.browser),
    feedback: parseBrowserFeedbackResponse(value.feedback),
    deliveries: deliveries.map(parseBrowserFeedbackDeliveryStatus),
    drafts: value.drafts === null || value.drafts === undefined ? null : parseBrowserViewDraftInventory(value.drafts),
  };
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
