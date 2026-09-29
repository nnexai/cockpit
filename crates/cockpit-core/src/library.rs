//! Durable, session-independent provider snapshots. Legacy source caches are inert.
mod attachments;
mod folder;
mod follow;
mod jira_follow;
mod operations;
mod related;
pub(crate) mod refs;
mod layout;
mod reader;
pub(crate) mod store;
pub mod space;

use crate::{
    InspectionError,
    project_store::timestamp,
    repositories::{confluence_provider_for_input, resolve_artifact},
    sources::{
        ConfluencePage, FetchedAssets, FrontmatterValue, IssueRow, MAX_REFERENCE_DEPTH,
        ProviderResolution, ReferenceSeed, SourceAsset, SourceFetchRequest, SourceRef,
        SourceService, asset_label, asset_references, confluence_instance_authority,
        confluence_page_id, confluence_page_url, content_revision, instance_authority,
        site_authority,
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
/// The listing page of `entries` from `offset`, with each Jira issue's
/// `parent_item_id` set to the Library item of its parent issue.
///
/// The parent key is the stored `parent` reference of the issue's document.
/// It is resolved against the whole index (never only the page) within the
/// issue's own provider instance. This is a listing projection: the stored
/// `parent_item_id` of a Jira issue stays empty because it also decides the
/// on-disk layout, and Jira issues keep the flat `<PROJECT>/<KEY>/<Title>.md`
/// path. An issue whose parent is not in the Library, or whose stored copy
/// predates reference capture, has no parent.
fn jira_parent_projection(
    entries: &[LibraryIndexEntry],
    offset: usize,
    limit: usize,
) -> Vec<LibraryItemSummary> {
    let is_issue = |summary: &LibraryItemSummary| {
        summary.kind == LibraryItemKind::ProviderSnapshot
            && summary.resource_type.as_deref() == Some("issue")
    };
    let mut issues: std::collections::HashMap<(&str, &str, &str), &str> =
        std::collections::HashMap::new();
    for entry in entries {
        let summary = &entry.summary;
        if let (true, Some(provider), Some(instance), Some(key)) = (
            is_issue(summary),
            summary.provider_id.as_deref(),
            summary.provider_instance.as_deref(),
            summary.canonical_id.as_deref(),
        ) {
            issues.insert((provider, instance, key), summary.item_id.as_str());
        }
    }
    entries
        .iter()
        .skip(offset)
        .take(limit)
        .map(|entry| {
            let mut summary = entry.summary.clone();
            if summary.parent_item_id.is_none()
                && let (Some(provider), Some(instance), Some(references)) = (
                    summary.provider_id.as_deref(),
                    summary.provider_instance.as_deref(),
                    entry.references.as_deref(),
                )
                && let Some(key) = references.iter().find_map(|reference| {
                    match (&reference.target, reference.relation.as_str()) {
                        (
                            crate::sources::ReferenceTarget::JiraKey {
                                provider_id,
                                key,
                            },
                            "parent",
                        ) if provider_id == provider => Some(key.as_str()),
                        _ => None,
                    }
                })
                && let Some(parent) = issues.get(&(provider, instance, key))
                && *parent != summary.item_id
            {
                summary.parent_item_id = Some((*parent).to_owned());
            }
            summary
        })
        .collect()
}
fn confluence_input_matches_instance(base_url: &str, input: &str) -> bool {
    let (Ok(base), Ok(input)) = (url::Url::parse(base_url), url::Url::parse(input)) else {
        return false;
    };
    let base_path = base.path().trim_end_matches('/');
    let input_path = input.path();
    input.username().is_empty()
        && input.password().is_none()
        && base.scheme() == input.scheme()
        && base.host_str().map(str::to_ascii_lowercase)
            == input.host_str().map(str::to_ascii_lowercase)
        && base.port_or_known_default() == input.port_or_known_default()
        && (base_path.is_empty()
            || input_path == base_path
            || input_path.starts_with(&format!("{base_path}/")))
}
fn looks_like_confluence_url(input: &str) -> bool {
    let Ok(url) = url::Url::parse(input) else {
        return false;
    };
    let path = url.path();
    path.contains("/spaces/")
        || path.contains("/display/")
        || path.ends_with("/pages/viewpage.action")
        || url.query_pairs().any(|(key, _)| key == "pageId")
}
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
    /// Configured global root used by agents to traverse durable Library items.
    pub fn root_path(&self) -> &str {
        &self.configuration.library_root
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
        let service = self.clone();
        tokio::task::spawn_blocking(move || {
            let store = service.open()?;
            let _lock = store.shared()?;
            let index = store.index_shared()?;
            let offset = offset.unwrap_or(0) as usize;
            let end = offset.saturating_add(256).min(index.items.len());
            let items = jira_parent_projection(&index.items, offset, 256);
            Ok(LibraryListing {
                root: service.authorized_root(&store)?.summary(),
                generation: index.generation.clone(),
                items,
                follows: index.follows.clone(),
                next_offset: (end < index.items.len()).then_some(end as u32),
                diagnostics: vec![],
            })
        })
        .await
        .map_err(|error| InspectionError::new("library_unavailable", error.to_string()))?
    }
    fn is_jira_provider(&self, provider_id: &str) -> bool {
        self.configuration.providers.iter().any(|provider| {
            provider.id == provider_id && crate::repositories::is_jira_executable(&provider.executable)
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
    /// The selected Confluence provider's recognition of `input`, or `None`
    /// when the input belongs to forge/Jira resolution.
    async fn confluence_resolution(
        &self,
        input: &str,
        selected: Option<&str>,
    ) -> Result<Option<(String, ProviderResolution)>, InspectionError> {
        let Some(provider_id) =
            confluence_provider_for_input(&self.configuration, input, selected)
        else {
            let page_id = !input.trim().is_empty()
                && input.trim().bytes().all(|byte| byte.is_ascii_digit());
            if page_id
                && (selected.is_some()
                    || self.configuration.providers.iter().any(|provider| {
                        crate::repositories::is_confluence_executable(&provider.executable)
                    }))
            {
                return Err(error(
                    "source_authority_mismatch",
                    "Select a configured Confluence provider for this page id",
                ));
            }
            if self.configuration.providers.iter().any(|provider| {
                crate::repositories::is_confluence_executable(&provider.executable)
                    && confluence_input_matches_instance(&provider.base_url, input)
            }) {
                return Err(error(
                    "source_authority_mismatch",
                    "Confluence URL requires an unambiguous selected provider",
                ));
            }
            if looks_like_confluence_url(input) {
                return Err(error(
                    "source_authority_mismatch",
                    "Select the configured Confluence provider for this URL",
                ));
            }
            return Ok(None);
        };
        let resolution = self.sources.resolve_input(&provider_id, input).await?;
        Ok(Some((provider_id, resolution)))
    }
    async fn confluence_request(
        &self,
        input: &str,
        selected: Option<&str>,
    ) -> Result<Option<(ConfluencePage, SourceFetchRequest)>, InspectionError> {
        let Some((provider_id, resolution)) = self.confluence_resolution(input, selected).await?
        else {
            return Ok(None);
        };
        let page = match resolution {
            ProviderResolution::ConfluencePage(page) => page,
            ProviderResolution::ConfluenceSpace { .. } => {
                return Err(error(
                    "source_capability_unavailable",
                    "A Confluence space is added by following it",
                ));
            }
        };
        self.confluence_page_request(provider_id, page).map(Some)
    }
    fn confluence_page_request(
        &self,
        provider_id: String,
        page: ConfluencePage,
    ) -> Result<(ConfluencePage, SourceFetchRequest), InspectionError> {
        let authority = confluence_instance_authority(
            &self.configuration,
            &provider_id,
            &page,
            &page.canonical_url,
        )?;
        Ok((
            page.clone(),
            SourceFetchRequest {
                provider_id,
                artifact_url: confluence_page_url(&authority.provider_instance, &page.page_id),
                authority,
            },
        ))
    }
    pub async fn resolve(
        &self,
        request: LibraryResolveRequest,
    ) -> Result<LibraryResolution, InspectionError> {
        if folder::recognizes(&request.input) {
            return self.resolve_folder(&request.input).await;
        }
        if let Some((query, provider_id)) =
            self.jira_query(&request.input, request.provider_id.as_deref())?
        {
            return self.resolve_jira_query(&provider_id, &query).await;
        }
        let page_request = match self
            .confluence_resolution(&request.input, request.provider_id.as_deref())
            .await?
        {
            Some((provider_id, ProviderResolution::ConfluenceSpace { space_key })) => {
                return self.resolve_space(&provider_id, &space_key).await;
            }
            Some((provider_id, ProviderResolution::ConfluencePage(page))) => {
                Some(self.confluence_page_request(provider_id, page)?)
            }
            None => None,
        };
        if let Some((page, fetch)) = page_request {
            let fetched = self.sources.fetch_assets(fetch.clone()).await?;
            let asset = fetched
                .assets
                .into_iter()
                .find(|asset| {
                    asset.source.provider_id == fetch.provider_id
                        && asset.source.resource_type == "page"
                        && asset.source.canonical_id == page.page_id
                })
                .ok_or_else(|| {
                    error(
                        "source_identity_mismatch",
                        "Confluence provider omitted the requested page",
                    )
                })?;
            let asset_page = ConfluencePage {
                page_id: asset.source.canonical_id.clone(),
                space_key: asset
                    .container
                    .as_ref()
                    .map(|container| container.id.clone())
                    .unwrap_or_default(),
                title: asset.title.clone(),
                version: asset.source_revision.as_deref().and_then(|version| version.parse().ok()),
                source_url: asset.source_url.clone().ok_or_else(|| {
                    error("source_identity_mismatch", "Confluence page has no validated source URL")
                })?,
                canonical_url: page.canonical_url.clone(),
            };
            let authority = confluence_instance_authority(
                &self.configuration,
                &fetch.provider_id,
                &asset_page,
                &page.canonical_url,
            )?;
            if asset.source.provider_instance != authority.provider_instance {
                return Err(error(
                    "source_identity_mismatch",
                    "Confluence page belongs to a different configured site",
                ));
            }
            let source = asset.source;
            let store = self.open()?;
            let _lock = store.shared()?;
            let index = store.index()?;
            let existing = index
                .items
                .iter()
                .find(|entry| entry.summary.item_id == item_id(&source));
            let existing_follow_id = index
                .follows
                .iter()
                .find(|follow| {
                    follow.provider_id == source.provider_id
                        && follow.provider_instance == source.provider_instance
                        && refs::space_key(follow) == Some(page.space_key.as_str())
                })
                .map(|follow| follow.follow_id.clone());
            return Ok(LibraryResolution {
                kind: LibraryInputKind::ConfluencePage,
                provider_id: Some(source.provider_id),
                provider_instance: Some(source.provider_instance),
                title: asset.title,
                canonical_id: Some(page.page_id),
                container_label: asset.container.map(|container| container.label),
                existing_item_id: existing.map(|entry| entry.summary.item_id.clone()),
                existing_follow_id,
                item_count: None,
                item_count_exact: true,
                follow_mode: None,
                git_working_tree: None,
                file_count: None,
                diagnostics: vec![],
                reference_depth: None,
            });
        }
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
        let reference_depth = existing.as_ref().map(|e| e.summary.reference_depth.unwrap_or(0));
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
            item_count: None,
            item_count_exact: true,
            follow_mode: None,
            git_working_tree: None,
            file_count: None,
            diagnostics: vec![],
            reference_depth,
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
        if request.reference_depth > MAX_REFERENCE_DEPTH {
            return Err(error(
                "library_reference_depth_invalid",
                format!("Reference depth must be 0 to {MAX_REFERENCE_DEPTH}"),
            ));
        }
        if !folder::recognizes(&request.input) {
            if let Some((query, provider_id)) =
                self.jira_query(&request.input, request.provider_id.as_deref())?
            {
                if !request.follow {
                    return Err(error(
                        "source_capability_unavailable",
                        "A Jira query is added by following it",
                    ));
                }
                return self.start_jira_follow_add(request, query, provider_id).await;
            }
        }
        if request.follow {
            return self.start_follow_add(request).await;
        }
        if folder::recognizes(&request.input) {
            if request.download_attachments {
                return Err(error("source_capability_unavailable", "Folders do not support attachment downloads"));
            }
            return self.start_folder_add(request).await;
        }
        let page_request = self
            .confluence_request(&request.input, request.provider_id.as_deref())
            .await?;
        if let Some((_, fetch)) = page_request.as_ref().filter(|_| request.download_attachments) {
            self.sources.attachment_downloads(&fetch.provider_id, "page").await?;
        }
        if page_request.is_some() && request.reference_depth > 0 {
            return Err(error(
                "source_capability_unavailable",
                "Confluence page imports do not follow related items",
            ));
        }
        let handle = operations::runtime()?;
        let store = self.open()?;
        let (fetch, primary_id) = if let Some((page, fetch)) = page_request {
            let primary_id = item_id(&SourceRef {
                provider_id: fetch.provider_id.clone(),
                provider_instance: fetch.authority.provider_instance.clone(),
                resource_type: "page".into(),
                canonical_id: page.page_id,
            });
            (fetch, primary_id)
        } else {
            let fetch = self.request(&request.input, request.provider_id.as_deref())?;
            let artifact = resolve_artifact(&self.configuration, &fetch.artifact_url)?;
            if request.download_attachments {
                self.sources.attachment_downloads(&fetch.provider_id, &artifact.kind).await?;
            }
            let primary_id = item_id(&SourceRef {
                provider_id: fetch.provider_id.clone(),
                provider_instance: fetch.authority.provider_instance.clone(),
                resource_type: artifact.kind,
                canonical_id: artifact.canonical_id,
            });
            (fetch, primary_id)
        };
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
            let fetched = service.sources.fetch_assets(fetch).await?;
            // Re-adding an existing item keeps its stored depth unless the add
            // refreshes it, so `Keep in Library` never resets the policy.
            let apply_depth = request.refresh_existing
                || service.entry(&worker_store, &primary_id)?.is_none();
            let mut seed = None;
            if request.reference_depth > 0 {
                // An add starts with an unknown total; the seed is its first unit.
                operations::add_total(&worker_store, &id, 1)?;
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
                    if request.reference_depth > 0 {
                        seed = Some(ReferenceSeed {
                            source: asset.source.clone(),
                            label: asset_label(&asset),
                            references: asset_references(&service.configuration, &asset),
                        });
                    }
                }
                let old = service.entry(&worker_store, &asset_id)?;
                if old.is_some() && !request.refresh_existing && !request.download_attachments {
                    let saved = old.as_ref().expect("existing item");
                    if !saved.summary.refs.contains(&LibraryItemRef::Manual) {
                        worker_store.add_ref(&asset_id, LibraryItemRef::Manual)?;
                    }
                    if let Some(target) = &request.target {
                        space::prepare_saved_item(&worker_store, &id, target, &saved.summary)?;
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
                service.save_asset_with(&worker_store, &id, asset, old, SaveOptions {
                    target: request.target.as_ref(),
                    reference: Some(LibraryItemRef::Manual),
                    download_all: request.download_attachments && asset_id == primary_id,
                    ..SaveOptions::default()
                }).await?;
            }
            if apply_depth && !operations::cancelled(&worker_store, &id)? {
                service
                    .apply_reference_depth(
                        &worker_store,
                        &id,
                        &primary_id,
                        seed,
                        request.reference_depth,
                        request.target.as_ref(),
                    )
                    .await?;
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
                    service.save_asset_with(&worker_store, &worker_id, asset, None, SaveOptions {
                        target: Some(&target),
                        reference: Some(LibraryItemRef::Manual),
                        ..SaveOptions::default()
                    }).await?;
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
    /// Items refreshed one by one, and follows refreshed by D20 enumeration.
    /// `All` refreshes each follow once and every item outside a follow.
    fn select(
        &self,
        store: &Store,
        request: LibraryRefreshRequest,
    ) -> Result<(Vec<LibraryIndexEntry>, Vec<LibraryFollowSummary>), InspectionError> {
        let _lock = store.shared()?;
        let index = store.index()?;
        Ok((match request {
            LibraryRefreshRequest::All => {
                let followed = |entry: &LibraryIndexEntry| {
                    entry.summary.refs.iter().any(|reference| {
                        matches!(reference, LibraryItemRef::Follow { follow_id }
                            if index.follows.iter().any(|follow| &follow.follow_id == follow_id))
                    })
                };
                // A tombstoned item (no references) awaits purge, not a refresh.
                let items = index
                    .items
                    .iter()
                    .filter(|e| !followed(e) && !e.summary.refs.is_empty())
                    .cloned()
                    .collect();
                return Ok((items, index.follows));
            }
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
            } => {
                // A Jira query is its own container in the tree: its id is the follow id.
                if let Some(follow) = index.follows.iter().find(|follow| {
                    follow.follow_id == container_id && follow.provider_instance == provider_instance
                }) {
                    return Ok((vec![], vec![follow.clone()]));
                }
                index
                    .items
                    .into_iter()
                    .filter(|e| {
                        e.summary.provider_instance.as_deref() == Some(&provider_instance)
                            && e.summary
                                .container
                                .as_ref()
                                .is_some_and(|c| c.container_id == container_id)
                    })
                    .collect()
            }
            LibraryRefreshRequest::Follow { follow_id } => {
                let follow = index
                    .follows
                    .into_iter()
                    .find(|follow| follow.follow_id == follow_id)
                    .ok_or_else(|| {
                        error("library_item_not_found", "Followed space does not exist")
                    })?;
                return Ok((vec![], vec![follow]));
            }
        }, vec![]))
    }
    pub async fn start_refresh(
        &self,
        request: LibraryRefreshRequest,
    ) -> Result<LibraryOperation, InspectionError> {
        let handle = operations::runtime()?;
        let store = self.open()?;
        let (entries, follows) = self.select(&store, request)?;
        let leases: Vec<Lease> = entries
            .iter()
            .map(|e| e.summary.item_id.as_str())
            .chain(follows.iter().map(|f| f.follow_id.as_str()))
            .map(|id| store.lease(id))
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
            for follow in follows {
                if operations::cancelled(&worker_store, &id)? {
                    break;
                }
                service.refresh_follow(&worker_store, &id, follow, false, None).await?;
            }
            refs::purge_expired(&worker_store, &id)
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
        if entry.summary.kind == LibraryItemKind::FolderCopy {
            let input = entry.summary.folder.as_ref().ok_or_else(||
                error("library_corrupt", "Folder item has no origin"))?.origin_path.clone();
            return match self.save_folder(store, operation, &input, None, Some(entry.clone()),
                confirmed.as_deref(), None).await {
                Ok(()) => Ok(()),
                Err(e) => self.fetch_failed(store, operation, entry, e, true),
            };
        }
        let result = async {
            let url = entry.canonical_url.as_deref().ok_or_else(|| {
                error(
                    "source_capability_unavailable",
                    "Item has no refreshable provider origin",
                )
            })?;
            let confluence = entry.summary.provider_id.as_deref().is_some_and(|provider_id| {
                self.configuration.providers.iter().any(|provider| {
                    provider.id == provider_id
                        && crate::repositories::is_confluence_executable(&provider.executable)
                })
            });
            if confluence && entry.summary.resource_type.as_deref() != Some("page") {
                return Err(error(
                    "source_identity_mismatch",
                    "Confluence Library item is not a page",
                ));
            }
            if confluence {
                let provider_id = entry.summary.provider_id.as_deref().unwrap_or_default();
                let Some((page, request)) = self.confluence_request(url, Some(provider_id)).await?
                else {
                    return Err(error(
                        "source_identity_mismatch",
                        "Confluence page no longer resolves to its selected provider",
                    ));
                };
                if entry.summary.canonical_id.as_deref() != Some(page.page_id.as_str()) {
                    return Err(error(
                        "source_identity_mismatch",
                        "Confluence page identity changed during refresh",
                    ));
                }
                self.sources.fetch_assets(request).await
            } else {
                let request = self.request(url, entry.summary.provider_id.as_deref())?;
                self.sources.fetch_assets(request).await
            }
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
                    let seed = ReferenceSeed {
                        source: asset.source.clone(),
                        label: asset_label(&asset),
                        references: asset_references(&self.configuration, &asset),
                    };
                    match self.save_asset(store, operation, asset, Some(entry.clone()), confirmed.as_deref(), None).await {
                        Ok(()) => self.refresh_related(store, operation, &entry.summary.item_id, seed).await,
                        Err(e) => self.fetch_failed(store, operation, entry, e, true),
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
                        true,
                    )
                }
            }
            Err(e) => self.fetch_failed(store, operation, entry, e, true),
        }
    }
    fn fetch_failed(
        &self,
        store: &Store,
        operation: &str,
        mut entry: LibraryIndexEntry,
        e: InspectionError,
        confirm_not_found: bool,
    ) -> Result<(), InspectionError> {
        let removed = confirm_not_found && e.code == "source_not_found";
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
    async fn save_asset(
        &self,
        store: &Arc<Store>,
        operation: &str,
        asset: SourceAsset,
        old: Option<LibraryIndexEntry>,
        confirmed: Option<&[LibraryConflictFile]>,
        target: Option<&SpaceTarget>,
    ) -> Result<(), InspectionError> {
        self.save_asset_with(store, operation, asset, old, SaveOptions { confirmed, target, ..SaveOptions::default() }).await
    }
    async fn save_asset_with(
        &self,
        store: &Arc<Store>,
        operation: &str,
        mut asset: SourceAsset,
        old: Option<LibraryIndexEntry>,
        options: SaveOptions<'_>,
    ) -> Result<(), InspectionError> {
        let SaveOptions { confirmed, target, reference, reason, download_all, attachment_request, issue_row } = options;
        if let Some(old) = &old {
            asset.original_url = old.summary.original_url.clone();
        }
        let canonical_url = asset.source_url.as_deref().ok_or_else(|| {
            error(
                "source_identity_mismatch",
                "Provider asset has no validated canonical URL",
            )
        })?;
        let is_confluence = self.configuration.providers.iter().any(|provider| {
            provider.id == asset.source.provider_id
                && crate::repositories::is_confluence_executable(&provider.executable)
        });
        if is_confluence && asset.source.resource_type != "page" {
            return Err(error(
                "source_identity_mismatch",
                "Confluence provider returned a non-page Library asset",
            ));
        }
        let canonical_url = if is_confluence {
            if !confluence_page_id(&asset.source.canonical_id) {
                return Err(error(
                    "source_identity_mismatch",
                    "Confluence provider asset has an invalid page id",
                ));
            }
            let site = site_authority(&self.configuration, &asset.source.provider_id)?;
            let canonical = confluence_page_url(&site.provider_instance, &asset.source.canonical_id);
            let page = ConfluencePage {
                page_id: asset.source.canonical_id.clone(),
                space_key: asset
                    .container
                    .as_ref()
                    .map(|container| container.id.clone())
                    .unwrap_or_default(),
                title: asset.title.clone(),
                version: asset.source_revision.as_deref().and_then(|version| version.parse().ok()),
                source_url: canonical_url.to_owned(),
                canonical_url: canonical.clone(),
            };
            let authority = confluence_instance_authority(
                &self.configuration,
                &asset.source.provider_id,
                &page,
                &canonical,
            )?;
            if asset.source.provider_instance != authority.provider_instance {
                return Err(error(
                    "source_identity_mismatch",
                    "Confluence asset belongs to a different configured site",
                ));
            }
            canonical
        } else {
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
            canonical.canonical_url
        };
        let mut entry = asset_entry(&asset, old.as_ref());
        entry.references = self
            .is_jira_provider(&asset.source.provider_id)
            .then(|| asset_references(&self.configuration, &asset));
        entry.canonical_url = Some(canonical_url);
        if let Some(row) = issue_row {
            entry.summary.issue = Some(LibraryIssueMeta {
                updated: row.updated.clone(),
                fetched_updated: Some(row.updated.clone()),
                status: row.status.clone(),
                issue_type: row.issue_type.clone(),
                assignee: row.assignee.clone(),
            });
        }
        if old.is_none() {
            if let Some(reference) = &reference {
                refs::insert_ref(&mut entry.summary, reference.clone());
            }
        }
        let index_items = {
            let _lock = store.shared()?;
            store.index()?.items
        };
        if is_confluence {
            if let Some(parent_id) = field_string(&asset, "parent_id") {
                entry.summary.parent_item_id = index_items
                    .iter()
                    .find(|candidate| {
                        candidate.summary.provider_id.as_deref()
                            == Some(asset.source.provider_id.as_str())
                            && candidate.summary.provider_instance.as_deref()
                                == Some(asset.source.provider_instance.as_str())
                            && candidate.summary.resource_type.as_deref() == Some("page")
                            && candidate.summary.canonical_id.as_deref() == Some(parent_id.as_str())
                    })
                    .map(|candidate| candidate.summary.item_id.clone());
            }
        }
        resolve_asset_path(&mut entry, &asset, old.as_ref(), &index_items);
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
        // Any provider's attachments go through the same staging; the download
        // gate (`SourceService::attachment_downloads`) decides who may download.
        let prepared = if is_confluence || !asset.attachments.is_empty() {
            let Some(prepared) = self.prepare_attachments(store, operation, &mut asset, old.as_ref(), download_all, attachment_request, confirmed).await? else {
                return Ok(());
            };
            entry.summary.attachments = attachments::summaries(&asset);
            entry.summary.revision = prepared.revision(&asset);
            Some(prepared)
        } else {
            None
        };
        let attachment_partial = prepared.as_ref().is_some_and(|p| p.partial);
        let reason = prepared.as_ref().and_then(|p| p.reason.clone()).or(reason);
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
            let stage = match prepared {
                Some(prepared) => store.stage_asset_into(prepared.stage, &mut entry, &asset, prepared.files)?,
                None => store.stage_asset(&mut entry, &asset)?,
            };
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
        // An existing item gains its reference only after the save succeeded, and
        // through the index so a concurrent reference change is never overwritten.
        if old.is_some() {
            if let Some(reference) = reference {
                store.add_ref(&entry.summary.item_id, reference)?;
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
            if attachment_partial {
                LibraryReportOutcome::Partial
            } else if old.is_none() {
                LibraryReportOutcome::New
            } else if equal {
                LibraryReportOutcome::Unchanged
            } else {
                LibraryReportOutcome::Updated
            },
            reason,
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
            } => self.remove_item(&item_id, &expected_revision)?,
            LibraryRemoveRequest::StopFollowing { follow_id } => {
                self.remove_follow(&follow_id, false)?
            }
            LibraryRemoveRequest::Follow { follow_id } => self.remove_follow(&follow_id, true)?,
        }
        self.listing(None).await
    }
}
/// Save-path choices beyond the asset itself.
#[derive(Default)]
struct SaveOptions<'a> {
    confirmed: Option<&'a [LibraryConflictFile]>,
    target: Option<&'a SpaceTarget>,
    /// The reference this save adds: the birth reference of a new item, or added
    /// to an existing one after the save. `None` leaves references untouched.
    reference: Option<LibraryItemRef>,
    /// Report reason for this item's row, e.g. why a followed page changed.
    reason: Option<String>,
    download_all: bool,
    attachment_request: Option<&'a LibraryAttachmentRequest>,
    /// The listing row that led to this save of a Jira issue; sets `issue`.
    issue_row: Option<&'a IssueRow>,
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
fn field_strings(asset: &SourceAsset, key: &str) -> Vec<String> {
    asset
        .fields
        .iter()
        .find(|field| field.key == key)
        .and_then(|field| match &field.value {
            FrontmatterValue::Strings(values) => Some(values.clone()),
            _ => None,
        })
        .unwrap_or_default()
}
fn field_string(asset: &SourceAsset, key: &str) -> Option<String> {
    asset
        .fields
        .iter()
        .find(|field| field.key == key)
        .and_then(|field| match &field.value {
            FrontmatterValue::String(value) => Some(value.clone()),
            _ => None,
        })
}
fn asset_entry(asset: &SourceAsset, old: Option<&LibraryIndexEntry>) -> LibraryIndexEntry {
    let id = item_id(&asset.source);
    let revision = content_revision(asset);
    let now = timestamp();
    let placement = layout::source_placement(asset);
    let mut path = placement.container.join("/");
    if !path.is_empty() {
        path.push('/');
    }
    path.push_str(&placement.leaf);
    let ancestor_titles = field_strings(asset, "ancestors");
    let ancestor_ids = field_strings(asset, "ancestor_ids");
    let ancestors = if asset.source.resource_type == "page" {
        ancestor_ids
            .into_iter()
            .zip(ancestor_titles)
            .map(|(id, title)| LibraryAncestor { id, title })
            .collect()
    } else {
        vec![]
    };
    LibraryIndexEntry {
        inventory: old.map(|e| e.inventory.clone()).unwrap_or_default(),
        references: None,
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
            ancestors,
            order: old.and_then(|e| e.summary.order),
            title: asset.title.clone(),
            document_path: Some(format!("{path}/{}", placement.document)),
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
            refs: old.map(|e| e.summary.refs.clone()).unwrap_or_default(),
            purge_after: old.and_then(|e| e.summary.purge_after.clone()),
            issue: old.and_then(|e| e.summary.issue.clone()),
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
            reference_depth: old.and_then(|e| e.summary.reference_depth),
            included_by: old.and_then(|e| e.summary.included_by.clone()),
        },
    }
}
fn resolve_asset_path(
    entry: &mut LibraryIndexEntry,
    asset: &SourceAsset,
    old: Option<&LibraryIndexEntry>,
    items: &[LibraryIndexEntry],
) {
    let placement = layout::source_placement(asset);
    let parent = entry
        .summary
        .parent_item_id
        .as_ref()
        .and_then(|parent_id| items.iter().find(|item| &item.summary.item_id == parent_id));
    let mut base = parent
        .map(|item| item.summary.item_path.clone())
        .unwrap_or_else(|| placement.container.join("/"));
    if !base.is_empty() {
        base.push('/');
    }
    let candidate = format!("{base}{}", placement.leaf);
    let stable_old = old
        .map(|old| old.summary.item_path.as_str())
        .filter(|path| {
            path.rsplit_once('/').map(|(parent, _)| parent).unwrap_or("")
                == candidate.rsplit_once('/').map(|(parent, _)| parent).unwrap_or("")
                && path.rsplit_once('/').map(|(_, leaf)| leaf).unwrap_or(path)
                    .starts_with(&placement.leaf)
        });
    let mut path = stable_old.unwrap_or(&candidate).to_owned();
    let occupied = |path: &str| {
        items.iter().any(|item| {
            item.summary.item_id != entry.summary.item_id && item.summary.item_path == path
        })
    };
    let leaf = path.rsplit_once('/').map(|(_, leaf)| leaf).unwrap_or(&path);
    if layout::is_reserved(leaf, !path.contains('/')) || occupied(&path) {
        let tag = if asset.source.canonical_id.is_empty() {
            entry.summary.item_id.rsplit(':').next().unwrap_or(&entry.summary.item_id)
        } else {
            &asset.source.canonical_id
        };
        path = format!("{base}{}", layout::tagged(&placement.leaf, tag));
    }
    entry.summary.item_path = path.clone();
    entry.summary.document_path = Some(format!("{path}/{}", placement.document));
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
            cache_root: root.join("cache").to_string_lossy().into_owned(),
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
            reference_depth: 0,
            follow: false,
            follow_mode: None,
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
            assert!(store.item_dir(&entry.summary.item_path).is_ok());
            assert!(
                store.conflicts(&entry).unwrap().is_empty(),
                "{}",
                entry.summary.item_id
            );
        }
    }
    fn jira_entry(instance: &str, key: &str, parent: Option<&str>) -> LibraryIndexEntry {
        let mut asset = asset(1, "body");
        asset.source.provider_id = "jira".into();
        asset.source.provider_instance = instance.into();
        asset.source.canonical_id = key.into();
        let mut entry = asset_entry(&asset, None);
        entry.references = Some(
            parent
                .map(|parent| crate::sources::SourceReference {
                    target: crate::sources::ReferenceTarget::JiraKey {
                        provider_id: "jira".into(),
                        key: parent.into(),
                    },
                    relation: "parent".into(),
                })
                .into_iter()
                .collect(),
        );
        entry
    }

    #[test]
    fn listing_projects_the_jira_parent_across_pages_and_leaves_storage_alone() {
        let site = "https://acme.atlassian.net";
        let mut entries = vec![
            jira_entry(site, "OPS-2", Some("OPS-1")),
            jira_entry(site, "OPS-3", Some("OPS-9")),
            jira_entry(site, "OPS-1", None),
            jira_entry("https://other.atlassian.net", "OPS-4", Some("OPS-1")),
        ];
        let mut legacy = jira_entry(site, "OPS-5", Some("OPS-1"));
        legacy.references = None;
        entries.push(legacy);
        // The child is on the first page, its parent on the second.
        let first = jira_parent_projection(&entries, 0, 1);
        assert_eq!(first[0].parent_item_id.as_deref(), Some(entries[2].summary.item_id.as_str()));
        let rest = jira_parent_projection(&entries, 1, 10);
        let parents: Vec<_> = rest.iter().map(|item| item.parent_item_id.as_deref()).collect();
        // A parent outside the Library, another site's issue and a legacy copy have none.
        assert_eq!(parents, vec![None, None, None, None]);
        assert!(entries.iter().all(|entry| entry.summary.parent_item_id.is_none()));
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
        let document = Path::new(&f.service.configuration.library_root)
            .join(item.document_path.as_deref().unwrap());
        let before = std::fs::read(&document).unwrap();
        let modified = std::fs::metadata(&document).unwrap().modified().unwrap();
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
        assert_eq!(std::fs::read(&document).unwrap(), before);
        assert_eq!(std::fs::metadata(&document).unwrap().modified().unwrap(), modified);
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
        request.reference_depth = 1;
        request
    }

    #[tokio::test]
    async fn busy_related_item_is_reported_and_keeps_the_saved_seed() {
        let f = fixture();
        let store = f.service.open().unwrap();
        let _linked_lease = store.lease(&item_id(&asset(2, "").source)).unwrap();
        let request = linked_add(&f);
        let operation = finished(&f.service, f.service.start_add(request).await.unwrap()).await;
        assert_eq!(operation.phases[0].state, LibraryPhaseState::Partial, "{operation:?}");
        let reason = operation
            .report
            .as_ref()
            .unwrap()
            .rows
            .iter()
            .find(|row| row.outcome == LibraryReportOutcome::Partial)
            .and_then(|row| row.reason.as_deref())
            .unwrap();
        assert!(reason.contains("1 not saved"), "{reason}");
        let saved = reopen(&f).listing(None).await.unwrap().items;
        assert_eq!(saved.iter().map(|item| item.canonical_id.as_deref()).collect::<Vec<_>>(), vec![Some("acme/repo#1")]);
        assert_eq!(saved[0].reference_depth, Some(1));
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
            None, None, Some(&target)).await.unwrap_err();
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
        request.reference_depth = 1;
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
        assert!(reason.contains("1 not saved"), "{reason}");
        assert!(reason.contains("issues/2"), "{reason}");
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
    async fn markup_diagnostic_alone_does_not_make_related_pass_partial() {
        let f = fixture();
        f.provider
            .state
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .diagnostic = true;
        let mut request = add(1);
        request.reference_depth = 1;
        let operation = finished(&f.service, f.service.start_add(request).await.unwrap()).await;
        assert_eq!(operation.phases[0].state, LibraryPhaseState::Done);
        assert_eq!(operation.report.unwrap().partial, 0);
        assert_eq!(
            f.service.listing(None).await.unwrap().items[0].diagnostics[0].code,
            "source_markup_unconverted"
        );
    }

    #[tokio::test]
    async fn single_reference_depth_is_a_policy_only_a_refreshing_add_changes() {
        let f = fixture();
        f.provider.set_body("See https://forge.test/gitea/acme/repo/issues/2");
        let by_id = |items: Vec<LibraryItemSummary>, n: u32| {
            items
                .into_iter()
                .find(|i| i.canonical_id.as_deref() == Some(&format!("acme/repo#{n}")))
        };
        let run = |request: LibraryAddRequest| {
            let f = &f;
            async move {
                finished(&f.service, f.service.start_add(request).await.unwrap()).await;
                f.service.listing(None).await.unwrap().items
            }
        };
        // Depth 0 imports the seed alone.
        let items = run(add(1)).await;
        assert_eq!(items.len(), 1);
        assert_eq!(items[0].reference_depth, None);
        let resolution = f.service.resolve(LibraryResolveRequest {
            input: add(1).input,
            provider_id: None,
        }).await.unwrap();
        assert_eq!(resolution.reference_depth, Some(0));

        // Adding again without a refresh (`Keep in Library`) never changes the policy.
        let mut request = add(1);
        request.reference_depth = 1;
        assert_eq!(run(request.clone()).await.len(), 1);

        // A refreshing add applies it even though the source is unchanged.
        request.refresh_existing = true;
        let items = run(request.clone()).await;
        assert_eq!(items.len(), 2);
        let seed = by_id(items.clone(), 1).unwrap();
        let related = by_id(items, 2).unwrap();
        assert_eq!(seed.reference_depth, Some(1));
        assert_eq!(related.refs, [LibraryItemRef::Manual]);
        assert_eq!(
            related.included_by,
            Some(vec![LibraryInclusion {
                holder: LibraryInclusionHolder::Item { item_id: seed.item_id.clone() },
                from_item_id: Some(seed.item_id.clone()),
                from_label: "Issue 1".into(),
                relation: "body".into(),
                depth: 1,
            }])
        );

        // Keep in Library at depth 0 keeps the stored depth.
        assert_eq!(by_id(run(add(1)).await, 1).unwrap().reference_depth, Some(1));

        // When the seed stops linking, a complete refresh strips the reason but the
        // related item stays Manual, like a hydrated linked item always did.
        f.provider.set_body("no links");
        refresh(&f, &seed).await;
        let items = f.service.listing(None).await.unwrap().items;
        let related = by_id(items, 2).unwrap();
        assert_eq!((related.included_by, related.refs), (None, vec![LibraryItemRef::Manual]));

        // A refreshing add at depth 0 clears the policy.
        request.reference_depth = 0;
        assert_eq!(by_id(run(request).await, 1).unwrap().reference_depth, None);
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
            vec!["Issue 1.md"]
        );
        let e = f
            .service
            .document(LibraryDocumentRequest {
                path: ".cockpit/index.json".into(),
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
#[cfg(test)]
mod confluence {
    use super::*;
    use crate::sources::{
        FrontmatterField, ProviderResolution, SourceAttachment, SourceContainer, SourceProvider,
    };
    use async_trait::async_trait;
    use cockpit_protocol::{
        projects::ProjectProvider,
        sources::SourceCapability,
    };
    use std::sync::{Arc, Mutex};

    struct PageState {
        version: u64,
        body: String,
        foreign_identity: bool,
        fetches: usize,
    }
    struct FakeConfluence {
        base_url: String,
        state: Mutex<PageState>,
    }
    impl FakeConfluence {
        fn set_page(&self, version: u64, body: &str) {
            let mut state = self.state.lock().unwrap_or_else(|e| e.into_inner());
            state.version = version;
            state.body = body.to_owned();
        }
    }
    #[async_trait]
    impl SourceProvider for FakeConfluence {
        fn provider_id(&self) -> &str {
            "confluence"
        }
        fn capabilities(&self) -> Vec<SourceCapability> {
            vec![]
        }
        async fn resolve_input(
            &self,
            input: &str,
        ) -> Result<ProviderResolution, InspectionError> {
            let page_id = if input.bytes().all(|byte| byte.is_ascii_digit()) {
                input.to_owned()
            } else if let Some((_, id)) = input.split_once("pageId=") {
                id.split('&').next().unwrap_or_default().to_owned()
            } else {
                input
                    .split("/pages/")
                    .nth(1)
                    .and_then(|rest| rest.split('/').next())
                    .unwrap_or_default()
                    .to_owned()
            };
            let foreign = self.state.lock().unwrap_or_else(|e| e.into_inner()).foreign_identity;
            let canonical_base = if foreign {
                "https://other.example/wiki"
            } else {
                &self.base_url
            };
            Ok(ProviderResolution::ConfluencePage(ConfluencePage {
                page_id: page_id.clone(),
                space_key: "SD".into(),
                title: "Release checklist".into(),
                version: Some(1),
                source_url: format!("{}/spaces/SD/pages/{page_id}/Release", self.base_url),
                canonical_url: confluence_page_url(canonical_base, &page_id),
            }))
        }
        async fn fetch(
            &self,
            request: &SourceFetchRequest,
        ) -> Result<Vec<SourceAsset>, InspectionError> {
            assert_eq!(request.provider_id, "confluence");
            assert_eq!(request.authority.provider_instance, self.base_url);
            let page_id = request
                .artifact_url
                .split("pageId=")
                .nth(1)
                .unwrap()
                .to_owned();
            assert_eq!(
                request.artifact_url,
                confluence_page_url(&self.base_url, &page_id)
            );
            let mut state = self.state.lock().unwrap_or_else(|e| e.into_inner());
            state.fetches += 1;
            let version = state.version;
            let body = state.body.clone();
            Ok(vec![SourceAsset {
                source: SourceRef {
                    provider_id: "confluence".into(),
                    provider_instance: request.authority.provider_instance.clone(),
                    resource_type: "page".into(),
                    canonical_id: page_id.clone(),
                },
                title: "Release checklist".into(),
                source_url: Some(format!(
                    "{}/spaces/SD/pages/{page_id}/Release",
                    self.base_url
                )),
                original_url: None,
                source_revision: Some(version.to_string()),
                complete: true,
                diagnostics: vec![],
                body,
                container: Some(SourceContainer {
                    id: "SD".into(),
                    label: "SD · Software Development".into(),
                }),
                fields: vec![
                    FrontmatterField {
                        key: "space_key".into(),
                        value: FrontmatterValue::String("SD".into()),
                    },
                    FrontmatterField {
                        key: "space_name".into(),
                        value: FrontmatterValue::String("Software Development".into()),
                    },
                    FrontmatterField {
                        key: "page_id".into(),
                        value: FrontmatterValue::String(page_id.clone()),
                    },
                    FrontmatterField {
                        key: "parent_id".into(),
                        value: FrontmatterValue::String("41".into()),
                    },
                    FrontmatterField {
                        key: "ancestors".into(),
                        value: FrontmatterValue::Strings(vec!["Release process".into()]),
                    },
                    FrontmatterField {
                        key: "ancestor_ids".into(),
                        value: FrontmatterValue::Strings(vec!["41".into()]),
                    },
                    FrontmatterField {
                        key: "version".into(),
                        value: FrontmatterValue::Number(version as i64),
                    },
                    FrontmatterField {
                        key: "last_modified".into(),
                        value: FrontmatterValue::String("2026-09-27T10:00:00Z".into()),
                    },
                    FrontmatterField {
                        key: "last_modified_by".into(),
                        value: FrontmatterValue::String("Casey Maintainer".into()),
                    },
                    FrontmatterField {
                        key: "labels".into(),
                        value: FrontmatterValue::Strings(vec!["release".into()]),
                    },
                ],
                attachments: vec![SourceAttachment {
                    id: "att-10".into(),
                    title: "release-flow.png".into(),
                    media_type: Some("image/png".into()),
                    size: Some(2048),
                    source_url: None,
                    source_revision: Some("3".into()),
                    path: None,
                    not_downloaded: Some("not downloaded".into()),
                }],
            }])
        }
    }

    struct Fixture {
        _base: tests::Fixture,
        service: LibraryService,
        provider: Arc<FakeConfluence>,
    }
    fn fixture(base_url: &str, foreign_identity: bool) -> Fixture {
        let base = tests::fixture();
        let mut configuration = base.service.configuration.clone();
        configuration.providers = vec![ProjectProvider {
            id: "confluence".into(),
            base_url: base_url.into(),
            executable: "/usr/local/bin/confluence".into(),
            login: Some("read-only".into()),
        }];
        let provider = Arc::new(FakeConfluence {
            base_url: base_url.into(),
            state: Mutex::new(PageState {
                version: 1,
                body: "Initial page body".into(),
                foreign_identity,
                fetches: 0,
            }),
        });
        let sources = Arc::new(
            SourceService::new(&configuration, vec![provider.clone()]).unwrap(),
        );
        Fixture {
            _base: base,
            service: LibraryService::new(configuration, sources),
            provider,
        }
    }
    fn add_request(input: &str, provider_id: Option<&str>) -> LibraryAddRequest {
        LibraryAddRequest {
            input: input.into(),
            provider_id: provider_id.map(str::to_owned),
            reference_depth: 0,
            follow: false,
            follow_mode: None,
            download_attachments: false,
            refresh_existing: false,
            label: None,
            target: None,
        }
    }

    #[tokio::test]
    async fn cloud_and_dc_page_resolve_add_and_refresh_use_selected_authority() {
        for base_url in [
            "https://acme.atlassian.net/wiki",
            "https://dc.example.test/confluence",
        ] {
            let fixture = fixture(base_url, false);
            let page_url = format!("{base_url}/spaces/SD/pages/123456/Release");
            let resolved = fixture
                .service
                .resolve(LibraryResolveRequest {
                    input: page_url.clone(),
                    provider_id: None,
                })
                .await
                .unwrap();
            assert_eq!(resolved.kind, LibraryInputKind::ConfluencePage);
            assert_eq!(resolved.provider_id.as_deref(), Some("confluence"));
            assert_eq!(resolved.provider_instance.as_deref(), Some(base_url));
            assert_eq!(resolved.canonical_id.as_deref(), Some("123456"));
            assert_eq!(resolved.title, "Release checklist");
            assert_eq!(
                resolved.container_label.as_deref(),
                Some("SD · Software Development")
            );

            let resolved_by_id = fixture
                .service
                .resolve(LibraryResolveRequest {
                    input: "123456".into(),
                    provider_id: Some("confluence".into()),
                })
                .await
                .unwrap();
            assert_eq!(resolved_by_id.canonical_id.as_deref(), Some("123456"));
            let unselected_id = fixture
                .service
                .resolve(LibraryResolveRequest {
                    input: "123456".into(),
                    provider_id: None,
                })
                .await
                .unwrap_err();
            assert_eq!(unselected_id.code, "source_authority_mismatch");

            let operation = fixture
                .service
                .start_add(add_request(&page_url, Some("confluence")))
                .await
                .unwrap();
            let operation = tests::finished(&fixture.service, operation).await;
            assert!(operation.finished);
            let listing = fixture.service.listing(None).await.unwrap();
            assert_eq!(listing.items.len(), 1);
            let initial = listing.items[0].clone();
            assert_eq!(initial.provider_id.as_deref(), Some("confluence"));
            assert_eq!(initial.provider_instance.as_deref(), Some(base_url));
            assert_eq!(initial.resource_type.as_deref(), Some("page"));
            assert_eq!(initial.canonical_id.as_deref(), Some("123456"));
            assert_eq!(initial.source_revision.as_deref(), Some("1"));
            assert_eq!(
                initial.container.as_ref().map(|container| container.container_id.as_str()),
                Some("SD")
            );
            assert_eq!(
                initial.container.as_ref().map(|container| container.label.as_str()),
                Some("SD · Software Development")
            );
            assert_eq!(initial.ancestors.len(), 1);
            assert_eq!(initial.ancestors[0].id, "41");
            assert_eq!(initial.ancestors[0].title, "Release process");
            assert_eq!(initial.parent_item_id, None);
            assert_eq!(initial.attachments.len(), 1);
            assert_eq!(
                initial.attachments[0].state,
                LibraryAttachmentState::NotDownloaded
            );
            assert_eq!(initial.attachments[0].bytes, Some(2048));
            let document = std::fs::read_to_string(
                Path::new(&fixture.service.configuration.library_root)
                    .join(initial.document_path.as_ref().unwrap()),
            )
            .unwrap();
            assert!(document.contains("space_name: \"Software Development\""));
            assert!(document.contains("space_key: \"SD\""));
            assert!(document.contains("page_id: \"123456\""));
            assert!(document.contains("parent_id: \"41\""));
            assert!(document.contains("ancestors: [\"Release process\"]"));
            assert!(document.contains("version: 1"));
            assert!(document.contains("last_modified: \"2026-09-27T10:00:00Z\""));
            assert!(document.contains("labels: [\"release\"]"));
            assert!(document.contains("last_modified_by: \"Casey Maintainer\""));
            assert!(document.contains("Release process"));
            assert!(document.contains("not_downloaded: \"not downloaded\""));
            assert!(!document.contains("casey@example.test"));
            assert!(document.contains("Initial page body"));

            fixture.provider.set_page(2, "Updated page body");
            let operation = fixture
                .service
                .start_refresh(LibraryRefreshRequest::Items {
                    item_ids: vec![initial.item_id.clone()],
                })
                .await
                .unwrap();
            let operation = tests::finished(&fixture.service, operation).await;
            assert!(operation.finished);
            let refreshed = fixture.service.listing(None).await.unwrap().items.remove(0);
            assert_eq!(refreshed.source_revision.as_deref(), Some("2"));
            assert_eq!(refreshed.state, LibraryItemState::Changed);
            assert_ne!(refreshed.revision, initial.revision);
            let document = std::fs::read_to_string(
                Path::new(&fixture.service.configuration.library_root)
                    .join(refreshed.document_path.as_ref().unwrap()),
            )
            .unwrap();
            assert!(document.contains("Updated page body"));
            assert!(document.contains("version: 2"));
            assert_eq!(
                fixture.provider.state.lock().unwrap_or_else(|e| e.into_inner()).fetches,
                4
            );
        }
    }

    #[tokio::test]
    async fn refuses_foreign_canonical_identity_before_fetch_or_save() {
        let fixture = fixture("https://acme.atlassian.net/wiki", true);
        let request = LibraryResolveRequest {
            input: "https://acme.atlassian.net/wiki/spaces/SD/pages/123456/Release".into(),
            provider_id: Some("confluence".into()),
        };
        let error = fixture.service.resolve(request).await.unwrap_err();
        assert_eq!(error.code, "source_identity_mismatch");
        assert_eq!(
            fixture.provider.state.lock().unwrap_or_else(|e| e.into_inner()).fetches,
            0
        );
        assert!(fixture.service.listing(None).await.unwrap().items.is_empty());
        let add_error = fixture
            .service
            .start_add(add_request(
                "https://acme.atlassian.net/wiki/spaces/SD/pages/123456/Release",
                Some("confluence"),
            ))
            .await
            .unwrap_err();
        assert_eq!(add_error.code, "source_identity_mismatch");
        assert_eq!(
            fixture.provider.state.lock().unwrap_or_else(|e| e.into_inner()).fetches,
            0
        );
        let authority = site_authority(&fixture.service.configuration, "confluence").unwrap();
        let canonical = confluence_page_url(&authority.provider_instance, "123456");
        let page = ConfluencePage {
            page_id: "123456".into(),
            space_key: "SD".into(),
            title: "Release checklist".into(),
            version: Some(1),
            source_url: "https://other.example/wiki/spaces/SD/pages/123456/Release".into(),
            canonical_url: canonical.clone(),
        };
        assert_eq!(
            confluence_instance_authority(
                &fixture.service.configuration,
                "confluence",
                &page,
                &canonical,
            )
            .unwrap_err()
            .code,
            "source_identity_mismatch"
        );
    }
}
