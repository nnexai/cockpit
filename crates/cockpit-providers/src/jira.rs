use std::collections::BTreeSet;
use std::ffi::OsString;
use std::path::Path;
use std::process::Output;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use async_trait::async_trait;
use cockpit_core::InspectionError;
use cockpit_core::jira_query::{
    format_wall_minute, instant_seconds, is_normalized_jql, wall_minute,
};
use cockpit_core::process::run_bounded_command;
use cockpit_core::repositories::{is_jira_key, resolve_jira_url};
use cockpit_core::sources::{
    FrontmatterField, FrontmatterValue, IssueListing, IssueQuery, IssueRow, SourceAsset,
    SourceContainer, SourceFetchRequest, SourceMetadata, SourceProvider, SourceRef,
};
use cockpit_protocol::projects::{ProjectConfiguration, ProjectDiagnostic, ProjectProvider};
use cockpit_protocol::sources::SourceCapability;
use serde_json::Value;
use tokio::process::Command;
use url::Url;

const MAX_ISSUE_BYTES: usize = 1024 * 1024;
const MAX_DOCUMENT_DEPTH: usize = 48;

pub(crate) fn executable(value: &str) -> bool {
    Path::new(value)
        .file_name()
        .is_some_and(|name| name == "jira")
}

/// Read-only Jira work items through the owner's configured `jira` CLI
/// (ankitpokhrel/jira-cli). The CLI owns the site login and token; Cockpit
/// only asks for one work item's raw API record and checks that the answer
/// came from the configured site.
#[derive(Debug)]
pub struct JiraSourceProvider {
    provider: ProjectProvider,
    base_url: Url,
    limits: (usize, Duration),
}

impl JiraSourceProvider {
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
                    "Jira provider is not configured",
                )
            })?;
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
        Ok(Self {
            provider: provider.clone(),
            base_url,
            limits: (
                (configuration.limits.git_output_bytes as usize).max(64 * 1024),
                Duration::from_millis(configuration.limits.operation_timeout_ms as u64),
            ),
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

    async fn run(&self, call: &JiraCall<'_>) -> Result<Output, InspectionError> {
        let mut command = Command::new(&self.provider.executable);
        command
            .args(jira_args(call)?)
            .env("NO_COLOR", "1")
            .env("TERM", "dumb");
        run_bounded_command(
            command,
            self.limits.0,
            self.limits.0,
            self.limits.1,
            "Jira source",
        )
        .await
        .map_err(|error| match error.code.as_str() {
            "execution_timeout" => InspectionError::new(
                "source_provider_timeout",
                "Jira CLI request exceeded the configured deadline",
            ),
            "bounded_output" => InspectionError::new(
                "source_truncated",
                "Jira CLI response exceeded Cockpit's explicit process limit",
            ),
            "execution_failed" => {
                InspectionError::new("source_cli_unavailable", "Jira CLI could not be started")
            }
            _ => InspectionError::new("source_provider_failed", "Jira CLI request failed"),
        })
    }

    /// One `issue list` call: at most `limit` rows, newest first. The CLI's
    /// "No result found" failure is an empty answer, not an error.
    async fn run_list(&self, jql: &str, limit: u32) -> Result<Vec<IssueRow>, ListFailure> {
        let output = self.run(&JiraCall::List { jql, limit }).await?;
        if output.status.success() {
            return Ok(parse_issue_rows(&output.stdout, limit)?);
        }
        if String::from_utf8_lossy(&output.stderr).contains(NO_RESULT) {
            return Ok(Vec::new());
        }
        let error = classify_failure(&output.stderr);
        let rejected = error.code == "source_not_found"
            || String::from_utf8_lossy(&output.stderr).contains("400 Bad Request");
        Err(ListFailure { error, rejected })
    }

    /// Newest-first windowed listing of `jql`. Paging is by narrowing
    /// `updated`, never by offset (the CLI ignores `from` on Cloud): each
    /// window is the query with `updated < <oldest minute seen + 1 min>`.
    async fn list_jql(
        &self,
        jql: &str,
        updated_since: Option<&str>,
        max: u32,
        cancel: &AtomicBool,
    ) -> Result<IssueListing, InspectionError> {
        if !is_normalized_jql(jql) || jql.len() > 2048 {
            return Err(list_contract("Jira query is not a normalized query"));
        }
        let since = match updated_since {
            Some(since) => {
                if instant_seconds(since).is_none() {
                    return Err(list_contract("Jira probe time is not an ISO instant"));
                }
                let minute = wall_minute(since)
                    .ok_or_else(|| list_contract("Jira probe time is not an ISO instant"))?;
                Some(format_wall_minute(minute - 1))
            }
            None => None,
        };
        let mut rows: Vec<IssueRow> = Vec::new();
        let mut seen = BTreeSet::new();
        let mut upper: Option<String> = None;
        let complete = loop {
            if cancel.load(Ordering::Relaxed) {
                break false;
            }
            let mut body = if since.is_some() || upper.is_some() {
                format!("({jql})")
            } else {
                jql.to_owned()
            };
            if let Some(since) = &since {
                body.push_str(&format!(" AND updated >= \"{since}\""));
            }
            if let Some(upper) = &upper {
                body.push_str(&format!(" AND updated < \"{upper}\""));
            }
            let window = self.run_list(&body, PAGE).await.map_err(|failure| failure.error)?;
            let full = window.len() == PAGE as usize;
            let oldest = window
                .iter()
                .filter_map(|row| wall_minute(&row.updated))
                .min();
            let fresh = unique_new(window, &mut seen);
            let progressed = !fresh.is_empty();
            rows.extend(fresh);
            if rows.len() > max as usize {
                rows.truncate(max as usize);
                break false;
            }
            if !full {
                break true;
            }
            // A full window with nothing new means more than a page of
            // issues share one minute, which windowing cannot split.
            let Some(oldest) = oldest.filter(|_| progressed) else {
                break false;
            };
            upper = Some(format_wall_minute(oldest + 1));
        };
        Ok(IssueListing { rows, complete })
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
            match self.run_list(&jql, part.len() as u32).await {
                Ok(found) => {
                    // Moved issues answer under a new key; ignore them.
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

    async fn issue(&self, key: &str) -> Result<Value, InspectionError> {
        let output = self.run(&JiraCall::View { key }).await?;
        if !output.status.success() {
            return Err(classify_failure(&output.stderr));
        }
        let value: Value = serde_json::from_slice(&output.stdout).map_err(|_| {
            InspectionError::new(
                "source_provider_contract",
                "Jira CLI did not return a JSON work item",
            )
        })?;
        if value.get("key").and_then(Value::as_str) != Some(key) {
            return Err(InspectionError::new(
                "source_identity_mismatch",
                "Jira returned a different work item",
            ));
        }
        // The CLI's own site configuration decides where it connects; the
        // record's API URL proves the answer came from the configured site.
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
                "Jira CLI is connected to a different site than the configured provider",
            ));
        }
        Ok(value)
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

    fn capabilities(&self) -> Vec<SourceCapability> {
        vec![SourceCapability::Issue, SourceCapability::IssueComments]
    }

    async fn metadata(
        &self,
        request: &SourceFetchRequest,
    ) -> Result<SourceMetadata, InspectionError> {
        let key = self.key(request)?;
        let issue = self.issue(&key).await?;
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
        let issue = self.issue(&key).await?;
        let title = summary(&issue)?;
        let source_url = self.browse_url(&key);
        let (body, complete) = issue_markdown(&issue, &source_url)?;
        let mut diagnostics = if complete {
            Vec::new()
        } else {
            vec![ProjectDiagnostic {
                code: "source_comments_partial".into(),
                message: "Jira returned only the most recent comments".into(),
                path: None,
            }]
        };
        if has_wiki_markup(&issue) {
            diagnostics.push(ProjectDiagnostic {
                code: "source_markup_unconverted".into(),
                message: "Jira returned wiki markup; shown unconverted".into(),
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
            attachments: Vec::new(),
        }])
    }

    async fn list_issues(
        &self,
        query: &IssueQuery<'_>,
        max: u32,
        cancel: &AtomicBool,
    ) -> Result<IssueListing, InspectionError> {
        match query {
            IssueQuery::Jql { jql, updated_since } => {
                self.list_jql(jql, *updated_since, max, cancel).await
            }
            IssueQuery::Keys(keys) => self.list_keys(keys, max, cancel).await,
        }
    }
}

fn classify_failure(stderr: &[u8]) -> InspectionError {
    let text = String::from_utf8_lossy(stderr).to_ascii_lowercase();
    if text.contains("400 bad request") {
        InspectionError::new(
            "source_provider_failed",
            "Jira rejected the request (400); check the query",
        )
    } else if text.contains("404") || text.contains("does not exist") {
        InspectionError::new(
            "source_not_found",
            "Jira work item does not exist or is not visible to the configured login",
        )
    } else if text.contains("401")
        || text.contains("403")
        || text.contains("unauthorized")
        || text.contains("token")
        || text.contains("config")
    {
        InspectionError::new(
            "source_auth_required",
            "Jira CLI is not logged in to this site; run `jira init`",
        )
    } else {
        InspectionError::new("source_provider_failed", "Jira CLI request failed")
    }
}

const NO_RESULT: &str = "No result found for given query";
const LIST_COLUMNS: &str = "key,updated,status,type,assignee";
const LIST_DELIMITER: char = '\u{1f}';
const GUARD_PREFIX: &str = "project IS NOT EMPTY AND (";
/// The CLI answers at most this many rows per call.
const PAGE: u32 = 100;
const MAX_LIST_JQL_BYTES: usize = 4096;

/// The only CLI calls Cockpit makes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum JiraCall<'a> {
    View { key: &'a str },
    /// `jql` is the query body; the builder wraps it in the guard group
    /// `project IS NOT EMPTY AND (<jql>)` that keeps the CLI from adding its
    /// configured default project. `limit` is 1..=100.
    List { jql: &'a str, limit: u32 },
}

fn refused(message: &str) -> InspectionError {
    InspectionError::new(
        "source_provider_contract",
        format!("refused Jira CLI argv: {message}"),
    )
}

/// Render one typed call as the argv after the executable. Invalid values are
/// refused here, before any process exists.
pub fn jira_args(call: &JiraCall) -> Result<Vec<OsString>, InspectionError> {
    let strings: Vec<String> = match call {
        JiraCall::View { key } => {
            if !is_jira_key(key) {
                return Err(refused("Jira work item key is malformed"));
            }
            vec!["issue".into(), "view".into(), (*key).into(), "--raw".into()]
        }
        JiraCall::List { jql, limit } => {
            if !(1..=PAGE).contains(limit) {
                return Err(refused("Jira list limit must be 1–100"));
            }
            vec![
                "issue".into(),
                "list".into(),
                "--jql".into(),
                format!("{GUARD_PREFIX}{jql})"),
                "--plain".into(),
                "--no-headers".into(),
                "--columns".into(),
                LIST_COLUMNS.into(),
                "--delimiter".into(),
                LIST_DELIMITER.to_string(),
                "--order-by".into(),
                "updated".into(),
                "--paginate".into(),
                format!("0:{limit}"),
            ]
        }
    };
    let argv: Vec<OsString> = strings.into_iter().map(OsString::from).collect();
    allowlisted_argv(&argv)?;
    Ok(argv)
}

fn guarded_jql_allowed(value: &str) -> bool {
    value.len() <= MAX_LIST_JQL_BYTES
        && is_normalized_jql(value)
        && value
            .strip_prefix(GUARD_PREFIX)
            .and_then(|rest| rest.strip_suffix(')'))
            .is_some_and(|inner| !inner.is_empty() && is_normalized_jql(inner))
}

/// The exhaustive allowlist, applied to the complete argv right before spawn.
/// Anything the typed builder cannot produce is refused.
pub fn allowlisted_argv(argv: &[OsString]) -> Result<(), InspectionError> {
    let mut strings = Vec::with_capacity(argv.len());
    for arg in argv {
        let Some(text) = arg.to_str() else {
            return Err(refused("non-UTF-8 argument"));
        };
        if text.contains('\0') {
            return Err(refused("NUL in argument"));
        }
        strings.push(text);
    }
    let delimiter = LIST_DELIMITER.to_string();
    let allowed = match strings.as_slice() {
        ["issue", "view", key, "--raw"] => is_jira_key(key),
        [
            "issue",
            "list",
            "--jql",
            jql,
            "--plain",
            "--no-headers",
            "--columns",
            LIST_COLUMNS,
            "--delimiter",
            separator,
            "--order-by",
            "updated",
            "--paginate",
            paginate,
        ] => {
            *separator == delimiter
                && guarded_jql_allowed(jql)
                && paginate
                    .strip_prefix("0:")
                    .and_then(|limit| {
                        limit.parse::<u32>().ok().filter(|parsed| {
                            (1..=PAGE).contains(parsed) && parsed.to_string() == limit
                        })
                    })
                    .is_some()
        }
        _ => false,
    };
    if allowed {
        Ok(())
    } else {
        Err(refused("not an allowlisted read call"))
    }
}

/// `YYYY-MM-DD HH:MM:SS`, the CLI's plain `updated` format.
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

/// Parse the plain listing: one line per issue, five cells split on the
/// unit separator. Blank lines are skipped; any other malformed row rejects
/// the whole answer rather than guessing.
fn parse_issue_rows(stdout: &[u8], limit: u32) -> Result<Vec<IssueRow>, InspectionError> {
    let text = std::str::from_utf8(stdout)
        .map_err(|_| list_contract("Jira CLI listing is not UTF-8"))?;
    let mut rows = Vec::new();
    for line in text.lines().map(|line| line.trim_end_matches('\r')) {
        if line.is_empty() {
            continue;
        }
        let cells: Vec<&str> = line.split(LIST_DELIMITER).collect();
        let [key, updated, status, issue_type, assignee] = cells.as_slice() else {
            return Err(list_contract("Jira CLI listing row does not have 5 cells"));
        };
        let (status, issue_type, assignee) = (status.trim(), issue_type.trim(), assignee.trim());
        if !is_jira_key(key)
            || !listed_updated(updated)
            || status.is_empty()
            || issue_type.is_empty()
        {
            return Err(list_contract("Jira CLI listing row is malformed"));
        }
        rows.push(IssueRow {
            key: (*key).into(),
            updated: (*updated).into(),
            status: status.into(),
            issue_type: issue_type.into(),
            assignee: (!assignee.is_empty()).then(|| assignee.into()),
        });
    }
    if rows.len() > limit as usize {
        return Err(list_contract("Jira CLI listing exceeded the requested limit"));
    }
    Ok(rows)
}

/// A failed `issue list` call. `rejected` means Jira refused the query
/// itself (400 or not found), which is what a key batch bisects on.
struct ListFailure {
    error: InspectionError,
    rejected: bool,
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

fn has_wiki_markup(issue: &Value) -> bool {
    issue
        .pointer("/fields/description")
        .is_some_and(Value::is_string)
        || issue
            .pointer("/fields/comment/comments")
            .and_then(Value::as_array)
            .is_some_and(|comments| {
                comments
                    .iter()
                    .any(|comment| comment.get("body").is_some_and(Value::is_string))
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
    fields
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
        Value::String(text) => text.clone(),
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
    use super::{classify_failure, document_markdown_at, issue_fields, issue_markdown};
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
    fn classifies_missing_items_and_logins() {
        assert_eq!(
            classify_failure(b"Issue does not exist ... 404 Not Found").code,
            "source_not_found"
        );
        assert_eq!(
            classify_failure(b"401 Unauthorized").code,
            "source_auth_required"
        );
        assert_eq!(classify_failure(b"boom").code, "source_provider_failed");
    }

    fn argv(items: &[&str]) -> Vec<std::ffi::OsString> {
        items.iter().map(std::ffi::OsString::from).collect()
    }

    #[test]
    fn list_rows_split_on_the_separator_and_bad_rows_reject_the_answer() {
        let good = "OPS-2\x1f2026-09-28 11:28:44\x1fIn Progress\x1fStory\x1fAnn Lee\n\
                    OPS-1\x1f2026-09-24 18:41:22\x1fTo Do\x1fTask\x1f\n\n";
        let rows = super::parse_issue_rows(good.as_bytes(), 100).unwrap();
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0].assignee.as_deref(), Some("Ann Lee"));
        assert_eq!(rows[0].status, "In Progress");
        assert_eq!(rows[1].assignee, None);
        for bad in [
            "OPS-1\x1f2026-09-24 18:41:22\x1fTo Do\x1fTask\n",
            "ops-1\x1f2026-09-24 18:41:22\x1fTo Do\x1fTask\x1f\n",
            "OPS-1\x1f2026-09-24T18:41:22\x1fTo Do\x1fTask\x1f\n",
            "OPS-1\x1f2026-02-30 18:41:22\x1fTo Do\x1fTask\x1f\n",
            "OPS-1\x1f2026-09-24 18:41:22\x1f\x1fTask\x1f\n",
            "OPS-1\x1f2026-09-24 18:41:22\x1fTo Do\x1fTask\x1fa\x1fb\n",
        ] {
            assert!(super::parse_issue_rows(bad.as_bytes(), 100).is_err(), "{bad:?}");
        }
        assert!(super::parse_issue_rows(good.as_bytes(), 1).is_err());
    }

    #[test]
    fn only_builder_argv_passes_the_allowlist() {
        use super::{JiraCall, allowlisted_argv, jira_args};
        let view = jira_args(&JiraCall::View { key: "OPS-7" }).unwrap();
        assert_eq!(view, argv(&["issue", "view", "OPS-7", "--raw"]));
        let list = jira_args(&JiraCall::List { jql: "project = OPS", limit: 100 }).unwrap();
        assert_eq!(list[3], "project IS NOT EMPTY AND (project = OPS)");
        assert_eq!(list.last().unwrap(), "0:100");
        assert!(jira_args(&JiraCall::List { jql: "project = OPS", limit: 101 }).is_err());
        assert!(jira_args(&JiraCall::List { jql: "a) OR (b", limit: 5 }).is_err());
        assert!(jira_args(&JiraCall::View { key: "--raw" }).is_err());

        let mut without_guard = list.clone();
        without_guard[3] = "project = OPS".into();
        let mut reordered = list.clone();
        reordered.swap(4, 5);
        let mut other_delimiter = list.clone();
        other_delimiter[9] = ",".into();
        let mut extra = list.clone();
        extra.push("--raw".into());
        for refused in [
            without_guard,
            reordered,
            other_delimiter,
            extra,
            argv(&["issue", "delete", "OPS-1"]),
            argv(&["issue", "view", "OPS-1"]),
            argv(&["me"]),
        ] {
            assert!(allowlisted_argv(&refused).is_err(), "{refused:?}");
        }
    }
}
