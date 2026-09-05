use serde::{Deserialize, Serialize};
use ts_rs::TS;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
pub enum WorkspaceTeardownAction {
    CloseSpace,
    RemoveOwnedWorktree,
    ReconcileRemoveOutcome,
    RemoveOrphanedCompanion,
    ForgetAssociation,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
pub enum WorkspaceTeardownOwnership {
    OwnedCreated,
    BorrowedOpened,
    Foreign,
    Unknown,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
pub enum WorkspaceTeardownDirtyState {
    Clean,
    Dirty,
    Unknown,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
pub enum WorkspaceTeardownWorkspaceState {
    Live,
    Missing,
    Ambiguous,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
pub enum WorkspaceTeardownCompanionState {
    Owned,
    Missing,
    Foreign,
    Ambiguous,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[serde(deny_unknown_fields)]
pub struct WorkspaceTeardownPreviewRequest {
    pub workspace_id: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[serde(deny_unknown_fields)]
pub struct WorkspaceTeardownExecuteRequest {
    pub operation_id: String,
    pub workspace_id: String,
    pub expected_endpoint_identity: String,
    pub expected_checkout_path: String,
    pub action: WorkspaceTeardownAction,
    pub confirmation: String,
}

/// A fresh, reviewable description of one requested teardown target. The
/// caller must send these exact endpoint and checkout values back on execute;
/// the core then recomputes the preview before allowing a command.
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
pub struct WorkspaceTeardownPreview {
    pub operation_id: String,
    pub workspace_id: String,
    pub endpoint_identity: String,
    pub repository_key: String,
    pub repository_root: String,
    pub checkout_path: String,
    pub ownership: WorkspaceTeardownOwnership,
    pub workspace_state: WorkspaceTeardownWorkspaceState,
    pub companion_state: WorkspaceTeardownCompanionState,
    pub is_linked_worktree: bool,
    pub dirty_state: WorkspaceTeardownDirtyState,
    pub companion_path: Option<String>,
    pub allowed_actions: Vec<WorkspaceTeardownAction>,
    pub blockers: Vec<String>,
    pub warnings: Vec<String>,
    pub required_confirmation: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
pub enum WorkspaceTeardownOutcome {
    Completed,
    OutcomeUnknown,
    OrphanedCompanion,
    Retained,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
pub enum WorkspaceTeardownRecoveryState {
    Pending,
    OutcomeUnknown,
    OrphanedCompanion,
}

/// A durable recovery entry. It intentionally contains only journal evidence;
/// opening it always obtains fresh Herdr and Git evidence before any action.
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
pub struct WorkspaceTeardownRecovery {
    pub operation_id: String,
    pub workspace_id: String,
    pub checkout_path: String,
    pub state: WorkspaceTeardownRecoveryState,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
pub struct WorkspaceTeardownRecoveryList {
    pub recoveries: Vec<WorkspaceTeardownRecovery>,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
pub struct WorkspaceTeardownResult {
    pub operation_id: String,
    pub workspace_id: String,
    pub action: WorkspaceTeardownAction,
    pub outcome: WorkspaceTeardownOutcome,
    pub message: String,
}
