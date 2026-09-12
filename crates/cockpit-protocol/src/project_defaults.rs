use serde::{Deserialize, Serialize};
use ts_rs::TS;

use crate::projects::{ProjectArtifact, RepositoryCandidate};

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[serde(deny_unknown_fields)]
pub struct WorkspaceDefaultsRequest {
    pub artifact_url: String,
    pub repository_id: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
pub struct WorkspaceDefaults {
    pub artifact: ProjectArtifact,
    pub repositories: Vec<RepositoryCandidate>,
    pub repository_id: Option<String>,
    pub branch: Option<String>,
    pub label: Option<String>,
    pub checkout_path: Option<String>,
}
