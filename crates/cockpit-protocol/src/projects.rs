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
    pub library_folder_files: u32,
    #[ts(type = "number")]
    pub library_folder_bytes: u64,
    #[ts(type = "number")]
    pub library_file_bytes: u64,
    pub library_space_pages: u32,
    #[ts(type = "number")]
    pub library_attachment_bytes: u64,
    #[ts(type = "number")]
    pub library_item_attachment_bytes: u64,
    pub library_max_items: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
pub enum ProviderKind {
    Github,
    Gitlab,
    Gitea,
    Jira,
    Confluence,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
pub enum ProviderDeployment {
    Cloud,
    DataCenter,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[serde(deny_unknown_fields)]
pub struct ProjectProvider {
    pub id: String,
    pub kind: ProviderKind,
    pub base_url: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub executable: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub login: Option<String>,
    /// Jira/Confluence only; always resolved after configuration loading.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub deployment: Option<ProviderDeployment>,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[serde(deny_unknown_fields)]
pub struct OrchestrationConfiguration {
    #[serde(default = "default_harness")]
    pub harness_kind: String,
    #[serde(default = "default_harness")]
    pub command: String,
    #[serde(default)]
    pub extra_args: Vec<String>,
    pub resume_args: Option<Vec<String>>,
    /// One whole placeholder; None uses a positional prompt for OMP.
    #[serde(default)]
    pub initial_prompt_args: Option<Vec<String>>,
    /// None enables the OMP hook; an empty argv disables it.
    #[serde(default)]
    pub hook_args: Option<Vec<String>>,
    #[serde(default = "default_max_parallel")]
    pub max_parallel: u32,
    #[serde(default)]
    pub process_scope: ScopeMode,
    #[serde(default)]
    pub inbox_prompt_hints: bool,
    #[serde(default)]
    pub routes: Vec<OrchestrationRoute>,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
pub enum ScopeMode {
    #[default]
    Auto,
    Off,
}

fn default_harness() -> String { "omp".into() }
fn default_max_parallel() -> u32 { 4 }

impl Default for OrchestrationConfiguration {
    fn default() -> Self {
        Self { harness_kind: default_harness(), command: default_harness(),
            extra_args: Vec::new(), resume_args: None, initial_prompt_args: None,
            hook_args: None, max_parallel: default_max_parallel(), process_scope: ScopeMode::Auto, inbox_prompt_hints: false,
            routes: Vec::new() }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[serde(deny_unknown_fields)]
pub struct OrchestrationRoute {
    pub provider: String,
    pub instance: String,
    pub project_id_prefix: String,
    pub repository_id: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[serde(deny_unknown_fields)]
pub struct ProjectConfiguration {
    pub version: u32,
    pub repository_roots: Vec<String>,
    pub worktree_root: String,
    pub state_root: String,
    pub cache_root: String,
    pub library_root: String,
    pub notes_root: String,
    pub branch_template: String,
    pub checkout_template: String,
    pub providers: Vec<ProjectProvider>,
    pub limits: ProjectLimits,
    #[serde(default)]
    pub orchestration: OrchestrationConfiguration,
}

#[cfg(any(test, feature = "test-support"))]
impl ProjectConfiguration {
    /// Builds an isolated configuration for tests rooted in one fixture directory.
    pub fn for_tests(root: &std::path::Path) -> Self {
        Self {
            version: 1,
            repository_roots: vec![root.to_string_lossy().into_owned()],
            worktree_root: root.join("worktrees").to_string_lossy().into_owned(),
            state_root: root.join("state").to_string_lossy().into_owned(),
            cache_root: root.join("cache").to_string_lossy().into_owned(),
            library_root: root.join("library").to_string_lossy().into_owned(),
            notes_root: root.join("notes").to_string_lossy().into_owned(),
            branch_template: "{task_id}".to_owned(),
            checkout_template: "{task_id}".to_owned(),
            providers: Vec::new(),
            limits: ProjectLimits {
                catalog_depth: 4,
                catalog_entries: 256,
                git_timeout_ms: 2_000,
                git_output_bytes: 2 * 1024 * 1024,
                operation_timeout_ms: 2_000,
                context_preview_bytes: 1024 * 1024,
                context_preview_lines: 2_000,
                context_directory_entries: 64,
                context_tree_depth: 8,
                library_folder_files: 512,
                library_folder_bytes: 32 * 1024 * 1024,
                library_file_bytes: 4 * 1024 * 1024,
                library_space_pages: 200,
                library_attachment_bytes: 25 * 1024 * 1024,
                library_item_attachment_bytes: 100 * 1024 * 1024,
                library_max_items: 20_000,
            },
            orchestration: OrchestrationConfiguration::default(),
        }
    }
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
        /// Work items linked from the artifact that setup also imports.
        #[serde(default)]
        linked_artifact_urls: Vec<String>,
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
    OwnedAttemptDirectory,
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
    pub ownership: WorkspaceCheckoutOwnership,
    pub branch: Option<String>,
    pub base: Option<String>,
    pub checkout_path: String,
    pub label: String,
    pub focus: bool,
    pub artifact: Option<ProjectArtifact>,
    #[serde(default)]
    pub linked_artifacts: Vec<ProjectArtifact>,
    pub effects: Vec<String>,
    pub warnings: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[serde(deny_unknown_fields)]
pub struct WorkspaceOperationRequest {
    pub operation_id: String,
    pub expected_generation: u32,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(deny_unknown_fields)]
pub struct OwnedCheckoutRequest {
    pub operation_id: String,
    pub repository_checkout: String,
    pub checkout_path: String,
    pub branch: String,
    pub base: String,
    pub artifact: Option<ProjectArtifact>,
    pub source_workspace_id: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(deny_unknown_fields)]
pub struct DirectoryCheckoutRequest {
    pub operation_id: String,
    pub checkout_path: String,
    pub source_workspace_id: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(deny_unknown_fields)]
pub struct OwnedAttemptDirectoryRequest {
    pub operation_id: String,
    pub run_id: String,
    pub source_root: String,
    pub source_workspace_id: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(deny_unknown_fields)]
pub struct BindMovedSpaceRequest {
    pub operation_id: String,
    pub expected_generation: u32,
    pub endpoint_identity: String,
    pub workspace_id: String,
    pub tab_id: String,
    pub pane_id: String,
    pub terminal_id: String,
    pub move_operation_id: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
pub enum WorkspaceRecoveryAction {
    AcceptExistingWorktree,
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
    ContextPreparing,
    ContextReady,
    Completed,
    CheckoutPrepared,
    SpaceBound,
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
    pub owned_resources: Vec<WorkspaceOwnedResource>,
    pub error: Option<ErrorResponse>,
    pub resume_allowed: bool,
    pub cancel_requested: bool,
    pub updated_at: String,
}

#[cfg(test)]
mod tests {
    use super::{ProjectProvider, ProviderDeployment, ProviderKind};
    use serde_json::json;

    #[test]
    fn stored_launch_config_defaults_to_four_workers_and_optional_hook() {
        let configuration: super::OrchestrationConfiguration = serde_json::from_value(json!({
            "harness_kind": "omp", "command": "omp", "extra_args": [],
            "resume_args": null, "inbox_prompt_hints": false, "routes": []
        })).unwrap();
        assert_eq!(configuration.max_parallel, 4);
        assert!(configuration.initial_prompt_args.is_none());
        assert!(configuration.hook_args.is_none());
        assert_eq!(configuration.process_scope, super::ScopeMode::Auto);
        let uncontained: super::OrchestrationConfiguration = serde_json::from_value(json!({"process_scope": "off"})).unwrap();
        assert_eq!(uncontained.process_scope, super::ScopeMode::Off);
        let disabled: super::OrchestrationConfiguration = serde_json::from_value(json!({"hook_args": []})).unwrap();
        assert_eq!(disabled.hook_args, Some(Vec::new()));
    }

    #[test]
    fn provider_wire_shape_requires_explicit_kind_and_omits_absent_cli_fields() {
        let value = json!({
            "id": "custom-id",
            "kind": "jira",
            "base_url": "https://jira.example/jira",
            "deployment": "data_center"
        });
        let provider: ProjectProvider = serde_json::from_value(value.clone()).expect("provider");
        assert_eq!(provider.kind, ProviderKind::Jira);
        assert_eq!(provider.deployment, Some(ProviderDeployment::DataCenter));
        assert!(provider.executable.is_none());
        assert!(provider.login.is_none());
        assert_eq!(serde_json::to_value(provider).expect("serialize"), value);
        for invalid in [
            json!({"id": "jira", "base_url": "https://jira.example", "executable": "jira"}),
            json!({"id": "jira", "kind": "unknown", "base_url": "https://jira.example"}),
            json!({"id": "jira", "kind": "jira", "base_url": "https://jira.example", "deployment": "unknown"}),
            json!({"id": "jira", "kind": "jira", "base_url": "https://jira.example", "profile": "default"}),
        ] {
            assert!(serde_json::from_value::<ProjectProvider>(invalid).is_err());
        }
    }
}
