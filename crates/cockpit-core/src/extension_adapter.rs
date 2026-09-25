use async_trait::async_trait;
use cockpit_protocol::context::{ContextSplitDirection, DetectionConfidence, ExtensionKind};

use crate::InspectionError;

/// Internal process evidence. No argv or command line crosses the host boundary.
#[derive(Debug, Clone)]
pub struct ExtensionPaneEvidence {
    pub endpoint_identity: String,
    pub pane_id: String,
    pub terminal_id: String,
    pub workspace_id: String,
    pub tab_id: String,
    pub cwd: Option<String>,
    pub foreground_cwd: Option<String>,
    /// File-viewer launch directory from confirmed Herdr context, never inferred
    /// from workspace membership. Core resolves and authorizes its browsing root.
    pub viewer_cwd: Option<String>,
    /// Herdr's pane label. Cockpit labels the Context viewers it opens, which
    /// tells them apart from file viewers started in the same checkout.
    pub label: Option<String>,
    pub process_identity: String,
    pub extension: Option<ExtensionKind>,
    pub confidence: DetectionConfidence,
    pub reason: String,
    pub can_open_context: bool,
    pub can_open_review: bool,
}

#[derive(Debug, Clone)]
pub struct ExtensionLaunch {
    pub endpoint_identity: String,
    pub pane_id: String,
    pub terminal_id: String,
    pub workspace_id: String,
    pub cwd: String,
    pub direction: ContextSplitDirection,
    /// Label given to the opened pane.
    pub label: Option<String>,
}

#[async_trait]
pub trait ExtensionHerdrAdapter: Send + Sync {
    async fn inspect_extension_pane(
        &self,
        session_id: &str,
        pane_id: &str,
    ) -> Result<ExtensionPaneEvidence, InspectionError>;

    async fn launch_context_pane(
        &self,
        session_id: &str,
        request: &ExtensionLaunch,
    ) -> Result<ExtensionPaneEvidence, InspectionError>;

    async fn launch_review_pane(
        &self,
        session_id: &str,
        request: &ExtensionLaunch,
    ) -> Result<ExtensionPaneEvidence, InspectionError>;
}
