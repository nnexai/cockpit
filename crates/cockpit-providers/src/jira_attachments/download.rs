//! Native attachment download for Jira. jira-cli 1.7.0 cannot fetch bytes,
//! so Cockpit asks the configured site itself with the token stored in its
//! OS vault, and only for one attachment at a time.
//!
//! The rules (D10): metadata comes from `{site}/rest/api/2/attachment/{id}`;
//! a supplied metadata id must match exactly, while an omitted id requires
//! the content path to identify the requested attachment. Its `content` link
//! must stay on the configured origin and base path; the `Authorization`
//! header goes only to the configured origin, on every hop;
//! redirects are followed by hand, up to [`MAX_REDIRECTS`], never from https
//! to http, and a cross-origin hop (Cloud's signed media host) carries no
//! credential. Every error is a fixed message: `reqwest::Error` text holds the
//! URL, and a media URL's query is a bearer secret.

use std::time::Duration;

use cockpit_core::InspectionError;
use cockpit_core::sources::AttachmentRef;
use reqwest::header::{ACCEPT, AUTHORIZATION, HeaderValue, LOCATION};
use reqwest::{Client, Response, StatusCode, redirect};
use serde_json::Value;
use tokio::io::AsyncWriteExt;
use tokio::sync::OnceCell;
use url::Url;

use cap_fs_ext::{FollowSymlinks, OpenOptionsFollowExt};
use cap_std::fs::{Dir, OpenOptions, OpenOptionsExt};

/// The file name inside the download directory; the core picks the stored name.
pub(crate) const DOWNLOADED_NAME: &str = "download";

const MAX_REDIRECTS: usize = 3;
const MAX_METADATA_BYTES: u64 = 256 * 1024;

/// One HTTP client per provider, built on first use so a machine without a
/// usable trust store still reads issues through the CLI.
#[derive(Debug)]
pub(crate) struct AttachmentDownloader {
    site: Url,
    timeout: Duration,
    client: OnceCell<Client>,
}

impl AttachmentDownloader {
    pub(crate) fn new(site: Url, timeout: Duration) -> Self {
        Self {
            site,
            timeout,
            client: OnceCell::new(),
        }
    }

    /// Write the attachment's bytes to `dest/download`. `cap` is the byte
    /// allowance for this attachment. On any error no file is left behind.
    pub(crate) async fn download(
        &self,
        authorization: &str,
        attachment: &AttachmentRef,
        cap: u64,
        dest: &Dir,
    ) -> Result<(), InspectionError> {
        let mut authorization = HeaderValue::from_str(authorization).map_err(|_| {
            InspectionError::new(
                "source_auth_failed",
                "The token stored in Cockpit cannot be sent as an HTTP header",
            )
        })?;
        authorization.set_sensitive(true);
        let client = self
            .client
            .get_or_try_init(|| async {
                Client::builder()
                    .redirect(redirect::Policy::none())
                    .timeout(self.timeout)
                    .build()
                    .map_err(|_| {
                        InspectionError::new(
                            "source_provider_failed",
                            "Cockpit could not set up HTTPS for the attachment download",
                        )
                    })
            })
            .await?;

        let metadata_url = format!(
            "{}/rest/api/2/attachment/{}",
            self.site.as_str().trim_end_matches('/'),
            attachment.id
        );
        let metadata_url = Url::parse(&metadata_url).map_err(|_| contract())?;
        let response = self
            .get(client, metadata_url, &authorization, "application/json")
            .await?;
        let metadata = self.metadata(response, &attachment.id).await?;
        let expected = match (metadata.size, attachment.bytes) {
            (Some(reported), Some(listed)) if reported != listed => return Err(size_error()),
            (reported, listed) => reported.or(listed),
        };
        if expected.is_some_and(|size| size > cap) {
            return Err(size_error());
        }

        let response = self
            .get(client, metadata.content, &authorization, "*/*")
            .await?;
        save(response, dest, cap, expected).await
    }

    /// One GET, following redirects by hand under the origin rules.
    async fn get(
        &self,
        client: &Client,
        start: Url,
        authorization: &HeaderValue,
        accept: &str,
    ) -> Result<Response, InspectionError> {
        let mut url = start;
        for followed in 0..=MAX_REDIRECTS {
            let mut request = client.get(url.clone()).header(ACCEPT, accept);
            if same_origin(&url, &self.site) {
                request = request.header(AUTHORIZATION, authorization.clone());
            }
            let response = request.send().await.map_err(transport_error)?;
            let status = response.status();
            if status.is_success() {
                return Ok(response);
            }
            if !is_followed_redirect(status) {
                return Err(status_error(status));
            }
            if followed == MAX_REDIRECTS {
                return Err(redirect_refused(
                    "Jira redirected the download too many times",
                ));
            }
            let location = response
                .headers()
                .get(LOCATION)
                .and_then(|value| value.to_str().ok())
                .ok_or_else(|| {
                    redirect_refused("Jira redirected the download without a location")
                })?;
            url = next_hop(&url, location)?;
        }
        Err(contract())
    }

    async fn metadata(
        &self,
        mut response: Response,
        id: &str,
    ) -> Result<Metadata, InspectionError> {
        if response
            .content_length()
            .is_some_and(|length| length > MAX_METADATA_BYTES)
        {
            return Err(contract());
        }
        let mut body = Vec::new();
        while let Some(chunk) = response.chunk().await.map_err(transport_error)? {
            if body.len() as u64 + chunk.len() as u64 > MAX_METADATA_BYTES {
                return Err(contract());
            }
            body.extend_from_slice(&chunk);
        }
        let value: Value = serde_json::from_slice(&body).map_err(|_| contract())?;
        let content = value
            .get("content")
            .and_then(Value::as_str)
            .and_then(|content| Url::parse(content).ok())
            .filter(|content| self.on_site(content))
            .ok_or_else(|| {
                InspectionError::new(
                    "source_provider_contract",
                    "Jira's attachment link is not on the configured site",
                )
            })?;
        // Data Center may omit the id in this endpoint's response. An invalid
        // or mismatched supplied id must never fall back to path matching.
        let matches = match value.get("id") {
            Some(Value::String(listed)) => listed == id,
            Some(Value::Number(listed)) => listed.to_string() == id,
            None => self.content_identifies_attachment(&content, id),
            _ => false,
        };
        if !matches {
            return Err(contract());
        }
        Ok(Metadata {
            size: value.get("size").and_then(Value::as_u64),
            content,
        })
    }

    /// Same origin as the site, and below its base path.
    fn on_site(&self, url: &Url) -> bool {
        let base = self.site.path().trim_end_matches('/');
        same_origin(url, &self.site)
            && url.username().is_empty()
            && url.password().is_none()
            && (url.path() == base || url.path().starts_with(&format!("{base}/")))
    }

    /// Only Jira's attachment-byte paths provide identity when metadata omits it.
    /// `content` has already passed the configured origin and base-path checks.
    fn content_identifies_attachment(&self, content: &Url, id: &str) -> bool {
        let base = self.site.path().trim_end_matches('/');
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
}

struct Metadata {
    size: Option<u64>,
    content: Url,
}

/// Stream the body into a new 0600 no-follow file. The partial file is
/// removed on every error.
async fn save(
    mut response: Response,
    dest: &Dir,
    cap: u64,
    expected: Option<u64>,
) -> Result<(), InspectionError> {
    let limit = expected.map_or(cap, |size| size.min(cap));
    if let Some(length) = response.content_length()
        && (length > limit || expected.is_some_and(|size| length != size))
    {
        return Err(size_error());
    }
    let mut options = OpenOptions::new();
    options
        .write(true)
        .create_new(true)
        .follow(FollowSymlinks::No)
        .mode(0o600);
    let file = dest
        .open_with(DOWNLOADED_NAME, &options)
        .map_err(|_| write_error())?;
    let mut file = tokio::fs::File::from_std(file.into_std());
    let result = async {
        let mut written = 0u64;
        while let Some(chunk) = response.chunk().await.map_err(transport_error)? {
            written += chunk.len() as u64;
            if written > limit {
                return Err(size_error());
            }
            file.write_all(&chunk).await.map_err(|_| write_error())?;
        }
        if expected.is_some_and(|size| written != size) {
            return Err(size_error());
        }
        file.flush().await.map_err(|_| write_error())
    }
    .await;
    drop(file);
    if result.is_err() {
        let _ = dest.remove_file(DOWNLOADED_NAME);
    }
    result
}

/// Scheme, host and effective port; an origin is what a credential is
/// scoped to.
pub(crate) fn same_origin(url: &Url, site: &Url) -> bool {
    url.scheme() == site.scheme()
        && url.host_str() == site.host_str()
        && url.port_or_known_default() == site.port_or_known_default()
}

/// The next URL of a redirect from `current`: resolved against it, http(s)
/// only, credential-free, never https to http.
pub(crate) fn next_hop(current: &Url, location: &str) -> Result<Url, InspectionError> {
    let next = current
        .join(location)
        .map_err(|_| redirect_refused("Jira redirected the download to an invalid address"))?;
    if !matches!(next.scheme(), "http" | "https")
        || next.host_str().is_none()
        || !next.username().is_empty()
        || next.password().is_some()
    {
        return Err(redirect_refused(
            "Jira redirected the download to an unsupported address",
        ));
    }
    if current.scheme() == "https" && next.scheme() == "http" {
        return Err(redirect_refused(
            "Jira redirected the download from https to http",
        ));
    }
    Ok(next)
}

fn is_followed_redirect(status: StatusCode) -> bool {
    matches!(status.as_u16(), 301 | 302 | 303 | 307 | 308)
}

fn status_error(status: StatusCode) -> InspectionError {
    match status.as_u16() {
        401 | 403 => InspectionError::new(
            "source_auth_failed",
            "Jira rejected the token stored in Cockpit for this site",
        ),
        404 | 410 => InspectionError::new("source_not_found", "Jira has no such attachment"),
        _ => InspectionError::new(
            "source_provider_failed",
            "Jira answered the attachment request with an error",
        ),
    }
}

fn transport_error(error: reqwest::Error) -> InspectionError {
    if error.is_timeout() {
        InspectionError::new(
            "source_provider_timeout",
            "Jira attachment request exceeded the configured deadline",
        )
    } else {
        InspectionError::new(
            "source_provider_failed",
            "Jira could not be reached for the attachment download",
        )
    }
}

fn redirect_refused(message: &'static str) -> InspectionError {
    InspectionError::new("source_provider_contract", message)
}

fn contract() -> InspectionError {
    InspectionError::new(
        "source_provider_contract",
        "Jira returned an unexpected attachment record",
    )
}

fn size_error() -> InspectionError {
    InspectionError::new(
        "source_attachment_size",
        "Attachment size differs from Jira's record or exceeds the limit",
    )
}

fn write_error() -> InspectionError {
    InspectionError::new(
        "source_attachment_failed",
        "Attachment could not be written to the download directory",
    )
}

#[cfg(test)]
mod tests;
