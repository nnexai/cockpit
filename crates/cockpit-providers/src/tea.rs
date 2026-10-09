use std::time::Duration;

use crate::forge::{self, ByteBudget, COMMENTS_PER_PAGE, CliRunner, Forge, MAX_COMMENT_PAGES};
use async_trait::async_trait;
use cockpit_core::InspectionError;
use cockpit_core::sources::{
    FrontmatterField, FrontmatterValue, SourceAsset, SourceContainer, SourceFetchRequest,
    SourceMetadata, SourceProvider, SourceRef,
};
use cockpit_protocol::projects::ProjectConfiguration;
use cockpit_protocol::sources::SourceCapability;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use url::Url;
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
            executable: provider.executable.clone().ok_or_else(|| {
                InspectionError::new(
                    "source_provider_invalid",
                    "Tea provider requires a configured executable",
                )
            })?,
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
        let prompt = ("GIT_TERMINAL_PROMPT", std::ffi::OsStr::new("0"));
        let env = [
            prompt,
            (
                "XDG_CONFIG_HOME",
                self.config_home
                    .as_deref()
                    .map(|home| home.as_os_str())
                    .unwrap_or_default(),
            ),
        ];
        let env = if self.config_home.is_some() {
            &env[..]
        } else {
            &env[..1]
        };
        CliRunner {
            executable: &self.executable,
            limits: self.limits,
            label: "tea source",
            failure: classify_cli_failure,
            execution_error: forge::unchanged_execution_error,
            empty_response: None,
        }
        .run(args, env)
        .await
    }

    async fn fetch_review(
        &self,
        request: &SourceFetchRequest,
        repo: &str,
        index: u64,
    ) -> Result<(Value, Option<String>), InspectionError> {
        let review: Value = serde_json::from_slice(
            &self
                .command(&vec![
                    "pulls".into(),
                    index.to_string(),
                    "--output".into(),
                    "json".into(),
                    "--fields".into(),
                    "index,title,body,url,base,base-commit,head,updated,state,priority,assignee,assignees,user,created,draft,merged".into(),
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
        let source_url = optional_verified_url(
            request,
            &self.base_url,
            value_string(&review, &["html_url", "url"]).as_deref(),
        )?;
        Ok((review, source_url))
    }

    async fn fetch_issue(
        &self,
        request: &SourceFetchRequest,
        repo: &str,
        index: u64,
    ) -> Result<(Issue, Option<String>), InspectionError> {
        let issue: Issue = serde_json::from_slice(
            &self
                .command(&vec![
                    "issues".into(),
                    index.to_string(),
                    "--output".into(),
                    "json".into(),
                    "--fields".into(),
                    "index,title,body,url,updated,state,priority,assignee,assignees,user,created,comments".into(),
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
                "Tea issue JSON did not match the verified contract",
            )
        })?;
        if issue.index != index {
            return Err(InspectionError::new(
                "source_provider_contract",
                "Tea returned a different issue index",
            ));
        }
        let source_url = optional_verified_url(request, &self.base_url, issue.html_url.as_deref())?;
        Ok((issue, source_url))
    }

    async fn fetch_issue_comments(
        &self,
        repo: &str,
        index: u64,
    ) -> Result<Vec<Comment>, InspectionError> {
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
                        repo.into(),
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
        Ok(comments)
    }

    async fn fetch_review_comments(
        &self,
        repo: &str,
        index: u64,
    ) -> Result<(Vec<Value>, Vec<Value>), InspectionError> {
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
        let review_comments = self.fetch_review_positions(repo, index).await?;
        comments.extend(review_comments.iter().cloned());
        comments.sort_by(|left, right| {
            comment_timestamp(left)
                .cmp(&comment_timestamp(right))
                .then_with(|| value_string(left, &["id"]).cmp(&value_string(right, &["id"])))
        });
        Ok((comments, review_comments))
    }

    async fn fetch_review_positions(
        &self,
        repo: &str,
        index: u64,
    ) -> Result<Vec<Value>, InspectionError> {
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
        Ok(review_comments)
    }

    fn assemble_review(
        &self,
        request: &SourceFetchRequest,
        repo: String,
        index: u64,
        review: Value,
        source_url: Option<String>,
        comments: Vec<Value>,
        review_comments: Vec<Value>,
    ) -> Result<SourceAsset, InspectionError> {
        let TeaReviewRendering {
            title,
            body,
            status,
            author,
            assignee,
            created,
            updated,
            comment_count,
        } = render_review(&review, comments, &review_comments)?;
        let item_type = "Pull request";
        Ok(SourceAsset {
            source: SourceRef {
                provider_id: self.provider_id.clone(),
                provider_instance: request.authority.provider_instance.clone(),
                resource_type: "review".into(),
                canonical_id: format!("{repo}!{index}"),
            },
            title,
            source_url,
            original_url: None,
            source_revision: value_string(
                &review,
                &["headSha", "head_commit", "head-commit", "updated"],
            ),
            complete: true,
            diagnostics: Vec::new(),
            body,
            container: Some(SourceContainer {
                id: repo.clone(),
                label: repo,
            }),
            fields: issue_fields(
                item_type,
                Some(status),
                value_string(&review, &["priority"]),
                assignee,
                author,
                created,
                updated,
                comment_count,
            ),
            attachments: Vec::new(),
        })
    }

    fn assemble_issue(
        &self,
        request: &SourceFetchRequest,
        repo: String,
        issue: Issue,
        source_url: Option<String>,
        comments: Vec<Comment>,
    ) -> Result<SourceAsset, InspectionError> {
        let source = SourceRef {
            provider_id: self.provider_id.clone(),
            provider_instance: request.authority.provider_instance.clone(),
            resource_type: "issue".into(),
            canonical_id: format!("{repo}#{}", issue.index),
        };
        let TeaIssueRendering {
            body,
            status,
            author,
            assignee,
            created,
            updated,
            comment_count,
        } = render_issue(&issue, &comments)?;
        let item_type = "Issue";
        Ok(SourceAsset {
            source,
            title: issue.title,
            source_url,
            original_url: None,
            source_revision: issue.updated,
            complete: true,
            diagnostics: Vec::new(),
            body,
            container: Some(SourceContainer {
                id: repo.clone(),
                label: repo,
            }),
            fields: issue_fields(
                item_type,
                Some(status.unwrap_or_else(|| "open".into())),
                issue.priority,
                assignee,
                Some(author.unwrap_or_else(|| "Unknown".into())),
                created,
                updated,
                comment_count,
            ),
            attachments: Vec::new(),
        })
    }

    async fn fetch_wiki(
        &self,
        request: &SourceFetchRequest,
        repo: &str,
        page: &str,
    ) -> Result<TeaItem, InspectionError> {
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
        let source_url = verified_url(
            request,
            &self.base_url,
            value_string(&wiki, &["html_url", "url"]).as_deref(),
        )?;
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
        Ok(TeaItem::Wiki {
            wiki,
            source_url,
            title,
            body,
        })
    }

    async fn fetch_wiki_revision(
        &self,
        wiki: &Value,
        repo: &str,
        page: &str,
    ) -> Result<String, InspectionError> {
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
        Ok(source_revision)
    }

    fn assemble_wiki(
        &self,
        request: &SourceFetchRequest,
        repo: String,
        page: String,
        source_url: String,
        title: String,
        body: String,
        source_revision: String,
    ) -> SourceAsset {
        SourceAsset {
            source: SourceRef {
                provider_id: self.provider_id.clone(),
                provider_instance: request.authority.provider_instance.clone(),
                resource_type: "wiki".into(),
                canonical_id: format!("{repo}:{page}"),
            },
            title,
            source_url: Some(source_url),
            original_url: None,
            source_revision: Some(source_revision),
            complete: true,
            diagnostics: Vec::new(),
            body,
            container: Some(SourceContainer {
                id: repo.clone(),
                label: repo,
            }),
            fields: Vec::new(),
            attachments: Vec::new(),
        }
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
pub(crate) struct Issue {
    index: u64,
    title: String,
    body: Option<String>,
    updated: Option<String>,
    #[serde(default, alias = "url")]
    html_url: Option<String>,
    #[serde(default)]
    state: Option<String>,
    #[serde(default)]
    priority: Option<String>,
    #[serde(default)]
    assignee: Option<Value>,
    #[serde(default)]
    assignees: Option<Value>,
    #[serde(default)]
    user: Option<Value>,
    #[serde(default)]
    created: Option<String>,
    #[serde(default)]
    created_at: Option<String>,
    #[serde(default)]
    comments: Option<Value>,
}
#[derive(Deserialize, Serialize)]
pub(crate) struct Comment {
    #[serde(deserialize_with = "string_or_number")]
    id: String,
    body: Option<String>,
    #[serde(default)]
    created: Option<String>,
    #[serde(default)]
    created_at: Option<String>,
    #[serde(default)]
    updated: Option<String>,
    #[serde(default)]
    updated_at: Option<String>,
    #[serde(default)]
    user: Option<Value>,
    #[serde(default)]
    author: Option<Value>,
    #[serde(default)]
    url: Option<String>,
    #[serde(default)]
    html_url: Option<String>,
    #[serde(default)]
    path: Option<String>,
    #[serde(default)]
    line: Option<Value>,
}

fn demote_headings(markdown: &str, minimum: usize) -> String {
    let mut result = String::with_capacity(markdown.len());
    let mut fence: Option<(u8, usize)> = None;
    for line in markdown.split_inclusive('\n') {
        let line_without_newline = line.strip_suffix('\n').unwrap_or(line);
        let trimmed = line_without_newline.trim_start();
        let indent = line_without_newline.len() - trimmed.len();
        let fence_marker = trimmed
            .as_bytes()
            .first()
            .copied()
            .filter(|byte| *byte == b'`' || *byte == b'~');
        let fence_len = fence_marker
            .map(|marker| trimmed.bytes().take_while(|byte| *byte == marker).count())
            .unwrap_or(0);
        if let Some((marker, length)) = fence {
            if fence_marker == Some(marker) && fence_len >= length {
                fence = None;
            }
        } else if let Some(marker) = fence_marker.filter(|_| fence_len >= 3) {
            fence = Some((marker, fence_len));
        } else if indent <= 3 && trimmed.starts_with('#') {
            let hashes = trimmed.bytes().take_while(|byte| *byte == b'#').count();
            if (1..=6).contains(&hashes)
                && trimmed
                    .as_bytes()
                    .get(hashes)
                    .is_some_and(|byte| byte.is_ascii_whitespace())
                && hashes < minimum
            {
                result.push_str(&line_without_newline[..indent]);
                result.push_str(&"#".repeat(minimum));
                result.push_str(&line_without_newline[indent + hashes..]);
                if line.ends_with('\n') {
                    result.push('\n');
                }
                continue;
            }
        }
        result.push_str(line);
    }
    result
}

fn person_name(value: Option<&Value>) -> Option<String> {
    value.and_then(|value| {
        value
            .as_str()
            .map(str::to_owned)
            .or_else(|| {
                value.as_array().map(|values| {
                    values
                        .iter()
                        .filter_map(|value| person_name(Some(value)))
                        .collect::<Vec<_>>()
                        .join(", ")
                })
            })
            .or_else(|| value_string(value, &["full_name", "name", "login", "username"]))
            .filter(|value| !value.is_empty())
    })
}

fn text_field(key: &str, value: impl Into<String>) -> FrontmatterField {
    FrontmatterField {
        key: key.into(),
        value: FrontmatterValue::String(value.into()),
    }
}

fn issue_fields(
    item_type: &str,
    status: Option<String>,
    priority: Option<String>,
    assignee: Option<String>,
    author: Option<String>,
    created: Option<String>,
    updated: Option<String>,
    comment_count: usize,
) -> Vec<FrontmatterField> {
    let mut fields = vec![text_field("item_type", item_type)];
    for (key, value) in [
        ("status", status.map(|value| status_value(&value))),
        ("priority", priority),
        ("author", author),
        (
            "created",
            created.map(|value| frontmatter_timestamp(&value)),
        ),
        (
            "updated",
            updated.map(|value| frontmatter_timestamp(&value)),
        ),
    ] {
        if let Some(value) = value {
            fields.push(text_field(key, value));
        }
    }
    fields.push(FrontmatterField {
        key: "assignee".into(),
        value: assignee.map_or(FrontmatterValue::Null, FrontmatterValue::String),
    });
    fields.push(FrontmatterField {
        key: "comment_count".into(),
        value: FrontmatterValue::Number(comment_count as i64),
    });
    fields
}

fn status_value(value: &str) -> String {
    match value.to_ascii_lowercase().as_str() {
        "open" | "opened" => "open".into(),
        "closed" => "closed".into(),
        "merged" => "merged".into(),
        "draft" => "draft".into(),
        other => other.into(),
    }
}

fn capitalize_status(value: &str) -> String {
    let mut characters = value.chars();
    characters
        .next()
        .map(|first| first.to_ascii_uppercase().to_string() + characters.as_str())
        .unwrap_or_default()
}

fn frontmatter_timestamp(value: &str) -> String {
    let mut value = value.to_owned();
    if let Some(dot) = value.find('.') {
        let suffix = value[dot..]
            .find(|character| matches!(character, '+' | '-' | 'Z'))
            .map(|offset| dot + offset)
            .unwrap_or(value.len());
        value.replace_range(dot..suffix, "");
    }
    let zone = value
        .get(19..)
        .and_then(|suffix| suffix.find(['+', '-']).map(|offset| offset + 19))
        .unwrap_or(value.len());
    if value[zone..].len() == 5 && value[zone..].as_bytes()[1..].iter().all(u8::is_ascii_digit) {
        value.insert(zone + 3, ':');
    }
    value
}

fn comment_timestamp(comment: &Value) -> Option<String> {
    value_string(comment, &["created", "created_at", "submitted_at"])
}

fn append_comment_card(
    body: &mut String,
    comment: &Value,
    review: bool,
) -> Result<(), InspectionError> {
    let id = value_string(comment, &["id"])
        .ok_or_else(|| InspectionError::new("source_provider_contract", "Tea comment has no ID"))?;
    let author = comment
        .get("poster")
        .or_else(|| comment.get("user"))
        .or_else(|| comment.get("author"))
        .or_else(|| comment.get("reviewer"))
        .and_then(|value| person_name(Some(value)))
        .unwrap_or_else(|| "Unknown".into());
    let created_raw = comment_timestamp(comment).unwrap_or_default();
    let created = card_timestamp(&created_raw);
    let updated = value_string(comment, &["updated", "updated_at"]);
    let edited = updated.filter(|updated| {
        !updated.is_empty() && !created_raw.is_empty() && updated.as_str() != created_raw.as_str()
    });
    let path = value_string(comment, &["path"]);
    let line = value_string(comment, &["line"]).filter(|line| !line.is_empty());
    let review_path = if review {
        path.map(|path| {
            format!(
                " · review on {path}{}",
                line.map(|line| format!(":{line}")).unwrap_or_default()
            )
        })
    } else {
        None
    };
    let mut heading = format!("### {author} · {created}");
    if let Some(edited) = edited {
        heading.push_str(&format!(" · edited {}", card_timestamp(&edited)));
    }
    if let Some(path) = review_path {
        heading.push_str(&path);
    }
    append_bounded(body, &format!("{heading}\n\n"), MAX_COMMENT_BYTES)?;
    let permalink = value_string(comment, &["html_url", "url"]);
    if let Some(url) = permalink {
        append_bounded(body, &format!("[#{id}]({url})\n\n"), MAX_COMMENT_BYTES)?;
    } else {
        append_bounded(body, &format!("#{id}\n\n"), MAX_COMMENT_BYTES)?;
    }
    if let Some(text) = value_string(comment, &["body"]) {
        append_bounded(body, &demote_headings(&text, 4), MAX_COMMENT_BYTES)?;
    }
    Ok(())
}

#[derive(Debug, PartialEq, Eq)]
pub(crate) enum ArtifactKind {
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
        || authority.origin_host != base_url.host_str().unwrap_or_default()
        || authority.origin_port != normalized_port(base_url)
        || base_path(&authority.origin_base_path) != base_path(base_url.path())
        || url.username() != ""
        || url.password().is_some()
        || url.scheme() != base_url.scheme()
        || url.host_str() != Some(authority.origin_host.as_str())
        || normalized_port(&url) != authority.origin_port
        || url.query().is_some()
        || url.fragment().is_some()
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
        "issues" if segments.len() == 4 => ArtifactKind::Issue(index(segments[3])?),
        "pulls" if segments.len() == 4 => ArtifactKind::Review(index(segments[3])?),
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

/// Tea's `url` JSON column is the API record's `html_url`.
fn verified_url(
    request: &SourceFetchRequest,
    base: &Url,
    value: Option<&str>,
) -> Result<String, InspectionError> {
    let mismatch = || {
        InspectionError::new(
            "source_identity_mismatch",
            "Tea returned a different canonical artifact URL",
        )
    };
    let value = value.ok_or_else(mismatch)?;
    let returned = SourceFetchRequest {
        provider_id: request.provider_id.clone(),
        artifact_url: value.into(),
        authority: request.authority.clone(),
    };
    let actual = artifact_kind(&returned, base).map_err(|_| mismatch())?;
    if actual != artifact_kind(request, base)? {
        return Err(mismatch());
    }
    Ok(value.into())
}
fn card_timestamp(value: &str) -> String {
    let value = value.replace('T', " ");
    if value.len() >= 16 {
        value[..16].to_owned()
    } else {
        value
    }
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

fn string_or_number<'de, D>(deserializer: D) -> Result<String, D::Error>
where
    D: serde::Deserializer<'de>,
{
    let value = Value::deserialize(deserializer)?;
    value
        .as_str()
        .map(str::to_owned)
        .or_else(|| value.as_u64().map(|number| number.to_string()))
        .ok_or_else(|| serde::de::Error::custom("expected string or integer"))
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

fn optional_verified_url(
    request: &SourceFetchRequest,
    base: &Url,
    value: Option<&str>,
) -> Result<Option<String>, InspectionError> {
    value
        .map(|value| verified_url(request, base, Some(value)))
        .transpose()
}

fn append_bounded(body: &mut String, value: &str, limit: usize) -> Result<(), InspectionError> {
    if !ByteBudget::for_output(limit, body).append_checked(body, value) {
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

struct TeaIssueRendering {
    body: String,
    status: Option<String>,
    author: Option<String>,
    assignee: Option<String>,
    created: Option<String>,
    updated: Option<String>,
    comment_count: usize,
}

fn render_issue(issue: &Issue, comments: &[Comment]) -> Result<TeaIssueRendering, InspectionError> {
    let status = issue.state.clone().map(|state| status_value(&state));
    let author = person_name(issue.user.as_ref());
    let created = issue.created.clone().or(issue.created_at.clone());
    let updated = issue.updated.clone();
    let assignee = issue
        .assignees
        .as_ref()
        .or(issue.assignee.as_ref())
        .and_then(|value| person_name(Some(value)));
    let comment_count = issue
        .comments
        .as_ref()
        .and_then(Value::as_u64)
        .or_else(|| {
            issue
                .comments
                .as_ref()
                .and_then(Value::as_array)
                .map(|values| values.len() as u64)
        })
        .unwrap_or(comments.len() as u64)
        .max(comments.len() as u64) as usize;
    let item_type = "Issue";
    let status_text = status.as_deref().unwrap_or("open");
    let author_text = author.as_deref().unwrap_or("Unknown");
    let assignee_text = assignee.as_deref().unwrap_or("Unassigned");
    let mut summary = vec![
        format!("**{item_type}**"),
        format!("**{}**", capitalize_status(status_text)),
    ];
    if let Some(priority) = &issue.priority {
        summary.push(format!("Priority {priority}"));
    }
    summary.push(format!("Author {author_text}"));
    summary.push(format!("Assignee {assignee_text}"));
    let mut body = format!("{}\n", summary.join(" · "));
    let description = demote_headings(issue.body.as_deref().unwrap_or_default(), 3);
    if !description.trim().is_empty() {
        append_bounded(&mut body, "\n## Description\n\n", MAX_COMMENT_BYTES)?;
        append_bounded(&mut body, &description, MAX_COMMENT_BYTES)?;
    }
    if !comments.is_empty() {
        let mut serialized = comments
            .iter()
            .map(|comment| serde_json::to_value(comment).unwrap_or(Value::Null))
            .collect::<Vec<_>>();
        serialized.sort_by(|left, right| {
            comment_timestamp(left)
                .cmp(&comment_timestamp(right))
                .then_with(|| value_string(left, &["id"]).cmp(&value_string(right, &["id"])))
        });
        append_bounded(
            &mut body,
            &format!(
                "\n\n## Comments ({}{})\n\n",
                comments.len(),
                if comment_count > comments.len() {
                    format!(" of {comment_count}")
                } else {
                    String::new()
                }
            ),
            MAX_COMMENT_BYTES,
        )?;
        for comment in serialized {
            append_comment_card(&mut body, &comment, false)?;
        }
    }
    Ok(TeaIssueRendering {
        body,
        status,
        author,
        assignee,
        created,
        updated,
        comment_count,
    })
}

fn append_review_metadata(body: &mut String, review: &Value) -> Result<(), InspectionError> {
    append_bounded(
        body,
        "\n\n### Review metadata\n\nProvider positions below are unverified reference metadata. Cockpit does not use them as local anchors.\n",
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
            append_bounded(body, &format!("- {label}: {value}\n"), MAX_COMMENT_BYTES)?;
        }
    }
    append_bounded(
        body,
        "- Diff and structured changed-file metadata: unavailable in Tea 0.15.1's verified read-only JSON path.\n- Review summaries and reply threads: unavailable in Tea 0.15.1's documented read-only JSON commands.\n",
        MAX_COMMENT_BYTES,
    )?;
    Ok(())
}

fn append_review_comments(
    body: &mut String,
    comments: Vec<Value>,
    review_comments: &[Value],
) -> Result<(), InspectionError> {
    let comment_count = comments.len();
    if !comments.is_empty() {
        append_bounded(
            body,
            &format!("\n## Comments ({comment_count})\n\n"),
            MAX_COMMENT_BYTES,
        )?;
        for comment in comments {
            let is_review = review_comments.iter().any(|review_comment| {
                value_string(review_comment, &["id"]) == value_string(&comment, &["id"])
                    && value_string(review_comment, &["path"]).is_some()
            });
            append_comment_card(body, &comment, is_review)?;
        }
    }
    Ok(())
}

struct TeaReviewRendering {
    title: String,
    body: String,
    status: String,
    author: Option<String>,
    assignee: Option<String>,
    created: Option<String>,
    updated: Option<String>,
    comment_count: usize,
}

fn render_review(
    review: &Value,
    comments: Vec<Value>,
    review_comments: &[Value],
) -> Result<TeaReviewRendering, InspectionError> {
    let title = value_string(&review, &["title"]).ok_or_else(|| {
        InspectionError::new("source_provider_contract", "Tea pull request has no title")
    })?;
    let item_type = "Pull request";
    let status = if review.get("draft").and_then(Value::as_bool) == Some(true) {
        "draft".into()
    } else if review.get("merged").and_then(Value::as_bool) == Some(true) {
        "merged".into()
    } else {
        status_value(&value_string(&review, &["state"]).unwrap_or_else(|| "open".into()))
    };
    let author = review
        .get("user")
        .or_else(|| review.get("author"))
        .and_then(|value| person_name(Some(value)));
    let assignee = review
        .get("assignees")
        .or_else(|| review.get("assignee"))
        .and_then(|value| person_name(Some(value)));
    let created = value_string(&review, &["created", "created_at"]);
    let updated = value_string(&review, &["updated", "updated_at"]);
    let comment_count = comments.len();
    let status_line = capitalize_status(&status);
    let mut summary = vec![format!("**{item_type}**"), format!("**{status_line}**")];
    if let Some(author) = &author {
        summary.push(format!("Author {author}"));
    }
    if let Some(priority) = value_string(&review, &["priority"]) {
        summary.push(format!("Priority {priority}"));
    }
    summary.push(assignee.as_deref().map_or_else(
        || "Unassigned".into(),
        |assignee| format!("Assignee {assignee}"),
    ));
    let mut body = format!("{}\n", summary.join(" · "));
    let description = demote_headings(&value_string(&review, &["body"]).unwrap_or_default(), 3);
    if !description.trim().is_empty() {
        append_bounded(&mut body, "\n## Description\n\n", MAX_COMMENT_BYTES)?;
        append_bounded(&mut body, &description, MAX_COMMENT_BYTES)?;
    }
    append_review_metadata(&mut body, review)?;
    append_review_comments(&mut body, comments, review_comments)?;
    Ok(TeaReviewRendering {
        title,
        body,
        status,
        author,
        assignee,
        created,
        updated,
        comment_count,
    })
}

pub(crate) enum TeaItem {
    Issue(Issue, Option<String>),
    Review(Value, Option<String>),
    Wiki {
        wiki: Value,
        source_url: String,
        title: String,
        body: String,
    },
}

pub(crate) enum TeaComments {
    Issue(Vec<Comment>),
    Review(Vec<Value>, Vec<Value>),
    Wiki(String),
}

impl Forge for TeaSourceProvider {
    type Identity = (String, ArtifactKind);
    type Item = TeaItem;
    type Comments = TeaComments;

    fn resolve(&self, request: &SourceFetchRequest) -> Result<Self::Identity, InspectionError> {
        artifact_kind(request, &self.base_url)
    }

    fn budget(&self) -> ByteBudget {
        ByteBudget::new(MAX_COMMENT_BYTES)
    }

    async fn fetch_item(
        &self,
        request: &SourceFetchRequest,
        identity: &Self::Identity,
        _: &mut ByteBudget,
    ) -> Result<TeaItem, InspectionError> {
        let (repo, kind) = identity;
        match kind {
            ArtifactKind::Issue(index) => {
                let (issue, url) = self.fetch_issue(request, repo, *index).await?;
                Ok(TeaItem::Issue(issue, url))
            }
            ArtifactKind::Review(index) => {
                let (review, url) = self.fetch_review(request, repo, *index).await?;
                Ok(TeaItem::Review(review, url))
            }
            ArtifactKind::Wiki(page) => self.fetch_wiki(request, repo, page).await,
        }
    }

    async fn fetch_comments(
        &self,
        identity: &Self::Identity,
        item: &TeaItem,
        _: &mut ByteBudget,
    ) -> Result<TeaComments, InspectionError> {
        let (repo, kind) = identity;
        match (kind, item) {
            (ArtifactKind::Issue(index), TeaItem::Issue(..)) => Ok(TeaComments::Issue(
                self.fetch_issue_comments(repo, *index).await?,
            )),
            (ArtifactKind::Review(index), TeaItem::Review(..)) => {
                let (comments, positions) = self.fetch_review_comments(repo, *index).await?;
                Ok(TeaComments::Review(comments, positions))
            }
            (ArtifactKind::Wiki(page), TeaItem::Wiki { wiki, .. }) => Ok(TeaComments::Wiki(
                self.fetch_wiki_revision(wiki, repo, page).await?,
            )),
            _ => unreachable!("resolved identity belongs to the fetched item kind"),
        }
    }

    fn assemble(
        &self,
        request: &SourceFetchRequest,
        identity: Self::Identity,
        item: TeaItem,
        comments: TeaComments,
        _: &mut ByteBudget,
    ) -> Result<SourceAsset, InspectionError> {
        let (repo, kind) = identity;
        match (kind, item, comments) {
            (ArtifactKind::Issue(_), TeaItem::Issue(issue, url), TeaComments::Issue(comments)) => {
                self.assemble_issue(request, repo, issue, url, comments)
            }
            (
                ArtifactKind::Review(index),
                TeaItem::Review(review, url),
                TeaComments::Review(comments, positions),
            ) => self.assemble_review(request, repo, index, review, url, comments, positions),
            (
                ArtifactKind::Wiki(page),
                TeaItem::Wiki {
                    source_url,
                    title,
                    body,
                    ..
                },
                TeaComments::Wiki(revision),
            ) => Ok(self.assemble_wiki(request, repo, page, source_url, title, body, revision)),
            _ => unreachable!("comments belong to the fetched item kind"),
        }
    }
}

fn classify_cli_failure(_: &[u8]) -> InspectionError {
    InspectionError::new("source_provider_failed", "Tea read request failed")
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
                            "--fields".into(),
                            "index,title,body,url,updated,state,priority,assignee,assignees,user,created,comments".into(),
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
                let source_url =
                    optional_verified_url(request, &self.base_url, issue.html_url.as_deref())?;
                Ok(SourceMetadata {
                    title: issue.title,
                    source_branch: None,
                    source_url,
                    source_commit: None,
                    description: issue.body,
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
                            "index,title,body,head,url,state,priority,assignee,assignees,user,created,updated,draft,merged".into(),
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
                let source_url = optional_verified_url(
                    request,
                    &self.base_url,
                    value_string(&review, &["html_url", "url"]).as_deref(),
                )?;
                Ok(SourceMetadata {
                    title,
                    source_branch,
                    source_url,
                    source_commit: None,
                    description: value_string(&review, &["body"]),
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
        forge::fetch(self, request).await
    }
}

#[cfg(test)]
mod tests {
    use super::{
        Duration, SourceFetchRequest, SourceProvider, TeaSourceProvider, Url, append_comment_card,
        base_path, demote_headings, parse_comments, provider_instance, review_source_branch,
        verified_url,
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
    fn tea_comment_cards_fallback_to_id_and_mark_only_real_edits() {
        let comment = serde_json::json!({
            "id": 7,
            "body": "### Details\n```md\n# preserved\n```",
            "created_at": "2026-01-01T00:00:00Z",
            "updated_at": "2026-01-01T01:02:00Z",
            "path": "src/lib.rs",
            "line": 12,
            "user": { "login": "reviewer" }
        });
        let mut body = String::new();
        append_comment_card(&mut body, &comment, true).unwrap();
        assert!(body.starts_with(
            "### reviewer · 2026-01-01 00:00 · edited 2026-01-01 01:02 · review on src/lib.rs:12"
        ));
        assert!(body.contains("\n#7\n\n"));
        assert!(body.contains("#### Details\n```md\n# preserved\n```"));
        assert_eq!(
            demote_headings("# top\n## nested\n```\n# code\n```\n", 3),
            "### top\n### nested\n```\n# code\n```\n"
        );
    }

    #[test]
    fn canonical_api_urls_match_instance_repository_and_artifact() {
        let base = Url::parse("https://forge.test:9443/gitea").unwrap();
        for path in ["issues/7", "pulls/7", "wiki/Guide"] {
            let url = format!("https://forge.test:9443/gitea/acme/repo/{path}");
            let request = source_request("tea", &base, url.clone());
            assert_eq!(verified_url(&request, &base, Some(&url)).unwrap(), url);
            for mismatch in [
                url.replace("https:", "http:"),
                url.replace("forge.test", "wrong.test"),
                url.replace(":9443", ""),
                url.replace("/gitea/", "/other/"),
                url.replace("acme/repo", "other/repo"),
                format!("{url}/extra"),
                format!("{url}?other=1"),
            ] {
                assert_eq!(
                    verified_url(&request, &base, Some(&mismatch))
                        .unwrap_err()
                        .code,
                    "source_identity_mismatch"
                );
            }
        }
        let base = Url::parse("https://forge.test/gitea").unwrap();
        let request = source_request(
            "tea",
            &base,
            "https://forge.test/gitea/acme/repo/issues/7".into(),
        );
        assert!(
            verified_url(
                &request,
                &base,
                Some("https://forge.test:443/gitea/acme/repo/issues/7")
            )
            .is_ok()
        );
        assert_eq!(
            verified_url(
                &request,
                &base,
                Some("https://forge.test/gitea/acme/repo/issues/8")
            )
            .unwrap_err()
            .code,
            "source_identity_mismatch"
        );
    }

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
        if path.contains("/pulls/1/reviews/7/comments") {
            return r###"[{"id":9,"body":"## review comment","path":"src/lib.rs","line":7,"created_at":"2026-01-01T00:00:00Z","updated_at":"2026-01-01T00:00:00Z","html_url":"http://x/comments/9","user":{"id":1,"login":"fixture"}}]"###.into();
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
        if path.contains("/pulls/1/reviews") {
            return r#"[{"id":7,"body":"review","state":"COMMENT","submitted_at":"2026-01-01T00:00:00Z","user":{"id":1,"login":"fixture"}}]"#.into();
        }
        if path.contains("/pulls/1") {
            return format!(
                r#"{{"id":1,"number":1,"title":"review","body":"review body","html_url":"{base_url}/acme/repo/pulls/1","diff_url":"{base_url}/api/v1/repos/acme/repo/pulls/1.diff","updated_at":"2026-01-01T00:00:00Z","created_at":"2025-12-31T23:00:00Z","state":"open","base":{{"ref":"main","sha":"base-sha"}},"head":{{"ref":"feature","sha":"head-sha"}},"user":{{"id":1,"login":"fixture"}}}}"#
            );
        }
        if path.contains("/wiki/") {
            return format!(
                r#"{{"title":"Guide","content_base64":"IyBndWlkZQo=","sha":"wiki-sha","html_url":"{base_url}/acme/repo/wiki/Guide","last_commit":{{"id":"wiki-sha"}}}}"#
            );
        }
        if path.contains("issues/1") {
            let number = match mode {
                FixtureMode::WrongIssueIndex => 2,
                _ => 1,
            };
            return format!(
                r###"{{"id":1,"index":{number},"number":{number},"title":"issue","body":"## Details\n# Top\n```md\n# fenced\n```\n","html_url":"{base_url}/acme/repo/issues/{number}","created_at":"2025-12-31T23:00:00Z","updated_at":"2026-01-01T00:00:00Z","state":"open","comments":1,"user":{{"id":1,"login":"fixture"}}}}"###
            );
        }
        r#"{"id":1,"login":"fixture","full_name":"Fixture","email":"x","avatar_url":"","language":"en-US","is_admin":false,"active":true,"restricted":false}"#.into()
    }

    fn comments_json(count: usize, body: &str) -> String {
        let comments = (1..=count)
            .map(|id| format!(r#"{{"id":{id},"body":"{body}","created_at":"2026-01-01T00:00:00Z","updated_at":"2026-01-01T00:00:00Z","html_url":"http://x/comments/{id}","user":{{"id":1,"login":"fixture"}}}}"#))
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
                let issue_json = format!(
                    r#"{{"id":1,"index":1,"number":1,"title":"issue","body":"body","html_url":"http://{addr}/acme/repo/issues/1","created_at":"2025-12-31T23:00:00Z","updated_at":"2026-01-01T00:00:00Z","state":"open","comments":1,"user":{{"id":1,"login":"fixture"}}}}"#
                );
                let body = if path.contains("/user/keys") || path.contains("reactions") {
                    "[]"
                } else if path.contains("comments") {
                    r#"[{"id":1,"body":"comment","created_at":"2026-01-01T00:00:00Z","updated_at":"2026-01-01T00:00:00Z","html_url":"http://x/comments/1","user":{"id":1,"login":"fixture"}}]"#
                } else if path.contains("issues/1") {
                    &issue_json
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
        assert!(assets[0].body.starts_with("**Issue** · **Open** · Author fixture · Assignee Unassigned"));
        assert!(assets[0].body.contains("## Description\n\nbody"));
        assert!(assets[0].body.contains("## Comments (1)\n\n### "));
        assert!(assets[0].body.contains("\n\n#1\n\n"));
        assert_eq!(
            assets[0].fields.iter().find(|field| field.key == "item_type").unwrap().value,
            cockpit_core::sources::FrontmatterValue::String("Issue".into())
        );
        assert_eq!(
            assets[0].fields.iter().find(|field| field.key == "comment_count").unwrap().value,
            cockpit_core::sources::FrontmatterValue::Number(1)
        );
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
        assert!(assets[0].body.starts_with("**Issue**"));
        assert!(assets[0].body.contains("## Description\n\n"));
        assert!(!assets[0].body.contains("## Comments ("));
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
        assert!(assets[0].body.starts_with("**Pull request** · **Open**"));
        assert!(assets[0].body.contains("## Description\n\nreview body"));
        assert!(assets[0].body.contains("## Comments (2)"));
        assert!(assets[0].body.contains("### fixture · 2026-01-01 00:00 · review on src/lib.rs"));
        assert!(assets[0].body.contains("[#9](http://x/comments/9)"));
        assert!(assets[0].body.contains("#### review comment"));
        assert!(!assets[0].body.contains("edited"));
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
