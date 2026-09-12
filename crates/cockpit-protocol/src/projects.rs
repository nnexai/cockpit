use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use ts_rs::TS;

use crate::v1::ErrorResponse;

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[serde(deny_unknown_fields)]
pub struct ProjectLimits {
    pub catalog_depth: u32,
    pub catalog_entries: u32,
    pub git_timeout_ms: u32,
    pub git_output_bytes: u32,
    pub operation_timeout_ms: u32,
    pub context_preview_bytes: u32,
    pub context_preview_lines: u32,
    pub context_directory_entries: u32,
    pub context_tree_depth: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[serde(deny_unknown_fields)]
pub struct ProjectProvider {
    pub id: String,
    pub base_url: String,
    pub executable: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub login: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[serde(deny_unknown_fields)]
pub struct ProjectConfiguration {
    pub version: u32,
    pub repository_roots: Vec<String>,
    pub worktree_root: String,
    pub companion_root: String,
    pub state_root: String,
    pub branch_template: String,
    pub checkout_template: String,
    pub providers: Vec<ProjectProvider>,
    pub limits: ProjectLimits,
    pub origins: BTreeMap<String, String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
pub struct RepositoryCandidate {
    pub repository_id: String,
    pub name: String,
    pub root: String,
    pub checkout_path: String,
    pub common_dir: String,
    pub branch: Option<String>,
    pub is_linked_worktree: bool,
    pub is_detached: bool,
    pub provenance: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
pub struct ProjectDiagnostic {
    pub code: String,
    pub message: String,
    pub path: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
pub struct RepositoryListResponse {
    pub repositories: Vec<RepositoryCandidate>,
    pub diagnostics: Vec<ProjectDiagnostic>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
pub enum WorkspaceSetupMode {
    Create,
    Open,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[serde(deny_unknown_fields)]
#[serde(tag = "operation", rename_all = "snake_case")]
#[ts(tag = "operation", rename_all = "snake_case")]
pub enum WorkspaceSetupRequest {
    Create {
        repository_id: String,
        branch: Option<String>,
        base_ref: Option<String>,
        checkout_path: Option<String>,
        label: Option<String>,
        task_name: Option<String>,
        artifact_url: Option<String>,
        focus: bool,
    },
    Open {
        path: String,
        label: Option<String>,
        task_name: Option<String>,
        focus: bool,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
pub enum WorkspaceCheckoutOwnership {
    OwnedWorktree,
    BorrowedDirectory,
}

impl Default for WorkspaceCheckoutOwnership {
    fn default() -> Self {
        // Legacy journals did not record ownership. Borrowed is the only safe
        // default because it can never authorize a filesystem removal.
        Self::BorrowedDirectory
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
pub struct ProjectArtifact {
    pub provider_id: String,
    pub kind: String,
    pub canonical_id: String,
    pub original_url: String,
    pub canonical_url: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
pub struct WorkspaceSetupPlan {
    pub operation_id: String,
    pub generation: u32,
    pub endpoint_identity: String,
    pub session_id: String,
    pub repository: Option<RepositoryCandidate>,
    pub mode: WorkspaceSetupMode,
    #[serde(default)]
    pub ownership: WorkspaceCheckoutOwnership,
    pub branch: Option<String>,
    pub base: Option<String>,
    pub checkout_path: String,
    pub companion_path: String,
    pub companion_id: String,
    pub companion_created_by_operation: bool,
    pub label: String,
    pub focus: bool,
    pub artifact: Option<ProjectArtifact>,
    pub effects: Vec<String>,
    pub warnings: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[serde(deny_unknown_fields)]
pub struct WorkspaceOperationRequest {
    pub operation_id: String,
    pub expected_generation: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
pub enum WorkspaceRecoveryAction {
    AcceptExistingWorktree,
    RetryEnvironment,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[serde(deny_unknown_fields)]
pub struct WorkspaceReconcileRequest {
    pub operation_id: String,
    pub expected_generation: u32,
    pub action: WorkspaceRecoveryAction,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
pub enum WorkspaceOperationState {
    Planned,
    Running,
    Completed,
    Partial,
    OutcomeUnknown,
    Cancelled,
    NeedsReview,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
pub enum WorkspaceOperationStep {
    Planned,
    Validated,
    HerdrRequested,
    HerdrObserved,
    WorktreeReady,
    WorkspaceVerified,
    CompanionReady,
    ContextPreparing,
    ContextReady,
    EnvironmentRequested,
    EnvironmentReady,
    Completed,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
pub struct WorkspaceOwnedResource {
    pub kind: String,
    pub path: String,
    pub created_by_operation: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
pub struct WorkspaceOperation {
    pub operation_id: String,
    pub generation: u32,
    pub sequence: u32,
    pub session_id: String,
    pub plan: WorkspaceSetupPlan,
    pub state: WorkspaceOperationState,
    pub step: WorkspaceOperationStep,
    pub workspace_id: Option<String>,
    pub tab_id: Option<String>,
    pub pane_id: Option<String>,
    pub companion_id: Option<String>,
    pub owned_resources: Vec<WorkspaceOwnedResource>,
    pub error: Option<ErrorResponse>,
    pub resume_allowed: bool,
    pub cancel_requested: bool,
    pub updated_at: String,
}
