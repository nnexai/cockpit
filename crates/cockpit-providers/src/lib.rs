use std::fmt;
use std::path::Path;
use std::sync::Arc;

use async_trait::async_trait;
use cockpit_core::InspectionError;
use cockpit_core::credentials::ProviderCredentials;
use cockpit_core::sources::{SourceAsset, SourceFetchRequest, SourceProvider};
use cockpit_protocol::credentials::ProviderAuthKind;
use cockpit_protocol::projects::{ProjectConfiguration, ProjectProvider};
use cockpit_protocol::sources::SourceCapability;

pub mod confluence;
pub mod github;
pub mod gitlab;
pub mod jira;
mod jira_attachments;
mod jira_wiki;
pub mod tea;

fn glab_executable(executable: &str) -> bool {
    Path::new(executable)
        .file_name()
        .is_some_and(|name| name == "glab")
}

/// The credential kinds a configured provider can hold in the OS vault.
/// Jira and Confluence take a bearer token (personal access token) or a
/// username with an API token; the other CLIs keep their own login.
pub fn credential_kinds(provider: &ProjectProvider) -> &'static [ProviderAuthKind] {
    if jira::executable(&provider.executable) || confluence::executable(&provider.executable) {
        &[ProviderAuthKind::Bearer, ProviderAuthKind::Basic]
    } else {
        &[]
    }
}

/// Provider structs derive `Debug`; the handle keeps the vault out of it.
#[derive(Clone)]
pub(crate) struct CredentialHandle(pub(crate) Arc<ProviderCredentials>);

impl CredentialHandle {
    /// Nothing stored, so a provider runs on its CLI's own login.
    pub(crate) fn none() -> Self {
        Self(ProviderCredentials::disabled())
    }
}

impl fmt::Debug for CredentialHandle {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("CredentialHandle")
    }
}

pub fn configured_providers(
    configuration: &ProjectConfiguration,
    credentials: Arc<ProviderCredentials>,
) -> Result<Vec<Arc<dyn SourceProvider>>, InspectionError> {
    configuration
        .providers
        .iter()
        .filter_map(|provider| {
            if tea_executable(&provider.executable) {
                Some(match provider.login.as_ref() {
                    Some(login) => tea::TeaSourceProvider::configured(
                        configuration,
                        &provider.id,
                        login.clone(),
                    )
                    .map(|provider| Arc::new(provider) as Arc<dyn SourceProvider>),
                    None => Ok(Arc::new(UnconfiguredTeaProvider {
                        provider_id: provider.id.clone(),
                    }) as Arc<dyn SourceProvider>),
                })
            } else if glab_executable(&provider.executable) {
                Some(
                    gitlab::GitlabSourceProvider::configured(configuration, &provider.id)
                        .map(|provider| Arc::new(provider) as Arc<dyn SourceProvider>),
                )
            } else if jira::executable(&provider.executable) {
                Some(
                    jira::JiraSourceProvider::configured(configuration, &provider.id)
                        .map(|provider| {
                            Arc::new(provider.with_credentials(credentials.clone()))
                                as Arc<dyn SourceProvider>
                        }),
                )
            } else if github::executable(&provider.executable) {
                Some(
                    github::GithubSourceProvider::configured(configuration, &provider.id)
                        .map(|provider| Arc::new(provider) as Arc<dyn SourceProvider>),
                )
            } else if confluence::executable(&provider.executable) {
                Some(
                    confluence::ConfluenceSourceProvider::configured(configuration, &provider.id)
                        .map(|provider| {
                            Arc::new(provider.with_credentials(credentials.clone()))
                                as Arc<dyn SourceProvider>
                        }),
                )
            } else {
                None
            }
        })
        .collect()
}

fn tea_executable(executable: &str) -> bool {
    Path::new(executable)
        .file_name()
        .is_some_and(|name| name == "tea")
}

struct UnconfiguredTeaProvider {
    provider_id: String,
}

#[async_trait]
impl SourceProvider for UnconfiguredTeaProvider {
    fn provider_id(&self) -> &str {
        &self.provider_id
    }

    fn capabilities(&self) -> Vec<SourceCapability> {
        vec![
            SourceCapability::Issue,
            SourceCapability::IssueComments,
            SourceCapability::Review,
            SourceCapability::Wiki,
        ]
    }

    async fn fetch(
        &self,
        _request: &SourceFetchRequest,
    ) -> Result<Vec<SourceAsset>, InspectionError> {
        Err(InspectionError::new(
            "source_login_unconfigured",
            "Tea source imports require a configured Tea login name",
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::{configured_providers, credential_kinds, github, glab_executable, tea_executable};
    use cockpit_core::credentials::ProviderCredentials;
    use cockpit_core::sources::{SourceAuthority, SourceFetchRequest};
    use cockpit_protocol::credentials::ProviderAuthKind;
    use cockpit_protocol::projects::{ProjectConfiguration, ProjectLimits, ProjectProvider};

    #[test]
    fn only_jira_and_confluence_can_hold_a_vault_credential() {
        let provider = |executable: &str| ProjectProvider {
            id: "p".into(),
            base_url: "https://x.test".into(),
            executable: executable.into(),
            login: None,
        };
        let both = [ProviderAuthKind::Bearer, ProviderAuthKind::Basic];
        assert_eq!(credential_kinds(&provider("jira")), both);
        assert_eq!(credential_kinds(&provider("/opt/bin/confluence")), both);
        for other in ["glab", "gh", "tea", "confluence-cli", "/x/unknown"] {
            assert!(credential_kinds(&provider(other)).is_empty(), "{other}");
        }
    }

    #[test]
    fn detects_tea_by_executable_basename() {
        assert!(tea_executable("tea"));
        assert!(tea_executable("/usr/local/bin/tea"));
        assert!(!tea_executable("gitea"));
    }

    #[test]
    fn detects_gitlab_cli_by_executable_basename() {
        assert!(glab_executable("glab"));
        assert!(glab_executable("/usr/local/bin/glab"));
        assert!(!glab_executable("gitlab"));
    }

    #[test]
    fn detects_github_cli_by_executable_basename() {
        assert!(github::executable("gh"));
        assert!(github::executable("/usr/local/bin/gh"));
        assert!(!github::executable("github"));
    }

    #[test]
    fn detects_confluence_cli_by_executable_basename() {
        assert!(super::confluence::executable("confluence"));
        assert!(super::confluence::executable("/home/linuxbrew/.linuxbrew/bin/confluence"));
        assert!(!super::confluence::executable("confluence-cli"));
        assert!(!super::confluence::executable("/opt/confluence/bin/start"));
    }

    #[test]
    fn configured_tea_without_a_login_refuses_fetches_without_startup_probing() {
        let configuration = ProjectConfiguration {
            version: 1,
            repository_roots: Vec::new(),
            worktree_root: "worktrees".into(),
            companion_root: "companions".into(),
            state_root: "state".into(),
            cache_root: "cache".into(),
            library_root: "library".into(),
            notes_root: "notes".into(),
            branch_template: "{repo}/{task_id}".into(),
            checkout_template: "{repo}-{task_id}".into(),
            providers: vec![ProjectProvider {
                id: "tea".into(),
                base_url: "https://forge.test".into(),
                executable: "/missing/tea".into(),
                login: None,
            }],
            limits: ProjectLimits {
                catalog_depth: 1,
                catalog_entries: 1,
                git_timeout_ms: 1,
                git_output_bytes: 1,
                operation_timeout_ms: 1,
                context_preview_bytes: 1,
                context_preview_lines: 1,
                context_directory_entries: 1,
                context_tree_depth: 1,
                library_folder_files: 512,
                library_folder_bytes: 32 * 1024 * 1024,
                library_file_bytes: 4 * 1024 * 1024,
                library_space_pages: 200,
                library_attachment_bytes: 25 * 1024 * 1024,
                library_item_attachment_bytes: 100 * 1024 * 1024,
                library_max_items: 20_000,
            },
            origins: Default::default(),
        };
        let providers = configured_providers(&configuration, ProviderCredentials::disabled()).unwrap();
        let error = tokio::runtime::Runtime::new()
            .unwrap()
            .block_on(providers[0].fetch(&SourceFetchRequest {
                provider_id: "tea".into(),
                artifact_url: "https://forge.test/acme/repo/issues/1".into(),
                authority: SourceAuthority {
                    provider_instance: "https://forge.test".into(),
                    origin_host: "forge.test".into(),
                    origin_port: None,
                    origin_base_path: String::new(),
                    owner: "acme".into(),
                    repository: "repo".into(),
                },
            }))
            .expect_err("a missing Tea login must fail at fetch time");
        assert_eq!(error.code, "source_login_unconfigured");
    }
}
