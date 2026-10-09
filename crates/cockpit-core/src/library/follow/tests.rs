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
    if key == "SD" {
        "Software Development"
    } else {
        "Operations"
    }
}
fn page(space: &str, title: &str, ancestors: &[(&str, &str)]) -> Page {
    Page {
        space: space.into(),
        title: title.into(),
        version: 1,
        ancestors: ancestors
            .iter()
            .map(|(id, t)| ((*id).into(), (*t).into()))
            .collect(),
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
            pages: pages
                .into_iter()
                .map(|(id, page)| (id.to_owned(), page))
                .collect(),
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
        std::mem::take(&mut self.site().fetched)
            .into_iter()
            .collect()
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
            "SD" | "OPS" => Ok(ProviderResolution::ConfluenceSpace {
                space_key: input.into(),
            }),
            _ => Err(error("source_not_found", "no such space")),
        }
    }
    async fn list_spaces(&self) -> Result<Vec<SpaceSummary>, InspectionError> {
        Ok(["SD", "OPS"]
            .map(|key| SpaceSummary {
                key: key.into(),
                name: space_name(key).into(),
            })
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
    async fn fetch(
        &self,
        request: &SourceFetchRequest,
    ) -> Result<Vec<SourceAsset>, InspectionError> {
        let id = request
            .artifact_url
            .split("pageId=")
            .nth(1)
            .unwrap()
            .to_owned();
        let mut site = self.site();
        site.fetched.push(id.clone());
        if site.fetch_failures.contains(&id) {
            return Err(error("source_not_found", "gone during fetch"));
        }
        let page = site
            .pages
            .get(&id)
            .cloned()
            .ok_or_else(|| error("source_not_found", "gone"))?;
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
            fields.push(strings(
                "ancestors",
                page.ancestors.iter().map(|a| a.1.clone()).collect(),
            ));
            fields.push(strings(
                "ancestor_ids",
                page.ancestors.iter().map(|a| a.0.clone()).collect(),
            ));
        }
        Ok(vec![SourceAsset {
            source: SourceRef {
                provider_id: "confluence".into(),
                provider_instance: request.authority.provider_instance.clone(),
                resource_type: "page".into(),
                canonical_id: id.clone(),
            },
            title: page.title.clone(),
            source_url: Some(format!(
                "{}/spaces/{}/pages/{id}",
                site.base_url, page.space
            )),
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
        kind: ProviderKind::Confluence,
        base_url: base_url.into(),
        executable: None,
        login: None,
        deployment: Some(if cloud {
            cockpit_protocol::projects::ProviderDeployment::Cloud
        } else {
            cockpit_protocol::projects::ProviderDeployment::DataCenter
        }),
    }];
    configuration.limits.library_space_pages = page_limit;
    let provider = Arc::new(FakeSpaces::new(base_url, cloud));
    let sources = Arc::new(SourceService::new(&configuration, vec![provider.clone()]).unwrap());
    base.service = LibraryService::new(configuration, sources);
    Fixture { base, provider }
}

#[test]
fn confluence_follow_authority_uses_kind_not_executable_or_id() {
    let mut f = fixture(CLOUD, true, 200);
    assert_eq!(
        f.base
            .service
            .confluence_site("confluence")
            .unwrap()
            .provider_instance,
        CLOUD
    );
    let provider = &mut f.base.service.configuration.providers[0];
    provider.kind = ProviderKind::Gitea;
    provider.executable = Some("/usr/local/bin/confluence".into());
    provider.deployment = None;
    assert_eq!(
        f.base
            .service
            .confluence_site("confluence")
            .unwrap_err()
            .code,
        "source_provider_unsupported"
    );
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
    let operation = finished(
        service,
        service.start_add(follow_request(key, None)).await.unwrap(),
    )
    .await;
    assert_eq!(
        operation.phases[0].state,
        LibraryPhaseState::Done,
        "{operation:?}"
    );
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
    assert!(
        operation.phases.iter().all(|p| p.error.is_none()),
        "{operation:?}"
    );
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
        assert_eq!(
            f.provider.take_fetched(),
            ["100", "110", "111", "112", "200", "201"]
                .map(String::from)
                .into()
        );
        let listing = service.listing(None).await.unwrap();
        let record = &listing.follows[0];
        assert_eq!(record.follow_id, follow_id);
        assert_eq!(
            (record.item_count, record.state),
            (6, LibraryItemState::Fresh)
        );
        assert_eq!(refs::follow_title(record), "SD · Software Development");
        assert!(record.last_refreshed_at.is_some());
        let before = pages(service).await;
        assert!(
            before
                .values()
                .all(|page| refs::has_follow(page, &follow_id))
        );
        let folder = before["200"]
            .ancestors
            .iter()
            .map(|a| (a.id.as_str(), a.title.as_str()))
            .collect::<Vec<_>>();
        assert_eq!(
            folder,
            if cloud {
                vec![("900", "Folder F")]
            } else {
                vec![]
            }
        );
        assert_eq!(before["200"].parent_item_id, None);
        assert_eq!(
            before["201"].parent_item_id.as_ref(),
            Some(&before["200"].item_id)
        );
        assert_eq!(
            before["111"].parent_item_id.as_ref(),
            Some(&before["110"].item_id)
        );

        {
            let mut site = f.provider.site();
            let mut n = site.pages["201"].clone();
            n.title = "N".into();
            n.body = "N body".into();
            site.pages.insert("202".into(), n);
            site.pages.get_mut("201").unwrap().ancestors =
                vec![("100".into(), "H".into()), ("110".into(), "A".into())];
            site.pages.remove("112");
        }
        f.provider.bump("111");
        let report = refresh(
            service,
            LibraryRefreshRequest::Follow {
                follow_id: follow_id.clone(),
            },
        )
        .await;
        assert_eq!(
            (
                report.new,
                report.updated,
                report.removed_at_source,
                report.unchanged
            ),
            (1, 2, 1, 3)
        );
        assert_eq!(
            f.provider.take_fetched(),
            ["111", "201", "202"].map(String::from).into()
        );
        assert_eq!(std::mem::take(&mut f.provider.site().confirmed), ["112"]);
        let after = pages(service).await;
        assert_eq!(reason(&report, &after["111"]), Some("changed"));
        assert_eq!(reason(&report, &after["201"]), Some("moved"));
        assert_eq!(
            after["201"].parent_item_id.as_ref(),
            Some(&after["110"].item_id)
        );
        assert_eq!(
            after["201"]
                .ancestors
                .iter()
                .map(|a| a.id.as_str())
                .collect::<Vec<_>>(),
            ["100", "110"]
        );
        // A move re-renders the page and moves its directory and subtree.
        assert_ne!(after["201"].item_path, before["201"].item_path);
        assert_ne!(after["201"].revision, before["201"].revision);
        assert_eq!(
            after["202"].parent_item_id.as_ref(),
            Some(&after["200"].item_id)
        );
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
        assert!(
            renamed["202"]
                .item_path
                .starts_with(&format!("{}/", renamed["200"].item_path))
        );
        assert!(
            renamed["202"]
                .document_path
                .as_deref()
                .unwrap()
                .starts_with(&format!("{}/", renamed["200"].item_path))
        );
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
    let added = finished(
        service,
        service.start_add(follow_request("SD", None)).await.unwrap(),
    )
    .await;
    assert_eq!(added.phases[0].state, LibraryPhaseState::Partial);
    let follow_id = service.listing(None).await.unwrap().follows[0]
        .follow_id
        .clone();
    let report = added.report.unwrap();
    assert_eq!((report.new, report.partial), (3, 1));
    assert!(
        report
            .rows
            .iter()
            .any(|row| row.follow_id.as_ref() == Some(&follow_id)
                && row.outcome == LibraryReportOutcome::Partial
                && row.reason.as_deref() == Some("3 of 6 pages (page limit)"))
    );
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
        .remove(LibraryRemoveRequest::Item {
            item_id: a1.item_id.clone(),
            expected_revision: a1.revision.clone(),
        })
        .await
        .unwrap();
    let record = service.listing(None).await.unwrap().follows.remove(0);
    assert_eq!(record.excluded_ids, ["111"]);
    assert_eq!(record.item_count, 5);
    f.provider.take_fetched();
    f.provider.bump("111");
    for _ in 0..2 {
        let report = refresh(
            service,
            LibraryRefreshRequest::Follow {
                follow_id: follow_id.clone(),
            },
        )
        .await;
        assert_eq!((report.new, report.removed_at_source), (0, 0));
        assert!(!pages(service).await.contains_key("111"));
    }
    assert!(f.provider.take_fetched().is_empty());
    // Following the whole space again clears exclusions (OQ4).
    let (again, same) = follow(service, "SD").await;
    assert_eq!(same, follow_id);
    assert_eq!(again.report.unwrap().new, 1);
    assert!(
        service.listing(None).await.unwrap().follows[0]
            .excluded_ids
            .is_empty()
    );
    assert!(refs::has_follow(&pages(service).await["111"], &follow_id));

    // Stop following keeps the pages as ordinary items.
    service
        .remove(LibraryRemoveRequest::StopFollowing {
            follow_id: follow_id.clone(),
        })
        .await
        .unwrap();
    let listing = service.listing(None).await.unwrap();
    assert!(listing.follows.is_empty());
    assert_eq!(listing.items.len(), 6);
    assert!(
        listing
            .items
            .iter()
            .all(|item| item.refs == [LibraryItemRef::Manual])
    );
    // Following again adopts them; they keep their Manual reference, so
    // removing the space forgets the record but keeps every page.
    let (again, _) = follow(service, "SD").await;
    assert_eq!(again.report.unwrap().new, 0);
    assert!(
        pages(service)
            .await
            .values()
            .all(|page| refs::has_follow(page, &follow_id))
    );
    let listing = service
        .remove(LibraryRemoveRequest::Follow {
            follow_id: follow_id.clone(),
        })
        .await
        .unwrap();
    assert!(listing.follows.is_empty() && listing.items.len() == 6);
    assert!(
        listing
            .items
            .iter()
            .all(|item| item.refs == [LibraryItemRef::Manual])
    );
    assert_eq!(
        service
            .remove(LibraryRemoveRequest::Follow { follow_id })
            .await
            .unwrap_err()
            .code,
        "library_item_not_found"
    );
}

#[tokio::test]
async fn remove_space_deletes_only_pages_the_follow_alone_holds() {
    let f = fixture(CLOUD, true, 200);
    let service = &f.base.service;
    let (_, follow_id) = follow(service, "SD").await;
    // Adding a followed page by hand gives it a second reference.
    let manual = LibraryAddRequest {
        follow: false,
        ..follow_request("200", None)
    };
    finished(service, service.start_add(manual).await.unwrap()).await;
    let kept = pages(service).await["200"].clone();
    assert_eq!(kept.refs.len(), 2);

    let listing = service
        .remove(LibraryRemoveRequest::Follow { follow_id })
        .await
        .unwrap();
    assert!(listing.follows.is_empty());
    assert_eq!(
        listing.items.len(),
        1,
        "{:?}",
        listing.items.iter().map(|i| &i.title).collect::<Vec<_>>()
    );
    assert_eq!(listing.items[0].item_id, kept.item_id);
    assert_eq!(listing.items[0].refs, [LibraryItemRef::Manual]);
    assert_eq!(listing.items[0].purge_after, None);
}

#[tokio::test]
async fn resolves_and_browses_spaces_of_the_selected_provider() {
    let f = fixture(CLOUD, true, 200);
    let service = &f.base.service;
    let resolved = service
        .resolve(LibraryResolveRequest {
            input: "SD".into(),
            provider_id: Some("confluence".into()),
        })
        .await
        .unwrap();
    assert_eq!(resolved.kind, LibraryInputKind::ConfluenceSpace);
    assert_eq!(resolved.canonical_id.as_deref(), Some("SD"));
    assert_eq!(resolved.title, "Software Development");
    assert_eq!(
        resolved.container_label.as_deref(),
        Some("SD · Software Development")
    );
    assert_eq!(
        (resolved.item_count, resolved.existing_follow_id),
        (Some(6), None)
    );
    let (_, follow_id) = follow(service, "SD").await;
    let spaces = service.confluence_spaces("confluence").await.unwrap();
    assert_eq!(
        spaces
            .iter()
            .map(|s| s.canonical_id.as_deref().unwrap())
            .collect::<Vec<_>>(),
        ["SD", "OPS"]
    );
    assert_eq!(spaces[0].existing_follow_id.as_ref(), Some(&follow_id));
    assert_eq!(spaces[0].item_count, Some(6));
    assert_eq!(spaces[1].existing_follow_id, None);
    assert!(
        spaces
            .iter()
            .all(|s| s.provider_instance.as_deref() == Some(CLOUD))
    );
    let page = service
        .resolve(LibraryResolveRequest {
            input: "201".into(),
            provider_id: Some("confluence".into()),
        })
        .await
        .unwrap();
    assert_eq!(page.existing_follow_id.as_ref(), Some(&follow_id));
    assert_eq!(
        service.confluence_spaces("other").await.unwrap_err().code,
        "source_provider_unsupported"
    );
    // Following through a page follows its space.
    let operation = finished(
        service,
        service
            .start_add(follow_request("301", None))
            .await
            .unwrap(),
    )
    .await;
    assert_eq!(operation.report.unwrap().new, 2);
    assert!(
        service
            .listing(None)
            .await
            .unwrap()
            .follows
            .iter()
            .any(|f| refs::space_key(f) == Some("OPS"))
    );
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
        service
            .start_add(follow_request("SD", Some(target())))
            .await
            .unwrap(),
    )
    .await;
    let saved_ids = vec![membership["110"].item_id.clone()];
    assert!(
        added
            .phases
            .iter()
            .all(|phase| phase.state == LibraryPhaseState::Done),
        "{added:?}"
    );
    assert_eq!(added.item_ids, saved_ids);
    assert_eq!(added.space.unwrap().item_ids, saved_ids);
    let selected = service
        .space_listing(&target())
        .await
        .unwrap()
        .items
        .into_iter()
        .map(|item| item.item_id)
        .collect::<Vec<_>>();
    assert_eq!(selected, saved_ids);
    let follow = service
        .listing(None)
        .await
        .unwrap()
        .follows
        .into_iter()
        .find(|follow| follow.follow_id == follow_id)
        .unwrap();
    assert_eq!(
        follow.item_count, 6,
        "selection leaves the global follow intact"
    );
}
