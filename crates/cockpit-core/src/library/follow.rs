//! Followed Confluence spaces: durable records, refresh, exclusions and removal.
use super::{
    LibraryService, SaveOptions, confluence_instance_authority, item_id, operations, refs,
    store::{Index, Lease, LibraryIndexEntry, Store, error},
};
use crate::{
    InspectionError,
    project_store::timestamp,
    sources::{
        ProviderResolution, SourceAsset, SourceFetchRequest, SourceRef, SpacePage,
        confluence_page_url, site_authority,
    },
};
use cockpit_protocol::{
    library::*,
    projects::{ProjectDiagnostic, ProviderKind},
};
use sha2::{Digest, Sha256};
use std::{
    collections::{BTreeMap, BTreeSet},
    sync::{Arc, atomic::AtomicBool},
};

#[path = "follow_plan/mod.rs"]
pub(super) mod plan;
use plan::{ConfluenceFollowPlan, PageClassify, PageOrder, PageProbe, Pass, Snapshot, Window};

pub(super) const FOLLOW_FETCH_CONCURRENCY: usize = 8;

/// One item a follow refresh fetches. `tag` carries what the caller needs to
/// save the result. A `container` demands the fetched asset lives there.
pub(super) struct FetchWork<T> {
    pub item_id: String,
    pub canonical_id: String,
    /// Names the item in a failure row.
    pub label: String,
    pub request: SourceFetchRequest,
    pub container: Option<String>,
    pub tag: T,
}

/// A fetched batch in work order. The item leases are held until it is dropped.
pub(super) struct FetchedBatch<'a, T> {
    pub results: Vec<(&'a FetchWork<T>, Result<SourceAsset, InspectionError>)>,
    pub leases: Vec<Lease>,
}

/// Stable identity derived from provider, instance and space key.
pub(super) fn follow_id(provider_id: &str, provider_instance: &str, space_key: &str) -> String {
    let mut hash = Sha256::new();
    for value in [provider_id, provider_instance, space_key] {
        hash.update(value.as_bytes());
        hash.update([0]);
    }
    format!("follow:{:x}", hash.finalize())
}

pub(super) fn page_item_id(follow: &LibraryFollowSummary, page_id: &str) -> String {
    item_id(&SourceRef {
        provider_id: follow.provider_id.clone(),
        provider_instance: follow.provider_instance.clone(),
        resource_type: "page".into(),
        canonical_id: page_id.to_owned(),
    })
}

/// A page item of `follow`'s site, whatever its current follow membership.
pub(super) fn same_site_page(entry: &LibraryIndexEntry, follow: &LibraryFollowSummary) -> bool {
    entry.summary.provider_id.as_deref() == Some(follow.provider_id.as_str())
        && entry.summary.provider_instance.as_deref() == Some(follow.provider_instance.as_str())
        && entry.summary.resource_type.as_deref() == Some("page")
}

impl LibraryService {
    fn confluence_site(
        &self,
        provider_id: &str,
    ) -> Result<crate::sources::SourceAuthority, InspectionError> {
        let confluence = self.configuration.providers.iter().any(|provider| {
            provider.id == provider_id && provider.kind == ProviderKind::Confluence
        });
        if !confluence {
            return Err(error(
                "source_provider_unsupported",
                "selected provider is not a configured Confluence provider",
            ));
        }
        site_authority(&self.configuration, provider_id)
    }

    /// Browse the spaces of one configured Confluence provider.
    pub async fn confluence_spaces(
        &self,
        provider_id: &str,
    ) -> Result<Vec<LibraryResolution>, InspectionError> {
        let site = self.confluence_site(provider_id)?;
        let spaces = self.sources.list_spaces(provider_id).await?;
        let store = self.open()?;
        let follows = {
            let _lock = store.shared()?;
            store.index()?.follows
        };
        Ok(spaces
            .into_iter()
            .map(|space| {
                let id = follow_id(provider_id, &site.provider_instance, &space.key);
                let existing = follows.iter().find(|follow| follow.follow_id == id);
                LibraryResolution {
                    kind: LibraryInputKind::ConfluenceSpace,
                    provider_id: Some(provider_id.to_owned()),
                    provider_instance: Some(site.provider_instance.clone()),
                    container_label: Some(format!("{} · {}", space.key, space.name)),
                    title: space.name,
                    canonical_id: Some(space.key),
                    existing_item_id: None,
                    existing_follow_id: existing.map(|follow| follow.follow_id.clone()),
                    item_count: existing.map(|follow| follow.item_count),
                    item_count_exact: true,
                    follow_mode: None,
                    git_working_tree: None,
                    file_count: None,
                    diagnostics: vec![],
                    reference_depth: None,
                }
            })
            .collect())
    }

    /// A space input: its name and size come from the first enumeration page.
    pub(super) async fn resolve_space(
        &self,
        provider_id: &str,
        space_key: &str,
    ) -> Result<LibraryResolution, InspectionError> {
        let site = self.confluence_site(provider_id)?;
        let listing = self
            .sources
            .list_space_pages(provider_id, space_key, 1, &AtomicBool::new(false))
            .await?;
        let id = follow_id(provider_id, &site.provider_instance, space_key);
        let store = self.open()?;
        let existing = {
            let _lock = store.shared()?;
            store
                .index()?
                .follows
                .into_iter()
                .find(|follow| follow.follow_id == id)
        };
        let page_count = listing
            .total
            .map(|total| total.min(u32::MAX as u64) as u32)
            .or(existing.as_ref().map(|follow| follow.item_count));
        Ok(LibraryResolution {
            kind: LibraryInputKind::ConfluenceSpace,
            provider_id: Some(provider_id.to_owned()),
            provider_instance: Some(site.provider_instance),
            title: listing.space_name.clone(),
            canonical_id: Some(space_key.to_owned()),
            container_label: Some(format!("{space_key} · {}", listing.space_name)),
            existing_item_id: None,
            existing_follow_id: existing.map(|follow| follow.follow_id),
            item_count: page_count,
            item_count_exact: true,
            follow_mode: None,
            git_working_tree: None,
            file_count: None,
            diagnostics: vec![],
            reference_depth: None,
        })
    }

    /// Follow a space named directly or through one of its pages. Following an
    /// already-followed space is `Follow whole space`: it clears exclusions.
    pub(super) async fn start_follow_add(
        &self,
        request: LibraryAddRequest,
    ) -> Result<LibraryOperation, InspectionError> {
        if request.reference_depth > 0 {
            return Err(error(
                "source_capability_unavailable",
                "Following a space does not follow related items",
            ));
        }
        let (provider_id, resolution) = self
            .confluence_resolution(&request.input, request.provider_id.as_deref())
            .await?
            .ok_or_else(|| {
                error(
                    "library_input_unrecognized",
                    "Following requires a Confluence space or page",
                )
            })?;
        let space_key = match resolution {
            ProviderResolution::ConfluenceSpace { space_key } => space_key,
            ProviderResolution::ConfluencePage(page) => {
                confluence_instance_authority(
                    &self.configuration,
                    &provider_id,
                    &page,
                    &page.canonical_url,
                )?;
                page.space_key
            }
        };
        let site = self.confluence_site(&provider_id)?;
        let follow = LibraryFollowSummary {
            follow_id: follow_id(&provider_id, &site.provider_instance, &space_key),
            provider_id,
            provider_instance: site.provider_instance,
            source: LibraryFollowSource::ConfluenceSpace {
                space_name: space_key.clone(),
                space_key,
            },
            include_attachments: request.download_attachments,
            item_count: 0,
            partial: None,
            excluded_ids: vec![],
            last_refreshed_at: None,
            state: LibraryItemState::Unknown,
            reference_depth: None,
        };
        let handle = operations::runtime()?;
        let store = self.open()?;
        let lease = super::sync::manual_lease(&store, &follow.follow_id).await?;
        let (record, operation_lease) =
            operations::create(&store, LibraryOperationKind::Add, Some(0))?;
        let record = match &request.target {
            Some(target) => operations::set_target(&store, &record.operation_id, target.clone())?,
            None => record,
        };
        let service = self.clone();
        let id = record.operation_id.clone();
        let worker_store = store.clone();
        operations::spawn(handle, store, id.clone(), operation_lease, async move {
            let _lease = lease;
            if operations::cancelled(&worker_store, &id)? {
                return Ok(());
            }
            service
                .refresh_follow(&worker_store, &id, follow, true)
                .await?;
            if let Some(target) = &request.target {
                let saved = operations::get(&worker_store, &id)?.item_ids;
                service
                    .select_saved_items(&worker_store, &id, target, &saved)
                    .await?;
            }
            refs::purge_expired(&worker_store, &id)
        });
        Ok(record)
    }

    /// Enumerate the space once, fetch only new or changed pages,
    /// confirm absent pages after a complete run, then commit order, parents
    /// and the follow record. `create` upserts the record after a successful
    /// enumeration; otherwise enumeration failures become report rows.
    pub(super) async fn refresh_follow(
        &self,
        store: &Arc<Store>,
        operation: &str,
        mut follow: LibraryFollowSummary,
        create: bool,
    ) -> Result<(), InspectionError> {
        if matches!(follow.source, LibraryFollowSource::JiraQuery { .. }) {
            return self
                .refresh_jira_follow(store, operation, follow, create)
                .await;
        }
        let listing = match async {
            let site = self.confluence_site(&follow.provider_id)?;
            if site.provider_instance != follow.provider_instance {
                return Err(error(
                    "source_authority_mismatch",
                    "Followed space belongs to a different configured site",
                ));
            }
            ConfluenceFollowPlan::list(
                self,
                &follow,
                refs::require_space_key(&follow)?,
                Window::Full,
                Pass::Manual { store, operation },
            )
            .await
        }
        .await
        {
            Ok(Some(listing)) => listing,
            Ok(None) => return Ok(()),
            Err(failure) if create => return Err(failure),
            Err(failure) => {
                plan::mark_failed(store, &follow.follow_id)?;
                return operations::follow_row(
                    store,
                    operation,
                    &follow,
                    LibraryReportOutcome::Failed,
                    Some(failure.message),
                );
            }
        };
        refs::set_space_name(&mut follow, &listing.space_name);
        if create {
            plan::upsert_record(store, &follow, false)?;
        }
        let mut pages = listing.pages.clone();
        if let Some(homepage) = listing
            .homepage_id
            .as_ref()
            .filter(|id| !pages.iter().any(|page| &page.page_id == *id))
        {
            // A homepage omitted by search is still present and fetched if new.
            pages.push(SpacePage {
                page_id: homepage.clone(),
                title: String::new(),
                version: 0,
                ancestors: vec![],
                position: None,
            });
        }
        // Parents first, so a new child's parent item exists when it is saved.
        pages.sort_by_key(|page| page.ancestors.len());
        let homepage_only = pages
            .iter()
            .find(|page| {
                listing.homepage_id.as_ref() == Some(&page.page_id)
                    && !listing
                        .pages
                        .iter()
                        .any(|listed| listed.page_id == page.page_id)
            })
            .map(|page| page.page_id.as_str());
        operations::add_total(store, operation, pages.len() as u32)?;
        let mut cancelled = false;
        let mut pipeline = ConfluenceFollowPlan {
            listing,
            snapshot: Snapshot::read(store, &follow.follow_id)?,
        };
        let classified = pipeline.classify(
            &follow,
            &pages,
            PageClassify {
                attachments: follow.include_attachments,
                homepage_only,
                take_existing: true,
            },
        );
        let unchanged = classified.unchanged;
        let space_key = refs::require_space_key(&follow)?.to_owned();
        let site = site_authority(&self.configuration, &follow.provider_id)?;
        let work = classified
            .fetch
            .into_iter()
            .map(|planned| {
                let page = planned.row.as_ref().expect("listed page");
                FetchWork {
                    item_id: page_item_id(&follow, &planned.key),
                    canonical_id: planned.key.to_string(),
                    label: if page.title.is_empty() {
                        planned.key.to_string()
                    } else {
                        page.title.clone()
                    },
                    request: SourceFetchRequest {
                        provider_id: follow.provider_id.clone(),
                        artifact_url: confluence_page_url(&site.provider_instance, &planned.key),
                        authority: site.clone(),
                    },
                    container: Some(space_key.clone()),
                    tag: planned,
                }
            })
            .collect::<Vec<_>>();
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
                let planned = &job.tag;
                self.save_follow_page_result(
                    store,
                    operation,
                    &follow,
                    planned.row.as_ref().expect("listed page"),
                    planned.old.clone(),
                    planned.reason.map(|reason| reason.to_string()),
                    result,
                )
                .await?;
            }
            drop(leases);
            if cancelled {
                break;
            }
        }
        operations::unchanged(store, operation, unchanged)?;
        cancelled = cancelled || operations::cancelled(store, operation)?;
        let listing = &pipeline.listing;
        let present = pages
            .iter()
            .map(|page| page.page_id.clone())
            .collect::<BTreeSet<_>>();
        if listing.complete && !cancelled {
            self.confirm_absent_pages(store, operation, &follow, &present)
                .await?;
        }
        let partial = (!listing.complete && !cancelled).then(|| LibraryPartial {
            unit: "pages".into(),
            have: listing.pages.len() as u64,
            total: listing.total,
            reason: "page limit".into(),
        });
        if let Some(partial) = &partial {
            let reason = match partial.total {
                Some(total) => format!("{} of {total} pages (page limit)", partial.have),
                None => format!("{} pages (page limit)", partial.have),
            };
            operations::follow_row(
                store,
                operation,
                &follow,
                LibraryReportOutcome::Partial,
                Some(reason),
            )?;
        }
        self.commit_follow(store, &follow, &listing.pages, partial, cancelled)
    }

    /// Fetches `batch` concurrently, each item under its lease. `None` means the
    /// operation was cancelled before the batch started. An item whose lease
    /// is busy gets a Failed row and is left out of the results.
    pub(super) async fn fetch_batch<'a, T>(
        &self,
        store: &Arc<Store>,
        operation: &str,
        follow: &LibraryFollowSummary,
        batch: &'a [FetchWork<T>],
    ) -> Result<Option<FetchedBatch<'a, T>>, InspectionError> {
        if operations::cancelled(store, operation)? {
            return Ok(None);
        }
        let mut leases = Vec::with_capacity(batch.len());
        let mut jobs = tokio::task::JoinSet::new();
        let mut ordered = Vec::with_capacity(batch.len());
        for (index, work) in batch.iter().enumerate() {
            let lease = if crate::sources::lane::current()
                == crate::sources::lane::RequestLane::Background
            {
                store.lease(&work.item_id)
            } else {
                super::sync::manual_lease(store, &work.item_id).await
            };
            match lease {
                Ok(lease) => leases.push(lease),
                Err(failure) => {
                    operations::follow_row(
                        store,
                        operation,
                        follow,
                        LibraryReportOutcome::Failed,
                        Some(format!("{}: {}", work.label, failure.message)),
                    )?;
                    continue;
                }
            }
            let sources = self.sources.clone();
            let request = work.request.clone();
            let canonical_id = work.canonical_id.clone();
            let container = work.container.clone();
            jobs.spawn(crate::sources::lane::inherit(async move {
                let result = async {
                    let fetched = sources.fetch_assets(request).await?;
                    let asset = fetched
                        .assets
                        .into_iter()
                        .find(|asset| asset.source.canonical_id == canonical_id)
                        .ok_or_else(|| {
                            error(
                                "source_identity_mismatch",
                                "Provider refresh omitted the requested item",
                            )
                        })?;
                    match &container {
                        None => Ok(asset),
                        Some(expected) => match asset.container.as_ref().map(|c| c.id.as_str()) {
                            Some(key) if key == expected => Ok(asset),
                            other => Err(error(
                                "source_not_found",
                                format!("moved to {}", other.unwrap_or("another container")),
                            )),
                        },
                    }
                }
                .await;
                (index, result)
            }));
            ordered.push(index);
        }
        let mut results = BTreeMap::new();
        while let Some(joined) = jobs.join_next().await {
            let (index, asset) = joined
                .map_err(|_| error("source_provider_failed", "Provider fetch task failed"))?;
            results.insert(index, asset);
        }
        // JoinSet completes in arbitrary order; hand results back in work order.
        let results = ordered
            .into_iter()
            .map(|index| {
                let result = results.remove(&index).unwrap_or_else(|| {
                    Err(error(
                        "source_provider_failed",
                        "Provider fetch result is missing",
                    ))
                });
                (&batch[index], result)
            })
            .collect();
        Ok(Some(FetchedBatch { results, leases }))
    }

    async fn save_follow_page_result(
        &self,
        store: &Arc<Store>,
        operation: &str,
        follow: &LibraryFollowSummary,
        page: &SpacePage,
        old: Option<LibraryIndexEntry>,
        reason: Option<String>,
        result: Result<crate::sources::SourceAsset, InspectionError>,
    ) -> Result<(), InspectionError> {
        let id = page_item_id(follow, &page.page_id);
        let failed = |message: String| -> Result<(), InspectionError> {
            let title = if page.title.is_empty() {
                &page.page_id
            } else {
                &page.title
            };
            operations::follow_row(
                store,
                operation,
                follow,
                LibraryReportOutcome::Failed,
                Some(format!("{title}: {message}")),
            )
        };
        if operations::cancelled(store, operation)? {
            return Ok(());
        }
        let saved = match result {
            Ok(asset) if item_id(&asset.source) == id => {
                self.save_asset_with(
                    store,
                    operation,
                    asset,
                    old.clone(),
                    SaveOptions {
                        reference: Some(LibraryItemRef::Follow {
                            follow_id: follow.follow_id.clone(),
                        }),
                        reason,
                        download_all: follow.include_attachments,
                        ..SaveOptions::default()
                    },
                )
                .await
            }
            Ok(_) => Err(error(
                "source_identity_mismatch",
                "Provider refresh returned a different page",
            )),
            Err(failure) => Err(failure),
        };
        match (saved, old) {
            (Ok(()), _) => Ok(()),
            (Err(failure), Some(old)) => self.fetch_failed(store, operation, old, failure, false),
            (Err(failure), None) => failed(failure.message),
        }
    }

    /// Complete, uncancelled listings authorize probes, not absence alone.
    /// Removal requires confirmation that the page is gone or moved.
    async fn confirm_absent_pages(
        &self,
        store: &Arc<Store>,
        operation: &str,
        follow: &LibraryFollowSummary,
        present: &BTreeSet<String>,
    ) -> Result<(), InspectionError> {
        let space_key = refs::require_space_key(follow)?;
        let index = {
            let _lock = store.shared()?;
            store.index()?
        };
        let excluded = index
            .follows
            .iter()
            .find(|f| f.follow_id == follow.follow_id)
            .map(|record| record.excluded_ids.iter().cloned().collect())
            .unwrap_or_default();
        let missing =
            ConfluenceFollowPlan::absent(&index.items, &excluded, follow, present).missing;
        let absent = index
            .items
            .into_iter()
            .filter(|entry| {
                same_site_page(entry, follow)
                    && refs::has_follow(&entry.summary, &follow.follow_id)
                    && entry.summary.state != LibraryItemState::RemovedAtSource
                    && entry
                        .summary
                        .canonical_id
                        .as_ref()
                        .is_some_and(|key| missing.contains(key))
            })
            .map(|entry| entry.summary.item_id)
            .collect::<Vec<_>>();
        for id in absent {
            if operations::cancelled(store, operation)? {
                break;
            }
            let Ok(_lease) = store.lease(&id) else {
                continue;
            };
            let Some(mut entry) = self.entry(store, &id)? else {
                continue;
            };
            let page_id = entry.summary.canonical_id.clone().unwrap_or_default();
            let (code, reason) =
                match ConfluenceFollowPlan::probe(self, follow, &page_id, space_key).await {
                    PageProbe::Gone => ("source_not_found", "Not found at source".to_owned()),
                    PageProbe::Moved(key) => ("source_moved", format!("moved to {key}")),
                    PageProbe::Present => continue,
                    PageProbe::Failed(failure) => {
                        operations::row(
                            store,
                            operation,
                            Some(&entry.summary),
                            LibraryReportOutcome::Failed,
                            Some(failure.message),
                        )?;
                        continue;
                    }
                };
            entry.summary.state = LibraryItemState::RemovedAtSource;
            entry.summary.checked_at = Some(timestamp());
            entry.summary.diagnostics = vec![ProjectDiagnostic {
                code: code.into(),
                message: reason.clone(),
                path: None,
            }];
            store.update(entry.clone())?;
            operations::row(
                store,
                operation,
                Some(&entry.summary),
                LibraryReportOutcome::RemovedAtSource,
                Some(reason),
            )?;
        }
        Ok(())
    }

    /// One index commit: adopt listed pages, provider tree order, parents and
    /// the follow record's counts. Item files and revisions are untouched.
    fn commit_follow(
        &self,
        store: &Store,
        follow: &LibraryFollowSummary,
        listed: &[SpacePage],
        partial: Option<LibraryPartial>,
        cancelled: bool,
    ) -> Result<(), InspectionError> {
        store.mutate_index(|index| {
            plan::apply_pages(index, follow, listed, PageOrder::Rank)
                .ok_or_else(|| error("library_item_not_found", "Followed space was removed"))?;
            let record = index
                .follows
                .iter_mut()
                .find(|f| f.follow_id == follow.follow_id)
                .expect("follow checked above");
            if !cancelled {
                record.source = follow.source.clone();
                record.state = if partial.is_some() {
                    LibraryItemState::Partial
                } else {
                    LibraryItemState::Fresh
                };
                record.partial = partial;
                record.last_refreshed_at = Some(timestamp());
            }
            Ok(())
        })
    }

    /// Removing an item excludes it from every follow that lists it, so a
    /// later refresh does not bring it back. The exclusions are durable before
    /// the removal and are rolled back if the removal fails.
    pub(super) fn remove_item(&self, id: &str, revision: &str) -> Result<(), InspectionError> {
        let store = self.open()?;
        // Per follow: a related item is excluded by its Library id (another site's
        // item may share its key), any other member by its key.
        let exclusion = self.entry(&store, id)?.and_then(|entry| {
            let key = entry.summary.canonical_id.clone()?;
            let tokens = refs::follow_ids(&entry.summary)
                .into_iter()
                .map(|follow_id| {
                    let token = if refs::related_of(&entry.summary, &follow_id) {
                        entry.summary.item_id.clone()
                    } else {
                        key.clone()
                    };
                    (follow_id, token)
                })
                .collect::<Vec<_>>();
            (!tokens.is_empty()).then_some(tokens)
        });
        let added = match &exclusion {
            Some(tokens) => store.mutate_index(|index| {
                let mut added = Vec::new();
                for follow in index.follows.iter_mut() {
                    let Some((_, token)) = tokens
                        .iter()
                        .find(|(follow_id, _)| follow_id == &follow.follow_id)
                    else {
                        continue;
                    };
                    if !follow.excluded_ids.contains(token) {
                        follow.excluded_ids.push(token.clone());
                        added.push((follow.follow_id.clone(), token.clone()));
                    }
                }
                Ok(added)
            })?,
            None => Vec::new(),
        };
        let removed = store.remove(id, revision);
        if let Some(tokens) = &exclusion {
            store.mutate_index(|index| {
                for (follow_id, _) in tokens {
                    recount(index, follow_id);
                }
                if removed.is_err() {
                    for (follow_id, token) in &added {
                        if let Some(follow) =
                            index.follows.iter_mut().find(|f| &f.follow_id == follow_id)
                        {
                            follow.excluded_ids.retain(|excluded| excluded != token);
                        }
                    }
                }
                Ok(())
            })?;
        }
        removed
    }

    /// `Stop following` keeps every member as an ordinary item (an item left
    /// without references becomes Manual). `Remove` also deletes the members
    /// this follow alone holds, refusing when one was edited in the Library.
    pub(super) fn remove_follow(
        &self,
        follow_id: &str,
        delete_pages: bool,
    ) -> Result<(), InspectionError> {
        let store = self.open()?;
        let _lease = store.lease(follow_id)?;
        let own = LibraryItemRef::Follow {
            follow_id: follow_id.to_owned(),
        };
        let exclusive = {
            let _lock = store.shared()?;
            let index = store.index()?;
            if !index.follows.iter().any(|f| f.follow_id == follow_id) {
                return Err(error(
                    "library_item_not_found",
                    "Followed space does not exist",
                ));
            }
            let exclusive = if delete_pages {
                index
                    .items
                    .into_iter()
                    .filter(|e| e.summary.refs == [own.clone()])
                    .collect::<Vec<_>>()
            } else {
                vec![]
            };
            for page in &exclusive {
                if !store.conflicts(page)?.is_empty() {
                    return Err(error(
                        "library_conflict",
                        "An item of this follow was edited in the Library",
                    ));
                }
            }
            exclusive
        };
        let now = timestamp();
        store.mutate_index(|index| {
            index.follows.retain(|f| f.follow_id != follow_id);
            for entry in &mut index.items {
                if refs::remove_ref(&mut entry.summary, &own) && entry.summary.refs.is_empty() {
                    if delete_pages {
                        entry.summary.purge_after = Some(now.clone());
                    } else {
                        refs::insert_ref(&mut entry.summary, LibraryItemRef::Manual);
                    }
                }
            }
            Ok(())
        })?;
        for page in &exclusive {
            let removed =
                store.remove_where(&page.summary.item_id, &page.summary.revision, |entry| {
                    if entry.summary.refs.is_empty() {
                        Ok(())
                    } else {
                        Err(error(
                            "library_item_referenced",
                            "Library item gained a reference",
                        ))
                    }
                });
            match removed {
                // A concurrent copy took a reference; the item stays.
                Err(failure) if failure.code == "library_item_referenced" => {}
                other => other?,
            }
        }
        Ok(())
    }
}

/// Recompute a follow record's member count from the items holding it.
pub(super) fn recount(index: &mut Index, follow_id: &str) {
    let count = index
        .items
        .iter()
        .filter(|e| refs::has_follow(&e.summary, follow_id))
        .count() as u32;
    if let Some(follow) = index.follows.iter_mut().find(|f| f.follow_id == follow_id) {
        follow.item_count = count;
    }
}

#[cfg(test)]
mod tests;
