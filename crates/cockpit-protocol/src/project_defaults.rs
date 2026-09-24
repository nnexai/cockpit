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
    /// The artifact's title, when the provider could be read.
    pub title: Option<String>,
    /// Work items the artifact links to, such as a Jira key in an MR.
    pub linked_artifacts: Vec<LinkedArtifact>,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
pub struct LinkedArtifact {
    pub artifact: ProjectArtifact,
    pub title: Option<String>,
    /// Why an explicitly linked item could not be read.
    pub error: Option<String>,
}
