use std::collections::HashMap;
use std::sync::Arc;

use cockpit_protocol::context::{ContextRootKind, ViewerSourceKind};
use cockpit_protocol::viewer::{ViewerContext, ViewerKind, ViewerOpenRequest, ViewerSourceOptions, ViewerSourceSelector};
use tokio::sync::Mutex;
use uuid::Uuid;

use crate::context::{ContextService, find_root};
use crate::extension_adapter::SourcePaneEvidence;
use crate::InspectionError;

const MAX_VIEWERS: usize = 256;

#[derive(Clone)]
struct ViewerEntry {
    context: ViewerContext,
    source: SourcePaneEvidence,
    server_instance: String,
    client_id: String,
}

/// A fresh authorization carries pinned source evidence, not a live addon pane.
#[derive(Clone)]
pub(crate) struct ViewerAuthorization {
    pub context: ViewerContext,
    pub source: SourcePaneEvidence,
    pub server_instance: String,
}

#[derive(Clone)]
pub struct ViewerService {
    context: Arc<ContextService>,
    entries: Arc<Mutex<HashMap<String, ViewerEntry>>>,
}

impl ViewerService {
    pub fn new(context: Arc<ContextService>) -> Self {
        Self { context, entries: Arc::new(Mutex::new(HashMap::new())) }
    }

    pub async fn sources(&self, session_id: &str, pane_id: &str) -> Result<ViewerSourceOptions, InspectionError> {
        let source = self.context.adapter.source_pane_evidence(session_id, pane_id).await?;
        if source.pane_id != pane_id {
            return Err(InspectionError::new("viewer_source_unavailable", "source terminal identity changed"));
        }
        let tab = self.context.adapter.tab_evidence(session_id, &source.tab_id).await?;
        self.prune_session(session_id, &tab.endpoint_identity).await?;
        if !tab.present || tab.endpoint_identity != source.endpoint_identity || tab.workspace_id != source.workspace_id {
            return Err(tab_absent());
        }
        self.context.source_options(session_id, &source).await
    }

    pub async fn open(&self, session_id: &str, request: &ViewerOpenRequest) -> Result<ViewerContext, InspectionError> {
        if request.client_id.is_empty() || request.client_id.len() > 256 || request.client_id.chars().any(char::is_control)
            || request.tab_id.is_empty() || request.source_pane_id.is_empty() {
            return Err(InspectionError::new("viewer_source_unavailable", "viewer source and client identity are required"));
        }
        let source = self.context.adapter.source_pane_evidence(session_id, &request.source_pane_id).await?;
        if source.pane_id != request.source_pane_id || source.tab_id != request.tab_id {
            return Err(InspectionError::new("viewer_source_not_in_tab", "source terminal is not in the requested tab"));
        }
        let tab = self.context.adapter.tab_evidence(session_id, &request.tab_id).await?;
        self.prune_session(session_id, &tab.endpoint_identity).await?;
        if !tab.present || tab.endpoint_identity != source.endpoint_identity || tab.workspace_id != source.workspace_id {
            return Err(tab_absent());
        }
        let options = self.context.source_options(session_id, &source).await?;
        let root_id = match (&request.source, request.kind) {
            (ViewerSourceSelector::FilesContext, ViewerKind::Files) => options.files_context_root_id.clone(),
            (ViewerSourceSelector::FilesFolder, ViewerKind::Files) => options.files_folder_root_id.clone(),
            (ViewerSourceSelector::Review { repository_id }, ViewerKind::Review) => options.roots.iter()
                .find(|root| root.kind == ContextRootKind::Repository && root.repository_id == *repository_id)
                .map(|root| root.root_id.clone()),
            _ => None,
        }.ok_or_else(|| InspectionError::new("viewer_source_unavailable", "requested viewer source is unavailable from this terminal"))?;
        let root = options.roots.iter().find(|root| root.root_id == root_id).expect("selected root exists");
        let source_id = if request.kind == ViewerKind::Review {
            crate::review::checkout_source_id(std::path::Path::new(&root.path))?
        } else { root.companion_id.clone().unwrap_or_else(|| root.root_id.clone()) };
        // Pin the selected root alone. A Files viewer cannot gain a repository
        // or unrelated companion merely because its source terminal can see it.
        let roots = vec![root.clone()];
        find_root(&roots, &root_id).map_err(root_not_authorized)?;
        let confirmed = self.context.adapter.tab_evidence(session_id, &request.tab_id).await?;
        if !confirmed.present || confirmed.endpoint_identity != source.endpoint_identity || confirmed.workspace_id != source.workspace_id {
            return Err(tab_absent());
        }
        let mut entries = self.entries.lock().await;
        let previous = entries.values().find(|entry| entry.context.session_id == session_id
            && entry.source.endpoint_identity == source.endpoint_identity
            && entry.context.tab_id == request.tab_id && entry.context.kind == request.kind && entry.client_id == request.client_id)
            .map(|entry| entry.context.viewer_id.clone());
        if previous.is_none() && entries.len() >= MAX_VIEWERS {
            return Err(InspectionError::new("viewer_limit", "too many open viewers; close viewers in other windows"));
        }
        let viewer_id = previous.unwrap_or_else(|| Uuid::new_v4().to_string());
        let context = ViewerContext {
            session_id: session_id.to_owned(), viewer_id: viewer_id.clone(), binding_id: Uuid::new_v4().to_string(),
            tab_id: request.tab_id.clone(), space_id: source.workspace_id.clone(), kind: request.kind,
            source_kind: if request.kind == ViewerKind::Review { ViewerSourceKind::Review } else { ViewerSourceKind::Context },
            source_id, roots, default_root_id: Some(root_id), diagnostics: options.diagnostics,
        };
        entries.insert(viewer_id, ViewerEntry { context: context.clone(), source, server_instance: confirmed.server_instance, client_id: request.client_id.clone() });
        Ok(context)
    }

    pub async fn release(&self, session_id: &str, viewer_id: &str) -> Result<(), InspectionError> {
        let mut entries = self.entries.lock().await;
        if entries.get(viewer_id).is_some_and(|entry| entry.context.session_id == session_id) {
            entries.remove(viewer_id);
        }
        Ok(())
    }

    pub(crate) async fn authorize(&self, session_id: &str, viewer_id: &str, binding_id: &str) -> Result<ViewerAuthorization, InspectionError> {
        let entry = self.entries.lock().await.get(viewer_id).filter(|entry| entry.context.session_id == session_id).cloned()
            .ok_or_else(|| InspectionError::new("viewer_not_found", "viewer context is no longer open; reopen it explicitly"))?;
        let tab = self.context.adapter.tab_evidence(session_id, &entry.context.tab_id).await?;
        if !tab.present || tab.endpoint_identity != entry.source.endpoint_identity || tab.workspace_id != entry.source.workspace_id {
            self.prune_session(session_id, &tab.endpoint_identity).await?;
            self.entries.lock().await.remove(viewer_id);
            return Err(tab_absent());
        }
        self.prune_session(session_id, &tab.endpoint_identity).await?;
        if binding_id != entry.context.binding_id { return Err(stale_binding()); }
        let options = self.context.source_options(session_id, &entry.source).await.map_err(root_not_authorized)?;
        for pinned in &entry.context.roots {
            if !options.roots.iter().any(|root| root.root_id == pinned.root_id && root.path == pinned.path
                && root.repository_id == pinned.repository_id && root.checkout_path == pinned.checkout_path && root.companion_id == pinned.companion_id) {
                return Err(root_not_authorized(InspectionError::new("context_root_not_authorized", "pinned viewer root changed")));
            }
            find_root(&entry.context.roots, &pinned.root_id).map_err(root_not_authorized)?;
        }
        // A source-switch or release racing with a slow root resolution must
        // not authorize the retired binding.
        let entries = self.entries.lock().await;
        if !entries.get(viewer_id).is_some_and(|current| current.context.binding_id == binding_id) {
            return Err(stale_binding());
        }
        Ok(ViewerAuthorization { context: entry.context, source: entry.source, server_instance: entry.server_instance })
    }

    async fn prune_session(&self, session_id: &str, endpoint: &str) -> Result<(), InspectionError> {
        let candidates: Vec<_> = self.entries.lock().await.values().filter(|entry| entry.context.session_id == session_id)
            .map(|entry| (entry.context.viewer_id.clone(), entry.context.binding_id.clone(), entry.context.tab_id.clone(), entry.source.endpoint_identity.clone())).collect();
        let mut tabs = HashMap::new();
        for (id, binding, tab_id, expected) in candidates {
            let invalid = if expected != endpoint { true } else if let Some(invalid) = tabs.get(&tab_id) {
                *invalid
            } else {
                let tab = self.context.adapter.tab_evidence(session_id, &tab_id).await?;
                let invalid = !tab.present || tab.endpoint_identity != expected;
                tabs.insert(tab_id, invalid);
                invalid
            };
            if invalid {
                let mut entries = self.entries.lock().await;
                if entries.get(&id).is_some_and(|entry| entry.context.binding_id == binding) { entries.remove(&id); }
            }
        }
        Ok(())
    }
}

fn tab_absent() -> InspectionError { InspectionError::new("viewer_tab_absent", "viewer tab or server identity is no longer current") }
fn stale_binding() -> InspectionError { InspectionError::new("context_stale_binding", "viewer source binding changed") }
fn root_not_authorized(_: InspectionError) -> InspectionError { InspectionError::new("context_root_not_authorized", "pinned viewer root is unavailable or its filesystem identity changed") }
