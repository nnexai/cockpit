use std::path::Path;

use cockpit_protocol::project_defaults::{WorkspaceDefaults, WorkspaceDefaultsRequest};
use cockpit_protocol::projects::{ProjectArtifact, RepositoryCandidate};

use crate::InspectionError;
use crate::repositories::{self, RepositoryCatalog};
use crate::sources::{SourceFetchRequest, source_authority_for_checkout};

use super::ProjectService;

impl ProjectService {
    pub async fn resolve_defaults(
        &self,
        request: &WorkspaceDefaultsRequest,
    ) -> Result<WorkspaceDefaults, InspectionError> {
        super::validate_text(&request.artifact_url, "artifact_url", 2048)?;
        if let Some(repository_id) = request.repository_id.as_deref() {
            super::validate_text(repository_id, "repository_id", 256)?;
        }
        let mut artifact =
            repositories::resolve_artifact(&self.configuration, &request.artifact_url)?;
        let catalog = RepositoryCatalog::new(self.configuration.clone());
        let repository_list = catalog.list().await?;
        let repositories =
            matching_repositories(&self.configuration, &artifact, repository_list.repositories)
                .await;
        let selected = select_repository(&repositories, request.repository_id.as_deref())?;

        let Some(repository) = selected else {
            return Ok(WorkspaceDefaults {
                artifact,
                repositories,
                repository_id: None,
                branch: None,
                label: None,
                checkout_path: None,
            });
        };
        let sources = self.sources.as_ref().ok_or_else(|| {
            InspectionError::new(
                "source_provider_unsupported",
                "source defaults are not configured in this host",
            )
        })?;
        let authority = source_authority_for_checkout(
            &self.configuration,
            Path::new(&repository.checkout_path),
            &artifact.provider_id,
        )
        .await?;
        let metadata = sources
            .metadata(SourceFetchRequest {
                provider_id: artifact.provider_id.clone(),
                artifact_url: artifact.canonical_url.clone(),
                authority,
            })
            .await?;
        if let Some(source_url) = metadata.source_url.clone() {
            artifact.canonical_url = source_url;
        }
        let branch = match artifact.kind.as_str() {
            "issue" => super::expand_template(
                &self.configuration.branch_template,
                &repository,
                Some(&metadata.title),
                Some(&artifact),
            )?,
            "review" => metadata.source_branch.ok_or_else(|| {
                InspectionError::new(
                    "source_provider_contract",
                    "review metadata has no source branch",
                )
            })?,
            _ => {
                return Err(InspectionError::new(
                    "unsupported_artifact",
                    "workspace defaults support issue and review artifacts",
                ));
            }
        };
        catalog.validate_branch(&repository, &branch).await?;

        Ok(WorkspaceDefaults {
            artifact,
            repositories,
            repository_id: Some(repository.repository_id),
            label: Some(branch.clone()),
            branch: Some(branch),
            // The configured checkout template includes the operation ID, which
            // does not exist during defaults lookup. Leaving this unset keeps
            // the eventual lifecycle-generated destination authoritative.
            checkout_path: None,
        })
    }
}

async fn matching_repositories(
    configuration: &cockpit_protocol::projects::ProjectConfiguration,
    artifact: &ProjectArtifact,
    repositories: Vec<RepositoryCandidate>,
) -> Vec<RepositoryCandidate> {
    let Some(expected_repository) = artifact_repository(artifact) else {
        return Vec::new();
    };
    let mut matches = Vec::new();
    for repository in repositories {
        let Ok(authority) = source_authority_for_checkout(
            configuration,
            Path::new(&repository.checkout_path),
            &artifact.provider_id,
        )
        .await
        else {
            continue;
        };
        if format!("{}/{}", authority.owner, authority.repository) == expected_repository {
            matches.push(repository);
        }
    }
    matches
}

fn artifact_repository(artifact: &ProjectArtifact) -> Option<&str> {
    match artifact.kind.as_str() {
        "issue" => artifact
            .canonical_id
            .rsplit_once('#')
            .map(|(repository, _)| repository),
        "review" => artifact
            .canonical_id
            .rsplit_once('!')
            .map(|(repository, _)| repository),
        _ => None,
    }
}

fn select_repository(
    repositories: &[RepositoryCandidate],
    requested_id: Option<&str>,
) -> Result<Option<RepositoryCandidate>, InspectionError> {
    match requested_id {
        Some(repository_id) => repositories
            .iter()
            .find(|repository| repository.repository_id == repository_id)
            .cloned()
            .map(Some)
            .ok_or_else(|| {
                InspectionError::new(
                    "repository_artifact_mismatch",
                    "selected repository does not match the artifact provider and remote origin",
                )
            }),
        None => match repositories {
            [repository] => Ok(Some(repository.clone())),
            _ => Ok(None),
        },
    }
}

#[cfg(test)]
mod tests {
    use cockpit_protocol::projects::ProjectArtifact;

    use super::{artifact_repository, select_repository};

    fn artifact(kind: &str, canonical_id: &str) -> ProjectArtifact {
        ProjectArtifact {
            provider_id: "provider".into(),
            kind: kind.into(),
            canonical_id: canonical_id.into(),
            original_url: "https://forge.test/acme/app/issues/1".into(),
            canonical_url: "https://forge.test/acme/app/issues/1".into(),
        }
    }

    #[test]
    fn extracts_issue_and_review_repository_identity() {
        assert_eq!(
            artifact_repository(&artifact("issue", "acme/app#42")),
            Some("acme/app")
        );
        assert_eq!(
            artifact_repository(&artifact("review", "acme/app!42")),
            Some("acme/app")
        );
        assert_eq!(
            artifact_repository(&artifact("wiki", "acme/app/wiki/start")),
            None
        );
    }

    #[test]
    fn no_match_and_ambiguous_matches_do_not_select_a_repository() {
        assert!(select_repository(&[], None).unwrap().is_none());
        let repository = cockpit_protocol::projects::RepositoryCandidate {
            repository_id: "one".into(),
            name: "app".into(),
            root: "/app".into(),
            checkout_path: "/app".into(),
            common_dir: "/app/.git".into(),
            branch: None,
            is_linked_worktree: false,
            is_detached: false,
            provenance: "catalog".into(),
        };
        assert!(
            select_repository(&[repository.clone(), repository], None)
                .unwrap()
                .is_none()
        );
    }
}
