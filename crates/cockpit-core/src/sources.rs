//! Provider-neutral fetch, metadata and reference traversal; Library owns persistence.

use std::collections::BTreeMap;
use std::sync::Arc;
use std::time::{Duration, Instant};

use async_trait::async_trait;
use cockpit_protocol::projects::{ProjectConfiguration, ProjectDiagnostic};
use cockpit_protocol::sources::SourceCapability;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use tokio::time::timeout;

use crate::InspectionError;
use crate::repositories::is_jira_key;
pub mod references;
pub use references::{
    MAX_ASSET_REFERENCES, MAX_REFERENCE_DEPTH, ReferenceSeed, ReferenceTarget, RelatedAsset,
    RelatedFailure, RelatedResult, SourceReference, TraversalBudget, TraversalStop,
    asset_label, asset_references, references_truncated,
};

const MAX_ASSETS_PER_FETCH: usize = 64;
const MAX_ASSET_BYTES: usize = 1024 * 1024;
const MAX_METADATA_BYTES: usize = 16 * 1024;
const MAX_URL_BYTES: usize = 8 * 1024;
const MAX_LISTED_SPACES: usize = 10_000;
const MAX_METADATA_DESCRIPTION_BYTES: usize = 256 * 1024;
/// Setup reads one artifact while planning, checking before start and
/// importing, usually within seconds. It may reuse a provider result this
/// recent; explicit imports and refreshes always ask the provider.
const SETUP_REUSE_WINDOW: Duration = Duration::from_secs(120);
const MAX_RECENT_READS: usize = 16;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SourceRef {
    pub provider_id: String,
    /// Credential-free base URL normalized from the selected configured instance.
    pub provider_instance: String,
    pub resource_type: String,
    pub canonical_id: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SourceContainer {
    pub id: String,
    pub label: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum FrontmatterValue {
    Null,
    String(String),
    Number(i64),
    Boolean(bool),
    Strings(Vec<String>),
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FrontmatterField {
    pub key: String,
    pub value: FrontmatterValue,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SourceAttachment {
    pub id: String,
    pub title: String,
    pub media_type: Option<String>,
    pub size: Option<u64>,
    pub source_url: Option<String>,
    pub source_revision: Option<String>,
    pub path: Option<String>,
    pub not_downloaded: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SourceAsset {
    pub source: SourceRef,
    pub title: String,
    /// API-verified canonical URL, when the provider returned one.
    pub source_url: Option<String>,
    /// URL supplied by the user, retained as provenance without becoming identity.
    pub original_url: Option<String>,
    pub source_revision: Option<String>,
    pub complete: bool,
    pub diagnostics: Vec<ProjectDiagnostic>,
    pub body: String,
    #[serde(default)]
    pub container: Option<SourceContainer>,
    #[serde(default)]
    pub fields: Vec<FrontmatterField>,
    #[serde(default)]
    pub attachments: Vec<SourceAttachment>,
}

/// Proof derived from the configured provider and an authorized artifact identity.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SourceAuthority {
    pub provider_instance: String,
    pub origin_host: String,
    pub origin_port: Option<u16>,
    pub origin_base_path: String,
    pub owner: String,
    pub repository: String,
}

#[derive(Debug, Clone)]
pub struct SourceFetchRequest {
    pub provider_id: String,
    pub artifact_url: String,
    pub authority: SourceAuthority,
}

/// Small provider-owned artifact facts used to propose workspace setup values.
/// They are deliberately separate from source import so setup does not cache or
/// materialize an artifact merely to read its title or review source branch.
#[derive(Debug, Clone)]
pub struct SourceMetadata {
    pub title: String,
    pub source_branch: Option<String>,
    pub source_url: Option<String>,
    pub source_commit: Option<String>,
    /// Bounded description text, read only to find linked work items.
    pub description: Option<String>,
}
#[derive(Debug, Clone)]
pub struct FetchedAssets {
    pub assets: Vec<SourceAsset>,
    pub diagnostics: Vec<ProjectDiagnostic>,
}

/// A Confluence page identity proved by the selected provider: `Info` answered
/// for this id from the configured instance.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConfluencePage {
    /// Decimal page id, 1–20 digits.
    pub page_id: String,
    pub space_key: String,
    pub title: String,
    pub version: Option<u64>,
    /// The page URL reported by the CLI, validated against the configured
    /// instance's scheme, host, port and path prefix.
    pub source_url: String,
    /// [`confluence_page_url`] for the provider instance; the fetch URL for
    /// both add and refresh.
    pub canonical_url: String,
}

/// What a provider recognized in a user input (URL, id or key).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ProviderResolution {
    ConfluencePage(ConfluencePage),
    ConfluenceSpace { space_key: String },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SpaceSummary {
    pub key: String,
    pub name: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SpacePage {
    pub page_id: String,
    pub title: String,
    pub version: u64,
    /// Ancestor page/folder ids, root first.
    pub ancestors: Vec<String>,
    pub position: Option<i64>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SpacePageListing {
    pub space_name: String,
    pub homepage_id: Option<String>,
    pub pages: Vec<SpacePage>,
    pub total: Option<u64>,
    pub complete: bool,
}

/// One row of a Jira issue listing. `updated` is the CLI's plain format,
/// `YYYY-MM-DD HH:MM:SS`, which is what a listing is compared against.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IssueRow {
    pub key: String,
    pub updated: String,
    pub status: String,
    pub issue_type: String,
    pub assignee: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IssueListing {
    pub rows: Vec<IssueRow>,
    /// False when the cap was reached or the listing could not be proven
    /// exhaustive; a partial listing must never drop members.
    pub complete: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IssueQuery<'a> {
    /// `updated_since` is a view-format ISO instant (probe lower bound).
    Jql {
        jql: &'a str,
        updated_since: Option<&'a str>,
    },
    Keys(&'a [String]),
}

const MAX_ISSUE_JQL_BYTES: usize = 2048;
const MAX_ISSUE_KEYS: usize = 10_000;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AttachmentRef {
    pub id: String,
    pub title: String,
    pub bytes: Option<u64>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DownloadedAttachment {
    pub attachment_id: String,
    /// One path component inside the download destination.
    pub file_name: String,
}

/// Stable page URL for a Confluence instance, valid on Cloud and Data Center
/// and independent of the page's title, space or parent.
pub fn confluence_page_url(provider_instance: &str, page_id: &str) -> String {
    format!(
        "{}/pages/viewpage.action?pageId={page_id}",
        provider_instance.trim_end_matches('/')
    )
}

/// D21 `--pattern`: the attachment title with every `*`, `?` and leading or
/// trailing whitespace character replaced by `?`, so the CLI glob never widens
/// beyond single-character wildcards.
pub fn confluence_attachment_pattern(title: &str) -> String {
    let whitespace = |c: &char| matches!(*c, '\u{0009}'..='\u{000d}' | '\u{0020}' | '\u{00a0}' | '\u{1680}' | '\u{2000}'..='\u{200a}' | '\u{2028}' | '\u{2029}' | '\u{202f}' | '\u{205f}' | '\u{3000}' | '\u{feff}');
    let leading = title.chars().take_while(whitespace).count();
    let trailing = title.chars().rev().take_while(whitespace).count();
    let count = title.chars().count();
    title
        .chars()
        .enumerate()
        .map(|(index, c)| {
            if matches!(c, '*' | '?') || index < leading || index >= count.saturating_sub(trailing) {
                '?'
            } else {
                c
            }
        })
        .collect()
}

/// The CLI's `globToRegExp` match: anchored, case-insensitive (JavaScript
/// non-Unicode `i` canonicalization), over UTF-16 code units; `?` is one unit
/// and `*` any run.
pub fn confluence_glob_matches(pattern: &str, title: &str) -> bool {
    fn fold(unit: u16) -> u16 {
        let Some(c) = char::from_u32(unit as u32) else {
            return unit;
        };
        let mut upper = c.to_uppercase();
        match (upper.next(), upper.next()) {
            (Some(u), None) if u.len_utf16() == 1 && !(unit >= 128 && (u as u32) < 128) => u as u16,
            _ => unit,
        }
    }
    let pattern: Vec<u16> = pattern.encode_utf16().collect();
    let title: Vec<u16> = title.encode_utf16().map(fold).collect();
    let (mut p, mut t) = (0, 0);
    let mut star: Option<(usize, usize)> = None;
    while t < title.len() {
        if p < pattern.len() && pattern[p] == u16::from(b'*') {
            star = Some((p, t));
            p += 1;
        } else if p < pattern.len()
            && (pattern[p] == u16::from(b'?') || fold(pattern[p]) == title[t])
        {
            p += 1;
            t += 1;
        } else if let Some((star_p, star_t)) = star {
            p = star_p + 1;
            t = star_t + 1;
            star = Some((star_p, star_t + 1));
        } else {
            return false;
        }
    }
    pattern[p..].iter().all(|unit| *unit == u16::from(b'*'))
}

fn capability_unavailable<T>() -> Result<T, InspectionError> {
    Err(InspectionError::new(
        "source_capability_unavailable",
        "selected source provider does not support this operation",
    ))
}

/// Resolve authority from the selected configured instance, never a checkout.
pub fn instance_authority(
    configuration: &ProjectConfiguration,
    provider_id: &str,
    artifact_url: &str,
) -> Result<SourceAuthority, InspectionError> {
    let artifact = crate::repositories::resolve_artifact(configuration, artifact_url)?;
    if artifact.provider_id != provider_id {
        return Err(InspectionError::new(
            "source_authority_mismatch",
            "artifact does not belong to the selected configured provider",
        ));
    }
    let mut authority = site_authority(configuration, provider_id)?;
    if crate::repositories::provider_is_repository_independent(configuration, provider_id) {
        return Ok(authority);
    }
    if artifact.kind == "wiki" {
        let mut parts = artifact.canonical_id.splitn(3, '/');
        let owner = parts.next().unwrap_or("");
        let repository = parts.next().unwrap_or("");
        let page = parts.next().unwrap_or("");
        if owner.is_empty()
            || repository.is_empty()
            || !page.starts_with("wiki/")
            || page == "wiki/"
        {
            return Err(InspectionError::new(
                "source_artifact_invalid",
                "artifact has no repository identity",
            ));
        }
        authority.owner = owner.into();
        authority.repository = repository.into();
        return Ok(authority);
    }
    let repository = match artifact.kind.as_str() {
        "issue" => artifact.canonical_id.rsplit_once('#'),
        "review" => artifact.canonical_id.rsplit_once('!'),
        _ => {
            return Err(InspectionError::new(
                "source_artifact_unsupported",
                "unsupported source kind",
            ));
        }
    }
    .map(|(repository, _)| repository)
    .ok_or_else(|| {
        InspectionError::new(
            "source_artifact_invalid",
            "artifact has no repository identity",
        )
    })?;
    let (owner, repository) = repository.rsplit_once('/').ok_or_else(|| {
        InspectionError::new("source_artifact_invalid", "artifact has no owner identity")
    })?;
    authority.owner = owner.into();
    authority.repository = repository.into();
    Ok(authority)
}
/// Authority for a provider whose work items are not tied to a Git remote.
pub(crate) fn site_authority(
    configuration: &ProjectConfiguration,
    provider_id: &str,
) -> Result<SourceAuthority, InspectionError> {
    let provider = configuration
        .providers
        .iter()
        .find(|provider| provider.id == provider_id)
        .ok_or_else(|| {
            InspectionError::new(
                "source_provider_unsupported",
                "selected source provider is not configured",
            )
        })?;
    let instance = normalized_provider_instance(&provider.base_url)?;
    Ok(SourceAuthority {
        provider_instance: instance.render(),
        origin_host: instance.host,
        origin_port: instance.port,
        origin_base_path: instance.base_path,
        owner: String::new(),
        repository: String::new(),
    })
}

#[derive(Debug)]
struct ProviderInstance {
    scheme: String,
    host: String,
    port: Option<u16>,
    base_path: String,
}

impl ProviderInstance {
    fn render(&self) -> String {
        format!(
            "{}://{}{}{}",
            self.scheme,
            self.host,
            self.port.map(|port| format!(":{port}")).unwrap_or_default(),
            self.base_path
        )
    }
}

fn normalized_provider_instance(value: &str) -> Result<ProviderInstance, InspectionError> {
    let url = url::Url::parse(value).map_err(|_| {
        InspectionError::new(
            "source_provider_invalid",
            "configured provider URL is invalid",
        )
    })?;
    if !matches!(url.scheme(), "http" | "https")
        || !url.username().is_empty()
        || url.password().is_some()
        || url.query().is_some()
        || url.fragment().is_some()
    {
        return Err(InspectionError::new(
            "source_provider_invalid",
            "configured provider URL must be credential-free HTTP(S)",
        ));
    }
    let host = url
        .host_str()
        .ok_or_else(|| {
            InspectionError::new(
                "source_provider_invalid",
                "configured provider URL lacks a host",
            )
        })?
        .to_ascii_lowercase();
    let default_port = match url.scheme() {
        "http" => 80,
        "https" => 443,
        _ => unreachable!(),
    };
    let port = url.port().filter(|port| *port != default_port);
    let path = url.path().trim_end_matches('/');
    let base_path = if path.is_empty() {
        String::new()
    } else {
        path.to_owned()
    };
    Ok(ProviderInstance {
        scheme: url.scheme().to_ascii_lowercase(),
        host,
        port,
        base_path,
    })
}
#[async_trait]
pub trait SourceProvider: Send + Sync {
    fn provider_id(&self) -> &str;
    fn capabilities(&self) -> Vec<SourceCapability>;
    async fn metadata(
        &self,
        _request: &SourceFetchRequest,
    ) -> Result<SourceMetadata, InspectionError> {
        Err(InspectionError::new(
            "source_metadata_unsupported",
            "selected source provider does not expose workspace defaults",
        ))
    }
    async fn fetch(
        &self,
        request: &SourceFetchRequest,
    ) -> Result<Vec<SourceAsset>, InspectionError>;
    /// Recognize a page or space input for this provider instance.
    async fn resolve_input(&self, _input: &str) -> Result<ProviderResolution, InspectionError> {
        capability_unavailable()
    }
    async fn list_spaces(&self) -> Result<Vec<SpaceSummary>, InspectionError> {
        capability_unavailable()
    }
    async fn list_space_pages(
        &self,
        _space_key: &str,
        _max_pages: u32,
        _cancel: &std::sync::atomic::AtomicBool,
    ) -> Result<SpacePageListing, InspectionError> {
        capability_unavailable()
    }
    /// A metadata listing of Jira issues for a query or a set of keys.
    async fn list_issues(
        &self,
        _query: &IssueQuery<'_>,
        _max: u32,
        _cancel: &std::sync::atomic::AtomicBool,
    ) -> Result<IssueListing, InspectionError> {
        capability_unavailable()
    }
    /// The page's current space key; `None` when the page no longer exists.
    async fn page_space(&self, _page_id: &str) -> Result<Option<String>, InspectionError> {
        capability_unavailable()
    }
    /// Whether attachment bytes of `resource_type` can be downloaded now.
    /// Providers refuse by default; `SourceService::attachment_downloads`
    /// is the single gate for callers.
    async fn attachment_downloads(&self, _resource_type: &str) -> Result<(), InspectionError> {
        capability_unavailable()
    }
    async fn download_attachment(
        &self,
        _canonical_id: &str,
        _attachment: &AttachmentRef,
        _siblings: &[AttachmentRef],
        _dest: &cap_std::fs::Dir,
        _dest_path: &std::path::Path,
        _budget: crate::process::StagingBudget,
    ) -> Result<DownloadedAttachment, InspectionError> {
        capability_unavailable()
    }
}
#[derive(Clone)]
pub struct SourceService {
    providers: Arc<Vec<Arc<dyn SourceProvider>>>,
    configuration: Arc<ProjectConfiguration>,
    operation_timeout: Duration,
    recent: Arc<std::sync::Mutex<RecentReads>>,
}

/// Successful provider results kept briefly for setup reuse, newest last.
/// A setup read of one artifact holds its key's gate, so a start that
/// arrives during the background prefetch waits for it instead of asking
/// the provider a second time.
#[derive(Default)]
struct RecentReads {
    fetches: Vec<(String, Instant, Vec<SourceAsset>)>,
    metadata: Vec<(String, Instant, SourceMetadata)>,
    gates: BTreeMap<String, Arc<tokio::sync::Mutex<()>>>,
}

impl RecentReads {
    fn gate(&mut self, key: &str) -> Arc<tokio::sync::Mutex<()>> {
        // Drop gates nobody holds or waits on.
        self.gates.retain(|_, gate| Arc::strong_count(gate) > 1);
        self.gates.entry(key.to_owned()).or_default().clone()
    }
}

fn read_key(request: &SourceFetchRequest) -> String {
    format!(
        "{}\0{}\0{:?}",
        request.provider_id, request.artifact_url, request.authority
    )
}

fn recent_read<T: Clone>(reads: &[(String, Instant, T)], key: &str) -> Option<T> {
    reads
        .iter()
        .rev()
        .find(|(candidate, at, _)| candidate == key && at.elapsed() < SETUP_REUSE_WINDOW)
        .map(|(_, _, value)| value.clone())
}

fn remember_read<T>(reads: &mut Vec<(String, Instant, T)>, key: String, value: T) {
    reads.retain(|(candidate, at, _)| *candidate != key && at.elapsed() < SETUP_REUSE_WINDOW);
    if reads.len() >= MAX_RECENT_READS {
        reads.remove(0);
    }
    reads.push((key, Instant::now(), value));
}

impl SourceService {
    pub fn new(
        configuration: &ProjectConfiguration,
        providers: Vec<Arc<dyn SourceProvider>>,
    ) -> Result<Self, InspectionError> {
        Ok(Self {
            providers: Arc::new(providers),
            configuration: Arc::new(configuration.clone()),
            operation_timeout: Duration::from_millis(
                configuration.limits.operation_timeout_ms.into(),
            ),
            recent: Arc::new(std::sync::Mutex::new(RecentReads::default())),
        })
    }

    /// Validate setup content without writing any persistent state.
    pub async fn validate_artifact_for_setup(
        &self, request: SourceFetchRequest,
    ) -> Result<FetchedAssets, InspectionError> {
        let fetched = self.fetch_assets_reusing(request, true, true).await?;
        if fetched.assets.is_empty() {
            return Err(InspectionError::new("source_provider_contract", "source provider returned no primary artifact"));
        }
        Ok(fetched)
    }

    /// Resolve provider metadata without persistence.
    pub async fn metadata(
        &self,
        request: SourceFetchRequest,
    ) -> Result<SourceMetadata, InspectionError> {
        self.metadata_reusing(request, false).await
    }

    /// Setup metadata, which may reuse a result from the last
    /// [`SETUP_REUSE_WINDOW`].
    pub async fn metadata_for_setup(
        &self,
        request: SourceFetchRequest,
    ) -> Result<SourceMetadata, InspectionError> {
        self.metadata_reusing(request, true).await
    }

    async fn metadata_reusing(
        &self,
        request: SourceFetchRequest,
        reuse_recent: bool,
    ) -> Result<SourceMetadata, InspectionError> {
        validate_request(&request)?;
        let key = read_key(&request);
        if reuse_recent
            && let Some(metadata) = recent_read(
                &self
                    .recent
                    .lock()
                    .unwrap_or_else(|e| e.into_inner())
                    .metadata,
                &key,
            )
        {
            return Ok(metadata);
        }
        let provider = self
            .providers
            .iter()
            .find(|provider| provider.provider_id() == request.provider_id)
            .ok_or_else(|| {
                InspectionError::new(
                    "source_provider_unsupported",
                    "selected source provider is unavailable",
                )
            })?;
        let metadata = timeout(self.operation_timeout, provider.metadata(&request))
            .await
            .map_err(|_| {
                InspectionError::new(
                    "source_metadata_timeout",
                    "source metadata exceeded the configured operation deadline",
                )
            })??;
        let valid_commit = metadata.source_commit.as_deref().is_none_or(|commit| {
            matches!(commit.len(), 40 | 64) && commit.bytes().all(|byte| byte.is_ascii_hexdigit())
        });
        if !bounded_text(&metadata.title, 256)
            || metadata
                .source_branch
                .as_deref()
                .is_some_and(|branch| !bounded_text(branch, 256))
            || metadata
                .source_url
                .as_deref()
                .is_some_and(|url| !bounded_text(url, MAX_URL_BYTES))
            || metadata
                .description
                .as_deref()
                .is_some_and(|description| description.len() > MAX_METADATA_DESCRIPTION_BYTES)
            || !valid_commit
        {
            return Err(InspectionError::new(
                "source_provider_contract",
                "source metadata contains invalid title, branch, URL, or commit text",
            ));
        }
        remember_read(
            &mut self
                .recent
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .metadata,
            key,
            metadata.clone(),
        );
        Ok(metadata)
    }
    /// Read an artifact into the short-lived setup reuse window, without persistence.
    pub async fn prefetch_for_setup(&self, request: SourceFetchRequest) {
        if validate_request(&request).is_err() {
            return;
        }
        let Some(provider) = self
            .providers
            .iter()
            .find(|provider| provider.provider_id() == request.provider_id)
            .cloned()
        else {
            return;
        };
        let key = read_key(&request);
        let gate = self
            .recent
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .gate(&key);
        let _gate = gate.lock().await;
        let recent = recent_read(
            &self
                .recent
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .fetches,
            &key,
        );
        if recent.is_some() {
            return;
        }
        if let Ok(Ok(assets)) = timeout(self.operation_timeout, provider.fetch(&request)).await {
            remember_read(
                &mut self
                    .recent
                    .lock()
                    .unwrap_or_else(|e| e.into_inner())
                    .fetches,
                key,
                assets,
            );
        }
    }
    /// Let the selected provider recognize a page or space input, within the
    /// operation deadline. The result is checked structurally here; callers
    /// still bind a page to its configured instance before fetching.
    pub async fn resolve_input(
        &self,
        provider_id: &str,
        input: &str,
    ) -> Result<ProviderResolution, InspectionError> {
        if !bounded_text(input.trim(), MAX_URL_BYTES) {
            return Err(InspectionError::new(
                "library_input_unrecognized",
                "input must be a bounded page URL, page id or space key",
            ));
        }
        let provider = self
            .providers
            .iter()
            .find(|provider| provider.provider_id() == provider_id)
            .ok_or_else(|| {
                InspectionError::new(
                    "source_provider_unsupported",
                    "selected source provider is unavailable",
                )
            })?;
        let resolution = timeout(self.operation_timeout, provider.resolve_input(input.trim()))
            .await
            .map_err(|_| {
                InspectionError::new(
                    "source_fetch_timeout",
                    "source resolution exceeded the configured operation deadline",
                )
            })??;
        let valid = match &resolution {
            ProviderResolution::ConfluencePage(page) => {
                confluence_page_id(&page.page_id)
                    && confluence_space_key(&page.space_key)
                    && bounded_text(&page.title, MAX_METADATA_BYTES)
                    && bounded_text(&page.source_url, MAX_URL_BYTES)
                    && bounded_text(&page.canonical_url, MAX_URL_BYTES)
            }
            ProviderResolution::ConfluenceSpace { space_key } => confluence_space_key(space_key),
        };
        if !valid {
            return Err(InspectionError::new(
                "source_provider_contract",
                "source provider returned an invalid page or space identity",
            ));
        }
        Ok(resolution)
    }

    fn selected_provider(
        &self,
        provider_id: &str,
    ) -> Result<&Arc<dyn SourceProvider>, InspectionError> {
        self.providers
            .iter()
            .find(|provider| provider.provider_id() == provider_id)
            .ok_or_else(|| {
                InspectionError::new(
                    "source_provider_unsupported",
                    "selected source provider is unavailable",
                )
            })
    }

    /// Spaces readable by the selected provider's profile, structurally checked.
    pub async fn list_spaces(
        &self,
        provider_id: &str,
    ) -> Result<Vec<SpaceSummary>, InspectionError> {
        let provider = self.selected_provider(provider_id)?;
        let spaces = timeout(self.operation_timeout, provider.list_spaces())
            .await
            .map_err(|_| {
                InspectionError::new(
                    "source_fetch_timeout",
                    "space listing exceeded the configured operation deadline",
                )
            })??;
        let mut keys = std::collections::BTreeSet::new();
        if spaces.len() > MAX_LISTED_SPACES
            || spaces.iter().any(|space| {
                !confluence_space_key(&space.key)
                    || !bounded_text(&space.name, MAX_METADATA_BYTES)
                    || !keys.insert(space.key.as_str())
            })
        {
            return Err(InspectionError::new(
                "source_provider_contract",
                "source provider returned an invalid space listing",
            ));
        }
        Ok(spaces)
    }

    /// D20 enumeration of one space. Each page of up to 100 results gets the
    /// operation deadline; the provider stops early when `cancel` is set.
    pub async fn list_space_pages(
        &self,
        provider_id: &str,
        space_key: &str,
        max_pages: u32,
        cancel: &std::sync::atomic::AtomicBool,
    ) -> Result<SpacePageListing, InspectionError> {
        if !confluence_space_key(space_key) || max_pages == 0 {
            return Err(InspectionError::new(
                "source_provider_contract",
                "space enumeration requires a valid space key and page limit",
            ));
        }
        let provider = self.selected_provider(provider_id)?;
        let calls = max_pages.div_ceil(100).saturating_add(2);
        let listing = timeout(
            self.operation_timeout.saturating_mul(calls),
            provider.list_space_pages(space_key, max_pages, cancel),
        )
        .await
        .map_err(|_| {
            InspectionError::new(
                "source_fetch_timeout",
                "space enumeration exceeded the configured operation deadline",
            )
        })??;
        let mut ids = std::collections::BTreeSet::new();
        // The homepage may be appended beyond the limit when the search omitted it.
        let valid = listing.pages.len() <= max_pages as usize + 1
            && bounded_text(&listing.space_name, MAX_METADATA_BYTES)
            && listing
                .homepage_id
                .as_deref()
                .is_none_or(confluence_page_id)
            && listing.pages.iter().all(|page| {
                confluence_page_id(&page.page_id)
                    && ids.insert(page.page_id.as_str())
                    && bounded_text(&page.title, MAX_METADATA_BYTES)
                    && page.ancestors.len() <= 256
                    && page
                        .ancestors
                        .iter()
                        .all(|ancestor| confluence_page_id(ancestor) && *ancestor != page.page_id)
            });
        if !valid {
            return Err(InspectionError::new(
                "source_provider_contract",
                "source provider returned an invalid space enumeration",
            ));
        }
        Ok(listing)
    }

    /// Metadata listing of Jira issues. The deadline scales with the number
    /// of CLI calls the provider may make (one per 100 rows or keys, twice
    /// over for bisection); the provider stops early when `cancel` is set.
    pub async fn list_issues(
        &self,
        provider_id: &str,
        query: &IssueQuery<'_>,
        max: u32,
        cancel: &std::sync::atomic::AtomicBool,
    ) -> Result<IssueListing, InspectionError> {
        let contract = |message: &str| InspectionError::new("source_provider_contract", message);
        let batches = match query {
            IssueQuery::Jql { jql, updated_since } => {
                if jql.trim().is_empty()
                    || !bounded_text(jql, MAX_ISSUE_JQL_BYTES)
                    || updated_since.is_some_and(|since| crate::jira_query::instant_seconds(since).is_none())
                {
                    return Err(contract("issue listing requires a bounded query"));
                }
                max.div_ceil(100)
            }
            IssueQuery::Keys(keys) => {
                if keys.is_empty()
                    || keys.len() > MAX_ISSUE_KEYS
                    || !keys.iter().all(|key| is_jira_key(key))
                {
                    return Err(contract("issue listing requires 1 to 10000 valid issue keys"));
                }
                (keys.len() as u32).div_ceil(100)
            }
        };
        if max == 0 {
            return Err(contract("issue listing requires a positive limit"));
        }
        let provider = self.selected_provider(provider_id)?;
        let calls = batches.saturating_mul(2).saturating_add(2);
        let listing = timeout(
            self.operation_timeout.saturating_mul(calls),
            provider.list_issues(query, max, cancel),
        )
        .await
        .map_err(|_| {
            InspectionError::new(
                "source_fetch_timeout",
                "issue listing exceeded the configured operation deadline",
            )
        })??;
        let mut keys = std::collections::BTreeSet::new();
        let valid = listing.rows.len() <= max as usize
            && listing.rows.iter().all(|row| {
                is_jira_key(&row.key)
                    && keys.insert(row.key.as_str())
                    && issue_updated(&row.updated)
                    && bounded_text(&row.status, MAX_METADATA_BYTES)
                    && bounded_text(&row.issue_type, MAX_METADATA_BYTES)
                    && row
                        .assignee
                        .as_deref()
                        .is_none_or(|name| bounded_text(name, MAX_METADATA_BYTES))
            });
        if !valid {
            return Err(contract("source provider returned an invalid issue listing"));
        }
        Ok(listing)
    }

    /// The page's current space, or `None` when the provider reports it missing.
    pub async fn page_space(
        &self,
        provider_id: &str,
        page_id: &str,
    ) -> Result<Option<String>, InspectionError> {
        if !confluence_page_id(page_id) {
            return Err(InspectionError::new(
                "source_provider_contract",
                "page confirmation requires a valid page id",
            ));
        }
        let provider = self.selected_provider(provider_id)?;
        let space = timeout(self.operation_timeout, provider.page_space(page_id))
            .await
            .map_err(|_| {
                InspectionError::new(
                    "source_fetch_timeout",
                    "page confirmation exceeded the configured operation deadline",
                )
            })??;
        if space.as_deref().is_some_and(|key| !confluence_space_key(key)) {
            return Err(InspectionError::new(
                "source_provider_contract",
                "source provider returned an invalid space key",
            ));
        }
        Ok(space)
    }

    /// Whether the selected provider can download attachments of
    /// `resource_type` now. The provider decides, including credential state.
    pub async fn attachment_downloads(
        &self,
        provider_id: &str,
        resource_type: &str,
    ) -> Result<(), InspectionError> {
        // The provider bounds its own work (the vault has its own deadline), so
        // a vault timeout keeps its precise code.
        self.selected_provider(provider_id)?
            .attachment_downloads(resource_type)
            .await
    }

    /// D21: one attachment through the selected provider into `dest`, a fresh
    /// private directory at `dest_path`. The result must name the requested
    /// attachment and one path component; the caller verifies the file.
    /// `budget` covers all predicted matches, including discarded siblings.
    /// `canonical_id` is a Confluence page id for `"page"` and a Jira key for
    /// `"issue"`; any other resource type is refused.
    pub async fn download_attachment(
        &self,
        provider_id: &str,
        resource_type: &str,
        canonical_id: &str,
        attachment: &AttachmentRef,
        siblings: &[AttachmentRef],
        dest: &cap_std::fs::Dir,
        dest_path: &std::path::Path,
        budget: crate::process::StagingBudget,
    ) -> Result<DownloadedAttachment, InspectionError> {
        let valid = match resource_type {
            "page" => confluence_page_id(canonical_id),
            "issue" => is_jira_key(canonical_id),
            _ => false,
        };
        if !valid {
            return Err(InspectionError::new(
                "source_provider_contract",
                "attachment download requires a valid page id or issue key",
            ));
        }
        let provider = self.selected_provider(provider_id)?;
        let downloaded = timeout(
            self.operation_timeout,
            provider.download_attachment(
                canonical_id,
                attachment,
                siblings,
                dest,
                dest_path,
                budget,
            ),
        )
        .await
        .map_err(|_| {
            InspectionError::new(
                "source_fetch_timeout",
                "attachment download exceeded the configured operation deadline",
            )
        })??;
        let name = std::path::Path::new(&downloaded.file_name);
        if downloaded.attachment_id != attachment.id
            || downloaded.file_name.len() > 255
            || downloaded.file_name.contains(['/', '\\'])
            || name.components().count() != 1
            || !matches!(name.components().next(), Some(std::path::Component::Normal(_)))
        {
            return Err(InspectionError::new(
                "source_capability_unavailable",
                "Attachment download returned an unsafe result",
            ));
        }
        Ok(downloaded)
    }

    /// Fetch and validate provider assets without accessing persistent state.
    pub async fn fetch_assets(
        &self,
        request: SourceFetchRequest,
    ) -> Result<FetchedAssets, InspectionError> {
        self.fetch_assets_reusing(request, false, false).await
    }

    async fn fetch_assets_reusing(
        &self,
        request: SourceFetchRequest,
        reuse_recent: bool,
        remember_recent: bool,
    ) -> Result<FetchedAssets, InspectionError> {
        validate_request(&request)?;
        let provider = self.selected_provider(&request.provider_id)?;
        let key = read_key(&request);
        let gate = reuse_recent.then(|| {
            self.recent
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .gate(&key)
        });
        let _gate = match &gate {
            Some(gate) => Some(gate.lock().await),
            None => None,
        };
        let reused = reuse_recent
            .then(|| {
                recent_read(
                    &self
                        .recent
                        .lock()
                        .unwrap_or_else(|e| e.into_inner())
                        .fetches,
                    &key,
                )
            })
            .flatten();
        let assets = match reused {
            Some(assets) => assets,
            None => {
                let assets = timeout(self.operation_timeout, provider.fetch(&request))
                    .await
                    .map_err(|_| {
                        InspectionError::new(
                            "source_fetch_timeout",
                            "source fetch exceeded the configured operation deadline",
                        )
                    })??;
                if remember_recent {
                    remember_read(
                        &mut self
                            .recent
                            .lock()
                            .unwrap_or_else(|e| e.into_inner())
                            .fetches,
                        key,
                        assets.clone(),
                    );
                }
                assets
            }
        };
        if assets.len() > MAX_ASSETS_PER_FETCH {
            return Err(InspectionError::new(
                "source_provider_contract",
                "provider returned too many source assets",
            ));
        }
        let mut diagnostics = Vec::new();
        for asset in &assets {
            validate_provider_asset(&request, asset)?;
            validate_asset(asset)?;
            diagnostics.extend(asset.diagnostics.iter().cloned());
        }
        validate_confluence_page(&request, &assets)?;
        Ok(FetchedAssets {
            assets,
            diagnostics,
        })
    }
}

fn validate_request(request: &SourceFetchRequest) -> Result<(), InspectionError> {
    if !bounded_text(&request.provider_id, 128)
        || !bounded_text(&request.artifact_url, MAX_URL_BYTES)
        || !bounded_text(&request.authority.provider_instance, 512)
        || !bounded_text(&request.authority.origin_host, 256)
        || (!request.authority.origin_base_path.is_empty()
            && !bounded_text(&request.authority.origin_base_path, 512))
        || !(request.authority.owner.is_empty() && request.authority.repository.is_empty()
            || bounded_text(&request.authority.owner, 256)
                && bounded_text(&request.authority.repository, 256))
    {
        return Err(InspectionError::new(
            "source_authority_invalid",
            "source authority is incomplete or exceeds bounds",
        ));
    }
    Ok(())
}

fn validate_provider_asset(
    request: &SourceFetchRequest,
    asset: &SourceAsset,
) -> Result<(), InspectionError> {
    if asset.source.provider_id != request.provider_id
        || asset.source.provider_instance != request.authority.provider_instance
    {
        return Err(InspectionError::new(
            "source_provider_contract",
            "provider output does not match selected authority",
        ));
    }
    Ok(())
}

/// A provider that answers with a Confluence page answers with exactly the
/// requested page of the requested instance.
fn validate_confluence_page(
    request: &SourceFetchRequest,
    assets: &[SourceAsset],
) -> Result<(), InspectionError> {
    if !assets
        .iter()
        .any(|asset| asset.source.resource_type == "page")
    {
        return Ok(());
    }
    let [asset] = assets else {
        return Err(InspectionError::new(
            "source_provider_contract",
            "a Confluence page fetch returns exactly one page",
        ));
    };
    if !confluence_page_id(&asset.source.canonical_id)
        || request.artifact_url
            != confluence_page_url(&request.authority.provider_instance, &asset.source.canonical_id)
    {
        return Err(InspectionError::new(
            "source_identity_mismatch",
            "provider returned a different Confluence page than requested",
        ));
    }
    Ok(())
}

pub(crate) fn confluence_page_id(value: &str) -> bool {
    (1..=20).contains(&value.len()) && value.bytes().all(|byte| byte.is_ascii_digit())
}

pub(crate) fn confluence_space_key(value: &str) -> bool {
    (1..=255).contains(&value.len())
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'~' | b'_' | b'-'))
}

/// Authority for a Confluence page the selected provider proved, bound to the
/// configured instance. `canonical_url` is the URL the page will be fetched by.
pub(crate) fn confluence_instance_authority(
    configuration: &ProjectConfiguration,
    selected_provider_id: &str,
    page: &ConfluencePage,
    canonical_url: &str,
) -> Result<SourceAuthority, InspectionError> {
    let provider = configuration
        .providers
        .iter()
        .find(|p| p.id == selected_provider_id)
        .ok_or_else(|| {
            InspectionError::new(
                "source_authority_mismatch",
                "selected Confluence provider is not configured",
            )
        })?;
    if !crate::repositories::is_confluence_executable(&provider.executable)
        || !confluence_url_belongs_to_instance(&provider.base_url, &page.source_url)
        || !confluence_page_id(&page.page_id)
        || !confluence_space_key(&page.space_key)
        || page.title.trim().is_empty()
        || page.title.chars().any(char::is_control)
    {
        return Err(InspectionError::new(
            "source_identity_mismatch",
            "Confluence page identity is invalid",
        ));
    }
    let authority = site_authority(configuration, selected_provider_id)?;
    let expected = confluence_page_url(&authority.provider_instance, &page.page_id);
    if page.canonical_url != expected || canonical_url != expected {
        return Err(InspectionError::new(
            "source_identity_mismatch",
            "Confluence page does not belong to the selected provider instance",
        ));
    }
    Ok(authority)
}

pub(crate) fn confluence_url_belongs_to_instance(base_url: &str, source_url: &str) -> bool {
    let (Ok(base), Ok(source)) = (url::Url::parse(base_url), url::Url::parse(source_url)) else {
        return false;
    };
    let base_path = base.path().trim_end_matches('/');
    let source_path = source.path();
    base.scheme() == source.scheme()
        && base.host_str().map(str::to_ascii_lowercase)
            == source.host_str().map(str::to_ascii_lowercase)
        && base.port_or_known_default() == source.port_or_known_default()
        && source.username().is_empty()
        && source.password().is_none()
        && source.query().is_none()
        && source.fragment().is_none()
        && (base_path.is_empty()
            || source_path == base_path
            || source_path.starts_with(&format!("{base_path}/")))
}

fn validate_asset(asset: &SourceAsset) -> Result<(), InspectionError> {
    let diagnostics_valid = asset.diagnostics.len() <= 32
        && asset.diagnostics.iter().all(|diagnostic| {
            bounded_text(&diagnostic.code, 128)
                && bounded_text(&diagnostic.message, MAX_METADATA_BYTES)
                && diagnostic
                    .path
                    .as_deref()
                    .is_none_or(|path| bounded_text(path, MAX_URL_BYTES))
        });
    if !bounded_text(&asset.source.provider_id, 128)
        || !valid_extended_metadata(asset)
        || !bounded_text(&asset.source.provider_instance, 512)
        || !identifier(&asset.source.resource_type, 64)
        || !bounded_text(&asset.source.canonical_id, 512)
        || !bounded_text(&asset.title, MAX_METADATA_BYTES)
        || asset.title.contains(['\n', '\r'])
        || asset
            .source_url
            .as_deref()
            .is_some_and(|value| !bounded_text(value, MAX_URL_BYTES))
        || asset
            .original_url
            .as_deref()
            .is_some_and(|value| !bounded_text(value, MAX_URL_BYTES))
        || asset
            .source_revision
            .as_deref()
            .is_some_and(|value| !bounded_text(value, MAX_METADATA_BYTES))
        || !diagnostics_valid
        || asset.body.len() > MAX_ASSET_BYTES
        || asset.body.contains('\0')
    {
        return Err(InspectionError::new(
            "source_asset_invalid",
            "provider returned incomplete or oversized source asset",
        ));
    }
    Ok(())
}
/// `YYYY-MM-DD HH:MM:SS`, the jira-cli plain listing format.
fn issue_updated(value: &str) -> bool {
    let bytes = value.as_bytes();
    bytes.len() == 19
        && bytes[16] == b':'
        && bytes[10] == b' '
        && bytes[17..].iter().all(u8::is_ascii_digit)
        && (bytes[17] - b'0') <= 5
        && crate::jira_query::wall_minute(value).is_some()
}
fn bounded_text(value: &str, max: usize) -> bool {
    !value.is_empty()
        && value.len() <= max
        && !value.contains('\0')
        && !value.chars().any(char::is_control)
}
fn identifier(value: &str, max: usize) -> bool {
    bounded_text(value, max)
        && value
            .bytes()
            .all(|value| value.is_ascii_alphanumeric() || matches!(value, b'_' | b'-'))
}

const RESERVED_FIELDS: &[&str] = &[
    "schema_version",
    "provider",
    "provider_instance",
    "resource_type",
    "canonical_id",
    "source_url",
    "original_url",
    "complete",
    "fetched_at",
    "source_revision",
    "content_hash",
    "generated",
    "container",
    "attachments",
    "library_item_id",
    "library_revision",
];

fn valid_frontmatter_value(value: &FrontmatterValue) -> bool {
    let valid_text = |text: &str| text.len() <= MAX_METADATA_BYTES && !text.contains('\0');
    let valid = match value {
        FrontmatterValue::Null => true,
        FrontmatterValue::String(text) => valid_text(text),
        FrontmatterValue::Strings(values) => {
            values.len() <= 256 && values.iter().all(|text| valid_text(text))
        }
        FrontmatterValue::Number(_) | FrontmatterValue::Boolean(_) => true,
    };
    valid && serde_json::to_vec(value).is_ok_and(|bytes| bytes.len() <= MAX_METADATA_BYTES)
}

fn valid_extended_metadata(asset: &SourceAsset) -> bool {
    let mut keys = std::collections::BTreeSet::new();
    asset.container.as_ref().is_none_or(|container| {
        bounded_text(&container.id, MAX_METADATA_BYTES)
            && bounded_text(&container.label, MAX_METADATA_BYTES)
    }) && asset.fields.len() <= 32
        && asset.fields.iter().all(|field| {
            !field.key.is_empty()
                && field.key.len() <= 48
                && field
                    .key
                    .bytes()
                    .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'_')
                && !RESERVED_FIELDS.contains(&field.key.as_str())
                && keys.insert(&field.key)
                && valid_frontmatter_value(&field.value)
        })
        && asset.attachments.len() <= 256
        && asset.attachments.iter().all(|attachment| {
            bounded_text(&attachment.id, MAX_METADATA_BYTES)
                && bounded_text(&attachment.title, MAX_METADATA_BYTES)
                && [
                    &attachment.media_type,
                    &attachment.source_revision,
                    &attachment.not_downloaded,
                ]
                .iter()
                .all(|value| {
                    value
                        .as_deref()
                        .is_none_or(|value| bounded_text(value, MAX_METADATA_BYTES))
                })
                && attachment
                    .source_url
                    .as_deref()
                    .is_none_or(|url| bounded_text(url, MAX_URL_BYTES))
                && attachment.path.as_deref().is_none_or(|path| {
                    path.strip_prefix("_files/").is_some_and(|name| {
                        bounded_text(name, 255)
                            && !matches!(name, "." | "..")
                            && !name.contains(['/', '\\'])
                    })
                })
                && !(attachment.path.is_some() && attachment.not_downloaded.is_some())
        })
}

/// Content identity, deliberately independent of provenance and presentation.
pub fn content_revision(asset: &SourceAsset) -> String {
    let mut hash = Sha256::new();
    for value in [
        asset.source.provider_id.as_str(),
        &asset.source.provider_instance,
        &asset.source.resource_type,
        &asset.source.canonical_id,
        &asset.title,
        asset.source_revision.as_deref().unwrap_or(""),
        &asset.body,
    ] {
        hash.update(value.as_bytes());
        hash.update([0]);
    }
    hash.update([u8::from(asset.complete)]);
    if !asset.fields.is_empty() {
        hash.update(b"\0fields\0");
        hash.update(serde_json::to_vec(&asset.fields).expect("field serialization"));
    }
    if !asset.attachments.is_empty() {
        hash.update(b"\0attachments\0");
        for attachment in &asset.attachments {
            // Download state and URLs are provenance, not source content.
            hash.update(
                serde_json::to_vec(&(
                    &attachment.id,
                    &attachment.title,
                    &attachment.media_type,
                    attachment.size,
                    &attachment.source_revision,
                ))
                .expect("attachment serialization"),
            );
            hash.update([0]);
        }
    }
    format!("sha256:{:x}", hash.finalize())
}

/// Stable Library document; the Library adds its own item and revision keys.
pub fn library_markdown(asset: &SourceAsset, revision: &str) -> String {
    let mut markdown = format!(
        "---\nschema_version: 1\nprovider: {}\nresource_type: {}\ncanonical_id: {}\nprovider_instance: {}\nsource_url: {}\noriginal_url: {}\ncomplete: {}\nsource_revision: {}\ncontent_hash: {}\ngenerated: true\n",
        yaml_scalar(&asset.source.provider_id),
        yaml_scalar(&asset.source.resource_type),
        yaml_scalar(&asset.source.canonical_id),
        yaml_scalar(&asset.source.provider_instance),
        yaml_scalar(asset.source_url.as_deref().unwrap_or("")),
        yaml_scalar(asset.original_url.as_deref().unwrap_or("")),
        asset.complete,
        yaml_scalar(asset.source_revision.as_deref().unwrap_or("")),
        yaml_scalar(revision),
    );
    if let Some(container) = &asset.container {
        markdown.push_str(&format!(
            "container:\n  id: {}\n  label: {}\n",
            yaml_scalar(&container.id),
            yaml_scalar(&container.label)
        ));
    }
    for field in &asset.fields {
        markdown.push_str(&format!(
            "{}: {}\n",
            field.key,
            serde_json::to_string(&field.value).expect("field serialization")
        ));
    }
    if !asset.attachments.is_empty() {
        markdown.push_str("attachments:\n");
        for attachment in &asset.attachments {
            markdown.push_str(&format!(
                "  - id: {}\n    title: {}\n",
                yaml_scalar(&attachment.id),
                yaml_scalar(&attachment.title)
            ));
            for (key, value) in [
                ("media_type", attachment.media_type.as_deref()),
                ("source_url", attachment.source_url.as_deref()),
                ("source_revision", attachment.source_revision.as_deref()),
                ("path", attachment.path.as_deref()),
            ] {
                if let Some(value) = value {
                    markdown.push_str(&format!("    {key}: {}\n", yaml_scalar(value)));
                }
            }
            if let Some(size) = attachment.size {
                markdown.push_str(&format!("    size: {size}\n"));
            }
            if attachment.path.is_none() {
                markdown.push_str(&format!(
                    "    not_downloaded: {}\n",
                    yaml_scalar(
                        attachment
                            .not_downloaded
                            .as_deref()
                            .unwrap_or("not_requested")
                    )
                ));
            }
        }
    }
    markdown.push_str(&format!("---\n\n# {}\n\n{}\n", asset.title, asset.body));
    markdown
}

fn yaml_scalar(value: &str) -> String {
    serde_json::to_string(value).expect("string serialization")
}
pub(crate) fn source_id(source: &SourceRef) -> String {
    let mut hash = Sha256::new();
    for value in [
        &source.provider_id,
        &source.provider_instance,
        &source.resource_type,
        &source.canonical_id,
    ] {
        hash.update(value.as_bytes());
        hash.update([0]);
    }
    format!("source:{:x}", hash.finalize())
}

#[cfg(test)]
mod tests {
    use super::*;
    use cockpit_protocol::projects::{ProjectLimits, ProjectProvider};
    use std::collections::BTreeMap;
    use std::sync::Mutex;
    use uuid::Uuid;

    #[derive(Clone)]
    struct Provider(Arc<Mutex<SourceAsset>>);
    #[async_trait]
    impl SourceProvider for Provider {
        fn provider_id(&self) -> &str {
            "tea"
        }
        fn capabilities(&self) -> Vec<SourceCapability> {
            vec![SourceCapability::Issue]
        }
        async fn fetch(&self, _: &SourceFetchRequest) -> Result<Vec<SourceAsset>, InspectionError> {
            Ok(vec![self.0.lock().expect("asset").clone()])
        }
    }
    fn authority() -> SourceAuthority {
        SourceAuthority {
            provider_instance: "https://forge.test/gitea".into(),
            origin_host: "forge.test".into(),
            origin_port: None,
            origin_base_path: "/gitea".into(),
            owner: "acme".into(),
            repository: "repo".into(),
        }
    }
    fn asset(body: &str) -> SourceAsset {
        SourceAsset {
            source: SourceRef {
                provider_id: "tea".into(),
                provider_instance: authority().provider_instance,
                resource_type: "issue".into(),
                canonical_id: "acme/repo#1".into(),
            },
            title: "issue".into(),
            source_url: Some("https://forge.test/gitea/acme/repo/issues/1".into()),
            original_url: None,
            source_revision: Some("1".into()),
            complete: true,
            diagnostics: Vec::new(),
            body: body.into(),
            container: None,
            fields: Vec::new(),
            attachments: Vec::new(),
        }
    }

    #[test]
    fn content_revision_ignores_provenance_but_tracks_source_content() {
        let original = asset("body");
        let revision = content_revision(&original);
        let mut metadata = original.clone();
        metadata.source_url = Some("https://forge.test/gitea/acme/repo/issues/01".into());
        metadata.original_url = Some("https://forge.test/gitea/acme/repo/issues/1".into());
        metadata.container = Some(SourceContainer {
            id: "acme/repo".into(),
            label: "Renamed repository".into(),
        });
        metadata.diagnostics.push(ProjectDiagnostic {
            code: "source_markup_unconverted".into(),
            message: "Jira returned wiki markup; shown unconverted".into(),
            path: None,
        });
        assert_eq!(content_revision(&metadata), revision);
        for changed in [
            SourceAsset {
                body: "edited body".into(),
                ..original.clone()
            },
            SourceAsset {
                title: "edited title".into(),
                ..original.clone()
            },
            SourceAsset {
                source_revision: Some("2".into()),
                ..original.clone()
            },
            SourceAsset {
                complete: false,
                ..original.clone()
            },
            SourceAsset {
                fields: vec![FrontmatterField {
                    key: "labels".into(),
                    value: FrontmatterValue::Strings(vec!["backend".into()]),
                }],
                ..original.clone()
            },
        ] {
            assert_ne!(content_revision(&changed), revision);
        }
    }

    #[test]
    fn old_source_assets_deserialize_with_empty_extensions() {
        let original = asset("body");
        let mut value = serde_json::to_value(&original).unwrap();
        for key in ["container", "fields", "attachments"] {
            value.as_object_mut().unwrap().remove(key);
        }
        let restored: SourceAsset = serde_json::from_value(value).unwrap();
        assert!(restored.container.is_none());
        assert!(restored.fields.is_empty());
        assert!(restored.attachments.is_empty());
        assert_eq!(content_revision(&restored), content_revision(&original));
    }
    #[test]
    fn library_markdown_preserves_structured_values_and_attachment_availability() {
        let mut source = asset("Body");
        source.container = Some(SourceContainer {
            id: "acme/repo".into(),
            label: "A: repository".into(),
        });
        source.fields = vec![
            FrontmatterField {
                key: "labels".into(),
                value: FrontmatterValue::Strings(vec!["a: b".into(), "quoted\\\"".into()]),
            },
            FrontmatterField {
                key: "version".into(),
                value: FrontmatterValue::Number(7),
            },
        ];
        source.attachments = vec![SourceAttachment {
            id: "11".into(),
            title: "reference.png".into(),
            media_type: Some("image/png".into()),
            size: Some(12),
            source_url: Some("https://forge.test/attachment/11".into()),
            source_revision: Some("1".into()),
            path: None,
            not_downloaded: Some("not_requested".into()),
        }];
        validate_asset(&source).unwrap();
        let revision = content_revision(&source);
        let document = library_markdown(&source, &revision);
        assert_eq!(document, library_markdown(&source, &revision));
        assert!(document.contains(&format!("content_hash: \"{revision}\"\n")));
        assert!(
            document.contains("labels: [\"a: b\",\"quoted\\\\\\\"\"]\nversion: 7\nattachments:\n")
        );
        assert!(document.contains("    not_downloaded: \"not_requested\"\n"));
        source.attachments[0].path = Some("_files/reference.png".into());
        source.attachments[0].not_downloaded = None;
        assert_eq!(content_revision(&source), revision);
        let downloaded = library_markdown(&source, &revision);
        assert!(downloaded.contains("    path: \"_files/reference.png\"\n"));
        assert!(!downloaded.contains("not_downloaded:"));
        source.attachments[0].source_revision = Some("2".into());
        assert_ne!(content_revision(&source), revision);
        source.fields[0].key = "content_hash".into();
        assert_eq!(
            validate_asset(&source).unwrap_err().code,
            "source_asset_invalid"
        );
        source.fields[0].key = "labels".into();
        source.attachments[0].path = Some("_files/../escape".into());
        assert_eq!(
            validate_asset(&source).unwrap_err().code,
            "source_asset_invalid"
        );
    }

    #[tokio::test]
    async fn fetch_only_neither_initializes_nor_reads_or_changes_legacy_cache() {
        let (service, shared, root) = service(asset("body"));
        let cache = root.join("sources");
        assert!(!cache.exists());
        let fetched = service.fetch_assets(request()).await.unwrap();
        assert_eq!(fetched.assets[0].body, "body");
        assert!(!cache.exists());
        assert!(service.recent.lock().unwrap().fetches.is_empty());
        service.prefetch_for_setup(request()).await;
        let recent_before = service.recent.lock().unwrap().fetches.clone();
        assert_eq!(recent_before.len(), 1);
        std::fs::create_dir(&cache).unwrap();
        let sentinel = cache.join("source-current.json");
        std::fs::write(&sentinel, b"legacy cache must not even be read").unwrap();
        let before: Vec<_> = std::fs::read_dir(&cache)
            .unwrap()
            .map(|entry| entry.unwrap().file_name())
            .collect();
        let fetched = service.fetch_assets(request()).await.unwrap();
        assert_eq!(fetched.assets[0].body, "body");
        let recent_after = service.recent.lock().unwrap();
        assert_eq!(recent_after.fetches.len(), recent_before.len());
        assert_eq!(recent_after.fetches[0].0, recent_before[0].0);
        assert_eq!(recent_after.fetches[0].1, recent_before[0].1);
        assert_eq!(
            recent_after.fetches[0].2[0].body,
            recent_before[0].2[0].body
        );
        assert_eq!(
            std::fs::read(&sentinel).unwrap(),
            b"legacy cache must not even be read"
        );
        let after: Vec<_> = std::fs::read_dir(&cache)
            .unwrap()
            .map(|entry| entry.unwrap().file_name())
            .collect();
        assert_eq!(after, before);
        shared.lock().unwrap().source.provider_instance = "https://other.test".into();
        assert_eq!(
            service
                .fetch_assets(request())
                .await
                .unwrap_err()
                .code,
            "source_provider_contract"
        );
        assert_eq!(
            std::fs::read(&sentinel).unwrap(),
            b"legacy cache must not even be read"
        );
        std::fs::remove_dir_all(root).unwrap();
    }
    fn service(asset: SourceAsset) -> (SourceService, Arc<Mutex<SourceAsset>>, std::path::PathBuf) {
        let root = std::env::temp_dir().join(format!("cockpit-source-test-{}", Uuid::new_v4()));
        std::fs::create_dir_all(&root).expect("root");
        let config = ProjectConfiguration {
            version: 1,
            repository_roots: vec![],
            worktree_root: "worktrees".into(),
            companion_root: "companions".into(),
            state_root: root.to_string_lossy().into_owned(),
            cache_root: root.join("cache").to_string_lossy().into_owned(),
            library_root: "library".into(),
            notes_root: root.join("notes").to_string_lossy().into_owned(),
            branch_template: "{repo}/{task_id}".into(),
            checkout_template: "{repo}-{task_id}".into(),
            providers: vec![ProjectProvider {
                id: "tea".into(),
                base_url: "https://forge.test/gitea".into(),
                executable: "tea".into(),
                login: None,
            }],
            limits: ProjectLimits {
                catalog_depth: 1,
                catalog_entries: 1,
                git_timeout_ms: 1000,
                git_output_bytes: 65536,
                operation_timeout_ms: 1000,
                context_preview_bytes: 1024,
                context_preview_lines: 100,
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
            origins: BTreeMap::new(),
        };
        let shared = Arc::new(Mutex::new(asset));
        let service =
            SourceService::new(&config, vec![Arc::new(Provider(shared.clone()))]).expect("service");
        (service, shared, root)
    }
    fn request() -> SourceFetchRequest {
        SourceFetchRequest {
            provider_id: "tea".into(),
            artifact_url: "https://forge.test/gitea/acme/repo/issues/1".into(),
            authority: authority(),
        }
    }
    #[tokio::test]
    async fn a_setup_start_reuses_the_background_prefetch() {
        let (service, shared, root) = service(asset("prefetched"));
        service.prefetch_for_setup(request()).await;
        *shared.lock().expect("asset") = asset("later");
        let started = service
            .validate_artifact_for_setup(request())
            .await
            .expect("start");
        let prefetched = service.fetch_assets(request()).await.expect("import");
        // The start used the prefetched body; an import asks the provider.
        assert_ne!(
            started.assets[0].body,
            prefetched.assets[0].body
        );
        std::fs::remove_dir_all(root).expect("cleanup");
    }

    #[tokio::test]
    async fn setup_reuses_a_recent_provider_result_but_imports_ask_again() {
        let (service, shared, root) = service(asset("first"));
        let planned = service
            .validate_artifact_for_setup(request())
            .await
            .expect("plan");
        *shared.lock().expect("asset") = asset("second");
        let started = service
            .validate_artifact_for_setup(request())
            .await
            .expect("start");
        assert_eq!(
            started.assets[0].body,
            planned.assets[0].body
        );
        let imported = service.fetch_assets(request()).await.expect("import");
        assert_ne!(
            imported.assets[0].body,
            planned.assets[0].body
        );
        std::fs::remove_dir_all(root).expect("cleanup");
    }
    #[tokio::test]
    async fn rejects_bad_provider_output_and_oversized_metadata() {
        let (service, shared, root) = service(asset("body"));
        shared.lock().expect("asset").source.provider_instance = "https://other.test".into();
        assert_eq!(
            service.fetch_assets(request()).await.expect_err("instance").code,
            "source_provider_contract"
        );
        *shared.lock().expect("asset") = asset("body");
        shared.lock().expect("asset").title = "x".repeat(MAX_METADATA_BYTES + 1);
        assert_eq!(
            service.fetch_assets(request()).await.expect_err("metadata").code,
            "source_asset_invalid"
        );
        std::fs::remove_dir_all(root).expect("cleanup");
    }
    #[tokio::test]
    async fn a_page_answer_must_be_exactly_the_requested_confluence_page() {
        let mut page = asset("body");
        page.source.resource_type = "page".into();
        page.source.canonical_id = "7".into();
        let (service, shared, root) = service(page.clone());
        let mut request = request();
        request.artifact_url = confluence_page_url(&request.authority.provider_instance, "7");
        service.fetch_assets(request.clone()).await.expect("requested page");
        for canonical_id in ["8", "7a"] {
            shared.lock().expect("asset").source.canonical_id = canonical_id.into();
            assert_eq!(
                service.fetch_assets(request.clone()).await.expect_err(canonical_id).code,
                "source_identity_mismatch"
            );
        }
        // Unsupported providers keep the default, never a fallback resolver.
        assert_eq!(
            service.resolve_input("tea", "7").await.expect_err("unsupported").code,
            "source_capability_unavailable"
        );
        std::fs::remove_dir_all(root).expect("cleanup");
    }
    #[tokio::test]
    async fn plain_import_has_one_provider_deadline() {
        struct PendingProvider;
        #[async_trait]
        impl SourceProvider for PendingProvider {
            fn provider_id(&self) -> &str {
                "tea"
            }
            fn capabilities(&self) -> Vec<SourceCapability> {
                vec![SourceCapability::Issue]
            }
            async fn fetch(
                &self,
                _: &SourceFetchRequest,
            ) -> Result<Vec<SourceAsset>, InspectionError> {
                std::future::pending().await
            }
        }
        let (mut service, _, root) = service(asset("previous"));
        service.providers = Arc::new(vec![Arc::new(PendingProvider)]);
        service.operation_timeout = Duration::from_millis(5);
        let result = tokio::time::timeout(Duration::from_secs(1), service.fetch_assets(request()))
            .await
            .expect("bounded request");
        assert_eq!(result.expect_err("deadline").code, "source_fetch_timeout");
        std::fs::remove_dir_all(root).expect("cleanup");
    }
    #[test]
    fn normalized_frontmatter_quotes_structural_values() {
        assert_eq!(yaml_scalar("x: [y]\nnext"), "\"x: [y]\\nnext\"");
    }
    #[test]
    fn root_provider_base_path_is_a_valid_authority() {
        let mut request = request();
        request.authority.origin_base_path.clear();
        request.authority.provider_instance = "https://forge.test".into();
        request.artifact_url = "https://forge.test/acme/repo/issues/1".into();
        assert!(validate_request(&request).is_ok());
    }
}
