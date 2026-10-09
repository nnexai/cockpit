use super::super::tests::{self as base, finished};
use super::*;
use crate::sources::{
    AttachmentRef, DownloadedAttachment, SourceAsset, SourceContainer, SourceFetchRequest,
    SourceProvider, SourceService,
};
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
        self.site()
            .queries
            .insert(jql.into(), keys.iter().map(|k| (*k).to_owned()).collect());
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
            id: id.into(),
            title: name.into(),
            media_type: Some("text/plain".into()),
            size: Some(size),
            source_url: None,
            source_revision: None,
            path: None,
            not_downloaded: Some("not_requested".into()),
        });
    }
    fn take_fetched(&self) -> BTreeSet<String> {
        std::mem::take(&mut self.site().fetched)
            .into_iter()
            .collect()
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
            IssueQuery::Jql {
                jql,
                updated_window,
            } => {
                assert_eq!(
                    *updated_window, None,
                    "manual follows must list the full query"
                );
                site.log.push(format!("jql {jql}"));
                if site.list_error {
                    return Err(error("source_provider_failed", "jira is down"));
                }
                let rows = site
                    .queries
                    .get(*jql)
                    .into_iter()
                    .flatten()
                    .filter(|key| !site.gone.contains(*key))
                    .filter_map(|key| Some((key, site.issues.get(key)?)))
                    .map(|(key, issue)| row(key, issue))
                    .collect();
                Ok(IssueListing {
                    rows,
                    complete: !site.truncated,
                })
            }
            IssueQuery::Keys(keys) => {
                site.log.push(format!("keys {}", keys.join(",")));
                let rows = keys
                    .iter()
                    .filter(|key| !site.gone.contains(*key))
                    .filter_map(|key| Some(row(key, site.issues.get(key)?)))
                    .collect();
                Ok(IssueListing {
                    rows,
                    complete: true,
                })
            }
        }
    }
    async fn fetch(
        &self,
        request: &SourceFetchRequest,
    ) -> Result<Vec<SourceAsset>, InspectionError> {
        let key = request.artifact_url.rsplit('/').next().unwrap().to_owned();
        let mut site = self.site();
        site.fetched.push(key.clone());
        if site.gone.contains(&key) {
            return Err(error("source_not_found", "issue does not exist"));
        }
        if site.broken.contains(&key) {
            return Err(error("source_provider_failed", "jira is down"));
        }
        let issue = site
            .issues
            .get(&key)
            .cloned()
            .ok_or_else(|| error("source_not_found", "gone"))?;
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
            container: Some(SourceContainer {
                id: project.clone(),
                label: project,
            }),
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
        self.site().downloads.push((
            canonical_id.into(),
            attachment.id.clone(),
            budget.bytes,
            budget.max_files,
            siblings.len(),
        ));
        dest.write("download", vec![b'x'; size as usize]).unwrap();
        Ok(DownloadedAttachment {
            attachment_id: attachment.id.clone(),
            file_name: "download".into(),
        })
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
        kind: ProviderKind::Jira,
        base_url: BASE.into(),
        executable: None,
        login: None,
        deployment: Some(cockpit_protocol::projects::ProviderDeployment::DataCenter),
    }];
    let mut site = Site::default();
    for (key, minute) in [("OPS-1", 1), ("OPS-2", 2), ("OPS-3", 3), ("OPS-4", 4)] {
        site.issues.insert(
            key.into(),
            Issue {
                minute,
                body: format!("{key} body"),
                attachments: vec![],
            },
        );
    }
    let provider = Arc::new(FakeIssues(Mutex::new(site)));
    let sources = Arc::new(SourceService::new(&configuration, vec![provider.clone()]).unwrap());
    base.service = LibraryService::new(configuration, sources);
    Fixture { base, provider }
}

#[test]
fn jira_query_and_follow_authority_use_kind_not_executable_or_id() {
    let mut f = fixture();
    assert_eq!(
        f.base.service.jira_site("jira").unwrap().provider_instance,
        BASE
    );
    let (_, provider_id) = f
        .base
        .service
        .jira_query("project = OPS", None)
        .unwrap()
        .unwrap();
    assert_eq!(provider_id, "jira");
    assert!(f.base.service.jira_query("OPS", None).unwrap().is_none());
    assert!(
        f.base
            .service
            .jira_query("OPS", Some("jira"))
            .unwrap()
            .is_some()
    );
    let provider = &mut f.base.service.configuration.providers[0];
    provider.kind = ProviderKind::Gitea;
    provider.executable = Some("/usr/local/bin/jira".into());
    provider.deployment = None;
    assert_eq!(
        f.base.service.jira_site("jira").unwrap_err().code,
        "source_provider_unsupported"
    );
    assert!(
        f.base
            .service
            .jira_query("project = OPS", Some("jira"))
            .unwrap()
            .is_none()
    );
    assert!(!f.base.service.is_jira_provider("jira"));
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
    assert_eq!(
        operation.phases[0].state,
        LibraryPhaseState::Done,
        "{operation:?}"
    );
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
    let request = LibraryRefreshRequest::Follow {
        follow_id: follow_id.into(),
    };
    let operation = finished(service, service.start_refresh(request).await.unwrap()).await;
    assert!(
        operation.phases.iter().all(|p| p.error.is_none()),
        "{operation:?}"
    );
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
    assert_eq!(
        f.provider.take_fetched(),
        keys(&["OPS-1", "OPS-2", "OPS-3"])
    );
    let items = issues(service).await;
    // D9: the existing Jira layout, one directory per key.
    assert_eq!(items["OPS-2"].item_path, "jira/jira.example.test/OPS/OPS-2");
    assert_eq!(
        items["OPS-2"].document_path.as_deref(),
        Some("jira/jira.example.test/OPS/OPS-2/Issue OPS-2.md")
    );
    let meta = items["OPS-2"].issue.clone().unwrap();
    assert_eq!(
        (meta.updated.as_str(), meta.fetched_updated.as_deref()),
        ("2026-03-01 10:02:00", Some("2026-03-01 10:02:00"))
    );
    assert!(items.values().all(|item| item.refs
        == [LibraryItemRef::Follow {
            follow_id: id.clone()
        }]));
    let follow_record = record(service, &id).await;
    assert_eq!(
        (follow_record.item_count, follow_record.state),
        (3, LibraryItemState::Fresh)
    );

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
    assert!(
        report
            .rows
            .iter()
            .any(|r| r.reason.as_deref() == Some("Query returned no issues; kept 2 issues"))
    );
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
async fn accumulate_lists_the_full_query_and_checks_only_members_outside_it() {
    let f = fixture();
    let service = &f.base.service;
    f.provider.set(OPS, &["OPS-1", "OPS-3"]);
    let (_, id) = follow(service, OPS, LibraryFollowMode::Accumulate).await;
    assert_eq!(f.provider.take_fetched(), keys(&["OPS-1", "OPS-3"]));
    f.provider.take_log();

    let report = refresh(service, &id).await;
    assert_eq!(
        (report.unchanged, report.updated, report.dropped),
        (2, 0, 0)
    );
    assert_eq!(f.provider.take_log(), [format!("jql {OPS}")]);
    assert!(f.provider.take_fetched().is_empty());

    // OPS-2 newly matches despite being older than the newest saved issue.
    f.provider.set(OPS, &["OPS-1", "OPS-2", "OPS-3"]);
    let report = refresh(service, &id).await;
    assert_eq!((report.new, report.unchanged, report.dropped), (1, 2, 0));
    assert_eq!(f.provider.take_log(), [format!("jql {OPS}")]);
    assert_eq!(f.provider.take_fetched(), keys(&["OPS-2"]));

    // OPS-2 is deleted at source and leaves the query; OPS-1 only leaves the query.
    f.provider.set(OPS, &["OPS-3"]);
    f.provider.site().gone.insert("OPS-2".into());
    let report = refresh(service, &id).await;
    assert_eq!((report.removed_at_source, report.dropped), (1, 0));
    assert_eq!(
        f.provider.take_log(),
        [format!("jql {OPS}"), "keys OPS-1,OPS-2".to_owned()]
    );
    assert_eq!(f.provider.take_fetched(), keys(&["OPS-2"]));
    let items = issues(service).await;
    assert_eq!(items["OPS-2"].state, LibraryItemState::RemovedAtSource);
    assert!(items.values().all(|item| held(item, &id)));

    // A changed issue outside the query is still refreshed; the confirmed one is not fetched again.
    f.provider.touch("OPS-1", 9);
    let report = refresh(service, &id).await;
    assert_eq!((report.updated, report.dropped), (1, 0));
    assert_eq!(f.provider.take_fetched(), keys(&["OPS-1"]));
    assert_eq!(
        f.provider.take_log(),
        [format!("jql {OPS}"), "keys OPS-1,OPS-2".to_owned()]
    );
}

#[tokio::test]
async fn accumulate_retries_failed_new_issues_after_newer_issues_are_saved() {
    let f = fixture();
    let service = &f.base.service;
    f.provider.set(OPS, &["OPS-3"]);
    let (_, id) = follow(service, OPS, LibraryFollowMode::Accumulate).await;
    f.provider.take_fetched();
    f.provider.take_log();

    f.provider.set(OPS, &["OPS-1", "OPS-2", "OPS-3"]);
    f.provider.touch("OPS-2", 9);
    f.provider.site().broken.insert("OPS-1".into());
    let report = refresh(service, &id).await;
    assert_eq!((report.new, report.failed, report.dropped), (1, 1, 0));
    assert_eq!(f.provider.take_log(), [format!("jql {OPS}")]);
    assert_eq!(f.provider.take_fetched(), keys(&["OPS-1", "OPS-2"]));
    let items = issues(service).await;
    assert!(!items.contains_key("OPS-1"));
    assert_eq!(
        items["OPS-2"].source_revision.as_deref(),
        Some(iso(9).as_str())
    );
    assert_eq!(record(service, &id).await.state, LibraryItemState::Partial);

    f.provider.site().broken.clear();
    let report = refresh(service, &id).await;
    assert_eq!(
        (report.new, report.unchanged, report.failed, report.dropped),
        (1, 2, 0, 0)
    );
    assert_eq!(f.provider.take_log(), [format!("jql {OPS}")]);
    assert_eq!(f.provider.take_fetched(), keys(&["OPS-1"]));
    assert!(issues(service).await.values().all(|item| held(item, &id)));
    assert_eq!(record(service, &id).await.state, LibraryItemState::Fresh);
}

#[tokio::test]
async fn accumulate_retries_failed_members_outside_the_query_without_dropping_refs() {
    let f = fixture();
    let service = &f.base.service;
    f.provider.set(OPS, &["OPS-1", "OPS-2"]);
    let (_, id) = follow(service, OPS, LibraryFollowMode::Accumulate).await;
    f.provider.take_fetched();
    f.provider.take_log();

    f.provider.set(OPS, &["OPS-2"]);
    f.provider.touch("OPS-1", 9);
    f.provider.site().broken.insert("OPS-1".into());
    let report = refresh(service, &id).await;
    assert_eq!((report.failed, report.dropped), (1, 0));
    assert_eq!(
        f.provider.take_log(),
        [format!("jql {OPS}"), "keys OPS-1".to_owned()]
    );
    assert_eq!(f.provider.take_fetched(), keys(&["OPS-1"]));
    let items = issues(service).await;
    assert_eq!(items["OPS-1"].state, LibraryItemState::Failed);
    assert_eq!(
        items["OPS-1"]
            .issue
            .as_ref()
            .unwrap()
            .fetched_updated
            .as_deref(),
        Some(plain(1).as_str())
    );
    assert!(items.values().all(|item| held(item, &id)));
    assert_eq!(record(service, &id).await.state, LibraryItemState::Partial);

    f.provider.site().broken.clear();
    let report = refresh(service, &id).await;
    assert_eq!(
        (
            report.updated,
            report.unchanged,
            report.failed,
            report.dropped
        ),
        (1, 1, 0, 0)
    );
    assert_eq!(
        f.provider.take_log(),
        [format!("jql {OPS}"), "keys OPS-1".to_owned()]
    );
    assert_eq!(f.provider.take_fetched(), keys(&["OPS-1"]));
    let items = issues(service).await;
    assert_eq!(
        items["OPS-1"]
            .issue
            .as_ref()
            .unwrap()
            .fetched_updated
            .as_deref(),
        Some(plain(9).as_str())
    );
    assert!(items.values().all(|item| held(item, &id)));
    assert_eq!(record(service, &id).await.state, LibraryItemState::Fresh);
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
    assert_eq!(
        (
            second.report.as_ref().unwrap().new,
            second.report.as_ref().unwrap().unchanged
        ),
        (1, 1)
    );
    let items = issues(service).await;
    assert_eq!(items.len(), 3);
    assert!(held(&items["OPS-2"], &a) && held(&items["OPS-2"], &b));
    assert_eq!(
        (
            record(service, &a).await.item_count,
            record(service, &b).await.item_count
        ),
        (2, 2)
    );

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
    assert!(
        issues(service).await["OPS-1"]
            .refs
            .contains(&LibraryItemRef::Manual)
    );

    // Stop following A: OPS-1 keeps its manual reference, OPS-2 stays with B.
    service
        .remove(LibraryRemoveRequest::StopFollowing {
            follow_id: a.clone(),
        })
        .await
        .unwrap();
    let items = issues(service).await;
    assert_eq!(items["OPS-1"].refs, [LibraryItemRef::Manual]);
    assert_eq!(
        items["OPS-2"].refs,
        [LibraryItemRef::Follow {
            follow_id: b.clone()
        }]
    );
    assert!(items.values().all(|item| item.purge_after.is_none()));

    // Remove B and its items: only the issues it alone holds go.
    service
        .remove(LibraryRemoveRequest::Follow {
            follow_id: b.clone(),
        })
        .await
        .unwrap();
    let items = issues(service).await;
    assert_eq!(items.keys().cloned().collect::<Vec<_>>(), ["OPS-1"]);

    // Stopping an exclusive follow keeps its issues as plain items.
    f.provider.set("status = Open", &["OPS-4"]);
    let (_, c) = follow(service, "status = Open", LibraryFollowMode::Live).await;
    service
        .remove(LibraryRemoveRequest::StopFollowing { follow_id: c })
        .await
        .unwrap();
    assert_eq!(
        issues(service).await["OPS-4"].refs,
        [LibraryItemRef::Manual]
    );
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
    std::fs::write(
        root.join(edited.document_path.as_deref().unwrap()),
        b"edited in Library",
    )
    .unwrap();
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
    assert_eq!(
        reason("OPS-2").as_deref(),
        Some("purged after 14 days unreferenced")
    );
    assert_eq!(reason("OPS-1").as_deref(), Some("kept: edited in Library"));
    let items = issues(service).await;
    assert_eq!(
        items.keys().cloned().collect::<Vec<_>>(),
        ["OPS-1", "OPS-3"]
    );
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
        (
            attachment.original_name.as_str(),
            attachment.bytes,
            attachment.state,
            attachment.relative_path.as_deref()
        ),
        (
            "trace.log",
            Some(2048),
            LibraryAttachmentState::NotDownloaded,
            None
        )
    );
    let root = std::path::Path::new(&service.configuration.library_root);
    let document =
        std::fs::read_to_string(root.join(item.document_path.as_deref().unwrap())).unwrap();
    assert!(
        document.contains("attachments:\n  - id: \"10100\"\n    title: \"trace.log\""),
        "{document}"
    );
    assert!(document.contains("not_downloaded: \"not_requested\""));
    assert!(!root.join(&item.item_path).join("_files").exists());

    // A later upload changes `updated`, so the refresh fetches the issue and lists both files.
    f.provider.attach("OPS-1", 5, "10101", "shot.png", 90);
    let report = refresh(service, &id).await;
    assert_eq!(report.updated, 1, "{report:?}");
    let names: Vec<_> = issues(service).await["OPS-1"]
        .attachments
        .iter()
        .map(|a| a.original_name.clone())
        .collect();
    assert_eq!(names, ["trace.log", "shot.png"]);
}

async fn download_one(
    service: &LibraryService,
    item: &LibraryItemSummary,
    id: &str,
) -> Result<LibraryOperation, InspectionError> {
    let request = LibraryAttachmentRequest {
        item_id: item.item_id.clone(),
        attachment_ids: vec![id.into()],
        action: LibraryAttachmentAction::Download,
    };
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
    assert_eq!(
        operation.phases[0].state,
        LibraryPhaseState::Done,
        "{operation:?}"
    );
    let item = issues(service).await["OPS-1"].clone();
    assert_eq!(
        item.attachments[0].state,
        LibraryAttachmentState::Downloaded
    );
    assert_eq!(
        item.attachments[1].state,
        LibraryAttachmentState::NotDownloaded
    );
    let root = std::path::Path::new(&service.configuration.library_root);
    let stored = root
        .join(&item.item_path)
        .join(item.attachments[0].relative_path.as_deref().unwrap());
    assert_eq!(std::fs::read(stored).unwrap(), b"xxxxxx");
    // The prediction is exactly that attachment: one file, its own size, no siblings.
    assert_eq!(
        f.provider.site().downloads,
        [("OPS-1".to_owned(), "10100".to_owned(), 6, 1, 0)]
    );
}

#[tokio::test]
async fn a_provider_that_refuses_downloads_is_refused_before_any_operation_or_download() {
    for code in [
        "source_credential_required",
        "credential_vault_unavailable",
        "source_capability_unavailable",
    ] {
        let f = fixture();
        let service = &f.base.service;
        f.provider.set(OPS, &["OPS-1"]);
        f.provider.attach("OPS-1", 1, "10100", "trace.log", 6);
        follow(service, OPS, LibraryFollowMode::Live).await;
        let item = issues(service).await["OPS-1"].clone();
        f.provider.site().download_gate = Some(code);

        assert_eq!(
            download_one(service, &item, "10100")
                .await
                .unwrap_err()
                .code,
            code
        );
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
        assert_eq!(
            item.attachments[0].state,
            LibraryAttachmentState::NotDownloaded
        );
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
    assert_eq!(
        operation.phases[0].state,
        LibraryPhaseState::Done,
        "{operation:?}"
    );
    let items = issues(service).await;
    assert_eq!(items.len(), 2, "{items:?}");
    assert_eq!(
        items["OPS-1"].attachments[0].state,
        LibraryAttachmentState::Downloaded
    );
    assert_eq!(
        items["OPS-2"].attachments[0].state,
        LibraryAttachmentState::NotDownloaded
    );
    assert_eq!(
        f.provider.site().downloads,
        [("OPS-1".to_owned(), "10100".to_owned(), 6, 1, 0)]
    );
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
    let operation = finished(
        service,
        service.start_add(follow_request(jql, true)).await.unwrap(),
    )
    .await;
    assert!(
        matches!(
            operation.phases[0].state,
            LibraryPhaseState::Done | LibraryPhaseState::Partial
        ),
        "{operation:?}"
    );
    let listing = service.listing(None).await.unwrap();
    listing
        .follows
        .into_iter()
        .find(|follow| refs::follow_title(follow) == jql)
        .unwrap()
        .follow_id
}
fn stored(service: &LibraryService, item: &LibraryItemSummary, index: usize) -> Option<Vec<u8>> {
    let root = std::path::Path::new(&service.configuration.library_root);
    let path = item.attachments[index].relative_path.as_deref()?;
    std::fs::read(root.join(&item.item_path).join(path)).ok()
}
fn downloaded(f: &Fixture) -> Vec<(String, String)> {
    std::mem::take(&mut f.provider.site().downloads)
        .into_iter()
        .map(|d| (d.0, d.1))
        .collect()
}

#[tokio::test]
async fn a_follow_that_opts_in_persists_it_and_downloads_new_issues_within_the_budget() {
    let f = fixture();
    let service = &f.base.service;
    f.provider.set(OPS, &["OPS-1", "OPS-2"]);
    f.provider.attach("OPS-1", 1, "10100", "trace.log", 6);
    f.provider
        .attach("OPS-2", 2, "20200", "big.bin", 26 * 1024 * 1024);
    let id = follow_downloading(service, OPS).await;

    assert!(record(service, &id).await.include_attachments);
    let items = issues(service).await;
    assert_eq!(
        items["OPS-1"].attachments[0].state,
        LibraryAttachmentState::Downloaded
    );
    assert_eq!(
        stored(service, &items["OPS-1"], 0).as_deref(),
        Some(&b"xxxxxx"[..])
    );
    // Over the per-file budget: recorded, never requested from the provider.
    assert_eq!(
        items["OPS-2"].attachments[0].state,
        LibraryAttachmentState::OverLimit
    );
    // One exact file per request: its own size, no siblings.
    assert_eq!(
        f.provider.site().downloads,
        [("OPS-1".to_owned(), "10100".to_owned(), 6, 1, 0)]
    );

    // Following without the opt-in keeps the default and downloads nothing.
    f.provider.set("status = Open", &["OPS-3"]);
    f.provider.attach("OPS-3", 3, "30300", "plain.log", 6);
    let (_, plain) = follow(service, "status = Open", LibraryFollowMode::Live).await;
    assert!(!record(service, &plain).await.include_attachments);
    assert_eq!(
        issues(service).await["OPS-3"].attachments[0].state,
        LibraryAttachmentState::NotDownloaded
    );
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
    assert_eq!(
        (report.updated, report.new, report.unchanged),
        (0, 0, 2),
        "{report:?}"
    );
    assert!(f.provider.take_fetched().is_empty() && downloaded(&f).is_empty());

    // A new attachment on OPS-1 and one on an issue that had none: only those two download.
    f.provider.attach("OPS-1", 5, "10101", "more.log", 6);
    f.provider.attach("OPS-2", 6, "20200", "shot.log", 6);
    let report = refresh(service, &id).await;
    assert_eq!(report.updated, 2, "{report:?}");
    assert_eq!(
        downloaded(&f),
        [
            ("OPS-1".to_owned(), "10101".to_owned()),
            ("OPS-2".to_owned(), "20200".to_owned())
        ]
    );
    let items = issues(service).await;
    assert!(
        items["OPS-1"]
            .attachments
            .iter()
            .all(|a| a.state == LibraryAttachmentState::Downloaded)
    );
    assert_eq!(
        stored(service, &items["OPS-1"], 0).as_deref(),
        Some(&b"xxxxxx"[..])
    );

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
    assert_eq!(
        stored(service, &items["OPS-1"], 1).as_deref(),
        Some(&b"xxxxxxxx"[..])
    );
    assert_eq!(
        stored(service, &items["OPS-1"], 0).as_deref(),
        Some(&b"xxxxxx"[..])
    );
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
        service
            .start_add(follow_request(OPS, true))
            .await
            .unwrap_err()
            .code,
        "source_credential_required"
    );
    f.provider.site().download_gate = None;
    let id = follow_downloading(service, OPS).await;
    assert!(record(service, &id).await.include_attachments);
    assert_eq!(downloaded(&f), [("OPS-1".to_owned(), "10100".to_owned())]);
    assert_eq!(
        issues(service).await["OPS-1"].attachments[0].state,
        LibraryAttachmentState::Downloaded
    );

    // The token goes away: the text still refreshes, the report says why the
    // file did not, and nothing is lost.
    f.provider.site().download_gate = Some("source_credential_required");
    f.provider.attach("OPS-1", 5, "10101", "more.log", 6);
    let report = refresh(service, &id).await;
    assert_eq!((report.updated, report.partial), (1, 1), "{report:?}");
    let note = report
        .rows
        .iter()
        .find_map(|row| {
            row.reason
                .clone()
                .filter(|r| r.starts_with("Attachments were not downloaded"))
        })
        .unwrap();
    assert!(note.contains("token stored in Cockpit"), "{note}");
    assert!(downloaded(&f).is_empty());
    let items = issues(service).await;
    assert_eq!(
        items["OPS-1"].attachments[0].state,
        LibraryAttachmentState::Downloaded
    );
    assert_eq!(
        items["OPS-1"].attachments[1].state,
        LibraryAttachmentState::NotDownloaded
    );

    // Once the token is back, the next refresh downloads what was pending although the issue is unchanged.
    f.provider.site().download_gate = None;
    f.provider.take_fetched();
    refresh(service, &id).await;
    assert_eq!(downloaded(&f), [("OPS-1".to_owned(), "10101".to_owned())]);
    assert!(
        issues(service).await["OPS-1"]
            .attachments
            .iter()
            .all(|a| a.state == LibraryAttachmentState::Downloaded)
    );
}

#[tokio::test]
async fn a_refused_opt_in_leaves_no_follow_item_or_operation() {
    for code in [
        "source_credential_required",
        "credential_vault_unavailable",
        "source_capability_unavailable",
    ] {
        let f = fixture();
        let service = &f.base.service;
        f.provider.set(OPS, &["OPS-1"]);
        f.provider.attach("OPS-1", 1, "10100", "trace.log", 6);
        f.provider.site().download_gate = Some(code);

        assert_eq!(
            service
                .start_add(follow_request(OPS, true))
                .await
                .unwrap_err()
                .code,
            code
        );
        let listing = service.listing(None).await.unwrap();
        assert!(listing.follows.is_empty() && listing.items.is_empty());
        let operations = std::fs::read_dir(
            std::path::Path::new(&service.configuration.library_root).join(".cockpit/operations"),
        );
        assert!(operations.map_or(true, |mut entries| entries.next().is_none()));
        let site = f.provider.site();
        assert!(site.downloads.is_empty() && site.fetched.is_empty() && site.log.is_empty());
    }
}

#[tokio::test]
async fn a_shared_issue_keeps_its_files_for_a_follow_that_does_not_download_and_they_go_with_the_item()
 {
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
    let file = root
        .join(&item.item_path)
        .join(item.attachments[0].relative_path.as_deref().unwrap());
    assert!(file.exists());

    // The issue changes; the non-downloading follow saves the new revision and keeps the file.
    f.provider.touch("OPS-1", 5);
    let report = refresh(service, &b).await;
    assert_eq!(report.updated, 1, "{report:?}");
    let item = issues(service).await["OPS-1"].clone();
    assert_eq!(
        item.attachments[0].state,
        LibraryAttachmentState::Downloaded
    );
    assert!(file.exists());
    assert!(downloaded(&f).len() == 1);

    // Stopping one follow keeps the item and its file; removing the last one removes both.
    service
        .remove(LibraryRemoveRequest::Follow { follow_id: a })
        .await
        .unwrap();
    assert!(file.exists() && issues(service).await.contains_key("OPS-1"));
    service
        .remove(LibraryRemoveRequest::Follow { follow_id: b })
        .await
        .unwrap();
    assert!(issues(service).await.is_empty());
    assert!(!file.exists() && !root.join(&item.item_path).exists());
}

fn reason_of(item: &LibraryItemSummary, follow_id: &str) -> Option<(String, String, u32)> {
    item.included_by
        .iter()
        .flatten()
        .find_map(|inclusion| match &inclusion.holder {
            LibraryInclusionHolder::Follow { follow_id: id } if id == follow_id => Some((
                inclusion.from_label.clone(),
                inclusion.relation.clone(),
                inclusion.depth,
            )),
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
    assert_eq!(
        reason_of(&items["OPS-2"], &id),
        Some(("OPS-1".into(), "description".into(), 1))
    );
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
    assert!(
        held(&items["OPS-3"], &id),
        "an unreachable related item is kept"
    );
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
    assert_eq!(
        reason_of(&items["OPS-3"], &id),
        Some(("OPS-1".into(), "description".into(), 1))
    );
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
    assert_eq!(
        record(service, &id).await.excluded_ids,
        vec![related.item_id.clone()]
    );
    let report = refresh(service, &id).await;
    assert_eq!((report.partial, report.dropped), (0, 0), "{report:?}");
    assert!(
        !issues(service).await.contains_key("OPS-2"),
        "the removed related item stays out"
    );
}
