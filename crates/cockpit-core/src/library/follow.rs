//! S6 followed Confluence spaces: durable follow records in the Library index,
//! D20 enumeration refresh, D17 exclusions and follow removal.
use super::{
    LibraryService, SaveOptions, confluence_instance_authority, item_id, operations, refs,
    store::{Index, LibraryIndexEntry, Lease, Store, error},
};
use crate::{
    InspectionError,
    project_store::timestamp,
    sources::{
        ProviderResolution, SourceAsset, SourceFetchRequest, SourceRef, SpacePage,
        confluence_page_url, site_authority,
    },
};
use cockpit_protocol::{library::*, projects::ProjectDiagnostic};
use sha2::{Digest, Sha256};
use std::{
    collections::{BTreeMap, BTreeSet},
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    time::Duration,
};

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

/// D3: `follow:<sha256(provider_id \0 instance \0 space_key)>`.
pub(super) fn follow_id(provider_id: &str, provider_instance: &str, space_key: &str) -> String {
    let mut hash = Sha256::new();
    for value in [provider_id, provider_instance, space_key] {
        hash.update(value.as_bytes());
        hash.update([0]);
    }
    format!("follow:{:x}", hash.finalize())
}

fn page_item_id(follow: &LibraryFollowSummary, page_id: &str) -> String {
    item_id(&SourceRef {
        provider_id: follow.provider_id.clone(),
        provider_instance: follow.provider_instance.clone(),
        resource_type: "page".into(),
        canonical_id: page_id.to_owned(),
    })
}

/// A page item of `follow`'s site, whatever its current follow membership.
fn same_site_page(entry: &LibraryIndexEntry, follow: &LibraryFollowSummary) -> bool {
    entry.summary.provider_id.as_deref() == Some(follow.provider_id.as_str())
        && entry.summary.provider_instance.as_deref() == Some(follow.provider_instance.as_str())
        && entry.summary.resource_type.as_deref() == Some("page")
}

/// Why a listed page needs a fetch, or `None` when the stored item matches it.
fn change_reason(old: &LibraryIndexEntry, page: &SpacePage) -> Option<String> {
    let ancestors = old.summary.ancestors.iter().map(|a| a.id.as_str());
    if !ancestors.eq(page.ancestors.iter().map(String::as_str)) {
        Some("moved".into())
    } else if old.summary.source_revision.as_deref() != Some(page.version.to_string().as_str()) {
        Some("changed".into())
    } else if old.summary.title != page.title {
        Some("renamed".into())
    } else if matches!(
        old.summary.state,
        LibraryItemState::RemovedAtSource | LibraryItemState::Failed | LibraryItemState::Unknown
    ) {
        Some("rechecked".into())
    } else {
        None
    }
}

impl LibraryService {
    fn confluence_site(
        &self,
        provider_id: &str,
    ) -> Result<crate::sources::SourceAuthority, InspectionError> {
        let confluence = self.configuration.providers.iter().any(|provider| {
            provider.id == provider_id
                && crate::repositories::is_confluence_executable(&provider.executable)
        });
        if !confluence {
            return Err(error(
                "source_provider_unsupported",
                "selected provider is not a configured Confluence provider",
            ));
        }
        site_authority(&self.configuration, provider_id)
    }

    /// UQ5a: browse the spaces of one configured Confluence provider.
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
            store.index()?.follows.into_iter().find(|follow| follow.follow_id == id)
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
            return Err(error("source_capability_unavailable", "Following a space does not follow related items"));
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
        let lease = store.lease(&follow.follow_id)?;
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
                service.select_saved_items(&worker_store, &id, target, &saved).await?;
            }
            refs::purge_expired(&worker_store, &id)
        });
        Ok(record)
    }

    /// Enumerate `follow`'s space once (D20), fetch only new or changed pages,
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
            return self.refresh_jira_follow(store, operation, follow, create).await;
        }
        let listing = match self.enumerate(store, operation, &follow).await {
            Ok(Some(listing)) => listing,
            Ok(None) => return Ok(()),
            Err(failure) if create => return Err(failure),
            Err(failure) => {
                let message = failure.message.clone();
                store.mutate_index(|index| {
                    if let Some(record) =
                        index.follows.iter_mut().find(|f| f.follow_id == follow.follow_id)
                    {
                        record.state = LibraryItemState::Failed;
                    }
                    Ok(())
                })?;
                return operations::follow_row(
                    store,
                    operation,
                    &follow,
                    LibraryReportOutcome::Failed,
                    Some(message),
                );
            }
        };
        refs::set_space_name(&mut follow, &listing.space_name);
        if create {
            let created = follow.clone();
            store.mutate_index(|index| {
                match index.follows.iter_mut().find(|f| f.follow_id == created.follow_id) {
                    Some(record) => {
                        record.excluded_ids.clear();
                        record.source = created.source.clone();
                        record.include_attachments = created.include_attachments;
                    }
                    None => index.follows.push(created),
                }
                Ok(())
            })?;
        }
        let mut pages = listing.pages.clone();
        if let Some(homepage) = listing
            .homepage_id
            .as_ref()
            .filter(|id| !pages.iter().any(|page| &page.page_id == *id))
        {
            // D20: the provider includes the homepage; a listing that omits it
            // still never marks it removed, and a missing item is fetched.
            pages.push(SpacePage {
                page_id: homepage.clone(),
                title: String::new(),
                version: 0,
                ancestors: vec![],
                position: None,
            });
        }
        let homepage_only = |page: &SpacePage| {
            listing.homepage_id.as_ref() == Some(&page.page_id)
                && !listing.pages.iter().any(|listed| listed.page_id == page.page_id)
        };
        // Parents first, so a new child's parent item exists when it is saved.
        pages.sort_by_key(|page| page.ancestors.len());
        operations::add_total(store, operation, pages.len() as u32)?;
        let mut cancelled = false;
        let mut unchanged = 0;
        let mut pending = Vec::new();
        let initial_index = {
            let _lock = store.shared()?;
            store.index()?
        };
        let excluded = initial_index
            .follows
            .iter()
            .find(|f| f.follow_id == follow.follow_id)
            .map(|f| f.excluded_ids.iter().cloned().collect::<BTreeSet<_>>())
            .unwrap_or_default();
        let mut existing = initial_index
            .items
            .into_iter()
            .map(|entry| (entry.summary.item_id.clone(), entry))
            .collect::<BTreeMap<_, _>>();
        for page in &pages {
            if excluded.contains(&page.page_id) {
                continue;
            }
            let id = page_item_id(&follow, &page.page_id);
            let old = existing.remove(&id);
            let reason = match &old {
                None => None,
                Some(_) if homepage_only(page) => {
                    unchanged += 1;
                    continue;
                }
                Some(old) => match change_reason(old, page) {
                    Some(reason) => Some(reason),
                    None if follow.include_attachments && old.summary.attachments.iter().any(|a| matches!(a.state, LibraryAttachmentState::NotDownloaded | LibraryAttachmentState::Failed)) => Some("attachments requested".into()),
                    None => {
                        unchanged += 1;
                        continue;
                    }
                },
            };
            pending.push((page.clone(), old, reason));
        }
        let space_key = refs::require_space_key(&follow)?.to_owned();
        let site = site_authority(&self.configuration, &follow.provider_id)?;
        let work = pending
            .into_iter()
            .map(|(page, old, reason)| FetchWork {
                item_id: page_item_id(&follow, &page.page_id),
                canonical_id: page.page_id.clone(),
                label: if page.title.is_empty() { page.page_id.clone() } else { page.title.clone() },
                request: SourceFetchRequest {
                    provider_id: follow.provider_id.clone(),
                    artifact_url: confluence_page_url(&site.provider_instance, &page.page_id),
                    authority: site.clone(),
                },
                container: Some(space_key.clone()),
                tag: (page, old, reason),
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
                let (page, old, reason) = &job.tag;
                self.save_follow_page_result(
                    store, operation, &follow, page, old.clone(), reason.clone(), result,
                ).await?;
            }
            drop(leases);
            if cancelled {
                break;
            }
        }
        operations::unchanged(store, operation, unchanged)?;
        cancelled = cancelled || operations::cancelled(store, operation)?;
        let present = pages.iter().map(|page| page.page_id.clone()).collect::<BTreeSet<_>>();
        if listing.complete && !cancelled {
            self.confirm_absent_pages(store, operation, &follow, &present).await?;
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
            match store.lease(&work.item_id) {
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
            jobs.spawn(async move {
                let result = async {
                    let fetched = sources.fetch_assets(request).await?;
                    let asset = fetched
                        .assets
                        .into_iter()
                        .find(|asset| asset.source.canonical_id == canonical_id)
                        .ok_or_else(|| error(
                            "source_identity_mismatch",
                            "Provider refresh omitted the requested item",
                        ))?;
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
                }.await;
                (index, result)
            });
            ordered.push(index);
        }
        let mut results = BTreeMap::new();
        while let Some(joined) = jobs.join_next().await {
            let (index, asset) = joined.map_err(|_| error(
                "source_provider_failed",
                "Provider fetch task failed",
            ))?;
            results.insert(index, asset);
        }
        // JoinSet completes in arbitrary order; hand results back in work order.
        let results = ordered
            .into_iter()
            .map(|index| {
                let result = results.remove(&index).unwrap_or_else(|| {
                    Err(error("source_provider_failed", "Provider fetch result is missing"))
                });
                (&batch[index], result)
            })
            .collect();
        Ok(Some(FetchedBatch { results, leases }))
    }

    async fn enumerate(
        &self,
        store: &Store,
        operation: &str,
        follow: &LibraryFollowSummary,
    ) -> Result<Option<crate::sources::SpacePageListing>, InspectionError> {
        let site = self.confluence_site(&follow.provider_id)?;
        if site.provider_instance != follow.provider_instance {
            return Err(error(
                "source_authority_mismatch",
                "Followed space belongs to a different configured site",
            ));
        }
        let space_key = refs::require_space_key(follow)?;
        let cancel = AtomicBool::new(false);
        let limit = self.configuration.limits.library_space_pages.max(1);
        let listing = {
            let listing =
                self.sources
                    .list_space_pages(&follow.provider_id, space_key, limit, &cancel);
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
            let title = if page.title.is_empty() { &page.page_id } else { &page.title };
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
            Ok(asset) if item_id(&asset.source) == id => self.save_asset_with(
                store, operation, asset, old.clone(),
                SaveOptions {
                    reference: Some(LibraryItemRef::Follow { follow_id: follow.follow_id.clone() }),
                    reason,
                    download_all: follow.include_attachments,
                    ..SaveOptions::default()
                },
            ).await,
            Ok(_) => Err(error("source_identity_mismatch", "Provider refresh returned a different page")),
            Err(failure) => Err(failure),
        };
        match (saved, old) {
            (Ok(()), _) => Ok(()),
            (Err(failure), Some(old)) => self.fetch_failed(store, operation, old, failure, false),
            (Err(failure), None) => failed(failure.message),
        }
    }


    /// D20: only a complete, non-partial run reaches here. A page absent from
    /// the enumeration is removed at source only when `Info` says it no longer
    /// exists or now lives in another space.
    async fn confirm_absent_pages(
        &self,
        store: &Arc<Store>,
        operation: &str,
        follow: &LibraryFollowSummary,
        present: &BTreeSet<String>,
    ) -> Result<(), InspectionError> {
        let space_key = refs::require_space_key(follow)?;
        let absent = {
            let _lock = store.shared()?;
            let index = store.index()?;
            let excluded = index
                .follows
                .iter()
                .find(|f| f.follow_id == follow.follow_id)
                .map(|f| f.excluded_ids.clone())
                .unwrap_or_default();
            index
                .items
                .into_iter()
                .filter(|entry| {
                    same_site_page(entry, follow)
                        && refs::has_follow(&entry.summary, &follow.follow_id)
                        && entry.summary.state != LibraryItemState::RemovedAtSource
                        && entry.summary.canonical_id.as_ref().is_some_and(|page_id| {
                            !present.contains(page_id) && !excluded.contains(page_id)
                        })
                })
                .map(|entry| entry.summary.item_id)
                .collect::<Vec<_>>()
        };
        for id in absent {
            if operations::cancelled(store, operation)? {
                break;
            }
            let Ok(_lease) = store.lease(&id) else { continue };
            let Some(mut entry) = self.entry(store, &id)? else { continue };
            let page_id = entry.summary.canonical_id.clone().unwrap_or_default();
            let (code, reason) = match self.sources.page_space(&follow.provider_id, &page_id).await
            {
                Ok(None) => ("source_not_found", "Not found at source".to_owned()),
                Err(failure) if failure.code == "source_not_found" => {
                    ("source_not_found", "Not found at source".to_owned())
                }
                Ok(Some(key)) if key != space_key => {
                    ("source_moved", format!("moved to {key}"))
                }
                // Still in this space but not yet in the search results.
                Ok(Some(_)) => continue,
                Err(failure) => {
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
        let mut ordered = listed.iter().collect::<Vec<_>>();
        ordered.sort_by(|a, b| {
            (a.position.is_none(), a.position, &a.title, &a.page_id)
                .cmp(&(b.position.is_none(), b.position, &b.title, &b.page_id))
        });
        let order = ordered
            .iter()
            .enumerate()
            .map(|(position, page)| (page.page_id.as_str(), (position as u32, *page)))
            .collect::<BTreeMap<_, _>>();
        store.mutate_index(|index| {
            let excluded = index
                .follows
                .iter()
                .find(|f| f.follow_id == follow.follow_id)
                .map(|f| f.excluded_ids.clone())
                .ok_or_else(|| error("library_item_not_found", "Followed space was removed"))?;
            let page_items = index
                .items
                .iter()
                .filter(|entry| same_site_page(entry, follow))
                .filter_map(|entry| {
                    Some((entry.summary.canonical_id.clone()?, entry.summary.item_id.clone()))
                })
                .collect::<BTreeMap<_, _>>();
            for entry in &mut index.items {
                if !same_site_page(entry, follow) {
                    continue;
                }
                let Some(page_id) = entry.summary.canonical_id.clone() else { continue };
                let Some((position, page)) = order.get(page_id.as_str()) else { continue };
                if excluded.contains(&page_id) {
                    continue;
                }
                refs::insert_ref(
                    &mut entry.summary,
                    LibraryItemRef::Follow { follow_id: follow.follow_id.clone() },
                );
                entry.summary.order = Some(*position);
                entry.summary.parent_item_id =
                    page.ancestors.last().and_then(|parent| page_items.get(parent).cloned());
            }
            recount(index, &follow.follow_id);
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

    /// D8: removing one item excludes it from every follow that lists it, so a
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
                    let Some((_, token)) = tokens.iter().find(|(follow_id, _)| follow_id == &follow.follow_id) else {
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
                        if let Some(follow) = index.follows.iter_mut().find(|f| &f.follow_id == follow_id) {
                            follow.excluded_ids.retain(|excluded| excluded != token);
                        }
                    }
                }
                Ok(())
            })?;
        }
        removed
    }

    /// D8. `Stop following` keeps every member as an ordinary item (an item left
    /// without references becomes Manual). `Remove` also deletes the members
    /// this follow alone holds, refusing when one was edited in the Library.
    pub(super) fn remove_follow(
        &self,
        follow_id: &str,
        delete_pages: bool,
    ) -> Result<(), InspectionError> {
        let store = self.open()?;
        let _lease = store.lease(follow_id)?;
        let own = LibraryItemRef::Follow { follow_id: follow_id.to_owned() };
        let exclusive = {
            let _lock = store.shared()?;
            let index = store.index()?;
            if !index.follows.iter().any(|f| f.follow_id == follow_id) {
                return Err(error("library_item_not_found", "Followed space does not exist"));
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
            let removed = store.remove_where(&page.summary.item_id, &page.summary.revision, |entry| {
                if entry.summary.refs.is_empty() {
                    Ok(())
                } else {
                    Err(error("library_item_referenced", "Library item gained a reference"))
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
mod tests {
    use super::super::{
        space::tests::{adapter, target},
        tests::{self as base, finished},
    };
    use super::*;
    use crate::sources::{
        ConfluencePage, FrontmatterField, FrontmatterValue, SourceAsset, SourceContainer,
        SourceProvider, SourceService, SpacePageListing, SpaceSummary,
    };
    use async_trait::async_trait;
    use cockpit_protocol::{projects::ProjectProvider, sources::SourceCapability};
    use std::sync::Mutex;

    #[derive(Clone)]
    struct Page {
        space: String,
        title: String,
        version: u64,
        /// Root first; Cloud folders appear here like pages.
        ancestors: Vec<(String, String)>,
        body: String,
    }
    struct Site {
        base_url: String,
        pages: BTreeMap<String, Page>,
        fetch_failures: BTreeSet<String>,
        fetched: Vec<String>,
        confirmed: Vec<String>,
    }
    struct FakeSpaces(Mutex<Site>);
    fn space_name(key: &str) -> &'static str {
        if key == "SD" { "Software Development" } else { "Operations" }
    }
    fn page(space: &str, title: &str, ancestors: &[(&str, &str)]) -> Page {
        Page {
            space: space.into(),
            title: title.into(),
            version: 1,
            ancestors: ancestors.iter().map(|(id, t)| ((*id).into(), (*t).into())).collect(),
            body: format!("{title} body"),
        }
    }
    impl FakeSpaces {
        /// SD: homepage H with A → A1 and X; a second top-level tree T → T1
        /// (under Cloud folder F). OPS: homepage O with P.
        fn new(base_url: &str, cloud: bool) -> Self {
            let folder: &[(&str, &str)] = if cloud { &[("900", "Folder F")] } else { &[] };
            let t = [folder, &[("200", "T")]].concat();
            let pages = [
                ("100", page("SD", "H", &[])),
                ("110", page("SD", "A", &[("100", "H")])),
                ("111", page("SD", "A1", &[("100", "H"), ("110", "A")])),
                ("112", page("SD", "X", &[("100", "H")])),
                ("200", page("SD", "T", folder)),
                ("201", page("SD", "T1", &t)),
                ("300", page("OPS", "O", &[])),
                ("301", page("OPS", "P", &[("300", "O")])),
            ];
            Self(Mutex::new(Site {
                base_url: base_url.into(),
                pages: pages.into_iter().map(|(id, page)| (id.to_owned(), page)).collect(),
                fetch_failures: BTreeSet::new(),
                fetched: vec![],
                confirmed: vec![],
            }))
        }
        fn site(&self) -> std::sync::MutexGuard<'_, Site> {
            self.0.lock().unwrap_or_else(|e| e.into_inner())
        }
        fn bump(&self, id: &str) {
            let mut site = self.site();
            let page = site.pages.get_mut(id).unwrap();
            page.version += 1;
            page.body = format!("{} body v{}", page.title, page.version);
        }
        fn take_fetched(&self) -> BTreeSet<String> {
            std::mem::take(&mut self.site().fetched).into_iter().collect()
        }
    }
    #[async_trait]
    impl SourceProvider for FakeSpaces {
        fn provider_id(&self) -> &str {
            "confluence"
        }
        fn capabilities(&self) -> Vec<SourceCapability> {
            vec![]
        }
        async fn resolve_input(&self, input: &str) -> Result<ProviderResolution, InspectionError> {
            let site = self.site();
            if let Some(page) = site.pages.get(input) {
                return Ok(ProviderResolution::ConfluencePage(ConfluencePage {
                    page_id: input.into(),
                    space_key: page.space.clone(),
                    title: page.title.clone(),
                    version: Some(page.version),
                    source_url: format!("{}/spaces/{}/pages/{input}", site.base_url, page.space),
                    canonical_url: confluence_page_url(&site.base_url, input),
                }));
            }
            match input {
                "SD" | "OPS" => Ok(ProviderResolution::ConfluenceSpace { space_key: input.into() }),
                _ => Err(error("source_not_found", "no such space")),
            }
        }
        async fn list_spaces(&self) -> Result<Vec<SpaceSummary>, InspectionError> {
            Ok(["SD", "OPS"]
                .map(|key| SpaceSummary { key: key.into(), name: space_name(key).into() })
                .into())
        }
        async fn list_space_pages(
            &self,
            space_key: &str,
            max_pages: u32,
            _cancel: &AtomicBool,
        ) -> Result<SpacePageListing, InspectionError> {
            let site = self.site();
            let all = site
                .pages
                .iter()
                .filter(|(_, page)| page.space == space_key)
                .map(|(id, page)| SpacePage {
                    page_id: id.clone(),
                    title: page.title.clone(),
                    version: page.version,
                    ancestors: page.ancestors.iter().map(|(id, _)| id.clone()).collect(),
                    position: None,
                })
                .collect::<Vec<_>>();
            Ok(SpacePageListing {
                space_name: space_name(space_key).into(),
                homepage_id: Some(if space_key == "SD" { "100" } else { "300" }.into()),
                total: Some(all.len() as u64),
                complete: all.len() <= max_pages as usize,
                pages: all.into_iter().take(max_pages as usize).collect(),
            })
        }
        async fn page_space(&self, page_id: &str) -> Result<Option<String>, InspectionError> {
            let mut site = self.site();
            site.confirmed.push(page_id.into());
            Ok(site.pages.get(page_id).map(|page| page.space.clone()))
        }
        async fn fetch(&self, request: &SourceFetchRequest) -> Result<Vec<SourceAsset>, InspectionError> {
            let id = request.artifact_url.split("pageId=").nth(1).unwrap().to_owned();
            let mut site = self.site();
            site.fetched.push(id.clone());
            if site.fetch_failures.contains(&id) {
                return Err(error("source_not_found", "gone during fetch"));
            }
            let page = site.pages.get(&id).cloned().ok_or_else(|| error("source_not_found", "gone"))?;
            let strings = |key: &str, values: Vec<String>| FrontmatterField {
                key: key.into(),
                value: FrontmatterValue::Strings(values),
            };
            let mut fields = vec![FrontmatterField {
                key: "page_id".into(),
                value: FrontmatterValue::String(id.clone()),
            }];
            if let Some((parent, _)) = page.ancestors.last() {
                fields.push(FrontmatterField {
                    key: "parent_id".into(),
                    value: FrontmatterValue::String(parent.clone()),
                });
                fields.push(strings("ancestors", page.ancestors.iter().map(|a| a.1.clone()).collect()));
                fields.push(strings("ancestor_ids", page.ancestors.iter().map(|a| a.0.clone()).collect()));
            }
            Ok(vec![SourceAsset {
                source: SourceRef {
                    provider_id: "confluence".into(),
                    provider_instance: request.authority.provider_instance.clone(),
                    resource_type: "page".into(),
                    canonical_id: id.clone(),
                },
                title: page.title.clone(),
                source_url: Some(format!("{}/spaces/{}/pages/{id}", site.base_url, page.space)),
                original_url: None,
                source_revision: Some(page.version.to_string()),
                complete: true,
                diagnostics: vec![],
                body: page.body.clone(),
                container: Some(SourceContainer {
                    id: page.space.clone(),
                    label: format!("{} · {}", page.space, space_name(&page.space)),
                }),
                fields,
                attachments: vec![],
            }])
        }
    }

    struct Fixture {
        base: base::Fixture,
        provider: Arc<FakeSpaces>,
    }
    const CLOUD: &str = "https://acme.atlassian.net/wiki";
    const DC: &str = "https://dc.example.test/confluence";
    fn fixture(base_url: &str, cloud: bool, page_limit: u32) -> Fixture {
        let mut base = base::fixture();
        let mut configuration = base.service.configuration.clone();
        configuration.providers = vec![ProjectProvider {
            id: "confluence".into(),
            base_url: base_url.into(),
            executable: "/usr/local/bin/confluence".into(),
            login: Some("read-only".into()),
        }];
        configuration.limits.library_space_pages = page_limit;
        let provider = Arc::new(FakeSpaces::new(base_url, cloud));
        let sources = Arc::new(SourceService::new(&configuration, vec![provider.clone()]).unwrap());
        base.service = LibraryService::new(configuration, sources);
        Fixture { base, provider }
    }
    fn follow_request(key: &str, target: Option<SpaceTarget>) -> LibraryAddRequest {
        LibraryAddRequest {
            input: key.into(),
            provider_id: Some("confluence".into()),
            reference_depth: 0,
            follow: true,
            follow_mode: None,
            download_attachments: false,
            refresh_existing: false,
            label: None,
            target,
        }
    }
    async fn follow(service: &LibraryService, key: &str) -> (LibraryOperation, String) {
        let operation = finished(service, service.start_add(follow_request(key, None)).await.unwrap()).await;
        assert_eq!(operation.phases[0].state, LibraryPhaseState::Done, "{operation:?}");
        let id = service
            .listing(None)
            .await
            .unwrap()
            .follows
            .into_iter()
            .find(|follow| refs::space_key(follow) == Some(key))
            .unwrap()
            .follow_id;
        (operation, id)
    }
    async fn refresh(service: &LibraryService, request: LibraryRefreshRequest) -> LibraryRefreshReport {
        let operation = finished(service, service.start_refresh(request).await.unwrap()).await;
        assert!(operation.phases.iter().all(|p| p.error.is_none()), "{operation:?}");
        operation.report.unwrap()
    }
    async fn pages(service: &LibraryService) -> BTreeMap<String, LibraryItemSummary> {
        let listing = service.listing(None).await.unwrap();
        listing
            .items
            .into_iter()
            .map(|item| (item.canonical_id.clone().unwrap(), item))
            .collect()
    }
    fn reason<'a>(report: &'a LibraryRefreshReport, item: &LibraryItemSummary) -> Option<&'a str> {
        report
            .rows
            .iter()
            .find(|row| row.item_id.as_ref() == Some(&item.item_id))
            .and_then(|row| row.reason.as_deref())
    }

    #[tokio::test]
    async fn refresh_enumerates_every_tree_fetches_only_changes_and_confirms_removals() {
        for (base_url, cloud) in [(CLOUD, true), (DC, false)] {
            let f = fixture(base_url, cloud, 200);
            let service = &f.base.service;
            let (added, follow_id) = follow(service, "SD").await;
            // Homepage tree and the second top-level tree, each page fetched once.
            assert_eq!(added.report.as_ref().unwrap().new, 6);
            assert_eq!(f.provider.take_fetched(), ["100", "110", "111", "112", "200", "201"].map(String::from).into());
            let listing = service.listing(None).await.unwrap();
            let record = &listing.follows[0];
            assert_eq!(record.follow_id, follow_id);
            assert_eq!((record.item_count, record.state), (6, LibraryItemState::Fresh));
            assert_eq!(refs::follow_title(record), "SD · Software Development");
            assert!(record.last_refreshed_at.is_some());
            let before = pages(service).await;
            assert!(before.values().all(|page| refs::has_follow(page, &follow_id)));
            let folder = before["200"].ancestors.iter().map(|a| (a.id.as_str(), a.title.as_str())).collect::<Vec<_>>();
            assert_eq!(folder, if cloud { vec![("900", "Folder F")] } else { vec![] });
            assert_eq!(before["200"].parent_item_id, None);
            assert_eq!(before["201"].parent_item_id.as_ref(), Some(&before["200"].item_id));
            assert_eq!(before["111"].parent_item_id.as_ref(), Some(&before["110"].item_id));

            {
                let mut site = f.provider.site();
                let mut n = site.pages["201"].clone();
                n.title = "N".into();
                n.body = "N body".into();
                site.pages.insert("202".into(), n);
                site.pages.get_mut("201").unwrap().ancestors = vec![("100".into(), "H".into()), ("110".into(), "A".into())];
                site.pages.remove("112");
            }
            f.provider.bump("111");
            let report = refresh(service, LibraryRefreshRequest::Follow { follow_id: follow_id.clone() }).await;
            assert_eq!((report.new, report.updated, report.removed_at_source, report.unchanged), (1, 2, 1, 3));
            assert_eq!(f.provider.take_fetched(), ["111", "201", "202"].map(String::from).into());
            assert_eq!(std::mem::take(&mut f.provider.site().confirmed), ["112"]);
            let after = pages(service).await;
            assert_eq!(reason(&report, &after["111"]), Some("changed"));
            assert_eq!(reason(&report, &after["201"]), Some("moved"));
            assert_eq!(after["201"].parent_item_id.as_ref(), Some(&after["110"].item_id));
            assert_eq!(after["201"].ancestors.iter().map(|a| a.id.as_str()).collect::<Vec<_>>(), ["100", "110"]);
            // A move re-renders the page and moves its directory and subtree.
            assert_ne!(after["201"].item_path, before["201"].item_path);
            assert_ne!(after["201"].revision, before["201"].revision);
            assert_eq!(after["202"].parent_item_id.as_ref(), Some(&after["200"].item_id));
            assert_eq!(after["112"].state, LibraryItemState::RemovedAtSource);
            assert_eq!(reason(&report, &after["112"]), Some("Not found at source"));
            assert_eq!(after["112"].item_path, before["112"].item_path);

            // A page moved to another space is removed at source with that reason.
            f.provider.site().pages.get_mut("110").unwrap().space = "OPS".into();
            f.provider.site().pages.get_mut("111").unwrap().space = "OPS".into();
            f.provider.site().pages.get_mut("201").unwrap().space = "OPS".into();
            let report = refresh(service, LibraryRefreshRequest::All).await;
            assert_eq!(report.removed_at_source, 3);
            let moved = pages(service).await;
            assert_eq!(reason(&report, &moved["110"]), Some("moved to OPS"));
            assert_eq!(moved["110"].state, LibraryItemState::RemovedAtSource);
            assert!(f.provider.take_fetched().is_empty());
            // A title change moves the page directory and every saved descendant with it.
            f.provider.site().pages.get_mut("200").unwrap().title = "T renamed".into();
            let report = refresh(service, LibraryRefreshRequest::All).await;
            assert_eq!(report.updated, 1);
            let renamed = pages(service).await;
            assert_eq!(renamed["200"].title, "T renamed");
            assert_ne!(renamed["200"].item_path, before["200"].item_path);
            assert!(renamed["202"].item_path.starts_with(&format!("{}/", renamed["200"].item_path)));
            assert!(renamed["202"].document_path.as_deref().unwrap()
                .starts_with(&format!("{}/", renamed["200"].item_path)));
            assert_eq!(f.provider.take_fetched(), ["200".to_owned()].into());
        }
    }

    #[tokio::test]
    async fn listed_page_fetch_not_found_stays_failed_until_absence_is_confirmed() {
        let f = fixture(DC, false, 200);
        let service = &f.base.service;
        let (_, follow_id) = follow(service, "SD").await;
        let mut site = f.provider.site();
        site.fetch_failures.insert("111".into());
        site.pages.get_mut("111").unwrap().version += 1;
        drop(site);

        let report = refresh(service, LibraryRefreshRequest::Follow { follow_id }).await;
        assert_eq!(report.failed, 1);
        assert_eq!(report.removed_at_source, 0);
        let page = pages(service).await.remove("111").unwrap();
        assert_eq!(page.state, LibraryItemState::Failed);
        assert_eq!(page.diagnostics[0].code, "source_not_found");
    }

    #[tokio::test]
    async fn page_limit_is_partial_and_never_marks_removals() {
        let f = fixture(DC, false, 3);
        let service = &f.base.service;
        let added = finished(service, service.start_add(follow_request("SD", None)).await.unwrap()).await;
        assert_eq!(added.phases[0].state, LibraryPhaseState::Partial);
        let follow_id = service.listing(None).await.unwrap().follows[0].follow_id.clone();
        let report = added.report.unwrap();
        assert_eq!((report.new, report.partial), (3, 1));
        assert!(report.rows.iter().any(|row| row.follow_id.as_ref() == Some(&follow_id)
            && row.outcome == LibraryReportOutcome::Partial
            && row.reason.as_deref() == Some("3 of 6 pages (page limit)")));
        let record = service.listing(None).await.unwrap().follows.remove(0);
        assert_eq!(record.state, LibraryItemState::Partial);
        let partial = record.partial.unwrap();
        assert_eq!((partial.have, partial.total), (3, Some(6)));
        assert_eq!(pages(service).await.len(), 3);
        // 110 disappears from the limited listing; only a complete run may remove it.
        f.provider.site().pages.remove("110");
        let report = refresh(service, LibraryRefreshRequest::Follow { follow_id }).await;
        assert_eq!((report.removed_at_source, report.partial), (0, 1));
        assert!(f.provider.site().confirmed.is_empty());
        assert_eq!(pages(service).await["110"].state, LibraryItemState::Fresh);
    }

    #[tokio::test]
    async fn removed_page_stays_excluded_until_follow_is_added_again_and_follow_removal_modes() {
        let f = fixture(CLOUD, true, 200);
        let service = &f.base.service;
        let (_, follow_id) = follow(service, "SD").await;
        let a1 = pages(service).await.remove("111").unwrap();
        service
            .remove(LibraryRemoveRequest::Item { item_id: a1.item_id.clone(), expected_revision: a1.revision.clone() })
            .await
            .unwrap();
        let record = service.listing(None).await.unwrap().follows.remove(0);
        assert_eq!(record.excluded_ids, ["111"]);
        assert_eq!(record.item_count, 5);
        f.provider.take_fetched();
        f.provider.bump("111");
        for _ in 0..2 {
            let report = refresh(service, LibraryRefreshRequest::Follow { follow_id: follow_id.clone() }).await;
            assert_eq!((report.new, report.removed_at_source), (0, 0));
            assert!(!pages(service).await.contains_key("111"));
        }
        assert!(f.provider.take_fetched().is_empty());
        // Following the whole space again clears exclusions (OQ4).
        let (again, same) = follow(service, "SD").await;
        assert_eq!(same, follow_id);
        assert_eq!(again.report.unwrap().new, 1);
        assert!(service.listing(None).await.unwrap().follows[0].excluded_ids.is_empty());
        assert!(refs::has_follow(&pages(service).await["111"], &follow_id));

        // Stop following keeps the pages as ordinary items.
        service.remove(LibraryRemoveRequest::StopFollowing { follow_id: follow_id.clone() }).await.unwrap();
        let listing = service.listing(None).await.unwrap();
        assert!(listing.follows.is_empty());
        assert_eq!(listing.items.len(), 6);
        assert!(listing.items.iter().all(|item| item.refs == [LibraryItemRef::Manual]));
        // Following again adopts them; they keep their Manual reference, so
        // removing the space forgets the record but keeps every page.
        let (again, _) = follow(service, "SD").await;
        assert_eq!(again.report.unwrap().new, 0);
        assert!(pages(service).await.values().all(|page| refs::has_follow(page, &follow_id)));
        let listing = service.remove(LibraryRemoveRequest::Follow { follow_id: follow_id.clone() }).await.unwrap();
        assert!(listing.follows.is_empty() && listing.items.len() == 6);
        assert!(listing.items.iter().all(|item| item.refs == [LibraryItemRef::Manual]));
        assert_eq!(
            service.remove(LibraryRemoveRequest::Follow { follow_id }).await.unwrap_err().code,
            "library_item_not_found"
        );
    }

    #[tokio::test]
    async fn remove_space_deletes_only_pages_the_follow_alone_holds() {
        let f = fixture(CLOUD, true, 200);
        let service = &f.base.service;
        let (_, follow_id) = follow(service, "SD").await;
        // Adding a followed page by hand gives it a second reference.
        let manual = LibraryAddRequest { follow: false, ..follow_request("200", None) };
        finished(service, service.start_add(manual).await.unwrap()).await;
        let kept = pages(service).await["200"].clone();
        assert_eq!(kept.refs.len(), 2);

        let listing = service.remove(LibraryRemoveRequest::Follow { follow_id }).await.unwrap();
        assert!(listing.follows.is_empty());
        assert_eq!(listing.items.len(), 1, "{:?}", listing.items.iter().map(|i| &i.title).collect::<Vec<_>>());
        assert_eq!(listing.items[0].item_id, kept.item_id);
        assert_eq!(listing.items[0].refs, [LibraryItemRef::Manual]);
        assert_eq!(listing.items[0].purge_after, None);
    }

    #[tokio::test]
    async fn resolves_and_browses_spaces_of_the_selected_provider() {
        let f = fixture(CLOUD, true, 200);
        let service = &f.base.service;
        let resolved = service
            .resolve(LibraryResolveRequest { input: "SD".into(), provider_id: Some("confluence".into()) })
            .await
            .unwrap();
        assert_eq!(resolved.kind, LibraryInputKind::ConfluenceSpace);
        assert_eq!(resolved.canonical_id.as_deref(), Some("SD"));
        assert_eq!(resolved.title, "Software Development");
        assert_eq!(resolved.container_label.as_deref(), Some("SD · Software Development"));
        assert_eq!((resolved.item_count, resolved.existing_follow_id), (Some(6), None));
        let (_, follow_id) = follow(service, "SD").await;
        let spaces = service.confluence_spaces("confluence").await.unwrap();
        assert_eq!(spaces.iter().map(|s| s.canonical_id.as_deref().unwrap()).collect::<Vec<_>>(), ["SD", "OPS"]);
        assert_eq!(spaces[0].existing_follow_id.as_ref(), Some(&follow_id));
        assert_eq!(spaces[0].item_count, Some(6));
        assert_eq!(spaces[1].existing_follow_id, None);
        assert!(spaces.iter().all(|s| s.provider_instance.as_deref() == Some(CLOUD)));
        let page = service
            .resolve(LibraryResolveRequest { input: "201".into(), provider_id: Some("confluence".into()) })
            .await
            .unwrap();
        assert_eq!(page.existing_follow_id.as_ref(), Some(&follow_id));
        assert_eq!(service.confluence_spaces("other").await.unwrap_err().code, "source_provider_unsupported");
        // Following through a page follows its space.
        let operation = finished(service, service.start_add(follow_request("301", None)).await.unwrap()).await;
        assert_eq!(operation.report.unwrap().new, 2);
        assert!(service.listing(None).await.unwrap().follows.iter().any(|f| refs::space_key(f) == Some("OPS")));
    }

    #[tokio::test]
    async fn follow_add_selects_only_items_saved_by_the_current_operation() {
        let f = fixture(CLOUD, true, 200);
        let service = f.base.service.clone().with_herdr(adapter("space"));
        let (_, follow_id) = follow(&service, "SD").await;
        let membership = pages(&service).await;
        assert_eq!(membership.len(), 6);
        f.provider.bump("110");

        let added = finished(
            &service,
            service.start_add(follow_request("SD", Some(target()))).await.unwrap(),
        ).await;
        let saved_ids = vec![membership["110"].item_id.clone()];
        assert!(added.phases.iter().all(|phase| phase.state == LibraryPhaseState::Done), "{added:?}");
        assert_eq!(added.item_ids, saved_ids);
        assert_eq!(added.space.unwrap().item_ids, saved_ids);
        let selected = service.space_listing(&target()).await.unwrap().items
            .into_iter().map(|item| item.item_id).collect::<Vec<_>>();
        assert_eq!(selected, saved_ids);
        let follow = service.listing(None).await.unwrap().follows
            .into_iter().find(|follow| follow.follow_id == follow_id).unwrap();
        assert_eq!(follow.item_count, 6, "selection leaves the global follow intact");
    }

}
