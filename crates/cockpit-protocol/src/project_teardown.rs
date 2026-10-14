use serde::{Deserialize, Serialize};
use ts_rs::TS;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
pub enum WorkspaceTeardownAction {
    CloseSpace,
    RemoveOwnedWorktree,
    ReconcileRemoveOutcome,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
pub enum WorkspaceTeardownOwnership {
    OwnedCreated,
    BorrowedOpened,
    SelectedLinked,
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
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub discard_fingerprint: Option<String>,
}

/// A fresh, reviewable description of one requested teardown target. The
/// caller must send these exact endpoint and checkout values back on execute;
/// the core then recomputes the preview before allowing a command.
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
pub struct WorkspaceTeardownPreview {
    pub operation_id: String,
    pub workspace_id: String,
    pub endpoint_identity: String,
    pub repository_key: Option<String>,
    pub repository_root: Option<String>,
    pub checkout_path: String,
    pub ownership: WorkspaceTeardownOwnership,
    pub workspace_state: WorkspaceTeardownWorkspaceState,
    pub is_linked_worktree: bool,
    pub dirty_state: WorkspaceTeardownDirtyState,
    pub discard_count: u32,
    pub discard_paths: Vec<String>,
    pub discard_fingerprint: Option<String>,
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
    Retained,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
pub enum WorkspaceTeardownRecoveryState {
    Pending,
    OutcomeUnknown,
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

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn execute_discard_fingerprint_is_optional_and_omitted_when_absent() {
        let clean = json!({
            "operation_id": "manual-operation", "workspace_id": "space",
            "expected_endpoint_identity": "endpoint", "expected_checkout_path": "/worktrees/task",
            "action": "remove_owned_worktree", "confirmation": "task"
        });
        let request: WorkspaceTeardownExecuteRequest =
            serde_json::from_value(clean.clone()).expect("clean request");
        assert!(request.discard_fingerprint.is_none());
        assert_eq!(serde_json::to_value(request).expect("serialize clean request"), clean);
        let mut dirty = clean;
        dirty["discard_fingerprint"] = json!("a".repeat(64));
        let request: WorkspaceTeardownExecuteRequest =
            serde_json::from_value(dirty.clone()).expect("dirty request");
        assert_eq!(request.discard_fingerprint.as_deref(), Some("a".repeat(64).as_str()));
        assert_eq!(serde_json::to_value(request).expect("serialize dirty request"), dirty);
    }

    #[test]
    fn selected_linked_is_distinct_from_setup_ownership() {
        let ownership: WorkspaceTeardownOwnership =
            serde_json::from_value(json!("selected_linked")).expect("selected linked ownership");
        assert_eq!(ownership, WorkspaceTeardownOwnership::SelectedLinked);
        assert_eq!(serde_json::to_value(ownership).expect("serialize ownership"), json!("selected_linked"));
    }
}
