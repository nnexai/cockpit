//! Publication regressions exercise real traversal, scheduler, storage and binary downloads.
use super::{
    LibraryService, item_id, operations, refs,
    store::{Index, LibraryIndexEntry},
    tests::{self as base, finished},
};
use crate::{
    InspectionError,
    config::LibrarySyncConfiguration,
    process::StagingBudget,
    sources::{
        self, AttachmentRef, ConfluencePage, DownloadedAttachment, PageAux, ProviderResolution,
        ReferenceSeed, ReferenceTarget, RelatedAsset, SourceAsset, SourceAttachment,
        SourceContainer, SourceFetchRequest, SourceProvider, SourceRef, SourceReference,
        SourceService, SpacePage, SpacePageListing, SpaceSummary, TraversalBudget,
    },
};
use async_trait::async_trait;
use cap_std::fs::Dir;
use cockpit_protocol::{
    library::*,
    projects::{ProjectDiagnostic, ProjectProvider, ProviderDeployment, ProviderKind},
    sources::SourceCapability,
};
use parking_lot::Mutex;
use serde_json::{Value, json};
use std::{
    path::Path,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
};
use tokio::sync::{Notify, Semaphore};

const SITE: &str = "https://acme.atlassian.net/wiki";
const NOW: i64 = 1_800_000_000_000;
const MINUTE: i64 = 60_000;
const BINARY: &[u8] = b"previously downloaded attachment\0\xff";

struct Gate {
    armed: AtomicBool,
    entered: Notify,
    release: Semaphore,
}
impl Gate {
    fn new() -> Self {
        Self {
            armed: AtomicBool::new(false),
            entered: Notify::new(),
            release: Semaphore::new(0),
        }
    }
    async fn wait(&self) {
        if self.armed.swap(false, Ordering::SeqCst) {
            self.entered.notify_one();
            self.release.acquire().await.unwrap().forget();
        }
    }
}
#[derive(Clone)]
struct Page {
    version: u64,
    partial: bool,
    attachments: Vec<SourceAttachment>,
}
struct Wiki {
    page: Mutex<Page>,
    fetch: Gate,
    download: Gate,
}
fn attachment(version: &str) -> SourceAttachment {
    SourceAttachment {
        id: "11".into(),
        title: "saved.bin".into(),
        media_type: Some("application/octet-stream".into()),
        size: Some(BINARY.len() as u64),
        source_url: None,
        source_revision: Some(version.into()),
        path: None,
        not_downloaded: None,
    }
}
fn source(id: &str) -> SourceRef {
    SourceRef {
        provider_id: "confluence".into(),
        provider_instance: SITE.into(),
        resource_type: "page".into(),
        canonical_id: id.into(),
    }
}
fn page_url() -> String {
    format!("{SITE}/spaces/SD/pages/1")
}
fn listing(page: &Page) -> SpacePageListing {
    SpacePageListing {
        space_name: "Software Development".into(),
        homepage_id: None,
        total: Some(1),
        complete: true,
        pages: vec![SpacePage {
            page_id: "1".into(),
            title: "Page 1".into(),
            version: page.version,
            ancestors: vec![],
            position: None,
        }],
    }
}
#[async_trait]
impl SourceProvider for Wiki {
    fn provider_id(&self) -> &str {
        "confluence"
    }
    fn capabilities(&self) -> Vec<SourceCapability> {
        vec![]
    }
    async fn resolve_input(&self, input: &str) -> Result<ProviderResolution, InspectionError> {
        if input == "SD" {
            return Ok(ProviderResolution::ConfluenceSpace {
                space_key: "SD".into(),
            });
        }
        assert!(
            input == "1" || input == page_url() || input == sources::confluence_page_url(SITE, "1")
        );
        Ok(ProviderResolution::ConfluencePage(ConfluencePage {
            page_id: "1".into(),
            space_key: "SD".into(),
            title: "Page 1".into(),
            version: Some(self.page.lock().version),
            source_url: page_url(),
            canonical_url: sources::confluence_page_url(SITE, "1"),
        }))
    }
    async fn page_space(&self, _id: &str) -> Result<Option<String>, InspectionError> {
        Ok(Some("SD".into()))
    }
    async fn list_spaces(&self) -> Result<Vec<SpaceSummary>, InspectionError> {
        Ok(vec![SpaceSummary {
            key: "SD".into(),
            name: "Software Development".into(),
        }])
    }
    async fn list_space_pages(
        &self,
        key: &str,
        _max: u32,
        _cancel: &AtomicBool,
    ) -> Result<SpacePageListing, InspectionError> {
        assert_eq!(key, "SD");
        Ok(listing(&self.page.lock()))
    }
    async fn page_versions(
        &self,
        ids: &[String],
        _cancel: &AtomicBool,
    ) -> Result<SpacePageListing, InspectionError> {
        assert_eq!(ids, ["1"]);
        Ok(listing(&self.page.lock()))
    }
    async fn page_aux(&self, _id: &str) -> Result<PageAux, InspectionError> {
        let page = self.page.lock();
        Ok(PageAux {
            labels: vec![],
            labels_complete: true,
            attachments: page.attachments.clone(),
            attachments_complete: !page.partial,
        })
    }
    async fn fetch(
        &self,
        request: &SourceFetchRequest,
    ) -> Result<Vec<SourceAsset>, InspectionError> {
        self.fetch.wait().await;
        assert_eq!(
            request.artifact_url,
            sources::confluence_page_url(SITE, "1")
        );
        let page = self.page.lock().clone();
        Ok(vec![SourceAsset {
            source: source("1"),
            title: "Page 1".into(),
            source_url: Some(page_url()),
            original_url: None,
            source_revision: Some(page.version.to_string()),
            complete: true,
            diagnostics: if page.partial {
                vec![ProjectDiagnostic {
                    code: "source_attachments_partial".into(),
                    message: "manifest truncated".into(),
                    path: None,
                }]
            } else {
                vec![]
            },
            body: format!("Page body {}", page.version),
            container: Some(SourceContainer {
                id: "SD".into(),
                label: "SD".into(),
            }),
            fields: vec![],
            attachments: page.attachments,
        }])
    }
    async fn attachment_downloads(&self, kind: &str) -> Result<(), InspectionError> {
        assert_eq!(kind, "page");
        Ok(())
    }
    async fn download_attachment(
        &self,
        id: &str,
        attachment: &AttachmentRef,
        _siblings: &[AttachmentRef],
        dest: &Dir,
        _path: &Path,
        _budget: StagingBudget,
    ) -> Result<DownloadedAttachment, InspectionError> {
        assert_eq!(id, "1");
        self.download.wait().await;
        dest.write("download.bin", BINARY).unwrap();
        Ok(DownloadedAttachment {
            attachment_id: attachment.id.clone(),
            file_name: "download.bin".into(),
        })
    }
}

struct Fixture {
    base: base::Fixture,
    wiki: Arc<Wiki>,
}
impl Fixture {
    fn new() -> Self {
        let mut base = base::fixture();
        let mut config = base.service.configuration.clone();
        config.providers = vec![ProjectProvider {
            id: "confluence".into(),
            kind: ProviderKind::Confluence,
            base_url: SITE.into(),
            executable: None,
            login: None,
            deployment: Some(ProviderDeployment::Cloud),
        }];
        let wiki = Arc::new(Wiki {
            page: Mutex::new(Page {
                version: 1,
                partial: false,
                attachments: vec![attachment("1")],
            }),
            fetch: Gate::new(),
            download: Gate::new(),
        });
        let sources = Arc::new(SourceService::new(&config, vec![wiki.clone()]).unwrap());
        base.service = LibraryService::new(config, sources);
        Self { base, wiki }
    }
    fn service(&self) -> &LibraryService {
        &self.base.service
    }
    fn index(&self) -> Index {
        let store = self.service().open().unwrap();
        let _lock = store.shared().unwrap();
        store.index().unwrap()
    }
    fn entry(&self) -> LibraryIndexEntry {
        self.index()
            .items
            .into_iter()
            .find(|e| e.summary.item_id == item_id(&source("1")))
            .unwrap()
    }
    fn index_bytes(&self) -> Vec<u8> {
        std::fs::read(self.base.root.join("library/.cockpit/index.json")).unwrap()
    }
    fn owned_bytes(&self) -> Vec<(String, Vec<u8>)> {
        let entry = self.entry();
        let dir = self
            .base
            .root
            .join("library")
            .join(&entry.summary.item_path);
        entry
            .inventory
            .iter()
            .filter(|file| file.hash != "directory")
            .map(|file| {
                (
                    file.path.clone(),
                    std::fs::read(dir.join(&file.path)).unwrap(),
                )
            })
            .collect::<Vec<_>>()
    }
    fn state(&self) -> Value {
        serde_json::from_slice(
            &std::fs::read(self.base.root.join("library/.cockpit/sync/state.json")).unwrap(),
        )
        .unwrap()
    }
    async fn import(&self) {
        let request = LibraryAddRequest {
            input: "1".into(),
            provider_id: Some("confluence".into()),
            reference_depth: 0,
            follow: false,
            follow_mode: None,
            download_attachments: true,
            refresh_existing: false,
            label: None,
            target: None,
        };
        let receipt = finished(
            self.service(),
            self.service().start_add(request).await.unwrap(),
        )
        .await;
        assert!(
            receipt.phases.iter().all(|phase| phase.error.is_none()),
            "{receipt:?}"
        );
        assert_eq!(
            self.entry().summary.attachments[0].state,
            LibraryAttachmentState::Downloaded
        );
        assert!(self.owned_bytes().iter().any(|(_, bytes)| bytes == BINARY));
    }
    async fn downloaded_follow(&self) -> String {
        let request = LibraryAddRequest {
            input: "SD".into(),
            provider_id: Some("confluence".into()),
            reference_depth: 0,
            follow: true,
            follow_mode: None,
            download_attachments: true,
            refresh_existing: false,
            label: None,
            target: None,
        };
        let receipt = finished(
            self.service(),
            self.service().start_add(request).await.unwrap(),
        )
        .await;
        assert!(
            receipt.phases.iter().all(|phase| phase.error.is_none()),
            "{receipt:?}"
        );
        let follow = self.index().follows.into_iter().next().unwrap();
        assert!(follow.include_attachments);
        assert!(refs::has_follow(&self.entry().summary, &follow.follow_id));
        follow.follow_id
    }
    fn only_follow_inventory_due(&self, follow: &str) {
        let later = NOW + 100 * MINUTE;
        let mut state =
            json!({"schema":1,"sources":{},"queue":{},"audits":{},"origins":{},"active_operation":null});
        for current in self.index().follows {
            state["sources"][&current.follow_id] = json!({
                "next_delta_ms": later, "next_inventory_ms": if current.follow_id == follow { 0 } else { later },
                "next_related_ms": later,
                "last_started_ms": 0,
                "last_success_ms": null,
                "committed_upper_ms": null,
                "window": null,
                "failures": 0,
                "last_error": null,
                "absent": {},
                "related_absent": {},
                "held": null,
                "inventory": null
            });
        }
        state["audits"][format!("confluence:{SITE}")] = json!({"cursor":null,"next_due_ms":later,"failures":{}});
        let path = self.base.root.join("library/.cockpit/sync/state.json");
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, serde_json::to_vec(&state).unwrap()).unwrap();
    }
    fn follow(&self, id: &str) {
        let store = self.service().open().unwrap();
        store
            .mutate_index(|index| {
                index.follows.push(LibraryFollowSummary {
                    follow_id: id.into(),
                    provider_id: "confluence".into(),
                    provider_instance: SITE.into(),
                    source: LibraryFollowSource::ConfluenceSpace {
                        space_name: "SD".into(),
                        space_key: "SD".into(),
                    },
                    include_attachments: false,
                    item_count: 0,
                    partial: None,
                    excluded_ids: vec![],
                    last_refreshed_at: None,
                    state: LibraryItemState::Unknown,
                    reference_depth: Some(1),
                });
                Ok(())
            })
            .unwrap();
    }
    async fn related(&self, follow: &str) -> super::related::RelatedPass {
        let store = self.service().open().unwrap();
        let (operation, _lease) = operations::create_background(&store).unwrap();
        let seed = ReferenceSeed {
            source: source("2"),
            label: "Seed page".into(),
            references: vec![SourceReference {
                target: ReferenceTarget::Url {
                    url: sources::confluence_page_url(SITE, "1"),
                },
                relation: "body".into(),
            }],
        };
        sources::lane::scope(
            sources::lane::RequestLane::Background,
            self.service().run_related(
                &store,
                &operation.operation_id,
                vec![seed],
                0,
                1,
                TraversalBudget::Single,
                &LibraryInclusionHolder::Follow {
                    follow_id: follow.into(),
                },
                &LibraryItemRef::Follow {
                    follow_id: follow.into(),
                },
                &|_: &RelatedAsset| false,
            ),
        )
        .await
        .unwrap()
    }
}

#[tokio::test]
async fn partial_manifest_retains_downloaded_binary_manifest_and_index_until_complete_queue_retry()
{
    let f = Fixture::new();
    f.import().await;
    let original_index = f.index_bytes();
    let original_files = f.owned_bytes();
    let id = f.entry().summary.item_id;
    {
        let mut page = f.wiki.page.lock();
        page.version = 2;
        page.partial = true;
        page.attachments.clear();
    }
    let policy = LibrarySyncConfiguration::default();
    let pending = f.service().sync_tick(policy.clone(), NOW).await.unwrap();
    assert_eq!(pending.pending, 1);
    assert_eq!(
        pending.fetched, 0,
        "an incomplete manifest must not report successful publication"
    );
    assert_eq!(f.index_bytes(), original_index);
    assert_eq!(f.owned_bytes(), original_files);
    assert_eq!(
        f.state()["queue"][&id]["last_error"],
        "source_attachments_partial"
    );
    assert_eq!(f.state()["queue"][&id]["failures"], 1);
    assert_eq!(f.state()["queue"][&id]["next_attempt_ms"], NOW + MINUTE);
    {
        let mut page = f.wiki.page.lock();
        page.partial = false;
        page.attachments = vec![attachment("1")];
    }
    let before_retry = f
        .service()
        .sync_tick(policy.clone(), NOW + MINUTE - 1)
        .await
        .unwrap();
    assert_eq!(before_retry.pending, 1);
    assert_eq!(before_retry.fetched, 0);
    assert_eq!(f.index_bytes(), original_index);
    assert_eq!(f.owned_bytes(), original_files);
    let restarted = LibraryService::new(
        f.service().configuration.clone(),
        f.service().sources.clone(),
    );
    let published = restarted.sync_tick(policy, NOW + MINUTE).await.unwrap();
    assert_eq!(published.pending, 0);
    assert_eq!(published.fetched, 1);
    assert!(f.state()["queue"].as_object().unwrap().is_empty());
    assert_eq!(f.entry().summary.source_revision.as_deref(), Some("2"));
    assert!(f.owned_bytes().iter().any(|(_, bytes)| bytes == BINARY));
    assert_eq!(
        f.entry().summary.attachments[0].state,
        LibraryAttachmentState::Downloaded
    );
}

#[tokio::test]
async fn partial_related_manifest_does_not_change_existing_files_or_reference_ownership() {
    let f = Fixture::new();
    f.import().await;
    f.follow("follow:partial");
    let index = f.index_bytes();
    let files = f.owned_bytes();
    {
        let mut page = f.wiki.page.lock();
        page.version = 2;
        page.partial = true;
        page.attachments.clear();
    }
    let pass = f.related("follow:partial").await;
    assert!(!pass.complete);
    assert!(pass.reached.contains(&f.entry().summary.item_id));
    assert_eq!(f.index_bytes(), index);
    assert_eq!(f.owned_bytes(), files);
    assert_eq!(f.entry().summary.refs, [LibraryItemRef::Manual]);
}

#[tokio::test]
async fn removing_related_item_while_provider_fetch_waits_cannot_resurrect_it() {
    let f = Fixture::new();
    f.follow("follow:removed");
    assert!(f.related("follow:removed").await.complete);
    let old = f.entry();
    assert!(refs::related_of(&old.summary, "follow:removed"));
    let path = f.base.root.join("library").join(&old.summary.item_path);
    f.wiki.fetch.armed.store(true, Ordering::SeqCst);
    let traversal = f.related("follow:removed");
    tokio::pin!(traversal);
    tokio::select! { _ = f.wiki.fetch.entered.notified() => {}, _ = &mut traversal => panic!("fetch did not reach gate") }
    f.service()
        .remove_item(&old.summary.item_id, &old.summary.revision)
        .unwrap();
    assert!(!path.exists());
    f.wiki.fetch.release.add_permits(1);
    assert!(!traversal.await.reached.contains(&old.summary.item_id));
    let index = f.index();
    assert!(index.items.is_empty());
    assert!(!path.exists());
    assert!(index.follows[0].excluded_ids.contains(&old.summary.item_id));
}

#[tokio::test]
async fn stop_follow_during_related_fetch_preserves_manual_content_and_other_holders() {
    let f = Fixture::new();
    f.import().await;
    f.follow("follow:stopped");
    f.follow("follow:other");
    assert!(f.related("follow:stopped").await.complete);
    assert!(f.related("follow:other").await.complete);
    let original_files = f.owned_bytes();
    f.wiki.page.lock().version = 2;
    f.wiki.fetch.armed.store(true, Ordering::SeqCst);
    let traversal = f.related("follow:stopped");
    tokio::pin!(traversal);
    tokio::select! { _ = f.wiki.fetch.entered.notified() => {}, _ = &mut traversal => panic!("publication did not reach gate") }
    f.service().remove_follow("follow:stopped", false).unwrap();
    let stopped_index = f.index_bytes();
    f.wiki.fetch.release.add_permits(1);
    assert!(!traversal.await.reached.contains(&f.entry().summary.item_id));
    assert_eq!(f.owned_bytes(), original_files);
    assert_eq!(f.index_bytes(), stopped_index);
    let saved = f.entry().summary;
    assert_eq!(saved.source_revision.as_deref(), Some("1"));
    assert!(saved.refs.contains(&LibraryItemRef::Manual));
    assert!(refs::has_follow(&saved, "follow:other"));
    assert!(!refs::has_follow(&saved, "follow:stopped"));
    assert!(!refs::related_of(&saved, "follow:stopped"));
}

#[tokio::test]
async fn stop_follow_during_attachment_download_is_rechecked_at_locked_publication() {
    let f = Fixture::new();
    f.import().await;
    let stopped = f.downloaded_follow().await;
    f.follow("follow:other");
    assert!(f.related("follow:other").await.complete);
    let original_files = f.owned_bytes();
    f.only_follow_inventory_due(&stopped);
    {
        let mut page = f.wiki.page.lock();
        page.version = 2;
        page.attachments = vec![attachment("2")];
    }
    f.wiki.download.armed.store(true, Ordering::SeqCst);
    let tick = f
        .service()
        .sync_tick(LibrarySyncConfiguration::default(), NOW);
    tokio::pin!(tick);
    tokio::select! { _ = f.wiki.download.entered.notified() => {}, _ = &mut tick => panic!("scheduled attachment replacement did not reach gate") }
    f.service().remove_follow(&stopped, false).unwrap();
    let stopped_index = f.index_bytes();
    f.wiki.download.release.add_permits(1);
    let report = tick.await.unwrap();
    assert_eq!(report.fetched, 0);
    assert_eq!(report.pending, 1);
    assert_eq!(f.owned_bytes(), original_files);
    assert_eq!(f.index_bytes(), stopped_index);
    let saved = f.entry().summary;
    assert_eq!(saved.source_revision.as_deref(), Some("1"));
    assert_eq!(saved.attachments[0].version.as_deref(), Some("1"));
    assert_eq!(
        saved.attachments[0].state,
        LibraryAttachmentState::Downloaded
    );
    assert!(f.owned_bytes().iter().any(|(_, bytes)| bytes == BINARY));
    assert!(saved.refs.contains(&LibraryItemRef::Manual));
    assert!(refs::has_follow(&saved, "follow:other"));
    assert!(!refs::has_follow(&saved, &stopped));
    assert!(!refs::related_of(&saved, &stopped));
    assert_eq!(
        f.state()["queue"][&saved.item_id]["last_error"],
        "library_follow_excluded"
    );
}
