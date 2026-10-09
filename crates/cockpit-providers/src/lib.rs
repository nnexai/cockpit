use std::fmt;
use std::sync::Arc;

use async_trait::async_trait;
use cockpit_core::InspectionError;
use cockpit_core::credentials::ProviderCredentials;
use cockpit_core::sources::{SourceAsset, SourceFetchRequest, SourceProvider};
use cockpit_protocol::credentials::ProviderAuthKind;
use cockpit_protocol::projects::{ProjectConfiguration, ProjectProvider, ProviderKind};
use cockpit_protocol::sources::SourceCapability;

pub mod confluence;
mod confluence_storage;
pub mod github;
pub mod gitlab;
pub mod jira;
mod jira_attachments;
mod jira_wiki;
mod site_http;
pub mod tea;

/// The credential kinds a configured provider can hold in the OS vault.
/// Jira and Confluence take a bearer token (personal access token) or a
/// username with an API token; the other CLIs keep their own login.
pub fn credential_kinds(provider: &ProjectProvider) -> &'static [ProviderAuthKind] {
    match provider.kind {
        ProviderKind::Jira | ProviderKind::Confluence => {
            &[ProviderAuthKind::Bearer, ProviderAuthKind::Basic]
        }
        ProviderKind::Github | ProviderKind::Gitlab | ProviderKind::Gitea => &[],
    }
}

/// Provider structs derive `Debug`; the handle keeps the vault out of it.
#[derive(Clone)]
pub(crate) struct CredentialHandle(pub(crate) Arc<ProviderCredentials>);

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
        .map(|provider| match provider.kind {
            ProviderKind::Gitea => match provider.login.as_ref() {
                Some(login) => tea::TeaSourceProvider::configured(
                    configuration,
                    &provider.id,
                    login.clone(),
                )
                .map(|provider| Arc::new(provider) as Arc<dyn SourceProvider>),
                None => Ok(Arc::new(UnconfiguredTeaProvider {
                    provider_id: provider.id.clone(),
                }) as Arc<dyn SourceProvider>),
            },
            ProviderKind::Gitlab => {
                gitlab::GitlabSourceProvider::configured(configuration, &provider.id)
                    .map(|provider| Arc::new(provider) as Arc<dyn SourceProvider>)
            }
            ProviderKind::Jira => jira::JiraSourceProvider::configured(
                configuration,
                &provider.id,
                credentials.clone(),
            )
            .map(|provider| Arc::new(provider) as Arc<dyn SourceProvider>),
            ProviderKind::Github => {
                github::GithubSourceProvider::configured(configuration, &provider.id)
                    .map(|provider| Arc::new(provider) as Arc<dyn SourceProvider>)
            }
            ProviderKind::Confluence => confluence::ConfluenceSourceProvider::configured(
                configuration,
                &provider.id,
                credentials.clone(),
            )
            .map(|provider| Arc::new(provider) as Arc<dyn SourceProvider>),
        })
        .collect()
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
    use super::{configured_providers, credential_kinds};
    use cockpit_core::credentials::ProviderCredentials;
    use cockpit_core::sources::{SourceAuthority, SourceFetchRequest};
    use cockpit_protocol::credentials::ProviderAuthKind;
    use cockpit_protocol::projects::{
        ProjectConfiguration, ProjectLimits, ProjectProvider, ProviderDeployment, ProviderKind,
    };

    #[test]
    fn only_jira_and_confluence_can_hold_a_vault_credential() {
        let provider = |kind| ProjectProvider {
            id: "p".into(),
            kind,
            base_url: "https://x.test".into(),
            executable: None,
            login: None,
            deployment: Some(ProviderDeployment::DataCenter),
        };
        let both = [ProviderAuthKind::Bearer, ProviderAuthKind::Basic];
        assert_eq!(credential_kinds(&provider(ProviderKind::Jira)), both);
        assert_eq!(credential_kinds(&provider(ProviderKind::Confluence)), both);
        for kind in [ProviderKind::Gitlab, ProviderKind::Github, ProviderKind::Gitea] {
            let mut provider = provider(kind);
            provider.executable = Some("jira".into());
            provider.deployment = None;
            assert!(credential_kinds(&provider).is_empty(), "{kind:?}");
        }
    }

    fn configuration(providers: Vec<ProjectProvider>) -> ProjectConfiguration {
        ProjectConfiguration {
        repository_roots: Vec::new(),
        branch_template: "{repo}/{task_id}".into(),
        checkout_template: "{repo}-{task_id}".into(),
        providers,
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
        ..ProjectConfiguration::for_tests(std::path::Path::new(""))
        }
    }

    #[test]
    fn registry_dispatches_every_kind_without_inspecting_executable_names() {
        use cockpit_protocol::sources::SourceCapability;

        let specifications = [
            (ProviderKind::Github, "https://github.com", None),
            (ProviderKind::Gitlab, "https://gitlab.test", None),
            (ProviderKind::Gitea, "https://forge.test", None),
            (ProviderKind::Jira, "https://jira.test/jira", Some(ProviderDeployment::DataCenter)),
            (ProviderKind::Confluence, "https://wiki.test/wiki", Some(ProviderDeployment::Cloud)),
        ];
        let configuration = configuration(specifications.into_iter().map(|(kind, base_url, deployment)| {
            ProjectProvider {
                id: format!("{kind:?}"),
                kind,
                base_url: base_url.into(),
                executable: deployment.is_none().then(|| "/missing/custom-forge".into()),
                login: (kind == ProviderKind::Gitea).then(|| "fixture".into()),
                deployment,
            }
        }).collect());
        let providers = configured_providers(&configuration, ProviderCredentials::disabled()).unwrap();
        assert_eq!(providers.len(), configuration.providers.len());
        for (provider, configured) in providers.iter().zip(&configuration.providers) {
            assert_eq!(provider.provider_id(), configured.id);
            let expected = match configured.kind {
                ProviderKind::Github | ProviderKind::Gitlab => {
                    vec![SourceCapability::Issue, SourceCapability::IssueComments, SourceCapability::Review]
                }
                ProviderKind::Gitea => vec![
                    SourceCapability::Issue, SourceCapability::IssueComments,
                    SourceCapability::Review, SourceCapability::Wiki,
                ],
                ProviderKind::Jira => vec![SourceCapability::Issue, SourceCapability::IssueComments],
                ProviderKind::Confluence => vec![SourceCapability::Wiki],
            };
            assert_eq!(provider.capabilities(), expected);
        }
    }

    #[test]
    fn forge_adapters_require_an_executable() {
        for kind in [ProviderKind::Github, ProviderKind::Gitlab, ProviderKind::Gitea] {
            let configuration = configuration(vec![ProjectProvider {
                id: "forge".into(),
                kind,
                base_url: "https://github.com".into(),
                executable: None,
                login: Some("fixture".into()),
                deployment: None,
            }]);
            let error = configured_providers(&configuration, ProviderCredentials::disabled())
                .err().expect("a CLI forge must have an executable");
            assert_eq!(error.code, "source_provider_invalid", "{kind:?}");
        }
    }

    #[test]
    fn configured_tea_without_a_login_refuses_fetches_without_startup_probing() {
        let configuration = configuration(vec![ProjectProvider {
            id: "tea".into(),
            kind: ProviderKind::Gitea,
            base_url: "https://forge.test".into(),
            executable: Some("/missing/custom-forge".into()),
            login: None,
            deployment: None,
        }]);
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
