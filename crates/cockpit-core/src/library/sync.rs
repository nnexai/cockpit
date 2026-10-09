//! Bounded discovery and durable, revision-deduplicated background publication.
//! This file is independent of index freshness: quiet passes never publish an index.
use super::{
    LibraryService, SaveOptions, follow, item_id, jira_follow, operations, refs,
    store::{self, Index, Lease, LibraryIndexEntry, Store, error},
};
use crate::{
    InspectionError,
    config::LibrarySyncConfiguration,
    project_store::read_json_bounded,
    sources::{
        self, IssueQuery, IssueRow, ReferenceSeed, SourceRef, SpacePage, TraversalBudget,
        lane::{self, BackgroundPolicy},
    },
};
use cap_fs_ext::{OpenOptionsFollowExt, OpenOptionsSyncExt};
use cap_std::fs::{Dir, OpenOptions};
use cockpit_protocol::{
    library::*,
    projects::{ProjectDiagnostic, ProviderKind},
};
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeMap, BTreeSet},
    io::{self, Read},
    sync::{Arc, atomic::AtomicBool},
    time::{Duration, Instant},
};

const MAX_STATE_BYTES: u64 = 64 * 1024 * 1024;
const MAX_QUEUE: usize = 100_000;
const MAX_SOURCES: usize = 10_000;
const MINUTE: i64 = 60_000;
const HOUR: i64 = 60 * MINUTE;
const ABSENCE_DELAY: i64 = 30 * MINUTE;
const DRAIN_ITEMS: usize = 100;
const DRAIN_SECONDS: u64 = 600;

/// Lifetime guard. Construction of LibraryService never starts background work.
pub struct LibrarySyncRuntime {
    task: Option<tokio::task::JoinHandle<()>>,
}
impl Drop for LibrarySyncRuntime {
    fn drop(&mut self) {
        if let Some(task) = self.task.take() {
            task.abort();
        }
    }
}

/// The same engine used by the long-lived scheduler. `now_ms` is explicit so
/// deterministic tests and disposable smoke fixtures exercise the real policy.
#[derive(Debug, Default, Clone, Serialize)]
pub struct LibrarySyncTick {
    pub discovered: u32,
    pub fetched: u32,
    pub pending: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct State {
    schema: u32,
    sources: BTreeMap<String, SourceState>,
    queue: BTreeMap<String, Candidate>,
    audits: BTreeMap<String, Audit>,
    origins: BTreeMap<String, i64>,
    active_operation: Option<String>,
}
impl Default for State {
    fn default() -> Self {
        Self {
            schema: 1,
            sources: BTreeMap::new(),
            queue: BTreeMap::new(),
            audits: BTreeMap::new(),
            origins: BTreeMap::new(),
            active_operation: None,
        }
    }
}
#[derive(Debug, Default, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct SourceState {
    next_delta_ms: i64,
    next_inventory_ms: i64,
    next_related_ms: i64,
    last_started_ms: i64,
    last_success_ms: Option<i64>,
    committed_upper_ms: Option<i64>,
    window: Option<(i64, i64)>,
    failures: u8,
    last_error: Option<String>,
    absent: BTreeMap<String, i64>,
    related_absent: BTreeMap<String, i64>,
    held: Option<String>,
    inventory: Option<StandaloneInventory>,
}
/// A frozen cohort grants absence authority only when every chunk completed.
/// Queue candidates and each completed prefix are committed in the same write.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct StandaloneInventory {
    ids: Vec<String>,
    cursor: usize,
    present: BTreeSet<String>,
    batch_size: usize,
    requests_per_id: u32,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Candidate {
    source: SourceRef,
    owners: BTreeSet<String>,
    revision: Option<String>,
    row: Option<IssueRow>,
    page: Option<SpacePage>,
    force: bool,
    confirm_missing: bool,
    enqueued_ms: i64,
    next_attempt_ms: i64,
    failures: u8,
    last_error: Option<String>,
}
#[derive(Debug, Default, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Audit {
    cursor: Option<String>,
    next_due_ms: i64,
    failures: BTreeMap<String, AuditFailure>,
}
#[derive(Debug, Default, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct AuditFailure {
    failures: u8,
    next_attempt_ms: i64,
    last_error: String,
}

fn wall_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis()
        .min(i64::MAX as u128) as i64
}
fn policy(config: &LibrarySyncConfiguration) -> BackgroundPolicy {
    BackgroundPolicy {
        min_interval_seconds: config.background_min_interval_seconds,
        in_flight: config.background_in_flight,
        hourly_request_cap: config.hourly_request_cap,
    }
}
fn state_dir(store: &Store) -> Result<Dir, InspectionError> {
    store::open_child(&store.meta, "sync")
}
fn load(store: &Store) -> Result<State, InspectionError> {
    let dir = state_dir(store)?;
    match dir.symlink_metadata("state.json") {
        Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(State::default()),
        Err(e) => return Err(error("library_unavailable", e.to_string())),
        Ok(metadata)
            if !metadata.is_file()
                || metadata.file_type().is_symlink()
                || metadata.len() > MAX_STATE_BYTES =>
        {
            return Err(error(
                "library_corrupt",
                "Sync state is not a bounded regular file",
            ));
        }
        Ok(_) => {}
    }
    let state: State = read_json_bounded(&dir, "state.json", MAX_STATE_BYTES)?;
    if state.schema != 1
        || state.queue.len() > MAX_QUEUE
        || state.sources.len() > MAX_SOURCES
        || state.audits.len() > MAX_SOURCES
        || state.origins.len() > MAX_SOURCES
        || state
            .audits
            .values()
            .any(|audit| audit.failures.len() > MAX_QUEUE)
        || state.sources.values().any(|source| {
            source.absent.len() > MAX_QUEUE
                || source.related_absent.len() > MAX_QUEUE
                || source.inventory.as_ref().is_some_and(|inventory| {
                    inventory.ids.len() > MAX_QUEUE
                        || inventory.cursor > inventory.ids.len()
                        || inventory.present.len() > inventory.ids.len()
                        || inventory.batch_size == 0
                        || inventory.batch_size > 100
                        || inventory.requests_per_id == 0
                        || inventory.ids.windows(2).any(|ids| ids[0] >= ids[1])
                        || inventory.present.iter().any(|id| {
                            inventory.ids[..inventory.cursor.min(inventory.ids.len())]
                                .binary_search(id)
                                .is_err()
                        })
                        || inventory
                            .ids
                            .iter()
                            .any(|id| id.is_empty() || id.len() > 256)
                })
                || source
                    .window
                    .is_some_and(|(lower, upper)| lower < 0 || lower >= upper)
        })
        || state.queue.iter().any(|(key, candidate)| {
            key != &item_id(&candidate.source)
                || candidate.owners.len() > MAX_SOURCES
                || candidate.source.provider_id.len() > 256
                || candidate.source.provider_instance.len() > 512
                || candidate.source.canonical_id.len() > 256
                || !matches!(candidate.source.resource_type.as_str(), "page" | "issue")
        })
    {
        return Err(error(
            "library_corrupt",
            "Unsupported or oversized sync state",
        ));
    }
    Ok(state)
}
fn save(store: &Store, state: &State) -> Result<(), InspectionError> {
    if state.queue.len() > MAX_QUEUE || state.sources.len() > MAX_SOURCES {
        return Err(error(
            "library_sync_capacity",
            "Synchronization state exceeds its bounded capacity",
        ));
    }
    store::bounded_write(&state_dir(store)?, "state.json", state, MAX_STATE_BYTES)
}
fn index(store: &Store) -> Result<Index, InspectionError> {
    let _lock = store.shared()?;
    store.index()
}
fn source(follow: &LibraryFollowSummary, id: &str, resource_type: &str) -> SourceRef {
    SourceRef {
        provider_id: follow.provider_id.clone(),
        provider_instance: follow.provider_instance.clone(),
        canonical_id: id.to_owned(),
        resource_type: resource_type.to_owned(),
    }
}
fn candidate(source: SourceRef, owner: &str, revision: Option<String>, now: i64) -> Candidate {
    Candidate {
        source,
        owners: BTreeSet::from([owner.to_owned()]),
        revision,
        row: None,
        page: None,
        force: false,
        confirm_missing: false,
        enqueued_ms: now,
        next_attempt_ms: now,
        failures: 0,
        last_error: None,
    }
}
fn enqueue(state: &mut State, mut work: Candidate) -> Result<(), InspectionError> {
    let id = item_id(&work.source);
    if let Some(old) = state.queue.get_mut(&id) {
        old.owners.append(&mut work.owners);
        // An overlapping earlier window must not turn a newer candidate backwards.
        // Listing metadata is only a fetch hint; publication always reads current content.
        let older = match (old.revision.as_deref(), work.revision.as_deref()) {
            (Some(old), Some(new)) if work.source.resource_type == "page" => old
                .parse::<u64>()
                .ok()
                .zip(new.parse::<u64>().ok())
                .is_some_and(|(old, new)| new < old),
            (Some(old), Some(new)) => new < old,
            _ => false,
        };
        let newer = work.revision.is_some() && !older && old.revision != work.revision;
        if newer || work.force && !old.force {
            old.next_attempt_ms = work.next_attempt_ms;
            old.failures = 0;
            old.last_error = None;
        }
        if !older {
            if work.revision.is_some() {
                old.revision = work.revision;
            }
            if work.row.is_some() {
                old.row = work.row;
            }
            if work.page.is_some() {
                old.page = work.page;
            }
        }
        old.force |= work.force;
        old.confirm_missing |= work.confirm_missing;
    } else {
        if state.queue.len() >= MAX_QUEUE {
            return Err(error(
                "library_sync_capacity",
                "Synchronization fetch queue is full",
            ));
        }
        state.queue.insert(id, work);
    }
    Ok(())
}
fn backoff(failures: u8, base: i64, cap: i64) -> i64 {
    base.saturating_mul(1i64 << u32::from(failures.saturating_sub(1).min(16)))
        .min(cap)
}
fn schedule(now: i64, duration: i64, identity: &str) -> i64 {
    // Stable per-source spreading also remains reproducible in smoke fixtures.
    use sha2::{Digest, Sha256};
    let hash = Sha256::digest(identity.as_bytes());
    let spread = i64::from(u16::from_be_bytes([hash[0], hash[1]])) % 201 - 100;
    now.saturating_add(duration)
        .saturating_add(duration / 1000 * spread)
}
fn confirmed(previous: &BTreeMap<String, i64>, id: &str, now: i64) -> bool {
    previous
        .get(id)
        .is_some_and(|first| now.saturating_sub(*first) >= ABSENCE_DELAY)
}
fn absence(
    previous: &BTreeMap<String, i64>,
    missing: &BTreeSet<String>,
    now: i64,
) -> BTreeMap<String, i64> {
    missing
        .iter()
        .map(|id| (id.clone(), *previous.get(id).unwrap_or(&now)))
        .collect()
}
fn allowed_mass(missing: usize, members: usize) -> bool {
    missing <= 25usize.max(members / 10)
}

impl LibraryService {
    fn sync_origin(&self, provider_id: &str) -> Option<String> {
        let provider = self
            .configuration
            .providers
            .iter()
            .find(|p| p.id == provider_id)?;
        let url = url::Url::parse(&provider.base_url).ok()?;
        Some(format!(
            "{}://{}:{}",
            url.scheme(),
            url.host_str()?.to_ascii_lowercase(),
            url.port_or_known_default()?
        ))
    }
    fn sync_blocked(&self, state: &mut State, provider_id: &str, now: i64) -> bool {
        let Some(origin) = self.sync_origin(provider_id) else {
            return false;
        };
        if let Some(until) = self.sources.blocked_until_ms(provider_id) {
            let saved = state.origins.entry(origin.clone()).or_default();
            *saved = (*saved).max(until);
        }
        state.origins.get(&origin).is_some_and(|until| *until > now)
    }
    fn sync_hold(
        &self,
        store: &Store,
        state: &mut State,
        follow: &LibraryFollowSummary,
        missing: usize,
        members: usize,
    ) -> Result<(), InspectionError> {
        let held = (!allowed_mass(missing, members)).then(|| {
            format!(
                "{missing} of {members} members absent; automatic removal held for manual Refresh"
            )
        });
        let previous = &mut state.sources.get_mut(&follow.follow_id).unwrap().held;
        if *previous == held {
            return Ok(());
        }
        *previous = held.clone();
        if let Some(message) = held {
            let (operation, _lease) = operations::create_background(store)?;
            let result = operations::follow_row(
                store,
                &operation.operation_id,
                follow,
                LibraryReportOutcome::Partial,
                Some(message),
            );
            operations::finish(store, &operation.operation_id, result)?;
        }
        Ok(())
    }
    pub fn start_sync(
        &self,
        config: LibrarySyncConfiguration,
    ) -> Result<LibrarySyncRuntime, InspectionError> {
        if !config.enabled {
            return Ok(LibrarySyncRuntime { task: None });
        }
        config.validate()?;
        let handle = operations::runtime()?;
        let service = self.clone();
        let task = handle.spawn(async move {
            // Give startup/manual work priority. Overdue windows coalesce; no replay loop.
            tokio::time::sleep(Duration::from_secs(60)).await;
            loop {
                if let Ok(store) = service.open() {
                    if let Ok(owner) = store.lease("sync:scheduler") {
                        let _owner = owner;
                        loop {
                            let _ = lane::scope_background(
                                policy(&config),
                                service.sync_tick_owned(&store, &config, wall_ms()),
                            )
                            .await;
                            tokio::time::sleep(Duration::from_secs(60)).await;
                        }
                    }
                }
                tokio::time::sleep(Duration::from_secs(60)).await;
            }
        });
        Ok(LibrarySyncRuntime { task: Some(task) })
    }

    pub async fn sync_tick(
        &self,
        config: LibrarySyncConfiguration,
        now_ms: i64,
    ) -> Result<LibrarySyncTick, InspectionError> {
        if !config.enabled {
            return Ok(LibrarySyncTick::default());
        }
        config.validate()?;
        if now_ms < 0 {
            return Err(error(
                "library_sync_invalid",
                "Sync time must be an epoch millisecond instant",
            ));
        }
        let store = self.open()?;
        let _owner = store.lease("sync:scheduler")?;
        lane::scope_background(
            policy(&config),
            self.sync_tick_owned(&store, &config, now_ms),
        )
        .await
    }

    async fn sync_tick_owned(
        &self,
        store: &Arc<Store>,
        config: &LibrarySyncConfiguration,
        now: i64,
    ) -> Result<LibrarySyncTick, InspectionError> {
        let mut state = load(store)?;
        // Stale active records are reconciled using existing OS operation ownership.
        if let Some(active) = state.active_operation.take() {
            let _ = operations::get(store, &active);
        }
        let mut report = LibrarySyncTick::default();
        // Spend a replenished origin budget on already-discovered work before
        // discovery can exhaust it again. Failed candidates retain their backoff.
        self.drain_sync(store, &mut state, now, &mut report).await?;
        let snapshot = index(store)?;
        let ids = snapshot
            .follows
            .iter()
            .map(|follow| follow.follow_id.clone())
            .collect::<BTreeSet<_>>();
        state.sources.retain(|id, _| {
            ids.contains(id) || id.starts_with("standalone:") || id.starts_with("item:")
        });
        let mut follows = snapshot.follows;
        follows.sort_by_key(|follow| {
            state
                .sources
                .get(&follow.follow_id)
                .map(|s| s.next_delta_ms.min(s.next_inventory_ms))
                .unwrap_or(0)
        });
        // Discovery holds no item/follow lease during HTTP waits. Every application
        // re-reads the follow and exclusions under the metadata lock.
        for follow in &follows {
            let source_state = state
                .sources
                .entry(follow.follow_id.clone())
                .or_default()
                .clone();
            if source_state.next_delta_ms > now && source_state.next_inventory_ms > now {
                continue;
            }
            if self.sync_blocked(&mut state, &follow.provider_id, now) {
                continue;
            }
            // A manual worker already owns this follow: yield, rather than competing.
            let Ok(probe) = store.lease(&follow.follow_id) else {
                continue;
            };
            drop(probe);
            let delta_due = source_state.next_delta_ms <= now;
            let inventory_due = source_state.next_inventory_ms <= now;
            {
                let s = state.sources.get_mut(&follow.follow_id).unwrap();
                s.last_started_ms = now;
                if delta_due {
                    s.next_delta_ms = now.saturating_add(i64::from(config.delta_minutes) * MINUTE);
                    if s.window.is_none() {
                        let upper = (now
                            .saturating_sub(i64::from(config.lag_allowance_minutes) * MINUTE)
                            / MINUTE)
                            * MINUTE;
                        let lower = s
                            .committed_upper_ms
                            .unwrap_or(
                                upper.saturating_sub(i64::from(config.delta_minutes) * MINUTE),
                            )
                            .saturating_sub(i64::from(config.overlap_minutes) * MINUTE)
                            .max(0);
                        if lower < upper {
                            s.window = Some((lower, upper));
                        }
                    }
                }
                if inventory_due {
                    s.next_inventory_ms = schedule(
                        now,
                        i64::from(config.inventory_hours) * HOUR,
                        &follow.follow_id,
                    );
                }
            }
            save(store, &state)?; // crash lease and fixed bounds precede remote work
            let mut failure = None;
            if delta_due {
                if let Some(window) = state.sources[&follow.follow_id].window {
                    match self
                        .discover_follow(store, &mut state, follow, Some(window), now)
                        .await
                    {
                        Ok((count, complete)) => {
                            report.discovered += count;
                            let s = state.sources.get_mut(&follow.follow_id).unwrap();
                            if complete {
                                s.committed_upper_ms = Some(window.1);
                                s.window = None;
                            } else {
                                // Partial/capped discovery retries this exact frozen
                                // interval. Only a complete listing can advance it.
                                failure = Some("library_sync_partial".to_owned());
                            }
                            save(store, &state)?; // checkpoint + every discovered candidate atomically
                        }
                        Err(e) => failure = Some(e.code),
                    }
                }
            }
            if inventory_due {
                match self
                    .discover_follow(store, &mut state, follow, None, now)
                    .await
                {
                    Ok((count, true)) => {
                        report.discovered += count;
                    }
                    Ok((count, false)) => {
                        report.discovered += count;
                        failure = Some("library_sync_partial".into());
                    }
                    Err(e) => failure = Some(e.code),
                }
            }
            self.sync_blocked(&mut state, &follow.provider_id, now);
            let s = state.sources.get_mut(&follow.follow_id).unwrap();
            if let Some(code) = failure {
                s.failures = s.failures.saturating_add(1);
                s.last_error = Some(code);
                let retry = now.saturating_add(backoff(s.failures, 5 * MINUTE, HOUR));
                if delta_due {
                    s.next_delta_ms = retry;
                }
                if inventory_due {
                    s.next_inventory_ms = retry;
                }
            } else {
                s.failures = 0;
                s.last_error = None;
                s.last_success_ms = Some(now);
            }
            save(store, &state)?;
        }
        self.discover_standalone(store, &mut state, config, now, &mut report)
            .await?;
        self.drain_sync(store, &mut state, now, &mut report).await?;
        self.sync_related_due(store, &mut state, &follows, config, now)
            .await?;
        self.sync_single_related_due(store, &mut state, config, now)
            .await?;
        self.audit_sync(store, &mut state, config, now).await?;
        report.pending = state.queue.len().min(u32::MAX as usize) as u32;
        save(store, &state)?;
        Ok(report)
    }

    async fn discover_follow(
        &self,
        store: &Arc<Store>,
        state: &mut State,
        follow: &LibraryFollowSummary,
        window: Option<(i64, i64)>,
        now: i64,
    ) -> Result<(u32, bool), InspectionError> {
        let site = sources::site_authority(&self.configuration, &follow.provider_id)?;
        if site.provider_instance != follow.provider_instance {
            return Err(error(
                "source_authority_mismatch",
                "Follow belongs to another configured site",
            ));
        }
        let cancel = AtomicBool::new(false);
        let limit = self.configuration.limits.library_space_pages.max(1);
        match &follow.source {
            LibraryFollowSource::ConfluenceSpace { space_key, .. } => {
                let listing = match window {
                    Some((lower, upper)) => {
                        self.sources
                            .list_page_changes(
                                &follow.provider_id,
                                space_key,
                                lower,
                                upper,
                                limit,
                                &cancel,
                            )
                            .await?
                    }
                    None => {
                        self.sources
                            .list_space_pages(&follow.provider_id, space_key, limit, &cancel)
                            .await?
                    }
                };
                let snapshot = index(store)?;
                let Some(current) = snapshot
                    .follows
                    .iter()
                    .find(|f| f.follow_id == follow.follow_id)
                else {
                    return Ok((0, false));
                };
                let mut count = 0;
                for page in &listing.pages {
                    if current.excluded_ids.contains(&page.page_id) {
                        continue;
                    }
                    let id = follow::page_item_id(follow, &page.page_id);
                    let old = snapshot
                        .items
                        .iter()
                        .find(|entry| entry.summary.item_id == id);
                    if old.is_none_or(|entry| follow::change_reason(entry, page).is_some()) {
                        let mut work = candidate(
                            source(follow, &page.page_id, "page"),
                            &follow.follow_id,
                            Some(page.version.to_string()),
                            now,
                        );
                        work.force = old.is_some_and(|entry| {
                            follow::change_reason(entry, page).is_some()
                                && entry.summary.source_revision.as_deref()
                                    == Some(page.version.to_string().as_str())
                        });
                        work.page = Some(page.clone());
                        enqueue(state, work)?;
                        count += 1;
                    }
                }
                self.adopt_pages(store, follow, &listing.pages)?;
                if window.is_none() && listing.complete {
                    let present = listing
                        .pages
                        .iter()
                        .map(|p| p.page_id.clone())
                        .chain(listing.homepage_id.clone())
                        .collect::<BTreeSet<_>>();
                    let members = snapshot
                        .items
                        .iter()
                        .filter(|entry| {
                            follow::same_site_page(entry, follow)
                                && refs::has_follow(&entry.summary, &follow.follow_id)
                        })
                        .collect::<Vec<_>>();
                    let missing = members
                        .iter()
                        .filter_map(|entry| entry.summary.canonical_id.as_ref())
                        .filter(|id| !present.contains(*id) && !current.excluded_ids.contains(*id))
                        .cloned()
                        .collect::<BTreeSet<_>>();
                    let previous = state.sources[&follow.follow_id].absent.clone();
                    self.sync_hold(store, state, follow, missing.len(), members.len())?;
                    if !present.is_empty() && allowed_mass(missing.len(), members.len()) {
                        for id in missing.iter().filter(|id| confirmed(&previous, id, now)) {
                            match self.sources.page_space(&follow.provider_id, id).await {
                                Ok(None) => {
                                    self.mark_sync_missing(
                                        store,
                                        state,
                                        follow,
                                        id,
                                        "source_not_found",
                                        "Not found or no longer visible at source",
                                    )
                                    .await?
                                }
                                Ok(Some(key)) if key != *space_key => {
                                    // Move is a membership exit, not a deletion of shared content.
                                    self.drop_sync_members(
                                        store,
                                        state,
                                        follow,
                                        &BTreeSet::from([id.clone()]),
                                        now,
                                    )?;
                                    let mut work =
                                        candidate(source(follow, id, "page"), "tracked", None, now);
                                    work.force = true;
                                    enqueue(state, work)?;
                                }
                                Err(e) if e.code == "source_not_found" => {
                                    self.mark_sync_missing(
                                        store,
                                        state,
                                        follow,
                                        id,
                                        "source_not_found",
                                        "Not found or no longer visible at source",
                                    )
                                    .await?
                                }
                                Ok(Some(_)) => {}
                                Err(e) => return Err(e),
                            }
                        }
                    }
                    state.sources.get_mut(&follow.follow_id).unwrap().absent =
                        absence(&previous, &missing, now);
                }
                Ok((count, listing.complete))
            }
            LibraryFollowSource::JiraQuery { jql, mode } => {
                let listing = self
                    .sources
                    .list_issues(
                        &follow.provider_id,
                        &IssueQuery::Jql {
                            jql,
                            updated_window: window,
                        },
                        limit,
                        &cancel,
                    )
                    .await?;
                let snapshot = index(store)?;
                let Some(current) = snapshot
                    .follows
                    .iter()
                    .find(|f| f.follow_id == follow.follow_id)
                else {
                    return Ok((0, false));
                };
                let mut rows = listing.rows.clone();
                let listed = rows
                    .iter()
                    .map(|row| row.key.clone())
                    .collect::<BTreeSet<_>>();
                let members = snapshot
                    .items
                    .iter()
                    .filter(|entry| {
                        jira_follow::is_issue_of(entry, follow)
                            && refs::has_follow(&entry.summary, &follow.follow_id)
                            && !refs::related_of(&entry.summary, &follow.follow_id)
                    })
                    .filter_map(|entry| entry.summary.canonical_id.clone())
                    .filter(|id| !current.excluded_ids.contains(id))
                    .collect::<BTreeSet<_>>();
                let mut complete = listing.complete;
                if window.is_none() && *mode == LibraryFollowMode::Accumulate {
                    let keys = members.difference(&listed).cloned().collect::<Vec<_>>();
                    for batch in keys.chunks(100) {
                        let checked = self
                            .sources
                            .list_issues(
                                &follow.provider_id,
                                &IssueQuery::Keys(batch),
                                batch.len() as u32,
                                &cancel,
                            )
                            .await?;
                        complete &= checked.complete;
                        rows.extend(checked.rows);
                    }
                }
                let mut count = 0;
                for row in &rows {
                    if current.excluded_ids.contains(&row.key) {
                        continue;
                    }
                    let id = jira_follow::issue_item_id(follow, &row.key);
                    let old = snapshot
                        .items
                        .iter()
                        .find(|entry| entry.summary.item_id == id);
                    if old.is_none_or(|entry| jira_follow::change_reason(entry, row).is_some()) {
                        let mut work = candidate(
                            source(follow, &row.key, "issue"),
                            if listed.contains(&row.key) {
                                &follow.follow_id
                            } else {
                                "tracked"
                            },
                            Some(row.updated.clone()),
                            now,
                        );
                        work.row = Some(row.clone());
                        enqueue(state, work)?;
                        count += 1;
                    }
                }
                self.adopt_issues(store, follow, &listing.rows)?;
                if window.is_none() && complete {
                    let present = rows
                        .iter()
                        .map(|row| row.key.clone())
                        .collect::<BTreeSet<_>>();
                    let missing = members
                        .difference(&present)
                        .cloned()
                        .collect::<BTreeSet<_>>();
                    let previous = state.sources[&follow.follow_id].absent.clone();
                    self.sync_hold(store, state, follow, missing.len(), members.len())?;
                    if !listed.is_empty() && allowed_mass(missing.len(), members.len()) {
                        let confirmed = missing
                            .iter()
                            .filter(|id| confirmed(&previous, id, now))
                            .cloned()
                            .collect::<BTreeSet<_>>();
                        if *mode == LibraryFollowMode::Live {
                            self.drop_sync_members(store, state, follow, &confirmed, now)?;
                        } else {
                            for id in confirmed {
                                let mut work =
                                    candidate(source(follow, &id, "issue"), "tracked", None, now);
                                work.force = true;
                                work.confirm_missing = true;
                                enqueue(state, work)?;
                            }
                        }
                    }
                    state.sources.get_mut(&follow.follow_id).unwrap().absent =
                        absence(&previous, &missing, now);
                }
                Ok((count, complete))
            }
        }
    }

    fn adopt_pages(
        &self,
        store: &Store,
        follow: &LibraryFollowSummary,
        pages: &[SpacePage],
    ) -> Result<(), InspectionError> {
        let listed = pages
            .iter()
            .map(|page| (page.page_id.as_str(), page))
            .collect::<BTreeMap<_, _>>();
        let changed = store.mutate_index_if(|index| {
            let Some(record) = index
                .follows
                .iter()
                .find(|f| f.follow_id == follow.follow_id)
            else {
                return Ok((false, false));
            };
            let excluded = record.excluded_ids.clone();
            let ids = index
                .items
                .iter()
                .filter(|entry| follow::same_site_page(entry, follow))
                .filter_map(|entry| {
                    Some((
                        entry.summary.canonical_id.clone()?,
                        entry.summary.item_id.clone(),
                    ))
                })
                .collect::<BTreeMap<_, _>>();
            let mut changed = false;
            for entry in &mut index.items {
                if !follow::same_site_page(entry, follow) {
                    continue;
                }
                let Some(page) = entry
                    .summary
                    .canonical_id
                    .as_deref()
                    .and_then(|id| listed.get(id))
                else {
                    continue;
                };
                if excluded.contains(&page.page_id) {
                    continue;
                }
                if !refs::has_follow(&entry.summary, &follow.follow_id) {
                    refs::insert_ref(
                        &mut entry.summary,
                        LibraryItemRef::Follow {
                            follow_id: follow.follow_id.clone(),
                        },
                    );
                    changed = true;
                }
                let parent = page.ancestors.last().and_then(|id| ids.get(id).cloned());
                if entry.summary.parent_item_id != parent {
                    entry.summary.parent_item_id = parent;
                    changed = true;
                }
                if let Some(position) = page.position.and_then(|p| u32::try_from(p).ok()) {
                    if entry.summary.order != Some(position) {
                        entry.summary.order = Some(position);
                        changed = true;
                    }
                }
            }
            if changed {
                follow::recount(index, &follow.follow_id);
            }
            Ok((changed, changed))
        })?;
        self.sync_metadata_report(store, follow, changed)
    }
    fn adopt_issues(
        &self,
        store: &Store,
        follow: &LibraryFollowSummary,
        rows: &[IssueRow],
    ) -> Result<(), InspectionError> {
        let listed = rows
            .iter()
            .map(|row| (row.key.as_str(), row))
            .collect::<BTreeMap<_, _>>();
        let changed = store.mutate_index_if(|index| {
            let Some(record) = index
                .follows
                .iter()
                .find(|f| f.follow_id == follow.follow_id)
            else {
                return Ok((false, false));
            };
            let excluded = record.excluded_ids.clone();
            let mut changed = false;
            for entry in &mut index.items {
                if !jira_follow::is_issue_of(entry, follow) {
                    continue;
                }
                let Some(row) = entry
                    .summary
                    .canonical_id
                    .as_deref()
                    .and_then(|id| listed.get(id))
                else {
                    continue;
                };
                if excluded.contains(&row.key) {
                    continue;
                }
                if !refs::has_follow(&entry.summary, &follow.follow_id)
                    || refs::related_of(&entry.summary, &follow.follow_id)
                {
                    refs::insert_ref(
                        &mut entry.summary,
                        LibraryItemRef::Follow {
                            follow_id: follow.follow_id.clone(),
                        },
                    );
                    refs::strip_inclusion(
                        &mut entry.summary,
                        &LibraryInclusionHolder::Follow {
                            follow_id: follow.follow_id.clone(),
                        },
                    );
                    changed = true;
                }
                let same = entry.summary.issue.as_ref().is_some_and(|meta| {
                    meta.updated == row.updated
                        && meta.status == row.status
                        && meta.issue_type == row.issue_type
                        && meta.assignee == row.assignee
                });
                if !same {
                    entry.summary.issue = Some(LibraryIssueMeta {
                        updated: row.updated.clone(),
                        fetched_updated: entry
                            .summary
                            .issue
                            .as_ref()
                            .and_then(|m| m.fetched_updated.clone()),
                        status: row.status.clone(),
                        issue_type: row.issue_type.clone(),
                        assignee: row.assignee.clone(),
                    });
                    changed = true;
                }
            }
            if changed {
                follow::recount(index, &follow.follow_id);
            }
            Ok((changed, changed))
        })?;
        self.sync_metadata_report(store, follow, changed)
    }

    fn sync_metadata_report(
        &self,
        store: &Store,
        follow: &LibraryFollowSummary,
        changed: bool,
    ) -> Result<(), InspectionError> {
        if !changed {
            return Ok(());
        }
        let (operation, _lease) = operations::create_background(store)?;
        let result = operations::follow_row(
            store,
            &operation.operation_id,
            follow,
            LibraryReportOutcome::Updated,
            Some("Membership, hierarchy or source metadata reconciled".into()),
        );
        operations::finish(store, &operation.operation_id, result)
    }

    fn sync_operation(
        &self,
        store: &Store,
        state: &mut State,
    ) -> Result<(String, Lease), InspectionError> {
        let (record, lease) = operations::create_background(store)?;
        state.active_operation = Some(record.operation_id.clone());
        save(store, state)?;
        Ok((record.operation_id, lease))
    }
    fn drop_sync_members(
        &self,
        store: &Store,
        state: &mut State,
        follow: &LibraryFollowSummary,
        keys: &BTreeSet<String>,
        now: i64,
    ) -> Result<(), InspectionError> {
        if keys.is_empty() {
            return Ok(());
        }
        let Ok(_follow_lease) = store.lease(&follow.follow_id) else {
            return Ok(());
        };
        let (operation, _lease) = self.sync_operation(store, state)?;
        let result = store.mutate_index_if(|index| {
            let Some(record) = index
                .follows
                .iter()
                .find(|f| f.follow_id == follow.follow_id)
            else {
                return Ok((vec![], false));
            };
            let excluded = record.excluded_ids.clone();
            let mut dropped = vec![];
            for entry in &mut index.items {
                let matches = keys.contains(&entry.summary.item_id)
                    || entry.summary.canonical_id.as_ref().is_some_and(|id| {
                        keys.contains(id)
                            && entry.summary.provider_id.as_deref() == Some(&follow.provider_id)
                            && entry.summary.provider_instance.as_deref()
                                == Some(&follow.provider_instance)
                    });
                if !matches
                    || !refs::has_follow(&entry.summary, &follow.follow_id)
                    || excluded.contains(&entry.summary.item_id)
                    || entry
                        .summary
                        .canonical_id
                        .as_ref()
                        .is_some_and(|id| excluded.contains(id))
                {
                    continue;
                }
                dropped.push(entry.summary.clone());
                refs::remove_ref(
                    &mut entry.summary,
                    &LibraryItemRef::Follow {
                        follow_id: follow.follow_id.clone(),
                    },
                );
                if entry.summary.refs.is_empty() {
                    entry.summary.purge_after =
                        Some((now as u128 + refs::TOMBSTONE_GRACE_MS).to_string());
                }
            }
            let changed = !dropped.is_empty();
            if changed {
                follow::recount(index, &follow.follow_id);
            }
            Ok((dropped, changed))
        });
        let outcome = match result {
            Ok(dropped) => {
                for summary in dropped {
                    operations::row(
                        store,
                        &operation,
                        Some(&summary),
                        LibraryReportOutcome::Dropped,
                        Some("No longer a member after two complete inventories".into()),
                    )?;
                }
                Ok(())
            }
            Err(e) => Err(e),
        };
        operations::finish(store, &operation, outcome)?;
        state.active_operation = None;
        save(store, state)
    }
    async fn mark_sync_missing(
        &self,
        store: &Arc<Store>,
        state: &mut State,
        follow: &LibraryFollowSummary,
        id: &str,
        code: &str,
        reason: &str,
    ) -> Result<(), InspectionError> {
        let item = follow::page_item_id(follow, id);
        let Ok(_lease) = store.lease(&item) else {
            return Ok(());
        };
        let Some(mut entry) = self.entry(store, &item)? else {
            return Ok(());
        };
        if entry.summary.state == LibraryItemState::RemovedAtSource {
            return Ok(());
        }
        let (operation, _op_lease) = self.sync_operation(store, state)?;
        entry.summary.state = LibraryItemState::RemovedAtSource;
        entry.summary.diagnostics = vec![ProjectDiagnostic {
            code: code.into(),
            message: reason.into(),
            path: None,
        }];
        let result = store.update(entry.clone()).and_then(|()| {
            operations::row(
                store,
                &operation,
                Some(&entry.summary),
                LibraryReportOutcome::RemovedAtSource,
                Some(reason.into()),
            )
        });
        operations::finish(store, &operation, result)?;
        state.active_operation = None;
        save(store, state)
    }

    async fn discover_standalone(
        &self,
        store: &Arc<Store>,
        state: &mut State,
        config: &LibrarySyncConfiguration,
        now: i64,
        report: &mut LibrarySyncTick,
    ) -> Result<(), InspectionError> {
        let snapshot = index(store)?;
        let mut groups: BTreeMap<String, Vec<LibraryIndexEntry>> = BTreeMap::new();
        for entry in snapshot.items {
            if entry.summary.refs.is_empty() {
                continue;
            }
            let Some(provider) = self.configuration.providers.iter().find(|p| {
                Some(&p.id) == entry.summary.provider_id.as_ref()
                    && matches!(p.kind, ProviderKind::Jira | ProviderKind::Confluence)
            }) else {
                continue;
            };
            // Related-only items are standalone checks, even when a follow owns them.
            let seed = snapshot.follows.iter().any(|f| {
                refs::has_follow(&entry.summary, &f.follow_id)
                    && !refs::related_of(&entry.summary, &f.follow_id)
                    && f.provider_id == provider.id
                    && Some(&f.provider_instance) == entry.summary.provider_instance.as_ref()
            });
            if seed {
                continue;
            }
            groups
                .entry(format!(
                    "standalone:{}:{}",
                    provider.id,
                    entry
                        .summary
                        .provider_instance
                        .as_deref()
                        .unwrap_or_default()
                ))
                .or_default()
                .push(entry);
        }
        let mut groups = groups.into_iter().collect::<Vec<_>>();
        groups
            .sort_by_key(|(group, _)| state.sources.get(group).map_or(0, |s| s.next_inventory_ms));
        for (group, entries) in groups {
            if state
                .sources
                .entry(group.clone())
                .or_default()
                .next_inventory_ms
                > now
            {
                continue;
            }
            let provider = entries[0]
                .summary
                .provider_id
                .as_deref()
                .unwrap_or_default();
            if self.sync_blocked(state, provider, now) {
                continue;
            }
            let is_issue = entries[0].summary.resource_type.as_deref() == Some("issue");
            if state.sources[&group].inventory.is_none() {
                let ids = entries
                    .iter()
                    .filter_map(|entry| entry.summary.canonical_id.clone())
                    .collect::<BTreeSet<_>>()
                    .into_iter()
                    .collect::<Vec<_>>();
                state.sources.get_mut(&group).unwrap().inventory = Some(StandaloneInventory {
                    ids,
                    cursor: 0,
                    present: BTreeSet::new(),
                    batch_size: 100,
                    requests_per_id: if is_issue {
                        1
                    } else {
                        2.min(config.hourly_request_cap)
                    },
                });
            }
            state.sources.get_mut(&group).unwrap().next_inventory_ms = now.saturating_add(MINUTE);
            save(store, state)?; // durable cohort before the first remote request
            let mut attempted_size = 100;
            let result: Result<bool, InspectionError> = async {
                let cancel = AtomicBool::new(false);
                // Yield among due groups and leave publication a turn even for
                // providers without request-accounting support.
                for _ in 0..10 {
                    let progress = state.sources[&group].inventory.as_ref().unwrap();
                    if progress.cursor == progress.ids.len() {
                        return Ok(true);
                    }
                    let remaining = self.sources.background_requests_remaining(provider);
                    // Preserve publication headroom. This is an estimate only;
                    // the HTTP origin still charges every actual hop.
                    let reserve = (config.hourly_request_cap / 4).min(32);
                    let affordable = remaining.map_or(100, |remaining| {
                        remaining.saturating_sub(reserve) / progress.requests_per_id
                    });
                    let size = progress.batch_size.min(affordable as usize);
                    if size == 0 {
                        return Ok(false);
                    }
                    attempted_size = size;
                    let end = (progress.cursor + size).min(progress.ids.len());
                    let ids = progress.ids[progress.cursor..end].to_vec();
                    let mut observed = BTreeSet::new();
                    let mut candidates = Vec::new();
                    if is_issue {
                        let listing = self
                            .sources
                            .list_issues(
                                provider,
                                &IssueQuery::Keys(&ids),
                                ids.len() as u32,
                                &cancel,
                            )
                            .await?;
                        if !listing.complete {
                            return Err(error(
                                "library_sync_partial",
                                "Standalone issue check was incomplete",
                            ));
                        }
                        for row in listing.rows {
                            if !ids.contains(&row.key) {
                                return Err(error(
                                    "source_provider_contract",
                                    "Standalone issue check returned an unsolicited identity",
                                ));
                            }
                            observed.insert(row.key.clone());
                            let Some(old) = entries.iter().find(|entry| {
                                entry.summary.canonical_id.as_ref() == Some(&row.key)
                            }) else {
                                continue;
                            };
                            if jira_follow::change_reason(old, &row).is_some() {
                                let source = SourceRef {
                                    provider_id: provider.into(),
                                    provider_instance: old
                                        .summary
                                        .provider_instance
                                        .clone()
                                        .unwrap_or_default(),
                                    resource_type: "issue".into(),
                                    canonical_id: row.key.clone(),
                                };
                                let mut work =
                                    candidate(source, &group, Some(row.updated.clone()), now);
                                work.row = Some(row);
                                candidates.push(work);
                            }
                        }
                    } else {
                        let listing = self.sources.page_versions(provider, &ids, &cancel).await?;
                        if !listing.complete {
                            return Err(error(
                                "library_sync_partial",
                                "Standalone page check was incomplete",
                            ));
                        }
                        for page in listing.pages {
                            observed.insert(page.page_id.clone());
                            let Some(old) = entries.iter().find(|entry| {
                                entry.summary.canonical_id.as_ref() == Some(&page.page_id)
                            }) else {
                                continue;
                            };
                            if follow::change_reason(old, &page).is_some() {
                                let source = SourceRef {
                                    provider_id: provider.into(),
                                    provider_instance: old
                                        .summary
                                        .provider_instance
                                        .clone()
                                        .unwrap_or_default(),
                                    resource_type: "page".into(),
                                    canonical_id: page.page_id.clone(),
                                };
                                let mut work =
                                    candidate(source, &group, Some(page.version.to_string()), now);
                                work.force = true;
                                work.page = Some(page);
                                candidates.push(work);
                            }
                        }
                    }
                    // Never advance a cursor without all the chunk's candidates.
                    for work in candidates {
                        enqueue(state, work)?;
                        report.discovered += 1;
                    }
                    let after = self.sources.background_requests_remaining(provider);
                    let progress = state
                        .sources
                        .get_mut(&group)
                        .unwrap()
                        .inventory
                        .as_mut()
                        .unwrap();
                    if let Some((before, after)) = remaining.zip(after) {
                        let cost = before
                            .saturating_sub(after)
                            .div_ceil(ids.len() as u32)
                            .max(1);
                        progress.requests_per_id = progress.requests_per_id.max(cost);
                    }
                    progress.present.extend(observed);
                    progress.cursor = end;
                    save(store, state)?;
                }
                Ok(state.sources[&group].inventory.as_ref().unwrap().cursor
                    == state.sources[&group].inventory.as_ref().unwrap().ids.len())
            }
            .await;
            self.sync_blocked(state, provider, now);
            match result {
                Ok(true) => {
                    let progress = state.sources[&group].inventory.as_ref().unwrap();
                    let cohort = progress.ids.iter().cloned().collect::<BTreeSet<_>>();
                    // Re-read membership: additions outside the frozen cohort
                    // have never been checked, and removals must not be revived.
                    let current = index(store)?
                        .items
                        .into_iter()
                        .filter(|entry| {
                            !entry.summary.refs.is_empty()
                                && entry.summary.provider_id.as_deref() == Some(provider)
                                && entry.summary.provider_instance
                                    == entries[0].summary.provider_instance
                                && entry
                                    .summary
                                    .canonical_id
                                    .as_ref()
                                    .is_some_and(|id| cohort.contains(id))
                        })
                        .collect::<Vec<_>>();
                    let missing = current
                        .iter()
                        .filter_map(|entry| entry.summary.canonical_id.clone())
                        .filter(|id| !progress.present.contains(id))
                        .collect::<BTreeSet<_>>();
                    let previous_absent = state.sources[&group].absent.clone();
                    if allowed_mass(missing.len(), current.len()) {
                        for entry in &current {
                            let Some(id) = entry.summary.canonical_id.as_ref() else {
                                continue;
                            };
                            if missing.contains(id)
                                && confirmed(&previous_absent, id, now)
                                && entry.summary.state != LibraryItemState::RemovedAtSource
                            {
                                let source = SourceRef {
                                    provider_id: provider.into(),
                                    provider_instance: entry
                                        .summary
                                        .provider_instance
                                        .clone()
                                        .unwrap_or_default(),
                                    resource_type: entry
                                        .summary
                                        .resource_type
                                        .clone()
                                        .unwrap_or_default(),
                                    canonical_id: id.clone(),
                                };
                                let mut work = candidate(source, &group, None, now);
                                work.force = true;
                                work.confirm_missing = true;
                                enqueue(state, work)?;
                            }
                        }
                    }
                    let s = state.sources.get_mut(&group).unwrap();
                    s.absent = absence(&previous_absent, &missing, now);
                    s.inventory = None;
                    s.failures = 0;
                    s.last_error = None;
                    s.last_success_ms = Some(now);
                    s.next_inventory_ms =
                        schedule(now, i64::from(config.inventory_hours) * HOUR, &group);
                }
                Ok(false) => {
                    state.sources.get_mut(&group).unwrap().last_error =
                        Some("library_sync_budget_deferred".into());
                }
                Err(e) => {
                    let s = state.sources.get_mut(&group).unwrap();
                    // An unusually expensive chunk can exceed an entire budget,
                    // not just its remainder. Reduce it durably before retrying.
                    if e.code == "source_rate_limited" {
                        let progress = s.inventory.as_mut().unwrap();
                        progress.batch_size = (attempted_size / 2).max(1);
                    }
                    s.failures = s.failures.saturating_add(1);
                    s.last_error = Some(e.code);
                    s.next_inventory_ms = now.saturating_add(backoff(s.failures, 5 * MINUTE, HOUR));
                }
            }
            save(store, state)?;
        }
        Ok(())
    }

    async fn drain_sync(
        &self,
        store: &Arc<Store>,
        state: &mut State,
        now: i64,
        report: &mut LibrarySyncTick,
    ) -> Result<(), InspectionError> {
        let mut due = state
            .queue
            .iter()
            .filter(|(_, work)| work.next_attempt_ms <= now)
            .map(|(id, work)| {
                (
                    work.next_attempt_ms,
                    work.enqueued_ms,
                    work.page.as_ref().map_or(0, |page| page.ancestors.len()),
                    id.clone(),
                )
            })
            .collect::<Vec<_>>();
        due.sort();
        let started = Instant::now();
        for (_, _, _, id) in due
            .into_iter()
            .take(DRAIN_ITEMS.saturating_sub(report.fetched as usize))
        {
            if started.elapsed() >= Duration::from_secs(DRAIN_SECONDS) {
                break;
            }
            let Some(work) = state.queue.get(&id).cloned() else {
                continue;
            };
            if self.sync_blocked(state, &work.source.provider_id, now) {
                continue;
            }
            let Ok(_item_lease) = store.lease(&id) else {
                continue;
            };
            let snapshot = index(store)?;
            let owners = work
                .owners
                .iter()
                .filter(|owner| {
                    if owner.starts_with("follow:") {
                        snapshot.follows.iter().any(|follow| {
                            &follow.follow_id == *owner
                                && !follow.excluded_ids.contains(&work.source.canonical_id)
                                && !follow.excluded_ids.contains(&id)
                        })
                    } else {
                        snapshot.items.iter().any(|entry| {
                            entry.summary.item_id == id && !entry.summary.refs.is_empty()
                        })
                    }
                })
                .cloned()
                .collect::<BTreeSet<_>>();
            if owners.is_empty() {
                state.queue.remove(&id);
                save(store, state)?;
                continue;
            }
            let old = snapshot
                .items
                .iter()
                .find(|entry| entry.summary.item_id == id)
                .cloned();
            let matching = old.as_ref().is_some_and(|entry| {
                !work.force
                    && !work.confirm_missing
                    && work.revision.is_some()
                    && !matches!(
                        entry.summary.state,
                        LibraryItemState::Failed
                            | LibraryItemState::Unknown
                            | LibraryItemState::RemovedAtSource
                            | LibraryItemState::Conflict
                    )
                    && if work.source.resource_type == "issue" {
                        entry
                            .summary
                            .issue
                            .as_ref()
                            .and_then(|meta| meta.fetched_updated.as_ref())
                            == work.revision.as_ref()
                    } else {
                        entry.summary.source_revision == work.revision
                    }
            });
            if matching {
                self.apply_queue_owners(store, &work, &owners)?;
                state.queue.remove(&id);
                save(store, state)?;
                continue;
            }
            let conflicts = match &old {
                Some(old) => {
                    let _lock = store.shared()?;
                    store.conflicts(old)?
                }
                None => vec![],
            };
            let already_reported = old.as_ref().is_some_and(|old| {
                old.summary.state == LibraryItemState::Conflict
                    && !conflicts.is_empty()
                    && old.summary.conflict.len() == conflicts.len()
                    && old
                        .summary
                        .conflict
                        .iter()
                        .zip(&conflicts)
                        .all(|(old, current)| {
                            old.path == current.path && old.current_hash == current.current_hash
                        })
            });
            if already_reported {
                let failure = state.queue.get_mut(&id).unwrap();
                failure.failures = failure.failures.saturating_add(1);
                failure.last_error = Some("library_conflict".into());
                failure.next_attempt_ms =
                    now.saturating_add(backoff(failure.failures, MINUTE, 24 * HOUR));
                save(store, state)?;
                continue;
            }
            let (operation, _operation_lease) = self.sync_operation(store, state)?;
            let result: Result<(), InspectionError> = async {
                if let Some(old) = &old {
                    if !conflicts.is_empty() {
                        self.record_conflict(store, &operation, old.clone(), "Local edits were preserved during synchronization".into())?;
                        return Err(error("library_conflict", "Local edits need manual resolution"));
                    }
                }
                let request = if work.source.resource_type == "page" {
                    let authority = sources::site_authority(&self.configuration, &work.source.provider_id)?;
                    if authority.provider_instance != work.source.provider_instance { return Err(error("source_authority_mismatch", "Queued page belongs to another configured site")); }
                    sources::SourceFetchRequest { provider_id: work.source.provider_id.clone(), artifact_url: sources::confluence_page_url(&authority.provider_instance, &work.source.canonical_id), authority }
                } else {
                    let base = self.configuration.providers.iter().find(|p| p.id == work.source.provider_id)
                        .ok_or_else(|| error("source_provider_unsupported", "Queued provider is unavailable"))?.base_url.trim_end_matches('/');
                    self.request(&format!("{base}/browse/{}", work.source.canonical_id), Some(&work.source.provider_id))?
                };
                let fetched = {
                    let future = self.sources.fetch_assets(request);
                    tokio::pin!(future);
                    loop {
                        tokio::select! {
                            result = &mut future => break result,
                            _ = tokio::time::sleep(Duration::from_millis(100)) => {
                                if operations::cancelled(store, &operation)? { return Err(error("library_sync_yielded", "Background synchronization yielded to manual work")); }
                            }
                        }
                    }
                }?;
                if operations::cancelled(store, &operation)? {
                    return Err(error("library_sync_yielded", "Background synchronization yielded to manual work"));
                }
                let asset = fetched.assets.into_iter().find(|asset| item_id(&asset.source) == id)
                    .ok_or_else(|| error("source_identity_mismatch", "Queued refresh returned a different source identity"))?;
                let seed = ReferenceSeed { source: asset.source.clone(), label: sources::asset_label(&asset), references: sources::asset_references(&self.configuration, &asset) };
                let download = snapshot.follows.iter().any(|follow| owners.contains(&follow.follow_id) && follow.include_attachments);
                // Hold publication to a currently eligible owner, not the
                // pre-fetch snapshot. Store checks it again under the actual
                // mutation lock after any attachment download awaits.
                let current = index(store)?;
                let summary = super::asset_entry(&asset, old.as_ref()).summary;
                let reference = owners.iter().filter(|owner| owner.starts_with("follow:"))
                    .map(|owner| LibraryItemRef::Follow { follow_id: owner.clone() })
                    .find(|reference| Store::reference_allowed(&current, &summary, reference));
                let standalone = owners.iter().any(|owner| !owner.starts_with("follow:"))
                    && current.items.iter().any(|entry| entry.summary.item_id == id && !entry.summary.refs.is_empty());
                if reference.is_none() && !standalone {
                    return Err(error("library_follow_excluded", "Queued follow was stopped or this item was excluded"));
                }
                // Listing timestamps are a hint, not proof of the revision of this
                // fetch. Only mark fetched_updated when the fetched revision agrees.
                let row = work.row.as_ref().filter(|row| asset.source_revision.as_deref()
                    .is_some_and(|revision| same_listing_revision(revision, &row.updated)));
                self.save_asset_with(store, &operation, asset, old.clone(), SaveOptions { download_all: download,
                    issue_row: row, reference, ..SaveOptions::default() }).await?;
                if operations::cancelled(store, &operation)? {
                    return Err(error("library_sync_yielded", "Background synchronization yielded to manual work"));
                }
                let saved = self.entry(store, &id)?.ok_or_else(|| error("library_item_not_found", "Queued snapshot was not published"))?;
                if saved.summary.state == LibraryItemState::Unknown {
                    return Err(error("library_sync_partial", "Incomplete source content remains queued"));
                }
                if matches!(saved.summary.state, LibraryItemState::Conflict | LibraryItemState::Failed | LibraryItemState::RemovedAtSource) {
                    return Err(error("library_conflict", "Snapshot publication did not complete"));
                }
                self.apply_queue_owners(store, &work, &owners)?;
                // Hourly growth traverses only freshly fetched seeds, never drops.
                for follow in snapshot.follows.iter().filter(|follow| owners.contains(&follow.follow_id)
                    && follow.reference_depth.unwrap_or(0) > 0
                    && state.sources.get(&follow.follow_id).is_some_and(|s| s.next_related_ms > now)) {
                    let holder = LibraryInclusionHolder::Follow { follow_id: follow.follow_id.clone() };
                    let reference = LibraryItemRef::Follow { follow_id: follow.follow_id.clone() };
                    let skip = |related: &sources::RelatedAsset| follow.excluded_ids.contains(&item_id(&related.asset.source))
                        || (related.asset.source.provider_id == follow.provider_id && related.asset.source.provider_instance == follow.provider_instance
                            && follow.excluded_ids.contains(&related.asset.source.canonical_id));
                    let pass = self.run_related(store, &operation, vec![seed.clone()], 0, follow.reference_depth.unwrap_or(0), TraversalBudget::Query, &holder, &reference, &skip).await?;
                    self.related_row(store, &operation, &pass)?;
                }
                if let Some(depth) = old.as_ref().and_then(|entry| entry.summary.reference_depth).filter(|depth| *depth > 0
                    && state.sources.get(&format!("item:{id}")).is_some_and(|s| s.next_related_ms > now)) {
                    let holder = LibraryInclusionHolder::Item { item_id: id.clone() };
                    let pass = self.run_related(store, &operation, vec![seed], 0, depth, TraversalBudget::Single,
                        &holder, &LibraryItemRef::Manual, &|_: &sources::RelatedAsset| false).await?;
                    self.related_row(store, &operation, &pass)?;
                }
                Ok(())
            }.await;
            self.sync_blocked(state, &work.source.provider_id, now);
            match result {
                Ok(()) => {
                    report.fetched += 1;
                    state.queue.remove(&id);
                    operations::finish(store, &operation, Ok(()))?;
                }
                Err(e) => {
                    if work.confirm_missing && e.code == "source_not_found" {
                        if let Some(old) = old {
                            self.fetch_failed(store, &operation, old, e, true)?;
                            state.queue.remove(&id);
                        }
                        operations::finish(store, &operation, Ok(()))?;
                    } else {
                        let failure = state.queue.get_mut(&id).unwrap();
                        failure.failures = failure.failures.saturating_add(1);
                        failure.last_error = Some(e.code.clone());
                        failure.next_attempt_ms =
                            now.saturating_add(backoff(failure.failures, MINUTE, 24 * HOUR));
                        if e.code == "library_sync_yielded" {
                            operations::finish(store, &operation, Ok(()))?;
                        } else {
                            operations::finish(store, &operation, Err(e))?;
                        }
                    }
                }
            }
            state.active_operation = None;
            save(store, state)?;
        }
        Ok(())
    }
    fn apply_queue_owners(
        &self,
        store: &Store,
        work: &Candidate,
        owners: &BTreeSet<String>,
    ) -> Result<(), InspectionError> {
        for owner in owners.iter().filter(|id| id.starts_with("follow:")) {
            let snapshot = index(store)?;
            let Some(follow) = snapshot.follows.iter().find(|f| &f.follow_id == owner) else {
                continue;
            };
            if let Some(page) = &work.page {
                self.adopt_pages(store, follow, std::slice::from_ref(page))?;
            } else if let Some(row) = &work.row {
                self.adopt_issues(store, follow, std::slice::from_ref(row))?;
            }
        }
        Ok(())
    }

    async fn sync_related_due(
        &self,
        store: &Arc<Store>,
        state: &mut State,
        follows: &[LibraryFollowSummary],
        config: &LibrarySyncConfiguration,
        now: i64,
    ) -> Result<(), InspectionError> {
        for follow in follows
            .iter()
            .filter(|f| f.reference_depth.unwrap_or(0) > 0)
        {
            if state
                .sources
                .get(&follow.follow_id)
                .is_some_and(|s| s.next_related_ms > now)
            {
                continue;
            }
            if self.sync_blocked(state, &follow.provider_id, now) {
                continue;
            }
            let Ok(_lease) = store.lease(&follow.follow_id) else {
                continue;
            };
            state
                .sources
                .entry(follow.follow_id.clone())
                .or_default()
                .next_related_ms = schedule(
                now,
                i64::from(config.related_hours) * HOUR,
                &follow.follow_id,
            );
            save(store, state)?;
            let snapshot = index(store)?;
            let member_count = snapshot
                .items
                .iter()
                .filter(|entry| refs::has_follow(&entry.summary, &follow.follow_id))
                .count();
            let mut seeds = vec![];
            let mut unknown = 0;
            for entry in snapshot.items.iter().filter(|entry| {
                refs::has_follow(&entry.summary, &follow.follow_id)
                    && !refs::related_of(&entry.summary, &follow.follow_id)
            }) {
                if let (Some(provider), Some(instance), Some(kind), Some(id), Some(references)) = (
                    &entry.summary.provider_id,
                    &entry.summary.provider_instance,
                    &entry.summary.resource_type,
                    &entry.summary.canonical_id,
                    &entry.references,
                ) {
                    seeds.push(ReferenceSeed {
                        source: SourceRef {
                            provider_id: provider.clone(),
                            provider_instance: instance.clone(),
                            resource_type: kind.clone(),
                            canonical_id: id.clone(),
                        },
                        label: entry.summary.title.clone(),
                        references: references.clone(),
                    });
                    if matches!(
                        entry.summary.state,
                        LibraryItemState::Failed
                            | LibraryItemState::Conflict
                            | LibraryItemState::RemovedAtSource
                    ) {
                        unknown += 1;
                    }
                } else {
                    unknown += 1;
                }
            }
            // No outgoing references: an empty traversal is a metadata-only pass.
            let outgoing = seeds.iter().any(|seed| !seed.references.is_empty());
            let (operation, _op_lease) = if outgoing {
                self.sync_operation(store, state)?
            } else {
                // Complete empty reachability still reconciles former related refs.
                let missing = snapshot
                    .items
                    .iter()
                    .filter(|entry| refs::related_of(&entry.summary, &follow.follow_id))
                    .map(|entry| entry.summary.item_id.clone())
                    .collect::<BTreeSet<_>>();
                let previous = state.sources[&follow.follow_id].related_absent.clone();
                if unknown == 0 {
                    state
                        .sources
                        .get_mut(&follow.follow_id)
                        .unwrap()
                        .related_absent = absence(&previous, &missing, now);
                } else {
                    let s = state.sources.get_mut(&follow.follow_id).unwrap();
                    s.next_related_ms = now.saturating_add(5 * MINUTE);
                    s.last_error = Some("library_sync_related_partial".into());
                }
                drop(_lease);
                if unknown == 0 && allowed_mass(missing.len(), member_count) {
                    let confirmed = missing
                        .into_iter()
                        .filter(|id| confirmed(&previous, id, now))
                        .collect();
                    self.drop_sync_members(store, state, follow, &confirmed, now)?;
                }
                save(store, state)?;
                continue;
            };
            let holder = LibraryInclusionHolder::Follow {
                follow_id: follow.follow_id.clone(),
            };
            let reference = LibraryItemRef::Follow {
                follow_id: follow.follow_id.clone(),
            };
            let skip = |related: &sources::RelatedAsset| {
                follow
                    .excluded_ids
                    .contains(&item_id(&related.asset.source))
                    || (related.asset.source.provider_id == follow.provider_id
                        && related.asset.source.provider_instance == follow.provider_instance
                        && follow
                            .excluded_ids
                            .contains(&related.asset.source.canonical_id))
            };
            let result = self
                .run_related(
                    store,
                    &operation,
                    seeds,
                    unknown,
                    follow.reference_depth.unwrap_or(0),
                    TraversalBudget::Query,
                    &holder,
                    &reference,
                    &skip,
                )
                .await;
            self.sync_blocked(state, &follow.provider_id, now);
            match result {
                Ok(pass) => {
                    self.related_row(store, &operation, &pass)?;
                    operations::finish(store, &operation, Ok(()))?;
                    state.active_operation = None;
                    if pass.complete {
                        let missing = snapshot
                            .items
                            .iter()
                            .filter(|entry| {
                                refs::related_of(&entry.summary, &follow.follow_id)
                                    && !pass.reached.contains(&entry.summary.item_id)
                            })
                            .map(|entry| entry.summary.item_id.clone())
                            .collect::<BTreeSet<_>>();
                        let previous = state.sources[&follow.follow_id].related_absent.clone();
                        state
                            .sources
                            .get_mut(&follow.follow_id)
                            .unwrap()
                            .related_absent = absence(&previous, &missing, now);
                        drop(_lease);
                        if allowed_mass(missing.len(), member_count) {
                            let confirmed = missing
                                .into_iter()
                                .filter(|id| confirmed(&previous, id, now))
                                .collect();
                            self.drop_sync_members(store, state, follow, &confirmed, now)?;
                        }
                    }
                    if !pass.complete {
                        let s = state.sources.get_mut(&follow.follow_id).unwrap();
                        s.next_related_ms = now.saturating_add(5 * MINUTE);
                        s.last_error = Some("library_sync_related_partial".into());
                    }
                }
                Err(e) => {
                    operations::finish(store, &operation, Err(e))?;
                    state.active_operation = None;
                    state
                        .sources
                        .get_mut(&follow.follow_id)
                        .unwrap()
                        .next_related_ms = now.saturating_add(5 * MINUTE);
                }
            }
            save(store, state)?;
        }
        Ok(())
    }

    async fn sync_single_related_due(
        &self,
        store: &Arc<Store>,
        state: &mut State,
        config: &LibrarySyncConfiguration,
        now: i64,
    ) -> Result<(), InspectionError> {
        let snapshot = index(store)?;
        for entry in snapshot.items.iter().filter(|entry| {
            !entry.summary.refs.is_empty() && entry.summary.reference_depth.unwrap_or(0) > 0
        }) {
            let id = &entry.summary.item_id;
            let key = format!("item:{id}");
            let (Some(provider), Some(instance), Some(kind), Some(canonical), Some(references)) = (
                &entry.summary.provider_id,
                &entry.summary.provider_instance,
                &entry.summary.resource_type,
                &entry.summary.canonical_id,
                &entry.references,
            ) else {
                continue;
            };
            if !self.configuration.providers.iter().any(|p| {
                &p.id == provider && matches!(p.kind, ProviderKind::Jira | ProviderKind::Confluence)
            }) {
                continue;
            }
            let previous = state.sources.entry(key.clone()).or_default().clone();
            if previous.next_related_ms > now {
                continue;
            }
            if self.sync_blocked(state, provider, now)
                || matches!(
                    entry.summary.state,
                    LibraryItemState::Conflict
                        | LibraryItemState::Failed
                        | LibraryItemState::RemovedAtSource
                )
            {
                continue;
            }
            let Ok(_item_lease) = store.lease(id) else {
                continue;
            };
            state.sources.get_mut(&key).unwrap().next_related_ms =
                schedule(now, i64::from(config.related_hours) * HOUR, &key);
            save(store, state)?;
            let holder = LibraryInclusionHolder::Item {
                item_id: id.clone(),
            };
            let seed = ReferenceSeed {
                source: SourceRef {
                    provider_id: provider.clone(),
                    provider_instance: instance.clone(),
                    resource_type: kind.clone(),
                    canonical_id: canonical.clone(),
                },
                label: entry.summary.title.clone(),
                references: references.clone(),
            };
            let pass = if references.is_empty() {
                super::related::RelatedPass::none()
            } else {
                let (operation, _operation_lease) = self.sync_operation(store, state)?;
                let result = self
                    .run_related(
                        store,
                        &operation,
                        vec![seed],
                        0,
                        entry.summary.reference_depth.unwrap_or(0),
                        TraversalBudget::Single,
                        &holder,
                        &LibraryItemRef::Manual,
                        &|_: &sources::RelatedAsset| false,
                    )
                    .await;
                self.sync_blocked(state, provider, now);
                match result {
                    Ok(pass) => {
                        self.related_row(store, &operation, &pass)?;
                        operations::finish(store, &operation, Ok(()))?;
                        state.active_operation = None;
                        pass
                    }
                    Err(e) => {
                        operations::finish(store, &operation, Err(e))?;
                        state.active_operation = None;
                        state.sources.get_mut(&key).unwrap().next_related_ms =
                            now.saturating_add(5 * MINUTE);
                        save(store, state)?;
                        continue;
                    }
                }
            };
            if pass.complete && !pass.cancelled {
                let held = |item: &&LibraryIndexEntry| {
                    item.summary
                        .included_by
                        .iter()
                        .flatten()
                        .any(|inclusion| inclusion.holder == holder)
                };
                let members = snapshot.items.iter().filter(held).collect::<Vec<_>>();
                let missing = members
                    .iter()
                    .filter(|item| !pass.reached.contains(&item.summary.item_id))
                    .map(|item| item.summary.item_id.clone())
                    .collect::<BTreeSet<_>>();
                state.sources.get_mut(&key).unwrap().related_absent =
                    absence(&previous.related_absent, &missing, now);
                if allowed_mass(missing.len(), members.len()) {
                    let confirmed = missing
                        .iter()
                        .filter(|id| confirmed(&previous.related_absent, id, now))
                        .collect::<BTreeSet<_>>();
                    if !confirmed.is_empty() {
                        let changed = store.mutate_index_if(|index| {
                            let mut changed = false;
                            for item in &mut index.items {
                                if confirmed.contains(&item.summary.item_id)
                                    && item
                                        .summary
                                        .included_by
                                        .iter()
                                        .flatten()
                                        .any(|inclusion| inclusion.holder == holder)
                                {
                                    refs::strip_inclusion(&mut item.summary, &holder);
                                    changed = true;
                                }
                            }
                            Ok((changed, changed))
                        })?;
                        if changed {
                            let (operation, _operation_lease) =
                                self.sync_operation(store, state)?;
                            let result = operations::row(
                                store,
                                &operation,
                                Some(&entry.summary),
                                LibraryReportOutcome::Updated,
                                Some(
                                    "Related inclusions reconciled after two complete passes"
                                        .into(),
                                ),
                            );
                            operations::finish(store, &operation, result)?;
                            state.active_operation = None;
                        }
                    }
                }
            }
            if !pass.complete {
                let s = state.sources.get_mut(&key).unwrap();
                s.next_related_ms = now.saturating_add(5 * MINUTE);
                s.last_error = Some("library_sync_related_partial".into());
            }
            save(store, state)?;
        }
        Ok(())
    }

    async fn audit_sync(
        &self,
        store: &Arc<Store>,
        state: &mut State,
        config: &LibrarySyncConfiguration,
        now: i64,
    ) -> Result<(), InspectionError> {
        let snapshot = index(store)?;
        let mut groups: BTreeMap<String, Vec<&LibraryIndexEntry>> = BTreeMap::new();
        for entry in snapshot
            .items
            .iter()
            .filter(|entry| !entry.summary.refs.is_empty())
        {
            let Some(provider) = self.configuration.providers.iter().find(|p| {
                Some(&p.id) == entry.summary.provider_id.as_ref()
                    && matches!(p.kind, ProviderKind::Jira | ProviderKind::Confluence)
            }) else {
                continue;
            };
            groups
                .entry(format!(
                    "{}:{}",
                    provider.id,
                    entry
                        .summary
                        .provider_instance
                        .as_deref()
                        .unwrap_or_default()
                ))
                .or_default()
                .push(entry);
        }
        for (group, mut entries) in groups {
            entries.sort_by_key(|entry| &entry.summary.item_id);
            let audit = state.audits.entry(group.clone()).or_default().clone();
            if audit.next_due_ms > now {
                continue;
            }
            // Spread a whole audit across audit_days. A failed page has its own
            // retry slot; it cannot pin the cursor and starve later pages.
            let count = entries
                .len()
                .div_ceil(config.audit_days as usize * 24 * 60)
                .clamp(1, 32);
            let start = audit
                .cursor
                .as_ref()
                .map(|id| entries.partition_point(|entry| entry.summary.item_id <= *id))
                .unwrap_or(0);
            let interval = (i64::from(config.audit_days) * 24 * HOUR
                / entries.len().div_ceil(count).max(1) as i64)
                .max(MINUTE);
            state.audits.get_mut(&group).unwrap().next_due_ms = now.saturating_add(interval);
            save(store, state)?;
            let start = if start >= entries.len() { 0 } else { start };
            let mut selected = entries
                .iter()
                .skip(start)
                .take(count)
                .map(|entry| (*entry, true))
                .collect::<Vec<_>>();
            if let Some(retry) = entries.iter().find(|entry| {
                audit
                    .failures
                    .get(&entry.summary.item_id)
                    .is_some_and(|failure| failure.next_attempt_ms <= now)
                    && !selected
                        .iter()
                        .any(|(selected, _)| selected.summary.item_id == entry.summary.item_id)
            }) {
                selected.push((*retry, false));
            }
            for (entry, advance_cursor) in selected {
                let Some(provider) = &entry.summary.provider_id else {
                    continue;
                };
                if self.sync_blocked(state, provider, now) {
                    break;
                }
                let Some(id) = &entry.summary.canonical_id else {
                    continue;
                };
                let source = SourceRef {
                    provider_id: provider.clone(),
                    provider_instance: entry.summary.provider_instance.clone().unwrap_or_default(),
                    resource_type: entry.summary.resource_type.clone().unwrap_or_default(),
                    canonical_id: id.clone(),
                };
                let mut failure = None;
                if source.resource_type == "page" {
                    match self.sources.page_aux(provider, id).await {
                        Ok(aux) if aux.labels_complete && aux.attachments_complete => {
                            let labels = saved_labels(store, entry)?;
                            let mut current = aux.labels;
                            current.sort();
                            let attachments_equal = aux.attachments.len()
                                == entry.summary.attachments.len()
                                && aux.attachments.iter().all(|remote| {
                                    entry.summary.attachments.iter().any(|saved| {
                                        saved.attachment_id == remote.id
                                            && saved.version == remote.source_revision
                                            && saved.original_name == remote.title
                                            && saved.bytes == remote.size
                                            && saved.media_type == remote.media_type
                                    })
                                });
                            if labels.as_ref() != Some(&current) || !attachments_equal {
                                let mut work = candidate(source, "audit", None, now);
                                work.force = true;
                                enqueue(state, work)?;
                            }
                        }
                        Ok(_) => failure = Some("library_sync_partial".to_owned()),
                        Err(e) => {
                            self.sync_blocked(state, provider, now);
                            failure = Some(e.code);
                        }
                    }
                } else if source.resource_type == "issue" {
                    // Jira children have no independent metadata revision contract;
                    // the rotating audit is intentionally a bounded content fetch.
                    let mut work = candidate(source, "audit", None, now);
                    work.force = true;
                    enqueue(state, work)?;
                }
                let progress = state.audits.get_mut(&group).unwrap();
                if advance_cursor {
                    progress.cursor = Some(entry.summary.item_id.clone());
                }
                if let Some(code) = failure {
                    let retry = progress
                        .failures
                        .entry(entry.summary.item_id.clone())
                        .or_default();
                    retry.failures = retry.failures.saturating_add(1);
                    retry.next_attempt_ms =
                        now.saturating_add(backoff(retry.failures, 5 * MINUTE, 24 * HOUR));
                    retry.last_error = code;
                    progress.next_due_ms = progress.next_due_ms.min(retry.next_attempt_ms);
                } else {
                    progress.failures.remove(&entry.summary.item_id);
                }
                save(store, state)?;
            }
        }
        Ok(())
    }
}

fn same_listing_revision(revision: &str, listing: &str) -> bool {
    // Jira's stored listing presentation preserves the API's wall date/time,
    // while the durable delta checkpoint itself uses exact epoch milliseconds.
    revision == listing
        || revision.get(..10) == listing.get(..10)
            && revision.get(11..19).is_some()
            && revision.get(11..19) == listing.get(11..19)
}

fn saved_labels(
    store: &Store,
    entry: &LibraryIndexEntry,
) -> Result<Option<Vec<String>>, InspectionError> {
    let Some(document) = entry
        .summary
        .document_path
        .as_deref()
        .and_then(|path| path.strip_prefix(&format!("{}/", entry.summary.item_path)))
    else {
        return Ok(None);
    };
    if document.contains(['/', '\\']) {
        return Err(error("library_corrupt", "Unsafe page document path"));
    }
    let dir = store.item_dir(&entry.summary.item_path)?;
    let mut options = OpenOptions::new();
    options
        .read(true)
        .follow(cap_fs_ext::FollowSymlinks::No)
        .nonblock(true);
    let mut file = dir
        .open_with(document, &options)
        .map_err(|e| error("library_unavailable", e.to_string()))?;
    if !file
        .metadata()
        .map_err(|e| error("library_unavailable", e.to_string()))?
        .is_file()
    {
        return Err(error(
            "library_corrupt",
            "Page document is not a regular file",
        ));
    }
    let mut bytes = Vec::new();
    file.by_ref()
        .take(1024 * 1024)
        .read_to_end(&mut bytes)
        .map_err(|e| error("library_unavailable", e.to_string()))?;
    let text = String::from_utf8_lossy(&bytes);
    if !text.starts_with("---\n") {
        return Ok(None);
    }
    for line in text.lines().skip(1).take_while(|line| *line != "---") {
        if let Some(value) = line.strip_prefix("labels: ") {
            let mut labels: Vec<String> = match serde_json::from_str(value) {
                Ok(labels) => labels,
                Err(_) => return Ok(None),
            };
            labels.sort();
            return Ok(Some(labels));
        }
    }
    Ok(Some(vec![]))
}

/// Manual mutations cancel only an actively owned background operation. They do
/// not wait behind discovery, pacing or the durable backlog.
pub(super) fn preempt(store: &Store) -> Result<(), InspectionError> {
    let state = load(store)?;
    if let Some(operation) = state.active_operation {
        match operations::cancel(store, &operation) {
            Ok(_) => {}
            Err(e) if e.code == "library_operation_not_found" => {}
            Err(e) => return Err(e),
        }
    }
    Ok(())
}

pub(super) async fn manual_lease(store: &Store, id: &str) -> Result<Lease, InspectionError> {
    let until = Instant::now() + Duration::from_secs(5);
    loop {
        match store.lease(id) {
            Ok(lease) => return Ok(lease),
            Err(e) if e.code == "library_item_busy" && Instant::now() < until => {
                preempt(store)?;
                tokio::time::sleep(Duration::from_millis(50)).await;
            }
            Err(e) => return Err(e),
        }
    }
}
