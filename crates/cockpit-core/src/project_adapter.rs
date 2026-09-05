use std::collections::BTreeMap;

use async_trait::async_trait;
use cockpit_protocol::projects::WorkspaceSetupMode;

use crate::{HerdrAdapter, InspectionError};

#[derive(Debug, Clone)]
pub struct ProjectWorktreeEntry {
    pub checkout_path: String,
    pub branch: Option<String>,
    pub open_workspace_id: Option<String>,
    pub is_primary: bool,
    pub is_linked_worktree: bool,
    pub dirty: Option<bool>,
}

#[derive(Debug, Clone)]
pub struct ProjectInventory {
    pub endpoint_identity: String,
    pub repository_key: String,
    pub repository_root: String,
    pub worktrees: Vec<ProjectWorktreeEntry>,
    pub supported_methods: Vec<String>,
}

#[derive(Debug, Clone)]
pub struct ProjectWorktreeRequest {
    pub endpoint_identity: String,
    pub mode: WorkspaceSetupMode,
    pub source_cwd: String,
    pub branch: Option<String>,
    pub base: Option<String>,
    pub checkout_path: String,
    pub label: String,
    pub focus: bool,
    pub trust_repository: bool,
}

#[derive(Debug, Clone)]
pub struct ProjectWorktreeResult {
    pub workspace_id: String,
    pub tab_id: Option<String>,
    pub pane_id: Option<String>,
    pub checkout_path: String,
    pub branch: Option<String>,
    pub already_open: bool,
}

#[derive(Debug, Clone)]
pub struct ProjectTerminalRequest {
    pub endpoint_identity: String,
    pub workspace_id: String,
    pub cwd: String,
    pub label: String,
    pub focus: bool,
    pub env: BTreeMap<String, String>,
}

#[derive(Debug, Clone)]
pub struct ProjectTerminalResult {
    pub workspace_id: String,
    pub tab_id: String,
    pub pane_id: String,
}

#[derive(Debug, Clone)]
pub struct ProjectWorktreeRemoveRequest {
    pub endpoint_identity: String,
    pub workspace_id: String,
    pub checkout_path: String,
    pub force: bool,
}

/// Typed lifecycle operations, separate from ordinary workbench layout mutations.
#[async_trait]
pub trait ProjectHerdrAdapter: HerdrAdapter {
    async fn project_inventory(
        &self,
        session_id: &str,
        source_cwd: &str,
    ) -> Result<ProjectInventory, InspectionError>;

    async fn project_worktree(
        &self,
        session_id: &str,
        request: &ProjectWorktreeRequest,
    ) -> Result<ProjectWorktreeResult, InspectionError>;

    async fn project_terminal(
        &self,
        session_id: &str,
        request: &ProjectTerminalRequest,
    ) -> Result<ProjectTerminalResult, InspectionError>;

    async fn project_worktree_dirty(
        &self,
        checkout_path: &str,
        timeout_ms: u32,
        output_bytes: u32,
    ) -> Result<bool, InspectionError>;

    async fn project_close_workspace(
        &self,
        session_id: &str,
        endpoint_identity: &str,
        workspace_id: &str,
    ) -> Result<(), InspectionError>;

    async fn project_remove_worktree(
        &self,
        session_id: &str,
        request: &ProjectWorktreeRemoveRequest,
    ) -> Result<(), InspectionError>;
}
