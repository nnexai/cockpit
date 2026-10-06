use std::collections::BTreeSet;
use std::path::Path;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use async_trait::async_trait;
use cockpit_core::InspectionError;
use cockpit_core::credentials::ProviderCredentials;
use cockpit_core::jira_query::{instant_seconds, jira_query_input, wall_minute};
use cockpit_core::process::StagingBudget;
use cockpit_core::repositories::{is_jira_key, resolve_jira_url};
use cockpit_core::sources::{
    AttachmentRef, DownloadedAttachment, FrontmatterField, FrontmatterValue, IssueListing,
    IssueQuery, IssueRow, SourceAsset, SourceContainer, SourceFetchRequest, SourceMetadata,
    SourceProvider, SourceRef,
};
use cockpit_protocol::projects::{
    ProjectConfiguration, ProjectDiagnostic, ProjectProvider, ProviderDeployment, ProviderKind,
};
use cockpit_protocol::sources::SourceCapability;
use serde_json::Value;
use url::Url;

use crate::jira_attachments::{
    DOWNLOADED_NAME, download_issue_attachment, issue_attachments, partial_diagnostic,
    valid_attachment_id,
};
use crate::jira_wiki::wiki_to_markdown;
use crate::site_http::{FailureKind, HttpFailure, MAX_JSON_BYTES, Service, SiteHttp};

const MAX_ISSUE_BYTES: usize = 1024 * 1024;
const MAX_DOCUMENT_DEPTH: usize = 48;

const ISSUE_FIELDS: &str = "summary,description,issuetype,status,priority,assignee,reporter,created,updated,comment,attachment,parent,subtasks,issuelinks";
const SEARCH_FIELDS: &str = "updated,status,issuetype,assignee";
const PAGE: u32 = 100;
const MAX_COMMENTS: u64 = 1000;

/// Read-only Jira work items using the token stored in Cockpit's OS vault.
#[derive(Debug)]
pub struct JiraSourceProvider {
    provider: ProjectProvider,
    base_url: Url,
    deployment: ProviderDeployment,
    http: SiteHttp,
}

impl JiraSourceProvider {
    pub fn configured(
        configuration: &ProjectConfiguration,
        provider_id: &str,
        credentials: Arc<ProviderCredentials>,
    ) -> Result<Self, InspectionError> {
        let provider = configuration
            .providers
            .iter()
            .find(|provider| provider.id == provider_id)
            .ok_or_else(|| {
                InspectionError::new(
                    "source_provider_unsupported",
                    "Jira provider is not configured",
                )
            })?;
        if provider.kind != ProviderKind::Jira || provider.deployment.is_none() {
            return Err(InspectionError::new(
                "source_provider_invalid",
                "Jira provider requires kind jira and a resolved deployment",
            ));
        }
        let base_url = Url::parse(&provider.base_url).map_err(|_| {
            InspectionError::new("source_provider_invalid", "Jira site URL is invalid")
        })?;
        if !matches!(base_url.scheme(), "http" | "https")
            || base_url.host_str().is_none()
            || !base_url.username().is_empty()
            || base_url.password().is_some()
            || base_url.query().is_some()
            || base_url.fragment().is_some()
        {
            return Err(InspectionError::new(
                "source_provider_invalid",
                "Jira site URL must be credential-free HTTP(S)",
            ));
        }
        let http = SiteHttp::new(
            base_url.clone(),
            provider_id,
            Service::Jira,
            Duration::from_millis(configuration.limits.operation_timeout_ms as u64),
            credentials,
        );
        Ok(Self {
            provider: provider.clone(),
            base_url,
            deployment: provider.deployment.unwrap(),
            http,
        })
    }

    fn key(&self, request: &SourceFetchRequest) -> Result<String, InspectionError> {
        if request.provider_id != self.provider.id
            || request.authority.provider_instance != self.base_url.as_str().trim_end_matches('/')
            || request.authority.origin_port != self.base_url.port()
            || request.authority.origin_base_path.trim_end_matches('/')
                != self.base_url.path().trim_end_matches('/')
            || !request.authority.owner.is_empty()
            || !request.authority.repository.is_empty()
            || !request
                .authority
                .origin_host
                .eq_ignore_ascii_case(self.base_url.host_str().unwrap_or_default())
        {
            return Err(InspectionError::new(
                "source_identity_mismatch",
                "Jira request does not match the configured Jira site",
            ));
        }
        Ok(resolve_jira_url(&self.provider, &request.artifact_url)?.canonical_id)
    }

    fn api(&self) -> &'static str {
        match self.deployment {
            ProviderDeployment::Cloud => "3",
            ProviderDeployment::DataCenter => "2",
        }
    }

    /// Page only through Cockpit-built endpoints, never server-provided URLs.
    async fn search(
        &self,
        jql: &str,
        max: u32,
        cancel: &AtomicBool,
    ) -> Result<IssueListing, ListFailure> {
        let mut rows = Vec::new();
        let mut seen = BTreeSet::new();
        let mut tokens = BTreeSet::new();
        let mut token: Option<String> = None;
        let mut start = 0u64;
        let complete = loop {
            if cancel.load(Ordering::Relaxed) {
                break false;
            }
            let start_text = if self.deployment == ProviderDeployment::DataCenter {
                start.to_string()
            } else {
                String::new()
            };
            let continuation = match self.deployment {
                ProviderDeployment::Cloud => token.as_deref().map(|token| ("nextPageToken", token)),
                ProviderDeployment::DataCenter => Some(("startAt", start_text.as_str())),
            };
            let query = [
                ("jql", jql), ("fields", SEARCH_FIELDS), ("maxResults", "100"),
                continuation.unwrap_or(("nextPageToken", "")),
            ];
            let segments: &[&str] = match self.deployment {
                ProviderDeployment::Cloud => &["rest", "api", "3", "search", "jql"],
                ProviderDeployment::DataCenter => &["rest", "api", "2", "search"],
            };
            let pairs = if continuation.is_some() { &query[..] } else { &query[..3] };
            let value = self.http.get_json(self.http.endpoint(segments, pairs), MAX_JSON_BYTES).await?;
            let page = search_rows(&value)?;
            let count = page.len() as u64;
            let done = match self.deployment {
                ProviderDeployment::Cloud => {
                    let next = match value.get("nextPageToken") {
                        None | Some(Value::Null) => None,
                        Some(Value::String(next)) if !next.is_empty() => Some(next.clone()),
                        _ => return Err(list_contract("Jira search continuation is malformed").into()),
                    };
                    let last = match value.get("isLast") {
                        None => false,
                        Some(Value::Bool(last)) => *last,
                        _ => return Err(list_contract("Jira search completion flag is malformed").into()),
                    };
                    if value.get("isLast") == Some(&Value::Bool(false)) && next.is_none() {
                        return Err(list_contract("Jira search omitted a continuation token for a non-last page").into());
                    }
                    if !last && count == 0 && (next.is_some() || value.get("isLast").is_some()) {
                        return Err(list_contract("Jira search returned an empty non-last page").into());
                    }
                    if !last && let Some(next) = &next {
                        if !tokens.insert(next.clone()) {
                            return Err(list_contract("Jira search repeated a continuation token").into());
                        }
                    }
                    token = next;
                    last || token.is_none()
                }
                ProviderDeployment::DataCenter => {
                    let returned_start = value.get("startAt").and_then(Value::as_u64)
                        .ok_or_else(|| list_contract("Jira search has no page offset"))?;
                    let total = value.get("total").and_then(Value::as_u64)
                        .ok_or_else(|| list_contract("Jira search has no total"))?;
                    if returned_start != start {
                        return Err(list_contract("Jira search returned a different page offset").into());
                    }
                    start = start.checked_add(count)
                        .ok_or_else(|| list_contract("Jira search offset overflowed"))?;
                    start >= total
                }
            };
            let fresh = unique_new(page, &mut seen);
            if count != 0 && fresh.is_empty() && !done {
                return Err(list_contract("Jira search made no progress").into());
            }
            rows.extend(fresh);
            if rows.len() > max as usize {
                rows.truncate(max as usize);
                break false;
            }
            if done {
                break true;
            }
            if count == 0 || rows.len() == max as usize {
                break false;
            }
        };
        Ok(IssueListing { rows, complete: complete && !cancel.load(Ordering::Relaxed) })
    }
    /// Freeze an absolute, timezone-independent interval across all search pages.
    async fn list_jql(
        &self,
        jql: &str,
        updated_window: Option<(i64, i64)>,
        max: u32,
        cancel: &AtomicBool,
    ) -> Result<IssueListing, InspectionError> {
        if jql.len() > 2048
            || !jira_query_input(jql)
                .is_ok_and(|input| input.is_some_and(|input| input.jql == jql))
        {
            return Err(list_contract("Jira query is not a normalized query"));
        }
        let body = if let Some((lower, upper)) = updated_window {
            if lower < 0 || lower >= upper {
                return Err(list_contract("Jira update window must be a nonempty epoch-millisecond interval"));
            }
            // JQL documents unquoted numeric dates as epoch milliseconds. Quoted
            // wall-clock dates instead depend on the account/server timezone.
            format!("({jql}) AND updated >= {lower} AND updated < {upper} ORDER BY updated ASC, key ASC")
        } else {
            format!("{jql} ORDER BY updated DESC")
        };
        self.search(&body, max, cancel).await.map_err(|failure| failure.error)
    }

    /// Metadata for specific keys, 100 per call. A batch Jira rejects is
    /// bisected; a single rejected key is absent from the answer.
    async fn list_keys(
        &self,
        keys: &[String],
        max: u32,
        cancel: &AtomicBool,
    ) -> Result<IssueListing, InspectionError> {
        let mut requested = BTreeSet::new();
        if keys.iter().any(|key| !is_jira_key(key)) {
            return Err(list_contract("Jira work item key is malformed"));
        }
        let unique: Vec<&String> = keys.iter().filter(|key| requested.insert(key.as_str())).collect();
        let mut stack: Vec<Vec<&String>> = unique
            .chunks(PAGE as usize)
            .rev()
            .map(<[&String]>::to_vec)
            .collect();
        let mut rows: Vec<IssueRow> = Vec::new();
        let mut seen = BTreeSet::new();
        let mut complete = true;
        while let Some(part) = stack.pop() {
            if cancel.load(Ordering::Relaxed) {
                complete = false;
                break;
            }
            let names: Vec<&str> = part.iter().map(|key| key.as_str()).collect();
            let jql = format!("key in ({})", names.join(", "));
            match self.search(&jql, PAGE, cancel).await {
                Ok(listing) => {
                    complete &= listing.complete;
                    let found = listing.rows;
                    let found = found
                        .into_iter()
                        .filter(|row| requested.contains(row.key.as_str()))
                        .collect();
                    rows.extend(unique_new(found, &mut seen));
                }
                Err(failure) if failure.rejected && part.len() > 1 => {
                    let (left, right) = part.split_at(part.len() / 2);
                    stack.push(right.to_vec());
                    stack.push(left.to_vec());
                }
                Err(failure) if failure.rejected => {}
                Err(failure) => return Err(failure.error),
            }
        }
        if rows.len() > max as usize {
            rows.truncate(max as usize);
            complete = false;
        }
        Ok(IssueListing { rows, complete })
    }

    async fn issue(&self, key: &str, fields: &str) -> Result<Value, InspectionError> {
        let url = self.http.endpoint(&["rest", "api", self.api(), "issue", key], &[("fields", fields)]);
        let value = self.http.get_json(url, MAX_JSON_BYTES).await.map_err(|failure| {
            if failure.kind == FailureKind::NotFound {
                InspectionError::new(
                    "source_not_found",
                    "Jira work item does not exist or is not visible to the stored token",
                )
            } else {
                failure.error
            }
        })?;
        if value.get("key").and_then(Value::as_str) != Some(key) {
            return Err(InspectionError::new(
                "source_identity_mismatch",
                "Jira returned a different work item",
            ));
        }
        // The API record must independently identify this site and work item.
        let from_site = value
            .get("self")
            .and_then(Value::as_str)
            .and_then(|url| Url::parse(url).ok())
            .is_some_and(|url| {
                url.scheme() == self.base_url.scheme()
                    && url.host_str() == self.base_url.host_str()
                    && url.port_or_known_default() == self.base_url.port_or_known_default()
                    && url.username().is_empty()
                    && url.password().is_none()
                    && url.query().is_none()
                    && url.fragment().is_none()
                    && url
                        .path()
                        .strip_prefix(&format!(
                            "{}/rest/api/",
                            self.base_url.path().trim_end_matches('/')
                        ))
                        .is_some_and(|path| {
                            let parts: Vec<_> = path.split('/').collect();
                            parts.len() == 3
                                && matches!(parts[0], "2" | "3" | "latest")
                                && parts[1] == "issue"
                                && (parts[2] == key
                                    || value.get("id").and_then(Value::as_str) == Some(parts[2]))
                        })
            });
        if !from_site {
            return Err(InspectionError::new(
                "source_identity_mismatch",
                "Jira answered for a different site",
            ));
        }
        Ok(value)
    }

    async fn complete_comments(&self, key: &str, issue: &mut Value) -> Result<(), InspectionError> {
        let Some(comment) = issue.get("fields").and_then(|fields| fields.get("comment")) else {
            return Ok(());
        };
        let embedded = comment.get("comments").and_then(Value::as_array)
            .ok_or_else(|| list_contract("Jira comment page is malformed"))?;
        let total = comment.get("total").and_then(Value::as_u64).unwrap_or(embedded.len() as u64);
        if total <= embedded.len() as u64 {
            return Ok(());
        }
        let mut start = total.saturating_sub(MAX_COMMENTS);
        let mut comments = Vec::new();
        let mut ids = BTreeSet::new();
        while start < total {
            let start_text = start.to_string();
            let url = self.http.endpoint(
                &["rest", "api", self.api(), "issue", key, "comment"],
                &[("startAt", &start_text), ("maxResults", "100")],
            );
            let page = self.http.get_json(url, MAX_JSON_BYTES).await.map_err(InspectionError::from)?;
            if page.get("startAt").and_then(Value::as_u64) != Some(start) {
                return Err(list_contract("Jira comments returned a different page offset"));
            }
            let list = page.get("comments").and_then(Value::as_array)
                .ok_or_else(|| list_contract("Jira comment page is malformed"))?;
            if list.len() > PAGE as usize {
                return Err(list_contract("Jira comment page exceeded the requested limit"));
            }
            if list.is_empty() {
                break;
            }
            for comment in list {
                let id = comment.get("id").and_then(Value::as_str)
                    .ok_or_else(|| list_contract("Jira comment has no ID"))?;
                if !ids.insert(id.to_owned()) {
                    return Err(list_contract("Jira comments repeated an ID"));
                }
            }
            let remaining = (MAX_COMMENTS as usize).saturating_sub(comments.len());
            comments.extend(list.iter().take(remaining).cloned());
            start = start.checked_add(list.len() as u64)
                .ok_or_else(|| list_contract("Jira comment offset overflowed"))?;
            if comments.len() == MAX_COMMENTS as usize {
                break;
            }
        }
        issue["fields"]["comment"]["comments"] = Value::Array(comments);
        Ok(())
    }

    fn browse_url(&self, key: &str) -> String {
        format!(
            "{}/browse/{key}",
            self.base_url.as_str().trim_end_matches('/')
        )
    }
}

#[async_trait]
impl SourceProvider for JiraSourceProvider {
    fn provider_id(&self) -> &str {
        &self.provider.id
    }

    fn blocked_until_ms(&self) -> Option<i64> {
        self.http.blocked_until_ms()
    }

    fn background_requests_remaining(&self) -> Option<u32> {
        Some(self.http.remaining_background_requests())
    }

    fn capabilities(&self) -> Vec<SourceCapability> {
        vec![SourceCapability::Issue, SourceCapability::IssueComments]
    }

    async fn metadata(
        &self,
        request: &SourceFetchRequest,
    ) -> Result<SourceMetadata, InspectionError> {
        let key = self.key(request)?;
        let issue = self.issue(&key, "summary").await?;
        Ok(SourceMetadata {
            title: summary(&issue)?,
            source_branch: None,
            source_url: Some(self.browse_url(&key)),
            source_commit: None,
            description: None,
        })
    }

    async fn fetch(
        &self,
        request: &SourceFetchRequest,
    ) -> Result<Vec<SourceAsset>, InspectionError> {
        let key = self.key(request)?;
        let mut issue = self.issue(&key, ISSUE_FIELDS).await?;
        self.complete_comments(&key, &mut issue).await?;
        let title = summary(&issue)?;
        let source_url = self.browse_url(&key);
        let (body, complete) = issue_markdown(&issue, &source_url)?;
        let (attachments, attachments_complete) = issue_attachments(&issue, &self.base_url);
        let mut diagnostics = if complete {
            Vec::new()
        } else {
            vec![ProjectDiagnostic {
                code: "source_comments_partial".into(),
                message: "Jira returned only the most recent comments".into(),
                path: None,
            }]
        };
        if !attachments_complete {
            diagnostics.push(partial_diagnostic());
        }
        if reference_fields(&issue).1 {
            diagnostics.push(ProjectDiagnostic {
                code: "source_references_truncated".into(),
                message: "Jira returned more subtasks or links than Cockpit keeps; some related issues are not followed".into(),
                path: None,
            });
        }
        let project_key = key
            .rsplit_once('-')
            .map(|(project, _)| project)
            .unwrap_or(&key);
        Ok(vec![SourceAsset {
            source: SourceRef {
                provider_id: self.provider.id.clone(),
                provider_instance: request.authority.provider_instance.clone(),
                resource_type: "issue".into(),
                canonical_id: key.clone(),
            },
            title,
            source_url: Some(source_url),
            original_url: None,
            source_revision: field_str(&issue, "updated").map(str::to_owned),
            complete,
            diagnostics,
            body,
            container: Some(SourceContainer {
                id: project_key.into(),
                label: project_key.into(),
            }),
            fields: issue_fields(&issue),
            attachments,
        }])
    }

    async fn list_issues(
        &self,
        query: &IssueQuery<'_>,
        max: u32,
        cancel: &AtomicBool,
    ) -> Result<IssueListing, InspectionError> {
        match query {
            IssueQuery::Jql { jql, updated_window } => {
                self.list_jql(jql, *updated_window, max, cancel).await
            }
            IssueQuery::Keys(keys) => self.list_keys(keys, max, cancel).await,
        }
    }

    async fn attachment_downloads(&self, resource_type: &str) -> Result<(), InspectionError> {
        if resource_type != "issue" {
            return Err(InspectionError::new(
                "source_capability_unavailable",
                "selected source provider does not support this operation",
            ));
        }
        self.http.require_credentials().await
    }

    async fn download_attachment(
        &self,
        canonical_id: &str,
        attachment: &AttachmentRef,
        _siblings: &[AttachmentRef],
        dest: &cap_std::fs::Dir,
        _dest_path: &Path,
        budget: StagingBudget,
    ) -> Result<DownloadedAttachment, InspectionError> {
        if !is_jira_key(canonical_id) || !valid_attachment_id(&attachment.id) {
            return Err(InspectionError::new(
                "source_provider_contract",
                "Jira attachment download needs a valid issue key and attachment id",
            ));
        }
        download_issue_attachment(&self.http, attachment, budget.bytes, dest).await?;
        Ok(DownloadedAttachment {
            attachment_id: attachment.id.clone(),
            file_name: DOWNLOADED_NAME.into(),
        })
    }
}

/// Stable plain `updated` format persisted by Jira follow.
fn listed_updated(value: &str) -> bool {
    let bytes = value.as_bytes();
    bytes.len() == 19
        && bytes[10] == b' '
        && bytes[16] == b':'
        && bytes[17..].iter().all(u8::is_ascii_digit)
        && bytes[17] <= b'5'
        && wall_minute(value).is_some()
}

fn list_contract(message: &str) -> InspectionError {
    InspectionError::new("source_provider_contract", message)
}

/// Reject the whole answer if any row is malformed; never guess listing data.
fn search_rows(value: &Value) -> Result<Vec<IssueRow>, InspectionError> {
    let issues = value.get("issues").and_then(Value::as_array)
        .ok_or_else(|| list_contract("Jira search has no issues array"))?;
    if issues.len() > PAGE as usize {
        return Err(list_contract("Jira search exceeded the requested limit"));
    }
    issues.iter().map(|issue| {
        let key = issue.get("key").and_then(Value::as_str)
            .filter(|key| is_jira_key(key))
            .ok_or_else(|| list_contract("Jira search row has no valid key"))?;
        let iso = field_str(issue, "updated")
            .filter(|value| instant_seconds(value).is_some())
            .ok_or_else(|| list_contract("Jira search row has no valid update time"))?;
        let updated = format!("{} {}", &iso[..10], &iso[11..19]);
        if !listed_updated(&updated) {
            return Err(list_contract("Jira search row has no valid update time"));
        }
        let name = |field: &str| field_name(issue, field, "name")
            .map(str::trim).filter(|name| !name.is_empty())
            .ok_or_else(|| list_contract("Jira search row has no status or issue type"));
        let assignee = match issue.get("fields").and_then(|fields| fields.get("assignee")) {
            None | Some(Value::Null) => None,
            Some(value) => Some(value.get("displayName").and_then(Value::as_str)
                .map(str::trim).filter(|name| !name.is_empty())
                .ok_or_else(|| list_contract("Jira search row has a malformed assignee"))?.to_owned()),
        };
        Ok(IssueRow {
            key: key.to_owned(),
            updated,
            status: name("status")?.to_owned(),
            issue_type: name("issuetype")?.to_owned(),
            assignee,
        })
    }).collect()
}

/// Only a rejected query (HTTP 400) triggers key-batch bisection.
struct ListFailure {
    error: InspectionError,
    rejected: bool,
}

impl From<HttpFailure> for ListFailure {
    fn from(failure: HttpFailure) -> Self {
        Self { rejected: failure.kind == FailureKind::Rejected, error: failure.error }
    }
}

impl From<InspectionError> for ListFailure {
    fn from(error: InspectionError) -> Self {
        Self {
            error,
            rejected: false,
        }
    }
}

fn unique_new(rows: Vec<IssueRow>, seen: &mut BTreeSet<String>) -> Vec<IssueRow> {
    rows.into_iter()
        .filter(|row| seen.insert(row.key.clone()))
        .collect()
}

fn field_str<'a>(issue: &'a Value, name: &str) -> Option<&'a str> {
    issue.get("fields")?.get(name)?.as_str()
}

fn field_name<'a>(issue: &'a Value, name: &str, property: &str) -> Option<&'a str> {
    issue.get("fields")?.get(name)?.get(property)?.as_str()
}

fn summary(issue: &Value) -> Result<String, InspectionError> {
    field_str(issue, "summary")
        .map(str::trim)
        .filter(|title| !title.is_empty())
        .map(|title| title.chars().take(256).collect())
        .ok_or_else(|| {
            InspectionError::new("source_provider_contract", "Jira work item has no summary")
        })
}

/// Render an issue and comments as the stable provider Markdown contract.
fn issue_markdown(issue: &Value, source_url: &str) -> Result<(String, bool), InspectionError> {
    let fields = issue.get("fields");
    let comments = fields.and_then(|fields| fields.get("comment"));
    let list = comments
        .and_then(|comment| comment.get("comments"))
        .and_then(Value::as_array)
        .map(Vec::as_slice)
        .unwrap_or_default();
    let mut ordered: Vec<_> = list.iter().collect();
    ordered.sort_by_key(|comment| {
        comment
            .get("created")
            .and_then(Value::as_str)
            .unwrap_or_default()
    });
    let total = comments
        .and_then(|comment| comment.get("total"))
        .and_then(Value::as_u64)
        .unwrap_or(list.len() as u64);
    let mut body = format!("{}\n", summary_line(issue));
    if let Some(description) = fields.and_then(|fields| fields.get("description")) {
        let text = document_markdown_at(description, 2);
        if !text.trim().is_empty() {
            append_bounded(&mut body, &format!("\n## Description\n\n{text}\n"))?;
        }
    }
    if !ordered.is_empty() {
        let label = if total > ordered.len() as u64 {
            format!("{} of {total}", ordered.len())
        } else {
            total.to_string()
        };
        append_bounded(&mut body, &format!("\n## Comments ({label})\n"))?;
        for comment in ordered {
            let id = comment.get("id").and_then(Value::as_str).ok_or_else(|| {
                InspectionError::new("source_provider_contract", "Jira comment has no ID")
            })?;
            let author = comment
                .get("author")
                .and_then(|author| author.get("displayName"))
                .unwrap_or(&Value::Null)
                .as_str()
                .unwrap_or("Unknown");
            let created = comment.get("created").and_then(Value::as_str).unwrap_or_default();
            let updated = comment.get("updated").and_then(Value::as_str);
            let mut heading = format!("### {author} · {}", local_timestamp(created));
            if let Some(updated) = updated.filter(|updated| normalize_timestamp(updated) != normalize_timestamp(created)) {
                heading.push_str(&format!(" · edited {}", local_timestamp(updated)));
            }
            append_bounded(&mut body, &format!("\n{heading}\n"))?;
            let permalink = format!("[#{id}]({source_url}?focusedCommentId={id})");
            append_bounded(&mut body, &format!("{permalink}\n"))?;
            if let Some(text) = comment.get("body") {
                let text = document_markdown_at(text, 3);
                if !text.trim().is_empty() {
                    append_bounded(&mut body, &format!("\n{text}\n"))?;
                }
            }
        }
    }
    Ok((body, total <= list.len() as u64))
}

fn issue_fields(issue: &Value) -> Vec<FrontmatterField> {
    let mut fields = Vec::new();
    let mut push_text = |key: &str, value: Option<String>| {
        if let Some(value) = value {
            fields.push(FrontmatterField {
                key: key.into(),
                value: FrontmatterValue::String(value),
            });
        }
    };
    push_text("issue_id", issue.get("id").and_then(Value::as_str)
        .filter(|id| (1..=20).contains(&id.len()) && id.bytes().all(|byte| byte.is_ascii_digit()))
        .map(str::to_owned));
    push_text("item_type", field_name(issue, "issuetype", "name").map(str::to_owned));
    push_text("status", field_name(issue, "status", "name").map(str::to_owned));
    push_text("priority", field_name(issue, "priority", "name").map(str::to_owned));
    push_text("assignee", field_name(issue, "assignee", "displayName").map(str::to_owned));
    push_text("author", field_name(issue, "reporter", "displayName").map(str::to_owned));
    push_text("created", field_str(issue, "created").map(normalize_timestamp));
    push_text("updated", field_str(issue, "updated").map(normalize_timestamp));
    drop(push_text);
    if issue.pointer("/fields/assignee").is_some() && field_name(issue, "assignee", "displayName").is_none() {
        fields.push(FrontmatterField {
            key: "assignee".into(),
            value: FrontmatterValue::Null,
        });
    }
    let count = issue
        .pointer("/fields/comment/total")
        .and_then(Value::as_u64)
        .or_else(|| issue.pointer("/fields/comment/comments").and_then(Value::as_array).map(|v| v.len() as u64));
    if let Some(count) = count.and_then(|count| i64::try_from(count).ok()) {
        fields.push(FrontmatterField {
            key: "comment_count".into(),
            value: FrontmatterValue::Number(count),
        });
    }
    fields.extend(reference_fields(issue).0);
    fields
}

const MAX_REFERENCE_FIELD_ENTRIES: usize = 256;
/// Keeps each reference field well inside the 16 KiB frontmatter value bound.
const MAX_REFERENCE_FIELD_BYTES: usize = 12 * 1024;

/// Structured Jira relations as frontmatter: `parent` (key), `subtasks`
/// (keys) and `links` (`"<relation> <KEY>"`). The flag is true when entries
/// beyond the entry or byte bound were left out.
fn reference_fields(issue: &Value) -> (Vec<FrontmatterField>, bool) {
    let key_at = |value: &Value| {
        value
            .get("key")
            .and_then(Value::as_str)
            .filter(|key| is_jira_key(key))
            .map(str::to_owned)
    };
    let mut fields = Vec::new();
    let mut truncated = false;
    if let Some(parent) = issue.pointer("/fields/parent").and_then(key_at) {
        fields.push(FrontmatterField {
            key: "parent".into(),
            value: FrontmatterValue::String(parent),
        });
    }
    let subtasks: Vec<String> = issue
        .pointer("/fields/subtasks")
        .and_then(Value::as_array)
        .map(|list| list.iter().filter_map(key_at).collect())
        .unwrap_or_default();
    let relation = |link: &Value, direction: &str, side: &str| {
        let text: String = link
            .pointer(&format!("/type/{direction}"))
            .and_then(Value::as_str)
            .unwrap_or_default()
            .chars()
            .filter(|character| !character.is_control())
            .take(64)
            .collect();
        let text = text.trim();
        let text = if text.is_empty() { "relates to" } else { text };
        link.get(side).and_then(key_at).map(|key| format!("{text} {key}"))
    };
    let links: Vec<String> = issue
        .pointer("/fields/issuelinks")
        .and_then(Value::as_array)
        .map(|list| {
            list.iter()
                .filter_map(|link| {
                    relation(link, "outward", "outwardIssue")
                        .or_else(|| relation(link, "inward", "inwardIssue"))
                })
                .collect()
        })
        .unwrap_or_default();
    for (key, values) in [("subtasks", subtasks), ("links", links)] {
        let mut bytes = 0;
        let kept: Vec<String> = values
            .iter()
            .take(MAX_REFERENCE_FIELD_ENTRIES)
            .take_while(|value| {
                bytes += value.len() + 4;
                bytes <= MAX_REFERENCE_FIELD_BYTES
            })
            .cloned()
            .collect();
        truncated |= kept.len() < values.len();
        if !kept.is_empty() {
            fields.push(FrontmatterField {
                key: key.into(),
                value: FrontmatterValue::Strings(kept),
            });
        }
    }
    if truncated {
        fields.push(FrontmatterField {
            key: "references_truncated".into(),
            value: FrontmatterValue::Boolean(true),
        });
    }
    (fields, truncated)
}

fn summary_line(issue: &Value) -> String {
    let mut segments = Vec::new();
    for (label, value) in [
        (None, field_name(issue, "issuetype", "name")),
        (None, field_name(issue, "status", "name")),
        (Some("Priority"), field_name(issue, "priority", "name")),
        (Some("Reporter"), field_name(issue, "reporter", "displayName")),
        (Some("Assignee"), field_name(issue, "assignee", "displayName")),
    ] {
        if let Some(value) = value {
            segments.push(label.map_or_else(|| format!("**{value}**"), |label| format!("{label} {value}")));
        } else if label == Some("Assignee") {
            segments.push("Unassigned".into());
        }
    }
    segments.join(" · ")
}

fn normalize_timestamp(value: &str) -> String {
    let mut value = value.to_owned();
    if let Some(dot) = value.find('.') {
        let zone = value[dot..]
            .find(['+', '-'])
            .map(|offset| dot + offset)
            .or_else(|| value[dot..].find('Z').map(|offset| dot + offset));
        if let Some(zone) = zone {
            value.replace_range(dot..zone, "");
        }
    }
    if value.ends_with('Z') {
        return value;
    }
    let zone_start = value
        .get(19..)
        .and_then(|suffix| suffix.find(['+', '-']).map(|offset| offset + 19))
        .unwrap_or(value.len());
    let offset = &value[zone_start..];
    if offset.len() == 5 && offset[1..].bytes().all(|byte| byte.is_ascii_digit()) {
        value.insert(zone_start + 3, ':');
    }
    value
}

fn local_timestamp(value: &str) -> String {
    let normalized = normalize_timestamp(value);
    normalized.get(..16).unwrap_or(&normalized).replace('T', " ")
}

fn append_bounded(body: &mut String, value: &str) -> Result<(), InspectionError> {
    body.push_str(value);
    if body.len() > MAX_ISSUE_BYTES {
        return Err(InspectionError::new(
            "source_truncated",
            "Jira work item and comments exceed Cockpit's explicit byte limit",
        ));
    }
    Ok(())
}

fn document_markdown_at(value: &Value, heading_offset: usize) -> String {
    match value {
        Value::String(text) => wiki_to_markdown(text, heading_offset),
        Value::Object(_) => blocks(children(value), 0, heading_offset),
        _ => String::new(),
    }
}

fn children(node: &Value) -> &[Value] {
    node.get("content")
        .and_then(Value::as_array)
        .map(Vec::as_slice)
        .unwrap_or_default()
}


fn attr<'a>(node: &'a Value, name: &str) -> Option<&'a Value> {
    node.get("attrs")?.get(name)
}

fn blocks(nodes: &[Value], depth: usize, heading_offset: usize) -> String {
    joined_blocks(nodes, depth, heading_offset, "\n\n")
}

fn joined_blocks(nodes: &[Value], depth: usize, heading_offset: usize, separator: &str) -> String {
    nodes
        .iter()
        .map(|node| block(node, depth, heading_offset))
        .filter(|text| !text.is_empty())
        .collect::<Vec<_>>()
        .join(separator)
}

fn block(node: &Value, depth: usize, heading_offset: usize) -> String {
    if depth > MAX_DOCUMENT_DEPTH {
        return "[…]".into();
    }
    let next = depth + 1;
    match node.get("type").and_then(Value::as_str).unwrap_or_default() {
        "paragraph" => inline(children(node), next),
        "heading" => {
            let level = attr(node, "level")
                .and_then(Value::as_u64)
                .unwrap_or(1)
                .clamp(1, 6) as usize;
            format!("{} {}", "#".repeat((level + heading_offset).min(6)), inline(children(node), next))
        }
        "bulletList" => list(node, next, heading_offset, |_| "- ".into()),
        "orderedList" => {
            let start = attr(node, "order").and_then(Value::as_u64).unwrap_or(1);
            list(node, next, heading_offset, |index| format!("{}. ", start + index as u64))
        }
        "taskList" | "decisionList" => list(node, next, heading_offset, |_| String::new()),
        "taskItem" | "decisionItem" => {
            let done = attr(node, "state").and_then(Value::as_str) == Some("DONE");
            format!("- [{}] {}", if done { "x" } else { " " }, inline(children(node), next))
        }
        "codeBlock" => {
            let language = attr(node, "language").and_then(Value::as_str).unwrap_or_default();
            let text: String = children(node).iter().filter_map(|child| child.get("text").and_then(Value::as_str)).collect();
            format!("```{language}\n{text}\n```")
        }
        "blockquote" | "panel" => blocks(children(node), next, heading_offset)
            .lines().map(|line| format!("> {line}").trim_end().to_owned()).collect::<Vec<_>>().join("\n"),
        "rule" => "---".into(),
        "mediaSingle" | "mediaGroup" | "media" => "[attachment]".into(),
        "table" => table(node, next, heading_offset),
        "expand" | "nestedExpand" => {
            let title = attr(node, "title").and_then(Value::as_str).unwrap_or_default();
            let content = blocks(children(node), next, heading_offset);
            if title.is_empty() { content } else { format!("**{title}**\n\n{content}") }
        }
        "blockCard" | "embedCard" => attr(node, "url").and_then(Value::as_str).map(|url| format!("<{url}>")).unwrap_or_default(),
        _ if node.get("content").is_some() => {
            let content = children(node);
            if content.iter().all(is_inline) { inline(content, next) } else { blocks(content, next, heading_offset) }
        }
        _ => inline(std::slice::from_ref(node), next),
    }
}

fn is_inline(node: &Value) -> bool {
    matches!(
        node.get("type").and_then(Value::as_str),
        Some("text" | "hardBreak" | "mention" | "emoji" | "inlineCard" | "date" | "status")
    )
}

fn list(node: &Value, depth: usize, heading_offset: usize, marker: impl Fn(usize) -> String) -> String {
    children(node).iter().enumerate().map(|(index, item)| {
        let content = if item.get("type").and_then(Value::as_str) == Some("listItem") {
            joined_blocks(children(item), depth, heading_offset, "\n")
        } else {
            block(item, depth, heading_offset)
        };
        let marker = marker(index);
        let indent = " ".repeat(marker.len().max(2));
        let mut lines = content.lines();
        let first = lines.next().unwrap_or_default();
        std::iter::once(format!("{marker}{first}"))
            .chain(lines.map(|line| if line.is_empty() { String::new() } else { format!("{indent}{line}") }))
            .collect::<Vec<_>>().join("\n")
    }).collect::<Vec<_>>().join("\n")
}

fn table(node: &Value, depth: usize, heading_offset: usize) -> String {
    let table_rows = children(node);
    let has_header = table_rows.first().and_then(|row| children(row).first())
        .is_some_and(|cell| cell.get("type").and_then(Value::as_str) == Some("tableHeader"));
    let rows: Vec<Vec<String>> = table_rows.iter().map(|row| children(row).iter().map(|cell| {
        blocks(children(cell), depth, heading_offset).replace('\n', " ").replace('|', "\\|")
    }).collect()).collect();
    let columns = rows.iter().map(Vec::len).max().unwrap_or(0);
    if columns == 0 { return String::new(); }
    let render = |row: &Vec<String>| {
        let mut cells = row.clone();
        cells.resize(columns, String::new());
        format!("| {} |", cells.join(" | "))
    };
    let mut lines = Vec::new();
    if has_header {
        lines.push(render(&rows[0]));
        lines.push(format!("|{}", " --- |".repeat(columns)));
        lines.extend(rows.iter().skip(1).map(render));
    } else {
        lines.push(render(&vec![String::new(); columns]));
        lines.push(format!("|{}", " --- |".repeat(columns)));
        lines.extend(rows.iter().map(render));
    }
    lines.join("\n")
}

fn inline(nodes: &[Value], depth: usize) -> String {
    if depth > MAX_DOCUMENT_DEPTH {
        return "[…]".into();
    }
    let mut out = String::new();
    for node in nodes {
        match node.get("type").and_then(Value::as_str).unwrap_or_default() {
            "text" => out.push_str(&marked_text(node)),
            "hardBreak" => out.push('\n'),
            "mention" | "emoji" | "status" => {
                let text = attr(node, "text")
                    .or_else(|| attr(node, "shortName"))
                    .and_then(Value::as_str)
                    .unwrap_or_default();
                out.push_str(text);
            }
            "inlineCard" => {
                if let Some(url) = attr(node, "url").and_then(Value::as_str) {
                    out.push_str(&format!("<{url}>"));
                }
            }
            "date" => {
                if let Some(timestamp) = attr(node, "timestamp").and_then(Value::as_str) {
                    out.push_str(timestamp);
                }
            }
            _ => out.push_str(&inline(children(node), depth + 1)),
        }
    }
    out
}

fn marked_text(node: &Value) -> String {
    let mut text = node
        .get("text")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_owned();
    if text.is_empty() {
        return text;
    }
    let marks = node
        .get("marks")
        .and_then(Value::as_array)
        .map(Vec::as_slice)
        .unwrap_or_default();
    let has = |kind: &str| {
        marks
            .iter()
            .any(|mark| mark.get("type").and_then(Value::as_str) == Some(kind))
    };
    if has("code") {
        text = format!("`{text}`");
    }
    if has("strong") {
        text = format!("**{text}**");
    }
    if has("em") {
        text = format!("*{text}*");
    }
    if has("strike") {
        text = format!("~~{text}~~");
    }
    if let Some(href) = marks
        .iter()
        .find(|mark| mark.get("type").and_then(Value::as_str) == Some("link"))
        .and_then(|mark| attr(mark, "href"))
        .and_then(Value::as_str)
    {
        text = format!("[{text}]({href})");
    }
    text
}

#[cfg(test)]
mod tests {
    use super::{document_markdown_at, issue_fields, issue_markdown};
    use serde_json::json;

    #[test]
    fn converts_common_document_nodes_to_markdown() {
        let document = json!({"type": "doc", "version": 1, "content": [
            {"type": "heading", "attrs": {"level": 2}, "content": [{"type": "text", "text": "Acceptance"}]},
            {"type": "paragraph", "content": [
                {"type": "text", "text": "Use "},
                {"type": "text", "text": "retry", "marks": [{"type": "code"}]},
                {"type": "text", "text": " and see "},
                {"type": "text", "text": "docs", "marks": [{"type": "link", "attrs": {"href": "https://example.test/d"}}]},
                {"type": "hardBreak"},
                {"type": "mention", "attrs": {"text": "@Ann"}}
            ]},
            {"type": "bulletList", "content": [
                {"type": "listItem", "content": [{"type": "paragraph", "content": [{"type": "text", "text": "One"}]}]},
                {"type": "listItem", "content": [
                    {"type": "paragraph", "content": [{"type": "text", "text": "Two"}]},
                    {"type": "orderedList", "content": [
                        {"type": "listItem", "content": [{"type": "paragraph", "content": [{"type": "text", "text": "Nested"}]}]}
                    ]}
                ]}
            ]},
            {"type": "codeBlock", "attrs": {"language": "rust"}, "content": [{"type": "text", "text": "fn main() {}"}]},
            {"type": "mediaSingle", "content": [{"type": "media", "attrs": {"id": "x"}}]}
        ]});
        assert_eq!(
            document_markdown_at(&document, 0),
            "## Acceptance\n\nUse `retry` and see [docs](https://example.test/d)\n@Ann\n\n- One\n- Two\n  1. Nested\n\n```rust\nfn main() {}\n```\n\n[attachment]"
        );
    }

    #[test]
    fn structured_relations_become_reference_fields_and_overflow_is_reported() {
        let issue = json!({"fields": {
            "parent": {"key": "OPS-1"},
            "subtasks": [{"key": "OPS-2"}, {"key": "not a key"}],
            "issuelinks": [
                {"type": {"inward": "is blocked by", "outward": "blocks"}, "outwardIssue": {"key": "OPS-3"}},
                {"type": {"inward": "is blocked by", "outward": "blocks"}, "inwardIssue": {"key": "OPS-4"}},
            ],
        }});
        let (fields, truncated) = super::reference_fields(&issue);
        assert!(!truncated);
        let text = |key: &str| fields.iter().find(|field| field.key == key).map(|field| field.value.clone());
        assert_eq!(text("parent"), Some(super::FrontmatterValue::String("OPS-1".into())));
        assert_eq!(text("subtasks"), Some(super::FrontmatterValue::Strings(vec!["OPS-2".into()])));
        assert_eq!(
            text("links"),
            Some(super::FrontmatterValue::Strings(vec!["blocks OPS-3".into(), "is blocked by OPS-4".into()]))
        );
        let many: Vec<_> = (1..=300).map(|n| json!({"key": format!("OPS-{n}")})).collect();
        let (fields, truncated) = super::reference_fields(&json!({"fields": {"subtasks": many}}));
        assert!(truncated);
        assert!(matches!(&fields[0].value, super::FrontmatterValue::Strings(keys) if keys.len() == 256));
    }

    #[test]
    fn renders_contract_sections_and_partial_comments() {
        let issue = json!({"key": "SCRUM-5", "fields": {
            "summary": "Fix login",
            "issuetype": {"name": "Task"}, "status": {"name": "To Do"},
            "priority": {"name": "Medium"}, "reporter": {"displayName": "Konni"},
            "assignee": {"displayName": "Ann"},
            "created": "2026-09-24T18:40:53.898+0200",
            "updated": "2026-09-24T18:44:53.898+0200",
            "description": {"type": "doc", "content": [
                {"type": "heading", "attrs": {"level": 1}, "content": [{"type": "text", "text": "Body"}]},
                {"type": "paragraph", "content": [{"type": "text", "text": "Text"}]}
            ]},
            "comment": {"total": 2, "comments": [{"id": "10001", "author": {"displayName": "Ann"},
                "created": "2026-09-24T18:44:53.898+0200", "updated": "2026-09-24T18:44:53.898+0200",
                "body": {"type": "doc", "content": [{"type": "heading", "attrs": {"level": 2},
                    "content": [{"type": "text", "text": "Reply"}]}]}}]}
        }});
        let (body, complete) = issue_markdown(&issue, "https://site.test/browse/SCRUM-5").unwrap();
        assert!(body.contains("**Task** · **To Do** · Priority Medium · Reporter Konni · Assignee Ann"));
        assert!(body.contains("## Description\n\n### Body\n\nText"));
        assert!(!body.contains("edited"));
        assert!(body.contains("## Comments (1 of 2)\n\n### Ann · 2026-09-24 18:44\n[#10001](https://site.test/browse/SCRUM-5?focusedCommentId=10001)\n\n##### Reply"));
        assert!(!complete);
        let fields = issue_fields(&issue);
        assert!(fields.iter().any(|field| field.key == "comment_count" && field.value == super::FrontmatterValue::Number(2)));
        assert!(fields.iter().any(|field| field.key == "created" && field.value == super::FrontmatterValue::String("2026-09-24T18:40:53+02:00".into())));
        let unassigned = json!({"fields": {"assignee": null}});
        assert_eq!(
            issue_fields(&unassigned).iter().find(|field| field.key == "assignee").unwrap().value,
            super::FrontmatterValue::Null
        );
        let mut edited_issue = issue.clone();
        edited_issue["fields"]["comment"]["comments"][0]["updated"] =
            json!("2026-09-24T18:45:53+0200");
        let (edited, _) =
            issue_markdown(&edited_issue, "https://site.test/browse/SCRUM-5").unwrap();
        assert!(edited.contains("### Ann · 2026-09-24 18:44 · edited 2026-09-24 18:45"));
    }

    #[test]
    fn a_non_header_adf_table_keeps_first_data_row_as_data() {
        let table = json!({"type": "table", "content": [
            {"type": "tableRow", "content": [{"type": "tableCell", "content": [{"type": "paragraph", "content": [{"type": "text", "text": "first"}]}]}]},
            {"type": "tableRow", "content": [{"type": "tableCell", "content": [{"type": "paragraph", "content": [{"type": "text", "text": "second"}]}]}]}
        ]});
        assert_eq!(super::table(&table, 0, 0), "|  |\n| --- |\n| first |\n| second |");
    }

    #[test]
    fn numeric_issue_id_is_preserved_without_weakening_key_authority() {
        let fields = issue_fields(&json!({"id":"10007","key":"OPS-7","fields":{}}));
        assert!(fields.iter().any(|field| field.key == "issue_id"
            && field.value == super::FrontmatterValue::String("10007".into())));
        let fields = issue_fields(&json!({"id":"not an id","fields":{}}));
        assert!(!fields.iter().any(|field| field.key == "issue_id"));
    }

}
