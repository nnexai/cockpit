//! Provider-neutral fetch, metadata and bounded hydration; Library owns persistence.

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
mod hydration;

const MAX_ASSETS_PER_FETCH: usize = 64;
const MAX_ASSET_BYTES: usize = 1024 * 1024;
const MAX_METADATA_BYTES: usize = 16 * 1024;
const MAX_URL_BYTES: usize = 8 * 1024;
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
    pub hydration: Option<SourceHydration>,
}

#[derive(Debug, Clone)]
pub struct SourceHydration {
    pub completed: u32,
    pub skipped: u32,
    pub failed: u32,
    pub truncated: bool,
    pub total_bytes: u64,
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
}
#[derive(Clone)]
pub struct SourceService {
    providers: Arc<Vec<Arc<dyn SourceProvider>>>,
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
        let fetched = self.fetch_assets_reusing(request, false, true, true).await?;
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
    /// Fetch and validate provider assets without accessing persistent state.
    pub async fn fetch_assets(
        &self,
        request: SourceFetchRequest,
        hydrate_references: bool,
    ) -> Result<FetchedAssets, InspectionError> {
        self.fetch_assets_reusing(request, hydrate_references, false, false)
            .await
    }

    async fn fetch_assets_reusing(
        &self,
        request: SourceFetchRequest,
        hydrate_references: bool,
        reuse_recent: bool,
        remember_recent: bool,
    ) -> Result<FetchedAssets, InspectionError> {
        validate_request(&request)?;
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
        // A hydration operation has one deadline. Secondary references only
        // receive the remainder after the primary provider fetch completes.
        let started = Instant::now();
        let deadline_budget = if hydrate_references {
            self.operation_timeout.min(hydration::HYDRATION_TIMEOUT)
        } else {
            self.operation_timeout
        };
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
        let primary = match reused {
            Some(assets) => assets,
            None => {
                let assets = timeout(deadline_budget, provider.fetch(&request))
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
        if primary.len() > MAX_ASSETS_PER_FETCH {
            return Err(InspectionError::new(
                "source_provider_contract",
                "provider returned too many source assets",
            ));
        }
        for asset in &primary {
            validate_provider_asset(&request, asset)?;
            validate_asset(asset)?;
        }
        if hydrate_references
            && (primary.len() > hydration::HYDRATION_MAX_ASSETS
                || primary.iter().map(|asset| asset.body.len()).sum::<usize>()
                    > hydration::HYDRATION_MAX_TOTAL_BYTES)
        {
            return Err(InspectionError::new(
                "source_hydration_primary_budget",
                "primary provider result exceeds the configured hydration budget",
            ));
        }
        let hydration = if hydrate_references {
            hydration::hydrate(provider, &request, primary, started, deadline_budget).await
        } else {
            hydration::HydrationResult {
                assets: primary,
                diagnostics: Vec::new(),
                completed: 0,
                skipped: 0,
                failed: 0,
                truncated: false,
                total_bytes: 0,
            }
        };
        if hydration.assets.len() > MAX_ASSETS_PER_FETCH {
            return Err(InspectionError::new(
                "source_provider_contract",
                "provider returned too many source assets",
            ));
        }
        let mut diagnostics = hydration.diagnostics;
        for asset in &hydration.assets {
            validate_provider_asset(&request, asset)?;
            validate_asset(asset)?;
            diagnostics.extend(asset.diagnostics.iter().cloned());
        }
        Ok(FetchedAssets {
            assets: hydration.assets,
            diagnostics,
            hydration: hydrate_references.then_some(SourceHydration {
                completed: hydration.completed,
                skipped: hydration.skipped,
                failed: hydration.failed,
                truncated: hydration.truncated,
                total_bytes: hydration.total_bytes,
            }),
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
                    path.strip_prefix("attachments/").is_some_and(|name| {
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
    #[derive(Clone)]
    struct GraphProvider {
        assets: Arc<Mutex<BTreeMap<String, SourceAsset>>>,
        failures: Arc<Mutex<BTreeMap<String, String>>>,
        requests: Arc<Mutex<Vec<SourceFetchRequest>>>,
    }
    #[async_trait]
    impl SourceProvider for GraphProvider {
        fn provider_id(&self) -> &str {
            "tea"
        }
        fn capabilities(&self) -> Vec<SourceCapability> {
            vec![SourceCapability::Issue]
        }
        async fn fetch(
            &self,
            request: &SourceFetchRequest,
        ) -> Result<Vec<SourceAsset>, InspectionError> {
            self.requests
                .lock()
                .expect("requests")
                .push(request.clone());
            if let Some(message) = self
                .failures
                .lock()
                .expect("failures")
                .get(&request.artifact_url)
                .cloned()
            {
                return Err(InspectionError::new("fixture_failure", message));
            }
            self.assets
                .lock()
                .expect("assets")
                .get(&request.artifact_url)
                .cloned()
                .map(|asset| vec![asset])
                .ok_or_else(|| InspectionError::new("fixture_missing", "fixture source is missing"))
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
        source.attachments[0].path = Some("attachments/reference.png".into());
        source.attachments[0].not_downloaded = None;
        assert_eq!(content_revision(&source), revision);
        let downloaded = library_markdown(&source, &revision);
        assert!(downloaded.contains("    path: \"attachments/reference.png\"\n"));
        assert!(!downloaded.contains("not_downloaded:"));
        source.attachments[0].source_revision = Some("2".into());
        assert_ne!(content_revision(&source), revision);
        source.fields[0].key = "content_hash".into();
        assert_eq!(
            validate_asset(&source).unwrap_err().code,
            "source_asset_invalid"
        );
        source.fields[0].key = "labels".into();
        source.attachments[0].path = Some("attachments/../escape".into());
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
        let fetched = service.fetch_assets(request(), false).await.unwrap();
        assert_eq!(fetched.assets[0].body, "body");
        assert!(fetched.hydration.is_none());
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
        let fetched = service.fetch_assets(request(), true).await.unwrap();
        assert_eq!(fetched.assets[0].body, "body");
        assert!(fetched.hydration.is_some());
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
                .fetch_assets(request(), false)
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
            library_root: "library".into(),
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
    fn graph_asset(index: u64, body: &str) -> SourceAsset {
        SourceAsset {
            source: SourceRef {
                provider_id: "tea".into(),
                provider_instance: authority().provider_instance,
                resource_type: "issue".into(),
                canonical_id: format!("acme/repo#{index}"),
            },
            title: format!("issue {index}"),
            source_url: Some(format!("https://forge.test/gitea/acme/repo/issues/{index}")),
            original_url: None,
            source_revision: Some(index.to_string()),
            complete: true,
            diagnostics: Vec::new(),
            body: body.into(),
            container: None,
            fields: Vec::new(),
            attachments: Vec::new(),
        }
    }
    fn graph_service(
        assets: BTreeMap<String, SourceAsset>,
        failures: BTreeMap<String, String>,
    ) -> (
        SourceService,
        Arc<Mutex<Vec<SourceFetchRequest>>>,
        std::path::PathBuf,
    ) {
        let root = std::env::temp_dir().join(format!("cockpit-source-graph-{}", Uuid::new_v4()));
        std::fs::create_dir_all(&root).expect("root");
        let config = ProjectConfiguration {
            version: 1,
            repository_roots: vec![],
            worktree_root: "worktrees".into(),
            companion_root: "companions".into(),
            state_root: root.to_string_lossy().into_owned(),
            library_root: "library".into(),
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
        let requests = Arc::new(Mutex::new(Vec::new()));
        let provider = GraphProvider {
            assets: Arc::new(Mutex::new(assets)),
            failures: Arc::new(Mutex::new(failures)),
            requests: requests.clone(),
        };
        (
            SourceService::new(&config, vec![Arc::new(provider)]).expect("service"),
            requests,
            root,
        )
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
        let prefetched = service.fetch_assets(request(), false).await.expect("import");
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
        let imported = service.fetch_assets(request(), false).await.expect("import");
        assert_ne!(
            imported.assets[0].body,
            planned.assets[0].body
        );
        std::fs::remove_dir_all(root).expect("cleanup");
    }
    #[tokio::test]
    async fn hydration_is_opt_in_bounded_and_retains_primary_on_secondary_failure() {
        let one = "https://forge.test/gitea/acme/repo/issues/1".to_owned();
        let two = "https://forge.test/gitea/acme/repo/issues/2".to_owned();
        let three = "https://forge.test/gitea/acme/repo/issues/3".to_owned();
        let four = "https://forge.test/gitea/acme/repo/issues/4".to_owned();
        let assets = BTreeMap::from([
            (
                one.clone(),
                graph_asset(
                    1,
                    &format!(
                        "[related issue]({two}) {two} {one} {three} https://other.test/acme/repo/issues/9"
                    ),
                ),
            ),
            (two.clone(), graph_asset(2, &three)),
            (three.clone(), graph_asset(3, &four)),
            (four.clone(), graph_asset(4, "terminal")),
        ]);
        let (service, requests, root) = graph_service(
            assets,
            BTreeMap::from([(three.clone(), "secondary unavailable".into())]),
        );
        let primary = service
            .fetch_assets(request(), false)
            .await
            .expect("primary only");
        assert_eq!(primary.assets.len(), 1);
        let hydrated = service
            .fetch_assets(request(), true)
            .await
            .expect("hydrated");
        assert_eq!(
            hydrated
                .assets
                .iter()
                .map(|entry| entry.source.canonical_id.as_str())
                .collect::<Vec<_>>(),
            vec!["acme/repo#1", "acme/repo#2"]
        );
        assert!(
            hydrated
                .diagnostics
                .iter()
                .any(|entry| entry.code == "source_hydration_fetch_failed")
        );
        let calls = requests.lock().expect("requests");
        assert!(
            calls
                .iter()
                .all(|call| call.provider_id == "tea" && call.authority == authority())
        );
        assert!(calls.iter().any(|call| call.artifact_url == two));
        assert!(calls.iter().any(|call| call.artifact_url == three));
        drop(calls);
        let report = hydrated.hydration.unwrap();
        assert_eq!(report.completed, 1);
        assert_eq!(report.failed, 1);
        assert!(!root.join("sources").exists());
        std::fs::remove_dir_all(root).unwrap();
    }
    #[tokio::test]
    async fn hydration_deduplicates_cycles_and_stops_at_depth_limit() {
        let one = "https://forge.test/gitea/acme/repo/issues/1".to_owned();
        let two = "https://forge.test/gitea/acme/repo/issues/2".to_owned();
        let three = "https://forge.test/gitea/acme/repo/issues/3".to_owned();
        let four = "https://forge.test/gitea/acme/repo/issues/4".to_owned();
        let assets = BTreeMap::from([
            (one.clone(), graph_asset(1, &format!("{two} {two} {one}"))),
            (two.clone(), graph_asset(2, &three)),
            (three.clone(), graph_asset(3, &four)),
            (four.clone(), graph_asset(4, "terminal")),
        ]);
        let (service, requests, root) = graph_service(assets, BTreeMap::new());
        let hydrated = service
            .fetch_assets(request(), true)
            .await
            .expect("hydrated");
        assert_eq!(
            hydrated
                .assets
                .iter()
                .map(|entry| entry.source.canonical_id.as_str())
                .collect::<Vec<_>>(),
            vec!["acme/repo#1", "acme/repo#2", "acme/repo#3"]
        );
        assert!(
            hydrated
                .diagnostics
                .iter()
                .any(|entry| entry.code == "source_hydration_cycle")
        );
        assert!(
            hydrated
                .diagnostics
                .iter()
                .any(|entry| entry.code == "source_hydration_depth")
        );
        assert_eq!(requests.lock().expect("requests").len(), 3);
        std::fs::remove_dir_all(root).expect("cleanup");
    }
    #[tokio::test]
    async fn rejects_bad_provider_output_and_oversized_metadata() {
        let (service, shared, root) = service(asset("body"));
        shared.lock().expect("asset").source.provider_instance = "https://other.test".into();
        assert_eq!(
            service.fetch_assets(request(), false).await.expect_err("instance").code,
            "source_provider_contract"
        );
        *shared.lock().expect("asset") = asset("body");
        shared.lock().expect("asset").title = "x".repeat(MAX_METADATA_BYTES + 1);
        assert_eq!(
            service.fetch_assets(request(), false).await.expect_err("metadata").code,
            "source_asset_invalid"
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
        let result = tokio::time::timeout(Duration::from_secs(1), service.fetch_assets(request(), false))
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
