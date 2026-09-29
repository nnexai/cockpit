use serde::{Deserialize, Serialize};
use ts_rs::TS;

/// Authoritative Herdr evidence that identifies exactly one eligible tab.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize, TS)]
#[serde(deny_unknown_fields)]
pub struct BrowserTarget {
    pub session_id: String,
    pub tab_id: Option<String>,
    pub pane_id: Option<String>,
    pub endpoint_path: Option<String>,
}

/// A tab-scoped browser lifecycle operation.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize, TS)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum BrowserAction {
    Open { url: Option<String> },
    OpenFresh { url: Option<String> },
    Status,
    Close,
    Cleanup,
}

/// A browser operation and the Herdr identity it must revalidate.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize, TS)]
#[serde(deny_unknown_fields)]
pub struct BrowserRequest {
    pub target: BrowserTarget,
    pub action: BrowserAction,
}

/// The freshly inspected state of Cockpit's associated CLI session.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize, TS)]
#[serde(rename_all = "snake_case")]
pub enum BrowserConnectionState {
    Absent,
    Open,
    Closed,
    Disconnected,
    OutcomeUnknown,
}

/// Safe addressing information for a Cockpit-owned browser session.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize, TS)]
#[serde(deny_unknown_fields)]
pub struct BrowserAssociation {
    pub association_key: String,
    pub owner_id: String,
    pub session_id: String,
    pub space_id: String,
    pub space_label: String,
    pub tab_id: String,
    pub tab_label: String,
    pub playwright_session: String,
    pub working_directory: String,
    pub profile_path: String,
    pub invocation: String,
    pub connection: BrowserConnectionState,
    pub incarnation: Option<String>,
    pub opened_tab: Option<String>,
}

/// The stable result envelope for browser lifecycle actions.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize, TS)]
#[serde(deny_unknown_fields)]
pub struct BrowserResponse {
    pub association: Option<BrowserAssociation>,
    pub connection: BrowserConnectionState,
    pub message: String,
    pub cleanup: BrowserCleanupState,
    pub cleanup_reason: Option<String>,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize, TS)]
#[serde(rename_all = "snake_case")]
pub enum BrowserCleanupState {
    None,
    Pending,
    Done,
    Failed,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize, TS)]
#[serde(rename_all = "snake_case")]
pub enum BrowserCutoverState {
    NotNeeded,
    Running,
    Done,
    Failed,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize, TS)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum BrowserCleanupScope {
    LegacySpace { space_id: String },
    Tab { session_id: String, tab_id: String },
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize, TS)]
#[serde(deny_unknown_fields)]
pub struct BrowserCleanupFailure {
    pub association_key: String,
    pub scope: BrowserCleanupScope,
    pub reason: String,
    pub unproven_paths: Vec<String>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize, TS)]
#[serde(deny_unknown_fields)]
pub struct BrowserCleanupStatus {
    pub cutover: BrowserCutoverState,
    pub failures: Vec<BrowserCleanupFailure>,
    pub saved_tabs: Vec<BrowserSavedTabWork>,
}

/// Durable saved-work provenance remains discoverable after tab retirement.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize, TS)]
#[serde(deny_unknown_fields)]
pub struct BrowserSavedTabWork {
    pub association_key: String,
    pub session_id: String,
    pub tab_id: String,
    pub tab_label: String,
    pub space_id: String,
    pub space_label: String,
    pub saved_capture_count: u32,
    pub draft_count: u32,
    pub pending_capture: bool,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize, TS)]
#[serde(deny_unknown_fields)]
pub struct BrowserCleanupRetryRequest {
    pub association_key: String,
}

/// Durable browser work keeps its original identity across the tab cutover.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize, TS)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum BrowserWorkScope {
    Tab { target: BrowserTarget },
    LegacyArchive { association_key: String },
    SavedTab { association_key: String },
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize, TS)]
#[serde(rename_all = "snake_case")]
pub enum BrowserLegacyCandidateKind {
    Directory,
    File,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize, TS)]
#[serde(rename_all = "snake_case")]
pub enum BrowserLegacyCandidateState {
    Pending,
    Kept,
    Removed,
    Changed,
}

/// An exact no-follow object manifest reviewed before explicit removal.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize, TS)]
#[serde(deny_unknown_fields)]
pub struct BrowserLegacyArtifactCandidate {
    pub path: String,
    pub kind: BrowserLegacyCandidateKind,
    pub dev: String,
    pub inode: String,
    #[ts(type = "number")]
    pub entry_count: u64,
    #[ts(type = "number")]
    pub total_bytes: u64,
    pub captured_at: String,
    pub state: BrowserLegacyCandidateState,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize, TS)]
#[serde(deny_unknown_fields)]
pub struct BrowserLegacyArchive {
    pub association_key: String,
    pub session_id: String,
    pub space_id: String,
    pub space_label: String,
    pub archived_at: String,
    pub session_stopped: bool,
    pub saved_capture_count: u32,
    pub draft_count: u32,
    pub pending_capture: bool,
    pub candidates: Vec<BrowserLegacyArtifactCandidate>,
    pub not_candidates: Vec<String>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize, TS)]
#[serde(deny_unknown_fields)]
pub struct BrowserLegacyArchiveList {
    pub archives: Vec<BrowserLegacyArchive>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize, TS)]
#[serde(deny_unknown_fields)]
pub struct BrowserLegacyRemovalRequest {
    pub association_key: String,
    pub candidates: Vec<BrowserLegacyArtifactCandidate>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize, TS)]
#[serde(deny_unknown_fields)]
pub struct BrowserLegacyKeepRequest {
    pub association_key: String,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize, TS)]
#[serde(deny_unknown_fields)]
pub struct BrowserLegacyRecipientsRequest {
    pub session_id: String,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize, TS)]
#[serde(deny_unknown_fields)]
pub struct BrowserFeedbackRequest {
    pub scope: BrowserWorkScope,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize, TS)]
#[serde(deny_unknown_fields)]
pub struct BrowserFeedbackAckRequest {
    pub scope: BrowserWorkScope,
    pub ids: Vec<String>,
}

/// Durable delivery outcome for one pending saved capture. Lookup never retries paste.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize, TS)]
#[serde(deny_unknown_fields)]
pub struct BrowserFeedbackDeliveryStatus {
    pub capture_id: String,
    pub operation_id: String,
    pub selected_ids: Vec<String>,
    pub state: crate::comment_paste::CommentPasteState,
    pub message: String,
}

#[derive(Clone, Debug, Deserialize, Serialize, TS)]
#[serde(deny_unknown_fields)]
pub struct BrowserFeedbackLookup {
    pub browser: BrowserResponse,
    pub feedback: crate::browser_feedback::BrowserFeedbackResponse,
    #[serde(default)]
    pub deliveries: Vec<BrowserFeedbackDeliveryStatus>,
    #[serde(default)]
    pub drafts: Option<crate::browser_view::BrowserViewDraftInventory>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize, TS)]
#[serde(deny_unknown_fields)]
pub struct BrowserFeedbackImageRequest {
    pub scope: BrowserWorkScope,
    pub capture_id: String,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize, TS)]
#[serde(deny_unknown_fields)]
pub struct BrowserFeedbackImage {
    pub mime_type: String,
    pub data_base64: String,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize, TS)]
#[serde(deny_unknown_fields)]
pub struct BrowserFeedbackSendRequest {
    pub scope: BrowserWorkScope,
    pub ids: Vec<String>,
    pub operation_id: String,
    pub acknowledge_duplicate_risk: bool,
    pub recipient: Option<crate::comment_paste::CommentPasteTarget>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize, TS)]
#[serde(deny_unknown_fields)]
pub struct BrowserFeedbackSendResponse {
    pub operation_id: String,
    pub state: crate::comment_paste::CommentPasteState,
    pub target: Option<crate::comment_paste::CommentPasteTarget>,
    pub acknowledged_ids: Vec<String>,
    pub pending_count: usize,
    pub message: String,
}
