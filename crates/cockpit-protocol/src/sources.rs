use serde::{Deserialize, Serialize};
use ts_rs::TS;

use crate::projects::ProjectDiagnostic;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
pub enum SourceCapability {
    Issue,
    IssueComments,
    Review,
    Wiki,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
pub enum SourceFreshness {
    Fresh,
    Changed,
    Unknown,
    Unavailable,
    Conflict,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
pub enum SourceMaterializationStatus {
    Materialized,
    Unchanged,
    Conflict,
    Unsupported,
    Failed,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[serde(deny_unknown_fields)]
pub struct SourceImportRequest {
    pub binding_id: String,
    pub root_id: String,
    pub provider_id: String,
    pub artifact_url: String,
    /// Reference traversal is opt-in because following even same-repository
    /// links can produce surprising context expansion.
    #[serde(default)]
    pub hydrate_references: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[serde(deny_unknown_fields)]
pub struct SourceRefreshRequest {
    pub binding_id: String,
    pub root_id: String,
    pub source_id: String,
    #[serde(default)]
    pub hydrate_references: bool,
}

/// List the durable source records associated with one freshly authorized
/// companion root. The root identity is proof-of-location, not a filesystem
/// path supplied by the client.
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[serde(deny_unknown_fields)]
pub struct SourceListRequest {
    pub binding_id: String,
    pub root_id: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
pub struct SourceEntry {
    pub source_id: String,
    pub provider_id: String,
    pub provider_instance: String,
    pub resource_type: String,
    pub canonical_id: String,
    pub title: String,
    pub source_url: Option<String>,
    pub source_revision: Option<String>,
    pub content_hash: String,
    pub freshness: SourceFreshness,
    pub status: SourceMaterializationStatus,
    pub relative_path: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
pub struct SourceImportResponse {
    pub binding_id: String,
    pub root_id: String,
    pub entries: Vec<SourceEntry>,
    pub diagnostics: Vec<ProjectDiagnostic>,
}
