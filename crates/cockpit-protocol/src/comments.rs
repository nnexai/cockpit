use serde::{Deserialize, Serialize};
use ts_rs::TS;

use crate::context::ExtensionKind;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(deny_unknown_fields)]
pub struct CommentRequestScope {
    pub binding_id: String,
    pub client_id: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
pub struct CommentOwner {
    pub session_id: String,
    pub pane_id: String,
    pub terminal_id: String,
    pub source_kind: ExtensionKind,
    /// Companion identity for Context; a separate review identity for Review.
    pub source_id: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
pub struct CommentLocation {
    pub workspace_id: String,
    pub tab_id: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
pub struct CommentAttachment {
    pub owner: CommentOwner,
    pub location: CommentLocation,
    pub binding_id: String,
    pub client_id: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(deny_unknown_fields)]
pub struct CommentReviewRef {
    pub review_id: String,
    pub generation: u32,
    pub file_id: String,
    pub side: crate::review::ReviewSide,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
pub struct CommentFileRef {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub review: Option<CommentReviewRef>,
    pub root_id: String,
    pub path: String,
    pub absolute_path: String,
    pub revision: String,
    pub content_hash: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
pub enum CommentSourceState {
    Current,
    Changed,
    Missing,
    Unavailable,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum CommentAnchor {
    WholeFile,
    Lines {
        start_line: u32,
        end_line: u32,
        /// Exact physical source lines, retaining CRLF/LF and a missing final newline.
        selected_lines: Vec<String>,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
pub struct CommentDraft {
    pub draft_id: String,
    pub file_ref: CommentFileRef,
    pub anchor: CommentAnchor,
    pub comment_text: String,
    pub source_state: CommentSourceState,
    pub updated_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
pub struct CommentBatch {
    pub batch_id: String,
    pub generation: u32,
    pub owner: CommentOwner,
    pub last_known_location: CommentLocation,
    /// Derived from fresh runtime proof; never persisted as an active attachment.
    pub live_attachment: Option<CommentAttachment>,
    pub drafts: Vec<CommentDraft>,
    pub updated_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
pub struct CommentBatchSummary {
    pub batch_id: String,
    pub generation: u32,
    pub owner: CommentOwner,
    pub last_known_location: CommentLocation,
    pub draft_count: u32,
    pub updated_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
pub struct CommentBatchList {
    pub attachment: CommentAttachment,
    pub batches: Vec<CommentBatchSummary>,
    pub truncated: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[serde(deny_unknown_fields)]
pub struct CommentBatchRequest {
    pub scope: CommentRequestScope,
    /// None finds this pane's existing batch or returns an unpersisted empty batch.
    pub batch_id: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(deny_unknown_fields)]
pub struct CommentBatchMutation {
    pub scope: CommentRequestScope,
    pub batch_id: String,
    pub expected_generation: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[serde(deny_unknown_fields)]
pub struct CommentCapture {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub review: Option<CommentReviewRef>,
    pub root_id: String,
    pub path: String,
    pub expected_revision: String,
    /// Both null means whole-file; otherwise both are inclusive physical line numbers.
    pub start_line: Option<u32>,
    pub end_line: Option<u32>,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[serde(deny_unknown_fields)]
pub struct CommentUpsertRequest {
    pub batch: CommentBatchMutation,
    /// None creates a draft with capture; editing changes text, not captured source.
    pub draft_id: Option<String>,
    pub capture: Option<CommentCapture>,
    pub comment_text: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[serde(deny_unknown_fields)]
pub struct CommentRemoveRequest {
    pub batch: CommentBatchMutation,
    pub draft_id: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[serde(deny_unknown_fields)]
pub struct CommentPreviewRequest {
    pub batch: CommentBatchMutation,
    pub retain_stale_excerpts: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
pub struct CommentPreview {
    pub batch_id: String,
    pub generation: u32,
    pub payload: String,
    pub payload_bytes: u32,
    pub framed_bytes: u32,
    pub limit_bytes: u32,
    pub sanitized_controls: u32,
    pub stale_draft_ids: Vec<String>,
    pub exportable: bool,
    pub reason: Option<String>,
}
