//! Durable, session-independent provider snapshots. Legacy source caches are inert.
mod operations;
mod reader;
pub(crate) mod store;
pub mod space;

use crate::{
    InspectionError,
    project_store::timestamp,
    repositories::resolve_artifact,
    sources::{
        FetchedAssets, SourceAsset, SourceFetchRequest, SourceRef, SourceService, content_revision,
        instance_authority,
    },
};
use cockpit_protocol::{
    library::*,
    projects::{ProjectConfiguration, ProjectDiagnostic},
};
use sha2::{Digest, Sha256};
use std::{
    path::Path,
    sync::{Arc, OnceLock},
};
use store::{Lease, LibraryIndexEntry, Store, error};

#[derive(Clone)]
pub struct LibraryService {
    configuration: ProjectConfiguration,
    sources: Arc<SourceService>,
    projects: Option<Arc<crate::projects::ProjectService>>,
    herdr: Option<Arc<dyn crate::HerdrAdapter>>,
    store: Arc<OnceLock<Arc<Store>>>,
}
impl LibraryService {
    pub fn new(configuration: ProjectConfiguration, sources: Arc<SourceService>) -> Self {
        Self {
            configuration,
            sources,
            projects: None,
            herdr: None,
            store: Arc::new(OnceLock::new()),
        }
    }
    pub fn with_projects(
        mut self,
        projects: Arc<crate::projects::ProjectService>,
        herdr: Arc<dyn crate::HerdrAdapter>,
    ) -> Self {
        self.projects = Some(projects);
        self.herdr = Some(herdr);
        self
    }
    /// Lazy and independent of ContextService, companions and the obsolete cache.
    pub(crate) fn open(&self) -> Result<Arc<Store>, InspectionError> {
        if let Some(store) = self.store.get() {
            store.recover_pending()?;
            space::recover_attempts(store)?;
            return Ok(store.clone());
        }
        let store = Store::open(
            Path::new(&self.configuration.library_root),
            self.configuration.limits.library_max_items as usize,
        )?;
        let _ = self.store.set(store);
        space::recover_attempts(self.store.get().expect("Library store initialized"))?;
        Ok(self.store.get().expect("Library store initialized").clone())
    }
    pub async fn listing(&self, offset: Option<u32>) -> Result<LibraryListing, InspectionError> {
        let store = self.open()?;
        let _lock = store.shared()?;
        let index = store.index()?;
        let offset = offset.unwrap_or(0) as usize;
        let end = offset.saturating_add(256).min(index.items.len());
        let items = index
            .items
            .iter()
            .skip(offset)
            .take(256)
            .map(|e| e.summary.clone())
            .collect();
        Ok(LibraryListing {
            root: self.authorized_root(&store)?.summary(),
            generation: index.generation,
            items,
            follows: index.follows,
            next_offset: (end < index.items.len()).then_some(end as u32),
            diagnostics: vec![],
        })
    }
    fn request(
        &self,
        input: &str,
        selected: Option<&str>,
    ) -> Result<SourceFetchRequest, InspectionError> {
        let artifact = resolve_artifact(&self.configuration, input)?;
        if selected.is_some_and(|id| id != artifact.provider_id) {
            return Err(error(
                "source_authority_mismatch",
                "URL does not belong to the selected provider",
            ));
        }
        let authority = instance_authority(
            &self.configuration,
            &artifact.provider_id,
            &artifact.canonical_url,
        )?;
        Ok(SourceFetchRequest {
            provider_id: artifact.provider_id,
            artifact_url: artifact.canonical_url,
            authority,
        })
    }
    pub async fn resolve(
        &self,
        request: LibraryResolveRequest,
    ) -> Result<LibraryResolution, InspectionError> {
        let store = self.open()?;
        let fetch = self.request(&request.input, request.provider_id.as_deref())?;
        let artifact = resolve_artifact(&self.configuration, &fetch.artifact_url)?;
        let source = SourceRef {
            provider_id: artifact.provider_id.clone(),
            provider_instance: fetch.authority.provider_instance.clone(),
            resource_type: artifact.kind,
            canonical_id: artifact.canonical_id.clone(),
        };
        let metadata = self.sources.metadata(fetch).await?;
        let _lock = store.shared()?;
        let existing = store
            .index()?
            .items
            .into_iter()
            .find(|e| e.summary.item_id == item_id(&source));
        Ok(LibraryResolution {
            kind: LibraryInputKind::Artifact,
            provider_id: Some(source.provider_id),
            provider_instance: Some(source.provider_instance),
            title: metadata.title,
            canonical_id: Some(artifact.canonical_id),
            container_label: existing
                .as_ref()
                .and_then(|e| e.summary.container.as_ref().map(|c| c.label.clone())),
            existing_item_id: existing.map(|e| e.summary.item_id),
            existing_follow_id: None,
            page_count: None,
            git_working_tree: None,
            file_count: None,
            diagnostics: vec![],
        })
    }
    pub async fn operation(&self, id: &str) -> Result<LibraryOperation, InspectionError> {
        operations::get(self.open()?.as_ref(), id)
    }
    pub async fn cancel(&self, id: &str) -> Result<LibraryOperation, InspectionError> {
        operations::cancel(self.open()?.as_ref(), id)
    }
    pub async fn start_add(
        &self,
        request: LibraryAddRequest,
    ) -> Result<LibraryOperation, InspectionError> {
        if request.follow_space || request.download_attachments {
            return Err(error(
                "source_capability_unavailable",
                "This provider snapshot operation does not support follows or downloads",
            ));
        }
        let handle = operations::runtime()?;
        let store = self.open()?;
        let fetch = self.request(&request.input, request.provider_id.as_deref())?;
        let artifact = resolve_artifact(&self.configuration, &fetch.artifact_url)?;
        let primary_id = item_id(&SourceRef {
            provider_id: fetch.provider_id.clone(),
            provider_instance: fetch.authority.provider_instance.clone(),
            resource_type: artifact.kind,
            canonical_id: artifact.canonical_id,
        });
        let lease = store.lease(&primary_id)?;
        let (record, operation_lease) =
            operations::create(&store, LibraryOperationKind::Add, None)?;
        let record = if let Some(target) = &request.target {
            operations::set_target(&store, &record.operation_id, target.clone())?
        } else { record };
        let service = self.clone();
        let id = record.operation_id.clone();
        let worker_store = store.clone();
        operations::spawn(handle, store, id.clone(), operation_lease, async move {
            let _lease = lease;
            if operations::cancelled(&worker_store, &id)? {
                return Ok(());
            }
            let fetched = service
                .sources
                .fetch_assets(fetch, request.hydrate_references)
                .await?;
            if request.hydrate_references {
                let incomplete = fetched
                    .assets
                    .iter()
                    .any(|asset| item_id(&asset.source) != primary_id && !asset.complete);
                let partial = incomplete
                    || fetched
                        .hydration
                        .as_ref()
                        .is_none_or(|h| h.failed > 0 || h.skipped > 0 || h.truncated);
                if partial {
                    let mut reason = fetched.hydration.as_ref().map(|h| format!("Linked hydration: {} completed, {} skipped, {} failed, truncated={}", h.completed, h.skipped, h.failed, h.truncated)).unwrap_or_else(|| "Linked hydration did not complete".into());
                    if incomplete {
                        reason.push_str("; linked content incomplete");
                    }
                    for diagnostic in fetched
                        .diagnostics
                        .iter()
                        .filter(|d| d.code.starts_with("source_hydration_"))
                        .take(32)
                    {
                        reason.push_str("; ");
                        reason.extend(diagnostic.code.chars().take(128));
                        reason.push_str(": ");
                        reason.extend(diagnostic.message.chars().take(512));
                        if let Some(path) = &diagnostic.path {
                            reason.push_str(" [");
                            reason.extend(path.chars().take(512));
                            reason.push(']');
                        }
                    }
                    operations::row(
                        &worker_store,
                        &id,
                        None,
                        LibraryReportOutcome::Partial,
                        Some(reason),
                    )?;
                }
            }
            for mut asset in fetched.assets {
                if operations::cancelled(&worker_store, &id)? {
                    break;
                }
                let asset_id = item_id(&asset.source);
                let _linked_lease = if asset_id != primary_id {
                    Some(worker_store.lease(&asset_id)?)
                } else {
                    None
                };
                if asset_id == primary_id {
                    asset.original_url = Some(request.input.clone());
                }
                let old = service.entry(&worker_store, &asset_id)?;
                if old.is_some() && !request.refresh_existing {
                    if let Some(target) = &request.target {
                        space::prepare_saved_item(&worker_store, &id, target, &old.as_ref().expect("existing item").summary)?;
                    }
                    operations::row(
                        &worker_store,
                        &id,
                        old.as_ref().map(|e| &e.summary),
                        LibraryReportOutcome::Unchanged,
                        Some("Already saved in Library".into()),
                    )?;
                    continue;
                }
                service.save_asset(&worker_store, &id, asset, old, None, request.target.as_ref())?;
            }
            if let Some(target) = &request.target {
                let saved = operations::get(&worker_store, &id)?.item_ids;
                if saved.is_empty() && operations::cancelled(&worker_store, &id)? {
                    return Ok(());
                }
                service.copy_saved_items(&worker_store, &id, target, &saved).await?;
            }
            Ok(())
        });
        Ok(record)
    }

    /// Setup commits every prevalidated artifact before starting any Space copy.
    /// `saved_ids` identifies durable items reused on a resumed setup; no provider
    /// request is performed here or while retrying their companion copies.
    pub(crate) async fn add_fetched_and_copy(
        &self,
        target: SpaceTarget,
        fetched: Vec<FetchedAssets>,
        saved_ids: Vec<String>,
    ) -> Result<LibraryOperation, InspectionError> {
        let handle = operations::runtime()?;
        let store = self.open()?;
        let mut assets = std::collections::BTreeMap::new();
        for fetched in fetched {
            for asset in fetched.assets {
                assets.entry(item_id(&asset.source)).or_insert(asset);
            }
        }
        let mut ids = saved_ids;
        ids.extend(assets.keys().cloned());
        ids.sort();
        ids.dedup();
        let leases = ids.iter().map(|id| store.lease(id)).collect::<Result<Vec<_>, _>>()?;
        for id in &ids {
            if !assets.contains_key(id) && self.entry(&store, id)?.is_none() {
                return Err(error("library_item_not_found", "Saved setup item no longer exists"));
            }
        }
        let (record, operation_lease) =
            operations::create(&store, LibraryOperationKind::Add, Some(ids.len() as u32))?;
        let record = operations::set_target(&store, &record.operation_id, target.clone())?;
        let service = self.clone();
        let operation_id = record.operation_id.clone();
        let worker_id = operation_id.clone();
        let worker_store = store.clone();
        operations::spawn(handle, store, operation_id.clone(), operation_lease, async move {
            let _leases = leases;
            for id in &ids {
                if operations::cancelled(&worker_store, &worker_id)? { return Ok(()); }
                let old = service.entry(&worker_store, id)?;
                if let Some(old) = old {
                    space::prepare_saved_item(&worker_store, &worker_id, &target, &old.summary)?;
                    operations::row(&worker_store, &worker_id, Some(&old.summary),
                        LibraryReportOutcome::Unchanged, Some("Already saved in Library".into()))?;
                } else {
                    let asset = assets.remove(id).ok_or_else(||
                        error("library_item_not_found", "Saved setup item no longer exists"))?;
                    service.save_asset(&worker_store, &worker_id, asset, None, None, Some(&target))?;
                }
            }
            // Every item and its pending attempt are now durable. Companion
            // authorization/publication cannot prevent another item being saved.
            service.copy_saved_items(&worker_store, &worker_id, &target, &ids).await
        });
        loop {
            let record = self.operation(&operation_id).await?;
            if record.finished { return Ok(record); }
            tokio::time::sleep(std::time::Duration::from_millis(25)).await;
        }
    }
    fn entry(&self, store: &Store, id: &str) -> Result<Option<LibraryIndexEntry>, InspectionError> {
        let _lock = store.shared()?;
        Ok(store
            .index()?
            .items
            .into_iter()
            .find(|e| e.summary.item_id == id))
    }
    fn select(
        &self,
        store: &Store,
        request: LibraryRefreshRequest,
    ) -> Result<Vec<LibraryIndexEntry>, InspectionError> {
        let _lock = store.shared()?;
        let index = store.index()?;
        Ok(match request {
            LibraryRefreshRequest::All => index.items,
            LibraryRefreshRequest::Items { item_ids } => {
                let mut selected = vec![];
                for id in item_ids {
                    if selected
                        .iter()
                        .any(|e: &LibraryIndexEntry| e.summary.item_id == id)
                    {
                        continue;
                    }
                    selected.push(
                        index
                            .items
                            .iter()
                            .find(|e| e.summary.item_id == id)
                            .cloned()
                            .ok_or_else(|| {
                                error(
                                    "library_item_not_found",
                                    "Library refresh item does not exist",
                                )
                            })?,
                    );
                }
                selected
            }
            LibraryRefreshRequest::Container {
                provider_instance,
                container_id,
            } => index
                .items
                .into_iter()
                .filter(|e| {
                    e.summary.provider_instance.as_deref() == Some(&provider_instance)
                        && e.summary
                            .container
                            .as_ref()
                            .is_some_and(|c| c.container_id == container_id)
                })
                .collect(),
            LibraryRefreshRequest::Follow { .. } => {
                return Err(error(
                    "source_capability_unavailable",
                    "Follow refresh is not available for this provider",
                ));
            }
        })
    }
    pub async fn start_refresh(
        &self,
        request: LibraryRefreshRequest,
    ) -> Result<LibraryOperation, InspectionError> {
        let handle = operations::runtime()?;
        let store = self.open()?;
        let entries = self.select(&store, request)?;
        let leases: Vec<Lease> = entries
            .iter()
            .map(|e| store.lease(&e.summary.item_id))
            .collect::<Result<_, _>>()?;
        let (record, operation_lease) = operations::create(
            &store,
            LibraryOperationKind::Refresh,
            Some(entries.len() as u32),
        )?;
        let service = self.clone();
        let id = record.operation_id.clone();
        let worker_store = store.clone();
        operations::spawn(handle, store, id.clone(), operation_lease, async move {
            let _leases = leases;
            for entry in entries {
                if operations::cancelled(&worker_store, &id)? {
                    break;
                }
                service.refresh_one(&worker_store, &id, entry, None).await?;
            }
            Ok(())
        });
        Ok(record)
    }
    pub async fn start_replace(
        &self,
        request: LibraryReplaceRequest,
    ) -> Result<LibraryOperation, InspectionError> {
        let handle = operations::runtime()?;
        let store = self.open()?;
        let lease = store.lease(&request.item_id)?;
        let entry = self
            .entry(&store, &request.item_id)?
            .ok_or_else(|| error("library_item_not_found", "Library item does not exist"))?;
        {
            let _lock = store.shared()?;
            store.check_confirmation(&entry, Some(&request.confirmed))?;
        }
        let (record, operation_lease) =
            operations::create(&store, LibraryOperationKind::Refresh, Some(1))?;
        let service = self.clone();
        let id = record.operation_id.clone();
        let worker_store = store.clone();
        operations::spawn(handle, store, id.clone(), operation_lease, async move {
            let _lease = lease;
            if !operations::cancelled(&worker_store, &id)? {
                service
                    .refresh_one(&worker_store, &id, entry, Some(request.confirmed))
                    .await?;
            }
            Ok(())
        });
        Ok(record)
    }
    async fn refresh_one(
        &self,
        store: &Arc<Store>,
        operation: &str,
        mut entry: LibraryIndexEntry,
        confirmed: Option<Vec<LibraryConflictFile>>,
    ) -> Result<(), InspectionError> {
        if confirmed.is_none() {
            let conflicts = {
                let _lock = store.shared()?;
                store.conflicts(&entry)?
            };
            if !conflicts.is_empty() {
                entry.summary.state = LibraryItemState::Conflict;
                entry.summary.conflict = conflicts;
                store.update(entry.clone())?;
                return operations::row(
                    store,
                    operation,
                    Some(&entry.summary),
                    LibraryReportOutcome::Conflict,
                    None,
                );
            }
        }
        let result = async {
            let url = entry.canonical_url.as_deref().ok_or_else(|| {
                error(
                    "source_capability_unavailable",
                    "Item has no refreshable provider origin",
                )
            })?;
            let request = self.request(url, entry.summary.provider_id.as_deref())?;
            self.sources.fetch_assets(request, false).await
        }
        .await;
        if operations::cancelled(store, operation)? {
            return Ok(());
        }
        match result {
            Ok(fetched) => {
                let asset = fetched
                    .assets
                    .into_iter()
                    .find(|a| item_id(&a.source) == entry.summary.item_id);
                if let Some(asset) = asset {
                    match self.save_asset(
                        store,
                        operation,
                        asset,
                        Some(entry.clone()),
                        confirmed.as_deref(),
                        None,
                    ) {
                        Ok(()) => Ok(()),
                        Err(e) => self.fetch_failed(store, operation, entry, e),
                    }
                } else {
                    self.fetch_failed(
                        store,
                        operation,
                        entry,
                        error(
                            "source_identity_mismatch",
                            "Provider refresh omitted the requested item",
                        ),
                    )
                }
            }
            Err(e) => self.fetch_failed(store, operation, entry, e),
        }
    }
    fn fetch_failed(
        &self,
        store: &Store,
        operation: &str,
        mut entry: LibraryIndexEntry,
        e: InspectionError,
    ) -> Result<(), InspectionError> {
        let removed = e.code == "source_not_found";
        entry.summary.state = if removed {
            LibraryItemState::RemovedAtSource
        } else {
            LibraryItemState::Failed
        };
        entry.summary.checked_at = Some(timestamp());
        entry.summary.diagnostics = vec![ProjectDiagnostic {
            code: e.code,
            message: e.message.clone(),
            path: None,
        }];
        store.update(entry.clone())?;
        operations::row(
            store,
            operation,
            Some(&entry.summary),
            if removed {
                LibraryReportOutcome::RemovedAtSource
            } else {
                LibraryReportOutcome::Failed
            },
            Some(e.message),
        )
    }
    fn save_asset(
        &self,
        store: &Arc<Store>,
        operation: &str,
        mut asset: SourceAsset,
        old: Option<LibraryIndexEntry>,
        confirmed: Option<&[LibraryConflictFile]>,
        target: Option<&SpaceTarget>,
    ) -> Result<(), InspectionError> {
        if let Some(old) = &old {
            asset.original_url = old.summary.original_url.clone();
        }
        let canonical_url = asset.source_url.as_deref().ok_or_else(|| {
            error(
                "source_identity_mismatch",
                "Provider asset has no validated canonical URL",
            )
        })?;
        let canonical = resolve_artifact(&self.configuration, canonical_url)?;
        if canonical.provider_id != asset.source.provider_id
            || canonical.kind != asset.source.resource_type
            || canonical.canonical_id != asset.source.canonical_id
        {
            return Err(error(
                "source_identity_mismatch",
                "Provider canonical URL identifies a different item",
            ));
        }
        self.request(canonical_url, Some(&asset.source.provider_id))?;
        let mut entry = asset_entry(&asset, old.as_ref());
        entry.canonical_url = Some(canonical.canonical_url);
        if let Some(old) = &old {
            let check = {
                let _lock = store.shared()?;
                store.check_confirmation(old, confirmed)
            };
            if let Err(e) = check {
                if e.code != "library_conflict" {
                    return Err(e);
                }
                return self.record_conflict(store, operation, old.clone(), e.message);
            }
        }
        let equal = old
            .as_ref()
            .is_some_and(|e| e.summary.revision == entry.summary.revision);
        if !asset.complete || asset.source_revision.is_none() {
            entry.summary.state = LibraryItemState::Unknown;
        } else if old.is_some() && !equal {
            entry.summary.state = LibraryItemState::Changed;
        }
        if equal && confirmed.is_none() {
            // Preserve the snapshot timestamp: provenance refreshes do not rewrite files.
            entry.summary.fetched_at = old.as_ref().and_then(|e| e.summary.fetched_at.clone());
            if let Some(target) = target {
                space::prepare_saved_item(store, operation, target, &entry.summary)?;
            }
            store.update(entry.clone())?;
        } else {
            let stage = store.stage_asset(&mut entry, &asset)?;
            if operations::cancelled(store, operation)? {
                return Ok(());
            }
            if let Some(target) = target {
                space::prepare_saved_item(store, operation, target, &entry.summary)?;
            }
            if let Err(e) = store.publish(
                stage,
                entry.clone(),
                old.as_ref().map(|e| e.summary.revision.as_str()),
                confirmed,
            ) {
                if e.code == "library_conflict" {
                    if let Some(old) = old {
                        return self.record_conflict(store, operation, old, e.message);
                    }
                }
                return Err(e);
            }
        }
        #[cfg(test)] {
            let mut fault = store.fault.lock().unwrap_or_else(|e| e.into_inner());
            if *fault == Some("space_after_library_publish") {
                *fault = None;
                return Err(error("library_test_crash", "space_after_library_publish"));
            }
            if *fault == Some("space_cancel_after_library_publish") {
                *fault = None;
                operations::cancel(store, operation)?;
            }
        }
        operations::row(
            store,
            operation,
            Some(&entry.summary),
            if old.is_none() {
                LibraryReportOutcome::New
            } else if equal {
                LibraryReportOutcome::Unchanged
            } else {
                LibraryReportOutcome::Updated
            },
            None,
        )
    }
    fn record_conflict(
        &self,
        store: &Store,
        operation: &str,
        mut old: LibraryIndexEntry,
        reason: String,
    ) -> Result<(), InspectionError> {
        old.summary.conflict = {
            let _lock = store.shared()?;
            store.conflicts(&old)?
        };
        old.summary.state = LibraryItemState::Conflict;
        old.summary.checked_at = Some(timestamp());
        store.update(old.clone())?;
        operations::row(
            store,
            operation,
            Some(&old.summary),
            LibraryReportOutcome::Conflict,
            Some(reason),
        )
    }
    pub async fn remove(
        &self,
        request: LibraryRemoveRequest,
    ) -> Result<LibraryListing, InspectionError> {
        match request {
            LibraryRemoveRequest::Item {
                item_id,
                expected_revision,
            } => self.open()?.remove(&item_id, &expected_revision)?,
            _ => {
                return Err(error(
                    "source_capability_unavailable",
                    "Follow removal is not available for this provider",
                ));
            }
        }
        self.listing(None).await
    }
}
fn item_id(source: &SourceRef) -> String {
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
fn asset_entry(asset: &SourceAsset, old: Option<&LibraryIndexEntry>) -> LibraryIndexEntry {
    let id = item_id(&asset.source);
    let revision = content_revision(asset);
    let now = timestamp();
    let slug: String = asset
        .title
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() {
                c.to_ascii_lowercase()
            } else {
                '-'
            }
        })
        .take(60)
        .collect();
    let path = old
        .map(|e| e.summary.item_path.clone())
        .unwrap_or_else(|| format!("{}-{}", slug.trim_matches('-'), &id[7..]));
    LibraryIndexEntry {
        inventory: old.map(|e| e.inventory.clone()).unwrap_or_default(),
        marker_hash: old.and_then(|e| e.marker_hash.clone()),
        canonical_url: asset.source_url.clone(),
        summary: LibraryItemSummary {
            item_id: id,
            logical_id: format!(
                "source:{}:{}:{}:{}",
                asset.source.provider_id,
                asset.source.provider_instance,
                asset.source.resource_type,
                asset.source.canonical_id
            ),
            kind: LibraryItemKind::ProviderSnapshot,
            provider_id: Some(asset.source.provider_id.clone()),
            provider_instance: Some(asset.source.provider_instance.clone()),
            resource_type: Some(asset.source.resource_type.clone()),
            canonical_id: Some(asset.source.canonical_id.clone()),
            container: asset.container.as_ref().map(|c| LibraryContainer {
                container_id: c.id.clone(),
                label: c.label.clone(),
            }),
            parent_item_id: None,
            ancestors: vec![],
            order: old.and_then(|e| e.summary.order),
            title: asset.title.clone(),
            document_path: Some(format!("{path}/document.md")),
            item_path: path,
            source_url: asset.source_url.clone(),
            original_url: asset.original_url.clone(),
            source_revision: asset.source_revision.clone(),
            revision,
            state: LibraryItemState::Fresh,
            partial: None,
            conflict: vec![],
            fetched_at: Some(now.clone()),
            checked_at: Some(now),
            follow_id: old.and_then(|e| e.summary.follow_id.clone()),
            attachments: asset
                .attachments
                .iter()
                .map(|a| LibraryAttachment {
                    attachment_id: a.id.clone(),
                    original_name: a.title.clone(),
                    stored_name: a.title.clone(),
                    media_type: a.media_type.clone(),
                    bytes: a.size,
                    version: a.source_revision.clone(),
                    state: LibraryAttachmentState::NotDownloaded,
                    relative_path: None,
                })
                .collect(),
            folder: None,
            diagnostics: asset.diagnostics.clone(),
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sources::{SourceContainer, SourceMetadata, SourceProvider};
    use async_trait::async_trait;
    use cockpit_protocol::{
        projects::{ProjectLimits, ProjectProvider},
        sources::SourceCapability,
    };
    use std::{
        collections::BTreeMap,
        sync::{
            Mutex,
            atomic::{AtomicBool, Ordering},
        },
        time::Duration,
    };
    use tokio::sync::{Notify, Semaphore};
    use uuid::Uuid;

    pub(super) struct Fixture {
        pub root: std::path::PathBuf,
        pub service: LibraryService,
        pub provider: Arc<Provider>,
    }
    impl Drop for Fixture {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.root);
        }
    }
    #[derive(Clone)]
    struct ProviderState {
        body: String,
        revision: Option<String>,
        container: String,
        diagnostic: bool,
        suffix: String,
        complete: bool,
        failure: Option<String>,
        fail_linked: bool,
    }
    pub(super) struct Provider {
        state: Mutex<ProviderState>,
        block: AtomicBool,
        entered: Notify,
        release: Semaphore,
        pub(super) fetches: std::sync::atomic::AtomicUsize,
    }
    impl Provider {
        pub(super) fn set_body(&self, body: &str) {
            self.state.lock().unwrap_or_else(|e| e.into_inner()).body = body.into();
        }
        pub(super) fn set_failure(&self, failure: Option<&str>) {
            self.state.lock().unwrap_or_else(|e| e.into_inner()).failure = failure.map(str::to_owned);
        }
    }
    #[async_trait]
    impl SourceProvider for Provider {
        fn provider_id(&self) -> &str {
            "tea"
        }
        fn capabilities(&self) -> Vec<SourceCapability> {
            vec![SourceCapability::Issue]
        }
        async fn metadata(
            &self,
            _: &SourceFetchRequest,
        ) -> Result<SourceMetadata, InspectionError> {
            Ok(SourceMetadata {
                title: "Issue title".into(),
                source_branch: None,
                source_url: None,
                source_commit: None,
                description: None,
            })
        }
        async fn fetch(
            &self,
            request: &SourceFetchRequest,
        ) -> Result<Vec<SourceAsset>, InspectionError> {
            self.fetches.fetch_add(1, Ordering::SeqCst);
            if self.block.load(Ordering::SeqCst) {
                self.entered.notify_one();
                self.release
                    .acquire()
                    .await
                    .expect("release provider")
                    .forget();
            }
            let state = self.state.lock().unwrap_or_else(|e| e.into_inner()).clone();
            if let Some(code) = state.failure {
                return Err(error(&code, "fixture provider failure"));
            }
            let n: u32 = request
                .artifact_url
                .split('?')
                .next()
                .unwrap()
                .rsplit('/')
                .next()
                .unwrap()
                .parse()
                .unwrap();
            if n != 1 && state.fail_linked {
                return Err(error("source_unavailable", "linked issue unavailable"));
            }
            let mut asset = asset(n, &state.body);
            asset.source_revision = state.revision;
            asset.source_url = Some(format!("{}{}", asset.source_url.unwrap(), state.suffix));
            asset.container = Some(SourceContainer {
                id: state.container.clone(),
                label: state.container,
            });
            asset.complete = state.complete;
            if state.diagnostic {
                asset.diagnostics.push(ProjectDiagnostic {
                    code: "source_markup_unconverted".into(),
                    message: "unconverted markup".into(),
                    path: None,
                });
            }
            Ok(vec![asset])
        }
    }
    pub(super) fn asset(n: u32, body: &str) -> SourceAsset {
        SourceAsset {
            source: SourceRef {
                provider_id: "tea".into(),
                provider_instance: "https://forge.test/gitea".into(),
                resource_type: "issue".into(),
                canonical_id: format!("acme/repo#{n}"),
            },
            title: format!("Issue {n}"),
            source_url: Some(format!("https://forge.test/gitea/acme/repo/issues/{n}")),
            original_url: None,
            source_revision: Some("1".into()),
            complete: true,
            diagnostics: vec![],
            body: body.into(),
            container: None,
            fields: vec![],
            attachments: vec![],
        }
    }
    pub(super) fn fixture() -> Fixture {
        let root = std::env::temp_dir().join(format!("cockpit-library-{}", Uuid::new_v4()));
        std::fs::create_dir_all(&root).unwrap();
        let configuration = ProjectConfiguration {
            version: 1,
            repository_roots: vec![],
            worktree_root: root.join("worktrees").to_string_lossy().into_owned(),
            companion_root: root.join("companions").to_string_lossy().into_owned(),
            state_root: root.join("state").to_string_lossy().into_owned(),
            library_root: root.join("library").to_string_lossy().into_owned(),
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
                catalog_entries: 100,
                git_timeout_ms: 1000,
                git_output_bytes: 65536,
                operation_timeout_ms: 10000,
                context_preview_bytes: 65536,
                context_preview_lines: 1000,
                context_directory_entries: 256,
                context_tree_depth: 16,
                library_folder_files: 512,
                library_folder_bytes: 32 * 1024 * 1024,
                library_file_bytes: 4 * 1024 * 1024,
                library_space_pages: 200,
                library_attachment_bytes: 25 * 1024 * 1024,
                library_item_attachment_bytes: 100 * 1024 * 1024,
                library_max_items: 20000,
            },
            origins: BTreeMap::new(),
        };
        let provider = Arc::new(Provider {
            state: Mutex::new(ProviderState {
                body: "Original body".into(),
                revision: Some("1".into()),
                container: "acme/repo".into(),
                diagnostic: false,
                suffix: String::new(),
                complete: true,
                failure: None,
                fail_linked: false,
            }),
            block: AtomicBool::new(false),
            entered: Notify::new(),
            release: Semaphore::new(0),
            fetches: std::sync::atomic::AtomicUsize::new(0),
        });
        let sources = Arc::new(SourceService::new(&configuration, vec![provider.clone()]).unwrap());
        Fixture {
            root,
            service: LibraryService::new(configuration, sources),
            provider,
        }
    }
    pub(super) fn add(n: u32) -> LibraryAddRequest {
        LibraryAddRequest {
            input: format!("https://forge.test/gitea/acme/repo/issues/{n}"),
            provider_id: None,
            hydrate_references: false,
            follow_space: false,
            download_attachments: false,
            refresh_existing: false,
            label: None,
            target: None,
        }
    }
    pub(super) async fn finished(service: &LibraryService, operation: LibraryOperation) -> LibraryOperation {
        tokio::time::timeout(Duration::from_secs(10), async {
            loop {
                let current = service.operation(&operation.operation_id).await.unwrap();
                if current.finished {
                    return current;
                }
                tokio::time::sleep(Duration::from_millis(1)).await;
            }
        })
        .await
        .expect("operation terminates")
    }
    pub(super) async fn saved(f: &Fixture, n: u32) -> LibraryItemSummary {
        let operation = finished(&f.service, f.service.start_add(add(n)).await.unwrap()).await;
        assert_eq!(
            operation.phases[0].state,
            LibraryPhaseState::Done,
            "{operation:?}"
        );
        f.service
            .listing(None)
            .await
            .unwrap()
            .items
            .into_iter()
            .find(|i| i.canonical_id.as_deref() == Some(&format!("acme/repo#{n}")))
            .unwrap()
    }
    async fn refresh(f: &Fixture, item: &LibraryItemSummary) -> LibraryOperation {
        finished(
            &f.service,
            f.service
                .start_refresh(LibraryRefreshRequest::Items {
                    item_ids: vec![item.item_id.clone()],
                })
                .await
                .unwrap(),
        )
        .await
    }
    pub(super) fn reopen(f: &Fixture) -> LibraryService {
        LibraryService::new(f.service.configuration.clone(), f.service.sources.clone())
    }
    pub(super) fn assert_store_valid(store: &Store) {
        let _lock = store.shared().unwrap();
        for entry in store.index().unwrap().items {
            assert!(store.root.open_dir(&entry.summary.item_path).is_ok());
            assert!(
                store.conflicts(&entry).unwrap().is_empty(),
                "{}",
                entry.summary.item_id
            );
        }
    }

    #[tokio::test]
    async fn seventy_items_survive_listing_and_reopen_without_eviction() {
        let f = fixture();
        let mut ids = Vec::new();
        for n in 1..=70 {
            ids.push(saved(&f, n).await.item_id);
        }
        let listing = reopen(&f).listing(None).await.unwrap();
        assert_eq!(
            listing
                .items
                .iter()
                .map(|i| i.item_id.clone())
                .collect::<Vec<_>>(),
            ids
        );
        assert_eq!(listing.next_offset, None);
        assert_store_valid(&f.service.open().unwrap());
    }
    #[tokio::test]
    async fn edited_document_conflicts_and_stale_replace_hash_is_rejected() {
        let f = fixture();
        let item = saved(&f, 1).await;
        let path = Path::new(&f.service.configuration.library_root)
            .join(item.document_path.as_ref().unwrap());
        std::fs::write(&path, b"My edited document").unwrap();
        let operation = refresh(&f, &item).await;
        assert_eq!(operation.report.unwrap().conflict, 1);
        let conflict = f.service.listing(None).await.unwrap().items.remove(0);
        assert_eq!(conflict.state, LibraryItemState::Conflict);
        assert_eq!(std::fs::read(&path).unwrap(), b"My edited document");
        std::fs::write(&path, b"A later edit").unwrap();
        let e = f
            .service
            .start_replace(LibraryReplaceRequest {
                item_id: item.item_id.clone(),
                confirmed: conflict.conflict,
            })
            .await
            .unwrap_err();
        assert_eq!(e.code, "library_conflict");
        assert_eq!(std::fs::read(&path).unwrap(), b"A later edit");
        refresh(&f, &item).await;
        let confirmed = f
            .service
            .listing(None)
            .await
            .unwrap()
            .items
            .remove(0)
            .conflict;
        let replaced = finished(
            &f.service,
            f.service
                .start_replace(LibraryReplaceRequest {
                    item_id: item.item_id,
                    confirmed,
                })
                .await
                .unwrap(),
        )
        .await;
        assert_eq!(replaced.phases[0].state, LibraryPhaseState::Done);
        assert!(
            std::fs::read_to_string(&path)
                .unwrap()
                .contains("Original body")
        );
        assert_store_valid(&f.service.open().unwrap());
    }
    #[tokio::test]
    async fn provenance_only_refresh_is_fresh_and_does_not_publish() {
        let f = fixture();
        let item = saved(&f, 1).await;
        let marker = Path::new(&f.service.configuration.library_root)
            .join(&item.item_path)
            .join(".cockpit-item.json");
        let before = std::fs::read(&marker).unwrap();
        let modified = std::fs::metadata(&marker).unwrap().modified().unwrap();
        {
            let mut state = f.provider.state.lock().unwrap_or_else(|e| e.into_inner());
            state.container = "renamed/repo".into();
            state.diagnostic = true;
            state.suffix = "?verified=1".into();
        }
        assert_eq!(refresh(&f, &item).await.report.unwrap().unchanged, 1);
        let updated = f.service.listing(None).await.unwrap().items.remove(0);
        assert_eq!(updated.state, LibraryItemState::Fresh);
        assert_eq!(updated.revision, item.revision);
        assert_eq!(updated.container.unwrap().container_id, "renamed/repo");
        assert_eq!(updated.diagnostics[0].code, "source_markup_unconverted");
        assert!(updated.source_url.unwrap().ends_with("?verified=1"));
        assert_eq!(updated.original_url, item.original_url);
        assert_eq!(std::fs::read(&marker).unwrap(), before);
        assert_eq!(
            std::fs::metadata(&marker).unwrap().modified().unwrap(),
            modified
        );
        // The next refresh uses the stored API-validated canonical URL.
        assert_eq!(
            refresh(&f, &item).await.phases[0].state,
            LibraryPhaseState::Done
        );
    }
    #[tokio::test]
    async fn refresh_states_retain_content_on_provider_errors() {
        let f = fixture();
        let item = saved(&f, 1).await;
        {
            let mut state = f.provider.state.lock().unwrap_or_else(|e| e.into_inner());
            state.body = "Changed body".into();
            state.revision = Some("2".into());
        }
        refresh(&f, &item).await;
        let changed = f.service.listing(None).await.unwrap().items.remove(0);
        assert_eq!(changed.state, LibraryItemState::Changed);
        assert_ne!(changed.revision, item.revision);
        refresh(&f, &changed).await;
        assert_eq!(
            f.service.listing(None).await.unwrap().items[0].state,
            LibraryItemState::Fresh
        );
        let document = Path::new(&f.service.configuration.library_root)
            .join(changed.document_path.as_ref().unwrap());
        let retained = std::fs::read(&document).unwrap();
        for (code, expected) in [
            ("source_not_found", LibraryItemState::RemovedAtSource),
            ("source_auth_failed", LibraryItemState::Failed),
        ] {
            f.provider
                .state
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .failure = Some(code.into());
            refresh(&f, &changed).await;
            let current = f.service.listing(None).await.unwrap().items.remove(0);
            assert_eq!(current.state, expected);
            assert_eq!(current.diagnostics[0].code, code);
            assert_eq!(std::fs::read(&document).unwrap(), retained);
        }
        {
            let mut state = f.provider.state.lock().unwrap_or_else(|e| e.into_inner());
            state.failure = None;
            state.revision = None;
        }
        refresh(&f, &changed).await;
        assert_eq!(
            f.service.listing(None).await.unwrap().items[0].state,
            LibraryItemState::Unknown
        );
        assert_store_valid(&f.service.open().unwrap());
    }
    pub(super) fn linked_add(f: &Fixture) -> LibraryAddRequest {
        f.provider.state.lock().unwrap_or_else(|e| e.into_inner()).body =
            "See https://forge.test/gitea/acme/repo/issues/2".into();
        let mut request = add(1);
        request.hydrate_references = true;
        request
    }

    #[tokio::test]
    async fn later_linked_lease_failure_retains_saved_primary_attempt() {
        let f = fixture();
        let store = f.service.open().unwrap();
        let _linked_lease = store.lease(&item_id(&asset(2, "").source)).unwrap();
        let target = SpaceTarget { session_id: "session".into(), space_id: "space".into() };
        let mut request = linked_add(&f);
        request.target = Some(target.clone());
        let operation = finished(&f.service, f.service.start_add(request).await.unwrap()).await;
        assert_eq!(operation.phases[0].error.as_ref().unwrap().code, "library_item_busy");
        let reopened = reopen(&f);
        let saved = reopened.listing(None).await.unwrap().items;
        assert_eq!(saved.iter().map(|item| item.canonical_id.as_deref()).collect::<Vec<_>>(), vec![Some("acme/repo#1")]);
        let attempts = reopened.space_listing(target).await.unwrap().attempts;
        assert_eq!(attempts.len(), 1);
        assert_eq!(attempts[0].item_id.as_ref(), Some(&saved[0].item_id));
        assert_eq!(attempts[0].state, SpaceAddAttemptState::Failed);
        assert_eq!(attempts[0].error.as_ref().unwrap().code, "library_item_busy");
    }

    #[tokio::test]
    async fn crash_after_library_publish_before_report_recovers_retry_intent() {
        let f = fixture();
        let store = f.service.open().unwrap();
        let target = SpaceTarget { session_id: "session".into(), space_id: "space".into() };
        let (operation, lease) = operations::create(&store, LibraryOperationKind::Add, None).unwrap();
        operations::set_target(&store, &operation.operation_id, target.clone()).unwrap();
        *store.fault.lock().unwrap_or_else(|e| e.into_inner()) = Some("space_after_library_publish");
        let failure = f.service.save_asset(&store, &operation.operation_id, asset(1, "published"),
            None, None, Some(&target)).unwrap_err();
        assert_eq!(failure.code, "library_test_crash");
        assert!(operations::get(&store, &operation.operation_id).unwrap().item_ids.is_empty());
        drop(lease);
        let reopened = reopen(&f);
        let saved = reopened.listing(None).await.unwrap().items;
        assert_eq!(saved[0].canonical_id.as_deref(), Some("acme/repo#1"));
        let attempts = reopened.space_listing(target).await.unwrap().attempts;
        assert_eq!(attempts.len(), 1);
        assert_eq!(attempts[0].item_id.as_ref(), Some(&saved[0].item_id));
        assert_eq!(attempts[0].state, SpaceAddAttemptState::Failed);
        assert_eq!(attempts[0].error.as_ref().unwrap().code, "space_add_interrupted");
    }

    #[tokio::test]
    async fn interrupted_write_ahead_attempt_without_library_publish_is_discarded() {
        let f = fixture();
        let store = f.service.open().unwrap();
        let target = SpaceTarget { session_id: "session".into(), space_id: "space".into() };
        let (operation, lease) = operations::create(&store, LibraryOperationKind::Add, None).unwrap();
        operations::set_target(&store, &operation.operation_id, target.clone()).unwrap();
        let entry = asset_entry(&asset(1, "not published"), None);
        space::prepare_saved_item(&store, &operation.operation_id, &target, &entry.summary).unwrap();
        drop(lease);
        let reopened = reopen(&f);
        assert!(reopened.listing(None).await.unwrap().items.is_empty());
        assert!(reopened.space_listing(target).await.unwrap().attempts.is_empty());
    }

    #[tokio::test]
    async fn linked_fetch_failure_persists_partial_report_with_omitted_path() {
        let f = fixture();
        {
            let mut state = f.provider.state.lock().unwrap_or_else(|e| e.into_inner());
            state.body = "See https://forge.test/gitea/acme/repo/issues/2".into();
            state.fail_linked = true;
        }
        let mut request = add(1);
        request.hydrate_references = true;
        let operation = finished(&f.service, f.service.start_add(request).await.unwrap()).await;
        assert_eq!(operation.phases[0].state, LibraryPhaseState::Partial);
        let report = operation.report.as_ref().unwrap();
        assert_eq!(report.new, 1);
        assert_eq!(report.partial, 1);
        let reason = report
            .rows
            .iter()
            .find(|r| r.outcome == LibraryReportOutcome::Partial)
            .unwrap()
            .reason
            .as_deref()
            .unwrap();
        assert!(reason.contains("source_hydration_fetch_failed"));
        assert!(reason.contains("linked issue unavailable"));
        assert!(reason.contains("https://forge.test/gitea/acme/repo/issues/2"));
        let listing = f.service.listing(None).await.unwrap();
        assert_eq!(
            listing
                .items
                .iter()
                .map(|i| i.canonical_id.as_deref().unwrap())
                .collect::<Vec<_>>(),
            vec!["acme/repo#1"]
        );
        let reopened = reopen(&f).operation(&operation.operation_id).await.unwrap();
        assert_eq!(
            serde_json::to_value(reopened).unwrap(),
            serde_json::to_value(operation).unwrap()
        );
    }
    #[tokio::test]
    async fn markup_diagnostic_alone_does_not_make_hydration_partial() {
        let f = fixture();
        f.provider
            .state
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .diagnostic = true;
        let mut request = add(1);
        request.hydrate_references = true;
        let operation = finished(&f.service, f.service.start_add(request).await.unwrap()).await;
        assert_eq!(operation.phases[0].state, LibraryPhaseState::Done);
        assert_eq!(operation.report.unwrap().partial, 0);
        assert_eq!(
            f.service.listing(None).await.unwrap().items[0].diagnostics[0].code,
            "source_markup_unconverted"
        );
    }

    #[tokio::test]
    async fn target_add_cancelled_before_publish_does_not_start_space_phase() {
        let f = fixture();
        f.provider.block.store(true, Ordering::SeqCst);
        let mut request = add(1);
        request.target = Some(SpaceTarget { session_id: "session".into(), space_id: "space".into() });
        let operation = f.service.start_add(request).await.unwrap();
        f.provider.entered.notified().await;
        f.service.cancel(&operation.operation_id).await.unwrap();
        f.provider.release.add_permits(1);
        let operation = finished(&f.service, operation).await;
        assert_eq!(operation.phases[0].state, LibraryPhaseState::Cancelled);
        assert_eq!(operation.phases[1].state, LibraryPhaseState::Cancelled);
        assert!(f.service.listing(None).await.unwrap().items.is_empty());
        assert!(f.service.space_listing(SpaceTarget { session_id: "session".into(), space_id: "space".into() })
            .await.unwrap().attempts.is_empty());
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn concurrent_refresh_is_busy_across_hosts_and_cancel_keeps_snapshot() {
        let f = fixture();
        let item = saved(&f, 1).await;
        f.provider.block.store(true, Ordering::SeqCst);
        let first = f
            .service
            .start_refresh(LibraryRefreshRequest::All)
            .await
            .unwrap();
        f.provider.entered.notified().await;
        let live = reopen(&f).operation(&first.operation_id).await.unwrap();
        assert!(!live.finished);
        assert_eq!(live.phases[0].state, LibraryPhaseState::Running);
        let e = reopen(&f)
            .start_refresh(LibraryRefreshRequest::Items {
                item_ids: vec![item.item_id.clone()],
            })
            .await
            .unwrap_err();
        assert_eq!(e.code, "library_item_busy");
        let reading = f
            .service
            .document(LibraryDocumentRequest {
                path: item.document_path.clone().unwrap(),
                expected_revision: None,
                offset: None,
            })
            .await
            .unwrap();
        assert!(reading.text.unwrap().contains("Original body"));
        let cancelled = reopen(&f).cancel(&first.operation_id).await.unwrap();
        assert!(!cancelled.finished);
        assert!(cancelled.cancel_requested);
        f.provider.release.add_permits(1);
        assert_eq!(
            finished(&f.service, first).await.phases[0].state,
            LibraryPhaseState::Cancelled
        );
        assert_eq!(
            f.service.listing(None).await.unwrap().items[0].revision,
            item.revision
        );
        assert_store_valid(&f.service.open().unwrap());
    }
    #[tokio::test]
    async fn library_open_never_imports_or_mutates_legacy_or_companion_files() {
        let f = fixture();
        let legacy = Path::new(&f.service.configuration.state_root).join("sources");
        let companion = Path::new(&f.service.configuration.companion_root).join("existing");
        std::fs::create_dir_all(&legacy).unwrap();
        std::fs::create_dir_all(&companion).unwrap();
        let files = [
            (legacy.join("index.json"), b"legacy opaque index".as_slice()),
            (legacy.join("record.md"), b"legacy source".as_slice()),
            (
                companion.join("context-manifest.json"),
                b"existing manifest without Library links".as_slice(),
            ),
            (
                companion.join("context.md"),
                b"Space-owned context".as_slice(),
            ),
        ];
        for (path, bytes) in &files {
            std::fs::write(path, bytes).unwrap();
        }
        assert!(f.service.listing(None).await.unwrap().items.is_empty());
        assert!(reopen(&f).listing(None).await.unwrap().items.is_empty());
        for (path, bytes) in &files {
            assert_eq!(&std::fs::read(path).unwrap(), bytes);
        }
        // Explicit re-add does not consult or rewrite any legacy/companion file either.
        saved(&f, 1).await;
        for (path, bytes) in files {
            assert_eq!(std::fs::read(path).unwrap(), bytes);
        }
    }
    #[tokio::test]
    async fn library_reader_hides_metadata_and_honors_revision_and_depth() {
        let f = fixture();
        let item = saved(&f, 1).await;
        let root = f
            .service
            .directory(LibraryDirectoryRequest {
                path: String::new(),
                offset: None,
                revision: None,
            })
            .await
            .unwrap();
        assert!(root.entries.iter().all(|e| e.name != ".cockpit"));
        assert_eq!(root.binding_id, "library");
        assert!(root.root_id.starts_with("library:"));
        let directory = f
            .service
            .directory(LibraryDirectoryRequest {
                path: item.item_path.clone(),
                offset: None,
                revision: None,
            })
            .await
            .unwrap();
        assert_eq!(
            directory
                .entries
                .iter()
                .map(|e| e.name.as_str())
                .collect::<Vec<_>>(),
            vec!["document.md"]
        );
        let e = f
            .service
            .document(LibraryDocumentRequest {
                path: format!("{}/.cockpit-item.json", item.item_path),
                expected_revision: None,
                offset: None,
            })
            .await
            .unwrap_err();
        assert_eq!(e.code, "context_reserved_path");
        let e = f
            .service
            .document(LibraryDocumentRequest {
                path: item.document_path.unwrap(),
                expected_revision: Some("stale".into()),
                offset: None,
            })
            .await
            .unwrap_err();
        assert_eq!(e.code, "context_stale_revision");
    }
    #[tokio::test]
    async fn remove_is_revision_cas_and_does_not_delete_an_edited_snapshot() {
        let f = fixture();
        let item = saved(&f, 1).await;
        let e = f
            .service
            .remove(LibraryRemoveRequest::Item {
                item_id: item.item_id.clone(),
                expected_revision: "stale".into(),
            })
            .await
            .unwrap_err();
        assert_eq!(e.code, "library_conflict");
        let listing = f
            .service
            .remove(LibraryRemoveRequest::Item {
                item_id: item.item_id,
                expected_revision: item.revision,
            })
            .await
            .unwrap();
        assert!(listing.items.is_empty());
        assert!(
            !Path::new(&f.service.configuration.library_root)
                .join(item.item_path)
                .exists()
        );
        assert!(reopen(&f).listing(None).await.unwrap().items.is_empty());
    }
    #[test]
    fn constructor_requires_neither_runtime_nor_usable_root() {
        let f = fixture();
        std::fs::write(&f.service.configuration.library_root, b"not a directory").unwrap();
        let service = reopen(&f);
        assert_eq!(service.open().err().unwrap().code, "library_unavailable");
        assert_eq!(
            operations::runtime().unwrap_err().code,
            "library_unavailable"
        );
    }
}
