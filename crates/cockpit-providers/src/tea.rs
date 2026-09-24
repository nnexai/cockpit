use std::time::Duration;

use async_trait::async_trait;
use cockpit_core::InspectionError;
use cockpit_core::process::run_bounded_command;
use cockpit_core::sources::{
    SourceAsset, SourceFetchRequest, SourceMetadata, SourceProvider, SourceRef,
};
use cockpit_protocol::projects::ProjectConfiguration;
use cockpit_protocol::sources::SourceCapability;
use serde::Deserialize;
use serde_json::Value;
use tokio::process::Command;
use url::Url;
const MAX_COMMENT_PAGES: u32 = 5;
const COMMENTS_PER_PAGE: u32 = 100;
const MAX_COMMENT_BYTES: usize = 1024 * 1024;
const MAX_WIKI_BYTES: usize = 1024 * 1024;
const WIKI_REVISIONS_PER_PAGE: u32 = 1;

/// Tea 0.15 JSON contract: `tea issues <index> --output json --repo owner/repo`
/// followed by `tea comments list <index> --output json --page N --limit N`.
/// Both calls are fixed read-only argv; no `tea api` or mutating subcommand is used.
pub struct TeaSourceProvider {
    provider_id: String,
    executable: String,
    base_url: Url,
    login: String,
    limits: (usize, Duration),
    config_home: Option<std::path::PathBuf>,
}
impl TeaSourceProvider {
    pub fn configured(
        configuration: &ProjectConfiguration,
        provider_id: &str,
        login: String,
    ) -> Result<Self, InspectionError> {
        let provider = configuration
            .providers
            .iter()
            .find(|p| p.id == provider_id)
            .ok_or_else(|| {
                InspectionError::new(
                    "source_provider_unsupported",
                    "Tea provider is not configured",
                )
            })?;
        let base_url = Url::parse(&provider.base_url).map_err(|_| {
            InspectionError::new(
                "source_provider_invalid",
                "Tea provider base URL is invalid",
            )
        })?;
        if !matches!(base_url.scheme(), "http" | "https")
            || base_url.username() != ""
            || base_url.password().is_some()
            || base_url.query().is_some()
            || base_url.fragment().is_some()
            || base_url.host_str().is_none()
        {
            return Err(InspectionError::new(
                "source_provider_invalid",
                "Tea provider URL must not contain credentials",
            ));
        }
        Ok(Self {
            provider_id: provider.id.clone(),
            executable: provider.executable.clone(),
            base_url,
            login,
            limits: (
                configuration.limits.git_output_bytes as usize,
                Duration::from_millis(configuration.limits.operation_timeout_ms as u64),
            ),
            config_home: None,
        })
    }
    async fn command(&self, args: &[String]) -> Result<Vec<u8>, InspectionError> {
        let mut c = Command::new(&self.executable);
        c.args(args).env("GIT_TERMINAL_PROMPT", "0");
        if let Some(home) = &self.config_home {
            c.env("XDG_CONFIG_HOME", home);
        }
        let out = run_bounded_command(c, self.limits.0, self.limits.0, self.limits.1, "tea source")
            .await?;
        if !out.status.success() {
            return Err(InspectionError::new(
                "source_provider_failed",
                "Tea read request failed",
            ));
        }
        Ok(out.stdout)
    }

    async fn fetch_review(
        &self,
        request: &SourceFetchRequest,
        repo: &str,
        index: u64,
    ) -> Result<Vec<SourceAsset>, InspectionError> {
        let review: Value = serde_json::from_slice(
            &self
                .command(&vec![
                    "pulls".into(),
                    index.to_string(),
                    "--output".into(),
                    "json".into(),
                    "--fields".into(),
                    "index,title,body,url,base,base-commit,head,updated".into(),
                    "--repo".into(),
                    repo.into(),
                    "--login".into(),
                    self.login.clone(),
                ])
                .await?,
        )
        .map_err(|_| {
            InspectionError::new(
                "source_provider_contract",
                "Tea pull request JSON did not match the verified contract",
            )
        })?;
        if value_string(&review, &["index"]).and_then(|value| value.parse::<u64>().ok())
            != Some(index)
        {
            return Err(InspectionError::new(
                "source_provider_contract",
                "Tea returned a different pull request index",
            ));
        }
        let mut comments = Vec::new();
        for page in 1..=MAX_COMMENT_PAGES {
            let page_comments: Vec<Value> = serde_json::from_slice(
                &self
                    .command(&vec![
                        "comments".into(),
                        "list".into(),
                        index.to_string(),
                        "--output".into(),
                        "json".into(),
                        "--page".into(),
                        page.to_string(),
                        "--limit".into(),
                        COMMENTS_PER_PAGE.to_string(),
                        "--repo".into(),
                        repo.into(),
                        "--login".into(),
                        self.login.clone(),
                    ])
                    .await?,
            )
            .map_err(|_| {
                InspectionError::new(
                    "source_provider_contract",
                    "Tea pull-request comment JSON did not match the verified contract",
                )
            })?;
            let full = page_comments.len() == COMMENTS_PER_PAGE as usize;
            comments.extend(page_comments);
            if !full {
                break;
            }
            if page == MAX_COMMENT_PAGES {
                return Err(InspectionError::new(
                    "source_truncated",
                    "Tea pull-request comment pagination reached Cockpit's explicit limit",
                ));
            }
        }
        let title = value_string(&review, &["title"]).ok_or_else(|| {
            InspectionError::new("source_provider_contract", "Tea pull request has no title")
        })?;
        let mut body = String::new();
        append_bounded(
            &mut body,
            "## Review metadata\n\nProvider positions below are unverified reference metadata. Cockpit does not use them as local anchors.\n",
            MAX_COMMENT_BYTES,
        )?;
        for (label, value) in [
            ("Base", value_reference(&review, "base")),
            (
                "Base commit",
                value_string(&review, &["base_commit", "base-commit"]),
            ),
            ("Head", value_reference(&review, "head")),
            (
                "Head commit",
                value_string(&review, &["head_commit", "head-commit"]),
            ),
        ] {
            if let Some(value) = value {
                append_bounded(
                    &mut body,
                    &format!("- {label}: {value}\n"),
                    MAX_COMMENT_BYTES,
                )?;
            }
        }
        append_bounded(
            &mut body,
            "- Diff and structured changed-file metadata: unavailable in Tea 0.15.1's verified read-only JSON path.\n- Review summaries and reply threads: unavailable in Tea 0.15.1's documented read-only JSON commands.\n",
            MAX_COMMENT_BYTES,
        )?;
        if let Some(description) = value_string(&review, &["body"]) {
            append_bounded(&mut body, "\n## Description\n\n", MAX_COMMENT_BYTES)?;
            append_bounded(&mut body, &description, MAX_COMMENT_BYTES)?;
        }
        for comment in comments {
            let id = value_string(&comment, &["id"]).ok_or_else(|| {
                InspectionError::new("source_provider_contract", "Tea review comment has no ID")
            })?;
            append_bounded(
                &mut body,
                &format!("\n\n## Discussion comment {id}\n"),
                MAX_COMMENT_BYTES,
            )?;
            for (label, value) in [
                ("Path", value_string(&comment, &["path"])),
                ("Line", value_string(&comment, &["line"])),
                ("Created", value_string(&comment, &["created"])),
                ("Updated", value_string(&comment, &["updated"])),
            ] {
                if let Some(value) = value {
                    append_bounded(&mut body, &format!("{label}: {value}\n"), MAX_COMMENT_BYTES)?;
                }
            }
            if let Some(comment_body) = value_string(&comment, &["body"]) {
                append_bounded(&mut body, "\n", MAX_COMMENT_BYTES)?;
                append_bounded(&mut body, &comment_body, MAX_COMMENT_BYTES)?;
            }
        }
        let review_comments: Vec<Value> = serde_json::from_slice(
            &self
                .command(&vec![
                    "pulls".into(),
                    "review-comments".into(),
                    index.to_string(),
                    "--output".into(),
                    "json".into(),
                    "--fields".into(),
                    "id,body,reviewer,path,line,resolver,created,updated,url".into(),
                    "--repo".into(),
                    repo.into(),
                    "--login".into(),
                    self.login.clone(),
                ])
                .await?,
        )
        .map_err(|_| {
            InspectionError::new(
                "source_provider_contract",
                "Tea review-comment JSON did not match the verified contract",
            )
        })?;
        for comment in review_comments {
            let id = value_string(&comment, &["id"]).ok_or_else(|| {
                InspectionError::new("source_provider_contract", "Tea review comment has no ID")
            })?;
            append_bounded(
                &mut body,
                &format!("\n\n## Review comment {id}\n"),
                MAX_COMMENT_BYTES,
            )?;
            for (label, value) in [
                ("Path", value_string(&comment, &["path"])),
                ("Line", value_string(&comment, &["line"])),
                ("Reviewer", value_string(&comment, &["reviewer"])),
                ("Resolver", value_string(&comment, &["resolver"])),
                (
                    "Created",
                    value_string(&comment, &["created", "created_at"]),
                ),
                (
                    "Updated",
                    value_string(&comment, &["updated", "updated_at"]),
                ),
                ("URL", value_string(&comment, &["url", "html_url"])),
            ] {
                if let Some(value) = value {
                    append_bounded(&mut body, &format!("{label}: {value}\n"), MAX_COMMENT_BYTES)?;
                }
            }
            if let Some(comment_body) = value_string(&comment, &["body"]) {
                append_bounded(&mut body, "\n", MAX_COMMENT_BYTES)?;
                append_bounded(&mut body, &comment_body, MAX_COMMENT_BYTES)?;
            }
        }
        Ok(vec![SourceAsset {
            source: SourceRef {
                provider_id: self.provider_id.clone(),
                provider_instance: request.authority.provider_instance.clone(),
                resource_type: "review".into(),
                canonical_id: format!("{repo}!{index}"),
            },
            title,
            source_url: Some(request.artifact_url.clone()),
            original_url: None,
            source_revision: value_string(
                &review,
                &["headSha", "head_commit", "head-commit", "updated"],
            ),
            complete: true,
            diagnostics: Vec::new(),
            body,
        }])
    }

    async fn fetch_wiki(
        &self,
        request: &SourceFetchRequest,
        repo: &str,
        page: &str,
    ) -> Result<Vec<SourceAsset>, InspectionError> {
        let wiki: Value = serde_json::from_slice(
            &self
                .command(&vec![
                    "wiki".into(),
                    "view".into(),
                    page.into(),
                    "--output".into(),
                    "json".into(),
                    "--repo".into(),
                    repo.into(),
                    "--login".into(),
                    self.login.clone(),
                ])
                .await?,
        )
        .map_err(|_| {
            InspectionError::new(
                "source_provider_contract",
                "Tea wiki JSON did not match the verified contract",
            )
        })?;
        let wiki = match wiki {
            Value::Array(mut pages) if pages.len() == 1 => pages.pop().unwrap(),
            Value::Array(_) => {
                return Err(InspectionError::new(
                    "source_provider_contract",
                    "Tea returned an ambiguous wiki page result",
                ));
            }
            page => page,
        };
        let title = value_string(&wiki, &["title", "name"]).ok_or_else(|| {
            InspectionError::new("source_provider_contract", "Tea wiki page has no title")
        })?;
        let body = value_string(&wiki, &["content"]).ok_or_else(|| {
            InspectionError::new("source_provider_contract", "Tea wiki page has no content")
        })?;
        if body.len() > MAX_WIKI_BYTES {
            return Err(InspectionError::new(
                "source_truncated",
                "Tea wiki content exceeds Cockpit's explicit byte limit",
            ));
        }
        let source_revision = if let Some(revision) =
            value_string(&wiki, &["sha", "updated"]).filter(|value| !value.is_empty())
        {
            revision
        } else {
            let revisions: Vec<Value> = serde_json::from_slice(
                &self
                    .command(&vec![
                        "wiki".into(),
                        "revisions".into(),
                        page.into(),
                        "--output".into(),
                        "json".into(),
                        "--fields".into(),
                        "sha,date".into(),
                        "--page".into(),
                        "1".into(),
                        "--limit".into(),
                        WIKI_REVISIONS_PER_PAGE.to_string(),
                        "--repo".into(),
                        repo.into(),
                        "--login".into(),
                        self.login.clone(),
                    ])
                    .await?,
            )
            .map_err(|_| {
                InspectionError::new(
                    "source_provider_contract",
                    "Tea wiki-revision JSON did not match the verified contract",
                )
            })?;
            let first_revision = revisions.into_iter().next();
            first_revision
                .as_ref()
                .and_then(|revision| value_string(revision, &["sha", "date"]))
                .filter(|value| !value.is_empty())
                .ok_or_else(|| {
                    InspectionError::new(
                        "source_provider_contract",
                        "Tea wiki page has no current revision",
                    )
                })?
        };
        Ok(vec![SourceAsset {
            source: SourceRef {
                provider_id: self.provider_id.clone(),
                provider_instance: request.authority.provider_instance.clone(),
                resource_type: "wiki".into(),
                canonical_id: format!("{repo}:{page}"),
            },
            title,
            source_url: Some(request.artifact_url.clone()),
            original_url: None,
            source_revision: Some(source_revision),
            complete: true,
            diagnostics: Vec::new(),
            body,
        }])
    }

    #[cfg(test)]
    fn with_config_home(mut self, path: std::path::PathBuf) -> Self {
        self.config_home = Some(path);
        self
    }
}

fn provider_instance(url: &Url) -> String {
    let port = normalized_port(url)
        .map(|port| format!(":{port}"))
        .unwrap_or_default();
    let path = url.path().trim_end_matches('/');
    format!(
        "{}://{}{}{}",
        url.scheme().to_ascii_lowercase(),
        url.host_str().unwrap().to_ascii_lowercase(),
        port,
        path,
    )
}

fn base_path(path: &str) -> String {
    let path = path.trim_end_matches('/');
    if path.is_empty() {
        "/".into()
    } else {
        path.into()
    }
}

fn normalized_port(url: &Url) -> Option<u16> {
    let default_port = match url.scheme() {
        "http" => 80,
        "https" => 443,
        _ => return url.port(),
    };
    url.port().filter(|port| *port != default_port)
}
#[derive(Deserialize)]
struct Issue {
    index: u64,
    title: String,
    body: Option<String>,
    updated: Option<String>,
}
#[derive(Deserialize)]
struct Comment {
    id: String,
    body: Option<String>,
}

enum ArtifactKind {
    Issue(u64),
    Review(u64),
    Wiki(String),
}

fn artifact_kind(
    request: &SourceFetchRequest,
    base_url: &Url,
) -> Result<(String, ArtifactKind), InspectionError> {
    let url = Url::parse(&request.artifact_url)
        .map_err(|_| InspectionError::new("source_artifact_invalid", "artifact URL is invalid"))?;
    let authority = &request.authority;
    if provider_instance(base_url) != authority.provider_instance
        || url.username() != ""
        || url.password().is_some()
        || url.scheme() != base_url.scheme()
        || url.host_str() != Some(authority.origin_host.as_str())
        || normalized_port(&url) != authority.origin_port
    {
        return Err(InspectionError::new(
            "source_artifact_mismatch",
            "artifact does not match configured provider",
        ));
    }
    let origin_base_path = base_path(&authority.origin_base_path);
    let relative_path = if origin_base_path == "/" {
        url.path().strip_prefix('/').unwrap_or(url.path())
    } else {
        url.path()
            .strip_prefix(&format!("{origin_base_path}/"))
            .ok_or_else(|| {
                InspectionError::new(
                    "source_artifact_mismatch",
                    "artifact does not match configured provider",
                )
            })?
    };
    let segments: Vec<_> = relative_path.split('/').collect();
    if segments.len() < 4 || segments[0] != authority.owner || segments[1] != authority.repository {
        return Err(InspectionError::new(
            "source_artifact_mismatch",
            "artifact does not match the verified local primary repository",
        ));
    }
    let index = |value: &str| {
        value.parse::<u64>().map_err(|_| {
            InspectionError::new("source_artifact_invalid", "artifact index is invalid")
        })
    };
    let kind = match segments[2] {
        "issues" => ArtifactKind::Issue(index(segments[3])?),
        "pulls" => ArtifactKind::Review(index(segments[3])?),
        "wiki" if segments[3..].iter().all(|segment| !segment.is_empty()) => {
            ArtifactKind::Wiki(segments[3..].join("/"))
        }
        "wiki" => {
            return Err(InspectionError::new(
                "source_artifact_invalid",
                "wiki artifact page is invalid",
            ));
        }
        _ => {
            return Err(InspectionError::new(
                "source_artifact_unsupported",
                "Tea supports issue, pull request, and single wiki-page artifacts",
            ));
        }
    };
    Ok((format!("{}/{}", segments[0], segments[1]), kind))
}

fn value_string(value: &Value, names: &[&str]) -> Option<String> {
    names.iter().find_map(|name| {
        value
            .get(*name)
            .and_then(Value::as_str)
            .map(str::to_owned)
            .or_else(|| {
                value
                    .get(*name)
                    .and_then(Value::as_u64)
                    .map(|number| number.to_string())
            })
    })
}

fn value_reference(value: &Value, name: &str) -> Option<String> {
    value_string(value, &[name]).or_else(|| {
        value
            .get(name)
            .and_then(|reference| value_string(reference, &["sha", "ref", "label"]))
    })
}

fn review_source_branch(value: &Value) -> Option<String> {
    value
        .get("head")
        .and_then(|head| value_string(head, &["ref", "label"]))
        .or_else(|| value_string(value, &["head_ref", "headRef"]))
        .filter(|branch| !branch.is_empty())
}

fn append_bounded(body: &mut String, value: &str, limit: usize) -> Result<(), InspectionError> {
    body.push_str(value);
    if body.len() > limit {
        return Err(InspectionError::new(
            "source_truncated",
            "Tea source content exceeds Cockpit's explicit byte limit",
        ));
    }
    Ok(())
}
fn parse_comments(value: &[u8]) -> Result<Vec<Comment>, InspectionError> {
    if value.trim_ascii() == b"No comments found" {
        return Ok(Vec::new());
    }
    serde_json::from_slice(value).map_err(|_| {
        InspectionError::new(
            "source_provider_contract",
            "Tea comment JSON did not match the verified contract",
        )
    })
}

#[async_trait]
impl SourceProvider for TeaSourceProvider {
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
    async fn metadata(
        &self,
        request: &SourceFetchRequest,
    ) -> Result<SourceMetadata, InspectionError> {
        let (repo, kind) = artifact_kind(request, &self.base_url)?;
        match kind {
            ArtifactKind::Issue(index) => {
                let issue: Issue = serde_json::from_slice(
                    &self
                        .command(&vec![
                            "issues".into(),
                            index.to_string(),
                            "--output".into(),
                            "json".into(),
                            "--repo".into(),
                            repo,
                            "--login".into(),
                            self.login.clone(),
                        ])
                        .await?,
                )
                .map_err(|_| {
                    InspectionError::new(
                        "source_provider_contract",
                        "Tea issue JSON did not match the verified contract",
                    )
                })?;
                if issue.index != index {
                    return Err(InspectionError::new(
                        "source_provider_contract",
                        "Tea returned a different issue index",
                    ));
                }
                Ok(SourceMetadata {
                    title: issue.title,
                    source_branch: None,
                    source_url: None,
                    source_commit: None,
                    description: None,
                })
            }
            ArtifactKind::Review(index) => {
                let review: Value = serde_json::from_slice(
                    &self
                        .command(&vec![
                            "pulls".into(),
                            index.to_string(),
                            "--output".into(),
                            "json".into(),
                            "--fields".into(),
                            "index,title,head".into(),
                            "--repo".into(),
                            repo,
                            "--login".into(),
                            self.login.clone(),
                        ])
                        .await?,
                )
                .map_err(|_| {
                    InspectionError::new(
                        "source_provider_contract",
                        "Tea pull request JSON did not match the verified contract",
                    )
                })?;
                if value_string(&review, &["index"]).and_then(|value| value.parse::<u64>().ok())
                    != Some(index)
                {
                    return Err(InspectionError::new(
                        "source_provider_contract",
                        "Tea returned a different pull request index",
                    ));
                }
                let title = value_string(&review, &["title"]).ok_or_else(|| {
                    InspectionError::new(
                        "source_provider_contract",
                        "Tea pull request has no title",
                    )
                })?;
                let source_branch = review_source_branch(&review);
                Ok(SourceMetadata {
                    title,
                    source_branch,
                    source_url: None,
                    source_commit: None,
                    description: None,
                })
            }
            ArtifactKind::Wiki(_) => Err(InspectionError::new(
                "source_metadata_unsupported",
                "Tea wiki artifacts do not provide workspace defaults",
            )),
        }
    }
    async fn fetch(
        &self,
        request: &SourceFetchRequest,
    ) -> Result<Vec<SourceAsset>, InspectionError> {
        let (repo, kind) = artifact_kind(request, &self.base_url)?;
        let ArtifactKind::Issue(index) = kind else {
            return match kind {
                ArtifactKind::Review(index) => self.fetch_review(request, &repo, index).await,
                ArtifactKind::Wiki(page) => self.fetch_wiki(request, &repo, &page).await,
                ArtifactKind::Issue(_) => unreachable!(),
            };
        };
        let issue: Issue = serde_json::from_slice(
            &self
                .command(&vec![
                    "issues".into(),
                    index.to_string(),
                    "--output".into(),
                    "json".into(),
                    "--repo".into(),
                    repo.clone(),
                    "--login".into(),
                    self.login.clone(),
                ])
                .await?,
        )
        .map_err(|_| {
            InspectionError::new(
                "source_provider_contract",
                "Tea issue JSON did not match the verified contract",
            )
        })?;
        if issue.index != index {
            return Err(InspectionError::new(
                "source_provider_contract",
                "Tea returned a different issue index",
            ));
        }
        let mut comments = Vec::new();
        for page in 1..=MAX_COMMENT_PAGES {
            let page_comments = parse_comments(
                &self
                    .command(&vec![
                        "comments".into(),
                        "list".into(),
                        index.to_string(),
                        "--output".into(),
                        "json".into(),
                        "--page".into(),
                        page.to_string(),
                        "--limit".into(),
                        COMMENTS_PER_PAGE.to_string(),
                        "--repo".into(),
                        repo.clone(),
                        "--login".into(),
                        self.login.clone(),
                    ])
                    .await?,
            )?;
            let full = page_comments.len() == COMMENTS_PER_PAGE as usize;
            comments.extend(page_comments);
            if !full {
                break;
            }
            if page == MAX_COMMENT_PAGES {
                return Err(InspectionError::new(
                    "source_truncated",
                    "Tea comment pagination reached Cockpit's explicit limit",
                ));
            }
        }
        let source = SourceRef {
            provider_id: self.provider_id.clone(),
            provider_instance: request.authority.provider_instance.clone(),
            resource_type: "issue".into(),
            canonical_id: format!("{repo}#{}", issue.index),
        };
        let mut body = issue.body.unwrap_or_default();
        for c in comments {
            body.push_str(&format!(
                "\n\n## Comment {}\n\n{}",
                c.id,
                c.body.unwrap_or_default()
            ));
            if body.len() > MAX_COMMENT_BYTES {
                return Err(InspectionError::new(
                    "source_truncated",
                    "Tea comments exceed Cockpit's explicit byte limit",
                ));
            }
        }
        Ok(vec![SourceAsset {
            source,
            title: issue.title,
            source_url: Some(request.artifact_url.clone()),
            original_url: None,
            source_revision: issue.updated,
            complete: true,
            diagnostics: Vec::new(),
            body,
        }])
    }
}

#[cfg(test)]
mod tests {
    use super::{
        Duration, SourceFetchRequest, SourceProvider, TeaSourceProvider, Url, base_path,
        parse_comments, provider_instance, review_source_branch,
    };
    use cockpit_core::sources::SourceAuthority;
    use std::io::{Read, Write};
    use std::net::TcpListener;
    use std::path::PathBuf;
    use std::sync::{
        Arc,
        atomic::{AtomicBool, AtomicUsize, Ordering},
    };
    use std::thread::{self, JoinHandle};

    static FIXTURE_ID: AtomicUsize = AtomicUsize::new(0);

    #[test]
    fn review_metadata_uses_the_head_ref_not_the_commit_sha() {
        let review = serde_json::json!({
            "head": { "sha": "7f4c", "ref": "feature/workspace-defaults" }
        });
        assert_eq!(
            review_source_branch(&review).as_deref(),
            Some("feature/workspace-defaults")
        );
    }

    #[derive(Clone, Copy)]
    enum FixtureMode {
        WrongIssueIndex,
        EmptyComments,
        SecondCommentPage,
        ExhaustCommentBudget,
        Review,
        Wiki,
    }

    struct TeaFixture {
        base_url: Url,
        config_home: PathBuf,
        seen: Arc<std::sync::Mutex<Vec<String>>>,
        done: Arc<AtomicBool>,
        server: Option<JoinHandle<()>>,
    }

    impl TeaFixture {
        fn new(mode: FixtureMode) -> Option<Self> {
            if std::process::Command::new("tea")
                .arg("--version")
                .output()
                .is_err()
            {
                return None;
            }
            let listener = TcpListener::bind("127.0.0.1:0").unwrap();
            listener.set_nonblocking(true).unwrap();
            let addr = listener.local_addr().unwrap();
            let seen = Arc::new(std::sync::Mutex::new(Vec::new()));
            let done = Arc::new(AtomicBool::new(false));
            let server_seen = seen.clone();
            let server_done = done.clone();
            let server = thread::spawn(move || {
                loop {
                    if server_done.load(Ordering::Relaxed) {
                        break;
                    }
                    let Ok((mut stream, _)) = listener.accept() else {
                        thread::sleep(Duration::from_millis(2));
                        continue;
                    };
                    let mut bytes = [0; 4096];
                    let count = stream.read(&mut bytes).unwrap();
                    let request = String::from_utf8_lossy(&bytes[..count]);
                    let path = request.lines().next().unwrap().to_owned();
                    server_seen.lock().unwrap().push(path.clone());
                    let body = fixture_body(&path, mode, &format!("http://{addr}"));
                    write!(stream, "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\n\r\n{}", body.len(), body).unwrap();
                }
            });
            let config_home = std::env::temp_dir().join(format!(
                "cockpit-tea-fixture-{}-{}",
                std::process::id(),
                FIXTURE_ID.fetch_add(1, Ordering::Relaxed),
            ));
            std::fs::create_dir_all(&config_home).unwrap();
            let mut add = std::process::Command::new("tea");
            add.env("XDG_CONFIG_HOME", &config_home).args([
                "logins",
                "add",
                "--name",
                "fixture",
                "--url",
                &format!("http://{addr}"),
                "--token",
                "fake",
                "--no-version-check",
            ]);
            assert!(add.status().unwrap().success());
            Some(Self {
                base_url: Url::parse(&format!("http://{addr}")).unwrap(),
                config_home,
                seen,
                done,
                server: Some(server),
            })
        }

        fn provider(&self) -> TeaSourceProvider {
            TeaSourceProvider {
                provider_id: "fixture-provider".into(),
                executable: "tea".into(),
                base_url: self.base_url.clone(),
                login: "fixture".into(),
                limits: (64 * 1024, Duration::from_secs(2)),
                config_home: None,
            }
            .with_config_home(self.config_home.clone())
        }

        fn request(&self, index: u64) -> SourceFetchRequest {
            source_request(
                "fixture-provider",
                &self.base_url,
                self.base_url
                    .join(&format!("acme/repo/issues/{index}"))
                    .unwrap()
                    .to_string(),
            )
        }

        fn artifact_request(&self, path: &str) -> SourceFetchRequest {
            source_request(
                "fixture-provider",
                &self.base_url,
                format!(
                    "{}/acme/repo/{path}",
                    self.base_url.as_str().trim_end_matches('/')
                ),
            )
        }
    }

    impl Drop for TeaFixture {
        fn drop(&mut self) {
            self.done.store(true, Ordering::Relaxed);
            if let Some(server) = self.server.take() {
                server.join().unwrap();
            }
            std::fs::remove_dir_all(&self.config_home).unwrap();
        }
    }

    fn source_request(
        provider_id: &str,
        base_url: &Url,
        artifact_url: String,
    ) -> SourceFetchRequest {
        SourceFetchRequest {
            provider_id: provider_id.into(),
            artifact_url,
            authority: SourceAuthority {
                provider_instance: provider_instance(base_url),
                origin_host: base_url.host_str().unwrap().into(),
                origin_port: base_url.port(),
                origin_base_path: base_path(base_url.path()),
                owner: "acme".into(),
                repository: "repo".into(),
            },
        }
    }

    #[test]
    fn provider_instance_matches_the_shared_root_and_base_path_forms() {
        assert_eq!(
            provider_instance(&Url::parse("https://Forge.Test/").unwrap()),
            "https://forge.test"
        );
        assert_eq!(
            provider_instance(&Url::parse("https://forge.test/gitea/").unwrap()),
            "https://forge.test/gitea"
        );
    }

    fn fixture_body(path: &str, mode: FixtureMode, base_url: &str) -> String {
        if path.contains("/user/keys") || path.contains("reactions") {
            return "[]".into();
        }
        if path.contains("wiki") && path.contains("revision") {
            return r#"{"commits":[{"sha":"wiki-sha","date":"2026-01-01T00:00:00Z","message":"wiki revision","author":{"login":"fixture"}}]}"#.into();
        }
        if path.contains("pulls/1.diff") {
            return "diff --git a/src/lib.rs b/src/lib.rs\nindex 0000000..1111111 100644\n--- a/src/lib.rs\n+++ b/src/lib.rs\n".into();
        }
        if path.contains("comments") {
            let page = path
                .split("page=")
                .nth(1)
                .and_then(|value| value.split('&').next())
                .and_then(|value| value.split_whitespace().next())
                .and_then(|value| value.parse::<u32>().ok())
                .unwrap_or(1);
            return match mode {
                FixtureMode::WrongIssueIndex => comments_json(1, "comment"),
                FixtureMode::EmptyComments => "[]".into(),
                FixtureMode::SecondCommentPage if page == 1 => {
                    comments_json(100, "page-one-comment")
                }
                FixtureMode::SecondCommentPage => comments_json(1, "page-two-comment"),
                FixtureMode::ExhaustCommentBudget => comments_json(100, "bounded-comment"),
                FixtureMode::Review | FixtureMode::Wiki => comments_json(1, "review comment"),
            };
        }
        if path.contains("/pulls/1/reviews/7/comments") {
            return r#"[{"id":9,"body":"review comment","path":"src/lib.rs","line":7,"created_at":"2026-01-01T00:00:00Z","updated_at":"2026-01-01T00:00:00Z","html_url":"http://x/comments/9","user":{"id":1,"login":"fixture"}}]"#.into();
        }
        if path.contains("/pulls/1/reviews") {
            return r#"[{"id":7,"body":"review","state":"COMMENT","submitted_at":"2026-01-01T00:00:00Z","user":{"id":1,"login":"fixture"}}]"#.into();
        }
        if path.contains("/pulls/1") {
            return format!(
                r#"{{"id":1,"number":1,"title":"review","body":"review body","html_url":"{base_url}/pulls/1","diff_url":"{base_url}/api/v1/repos/acme/repo/pulls/1.diff","updated_at":"2026-01-01T00:00:00Z","base":{{"ref":"main","sha":"base-sha"}},"head":{{"ref":"feature","sha":"head-sha"}},"user":{{"id":1,"login":"fixture"}}}}"#
            );
        }
        if path.contains("/wiki/") {
            return r#"{"title":"Guide","content_base64":"IyBndWlkZQo=","sha":"wiki-sha","html_url":"http://x/wiki/Guide","last_commit":{"id":"wiki-sha"}}"#.into();
        }
        if path.contains("issues/1") {
            let number = match mode {
                FixtureMode::WrongIssueIndex => 2,
                _ => 1,
            };
            return format!(
                r#"{{"id":1,"number":{number},"title":"issue","body":"body","html_url":"http://x/issues/{number}","updated_at":"2026-01-01T00:00:00Z","user":{{"id":1,"login":"fixture"}}}}"#
            );
        }
        r#"{"id":1,"login":"fixture","full_name":"Fixture","email":"x","avatar_url":"","language":"en-US","is_admin":false,"active":true,"restricted":false}"#.into()
    }

    fn comments_json(count: usize, body: &str) -> String {
        let comments = (1..=count)
            .map(|id| format!(r#"{{"id":{id},"body":"{body}","updated_at":"2026-01-01T00:00:00Z","user":{{"id":1,"login":"fixture"}}}}"#))
            .collect::<Vec<_>>();
        format!("[{}]", comments.join(","))
    }
    #[test]
    fn localhost_tea_contract_is_read_only() {
        if std::process::Command::new("tea")
            .arg("--version")
            .output()
            .is_err()
        {
            return;
        }
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        let seen = Arc::new(std::sync::Mutex::new(Vec::new()));
        let done = Arc::new(AtomicBool::new(false));
        let s = seen.clone();
        let d = done.clone();
        thread::spawn(move || {
            for stream in listener.incoming() {
                if d.load(Ordering::Relaxed) {
                    break;
                }
                let mut stream = stream.unwrap();
                let mut b = [0; 4096];
                let n = stream.read(&mut b).unwrap();
                let line = String::from_utf8_lossy(&b[..n]);
                let path = line.lines().next().unwrap().to_string();
                s.lock().unwrap().push(path.clone());
                let body = if path.contains("/user/keys") || path.contains("reactions") {
                    "[]"
                } else if path.contains("comments") {
                    r#"[{"id":2,"body":"comment","updated_at":"2026-01-01T00:00:00Z","user":{"id":1,"login":"fixture"}}]"#
                } else if path.contains("issues/1") {
                    r#"{"id":1,"number":1,"title":"issue","body":"body","html_url":"http://x/issues/1","updated_at":"2026-01-01T00:00:00Z","user":{"id":1,"login":"fixture"}}"#
                } else {
                    r#"{"id":1,"login":"fixture","full_name":"Fixture","email":"x","avatar_url":"","language":"en-US","is_admin":false,"active":true,"restricted":false}"#
                };
                write!(stream,"HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\n\r\n{}",body.len(),body).unwrap();
            }
        });
        let dir = std::env::temp_dir().join(format!("tea-test-{}", std::process::id()));
        let _ = std::fs::create_dir_all(&dir);
        let mut add = std::process::Command::new("tea");
        add.env("XDG_CONFIG_HOME", &dir).args([
            "logins",
            "add",
            "--name",
            "fixture",
            "--url",
            &format!("http://{addr}"),
            "--token",
            "fake",
            "--no-version-check",
        ]);
        assert!(add.status().unwrap().success());
        for args in [
            [
                "issues",
                "1",
                "--output",
                "json",
                "--repo",
                "acme/repo",
                "--login",
                "fixture",
            ]
            .as_slice(),
            [
                "comments",
                "list",
                "1",
                "--output",
                "json",
                "--page",
                "1",
                "--limit",
                "100",
                "--repo",
                "acme/repo",
                "--login",
                "fixture",
            ]
            .as_slice(),
        ] {
            let out = std::process::Command::new("tea")
                .env("XDG_CONFIG_HOME", &dir)
                .args(args)
                .output()
                .unwrap();
            assert!(out.status.success());
        }
        let base_url = Url::parse(&format!("http://{addr}")).unwrap();
        let provider = TeaSourceProvider {
            provider_id: "fixture-provider".into(),
            executable: "tea".into(),
            base_url: base_url.clone(),
            login: "fixture".into(),
            limits: (64 * 1024, Duration::from_secs(2)),
            config_home: None,
        }
        .with_config_home(dir.clone());
        let assets = tokio::runtime::Runtime::new()
            .unwrap()
            .block_on(provider.fetch(&source_request(
                "fixture-provider",
                &base_url,
                format!("http://{addr}/acme/repo/issues/1"),
            )))
            .unwrap();
        assert_eq!(assets.len(), 1);
        assert_eq!(assets[0].title, "issue");
        assert_eq!(assets[0].source.provider_id, "fixture-provider");
        assert_eq!(assets[0].source.canonical_id, "acme/repo#1");
        assert!(assets[0].body.contains("Comment 2"));
        assert!(assets[0].body.contains("comment"));
        done.store(true, Ordering::Relaxed);
        let seen = seen.lock().unwrap();
        assert!(seen.iter().all(|line| line.starts_with("GET ")));
        assert!(seen.iter().any(|line| line.contains("issues/1")));
        assert!(seen.iter().any(|line| line.contains("comments")));
    }
    #[test]
    fn localhost_tea_treats_no_comments_as_empty() {
        let Some(fixture) = TeaFixture::new(FixtureMode::EmptyComments) else {
            return;
        };
        let assets = tokio::runtime::Runtime::new()
            .unwrap()
            .block_on(fixture.provider().fetch(&fixture.request(1)))
            .unwrap();
        assert_eq!(assets.len(), 1);
        assert_eq!(assets[0].body, "body");
    }
    #[test]
    fn tea_no_comments_message_is_treated_as_empty() {
        assert!(parse_comments(b"No comments found\n").unwrap().is_empty());
    }

    #[test]
    fn artifact_authority_rejects_userinfo_port_and_prefix_mismatches_before_process() {
        let provider = TeaSourceProvider {
            provider_id: "fixture".into(),
            executable: "definitely-not-a-command".into(),
            base_url: Url::parse("http://example.test:8080/gitea/").unwrap(),
            login: "fixture".into(),
            limits: (1024, Duration::from_secs(1)),
            config_home: None,
        };
        let runtime = tokio::runtime::Runtime::new().unwrap();
        for artifact in [
            "http://user@example.test:8080/gitea/acme/repo/issues/1",
            "http://example.test:8081/gitea/acme/repo/issues/1",
            "http://example.test:8080/other/acme/repo/issues/1",
        ] {
            let error = runtime
                .block_on(provider.fetch(&source_request(
                    "fixture",
                    &provider.base_url,
                    artifact.into(),
                )))
                .expect_err("authority mismatch");
            assert_eq!(error.code, "source_artifact_mismatch");
        }
    }

    #[test]
    fn localhost_tea_rejects_a_different_returned_issue_index() {
        let Some(fixture) = TeaFixture::new(FixtureMode::WrongIssueIndex) else {
            return;
        };
        let error = tokio::runtime::Runtime::new()
            .unwrap()
            .block_on(fixture.provider().fetch(&fixture.request(1)))
            .expect_err("Tea must not substitute a different issue");
        assert_eq!(error.code, "source_provider_contract");
        assert!(
            fixture
                .seen
                .lock()
                .unwrap()
                .iter()
                .any(|request| request.contains("issues/1"))
        );
    }

    #[test]
    fn localhost_tea_includes_comments_from_the_second_page() {
        let Some(fixture) = TeaFixture::new(FixtureMode::SecondCommentPage) else {
            return;
        };
        let result = tokio::runtime::Runtime::new()
            .unwrap()
            .block_on(fixture.provider().fetch(&fixture.request(1)));
        let assets = result.expect("Tea must return both comment pages");
        assert!(assets[0].body.contains("page-one-comment"));
        assert!(assets[0].body.contains("page-two-comment"));
        let seen = fixture.seen.lock().unwrap();
        assert!(
            seen.iter()
                .any(|request| request.contains("comments") && request.contains("page=2"))
        );
    }

    #[test]
    fn localhost_tea_refuses_to_silently_truncate_after_the_comment_page_budget() {
        let Some(fixture) = TeaFixture::new(FixtureMode::ExhaustCommentBudget) else {
            return;
        };
        let error = tokio::runtime::Runtime::new()
            .unwrap()
            .block_on(fixture.provider().fetch(&fixture.request(1)))
            .expect_err("Tea pagination must refuse a truncated comment set");
        assert_eq!(error.code, "source_truncated");
        let seen = fixture.seen.lock().unwrap();
        assert!(
            seen.iter()
                .any(|request| request.contains("comments") && request.contains("page=5"))
        );
        assert!(
            !seen
                .iter()
                .any(|request| request.contains("comments") && request.contains("page=6"))
        );
    }

    #[test]
    fn localhost_tea_imports_pull_request_metadata_and_review_comments_read_only() {
        let Some(fixture) = TeaFixture::new(FixtureMode::Review) else {
            return;
        };
        let result = tokio::runtime::Runtime::new().unwrap().block_on(
            fixture
                .provider()
                .fetch(&fixture.artifact_request("pulls/1")),
        );
        assert!(
            result.is_ok(),
            "requests: {:?}",
            fixture.seen.lock().unwrap()
        );
        let assets = result.unwrap();
        assert_eq!(assets[0].source.resource_type, "review");
        assert_eq!(assets[0].source.canonical_id, "acme/repo!1");
        assert_eq!(assets[0].source_revision.as_deref(), Some("head-sha"));
        assert!(assets[0].body.contains("Base: main"));
        assert!(assets[0].body.contains("review comment"));
        assert!(
            assets[0]
                .body
                .contains("Diff and structured changed-file metadata: unavailable")
        );
        assert!(
            assets[0]
                .body
                .contains("Review summaries and reply threads: unavailable")
        );
        assert!(
            fixture
                .seen
                .lock()
                .unwrap()
                .iter()
                .any(|request| request.contains("pulls/1/reviews/7/comments"))
        );
        assert!(assets[0].body.contains("unverified reference metadata"));
        assert!(
            fixture
                .seen
                .lock()
                .unwrap()
                .iter()
                .all(|request| request.starts_with("GET "))
        );
    }

    #[test]
    fn localhost_tea_imports_base64_wiki_content_read_only() {
        let Some(fixture) = TeaFixture::new(FixtureMode::Wiki) else {
            return;
        };
        let result = tokio::runtime::Runtime::new().unwrap().block_on(
            fixture
                .provider()
                .fetch(&fixture.artifact_request("wiki/Guide")),
        );
        assert!(
            result.is_ok(),
            "Tea wiki fixture must match the JSON contract: {result:?}; requests: {:?}",
            fixture.seen.lock().unwrap()
        );
        let assets = result.unwrap();
        assert_eq!(assets[0].source.resource_type, "wiki");
        assert_eq!(assets[0].source.canonical_id, "acme/repo:Guide");
        assert_eq!(assets[0].source_revision.as_deref(), Some("wiki-sh"));
        assert_eq!(assets[0].body, "# guide\n");
        assert!(
            fixture
                .seen
                .lock()
                .unwrap()
                .iter()
                .any(|request| request.contains("wiki") && request.contains("revision"))
        );
        assert!(
            fixture
                .seen
                .lock()
                .unwrap()
                .iter()
                .all(|request| request.starts_with("GET "))
        );
    }
}
