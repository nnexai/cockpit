//! Provider-neutral immutable source cache and provider contract.

use std::collections::BTreeMap;
use std::io::ErrorKind;
use std::path::Path;
use std::sync::Arc;
use std::time::{Duration, Instant};

use async_trait::async_trait;
use cap_std::fs::Dir;
use cockpit_protocol::projects::{ProjectConfiguration, ProjectDiagnostic};
use cockpit_protocol::sources::{
    SourceCapability, SourceEntry, SourceFreshness, SourceImportResponse,
    SourceMaterializationStatus,
};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use tokio::process::Command;
use tokio::time::timeout;

use crate::InspectionError;
use crate::context_assets::{materialize_source_markdown, source_materialization_state};
use crate::project_store::{ProjectStore, atomic_write_json, read_json_bounded, timestamp};

mod hydration;

const MAX_ASSETS_PER_FETCH: usize = 64;
const MAX_ASSET_BYTES: usize = 1024 * 1024;
const MAX_METADATA_BYTES: usize = 16 * 1024;
const MAX_URL_BYTES: usize = 8 * 1024;
const MAX_CACHE_RECORD_BYTES: u64 = 2 * 1024 * 1024;
const MAX_INDEX_BYTES: usize = 256 * 1024;
const MAX_CURRENT: usize = 64;
const MAX_IMMUTABLE: usize = 128;
const MAX_IMMUTABLE_BYTES: u64 = 16 * 1024 * 1024;
const INDEX_NAME: &str = "source-current.json";
const LOCK_NAME: &str = ".source-current.lock";
const IMPORT_LOCK_NAME: &str = ".source-import.lock";
const HYDRATION_REPORT_PREFIX: &str = "source-hydration-";
const MAX_HYDRATION_REPORT_BYTES: usize = 64 * 1024;
const MAX_HYDRATION_REPORT_DIAGNOSTIC_TEXT: usize = 512;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SourceRef {
    pub provider_id: String,
    /// Credential-free configured base URL, normalized by ContextService.
    pub provider_instance: String,
    pub resource_type: String,
    pub canonical_id: String,
}

#[derive(Debug, Clone)]
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
}

/// Proof derived from the configured provider and the primary checkout origin.
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
}

/// Derive source authority from the primary checkout's origin without trusting
/// caller-supplied repository or provider identity.
pub(crate) async fn source_authority_for_checkout(
    configuration: &ProjectConfiguration,
    checkout: &Path,
    provider_id: &str,
) -> Result<SourceAuthority, InspectionError> {
    let mut command = Command::new("git");
    command
        .current_dir(checkout)
        .args([
            "-c",
            "core.hooksPath=/dev/null",
            "remote",
            "get-url",
            "origin",
        ])
        .env("GIT_TERMINAL_PROMPT", "0")
        .env_remove("GIT_DIR")
        .env_remove("GIT_WORK_TREE");
    let output = crate::process::run_bounded_command(
        command,
        configuration.limits.git_output_bytes as usize,
        configuration.limits.git_output_bytes as usize,
        Duration::from_millis(configuration.limits.git_timeout_ms as u64),
        "source origin",
    )
    .await?;
    if !output.status.success() {
        return Err(InspectionError::new(
            "source_primary_origin_unavailable",
            "the companion primary checkout has no readable origin remote",
        ));
    }
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
    let value = std::str::from_utf8(&output.stdout)
        .map_err(|_| {
            InspectionError::new(
                "source_primary_origin_invalid",
                "origin remote is not UTF-8",
            )
        })?
        .trim();
    let (owner, repository) = origin_repository(value, &instance)?;
    Ok(SourceAuthority {
        provider_instance: instance.render(),
        origin_host: instance.host,
        origin_port: instance.port,
        origin_base_path: instance.base_path,
        owner,
        repository,
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

fn origin_repository(
    value: &str,
    instance: &ProviderInstance,
) -> Result<(String, String), InspectionError> {
    let (host, port, path, scheme) = if value.contains("://") {
        let url = url::Url::parse(value).map_err(|_| {
            InspectionError::new("source_primary_origin_invalid", "origin remote is invalid")
        })?;
        let host = url
            .host_str()
            .ok_or_else(|| {
                InspectionError::new(
                    "source_primary_origin_invalid",
                    "origin remote lacks a host",
                )
            })?
            .to_ascii_lowercase();
        (
            host,
            url.port(),
            url.path().to_owned(),
            Some(url.scheme().to_ascii_lowercase()),
        )
    } else {
        let (_, tail) = value.rsplit_once('@').unwrap_or(("", value));
        let (host, path) = tail.split_once(':').ok_or_else(|| {
            InspectionError::new(
                "source_primary_origin_invalid",
                "origin remote does not identify owner/repository",
            )
        })?;
        (host.to_ascii_lowercase(), None, format!("/{path}"), None)
    };
    if host != instance.host
        || scheme
            .as_deref()
            .is_some_and(|scheme| scheme != instance.scheme)
        || (scheme.is_some()
            && port.filter(|port| *port != if instance.scheme == "https" { 443 } else { 80 })
                != instance.port)
    {
        return Err(InspectionError::new(
            "source_primary_origin_mismatch",
            "primary origin does not match the selected configured provider instance",
        ));
    }
    let prefix = if instance.base_path.is_empty() {
        "/".to_owned()
    } else {
        format!("{}/", instance.base_path)
    };
    let remainder = path
        .strip_prefix(&prefix)
        .ok_or_else(|| {
            InspectionError::new(
                "source_primary_origin_mismatch",
                "primary origin does not match the configured provider base path",
            )
        })?
        .trim_matches('/')
        .trim_end_matches(".git");
    let parts: Vec<&str> = remainder.split('/').collect();
    if parts.len() < 2
        || parts.iter().any(|part| {
            part.is_empty() || *part == "." || *part == ".." || part.chars().any(char::is_control)
        })
    {
        return Err(InspectionError::new(
            "source_primary_origin_invalid",
            "origin remote does not identify a namespace and repository",
        ));
    }
    let repository = parts.last().expect("at least two parts").to_string();
    let owner = parts[..parts.len() - 1].join("/");
    Ok((owner, repository))
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
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct CachedSource {
    schema_version: u32,
    source: SourceRef,
    title: String,
    source_url: Option<String>,
    #[serde(default)]
    original_url: Option<String>,
    source_revision: Option<String>,
    #[serde(default = "default_complete")]
    complete: bool,
    #[serde(default)]
    diagnostics: Vec<ProjectDiagnostic>,
    content_hash: String,
    markdown: String,
    cached_at: String,
}

struct CacheOutcome {
    cached: CachedSource,
    freshness: SourceFreshness,
    inconsistency: Option<ProjectDiagnostic>,
    previous_identity: Option<(Option<String>, String)>,
}

/// Bounded evidence of an explicitly requested reference hydration. The
/// companion identifier is represented only by a deterministic hash, so the
/// state directory cannot disclose a user-supplied companion name.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct HydrationReport {
    schema_version: u32,
    primary_source_id: String,
    companion_key: Option<String>,
    completed: u32,
    skipped: u32,
    failed: u32,
    truncated: bool,
    max_depth: u32,
    max_assets: u32,
    max_total_bytes: u64,
    total_bytes: u64,
    diagnostics: Vec<ProjectDiagnostic>,
    updated_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct CachePointer {
    content_hash: String,
    updated_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct ImmutableRecord {
    source_id: String,
    content_hash: String,
    bytes: u64,
    cached_at: String,
}

#[derive(Debug, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct CacheIndex {
    schema_version: u32,
    current: BTreeMap<String, CachePointer>,
    immutable: Vec<ImmutableRecord>,
}

#[derive(Clone)]
pub struct SourceService {
    cache: Arc<ProjectStore>,
    providers: Arc<Vec<Arc<dyn SourceProvider>>>,
    operation_timeout: Duration,
}

impl SourceService {
    pub fn new(
        configuration: &ProjectConfiguration,
        providers: Vec<Arc<dyn SourceProvider>>,
    ) -> Result<Self, InspectionError> {
        Ok(Self {
            cache: Arc::new(ProjectStore::new(
                Path::new(&configuration.state_root).join("sources"),
            )?),
            providers: Arc::new(providers),
            operation_timeout: Duration::from_millis(
                configuration.limits.operation_timeout_ms.into(),
            ),
        })
    }

    pub async fn fetch(
        &self,
        request: SourceFetchRequest,
    ) -> Result<SourceImportResponse, InspectionError> {
        self.fetch_to_companion(request, None).await
    }

    /// Validate and cache a reviewed artifact before workspace creation. The
    /// provider fetch is bounded by the same lock, output, and timeout rules
    /// as materialization, but no companion is touched.
    pub async fn validate_artifact(
        &self,
        request: SourceFetchRequest,
    ) -> Result<SourceImportResponse, InspectionError> {
        let response = self.fetch_to_companion(request, None).await?;
        if response.entries.is_empty() {
            return Err(InspectionError::new(
                "source_provider_contract",
                "source provider returned no primary artifact",
            ));
        }
        Ok(response)
    }

    /// Resolve provider metadata without writing the source cache or companion.
    pub async fn metadata(
        &self,
        request: SourceFetchRequest,
    ) -> Result<SourceMetadata, InspectionError> {
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
            || !valid_commit
        {
            return Err(InspectionError::new(
                "source_provider_contract",
                "source metadata contains invalid title, branch, URL, or commit text",
            ));
        }
        Ok(metadata)
    }

    fn list_cached_records(
        &self,
    ) -> Result<Vec<(SourceEntry, bool, Vec<ProjectDiagnostic>)>, InspectionError> {
        let _lock = self
            .cache
            .acquire_named_lock(LOCK_NAME, "source_cache_lock")?;
        let index = self.read_index()?;
        index
            .current
            .into_iter()
            .map(|(id, pointer)| {
                let cached = self.read_cached(&pointer.content_hash)?;
                if source_id(&cached.source) != id {
                    return Err(InspectionError::new(
                        "source_cache_corrupt",
                        "current source pointer mismatches immutable record",
                    ));
                }
                let entry = entry_from_cached(
                    &cached,
                    cached_freshness(&cached),
                    SourceMaterializationStatus::Unchanged,
                    None,
                );
                Ok((entry, cached.complete, cached.diagnostics))
            })
            .collect()
    }

    /// Reads the bounded current index only. Hash-named immutable records are
    /// intentionally never selected by directory enumeration.
    pub fn list_cached(&self) -> Result<Vec<SourceEntry>, InspectionError> {
        Ok(self
            .list_cached_records()?
            .into_iter()
            .map(|(entry, _, _)| entry)
            .collect())
    }

    pub fn list_for_companion(
        &self,
        root: &Dir,
        companion_id: &str,
    ) -> Result<(Vec<SourceEntry>, Vec<ProjectDiagnostic>), InspectionError> {
        let mut entries = Vec::new();
        let mut diagnostics = Vec::new();
        for (mut entry, complete, entry_diagnostics) in self.list_cached_records()? {
            let (freshness, status, path) = source_materialization_state(
                root,
                companion_id,
                &entry.provider_id,
                &entry.provider_instance,
                &entry.resource_type,
                &entry.canonical_id,
                &entry.content_hash,
            )?;
            entry.freshness = freshness;
            if matches!(
                entry.freshness,
                SourceFreshness::Fresh | SourceFreshness::Changed
            ) && (!complete || entry.source_revision.is_none())
            {
                entry.freshness = SourceFreshness::Unknown;
            }
            entry.status = status;
            entry.relative_path = path;
            if entry.relative_path.is_some() {
                diagnostics.extend(entry_diagnostics);
                entries.push(entry);
            }
        }
        Ok((entries, diagnostics))
    }

    pub async fn refresh_cached(
        &self,
        source_id_value: &str,
        authority: SourceAuthority,
        companion: Option<(&Dir, &str)>,
    ) -> Result<SourceImportResponse, InspectionError> {
        self.refresh_cached_hydrated(source_id_value, authority, companion, false)
            .await
    }

    pub async fn refresh_cached_hydrated(
        &self,
        source_id_value: &str,
        authority: SourceAuthority,
        companion: Option<(&Dir, &str)>,
        hydrate_references: bool,
    ) -> Result<SourceImportResponse, InspectionError> {
        let cached = self.find_cached(source_id_value)?;
        if cached.source.provider_instance != authority.provider_instance {
            return Err(InspectionError::new(
                "source_authority_mismatch",
                "cached source belongs to another provider instance",
            ));
        }
        let artifact_url = cached.source_url.clone().ok_or_else(|| {
            InspectionError::new(
                "source_refresh_unavailable",
                "cached source has no provider URL",
            )
        })?;
        self.fetch_to_companion_hydrated_preserving(
            SourceFetchRequest {
                provider_id: cached.source.provider_id,
                artifact_url,
                authority,
            },
            companion,
            hydrate_references,
            cached.original_url.as_deref(),
        )
        .await
    }

    pub(crate) fn find_provider_id(
        &self,
        source_id_value: &str,
    ) -> Result<String, InspectionError> {
        Ok(self.find_cached(source_id_value)?.source.provider_id)
    }

    fn find_cached(&self, source_id_value: &str) -> Result<CachedSource, InspectionError> {
        if !bounded_text(source_id_value, 256) {
            return Err(InspectionError::new(
                "source_not_found",
                "source identity is invalid",
            ));
        }
        let _lock = self
            .cache
            .acquire_named_lock(LOCK_NAME, "source_cache_lock")?;
        let index = self.read_index()?;
        let pointer = index.current.get(source_id_value).ok_or_else(|| {
            InspectionError::new("source_not_found", "cached source identity was not found")
        })?;
        let cached = self.read_cached(&pointer.content_hash)?;
        if source_id(&cached.source) != source_id_value {
            return Err(InspectionError::new(
                "source_cache_corrupt",
                "current source pointer mismatches immutable record",
            ));
        }
        Ok(cached)
    }

    pub async fn fetch_to_companion(
        &self,
        request: SourceFetchRequest,
        companion: Option<(&Dir, &str)>,
    ) -> Result<SourceImportResponse, InspectionError> {
        self.fetch_to_companion_hydrated_preserving(request, companion, false, None)
            .await
    }

    pub async fn fetch_to_companion_hydrated(
        &self,
        request: SourceFetchRequest,
        companion: Option<(&Dir, &str)>,
        hydrate_references: bool,
    ) -> Result<SourceImportResponse, InspectionError> {
        self.fetch_to_companion_hydrated_preserving(request, companion, hydrate_references, None)
            .await
    }

    async fn fetch_to_companion_hydrated_preserving(
        &self,
        request: SourceFetchRequest,
        companion: Option<(&Dir, &str)>,
        hydrate_references: bool,
        preserved_original_url: Option<&str>,
    ) -> Result<SourceImportResponse, InspectionError> {
        validate_request(&request)?;
        // One nonblocking durable lease spans fetch, cache selection and
        // companion publication across browser/native hosts sharing this store.
        let _operation = self
            .cache
            .try_acquire_named_lock(IMPORT_LOCK_NAME, "source_import_lock")?
            .ok_or_else(|| {
                InspectionError::new(
                    "source_import_busy",
                    "Another source import is active. Retry when it finishes.",
                )
            })?;
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
        let primary = timeout(deadline_budget, provider.fetch(&request))
            .await
            .map_err(|_| {
                InspectionError::new(
                    "source_fetch_timeout",
                    "source import exceeded the configured operation deadline",
                )
            })??;
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
        let primary_source_id = hydration
            .assets
            .first()
            .map(|asset| source_id(&asset.source));
        let mut hydration_report = hydrate_references.then(|| HydrationReport {
            schema_version: 1,
            primary_source_id: primary_source_id.clone().unwrap_or_default(),
            companion_key: companion.map(|(_, companion_id)| companion_report_key(companion_id)),
            completed: hydration.completed,
            skipped: hydration.skipped,
            failed: hydration.failed,
            truncated: hydration.truncated,
            max_depth: hydration::HYDRATION_MAX_DEPTH,
            max_assets: hydration::HYDRATION_MAX_ASSETS as u32,
            max_total_bytes: hydration::HYDRATION_MAX_TOTAL_BYTES as u64,
            total_bytes: hydration.total_bytes,
            diagnostics: bounded_report_diagnostics(&hydration.diagnostics),
            updated_at: timestamp(),
        });
        let assets = hydration.assets;
        if assets.len() > MAX_ASSETS_PER_FETCH {
            return Err(InspectionError::new(
                "source_provider_contract",
                "provider returned too many source assets",
            ));
        }
        let mut entries = Vec::with_capacity(assets.len());
        let mut diagnostics = hydration.diagnostics;
        for (index, asset) in assets.into_iter().enumerate() {
            let mut asset = asset;
            if index == 0 {
                if let Some(original_url) = preserved_original_url {
                    asset.original_url = Some(original_url.to_owned());
                }
            }
            validate_provider_asset(&request, &asset)?;
            let artifact_url = asset.source_url.clone();
            let asset_diagnostics = asset.diagnostics.clone();
            diagnostics.extend(asset_diagnostics);
            let outcome = match self.cache_asset(asset) {
                Ok(outcome) => outcome,
                Err(error) if hydrate_references && index > 0 => {
                    diagnostics.push(ProjectDiagnostic {
                        code: "source_hydration_cache_failed".into(),
                        message: error.message,
                        path: artifact_url,
                    });
                    if let Some(report) = &mut hydration_report {
                        report.failed += 1;
                    }
                    continue;
                }
                Err(error) => return Err(error),
            };
            let provider_freshness = outcome.freshness;
            let changed_diagnostic = changed_diagnostic(
                provider_freshness,
                outcome.previous_identity.as_ref(),
                &outcome.cached,
            );
            let previous_identity = outcome.previous_identity;
            let inconsistency = outcome.inconsistency;
            let cached = outcome.cached;
            let (freshness, relative_path, status) = match companion {
                Some((root, companion_id)) => match materialize_source_markdown(
                    root,
                    companion_id,
                    &cached.source.provider_id,
                    &cached.source.provider_instance,
                    &cached.source.resource_type,
                    &cached.source.canonical_id,
                    cached.source_revision.as_deref(),
                    &cached.content_hash,
                    cached.markdown.as_bytes(),
                ) {
                    Ok((path, true)) => (
                        provider_freshness,
                        Some(path),
                        SourceMaterializationStatus::Materialized,
                    ),
                    Ok((path, false)) => (
                        provider_freshness,
                        Some(path),
                        SourceMaterializationStatus::Unchanged,
                    ),
                    Err(error) if error.code == "source_sync_conflict" => {
                        let (relative_path, state_error) = match source_materialization_state(
                            root,
                            companion_id,
                            &cached.source.provider_id,
                            &cached.source.provider_instance,
                            &cached.source.resource_type,
                            &cached.source.canonical_id,
                            &cached.content_hash,
                        ) {
                            Ok((_, _, path)) => (path, None),
                            Err(state_error) => (None, Some(state_error)),
                        };
                        diagnostics.push(materialization_diagnostic(
                            "source_materialization_conflict",
                            "local generated source was edited; provider content was preserved in the immutable cache",
                            &cached,
                            previous_identity.as_ref(),
                            relative_path.as_deref().or(cached.source_url.as_deref()),
                        ));
                        if let Some(state_error) = state_error {
                            diagnostics.push(materialization_failure_diagnostic(
                                "source_materialization_state_failed",
                                state_error,
                                cached.source_url.as_deref(),
                            ));
                        }
                        (
                            SourceFreshness::Conflict,
                            relative_path,
                            SourceMaterializationStatus::Conflict,
                        )
                    }
                    Err(error) if hydrate_references && index > 0 => {
                        diagnostics.push(ProjectDiagnostic {
                            code: "source_hydration_materialization_failed".into(),
                            message: truncate_diagnostic_text(&error.message),
                            path: cached.source_url.clone(),
                        });
                        if let Some(report) = &mut hydration_report {
                            report.failed += 1;
                        }
                        (
                            SourceFreshness::Unavailable,
                            None,
                            SourceMaterializationStatus::Failed,
                        )
                    }
                    Err(error) => {
                        let relative_path = source_materialization_state(
                            root,
                            companion_id,
                            &cached.source.provider_id,
                            &cached.source.provider_instance,
                            &cached.source.resource_type,
                            &cached.source.canonical_id,
                            &cached.content_hash,
                        )
                        .ok()
                        .and_then(|(_, _, path)| path);
                        diagnostics.push(materialization_failure_diagnostic(
                            "source_materialization_failed",
                            error,
                            cached.source_url.as_deref(),
                        ));
                        if hydrate_references {
                            if let Some(report) = &mut hydration_report {
                                report.failed += 1;
                            }
                        }
                        (
                            SourceFreshness::Unavailable,
                            relative_path,
                            SourceMaterializationStatus::Failed,
                        )
                    }
                },
                None => (
                    provider_freshness,
                    None,
                    SourceMaterializationStatus::Materialized,
                ),
            };
            entries.push(entry_from_cached(&cached, freshness, status, relative_path));
            if let Some(diagnostic) = inconsistency {
                diagnostics.push(diagnostic);
            }
            if let Some(diagnostic) = changed_diagnostic {
                diagnostics.push(diagnostic);
            }
        }
        if let (Some(primary_source_id), Some(mut report)) = (primary_source_id, hydration_report) {
            report.diagnostics = bounded_report_diagnostics(&diagnostics);
            if let Err(error) = self.persist_hydration_report(&primary_source_id, report) {
                diagnostics.push(ProjectDiagnostic {
                    code: "source_hydration_report_failed".into(),
                    message: error.message,
                    path: None,
                });
            }
        }
        Ok(SourceImportResponse {
            binding_id: String::new(),
            root_id: String::new(),
            entries,
            diagnostics,
        })
    }

    fn persist_hydration_report(
        &self,
        primary_source_id: &str,
        report: HydrationReport,
    ) -> Result<(), InspectionError> {
        let _lock = self
            .cache
            .acquire_named_lock(LOCK_NAME, "source_cache_lock")?;
        write_json_bounded(
            self.cache.state_dir(),
            &hydration_report_name(primary_source_id, report.companion_key.as_deref()),
            &report,
            MAX_HYDRATION_REPORT_BYTES,
        )
    }

    fn cache_asset(&self, asset: SourceAsset) -> Result<CacheOutcome, InspectionError> {
        validate_asset(&asset)?;
        let markdown = canonical_markdown(&asset);
        let content_hash = semantic_hash(&asset);
        let source_id_value = source_id(&asset.source);
        let cached = CachedSource {
            schema_version: 1,
            source: asset.source,
            title: asset.title,
            source_url: asset.source_url,
            original_url: asset.original_url,
            source_revision: asset.source_revision,
            complete: asset.complete,
            diagnostics: asset.diagnostics,
            content_hash: content_hash.clone(),
            markdown,
            cached_at: timestamp(),
        };
        let bytes = serialized_len(&cached)?;
        if bytes > MAX_CACHE_RECORD_BYTES as usize {
            return Err(InspectionError::new(
                "source_asset_invalid",
                "normalized source asset exceeds cache record limit",
            ));
        }

        let _lock = self
            .cache
            .acquire_named_lock(LOCK_NAME, "source_cache_lock")?;
        let mut index = self.read_index()?;
        self.remove_orphaned_records(&index)?;
        let previous = index
            .current
            .get(&source_id_value)
            .map(|pointer| self.read_cached(&pointer.content_hash))
            .transpose()?;
        let name = cache_name(&content_hash);
        let result = match self.cache.state_dir().symlink_metadata(&name) {
            Ok(_) => self.read_cached(&content_hash)?,
            Err(error) if error.kind() == ErrorKind::NotFound => {
                write_json_bounded(
                    self.cache.state_dir(),
                    &name,
                    &cached,
                    MAX_CACHE_RECORD_BYTES as usize,
                )?;
                cached
            }
            Err(error) => {
                return Err(InspectionError::new(
                    "source_cache_unavailable",
                    error.to_string(),
                ));
            }
        };

        let now = timestamp();
        index.current.insert(
            source_id_value.clone(),
            CachePointer {
                content_hash: content_hash.clone(),
                updated_at: now.clone(),
            },
        );
        index
            .immutable
            .retain(|entry| entry.content_hash != content_hash);
        index.immutable.push(ImmutableRecord {
            source_id: source_id_value,
            content_hash,
            bytes: bytes as u64,
            cached_at: now,
        });
        let obsolete = self.trim_index(&mut index)?;
        // Publish pointers before deleting objects. A crash during garbage
        // collection can leave an orphan, never a dangling durable pointer.
        write_json_bounded(self.cache.state_dir(), INDEX_NAME, &index, MAX_INDEX_BYTES)?;
        for hash in obsolete {
            match self.cache.state_dir().remove_file(cache_name(&hash)) {
                Ok(()) => {}
                Err(error) if error.kind() == ErrorKind::NotFound => {}
                Err(error) => {
                    return Err(InspectionError::new(
                        "source_cache_unavailable",
                        error.to_string(),
                    ));
                }
            }
        }
        let freshness = freshness(previous.as_ref(), &result);
        let previous_identity = previous.as_ref().map(|previous| {
            (
                previous.source_revision.clone(),
                previous.content_hash.clone(),
            )
        });
        let inconsistency = previous.as_ref().and_then(|previous| {
            (previous.source_revision.is_some()
                && previous.source_revision == result.source_revision
                && previous.content_hash != result.content_hash)
                .then(|| {
                    revision_content_diagnostic(
                        "source_provider_revision_inconsistent",
                        "provider returned different content for the same source revision; both immutable records were retained",
                        previous.source_revision.as_deref(),
                        result.source_revision.as_deref(),
                        &previous.content_hash,
                        &result.content_hash,
                        result.source_url.as_deref(),
                    )
                })
        });
        Ok(CacheOutcome {
            cached: result,
            freshness,
            inconsistency,
            previous_identity,
        })
    }

    fn read_index(&self) -> Result<CacheIndex, InspectionError> {
        match self.cache.state_dir().symlink_metadata(INDEX_NAME) {
            Ok(_) => {
                let index: CacheIndex =
                    read_json_bounded(self.cache.state_dir(), INDEX_NAME, MAX_INDEX_BYTES as u64)?;
                if index.schema_version != 1
                    || index.current.len() > MAX_CURRENT
                    || index.immutable.len() > MAX_IMMUTABLE
                    || index.immutable.iter().any(|entry| {
                        !bounded_text(&entry.source_id, 256)
                            || !bounded_text(&entry.content_hash, 128)
                            || entry.bytes > MAX_CACHE_RECORD_BYTES
                    })
                {
                    return Err(InspectionError::new(
                        "source_cache_corrupt",
                        "source cache index exceeds its schema or bounds",
                    ));
                }
                Ok(index)
            }
            Err(error) if error.kind() == ErrorKind::NotFound => Ok(CacheIndex {
                schema_version: 1,
                ..CacheIndex::default()
            }),
            Err(error) => Err(InspectionError::new(
                "source_cache_unavailable",
                error.to_string(),
            )),
        }
    }

    fn read_cached(&self, hash: &str) -> Result<CachedSource, InspectionError> {
        let cached: CachedSource = read_json_bounded(
            self.cache.state_dir(),
            &cache_name(hash),
            MAX_CACHE_RECORD_BYTES,
        )?;
        if cached.schema_version != 1 || cached.content_hash != hash {
            return Err(InspectionError::new(
                "source_cache_corrupt",
                "immutable source cache record does not match its pointer",
            ));
        }
        validate_asset(&SourceAsset {
            source: cached.source.clone(),
            title: cached.title.clone(),
            source_url: cached.source_url.clone(),
            original_url: cached.original_url.clone(),
            source_revision: cached.source_revision.clone(),
            complete: cached.complete,
            diagnostics: cached.diagnostics.clone(),
            body: String::new(),
        })?;
        Ok(cached)
    }

    fn remove_orphaned_records(&self, index: &CacheIndex) -> Result<(), InspectionError> {
        let entries =
            self.cache.state_dir().entries().map_err(|error| {
                InspectionError::new("source_cache_unavailable", error.to_string())
            })?;
        for (count, entry) in entries.enumerate() {
            if count >= 512 {
                return Err(InspectionError::new(
                    "source_cache_full",
                    "source cache directory exceeds the recovery scan limit",
                ));
            }
            let entry = entry.map_err(|error| {
                InspectionError::new("source_cache_unavailable", error.to_string())
            })?;
            let name = entry.file_name();
            let Some(name) = name.to_str() else { continue };
            let Some(hash) = name.strip_suffix(".json") else {
                continue;
            };
            let Some(hex) = hash.strip_prefix("sha256:") else {
                continue;
            };
            if hex.len() != 64 || !hex.bytes().all(|byte| byte.is_ascii_hexdigit()) {
                continue;
            }
            if index
                .immutable
                .iter()
                .any(|record| record.content_hash == hash)
                || index
                    .current
                    .values()
                    .any(|pointer| pointer.content_hash == hash)
            {
                continue;
            }
            // A strict bounded read validates the hash-named cache envelope;
            // symlinks and unrelated files never become cleanup authority.
            self.read_cached(hash)?;
            self.cache.state_dir().remove_file(name).map_err(|error| {
                InspectionError::new("source_cache_unavailable", error.to_string())
            })?;
        }
        Ok(())
    }

    fn trim_index(&self, index: &mut CacheIndex) -> Result<Vec<String>, InspectionError> {
        let mut obsolete = Vec::new();
        while index.current.len() > MAX_CURRENT {
            let id = index
                .current
                .iter()
                .min_by_key(|(_, entry)| &entry.updated_at)
                .map(|(id, _)| id.clone())
                .expect("current nonempty");
            index.current.remove(&id);
        }
        let mut total: u64 = index.immutable.iter().map(|entry| entry.bytes).sum();
        while index.immutable.len() > MAX_IMMUTABLE || total > MAX_IMMUTABLE_BYTES {
            let Some(position) = index
                .immutable
                .iter()
                .enumerate()
                .filter(|(_, entry)| {
                    index
                        .current
                        .get(&entry.source_id)
                        .is_none_or(|current| current.content_hash != entry.content_hash)
                })
                .min_by_key(|(_, entry)| &entry.cached_at)
                .map(|(position, _)| position)
            else {
                return Err(InspectionError::new(
                    "source_cache_full",
                    "current source records exceed cache bounds",
                ));
            };
            let stale = index.immutable.remove(position);
            total = total.saturating_sub(stale.bytes);
            obsolete.push(stale.content_hash);
        }
        Ok(obsolete)
    }
}

fn changed_diagnostic(
    freshness: SourceFreshness,
    previous: Option<&(Option<String>, String)>,
    current: &CachedSource,
) -> Option<ProjectDiagnostic> {
    if freshness != SourceFreshness::Changed {
        return None;
    }
    let Some((previous_revision, previous_content)) = previous else {
        return None;
    };
    if previous_revision.as_deref() == current.source_revision.as_deref()
        && previous_content != &current.content_hash
    {
        return None;
    }
    Some(revision_content_diagnostic(
        "source_provider_changed",
        "provider content changed; the previous immutable record remains available for comparison",
        previous_revision.as_deref(),
        current.source_revision.as_deref(),
        previous_content,
        &current.content_hash,
        current.source_url.as_deref(),
    ))
}

fn revision_content_diagnostic(
    code: &str,
    summary: &str,
    previous_revision: Option<&str>,
    current_revision: Option<&str>,
    previous_content: &str,
    current_content: &str,
    path: Option<&str>,
) -> ProjectDiagnostic {
    let message = format!(
        "{summary}; previous revision={}; current revision={}; previous content={}; current content={}",
        bounded_diagnostic_value(previous_revision),
        bounded_diagnostic_value(current_revision),
        bounded_diagnostic_value(Some(previous_content)),
        bounded_diagnostic_value(Some(current_content)),
    );
    ProjectDiagnostic {
        code: code.to_owned(),
        message: truncate_diagnostic_text(&message),
        path: path.map(truncate_diagnostic_text),
    }
}

fn bounded_diagnostic_value(value: Option<&str>) -> String {
    value
        .map(truncate_diagnostic_text)
        .unwrap_or_else(|| "<none>".to_owned())
}

fn materialization_diagnostic(
    code: &str,
    summary: &str,
    current: &CachedSource,
    previous: Option<&(Option<String>, String)>,
    path: Option<&str>,
) -> ProjectDiagnostic {
    let (previous_revision, previous_content) = previous
        .map(|(revision, content)| (revision.as_deref(), content.as_str()))
        .unwrap_or((None, "<none>"));
    revision_content_diagnostic(
        code,
        summary,
        previous_revision,
        current.source_revision.as_deref(),
        previous_content,
        &current.content_hash,
        path,
    )
}

fn materialization_failure_diagnostic(
    code: &str,
    error: InspectionError,
    path: Option<&str>,
) -> ProjectDiagnostic {
    ProjectDiagnostic {
        code: code.to_owned(),
        message: truncate_diagnostic_text(&error.message),
        path: path.map(truncate_diagnostic_text),
    }
}

fn entry_from_cached(
    cached: &CachedSource,
    freshness: SourceFreshness,
    status: SourceMaterializationStatus,
    relative_path: Option<String>,
) -> SourceEntry {
    SourceEntry {
        source_id: source_id(&cached.source),
        provider_id: cached.source.provider_id.clone(),
        provider_instance: cached.source.provider_instance.clone(),
        resource_type: cached.source.resource_type.clone(),
        canonical_id: cached.source.canonical_id.clone(),
        title: cached.title.clone(),
        source_url: cached.source_url.clone(),
        original_url: cached.original_url.clone(),
        source_revision: cached.source_revision.clone(),
        content_hash: cached.content_hash.clone(),
        freshness,
        status,
        relative_path,
    }
}
fn cached_freshness(cached: &CachedSource) -> SourceFreshness {
    if cached.complete && cached.source_revision.is_some() {
        SourceFreshness::Fresh
    } else {
        SourceFreshness::Unknown
    }
}

fn freshness(previous: Option<&CachedSource>, current: &CachedSource) -> SourceFreshness {
    let Some(previous) = previous else {
        return cached_freshness(current);
    };
    if !previous.complete || !current.complete {
        return SourceFreshness::Unknown;
    }
    match (&previous.source_revision, &current.source_revision) {
        (Some(previous_revision), Some(current_revision))
            if previous_revision == current_revision
                && previous.content_hash == current.content_hash =>
        {
            SourceFreshness::Fresh
        }
        (Some(_), Some(_)) => SourceFreshness::Changed,
        _ => SourceFreshness::Unknown,
    }
}

fn validate_request(request: &SourceFetchRequest) -> Result<(), InspectionError> {
    if !bounded_text(&request.provider_id, 128)
        || !bounded_text(&request.artifact_url, MAX_URL_BYTES)
        || !bounded_text(&request.authority.provider_instance, 512)
        || !bounded_text(&request.authority.origin_host, 256)
        || (!request.authority.origin_base_path.is_empty()
            && !bounded_text(&request.authority.origin_base_path, 512))
        || !bounded_text(&request.authority.owner, 256)
        || !bounded_text(&request.authority.repository, 256)
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
fn default_complete() -> bool {
    true
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

fn canonical_markdown(asset: &SourceAsset) -> String {
    format!(
        "---\nschema_version: 1\nprovider: {}\nresource_type: {}\ncanonical_id: {}\nprovider_instance: {}\nsource_url: {}\noriginal_url: {}\ncomplete: {}\nfetched_at: {}\nsource_revision: {}\ncontent_hash: {}\ngenerated: true\n---\n\n# {}\n\n{}\n",
        yaml_scalar(&asset.source.provider_id),
        yaml_scalar(&asset.source.resource_type),
        yaml_scalar(&asset.source.canonical_id),
        yaml_scalar(&asset.source.provider_instance),
        yaml_scalar(asset.source_url.as_deref().unwrap_or("")),
        yaml_scalar(asset.original_url.as_deref().unwrap_or("")),
        asset.complete,
        yaml_scalar(&timestamp()),
        yaml_scalar(asset.source_revision.as_deref().unwrap_or("")),
        yaml_scalar(&semantic_hash(asset)),
        asset.title,
        asset.body
    )
}
fn yaml_scalar(value: &str) -> String {
    serde_json::to_string(value).expect("string serialization")
}
fn semantic_hash(asset: &SourceAsset) -> String {
    let mut hash = Sha256::new();
    for value in [
        &asset.source.provider_id,
        &asset.source.provider_instance,
        &asset.source.resource_type,
        &asset.source.canonical_id,
        &asset.title,
        asset.source_url.as_deref().unwrap_or(""),
        asset.original_url.as_deref().unwrap_or(""),
        asset.source_revision.as_deref().unwrap_or(""),
        &asset.body,
    ] {
        hash.update(value.as_bytes());
        hash.update([0]);
    }
    hash.update([u8::from(asset.complete)]);
    hash.update([0]);
    hash.update(serde_json::to_vec(&asset.diagnostics).expect("diagnostic serialization"));
    format!("sha256:{:x}", hash.finalize())
}
fn source_id(source: &SourceRef) -> String {
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
fn companion_report_key(companion_id: &str) -> String {
    let mut hash = Sha256::new();
    hash.update(b"companion-hydration-report\0");
    hash.update(companion_id.as_bytes());
    format!("sha256:{:x}", hash.finalize())
}
fn hydration_report_name(primary_source_id: &str, companion_key: Option<&str>) -> String {
    let mut hash = Sha256::new();
    hash.update(b"source-hydration-report\0");
    hash.update(primary_source_id.as_bytes());
    hash.update([0]);
    hash.update(companion_key.unwrap_or("").as_bytes());
    format!("{HYDRATION_REPORT_PREFIX}{:x}.json", hash.finalize())
}
fn bounded_report_diagnostics(diagnostics: &[ProjectDiagnostic]) -> Vec<ProjectDiagnostic> {
    diagnostics
        .iter()
        .map(|diagnostic| ProjectDiagnostic {
            code: diagnostic.code.clone(),
            message: truncate_diagnostic_text(&diagnostic.message),
            path: diagnostic.path.as_deref().map(truncate_diagnostic_text),
        })
        .collect()
}
fn truncate_diagnostic_text(value: &str) -> String {
    let mut end = value.len().min(MAX_HYDRATION_REPORT_DIAGNOSTIC_TEXT);
    while end > 0 && !value.is_char_boundary(end) {
        end -= 1;
    }
    value[..end].to_owned()
}
fn cache_name(hash: &str) -> String {
    format!("{hash}.json")
}
fn serialized_len<T: Serialize>(value: &T) -> Result<usize, InspectionError> {
    serde_json::to_vec(value)
        .map(|bytes| bytes.len())
        .map_err(|error| InspectionError::new("source_cache_write", error.to_string()))
}
fn write_json_bounded<T: Serialize>(
    dir: &Dir,
    name: &str,
    value: &T,
    maximum: usize,
) -> Result<(), InspectionError> {
    if serialized_len(value)? > maximum {
        return Err(InspectionError::new(
            "source_cache_write",
            "serialized source cache data exceeds its bound",
        ));
    }
    atomic_write_json(dir, name, value)
        .map_err(|error| InspectionError::new("source_cache_write", error.to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use cap_std::fs::Dir;
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
        }
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
    async fn refresh_moves_only_the_current_pointer() {
        let (service, shared, root) = service(asset("first"));
        let first = service.fetch(request()).await.expect("first");
        let id = first.entries[0].source_id.clone();
        *shared.lock().expect("asset") = asset("second");
        let second = service
            .refresh_cached(&id, authority(), None)
            .await
            .expect("refresh");
        let listed = service.list_cached().expect("list");
        assert_eq!(listed.len(), 1);
        assert_eq!(listed[0].content_hash, second.entries[0].content_hash);
        assert_ne!(listed[0].content_hash, first.entries[0].content_hash);
        std::fs::remove_dir_all(root).expect("cleanup");
    }
    #[tokio::test]
    async fn same_revision_with_different_content_is_changed_and_retains_evidence() {
        let (service, shared, root) = service(asset("first"));
        let first = service.fetch(request()).await.expect("first");
        *shared.lock().expect("asset") = asset("different body");
        let second = service
            .refresh_cached(&first.entries[0].source_id, authority(), None)
            .await
            .expect("refresh");
        assert_eq!(second.entries[0].freshness, SourceFreshness::Changed);
        let diagnostic = second
            .diagnostics
            .iter()
            .find(|entry| entry.code == "source_provider_revision_inconsistent")
            .expect("revision diagnostic");
        assert!(diagnostic.message.contains("previous revision=1"));
        assert!(diagnostic.message.contains("current revision=1"));
        assert!(diagnostic.message.contains(&format!(
            "previous content={}",
            first.entries[0].content_hash
        )));
        assert!(diagnostic.message.contains(&format!(
            "current content={}",
            second.entries[0].content_hash
        )));
        assert_ne!(
            first.entries[0].content_hash,
            second.entries[0].content_hash
        );
        std::fs::remove_dir_all(root).expect("cleanup");
    }
    #[tokio::test]
    async fn no_revision_is_unknown_in_fetch_and_cached_listing() {
        let mut unknown = asset("body");
        unknown.source_revision = None;
        let (service, _, root) = service(unknown);
        let response = service.fetch(request()).await.expect("fetch");
        assert_eq!(response.entries[0].freshness, SourceFreshness::Unknown);
        assert_eq!(
            service.list_cached().expect("list")[0].freshness,
            SourceFreshness::Unknown
        );
        std::fs::remove_dir_all(root).expect("cleanup");
    }
    #[tokio::test]
    async fn primary_materialization_failure_keeps_cached_entry_and_diagnostic() {
        let (service, _, root) = service(asset("body"));
        let companion =
            Dir::open_ambient_dir(&root, cap_std::ambient_authority()).expect("companion");
        let response = service
            .fetch_to_companion(request(), Some((&companion, "missing-companion")))
            .await
            .expect("provider success is distinct from local materialization failure");
        assert_eq!(response.entries.len(), 1);
        assert_eq!(response.entries[0].freshness, SourceFreshness::Unavailable);
        assert_eq!(
            response.entries[0].status,
            SourceMaterializationStatus::Failed
        );
        assert!(response.entries[0].relative_path.is_none());
        assert!(
            response
                .diagnostics
                .iter()
                .any(|entry| entry.code == "source_materialization_failed")
        );
        assert_eq!(
            service.list_cached().expect("cached provider record").len(),
            1
        );
        drop(companion);
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
            .fetch_to_companion(request(), None)
            .await
            .expect("primary only");
        assert_eq!(primary.entries.len(), 1);
        let hydrated = service
            .fetch_to_companion_hydrated(request(), None, true)
            .await
            .expect("hydrated");
        assert_eq!(
            hydrated
                .entries
                .iter()
                .map(|entry| entry.canonical_id.as_str())
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
        let reports = std::fs::read_dir(root.join("sources"))
            .expect("report directory")
            .filter_map(Result::ok)
            .filter(|entry| {
                entry
                    .file_name()
                    .to_string_lossy()
                    .starts_with(HYDRATION_REPORT_PREFIX)
            })
            .collect::<Vec<_>>();
        assert_eq!(reports.len(), 1);
        let report: HydrationReport =
            serde_json::from_slice(&std::fs::read(reports[0].path()).expect("read report"))
                .expect("valid report");
        assert_eq!(report.primary_source_id, hydrated.entries[0].source_id);
        assert_eq!(report.completed, 1);
        assert_eq!(report.failed, 1);
        assert!(!report.truncated);
        assert!(report.total_bytes > 0);
        std::fs::remove_dir_all(root).expect("cleanup");
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
            .fetch_to_companion_hydrated(request(), None, true)
            .await
            .expect("hydrated");
        assert_eq!(
            hydrated
                .entries
                .iter()
                .map(|entry| entry.canonical_id.as_str())
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
            service.fetch(request()).await.expect_err("instance").code,
            "source_provider_contract"
        );
        *shared.lock().expect("asset") = asset("body");
        shared.lock().expect("asset").title = "x".repeat(MAX_METADATA_BYTES + 1);
        assert_eq!(
            service.fetch(request()).await.expect_err("metadata").code,
            "source_asset_invalid"
        );
        assert!(service.list_cached().expect("empty").is_empty());
        std::fs::remove_dir_all(root).expect("cleanup");
    }
    #[tokio::test]
    async fn concurrent_host_import_is_busy_until_publication_lease_releases() {
        let (service, _, root) = service(asset("body"));
        let lease = service
            .cache
            .try_acquire_named_lock(IMPORT_LOCK_NAME, "test")
            .expect("lease")
            .expect("available");
        assert_eq!(
            service.fetch(request()).await.expect_err("busy").code,
            "source_import_busy"
        );
        assert!(service.list_cached().expect("unchanged").is_empty());
        drop(lease);
        assert_eq!(
            service
                .fetch(request())
                .await
                .expect("released")
                .entries
                .len(),
            1
        );
        std::fs::remove_dir_all(root).expect("cleanup");
    }

    #[tokio::test]
    async fn interrupted_unindexed_cache_record_is_reclaimed_on_next_write() {
        let (service, shared, root) = service(asset("orphan"));
        let first = service.fetch(request()).await.expect("first");
        let hash = &first.entries[0].content_hash;
        write_json_bounded(
            service.cache.state_dir(),
            INDEX_NAME,
            &CacheIndex {
                schema_version: 1,
                ..CacheIndex::default()
            },
            MAX_INDEX_BYTES,
        )
        .expect("simulate interrupted pointer publication");
        *shared.lock().expect("asset") = asset("new");
        service.fetch(request()).await.expect("next write");
        assert!(!service.cache.state_dir().exists(cache_name(hash)));
        assert_eq!(service.list_cached().expect("current").len(), 1);
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
        let result = tokio::time::timeout(Duration::from_secs(1), service.fetch(request()))
            .await
            .expect("bounded request");
        assert_eq!(result.expect_err("deadline").code, "source_fetch_timeout");
        assert!(
            service
                .list_cached()
                .expect("no incomplete cache")
                .is_empty()
        );
        std::fs::remove_dir_all(root).expect("cleanup");
    }

    #[tokio::test]
    async fn eviction_planning_keeps_the_previous_durable_pointer_readable() {
        let (service, _, root) = service(asset("previous"));
        let first = service.fetch(request()).await.expect("first");
        let hash = first.entries[0].content_hash.clone();
        let mut pending = service.read_index().expect("durable index");
        pending.current.clear();
        for number in 0..MAX_IMMUTABLE {
            pending.immutable.push(ImmutableRecord {
                source_id: format!("future-{number}"),
                content_hash: format!("future-{number}"),
                bytes: 1,
                cached_at: "z".into(),
            });
        }
        let obsolete = service.trim_index(&mut pending).expect("plan eviction");
        assert_eq!(obsolete, vec![hash.clone()]);
        // Simulate interruption before publishing the pending index. The
        // original durable pointer and its immutable bytes must still work.
        assert_eq!(
            service.list_cached().expect("restart list")[0].content_hash,
            hash
        );
        assert_eq!(
            service.read_cached(&hash).expect("old object").title,
            "issue"
        );
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
