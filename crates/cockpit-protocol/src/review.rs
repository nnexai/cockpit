use serde::{Deserialize, Serialize};
use ts_rs::TS;

use crate::projects::ProjectDiagnostic;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
pub enum ReviewComparison {
    AllLocal,
    Staged,
    Unstaged,
    Branch,
    Untracked,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
pub enum ReviewFileStatus {
    Added,
    Modified,
    Deleted,
    Renamed,
    Copied,
    Untracked,
    Binary,
    ModeOnly,
    Submodule,
    Unreadable,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
pub enum ReviewDiffLineKind {
    Context,
    Added,
    Deleted,
}

/// The immutable pre-image or post-image represented by a review file.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
pub enum ReviewSide {
    Old,
    New,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[serde(deny_unknown_fields)]
pub struct ReviewSnapshotRequest {
    pub binding_id: String,
    pub repository_id: String,
    pub comparison: ReviewComparison,
    /// Required only for branch comparison; resolved to an immutable commit.
    pub base_ref: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
pub struct ReviewChangedFile {
    pub file_id: String,
    /// All-local preserves each staged/unstaged/untracked anchor separately.
    pub comparison: ReviewComparison,
    pub status: ReviewFileStatus,
    pub old_path: Option<String>,
    pub new_path: Option<String>,
    pub binary: bool,
    pub summary: String,
    pub old_revision: Option<String>,
    pub new_revision: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
pub struct ReviewSnapshot {
    pub binding_id: String,
    pub session_id: String,
    pub pane_id: String,
    pub review_id: String,
    pub generation: u32,
    pub repository_id: String,
    pub checkout_path: String,
    /// Stable direct-checkout identity shared with Review comment batches.
    pub source_id: String,
    pub comparison: ReviewComparison,
    pub base_revision: Option<String>,
    pub head_revision: Option<String>,
    pub index_revision: String,
    pub worktree_revision: String,
    pub files: Vec<ReviewChangedFile>,
    pub truncated: bool,
    pub diagnostics: Vec<ProjectDiagnostic>,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[serde(deny_unknown_fields)]
pub struct ReviewFileRequest {
    pub binding_id: String,
    pub review_id: String,
    pub generation: u32,
    pub file_id: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
pub struct ReviewDiffLine {
    pub kind: ReviewDiffLineKind,
    pub old_line: Option<u32>,
    pub new_line: Option<u32>,
    /// Exact line bytes decoded as UTF-8 only for textual Git diff output.
    pub text: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
pub struct ReviewHunk {
    pub old_path: Option<String>,
    pub new_path: Option<String>,
    pub old_start: u32,
    pub new_start: u32,
    pub lines: Vec<ReviewDiffLine>,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
pub struct ReviewFileDiff {
    pub binding_id: String,
    pub session_id: String,
    pub pane_id: String,
    pub review_id: String,
    pub generation: u32,
    pub file: ReviewChangedFile,
    pub hunks: Vec<ReviewHunk>,
    /// Bounded immutable text retained for old/new review-side capture and
    /// expansion of unchanged lines. None means binary, unreadable, or capped.
    pub old_source: Option<String>,
    pub new_source: Option<String>,
    pub old_source_hash: Option<String>,
    pub new_source_hash: Option<String>,
    pub old_total_lines: Option<u32>,
    pub new_total_lines: Option<u32>,
    pub old_source_truncated: bool,
    pub new_source_truncated: bool,
    pub truncated: bool,
    pub diagnostics: Vec<ProjectDiagnostic>,
}
