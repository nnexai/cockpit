//! Followed Jira JQL queries: input recognition, add and the refresh engine.
//! An issue is one Library item whatever follows list it; a follow only holds
//! a reference to it. A `live` follow mirrors its query, an `accumulate`
//! follow keeps every issue that ever matched.
use super::{
    LibraryService, SaveOptions,
    follow::{FOLLOW_FETCH_CONCURRENCY, FetchWork, FetchedBatch, recount},
    item_id, operations,
    refs::{self, TOMBSTONE_GRACE_MS},
    related::RelatedPass,
    store::{LibraryIndexEntry, Store, error},
};
use crate::{
    InspectionError,
    jira_query::{JiraQueryInput, has_relative_dates, instant_seconds, jira_query_input},
    project_store::timestamp,
    repositories::is_jira_executable,
    sources::{
        IssueListing, IssueQuery, IssueRow, ReferenceSeed, RelatedAsset, SourceAuthority, SourceRef,
        TraversalBudget, site_authority,
    },
};
use cockpit_protocol::library::*;
use sha2::{Digest, Sha256};
use std::{
    collections::{BTreeMap, BTreeSet},
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    time::Duration,
};

/// The preview and the widest single window: the CLI returns at most 100 rows a call.
const PREVIEW_ROWS: u32 = 100;

/// `follow:<sha256(provider_id \0 instance \0 "jql" \0 jql)>`. The `follow:` prefix
/// is what Space routing keys on; Confluence ids stay as they are.
pub(super) fn jira_follow_id(provider_id: &str, provider_instance: &str, jql: &str) -> String {
    let mut hash = Sha256::new();
    for value in [provider_id, provider_instance, "jql", jql] {
        hash.update(value.as_bytes());
        hash.update([0]);
    }
    format!("follow:{:x}", hash.finalize())
}

fn issue_item_id(follow: &LibraryFollowSummary, key: &str) -> String {
    item_id(&SourceRef {
        provider_id: follow.provider_id.clone(),
        provider_instance: follow.provider_instance.clone(),
        resource_type: "issue".into(),
        canonical_id: key.to_owned(),
    })
}

fn is_issue_of(entry: &LibraryIndexEntry, follow: &LibraryFollowSummary) -> bool {
    entry.summary.provider_id.as_deref() == Some(follow.provider_id.as_str())
        && entry.summary.provider_instance.as_deref() == Some(follow.provider_instance.as_str())
        && entry.summary.resource_type.as_deref() == Some("issue")
}

/// D4: why a listed issue needs a fetch, or `None` when the stored item
/// matches the row. Compares listing format with listing format. With a
/// reference depth, an item saved before references were extracted is fetched once;
/// so is any item saved before the structured relations (its parent issue) were captured.
fn change_reason(old: &LibraryIndexEntry, row: &IssueRow, depth: u32) -> Option<&'static str> {
    if matches!(
        old.summary.state,
        LibraryItemState::RemovedAtSource | LibraryItemState::Failed | LibraryItemState::Unknown
    ) {
        Some("rechecked")
    } else if old.summary.issue.as_ref().and_then(|meta| meta.fetched_updated.as_deref())
        != Some(row.updated.as_str())
    {
        Some("changed")
    } else if (depth > 0 && old.references.is_none()) || !old.relations_captured {
        Some("rechecked")
    } else {
        None
    }
}

/// The newest `source_revision` of the members, as the ISO string the probe takes.
fn watermark<'a>(members: impl Iterator<Item = &'a LibraryIndexEntry>) -> Option<String> {
    members
        .filter_map(|entry| {
            let revision = entry.summary.source_revision.as_deref()?;
            Some((instant_seconds(revision)?, revision))
        })
        .max_by_key(|(seconds, _)| *seconds)
        .map(|(_, revision)| revision.to_owned())
}

/// What a refresh needs to know about the Library before it plans its fetches.
struct Snapshot {
    items: BTreeMap<String, LibraryIndexEntry>,
    /// Keys of the items holding this follow.
    members: BTreeSet<String>,
    excluded: BTreeSet<String>,
    /// A previous run did not see the whole query, so it cannot be probed.
    unsettled: bool,
    /// Reference depth of the follow record (0 when none is stored).
    depth: u32,
}

/// One planned fetch of a Jira issue.
struct Pending {
    key: String,
    /// The listing row that led here; `None` for the confirmation of an absent member.
    row: Option<IssueRow>,
    old: Option<LibraryIndexEntry>,
    reason: Option<String>,
}

impl LibraryService {
    fn jira_site(&self, provider_id: &str) -> Result<SourceAuthority, InspectionError> {
        let jira = self.configuration.providers.iter().any(|provider| {
            provider.id == provider_id && is_jira_executable(&provider.executable)
        });
        if !jira {
            return Err(error(
                "source_provider_unsupported",
                "selected provider is not a configured Jira provider",
            ));
        }
        site_authority(&self.configuration, provider_id)
    }

    /// D10: the Jira query an input denotes and the provider that runs it, or
    /// `None` when the input is not a Jira query. A selected Jira provider always
    /// wins; without one, only JQL with operators is a query, and it needs an
    /// unambiguous provider. A bare project key alone is left to Confluence.
    pub(super) fn jira_query(
        &self,
        input: &str,
        selected: Option<&str>,
    ) -> Result<Option<(JiraQueryInput, String)>, InspectionError> {
        let jira = self
            .configuration
            .providers
            .iter()
            .filter(|provider| is_jira_executable(&provider.executable))
            .map(|provider| provider.id.as_str())
            .collect::<Vec<_>>();
        if jira.is_empty() {
            return Ok(None);
        }
        if let Some(selected) = selected {
            if !jira.contains(&selected) {
                return Ok(None);
            }
        }
        let Some(query) = jira_query_input(input)? else {
            return Ok(None);
        };
        let provider_id = match (selected, jira.as_slice()) {
            (Some(selected), _) => selected,
            (None, _) if query.bare_project => return Ok(None),
            (None, [only]) => only,
            (None, _) => {
                return Err(error(
                    "source_authority_mismatch",
                    "Select the Jira provider for this query",
                ));
            }
        };
        Ok(Some((query, provider_id.to_owned())))
    }

    fn suggested_mode(jql: &str) -> LibraryFollowMode {
        if has_relative_dates(jql) {
            LibraryFollowMode::Accumulate
        } else {
            LibraryFollowMode::Live
        }
    }

    /// A Jira query input: one preview window gives the count.
    pub(super) async fn resolve_jira_query(
        &self,
        provider_id: &str,
        query: &JiraQueryInput,
    ) -> Result<LibraryResolution, InspectionError> {
        let site = self.jira_site(provider_id)?;
        let listing = self
            .sources
            .list_issues(
                provider_id,
                &IssueQuery::Jql { jql: &query.jql, updated_since: None },
                PREVIEW_ROWS,
                &AtomicBool::new(false),
            )
            .await?;
        let id = jira_follow_id(provider_id, &site.provider_instance, &query.jql);
        let existing = {
            let store = self.open()?;
            let _lock = store.shared()?;
            store.index()?.follows.into_iter().find(|follow| follow.follow_id == id)
        };
        let follow_mode = match existing.as_ref().map(|follow| &follow.source) {
            Some(LibraryFollowSource::JiraQuery { mode, .. }) => *mode,
            _ => Self::suggested_mode(&query.jql),
        };
        let reference_depth = existing.as_ref().and_then(|follow| follow.reference_depth);
        Ok(LibraryResolution {
            kind: LibraryInputKind::JiraQuery,
            provider_id: Some(provider_id.to_owned()),
            provider_instance: Some(site.provider_instance),
            title: query.jql.clone(),
            canonical_id: Some(query.jql.clone()),
            container_label: None,
            existing_item_id: None,
            existing_follow_id: existing.map(|follow| follow.follow_id),
            item_count: Some(listing.rows.len() as u32),
            item_count_exact: listing.complete,
            follow_mode: Some(follow_mode),
            git_working_tree: None,
            file_count: None,
            diagnostics: vec![],
            reference_depth,
        })
    }

    /// Follow a JQL query. Following an already-followed query clears its
    /// exclusions and sets the mode, like `Follow whole space`.
    pub(super) async fn start_jira_follow_add(
        &self,
        request: LibraryAddRequest,
        query: JiraQueryInput,
        provider_id: String,
    ) -> Result<LibraryOperation, InspectionError> {
        if request.target.is_some() {
            return Err(error(
                "source_capability_unavailable",
                "A Jira query follow can't be added to a Space",
            ));
        }
        // Before any state or operation: an opt-in the provider can't honor
        // (no stored token, unavailable vault) refuses the whole add.
        if request.download_attachments {
            self.sources.attachment_downloads(&provider_id, "issue").await?;
        }
        let site = self.jira_site(&provider_id)?;
        let mode = request
            .follow_mode
            .unwrap_or_else(|| Self::suggested_mode(&query.jql));
        let follow = LibraryFollowSummary {
            follow_id: jira_follow_id(&provider_id, &site.provider_instance, &query.jql),
            provider_id,
            provider_instance: site.provider_instance,
            source: LibraryFollowSource::JiraQuery { jql: query.jql, mode },
            include_attachments: request.download_attachments,
            item_count: 0,
            partial: None,
            excluded_ids: vec![],
            last_refreshed_at: None,
            state: LibraryItemState::Unknown,
            reference_depth: (request.reference_depth > 0).then_some(request.reference_depth),
        };
        let handle = operations::runtime()?;
        let store = self.open()?;
        let lease = store.lease(&follow.follow_id)?;
        let (record, operation_lease) =
            operations::create(&store, LibraryOperationKind::Add, Some(0))?;
        let service = self.clone();
        let id = record.operation_id.clone();
        let worker_store = store.clone();
        operations::spawn(handle, store, id.clone(), operation_lease, async move {
            let _lease = lease;
            if operations::cancelled(&worker_store, &id)? {
                return Ok(());
            }
            service.refresh_jira_follow(&worker_store, &id, follow, true).await?;
            refs::purge_expired(&worker_store, &id)
        });
        Ok(record)
    }

    fn snapshot(&self, store: &Store, follow: &LibraryFollowSummary) -> Result<Snapshot, InspectionError> {
        let index = {
            let _lock = store.shared()?;
            store.index()?
        };
        let record = index.follows.iter().find(|f| f.follow_id == follow.follow_id);
        let excluded = record
            .map(|f| f.excluded_ids.iter().cloned().collect())
            .unwrap_or_default();
        let unsettled = record.is_some_and(|f| f.partial.is_some() || f.state == LibraryItemState::Partial);
        let depth = record.and_then(|f| f.reference_depth).unwrap_or(0);
        let members = index
            .items
            .iter()
            .filter(|entry| refs::has_follow(&entry.summary, &follow.follow_id))
            .filter(|entry| !refs::related_of(&entry.summary, &follow.follow_id))
            .filter_map(|entry| entry.summary.canonical_id.clone())
            .collect();
        let items = index
            .items
            .into_iter()
            .map(|entry| (entry.summary.item_id.clone(), entry))
            .collect();
        Ok(Snapshot { items, members, excluded, unsettled, depth })
    }

    /// A listing that fails is a failed add, or a Failed follow that keeps every member.
    fn listing_failed(
        &self,
        store: &Store,
        operation: &str,
        follow: &LibraryFollowSummary,
        create: bool,
        failure: InspectionError,
    ) -> Result<(), InspectionError> {
        if create {
            return Err(failure);
        }
        store.mutate_index(|index| {
            if let Some(record) = index.follows.iter_mut().find(|f| f.follow_id == follow.follow_id) {
                record.state = LibraryItemState::Failed;
            }
            Ok(())
        })?;
        operations::follow_row(
            store,
            operation,
            follow,
            LibraryReportOutcome::Failed,
            Some(failure.message),
        )
    }

    /// One provider listing, cancelled between CLI calls. `None`: cancelled.
    async fn list_cancellable(
        &self,
        store: &Store,
        operation: &str,
        provider_id: &str,
        query: IssueQuery<'_>,
        max: u32,
    ) -> Result<Option<IssueListing>, InspectionError> {
        let cancel = AtomicBool::new(false);
        let listing = {
            let listing = self.sources.list_issues(provider_id, &query, max, &cancel);
            tokio::pin!(listing);
            loop {
                tokio::select! {
                    result = &mut listing => break result,
                    _ = tokio::time::sleep(Duration::from_millis(200)) => {
                        if operations::cancelled(store, operation)? {
                            cancel.store(true, Ordering::SeqCst);
                        }
                    }
                }
            }
        };
        if cancel.load(Ordering::SeqCst) || operations::cancelled(store, operation)? {
            return Ok(None);
        }
        listing.map(Some)
    }

    /// D7. Lists the query (or probes it, or batches the members' keys), fetches
    /// only issues whose `updated` moved, then commits references and the record
    /// in one index change. A listing that failed, is truncated, is cancelled or
    /// came back empty never drops a member. `create` upserts the record after
    /// the first successful listing and makes a listing failure fail the add.
    pub(super) async fn refresh_jira_follow(
        &self,
        store: &Arc<Store>,
        operation: &str,
        follow: LibraryFollowSummary,
        create: bool,
    ) -> Result<(), InspectionError> {
        let LibraryFollowSource::JiraQuery { jql, mode } = follow.source.clone() else {
            return Err(error("library_corrupt", "Followed query has no Jira source"));
        };
        let live = mode == LibraryFollowMode::Live;
        match self.jira_site(&follow.provider_id) {
            Ok(site) if site.provider_instance == follow.provider_instance => {}
            Ok(_) => {
                let failure = error(
                    "source_authority_mismatch",
                    "Followed query belongs to a different configured site",
                );
                return self.listing_failed(store, operation, &follow, create, failure);
            }
            Err(failure) => return self.listing_failed(store, operation, &follow, create, failure),
        }
        let limit = self.configuration.limits.library_space_pages.max(1);

        // 2. Listing: everything for live (and for a follow with no members or
        // an unsettled last run), the changes since the watermark otherwise.
        let before = self.snapshot(store, &follow)?;
        // A member saved before relations were captured needs its fetch, so the whole query is listed.
        let legacy = before.items.values().any(|entry| {
            is_issue_of(entry, &follow)
                && refs::has_follow(&entry.summary, &follow.follow_id)
                && !entry.relations_captured
        });
        let since = (!live && !before.unsettled && !legacy)
            .then(|| {
                watermark(before.items.values().filter(|entry| {
                    is_issue_of(entry, &follow)
                        && refs::has_follow(&entry.summary, &follow.follow_id)
                        && !refs::related_of(&entry.summary, &follow.follow_id)
                }))
            })
            .flatten();
        let query = IssueQuery::Jql { jql: &jql, updated_since: since.as_deref() };
        let listing = match self.list_cancellable(store, operation, &follow.provider_id, query, limit).await {
            Ok(Some(listing)) => listing,
            Ok(None) => return Ok(()),
            Err(failure) => return self.listing_failed(store, operation, &follow, create, failure),
        };
        if create {
            let created = follow.clone();
            store.mutate_index(|index| {
                match index.follows.iter_mut().find(|f| f.follow_id == created.follow_id) {
                    Some(record) => {
                        record.excluded_ids.clear();
                        record.source = created.source.clone();
                        record.reference_depth = created.reference_depth;
                        record.include_attachments = created.include_attachments;
                    }
                    None => index.follows.push(created),
                }
                Ok(())
            })?;
        }
        let snapshot = self.snapshot(store, &follow)?;
        let listed = listing
            .rows
            .iter()
            .filter(|row| !snapshot.excluded.contains(&row.key))
            .map(|row| (row.key.clone(), row.clone()))
            .collect::<BTreeMap<_, _>>();

        // 3. Accumulate keeps every member: check the ones the listing did not
        // return by key, and confirm the ones that are gone.
        let mut checked = BTreeMap::new();
        let mut absent = Vec::new();
        let mut unverified = false;
        if !live {
            let keys = snapshot
                .members
                .iter()
                .filter(|key| !snapshot.excluded.contains(*key) && !listed.contains_key(*key))
                .cloned()
                .collect::<Vec<_>>();
            if !keys.is_empty() {
                let max = keys.len() as u32;
                let batch = match self
                    .list_cancellable(store, operation, &follow.provider_id, IssueQuery::Keys(&keys), max)
                    .await
                {
                    Ok(Some(batch)) => batch,
                    Ok(None) => return Ok(()),
                    Err(failure) => return self.listing_failed(store, operation, &follow, create, failure),
                };
                for row in batch.rows {
                    checked.insert(row.key.clone(), row);
                }
                if batch.complete {
                    absent = keys.into_iter().filter(|key| !checked.contains_key(key)).collect();
                } else {
                    unverified = true;
                }
            }
        }

        // Attachments stay with a follow that asked for them, for as long as the
        // provider can download them (a token stored in Cockpit). Otherwise the
        // text still refreshes and the report says why the files did not.
        let mut download = false;
        if follow.include_attachments {
            match self.sources.attachment_downloads(&follow.provider_id, "issue").await {
                Ok(()) => download = true,
                Err(failure) => {
                    let why = match failure.code.as_str() {
                        "source_credential_required" => "attachments need a token stored in Cockpit for this Jira site",
                        "credential_vault_unavailable" | "credential_vault_timeout" => {
                            "the credential vault is unavailable"
                        }
                        _ => "the provider cannot download attachments",
                    };
                    operations::follow_row(
                        store,
                        operation,
                        &follow,
                        LibraryReportOutcome::Partial,
                        Some(format!("Attachments were not downloaded: {why}")),
                    )?;
                }
            }
        }

        // 4. Pending work.
        let mut unchanged = 0;
        let mut pending = Vec::new();
        for row in listed.values().chain(checked.values()) {
            let old = snapshot.items.get(&issue_item_id(&follow, &row.key)).cloned();
            let reason = match &old {
                None => None,
                Some(old) => match change_reason(old, row, snapshot.depth) {
                    Some(reason) => Some(reason.to_owned()),
                    None if download
                        && old.summary.attachments.iter().any(|a| {
                            matches!(a.state, LibraryAttachmentState::NotDownloaded | LibraryAttachmentState::Failed)
                        }) =>
                    {
                        Some("attachments requested".to_owned())
                    }
                    None => {
                        unchanged += 1;
                        continue;
                    }
                },
            };
            pending.push(Pending { key: row.key.clone(), row: Some(row.clone()), old, reason });
        }
        // Oldest first: the watermark only ever moves past issues that were saved.
        pending.sort_by(|a, b| {
            let updated = |p: &Pending| p.row.as_ref().map(|row| row.updated.clone());
            updated(a).cmp(&updated(b))
        });
        for key in absent {
            let old = snapshot.items.get(&issue_item_id(&follow, &key)).cloned();
            if old.as_ref().is_some_and(|old| old.summary.state == LibraryItemState::RemovedAtSource) {
                continue;
            }
            pending.push(Pending { key, row: None, old, reason: Some("confirming".into()) });
        }
        operations::add_total(store, operation, (pending.len() + unchanged) as u32)?;

        // 5. Fetch in batches of 8 under each item's lease.
        let base = self
            .configuration
            .providers
            .iter()
            .find(|provider| provider.id == follow.provider_id)
            .map(|provider| provider.base_url.trim_end_matches('/').to_owned())
            .ok_or_else(|| error("source_authority_mismatch", "selected Jira provider is not configured"))?;
        // Seeds this run tried to refresh: a failed or conflicted one has no
        // current reference set, whatever an earlier save stored.
        let attempted = pending
            .iter()
            .filter(|p| p.row.is_some())
            .map(|p| p.key.clone())
            .collect::<BTreeSet<String>>();
        let work = pending
            .into_iter()
            .map(|pending| {
                Ok(FetchWork {
                    item_id: issue_item_id(&follow, &pending.key),
                    canonical_id: pending.key.clone(),
                    label: pending.key.clone(),
                    request: self.request(
                        &format!("{base}/browse/{}", pending.key),
                        Some(&follow.provider_id),
                    )?,
                    container: None,
                    tag: pending,
                })
            })
            .collect::<Result<Vec<_>, InspectionError>>()?;
        let mut cancelled = false;
        let mut failures = 0u32;
        for batch in work.chunks(FOLLOW_FETCH_CONCURRENCY) {
            let Some(fetched) = self.fetch_batch(store, operation, &follow, batch).await? else {
                cancelled = true;
                break;
            };
            let FetchedBatch { results, leases } = fetched;
            for (job, result) in results {
                if operations::cancelled(store, operation)? {
                    cancelled = true;
                    break;
                }
                if self.save_issue_result(store, operation, &follow, job, result, download).await? {
                    failures += 1;
                }
            }
            drop(leases);
            if cancelled {
                break;
            }
        }
        operations::unchanged(store, operation, unchanged as u32)?;
        cancelled = cancelled || operations::cancelled(store, operation)?;

        // 5b. Reference depth: traverse from the seeds and save what they reach.
        // Live follows use the listed seeds; accumulate uses every seed member,
        // whose stored references stand in for the ones the listing did not return.
        let mut pass = RelatedPass::none();
        if !cancelled && snapshot.depth > 0 {
            let keys = if live {
                listed.keys().cloned().collect::<BTreeSet<_>>()
            } else {
                listed
                    .keys()
                    .chain(checked.keys())
                    .chain(snapshot.members.iter().filter(|key| !snapshot.excluded.contains(*key)))
                    .cloned()
                    .collect()
            };
            let stored = {
                let _lock = store.shared()?;
                store
                    .index()?
                    .items
                    .into_iter()
                    .filter(|entry| is_issue_of(entry, &follow))
                    .filter_map(|entry| {
                        Some((entry.summary.canonical_id.clone()?, (entry.references, entry.summary.state)))
                    })
                    .collect::<BTreeMap<_, _>>()
            };
            let mut seeds = Vec::new();
            let mut unknown = 0u32;
            for key in keys {
                let (references, state) = match stored.get(&key).cloned() {
                    Some((references, state)) => (references, Some(state)),
                    None => (None, None),
                };
                let not_current = attempted.contains(&key)
                    && matches!(
                        state,
                        Some(
                            LibraryItemState::Failed
                                | LibraryItemState::Conflict
                                | LibraryItemState::RemovedAtSource
                        )
                    );
                // Stale references still grow the set, but never authorize a drop.
                if not_current {
                    unknown += 1;
                }
                match references {
                    Some(references) => seeds.push(ReferenceSeed {
                        source: SourceRef {
                            provider_id: follow.provider_id.clone(),
                            provider_instance: follow.provider_instance.clone(),
                            resource_type: "issue".into(),
                            canonical_id: key.clone(),
                        },
                        label: key,
                        references,
                    }),
                    None => unknown += 1,
                }
            }
            // A related item is excluded by its Library id; a former seed of this
            // follow's own site by its key (the scoped compat form).
            let skip = |related: &RelatedAsset| {
                let source = &related.asset.source;
                snapshot.excluded.contains(&item_id(source))
                    || (source.provider_id == follow.provider_id
                        && source.provider_instance == follow.provider_instance
                        && source.resource_type == "issue"
                        && snapshot.excluded.contains(&source.canonical_id))
            };
            pass = self
                .run_related(
                    store,
                    operation,
                    seeds,
                    unknown,
                    snapshot.depth,
                    TraversalBudget::Query,
                    &LibraryInclusionHolder::Follow { follow_id: follow.follow_id.clone() },
                    &LibraryItemRef::Follow { follow_id: follow.follow_id.clone() },
                    &skip,
                )
                .await?;
            cancelled = cancelled || pass.cancelled;
        }

        // 6. Commit. Only a complete, uncancelled, non-empty live listing drops members.
        let listing_complete = listing.complete;
        let mut have = listed.len() as u64;
        have += checked.len() as u64;
        let partial = if cancelled {
            (!live).then(|| LibraryPartial {
                unit: "issues".into(),
                have,
                total: None,
                reason: "cancelled".into(),
            })
        } else if !listing_complete {
            Some(LibraryPartial { unit: "issues".into(), have, total: None, reason: "issue limit".into() })
        } else if unverified {
            Some(LibraryPartial {
                unit: "issues".into(),
                have,
                total: None,
                reason: "members could not all be checked".into(),
            })
        } else if !live && failures > 0 {
            // Fetches failed after newer issues may have been saved, so the
            // watermark can no longer be trusted: the next run lists everything.
            Some(LibraryPartial {
                unit: "issues".into(),
                have,
                total: None,
                reason: format!("{failures} issues failed to fetch"),
            })
        } else if !pass.complete {
            Some(LibraryPartial {
                unit: "related items".into(),
                have: pass.reached.len() as u64,
                total: None,
                reason: "related items incomplete".into(),
            })
        } else {
            None
        };
        // A related pass that could not finish never lets an unlisted seed or
        // an unreached related item go: either might still be reachable.
        let drop_allowed = live && listing_complete && !cancelled && pass.complete;
        let rows = listed
            .iter()
            .chain(checked.iter())
            .map(|(key, row)| (key.clone(), row.clone()))
            .collect::<BTreeMap<_, _>>();
        let now = timestamp().parse::<u128>().unwrap_or(0);
        let (dropped, kept) = store.mutate_index(|index| {
            let record = index
                .follows
                .iter()
                .find(|f| f.follow_id == follow.follow_id)
                .ok_or_else(|| error("library_item_not_found", "Followed query was removed"))?;
            let excluded = record.excluded_ids.iter().cloned().collect::<BTreeSet<_>>();
            let mut members = 0u32;
            for entry in &mut index.items {
                if !is_issue_of(entry, &follow) {
                    continue;
                }
                let Some(key) = entry.summary.canonical_id.clone() else { continue };
                if excluded.contains(&key) {
                    continue;
                }
                if listed.contains_key(&key) {
                    refs::insert_ref(
                        &mut entry.summary,
                        LibraryItemRef::Follow { follow_id: follow.follow_id.clone() },
                    );
                    // A listed item is a seed, whatever else reaches it.
                    refs::strip_inclusion(
                        &mut entry.summary,
                        &LibraryInclusionHolder::Follow { follow_id: follow.follow_id.clone() },
                    );
                }
                if let Some(row) = rows.get(&key) {
                    let fetched_updated = entry
                        .summary
                        .issue
                        .as_ref()
                        .and_then(|meta| meta.fetched_updated.clone());
                    entry.summary.issue = Some(LibraryIssueMeta {
                        updated: row.updated.clone(),
                        fetched_updated,
                        status: row.status.clone(),
                        issue_type: row.issue_type.clone(),
                        assignee: row.assignee.clone(),
                    });
                }
                if refs::has_follow(&entry.summary, &follow.follow_id)
                    && !refs::related_of(&entry.summary, &follow.follow_id)
                {
                    members += 1;
                }
            }
            let empty_with_members = listed.is_empty() && members > 0;
            let mut dropped = Vec::new();
            if drop_allowed && !empty_with_members {
                for entry in &mut index.items {
                    if !refs::has_follow(&entry.summary, &follow.follow_id) {
                        continue;
                    }
                    if refs::related_of(&entry.summary, &follow.follow_id) {
                        if pass.reached.contains(&entry.summary.item_id) {
                            continue;
                        }
                    } else {
                        if !is_issue_of(entry, &follow) {
                            continue;
                        }
                        let Some(key) = entry.summary.canonical_id.clone() else { continue };
                        if listed.contains_key(&key) || excluded.contains(&key) {
                            continue;
                        }
                    }
                    let before = entry.summary.clone();
                    refs::remove_ref(
                        &mut entry.summary,
                        &LibraryItemRef::Follow { follow_id: follow.follow_id.clone() },
                    );
                    if entry.summary.refs.is_empty() {
                        entry.summary.purge_after = Some((now + TOMBSTONE_GRACE_MS).to_string());
                    }
                    dropped.push(before);
                }
            }
            let kept = (drop_allowed && empty_with_members).then_some(members);
            recount(index, &follow.follow_id);
            let partial = partial.clone().or_else(|| {
                kept.map(|members| LibraryPartial {
                    unit: "issues".into(),
                    have: 0,
                    total: Some(u64::from(members)),
                    reason: "empty listing".into(),
                })
            });
            if !cancelled || partial.is_some() {
                let record = index
                    .follows
                    .iter_mut()
                    .find(|f| f.follow_id == follow.follow_id)
                    .expect("follow checked above");
                record.state = if partial.is_some() {
                    LibraryItemState::Partial
                } else {
                    LibraryItemState::Fresh
                };
                record.partial = partial;
                record.last_refreshed_at = Some(timestamp());
            }
            Ok((dropped, kept))
        })?;
        for summary in &dropped {
            operations::row(
                store,
                operation,
                Some(summary),
                LibraryReportOutcome::Dropped,
                Some("no longer matches the query".into()),
            )?;
        }
        if cancelled {
            return Ok(());
        }
        if let Some(note) = &pass.note {
            operations::follow_row(
                store,
                operation,
                &follow,
                LibraryReportOutcome::Partial,
                Some(note.clone()),
            )?;
        }
        if let Some(members) = kept {
            operations::follow_row(
                store,
                operation,
                &follow,
                LibraryReportOutcome::Partial,
                Some(format!("Query returned no issues; kept {members} issues")),
            )?;
        } else if !listing_complete {
            operations::follow_row(
                store,
                operation,
                &follow,
                LibraryReportOutcome::Partial,
                Some(format!("{} issues (issue limit)", listing.rows.len())),
            )?;
        } else if unverified {
            operations::follow_row(
                store,
                operation,
                &follow,
                LibraryReportOutcome::Partial,
                Some("Members could not all be checked; none were removed".into()),
            )?;
        }
        Ok(())
    }

    /// Saves one fetched issue. Returns whether it counts as a failure that
    /// leaves the follow's view of the query incomplete.
    async fn save_issue_result(
        &self,
        store: &Arc<Store>,
        operation: &str,
        follow: &LibraryFollowSummary,
        job: &FetchWork<Pending>,
        result: Result<crate::sources::SourceAsset, InspectionError>,
        download: bool,
    ) -> Result<bool, InspectionError> {
        let pending = &job.tag;
        if operations::cancelled(store, operation)? {
            return Ok(false);
        }
        let confirming = pending.row.is_none();
        let saved = match result {
            Ok(asset) if item_id(&asset.source) == job.item_id => {
                self.save_asset_with(
                    store,
                    operation,
                    asset,
                    pending.old.clone(),
                    SaveOptions {
                        reference: Some(LibraryItemRef::Follow { follow_id: follow.follow_id.clone() }),
                        reason: pending.reason.clone(),
                        issue_row: pending.row.as_ref(),
                        download_all: download,
                        ..SaveOptions::default()
                    },
                )
                .await
            }
            Ok(_) => Err(error("source_identity_mismatch", "Provider refresh returned a different issue")),
            Err(failure) => Err(failure),
        };
        match (saved, &pending.old) {
            (Ok(()), _) => Ok(false),
            (Err(failure), Some(old)) => {
                let removed = confirming && failure.code == "source_not_found";
                self.fetch_failed(store, operation, old.clone(), failure, confirming)?;
                Ok(!removed)
            }
            (Err(failure), None) => {
                operations::follow_row(
                    store,
                    operation,
                    follow,
                    LibraryReportOutcome::Failed,
                    Some(format!("{}: {}", pending.key, failure.message)),
                )?;
                Ok(true)
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::super::tests::{self as base, finished};
    use super::*;
    use crate::sources::{AttachmentRef, DownloadedAttachment, SourceAsset, SourceContainer, SourceFetchRequest, SourceProvider, SourceService};
    use async_trait::async_trait;
    use cockpit_protocol::{projects::ProjectProvider, sources::SourceCapability};
    use std::sync::Mutex;

    const BASE: &str = "https://jira.example.test";
    const OPS: &str = "project = OPS";

    #[derive(Clone)]
    struct Issue {
        minute: u32,
        body: String,
        attachments: Vec<crate::sources::SourceAttachment>,
    }
    #[derive(Default)]
    struct Site {
        issues: BTreeMap<String, Issue>,
        /// What each JQL matches, whatever else exists.
        queries: BTreeMap<String, Vec<String>>,
        list_error: bool,
        truncated: bool,
        /// Deleted at source: unlisted by key, and its fetch is not found.
        gone: BTreeSet<String>,
        /// Listed normally, but its fetch fails.
        broken: BTreeSet<String>,
        log: Vec<String>,
        fetched: Vec<String>,
        /// The error code `attachment_downloads` answers; `None` answers Ok.
        download_gate: Option<&'static str>,
        /// (issue key, attachment id, budget bytes, budget max_files, sibling count) per download.
        downloads: Vec<(String, String, u64, usize, usize)>,
    }
    struct FakeIssues(Mutex<Site>);
    impl FakeIssues {
        fn site(&self) -> std::sync::MutexGuard<'_, Site> {
            self.0.lock().unwrap_or_else(|e| e.into_inner())
        }
        fn set(&self, jql: &str, keys: &[&str]) {
            self.site().queries.insert(jql.into(), keys.iter().map(|k| (*k).to_owned()).collect());
        }
        fn touch(&self, key: &str, minute: u32) {
            let mut site = self.site();
            let issue = site.issues.get_mut(key).unwrap();
            issue.minute = minute;
            issue.body = format!("{key} body at {minute}");
        }
        /// The issue's description mentions `text` (keys become references).
        fn mention(&self, key: &str, minute: u32, text: &str) {
            let mut site = self.site();
            let issue = site.issues.get_mut(key).unwrap();
            issue.minute = minute;
            issue.body = format!("## Description\n{text}\n");
        }
        fn attach(&self, key: &str, minute: u32, id: &str, name: &str, size: u64) {
            let mut site = self.site();
            let issue = site.issues.get_mut(key).unwrap();
            issue.minute = minute;
            issue.attachments.push(crate::sources::SourceAttachment {
                id: id.into(), title: name.into(), media_type: Some("text/plain".into()), size: Some(size),
                source_url: None, source_revision: None, path: None, not_downloaded: Some("not_requested".into()),
            });
        }
        fn take_fetched(&self) -> BTreeSet<String> {
            std::mem::take(&mut self.site().fetched).into_iter().collect()
        }
        fn take_log(&self) -> Vec<String> {
            std::mem::take(&mut self.site().log)
        }
    }
    fn plain(minute: u32) -> String {
        format!("2026-03-01 10:{minute:02}:00")
    }
    fn iso(minute: u32) -> String {
        format!("2026-03-01T10:{minute:02}:00.000+0000")
    }
    fn row(key: &str, issue: &Issue) -> IssueRow {
        IssueRow {
            key: key.into(),
            updated: plain(issue.minute),
            status: "Open".into(),
            issue_type: "Task".into(),
            assignee: None,
        }
    }
    #[async_trait]
    impl SourceProvider for FakeIssues {
        fn provider_id(&self) -> &str {
            "jira"
        }
        fn capabilities(&self) -> Vec<SourceCapability> {
            vec![]
        }
        async fn list_issues(
            &self,
            query: &IssueQuery<'_>,
            _max: u32,
            _cancel: &AtomicBool,
        ) -> Result<IssueListing, InspectionError> {
            let mut site = self.site();
            match query {
                IssueQuery::Jql { jql, updated_since } => {
                    site.log.push(format!("jql {jql} since {}", updated_since.unwrap_or("-")));
                    if site.list_error {
                        return Err(error("source_provider_failed", "jira is down"));
                    }
                    let since = updated_since.and_then(instant_seconds);
                    let rows = site
                        .queries
                        .get(*jql)
                        .into_iter()
                        .flatten()
                        .filter(|key| !site.gone.contains(*key))
                        .filter_map(|key| Some((key, site.issues.get(key)?)))
                        .filter(|(_, issue)| since.is_none_or(|since| instant_seconds(&iso(issue.minute)).unwrap() >= since))
                        .map(|(key, issue)| row(key, issue))
                        .collect();
                    Ok(IssueListing { rows, complete: !site.truncated })
                }
                IssueQuery::Keys(keys) => {
                    site.log.push(format!("keys {}", keys.join(",")));
                    let rows = keys
                        .iter()
                        .filter(|key| !site.gone.contains(*key))
                        .filter_map(|key| Some(row(key, site.issues.get(key)?)))
                        .collect();
                    Ok(IssueListing { rows, complete: true })
                }
            }
        }
        async fn fetch(&self, request: &SourceFetchRequest) -> Result<Vec<SourceAsset>, InspectionError> {
            let key = request.artifact_url.rsplit('/').next().unwrap().to_owned();
            let mut site = self.site();
            site.fetched.push(key.clone());
            if site.gone.contains(&key) {
                return Err(error("source_not_found", "issue does not exist"));
            }
            if site.broken.contains(&key) {
                return Err(error("source_provider_failed", "jira is down"));
            }
            let issue = site.issues.get(&key).cloned().ok_or_else(|| error("source_not_found", "gone"))?;
            let project = key.rsplit_once('-').unwrap().0.to_owned();
            Ok(vec![SourceAsset {
                source: SourceRef {
                    provider_id: "jira".into(),
                    provider_instance: request.authority.provider_instance.clone(),
                    resource_type: "issue".into(),
                    canonical_id: key.clone(),
                },
                title: format!("Issue {key}"),
                source_url: Some(format!("{BASE}/browse/{key}")),
                original_url: None,
                source_revision: Some(iso(issue.minute)),
                complete: true,
                diagnostics: vec![],
                body: issue.body,
                container: Some(SourceContainer { id: project.clone(), label: project }),
                fields: vec![],
                attachments: issue.attachments,
            }])
        }
        async fn attachment_downloads(&self, resource_type: &str) -> Result<(), InspectionError> {
            assert_eq!(resource_type, "issue");
            match self.site().download_gate {
                None => Ok(()),
                Some(code) => Err(error(code, "attachment downloads are not available")),
            }
        }
        async fn download_attachment(
            &self,
            canonical_id: &str,
            attachment: &AttachmentRef,
            siblings: &[AttachmentRef],
            dest: &cap_std::fs::Dir,
            _dest_path: &std::path::Path,
            budget: crate::process::StagingBudget,
        ) -> Result<DownloadedAttachment, InspectionError> {
            let size = attachment.bytes.unwrap();
            self.site().downloads.push((canonical_id.into(), attachment.id.clone(), budget.bytes, budget.max_files, siblings.len()));
            dest.write("download", vec![b'x'; size as usize]).unwrap();
            Ok(DownloadedAttachment { attachment_id: attachment.id.clone(), file_name: "download".into() })
        }
    }

    struct Fixture {
        base: base::Fixture,
        provider: Arc<FakeIssues>,
    }
    fn fixture() -> Fixture {
        let mut base = base::fixture();
        let mut configuration = base.service.configuration.clone();
        configuration.providers = vec![ProjectProvider {
            id: "jira".into(),
            base_url: BASE.into(),
            executable: "/usr/local/bin/jira".into(),
            login: Some("read-only".into()),
        }];
        let mut site = Site::default();
        for (key, minute) in [("OPS-1", 1), ("OPS-2", 2), ("OPS-3", 3), ("OPS-4", 4)] {
            site.issues.insert(key.into(), Issue { minute, body: format!("{key} body"), attachments: vec![] });
        }
        let provider = Arc::new(FakeIssues(Mutex::new(site)));
        let sources = Arc::new(SourceService::new(&configuration, vec![provider.clone()]).unwrap());
        base.service = LibraryService::new(configuration, sources);
        Fixture { base, provider }
    }
    async fn follow(
        service: &LibraryService,
        jql: &str,
        mode: LibraryFollowMode,
    ) -> (LibraryOperation, String) {
        follow_at(service, jql, mode, 0).await
    }
    async fn follow_at(
        service: &LibraryService,
        jql: &str,
        mode: LibraryFollowMode,
        depth: u32,
    ) -> (LibraryOperation, String) {
        let request = LibraryAddRequest {
            input: jql.into(),
            provider_id: Some("jira".into()),
            reference_depth: depth,
            follow: true,
            follow_mode: Some(mode),
            download_attachments: false,
            refresh_existing: false,
            label: None,
            target: None,
        };
        let operation = finished(service, service.start_add(request).await.unwrap()).await;
        assert_eq!(operation.phases[0].state, LibraryPhaseState::Done, "{operation:?}");
        let listing = service.listing(None).await.unwrap();
        let id = listing
            .follows
            .into_iter()
            .find(|follow| refs::follow_title(follow) == jql)
            .unwrap()
            .follow_id;
        (operation, id)
    }
    async fn refresh(service: &LibraryService, follow_id: &str) -> LibraryRefreshReport {
        let request = LibraryRefreshRequest::Follow { follow_id: follow_id.into() };
        let operation = finished(service, service.start_refresh(request).await.unwrap()).await;
        assert!(operation.phases.iter().all(|p| p.error.is_none()), "{operation:?}");
        operation.report.unwrap()
    }
    async fn issues(service: &LibraryService) -> BTreeMap<String, LibraryItemSummary> {
        service
            .listing(None)
            .await
            .unwrap()
            .items
            .into_iter()
            .map(|item| (item.canonical_id.clone().unwrap(), item))
            .collect()
    }
    async fn record(service: &LibraryService, follow_id: &str) -> LibraryFollowSummary {
        service
            .listing(None)
            .await
            .unwrap()
            .follows
            .into_iter()
            .find(|follow| follow.follow_id == follow_id)
            .unwrap()
    }
    fn held(item: &LibraryItemSummary, follow_id: &str) -> bool {
        refs::has_follow(item, follow_id)
    }
    fn keys(set: &[&str]) -> BTreeSet<String> {
        set.iter().map(|key| (*key).to_owned()).collect()
    }

    #[tokio::test]
    async fn live_follow_mirrors_its_query_and_a_bad_listing_never_drops() {
        let f = fixture();
        let service = &f.base.service;
        f.provider.set(OPS, &["OPS-1", "OPS-2", "OPS-3"]);
        let (added, id) = follow(service, OPS, LibraryFollowMode::Live).await;
        assert_eq!(added.report.as_ref().unwrap().new, 3);
        assert_eq!(f.provider.take_fetched(), keys(&["OPS-1", "OPS-2", "OPS-3"]));
        let items = issues(service).await;
        // D9: the existing Jira layout, one directory per key.
        assert_eq!(items["OPS-2"].item_path, "jira/jira.example.test/OPS/OPS-2");
        assert_eq!(items["OPS-2"].document_path.as_deref(), Some("jira/jira.example.test/OPS/OPS-2/Issue OPS-2.md"));
        let meta = items["OPS-2"].issue.clone().unwrap();
        assert_eq!((meta.updated.as_str(), meta.fetched_updated.as_deref()), ("2026-03-01 10:02:00", Some("2026-03-01 10:02:00")));
        assert!(items.values().all(|item| item.refs == [LibraryItemRef::Follow { follow_id: id.clone() }]));
        let follow_record = record(service, &id).await;
        assert_eq!((follow_record.item_count, follow_record.state), (3, LibraryItemState::Fresh));

        // KEY-2 leaves the query: it loses the reference and is tombstoned, not deleted.
        f.provider.set(OPS, &["OPS-1", "OPS-3"]);
        let report = refresh(service, &id).await;
        assert_eq!((report.dropped, report.unchanged, report.new), (1, 2, 0));
        assert!(f.provider.take_fetched().is_empty());
        let items = issues(service).await;
        assert!(items["OPS-2"].refs.is_empty());
        assert!(items["OPS-2"].purge_after.is_some());
        assert_eq!(record(service, &id).await.item_count, 2);

        // An errored, a truncated and an empty listing all keep the members.
        f.provider.set(OPS, &["OPS-1"]);
        f.provider.site().list_error = true;
        let report = refresh(service, &id).await;
        assert_eq!((report.failed, report.dropped), (1, 0));
        assert_eq!(record(service, &id).await.state, LibraryItemState::Failed);
        f.provider.site().list_error = false;
        f.provider.site().truncated = true;
        let report = refresh(service, &id).await;
        assert_eq!((report.partial, report.dropped), (1, 0));
        assert_eq!(record(service, &id).await.state, LibraryItemState::Partial);
        f.provider.site().truncated = false;
        f.provider.set(OPS, &[]);
        let report = refresh(service, &id).await;
        assert_eq!((report.partial, report.dropped), (1, 0));
        assert!(report.rows.iter().any(|r| r.reason.as_deref() == Some("Query returned no issues; kept 2 issues")));
        let items = issues(service).await;
        assert!(held(&items["OPS-1"], &id) && held(&items["OPS-3"], &id));

        // A listing that matches again revives the tombstoned issue without a fetch.
        f.provider.set(OPS, &["OPS-1", "OPS-2", "OPS-3"]);
        let report = refresh(service, &id).await;
        assert_eq!((report.new, report.dropped, report.unchanged), (0, 0, 3));
        let items = issues(service).await;
        assert!(held(&items["OPS-2"], &id) && items["OPS-2"].purge_after.is_none());
        assert_eq!(record(service, &id).await.state, LibraryItemState::Fresh);
    }

    #[tokio::test]
    async fn changed_updated_fetches_exactly_that_issue() {
        let f = fixture();
        let service = &f.base.service;
        f.provider.set(OPS, &["OPS-1", "OPS-2", "OPS-3"]);
        let (_, id) = follow(service, OPS, LibraryFollowMode::Live).await;
        f.provider.take_fetched();
        let report = refresh(service, &id).await;
        assert_eq!((report.unchanged, report.updated), (3, 0));
        assert!(f.provider.take_fetched().is_empty());
        f.provider.touch("OPS-2", 20);
        let report = refresh(service, &id).await;
        assert_eq!((report.unchanged, report.updated), (2, 1));
        assert_eq!(f.provider.take_fetched(), keys(&["OPS-2"]));
        let meta = issues(service).await["OPS-2"].issue.clone().unwrap();
        assert_eq!(meta.fetched_updated.as_deref(), Some(plain(20).as_str()));
    }

    #[tokio::test]
    async fn a_copy_saved_before_relations_were_captured_is_fetched_once() {
        let f = fixture();
        let service = &f.base.service;
        f.provider.set(OPS, &["OPS-1", "OPS-2", "OPS-3"]);
        let (_, id) = follow(service, OPS, LibraryFollowMode::Accumulate).await;
        f.provider.take_fetched();
        // OPS-2 predates parent capture: the copy is unchanged at source, yet its next refresh fetches it.
        service
            .open()
            .unwrap()
            .mutate_index(|index| {
                for entry in &mut index.items {
                    entry.relations_captured = entry.summary.canonical_id.as_deref() != Some("OPS-2");
                }
                Ok(())
            })
            .unwrap();
        refresh(service, &id).await;
        assert_eq!(f.provider.take_fetched(), keys(&["OPS-2"]));
        // Once fetched it is settled: nothing is fetched again.
        refresh(service, &id).await;
        assert!(f.provider.take_fetched().is_empty());
    }

    #[tokio::test]
    async fn accumulate_probes_from_the_watermark_and_never_drops() {
        let f = fixture();
        let service = &f.base.service;
        f.provider.set(OPS, &["OPS-1", "OPS-2", "OPS-3"]);
        let (_, id) = follow(service, OPS, LibraryFollowMode::Accumulate).await;
        assert_eq!(f.provider.take_fetched().len(), 3);
        f.provider.take_log();

        // Probe from the newest member; the older members are checked by key.
        let report = refresh(service, &id).await;
        assert_eq!((report.unchanged, report.updated, report.dropped), (3, 0, 0));
        assert_eq!(f.provider.take_log(), [format!("jql {OPS} since {}", iso(3)), "keys OPS-1,OPS-2".to_owned()]);
        assert!(f.provider.take_fetched().is_empty());

        // OPS-2 is deleted at source and leaves the query; OPS-1 only leaves the query.
        f.provider.set(OPS, &["OPS-3"]);
        f.provider.site().gone.insert("OPS-2".into());
        let report = refresh(service, &id).await;
        assert_eq!((report.removed_at_source, report.dropped), (1, 0));
        assert_eq!(f.provider.take_fetched(), keys(&["OPS-2"]));
        let items = issues(service).await;
        assert_eq!(items["OPS-2"].state, LibraryItemState::RemovedAtSource);
        assert!(items.values().all(|item| held(item, &id)));

        // A changed issue outside the query is still refreshed; the confirmed one is not asked again.
        f.provider.touch("OPS-1", 9);
        f.provider.take_log();
        let report = refresh(service, &id).await;
        assert_eq!((report.updated, report.dropped), (1, 0));
        assert_eq!(f.provider.take_fetched(), keys(&["OPS-1"]));
        assert_eq!(f.provider.take_log(), [format!("jql {OPS} since {}", iso(3)), "keys OPS-1,OPS-2".to_owned()]);
    }

    #[tokio::test]
    async fn overlapping_follows_share_one_item_and_unfollow_keeps_what_is_held_elsewhere() {
        let f = fixture();
        let service = &f.base.service;
        let mine = "assignee = currentUser()";
        f.provider.set(OPS, &["OPS-1", "OPS-2"]);
        f.provider.set(mine, &["OPS-2", "OPS-3"]);
        let (_, a) = follow(service, OPS, LibraryFollowMode::Live).await;
        let (second, b) = follow(service, mine, LibraryFollowMode::Live).await;
        assert_eq!((second.report.as_ref().unwrap().new, second.report.as_ref().unwrap().unchanged), (1, 1));
        let items = issues(service).await;
        assert_eq!(items.len(), 3);
        assert!(held(&items["OPS-2"], &a) && held(&items["OPS-2"], &b));
        assert_eq!((record(service, &a).await.item_count, record(service, &b).await.item_count), (2, 2));

        // Keep in Library: adding the saved issue again holds it manually.
        let keep = LibraryAddRequest {
            input: format!("{BASE}/browse/OPS-1"),
            provider_id: Some("jira".into()),
            reference_depth: 0,
            follow: false,
            follow_mode: None,
            download_attachments: false,
            refresh_existing: false,
            label: None,
            target: None,
        };
        finished(service, service.start_add(keep).await.unwrap()).await;
        assert!(issues(service).await["OPS-1"].refs.contains(&LibraryItemRef::Manual));

        // Stop following A: OPS-1 keeps its manual reference, OPS-2 stays with B.
        service.remove(LibraryRemoveRequest::StopFollowing { follow_id: a.clone() }).await.unwrap();
        let items = issues(service).await;
        assert_eq!(items["OPS-1"].refs, [LibraryItemRef::Manual]);
        assert_eq!(items["OPS-2"].refs, [LibraryItemRef::Follow { follow_id: b.clone() }]);
        assert!(items.values().all(|item| item.purge_after.is_none()));

        // Remove B and its items: only the issues it alone holds go.
        service.remove(LibraryRemoveRequest::Follow { follow_id: b.clone() }).await.unwrap();
        let items = issues(service).await;
        assert_eq!(items.keys().cloned().collect::<Vec<_>>(), ["OPS-1"]);

        // Stopping an exclusive follow keeps its issues as plain items.
        f.provider.set("status = Open", &["OPS-4"]);
        let (_, c) = follow(service, "status = Open", LibraryFollowMode::Live).await;
        service.remove(LibraryRemoveRequest::StopFollowing { follow_id: c }).await.unwrap();
        assert_eq!(issues(service).await["OPS-4"].refs, [LibraryItemRef::Manual]);
    }

    #[tokio::test]
    async fn purge_removes_expired_tombstones_and_keeps_edited_issues() {
        let f = fixture();
        let service = &f.base.service;
        f.provider.set(OPS, &["OPS-1", "OPS-2", "OPS-3"]);
        let (_, id) = follow(service, OPS, LibraryFollowMode::Live).await;
        f.provider.set(OPS, &["OPS-3"]);
        let report = refresh(service, &id).await;
        assert_eq!(report.dropped, 2);
        // Within the grace period the sweep leaves the tombstones alone.
        assert_eq!(issues(service).await.len(), 3);

        let items = issues(service).await;
        let edited = items["OPS-1"].clone();
        let plain_path = items["OPS-2"].item_path.clone();
        let root = std::path::Path::new(&service.configuration.library_root);
        std::fs::write(root.join(edited.document_path.as_deref().unwrap()), b"edited in Library").unwrap();
        service
            .open()
            .unwrap()
            .mutate_index(|index| {
                for entry in &mut index.items {
                    if entry.summary.refs.is_empty() {
                        entry.summary.purge_after = Some("0".into());
                    }
                }
                Ok(())
            })
            .unwrap();
        let report = refresh(service, &id).await;
        assert_eq!(report.dropped, 2);
        let reason = |key: &str| {
            report
                .rows
                .iter()
                .find(|row| row.item_id.as_deref() == Some(items[key].item_id.as_str()))
                .and_then(|row| row.reason.clone())
        };
        assert_eq!(reason("OPS-2").as_deref(), Some("purged after 14 days unreferenced"));
        assert_eq!(reason("OPS-1").as_deref(), Some("kept: edited in Library"));
        let items = issues(service).await;
        assert_eq!(items.keys().cloned().collect::<Vec<_>>(), ["OPS-1", "OPS-3"]);
        assert!(!root.join(plain_path).exists());
        assert!(root.join(edited.document_path.unwrap()).exists());
    }

    #[tokio::test]
    async fn followed_issue_lists_attachments_read_only_and_a_new_one_refreshes_the_item() {
        let f = fixture();
        let service = &f.base.service;
        f.provider.set(OPS, &["OPS-1"]);
        f.provider.attach("OPS-1", 1, "10100", "trace.log", 2048);
        let (_, id) = follow(service, OPS, LibraryFollowMode::Live).await;
        let item = issues(service).await["OPS-1"].clone();
        assert_eq!(item.attachments.len(), 1);
        let attachment = &item.attachments[0];
        assert_eq!(
            (attachment.original_name.as_str(), attachment.bytes, attachment.state, attachment.relative_path.as_deref()),
            ("trace.log", Some(2048), LibraryAttachmentState::NotDownloaded, None)
        );
        let root = std::path::Path::new(&service.configuration.library_root);
        let document = std::fs::read_to_string(root.join(item.document_path.as_deref().unwrap())).unwrap();
        assert!(document.contains("attachments:\n  - id: \"10100\"\n    title: \"trace.log\""), "{document}");
        assert!(document.contains("not_downloaded: \"not_requested\""));
        assert!(!root.join(&item.item_path).join("_files").exists());

        // A later upload changes `updated`, so the refresh fetches the issue and lists both files.
        f.provider.attach("OPS-1", 5, "10101", "shot.png", 90);
        let report = refresh(service, &id).await;
        assert_eq!(report.updated, 1, "{report:?}");
        let names: Vec<_> = issues(service).await["OPS-1"].attachments.iter().map(|a| a.original_name.clone()).collect();
        assert_eq!(names, ["trace.log", "shot.png"]);
    }

    async fn download_one(service: &LibraryService, item: &LibraryItemSummary, id: &str) -> Result<LibraryOperation, InspectionError> {
        let request = LibraryAttachmentRequest { item_id: item.item_id.clone(), attachment_ids: vec![id.into()], action: LibraryAttachmentAction::Download };
        Ok(finished(service, service.start_attachments(request).await?).await)
    }

    #[tokio::test]
    async fn a_provider_that_allows_downloads_stores_exactly_the_requested_issue_attachment() {
        let f = fixture();
        let service = &f.base.service;
        f.provider.set(OPS, &["OPS-1"]);
        f.provider.attach("OPS-1", 1, "10100", "trace.log", 6);
        f.provider.attach("OPS-1", 1, "10101", "trace-2.log", 6);
        follow(service, OPS, LibraryFollowMode::Live).await;
        let item = issues(service).await["OPS-1"].clone();

        let operation = download_one(service, &item, "10100").await.unwrap();
        assert_eq!(operation.phases[0].state, LibraryPhaseState::Done, "{operation:?}");
        let item = issues(service).await["OPS-1"].clone();
        assert_eq!(item.attachments[0].state, LibraryAttachmentState::Downloaded);
        assert_eq!(item.attachments[1].state, LibraryAttachmentState::NotDownloaded);
        let root = std::path::Path::new(&service.configuration.library_root);
        let stored = root.join(&item.item_path).join(item.attachments[0].relative_path.as_deref().unwrap());
        assert_eq!(std::fs::read(stored).unwrap(), b"xxxxxx");
        // The prediction is exactly that attachment: one file, its own size, no siblings.
        assert_eq!(f.provider.site().downloads, [("OPS-1".to_owned(), "10100".to_owned(), 6, 1, 0)]);
    }

    #[tokio::test]
    async fn a_provider_that_refuses_downloads_is_refused_before_any_operation_or_download() {
        for code in ["source_credential_required", "credential_vault_unavailable", "source_capability_unavailable"] {
            let f = fixture();
            let service = &f.base.service;
            f.provider.set(OPS, &["OPS-1"]);
            f.provider.attach("OPS-1", 1, "10100", "trace.log", 6);
            follow(service, OPS, LibraryFollowMode::Live).await;
            let item = issues(service).await["OPS-1"].clone();
            f.provider.site().download_gate = Some(code);

            assert_eq!(download_one(service, &item, "10100").await.unwrap_err().code, code);
            let add = LibraryAddRequest {
                input: format!("{BASE}/browse/OPS-1"),
                provider_id: Some("jira".into()),
                reference_depth: 0,
                follow: false,
                follow_mode: None,
                download_attachments: true,
                refresh_existing: false,
                label: None,
                target: None,
            };
            assert_eq!(service.start_add(add).await.unwrap_err().code, code);
            assert!(f.provider.site().downloads.is_empty());
            let item = issues(service).await["OPS-1"].clone();
            assert_eq!(item.attachments[0].state, LibraryAttachmentState::NotDownloaded);
        }
    }

    #[tokio::test]
    async fn add_with_downloads_downloads_the_primary_issue_only() {
        let f = fixture();
        let service = &f.base.service;
        f.provider.mention("OPS-1", 1, "See OPS-2 for details");
        f.provider.attach("OPS-1", 1, "10100", "trace.log", 6);
        f.provider.attach("OPS-2", 2, "20200", "other.log", 6);
        let add = LibraryAddRequest {
            input: format!("{BASE}/browse/OPS-1"),
            provider_id: Some("jira".into()),
            reference_depth: 1,
            follow: false,
            follow_mode: None,
            download_attachments: true,
            refresh_existing: false,
            label: None,
            target: None,
        };
        let operation = finished(service, service.start_add(add).await.unwrap()).await;
        assert_eq!(operation.phases[0].state, LibraryPhaseState::Done, "{operation:?}");
        let items = issues(service).await;
        assert_eq!(items.len(), 2, "{items:?}");
        assert_eq!(items["OPS-1"].attachments[0].state, LibraryAttachmentState::Downloaded);
        assert_eq!(items["OPS-2"].attachments[0].state, LibraryAttachmentState::NotDownloaded);
        assert_eq!(f.provider.site().downloads, [("OPS-1".to_owned(), "10100".to_owned(), 6, 1, 0)]);
    }

    fn follow_request(jql: &str, download: bool) -> LibraryAddRequest {
        LibraryAddRequest {
            input: jql.into(),
            provider_id: Some("jira".into()),
            reference_depth: 0,
            follow: true,
            follow_mode: Some(LibraryFollowMode::Live),
            download_attachments: download,
            refresh_existing: false,
            label: None,
            target: None,
        }
    }
    async fn follow_downloading(service: &LibraryService, jql: &str) -> String {
        let operation = finished(service, service.start_add(follow_request(jql, true)).await.unwrap()).await;
        assert!(matches!(operation.phases[0].state, LibraryPhaseState::Done | LibraryPhaseState::Partial), "{operation:?}");
        let listing = service.listing(None).await.unwrap();
        listing.follows.into_iter().find(|follow| refs::follow_title(follow) == jql).unwrap().follow_id
    }
    fn stored(service: &LibraryService, item: &LibraryItemSummary, index: usize) -> Option<Vec<u8>> {
        let root = std::path::Path::new(&service.configuration.library_root);
        let path = item.attachments[index].relative_path.as_deref()?;
        std::fs::read(root.join(&item.item_path).join(path)).ok()
    }
    fn downloaded(f: &Fixture) -> Vec<(String, String)> {
        std::mem::take(&mut f.provider.site().downloads).into_iter().map(|d| (d.0, d.1)).collect()
    }

    #[tokio::test]
    async fn a_follow_that_opts_in_persists_it_and_downloads_new_issues_within_the_budget() {
        let f = fixture();
        let service = &f.base.service;
        f.provider.set(OPS, &["OPS-1", "OPS-2"]);
        f.provider.attach("OPS-1", 1, "10100", "trace.log", 6);
        f.provider.attach("OPS-2", 2, "20200", "big.bin", 26 * 1024 * 1024);
        let id = follow_downloading(service, OPS).await;

        assert!(record(service, &id).await.include_attachments);
        let items = issues(service).await;
        assert_eq!(items["OPS-1"].attachments[0].state, LibraryAttachmentState::Downloaded);
        assert_eq!(stored(service, &items["OPS-1"], 0).as_deref(), Some(&b"xxxxxx"[..]));
        // Over the per-file budget: recorded, never requested from the provider.
        assert_eq!(items["OPS-2"].attachments[0].state, LibraryAttachmentState::OverLimit);
        // One exact file per request: its own size, no siblings.
        assert_eq!(f.provider.site().downloads, [("OPS-1".to_owned(), "10100".to_owned(), 6, 1, 0)]);

        // Following without the opt-in keeps the default and downloads nothing.
        f.provider.set("status = Open", &["OPS-3"]);
        f.provider.attach("OPS-3", 3, "30300", "plain.log", 6);
        let (_, plain) = follow(service, "status = Open", LibraryFollowMode::Live).await;
        assert!(!record(service, &plain).await.include_attachments);
        assert_eq!(issues(service).await["OPS-3"].attachments[0].state, LibraryAttachmentState::NotDownloaded);
        assert_eq!(f.provider.site().downloads.len(), 1);
    }

    #[tokio::test]
    async fn refresh_downloads_only_new_or_changed_attachments() {
        let f = fixture();
        let service = &f.base.service;
        f.provider.set(OPS, &["OPS-1", "OPS-2"]);
        f.provider.attach("OPS-1", 1, "10100", "trace.log", 6);
        let id = follow_downloading(service, OPS).await;
        assert_eq!(downloaded(&f), [("OPS-1".to_owned(), "10100".to_owned())]);

        // Nothing moved: nothing is fetched or downloaded again.
        f.provider.take_fetched();
        let report = refresh(service, &id).await;
        assert_eq!((report.updated, report.new, report.unchanged), (0, 0, 2), "{report:?}");
        assert!(f.provider.take_fetched().is_empty() && downloaded(&f).is_empty());

        // A new attachment on OPS-1 and one on an issue that had none: only those two download.
        f.provider.attach("OPS-1", 5, "10101", "more.log", 6);
        f.provider.attach("OPS-2", 6, "20200", "shot.log", 6);
        let report = refresh(service, &id).await;
        assert_eq!(report.updated, 2, "{report:?}");
        assert_eq!(
            downloaded(&f),
            [("OPS-1".to_owned(), "10101".to_owned()), ("OPS-2".to_owned(), "20200".to_owned())]
        );
        let items = issues(service).await;
        assert!(items["OPS-1"].attachments.iter().all(|a| a.state == LibraryAttachmentState::Downloaded));
        assert_eq!(stored(service, &items["OPS-1"], 0).as_deref(), Some(&b"xxxxxx"[..]));

        // A changed attachment (new size) downloads again; its neighbor is copied, not fetched.
        {
            let mut site = f.provider.site();
            let issue = site.issues.get_mut("OPS-1").unwrap();
            issue.minute = 7;
            issue.attachments[1].size = Some(8);
        }
        refresh(service, &id).await;
        assert_eq!(downloaded(&f), [("OPS-1".to_owned(), "10101".to_owned())]);
        let items = issues(service).await;
        assert_eq!(stored(service, &items["OPS-1"], 1).as_deref(), Some(&b"xxxxxxxx"[..]));
        assert_eq!(stored(service, &items["OPS-1"], 0).as_deref(), Some(&b"xxxxxx"[..]));
    }

    #[tokio::test]
    async fn attachments_pending_on_unchanged_issues_download_once_the_provider_allows_it() {
        let f = fixture();
        let service = &f.base.service;
        f.provider.set(OPS, &["OPS-1"]);
        f.provider.attach("OPS-1", 1, "10100", "trace.log", 6);
        // The plain follow saved the issue with its attachment not downloaded.
        follow(service, OPS, LibraryFollowMode::Live).await;
        assert!(downloaded(&f).is_empty());

        // Following the same query again with the opt-in updates the record and
        // downloads for the issue although it did not change at the source.
        f.provider.site().download_gate = Some("source_credential_required");
        assert_eq!(
            service.start_add(follow_request(OPS, true)).await.unwrap_err().code,
            "source_credential_required"
        );
        f.provider.site().download_gate = None;
        let id = follow_downloading(service, OPS).await;
        assert!(record(service, &id).await.include_attachments);
        assert_eq!(downloaded(&f), [("OPS-1".to_owned(), "10100".to_owned())]);
        assert_eq!(issues(service).await["OPS-1"].attachments[0].state, LibraryAttachmentState::Downloaded);

        // The token goes away: the text still refreshes, the report says why the
        // file did not, and nothing is lost.
        f.provider.site().download_gate = Some("source_credential_required");
        f.provider.attach("OPS-1", 5, "10101", "more.log", 6);
        let report = refresh(service, &id).await;
        assert_eq!((report.updated, report.partial), (1, 1), "{report:?}");
        let note = report.rows.iter().find_map(|row| row.reason.clone().filter(|r| r.starts_with("Attachments were not downloaded"))).unwrap();
        assert!(note.contains("token stored in Cockpit"), "{note}");
        assert!(downloaded(&f).is_empty());
        let items = issues(service).await;
        assert_eq!(items["OPS-1"].attachments[0].state, LibraryAttachmentState::Downloaded);
        assert_eq!(items["OPS-1"].attachments[1].state, LibraryAttachmentState::NotDownloaded);

        // Once the token is back, the next refresh downloads what was pending although the issue is unchanged.
        f.provider.site().download_gate = None;
        f.provider.take_fetched();
        refresh(service, &id).await;
        assert_eq!(downloaded(&f), [("OPS-1".to_owned(), "10101".to_owned())]);
        assert!(issues(service).await["OPS-1"].attachments.iter().all(|a| a.state == LibraryAttachmentState::Downloaded));
    }

    #[tokio::test]
    async fn a_refused_opt_in_leaves_no_follow_item_or_operation() {
        for code in ["source_credential_required", "credential_vault_unavailable", "source_capability_unavailable"] {
            let f = fixture();
            let service = &f.base.service;
            f.provider.set(OPS, &["OPS-1"]);
            f.provider.attach("OPS-1", 1, "10100", "trace.log", 6);
            f.provider.site().download_gate = Some(code);

            assert_eq!(service.start_add(follow_request(OPS, true)).await.unwrap_err().code, code);
            let listing = service.listing(None).await.unwrap();
            assert!(listing.follows.is_empty() && listing.items.is_empty());
            let operations = std::fs::read_dir(std::path::Path::new(&service.configuration.library_root).join(".cockpit/operations"));
            assert!(operations.map_or(true, |mut entries| entries.next().is_none()));
            let site = f.provider.site();
            assert!(site.downloads.is_empty() && site.fetched.is_empty() && site.log.is_empty());
        }
    }

    #[tokio::test]
    async fn a_shared_issue_keeps_its_files_for_a_follow_that_does_not_download_and_they_go_with_the_item() {
        let f = fixture();
        let service = &f.base.service;
        f.provider.set(OPS, &["OPS-1"]);
        f.provider.set("status = Open", &["OPS-1"]);
        f.provider.attach("OPS-1", 1, "10100", "trace.log", 6);
        let a = follow_downloading(service, OPS).await;
        let (_, b) = follow(service, "status = Open", LibraryFollowMode::Live).await;
        assert!(!record(service, &b).await.include_attachments);
        let item = issues(service).await["OPS-1"].clone();
        let root = std::path::Path::new(&service.configuration.library_root);
        let file = root.join(&item.item_path).join(item.attachments[0].relative_path.as_deref().unwrap());
        assert!(file.exists());

        // The issue changes; the non-downloading follow saves the new revision and keeps the file.
        f.provider.touch("OPS-1", 5);
        let report = refresh(service, &b).await;
        assert_eq!(report.updated, 1, "{report:?}");
        let item = issues(service).await["OPS-1"].clone();
        assert_eq!(item.attachments[0].state, LibraryAttachmentState::Downloaded);
        assert!(file.exists());
        assert!(downloaded(&f).len() == 1);

        // Stopping one follow keeps the item and its file; removing the last one removes both.
        service.remove(LibraryRemoveRequest::Follow { follow_id: a }).await.unwrap();
        assert!(file.exists() && issues(service).await.contains_key("OPS-1"));
        service.remove(LibraryRemoveRequest::Follow { follow_id: b }).await.unwrap();
        assert!(issues(service).await.is_empty());
        assert!(!file.exists() && !root.join(&item.item_path).exists());
    }

    fn reason_of(item: &LibraryItemSummary, follow_id: &str) -> Option<(String, String, u32)> {
        item.included_by.iter().flatten().find_map(|inclusion| match &inclusion.holder {
            LibraryInclusionHolder::Follow { follow_id: id } if id == follow_id => {
                Some((inclusion.from_label.clone(), inclusion.relation.clone(), inclusion.depth))
            }
            _ => None,
        })
    }

    #[tokio::test]
    async fn reference_depth_follows_related_items_and_never_drops_on_an_incomplete_pass() {
        let f = fixture();
        let service = &f.base.service;
        f.provider.set(OPS, &["OPS-1"]);
        f.provider.mention("OPS-1", 5, "See OPS-2");
        let (_, id) = follow_at(service, OPS, LibraryFollowMode::Live, 1).await;
        assert_eq!(f.provider.take_fetched(), keys(&["OPS-1", "OPS-2"]));
        let items = issues(service).await;
        assert!(held(&items["OPS-1"], &id) && refs::related_of(&items["OPS-2"], &id));
        assert!(items["OPS-1"].included_by.is_none());
        assert_eq!(reason_of(&items["OPS-2"], &id), Some(("OPS-1".into(), "description".into(), 1)));
        assert!(held(&items["OPS-2"], &id));
        assert_eq!(record(service, &id).await.reference_depth, Some(1));
        assert_eq!(record(service, &id).await.item_count, 2);

        // Nothing changed: the seed is traversed from its stored references (it is
        // not fetched again), the related item is fetched and is not a dropped member.
        let report = refresh(service, &id).await;
        assert_eq!((report.dropped, report.partial), (0, 0), "{report:?}");
        assert_eq!(f.provider.take_fetched(), keys(&["OPS-2"]));

        // The seed stops mentioning it: a complete pass drops the related item.
        f.provider.mention("OPS-1", 6, "nothing to see");
        let report = refresh(service, &id).await;
        assert_eq!(report.dropped, 1, "{report:?}");
        let items = issues(service).await;
        assert!(items["OPS-2"].refs.is_empty() && items["OPS-2"].purge_after.is_some());
        assert!(items["OPS-2"].included_by.is_none());

        // A listed item is a seed, whatever else mentions it.
        f.provider.mention("OPS-1", 7, "See OPS-2 and OPS-3");
        f.provider.set(OPS, &["OPS-1", "OPS-2"]);
        refresh(service, &id).await;
        let items = issues(service).await;
        assert!(held(&items["OPS-2"], &id) && items["OPS-2"].included_by.is_none());
        assert!(refs::related_of(&items["OPS-3"], &id));

        // OPS-2 leaves the query but OPS-1 still mentions it, and OPS-3 cannot be
        // fetched: the pass is incomplete, so nothing is dropped and the follow says so.
        f.provider.set(OPS, &["OPS-1"]);
        f.provider.site().gone.insert("OPS-3".into());
        let report = refresh(service, &id).await;
        assert_eq!((report.dropped, report.partial), (0, 1), "{report:?}");
        let items = issues(service).await;
        assert!(refs::related_of(&items["OPS-2"], &id) && held(&items["OPS-2"], &id));
        assert!(held(&items["OPS-3"], &id), "an unreachable related item is kept");
        assert_eq!(record(service, &id).await.state, LibraryItemState::Partial);
        f.provider.site().gone.clear();
        let report = refresh(service, &id).await;
        assert_eq!((report.dropped, report.partial), (0, 0), "{report:?}");
        assert_eq!(record(service, &id).await.state, LibraryItemState::Fresh);

        // Depth 0 reconciles the related items away on a complete live refresh.
        let (readded, _) = follow_at(service, OPS, LibraryFollowMode::Live, 0).await;
        assert_eq!(readded.report.unwrap().dropped, 2);
        let items = issues(service).await;
        assert!(items["OPS-2"].refs.is_empty() && items["OPS-3"].refs.is_empty());
        assert!(held(&items["OPS-1"], &id));
        assert_eq!(record(service, &id).await.reference_depth, None);
    }

    #[tokio::test]
    async fn failed_seed_refresh_never_authorizes_a_related_drop() {
        let f = fixture();
        let service = &f.base.service;
        f.provider.set(OPS, &["OPS-1", "OPS-2"]);
        f.provider.mention("OPS-2", 5, "See OPS-3");
        let (_, id) = follow_at(service, OPS, LibraryFollowMode::Live, 1).await;
        assert!(refs::related_of(&issues(service).await["OPS-3"], &id));

        // OPS-2 leaves the query and OPS-1 now mentions OPS-3, but OPS-1 cannot be
        // fetched: its stored (empty) references are not current, so nothing drops.
        f.provider.set(OPS, &["OPS-1"]);
        f.provider.mention("OPS-1", 6, "See OPS-3");
        f.provider.site().broken.insert("OPS-1".into());
        let report = refresh(service, &id).await;
        assert_eq!(report.dropped, 0, "{report:?}");
        let items = issues(service).await;
        assert!(held(&items["OPS-2"], &id) && held(&items["OPS-3"], &id));
        assert_eq!(record(service, &id).await.state, LibraryItemState::Partial);

        // Once OPS-1 is fetched, the drop is safe: OPS-2 goes, OPS-3 stays reachable.
        f.provider.site().broken.clear();
        let report = refresh(service, &id).await;
        assert_eq!(report.dropped, 1, "{report:?}");
        let items = issues(service).await;
        assert!(items["OPS-2"].refs.is_empty());
        assert_eq!(reason_of(&items["OPS-3"], &id), Some(("OPS-1".into(), "description".into(), 1)));
    }

    #[tokio::test]
    async fn removing_a_related_item_excludes_its_library_id_not_its_bare_key() {
        let f = fixture();
        let service = &f.base.service;
        f.provider.set(OPS, &["OPS-1"]);
        f.provider.mention("OPS-1", 5, "See OPS-2");
        let (_, id) = follow_at(service, OPS, LibraryFollowMode::Live, 1).await;
        let related = issues(service).await["OPS-2"].clone();
        service
            .remove(LibraryRemoveRequest::Item {
                item_id: related.item_id.clone(),
                expected_revision: related.revision.clone(),
            })
            .await
            .unwrap();
        // Another site's OPS-2 has a different Library id, so it is not excluded.
        assert_eq!(record(service, &id).await.excluded_ids, vec![related.item_id.clone()]);
        let report = refresh(service, &id).await;
        assert_eq!((report.partial, report.dropped), (0, 0), "{report:?}");
        assert!(!issues(service).await.contains_key("OPS-2"), "the removed related item stays out");
    }
}
