use serde::{Deserialize, Serialize};
use ts_rs::TS;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(deny_unknown_fields)]
pub struct ContextSearchRequest {
    pub binding_id: String,
    pub root_id: String,
    pub query: String,
    pub request_generation: u32,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
pub struct ContextSearchResult {
    pub path: String,
    pub line: u32,
    pub excerpt: String,
    pub revision: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
pub struct ContextSearchResponse {
    pub binding_id: String,
    pub root_id: String,
    pub query: String,
    pub request_generation: u32,
    pub results: Vec<ContextSearchResult>,
    pub scanned_files: u32,
    pub truncated: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(deny_unknown_fields)]
pub struct ContextKnownRevision {
    pub path: String,
    pub revision: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
pub enum ContextInvalidationState {
    Changed,
    Missing,
    Unavailable,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
pub struct ContextInvalidation {
    pub path: String,
    pub state: ContextInvalidationState,
    pub revision: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(deny_unknown_fields)]
pub struct ContextInvalidationRequest {
    pub binding_id: String,
    pub root_id: String,
    pub request_generation: u32,
    pub known: Vec<ContextKnownRevision>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
pub struct ContextInvalidationResponse {
    pub binding_id: String,
    pub root_id: String,
    pub request_generation: u32,
    pub invalidations: Vec<ContextInvalidation>,
    pub truncated: bool,
}
