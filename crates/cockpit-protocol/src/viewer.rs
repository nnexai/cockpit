use serde::{Deserialize, Serialize};
use ts_rs::TS;
use crate::context::{ContextRoot, ViewerSourceKind};
use crate::projects::ProjectDiagnostic;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
pub enum ViewerKind { Files, Review }

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum ViewerSourceSelector {
    FilesContext,
    FilesFolder,
    Review { repository_id: String },
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
pub struct ViewerSourceOptions {
    pub session_id: String,
    pub pane_id: String,
    pub tab_id: String,
    pub space_id: String,
    pub files_context_root_id: Option<String>,
    pub files_folder_root_id: Option<String>,
    pub review_repository_ids: Vec<String>,
    pub roots: Vec<ContextRoot>,
    pub reason: String,
    pub diagnostics: Vec<ProjectDiagnostic>,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[serde(deny_unknown_fields)]
pub struct ViewerOpenRequest {
    pub tab_id: String,
    pub kind: ViewerKind,
    pub source_pane_id: String,
    pub source: ViewerSourceSelector,
    pub client_id: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
pub struct ViewerContext {
    pub session_id: String,
    pub viewer_id: String,
    pub binding_id: String,
    pub tab_id: String,
    pub space_id: String,
    pub kind: ViewerKind,
    pub source_kind: ViewerSourceKind,
    pub source_id: String,
    pub roots: Vec<ContextRoot>,
    pub default_root_id: Option<String>,
    pub diagnostics: Vec<ProjectDiagnostic>,
}
