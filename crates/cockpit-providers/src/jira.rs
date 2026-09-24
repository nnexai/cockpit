use std::path::Path;
use std::time::Duration;

use async_trait::async_trait;
use cockpit_core::InspectionError;
use cockpit_core::process::run_bounded_command;
use cockpit_core::repositories::resolve_jira_url;
use cockpit_core::sources::{
    SourceAsset, SourceFetchRequest, SourceMetadata, SourceProvider, SourceRef,
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

    async fn issue(&self, key: &str) -> Result<Value, InspectionError> {
        let mut command = Command::new(&self.provider.executable);
        command
            .args(["issue", "view", key, "--raw"])
            .env("NO_COLOR", "1")
            .env("TERM", "dumb");
        let output = run_bounded_command(
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
        })?;
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
        let (body, complete) = issue_markdown(&issue)?;
        let diagnostics = if complete {
            Vec::new()
        } else {
            vec![ProjectDiagnostic {
                code: "source_comments_partial".into(),
                message: "Jira returned only the most recent comments".into(),
                path: None,
            }]
        };
        Ok(vec![SourceAsset {
            source: SourceRef {
                provider_id: self.provider.id.clone(),
                provider_instance: request.authority.provider_instance.clone(),
                resource_type: "issue".into(),
                canonical_id: key.clone(),
            },
            title,
            source_url: Some(self.browse_url(&key)),
            original_url: None,
            source_revision: field_str(&issue, "updated").map(str::to_owned),
            complete,
            diagnostics,
            body,
        }])
    }
}

fn classify_failure(stderr: &[u8]) -> InspectionError {
    let text = String::from_utf8_lossy(stderr).to_ascii_lowercase();
    if text.contains("404") || text.contains("does not exist") {
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

/// Render the work item and its comments as Markdown. Returns whether every
/// comment Jira reported was included.
fn issue_markdown(issue: &Value) -> Result<(String, bool), InspectionError> {
    let mut body = String::new();
    let facts = [
        ("Type", field_name(issue, "issuetype", "name")),
        ("Status", field_name(issue, "status", "name")),
        ("Priority", field_name(issue, "priority", "name")),
        ("Assignee", field_name(issue, "assignee", "displayName")),
        ("Reporter", field_name(issue, "reporter", "displayName")),
        ("Created", field_str(issue, "created")),
        ("Updated", field_str(issue, "updated")),
    ];
    for (label, value) in facts {
        if let Some(value) = value {
            append_bounded(&mut body, &format!("{label}: {value}\n"))?;
        }
    }
    if let Some(description) = issue
        .get("fields")
        .and_then(|fields| fields.get("description"))
    {
        let text = document_markdown(description);
        if !text.is_empty() {
            append_bounded(&mut body, &format!("\n{text}\n"))?;
        }
    }
    let comments = issue.get("fields").and_then(|fields| fields.get("comment"));
    let list = comments
        .and_then(|comment| comment.get("comments"))
        .and_then(Value::as_array)
        .map(Vec::as_slice)
        .unwrap_or_default();
    for comment in list {
        let id = comment.get("id").and_then(Value::as_str).ok_or_else(|| {
            InspectionError::new("source_provider_contract", "Jira comment has no ID")
        })?;
        append_bounded(&mut body, &format!("\n## Jira comment {id}\n"))?;
        let author = comment
            .get("author")
            .and_then(|author| author.get("displayName"))
            .and_then(Value::as_str);
        for (label, value) in [
            ("Author", author),
            ("Created", comment.get("created").and_then(Value::as_str)),
            ("Updated", comment.get("updated").and_then(Value::as_str)),
        ] {
            if let Some(value) = value {
                append_bounded(&mut body, &format!("{label}: {value}\n"))?;
            }
        }
        if let Some(text) = comment.get("body") {
            append_bounded(&mut body, &format!("\n{}\n", document_markdown(text)))?;
        }
    }
    let total = comments
        .and_then(|comment| comment.get("total"))
        .and_then(Value::as_u64)
        .unwrap_or(list.len() as u64);
    Ok((body, total <= list.len() as u64))
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

/// Convert an Atlassian Document Format value (or legacy plain text) to
/// Markdown. Unknown nodes keep their text; media becomes a placeholder.
pub(crate) fn document_markdown(value: &Value) -> String {
    match value {
        Value::String(text) => text.trim().to_owned(),
        Value::Object(_) => blocks(children(value), 0),
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

fn blocks(nodes: &[Value], depth: usize) -> String {
    joined_blocks(nodes, depth, "\n\n")
}

fn joined_blocks(nodes: &[Value], depth: usize, separator: &str) -> String {
    nodes
        .iter()
        .map(|node| block(node, depth))
        .filter(|text| !text.is_empty())
        .collect::<Vec<_>>()
        .join(separator)
}

fn block(node: &Value, depth: usize) -> String {
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
            format!("{} {}", "#".repeat(level), inline(children(node), next))
        }
        "bulletList" => list(node, next, |_| "- ".into()),
        "orderedList" => {
            let start = attr(node, "order").and_then(Value::as_u64).unwrap_or(1);
            list(node, next, |index| format!("{}. ", start + index as u64))
        }
        "taskList" | "decisionList" => list(node, next, |_| String::new()),
        "taskItem" | "decisionItem" => {
            let done = attr(node, "state").and_then(Value::as_str) == Some("DONE");
            format!(
                "- [{}] {}",
                if done { "x" } else { " " },
                inline(children(node), next)
            )
        }
        "codeBlock" => {
            let language = attr(node, "language")
                .and_then(Value::as_str)
                .unwrap_or_default();
            let text: String = children(node)
                .iter()
                .filter_map(|child| child.get("text").and_then(Value::as_str))
                .collect();
            format!("```{language}\n{text}\n```")
        }
        "blockquote" | "panel" => blocks(children(node), next)
            .lines()
            .map(|line| format!("> {line}").trim_end().to_owned())
            .collect::<Vec<_>>()
            .join("\n"),
        "rule" => "---".into(),
        "mediaSingle" | "mediaGroup" | "media" => "[attachment]".into(),
        "table" => table(node, next),
        "expand" | "nestedExpand" => {
            let title = attr(node, "title")
                .and_then(Value::as_str)
                .unwrap_or_default();
            let content = blocks(children(node), next);
            if title.is_empty() {
                content
            } else {
                format!("**{title}**\n\n{content}")
            }
        }
        "blockCard" | "embedCard" => attr(node, "url")
            .and_then(Value::as_str)
            .map(|url| format!("<{url}>"))
            .unwrap_or_default(),
        _ if node.get("content").is_some() => {
            let content = children(node);
            if content.iter().all(is_inline) {
                inline(content, next)
            } else {
                blocks(content, next)
            }
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

fn list(node: &Value, depth: usize, marker: impl Fn(usize) -> String) -> String {
    children(node)
        .iter()
        .enumerate()
        .map(|(index, item)| {
            let content = if item.get("type").and_then(Value::as_str) == Some("listItem") {
                // Tight lists: an item's paragraph and nested list share lines.
                joined_blocks(children(item), depth, "\n")
            } else {
                block(item, depth)
            };
            let marker = marker(index);
            let indent = " ".repeat(marker.len().max(2));
            let mut lines = content.lines();
            let first = lines.next().unwrap_or_default();
            std::iter::once(format!("{marker}{first}"))
                .chain(lines.map(|line| {
                    if line.is_empty() {
                        String::new()
                    } else {
                        format!("{indent}{line}")
                    }
                }))
                .collect::<Vec<_>>()
                .join("\n")
        })
        .collect::<Vec<_>>()
        .join("\n")
}

fn table(node: &Value, depth: usize) -> String {
    let rows: Vec<Vec<String>> = children(node)
        .iter()
        .map(|row| {
            children(row)
                .iter()
                .map(|cell| {
                    blocks(children(cell), depth)
                        .replace('\n', " ")
                        .replace('|', "\\|")
                })
                .collect()
        })
        .collect();
    let columns = rows.iter().map(Vec::len).max().unwrap_or(0);
    if columns == 0 {
        return String::new();
    }
    let render = |row: &Vec<String>| {
        let mut cells = row.clone();
        cells.resize(columns, String::new());
        format!("| {} |", cells.join(" | "))
    };
    let mut lines = Vec::new();
    for (index, row) in rows.iter().enumerate() {
        lines.push(render(row));
        if index == 0 {
            lines.push(format!("|{}", " --- |".repeat(columns)));
        }
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
    use super::{classify_failure, document_markdown, issue_markdown};
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
            document_markdown(&document),
            "## Acceptance\n\nUse `retry` and see [docs](https://example.test/d)\n@Ann\n\n- One\n- Two\n  1. Nested\n\n```rust\nfn main() {}\n```\n\n[attachment]"
        );
    }

    #[test]
    fn renders_facts_description_and_comments_and_reports_partial_comments() {
        let issue = json!({"key": "SCRUM-5", "self": "https://site.test/rest/api/3/issue/1", "fields": {
            "summary": "Fix login",
            "issuetype": {"name": "Task"}, "status": {"name": "To Do"},
            "updated": "2026-09-24T18:44:53.898+0200",
            "description": {"type": "doc", "content": [{"type": "paragraph", "content": [{"type": "text", "text": "Body"}]}]},
            "comment": {"total": 2, "comments": [{"id": "10001", "author": {"displayName": "Ann"}, "created": "c",
                "body": {"type": "doc", "content": [{"type": "paragraph", "content": [{"type": "text", "text": "Seen"}]}]}}]}
        }});
        let (body, complete) = issue_markdown(&issue).unwrap();
        assert_eq!(
            body,
            "Type: Task\nStatus: To Do\nUpdated: 2026-09-24T18:44:53.898+0200\n\nBody\n\n## Jira comment 10001\nAuthor: Ann\nCreated: c\n\nSeen\n"
        );
        assert!(!complete);
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
}
