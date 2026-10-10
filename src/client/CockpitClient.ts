import * as Wire from "../protocol/generated/validate";
import { parseWire, tryParseWire } from "./wire";
import { CLIENT } from "./clientPolicy";
import type {
  OrchestrationSnapshotRequest, OrchestrationSnapshot,
  OrchestrationMutationRequest, OrchestrationMutationResponse,
  OrchestrationWaitRequest, OrchestrationWaitResponse,
} from "../protocol/generated/v1";
export type {
  OrchestrationSnapshotRequest, OrchestrationSnapshot,
  OrchestrationMutationRequest, OrchestrationMutationResponse,
  OrchestrationWaitRequest, OrchestrationWaitResponse,
} from "../protocol/generated/v1";
import type {
  WidgetEvent, WidgetWindowReport, WidgetContentRequest, WidgetContent,
  WidgetRemoveRequest, WidgetRemoveResponse, WidgetSelectRequest, WidgetSelectResponse,
} from "../protocol/generated/v1";
export type {
  WidgetKey, WidgetSummary, WidgetEvent, WidgetWindowReport, WidgetContentRequest, WidgetContent,
  WidgetRemoveRequest, WidgetRemoveResponse, WidgetSelectRequest, WidgetSelectResponse,
} from "../protocol/generated/v1";
import type { NotesRequest, NotesResponse } from "../protocol/generated/v1";
export type {
  NotesRequest, NotesResponse, NotesTarget, NotesOperation, NotesTodoSelector,
  NotesResult, NotesCatalogEntry, NotesTargetInfo, NotesSpaceInfo, NotesChangeTokens,
  NotesDocument, NotesTodo, NotesTodoProblem, NotesBoard, NotesLane, NotesColumn,
  NotesTodoFilter, NotesDecisionFilter, NotesDecisionStatus, NotesDecisionSummary,
  NotesDecision, NotesComment,
} from "../protocol/generated/v1";
import type { ContextMedia, ContextMediaRequest } from "../protocol/generated/v1";
import type { SpaceContextRequest, SpaceContextListing, SpaceAddRequest, SpaceRepositoriesRequest, SpaceRemoveRequest } from "../protocol/generated/v1";
import type { ReviewSnapshotRequest, ReviewSnapshot, ReviewFileRequest, ReviewFileDiff } from "../protocol/generated/v1";
import type { CommentPastePrepareRequest, CommentPastePrepareResponse, CommentPasteReceipt, CommentPasteMarkPastedRequest, CommentPasteSendRequest } from "../protocol/generated/v1";
import type {
  ContextSearchRequest, ContextSearchResponse, ContextInvalidationRequest, ContextInvalidationResponse,
  CommentRequestScope, CommentBatchList, CommentBatchRequest, CommentBatch,
  CommentBatchMutation, CommentUpsertRequest, CommentRemoveRequest, CommentPreviewRequest, CommentPreview,
} from "../protocol/generated/v1";
import type {
  BrowserCleanupState, BrowserCleanupScope, BrowserCleanupFailure, BrowserCleanupStatus, BrowserCleanupRetryRequest,
  BrowserWorkScope, CreatedPane,
  ViewerKind, ViewerSourceKind, ViewerSourceSelector, ViewerSourceOptions, ViewerOpenRequest, ViewerContext,
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
  PaneMoveDestination,
  ResourceMutationRequest,
  ResourceMutationResponse,
  PaneSummary,
  SessionListResponse,
  SessionSnapshotResponse,
  SessionStreamMessage,
  SessionSummary,
  SpaceGitStatus,
  SpaceGitStatusResponse,
  SpaceGitCheckout,
  SpaceGitUpstream,
  SpaceGitActionRequest,
  SpaceGitActionResponse,
  SpaceGitActionOutcome,
  SpaceGitSummary,
  SpaceSummary,
  TabSummary,
  StatusResponse,
  TerminalCommand,
  TerminalOpenRequest,
  TerminalOwnershipState,
  TerminalStreamMessage,
  ProjectConfiguration,
  ProviderCredentialClearRequest,
  ProviderCredentialSetRequest,
  ProviderCredentialStatus,
  ProviderCredentialStatusList,
  QuotaStatusRequest,
  QuotaStatusResponse,
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
  ContextDirectoryRequest,
  ContextDirectory,
  ContextFileIndexRequest,
  ContextFileIndex,
  ContextDocumentRequest,
  ContextDocument,
  LibraryAttachmentRequest,
  LibraryAddRequest,
  LibraryDirectoryRequest,
  LibraryFileIndexRequest,
  LibraryDocumentRequest,
  LibraryListing,
  LibraryMediaRequest,
  LibraryOperation,
  LibraryRefreshRequest,
  LibraryRemoveRequest,
  LibraryReplaceRequest,
  LibraryConfluenceSpacesRequest,
  LibraryResolution,
  LibraryResolveRequest,
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
  BrowserCleanupState, BrowserCleanupScope, BrowserCleanupFailure, BrowserCleanupStatus, BrowserCleanupRetryRequest,
  BrowserWorkScope, CreatedPane,
  ViewerKind, ViewerSourceKind, ViewerSourceSelector, ViewerSourceOptions, ViewerOpenRequest, ViewerContext,
  AgentSummary,
  ContextDirectory,
  ContextDirectoryRequest,
  ContextDocument,
  ContextDocumentRequest,
  ContextFileIndex,
  ContextFileIndexRequest,
  ContextMedia,
  ContextMediaRequest,
  FocusRequest,
  LibraryAttachmentRequest,
  LibraryAddRequest,
  LibraryDirectoryRequest,
  LibraryFileIndexRequest,
  LibraryListing,
  LibraryDocumentRequest,
  LibraryMediaRequest,
  LibraryOperation,
  LibraryRefreshRequest,
  LibraryRemoveRequest,
  LibraryReplaceRequest,
  LibraryResolution,
  LibraryResolveRequest,
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

export interface WidgetStream extends ClosableStream {
  /** Coalesced latest window state, sent at most once per 100 ms. */
  report(report: WidgetWindowReport): void;
}
export type WidgetEventHandler = (event: WidgetEvent) => void;

export interface TerminalStream extends ClosableStream {
  send(command: TerminalCommand): void;
}

export interface CockpitClient {
  status(): Promise<StatusResponse>;
  orchestrationSnapshot(request: OrchestrationSnapshotRequest): Promise<OrchestrationSnapshot>;
  orchestrationMutate(request: OrchestrationMutationRequest): Promise<OrchestrationMutationResponse>;
  orchestrationWait(request: OrchestrationWaitRequest): Promise<OrchestrationWaitResponse>;
  /** Cached subscription limits; provider CLIs own authentication and collection. */
  quotaStatus(request: QuotaStatusRequest, signal?: AbortSignal): Promise<QuotaStatusResponse>;
  browserAction(request: BrowserRequest): Promise<BrowserResponse>;
  browserCleanupStatus(): Promise<BrowserCleanupStatus>;
  browserCleanupRetry(request: BrowserCleanupRetryRequest): Promise<BrowserCleanupStatus>;
  browserFeedback(request: BrowserFeedbackRequest): Promise<BrowserFeedbackLookup>;
  browserDraftRecovery(request: BrowserDraftRecoveryRequest): Promise<BrowserViewCommandOutcome>;
  acknowledgeBrowserFeedback(request: BrowserFeedbackAckRequest): Promise<BrowserFeedbackAck>;
  browserFeedbackImage(request: BrowserFeedbackImageRequest): Promise<BrowserFeedbackImage>;
  sendBrowserFeedback(request: BrowserFeedbackSendRequest): Promise<BrowserFeedbackSendResponse>;
  projectConfiguration(): Promise<ProjectConfiguration>;
  /** Whether a token is stored for each configured provider. Write-only: no method returns a token or username. */
  providerCredentials(): Promise<ProviderCredentialStatusList>;
  /** Stores or replaces a provider's token in the OS vault; resolves with the provider's new status. */
  setProviderCredential(request: ProviderCredentialSetRequest): Promise<ProviderCredentialStatus>;
  clearProviderCredential(request: ProviderCredentialClearRequest): Promise<ProviderCredentialStatus>;
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
  viewerSources(sessionId: string, paneId: string, signal?: AbortSignal): Promise<ViewerSourceOptions>;
  viewerOpen(sessionId: string, request: ViewerOpenRequest): Promise<ViewerContext>;
  viewerRelease(sessionId: string, viewerId: string): Promise<void>;
  contextDirectory(sessionId: string, viewerId: string, request: ContextDirectoryRequest, signal?: AbortSignal): Promise<ContextDirectory>;
  contextFileIndex(sessionId: string, viewerId: string, request: ContextFileIndexRequest, signal?: AbortSignal): Promise<ContextFileIndex>;
  contextDocument(sessionId: string, viewerId: string, request: ContextDocumentRequest, signal?: AbortSignal): Promise<ContextDocument>;
  reviewSnapshot(sessionId: string, viewerId: string, request: ReviewSnapshotRequest, signal?: AbortSignal): Promise<ReviewSnapshot>;
  reviewFile(sessionId: string, viewerId: string, request: ReviewFileRequest, signal?: AbortSignal): Promise<ReviewFileDiff>;
  contextSearch(sessionId: string, viewerId: string, request: ContextSearchRequest, signal?: AbortSignal): Promise<ContextSearchResponse>;
  contextInvalidate(sessionId: string, viewerId: string, request: ContextInvalidationRequest, signal?: AbortSignal): Promise<ContextInvalidationResponse>;
  contextMedia(sessionId: string, viewerId: string, request: ContextMediaRequest, signal?: AbortSignal): Promise<ContextMedia>;
  /** Pinned durable Notes operations; aborting does not roll back a dispatched write. */
  notes(request: NotesRequest, signal?: AbortSignal): Promise<NotesResponse>;
  librarySpaceList(request: SpaceContextRequest, signal?: AbortSignal): Promise<SpaceContextListing>;
  librarySpaceAdd(request: SpaceAddRequest): Promise<LibraryOperation>;
  /** Replaces this Space's additional existing repository selections. */
  librarySpaceRepositories(request: SpaceRepositoriesRequest): Promise<SpaceContextListing>;
  /** Unselects Library items without changing their files. */
  librarySpaceRemove(request: SpaceRemoveRequest): Promise<SpaceContextListing>;
  commentBatches(sessionId: string, viewerId: string, request: CommentRequestScope, signal?: AbortSignal): Promise<CommentBatchList>;
  commentBatch(sessionId: string, viewerId: string, request: CommentBatchRequest, signal?: AbortSignal): Promise<CommentBatch>;
  commentUpsert(sessionId: string, viewerId: string, request: CommentUpsertRequest, signal?: AbortSignal): Promise<CommentBatch>;
  commentRemove(sessionId: string, viewerId: string, request: CommentRemoveRequest, signal?: AbortSignal): Promise<CommentBatch>;
  commentDiscard(sessionId: string, viewerId: string, request: CommentBatchMutation, signal?: AbortSignal): Promise<CommentBatchList>;
  commentAttach(sessionId: string, viewerId: string, request: CommentBatchMutation, signal?: AbortSignal): Promise<CommentBatch>;
  commentPastePrepare(sessionId: string, viewerId: string, request: CommentPastePrepareRequest, signal?: AbortSignal): Promise<CommentPastePrepareResponse>;
  commentPasteMarkPasted(sessionId: string, viewerId: string, request: CommentPasteMarkPastedRequest): Promise<CommentPasteReceipt>;
  commentPasteSend(sessionId: string, viewerId: string, request: CommentPasteSendRequest): Promise<CommentPasteReceipt>;
  commentPreview(sessionId: string, viewerId: string, request: CommentPreviewRequest, signal?: AbortSignal): Promise<CommentPreview>;
  libraryListing(offset?: number | null): Promise<LibraryListing>;
  libraryResolve(request: LibraryResolveRequest): Promise<LibraryResolution>;
  /** Spaces readable by one configured Confluence provider, each with its existing follow. */
  libraryConfluenceSpaces(request: LibraryConfluenceSpacesRequest): Promise<LibraryResolution[]>;
  libraryAdd(request: LibraryAddRequest): Promise<LibraryOperation>;
  libraryAttachments(request: LibraryAttachmentRequest): Promise<LibraryOperation>;
  libraryRefresh(request: LibraryRefreshRequest): Promise<LibraryOperation>;
  libraryOperation(operationId: string): Promise<LibraryOperation>;
  libraryOperationCancel(operationId: string): Promise<LibraryOperation>;
  libraryReplace(request: LibraryReplaceRequest): Promise<LibraryOperation>;
  libraryRemove(request: LibraryRemoveRequest): Promise<LibraryListing>;
  libraryDirectory(request: LibraryDirectoryRequest, signal?: AbortSignal): Promise<ContextDirectory>;
  libraryFileIndex(request: LibraryFileIndexRequest, signal?: AbortSignal): Promise<ContextFileIndex>;
  libraryDocument(request: LibraryDocumentRequest, signal?: AbortSignal): Promise<ContextDocument>;
  libraryMedia(request: LibraryMediaRequest, signal?: AbortSignal): Promise<ContextMedia>;
  sessions(): Promise<SessionListResponse>;
  sessionSnapshot(sessionId: string, signal?: AbortSignal): Promise<CockpitSessionSnapshot>;
  spaceGitStatus(sessionId: string, signal?: AbortSignal): Promise<SpaceGitStatusResponse>;
  spaceGitAction(sessionId: string, request: SpaceGitActionRequest): Promise<SpaceGitActionResponse>;
  focus(sessionId: string, request: FocusRequest): Promise<FocusResponse>;
  mutate(sessionId: string, request: ResourceMutationRequest): Promise<ResourceMutationResponse>;
  subscribeSession(sessionId: string, onMessage: (message: SessionStreamMessage) => void, onError: (error: CockpitClientError) => void, signal?: AbortSignal): Promise<ClosableStream>;
  openTerminal(request: TerminalOpenRequest, onMessage: (message: TerminalStreamMessage) => void, onError: (error: CockpitClientError) => void, signal?: AbortSignal): Promise<TerminalStream>;
  openBrowserView(request: BrowserViewOpenRequest, onEvent: BrowserViewEventHandler, onFrame: BrowserViewFrameHandler, onError: (error: CockpitClientError) => void, signal?: AbortSignal): Promise<BrowserViewStream>;
  subscribeWidgets(onEvent: WidgetEventHandler, onError: (error: CockpitClientError) => void, signal?: AbortSignal): Promise<WidgetStream>;
  widgetContent(request: WidgetContentRequest, signal?: AbortSignal): Promise<WidgetContent>;
  widgetRemove(request: WidgetRemoveRequest, signal?: AbortSignal): Promise<WidgetRemoveResponse>;
  widgetSelect(request: WidgetSelectRequest, signal?: AbortSignal): Promise<WidgetSelectResponse>;
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

function malformed(message: string): never {
  throw new CockpitClientError("malformed_response", message);
}
const isU64 = (value: unknown): value is number =>
  typeof value === "number" && Number.isSafeInteger(value) && value >= 0;

export function parseErrorEnvelope(value: unknown): ErrorResponse | undefined {
  return tryParseWire(value, Wire.wireErrorResponse, CLIENT);
}
export function parseStatusResponse(value: unknown): StatusResponse { return parseWire(value, Wire.wireStatusResponse, CLIENT); }
export function parseBrowserTarget(value: unknown): BrowserTarget { return parseWire(value, Wire.wireBrowserTarget, CLIENT); }
export function parseBrowserRequest(value: unknown): BrowserRequest { return parseWire(value, Wire.wireBrowserRequest, CLIENT); }
export function parseBrowserResponse(value: unknown): BrowserResponse { return parseWire(value, Wire.wireBrowserResponse, CLIENT); }
export function parseBrowserWorkScope(value: unknown): BrowserWorkScope { return parseWire(value, Wire.wireBrowserWorkScope, CLIENT); }
export function parseBrowserCleanupScope(value: unknown): BrowserCleanupScope { return parseWire(value, Wire.wireBrowserCleanupScope, CLIENT); }
export function parseBrowserCleanupFailure(value: unknown): BrowserCleanupFailure { return parseWire(value, Wire.wireBrowserCleanupFailure, CLIENT); }
export function parseBrowserCleanupStatus(value: unknown): BrowserCleanupStatus { return parseWire(value, Wire.wireBrowserCleanupStatus, CLIENT); }
export function parseBrowserCleanupRetryRequest(value: unknown): BrowserCleanupRetryRequest { return parseWire(value, Wire.wireBrowserCleanupRetryRequest, CLIENT); }
export function parseBrowserViewViewportRequest(value: unknown): BrowserViewViewportRequest { return parseWire(value, Wire.wireBrowserViewViewportRequest, CLIENT); }
export function parseBrowserViewOpenRequest(value: unknown): BrowserViewOpenRequest { return parseWire(value, Wire.wireBrowserViewOpenRequest, CLIENT); }
export function parseBrowserViewIdentity(value: unknown): BrowserViewIdentity { return parseWire(value, Wire.wireBrowserViewIdentity, CLIENT); }
export function parseBrowserViewTargetSummary(value: unknown): BrowserViewTargetSummary { return parseWire(value, Wire.wireBrowserViewTargetSummary, CLIENT); }
export function parseBrowserViewDocumentState(value: unknown): BrowserViewDocumentState { return parseWire(value, Wire.wireBrowserViewDocumentState, CLIENT); }
export function parseBrowserViewViewportState(value: unknown): BrowserViewViewportState { return parseWire(value, Wire.wireBrowserViewViewportState, CLIENT); }
export function parseBrowserViewNavigationState(value: unknown): BrowserViewNavigationState { return parseWire(value, Wire.wireBrowserViewNavigationState, CLIENT); }
export function parseBrowserViewCursorState(value: unknown): BrowserViewCursorState { return parseWire(value, Wire.wireBrowserViewCursorState, CLIENT); }
export function parseBrowserViewFocusState(value: unknown): BrowserViewFocusState { return parseWire(value, Wire.wireBrowserViewFocusState, CLIENT); }
export function parseBrowserViewBlocker(value: unknown): BrowserViewBlocker { return parseWire(value, Wire.wireBrowserViewBlocker, CLIENT); }
export function parseBrowserViewCapabilities(value: unknown): BrowserViewCapabilities { return parseWire(value, Wire.wireBrowserViewCapabilities, CLIENT); }
export function parseBrowserViewControlState(value: unknown): BrowserViewControlState { return parseWire(value, Wire.wireBrowserViewControlState, CLIENT); }
export function parseBrowserViewFrameEnvelope(value: unknown): BrowserViewFrameEnvelopeV2 { return parseWire(value, Wire.wireBrowserViewFrameEnvelopeV2, CLIENT); }
export function parseBrowserViewFrameGrant(value: unknown): BrowserViewFrameGrant { return parseWire(value, Wire.wireBrowserViewFrameGrant, CLIENT); }
export function parseBrowserViewFrameDescriptor(value: unknown): BrowserViewFrameDescriptor { return parseWire(value, Wire.wireBrowserViewFrameDescriptor, CLIENT); }
export function parseBrowserViewSnapshot(value: unknown): BrowserViewSnapshot { return parseWire(value, Wire.wireBrowserViewSnapshot, CLIENT); }
export function parseBrowserViewEventMetadata(value: unknown): BrowserViewEventMetadata { return parseWire(value, Wire.wireBrowserViewEventMetadata, CLIENT); }
export function parseBrowserViewEvent(value: unknown): BrowserViewEvent { return parseWire(value, Wire.wireBrowserViewEvent, CLIENT); }
export function parseBrowserDraftRecoveryRequest(value: unknown): BrowserDraftRecoveryRequest { return parseWire(value, Wire.wireBrowserDraftRecoveryRequest, CLIENT); }
export function parseBrowserViewCommand(value: unknown): BrowserViewCommand { return parseWire(value, Wire.wireBrowserViewCommand, CLIENT); }
export function parseBrowserViewCommandRequest(value: unknown): BrowserViewCommandRequest { return parseWire(value, Wire.wireBrowserViewCommandRequest, CLIENT); }
export function parseBrowserViewCommandOutcome(value: unknown): BrowserViewCommandOutcome { return parseWire(value, Wire.wireBrowserViewCommandOutcome, CLIENT); }
export function parseBrowserViewCommandResponse(value: unknown): BrowserViewCommandResponse { return parseWire(value, Wire.wireBrowserViewCommandResponse, CLIENT); }

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
export function parseBrowserFeedbackRequest(value: unknown): BrowserFeedbackRequest { return parseWire(value, Wire.wireBrowserFeedbackRequest, CLIENT); }
export function parseBrowserFeedbackAckRequest(value: unknown): BrowserFeedbackAckRequest { return parseWire(value, Wire.wireBrowserFeedbackAckRequest, CLIENT); }
export function parseBrowserFeedbackImageRequest(value: unknown): BrowserFeedbackImageRequest { return parseWire(value, Wire.wireBrowserFeedbackImageRequest, CLIENT); }
export function parseBrowserFeedbackSendRequest(value: unknown): BrowserFeedbackSendRequest { return parseWire(value, Wire.wireBrowserFeedbackSendRequest, CLIENT); }
export function parseBrowserFeedbackLookup(value: unknown): BrowserFeedbackLookup { return parseWire(value, Wire.wireBrowserFeedbackLookup, CLIENT); }
export function parseBrowserFeedbackImage(value: unknown): BrowserFeedbackImage { return parseWire(value, Wire.wireBrowserFeedbackImage, CLIENT); }
export function parseBrowserFeedbackSendResponse(value: unknown): BrowserFeedbackSendResponse { return parseWire(value, Wire.wireBrowserFeedbackSendResponse, CLIENT); }
export function parseBrowserFeedbackAck(value: unknown): BrowserFeedbackAck { return parseWire(value, Wire.wireBrowserFeedbackAck, CLIENT); }
export function parseSessionSnapshotResponse(value: unknown): SessionSnapshotResponse { return parseWire(value, Wire.wireSessionSnapshotResponse, CLIENT); }
export function parseSpaceGitStatusResponse(value: unknown): SpaceGitStatusResponse { return parseWire(value, Wire.wireSpaceGitStatusResponse, CLIENT); }
export function parseSpaceGitActionResponse(value: unknown): SpaceGitActionResponse { return parseWire(value, Wire.wireSpaceGitActionResponse, CLIENT); }

export function matchSpaceGitActionResponse(value: unknown, sessionId: string, request: SpaceGitActionRequest): SpaceGitActionResponse {
  const response = parseSpaceGitActionResponse(value);
  if (response.session_id !== sessionId || response.space_id !== request.space_id || response.action !== request.action
    || response.root !== request.expected_root || response.branch !== request.expected_branch || response.upstream !== request.expected_upstream) {
    return malformed("Space Git action response belongs to another target");
  }
  return response;
}

export function parseSessionSummary(value: unknown): SessionSummary { return parseWire(value, Wire.wireSessionSummary, CLIENT); }
export function parseSessionListResponse(value: unknown): SessionListResponse { return parseWire(value, Wire.wireSessionListResponse, CLIENT); }
export function parseFocusRequest(value: unknown): FocusRequest { return parseWire(value, Wire.wireFocusRequest, CLIENT); }
export function parseFocusResponse(value: unknown): FocusResponse { return parseWire(value, Wire.wireFocusResponse, CLIENT); }
export function parseResourceMutationRequest(value: unknown): ResourceMutationRequest { return parseWire(value, Wire.wireResourceMutationRequest, CLIENT); }
export function parseCreatedPane(value: unknown): CreatedPane { return parseWire(value, Wire.wireCreatedPane, CLIENT); }
export function parseResourceMutationResponse(value: unknown): ResourceMutationResponse { return parseWire(value, Wire.wireResourceMutationResponse, CLIENT); }
export function parseSessionStreamMessage(value: unknown): SessionStreamMessage { return parseWire(value, Wire.wireSessionStreamMessage, CLIENT); }
export function parseTerminalOpenRequest(value: unknown): TerminalOpenRequest { return parseWire(value, Wire.wireTerminalOpenRequest, CLIENT); }
export function parseTerminalCommand(value: unknown): TerminalCommand { return parseWire(value, Wire.wireTerminalCommand, CLIENT); }
export function parseTerminalStreamMessage(value: unknown): TerminalStreamMessage { return parseWire(value, Wire.wireTerminalStreamMessage, CLIENT); }

export function validateSessionId(sessionId: string): string {
  if (typeof sessionId !== "string" || sessionId.length === 0 || sessionId.length > 96 || !/^[A-Za-z0-9_-]+$/.test(sessionId)) malformed("Session id must be a valid name");
  return sessionId;
}

export function validateResourceId(resourceId: string): string {
  if (typeof resourceId !== "string" || resourceId.length === 0 || resourceId.length > 128 || !/^[A-Za-z0-9:_-]+$/.test(resourceId)) malformed("Resource id must be a valid Herdr identifier");
  return resourceId;
}
