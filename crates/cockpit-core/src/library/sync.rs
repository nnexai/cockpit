//! Bounded discovery and durable, revision-deduplicated background publication.
//! This file is independent of index freshness: quiet passes never publish an index.
use super::{
    LibraryService, SaveOptions, follow, item_id, operations, refs,
    store::{self, Index, Lease, LibraryIndexEntry, Store, error},
};
use crate::{
    InspectionError,
    config::LibrarySyncConfiguration,
    project_store::read_json_bounded,
    sources::{
        self, IssueQuery, IssueRow, ReferenceSeed, SourceRef, SpacePage,
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

#[path = "follow_plan/discover.rs"]
mod discover;
#[path = "follow_plan/drain.rs"]
mod drain;
#[path = "follow_plan/related_due.rs"]
mod related_due;
#[path = "follow_plan/standalone.rs"]
mod standalone;

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
fn schedule_follow_windows(
    source: &mut SourceState,
    follow_id: &str,
    config: &LibrarySyncConfiguration,
    now: i64,
    delta_due: bool,
    inventory_due: bool,
) {
    source.last_started_ms = now;
    if delta_due {
        source.next_delta_ms = now.saturating_add(i64::from(config.delta_minutes) * MINUTE);
        if source.window.is_none() {
            let upper = (now
                .saturating_sub(i64::from(config.lag_allowance_minutes) * MINUTE)
                / MINUTE)
                * MINUTE;
            let lower = source
                .committed_upper_ms
                .unwrap_or(upper.saturating_sub(i64::from(config.delta_minutes) * MINUTE))
                .saturating_sub(i64::from(config.overlap_minutes) * MINUTE)
                .max(0);
            if lower < upper {
                source.window = Some((lower, upper));
            }
        }
    }
    if inventory_due {
        source.next_inventory_ms = schedule(
            now,
            i64::from(config.inventory_hours) * HOUR,
            follow_id,
        );
    }
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

/// Confirm delayed absence only below the mass-removal guard.
fn absence_step(
    previous: &BTreeMap<String, i64>,
    missing: &BTreeSet<String>,
    members: usize,
    now: i64,
) -> (BTreeSet<String>, BTreeMap<String, i64>) {
    let confirmed = if allowed_mass(missing.len(), members) {
        missing
            .iter()
            .filter(|id| confirmed(previous, id, now))
            .cloned()
            .collect()
    } else {
        BTreeSet::new()
    };
    (confirmed, absence(previous, missing, now))
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
                let source = state.sources.get_mut(&follow.follow_id).unwrap();
                schedule_follow_windows(
                    source,
                    &follow.follow_id,
                    config,
                    now,
                    delta_due,
                    inventory_due,
                );
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
