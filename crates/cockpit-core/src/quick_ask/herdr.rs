use async_trait::async_trait;
use crate::InspectionError;

#[derive(Clone)]
pub struct QuickAskBinding {
    pub session_id: String,
    pub endpoint_identity: String,
    pub server_instance: String,
    pub space_id: String,
}
#[derive(Clone)]
pub struct QuickAskPane {
    pub tab_id: String,
    pub pane_id: String,
    pub terminal_id: String,
}
#[derive(Clone)]
pub struct QuickAskAgent {
    pub pane: QuickAskPane,
    pub kind: String,
    pub name: String,
    pub session_identity: Option<String>,
}
pub enum AgentStart { Ready(QuickAskAgent), Blocked(QuickAskAgent) }
pub enum PromptError { NotSent(InspectionError), Unknown(InspectionError) }
pub enum LaunchError { NotSent(InspectionError), Unknown(InspectionError) }

#[async_trait]
pub trait QuickAskHerdr: Send + Sync {
    async fn bind(&self, session_id: &str, server_instance: &str, space_id: &str) -> Result<QuickAskBinding, InspectionError>;
    /// Create a tab that launches the harness with an explicitly sanitized environment.
    async fn create_tab(&self, binding: &QuickAskBinding, cwd: &str, label: &str, kind: &str, name: &str, initial_prompt: Option<&str>) -> Result<QuickAskPane, LaunchError>;
    /// Observe and bind the already launched agent; never launch it again.
    async fn start_agent(&self, binding: &QuickAskBinding, pane: &QuickAskPane, kind: &str, name: &str) -> Result<AgentStart, InspectionError>;
    async fn wait_ready(&self, binding: &QuickAskBinding, agent: &QuickAskAgent, timeout_ms: u64) -> Result<bool, InspectionError>;
    async fn prompt_once(&self, binding: &QuickAskBinding, agent: &QuickAskAgent, text: &str) -> Result<(), PromptError>;
}
