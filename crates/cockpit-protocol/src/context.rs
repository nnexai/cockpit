use serde::{Deserialize, Serialize};
use ts_rs::TS;

use crate::projects::ProjectDiagnostic;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
pub enum ViewerSourceKind {
    Context,
    Review,
}


#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
pub enum ContextRootKind {
    Repository,
    Companion,
    Folder,
    Library,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
pub struct ContextRoot {
    pub root_id: String,
    pub kind: ContextRootKind,
    pub label: String,
    pub path: String,
    pub repository_id: String,
    pub checkout_path: String,
    pub companion_id: Option<String>,
}


#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[serde(deny_unknown_fields)]
pub struct ContextDirectoryRequest {
    pub binding_id: String,
    pub root_id: String,
    pub path: String,
    #[serde(default)]
    #[ts(optional)]
    pub offset: Option<u32>,
    #[serde(default)]
    #[ts(optional)]
    pub revision: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
pub enum ContextFileIndexMode {
    Cached,
    Fresh,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[serde(deny_unknown_fields)]
pub struct ContextFileIndexRequest {
    pub binding_id: String,
    pub root_id: String,
    pub mode: ContextFileIndexMode,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
pub struct ContextIndexedFile {
    pub path: String,
    #[ts(type = "number | null")]
    pub bytes: Option<u64>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
pub enum ContextFileIndexSource {
    Git,
    Walk,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
pub enum ContextFileIndexState {
    Fresh,
    Cached,
    Miss,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
pub struct ContextFileIndex {
    pub binding_id: String,
    pub root_id: String,
    pub files: Vec<ContextIndexedFile>,
    pub truncated: bool,
    pub source: ContextFileIndexSource,
    pub state: ContextFileIndexState,
    pub diagnostics: Vec<ProjectDiagnostic>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
pub enum ContextEntryKind {
    Directory,
    File,
    Symlink,
    Other,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
pub struct ContextEntry {
    pub entry_id: String,
    pub name: String,
    pub path: Option<String>,
    pub kind: ContextEntryKind,
    #[ts(type = "number | null")]
    pub bytes: Option<u64>,
    pub revision: String,
    pub refusal: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
pub struct ContextDirectory {
    pub binding_id: String,
    pub root_id: String,
    pub path: String,
    pub entries: Vec<ContextEntry>,
    pub truncated: bool,
    #[serde(default)]
    #[ts(optional)]
    pub revision: Option<String>,
    #[serde(default)]
    #[ts(optional)]
    pub next_offset: Option<u32>,
    #[serde(default)]
    #[ts(optional)]
    pub total_entries: Option<u32>,
    pub diagnostics: Vec<ProjectDiagnostic>,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[serde(deny_unknown_fields)]
pub struct ContextDocumentRequest {
    pub binding_id: String,
    pub root_id: String,
    pub path: String,
    pub expected_revision: Option<String>,
    #[serde(default)]
    #[ts(optional)]
    pub offset: Option<u32>,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
pub struct ContextDocument {
    pub binding_id: String,
    pub root_id: String,
    pub path: String,
    pub revision: String,
    pub content_hash: Option<String>,
    #[ts(type = "number")]
    pub bytes: u64,
    pub media_type: String,
    pub text: Option<String>,
    pub truncated: bool,
    #[serde(default)]
    #[ts(optional)]
    pub offset: Option<u32>,
    #[serde(default)]
    #[ts(optional)]
    pub next_offset: Option<u32>,
    #[serde(default)]
    #[ts(optional)]
    #[ts(type = "number | null")]
    pub total_bytes: Option<u64>,
    #[serde(default)]
    #[ts(optional)]
    pub line_offset: Option<u32>,
    pub diagnostics: Vec<ProjectDiagnostic>,
}

