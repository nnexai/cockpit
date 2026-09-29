use async_trait::async_trait;
use cockpit_core::{InspectionError, SourcePaneAdapter, SourcePaneEvidence, TabEvidence};
use serde_json::json;

use super::HerdrCliAdapter;

/// Viewer sources come solely from an endpoint-pinned authoritative snapshot.
/// Existing addon processes are ordinary terminals and are never inspected.
#[derive(Clone)]
pub(crate) struct HerdrSourcePaneAdapter {
    herdr: HerdrCliAdapter,
}

impl HerdrSourcePaneAdapter {
    pub(crate) fn new(herdr: HerdrCliAdapter) -> Self {
        Self { herdr }
    }

    async fn snapshot(
        &self,
        session_id: &str,
    ) -> Result<(cockpit_protocol::SessionSnapshotResponse, String), InspectionError> {
        self.herdr.selected_session(session_id)?;
        let (_, identity) = self.herdr
            .socket_request_with_identity(session_id, "ping", json!({}), None)
            .await?;
        let snapshot = self.herdr
            .read_structure_with_identity(session_id, Some(&identity))
            .await?;
        Ok((snapshot, identity))
    }
}

#[async_trait]
impl SourcePaneAdapter for HerdrSourcePaneAdapter {
    async fn source_pane_evidence(
        &self,
        session_id: &str,
        pane_id: &str,
    ) -> Result<SourcePaneEvidence, InspectionError> {
        if !super::valid_pane_id(pane_id) {
            return Err(InspectionError::new("invalid_pane_id", "pane ID contains unsupported characters"));
        }
        let (snapshot, endpoint_identity) = self.snapshot(session_id).await?;
        let pane = snapshot.panes.iter().find(|pane| pane.id == pane_id)
            .filter(|pane| snapshot.tabs.iter().any(|tab| tab.id == pane.tab_id && tab.space_id == pane.space_id))
            .ok_or_else(|| InspectionError::new("viewer_source_unavailable", "source terminal is absent from its tab"))?;
        Ok(SourcePaneEvidence {
            endpoint_identity,
            pane_id: pane.id.clone(),
            terminal_id: pane.terminal_id.clone(),
            workspace_id: pane.space_id.clone(),
            tab_id: pane.tab_id.clone(),
            // The snapshot adapter has already preferred foreground_cwd over
            // shell cwd and rejected relative paths in pane_folder.
            cwd: pane.cwd.clone(),
            foreground_cwd: None,
        })
    }

    async fn tab_evidence(
        &self,
        session_id: &str,
        tab_id: &str,
    ) -> Result<TabEvidence, InspectionError> {
        let (snapshot, endpoint_identity) = self.snapshot(session_id).await?;
        let tab = snapshot.tabs.iter().find(|tab| tab.id == tab_id);
        let present = tab.is_some() && snapshot.panes.iter().any(|pane| pane.tab_id == tab_id
            && tab.is_some_and(|tab| tab.space_id == pane.space_id));
        Ok(TabEvidence {
            endpoint_identity,
            server_instance: snapshot.server_instance,
            workspace_id: tab.map(|tab| tab.space_id.clone()).unwrap_or_default(),
            present,
        })
    }
}
