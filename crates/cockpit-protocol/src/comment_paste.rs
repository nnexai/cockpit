use serde::{Deserialize, Serialize};
use ts_rs::TS;

use crate::comments::CommentBatchMutation;

/// A current agent target derived by the host from an authoritative Herdr snapshot.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
pub struct CommentPasteTarget {
    pub endpoint_identity: String,
    pub session_id: String,
    pub workspace_id: String,
    pub tab_id: String,
    pub pane_id: String,
    pub terminal_id: String,
    /// Current agent name from Herdr, for target selection only.
    pub agent_label: String,
    /// Opaque fingerprint built from current terminal and agent identity evidence.
    pub agent_fingerprint: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
pub enum CommentPasteState {
    Pending,
    Accepted,
    Rejected,
    OutcomeUnknown,
}

/// Prepare returns the exact bounded payload digest and the only current same-tab targets.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(deny_unknown_fields)]
pub struct CommentPastePrepareRequest {
    pub batch: CommentBatchMutation,
    pub retain_stale_excerpts: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
pub struct CommentPastePrepareResponse {
    pub batch_id: String,
    pub generation: u32,
    pub payload_hash: String,
    pub payload_bytes: u32,
    pub framed_bytes: u32,
    pub limit_bytes: u32,
    pub targets: Vec<CommentPasteTarget>,
    /// Recent durable receipts for this batch; pending records are recovered as outcome-unknown.
    pub receipts: Vec<CommentPasteReceipt>,
    pub paste_available: bool,
    pub reason: Option<String>,
}

/// An explicit user-requested send of a prepared payload to one frozen target.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(deny_unknown_fields)]
pub struct CommentPasteSendRequest {
    pub batch: CommentBatchMutation,
    pub target: CommentPasteTarget,
    pub expected_payload_hash: String,
    pub retain_stale_excerpts: bool,
    /// Stable across duplicate transport delivery of the same user action.
    pub operation_id: String,
    /// Host/client request correlation. It is recorded but never used as a lease.
    pub request_id: String,
    /// Required only when the user consciously retries an outcome-unknown receipt.
    pub acknowledge_duplicate_risk: bool,
}

/// Explicit user resolution after inspecting an uncertain paste outcome.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(deny_unknown_fields)]
pub struct CommentPasteMarkPastedRequest {
    pub batch: CommentBatchMutation,
    pub operation_id: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
pub struct CommentPasteReceipt {
    pub operation_id: String,
    pub request_id: String,
    pub batch_id: String,
    pub batch_generation: u32,
    pub payload_hash: String,
    pub target: CommentPasteTarget,
    pub state: CommentPasteState,
    pub sent_draft_ids: Vec<String>,
    pub created_at: String,
    pub completed_at: Option<String>,
    pub message: Option<String>,
    /// The user explicitly resolved this prior outcome after inspection.
    #[serde(default)]
    pub user_confirmed: bool,
}
