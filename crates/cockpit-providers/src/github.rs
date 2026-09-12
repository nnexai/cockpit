use std::path::Path;
use std::time::Duration;

use async_trait::async_trait;
use cockpit_core::InspectionError;
use cockpit_core::process::run_bounded_command;
use cockpit_core::sources::{SourceAsset, SourceFetchRequest, SourceProvider, SourceRef};
use cockpit_protocol::projects::ProjectConfiguration;
use cockpit_protocol::sources::SourceCapability;
use serde::Deserialize;
use serde_json::Value;
use tokio::process::Command;
use url::Url;

const MAX_ISSUE_BYTES: usize = 1024 * 1024;
const COMMENTS_PER_PAGE: usize = 100;
const MAX_COMMENT_PAGES: usize = 5;

pub(crate) fn executable(value: &str) -> bool {
    Path::new(value)
        .file_name()
        .is_some_and(|name| name == "gh")
}

/// Read-only GitHub issue access through the owner's authenticated `gh` CLI.
/// The provider is intentionally restricted to github.com; a configured
/// Enterprise host must use a separately implemented provider instead of being
/// silently sent to the public GitHub API.
#[derive(Debug)]
pub struct GithubSourceProvider {
    provider_id: String,
    executable: String,
    base_url: Url,
    limits: (usize, Duration),
}

impl GithubSourceProvider {
    pub fn configured(
        configuration: &ProjectConfiguration,
        provider_id: &str,
    ) -> Result<Self, InspectionError> {
        let provider = configuration
            .providers
            .iter()
            .find(|provider| provider.id == provider_id)
            .ok_or_else(|| {
                InspectionError::new(
                    "source_provider_unsupported",
                    "GitHub provider is not configured",
                )
            })?;
        let base_url = Url::parse(&provider.base_url).map_err(|_| {
            InspectionError::new(
                "source_provider_invalid",
                "GitHub provider base URL is invalid",
            )
        })?;
        if !is_supported_base(&base_url) {
            return Err(InspectionError::new(
                "source_provider_invalid",
                "GitHub provider base URL must be https://github.com",
            ));
        }
        Ok(Self {
            provider_id: provider.id.clone(),
            executable: provider.executable.clone(),
            base_url,
            limits: (
                configuration.limits.git_output_bytes as usize,
                Duration::from_millis(configuration.limits.operation_timeout_ms as u64),
            ),
        })
    }

    async fn command(&self, args: &[String]) -> Result<Vec<u8>, InspectionError> {
        let mut command = Command::new(&self.executable);
        command
            .args(args)
            .env("GH_HOST", "github.com")
            .env("GH_PROMPT_DISABLED", "1")
            .env("GIT_TERMINAL_PROMPT", "0");
        let output = run_bounded_command(
            command,
            self.limits.0,
            self.limits.0,
            self.limits.1,
            "GitHub source",
        )
        .await
        .map_err(|error| {
            if matches!(error.code.as_str(), "bounded_output" | "execution_timeout")
                && args.first().is_some_and(|arg| arg == "api")
            {
                InspectionError::new(
                    "source_truncated",
                    "GitHub comment pagination exceeded Cockpit's explicit process limit",
                )
            } else {
                error
            }
        })?;
        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr).to_ascii_lowercase();
            if stderr.contains("auth")
                || stderr.contains("logged in")
                || stderr.contains("token")
                || stderr.contains("credential")
            {
                return Err(InspectionError::new(
                    "source_auth_required",
                    "GitHub CLI authentication is unavailable",
                ));
            }
            return Err(InspectionError::new(
                "source_provider_failed",
                "GitHub read request failed",
            ));
        }
        Ok(output.stdout)
    }

    async fn fetch_comments(
        &self,
        repository: &str,
        number: u64,
    ) -> Result<Vec<Comment>, InspectionError> {
        let mut comments = Vec::new();
        for page_number in 1..=MAX_COMMENT_PAGES {
            let response = self
                .command(&[
                    "api".into(),
                    format!("repos/{repository}/issues/{number}/comments"),
                    "--method".into(),
                    "GET".into(),
                    "--hostname".into(),
                    "github.com".into(),
                    "--include".into(),
                    "--field".into(),
                    format!("per_page={COMMENTS_PER_PAGE}"),
                    "--field".into(),
                    format!("page={page_number}"),
                ])
                .await?;
            let page = parse_comment_page(&response)?;
            comments.extend(page.comments);
            if !page.has_next {
                return Ok(comments);
            }
            ensure_comment_continuation(page_number)?;
        }
        unreachable!("bounded comment page loop always returns")
    }
}

#[derive(Debug, Deserialize)]
struct Issue {
    number: u64,
    title: String,
    body: Option<String>,
    #[serde(rename = "updatedAt")]
    updated_at: Option<String>,
}

#[derive(Debug, Deserialize)]
struct Comment {
    id: Value,
    body: Option<String>,
    #[serde(alias = "user")]
    author: Option<CommentAuthor>,
    #[serde(rename = "createdAt", alias = "created_at")]
    created_at: Option<String>,
    #[serde(rename = "updatedAt", alias = "updated_at")]
    updated_at: Option<String>,
    url: Option<String>,
    #[serde(rename = "html_url")]
    html_url: Option<String>,
}

#[derive(Debug, Deserialize)]
struct CommentAuthor {
    login: Option<String>,
}

fn is_supported_base(url: &Url) -> bool {
    url.scheme() == "https"
        && url.host_str() == Some("github.com")
        && url.port().is_none()
        && url.username().is_empty()
        && url.password().is_none()
        && url.query().is_none()
        && url.fragment().is_none()
        && matches!(url.path(), "" | "/")
}

fn provider_instance(url: &Url) -> String {
    format!("{}://{}", url.scheme(), url.host_str().unwrap_or_default())
}

fn issue_kind(
    request: &SourceFetchRequest,
    base_url: &Url,
) -> Result<(String, u64), InspectionError> {
    let url = Url::parse(&request.artifact_url)
        .map_err(|_| InspectionError::new("source_artifact_invalid", "artifact URL is invalid"))?;
    let instance = provider_instance(base_url);
    if !is_supported_base(base_url)
        || request.authority.provider_instance != instance
        || request.authority.origin_host != "github.com"
        || request.authority.origin_port.is_some()
        || !request.authority.origin_base_path.is_empty()
        || url.scheme() != "https"
        || url.host_str() != Some("github.com")
        || url.port().is_some()
        || !url.username().is_empty()
        || url.password().is_some()
        || url.query().is_some()
        || url.fragment().is_some()
    {
        return Err(InspectionError::new(
            "source_artifact_mismatch",
            "artifact does not match configured GitHub provider",
        ));
    }
    let pieces: Vec<_> = url.path().trim_matches('/').split('/').collect();
    if pieces.len() != 4
        || pieces[..3]
            .iter()
            .any(|piece| piece.is_empty() || *piece == "." || *piece == ".." || piece.contains('%'))
        || pieces[2] != "issues"
        || pieces[3].is_empty()
        || !pieces[3].bytes().all(|byte| byte.is_ascii_digit())
    {
        return Err(InspectionError::new(
            "source_artifact_unsupported",
            "GitHub artifact must identify owner, repository, and numeric issue ID",
        ));
    }
    if pieces[0] != request.authority.owner || pieces[1] != request.authority.repository {
        return Err(InspectionError::new(
            "source_artifact_mismatch",
            "artifact does not match the verified local primary repository",
        ));
    }
    let number = pieces[3].parse::<u64>().map_err(|_| {
        InspectionError::new("source_artifact_invalid", "GitHub issue ID is invalid")
    })?;
    Ok((format!("{}/{}", pieces[0], pieces[1]), number))
}

fn append_bounded(body: &mut String, value: &str) -> Result<(), InspectionError> {
    body.push_str(value);
    if body.len() > MAX_ISSUE_BYTES {
        return Err(InspectionError::new(
            "source_truncated",
            "GitHub issue body and comments exceed Cockpit's explicit byte limit",
        ));
    }
    Ok(())
}

fn parse_issue(value: &[u8]) -> Result<Issue, InspectionError> {
    serde_json::from_slice(value).map_err(|_| {
        InspectionError::new(
            "source_provider_contract",
            "GitHub issue JSON did not match the verified contract",
        )
    })
}

struct CommentPage {
    comments: Vec<Comment>,
    has_next: bool,
}

fn parse_comment_page(value: &[u8]) -> Result<CommentPage, InspectionError> {
    let value = std::str::from_utf8(value).map_err(|_| {
        InspectionError::new(
            "source_provider_contract",
            "GitHub comment response was not UTF-8",
        )
    })?;
    let (headers, body) = value
        .split_once("\r\n\r\n")
        .or_else(|| value.split_once("\n\n"))
        .ok_or_else(|| {
            InspectionError::new(
                "source_provider_contract",
                "GitHub comment response omitted HTTP headers or body",
            )
        })?;
    let has_next = headers.lines().any(|line| {
        let line = line.to_ascii_lowercase();
        line.starts_with("link:") && (line.contains("rel=\"next\"") || line.contains("rel=next"))
    });
    let comments = serde_json::from_str(body).map_err(|_| {
        InspectionError::new(
            "source_provider_contract",
            "GitHub comment JSON did not match the verified contract",
        )
    })?;
    Ok(CommentPage { comments, has_next })
}

fn ensure_comment_continuation(page_number: usize) -> Result<(), InspectionError> {
    if page_number == MAX_COMMENT_PAGES {
        Err(InspectionError::new(
            "source_truncated",
            "GitHub issue comments exceeded Cockpit's explicit page limit",
        ))
    } else {
        Ok(())
    }
}

fn comment_id(value: &Value) -> Option<String> {
    value
        .as_str()
        .filter(|value| !value.is_empty())
        .map(str::to_owned)
        .or_else(|| value.as_u64().map(|number| number.to_string()))
}

#[async_trait]
impl SourceProvider for GithubSourceProvider {
    fn provider_id(&self) -> &str {
        &self.provider_id
    }

    fn capabilities(&self) -> Vec<SourceCapability> {
        vec![SourceCapability::Issue, SourceCapability::IssueComments]
    }

    async fn fetch(
        &self,
        request: &SourceFetchRequest,
    ) -> Result<Vec<SourceAsset>, InspectionError> {
        let (repository, number) = issue_kind(request, &self.base_url)?;
        let issue = parse_issue(
            &self
                .command(&[
                    "issue".into(),
                    "view".into(),
                    number.to_string(),
                    "--repo".into(),
                    repository.clone(),
                    "--json".into(),
                    "number,title,body,updatedAt".into(),
                ])
                .await?,
        )?;
        if issue.number != number {
            return Err(InspectionError::new(
                "source_provider_contract",
                "GitHub returned a different issue number",
            ));
        }
        let mut body = String::new();
        if let Some(issue_body) = issue.body.as_deref() {
            append_bounded(&mut body, issue_body)?;
        }
        for comment in self.fetch_comments(&repository, number).await? {
            let comment_id = comment_id(&comment.id).ok_or_else(|| {
                InspectionError::new("source_provider_contract", "GitHub comment has no ID")
            })?;
            append_bounded(&mut body, &format!("\n\n## GitHub comment {comment_id}\n"))?;
            for (label, value) in [
                ("Author", comment.author.and_then(|author| author.login)),
                ("Created", comment.created_at),
                ("Updated", comment.updated_at),
                ("URL", comment.html_url.or(comment.url)),
            ] {
                if let Some(value) = value {
                    append_bounded(&mut body, &format!("{label}: {value}\n"))?;
                }
            }
            if let Some(comment_body) = comment.body {
                append_bounded(&mut body, &format!("\n{comment_body}"))?;
            }
        }
        Ok(vec![SourceAsset {
            source: SourceRef {
                provider_id: self.provider_id.clone(),
                provider_instance: request.authority.provider_instance.clone(),
                resource_type: "issue".into(),
                canonical_id: format!("{repository}#{number}"),
            },
            title: issue.title,
            source_url: Some(request.artifact_url.clone()),
            source_revision: issue.updated_at,
            body,
        }])
    }
}

#[cfg(test)]
mod tests {
    use super::{
        GithubSourceProvider, MAX_COMMENT_PAGES, ensure_comment_continuation, is_supported_base,
        issue_kind, parse_comment_page, parse_issue,
    };
    use cockpit_core::sources::{SourceAuthority, SourceFetchRequest};
    use cockpit_protocol::projects::{ProjectConfiguration, ProjectLimits, ProjectProvider};
    use url::Url;

    fn authority() -> SourceAuthority {
        SourceAuthority {
            provider_instance: "https://github.com".into(),
            origin_host: "github.com".into(),
            origin_port: None,
            origin_base_path: String::new(),
            owner: "nnexai".into(),
            repository: "cockpit".into(),
        }
    }

    fn request(url: &str) -> SourceFetchRequest {
        SourceFetchRequest {
            provider_id: "github".into(),
            artifact_url: url.into(),
            authority: authority(),
        }
    }

    fn config(base_url: &str) -> ProjectConfiguration {
        ProjectConfiguration {
            version: 1,
            repository_roots: Vec::new(),
            worktree_root: "worktrees".into(),
            companion_root: "companions".into(),
            state_root: "state".into(),
            branch_template: "{repo}/{task_id}".into(),
            checkout_template: "{repo}-{task_id}".into(),
            providers: vec![ProjectProvider {
                id: "github".into(),
                base_url: base_url.into(),
                executable: "gh".into(),
                login: None,
            }],
            limits: ProjectLimits {
                catalog_depth: 1,
                catalog_entries: 1,
                git_timeout_ms: 1000,
                git_output_bytes: 1024,
                operation_timeout_ms: 1000,
                context_preview_bytes: 1024,
                context_preview_lines: 1,
                context_directory_entries: 1,
                context_tree_depth: 1,
            },
            origins: Default::default(),
        }
    }

    #[test]
    fn accepts_only_public_github_base() {
        assert!(is_supported_base(
            &Url::parse("https://github.com/").unwrap()
        ));
        assert!(!is_supported_base(
            &Url::parse("https://github.example.com/").unwrap()
        ));
        assert!(!is_supported_base(
            &Url::parse("http://github.com/").unwrap()
        ));
    }

    #[test]
    fn parses_issue_and_preserves_repository_authority() {
        let kind = issue_kind(
            &request("https://github.com/nnexai/cockpit/issues/4"),
            &Url::parse("https://github.com").unwrap(),
        )
        .unwrap();
        assert_eq!(kind, ("nnexai/cockpit".into(), 4));
    }

    #[test]
    fn parses_issue_comments_with_timestamps_and_provenance() {
        let issue = parse_issue(
            br#"{
                "number": 4,
                "title": "Fix drift",
                "body": "Description",
                "updatedAt": "2026-09-10T13:01:16Z"
            }"#,
        )
        .unwrap();
        let comments = parse_comment_page(
            br#"HTTP/1.1 200 OK
Link: <https://api.github.com/repos/nnexai/cockpit/issues/4/comments?page=2>; rel="next"

            [{
                    "id": 17,
                    "body": "Details",
                    "user": {"login": "reviewer"},
                    "created_at": "2026-09-09T10:00:00Z",
                    "updated_at": "2026-09-09T11:00:00Z",
                    "html_url": "https://github.com/nnexai/cockpit/issues/4#issuecomment-17"
            }]
            "#,
        )
        .unwrap();
        assert_eq!(issue.number, 4);
        assert_eq!(issue.updated_at.as_deref(), Some("2026-09-10T13:01:16Z"));
        assert!(comments.has_next);
        assert_eq!(comments.comments.len(), 1);
        assert_eq!(
            comments.comments[0]
                .author
                .as_ref()
                .unwrap()
                .login
                .as_deref(),
            Some("reviewer")
        );
        assert_eq!(
            comments.comments[0].created_at.as_deref(),
            Some("2026-09-09T10:00:00Z")
        );
        assert_eq!(comments.comments[0].html_url.as_deref(), Some("https://github.com/nnexai/cockpit/issues/4#issuecomment-17"));
        assert_eq!(
            ensure_comment_continuation(MAX_COMMENT_PAGES)
                .unwrap_err()
                .code,
            "source_truncated"
        );
    }

    #[test]
    fn rejects_other_hosts_kinds_and_repositories() {
        let base = Url::parse("https://github.com").unwrap();
        assert_eq!(
            issue_kind(
                &request("https://github.example.com/nnexai/cockpit/issues/4"),
                &base
            )
            .unwrap_err()
            .code,
            "source_artifact_mismatch"
        );
        assert_eq!(
            issue_kind(&request("https://github.com/nnexai/cockpit/pulls/4"), &base)
                .unwrap_err()
                .code,
            "source_artifact_unsupported"
        );
        assert_eq!(
            issue_kind(&request("https://github.com/other/cockpit/issues/4"), &base)
                .unwrap_err()
                .code,
            "source_artifact_mismatch"
        );
    }

    #[test]
    fn rejects_enterprise_configuration_in_factory() {
        let error =
            GithubSourceProvider::configured(&config("https://github.example.com"), "github")
                .unwrap_err();
        assert_eq!(error.code, "source_provider_invalid");
    }

}
