use serde::{Deserialize, Serialize};
use ts_rs::TS;

/// A bounded raster request scoped to the currently verified Context companion.
/// `path` is always a normalized relative path; it is never a URL or host path.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(deny_unknown_fields)]
pub struct ContextMediaRequest {
    pub binding_id: String,
    pub root_id: String,
    pub path: String,
    pub expected_revision: Option<String>,
}

/// Host-authorized raster bytes. The client must use `mime_type` and the bytes
/// from this response, never reconstruct a URL from the requested path.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
pub struct ContextMedia {
    pub binding_id: String,
    pub root_id: String,
    pub path: String,
    pub revision: String,
    pub content_hash: String,
    #[ts(type = "number")]
    pub bytes: u64,
    pub mime_type: String,
    pub width: u32,
    pub height: u32,
    pub data_base64: String,
}
