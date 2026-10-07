use crate::InspectionError;
use async_trait::async_trait;
use cockpit_protocol::orchestration::{NativeShellIdentity, RunLocation};
use std::collections::BTreeMap;

#[async_trait]
pub trait OrchestrationHerdr: Send + Sync {
    async fn runtime(&self, session_id: &str) -> Result<RuntimeView, InspectionError>;
    async fn create_agent_tab(
        &self,
        session_id: &str,
        request: &AgentTabRequest,
    ) -> Result<RunLocation, InspectionError>;
    /// A successful response means the shell command was accepted, not that
    /// OMP started or bound its Cockpit integration.
    async fn start_agent(
        &self,
        session_id: &str,
        request: &AgentStartRequest,
    ) -> Result<(), InspectionError>;
    async fn pane_process_info(
        &self,
        session_id: &str,
        endpoint_identity: &str,
        pane_id: &str,
    ) -> Result<PaneProcessInfo, InspectionError>;
    /// Reconcile Herdr's expired Pending metadata on an exact recorded pane.
    /// Success is not evidence that its queued command or native process stopped.
    async fn expire_pending_agent(
        &self,
        session_id: &str,
        endpoint_identity: &str,
        pane_id: &str,
    ) -> Result<(), InspectionError>;
    /// Reclaim an exclusive recorded launch tab by closing its exact owned pane
    /// after fresh identity validation, preserving any racing foreign split.
    /// Requires a committed Core close intent; SDK-binding revocation is Core's
    /// responsibility. The acknowledgement is not an absence/cancellation proof.
    async fn close_owned_launch_tab(
        &self,
        session_id: &str,
        location: &RunLocation,
    ) -> Result<(), InspectionError>;
    /// A close acknowledgement is not proof of absence; callers must obtain a
    /// fresh observation and must never repeat an uncertain close.
    async fn close_pane(
        &self,
        session_id: &str,
        endpoint_identity: &str,
        pane_id: &str,
    ) -> Result<(), InspectionError>;
}
#[derive(Debug, Clone)]
pub struct AgentTabRequest {
    pub endpoint_identity: String,
    pub workspace_id: String,
    pub cwd: String,
    pub label: String,
    pub env: BTreeMap<String, String>,
    pub launch_tag: String,
}
#[derive(Debug, Clone)]
pub struct AgentStartRequest {
    pub endpoint_identity: String,
    pub pane_id: String,
    pub name: String,
    pub kind: String,
    pub args: Vec<String>,
    pub timeout_ms: u64,
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PaneProcessInfo {
    pub pane_id: String,
    pub shell_pid: Option<u32>,
    pub foreground_pgid: Option<u32>,
    pub processes: Vec<(u32, String)>,
    /// Fresh local OS evidence, never inferred from Herdr's shell PID/name.
    pub shell_identity: Option<NativeShellIdentity>,
}
#[derive(Debug, Clone)]
pub struct RuntimeView {
    pub endpoint_identity: String,
    pub boot_id: Option<String>,
    pub workspaces: Vec<RuntimeWorkspace>,
    pub panes: Vec<RuntimePane>,
}
#[derive(Debug, Clone)]
pub struct RuntimeWorkspace {
    pub workspace_id: String,
    pub label: String,
    pub cwd: String,
}
#[derive(Debug, Clone)]
pub struct RuntimePane {
    pub workspace_id: String,
    pub workspace_label: String,
    pub tab_id: String,
    pub tab_label: String,
    pub pane_id: String,
    pub terminal_id: Option<String>,
    pub native_session_id: Option<String>,
    pub agent_name: Option<String>,
    pub agent_kind: Option<String>,
    pub launch_pending: bool,
    pub interactive_ready: bool,
    pub agent_status: Option<String>,
    pub state_changed_at: Option<String>,
}
