use std::path::Path;
use std::sync::Arc;

use async_trait::async_trait;
use cockpit_core::InspectionError;
use cockpit_core::sources::{SourceAsset, SourceFetchRequest, SourceProvider};
use cockpit_protocol::projects::ProjectConfiguration;
use cockpit_protocol::sources::SourceCapability;

pub mod tea;

pub fn configured_providers(
    configuration: &ProjectConfiguration,
) -> Result<Vec<Arc<dyn SourceProvider>>, InspectionError> {
    configuration
        .providers
        .iter()
        .filter(|provider| tea_executable(&provider.executable))
        .map(|provider| match provider.login.as_ref() {
            Some(login) => Ok(Arc::new(tea::TeaSourceProvider::configured(
                configuration,
                &provider.id,
                login.clone(),
            )?) as Arc<dyn SourceProvider>),
            None => Ok(Arc::new(UnconfiguredTeaProvider {
                provider_id: provider.id.clone(),
            }) as Arc<dyn SourceProvider>),
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
    use super::{configured_providers, tea_executable};
    use cockpit_core::sources::{SourceAuthority, SourceFetchRequest};
    use cockpit_protocol::projects::{ProjectConfiguration, ProjectLimits, ProjectProvider};

    #[test]
    fn detects_tea_by_executable_basename() {
        assert!(tea_executable("tea"));
        assert!(tea_executable("/usr/local/bin/tea"));
        assert!(!tea_executable("gitea"));
    }

    #[test]
    fn configured_tea_without_a_login_refuses_fetches_without_startup_probing() {
        let configuration = ProjectConfiguration {
            version: 1,
            repository_roots: Vec::new(),
            worktree_root: "worktrees".into(),
            companion_root: "companions".into(),
            state_root: "state".into(),
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
            },
            origins: Default::default(),
        };
        let providers = configured_providers(&configuration).unwrap();
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
