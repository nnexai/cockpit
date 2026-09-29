use async_trait::async_trait;

use crate::InspectionError;

/// Fresh structural evidence for a real Herdr source terminal. No addon or
/// process detection is involved in granting a local viewer a source.
#[derive(Debug, Clone)]
pub struct SourcePaneEvidence {
    pub endpoint_identity: String,
    pub pane_id: String,
    pub terminal_id: String,
    pub workspace_id: String,
    pub tab_id: String,
    pub cwd: Option<String>,
    pub foreground_cwd: Option<String>,
}

#[derive(Debug, Clone)]
pub struct TabEvidence {
    pub endpoint_identity: String,
    pub server_instance: String,
    pub workspace_id: String,
    pub present: bool,
}

#[async_trait]
pub trait SourcePaneAdapter: Send + Sync {
    async fn source_pane_evidence(
        &self,
        session_id: &str,
        pane_id: &str,
    ) -> Result<SourcePaneEvidence, InspectionError>;

    /// An absent tab is meaningful only in a fresh, successful structural read.
    async fn tab_evidence(
        &self,
        session_id: &str,
        tab_id: &str,
    ) -> Result<TabEvidence, InspectionError>;
}
