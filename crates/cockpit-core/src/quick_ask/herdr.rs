use async_trait::async_trait;
use cockpit_protocol::quick_ask::QuickAskLocation;
use crate::InspectionError;

#[derive(Clone)]
pub struct QuickAskBinding {
    pub session_id: String,
    pub endpoint_identity: String,
    pub server_instance: String,
    pub space_id: String,
}

#[async_trait]
pub trait QuickAskHerdr: Send + Sync {
    async fn bind(&self, session_id: &str, server_instance: &str, space_id: &str) -> Result<QuickAskBinding, InspectionError>;
    /// Create a tab that launches the harness with an explicitly sanitized environment and initial prompt.
    async fn create_tab(&self, binding: &QuickAskBinding, cwd: &str, label: &str, kind: &str, prompt: &str, path: &str) -> Result<QuickAskLocation, InspectionError>;
}
