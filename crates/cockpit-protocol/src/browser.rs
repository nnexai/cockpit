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

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize, TS)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum BrowserCleanupScope {
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
    pub failures: Vec<BrowserCleanupFailure>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize, TS)]
#[serde(deny_unknown_fields)]
pub struct BrowserCleanupRetryRequest {
    pub association_key: String,
}

/// Browser work belongs to a live tab for the current owner run.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize, TS)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum BrowserWorkScope {
    Tab { target: BrowserTarget },
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

/// Current-run delivery outcome for one pending capture. Lookup never retries paste.
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
