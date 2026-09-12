use serde::{Deserialize, Serialize};
use ts_rs::TS;

use crate::projects::ProjectDiagnostic;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
pub enum ExtensionKind {
    Context,
    Review,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
pub enum DetectionConfidence {
    VerifiedLaunch,
    VerifiedProcess,
    Candidate,
    None,
    Unsupported,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
pub enum ContextRootKind {
    Repository,
    Companion,
    Folder,
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
pub struct PanePresentation {
    pub session_id: String,
    pub pane_id: String,
    pub terminal_id: String,
    pub binding_id: String,
    pub extension: Option<ExtensionKind>,
    /// Eligible automatic replacement, distinct from detected extension identity.
    pub renderer: Option<ExtensionKind>,
    pub confidence: DetectionConfidence,
    pub reason: String,
    pub roots: Vec<ContextRoot>,
    pub default_root_id: Option<String>,
    /// Installed file-viewer support can open a fresh, source-pane-derived
    /// Folder root. This remains distinct from Cockpit companion Context.
    pub can_open_files: bool,
    /// The only Folder root accepted by `ContextLaunchRequest` when opening
    /// files from the current source pane.
    pub files_root_id: Option<String>,
    pub can_open_context: bool,
    /// Reviewr is installed/enabled at this endpoint and the current pane can
    /// launch it only from an authorized primary repository checkout.
    pub can_open_review: bool,
    pub diagnostics: Vec<ProjectDiagnostic>,
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

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
pub enum ContextSplitDirection {
    Right,
    Down,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[serde(deny_unknown_fields)]
pub struct ContextLaunchRequest {
    pub pane_id: String,
    pub binding_id: String,
    pub root_id: String,
    pub direction: ContextSplitDirection,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[serde(deny_unknown_fields)]
pub struct ReviewLaunchRequest {
    pub pane_id: String,
    pub binding_id: String,
    /// Opaque repository identity resolved from the authoritative source
    /// pane's Git checkout at the launch boundary.
    pub repository_id: String,
    pub direction: ContextSplitDirection,
}
