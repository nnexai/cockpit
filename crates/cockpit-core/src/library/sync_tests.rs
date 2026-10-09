//! Real scheduler regressions: providers are fake, persistence and publication are not.
use super::{
    LibraryService, follow, item_id, refs,
    store::{self, Index, LibraryIndexEntry},
    tests::{self as base, finished},
};
use crate::{
    InspectionError,
    config::LibrarySyncConfiguration,
    sources::{
        self, ConfluencePage, FrontmatterField, FrontmatterValue, IssueListing, IssueQuery,
        IssueRow, PageAux, ProviderResolution, SourceAsset, SourceAttachment, SourceContainer,
        SourceFetchRequest, SourceProvider, SourceRef, SourceService, SpacePage, SpacePageListing,
        SpaceSummary,
    },
};
use async_trait::async_trait;
use cockpit_protocol::{
    library::*,
    projects::{ProjectProvider, ProviderDeployment, ProviderKind},
    sources::SourceCapability,
};
use parking_lot::{Mutex, MutexGuard};
use serde_json::{Value, json};
use std::{
    collections::{BTreeMap, BTreeSet},
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
};
use tokio::sync::{Notify, Semaphore};

const JIRA: &str = "https://jira.example.test";
const WIKI: &str = "https://acme.atlassian.net/wiki";
const QUERY: &str = "project = OPS";
const MINUTE: i64 = 60_000;
const DAY: i64 = 24 * 60 * MINUTE;
fn now() -> i64 {
    crate::jira_query::instant_seconds("2026-03-01T12:00:00Z").unwrap() * 1000
}
fn updated(revision: u32) -> String {
    format!("2026-03-01 11:{revision:02}:00")
}
fn iso(revision: u32) -> String {
    format!("2026-03-01T11:{revision:02}:00.000+0000")
}
fn policy() -> LibrarySyncConfiguration {
    LibrarySyncConfiguration {
        lag_allowance_minutes: 0,
        ..Default::default()
    }
}
fn failure(code: &str) -> InspectionError {
    InspectionError::new(code, "deterministic provider failure")
}

#[derive(Clone)]
struct Issue {
    revision: u32,
    body: String,
}
#[derive(Default)]
struct JiraState {
    issues: BTreeMap<String, Issue>,
    queries: BTreeMap<String, Vec<String>>,
    delta: Option<Vec<String>>,
    partial: bool,
    list_failure: bool,
    broken: BTreeSet<String>,
    calls: Vec<(String, Option<(i64, i64)>)>,
    batches: Vec<Vec<String>>,
    fetched: Vec<String>,
}
struct FakeJira {
    state: Mutex<JiraState>,
    block: AtomicBool,
    entered: Notify,
    release: Semaphore,
    block_fetch_once: AtomicBool,
    fetch_entered: Notify,
    fetch_release: Semaphore,
}
impl FakeJira {
    fn new() -> Self {
        Self {
            state: Mutex::new(JiraState::default()),
            block: AtomicBool::new(false),
            entered: Notify::new(),
            release: Semaphore::new(0),
            block_fetch_once: AtomicBool::new(false),
            fetch_entered: Notify::new(),
            fetch_release: Semaphore::new(0),
        }
    }
    fn state(&self) -> MutexGuard<'_, JiraState> {
        self.state.lock()
    }
    fn insert(&self, key: &str, revision: u32) {
        self.state().issues.insert(
            key.into(),
            Issue {
                revision,
                body: format!("{key} body {revision}"),
            },
        );
    }
    fn query(&self, query: &str, keys: &[&str]) {
        self.state()
            .queries
            .insert(query.into(), keys.iter().map(|s| (*s).into()).collect());
    }
    fn clear_calls(&self) {
        let mut s = self.state();
        s.calls.clear();
        s.batches.clear();
        s.fetched.clear();
    }
}
fn issue_row(key: &str, issue: &Issue) -> IssueRow {
    IssueRow {
        key: key.into(),
        updated: updated(issue.revision),
        status: "Open".into(),
        issue_type: "Task".into(),
        assignee: None,
    }
}
#[async_trait]
impl SourceProvider for FakeJira {
    fn provider_id(&self) -> &str {
        "jira"
    }
    fn capabilities(&self) -> Vec<SourceCapability> {
        vec![]
    }
    async fn list_issues(
        &self,
        query: &IssueQuery<'_>,
        max: u32,
        _cancel: &AtomicBool,
    ) -> Result<IssueListing, InspectionError> {
        if self.block.load(Ordering::SeqCst) {
            self.entered.notify_one();
            self.release.acquire().await.unwrap().forget();
        }
        let mut s = self.state();
        let keys = match query {
            IssueQuery::Jql {
                jql,
                updated_window,
            } => {
                s.calls.push(((*jql).into(), *updated_window));
                if s.list_failure {
                    return Err(failure("source_provider_failed"));
                }
                if updated_window.is_some() {
                    s.delta
                        .clone()
                        .unwrap_or_else(|| s.queries.get(*jql).cloned().unwrap_or_default())
                } else {
                    s.queries.get(*jql).cloned().unwrap_or_default()
                }
            }
            IssueQuery::Keys(keys) => {
                s.batches.push(keys.to_vec());
                keys.to_vec()
            }
        };
        let rows = keys
            .iter()
            .filter_map(|key| s.issues.get(key).map(|issue| issue_row(key, issue)))
            .take(max as usize)
            .collect();
        Ok(IssueListing {
            rows,
            complete: !s.partial && keys.len() <= max as usize,
        })
    }
    async fn fetch(
        &self,
        request: &SourceFetchRequest,
    ) -> Result<Vec<SourceAsset>, InspectionError> {
        if self.block_fetch_once.swap(false, Ordering::SeqCst) {
            self.fetch_entered.notify_one();
            self.fetch_release.acquire().await.unwrap().forget();
        }
        let key = request.artifact_url.rsplit('/').next().unwrap();
        let mut s = self.state();
        s.fetched.push(key.into());
        if s.broken.contains(key) {
            return Err(failure("source_provider_failed"));
        }
        let issue = s
            .issues
            .get(key)
            .ok_or_else(|| failure("source_not_found"))?;
        Ok(vec![SourceAsset {
            source: SourceRef {
                provider_id: "jira".into(),
                provider_instance: request.authority.provider_instance.clone(),
                resource_type: "issue".into(),
                canonical_id: key.into(),
            },
            title: format!("Issue {key}"),
            source_url: Some(format!("{JIRA}/browse/{key}")),
            original_url: None,
            source_revision: Some(iso(issue.revision)),
            complete: true,
            diagnostics: vec![],
            body: issue.body.clone(),
            container: Some(SourceContainer {
                id: "OPS".into(),
                label: "OPS".into(),
            }),
            fields: vec![],
            attachments: vec![],
        }])
    }
}

#[derive(Clone)]
struct Page {
    version: u64,
    labels: Vec<String>,
    attachments: Vec<SourceAttachment>,
    ancestors: Vec<String>,
}
#[derive(Default)]
struct WikiState {
    pages: BTreeMap<String, Page>,
    delta: Vec<String>,
    batches: Vec<Vec<String>>,
    inventories: usize,
    deltas: Vec<(i64, i64)>,
    fetched: Vec<String>,
    aux: Vec<String>,
    broken_aux: BTreeSet<String>,
    budget: Option<MetadataBudget>,
    partial_versions: bool,
    rate_limited_fetch: BTreeSet<String>,
}
struct MetadataBudget {
    remaining: u32,
    per_page: u32,
    blocked_until: Option<i64>,
}
fn charge_metadata(s: &mut WikiState, requests: u32) -> Result<(), InspectionError> {
    if sources::lane::current() != sources::lane::RequestLane::Background {
        return Ok(());
    }
    if let Some(budget) = &mut s.budget {
        if requests > budget.remaining {
            budget.remaining = 0;
            budget.blocked_until = Some(now() + 60 * MINUTE);
            return Err(failure("source_rate_limited"));
        }
        budget.remaining -= requests;
    }
    Ok(())
}
struct FakeWiki(Mutex<WikiState>);
impl FakeWiki {
    fn state(&self) -> MutexGuard<'_, WikiState> {
        self.0.lock()
    }
    fn insert(&self, id: &str) {
        self.state().pages.insert(
            id.into(),
            Page {
                version: 1,
                labels: vec![],
                attachments: vec![],
                ancestors: vec![],
            },
        );
    }
    fn clear_calls(&self) {
        let mut s = self.state();
        s.batches.clear();
        s.inventories = 0;
        s.deltas.clear();
        s.fetched.clear();
        s.aux.clear();
    }
}
fn page_row(id: &str, page: &Page) -> SpacePage {
    SpacePage {
        page_id: id.into(),
        title: format!("Page {id}"),
        version: page.version,
        ancestors: page.ancestors.clone(),
        position: None,
    }
}
fn page_listing(pages: Vec<SpacePage>, complete: bool) -> SpacePageListing {
    SpacePageListing {
        space_name: "Software Development".into(),
        homepage_id: None,
        total: Some(pages.len() as u64),
        pages,
        complete,
    }
}
#[async_trait]
impl SourceProvider for FakeWiki {
    fn provider_id(&self) -> &str {
        "confluence"
    }
    fn capabilities(&self) -> Vec<SourceCapability> {
        vec![]
    }
    fn blocked_until_ms(&self) -> Option<i64> {
        self.state()
            .budget
            .as_ref()
            .and_then(|budget| budget.blocked_until)
    }
    fn background_requests_remaining(&self) -> Option<u32> {
        self.state().budget.as_ref().map(|budget| budget.remaining)
    }
    async fn resolve_input(&self, input: &str) -> Result<ProviderResolution, InspectionError> {
        if input == "SD" {
            return Ok(ProviderResolution::ConfluenceSpace {
                space_key: "SD".into(),
            });
        }
        let page_id = match url::Url::parse(input) {
            Ok(url) => {
                let configured = url::Url::parse(WIKI).unwrap();
                if url.scheme() != configured.scheme()
                    || url.host_str() != configured.host_str()
                    || url.port_or_known_default() != configured.port_or_known_default()
                    || !url.username().is_empty()
                    || url.password().is_some()
                {
                    return Err(failure("source_authority_mismatch"));
                }
                url.query_pairs()
                    .find(|(key, _)| key == "pageId")
                    .map(|(_, value)| value.into_owned())
                    .or_else(|| {
                        url.path()
                            .strip_prefix("/wiki/spaces/SD/pages/")
                            .map(str::to_owned)
                    })
                    .ok_or_else(|| failure("library_input_unrecognized"))?
            }
            Err(_) => input.to_owned(),
        };
        let s = self.state();
        let page = s
            .pages
            .get(&page_id)
            .ok_or_else(|| failure("source_not_found"))?;
        Ok(ProviderResolution::ConfluencePage(ConfluencePage {
            page_id: page_id.clone(),
            space_key: "SD".into(),
            title: format!("Page {page_id}"),
            version: Some(page.version),
            source_url: format!("{WIKI}/spaces/SD/pages/{page_id}"),
            canonical_url: sources::confluence_page_url(WIKI, &page_id),
        }))
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
        max: u32,
        _cancel: &AtomicBool,
    ) -> Result<SpacePageListing, InspectionError> {
        assert_eq!(key, "SD");
        let mut s = self.state();
        s.inventories += 1;
        Ok(page_listing(
            s.pages
                .iter()
                .take(max as usize)
                .map(|(id, page)| page_row(id, page))
                .collect(),
            s.pages.len() <= max as usize,
        ))
    }
    async fn list_page_changes(
        &self,
        key: &str,
        lower: i64,
        upper: i64,
        max: u32,
        _cancel: &AtomicBool,
    ) -> Result<SpacePageListing, InspectionError> {
        assert_eq!(key, "SD");
        let mut s = self.state();
        s.deltas.push((lower, upper));
        let mut listing = page_listing(
            s.delta
                .iter()
                .filter_map(|id| s.pages.get(id).map(|page| page_row(id, page)))
                .take(max as usize)
                .collect(),
            s.delta.len() <= max as usize,
        );
        listing.space_name.clear();
        Ok(listing)
    }
    async fn page_versions(
        &self,
        ids: &[String],
        _cancel: &AtomicBool,
    ) -> Result<SpacePageListing, InspectionError> {
        let mut s = self.state();
        s.batches.push(ids.to_vec());
        let cost = s
            .budget
            .as_ref()
            .map_or(0, |budget| 1 + ids.len() as u32 * budget.per_page);
        charge_metadata(&mut s, cost)?;
        let mut listing = page_listing(
            ids.iter()
                .filter_map(|id| s.pages.get(id).map(|page| page_row(id, page)))
                .collect(),
            !s.partial_versions,
        );
        listing.space_name.clear();
        Ok(listing)
    }
    async fn page_aux(&self, id: &str) -> Result<PageAux, InspectionError> {
        let mut s = self.state();
        s.aux.push(id.into());
        if s.broken_aux.contains(id) {
            return Err(failure("source_provider_failed"));
        }
        let page = s.pages.get(id).ok_or_else(|| failure("source_not_found"))?;
        Ok(PageAux {
            labels: page.labels.clone(),
            labels_complete: true,
            attachments: page.attachments.clone(),
            attachments_complete: true,
        })
    }
    async fn page_space(&self, id: &str) -> Result<Option<String>, InspectionError> {
        Ok(self.state().pages.contains_key(id).then(|| "SD".into()))
    }
    async fn fetch(
        &self,
        request: &SourceFetchRequest,
    ) -> Result<Vec<SourceAsset>, InspectionError> {
        let id = request.artifact_url.split("pageId=").nth(1).unwrap();
        let mut s = self.state();
        s.fetched.push(id.into());
        charge_metadata(&mut s, 1)?;
        if s.rate_limited_fetch.contains(id) {
            s.budget.as_mut().unwrap().blocked_until = Some(now() + 60 * MINUTE);
            return Err(failure("source_rate_limited"));
        }
        let page = s.pages.get(id).ok_or_else(|| failure("source_not_found"))?;
        let mut fields = vec![
            FrontmatterField {
                key: "page_id".into(),
                value: FrontmatterValue::String(id.into()),
            },
            FrontmatterField {
                key: "labels".into(),
                value: FrontmatterValue::Strings(page.labels.clone()),
            },
            FrontmatterField {
                key: "ancestor_ids".into(),
                value: FrontmatterValue::Strings(page.ancestors.clone()),
            },
            FrontmatterField {
                key: "ancestors".into(),
                value: FrontmatterValue::Strings(
                    page.ancestors
                        .iter()
                        .map(|id| format!("Page {id}"))
                        .collect(),
                ),
            },
        ];
        if let Some(parent) = page.ancestors.last() {
            fields.push(FrontmatterField {
                key: "parent_id".into(),
                value: FrontmatterValue::String(parent.clone()),
            });
        }
        Ok(vec![SourceAsset {
            source: SourceRef {
                provider_id: "confluence".into(),
                provider_instance: request.authority.provider_instance.clone(),
                resource_type: "page".into(),
                canonical_id: id.into(),
            },
            title: format!("Page {id}"),
            source_url: Some(format!("{WIKI}/spaces/SD/pages/{id}")),
            original_url: None,
            source_revision: Some(page.version.to_string()),
            complete: true,
            diagnostics: vec![],
            body: format!("Page {id} body"),
            container: Some(SourceContainer {
                id: "SD".into(),
                label: "SD · Software Development".into(),
            }),
            fields,
            attachments: page.attachments.clone(),
        }])
    }
}

struct Fixture {
    base: base::Fixture,
    jira: Arc<FakeJira>,
    wiki: Arc<FakeWiki>,
}
impl Fixture {
    fn new() -> Self {
        let mut base = base::fixture();
        let mut configuration = base.service.configuration.clone();
        configuration.providers = vec![
            ProjectProvider {
                id: "jira".into(),
                kind: ProviderKind::Jira,
                base_url: JIRA.into(),
                executable: None,
                login: None,
                deployment: Some(ProviderDeployment::DataCenter),
            },
            ProjectProvider {
                id: "confluence".into(),
                kind: ProviderKind::Confluence,
                base_url: WIKI.into(),
                executable: None,
                login: None,
                deployment: Some(ProviderDeployment::Cloud),
            },
        ];
        configuration.limits.library_space_pages = 5000;
        let jira = Arc::new(FakeJira::new());
        let wiki = Arc::new(FakeWiki(Mutex::new(WikiState::default())));
        let sources =
            Arc::new(SourceService::new(&configuration, vec![jira.clone(), wiki.clone()]).unwrap());
        base.service = LibraryService::new(configuration, sources);
        Self { base, jira, wiki }
    }
    fn service(&self) -> &LibraryService {
        &self.base.service
    }
    fn restarted(&self) -> LibraryService {
        LibraryService::new(
            self.service().configuration.clone(),
            self.service().sources.clone(),
        )
    }
    fn index(&self) -> Index {
        let store = self.service().open().unwrap();
        let _lock = store.shared().unwrap();
        store.index().unwrap()
    }
    fn entry(&self, key: &str) -> LibraryIndexEntry {
        self.index()
            .items
            .into_iter()
            .find(|e| e.summary.canonical_id.as_deref() == Some(key))
            .unwrap()
    }
    fn document(&self, key: &str) -> std::path::PathBuf {
        self.base
            .root
            .join("library")
            .join(self.entry(key).summary.document_path.unwrap())
    }
    fn state_path(&self) -> std::path::PathBuf {
        self.base.root.join("library/.cockpit/sync/state.json")
    }
    fn state(&self) -> Value {
        serde_json::from_slice(&std::fs::read(self.state_path()).unwrap()).unwrap()
    }
    fn edit_state(&self, change: impl FnOnce(&mut Value)) {
        let mut state = if self.state_path().exists() {
            self.state()
        } else {
            json!({"schema":1,"sources":{},"queue":{},"audits":{},"origins":{},"active_operation":null})
        };
        change(&mut state);
        std::fs::create_dir_all(self.state_path().parent().unwrap()).unwrap();
        std::fs::write(
            self.state_path(),
            serde_json::to_vec_pretty(&state).unwrap(),
        )
        .unwrap();
    }
    fn quiet_audits(&self) {
        self.edit_state(|s| {
            for group in [format!("jira:{JIRA}"), format!("confluence:{WIKI}")] {
                s["audits"][group] =
                    json!({"cursor":null,"next_due_ms":now()+100*DAY,"failures":{}});
            }
        });
    }
    fn due(&self, id: &str, delta: bool, inventory: bool) {
        self.edit_state(|s| {
            if s["sources"].get(id).is_none() {
                s["sources"][id] = json!({
                    "next_delta_ms": 0,
                    "next_inventory_ms": 0,
                    "next_related_ms": 0,
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
            s["sources"][id]["next_delta_ms"] = json!(if delta { 0 } else { now() + 100 * DAY });
            s["sources"][id]["next_inventory_ms"] =
                json!(if inventory { 0 } else { now() + 100 * DAY });
        });
    }
    async fn add(
        &self,
        provider: &str,
        input: &str,
        follow: bool,
        mode: Option<LibraryFollowMode>,
    ) -> Option<String> {
        let request = LibraryAddRequest {
            input: input.into(),
            provider_id: Some(provider.into()),
            reference_depth: 0,
            follow,
            follow_mode: mode,
            download_attachments: false,
            refresh_existing: false,
            label: None,
            target: None,
        };
        let operation = finished(
            self.service(),
            self.service().start_add(request).await.unwrap(),
        )
        .await;
        assert!(
            operation.phases.iter().all(|p| p.error.is_none()),
            "{operation:?}"
        );
        if !follow {
            return None;
        }
        Some(
            self.index()
                .follows
                .into_iter()
                .find(|f| {
                    f.provider_id == provider
                        && match &f.source {
                            LibraryFollowSource::JiraQuery { jql, .. } => jql == input,
                            LibraryFollowSource::ConfluenceSpace { space_key, .. } => {
                                space_key == input
                            }
                        }
                })
                .unwrap()
                .follow_id,
        )
    }
}

#[tokio::test]
async fn complete_and_empty_windows_checkpoint_but_partial_and_failed_windows_stay_uncommitted() {
    let f = Fixture::new();
    f.jira.insert("OPS-1", 1);
    f.jira.query(QUERY, &["OPS-1"]);
    let id = f
        .add("jira", QUERY, true, Some(LibraryFollowMode::Live))
        .await
        .unwrap();
    f.quiet_audits();
    f.due(&id, true, false);
    f.jira.state().delta = Some(vec![]);
    f.service().sync_tick(policy(), now()).await.unwrap();
    assert_eq!(f.state()["sources"][&id]["committed_upper_ms"], now());
    assert!(f.state()["sources"][&id]["window"].is_null());
    f.jira.insert("OPS-1", 2);
    f.jira.state().delta = Some(vec!["OPS-1".into()]);
    f.due(&id, true, false);
    let report = f
        .service()
        .sync_tick(policy(), now() + 60 * MINUTE)
        .await
        .unwrap();
    assert_eq!(report.fetched, 1);
    assert_eq!(
        f.state()["sources"][&id]["committed_upper_ms"],
        now() + 60 * MINUTE
    );
    let mut small = policy();
    small.overlap_minutes = 0;
    f.edit_state(|s| {
        s["sources"][&id]["committed_upper_ms"] = json!(now() + 60 * MINUTE);
    });
    f.jira.state().partial = true;
    f.due(&id, true, false);
    f.service()
        .sync_tick(small.clone(), now() + 80 * MINUTE)
        .await
        .unwrap();
    let frozen = f.state()["sources"][&id]["window"].clone();
    assert_eq!(frozen, json!([now() + 60 * MINUTE, now() + 80 * MINUTE]));
    assert_eq!(
        f.state()["sources"][&id]["committed_upper_ms"],
        now() + 60 * MINUTE
    );
    f.jira.state().list_failure = true;
    f.due(&id, true, false);
    f.service()
        .sync_tick(small.clone(), now() + 90 * MINUTE)
        .await
        .unwrap();
    assert_eq!(f.state()["sources"][&id]["window"], frozen);
    {
        let mut s = f.jira.state();
        s.partial = false;
        s.list_failure = false;
        s.delta = Some(vec![]);
    }
    f.due(&id, true, false);
    f.service()
        .sync_tick(small, now() + 100 * MINUTE)
        .await
        .unwrap();
    assert_eq!(
        f.jira.state().calls.last().unwrap().1,
        Some((now() + 60 * MINUTE, now() + 80 * MINUTE))
    );
    assert_eq!(
        f.state()["sources"][&id]["committed_upper_ms"],
        now() + 80 * MINUTE
    );
}

#[tokio::test]
async fn failed_queue_survives_restart_and_overlapping_follows_fetch_each_revision_once() {
    let f = Fixture::new();
    f.jira.insert("OPS-1", 1);
    f.jira.query(QUERY, &["OPS-1"]);
    f.jira.query("status = Open", &["OPS-1"]);
    let a = f
        .add("jira", QUERY, true, Some(LibraryFollowMode::Live))
        .await
        .unwrap();
    let b = f
        .add("jira", "status = Open", true, Some(LibraryFollowMode::Live))
        .await
        .unwrap();
    f.quiet_audits();
    f.jira.clear_calls();
    f.jira.insert("OPS-1", 3);
    f.jira.state().broken.insert("OPS-1".into());
    let report = f.service().sync_tick(policy(), now()).await.unwrap();
    assert_eq!(report.pending, 1);
    assert_eq!(f.jira.state().fetched, ["OPS-1"]);
    let queue = f.state()["queue"].as_object().unwrap().clone();
    let candidate = queue.values().next().unwrap();
    assert_eq!(candidate["owners"].as_array().unwrap().len(), 2);
    assert_eq!(candidate["failures"], 1);
    assert_eq!(candidate["revision"], updated(3));
    assert_eq!(candidate["next_attempt_ms"], now() + MINUTE);
    f.jira.state().broken.clear();
    f.jira.clear_calls();
    let restarted = f.restarted();
    // A delayed overlapping listing must not rewind a newer durable candidate
    // or reset its retry deadline after service reconstruction.
    f.jira.insert("OPS-1", 2);
    f.due(&a, true, false);
    f.due(&b, true, false);
    restarted
        .sync_tick(policy(), now() + MINUTE / 2)
        .await
        .unwrap();
    let overlap_state = f.state();
    let overlap = overlap_state["queue"]
        .as_object()
        .unwrap()
        .values()
        .next()
        .unwrap();
    assert_eq!(overlap["revision"], updated(3));
    assert_eq!(overlap["row"]["updated"], updated(3));
    assert_eq!(overlap["next_attempt_ms"], now() + MINUTE);
    assert_eq!(overlap["failures"], 1);
    assert!(f.jira.state().fetched.is_empty());
    f.jira.insert("OPS-1", 3);
    let before_retry = restarted
        .sync_tick(policy(), now() + MINUTE - 1)
        .await
        .unwrap();
    assert_eq!(before_retry.pending, 1);
    assert!(f.jira.state().fetched.is_empty());
    let retried = restarted.sync_tick(policy(), now() + MINUTE).await.unwrap();
    assert_eq!(retried.fetched, 1);
    assert_eq!(retried.pending, 0);
    assert_eq!(f.jira.state().fetched, ["OPS-1"]);
    let saved = f.entry("OPS-1");
    assert_eq!(
        saved.summary.issue.as_ref().unwrap().fetched_updated,
        Some(updated(3))
    );
    assert!(refs::has_follow(&saved.summary, &a));
    assert!(refs::has_follow(&saved.summary, &b));
    let bytes = std::fs::read(f.document("OPS-1")).unwrap();
    assert!(String::from_utf8_lossy(&bytes).contains("OPS-1 body 3"));
    let generation = f.index().generation;
    f.jira.clear_calls();
    f.due(&a, true, true);
    f.due(&b, true, true);
    restarted
        .sync_tick(policy(), now() + 2 * MINUTE)
        .await
        .unwrap();
    assert!(f.jira.state().fetched.is_empty());
    assert_eq!(f.index().generation, generation);
}

#[tokio::test]
async fn only_two_complete_inventories_drop_one_follow_ref_without_erasing_shared_or_excluded_items()
 {
    let f = Fixture::new();
    for key in ["OPS-1", "OPS-2", "OPS-3"] {
        f.jira.insert(key, 1);
    }
    f.jira.query(QUERY, &["OPS-1", "OPS-2", "OPS-3"]);
    f.jira.query("status = Open", &["OPS-1", "OPS-2"]);
    let a = f
        .add("jira", QUERY, true, Some(LibraryFollowMode::Live))
        .await
        .unwrap();
    let b = f
        .add("jira", "status = Open", true, Some(LibraryFollowMode::Live))
        .await
        .unwrap();
    let store = f.service().open().unwrap();
    store
        .mutate_index(|index| {
            index
                .follows
                .iter_mut()
                .find(|r| r.follow_id == a)
                .unwrap()
                .excluded_ids
                .push("OPS-2".into());
            for entry in &mut index.items {
                if entry.summary.canonical_id.as_deref() == Some("OPS-2") {
                    refs::remove_ref(
                        &mut entry.summary,
                        &LibraryItemRef::Follow {
                            follow_id: a.clone(),
                        },
                    );
                }
            }
            follow::recount(index, &a);
            Ok(())
        })
        .unwrap();
    f.quiet_audits();
    f.due(&b, false, false);
    f.jira.query(QUERY, &["OPS-2", "OPS-3"]);
    f.due(&a, false, true);
    f.service().sync_tick(policy(), now()).await.unwrap();
    assert!(refs::has_follow(&f.entry("OPS-1").summary, &a));
    assert_eq!(f.state()["sources"][&a]["absent"]["OPS-1"], now());
    f.jira.state().partial = true;
    f.due(&a, false, true);
    f.service().sync_tick(policy(), now() + DAY).await.unwrap();
    assert!(refs::has_follow(&f.entry("OPS-1").summary, &a));
    f.jira.state().partial = false;
    f.due(&a, false, true);
    f.service()
        .sync_tick(policy(), now() + 2 * DAY)
        .await
        .unwrap();
    let one = f.entry("OPS-1").summary;
    assert!(!refs::has_follow(&one, &a));
    assert!(refs::has_follow(&one, &b));
    assert!(one.purge_after.is_none());
    let two = f.entry("OPS-2").summary;
    assert!(!refs::has_follow(&two, &a));
    assert!(refs::has_follow(&two, &b));
    assert!(f.document("OPS-1").is_file());
    assert!(f.document("OPS-2").is_file());
}

#[tokio::test]
async fn queued_work_rechecks_exclusions_and_preserves_local_edit_conflicts() {
    let f = Fixture::new();
    for key in ["OPS-1", "OPS-2"] {
        f.jira.insert(key, 1);
    }
    f.jira.query(QUERY, &["OPS-1", "OPS-2"]);
    let id = f
        .add("jira", QUERY, true, Some(LibraryFollowMode::Live))
        .await
        .unwrap();
    f.quiet_audits();
    f.jira.clear_calls();
    for key in ["OPS-1", "OPS-2"] {
        f.jira.insert(key, 2);
        f.jira.state().broken.insert(key.into());
    }
    f.service().sync_tick(policy(), now()).await.unwrap();
    assert_eq!(f.state()["queue"].as_object().unwrap().len(), 2);
    let path = f.document("OPS-1");
    let local = b"User's locally edited document\n";
    std::fs::write(&path, local).unwrap();
    let store = f.service().open().unwrap();
    store
        .mutate_index(|index| {
            index
                .follows
                .iter_mut()
                .find(|r| r.follow_id == id)
                .unwrap()
                .excluded_ids
                .push("OPS-2".into());
            Ok(())
        })
        .unwrap();
    f.jira.state().broken.clear();
    f.jira.clear_calls();
    let report = f
        .restarted()
        .sync_tick(policy(), now() + MINUTE)
        .await
        .unwrap();
    assert_eq!(std::fs::read(&path).unwrap(), local);
    assert_eq!(f.entry("OPS-1").summary.state, LibraryItemState::Conflict);
    assert!(refs::has_follow(&f.entry("OPS-1").summary, &id));
    assert!(f.jira.state().fetched.is_empty());
    assert_eq!(report.pending, 1);
    let state = f.state();
    let work = state["queue"].as_object().unwrap().values().next().unwrap();
    assert_eq!(work["source"]["canonical_id"], "OPS-1");
    assert_eq!(work["last_error"], "library_conflict");
    assert_eq!(
        f.entry("OPS-2").summary.source_revision.as_deref(),
        Some(iso(1).as_str())
    );
}

#[tokio::test]
async fn daily_accumulate_checks_and_refreshes_tracked_issues_after_they_exit_the_predicate() {
    let f = Fixture::new();
    for key in ["OPS-1", "OPS-2"] {
        f.jira.insert(key, 1);
    }
    f.jira.query(QUERY, &["OPS-1", "OPS-2"]);
    let id = f
        .add("jira", QUERY, true, Some(LibraryFollowMode::Accumulate))
        .await
        .unwrap();
    f.quiet_audits();
    f.jira.clear_calls();
    f.jira.query(QUERY, &["OPS-2"]);
    f.jira.state().delta = Some(vec![]);
    f.jira.insert("OPS-1", 2);
    f.due(&id, true, false);
    f.service().sync_tick(policy(), now()).await.unwrap();
    assert!(f.jira.state().batches.is_empty());
    assert!(f.jira.state().fetched.is_empty());
    f.due(&id, false, true);
    let report = f.service().sync_tick(policy(), now() + DAY).await.unwrap();
    assert_eq!(f.jira.state().batches, [vec!["OPS-1".to_owned()]]);
    assert_eq!(report.fetched, 1);
    assert!(refs::has_follow(&f.entry("OPS-1").summary, &id));
    assert_eq!(
        f.entry("OPS-1").summary.issue.unwrap().fetched_updated,
        Some(updated(2))
    );
    assert!(
        std::fs::read_to_string(f.document("OPS-1"))
            .unwrap()
            .contains("OPS-1 body 2")
    );
}

// Seed real Store metadata and owned documents from one actual provider publication.
// This avoids 4000 imports/body requests while exercising the production index reader.
fn seed_items(f: &Fixture, provider: &str, count: usize, follow_id: Option<&str>) {
    let store = f.service().open().unwrap();
    let _lock = store.exclusive().unwrap();
    let mut index = store.index().unwrap();
    let template = index
        .items
        .iter()
        .find(|entry| entry.summary.provider_id.as_deref() == Some(provider))
        .unwrap()
        .clone();
    let document = template
        .summary
        .document_path
        .as_ref()
        .unwrap()
        .rsplit('/')
        .next()
        .unwrap();
    let bytes = std::fs::read(
        store
            .path
            .join(template.summary.document_path.as_ref().unwrap()),
    )
    .unwrap();
    index
        .items
        .retain(|entry| entry.summary.provider_id.as_deref() != Some(provider));
    for n in 1..=count {
        let key = if provider == "jira" {
            format!("OPS-{n}")
        } else {
            n.to_string()
        };
        let source = SourceRef {
            provider_id: provider.into(),
            provider_instance: if provider == "jira" {
                JIRA.into()
            } else {
                WIKI.into()
            },
            resource_type: if provider == "jira" {
                "issue".into()
            } else {
                "page".into()
            },
            canonical_id: key.clone(),
        };
        let mut entry = template.clone();
        entry.summary.item_id = item_id(&source);
        entry.summary.logical_id = entry.summary.item_id.clone();
        entry.summary.canonical_id = Some(key.clone());
        entry.summary.title = if provider == "jira" {
            format!("Issue {key}")
        } else {
            format!("Page {key}")
        };
        entry.summary.item_path = format!("{provider}/seed/{key}");
        entry.summary.document_path = Some(format!("{}/{document}", entry.summary.item_path));
        entry.summary.refs = vec![match follow_id {
            Some(id) => LibraryItemRef::Follow {
                follow_id: id.into(),
            },
            None => LibraryItemRef::Manual,
        }];
        entry.summary.parent_item_id = None;
        entry.summary.ancestors.clear();
        entry.summary.order = None;
        if provider == "jira" {
            entry.summary.issue = Some(LibraryIssueMeta {
                updated: updated(1),
                fetched_updated: Some(updated(1)),
                status: "Open".into(),
                issue_type: "Task".into(),
                assignee: None,
            });
            entry.relations_captured = true;
        }
        entry.summary.source_url = Some(if provider == "jira" {
            format!("{JIRA}/browse/{key}")
        } else {
            format!("{WIKI}/spaces/SD/pages/{key}")
        });
        entry.canonical_url = Some(if provider == "jira" {
            format!("{JIRA}/browse/{key}")
        } else {
            sources::confluence_page_url(WIKI, &key)
        });
        std::fs::create_dir_all(store.path.join(&entry.summary.item_path)).unwrap();
        std::fs::write(
            store
                .path
                .join(entry.summary.document_path.as_ref().unwrap()),
            &bytes,
        )
        .unwrap();
        if provider == "jira" {
            f.jira.insert(&key, 1);
        } else {
            f.wiki.insert(&key);
        }
        index.items.push(entry);
    }
    if let Some(id) = follow_id {
        follow::recount(&mut index, id);
    }
    store::bounded_write(&store.meta, "index.json", &index, 64 * 1024 * 1024).unwrap();
}

#[tokio::test]
async fn standalone_metadata_checks_batch_ids_without_fetching_unchanged_bodies() {
    let f = Fixture::new();
    f.jira.insert("OPS-1", 1);
    f.wiki.insert("1");
    f.add("jira", &format!("{JIRA}/browse/OPS-1"), false, None)
        .await;
    f.add("confluence", "1", false, None).await;
    seed_items(&f, "jira", 205, None);
    seed_items(&f, "confluence", 205, None);
    f.quiet_audits();
    f.jira.clear_calls();
    f.wiki.clear_calls();
    let generation = f.index().generation;
    let report = f.service().sync_tick(policy(), now()).await.unwrap();
    let jira = f.jira.state();
    let wiki = f.wiki.state();
    for batches in [&jira.batches, &wiki.batches] {
        assert_eq!(
            batches.iter().map(Vec::len).collect::<Vec<_>>(),
            [100, 100, 5]
        );
        assert_eq!(batches.iter().flatten().collect::<BTreeSet<_>>().len(), 205);
    }
    assert!(jira.fetched.is_empty());
    assert!(wiki.fetched.is_empty());
    assert_eq!(report.fetched, 0);
    assert_eq!(report.pending, 0);
    assert_eq!(f.index().generation, generation);
}

#[tokio::test]
async fn auxiliary_only_labels_and_first_attachment_are_discovered_even_at_unchanged_page_version()
{
    let f = Fixture::new();
    f.wiki.insert("1");
    let id = f.add("confluence", "SD", true, None).await.unwrap();
    f.due(&id, false, false);
    f.wiki.clear_calls();
    {
        let mut s = f.wiki.state();
        let p = s.pages.get_mut("1").unwrap();
        p.labels = vec!["new-label".into()];
        p.attachments.push(SourceAttachment {
            id: "17".into(),
            title: "first.txt".into(),
            media_type: Some("text/plain".into()),
            size: Some(4),
            source_url: None,
            source_revision: Some("1".into()),
            path: None,
            not_downloaded: Some("not_requested".into()),
        });
    }
    let generation = f.index().generation;
    let discovery = f.service().sync_tick(policy(), now()).await.unwrap();
    assert_eq!(f.wiki.state().aux, ["1"]);
    assert!(f.wiki.state().fetched.is_empty());
    assert_eq!(discovery.pending, 1);
    assert_eq!(f.index().generation, generation);
    let publication = f.restarted().sync_tick(policy(), now() + 1).await.unwrap();
    assert_eq!(publication.fetched, 1);
    assert_eq!(publication.pending, 0);
    assert_eq!(f.wiki.state().fetched, ["1"]);
    let entry = f.entry("1");
    assert_eq!(entry.summary.source_revision.as_deref(), Some("1"));
    assert_eq!(entry.summary.attachments.len(), 1);
    assert_eq!(entry.summary.attachments[0].attachment_id, "17");
    assert_eq!(entry.summary.attachments[0].original_name, "first.txt");
    assert!(
        std::fs::read_to_string(f.document("1"))
            .unwrap()
            .contains("new-label")
    );
    assert_ne!(f.index().generation, generation);
}

#[tokio::test]
async fn user_scale_quiet_inventory_and_delta_do_not_refetch_bodies_or_rewrite_index_generation() {
    let f = Fixture::new();
    f.jira.insert("OPS-1", 1);
    f.wiki.insert("1");
    f.jira.query(QUERY, &["OPS-1"]);
    let jira_id = f
        .add("jira", QUERY, true, Some(LibraryFollowMode::Live))
        .await
        .unwrap();
    let wiki_id = f.add("confluence", "SD", true, None).await.unwrap();
    seed_items(&f, "jira", 3000, Some(&jira_id));
    seed_items(&f, "confluence", 1000, Some(&wiki_id));
    {
        let mut s = f.jira.state();
        s.queries.insert(
            QUERY.into(),
            (1..=3000).map(|n| format!("OPS-{n}")).collect(),
        );
        s.delta = Some(vec![]);
    }
    // Jira's rolling audit deliberately fetches content: persist its real future
    // due time, rather than bypassing that production path in the engine.
    f.quiet_audits();
    f.jira.clear_calls();
    f.wiki.clear_calls();
    let before = f.index();
    assert_eq!(before.items.len(), 4000);
    let index_path = f.base.root.join("library/.cockpit/index.json");
    let index_bytes = std::fs::read(&index_path).unwrap();
    let inventory = f.service().sync_tick(policy(), now()).await.unwrap();
    assert_eq!(inventory.discovered, 0);
    assert_eq!(inventory.fetched, 0);
    assert_eq!(inventory.pending, 0);
    assert_eq!(f.jira.state().calls.len(), 2);
    assert_eq!(f.wiki.state().inventories, 1);
    assert_eq!(f.wiki.state().deltas.len(), 1);
    assert!(f.jira.state().fetched.is_empty());
    assert!(f.wiki.state().fetched.is_empty());
    assert!(f.wiki.state().aux.is_empty());
    assert_eq!(f.index().generation, before.generation);
    assert_eq!(std::fs::read(&index_path).unwrap(), index_bytes);
    f.jira.clear_calls();
    f.wiki.clear_calls();
    f.due(&jira_id, true, false);
    f.due(&wiki_id, true, false);
    let delta = f
        .restarted()
        .sync_tick(policy(), now() + 60 * MINUTE)
        .await
        .unwrap();
    assert_eq!(delta.discovered, 0);
    assert_eq!(delta.fetched, 0);
    assert_eq!(delta.pending, 0);
    assert_eq!(f.jira.state().calls.len(), 1);
    assert!(f.jira.state().calls[0].1.is_some());
    assert_eq!(f.wiki.state().deltas.len(), 1);
    assert_eq!(f.wiki.state().inventories, 0);
    assert!(f.jira.state().fetched.is_empty());
    assert!(f.wiki.state().fetched.is_empty());
    assert!(f.wiki.state().aux.is_empty());
    assert_eq!(f.index().generation, before.generation);
    assert_eq!(std::fs::read(&index_path).unwrap(), index_bytes);
    assert_eq!(
        f.state()["sources"][&jira_id]["committed_upper_ms"],
        now() + 60 * MINUTE
    );
    assert_eq!(
        f.state()["sources"][&wiki_id]["committed_upper_ms"],
        now() + 60 * MINUTE
    );
}

#[tokio::test]
async fn disabled_sync_does_no_io_even_with_invalid_policy_and_missing_library_root() {
    let f = Fixture::new();
    let root = f.base.root.join("library");
    assert!(!root.exists());
    let mut disabled = policy();
    disabled.enabled = false;
    disabled.delta_minutes = 0;
    let tick = f.service().sync_tick(disabled.clone(), -1).await.unwrap();
    let _runtime = f.service().start_sync(disabled).unwrap();
    assert_eq!(tick.discovered, 0);
    assert_eq!(tick.fetched, 0);
    assert_eq!(tick.pending, 0);
    assert!(!root.exists());
    assert!(f.jira.state().calls.is_empty());
    assert!(f.jira.state().fetched.is_empty());
    assert_eq!(f.wiki.state().inventories, 0);
    assert!(f.wiki.state().aux.is_empty());
}

#[tokio::test]
async fn root_scheduler_lease_excludes_a_second_service_while_the_first_waits_for_remote_io() {
    let f = Fixture::new();
    f.jira.insert("OPS-1", 1);
    f.jira.query(QUERY, &["OPS-1"]);
    f.add("jira", QUERY, true, Some(LibraryFollowMode::Live))
        .await;
    f.quiet_audits();
    f.jira.clear_calls();
    f.jira.block.store(true, Ordering::SeqCst);
    let first_service = f.restarted();
    let first = tokio::spawn(async move { first_service.sync_tick(policy(), now()).await });
    tokio::time::timeout(std::time::Duration::from_secs(5), f.jira.entered.notified())
        .await
        .unwrap();
    let second_service = f.restarted();
    let error = second_service.sync_tick(policy(), now()).await.unwrap_err();
    assert_eq!(error.code, "library_item_busy");
    assert!(f.jira.state().calls.is_empty());
    f.jira.block.store(false, Ordering::SeqCst);
    f.jira.release.add_permits(1);
    first.await.unwrap().unwrap();
    let calls = f.jira.state().calls.len();
    assert_eq!(calls, 2);
    second_service.sync_tick(policy(), now() + 1).await.unwrap();
    assert_eq!(f.jira.state().calls.len(), calls);
}

#[tokio::test]
async fn dropping_enabled_runtime_guard_aborts_startup_without_provider_or_state_io() {
    let f = Fixture::new();
    let root = f.base.root.join("library");
    assert!(!root.exists());
    let runtime = f.service().start_sync(policy()).unwrap();
    // Give the actual spawned scheduler a turn to arm its startup delay.
    // Dropping the guard aborts that task; there is no 60-second wall-time wait.
    tokio::task::yield_now().await;
    drop(runtime);
    tokio::task::yield_now().await;
    assert!(!root.exists());
    assert!(!f.state_path().exists());
    assert!(f.jira.state().calls.is_empty());
    assert!(f.jira.state().fetched.is_empty());
    assert_eq!(f.wiki.state().inventories, 0);
    assert!(f.wiki.state().aux.is_empty());
}

#[tokio::test]
async fn manual_refresh_preempts_blocked_background_fetch_without_releasing_remote_gate() {
    let f = Fixture::new();
    f.jira.insert("OPS-1", 1);
    f.jira.query(QUERY, &["OPS-1"]);
    let follow_id = f
        .add("jira", QUERY, true, Some(LibraryFollowMode::Live))
        .await
        .unwrap();
    let item_id = f.entry("OPS-1").summary.item_id;
    let store = f.service().open().unwrap();
    store
        .mutate_index(|index| {
            let entry = index
                .items
                .iter_mut()
                .find(|e| e.summary.item_id == item_id)
                .unwrap();
            refs::insert_ref(&mut entry.summary, LibraryItemRef::Manual);
            Ok(())
        })
        .unwrap();
    let original_refs = f.entry("OPS-1").summary.refs;
    f.quiet_audits();
    f.jira.clear_calls();
    f.jira.insert("OPS-1", 2);
    f.jira.block_fetch_once.store(true, Ordering::SeqCst);
    let background_service = f.restarted();
    let background =
        tokio::spawn(async move { background_service.sync_tick(policy(), now()).await });
    tokio::time::timeout(
        std::time::Duration::from_secs(5),
        f.jira.fetch_entered.notified(),
    )
    .await
    .unwrap();
    let background_operation = f.state()["active_operation"].as_str().unwrap().to_owned();
    assert_eq!(
        store.lease(&item_id).err().unwrap().code,
        "library_item_busy"
    );
    let manual = tokio::time::timeout(
        std::time::Duration::from_secs(5),
        f.service().start_refresh(LibraryRefreshRequest::Items {
            item_ids: vec![item_id.clone()],
        }),
    )
    .await
    .expect("manual refresh must acquire the preempted item lease")
    .unwrap();
    let manual = finished(f.service(), manual).await;
    assert!(
        manual.phases.iter().all(|p| p.error.is_none()),
        "{manual:?}"
    );
    let tick = tokio::time::timeout(std::time::Duration::from_secs(5), background)
        .await
        .expect("cancelled background fetch must stop while its remote gate remains closed")
        .unwrap()
        .unwrap();
    assert_eq!(f.jira.fetch_release.available_permits(), 0);
    assert_eq!(tick.pending, 1);
    let state = f.state();
    assert!(state["active_operation"].is_null());
    assert_eq!(
        state["queue"][&item_id]["last_error"],
        "library_sync_yielded"
    );
    assert_eq!(state["queue"][&item_id]["revision"], updated(2));
    let cancelled = f.service().operation(&background_operation).await.unwrap();
    assert!(cancelled.finished);
    assert!(cancelled.cancel_requested);
    let saved = f.entry("OPS-1").summary;
    assert_eq!(saved.refs, original_refs);
    assert!(refs::has_follow(&saved, &follow_id));
    assert_eq!(saved.source_revision.as_deref(), Some(iso(2).as_str()));
    assert_eq!(f.jira.state().fetched, ["OPS-1"]);
    assert!(
        std::fs::read_to_string(f.document("OPS-1"))
            .unwrap()
            .contains("OPS-1 body 2")
    );
    assert!(store.lease(&item_id).is_ok());
}

#[tokio::test]
async fn mass_membership_loss_is_held_with_visible_partial_receipt_instead_of_dropping_refs() {
    let f = Fixture::new();
    f.jira.insert("OPS-1", 1);
    f.jira.query(QUERY, &["OPS-1"]);
    let id = f
        .add("jira", QUERY, true, Some(LibraryFollowMode::Live))
        .await
        .unwrap();
    seed_items(&f, "jira", 100, Some(&id));
    f.jira.state().queries.insert(
        QUERY.into(),
        (31..=100).map(|n| format!("OPS-{n}")).collect(),
    );
    f.quiet_audits();
    f.jira.clear_calls();
    for instant in [now(), now() + DAY] {
        f.due(&id, false, true);
        let tick = f.service().sync_tick(policy(), instant).await.unwrap();
        assert_eq!(tick.fetched, 0);
        let index = f.index();
        assert_eq!(index.items.len(), 100);
        assert!(
            index
                .items
                .iter()
                .all(|entry| refs::has_follow(&entry.summary, &id)
                    && entry.summary.purge_after.is_none())
        );
    }
    let state = f.state();
    assert_eq!(
        state["sources"][&id]["absent"].as_object().unwrap().len(),
        30
    );
    let held = state["sources"][&id]["held"].as_str().unwrap();
    assert!(held.contains("30 of 100"));
    assert!(held.contains("manual Refresh"));
    let store = f.service().open().unwrap();
    let receipts = store
        .operations
        .entries()
        .unwrap()
        .map(|entry| {
            let entry = entry.unwrap();
            serde_json::from_slice::<LibraryOperation>(
                &store.operations.read(entry.file_name()).unwrap(),
            )
            .unwrap()
        })
        .collect::<Vec<_>>();
    let partial = receipts
        .iter()
        .filter_map(|operation| operation.report.as_ref())
        .flat_map(|report| &report.rows)
        .find(|row| {
            row.follow_id.as_deref() == Some(id.as_str())
                && row.outcome == LibraryReportOutcome::Partial
        })
        .unwrap();
    assert!(partial.reason.as_deref().unwrap().contains("30 of 100"));
    assert!(f.jira.state().fetched.is_empty());
}

#[cfg(unix)]
#[tokio::test]
async fn symlinked_and_sparse_oversized_sync_state_are_rejected_before_any_provider_io() {
    for symlink in [true, false] {
        let f = Fixture::new();
        f.jira.insert("OPS-1", 1);
        f.jira.query(QUERY, &["OPS-1"]);
        f.add("jira", QUERY, true, Some(LibraryFollowMode::Live))
            .await;
        f.quiet_audits();
        f.jira.clear_calls();
        f.wiki.clear_calls();
        let index_bytes = std::fs::read(f.base.root.join("library/.cockpit/index.json")).unwrap();
        let original = std::fs::read(f.state_path()).unwrap();
        let outside = f.base.root.join("outside-sync-state.json");
        if symlink {
            std::fs::write(&outside, &original).unwrap();
            std::fs::remove_file(f.state_path()).unwrap();
            std::os::unix::fs::symlink(&outside, f.state_path()).unwrap();
        } else {
            let file = std::fs::OpenOptions::new()
                .write(true)
                .truncate(true)
                .open(f.state_path())
                .unwrap();
            file.set_len(64 * 1024 * 1024 + 1).unwrap();
        }
        let error = f.restarted().sync_tick(policy(), now()).await.unwrap_err();
        assert_eq!(error.code, "library_corrupt");
        assert!(f.jira.state().calls.is_empty());
        assert!(f.jira.state().batches.is_empty());
        assert!(f.jira.state().fetched.is_empty());
        assert_eq!(f.wiki.state().inventories, 0);
        assert!(f.wiki.state().deltas.is_empty());
        assert!(f.wiki.state().aux.is_empty());
        assert_eq!(
            std::fs::read(f.base.root.join("library/.cockpit/index.json")).unwrap(),
            index_bytes
        );
        if symlink {
            assert_eq!(std::fs::read(outside).unwrap(), original);
        } else {
            assert_eq!(
                std::fs::metadata(f.state_path()).unwrap().len(),
                64 * 1024 * 1024 + 1
            );
        }
    }
}

#[tokio::test]
async fn auxiliary_failure_advances_cursor_without_starving_later_pages_and_backoff_survives_restart()
 {
    let f = Fixture::new();
    f.wiki.insert("1");
    f.wiki.insert("2");
    let follow_id = f.add("confluence", "SD", true, None).await.unwrap();
    f.due(&follow_id, false, false);
    f.wiki.clear_calls();
    let mut entries = f.index().items;
    entries.sort_by(|a, b| a.summary.item_id.cmp(&b.summary.item_id));
    let bad_id = entries[0].summary.item_id.clone();
    let bad_page = entries[0].summary.canonical_id.clone().unwrap();
    let good_id = entries[1].summary.item_id.clone();
    let good_page = entries[1].summary.canonical_id.clone().unwrap();
    let group = format!("confluence:{WIKI}");
    f.wiki.state().broken_aux.insert(bad_page.clone());
    f.wiki.state().pages.get_mut(&good_page).unwrap().labels = vec!["healthy-label".into()];
    f.service().sync_tick(policy(), now()).await.unwrap();
    let first = f.state();
    assert_eq!(first["audits"][&group]["cursor"], bad_id);
    assert_eq!(first["audits"][&group]["failures"][&bad_id]["failures"], 1);
    assert_eq!(
        first["audits"][&group]["failures"][&bad_id]["next_attempt_ms"],
        now() + 5 * MINUTE
    );
    assert_eq!(
        first["audits"][&group]["failures"][&bad_id]["last_error"],
        "source_provider_failed"
    );
    f.restarted()
        .sync_tick(policy(), now() + 5 * MINUTE - 1)
        .await
        .unwrap();
    assert_eq!(f.wiki.state().aux, [bad_page.clone()]);
    let discovery = f
        .restarted()
        .sync_tick(policy(), now() + 5 * MINUTE)
        .await
        .unwrap();
    assert_eq!(
        f.wiki.state().aux,
        [bad_page.clone(), good_page.clone(), bad_page.clone()]
    );
    assert_eq!(discovery.pending, 1);
    let second = f.state();
    assert_eq!(second["audits"][&group]["cursor"], good_id);
    assert_eq!(second["audits"][&group]["failures"][&bad_id]["failures"], 2);
    assert_eq!(
        second["audits"][&group]["failures"][&bad_id]["next_attempt_ms"],
        now() + 15 * MINUTE
    );
    let publication = f
        .restarted()
        .sync_tick(policy(), now() + 5 * MINUTE + 1)
        .await
        .unwrap();
    assert_eq!(publication.fetched, 1);
    assert_eq!(f.wiki.state().fetched, [good_page.clone()]);
    assert!(
        std::fs::read_to_string(f.document(&good_page))
            .unwrap()
            .contains("healthy-label")
    );
    f.wiki.state().broken_aux.clear();
    f.restarted()
        .sync_tick(policy(), now() + 15 * MINUTE - 1)
        .await
        .unwrap();
    assert_eq!(f.wiki.state().aux.len(), 3);
    f.restarted()
        .sync_tick(policy(), now() + 15 * MINUTE)
        .await
        .unwrap();
    assert!(
        f.state()["audits"][&group]["failures"]
            .as_object()
            .unwrap()
            .is_empty()
    );
    assert_eq!(f.wiki.state().aux.last(), Some(&bad_page));
}

#[tokio::test]
async fn unchanged_jira_content_audit_does_not_rewrite_index_or_add_durable_operation_history() {
    let f = Fixture::new();
    f.jira.insert("OPS-1", 1);
    f.jira.query(QUERY, &["OPS-1"]);
    let follow_id = f
        .add("jira", QUERY, true, Some(LibraryFollowMode::Live))
        .await
        .unwrap();
    // Suppress follow discovery only: the actual rolling Jira audit remains due.
    f.due(&follow_id, false, false);
    f.jira.clear_calls();
    let store = f.service().open().unwrap();
    let receipts = || {
        store
            .operations
            .entries()
            .unwrap()
            .map(|entry| {
                let entry = entry.unwrap();
                let name = entry.file_name();
                (
                    name.to_string_lossy().into_owned(),
                    store.operations.read(&name).unwrap(),
                )
            })
            .collect::<BTreeMap<_, _>>()
    };
    let before_receipts = receipts();
    assert!(
        !before_receipts.is_empty(),
        "the manual import receipt must exist"
    );
    let index_path = f.base.root.join("library/.cockpit/index.json");
    let index_bytes = std::fs::read(&index_path).unwrap();
    let generation = f.index().generation;
    let document = std::fs::read(f.document("OPS-1")).unwrap();
    let saved = f.entry("OPS-1").summary;
    let discovery = f.service().sync_tick(policy(), now()).await.unwrap();
    assert_eq!(discovery.pending, 1);
    assert!(f.jira.state().fetched.is_empty());
    assert!(f.jira.state().calls.is_empty());
    let state = f.state();
    assert_eq!(state["queue"][&saved.item_id]["owners"], json!(["audit"]));
    assert_eq!(state["queue"][&saved.item_id]["force"], true);
    assert_eq!(receipts(), before_receipts);
    let audit = f.restarted().sync_tick(policy(), now() + 1).await.unwrap();
    assert_eq!(audit.fetched, 1);
    assert_eq!(audit.pending, 0);
    assert_eq!(f.jira.state().fetched, ["OPS-1"]);
    assert!(f.jira.state().calls.is_empty());
    assert_eq!(f.index().generation, generation);
    assert_eq!(std::fs::read(index_path).unwrap(), index_bytes);
    assert_eq!(std::fs::read(f.document("OPS-1")).unwrap(), document);
    assert_eq!(f.entry("OPS-1").summary.refs, saved.refs);
    assert_eq!(
        receipts(),
        before_receipts,
        "a successful unchanged audit must not add history or evict/rewrite the manual import receipt"
    );
    assert!(f.state()["active_operation"].is_null());
}

#[tokio::test]
async fn daily_inventory_moves_unchanged_version_descendant_without_losing_manual_or_overlapping_follow_refs()
 {
    let f = Fixture::new();
    for page in ["1", "2", "3"] {
        f.wiki.insert(page);
    }
    f.wiki.state().pages.get_mut("1").unwrap().ancestors = vec!["3".into()];
    let wiki_follow = f.add("confluence", "SD", true, None).await.unwrap();
    f.add("confluence", "1", false, None).await;
    // A real Jira reference traversal supplies the overlapping follow holder.
    f.jira.insert("OPS-1", 1);
    f.jira.state().issues.get_mut("OPS-1").unwrap().body = format!(
        "## Description\n[Shared page]({})\n",
        sources::confluence_page_url(WIKI, "1")
    );
    f.jira.query(QUERY, &["OPS-1"]);
    let request = LibraryAddRequest {
        input: QUERY.into(),
        provider_id: Some("jira".into()),
        reference_depth: 1,
        follow: true,
        follow_mode: Some(LibraryFollowMode::Live),
        download_attachments: false,
        refresh_existing: false,
        label: None,
        target: None,
    };
    let operation = finished(f.service(), f.service().start_add(request).await.unwrap()).await;
    assert!(
        operation.phases.iter().all(|phase| phase.error.is_none()),
        "{operation:?}"
    );
    assert_eq!(
        operation.report.as_ref().unwrap().partial,
        0,
        "{operation:?}"
    );
    assert_eq!(
        operation.report.as_ref().unwrap().failed,
        0,
        "{operation:?}"
    );
    let jira_follow = f
        .index()
        .follows
        .into_iter()
        .find(|follow| follow.provider_id == "jira")
        .unwrap()
        .follow_id;
    let before = f.entry("1").summary;
    assert!(before.refs.contains(&LibraryItemRef::Manual));
    assert!(refs::has_follow(&before, &wiki_follow));
    assert!(refs::has_follow(&before, &jira_follow));
    assert_eq!(
        before.parent_item_id.as_deref(),
        Some(f.entry("3").summary.item_id.as_str())
    );
    let old_document = f.document("1");
    f.quiet_audits();
    f.due(&wiki_follow, false, true);
    f.due(&jira_follow, false, false);
    f.edit_state(|state| {
        state["sources"][&jira_follow]["next_related_ms"] = json!(now() + 100 * DAY);
    });
    f.wiki.state().pages.get_mut("1").unwrap().ancestors = vec!["2".into()];
    f.jira.clear_calls();
    f.wiki.clear_calls();
    let tick = f
        .restarted()
        .sync_tick(policy(), now() + DAY)
        .await
        .unwrap();
    assert_eq!(tick.discovered, 1);
    assert_eq!(tick.fetched, 1);
    assert_eq!(tick.pending, 0);
    assert_eq!(f.wiki.state().inventories, 1);
    assert!(f.wiki.state().deltas.is_empty());
    assert_eq!(f.wiki.state().fetched, ["1"]);
    assert!(f.jira.state().fetched.is_empty());
    let moved = f.entry("1").summary;
    let parent = f.entry("2").summary;
    assert_eq!(
        moved.parent_item_id.as_deref(),
        Some(parent.item_id.as_str())
    );
    assert_eq!(
        moved
            .ancestors
            .iter()
            .map(|ancestor| ancestor.id.as_str())
            .collect::<Vec<_>>(),
        ["2"]
    );
    assert!(
        moved
            .item_path
            .starts_with(&format!("{}/", parent.item_path))
    );
    assert_ne!(moved.item_path, before.item_path);
    assert_eq!(moved.source_revision.as_deref(), Some("1"));
    assert_eq!(moved.refs, before.refs);
    assert_eq!(
        serde_json::to_value(&moved.included_by).unwrap(),
        serde_json::to_value(&before.included_by).unwrap()
    );
    assert!(!old_document.exists());
    assert!(f.document("1").is_file());
}

#[tokio::test]
async fn standalone_inventory_resumes_budget_prefix_after_restart_and_drains_old_work_before_discovery()
 {
    let f = Fixture::new();
    for id in ["1", "2", "3", "4", "5", "6"] {
        f.wiki.insert(id);
        f.add("confluence", id, false, None).await;
    }
    f.quiet_audits();
    f.wiki.clear_calls();
    let group = format!("standalone:confluence:{WIKI}");
    let first_id = f.entry("1").summary.item_id.clone();
    let original_first = f.entry("1").summary;
    let original_missing = f.entry("2").summary;
    let first_bytes = std::fs::read(f.document("1")).unwrap();
    let missing_bytes = std::fs::read(f.document("2")).unwrap();
    let cohort = json!(["1", "2", "3", "4", "5", "6"]);
    {
        let mut s = f.wiki.state();
        s.pages.get_mut("1").unwrap().version = 2;
        s.pages.remove("2");
        s.rate_limited_fetch.insert("1".into());
        // A Cloud bulk request plus a distinct outside-parent lookup per page.
        s.budget = Some(MetadataBudget {
            remaining: 5,
            per_page: 1,
            blocked_until: None,
        });
    }
    let config = LibrarySyncConfiguration {
        hourly_request_cap: 5,
        ..policy()
    };
    let first = f.service().sync_tick(config.clone(), now()).await.unwrap();
    assert_eq!(first.pending, 1);
    assert_eq!(first.fetched, 0);
    let state = f.state();
    assert_eq!(state["sources"][&group]["inventory"]["ids"], cohort);
    assert_eq!(state["sources"][&group]["inventory"]["cursor"], 2);
    assert_eq!(
        state["sources"][&group]["inventory"]["present"],
        json!(["1"])
    );
    assert!(
        state["sources"][&group]["absent"]
            .as_object()
            .unwrap()
            .is_empty()
    );
    assert_eq!(
        state["queue"][&first_id]["last_error"],
        "source_rate_limited"
    );
    assert_eq!(state["queue"][&first_id]["failures"], 1);
    assert_eq!(state["queue"][&first_id]["next_attempt_ms"], now() + MINUTE);
    assert_eq!(
        f.entry("1").summary.source_revision,
        original_first.source_revision
    );
    assert_eq!(std::fs::read(f.document("1")).unwrap(), first_bytes);
    let cooldown = f
        .restarted()
        .sync_tick(config.clone(), now() + MINUTE)
        .await
        .unwrap();
    assert_eq!(cooldown.pending, 1);
    assert_eq!(cooldown.fetched, 0);
    assert!(
        f.state()["origins"]
            .as_object()
            .unwrap()
            .values()
            .any(|until| until.as_i64() == Some(now() + 60 * MINUTE))
    );
    let cooled = f.state();
    assert_eq!(
        cooled["sources"][&group]["inventory"],
        state["sources"][&group]["inventory"]
    );
    assert_eq!(cooled["queue"][&first_id], state["queue"][&first_id]);
    assert_eq!(std::fs::read(f.document("1")).unwrap(), first_bytes);
    // Manual changes between budget turns are not part of the frozen cohort.
    let removed = f.entry("3").summary;
    f.service()
        .remove_item(&removed.item_id, &removed.revision)
        .unwrap();
    f.wiki.insert("7");
    f.add("confluence", "7", false, None).await;
    let new_item = f.entry("7").summary;
    let new_bytes = std::fs::read(f.document("7")).unwrap();
    {
        let mut s = f.wiki.state();
        s.rate_limited_fetch.clear();
        let budget = s.budget.as_mut().unwrap();
        budget.remaining = 5;
        budget.blocked_until = None;
    }
    let second = f
        .restarted()
        .sync_tick(config.clone(), now() + 60 * MINUTE)
        .await
        .unwrap();
    assert_eq!(second.fetched, 1);
    assert_eq!(second.pending, 0);
    assert_eq!(f.entry("1").summary.source_revision.as_deref(), Some("2"));
    assert_eq!(f.entry("1").summary.refs, original_first.refs);
    assert_ne!(std::fs::read(f.document("1")).unwrap(), first_bytes);
    let drained = f.state();
    assert!(drained["queue"].as_object().unwrap().is_empty());
    // With this replenished budget, consuming the old content retry leaves
    // room for one metadata identity, not two. Durable progress plus the
    // published revision proves discovery did not starve old queued work.
    assert_eq!(drained["sources"][&group]["inventory"]["cursor"], 3);
    assert_eq!(drained["sources"][&group]["inventory"]["ids"], cohort);
    assert_eq!(
        drained["sources"][&group]["inventory"]["present"],
        json!(["1", "3"])
    );
    assert!(
        drained["sources"][&group]["absent"]
            .as_object()
            .unwrap()
            .is_empty()
    );
    assert!(
        !f.index()
            .items
            .iter()
            .any(|entry| entry.summary.item_id == removed.item_id)
    );
    {
        let mut s = f.wiki.state();
        s.budget.as_mut().unwrap().remaining = 5;
    }
    f.restarted()
        .sync_tick(config.clone(), now() + 120 * MINUTE)
        .await
        .unwrap();
    let resumed = f.state();
    assert_eq!(resumed["sources"][&group]["inventory"]["cursor"], 5);
    assert_eq!(resumed["sources"][&group]["inventory"]["ids"], cohort);
    assert_eq!(
        resumed["sources"][&group]["inventory"]["present"],
        json!(["1", "3", "4", "5"])
    );
    assert!(
        resumed["sources"][&group]["absent"]
            .as_object()
            .unwrap()
            .is_empty()
    );
    {
        let mut s = f.wiki.state();
        s.budget.as_mut().unwrap().remaining = 5;
    }
    f.restarted()
        .sync_tick(config, now() + 180 * MINUTE)
        .await
        .unwrap();
    let state = f.state();
    assert!(state["sources"][&group]["inventory"].is_null());
    assert_eq!(
        state["sources"][&group]["absent"],
        json!({"2": now()+180*MINUTE})
    );
    assert_eq!(
        state["sources"][&group]["last_success_ms"],
        now() + 180 * MINUTE
    );
    assert!(state["sources"][&group]["last_error"].is_null());
    assert!(state["queue"].as_object().unwrap().is_empty());
    assert!(
        !f.index()
            .items
            .iter()
            .any(|entry| entry.summary.item_id == removed.item_id)
    );
    assert_eq!(f.entry("7").summary.revision, new_item.revision);
    assert_eq!(f.entry("7").summary.state, LibraryItemState::Fresh);
    assert_eq!(std::fs::read(f.document("7")).unwrap(), new_bytes);
    assert_eq!(f.entry("2").summary.refs, original_missing.refs);
    assert_eq!(f.entry("2").summary.revision, original_missing.revision);
    assert_eq!(std::fs::read(f.document("2")).unwrap(), missing_bytes);
    assert_ne!(
        f.entry("2").summary.state,
        LibraryItemState::RemovedAtSource
    );
}

#[tokio::test]
async fn expensive_single_chunk_shrinks_durably_after_origin_cooldown_instead_of_replaying_forever()
{
    let f = Fixture::new();
    for id in ["1", "2", "3"] {
        f.wiki.insert(id);
        f.add("confluence", id, false, None).await;
    }
    f.quiet_audits();
    f.wiki.clear_calls();
    let group = format!("standalone:confluence:{WIKI}");
    // The initial conservative two-requests/page estimate still underestimates
    // paginated ancestry. Two pages cost seven requests, over this whole cap.
    f.wiki.state().budget = Some(MetadataBudget {
        remaining: 6,
        per_page: 3,
        blocked_until: None,
    });
    let config = LibrarySyncConfiguration {
        hourly_request_cap: 6,
        ..policy()
    };
    f.service().sync_tick(config.clone(), now()).await.unwrap();
    let state = f.state();
    assert_eq!(state["sources"][&group]["inventory"]["cursor"], 0);
    assert_eq!(state["sources"][&group]["inventory"]["batch_size"], 1);
    assert_eq!(
        state["sources"][&group]["last_error"],
        "source_rate_limited"
    );
    assert!(
        state["sources"][&group]["absent"]
            .as_object()
            .unwrap()
            .is_empty()
    );
    f.restarted()
        .sync_tick(config.clone(), now() + 59 * MINUTE)
        .await
        .unwrap();
    assert_eq!(
        f.wiki.state().batches.len(),
        1,
        "persisted origin cooldown blocks premature retries"
    );
    for turn in 1..=3 {
        {
            let mut s = f.wiki.state();
            let budget = s.budget.as_mut().unwrap();
            budget.remaining = 6;
            budget.blocked_until = None;
        }
        f.restarted()
            .sync_tick(config.clone(), now() + turn * 60 * MINUTE)
            .await
            .unwrap();
        if turn < 3 {
            assert_eq!(f.state()["sources"][&group]["inventory"]["cursor"], turn);
            assert!(
                f.state()["sources"][&group]["absent"]
                    .as_object()
                    .unwrap()
                    .is_empty()
            );
        }
    }
    assert_eq!(
        f.wiki.state().batches,
        [
            vec!["1".to_owned(), "2".to_owned()],
            vec!["1".to_owned()],
            vec!["2".to_owned()],
            vec!["3".to_owned()]
        ]
    );
    assert!(f.state()["sources"][&group]["inventory"].is_null());
    assert!(f.state()["sources"][&group]["last_error"].is_null());
}

#[tokio::test]
async fn incomplete_standalone_chunk_never_commits_cursor_or_absence_and_due_groups_remain_independent()
 {
    let f = Fixture::new();
    f.wiki.insert("1");
    f.add("confluence", "1", false, None).await;
    f.jira.insert("OPS-1", 1);
    f.add("jira", &format!("{JIRA}/browse/OPS-1"), false, None)
        .await;
    f.quiet_audits();
    f.wiki.clear_calls();
    f.jira.clear_calls();
    let wiki_group = format!("standalone:confluence:{WIKI}");
    let jira_group = format!("standalone:jira:{JIRA}");
    f.wiki.state().pages.remove("1");
    f.wiki.state().partial_versions = true;
    f.service().sync_tick(policy(), now()).await.unwrap();
    let state = f.state();
    assert_eq!(state["sources"][&wiki_group]["inventory"]["cursor"], 0);
    assert!(
        state["sources"][&wiki_group]["absent"]
            .as_object()
            .unwrap()
            .is_empty()
    );
    assert_eq!(
        state["sources"][&wiki_group]["last_error"],
        "library_sync_partial"
    );
    assert_eq!(state["sources"][&jira_group]["last_success_ms"], now());
    assert_eq!(f.jira.state().batches, [vec!["OPS-1".to_owned()]]);
    f.wiki.state().partial_versions = false;
    f.restarted()
        .sync_tick(policy(), now() + 5 * MINUTE)
        .await
        .unwrap();
    assert!(f.state()["sources"][&wiki_group]["inventory"].is_null());
    assert_eq!(
        f.state()["sources"][&wiki_group]["absent"],
        json!({"1": now()+5*MINUTE})
    );
    assert_ne!(
        f.entry("1").summary.state,
        LibraryItemState::RemovedAtSource
    );
}
