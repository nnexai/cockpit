use serde::{Deserialize, Serialize};
use ts_rs::TS;

pub const WIDGET_MAX_HTML_BYTES: usize = 1024 * 1024;
pub const WIDGET_MAX_CHOICES_BYTES: usize = 64 * 1024;
pub const WIDGET_MAX_SELECTION_BYTES: usize = 16 * 1024;
pub const WIDGET_MAX_LIVE_PER_TAB: usize = 8;
pub const WIDGET_MAX_TOMBSTONES_PER_TAB: usize = 64;
pub const WIDGET_MAX_TOTAL_HTML_BYTES: usize = 64 * 1024 * 1024;
pub const WIDGET_MAX_SNAPSHOT_BYTES: usize = 8 * 1024 * 1024;
pub const WIDGET_MAX_ARRIVALS_PER_MINUTE: usize = 6;
pub const WIDGET_MAX_SELECTION_WAITERS: usize = 8;
pub const WIDGET_MAX_WAIT_SECONDS: u64 = 3600;
pub const WIDGET_DEFAULT_WAIT_SECONDS: u64 = 300;

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize, TS)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum WidgetLocator {
    CurrentPane,
    Pane { pane_id: String },
    Tab { tab_id: String },
    Space { space_id: String },
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize, TS)]
pub struct WidgetAddress {
    pub session_id: String,
    pub endpoint_path: Option<String>,
    pub source_pane_id: Option<String>,
    pub locator: WidgetLocator,
    pub space_check: Option<String>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize, TS)]
#[serde(rename_all = "snake_case")]
pub enum WidgetInputKind {
    File,
    Stdin,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize, TS)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum WidgetContentInput {
    Html {
        content_base64: String,
        sha256: String,
        from: WidgetInputKind,
        name: Option<String>,
    },
    Choices {
        spec_json: String,
        sha256: String,
        from: WidgetInputKind,
        name: Option<String>,
    },
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize, TS)]
pub struct WidgetShowRequest {
    pub address: WidgetAddress,
    pub id: String,
    pub title: Option<String>,
    pub content: WidgetContentInput,
    pub reopen: bool,
    pub clear_selection: bool,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize, TS)]
#[serde(rename_all = "snake_case")]
pub enum WidgetShowResult {
    Opened,
    Replaced,
    Unchanged,
    Reopened,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize, TS)]
#[serde(rename_all = "snake_case")]
pub enum WidgetDisplayed {
    Now,
    WhenVisible,
    WhenTabSelected,
    WhenOpened,
    NoWindow,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize, TS)]
#[serde(rename_all = "snake_case")]
pub enum WidgetPresentation {
    Active,
    Choices,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize, TS)]
#[serde(rename_all = "snake_case")]
pub enum WidgetResolvedFrom {
    CurrentPane,
    Pane,
    Tab,
    SpaceFocusedTab,
    Stored,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize, TS)]
pub struct WidgetContentFacts {
    pub sha256: String,
    #[ts(type = "number")]
    pub bytes: u64,
    pub from: WidgetInputKind,
    pub name: Option<String>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize, TS)]
pub struct WidgetSourceFacts {
    pub pane_id: String,
    pub tab_id: String,
    pub space_id: String,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize, TS)]
pub struct WidgetTargetFacts {
    pub session_id: String,
    pub space_id: String,
    pub tab_id: String,
    pub resolved_from: WidgetResolvedFrom,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize, TS)]
pub struct WidgetShowResponse {
    pub id: String,
    #[ts(type = "number")]
    pub revision: u64,
    pub result: WidgetShowResult,
    pub displayed: WidgetDisplayed,
    pub location: String,
    pub presentation: WidgetPresentation,
    pub content: WidgetContentFacts,
    pub source: Option<WidgetSourceFacts>,
    pub target: WidgetTargetFacts,
    pub warnings: Vec<String>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize, TS)]
pub struct WidgetCloseRequest {
    pub address: WidgetAddress,
    pub id: String,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize, TS)]
#[serde(rename_all = "snake_case")]
pub enum WidgetCloseResult {
    Closed,
    AlreadyRemoved,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize, TS)]
pub struct WidgetCloseResponse {
    pub id: String,
    pub result: WidgetCloseResult,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize, TS)]
pub struct WidgetListRequest {
    pub address: WidgetAddress,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize, TS)]
#[serde(rename_all = "snake_case")]
pub enum WidgetListState {
    Live,
    RemovedByUser,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize, TS)]
pub struct WidgetListEntry {
    pub id: String,
    pub state: WidgetListState,
    #[ts(type = "number")]
    pub revision: u64,
    pub presentation: Option<WidgetPresentation>,
    pub displayed: Option<WidgetDisplayed>,
    #[ts(type = "number | null")]
    pub removed_at_ms: Option<u64>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize, TS)]
pub struct WidgetListResponse {
    pub widgets: Vec<WidgetListEntry>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize, TS)]
pub struct WidgetSelectionRequest {
    pub address: WidgetAddress,
    pub id: String,
    #[ts(type = "number | null")]
    pub wait_seconds: Option<u64>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize, TS)]
#[serde(rename_all = "snake_case")]
pub enum WidgetSelectionStatus {
    None,
    Selected,
    Timeout,
    Dismissed,
    Retired,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize, TS)]
pub struct WidgetSelectionResponse {
    pub id: String,
    #[ts(type = "number | null")]
    pub revision: Option<u64>,
    pub status: WidgetSelectionStatus,
    pub value_json: Option<String>,
    #[ts(type = "number | null")]
    pub at_ms: Option<u64>,
    #[ts(type = "number | null")]
    pub removed_at_ms: Option<u64>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize, TS)]
pub struct WidgetKey {
    pub session_id: String,
    pub tab_id: String,
    pub id: String,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize, TS)]
#[serde(rename_all = "snake_case")]
pub enum WidgetKind {
    Html,
    Choices,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize, TS)]
#[serde(rename_all = "snake_case")]
pub enum WidgetArrival {
    OwnTab,
    CrossSource,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize, TS)]
#[serde(rename_all = "snake_case")]
pub enum WidgetChange {
    Opened,
    Replaced,
    Reopened,
    Updated,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize, TS)]
#[serde(rename_all = "snake_case")]
pub enum WidgetSourceStatus {
    Present,
    Closed,
    Restarted,
    Unknown,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize, TS)]
pub struct WidgetSourceSummary {
    pub pane_id: String,
    pub tab_id: String,
    pub space_id: String,
    pub terminal_id: String,
    pub agent_label: Option<String>,
    pub fingerprint_prefix: Option<String>,
    pub status: WidgetSourceStatus,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize, TS)]
pub struct WidgetSelectionFacts {
    #[ts(type = "number")]
    pub revision: u64,
    #[ts(type = "number")]
    pub at_ms: u64,
    #[ts(type = "number | null")]
    pub read_at_ms: Option<u64>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize, TS)]
pub struct WidgetSummary {
    pub key: WidgetKey,
    pub space_id: String,
    pub title: String,
    #[ts(type = "number")]
    pub revision: u64,
    #[ts(type = "number")]
    pub created_seq: u64,
    pub kind: WidgetKind,
    pub presentation: WidgetPresentation,
    pub content: WidgetContentFacts,
    pub warnings: Vec<String>,
    pub source: Option<WidgetSourceSummary>,
    pub arrival: WidgetArrival,
    pub resolved_from: WidgetResolvedFrom,
    pub change: WidgetChange,
    #[ts(type = "number")]
    pub created_at_ms: u64,
    #[ts(type = "number")]
    pub updated_at_ms: u64,
    pub selection: Option<WidgetSelectionFacts>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize, TS)]
#[serde(rename_all = "snake_case")]
pub enum WidgetRemovalReason {
    User,
    Agent,
    Retired,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize, TS)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum WidgetEvent {
    Snapshot {
        #[ts(type = "number")]
        sequence: u64,
        widgets: Vec<WidgetSummary>,
    },
    Upserted {
        #[ts(type = "number")]
        sequence: u64,
        widget: WidgetSummary,
    },
    Removed {
        #[ts(type = "number")]
        sequence: u64,
        key: WidgetKey,
        reason: WidgetRemovalReason,
    },
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize, TS)]
#[serde(rename_all = "snake_case")]
pub enum WidgetBlocker {
    Library,
    Zoom,
    Drag,
    TooNarrow,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize, TS)]
pub struct WidgetWindowReport {
    pub session_id: Option<String>,
    pub displayed_tab_id: Option<String>,
    pub blocker: Option<WidgetBlocker>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize, TS)]
pub struct WidgetContentRequest {
    pub key: WidgetKey,
    #[ts(type = "number")]
    pub revision: u64,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize, TS)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum WidgetBody {
    Html { document: String },
    Choices { spec: WidgetChoicesSpec },
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize, TS)]
pub struct WidgetContent {
    pub key: WidgetKey,
    #[ts(type = "number")]
    pub revision: u64,
    pub sha256: String,
    pub body: WidgetBody,
    pub selection: Option<WidgetSelectionResponse>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize, TS)]
pub struct WidgetRemoveRequest {
    pub key: WidgetKey,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize, TS)]
#[serde(rename_all = "snake_case")]
pub enum WidgetRemoveResult {
    Removed,
    AlreadyRemoved,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize, TS)]
pub struct WidgetRemoveResponse {
    pub result: WidgetRemoveResult,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize, TS)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum WidgetSelectValue {
    Choice { choice_id: String },
    Page { value_json: String },
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize, TS)]
pub struct WidgetSelectRequest {
    pub key: WidgetKey,
    #[ts(type = "number")]
    pub revision: u64,
    pub value: WidgetSelectValue,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize, TS)]
pub struct WidgetSelectResponse {
    #[ts(type = "number")]
    pub at_ms: u64,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize, TS)]
#[serde(deny_unknown_fields)]
pub struct WidgetChoicesSpec {
    pub prompt: Option<String>,
    pub choices: Vec<WidgetChoice>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize, TS)]
#[serde(deny_unknown_fields)]
pub struct WidgetChoice {
    pub id: String,
    pub label: String,
    pub detail: Option<String>,
}
