use crate::browser::{
    BrowserAction, BrowserAssociation, BrowserConnectionState, BrowserFeedbackAckRequest,
    BrowserFeedbackDeliveryStatus, BrowserFeedbackImage, BrowserFeedbackImageRequest,
    BrowserFeedbackLookup, BrowserFeedbackRequest, BrowserFeedbackSendRequest,
    BrowserFeedbackSendResponse, BrowserRequest, BrowserResponse, BrowserTarget,
};
use crate::browser_view::{
    BrowserViewBlocker, BrowserViewBlockerKind, BrowserViewCapabilities, BrowserViewCapability,
    BrowserViewCaptureCommand, BrowserViewClipboardCommand, BrowserViewCommand,
    BrowserViewCommandOutcome, BrowserViewCommandRequest, BrowserViewCommandResponse,
    BrowserViewCompositionInput, BrowserViewCompositionKind, BrowserViewControlState,
    BrowserViewControlStatus, BrowserViewCursor, BrowserViewCursorState, BrowserViewDialogCommand,
    BrowserViewDocumentCommandContext, BrowserViewDocumentState, BrowserViewDownloadCommand,
    BrowserDraftRecoveryAction, BrowserDraftRecoveryRequest, BrowserViewCaptureOutcome,
    BrowserViewDraftAnnotation, BrowserViewDraftCommand,
    BrowserViewDraftEditorState, BrowserViewDraftInventory, BrowserViewDraftState,
    BrowserViewEvent, BrowserViewPendingCapture,
    BrowserViewEventMetadata, BrowserViewFileCommand, BrowserViewFocusState,
    BrowserViewFrameDescriptor, BrowserViewFrameEnvelopeV2, BrowserViewFrameGrant,
    BrowserViewIdentity, BrowserViewInspectCommand, BrowserViewInspectResult,
    BrowserViewInspectionFreshness, BrowserViewKeyKind, BrowserViewKeyboardInput,
    BrowserViewLocation, BrowserViewNavigationCommand, BrowserViewNavigationState,
    BrowserViewOpenRequest, BrowserViewPermissionCommand, BrowserViewPermissionDecision,
    BrowserViewPointerButton, BrowserViewPointerInput, BrowserViewPointerKind,
    BrowserViewPresentation, BrowserViewSnapshot, BrowserViewTabCommand, BrowserViewTargetKind,
    BrowserViewTargetSummary, BrowserViewTextInput, BrowserViewViewportRequest,
    BrowserViewViewportState, BrowserViewWheelInput,
};

use crate::browser_feedback::{
    BrowserAnnotation, BrowserAnnotationKind, BrowserCaptureContext, BrowserCaptureSaved,
    BrowserCaptureSubmission, BrowserElementEvidence, BrowserFeedbackAck, BrowserFeedbackCapture,
    BrowserFeedbackResponse, BrowserInlineCaptureProvenance, BrowserPageEvidence, BrowserPoint,
    BrowserRect, BrowserViewport,
};

use crate::comment_paste::{
    CommentPasteMarkPastedRequest, CommentPastePrepareRequest, CommentPastePrepareResponse,
    CommentPasteReceipt, CommentPasteSendRequest, CommentPasteState, CommentPasteTarget,
};
use crate::context_assets::{
    ContextSnapshotCopyMode, ContextSnapshotMode, ContextSnapshotRequest, ContextSnapshotResponse,
};
use crate::context_media::{ContextMedia, ContextMediaRequest};
use crate::context_search::{
    ContextInvalidation, ContextInvalidationRequest, ContextInvalidationResponse,
    ContextInvalidationState, ContextKnownRevision, ContextSearchRequest, ContextSearchResponse,
    ContextSearchResult,
};
use crate::project_teardown::{
    WorkspaceTeardownAction, WorkspaceTeardownCompanionState, WorkspaceTeardownDirtyState,
    WorkspaceTeardownExecuteRequest, WorkspaceTeardownOutcome, WorkspaceTeardownOwnership,
    WorkspaceTeardownPreview, WorkspaceTeardownPreviewRequest, WorkspaceTeardownRecovery,
    WorkspaceTeardownRecoveryList, WorkspaceTeardownRecoveryState, WorkspaceTeardownResult,
    WorkspaceTeardownWorkspaceState,
};
use crate::review::{
    ReviewChangedFile, ReviewComparison, ReviewDiffLine, ReviewDiffLineKind, ReviewFileDiff,
    ReviewFileRequest, ReviewFileStatus, ReviewHunk, ReviewSide, ReviewSnapshot,
    ReviewSnapshotRequest,
};
use crate::sources::{
    SourceCapability, SourceEntry, SourceFreshness, SourceImportRequest, SourceImportResponse,
    SourceListRequest, SourceMaterializationStatus, SourceRefreshRequest,
};
use std::fs::{self, OpenOptions};
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::process;

use crate::comments::{
    CommentAnchor, CommentAttachment, CommentBatch, CommentBatchList, CommentBatchMutation,
    CommentBatchRequest, CommentBatchSummary, CommentCapture, CommentDraft, CommentFileRef,
    CommentLocation, CommentOwner, CommentPreview, CommentPreviewRequest, CommentRemoveRequest,
    CommentRequestScope, CommentReviewRef, CommentSourceState, CommentUpsertRequest,
};
use crate::context::{
    ContextDirectory, ContextDirectoryRequest, ContextDocument, ContextDocumentRequest,
    ContextEntry, ContextEntryKind, ContextLaunchRequest, ContextRoot, ContextRootKind,
    ContextSplitDirection, DetectionConfidence, ExtensionKind, PanePresentation,
    ReviewLaunchRequest,
};
use crate::project_defaults::{WorkspaceDefaults, WorkspaceDefaultsRequest};
use crate::projects::{
    ProjectArtifact, ProjectConfiguration, ProjectDiagnostic, ProjectLimits, ProjectProvider,
    RepositoryCandidate, RepositoryListResponse, WorkspaceCheckoutOwnership, WorkspaceOperation,
    WorkspaceOperationRequest, WorkspaceOperationState, WorkspaceOperationStep,
    WorkspaceOwnedResource, WorkspaceReconcileRequest, WorkspaceRecoveryAction, WorkspaceSetupMode,
    WorkspaceSetupPlan, WorkspaceSetupRequest,
};
use ts_rs::{Config, TS};

use crate::v1::{
    AgentSummary, CockpitCapabilities, CockpitMode, ErrorResponse, FocusKind, FocusRequest,
    FocusResponse, HerdrCompatibility, HerdrIdentity, LayoutPane, LayoutRect, PaneMoveDestination,
    PaneOutputResponse, PaneResizeDirection, PaneSplitDirection, PaneSummary, PaneZoomMode,
    ResourceMutationRequest, ResourceMutationResponse, SessionListResponse,
    SessionSnapshotResponse, SessionStreamMessage, SessionSummary, SpaceGitSummary, SpaceSummary,
    StatusResponse, TabLayout, TabSummary, TerminalCommand, TerminalMode, TerminalMouseButton,
    TerminalMouseKind, TerminalOpenRequest, TerminalOwnershipState, TerminalScrollDirection,
    TerminalScrollSource, TerminalStreamMessage,
};

const HEADER: &str = "// This file was generated by [ts-rs](https://github.com/Aleph-Alpha/ts-rs). Do not edit this file manually.";

/// Render every v1 DTO in a stable order using the declarations generated by ts-rs.
pub fn render_v1() -> String {
    let config = Config::default();
    let declarations = [
        CockpitMode::decl(&config),
        HerdrIdentity::decl(&config),
        HerdrCompatibility::decl(&config),
        CockpitCapabilities::decl(&config),
        StatusResponse::decl(&config),
        ErrorResponse::decl(&config),
        BrowserTarget::decl(&config),
        BrowserAction::decl(&config),
        BrowserRequest::decl(&config),
        BrowserConnectionState::decl(&config),
        BrowserAssociation::decl(&config),
        BrowserResponse::decl(&config),
        BrowserFeedbackRequest::decl(&config),
        BrowserFeedbackAckRequest::decl(&config),
        BrowserFeedbackDeliveryStatus::decl(&config),
        BrowserFeedbackLookup::decl(&config),
        BrowserFeedbackImageRequest::decl(&config),
        BrowserFeedbackImage::decl(&config),
        BrowserFeedbackSendRequest::decl(&config),
        BrowserFeedbackSendResponse::decl(&config),
        BrowserPoint::decl(&config),
        BrowserRect::decl(&config),
        BrowserViewport::decl(&config),
        BrowserElementEvidence::decl(&config),
        BrowserAnnotationKind::decl(&config),
        BrowserAnnotation::decl(&config),
        BrowserPageEvidence::decl(&config),
        BrowserCaptureSubmission::decl(&config),
        BrowserCaptureContext::decl(&config),
        BrowserInlineCaptureProvenance::decl(&config),
        BrowserFeedbackCapture::decl(&config),
        BrowserCaptureSaved::decl(&config),
        BrowserFeedbackResponse::decl(&config),
        BrowserFeedbackAck::decl(&config),
        BrowserViewPresentation::decl(&config),
        BrowserViewViewportRequest::decl(&config),
        BrowserViewOpenRequest::decl(&config),
        BrowserViewIdentity::decl(&config),
        BrowserViewTargetKind::decl(&config),
        BrowserViewTargetSummary::decl(&config),
        BrowserViewDocumentState::decl(&config),
        BrowserViewViewportState::decl(&config),
        BrowserViewNavigationState::decl(&config),
        BrowserViewCursor::decl(&config),
        BrowserViewCursorState::decl(&config),
        BrowserViewFocusState::decl(&config),
        BrowserViewBlockerKind::decl(&config),
        BrowserViewBlocker::decl(&config),
        BrowserViewCapability::decl(&config),
        BrowserViewCapabilities::decl(&config),
        BrowserViewControlStatus::decl(&config),
        BrowserViewControlState::decl(&config),
        BrowserViewFrameEnvelopeV2::decl(&config),
        BrowserViewFrameGrant::decl(&config),
        BrowserViewFrameDescriptor::decl(&config),
        BrowserViewSnapshot::decl(&config),
        BrowserViewEventMetadata::decl(&config),
        BrowserViewEvent::decl(&config),
        BrowserViewLocation::decl(&config),
        BrowserViewDocumentCommandContext::decl(&config),
        BrowserViewPointerKind::decl(&config),
        BrowserViewPointerButton::decl(&config),
        BrowserViewPointerInput::decl(&config),
        BrowserViewWheelInput::decl(&config),
        BrowserViewKeyKind::decl(&config),
        BrowserViewKeyboardInput::decl(&config),
        BrowserViewTextInput::decl(&config),
        BrowserViewCompositionKind::decl(&config),
        BrowserViewCompositionInput::decl(&config),
        BrowserViewClipboardCommand::decl(&config),
        BrowserViewNavigationCommand::decl(&config),
        BrowserViewTabCommand::decl(&config),
        BrowserViewDialogCommand::decl(&config),
        BrowserViewFileCommand::decl(&config),
        BrowserViewDownloadCommand::decl(&config),
        BrowserViewPermissionDecision::decl(&config),
        BrowserViewPermissionCommand::decl(&config),
        BrowserViewInspectionFreshness::decl(&config),
        BrowserViewInspectResult::decl(&config),
        BrowserViewInspectCommand::decl(&config),
        BrowserViewCaptureCommand::decl(&config),
        BrowserViewDraftAnnotation::decl(&config),
        BrowserViewDraftEditorState::decl(&config),
        BrowserViewDraftState::decl(&config),
        BrowserViewDraftInventory::decl(&config),
        BrowserViewPendingCapture::decl(&config),
        BrowserViewCaptureOutcome::decl(&config),
        BrowserViewDraftCommand::decl(&config),
        BrowserDraftRecoveryAction::decl(&config),
        BrowserDraftRecoveryRequest::decl(&config),
        BrowserViewCommand::decl(&config),
        BrowserViewCommandRequest::decl(&config),
        BrowserViewCommandOutcome::decl(&config),
        BrowserViewCommandResponse::decl(&config),
        SpaceGitSummary::decl(&config),
        SpaceSummary::decl(&config),
        TabSummary::decl(&config),
        PaneSummary::decl(&config),
        AgentSummary::decl(&config),
        LayoutRect::decl(&config),
        LayoutPane::decl(&config),
        TabLayout::decl(&config),
        SessionSnapshotResponse::decl(&config),
        PaneOutputResponse::decl(&config),
        SessionSummary::decl(&config),
        SessionListResponse::decl(&config),
        FocusKind::decl(&config),
        FocusRequest::decl(&config),
        FocusResponse::decl(&config),
        PaneSplitDirection::decl(&config),
        PaneResizeDirection::decl(&config),
        PaneZoomMode::decl(&config),
        PaneMoveDestination::decl(&config),
        ResourceMutationRequest::decl(&config),
        ResourceMutationResponse::decl(&config),
        SessionStreamMessage::decl(&config),
        TerminalMode::decl(&config),
        TerminalOpenRequest::decl(&config),
        TerminalScrollDirection::decl(&config),
        TerminalScrollSource::decl(&config),
        TerminalMouseButton::decl(&config),
        TerminalMouseKind::decl(&config),
        TerminalCommand::decl(&config),
        TerminalOwnershipState::decl(&config),
        TerminalStreamMessage::decl(&config),
        ProjectLimits::decl(&config),
        ProjectProvider::decl(&config),
        ProjectConfiguration::decl(&config),
        RepositoryCandidate::decl(&config),
        ProjectDiagnostic::decl(&config),
        RepositoryListResponse::decl(&config),
        WorkspaceDefaultsRequest::decl(&config),
        WorkspaceDefaults::decl(&config),
        WorkspaceSetupMode::decl(&config),
        WorkspaceCheckoutOwnership::decl(&config),
        WorkspaceSetupRequest::decl(&config),
        ProjectArtifact::decl(&config),
        WorkspaceSetupPlan::decl(&config),
        WorkspaceOperationRequest::decl(&config),
        WorkspaceRecoveryAction::decl(&config),
        WorkspaceReconcileRequest::decl(&config),
        WorkspaceOperationState::decl(&config),
        WorkspaceOperationStep::decl(&config),
        WorkspaceOwnedResource::decl(&config),
        WorkspaceOperation::decl(&config),
        ExtensionKind::decl(&config),
        DetectionConfidence::decl(&config),
        ContextRootKind::decl(&config),
        ContextRoot::decl(&config),
        PanePresentation::decl(&config),
        ContextDirectoryRequest::decl(&config),
        ContextEntryKind::decl(&config),
        ContextEntry::decl(&config),
        ContextDirectory::decl(&config),
        ContextDocumentRequest::decl(&config),
        ContextDocument::decl(&config),
        ContextMediaRequest::decl(&config),
        ContextMedia::decl(&config),
        ContextSplitDirection::decl(&config),
        ContextLaunchRequest::decl(&config),
        ReviewLaunchRequest::decl(&config),
        CommentRequestScope::decl(&config),
        CommentOwner::decl(&config),
        CommentLocation::decl(&config),
        CommentAttachment::decl(&config),
        CommentFileRef::decl(&config),
        CommentSourceState::decl(&config),
        CommentReviewRef::decl(&config),
        CommentAnchor::decl(&config),
        CommentDraft::decl(&config),
        CommentBatch::decl(&config),
        CommentBatchSummary::decl(&config),
        CommentBatchList::decl(&config),
        CommentBatchRequest::decl(&config),
        CommentBatchMutation::decl(&config),
        CommentCapture::decl(&config),
        CommentUpsertRequest::decl(&config),
        CommentRemoveRequest::decl(&config),
        CommentPreviewRequest::decl(&config),
        CommentPreview::decl(&config),
        CommentPasteTarget::decl(&config),
        CommentPasteState::decl(&config),
        CommentPastePrepareRequest::decl(&config),
        CommentPastePrepareResponse::decl(&config),
        CommentPasteSendRequest::decl(&config),
        CommentPasteMarkPastedRequest::decl(&config),
        CommentPasteReceipt::decl(&config),
        WorkspaceTeardownAction::decl(&config),
        WorkspaceTeardownOwnership::decl(&config),
        WorkspaceTeardownDirtyState::decl(&config),
        WorkspaceTeardownWorkspaceState::decl(&config),
        WorkspaceTeardownCompanionState::decl(&config),
        WorkspaceTeardownPreviewRequest::decl(&config),
        WorkspaceTeardownExecuteRequest::decl(&config),
        WorkspaceTeardownPreview::decl(&config),
        WorkspaceTeardownOutcome::decl(&config),
        WorkspaceTeardownResult::decl(&config),
        WorkspaceTeardownRecoveryState::decl(&config),
        WorkspaceTeardownRecovery::decl(&config),
        WorkspaceTeardownRecoveryList::decl(&config),
        SourceCapability::decl(&config),
        SourceFreshness::decl(&config),
        SourceMaterializationStatus::decl(&config),
        SourceImportRequest::decl(&config),
        SourceListRequest::decl(&config),
        SourceRefreshRequest::decl(&config),
        SourceEntry::decl(&config),
        SourceImportResponse::decl(&config),
        ReviewSide::decl(&config),
        ReviewComparison::decl(&config),
        ReviewFileStatus::decl(&config),
        ReviewDiffLineKind::decl(&config),
        ReviewSnapshotRequest::decl(&config),
        ReviewChangedFile::decl(&config),
        ReviewSnapshot::decl(&config),
        ReviewFileRequest::decl(&config),
        ReviewDiffLine::decl(&config),
        ReviewHunk::decl(&config),
        ReviewFileDiff::decl(&config),
        ContextSnapshotRequest::decl(&config),
        ContextSnapshotResponse::decl(&config),
        ContextSnapshotMode::decl(&config),
        ContextSnapshotCopyMode::decl(&config),
        ContextSearchRequest::decl(&config),
        ContextSearchResult::decl(&config),
        ContextSearchResponse::decl(&config),
        ContextKnownRevision::decl(&config),
        ContextInvalidationState::decl(&config),
        ContextInvalidation::decl(&config),
        ContextInvalidationRequest::decl(&config),
        ContextInvalidationResponse::decl(&config),
    ]
    .into_iter()
    .map(|declaration| format!("export {declaration}"))
    .collect::<Vec<_>>()
    .join("\n\n");

    format!(
        "{HEADER}\n\n{}\n",
        declarations
            .lines()
            .map(str::trim_end)
            .collect::<Vec<_>>()
            .join("\n")
    )
}

/// Replace `path` with `contents` using a same-directory temporary file and rename.
pub fn write_atomic(path: &Path, contents: &[u8]) -> io::Result<()> {
    let parent = path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."));
    fs::create_dir_all(parent)?;

    let file_name = path
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("generated.ts");
    let process_id = process::id();
    let mut temporary_path = PathBuf::new();
    let mut temporary_file = None;

    for attempt in 0..100u32 {
        let candidate = parent.join(format!(".{file_name}.{process_id}.{attempt}.tmp"));
        match OpenOptions::new()
            .create_new(true)
            .write(true)
            .open(&candidate)
        {
            Ok(file) => {
                temporary_path = candidate;
                temporary_file = Some(file);
                break;
            }
            Err(error) if error.kind() == io::ErrorKind::AlreadyExists => continue,
            Err(error) => return Err(error),
        }
    }

    let mut file = temporary_file.ok_or_else(|| {
        io::Error::new(
            io::ErrorKind::AlreadyExists,
            "could not allocate a temporary path for atomic replacement",
        )
    })?;
    let result = (|| {
        file.write_all(contents)?;
        file.sync_all()?;
        drop(file);
        fs::rename(&temporary_path, path)
    })();

    if result.is_err() {
        let _ = fs::remove_file(&temporary_path);
    }
    result
}

/// Return whether `path` contains exactly the generated v1 bytes.
pub fn check(path: &Path) -> io::Result<bool> {
    match fs::read(path) {
        Ok(actual) => Ok(actual == render_v1().as_bytes()),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(false),
        Err(error) => Err(error),
    }
}
