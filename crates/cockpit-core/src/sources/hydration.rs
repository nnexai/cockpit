use std::{
    collections::{BTreeSet, VecDeque},
    sync::Arc,
    time::{Duration, Instant},
};

use cockpit_protocol::projects::ProjectDiagnostic;
use tokio::time::timeout;
use url::Url;

use super::{SourceAsset, SourceAuthority, SourceFetchRequest, SourceProvider};

pub(crate) const HYDRATION_MAX_DEPTH: u32 = 2;
pub(crate) const HYDRATION_MAX_ASSETS: usize = 32;
pub(crate) const HYDRATION_MAX_TOTAL_BYTES: usize = 4 * 1024 * 1024;
pub(crate) const HYDRATION_TIMEOUT: Duration = Duration::from_secs(10);
const MAX_REFERENCES_PER_ASSET: usize = 16;
const MAX_DIAGNOSTICS: usize = 32;
const MAX_REFERENCE_URL_BYTES: usize = 8 * 1024;

pub(crate) struct HydrationResult {
    pub assets: Vec<SourceAsset>,
    pub diagnostics: Vec<ProjectDiagnostic>,
    pub completed: u32,
    pub skipped: u32,
    pub failed: u32,
    pub truncated: bool,
    pub total_bytes: u64,
}

#[derive(Debug)]
struct QueuedReference {
    canonical_id: String,
    artifact_url: String,
    depth: u32,
}

pub(crate) async fn hydrate(
    provider: &Arc<dyn SourceProvider>,
    request: &SourceFetchRequest,
    primary: Vec<SourceAsset>,
    started: Instant,
    deadline_budget: Duration,
) -> HydrationResult {
    let primary_count = primary.len();
    let mut assets = primary;
    let mut diagnostics = Vec::new();
    let mut seen: BTreeSet<String> = assets
        .iter()
        .map(|asset| asset.source.canonical_id.clone())
        .collect();
    let mut scheduled = BTreeSet::new();
    let mut queue = VecDeque::new();
    let mut total_bytes: usize = assets.iter().map(|asset| asset.body.len()).sum();
    for asset in &assets {
        enqueue_references(
            &mut queue,
            &seen,
            &mut scheduled,
            &mut diagnostics,
            asset,
            &request.authority,
            1,
        );
    }

    while let Some(reference) = queue.pop_front() {
        if reference.depth > HYDRATION_MAX_DEPTH {
            diagnostic(
                &mut diagnostics,
                "source_hydration_depth",
                "reference depth limit reached",
                None,
            );
            continue;
        }
        if assets.len() >= HYDRATION_MAX_ASSETS {
            diagnostic(
                &mut diagnostics,
                "source_hydration_assets",
                "reference asset limit reached",
                None,
            );
            break;
        }
        if total_bytes >= HYDRATION_MAX_TOTAL_BYTES {
            diagnostic(
                &mut diagnostics,
                "source_hydration_bytes",
                "reference byte limit reached",
                None,
            );
            break;
        }
        let Some(remaining) = deadline_budget.checked_sub(started.elapsed()) else {
            diagnostic(
                &mut diagnostics,
                "source_hydration_timeout",
                "reference hydration timed out",
                None,
            );
            break;
        };
        let follow = SourceFetchRequest {
            provider_id: request.provider_id.clone(),
            artifact_url: reference.artifact_url.clone(),
            authority: request.authority.clone(),
        };
        let fetched = match timeout(remaining, provider.fetch(&follow)).await {
            Ok(Ok(fetched)) => fetched,
            Ok(Err(error)) => {
                diagnostic(
                    &mut diagnostics,
                    "source_hydration_fetch_failed",
                    &error.message,
                    Some(&reference.artifact_url),
                );
                continue;
            }
            Err(_) => {
                diagnostic(
                    &mut diagnostics,
                    "source_hydration_timeout",
                    "reference hydration timed out",
                    Some(&reference.artifact_url),
                );
                break;
            }
        };
        if fetched.is_empty() || fetched.len() > HYDRATION_MAX_ASSETS {
            diagnostic(
                &mut diagnostics,
                "source_hydration_provider_contract",
                "reference provider returned an invalid asset set",
                Some(&reference.artifact_url),
            );
            continue;
        }
        if fetched.iter().any(|asset| {
            asset.source.provider_id != request.provider_id
                || asset.source.provider_instance != request.authority.provider_instance
        }) {
            diagnostic(
                &mut diagnostics,
                "source_hydration_provider_contract",
                "reference provider changed the configured authority",
                Some(&reference.artifact_url),
            );
            continue;
        }
        if !fetched
            .iter()
            .any(|asset| asset.source.canonical_id == reference.canonical_id)
        {
            diagnostic(
                &mut diagnostics,
                "source_hydration_provider_contract",
                "reference provider did not return the requested canonical source",
                Some(&reference.artifact_url),
            );
            continue;
        }
        scheduled.remove(&reference.canonical_id);
        for asset in fetched {
            if assets.len() >= HYDRATION_MAX_ASSETS
                || total_bytes.saturating_add(asset.body.len()) > HYDRATION_MAX_TOTAL_BYTES
            {
                diagnostic(
                    &mut diagnostics,
                    "source_hydration_truncated",
                    "reference hydration budget reached",
                    Some(&reference.artifact_url),
                );
                break;
            }
            let canonical = asset.source.canonical_id.clone();
            if !seen.insert(canonical.clone()) {
                diagnostic(
                    &mut diagnostics,
                    "source_hydration_cycle",
                    "reference graph repeats a canonical source",
                    Some(&reference.artifact_url),
                );
                continue;
            }
            if let Err(error) = super::validate_provider_asset(request, &asset)
                .and_then(|()| super::validate_asset(&asset))
            {
                diagnostic(
                    &mut diagnostics,
                    "source_hydration_provider_contract",
                    &error.message,
                    Some(&reference.artifact_url),
                );
                continue;
            }
            total_bytes += asset.body.len();
            enqueue_references(
                &mut queue,
                &seen,
                &mut scheduled,
                &mut diagnostics,
                &asset,
                &request.authority,
                reference.depth + 1,
            );
            assets.push(asset);
        }
    }
    HydrationResult {
        completed: assets.len().saturating_sub(primary_count) as u32,
        skipped: diagnostics
            .iter()
            .filter(|diagnostic| {
                matches!(
                    diagnostic.code.as_str(),
                    "source_hydration_cycle"
                        | "source_hydration_authority_refused"
                        | "source_hydration_unsupported_reference"
                        | "source_hydration_reference_invalid"
                        | "source_hydration_references"
                )
            })
            .count() as u32,
        failed: diagnostics
            .iter()
            .filter(|diagnostic| {
                matches!(
                    diagnostic.code.as_str(),
                    "source_hydration_fetch_failed" | "source_hydration_provider_contract"
                )
            })
            .count() as u32,
        truncated: diagnostics.iter().any(|diagnostic| {
            matches!(
                diagnostic.code.as_str(),
                "source_hydration_depth"
                    | "source_hydration_assets"
                    | "source_hydration_bytes"
                    | "source_hydration_timeout"
                    | "source_hydration_truncated"
                    | "source_hydration_references"
            )
        }),
        total_bytes: total_bytes as u64,
        assets,
        diagnostics,
    }
}

fn enqueue_references(
    queue: &mut VecDeque<QueuedReference>,
    seen: &BTreeSet<String>,
    scheduled: &mut BTreeSet<String>,
    diagnostics: &mut Vec<ProjectDiagnostic>,
    asset: &SourceAsset,
    authority: &SourceAuthority,
    depth: u32,
) {
    for reference in recognized_references(asset, authority, diagnostics) {
        if seen.contains(&reference.canonical_id)
            || !scheduled.insert(reference.canonical_id.clone())
        {
            diagnostic(
                diagnostics,
                "source_hydration_cycle",
                "reference graph repeats a canonical source",
                Some(&reference.artifact_url),
            );
            continue;
        }
        queue.push_back(QueuedReference {
            canonical_id: reference.canonical_id,
            artifact_url: reference.artifact_url,
            depth,
        });
    }
}

#[derive(Debug)]
struct SourceReference {
    canonical_id: String,
    artifact_url: String,
}

fn recognized_references(
    asset: &SourceAsset,
    authority: &SourceAuthority,
    diagnostics: &mut Vec<ProjectDiagnostic>,
) -> Vec<SourceReference> {
    let mut references = Vec::new();
    for candidate in url_candidates(&asset.body) {
        if references.len() >= MAX_REFERENCES_PER_ASSET {
            diagnostic(
                diagnostics,
                "source_hydration_references",
                "reference count limit reached for a source asset",
                None,
            );
            break;
        }
        if candidate.len() > MAX_REFERENCE_URL_BYTES {
            diagnostic(
                diagnostics,
                "source_hydration_reference_invalid",
                "reference URL exceeds the configured bound",
                None,
            );
            continue;
        }
        match parse_reference(&candidate, authority) {
            Ok(Some(reference)) => references.push(reference),
            Ok(None) => diagnostic(
                diagnostics,
                "source_hydration_unsupported_reference",
                "same-repository reference is not a supported source artifact",
                Some(&candidate),
            ),
            Err(()) => diagnostic(
                diagnostics,
                "source_hydration_authority_refused",
                "reference is outside the configured primary repository authority",
                Some(&candidate),
            ),
        }
    }
    references
}

/// Finds absolute HTTP(S) URLs in plain text and Markdown destinations. The
/// provider remains responsible for fetching them; this scanner only exposes
/// candidates to the exact-authority parser below.
fn url_candidates(body: &str) -> Vec<String> {
    const MAX_CANDIDATES: usize = MAX_REFERENCES_PER_ASSET * 2;
    let mut candidates = Vec::new();
    let mut offset = 0;
    while offset < body.len() && candidates.len() < MAX_CANDIDATES {
        let remaining = &body[offset..];
        let http = remaining.find("http://");
        let https = remaining.find("https://");
        let Some(relative) = (match (http, https) {
            (Some(http), Some(https)) => Some(http.min(https)),
            (Some(found), None) | (None, Some(found)) => Some(found),
            (None, None) => None,
        }) else {
            break;
        };
        let start = offset + relative;
        let tail = &body[start..];
        let end = tail
            .find(|character: char| {
                character.is_whitespace()
                    || matches!(character, '<' | '>' | '"' | '\'' | '`' | ']' | ')')
            })
            .unwrap_or(tail.len());
        let candidate = tail[..end]
            .trim_end_matches(|character: char| matches!(character, ',' | '.' | ';' | ':'));
        if !candidate.is_empty() {
            candidates.push(candidate.to_owned());
        }
        offset = start.saturating_add(end.max(1));
    }
    candidates
}

fn parse_reference(
    value: &str,
    authority: &SourceAuthority,
) -> Result<Option<SourceReference>, ()> {
    let mut url = Url::parse(value).map_err(|_| ())?;
    if !matches!(url.scheme(), "http" | "https")
        || url.username() != ""
        || url.password().is_some()
        || url.host_str().map(str::to_ascii_lowercase).as_deref()
            != Some(authority.origin_host.to_ascii_lowercase().as_str())
        || url.port().or_else(|| match url.scheme() {
            "http" => Some(80),
            "https" => Some(443),
            _ => None,
        }) != authority.origin_port.or_else(|| {
            if authority.provider_instance.starts_with("https://") {
                Some(443)
            } else {
                Some(80)
            }
        })
    {
        return Err(());
    }
    url.set_fragment(None);
    url.set_query(None);
    let prefix = format!(
        "{}/{}/",
        authority.origin_base_path.trim_end_matches('/'),
        authority.owner
    );
    let expected = format!("{}{}", prefix, authority.repository);
    let Some(tail) = url.path().strip_prefix(&format!("{expected}/")) else {
        return Err(());
    };
    let mut parts = tail.split('/');
    let kind = parts.next().unwrap_or("");
    let value = parts.next().unwrap_or("");
    if value.is_empty() || parts.next().is_some() {
        return Ok(None);
    }
    let canonical_id = match kind {
        "issues" if value.parse::<u64>().is_ok() => {
            format!("{}/{}#{}", authority.owner, authority.repository, value)
        }
        "pulls" if value.parse::<u64>().is_ok() => {
            format!("{}/{}!{}", authority.owner, authority.repository, value)
        }
        "wiki"
            if value
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-' | b'.')) =>
        {
            format!("{}/{}:{}", authority.owner, authority.repository, value)
        }
        _ => return Ok(None),
    };
    Ok(Some(SourceReference {
        canonical_id,
        artifact_url: url.into(),
    }))
}

fn diagnostic(
    diagnostics: &mut Vec<ProjectDiagnostic>,
    code: &str,
    message: &str,
    path: Option<&str>,
) {
    if diagnostics.len() < MAX_DIAGNOSTICS {
        diagnostics.push(ProjectDiagnostic {
            code: code.to_owned(),
            message: message.to_owned(),
            path: path.map(str::to_owned),
        });
    }
}
