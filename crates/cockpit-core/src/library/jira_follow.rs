//! Followed Jira JQL queries: input recognition, add and the refresh engine.
//! An issue is one Library item whatever follows list it; a follow only holds
//! a reference to it. A `live` follow mirrors its query, an `accumulate`
//! follow keeps every issue that ever matched.
use super::{
    LibraryService, SaveOptions,
    follow::{
        FOLLOW_FETCH_CONCURRENCY, FetchWork, FetchedBatch,
        plan::{self, ChangeReason, JiraFollowPlan, MemberScope, Pass, Planned, Snapshot, Window},
        recount,
    },
    item_id, operations, refs,
    related::{Holder, RelatedPass, Seeds, stale_state},
    store::{LibraryIndexEntry, Store, error},
};
use crate::{
    InspectionError,
    jira_query::{JiraQueryInput, has_relative_dates, jira_query_input},
    project_store::timestamp,
    sources::{IssueListing, IssueQuery, IssueRow, SourceAuthority, SourceRef, site_authority},
};
use cockpit_protocol::{library::*, projects::ProviderKind};
use sha2::{Digest, Sha256};
use std::{
    collections::{BTreeMap, BTreeSet},
    sync::{Arc, atomic::AtomicBool},
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

pub(super) fn issue_item_id(follow: &LibraryFollowSummary, key: &str) -> String {
    item_id(&SourceRef {
        provider_id: follow.provider_id.clone(),
        provider_instance: follow.provider_instance.clone(),
        resource_type: "issue".into(),
        canonical_id: key.to_owned(),
    })
}

pub(super) fn is_issue_of(entry: &LibraryIndexEntry, follow: &LibraryFollowSummary) -> bool {
    entry.summary.provider_id.as_deref() == Some(follow.provider_id.as_str())
        && entry.summary.provider_instance.as_deref() == Some(follow.provider_instance.as_str())
        && entry.summary.resource_type.as_deref() == Some("issue")
}

impl LibraryService {
    fn jira_site(&self, provider_id: &str) -> Result<SourceAuthority, InspectionError> {
        let jira = self
            .configuration
            .providers
            .iter()
            .any(|provider| provider.id == provider_id && provider.kind == ProviderKind::Jira);
        if !jira {
            return Err(error(
                "source_provider_unsupported",
                "selected provider is not a configured Jira provider",
            ));
        }
        site_authority(&self.configuration, provider_id)
    }

    /// The Jira query an input denotes and the provider that runs it, or
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
            .filter(|provider| provider.kind == ProviderKind::Jira)
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
                &IssueQuery::Jql {
                    jql: &query.jql,
                    updated_window: None,
                },
                PREVIEW_ROWS,
                &AtomicBool::new(false),
            )
            .await?;
        let id = jira_follow_id(provider_id, &site.provider_instance, &query.jql);
        let existing = {
            let store = self.open()?;
            let _lock = store.shared()?;
            store
                .index()?
                .follows
                .into_iter()
                .find(|follow| follow.follow_id == id)
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
            self.sources
                .attachment_downloads(&provider_id, "issue")
                .await?;
        }
        let site = self.jira_site(&provider_id)?;
        let mode = request
            .follow_mode
            .unwrap_or_else(|| Self::suggested_mode(&query.jql));
        let follow = LibraryFollowSummary {
            follow_id: jira_follow_id(&provider_id, &site.provider_instance, &query.jql),
            provider_id,
            provider_instance: site.provider_instance,
            source: LibraryFollowSource::JiraQuery {
                jql: query.jql,
                mode,
            },
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
        let lease = super::sync::manual_lease(&store, &follow.follow_id).await?;
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
            service
                .refresh_jira_follow(&worker_store, &id, follow, true)
                .await?;
            refs::purge_expired(&worker_store, &id)
        });
        Ok(record)
    }

    /// A failed listing leaves membership untouched.
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
        plan::mark_failed(store, &follow.follow_id)?;
        operations::follow_row(
            store,
            operation,
            follow,
            LibraryReportOutcome::Failed,
            Some(failure.message),
        )
    }

    /// Manual refresh shares provider stages; only complete live passes may drop.
    pub(super) async fn refresh_jira_follow(
        &self,
        store: &Arc<Store>,
        operation: &str,
        follow: LibraryFollowSummary,
        create: bool,
    ) -> Result<(), InspectionError> {
        let LibraryFollowSource::JiraQuery { jql, mode } = &follow.source else {
            return Err(error(
                "library_corrupt",
                "Followed query has no Jira source",
            ));
        };
        let live = *mode == LibraryFollowMode::Live;
        match self.jira_site(&follow.provider_id) {
            Ok(site) if site.provider_instance == follow.provider_instance => {}
            Ok(_) => {
                return self.listing_failed(
                    store,
                    operation,
                    &follow,
                    create,
                    error(
                        "source_authority_mismatch",
                        "Followed query belongs to a different configured site",
                    ),
                );
            }
            Err(failure) => return self.listing_failed(store, operation, &follow, create, failure),
        }
        let manual = Pass::Manual { store, operation };
        let listing = match JiraFollowPlan::list(self, &follow, jql, Window::Full, manual).await {
            Ok(Some(listing)) => listing,
            Ok(None) => return Ok(()),
            Err(failure) => return self.listing_failed(store, operation, &follow, create, failure),
        };
        if create {
            plan::upsert_record(store, &follow, true)?;
        }
        let pipeline = JiraFollowPlan {
            listing,
            snapshot: Snapshot::read(store, &follow.follow_id)?,
        };
        let members = pipeline.members(&follow, MemberScope::Manual);
        let listed = pipeline
            .listing
            .rows
            .iter()
            .filter(|row| !pipeline.snapshot.excluded.contains(&row.key))
            .map(|row| (row.key.clone(), row.clone()))
            .collect::<BTreeMap<_, _>>();
        let present = listed.keys().cloned().collect();
        let missing = pipeline.absent(&members, &present).missing;
        let mut checked = BTreeMap::new();
        let mut absent = BTreeSet::new();
        let mut unverified = false;
        if !live && !missing.is_empty() {
            let keys = missing.into_iter().collect::<Vec<_>>();
            let batch = match pipeline.check_members(self, &follow, &keys, manual).await {
                Ok(Some(batch)) => batch,
                Ok(None) => return Ok(()),
                Err(failure) => {
                    return self.listing_failed(store, operation, &follow, create, failure);
                }
            };
            checked = batch
                .rows
                .into_iter()
                .map(|row| (row.key.clone(), row))
                .collect();
            if batch.complete {
                absent = pipeline
                    .absent(
                        &keys.into_iter().collect(),
                        &checked.keys().cloned().collect(),
                    )
                    .missing;
            } else {
                unverified = true;
            }
        }
        let download = self.attachments_allowed(store, operation, &follow).await?;
        let mut classified = pipeline.classify(
            &follow,
            listed.values().chain(checked.values()),
            download,
            true,
        );
        for key in absent {
            let old = pipeline.snapshot.items.get(&issue_item_id(&follow, &key));
            if old.is_some_and(|old| old.summary.state == LibraryItemState::RemovedAtSource) {
                continue;
            }
            classified.fetch.push(Planned {
                key: key.into(),
                row: None,
                old: old.cloned(),
                reason: Some(ChangeReason::Confirming),
            });
        }
        operations::add_total(
            store,
            operation,
            classified.fetch.len() as u32 + classified.unchanged,
        )?;
        let (mut cancelled, failures, attempted) = self
            .fetch_issues(store, operation, &follow, classified.fetch, download)
            .await?;
        operations::unchanged(store, operation, classified.unchanged)?;
        cancelled = cancelled || operations::cancelled(store, operation)?;
        let pass = if !cancelled && pipeline.snapshot.depth() > 0 {
            self.issue_related(
                store,
                operation,
                &follow,
                &pipeline.snapshot,
                &members,
                &listed,
                &checked,
                &attempted,
                live,
            )
            .await?
        } else {
            RelatedPass::none()
        };
        cancelled |= pass.cancelled;
        let partial = Self::jira_partial(
            live,
            cancelled,
            pipeline.listing.complete,
            unverified,
            failures,
            (listed.len() + checked.len()) as u64,
            &pass,
        );
        let (dropped, kept) = self.commit_jira_follow(
            store,
            &follow,
            &listed,
            &checked,
            &pass,
            partial,
            cancelled,
            live && pipeline.listing.complete && !cancelled && pass.complete,
        )?;
        self.report_jira_follow(
            store,
            operation,
            &follow,
            &dropped,
            kept,
            cancelled,
            &pass,
            &pipeline.listing,
            unverified,
        )
    }

    async fn attachments_allowed(
        &self,
        store: &Store,
        operation: &str,
        follow: &LibraryFollowSummary,
    ) -> Result<bool, InspectionError> {
        // Attachments stay with a follow that asked for them, for as long as the
        // provider can download them (a token stored in Cockpit). Otherwise the
        // text still refreshes and the report says why the files did not.
        let mut download = false;
        if follow.include_attachments {
            match self
                .sources
                .attachment_downloads(&follow.provider_id, "issue")
                .await
            {
                Ok(()) => download = true,
                Err(failure) => {
                    let why = match failure.code.as_str() {
                        "source_credential_required" => {
                            "attachments need a token stored in Cockpit for this Jira site"
                        }
                        "credential_vault_unavailable" | "credential_vault_timeout" => {
                            "the credential vault is unavailable"
                        }
                        _ => "the provider cannot download attachments",
                    };
                    operations::follow_row(
                        store,
                        operation,
                        follow,
                        LibraryReportOutcome::Partial,
                        Some(format!("Attachments were not downloaded: {why}")),
                    )?;
                }
            }
        }
        Ok(download)
    }

    async fn fetch_issues(
        &self,
        store: &Arc<Store>,
        operation: &str,
        follow: &LibraryFollowSummary,
        pending: Vec<Planned<'_, IssueRow>>,
        download: bool,
    ) -> Result<(bool, u32, BTreeSet<String>), InspectionError> {
        // 5. Fetch in batches of 8 under each item's lease.
        let base = self
            .configuration
            .providers
            .iter()
            .find(|provider| provider.id == follow.provider_id)
            .map(|provider| provider.base_url.trim_end_matches('/').to_owned())
            .ok_or_else(|| {
                error(
                    "source_authority_mismatch",
                    "selected Jira provider is not configured",
                )
            })?;
        // Seeds this run tried to refresh: a failed or conflicted one has no
        // current reference set, whatever an earlier save stored.
        let attempted = pending
            .iter()
            .filter(|p| p.row.is_some())
            .map(|p| p.key.to_string())
            .collect::<BTreeSet<String>>();
        let work = pending
            .into_iter()
            .map(|pending| {
                Ok(FetchWork {
                    item_id: issue_item_id(follow, &pending.key),
                    canonical_id: pending.key.to_string(),
                    label: pending.key.to_string(),
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
            let Some(fetched) = self.fetch_batch(store, operation, follow, batch).await? else {
                cancelled = true;
                break;
            };
            let FetchedBatch { results, leases } = fetched;
            for (job, result) in results {
                if operations::cancelled(store, operation)? {
                    cancelled = true;
                    break;
                }
                if self
                    .save_issue_result(store, operation, follow, job, result, download)
                    .await?
                {
                    failures += 1;
                }
            }
            drop(leases);
            if cancelled {
                break;
            }
        }
        Ok((cancelled, failures, attempted))
    }

    async fn issue_related(
        &self,
        store: &Arc<Store>,
        operation: &str,
        follow: &LibraryFollowSummary,
        snapshot: &Snapshot,
        members: &BTreeSet<String>,
        listed: &BTreeMap<String, IssueRow>,
        checked: &BTreeMap<String, IssueRow>,
        attempted: &BTreeSet<String>,
        live: bool,
    ) -> Result<RelatedPass, InspectionError> {
        let keys = if live {
            listed.keys().cloned().collect::<BTreeSet<_>>()
        } else {
            listed
                .keys()
                .chain(checked.keys())
                .chain(
                    members
                        .iter()
                        .filter(|key| !snapshot.excluded.contains(*key)),
                )
                .cloned()
                .collect()
        };
        let stored = {
            let _lock = store.shared()?;
            store
                .index()?
                .items
                .into_iter()
                .filter(|entry| is_issue_of(entry, follow))
                .filter_map(|entry| Some((entry.summary.canonical_id.clone()?, entry)))
                .collect::<BTreeMap<_, _>>()
        };
        let mut seeds = Seeds::default();
        for key in keys {
            if let Some(entry) = stored.get(&key) {
                let stale = attempted.contains(&key) && stale_state(entry.summary.state);
                seeds.push_stored(entry, key, stale);
            } else {
                seeds.unknown += 1;
            }
        }
        self.traverse(
            store,
            operation,
            &Holder::FollowIssues {
                follow,
                excluded: &snapshot.excluded,
            },
            snapshot.depth(),
            seeds,
        )
        .await
    }

    fn jira_partial(
        live: bool,
        cancelled: bool,
        listing_complete: bool,
        unverified: bool,
        failures: u32,
        have: u64,
        pass: &RelatedPass,
    ) -> Option<LibraryPartial> {
        if cancelled {
            (!live).then(|| LibraryPartial {
                unit: "issues".into(),
                have,
                total: None,
                reason: "cancelled".into(),
            })
        } else if !listing_complete {
            Some(LibraryPartial {
                unit: "issues".into(),
                have,
                total: None,
                reason: "issue limit".into(),
            })
        } else if unverified {
            Some(LibraryPartial {
                unit: "issues".into(),
                have,
                total: None,
                reason: "members could not all be checked".into(),
            })
        } else if !live && failures > 0 {
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
        }
    }

    fn commit_jira_follow(
        &self,
        store: &Store,
        follow: &LibraryFollowSummary,
        listed: &BTreeMap<String, IssueRow>,
        checked: &BTreeMap<String, IssueRow>,
        pass: &RelatedPass,
        partial: Option<LibraryPartial>,
        cancelled: bool,
        drop_allowed: bool,
    ) -> Result<(Vec<LibraryItemSummary>, Option<u32>), InspectionError> {
        let listed_rows = listed
            .iter()
            .map(|(key, row)| (key.as_str(), row))
            .collect::<BTreeMap<_, _>>();
        let meta_rows = listed
            .iter()
            .chain(checked.iter())
            .map(|(key, row)| (key.as_str(), row))
            .collect::<BTreeMap<_, _>>();
        let now = timestamp().parse::<u128>().unwrap_or(0);
        store.mutate_index(|index| {
            plan::apply_issues(index, follow, &listed_rows, &meta_rows, true)
                .ok_or_else(|| error("library_item_not_found", "Followed query was removed"))?;
            let excluded = index
                .follows
                .iter()
                .find(|f| f.follow_id == follow.follow_id)
                .expect("follow checked above")
                .excluded_ids
                .iter()
                .cloned()
                .collect::<BTreeSet<_>>();
            let members = index
                .items
                .iter()
                .filter(|entry| is_issue_of(entry, follow))
                .filter(|entry| {
                    entry
                        .summary
                        .canonical_id
                        .as_ref()
                        .is_some_and(|key| !excluded.contains(key))
                })
                .filter(|entry| {
                    refs::has_follow(&entry.summary, &follow.follow_id)
                        && !refs::related_of(&entry.summary, &follow.follow_id)
                })
                .count() as u32;
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
                        if !is_issue_of(entry, follow) {
                            continue;
                        }
                        let Some(key) = entry.summary.canonical_id.clone() else {
                            continue;
                        };
                        if listed.contains_key(&key) || excluded.contains(&key) {
                            continue;
                        }
                    }
                    let before = entry.summary.clone();
                    refs::release_follow(&mut entry.summary, &follow.follow_id, now);
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
        })
    }

    fn report_jira_follow(
        &self,
        store: &Store,
        operation: &str,
        follow: &LibraryFollowSummary,
        dropped: &[LibraryItemSummary],
        kept: Option<u32>,
        cancelled: bool,
        pass: &RelatedPass,
        listing: &IssueListing,
        unverified: bool,
    ) -> Result<(), InspectionError> {
        let listing_complete = listing.complete;
        for summary in dropped {
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
                follow,
                LibraryReportOutcome::Partial,
                Some(note.clone()),
            )?;
        }
        if let Some(members) = kept {
            operations::follow_row(
                store,
                operation,
                follow,
                LibraryReportOutcome::Partial,
                Some(format!("Query returned no issues; kept {members} issues")),
            )?;
        } else if !listing_complete {
            operations::follow_row(
                store,
                operation,
                follow,
                LibraryReportOutcome::Partial,
                Some(format!("{} issues (issue limit)", listing.rows.len())),
            )?;
        } else if unverified {
            operations::follow_row(
                store,
                operation,
                follow,
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
        job: &FetchWork<Planned<'_, IssueRow>>,
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
                        reference: Some(LibraryItemRef::Follow {
                            follow_id: follow.follow_id.clone(),
                        }),
                        reason: pending.reason.map(|reason| reason.to_string()),
                        issue_row: pending.row.as_ref(),
                        download_all: download,
                        ..SaveOptions::default()
                    },
                )
                .await
            }
            Ok(_) => Err(error(
                "source_identity_mismatch",
                "Provider refresh returned a different issue",
            )),
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
mod tests;
