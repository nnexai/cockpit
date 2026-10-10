//! Reference extraction and bounded breadth-first traversal of related source
//! items. This module only reads; the Library decides what to persist.

use std::collections::BTreeSet;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use cockpit_protocol::projects::{ProjectConfiguration, ProviderKind};
use serde::{Deserialize, Serialize};
use tokio::task::JoinSet;
use tokio::time::timeout;

use super::{
    ConfluencePage, FrontmatterValue, ProviderResolution, SourceAsset, SourceAuthority,
    SourceFetchRequest, SourceProvider, SourceRef, SourceService, confluence_instance_authority,
    confluence_page_url, instance_authority, site_authority, source_id, validate_asset,
    validate_confluence_page, validate_provider_asset, validate_request,
};
use crate::InspectionError;
use crate::repositories::{
    confluence_provider_for_input, is_jira_key, jira_artifact, resolve_artifact,
};

pub const MAX_REFERENCE_DEPTH: u32 = 5;
/// References kept per asset. A node that reaches this count may have had
/// more, so traversal reports it as incomplete instead of hiding the rest.
pub const MAX_ASSET_REFERENCES: usize = 64;
const MAX_REFERENCE_URL_BYTES: usize = 8 * 1024;
const FETCHES_IN_FLIGHT: usize = 8;
const TRUNCATED_CODE: &str = "source_references_truncated";

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ReferenceTarget {
    JiraKey {
        provider_id: String,
        key: String,
    },
    Url {
        url: String,
    },
    /// Marks a reference list that was cut short; never a real target.
    Truncated,
}

impl ReferenceTarget {
    fn display(&self) -> String {
        match self {
            Self::JiraKey { key, .. } => key.clone(),
            Self::Url { url } => url.clone(),
            Self::Truncated => String::new(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SourceReference {
    pub target: ReferenceTarget,
    /// Jira link text ("blocks", "is blocked by"), "parent", "subtask",
    /// "description", "comment" or "body".
    pub relation: String,
}

struct Collector<'a> {
    items: Vec<SourceReference>,
    seen: BTreeSet<ReferenceTarget>,
    own_url: Option<&'a str>,
    own_key: Option<&'a str>,
    truncated: bool,
    provider_id: &'a str,
}

impl Collector<'_> {
    fn push(&mut self, target: ReferenceTarget, relation: &str) {
        let own = match &target {
            ReferenceTarget::JiraKey { provider_id, key } => {
                provider_id == self.provider_id && Some(key.as_str()) == self.own_key
            }
            ReferenceTarget::Url { url } => self
                .own_url
                .is_some_and(|own| without_query_and_fragment(url) == own),
            ReferenceTarget::Truncated => false,
        };
        if own || !self.seen.insert(target.clone()) {
            return;
        }
        if self.items.len() >= MAX_ASSET_REFERENCES {
            self.truncated = true;
            return;
        }
        self.items.push(SourceReference {
            target,
            relation: relation.to_owned(),
        });
    }

    /// The kept references, followed by a marker when any were left out so the
    /// stored list still says it is incomplete.
    fn finish(mut self) -> Vec<SourceReference> {
        if self.truncated {
            self.items.push(SourceReference {
                target: ReferenceTarget::Truncated,
                relation: TRUNCATION_RELATION.into(),
            });
        }
        self.items
    }
}

const TRUNCATION_RELATION: &str = "truncated";

fn is_truncation_marker(reference: &SourceReference) -> bool {
    matches!(reference.target, ReferenceTarget::Truncated)
}

/// True when the list was cut short: it carries the truncation marker or is
/// at the cap, so more may exist than were kept.
pub fn references_truncated(references: &[SourceReference]) -> bool {
    references.len() >= MAX_ASSET_REFERENCES || references.iter().any(is_truncation_marker)
}

fn without_query_and_fragment(url: &str) -> &str {
    url.split(['#', '?']).next().unwrap_or(url)
}

fn is_jira_issue(configuration: &ProjectConfiguration, asset: &SourceAsset) -> bool {
    asset.source.resource_type == "issue"
        && configuration.providers.iter().any(|provider| {
            provider.id == asset.source.provider_id && provider.kind == ProviderKind::Jira
        })
}

/// Structured Jira fields plus text references, deduplicated by target, in
/// this order: structured fields, description, comments; non-Jira bodies give
/// URLs only. Capped at [`MAX_ASSET_REFERENCES`].
pub fn asset_references(
    configuration: &ProjectConfiguration,
    asset: &SourceAsset,
) -> Vec<SourceReference> {
    let jira = is_jira_issue(configuration, asset);
    let own_url = asset.source_url.as_deref().map(without_query_and_fragment);
    let mut collector = Collector {
        items: Vec::new(),
        truncated: false,
        seen: BTreeSet::new(),
        own_url,
        own_key: jira.then_some(asset.source.canonical_id.as_str()),
        provider_id: &asset.source.provider_id,
    };
    // A provider answer that is not complete (for example a partial comment
    // window) may have lost references; keep the stored list marked incomplete.
    collector.truncated = !asset.complete;
    if !jira {
        scan(&asset.body, None, "body", &mut collector);
        return collector.finish();
    }
    let provider_id = asset.source.provider_id.as_str();
    let key_target = |key: &str| ReferenceTarget::JiraKey {
        provider_id: provider_id.to_owned(),
        key: key.to_owned(),
    };
    for field in &asset.fields {
        match (field.key.as_str(), &field.value) {
            ("parent", FrontmatterValue::String(key)) if is_jira_key(key) => {
                collector.push(key_target(key), "parent");
            }
            ("subtasks", FrontmatterValue::Strings(keys)) => {
                for key in keys.iter().filter(|key| is_jira_key(key)) {
                    collector.push(key_target(key), "subtask");
                }
            }
            ("links", FrontmatterValue::Strings(links)) => {
                for link in links {
                    if let Some((relation, key)) = link.rsplit_once(' ')
                        && is_jira_key(key)
                    {
                        collector.push(key_target(key), relation);
                    }
                }
            }
            ("references_truncated", FrontmatterValue::Boolean(true)) => collector.truncated = true,
            _ => {}
        }
    }
    enum Section {
        Other,
        Description,
        Comments,
    }
    let mut section = Section::Other;
    for line in asset.body.split_inclusive('\n') {
        if line.starts_with("## ") {
            section = if line.starts_with("## Description") {
                Section::Description
            } else if line.starts_with("## Comments") {
                Section::Comments
            } else {
                Section::Other
            };
            continue;
        }
        match section {
            Section::Other => {}
            Section::Description => scan(line, Some(provider_id), "description", &mut collector),
            Section::Comments => scan(line, Some(provider_id), "comment", &mut collector),
        }
    }
    collector.finish()
}

/// Jira issue: its key; anything else: its title.
pub fn asset_label(asset: &SourceAsset) -> String {
    if asset.source.resource_type == "issue" && is_jira_key(&asset.source.canonical_id) {
        asset.source.canonical_id.clone()
    } else {
        asset.title.clone()
    }
}

/// Finds absolute HTTP(S) URLs and, when `jira` names a provider, bare Jira
/// keys outside URLs. A key needs a clean boundary on both sides: not glued to
/// `[A-Za-z0-9_\-./]` before, not followed by `[A-Za-z0-9_]` or `-<alnum>`.
fn scan(text: &str, jira: Option<&str>, relation: &str, out: &mut Collector<'_>) {
    let bytes = text.as_bytes();
    let mut index = 0;
    while index < bytes.len() {
        let byte = bytes[index];
        if byte == b'h'
            && (text[index..].starts_with("http://") || text[index..].starts_with("https://"))
        {
            let tail = &text[index..];
            let end = tail
                .find(|character: char| {
                    character.is_whitespace()
                        || matches!(character, '<' | '>' | '"' | '\'' | '`' | ']' | ')')
                })
                .unwrap_or(tail.len());
            let candidate = tail[..end]
                .trim_end_matches(|character: char| matches!(character, ',' | '.' | ';' | ':'));
            if candidate.len() > "https://".len() && candidate.len() <= MAX_REFERENCE_URL_BYTES {
                out.push(
                    ReferenceTarget::Url {
                        url: candidate.to_owned(),
                    },
                    relation,
                );
            }
            index += end.max(1);
            continue;
        }
        if let Some(provider_id) = jira
            && byte.is_ascii_uppercase()
            && (index == 0
                || !matches!(bytes[index - 1], b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'_' | b'-' | b'.' | b'/'))
        {
            let mut project_end = index;
            while project_end < bytes.len()
                && (bytes[project_end].is_ascii_uppercase()
                    || bytes[project_end].is_ascii_digit()
                    || bytes[project_end] == b'_')
            {
                project_end += 1;
            }
            if bytes.get(project_end) == Some(&b'-') {
                let mut end = project_end + 1;
                while end < bytes.len() && bytes[end].is_ascii_digit() {
                    end += 1;
                }
                let clean_end = match bytes.get(end) {
                    Some(next) if next.is_ascii_alphanumeric() || *next == b'_' => false,
                    Some(b'-') => !bytes.get(end + 1).is_some_and(u8::is_ascii_alphanumeric),
                    _ => true,
                };
                if end > project_end + 1 && clean_end && is_jira_key(&text[index..end]) {
                    out.push(
                        ReferenceTarget::JiraKey {
                            provider_id: provider_id.to_owned(),
                            key: text[index..end].to_owned(),
                        },
                        relation,
                    );
                    index = end;
                    continue;
                }
            }
        }
        index += 1;
    }
}

#[derive(Debug, Clone)]
pub struct ReferenceSeed {
    pub source: SourceRef,
    pub label: String,
    pub references: Vec<SourceReference>,
}

#[derive(Debug, Clone, Copy)]
pub enum TraversalBudget {
    Single,
    Query,
}

struct Limits {
    items: usize,
    bytes: usize,
    time: Duration,
}

impl TraversalBudget {
    fn limits(self) -> Limits {
        match self {
            Self::Single => Limits {
                items: 32,
                bytes: 8 * 1024 * 1024,
                time: Duration::from_secs(60),
            },
            Self::Query => Limits {
                items: 100,
                bytes: 16 * 1024 * 1024,
                time: Duration::from_secs(180),
            },
        }
    }
}

#[derive(Debug, Clone)]
pub struct RelatedAsset {
    /// Validated like `fetch_assets` output.
    pub asset: SourceAsset,
    /// The fetch request's artifact URL, for the index entry's canonical URL.
    pub canonical_url: String,
    /// 1..=depth.
    pub depth: u32,
    /// The node whose references reached it (shallowest, first in BFS order).
    pub from: SourceRef,
    pub from_label: String,
    pub relation: String,
    /// `asset_references` of `asset`.
    pub references: Vec<SourceReference>,
}

#[derive(Debug, Clone)]
pub struct RelatedFailure {
    /// A Jira key or URL.
    pub target: String,
    pub from_label: String,
    pub depth: u32,
    pub code: String,
    pub message: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TraversalStop {
    Items,
    Bytes,
    Time,
    Cancelled,
}

#[derive(Debug, Clone)]
pub struct RelatedResult {
    pub assets: Vec<RelatedAsset>,
    pub failures: Vec<RelatedFailure>,
    pub stopped: Option<TraversalStop>,
}

impl RelatedResult {
    pub fn complete(&self) -> bool {
        self.failures.is_empty() && self.stopped.is_none()
    }
}

/// A node whose references are followed at the next level.
struct Node {
    source: SourceRef,
    label: String,
    references: Vec<SourceReference>,
}

struct Prepared {
    request: SourceFetchRequest,
    expected: SourceRef,
}

enum Classified {
    Ready(Prepared),
    /// A URL of a configured Confluence provider; needs the provider to say
    /// whether it is a page.
    Confluence {
        provider_id: String,
        url: String,
    },
    /// Not a supported source; stays an ordinary link.
    Link,
    Failed {
        code: String,
        message: String,
    },
}

struct Pending {
    target: ReferenceTarget,
    from: usize,
    relation: String,
}

struct Candidate {
    prepared: Prepared,
    from: usize,
    relation: String,
}

fn failure(code: &str, message: impl Into<String>) -> Classified {
    Classified::Failed {
        code: code.into(),
        message: message.into(),
    }
}

fn pending_references(
    frontier: &[Node],
    seen_targets: &mut BTreeSet<ReferenceTarget>,
    level: u32,
    result: &mut RelatedResult,
) -> Vec<Pending> {
    let mut pending = Vec::new();
    for (from, node) in frontier.iter().enumerate() {
        if references_truncated(&node.references) {
            result.failures.push(RelatedFailure {
                target: node.label.clone(),
                from_label: node.label.clone(),
                depth: level - 1,
                code: TRUNCATED_CODE.into(),
                message: "references are incomplete or truncated; some related items may be missing".into(),
            });
        }
        for reference in node
            .references
            .iter()
            .filter(|reference| !is_truncation_marker(reference))
        {
            if seen_targets.insert(reference.target.clone()) {
                pending.push(Pending {
                    target: reference.target.clone(),
                    from,
                    relation: reference.relation.clone(),
                });
            }
        }
    }
    pending
}

impl SourceService {
    /// Breadth-first, level by level, with 8 fetches in flight. Seeds are
    /// marked visited and never fetched. Identity is (provider_id,
    /// provider_instance, resource_type, canonical_id), the Library item
    /// identity. `depth == 0` returns an empty result.
    pub async fn collect_related(
        &self,
        seeds: Vec<ReferenceSeed>,
        depth: u32,
        budget: TraversalBudget,
        cancel: &AtomicBool,
    ) -> RelatedResult {
        let mut result = RelatedResult {
            assets: Vec::new(),
            failures: Vec::new(),
            stopped: None,
        };
        let depth = depth.min(MAX_REFERENCE_DEPTH);
        if depth == 0 {
            return result;
        }
        let limits = budget.limits();
        let time = limits.time.saturating_mul(
            if super::lane::current() == super::lane::RequestLane::Background { 10 } else { 1 },
        );
        let deadline = Instant::now() + time;
        let mut visited: BTreeSet<String> =
            seeds.iter().map(|seed| source_id(&seed.source)).collect();
        let mut seen_targets = BTreeSet::new();
        let mut bytes = 0usize;
        let mut frontier: Vec<Node> = seeds
            .into_iter()
            .map(|seed| Node {
                source: seed.source,
                label: seed.label,
                references: seed.references,
            })
            .collect();
        for level in 1..=depth {
            if frontier.is_empty() {
                break;
            }
            let pending = pending_references(&frontier, &mut seen_targets, level, &mut result);
            if self.stop_requested(cancel, deadline, &mut result) {
                break;
            }
            let Some(mut candidates) = self
                .resolve_pending(
                    &pending, &frontier, level, &mut visited, deadline, cancel, &mut result,
                )
                .await
            else {
                break;
            };
            let room = limits.items.saturating_sub(result.assets.len());
            let items_limited = candidates.len() > room;
            candidates.truncate(room);
            let mut next = Vec::new();
            for chunk in candidates.chunks(FETCHES_IN_FLIGHT) {
                if self.stop_requested(cancel, deadline, &mut result) {
                    break;
                }
                let fetched = self.fetch_related_chunk(chunk, deadline).await;
                for (candidate, outcome) in chunk.iter().zip(fetched) {
                    let from = &frontier[candidate.from];
                    let target = candidate.prepared.request.artifact_url.clone();
                    let target = match candidate.prepared.expected.resource_type.as_str() {
                        "issue" if is_jira_key(&candidate.prepared.expected.canonical_id) => {
                            candidate.prepared.expected.canonical_id.clone()
                        }
                        _ => target,
                    };
                    let error = match outcome {
                        Some(Ok(asset)) => {
                            if bytes.saturating_add(asset.body.len()) > limits.bytes {
                                result.stopped = Some(TraversalStop::Bytes);
                                break;
                            }
                            bytes += asset.body.len();
                            let references = asset_references(&self.configuration, &asset);
                            next.push(Node {
                                source: asset.source.clone(),
                                label: asset_label(&asset),
                                references: references.clone(),
                            });
                            result.assets.push(RelatedAsset {
                                canonical_url: candidate.prepared.request.artifact_url.clone(),
                                depth: level,
                                from: from.source.clone(),
                                from_label: from.label.clone(),
                                relation: candidate.relation.clone(),
                                references,
                                asset,
                            });
                            continue;
                        }
                        Some(Err(error)) => error,
                        None => InspectionError::new(
                            "source_reference_failed",
                            "reference fetch did not complete",
                        ),
                    };
                    if error.code == "source_fetch_timeout" && Instant::now() >= deadline {
                        result.stopped = Some(TraversalStop::Time);
                        break;
                    }
                    result.failures.push(RelatedFailure {
                        target,
                        from_label: from.label.clone(),
                        depth: level,
                        code: error.code,
                        message: error.message,
                    });
                }
                if result.stopped.is_some() {
                    break;
                }
            }
            if items_limited {
                result.stopped.get_or_insert(TraversalStop::Items);
            }
            if result.stopped.is_some() {
                break;
            }
            frontier = next;
        }
        result
    }

    /// Resolve in bounded batches, then classify in the original reference order.
    async fn resolve_pending(
        &self,
        pending: &[Pending],
        frontier: &[Node],
        level: u32,
        visited: &mut BTreeSet<String>,
        deadline: Instant,
        cancel: &AtomicBool,
        result: &mut RelatedResult,
    ) -> Option<Vec<Candidate>> {
        let mut outcomes: Vec<Option<Classified>> = pending
            .iter()
            .map(|pending| Some(self.classify(&pending.target)))
            .collect();
        let confluence: Vec<usize> = outcomes
            .iter()
            .enumerate()
            .filter(|(_, outcome)| matches!(outcome, Some(Classified::Confluence { .. })))
            .map(|(index, _)| index)
            .collect();
        let mut aborted = false;
        for chunk in confluence.chunks(FETCHES_IN_FLIGHT) {
            if self.stop_requested(cancel, deadline, result) {
                aborted = true;
                break;
            }
            let mut set = JoinSet::new();
            for &index in chunk {
                let Some(Classified::Confluence { provider_id, url }) = outcomes[index].take()
                else {
                    continue;
                };
                let service = self.clone();
                set.spawn(super::lane::inherit(async move {
                    (index, service.resolve_confluence(&provider_id, &url).await)
                }));
            }
            while let Some(joined) = set.join_next().await {
                if let Ok((index, classified)) = joined {
                    outcomes[index] = Some(classified);
                }
            }
            for &index in chunk {
                if outcomes[index].is_none() {
                    outcomes[index] = Some(failure(
                        "source_reference_failed",
                        "reference resolution did not complete",
                    ));
                }
            }
        }
        if aborted {
            return None;
        }
        let mut candidates = Vec::new();
        for (index, pending) in pending.iter().enumerate() {
            match outcomes[index].take() {
                Some(Classified::Ready(prepared)) => {
                    if visited.insert(source_id(&prepared.expected)) {
                        candidates.push(Candidate {
                            prepared,
                            from: pending.from,
                            relation: pending.relation.clone(),
                        });
                    }
                }
                Some(Classified::Failed { code, message }) => {
                    result.failures.push(RelatedFailure {
                        target: pending.target.display(),
                        from_label: frontier[pending.from].label.clone(),
                        depth: level,
                        code,
                        message,
                    });
                }
                _ => {}
            }
        }
        Some(candidates)
    }

    /// Join a fetch batch into stable slots, independent of completion order.
    async fn fetch_related_chunk(
        &self,
        chunk: &[Candidate],
        deadline: Instant,
    ) -> Vec<Option<Result<SourceAsset, InspectionError>>> {
        let per_call = self.deadline()
            .min(deadline.saturating_duration_since(Instant::now()));
        let mut set = JoinSet::new();
        for (slot, candidate) in chunk.iter().enumerate() {
            let Ok(provider) = self
                .selected_provider(&candidate.prepared.request.provider_id)
                .cloned()
            else {
                continue;
            };
            let request = candidate.prepared.request.clone();
            let expected = candidate.prepared.expected.clone();
            set.spawn(super::lane::inherit(async move {
                (
                    slot,
                    fetch_related(provider, request, expected, per_call).await,
                )
            }));
        }
        let mut fetched: Vec<Option<Result<SourceAsset, InspectionError>>> =
            chunk.iter().map(|_| None).collect();
        while let Some(joined) = set.join_next().await {
            if let Ok((slot, outcome)) = joined {
                fetched[slot] = Some(outcome);
            }
        }
        fetched
    }

    /// Records the stop reason when the operation was cancelled or ran out of time.
    fn stop_requested(
        &self,
        cancel: &AtomicBool,
        deadline: Instant,
        result: &mut RelatedResult,
    ) -> bool {
        if cancel.load(Ordering::Relaxed) {
            result.stopped = Some(TraversalStop::Cancelled);
        } else if Instant::now() >= deadline {
            result.stopped = Some(TraversalStop::Time);
        }
        result.stopped.is_some()
    }

    /// The fetch request for a reference that needs no provider round trip.
    fn classify(&self, target: &ReferenceTarget) -> Classified {
        let configuration = &*self.configuration;
        match target {
            ReferenceTarget::JiraKey { provider_id, key } => {
                let Some(provider) = configuration.providers.iter().find(|provider| {
                    provider.id == *provider_id && provider.kind == ProviderKind::Jira
                }) else {
                    return failure(
                        "source_provider_unsupported",
                        "the Jira provider is not configured",
                    );
                };
                let artifact = match jira_artifact(provider, key, key) {
                    Ok(artifact) => artifact,
                    Err(_) => return Classified::Link,
                };
                match site_authority(configuration, provider_id) {
                    Ok(authority) => prepared(
                        artifact.provider_id,
                        "issue",
                        key.clone(),
                        artifact.canonical_url,
                        authority,
                    ),
                    Err(error) => failure(&error.code, error.message),
                }
            }
            ReferenceTarget::Truncated => Classified::Link,
            ReferenceTarget::Url { url } => {
                let url = url.split('#').next().unwrap_or(url);
                if let Some(provider_id) = confluence_provider_for_input(configuration, url, None) {
                    return Classified::Confluence {
                        provider_id,
                        url: url.to_owned(),
                    };
                }
                let artifact = resolve_artifact(configuration, url).or_else(|error| {
                    let bare = without_query_and_fragment(url);
                    if bare.len() == url.len() {
                        Err(error)
                    } else {
                        resolve_artifact(configuration, bare)
                    }
                });
                // An unrecognized URL, or a wiki page, is an ordinary link.
                let Ok(artifact) = artifact else {
                    return Classified::Link;
                };
                if !matches!(artifact.kind.as_str(), "issue" | "review") {
                    return Classified::Link;
                }
                match instance_authority(
                    configuration,
                    &artifact.provider_id,
                    &artifact.canonical_url,
                ) {
                    Ok(authority) => prepared(
                        artifact.provider_id,
                        &artifact.kind,
                        artifact.canonical_id,
                        artifact.canonical_url,
                        authority,
                    ),
                    Err(error) => failure(&error.code, error.message),
                }
            }
        }
    }

    async fn resolve_confluence(&self, provider_id: &str, url: &str) -> Classified {
        match self.resolve_input(provider_id, url).await {
            Ok(ProviderResolution::ConfluencePage(page)) => {
                self.confluence_prepared(provider_id, &page)
            }
            Ok(ProviderResolution::ConfluenceSpace { .. }) => Classified::Link,
            Err(error) if error.code == "library_input_unrecognized" => Classified::Link,
            Err(error) => failure(&error.code, error.message),
        }
    }

    fn confluence_prepared(&self, provider_id: &str, page: &ConfluencePage) -> Classified {
        match confluence_instance_authority(
            &self.configuration,
            provider_id,
            page,
            &page.canonical_url,
        ) {
            Ok(authority) => {
                let url = confluence_page_url(&authority.provider_instance, &page.page_id);
                prepared(
                    provider_id.to_owned(),
                    "page",
                    page.page_id.clone(),
                    url,
                    authority,
                )
            }
            Err(error) => failure(&error.code, error.message),
        }
    }
}

fn prepared(
    provider_id: String,
    resource_type: &str,
    canonical_id: String,
    artifact_url: String,
    authority: SourceAuthority,
) -> Classified {
    Classified::Ready(Prepared {
        expected: SourceRef {
            provider_id: provider_id.clone(),
            provider_instance: authority.provider_instance.clone(),
            resource_type: resource_type.to_owned(),
            canonical_id,
        },
        request: SourceFetchRequest {
            provider_id,
            artifact_url,
            authority,
        },
    })
}

async fn fetch_related(
    provider: Arc<dyn SourceProvider>,
    request: SourceFetchRequest,
    expected: SourceRef,
    limit: Duration,
) -> Result<SourceAsset, InspectionError> {
    validate_request(&request)?;
    let assets = timeout(limit, provider.fetch(&request))
        .await
        .map_err(|_| {
            InspectionError::new(
                "source_fetch_timeout",
                "source fetch exceeded the configured operation deadline",
            )
        })??;
    let [asset] = assets.as_slice() else {
        return Err(InspectionError::new(
            "source_provider_contract",
            "a related item fetch returns exactly one asset",
        ));
    };
    validate_provider_asset(&request, asset)?;
    validate_asset(asset)?;
    validate_confluence_page(&request, &assets)?;
    let source = &asset.source;
    if source.provider_id != expected.provider_id
        || source.provider_instance != expected.provider_instance
        || source.resource_type != expected.resource_type
        || source.canonical_id != expected.canonical_id
    {
        return Err(InspectionError::new(
            "source_identity_mismatch",
            "provider returned a different item than requested",
        ));
    }
    Ok(assets.into_iter().next().expect("one asset"))
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;
    use std::sync::Mutex;

    use async_trait::async_trait;
    use cockpit_protocol::projects::{ProjectLimits, ProjectProvider};
    use cockpit_protocol::sources::SourceCapability;

    use super::*;
    use crate::sources::FrontmatterField;

    fn configuration(providers: &[(&str, &str, ProviderKind)]) -> ProjectConfiguration {
        ProjectConfiguration {
        repository_roots: vec![],
        branch_template: "{repo}/{task_id}".into(),
        checkout_template: "{repo}-{task_id}".into(),
        providers: providers
            .iter()
            .map(|(id, base_url, kind)| ProjectProvider {
                id: (*id).into(),
                kind: *kind,
                base_url: (*base_url).into(),
                executable: (*kind == ProviderKind::Gitea).then(|| "custom-forge-client".into()),
                login: None,
                deployment: (*kind == ProviderKind::Jira)
                    .then_some(cockpit_protocol::projects::ProviderDeployment::DataCenter),
            })
            .collect(),
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
        ..ProjectConfiguration::for_tests(std::path::Path::new(""))
        }
    }

    fn asset(
        provider_id: &str,
        instance: &str,
        resource_type: &str,
        canonical_id: &str,
        body: &str,
    ) -> SourceAsset {
        SourceAsset {
            source: SourceRef {
                provider_id: provider_id.into(),
                provider_instance: instance.into(),
                resource_type: resource_type.into(),
                canonical_id: canonical_id.into(),
            },
            title: format!("title {canonical_id}"),
            source_url: None,
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

    fn keys(references: &[SourceReference]) -> Vec<(String, String)> {
        references
            .iter()
            .map(|reference| {
                (
                    match &reference.target {
                        ReferenceTarget::JiraKey { key, .. } => key.clone(),
                        ReferenceTarget::Url { url } => url.clone(),
                        ReferenceTarget::Truncated => String::new(),
                    },
                    reference.relation.clone(),
                )
            })
            .collect()
    }

    #[test]
    fn jira_references_come_from_fields_description_and_comments_with_clean_boundaries() {
        let configuration = configuration(&[("jira", "https://jira.test", ProviderKind::Jira)]);
        let mut issue = asset(
            "jira",
            "https://jira.test",
            "issue",
            "OPS-1",
            "**Task**\n\n## Description\n\nSee OPS-12. Not OPS-12-fix, xOPS-1, OPS-13x or \
             [OPS-14](https://jira.test/browse/OPS-14) https://other.test/OPS-15 and OPS-1 itself.\n\
             \n## Comments (1)\n\n### Ann · today\n[#5](https://jira.test/browse/OPS-1?focusedCommentId=5)\n\
             \nBlocked by OPS-16 (see https://forge.test/acme/repo/issues/2).\n",
        );
        issue.source_url = Some("https://jira.test/browse/OPS-1".into());
        issue.fields = vec![
            FrontmatterField {
                key: "parent".into(),
                value: FrontmatterValue::String("OPS-9".into()),
            },
            FrontmatterField {
                key: "subtasks".into(),
                value: FrontmatterValue::Strings(vec!["OPS-10".into()]),
            },
            FrontmatterField {
                key: "links".into(),
                value: FrontmatterValue::Strings(vec!["is blocked by OPS-11".into()]),
            },
        ];
        let references = keys(&asset_references(&configuration, &issue));
        let expected: Vec<(String, String)> = [
            ("OPS-9", "parent"),
            ("OPS-10", "subtask"),
            ("OPS-11", "is blocked by"),
            ("OPS-12", "description"),
            ("OPS-14", "description"),
            ("https://jira.test/browse/OPS-14", "description"),
            ("https://other.test/OPS-15", "description"),
            ("OPS-16", "comment"),
            ("https://forge.test/acme/repo/issues/2", "comment"),
        ]
        .into_iter()
        .map(|(target, relation)| (target.to_owned(), relation.to_owned()))
        .collect();
        assert_eq!(references, expected);
        // Non-Jira bodies contribute URLs only.
        let forge = asset(
            "tea",
            "https://forge.test",
            "issue",
            "acme/repo#1",
            "OPS-1 https://forge.test/acme/repo/issues/2.",
        );
        assert_eq!(
            keys(&asset_references(&configuration, &forge)),
            vec![(
                "https://forge.test/acme/repo/issues/2".to_owned(),
                "body".to_owned()
            )]
        );
    }

    #[test]
    fn jira_reference_extraction_uses_kind_not_provider_id_or_executable() {
        let mut configuration = configuration(&[("jira", "https://jira.test", ProviderKind::Gitea)]);
        configuration.providers[0].executable = Some("/usr/local/bin/jira".into());
        let issue = asset("jira", "https://jira.test", "issue", "OPS-1", "## Description\n\nSee OPS-2");
        assert!(asset_references(&configuration, &issue).is_empty());
        configuration.providers[0].kind = ProviderKind::Jira;
        configuration.providers[0].executable = None;
        configuration.providers[0].deployment =
            Some(cockpit_protocol::projects::ProviderDeployment::DataCenter);
        assert_eq!(
            keys(&asset_references(&configuration, &issue)),
            vec![("OPS-2".into(), "description".into())]
        );
    }

    #[tokio::test]
    async fn an_incomplete_source_keeps_its_references_incomplete_when_expanded() {
        let configuration = configuration(&[("jira", "https://jira.test", ProviderKind::Jira)]);
        let body = "## Description\n\nsee OPS-2\n";
        let mut issue = asset("jira", "https://jira.test", "issue", "OPS-1", body);
        assert!(!references_truncated(&asset_references(
            &configuration,
            &issue
        )));
        issue.complete = false;
        let stored = asset_references(&configuration, &issue);
        assert!(references_truncated(&stored));
        assert_eq!(keys(&stored)[0].0, "OPS-2");
        let (service, source, cancel) = graph(&[("OPS-2", "")], &[]);
        let seed = ReferenceSeed {
            source,
            label: "OPS-1".into(),
            references: stored,
        };
        let result = service
            .collect_related(vec![seed], 1, TraversalBudget::Single, &cancel)
            .await;
        assert_eq!(ids(&result), vec![("OPS-2".to_owned(), 1)]);
        assert!(!result.complete());
        assert_eq!(result.failures[0].code, "source_references_truncated");
    }

    #[derive(Clone)]
    struct Fake {
        id: &'static str,
        instance: &'static str,
        assets: Arc<Mutex<BTreeMap<String, SourceAsset>>>,
    }

    #[async_trait]
    impl SourceProvider for Fake {
        fn provider_id(&self) -> &str {
            self.id
        }
        fn capabilities(&self) -> Vec<SourceCapability> {
            vec![SourceCapability::Issue]
        }
        async fn fetch(
            &self,
            request: &SourceFetchRequest,
        ) -> Result<Vec<SourceAsset>, InspectionError> {
            assert_eq!(request.authority.provider_instance, self.instance);
            self.assets
                .lock()
                .unwrap()
                .get(&request.artifact_url)
                .cloned()
                .map(|asset| vec![asset])
                .ok_or_else(|| InspectionError::new("fixture_missing", "not found"))
        }
    }

    fn graph(
        jira: &[(&str, &str)],
        tea: &[(&str, &str)],
    ) -> (SourceService, SourceRef, Arc<AtomicBool>) {
        let configuration = configuration(&[
            ("jira", "https://jira.test", ProviderKind::Jira),
            ("tea", "https://forge.test/gitea", ProviderKind::Gitea),
        ]);
        let mut jira_assets = BTreeMap::new();
        for (key, body) in jira {
            jira_assets.insert(
                format!("https://jira.test/browse/{key}"),
                asset(
                    "jira",
                    "https://jira.test",
                    "issue",
                    key,
                    &format!("## Description\n\n{body}\n"),
                ),
            );
        }
        let mut tea_assets = BTreeMap::new();
        for (id, body) in tea {
            tea_assets.insert(
                format!("https://forge.test/gitea/acme/repo/issues/{id}"),
                asset(
                    "tea",
                    "https://forge.test/gitea",
                    "issue",
                    &format!("acme/repo#{id}"),
                    body,
                ),
            );
        }
        let providers: Vec<Arc<dyn SourceProvider>> = vec![
            Arc::new(Fake {
                id: "jira",
                instance: "https://jira.test",
                assets: Arc::new(Mutex::new(jira_assets)),
            }),
            Arc::new(Fake {
                id: "tea",
                instance: "https://forge.test/gitea",
                assets: Arc::new(Mutex::new(tea_assets)),
            }),
        ];
        let service = SourceService::new(&configuration, providers).unwrap();
        let seed = SourceRef {
            provider_id: "jira".into(),
            provider_instance: "https://jira.test".into(),
            resource_type: "issue".into(),
            canonical_id: "OPS-1".into(),
        };
        (service, seed, Arc::new(AtomicBool::new(false)))
    }

    fn seed(source: &SourceRef, text: &str) -> ReferenceSeed {
        let configuration = configuration(&[("jira", "https://jira.test", ProviderKind::Jira)]);
        let seed_asset = asset(
            "jira",
            "https://jira.test",
            "issue",
            &source.canonical_id,
            &format!("## Description\n\n{text}\n"),
        );
        ReferenceSeed {
            source: source.clone(),
            label: asset_label(&seed_asset),
            references: asset_references(&configuration, &seed_asset),
        }
    }

    fn ids(result: &RelatedResult) -> Vec<(String, u32)> {
        result
            .assets
            .iter()
            .map(|related| (related.asset.source.canonical_id.clone(), related.depth))
            .collect()
    }

    #[tokio::test]
    async fn traversal_crosses_providers_by_depth_and_ignores_cycles() {
        let (service, source, cancel) = graph(
            &[
                ("OPS-2", "forge https://forge.test/gitea/acme/repo/issues/7"),
                ("OPS-3", ""),
            ],
            &[(
                "7",
                "back to https://jira.test/browse/OPS-1 and https://jira.test/browse/OPS-3 and https://forge.test/gitea/acme/repo/issues/7",
            )],
        );
        let root = seed(&source, "see OPS-2");
        let none = service
            .collect_related(vec![root.clone()], 0, TraversalBudget::Single, &cancel)
            .await;
        assert!(none.assets.is_empty() && none.complete());
        let one = service
            .collect_related(vec![root.clone()], 1, TraversalBudget::Single, &cancel)
            .await;
        assert_eq!(ids(&one), vec![("OPS-2".to_owned(), 1)]);
        let two = service
            .collect_related(vec![root.clone()], 2, TraversalBudget::Single, &cancel)
            .await;
        assert_eq!(
            ids(&two),
            vec![("OPS-2".to_owned(), 1), ("acme/repo#7".to_owned(), 2)]
        );
        assert_eq!(two.assets[1].from_label, "OPS-2");
        assert_eq!(two.assets[1].relation, "description");
        assert!(two.complete());
        // The cycle back to the seed and the second route to OPS-3 dedupe.
        let three = service
            .collect_related(vec![root.clone()], 3, TraversalBudget::Single, &cancel)
            .await;
        assert_eq!(
            ids(&three),
            vec![
                ("OPS-2".to_owned(), 1),
                ("acme/repo#7".to_owned(), 2),
                ("OPS-3".to_owned(), 3)
            ]
        );
        assert!(three.complete());
        // A missing related item is a failure, never a silent drop.
        let missing = service
            .collect_related(
                vec![seed(&source, "OPS-2 OPS-404")],
                1,
                TraversalBudget::Single,
                &cancel,
            )
            .await;
        assert_eq!(ids(&missing), vec![("OPS-2".to_owned(), 1)]);
        assert_eq!(missing.failures.len(), 1);
        assert_eq!(missing.failures[0].target, "OPS-404");
        assert!(!missing.complete());
    }

    #[tokio::test]
    async fn the_item_limit_stops_traversal_and_says_so() {
        let text: String = (2..=40).map(|number| format!("OPS-{number} ")).collect();
        let jira: Vec<(String, &str)> = (2..=40)
            .map(|number| (format!("OPS-{number}"), ""))
            .collect();
        let borrowed: Vec<(&str, &str)> = jira
            .iter()
            .map(|(key, body)| (key.as_str(), *body))
            .collect();
        let (service, source, cancel) = graph(&borrowed, &[]);
        let result = service
            .collect_related(
                vec![seed(&source, &text)],
                1,
                TraversalBudget::Single,
                &cancel,
            )
            .await;
        assert_eq!(result.assets.len(), 32);
        assert_eq!(result.stopped, Some(TraversalStop::Items));
        assert!(!result.complete());
        cancel.store(true, Ordering::Relaxed);
        let cancelled = service
            .collect_related(
                vec![seed(&source, "OPS-2")],
                1,
                TraversalBudget::Single,
                &cancel,
            )
            .await;
        assert_eq!(cancelled.stopped, Some(TraversalStop::Cancelled));
    }
}
