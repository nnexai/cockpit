//! Attachments of a Jira work item: metadata from its raw API record, and
//! bytes fetched natively.
//!
//! jira-cli 1.7.0 has no attachment command, so the listing is what
//! `fields.attachment[]` reports (name, size, media type, content link).
//! Bytes are downloaded only on request, by Cockpit's own HTTP client with
//! the token stored in its OS vault (`download`); with no stored token they
//! stay "not requested".

mod download;

pub(crate) use download::{AttachmentDownloader, DOWNLOADED_NAME};

use cockpit_core::sources::SourceAttachment;
use cockpit_protocol::projects::ProjectDiagnostic;
use serde_json::Value;
use url::Url;

const MAX_ATTACHMENTS: usize = 256;
const MAX_NAME_BYTES: usize = 255;
const NOT_REQUESTED: &str = "not_requested";

/// The listed attachments and whether every entry could be listed.
pub(crate) fn issue_attachments(issue: &Value, site: &Url) -> (Vec<SourceAttachment>, bool) {
    let Some(list) = issue
        .get("fields")
        .and_then(|fields| fields.get("attachment"))
        .and_then(Value::as_array)
    else {
        return (Vec::new(), true);
    };
    let attachments: Vec<_> = list
        .iter()
        .filter_map(|item| attachment(item, site))
        .take(MAX_ATTACHMENTS)
        .collect();
    let complete = attachments.len() == list.len();
    (attachments, complete)
}

pub(crate) fn partial_diagnostic() -> ProjectDiagnostic {
    ProjectDiagnostic {
        code: "source_attachments_partial".into(),
        message: format!(
            "Jira attachments are listed only when well-formed, up to {MAX_ATTACHMENTS}"
        ),
        path: None,
    }
}

/// Ids are interpolated into a request path, so only short alphanumerics pass.
pub(crate) fn valid_attachment_id(id: &str) -> bool {
    !id.is_empty() && id.len() <= 64 && id.bytes().all(|byte| byte.is_ascii_alphanumeric())
}

fn attachment(item: &Value, site: &Url) -> Option<SourceAttachment> {
    let id = match item.get("id")? {
        Value::String(id) => id.clone(),
        Value::Number(id) => id.to_string(),
        _ => return None,
    };
    if !valid_attachment_id(&id) {
        return None;
    }
    let title = item.get("filename")?.as_str()?;
    if title.is_empty()
        || title.len() > MAX_NAME_BYTES
        || title.chars().any(char::is_control)
    {
        return None;
    }
    Some(SourceAttachment {
        id,
        title: title.into(),
        media_type: item
            .get("mimeType")
            .and_then(Value::as_str)
            .filter(|value| !value.is_empty() && value.len() <= 255 && !value.chars().any(char::is_control))
            .map(str::to_owned),
        size: item
            .get("size")
            .and_then(Value::as_u64)
            .filter(|size| *size > 0),
        source_url: item
            .get("content")
            .and_then(Value::as_str)
            .filter(|url| same_site(url, site))
            .map(str::to_owned),
        source_revision: None,
        path: None,
        not_downloaded: Some(NOT_REQUESTED.into()),
    })
}

fn same_site(value: &str, site: &Url) -> bool {
    Url::parse(value).is_ok_and(|url| {
        url.username().is_empty()
            && url.password().is_none()
            && url.scheme() == site.scheme()
            && url.host_str() == site.host_str()
            && url.port_or_known_default() == site.port_or_known_default()
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn site() -> Url {
        Url::parse("https://jira.example.test/jira").unwrap()
    }

    #[test]
    fn lists_data_center_attachment_metadata() {
        let issue = json!({"fields": {"attachment": [
            {"id": "10001", "filename": "trace.log", "size": 2048, "mimeType": "text/plain",
             "content": "https://jira.example.test/jira/secure/attachment/10001/trace.log"},
            {"id": 10002, "filename": "empty.bin", "size": 0,
             "content": "https://elsewhere.test/secure/attachment/10002/empty.bin"}
        ]}});
        let (list, complete) = issue_attachments(&issue, &site());
        assert!(complete);
        assert_eq!(list.len(), 2);
        assert_eq!(list[0].id, "10001");
        assert_eq!(list[0].title, "trace.log");
        assert_eq!(list[0].size, Some(2048));
        assert_eq!(list[0].media_type.as_deref(), Some("text/plain"));
        assert!(list[0].source_url.is_some());
        assert_eq!(list[0].not_downloaded.as_deref(), Some("not_requested"));
        assert!(list[0].path.is_none());
        assert_eq!(list[1].id, "10002");
        assert_eq!(list[1].size, None);
        assert_eq!(list[1].source_url, None, "foreign hosts are not kept as links");
    }

    #[test]
    fn malformed_entries_are_dropped_and_reported_partial() {
        let issue = json!({"fields": {"attachment": [
            {"id": "1", "filename": "ok.txt"},
            {"id": "2", "filename": "bad\nname"},
            {"id": "../3", "filename": "x"},
            {"filename": "no-id"},
        ]}});
        let (list, complete) = issue_attachments(&issue, &site());
        assert_eq!(list.iter().map(|a| a.id.as_str()).collect::<Vec<_>>(), ["1"]);
        assert!(!complete);
    }

    #[test]
    fn absent_or_empty_attachment_field_is_complete_and_empty() {
        assert_eq!(issue_attachments(&json!({"fields": {}}), &site()), (Vec::new(), true));
        assert_eq!(
            issue_attachments(&json!({"fields": {"attachment": []}}), &site()),
            (Vec::new(), true)
        );
    }

    #[test]
    fn more_than_the_limit_is_truncated_and_reported_partial() {
        let many: Vec<_> = (0..MAX_ATTACHMENTS + 1)
            .map(|index| json!({"id": index.to_string(), "filename": format!("f{index}")}))
            .collect();
        let (list, complete) = issue_attachments(&json!({"fields": {"attachment": many}}), &site());
        assert_eq!(list.len(), MAX_ATTACHMENTS);
        assert!(!complete);
    }
}
