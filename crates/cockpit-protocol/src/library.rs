use serde::{Deserialize, Serialize};
use ts_rs::TS;

use crate::context::ContextRoot;
use crate::projects::ProjectDiagnostic;
use crate::v1::ErrorResponse;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
pub enum LibraryItemKind {
    ProviderSnapshot,
    FolderCopy,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
pub enum LibraryItemState {
    Fresh,
    Changed,
    Unknown,
    RemovedAtSource,
    Conflict,
    Failed,
    Partial,
}
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
pub struct LibraryContainer {
    pub container_id: String,
    pub label: String,
}
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
pub struct LibraryAncestor {
    pub id: String,
    pub title: String,
}
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
pub struct LibraryPartial {
    pub unit: String,
    #[ts(type = "number")]
    pub have: u64,
    #[ts(type = "number | null")]
    pub total: Option<u64>,
    pub reason: String,
}
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
pub struct LibraryConflictFile {
    pub path: String,
    pub current_hash: String,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
pub enum LibraryAttachmentState {
    NotDownloaded,
    Downloaded,
    OverLimit,
    Failed,
}
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
pub struct LibraryAttachment {
    pub attachment_id: String,
    pub original_name: String,
    pub stored_name: String,
    pub media_type: Option<String>,
    #[ts(type = "number | null")]
    pub bytes: Option<u64>,
    pub version: Option<String>,
    pub state: LibraryAttachmentState,
    pub relative_path: Option<String>,
}
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
pub struct LibraryFolderInfo {
    pub origin_path: String,
    pub git_working_tree: bool,
    #[ts(type = "number")]
    pub files: u64,
    #[ts(type = "number")]
    pub bytes: u64,
    pub skipped_symlinks: u32,
    pub skipped_special: u32,
    pub skipped_ignored: u32,
    pub skipped_other: u32,
}
/// One reason an item is kept in the Library. An item with no references is
/// tombstoned and purged after the grace period.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize, TS)]
#[serde(tag = "kind", rename_all = "snake_case")]
#[ts(tag = "kind", rename_all = "snake_case")]
pub enum LibraryItemRef {
    Manual,
    Follow { follow_id: String },
    Space { space_context_id: String },
}
/// Who holds an inclusion: a single-import seed item or a follow.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(tag = "kind", rename_all = "snake_case")]
#[ts(tag = "kind", rename_all = "snake_case")]
pub enum LibraryInclusionHolder {
    Item { item_id: String },
    Follow { follow_id: String },
}
/// Why a related item is in the Library: the shallowest route from one holder.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
pub struct LibraryInclusion {
    pub holder: LibraryInclusionHolder,
    pub from_item_id: Option<String>,
    pub from_label: String,
    pub relation: String,
    pub depth: u32,
}
/// Per-issue metadata of a Jira item, shared by every follow that lists it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
pub struct LibraryIssueMeta {
    pub updated: String,
    pub fetched_updated: Option<String>,
    pub status: String,
    pub issue_type: String,
    pub assignee: Option<String>,
}
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
pub struct LibraryItemSummary {
    pub item_id: String,
    pub logical_id: String,
    pub kind: LibraryItemKind,
    pub provider_id: Option<String>,
    pub provider_instance: Option<String>,
    pub resource_type: Option<String>,
    pub canonical_id: Option<String>,
    pub container: Option<LibraryContainer>,
    pub parent_item_id: Option<String>,
    pub ancestors: Vec<LibraryAncestor>,
    pub order: Option<u32>,
    pub title: String,
    pub document_path: Option<String>,
    pub item_path: String,
    pub source_url: Option<String>,
    pub original_url: Option<String>,
    pub source_revision: Option<String>,
    pub revision: String,
    pub state: LibraryItemState,
    pub partial: Option<LibraryPartial>,
    pub conflict: Vec<LibraryConflictFile>,
    pub fetched_at: Option<String>,
    pub checked_at: Option<String>,
    pub refs: Vec<LibraryItemRef>,
    pub purge_after: Option<String>,
    pub issue: Option<LibraryIssueMeta>,
    pub attachments: Vec<LibraryAttachment>,
    pub folder: Option<LibraryFolderInfo>,
    pub diagnostics: Vec<ProjectDiagnostic>,
    /// Reference depth of a single-import seed; `None` means 0.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub reference_depth: Option<u32>,
    /// Minimal reason a related item is included, one entry per holder.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub included_by: Option<Vec<LibraryInclusion>>,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
pub enum LibraryFollowMode {
    Live,
    Accumulate,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(tag = "kind", rename_all = "snake_case")]
#[ts(tag = "kind", rename_all = "snake_case")]
pub enum LibraryFollowSource {
    ConfluenceSpace {
        space_key: String,
        space_name: String,
    },
    JiraQuery {
        jql: String,
        mode: LibraryFollowMode,
    },
}
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
pub struct LibraryFollowSummary {
    pub follow_id: String,
    pub provider_id: String,
    pub provider_instance: String,
    pub source: LibraryFollowSource,
    pub include_attachments: bool,
    pub item_count: u32,
    pub partial: Option<LibraryPartial>,
    pub excluded_ids: Vec<String>,
    pub last_refreshed_at: Option<String>,
    pub state: LibraryItemState,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub reference_depth: Option<u32>,
}
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
pub struct LibraryListing {
    pub root: ContextRoot,
    pub generation: String,
    pub items: Vec<LibraryItemSummary>,
    pub follows: Vec<LibraryFollowSummary>,
    pub next_offset: Option<u32>,
    pub diagnostics: Vec<ProjectDiagnostic>,
}
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
pub struct SpaceTarget {
    pub session_id: String,
    pub space_id: String,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
pub enum LibraryInputKind {
    Artifact,
    ConfluencePage,
    ConfluenceSpace,
    Folder,
    JiraQuery,
}
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[serde(deny_unknown_fields)]
pub struct LibraryResolveRequest {
    pub input: String,
    pub provider_id: Option<String>,
}
/// Browse the spaces of one configured Confluence provider (UQ5a).
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[serde(deny_unknown_fields)]
pub struct LibraryConfluenceSpacesRequest {
    pub provider_id: String,
}
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
pub struct LibraryResolution {
    pub kind: LibraryInputKind,
    pub provider_id: Option<String>,
    pub provider_instance: Option<String>,
    pub title: String,
    pub canonical_id: Option<String>,
    pub container_label: Option<String>,
    pub existing_item_id: Option<String>,
    pub existing_follow_id: Option<String>,
    pub item_count: Option<u32>,
    pub item_count_exact: bool,
    pub follow_mode: Option<LibraryFollowMode>,
    pub git_working_tree: Option<bool>,
    #[ts(type = "number | null")]
    pub file_count: Option<u64>,
    pub diagnostics: Vec<ProjectDiagnostic>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub reference_depth: Option<u32>,
}
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[serde(deny_unknown_fields)]
pub struct LibraryAddRequest {
    pub input: String,
    pub provider_id: Option<String>,
    #[serde(default)]
    pub reference_depth: u32,
    #[serde(default)]
    pub follow: bool,
    #[serde(default)]
    pub follow_mode: Option<LibraryFollowMode>,
    #[serde(default)]
    pub download_attachments: bool,
    #[serde(default)]
    pub refresh_existing: bool,
    pub label: Option<String>,
    pub target: Option<SpaceTarget>,
}
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[serde(tag = "scope", rename_all = "snake_case", deny_unknown_fields)]
#[ts(tag = "scope", rename_all = "snake_case")]
pub enum LibraryRefreshRequest {
    Items {
        item_ids: Vec<String>,
    },
    Follow {
        follow_id: String,
    },
    Container {
        provider_instance: String,
        container_id: String,
    },
    All,
}
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[serde(deny_unknown_fields)]
pub struct LibraryReplaceRequest {
    pub item_id: String,
    pub confirmed: Vec<LibraryConflictFile>,
}
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[serde(tag = "mode", rename_all = "snake_case", deny_unknown_fields)]
#[ts(tag = "mode", rename_all = "snake_case")]
pub enum LibraryRemoveRequest {
    Item {
        item_id: String,
        expected_revision: String,
    },
    StopFollowing {
        follow_id: String,
    },
    Follow {
        follow_id: String,
    },
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
pub enum LibraryAttachmentAction {
    Download,
    RemoveDownloaded,
}
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[serde(deny_unknown_fields)]
pub struct LibraryAttachmentRequest {
    pub item_id: String,
    pub attachment_ids: Vec<String>,
    pub action: LibraryAttachmentAction,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
pub enum LibraryOperationKind {
    Add,
    Refresh,
    SpaceAdd,
    Attachments,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
pub enum LibraryPhaseName {
    Library,
    Space,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
pub enum LibraryPhaseState {
    Pending,
    Running,
    Done,
    Partial,
    Failed,
    Cancelled,
}
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
pub struct LibraryPhase {
    pub phase: LibraryPhaseName,
    pub state: LibraryPhaseState,
    pub done: u32,
    pub total: Option<u32>,
    pub message: Option<String>,
    pub error: Option<ErrorResponse>,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
pub enum LibraryReportOutcome {
    New,
    Updated,
    Unchanged,
    RemovedAtSource,
    Partial,
    Dropped,
    Failed,
    Conflict,
}
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
pub struct LibraryReportRow {
    pub item_id: Option<String>,
    pub follow_id: Option<String>,
    pub title: String,
    pub outcome: LibraryReportOutcome,
    pub reason: Option<String>,
}
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
pub struct LibraryRefreshReport {
    pub new: u32,
    pub updated: u32,
    pub unchanged: u32,
    pub removed_at_source: u32,
    pub dropped: u32,
    pub partial: u32,
    pub failed: u32,
    pub conflict: u32,
    pub rows: Vec<LibraryReportRow>,
    pub truncated_rows: bool,
}
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
pub struct SpacePhaseResult {
    pub space_id: String,
    pub item_ids: Vec<String>,
}
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
pub struct LibraryOperation {
    pub operation_id: String,
    pub kind: LibraryOperationKind,
    pub phases: Vec<LibraryPhase>,
    pub item_ids: Vec<String>,
    pub report: Option<LibraryRefreshReport>,
    pub space: Option<SpacePhaseResult>,
    pub target: Option<SpaceTarget>,
    pub cancel_requested: bool,
    pub finished: bool,
    pub created_at: String,
    pub updated_at: String,
}
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[serde(deny_unknown_fields)]
pub struct LibraryDirectoryRequest {
    pub path: String,
    pub offset: Option<u32>,
    pub revision: Option<String>,
}
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[serde(deny_unknown_fields)]
pub struct LibraryDocumentRequest {
    pub path: String,
    pub expected_revision: Option<String>,
    pub offset: Option<u32>,
}
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[serde(deny_unknown_fields)]
pub struct LibraryMediaRequest {
    pub path: String,
    pub expected_revision: Option<String>,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
pub enum LibraryFileIndexMode {
    Cached,
    Fresh,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[serde(deny_unknown_fields)]
pub struct LibraryFileIndexRequest {
    pub mode: LibraryFileIndexMode,
}
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[serde(deny_unknown_fields)]
pub struct SpaceContextRequest {
    pub target: SpaceTarget,
}
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
pub struct SpaceContextListing {
    pub target: SpaceTarget,
    pub space_label: String,
    pub library_root: String,
    pub checkout_path: Option<String>,
    pub items: Vec<LibraryItemSummary>,
    pub repository_paths: Vec<String>,
    pub diagnostics: Vec<ProjectDiagnostic>,
}
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[serde(deny_unknown_fields)]
pub struct SpaceAddRequest {
    pub target: SpaceTarget,
    pub item_ids: Vec<String>,
}
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[serde(deny_unknown_fields)]
pub struct SpaceRemoveRequest {
    pub target: SpaceTarget,
    pub item_ids: Vec<String>,
}
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[serde(deny_unknown_fields)]
pub struct SpaceRepositoriesRequest {
    pub target: SpaceTarget,
    pub repository_paths: Vec<String>,
}

// Context read responses are the existing ContextDirectory, ContextDocument, and ContextMedia DTOs.
