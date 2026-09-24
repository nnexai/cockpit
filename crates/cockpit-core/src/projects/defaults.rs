use std::path::Path;
use std::sync::Arc;

use cockpit_protocol::project_defaults::{
    LinkedArtifact, WorkspaceDefaults, WorkspaceDefaultsRequest,
};
use cockpit_protocol::projects::{
    ProjectArtifact, ProjectConfiguration, ProjectProvider, RepositoryCandidate,
};

use crate::InspectionError;
use crate::repositories::{self, RepositoryCatalog};
use crate::sources::{
    SourceFetchRequest, SourceMetadata, SourceService, site_authority,
    source_authority_for_checkout,
};

const MAX_LINKED_ARTIFACTS: usize = 4;

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
        // A Jira work item names no repository, so every local repository is
        // eligible and the user chooses one; forge artifacts must match.
        let independent = repositories::provider_is_repository_independent(
            &self.configuration,
            &artifact.provider_id,
        );
        let (repositories, selected) = if independent {
            let selected = match request.repository_id.as_deref() {
                Some(id) => Some(
                    repository_list
                        .repositories
                        .iter()
                        .find(|repository| repository.repository_id == id)
                        .cloned()
                        .ok_or_else(|| {
                            InspectionError::new(
                                "repository_unknown",
                                "selected repository is not in the local catalog",
                            )
                        })?,
                ),
                None => None,
            };
            (repository_list.repositories, selected)
        } else {
            let repositories =
                matching_repositories(&self.configuration, &artifact, repository_list.repositories)
                    .await;
            let selected = select_repository(&repositories, request.repository_id.as_deref())?;
            (repositories, selected)
        };
        let authority = match (&selected, independent) {
            (_, true) => site_authority(&self.configuration, &artifact.provider_id)?,
            (Some(repository), false) => {
                source_authority_for_checkout(
                    &self.configuration,
                    Path::new(&repository.checkout_path),
                    &artifact.provider_id,
                )
                .await?
            }
            (None, false) => {
                return Ok(WorkspaceDefaults {
                    artifact,
                    repositories,
                    repository_id: None,
                    branch: None,
                    label: None,
                    checkout_path: None,
                    title: None,
                    linked_artifacts: Vec::new(),
                });
            }
        };
        let sources = self.sources.as_ref().ok_or_else(|| {
            InspectionError::new(
                "source_provider_unsupported",
                "source defaults are not configured in this host",
            )
        })?;
        let metadata = sources
            .metadata_for_setup(SourceFetchRequest {
                provider_id: artifact.provider_id.clone(),
                artifact_url: artifact.canonical_url.clone(),
                authority,
            })
            .await?;
        if let Some(source_url) = metadata.source_url.clone() {
            artifact.canonical_url = source_url;
        }
        let linked_artifacts = if independent {
            Vec::new()
        } else {
            linked_work_items(&self.configuration, sources, &metadata).await
        };
        let Some(repository) = selected else {
            return Ok(WorkspaceDefaults {
                artifact,
                repositories,
                repository_id: None,
                branch: None,
                label: None,
                checkout_path: None,
                title: Some(metadata.title),
                linked_artifacts,
            });
        };
        let branch = match artifact.kind.as_str() {
            "issue" => super::expand_template(
                &self.configuration.branch_template,
                &repository,
                Some(&metadata.title),
                Some(&artifact),
            )?,
            "review" => metadata.source_branch.clone().ok_or_else(|| {
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
            title: Some(metadata.title),
            linked_artifacts,
        })
    }
}

/// Jira work items an MR or issue links to. Full Jira URLs are always kept,
/// with an error when they cannot be read. Bare keys such as `SCRUM-5` in the
/// title, branch or description are only kept when the configured Jira site
/// confirms them, so look-alikes such as `UTF-8` disappear.
async fn linked_work_items(
    configuration: &ProjectConfiguration,
    sources: &Arc<SourceService>,
    metadata: &SourceMetadata,
) -> Vec<LinkedArtifact> {
    let jira: Vec<&ProjectProvider> = configuration
        .providers
        .iter()
        .filter(|provider| repositories::is_jira_executable(&provider.executable))
        .collect();
    if jira.is_empty() {
        return Vec::new();
    }
    let text = [
        Some(metadata.title.as_str()),
        metadata.source_branch.as_deref(),
        metadata.description.as_deref(),
    ]
    .into_iter()
    .flatten()
    .collect::<Vec<_>>()
    .join("\n");
    let mut candidates: Vec<(ProjectArtifact, bool)> = Vec::new();
    let mut add = |artifact: ProjectArtifact, explicit: bool| {
        let known = candidates.iter().position(|(known, _)| {
            known.provider_id == artifact.provider_id && known.canonical_id == artifact.canonical_id
        });
        match known {
            Some(index) => candidates[index].1 |= explicit,
            None if candidates.len() < MAX_LINKED_ARTIFACTS => {
                candidates.push((artifact, explicit))
            }
            None => {}
        }
    };
    for url in url_candidates(&text) {
        if let Some(artifact) = jira
            .iter()
            .find_map(|provider| repositories::resolve_jira_url(provider, url).ok())
        {
            add(artifact, true);
        }
    }
    // A bare key is only unambiguous with a single configured Jira site.
    if let [provider] = jira.as_slice() {
        for key in key_candidates(&text) {
            if let Ok(artifact) = repositories::jira_artifact(provider, key, "") {
                let artifact = ProjectArtifact {
                    original_url: artifact.canonical_url.clone(),
                    ..artifact
                };
                add(artifact, false);
            }
        }
    }
    let mut lookups = tokio::task::JoinSet::new();
    for (index, (artifact, explicit)) in candidates.into_iter().enumerate() {
        let sources = Arc::clone(sources);
        let authority = site_authority(configuration, &artifact.provider_id);
        lookups.spawn(async move {
            let result = match authority {
                Ok(authority) => {
                    sources
                        .metadata_for_setup(SourceFetchRequest {
                            provider_id: artifact.provider_id.clone(),
                            artifact_url: artifact.canonical_url.clone(),
                            authority,
                        })
                        .await
                }
                Err(error) => Err(error),
            };
            (index, artifact, explicit, result)
        });
    }
    let mut found = Vec::new();
    while let Some(Ok((index, artifact, explicit, result))) = lookups.join_next().await {
        match result {
            Ok(metadata) => found.push((
                index,
                LinkedArtifact {
                    artifact,
                    title: Some(metadata.title),
                    error: None,
                },
            )),
            Err(error) if explicit => found.push((
                index,
                LinkedArtifact {
                    artifact,
                    title: None,
                    error: Some(error.message),
                },
            )),
            Err(_) => {}
        }
    }
    found.sort_by_key(|(index, _)| *index);
    found.into_iter().map(|(_, linked)| linked).collect()
}

fn url_candidates(text: &str) -> Vec<&str> {
    text.split(|character: char| {
        character.is_whitespace()
            || matches!(
                character,
                '<' | '>' | '"' | '\'' | '`' | '(' | ')' | '[' | ']'
            )
    })
    .filter(|token| token.starts_with("https://") || token.starts_with("http://"))
    .map(|token| {
        token.trim_end_matches(|character: char| {
            matches!(character, ',' | '.' | ';' | ':' | '!' | '?')
        })
    })
    .collect()
}

/// Words shaped like `ABC-123`, including inside branch names such as
/// `feature/ABC-123-fix`, but not inside URLs or longer words.
fn key_candidates(text: &str) -> Vec<&str> {
    let bytes = text.as_bytes();
    let mut keys = Vec::new();
    let mut index = 0;
    while index < bytes.len() {
        let boundary =
            index == 0 || !(bytes[index - 1].is_ascii_alphanumeric() || bytes[index - 1] == b'_');
        if !(boundary && bytes[index].is_ascii_uppercase()) {
            index += 1;
            continue;
        }
        let start = index;
        index += 1;
        while index < bytes.len()
            && (bytes[index].is_ascii_uppercase()
                || bytes[index].is_ascii_digit()
                || bytes[index] == b'_')
        {
            index += 1;
        }
        if index >= bytes.len() || bytes[index] != b'-' {
            continue;
        }
        let digits = index + 1;
        let mut end = digits;
        while end < bytes.len() && bytes[end].is_ascii_digit() {
            end += 1;
        }
        let followed_by_word =
            end < bytes.len() && (bytes[end].is_ascii_alphanumeric() || bytes[end] == b'_');
        if end > digits && !followed_by_word {
            keys.push(&text[start..end]);
        }
        index = end.max(index + 1);
    }
    keys
}

async fn matching_repositories(
    configuration: &ProjectConfiguration,
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

    use super::{artifact_repository, key_candidates, select_repository, url_candidates};

    #[test]
    fn finds_work_item_keys_in_titles_branches_and_prose() {
        assert_eq!(
            key_candidates(
                "SCRUM-5: fix login\nfeature/SCRUM-6-timeout\n(see ABC-12), UTF-8 and xSCRUM-7 or SCRUM-8a"
            ),
            vec!["SCRUM-5", "SCRUM-6", "ABC-12", "UTF-8"]
        );
        assert_eq!(
            url_candidates(
                "Jira: <https://team.atlassian.test/browse/SCRUM-5>. See (http://x.test/a)."
            ),
            vec![
                "https://team.atlassian.test/browse/SCRUM-5",
                "http://x.test/a"
            ]
        );
    }

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
