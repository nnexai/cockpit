use std::path::Path;
use std::time::Duration;

use async_trait::async_trait;
use cockpit_core::InspectionError;
use cockpit_core::process::run_bounded_command;
use cockpit_core::sources::{
    FrontmatterField, FrontmatterValue, SourceAsset, SourceContainer, SourceFetchRequest,
    SourceMetadata, SourceProvider, SourceRef,
};
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

/// Read-only GitHub issue and pull request access through the owner's authenticated `gh` CLI.
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
        kind: &str,
        number: u64,
    ) -> Result<Vec<Comment>, InspectionError> {
        let mut comments = Vec::new();
        for page_number in 1..=MAX_COMMENT_PAGES {
            let response = self
                .command(&[
                    "api".into(),
                    format!("repos/{repository}/{kind}/{number}/comments"),
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
    url: String,
    #[serde(rename = "headRefName")]
    head_ref_name: Option<String>,
    #[serde(rename = "headRefOid")]
    head_ref_oid: Option<String>,
    #[serde(rename = "isCrossRepository")]
    is_cross_repository: Option<bool>,
    body: Option<String>,
    state: Option<String>,
    #[serde(rename = "isDraft")]
    is_draft: Option<bool>,
    #[serde(rename = "mergedAt")]
    merged_at: Option<String>,
    author: Option<CommentAuthor>,
    assignees: Option<Vec<CommentAuthor>>,
    #[serde(rename = "createdAt")]
    created_at: Option<String>,
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
    path: Option<String>,
    line: Option<u64>,
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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum GithubKind {
    Issue,
    PullRequest,
}

impl GithubKind {
    fn command(self) -> &'static str {
        match self {
            Self::Issue => "issue",
            Self::PullRequest => "pr",
        }
    }
    fn path(self) -> &'static str {
        match self {
            Self::Issue => "issues",
            Self::PullRequest => "pull",
        }
    }
}

fn github_artifact(
    request: &SourceFetchRequest,
    base_url: &Url,
) -> Result<(String, GithubKind, u64), InspectionError> {
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
        || !matches!(pieces[2], "issues" | "pull")
        || pieces[3].is_empty()
        || !pieces[3].bytes().all(|byte| byte.is_ascii_digit())
    {
        return Err(InspectionError::new(
            "source_artifact_unsupported",
            "GitHub artifact must identify owner, repository, and numeric issue or pull request ID",
        ));
    }
    if pieces[0] != request.authority.owner || pieces[1] != request.authority.repository {
        return Err(InspectionError::new(
            "source_artifact_mismatch",
            "artifact does not match the authorized repository",
        ));
    }
    let number = pieces[3].parse::<u64>().map_err(|_| {
        InspectionError::new("source_artifact_invalid", "GitHub issue ID is invalid")
    })?;
    let kind = if pieces[2] == "pull" {
        GithubKind::PullRequest
    } else {
        GithubKind::Issue
    };
    Ok((format!("{}/{}", pieces[0], pieces[1]), kind, number))
}

fn verify_identity(
    issue: &Issue,
    repository: &str,
    kind: GithubKind,
    number: u64,
) -> Result<(), InspectionError> {
    if issue.number != number
        || issue.url != format!("https://github.com/{repository}/{}/{number}", kind.path())
    {
        return Err(InspectionError::new(
            "source_identity_mismatch",
            "GitHub returned a different artifact URL or number",
        ));
    }
    if kind == GithubKind::PullRequest
        && issue.head_ref_oid.as_deref().is_none_or(|sha| {
            !matches!(sha.len(), 40 | 64) || !sha.bytes().all(|byte| byte.is_ascii_hexdigit())
        })
    {
        return Err(InspectionError::new(
            "source_provider_contract",
            "GitHub pull request has no valid current head revision",
        ));
    }
    Ok(())
}

fn append_comment(
    body: &mut String,
    comment: Comment,
    review: bool,
) -> Result<(), InspectionError> {
    let id = comment_id(&comment.id).ok_or_else(|| {
        InspectionError::new("source_provider_contract", "GitHub comment has no ID")
    })?;
    let author = comment
        .author
        .and_then(|author| author.login)
        .unwrap_or_else(|| "Unknown author".into());
    let created = comment.created_at.as_deref().unwrap_or_default();
    append_bounded(body, &format!("\n\n### {author} · {}", local_timestamp(created)))?;
    if let Some(updated) = comment
        .updated_at
        .as_deref()
        .filter(|updated| rfc3339_seconds(updated) != rfc3339_seconds(created))
    {
        append_bounded(body, &format!(" · edited {}", local_timestamp(updated)))?;
    }
    if review {
        if let Some(path) = comment.path.as_deref() {
            append_bounded(body, &format!(" · review on {path}"))?;
            if let Some(line) = comment.line {
                append_bounded(body, &format!(":{line}"))?;
            }
        }
    }
    append_bounded(body, "\n")?;
    if let Some(url) = comment.html_url.as_deref().or(comment.url.as_deref()) {
        append_bounded(body, &format!("[#{id}]({url})"))?;
    } else {
        append_bounded(body, &format!("#{id}"))?;
    }
    if let Some(text) = comment.body.as_deref() {
        append_bounded(body, "\n\n")?;
        append_markdown(body, text, 4)?;
    }
    Ok(())
}

fn local_timestamp(value: &str) -> String {
    let value = rfc3339_seconds(value);
    value.get(..16).unwrap_or(&value).replace('T', " ")
}

fn rfc3339_seconds(value: &str) -> String {
    if let Some(dot) = value.find('.') {
        let suffix = value[dot..]
            .find(|character| matches!(character, '+' | '-' | 'Z'))
            .map(|offset| dot + offset)
            .unwrap_or(value.len());
        format!("{}{}", &value[..dot], &value[suffix..])
    } else {
        value.to_owned()
    }
}

fn capitalize_status(value: &str) -> String {
    let mut characters = value.chars();
    characters
        .next()
        .map(|first| first.to_ascii_uppercase().to_string() + characters.as_str())
        .unwrap_or_default()
}

fn append_markdown(body: &mut String, markdown: &str, minimum_heading: usize) -> Result<(), InspectionError> {
    let mut fenced = None;
    for (index, line) in markdown.lines().enumerate() {
        if index > 0 {
            append_bounded(body, "\n")?;
        }
        let trimmed = line.trim_start();
        let indent = line.len() - trimmed.len();
        let fence = if indent <= 3 {
            trimmed
                .chars()
                .take_while(|character| matches!(character, '`' | '~'))
                .collect::<String>()
        } else {
            String::new()
        };
        if let Some(current) = fenced.as_ref() {
            append_bounded(body, line)?;
            if !fence.is_empty()
                && fence.starts_with(current)
                && trimmed[fence.len()..].trim().is_empty()
            {
                fenced = None;
            }
            continue;
        }
        if fence.len() >= 3 {
            fenced = Some(fence);
            append_bounded(body, line)?;
            continue;
        }
        let hashes = trimmed.chars().take_while(|character| *character == '#').count();
        if indent <= 3
            && (1..=6).contains(&hashes)
            && trimmed[hashes..]
                .chars()
                .next()
                .is_some_and(char::is_whitespace)
            && hashes < minimum_heading
        {
            append_bounded(body, &" ".repeat(indent))?;
            append_bounded(body, &"#".repeat(minimum_heading))?;
            append_bounded(body, &trimmed[hashes..])?;
        } else {
            append_bounded(body, line)?;
        }
    }
    Ok(())
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

fn frontmatter(key: &str, value: FrontmatterValue) -> FrontmatterField {
    FrontmatterField {
        key: key.into(),
        value,
    }
}

#[async_trait]
impl SourceProvider for GithubSourceProvider {
    fn provider_id(&self) -> &str {
        &self.provider_id
    }

    fn capabilities(&self) -> Vec<SourceCapability> {
        vec![
            SourceCapability::Issue,
            SourceCapability::IssueComments,
            SourceCapability::Review,
        ]
    }

    async fn metadata(
        &self,
        request: &SourceFetchRequest,
    ) -> Result<SourceMetadata, InspectionError> {
        let (repository, kind, number) = github_artifact(request, &self.base_url)?;
        let issue = parse_issue(
            &self
                .command(&[
                    kind.command().into(),
                    "view".into(),
                    number.to_string(),
                    "--repo".into(),
                    repository.clone(),
                    "--json".into(),
                    match kind {
                        GithubKind::Issue => "number,title,url",
                        GithubKind::PullRequest => {
                            "number,title,url,headRefName,headRefOid,isCrossRepository"
                        }
                    }
                    .into(),
                ])
                .await?,
        )?;
        verify_identity(&issue, &repository, kind, number)?;
        if kind == GithubKind::PullRequest && issue.is_cross_repository.is_none() {
            return Err(InspectionError::new(
                "source_provider_contract",
                "GitHub omitted pull request fork identity",
            ));
        }
        Ok(SourceMetadata {
            title: issue.title,
            source_branch: if issue.is_cross_repository == Some(false) {
                issue.head_ref_name
            } else {
                None
            },
            source_url: Some(issue.url),
            source_commit: issue.head_ref_oid,
            description: None,
        })
    }

    async fn fetch(
        &self,
        request: &SourceFetchRequest,
    ) -> Result<Vec<SourceAsset>, InspectionError> {
        let (repository, kind, number) = github_artifact(request, &self.base_url)?;
        let issue = parse_issue(
            &self
                .command(&[
                    kind.command().into(),
                    "view".into(),
                    number.to_string(),
                    "--repo".into(),
                    repository.clone(),
                    "--json".into(),
                    match kind {
                        GithubKind::Issue => {
                            "number,title,body,url,state,author,assignees,createdAt,updatedAt"
                        }
                        GithubKind::PullRequest => {
                            "number,title,body,url,state,isDraft,mergedAt,author,assignees,createdAt,updatedAt,headRefName,headRefOid,baseRefName"
                        }
                    }
                    .into(),
                ])
                .await?,
        )?;
        verify_identity(&issue, &repository, kind, number)?;

        let mut comments: Vec<(bool, Comment)> = self
            .fetch_comments(&repository, "issues", number)
            .await?
            .into_iter()
            .map(|comment| (false, comment))
            .collect();
        if kind == GithubKind::PullRequest {
            comments.extend(
                self.fetch_comments(&repository, "pulls", number)
                    .await?
                    .into_iter()
                    .map(|comment| (true, comment)),
            );
        }
        comments.sort_by(|(_, left), (_, right)| {
            left.created_at.cmp(&right.created_at)
        });

        let item_type = match kind {
            GithubKind::Issue => "Issue",
            GithubKind::PullRequest => "Pull request",
        };
        let status = if issue.merged_at.is_some() {
            Some("merged".to_owned())
        } else if issue.is_draft == Some(true) {
            Some("draft".to_owned())
        } else {
            issue.state.clone().map(|state| state.to_ascii_lowercase())
        };
        let author = issue.author.as_ref().and_then(|author| author.login.clone());
        let assignees: Vec<String> = issue
            .assignees
            .as_deref()
            .unwrap_or_default()
            .iter()
            .filter_map(|assignee| assignee.login.clone())
            .collect();
        let comment_count = comments.len();
        let mut summary = vec![format!("**{item_type}**")];
        if let Some(status) = &status {
            summary.push(format!("**{}**", capitalize_status(status)));
        }
        if let Some(author) = &author {
            summary.push(format!("Reporter {author}"));
        }
        if issue.assignees.is_some() {
            summary.push(if assignees.is_empty() {
                "Unassigned".into()
            } else {
                format!("Assignee {}", assignees.join(", "))
            });
        }
        let mut body = format!("{}\n", summary.join(" · "));
        if let Some(description) = issue.body.as_deref().filter(|body| !body.trim().is_empty()) {
            append_bounded(&mut body, "\n## Description\n\n")?;
            append_markdown(&mut body, description, 3)?;
        }
        if comment_count > 0 {
            append_bounded(&mut body, &format!("\n\n## Comments ({comment_count})"))?;
            for (review, comment) in comments {
                append_comment(&mut body, comment, review)?;
            }
        }

        let mut fields = vec![
            frontmatter("item_type", FrontmatterValue::String(item_type.into())),
            frontmatter("comment_count", FrontmatterValue::Number(comment_count as i64)),
        ];
        if let Some(status) = status {
            fields.push(frontmatter("status", FrontmatterValue::String(status)));
        }
        if let Some(author) = author {
            fields.push(frontmatter("author", FrontmatterValue::String(author)));
        }
        if let Some(created) = issue.created_at {
            fields.push(frontmatter("created", FrontmatterValue::String(rfc3339_seconds(&created))));
        }
        if let Some(updated) = issue.updated_at.clone() {
            fields.push(frontmatter("updated", FrontmatterValue::String(rfc3339_seconds(&updated))));
        }
        if issue.assignees.is_some() {
            fields.push(frontmatter(
                "assignee",
                if assignees.is_empty() {
                    FrontmatterValue::Null
                } else {
                    FrontmatterValue::String(assignees.join(", "))
                },
            ));
        }
        Ok(vec![SourceAsset {
            source: SourceRef {
                provider_id: self.provider_id.clone(),
                provider_instance: request.authority.provider_instance.clone(),
                resource_type: if kind == GithubKind::PullRequest {
                    "review"
                } else {
                    "issue"
                }
                .into(),
                canonical_id: format!(
                    "{repository}{}{number}",
                    if kind == GithubKind::PullRequest {
                        '!'
                    } else {
                        '#'
                    }
                ),
            },
            title: issue.title,
            source_url: Some(issue.url),
            original_url: None,
            source_revision: if kind == GithubKind::PullRequest {
                issue.head_ref_oid
            } else {
                issue.updated_at
            },
            complete: true,
            diagnostics: Vec::new(),
            body,
            container: Some(SourceContainer {
                id: repository.clone(),
                label: repository,
            }),
            fields,
            attachments: Vec::new(),
        }])
    }
}

#[cfg(test)]
mod tests {
    use super::{
        append_comment, append_markdown, ensure_comment_continuation, github_artifact,
        is_supported_base, parse_comment_page, parse_issue, Comment, CommentAuthor,
        GithubKind, GithubSourceProvider, MAX_ISSUE_BYTES, MAX_COMMENT_PAGES,
    };
    use cockpit_core::sources::{FrontmatterValue, SourceAuthority, SourceFetchRequest};
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
        ProjectConfiguration { version: 1, orchestration: Default::default(), repository_roots: Vec::new(),
        worktree_root: "worktrees".into(),
        companion_root: "companions".into(),
        state_root: "state".into(),
        cache_root: "cache".into(),
        library_root: "library".into(),
        notes_root: "notes".into(),
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
            library_folder_files: 512,
            library_folder_bytes: 32 * 1024 * 1024,
            library_file_bytes: 4 * 1024 * 1024,
            library_space_pages: 200,
            library_attachment_bytes: 25 * 1024 * 1024,
            library_item_attachment_bytes: 100 * 1024 * 1024,
            library_max_items: 20_000,
        },
        origins: Default::default(), }
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
        let kind = github_artifact(
            &request("https://github.com/nnexai/cockpit/issues/4"),
            &Url::parse("https://github.com").unwrap(),
        )
        .unwrap();
        assert_eq!(kind, ("nnexai/cockpit".into(), GithubKind::Issue, 4));
    }

    #[test]
    fn parses_issue_comments_with_timestamps_and_provenance() {
        let issue = parse_issue(
            br#"{
                "number": 4,
                "url": "https://github.com/nnexai/cockpit/issues/4",
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
        assert_eq!(
            comments.comments[0].html_url.as_deref(),
            Some("https://github.com/nnexai/cockpit/issues/4#issuecomment-17")
        );
        assert_eq!(
            ensure_comment_continuation(MAX_COMMENT_PAGES)
                .unwrap_err()
                .code,
            "source_truncated"
        );
    }

    #[test]
    fn markdown_headings_are_demoted_only_outside_fences() {
        let mut description = String::new();
        append_markdown(
            &mut description,
            "# Top\n#### Already deep\n```md\n# literal\n```\n~~~\n## also literal\n~~~",
            3,
        )
        .unwrap();
        assert_eq!(
            description,
            "### Top\n#### Already deep\n```md\n# literal\n```\n~~~\n## also literal\n~~~"
        );

        let mut comment = String::new();
        append_markdown(&mut comment, "## Comment\n##### Deep\n    # indented", 4).unwrap();
        assert_eq!(comment, "#### Comment\n##### Deep\n    # indented");
    }

    #[test]
    fn comment_cards_include_link_edit_review_context_and_safe_markdown() {
        let comment = Comment {
            id: serde_json::json!(17),
            body: Some("# Finding\n\n```md\n# literal\n```".into()),
            author: Some(CommentAuthor {
                login: Some("reviewer".into()),
            }),
            created_at: Some("2026-09-09T10:00:00Z".into()),
            updated_at: Some("2026-09-09T11:00:00Z".into()),
            url: None,
            html_url: Some("https://github.com/o/r/pull/4#discussion_r17".into()),
            path: Some("src/lib.rs".into()),
            line: Some(12),
        };
        let mut body = String::new();
        append_comment(&mut body, comment, true).unwrap();
        assert!(body.starts_with(
            "\n\n### reviewer · 2026-09-09 10:00 · edited 2026-09-09 11:00 · review on src/lib.rs:12\n[#17](https://github.com/o/r/pull/4#discussion_r17)\n\n#### Finding"
        ));
        assert!(body.contains("```md\n# literal\n```"));
    }

    #[test]
    fn comment_markdown_obeys_asset_byte_limit_boundary() {
        let at_limit = "x".repeat(MAX_ISSUE_BYTES);
        let mut body = String::new();
        append_markdown(&mut body, &at_limit, 4).unwrap();
        assert_eq!(body.len(), MAX_ISSUE_BYTES);

        let mut body = String::new();
        assert_eq!(
            append_markdown(&mut body, &format!("{at_limit}x"), 4)
                .unwrap_err()
                .code,
            "source_truncated"
        );
    }

    #[test]
    fn typed_frontmatter_values_preserve_contract_types() {
        let item_type = super::frontmatter(
            "item_type",
            FrontmatterValue::String("Pull request".into()),
        );
        let count = super::frontmatter("comment_count", FrontmatterValue::Number(3));
        assert_eq!(item_type.key, "item_type");
        assert_eq!(item_type.value, FrontmatterValue::String("Pull request".into()));
        assert_eq!(count.value, FrontmatterValue::Number(3));
    }

    #[test]
    fn rejects_other_hosts_kinds_and_repositories() {
        let base = Url::parse("https://github.com").unwrap();
        assert_eq!(
            github_artifact(
                &request("https://github.example.com/nnexai/cockpit/issues/4"),
                &base
            )
            .unwrap_err()
            .code,
            "source_artifact_mismatch"
        );
        assert_eq!(
            github_artifact(&request("https://github.com/nnexai/cockpit/pulls/4"), &base)
                .unwrap_err()
                .code,
            "source_artifact_unsupported"
        );
        assert_eq!(
            github_artifact(&request("https://github.com/other/cockpit/issues/4"), &base)
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
