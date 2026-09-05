use serde::{Deserialize, Serialize};
use ts_rs::TS;

use crate::projects::ProjectDiagnostic;

/// The only local repository snapshot policy currently supported by Cockpit.
/// It captures current regular-file bytes, not a Git clone or checkout.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
pub enum ContextSnapshotMode {
    WorkingTree,
}

/// How snapshot bytes reached the companion. `copy` always creates an
/// independent inode; Cockpit does not use hardlinks or Git alternates.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
pub enum ContextSnapshotCopyMode {
    Reflink,
    Copy,
    Mixed,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[serde(deny_unknown_fields)]
pub struct ContextSnapshotRequest {
    pub binding_id: String,
    pub root_id: String,
    pub repository_id: String,
    pub mode: ContextSnapshotMode,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
pub struct ContextSnapshotResponse {
    pub binding_id: String,
    pub root_id: String,
    pub repository_id: String,
    /// A path relative to the authorized companion root.
    pub snapshot_path: String,
    pub generation: String,
    pub mode: ContextSnapshotMode,
    pub copy_mode: ContextSnapshotCopyMode,
    #[ts(type = "number")]
    pub files: u64,
    #[ts(type = "number")]
    pub bytes: u64,
    pub source_head: Option<String>,
    pub dirty: bool,
    pub diagnostics: Vec<ProjectDiagnostic>,
}
