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
    /// Create a new, unfocused tab whose PTY owns the direct command process.
    /// Requires a persisted launch intent. An acknowledgement is not a connected
    /// OMP attachment; uncertain effects must be reconciled, never replayed.
    async fn create_direct_agent_tab(
        &self,
        _session_id: &str,
        _request: &DirectAgentTabRequest,
    ) -> Result<RunLocation, InspectionError> {
        Err(InspectionError::new(
            "direct_agent_launch_unsupported",
            "This Herdr adapter does not support direct agent tab creation",
        ))
    }
    async fn pane_process_info(
        &self,
        session_id: &str,
        endpoint_identity: &str,
        pane_id: &str,
    ) -> Result<PaneProcessInfo, InspectionError>;
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
/// Direct-process launch provenance is recorded on the execution attachment,
/// separately from this location receipt and subsequent native OMP evidence.
#[derive(Debug, Clone)]
pub struct DirectAgentTabRequest {
    pub endpoint_identity: String,
    pub workspace_id: String,
    pub cwd: String,
    pub label: String,
    pub env: BTreeMap<String, String>,
    pub launch_tag: String,
    /// Executable followed by literal arguments, never shell command text.
    pub command: Vec<String>,
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

#[cfg(test)]
mod tests {
    use super::*;

    struct UnsupportedDirectAdapter;

    #[async_trait]
    impl OrchestrationHerdr for UnsupportedDirectAdapter {
        async fn runtime(&self, _: &str) -> Result<RuntimeView, InspectionError> {
            panic!("unsupported direct launch must not inspect runtime")
        }
        async fn create_agent_tab(&self, _: &str, _: &AgentTabRequest) -> Result<RunLocation, InspectionError> {
            panic!("unsupported direct launch must not fall back to a shell tab")
        }
        async fn pane_process_info(&self, _: &str, _: &str, _: &str) -> Result<PaneProcessInfo, InspectionError> {
            panic!("unsupported direct launch must not inspect a pane")
        }
        async fn close_pane(&self, _: &str, _: &str, _: &str) -> Result<(), InspectionError> {
            panic!("unsupported direct launch must not close a pane")
        }
    }

    #[tokio::test]
    async fn direct_launch_default_explicitly_refuses_unsupported_adapters() {
        let request = DirectAgentTabRequest {
            endpoint_identity: "endpoint".into(),
            workspace_id: "workspace".into(),
            cwd: "/work/repository".into(),
            label: "launch-tag".into(),
            env: BTreeMap::new(),
            launch_tag: "launch-tag".into(),
            command: vec!["omp".into()],
        };
        let error = UnsupportedDirectAdapter.create_direct_agent_tab("fixture", &request).await.unwrap_err();
        assert_eq!(error.code, "direct_agent_launch_unsupported");
    }
}
