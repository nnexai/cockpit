//! Credential-scoped, GET-only HTTP transport for Atlassian providers.
//! Error text never includes response bodies, addresses or signed media URLs.

use std::sync::Arc;
use std::sync::atomic::{AtomicI64, Ordering};
use std::time::Duration;

use cap_fs_ext::{FollowSymlinks, OpenOptionsFollowExt};
use cap_std::fs::{Dir, OpenOptions, OpenOptionsExt};
use cockpit_core::InspectionError;
use cockpit_core::sources::lane::{self, RequestLane};
use cockpit_core::credentials::ProviderCredentials;
use reqwest::header::{ACCEPT, AUTHORIZATION, CONTENT_TYPE, HeaderValue, LOCATION};
use reqwest::{Client, Response, StatusCode, redirect};
use tokio::io::AsyncWriteExt;
use tokio::sync::OnceCell;
use url::Url;

use crate::CredentialHandle;

mod pacing;

struct PacedResponse {
    response: Response,
    _permit: pacing::Permit,
}

impl std::ops::Deref for PacedResponse {
    type Target = Response;
    fn deref(&self) -> &Response {
        &self.response
    }
}

impl std::ops::DerefMut for PacedResponse {
    fn deref_mut(&mut self) -> &mut Response {
        &mut self.response
    }
}

pub(crate) const MAX_JSON_BYTES: usize = 4 * 1024 * 1024;
pub(crate) const DOWNLOADED_NAME: &str = "download";
pub(crate) const MAX_REDIRECTS: usize = 3;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Service {
    Jira,
    Confluence,
}

impl Service {
    pub(crate) fn name(self) -> &'static str {
        match self {
            Self::Jira => "Jira",
            Self::Confluence => "Confluence",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum FailureKind {
    Rejected,
    Auth,
    NotFound,
    RateLimited,
    Timeout,
    Unreachable,
    Status,
    Contract,
    TooLarge,
    Write,
}

#[derive(Debug)]
pub(crate) struct HttpFailure {
    pub(crate) kind: FailureKind,
    pub(crate) error: InspectionError,
    /// Kept separate from the fixed error text for endpoint-specific 403 policy.
    pub(crate) status: Option<StatusCode>,
}

impl From<HttpFailure> for InspectionError {
    fn from(failure: HttpFailure) -> Self {
        failure.error
    }
}

#[derive(Debug)]
pub(crate) struct SiteHttp {
    site: Url,
    provider_id: String,
    service: Service,
    timeout: Duration,
    credentials: CredentialHandle,
    client: OnceCell<Client>,
    pacer: Arc<pacing::Pacer>,
    observed_block_ms: AtomicI64,
}

impl SiteHttp {
    pub(crate) fn new(
        site: Url,
        provider_id: &str,
        service: Service,
        timeout: Duration,
        credentials: Arc<ProviderCredentials>,
    ) -> Self {
        Self {
            pacer: pacing::origin(&site),
            observed_block_ms: AtomicI64::new(0),
            site,
            provider_id: provider_id.into(),
            service,
            timeout,
            credentials: CredentialHandle(credentials),
            client: OnceCell::new(),
        }
    }

    pub(crate) fn site(&self) -> &Url {
        &self.site
    }

    /// Shared cooldown/budget deadline for the scheduler, never response text.
    pub(crate) fn blocked_until_ms(&self) -> Option<i64> {
        let observed = self.observed_block_ms.load(Ordering::Relaxed);
        let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap_or_default().as_millis();
        self.pacer.blocked_until_ms().into_iter()
            .chain((observed > 0 && observed as u128 > now).then_some(observed)).max()
    }

    /// Remaining shared-origin rolling-hour budget under the scoped sync policy.
    /// Observation only: the transport still authoritatively charges each hop.
    pub(crate) fn remaining_background_requests(&self) -> u32 {
        self.pacer.remaining_requests(lane::background_policy())
    }

    /// Capability preflight without sending a request or retaining a token.
    pub(crate) async fn require_credentials(&self) -> Result<(), InspectionError> {
        self.credentials
            .0
            .required(&self.provider_id)
            .await
            .map(|_| ())
    }

    pub(crate) fn endpoint(&self, segments: &[&str], query: &[(&str, &str)]) -> Url {
        let mut url = self.site.clone();
        url.set_query(None);
        url.set_fragment(None);
        {
            let mut path = url
                .path_segments_mut()
                .expect("configured HTTP site has path segments");
            path.pop_if_empty();
            path.extend(segments.iter().copied());
        }
        if !query.is_empty() {
            url.query_pairs_mut().extend_pairs(query.iter().copied());
        }
        url
    }

    pub(crate) fn on_site(&self, url: &Url) -> bool {
        let base = self.site.path().trim_end_matches('/');
        same_origin(url, &self.site)
            && valid_address(url)
            && (url.path() == base
                || url
                    .path()
                    .strip_prefix(base)
                    .is_some_and(|rest| rest.starts_with('/')))
    }

    pub(crate) fn link(&self, link: &str) -> Result<Url, HttpFailure> {
        if link.chars().any(char::is_control) || link.contains('\\') {
            return Err(self.contract("returned an invalid download link"));
        }
        let url = if let Ok(url) = Url::parse(link) {
            url
        } else if link.starts_with('/') && !link.starts_with("//") {
            let base = self.site.path().trim_end_matches('/');
            let path = link.split(['?', '#']).next().unwrap_or(link);
            let rooted = path == base
                || path
                    .strip_prefix(base)
                    .is_some_and(|rest| rest.starts_with('/'));
            let resolved = if rooted {
                link.to_owned()
            } else {
                format!("{base}{link}")
            };
            self.site
                .join(&resolved)
                .map_err(|_| self.contract("returned an invalid download link"))?
        } else {
            return Err(self.contract("returned an invalid download link"));
        };
        if !self.on_site(&url) {
            return Err(self.contract("returned a download link outside the configured site"));
        }
        Ok(url)
    }

    pub(crate) async fn get_json(
        &self,
        url: Url,
        max_bytes: usize,
    ) -> Result<serde_json::Value, HttpFailure> {
        let mut response = self.get(url, true).await?;
        let is_json = response
            .headers()
            .get(CONTENT_TYPE)
            .and_then(|value| value.to_str().ok())
            .and_then(|value| value.split(';').next())
            .is_some_and(|mime| {
                let mime = mime.trim();
                mime.eq_ignore_ascii_case("application/json")
                    || (mime
                        .get(..12)
                        .is_some_and(|prefix| prefix.eq_ignore_ascii_case("application/"))
                        && mime
                            .rsplit_once('+')
                            .is_some_and(|(_, suffix)| suffix.eq_ignore_ascii_case("json")))
            });
        if !is_json {
            return Err(self.contract("returned a non-JSON response"));
        }
        if response
            .content_length()
            .is_some_and(|length| length > max_bytes as u64)
        {
            return Err(self.json_size_error());
        }
        let mut body = Vec::new();
        while let Some(chunk) = response
            .chunk()
            .await
            .map_err(|error| self.transport_error(error))?
        {
            if chunk.len() > max_bytes.saturating_sub(body.len()) {
                return Err(self.json_size_error());
            }
            body.extend_from_slice(&chunk);
        }
        serde_json::from_slice(&body).map_err(|_| self.contract("returned invalid JSON"))
    }

    pub(crate) async fn download(
        &self,
        url: Url,
        cap: u64,
        expected: Option<u64>,
        dest: &Dir,
    ) -> Result<(), HttpFailure> {
        if expected.is_some_and(|size| size > cap) {
            return Err(self.size_error());
        }
        let response = self.get(url, false).await?;
        self.save(response, dest, cap, expected).await
    }

    async fn get(&self, start: Url, json: bool) -> Result<PacedResponse, HttpFailure> {
        if !self.on_site(&start) {
            return Err(self.contract("request is outside the configured site"));
        }
        let client = self
            .client
            .get_or_try_init(|| async {
                Client::builder()
                    .redirect(redirect::Policy::none())
                    .timeout(self.timeout)
                    .build()
                    .map_err(|_| {
                        self.failure(
                            FailureKind::Unreachable,
                            "source_provider_failed",
                            "could not initialize HTTPS",
                        )
                    })
            })
            .await?;
        let mut url = start;
        for followed in 0..=MAX_REDIRECTS {
            // Read each time: replacing the stored credential takes effect on the next hop.
            let pacer = if same_origin(&url, &self.site) {
                self.pacer.clone()
            } else {
                pacing::origin(&url)
            };
            let mut throttled_retries = 0;
            let mut transport_retried = false;
            let response = loop {
                let credential = self
                    .credentials
                    .0
                    .required(&self.provider_id)
                    .await
                    .map_err(|error| HttpFailure {
                        kind: FailureKind::Auth,
                        error,
                        status: None,
                    })?;
                let mut authorization =
                    HeaderValue::from_str(&credential.authorization()).map_err(|_| HttpFailure {
                        kind: FailureKind::Auth,
                        status: None,
                        error: InspectionError::new(
                            "source_auth_failed",
                            "The token stored in Cockpit cannot be sent as an HTTP header",
                        ),
                    })?;
                authorization.set_sensitive(true);
                let permit = pacer.acquire(lane::current(), lane::background_policy()).await
                    .map_err(|()| {
                        if let Some(until) = pacer.blocked_until_ms() {
                            self.observed_block_ms.fetch_max(until, Ordering::Relaxed);
                        }
                        self.failure(FailureKind::RateLimited, "source_rate_limited", "is temporarily blocked by the shared request budget or cooldown")
                    })?;
                let mut request = client
                    .get(url.clone())
                    .header(ACCEPT, if json { "application/json" } else { "*/*" });
                if same_origin(&url, &self.site) {
                    request = request.header(AUTHORIZATION, authorization);
                }
                let response = match request.send().await {
                    Ok(response) => response,
                    Err(error) => {
                        drop(permit);
                        if lane::current() == RequestLane::Background && !transport_retried
                            && (error.is_timeout() || error.is_connect() || error.is_request())
                        {
                            transport_retried = true;
                            tokio::time::sleep(Duration::from_secs(1)).await;
                            continue;
                        }
                        return Err(self.transport_error(error));
                    }
                };
                pacer.observe(response.headers());
                if matches!(response.status().as_u16(), 429 | 503) {
                    let wait = pacer.throttle(response.headers(), throttled_retries);
                    if let Some(until) = pacer.blocked_until_ms() {
                        self.observed_block_ms.fetch_max(until, Ordering::Relaxed);
                    }
                    let failure = self.status_error(response.status());
                    drop(response);
                    drop(permit);
                    if throttled_retries >= 3 || wait > pacing::MAX_RETRY_WAIT {
                        return Err(failure);
                    }
                    throttled_retries += 1;
                    continue;
                }
                break PacedResponse { response, _permit: permit };
            };
            let status = response.status();
            if status.is_success() {
                if json && !self.on_site(response.url()) {
                    return Err(self.contract("answered outside the configured site"));
                }
                return Ok(response);
            }
            if !matches!(status.as_u16(), 301 | 302 | 303 | 307 | 308) {
                return Err(self.status_error(status));
            }
            if followed == MAX_REDIRECTS {
                return Err(self.contract("redirected the request too many times"));
            }
            let location = response
                .headers()
                .get(LOCATION)
                .and_then(|value| value.to_str().ok())
                .ok_or_else(|| self.contract("redirected the request without a location"))?;
            let next = self.next_hop(&url, location)?;
            if json && !self.on_site(&next) {
                return Err(self.contract("redirected JSON outside the configured site"));
            }
            url = next;
        }
        Err(self.contract("returned an unexpected response"))
    }

    fn next_hop(&self, current: &Url, location: &str) -> Result<Url, HttpFailure> {
        let next = current
            .join(location)
            .map_err(|_| self.contract("redirected to an invalid address"))?;
        if !valid_address(&next) {
            return Err(self.contract("redirected to an unsupported address"));
        }
        if current.scheme() == "https" && next.scheme() == "http" {
            return Err(self.contract("redirected from https to http"));
        }
        Ok(next)
    }

    async fn save(
        &self,
        mut response: PacedResponse,
        dest: &Dir,
        cap: u64,
        expected: Option<u64>,
    ) -> Result<(), HttpFailure> {
        let limit = expected.map_or(cap, |size| size.min(cap));
        if response
            .content_length()
            .is_some_and(|length| length > limit || expected.is_some_and(|size| length != size))
        {
            return Err(self.size_error());
        }
        let mut options = OpenOptions::new();
        options
            .write(true)
            .create_new(true)
            .follow(FollowSymlinks::No)
            .mode(0o600);
        let file = dest
            .open_with(DOWNLOADED_NAME, &options)
            .map_err(|_| self.write_error())?;
        let mut file = tokio::fs::File::from_std(file.into_std());
        let result = async {
            let mut written = 0u64;
            while let Some(chunk) = response
                .chunk()
                .await
                .map_err(|error| self.transport_error(error))?
            {
                if chunk.len() as u64 > limit.saturating_sub(written) {
                    return Err(self.size_error());
                }
                written += chunk.len() as u64;
                file.write_all(&chunk)
                    .await
                    .map_err(|_| self.write_error())?;
            }
            if expected.is_some_and(|size| written != size) {
                return Err(self.size_error());
            }
            file.flush().await.map_err(|_| self.write_error())
        }
        .await;
        drop(file);
        if result.is_err() {
            let _ = dest.remove_file(DOWNLOADED_NAME);
        }
        result
    }

    fn failure(&self, kind: FailureKind, code: &str, message: &str) -> HttpFailure {
        HttpFailure {
            kind,
            error: InspectionError::new(code, format!("{} {message}", self.service.name())),
            status: None,
        }
    }

    fn contract(&self, message: &str) -> HttpFailure {
        self.failure(FailureKind::Contract, "source_provider_contract", message)
    }
    fn json_size_error(&self) -> HttpFailure {
        self.failure(
            FailureKind::TooLarge,
            "source_truncated",
            "response exceeds the JSON byte limit",
        )
    }
    fn size_error(&self) -> HttpFailure {
        self.failure(
            FailureKind::TooLarge,
            "source_attachment_size",
            "attachment size differs from the record or exceeds the limit",
        )
    }
    fn write_error(&self) -> HttpFailure {
        self.failure(
            FailureKind::Write,
            "source_attachment_failed",
            "attachment could not be written to the download directory",
        )
    }

    fn status_error(&self, status: StatusCode) -> HttpFailure {
        let (kind, code, message) = match status.as_u16() {
            400 => (
                FailureKind::Rejected,
                "source_provider_failed",
                "rejected the request",
            ),
            401 | 403 => (
                FailureKind::Auth,
                "source_auth_failed",
                "rejected the token stored in Cockpit for this site",
            ),
            404 | 410 => (
                FailureKind::NotFound,
                "source_not_found",
                "has no such item, or the stored token cannot see it",
            ),
            429 | 503 => (
                FailureKind::RateLimited,
                "source_rate_limited",
                "rate limited the request",
            ),
            _ => (
                FailureKind::Status,
                "source_provider_failed",
                "answered the request with an error",
            ),
        };
        HttpFailure {
            status: Some(status),
            ..self.failure(kind, code, message)
        }
    }

    fn transport_error(&self, error: reqwest::Error) -> HttpFailure {
        if error.is_timeout() {
            self.failure(
                FailureKind::Timeout,
                "source_provider_timeout",
                "request exceeded the configured deadline",
            )
        } else {
            self.failure(
                FailureKind::Unreachable,
                "source_provider_failed",
                "could not be reached",
            )
        }
    }
}

fn valid_address(url: &Url) -> bool {
    matches!(url.scheme(), "http" | "https")
        && url.host_str().is_some()
        && url.username().is_empty()
        && url.password().is_none()
}

fn same_origin(url: &Url, site: &Url) -> bool {
    url.scheme() == site.scheme()
        && url
            .host_str()
            .zip(site.host_str())
            .is_some_and(|(host, site)| host.eq_ignore_ascii_case(site))
        && url.port_or_known_default() == site.port_or_known_default()
}

#[cfg(test)]
#[path = "site_http/test_server.rs"]
pub(crate) mod test_server;
#[cfg(test)]
mod tests;
