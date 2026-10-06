//! Jira attachment identity checks; shared transport owns auth, redirects and bytes.

use cap_std::fs::Dir;
use cockpit_core::InspectionError;
use cockpit_core::sources::AttachmentRef;
use serde_json::Value;
use url::Url;

use crate::site_http::SiteHttp;

const MAX_METADATA_BYTES: usize = 256 * 1024;

/// Metadata is always on Jira's v2 endpoint (Cloud and Data Center).
/// A supplied id must match exactly. An omitted id requires a byte path that
/// identifies this attachment, and every content link must be on the site.
pub(crate) async fn download_issue_attachment(
    http: &SiteHttp,
    attachment: &AttachmentRef,
    cap: u64,
    dest: &Dir,
) -> Result<(), InspectionError> {
    let value = http
        .get_json(
            http.endpoint(&["rest", "api", "2", "attachment", &attachment.id], &[]),
            MAX_METADATA_BYTES,
        )
        .await?;
    let content = value
        .get("content")
        .and_then(Value::as_str)
        .and_then(|content| Url::parse(content).ok())
        .filter(|content| http.on_site(content))
        .ok_or_else(|| {
            InspectionError::new(
                "source_provider_contract",
                "Jira's attachment link is not on the configured site",
            )
        })?;
    // Never fall back to path matching when Jira supplied an invalid id.
    let matches = match value.get("id") {
        Some(Value::String(listed)) => listed == &attachment.id,
        Some(Value::Number(listed)) => listed.to_string() == attachment.id,
        None => content_identifies_attachment(http.site(), &content, &attachment.id),
        _ => false,
    };
    if !matches {
        return Err(InspectionError::new(
            "source_provider_contract",
            "Jira returned an unexpected attachment record",
        ));
    }
    let size = value.get("size").and_then(Value::as_u64);
    let expected = match (size, attachment.bytes) {
        (Some(reported), Some(listed)) if reported != listed => return Err(size_error()),
        (reported, listed) => reported.or(listed),
    };
    if expected.is_some_and(|size| size > cap) {
        return Err(size_error());
    }
    http.download(content, cap, expected, dest)
        .await
        .map_err(Into::into)
}

fn content_identifies_attachment(site: &Url, content: &Url, id: &str) -> bool {
    let base = site.path().trim_end_matches('/');
    let Some(path) = content.path().strip_prefix(base) else {
        return false;
    };
    if let Some(path) = path.strip_prefix("/secure/attachment/") {
        return path
            .split_once('/')
            .is_some_and(|(listed, file)| listed == id && !file.is_empty());
    }
    path.strip_prefix("/rest/api/")
        .and_then(|path| path.split_once('/'))
        .is_some_and(|(version, path)| {
            !version.is_empty() && path.strip_prefix("attachment/content/") == Some(id)
        })
}

fn size_error() -> InspectionError {
    InspectionError::new(
        "source_attachment_size",
        "Attachment size differs from Jira's record or exceeds the limit",
    )
}

#[cfg(test)]
mod tests;
