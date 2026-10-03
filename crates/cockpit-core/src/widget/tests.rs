use super::*;
use async_trait::async_trait;
use serde_json::json;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use tokio::task::JoinHandle;

struct FakeBrowser {
    snapshot: Mutex<BrowserHerdrSnapshot>,
    failed: AtomicBool,
}

#[async_trait]
impl BrowserHerdrAdapter for FakeBrowser {
    async fn browser_snapshot(&self, _: &str) -> Result<BrowserHerdrSnapshot, InspectionError> {
        if self.failed.load(Ordering::Relaxed) {
            return Err(InspectionError::new("offline", "fake Herdr is unavailable"));
        }
        Ok(self.snapshot.lock().clone())
    }
}

struct FakePaste {
    targets: Mutex<Vec<CommentPasteTarget>>,
    failed: AtomicBool,
}

#[async_trait]
impl CommentPasteAdapter for FakePaste {
    async fn comment_paste_targets(
        &self,
        _: &str,
    ) -> Result<Vec<CommentPasteTarget>, InspectionError> {
        if self.failed.load(Ordering::Relaxed) {
            return Err(InspectionError::new(
                "offline",
                "fake paste snapshot is unavailable",
            ));
        }
        Ok(self.targets.lock().clone())
    }

    async fn focus_comment_paste_target(
        &self,
        _: &CommentPasteTarget,
    ) -> Result<(), InspectionError> {
        panic!("widget operations must not focus Herdr")
    }

    async fn confirm_comment_paste_target_focus(
        &self,
        _: &CommentPasteTarget,
    ) -> Result<(), InspectionError> {
        panic!("widget operations must not request paste focus")
    }

    async fn send_comment_paste(
        &self,
        _: &CommentPasteTarget,
        _: &str,
    ) -> Result<(), InspectionError> {
        panic!("widget selections must never paste into a terminal")
    }
}

struct Fixture {
    service: Arc<WidgetService>,
    browser: Arc<FakeBrowser>,
    paste: Arc<FakePaste>,
    clock: Arc<AtomicU64>,
}

impl Fixture {
    fn new() -> Self {
        let snapshot = serde_json::from_value(json!({
            "session_id": "daily", "server_instance": "fixture", "version": "fixture", "protocol": 22,
            "focused_space_id": "s1", "focused_tab_id": "t1", "focused_pane_id": "p1",
            "spaces": [
                {"id":"s1","label":"API","number":1,"tab_count":2,"pane_count":3,"focused":true,"agent_status":"idle","git":null},
                {"id":"s2","label":"Other","number":2,"tab_count":1,"pane_count":1,"focused":false,"agent_status":"idle","git":null}
            ],
            "tabs": [
                {"id":"t1","space_id":"s1","label":"One","number":1,"pane_count":2,"focused":true,"focused_pane_id":"p1"},
                {"id":"t2","space_id":"s1","label":"Two","number":2,"pane_count":1,"focused":false,"focused_pane_id":"p2"},
                {"id":"t3","space_id":"s2","label":"Three","number":1,"pane_count":1,"focused":true,"focused_pane_id":"p3"}
            ],
            "panes": [
                {"id":"p1","terminal_id":"term1","space_id":"s1","tab_id":"t1","focused":true,"agent_status":"idle","revision":0},
                {"id":"p1b","terminal_id":"term1b","space_id":"s1","tab_id":"t1","focused":false,"agent_status":"idle","revision":0},
                {"id":"p2","terminal_id":"term2","space_id":"s1","tab_id":"t2","focused":false,"agent_status":"idle","revision":0},
                {"id":"p3","terminal_id":"term3","space_id":"s2","tab_id":"t3","focused":false,"agent_status":"idle","revision":0}
            ], "agents": []
        })).unwrap();
        let browser = Arc::new(FakeBrowser {
            snapshot: Mutex::new(BrowserHerdrSnapshot {
                endpoint_identity: "endpoint-generation-1".into(),
                endpoint_path: "/fixture/herdr.sock".into(),
                snapshot,
            }),
            failed: AtomicBool::new(false),
        });
        let paste = Arc::new(FakePaste {
            targets: Mutex::new(vec![
                agent("p1", "t1", "s1", "term1", "fingerprint-agent-one"),
                agent("p2", "t2", "s1", "term2", "fingerprint-agent-two"),
            ]),
            failed: AtomicBool::new(false),
        });
        let clock = Arc::new(AtomicU64::new(1_000));
        let read_clock = clock.clone();
        let service = Arc::new(
            WidgetService::new(browser.clone(), paste.clone())
                .with_clock(Arc::new(move || read_clock.load(Ordering::Relaxed))),
        );
        Self {
            service,
            browser,
            paste,
            clock,
        }
    }

    fn advance(&self, ms: u64) {
        self.clock.fetch_add(ms, Ordering::Relaxed);
    }

    fn request(&self, id: &str, label: &str) -> WidgetShowRequest {
        WidgetShowRequest {
            address: address(),
            id: id.into(),
            title: None,
            content: choices(label),
            reopen: false,
            clear_selection: false,
        }
    }

    async fn show(&self, id: &str, label: &str) -> WidgetShowResponse {
        self.service.show(self.request(id, label)).await.unwrap()
    }

    async fn list(&self) -> Vec<WidgetListEntry> {
        self.service
            .list(WidgetListRequest { address: address() })
            .await
            .unwrap()
            .widgets
    }

    async fn selection(&self, id: &str) -> WidgetSelectionResponse {
        self.service
            .selection(selection_request(id, None))
            .await
            .unwrap()
    }

    fn select(
        &self,
        id: &str,
        revision: u64,
        choice_id: &str,
    ) -> Result<WidgetSelectResponse, InspectionError> {
        self.service.select(WidgetSelectRequest {
            key: key(id),
            revision,
            value: WidgetSelectValue::Choice {
                choice_id: choice_id.into(),
            },
        })
    }

    fn select_page(
        &self,
        id: &str,
        revision: u64,
        value_json: &str,
    ) -> Result<WidgetSelectResponse, InspectionError> {
        self.service.select(WidgetSelectRequest {
            key: key(id),
            revision,
            value: WidgetSelectValue::Page {
                value_json: value_json.into(),
            },
        })
    }

    fn remove(&self, id: &str) -> WidgetRemoveResponse {
        self.service
            .remove(WidgetRemoveRequest { key: key(id) })
            .unwrap()
    }

    async fn close(&self, id: &str) -> WidgetCloseResponse {
        self.service
            .close(WidgetCloseRequest {
                address: address(),
                id: id.into(),
            })
            .await
            .unwrap()
    }

    fn summaries(&self) -> Vec<WidgetSummary> {
        let subscription = self.service.subscribe();
        match subscription.snapshot {
            WidgetEvent::Snapshot { widgets, .. } => widgets,
            _ => panic!("subscription must start with a snapshot"),
        }
    }

    fn waiter(&self, id: &str) -> JoinHandle<Result<WidgetSelectionResponse, InspectionError>> {
        let service = self.service.clone();
        let request = selection_request(id, Some(3600));
        tokio::spawn(async move { service.selection(request).await })
    }
}

fn agent(
    pane: &str,
    tab: &str,
    space: &str,
    terminal: &str,
    fingerprint: &str,
) -> CommentPasteTarget {
    CommentPasteTarget {
        endpoint_identity: "endpoint-generation-1".into(),
        session_id: "daily".into(),
        workspace_id: space.into(),
        tab_id: tab.into(),
        pane_id: pane.into(),
        terminal_id: terminal.into(),
        agent_label: "omp".into(),
        agent_fingerprint: fingerprint.into(),
    }
}

fn address() -> WidgetAddress {
    WidgetAddress {
        session_id: "daily".into(),
        endpoint_path: Some("/fixture/herdr.sock".into()),
        source_pane_id: Some("p1".into()),
        locator: WidgetLocator::CurrentPane,
        space_check: None,
    }
}

fn key(id: &str) -> WidgetKey {
    WidgetKey {
        session_id: "daily".into(),
        tab_id: "t1".into(),
        id: id.into(),
    }
}

fn choices(label: &str) -> WidgetContentInput {
    let spec_json = json!({"prompt": "Choose a view", "choices": [
        {"id": "latency", "label": label}, {"id": "errors", "label": "Errors"}
    ]})
    .to_string();
    WidgetContentInput::Choices {
        sha256: format!("{:x}", Sha256::digest(spec_json.as_bytes())),
        spec_json,
        from: WidgetInputKind::File,
        name: Some("choices.json".into()),
    }
}

fn html(document: &str) -> WidgetContentInput {
    WidgetContentInput::Html {
        content_base64: STANDARD.encode(document),
        sha256: format!("{:x}", Sha256::digest(document.as_bytes())),
        from: WidgetInputKind::Stdin,
        name: None,
    }
}

fn selection_request(id: &str, wait_seconds: Option<u64>) -> WidgetSelectionRequest {
    WidgetSelectionRequest {
        address: address(),
        id: id.into(),
        wait_seconds,
    }
}

async fn await_waiters(service: &WidgetService, expected: usize) {
    for _ in 0..1_000 {
        if service.store.lock().waiters.len() == expected {
            return;
        }
        tokio::task::yield_now().await;
    }
    panic!(
        "expected {expected} pending waiters, found {}",
        service.store.lock().waiters.len()
    );
}

async fn completed<T>(task: JoinHandle<T>) -> T {
    for _ in 0..1_000 {
        if task.is_finished() {
            return task.await.unwrap();
        }
        tokio::task::yield_now().await;
    }
    task.abort();
    panic!("selection task did not complete after the triggering operation");
}

#[tokio::test]
async fn identical_bytes_replace_when_content_kind_changes() {
    let f = Fixture::new();
    let choices_content = choices("Latency");
    let WidgetContentInput::Choices { spec_json, .. } = &choices_content else {
        unreachable!();
    };
    let html_content = html(spec_json);
    let mut request = f.request("kinds", "Latency");
    request.content = html_content.clone();
    let opened = f.service.show(request.clone()).await.unwrap();
    assert_eq!(
        (opened.result, opened.revision, opened.presentation),
        (WidgetShowResult::Opened, 1, WidgetPresentation::Active)
    );
    let hash = f.summaries()[0].content.sha256.clone();
    let html_bytes = f.service.store.lock().html_bytes;
    let mut stream = f.service.subscribe();

    for (content, kind, presentation, revision, expected_html_bytes) in [
        (
            choices_content.clone(),
            WidgetKind::Choices,
            WidgetPresentation::Choices,
            2,
            0,
        ),
        (
            html_content,
            WidgetKind::Html,
            WidgetPresentation::Active,
            3,
            html_bytes,
        ),
    ] {
        request.content = content;
        let replaced = f.service.show(request.clone()).await.unwrap();
        assert_eq!(
            (replaced.result, replaced.revision, replaced.presentation),
            (WidgetShowResult::Replaced, revision, presentation)
        );
        let summary = f.summaries()[0].clone();
        assert_eq!(summary.kind, kind);
        assert_eq!(summary.content.sha256, hash);
        assert_eq!(summary.change, WidgetChange::Replaced);
        assert!(
            matches!(stream.events.try_recv().unwrap(), WidgetEvent::Upserted { widget, .. } if widget == summary)
        );
        let body = f
            .service
            .content(WidgetContentRequest {
                key: key("kinds"),
                revision,
            })
            .unwrap()
            .body;
        assert!(matches!(
            (kind, body),
            (WidgetKind::Html, WidgetBody::Html { .. })
                | (WidgetKind::Choices, WidgetBody::Choices { .. })
        ));
        assert_eq!(f.service.store.lock().html_bytes, expected_html_bytes);
        assert_eq!(
            f.service.store.lock().snapshot_bytes,
            summary_snapshot_bytes(&summary).unwrap()
        );

        let unchanged = f.service.show(request.clone()).await.unwrap();
        assert_eq!(
            (unchanged.result, unchanged.revision),
            (WidgetShowResult::Unchanged, revision)
        );
        assert_eq!(f.summaries()[0], summary);
        assert!(matches!(
            stream.events.try_recv(),
            Err(broadcast::error::TryRecvError::Empty)
        ));
    }
}

#[tokio::test]
async fn replacement_preserves_order_and_old_selection_until_explicitly_cleared() {
    let f = Fixture::new();
    let mut stream = f.service.subscribe();
    assert_eq!(
        f.show("first", "Latency").await.result,
        WidgetShowResult::Opened
    );
    f.show("second", "Latency").await;
    f.select("first", 1, "latency").unwrap();
    let before = f.summaries();
    while stream.events.try_recv().is_ok() {}
    let unchanged = f.show("first", "Latency").await;
    assert_eq!(
        (unchanged.result, unchanged.revision),
        (WidgetShowResult::Unchanged, 1)
    );
    assert!(matches!(
        stream.events.try_recv(),
        Err(broadcast::error::TryRecvError::Empty)
    ));
    f.advance(100);
    let replacement = f.show("first", "New latency label").await;
    assert_eq!(
        (replacement.result, replacement.revision),
        (WidgetShowResult::Replaced, 2)
    );
    let after = f.summaries();
    assert_eq!(
        after.iter().map(|w| w.key.id.as_str()).collect::<Vec<_>>(),
        ["first", "second"]
    );
    assert_eq!(after[0].created_seq, before[0].created_seq);
    assert_eq!(after[0].created_at_ms, before[0].created_at_ms);
    assert_eq!(after[0].updated_at_ms, 1_100);
    let selected = f.selection("first").await;
    assert_eq!(
        (selected.status, selected.revision, selected.at_ms),
        (WidgetSelectionStatus::Selected, Some(1), Some(1_000))
    );
    assert_eq!(
        serde_json::from_str::<serde_json::Value>(&selected.value_json.unwrap()).unwrap(),
        json!({"id":"latency","label":"Latency"})
    );
    assert_eq!(
        f.summaries()[0].selection.as_ref().unwrap().read_at_ms,
        Some(1_100)
    );
    assert_eq!(
        f.select("first", 1, "errors").unwrap_err().code,
        "widget_stale"
    );
    assert_eq!(
        f.service
            .content(WidgetContentRequest {
                key: key("first"),
                revision: 1
            })
            .unwrap_err()
            .code,
        "widget_stale"
    );
    let body = f
        .service
        .content(WidgetContentRequest {
            key: key("first"),
            revision: 2,
        })
        .unwrap()
        .body;
    assert!(
        matches!(body, WidgetBody::Choices { spec } if spec.choices[0].label == "New latency label")
    );
    let mut clear = f.request("first", "Third label");
    clear.clear_selection = true;
    assert_eq!(f.service.show(clear).await.unwrap().revision, 3);
    assert_eq!(
        f.selection("first").await.status,
        WidgetSelectionStatus::None
    );
    assert!(f.summaries()[0].selection.is_none());
}

#[tokio::test]
async fn identical_content_clears_only_a_retained_selection_through_replacement() {
    for page in [false, true] {
        let f = Fixture::new();
        let mut request = f.request("selected", "Latency");
        if page {
            request.content = html("<button>Result</button>");
        }
        f.service.show(request.clone()).await.unwrap();
        if page {
            f.select_page("selected", 1, r#"{"action":"filter"}"#)
                .unwrap();
        } else {
            f.select("selected", 1, "latency").unwrap();
        }
        let selected = f.selection("selected").await;
        let before = f.summaries()[0].clone();
        let html_bytes = f.service.store.lock().html_bytes;
        let snapshot_bytes = f.service.store.lock().snapshot_bytes;
        let mut stream = f.service.subscribe();
        f.advance(100);

        let unchanged = f.service.show(request.clone()).await.unwrap();
        assert_eq!(
            (unchanged.result, unchanged.revision),
            (WidgetShowResult::Unchanged, 1)
        );
        assert_eq!(f.summaries()[0], before);
        assert_eq!(f.service.store.lock().snapshot_bytes, snapshot_bytes);
        assert!(matches!(
            stream.events.try_recv(),
            Err(broadcast::error::TryRecvError::Empty)
        ));
        assert_eq!(f.selection("selected").await, selected);
        while stream.events.try_recv().is_ok() {}

        request.clear_selection = true;
        let cleared = f.service.show(request.clone()).await.unwrap();
        assert_eq!(
            (cleared.result, cleared.revision),
            (WidgetShowResult::Replaced, 2)
        );
        let after = f.summaries()[0].clone();
        assert_eq!(after.content, before.content);
        assert_eq!(after.created_seq, before.created_seq);
        assert_eq!(after.created_at_ms, before.created_at_ms);
        assert_eq!(after.updated_at_ms, 1_100);
        assert_eq!(after.change, WidgetChange::Replaced);
        assert!(after.selection.is_none());
        assert!(
            matches!(stream.events.try_recv().unwrap(), WidgetEvent::Upserted { widget, .. } if widget == after)
        );
        assert_eq!(f.service.store.lock().html_bytes, html_bytes);
        assert_eq!(
            f.service.store.lock().snapshot_bytes,
            summary_snapshot_bytes(&after).unwrap()
        );
        assert!(
            f.service
                .content(WidgetContentRequest {
                    key: key("selected"),
                    revision: 2,
                })
                .unwrap()
                .selection
                .is_none()
        );
        assert_eq!(
            f.selection("selected").await.status,
            WidgetSelectionStatus::None
        );

        let unchanged = f.service.show(request).await.unwrap();
        assert_eq!(
            (unchanged.result, unchanged.revision),
            (WidgetShowResult::Unchanged, 2)
        );
        assert_eq!(f.summaries()[0], after);
        assert!(matches!(
            stream.events.try_recv(),
            Err(broadcast::error::TryRecvError::Empty)
        ));
    }
}

#[tokio::test]
async fn user_removal_requires_reopen_and_agent_close_leaves_no_tombstone() {
    let f = Fixture::new();
    f.show("first", "Latency").await;
    f.show("second", "Latency").await;
    f.select("first", 1, "errors").unwrap();
    f.advance(2_000);
    assert_eq!(f.remove("first").result, WidgetRemoveResult::Removed);
    assert_eq!(f.remove("first").result, WidgetRemoveResult::AlreadyRemoved);
    let dismissed = f.selection("first").await;
    assert_eq!(
        (
            dismissed.status,
            dismissed.revision,
            dismissed.removed_at_ms
        ),
        (WidgetSelectionStatus::Dismissed, Some(1), Some(3_000))
    );
    let error = f
        .service
        .show(f.request("first", "Latency"))
        .await
        .unwrap_err();
    assert_eq!(error.code, "widget_dismissed");
    assert!(error.message.contains("1970-01-01T00:00:03Z"));
    assert!(error.message.contains("revision 1"));
    assert!(error.message.contains("--reopen"));
    assert_eq!(
        f.close("first").await.result,
        WidgetCloseResult::AlreadyRemoved
    );
    assert!(
        f.list()
            .await
            .iter()
            .any(|w| w.id == "first" && w.state == WidgetListState::RemovedByUser)
    );
    let mut reopen = f.request("first", "Reopened");
    reopen.reopen = true;
    let reopened = f.service.show(reopen).await.unwrap();
    assert_eq!(
        (reopened.result, reopened.revision),
        (WidgetShowResult::Reopened, 2)
    );
    assert_eq!(
        f.summaries()
            .iter()
            .map(|w| w.key.id.as_str())
            .collect::<Vec<_>>(),
        ["second", "first"]
    );
    assert_eq!(
        f.selection("first").await.status,
        WidgetSelectionStatus::None
    );
    assert_eq!(f.close("first").await.result, WidgetCloseResult::Closed);
    assert_eq!(
        f.close("first").await.result,
        WidgetCloseResult::AlreadyRemoved
    );
    assert_eq!(
        f.list()
            .await
            .iter()
            .map(|w| w.id.as_str())
            .collect::<Vec<_>>(),
        ["second"]
    );
    assert_eq!(f.show("first", "Fresh").await.revision, 1);
}

#[tokio::test]
async fn ownership_uses_fresh_fingerprints_and_lists_only_the_calling_source() {
    let f = Fixture::new();
    f.show("owned", "Latency").await;
    let mut other = address();
    other.source_pane_id = Some("p2".into());
    other.locator = WidgetLocator::Tab {
        tab_id: "t1".into(),
    };
    let mut request = f.request("owned", "Different");
    request.address = other.clone();
    assert_eq!(
        f.service.show(request).await.unwrap_err().code,
        "widget_not_owner"
    );
    assert_eq!(
        f.service
            .close(WidgetCloseRequest {
                address: other.clone(),
                id: "owned".into()
            })
            .await
            .unwrap_err()
            .code,
        "widget_not_owner"
    );
    assert_eq!(
        f.service
            .selection(WidgetSelectionRequest {
                address: other.clone(),
                id: "owned".into(),
                wait_seconds: None
            })
            .await
            .unwrap_err()
            .code,
        "widget_not_owner"
    );
    assert!(
        f.service
            .list(WidgetListRequest {
                address: other.clone()
            })
            .await
            .unwrap()
            .widgets
            .is_empty()
    );
    let mut request = f.request("other", "Other");
    request.address = other;
    f.service.show(request).await.unwrap();
    assert_eq!(
        f.list()
            .await
            .iter()
            .map(|w| w.id.as_str())
            .collect::<Vec<_>>(),
        ["owned"]
    );
    f.remove("owned");
    f.paste.targets.lock()[0].agent_fingerprint = "new-agent-in-same-pane".into();
    let mut request = f.request("owned", "Reopened");
    request.reopen = true;
    assert_eq!(
        f.service.show(request).await.unwrap_err().code,
        "widget_not_owner"
    );
    assert!(f.list().await.is_empty());
}

#[tokio::test]
async fn sha256_source_prefix_is_wire_safe_without_truncating_ownership_or_restart_identity() {
    let f = Fixture::new();
    let original = format!("sha256:87a0abcdef12{}", "0".repeat(52));
    let restarted = format!("sha256:87a0abcdef12{}", "1".repeat(52));
    f.paste.targets.lock()[0].agent_fingerprint = original;
    f.show("digest-owner", "Original").await;
    let mut stream = f.service.subscribe();
    let snapshot = serde_json::to_value(&stream.snapshot).unwrap();
    assert_eq!(
        snapshot["widgets"][0]["source"]["fingerprint_prefix"],
        "87a0abcdef12"
    );

    // Changing only the undisplayed digest suffix must still change ownership.
    f.paste.targets.lock()[0].agent_fingerprint = restarted;
    assert_eq!(
        f.service
            .show(f.request("digest-owner", "Replacement"))
            .await
            .unwrap_err()
            .code,
        "widget_not_owner"
    );
    f.service.reconcile().await;
    let event = stream.events.try_recv().unwrap();
    assert!(matches!(&event, WidgetEvent::Upserted { widget, .. }
            if widget.source.as_ref().unwrap().status == WidgetSourceStatus::Restarted));
    let update = serde_json::to_value(event).unwrap();
    assert_eq!(
        update["widget"]["source"]["fingerprint_prefix"],
        "87a0abcdef12"
    );
    assert_eq!(f.summaries()[0].revision, 1);

    f.show("new-digest-owner", "New owner").await;
    assert!(
        matches!(stream.events.try_recv().unwrap(), WidgetEvent::Upserted { widget, .. }
            if widget.key.id == "new-digest-owner"
                && widget.source.as_ref().unwrap().status == WidgetSourceStatus::Present)
    );
}

#[tokio::test]
async fn nonstandard_agent_fingerprints_do_not_publish_invalid_prefixes() {
    for fingerprint in [
        "fingerprint-agent-one",
        "sha256:87a0a",
        "sha256:éééééééééééééééééééééééééééééééé",
    ] {
        let f = Fixture::new();
        f.paste.targets.lock()[0].agent_fingerprint = fingerprint.into();
        f.show("opaque", "Original").await;
        let snapshot = serde_json::to_value(f.service.subscribe().snapshot).unwrap();
        assert!(snapshot["widgets"][0]["source"]["fingerprint_prefix"].is_null());
        assert_eq!(
            f.show("opaque", "Replacement").await.result,
            WidgetShowResult::Replaced
        );
    }
}

#[tokio::test]
async fn non_agent_and_unattributed_sources_have_distinct_ownership() {
    let f = Fixture::new();
    f.paste.targets.lock().clear();
    let published = f.show("plain", "Latency").await;
    assert!(published.source.is_some());
    assert!(
        f.summaries()[0]
            .source
            .as_ref()
            .unwrap()
            .fingerprint_prefix
            .is_none()
    );
    let mut request = f.request("plain", "Other");
    request.address.source_pane_id = Some("p1b".into());
    assert_eq!(
        f.service.show(request).await.unwrap_err().code,
        "widget_not_owner"
    );
    let mut request = f.request("anonymous", "Latency");
    request.address.source_pane_id = None;
    request.address.locator = WidgetLocator::Tab {
        tab_id: "t1".into(),
    };
    let anonymous = f.service.show(request).await.unwrap();
    assert!(anonymous.source.is_none());
    assert!(
        anonymous
            .warnings
            .iter()
            .any(|w| w.starts_with("unattributed:"))
    );
    assert_eq!(f.summaries()[1].arrival, WidgetArrival::CrossSource);
}

#[tokio::test]
async fn rate_window_counts_creates_and_reopens_but_not_replacements() {
    let f = Fixture::new();
    for n in 0..6 {
        f.show(&format!("w{n}"), "Latency").await;
    }
    assert_eq!(
        f.service
            .show(f.request("seventh", "Latency"))
            .await
            .unwrap_err()
            .code,
        "widget_rate_limited"
    );
    assert_eq!(
        f.show("w0", "Replacement").await.result,
        WidgetShowResult::Replaced
    );
    f.remove("w1");
    let mut reopen = f.request("w1", "Reopen");
    reopen.reopen = true;
    assert_eq!(
        f.service.show(reopen.clone()).await.unwrap_err().code,
        "widget_rate_limited"
    );
    f.advance(59_999);
    assert_eq!(
        f.service.show(reopen.clone()).await.unwrap_err().code,
        "widget_rate_limited"
    );
    f.advance(1);
    assert_eq!(
        f.service.show(reopen).await.unwrap().result,
        WidgetShowResult::Reopened
    );
    f.show("seventh", "Latency").await;
}

#[tokio::test]
async fn ninth_live_widget_is_rejected_but_replacement_and_close_release_work() {
    let f = Fixture::new();
    for n in 0..8 {
        if n == 6 {
            f.advance(60_000);
        }
        f.show(&format!("w{n}"), "Latency").await;
    }
    assert_eq!(
        f.service
            .show(f.request("ninth", "Latency"))
            .await
            .unwrap_err()
            .code,
        "widget_limit"
    );
    assert_eq!(f.show("w0", "Replacement").await.revision, 2);
    f.close("w1").await;
    assert_eq!(
        f.show("ninth", "Latency").await.result,
        WidgetShowResult::Opened
    );
    assert_eq!(
        f.list()
            .await
            .iter()
            .filter(|w| w.state == WidgetListState::Live)
            .count(),
        8
    );
}

#[tokio::test]
async fn sixty_fifth_tombstone_evicts_oldest_without_evicting_recent_removals() {
    let f = Fixture::new();
    for n in 0..65 {
        f.advance(60_000);
        let id = format!("w{n}");
        f.show(&id, "Latency").await;
        f.remove(&id);
    }
    let list = f.list().await;
    assert_eq!(
        list.iter().map(|w| w.id.as_str()).collect::<Vec<_>>(),
        (1..65).map(|n| format!("w{n}")).collect::<Vec<_>>()
    );
    assert!(
        list.iter()
            .all(|w| w.state == WidgetListState::RemovedByUser)
    );
    assert_eq!(
        f.service
            .selection(selection_request("w0", None))
            .await
            .unwrap_err()
            .code,
        "widget_target_not_found"
    );
    f.advance(60_000);
    assert_eq!(f.show("w0", "New").await.revision, 1);
    assert_eq!(
        f.service
            .show(f.request("w1", "New"))
            .await
            .unwrap_err()
            .code,
        "widget_dismissed"
    );
}

#[tokio::test]
async fn locators_resolve_current_pane_explicit_pane_tab_and_space_without_focus_changes() {
    let f = Fixture::new();
    let original = f.browser.snapshot.lock().snapshot.clone();
    let cases = [
        (
            WidgetLocator::CurrentPane,
            "t1",
            WidgetResolvedFrom::CurrentPane,
        ),
        (
            WidgetLocator::Pane {
                pane_id: "p2".into(),
            },
            "t2",
            WidgetResolvedFrom::Pane,
        ),
        (
            WidgetLocator::Tab {
                tab_id: "t3".into(),
            },
            "t3",
            WidgetResolvedFrom::Tab,
        ),
        (
            WidgetLocator::Space {
                space_id: "s1".into(),
            },
            "t1",
            WidgetResolvedFrom::SpaceFocusedTab,
        ),
    ];
    for (n, (locator, tab, from)) in cases.into_iter().enumerate() {
        let mut request = f.request(&format!("w{n}"), "Latency");
        request.address.locator = locator;
        let shown = f.service.show(request).await.unwrap();
        assert_eq!(shown.target.tab_id, tab);
        assert_eq!(shown.target.resolved_from, from);
    }
    let cross = f
        .summaries()
        .into_iter()
        .find(|w| w.key.id == "w1")
        .unwrap();
    assert_eq!(cross.arrival, WidgetArrival::CrossSource);
    assert!(
        cross
            .warnings
            .iter()
            .any(|w| w.starts_with("not_focused_tab:") && w.contains("t2"))
    );
    assert_eq!(f.browser.snapshot.lock().snapshot, original);
}

#[tokio::test]
async fn space_locator_reuses_stored_tab_for_live_and_tombstoned_ids() {
    let f = Fixture::new();
    let mut request = f.request("stored", "Latency");
    request.address.locator = WidgetLocator::Space {
        space_id: "s1".into(),
    };
    assert_eq!(
        f.service.show(request.clone()).await.unwrap().target.tab_id,
        "t1"
    );
    {
        let mut fresh = f.browser.snapshot.lock();
        fresh.snapshot.tabs[0].focused = false;
        fresh.snapshot.tabs[1].focused = true;
    }
    request.content = choices("Replacement");
    let replacement = f.service.show(request.clone()).await.unwrap();
    assert_eq!(
        (
            replacement.target.tab_id.as_str(),
            replacement.target.resolved_from
        ),
        ("t1", WidgetResolvedFrom::Stored)
    );
    f.remove("stored");
    request.reopen = true;
    let reopened = f.service.show(request).await.unwrap();
    assert_eq!(
        (
            reopened.target.tab_id.as_str(),
            reopened.target.resolved_from
        ),
        ("t1", WidgetResolvedFrom::Stored)
    );
    let mut new = f.request("new", "Latency");
    new.address.locator = WidgetLocator::Space {
        space_id: "s1".into(),
    };
    assert_eq!(f.service.show(new).await.unwrap().target.tab_id, "t2");
}

#[tokio::test]
async fn targeting_rejects_missing_ambiguous_mismatched_and_inconsistent_fresh_targets() {
    let f = Fixture::new();
    for locator in [
        WidgetLocator::Pane {
            pane_id: "missing".into(),
        },
        WidgetLocator::Tab {
            tab_id: "missing".into(),
        },
        WidgetLocator::Space {
            space_id: "missing".into(),
        },
    ] {
        let mut request = f.request("w", "Latency");
        request.address.locator = locator;
        assert_eq!(
            f.service.show(request).await.unwrap_err().code,
            "widget_target_not_found"
        );
    }
    let mut request = f.request("w", "Latency");
    request.address.source_pane_id = None;
    assert_eq!(
        f.service.show(request).await.unwrap_err().code,
        "widget_target_not_found"
    );
    let mut request = f.request("w", "Latency");
    request.address.space_check = Some("s2".into());
    assert_eq!(
        f.service.show(request).await.unwrap_err().code,
        "widget_target_mismatch"
    );
    let mut request = f.request("w", "Latency");
    request.address.endpoint_path = Some("/different/socket".into());
    assert_eq!(
        f.service.show(request).await.unwrap_err().code,
        "widget_target_not_found"
    );
    for focused in [false, true] {
        {
            let mut fresh = f.browser.snapshot.lock();
            fresh.snapshot.tabs[0].focused = focused;
            fresh.snapshot.tabs[1].focused = focused;
        }
        let mut request = f.request("w", "Latency");
        request.address.locator = WidgetLocator::Space {
            space_id: "s1".into(),
        };
        assert_eq!(
            f.service.show(request).await.unwrap_err().code,
            "widget_target_no_focused_tab"
        );
    }
    f.paste.targets.lock()[0].terminal_id = "different-terminal".into();
    assert_eq!(
        f.service
            .show(f.request("w", "Latency"))
            .await
            .unwrap_err()
            .code,
        "widget_herdr_unavailable"
    );
    f.paste.targets.lock()[0].terminal_id = "term1".into();
    f.browser.snapshot.lock().snapshot.session_id = "wrong-session".into();
    assert_eq!(
        f.service
            .show(f.request("w", "Latency"))
            .await
            .unwrap_err()
            .code,
        "widget_target_not_found"
    );
}

#[tokio::test]
async fn changed_target_space_rejects_show_but_keeps_content_and_selection_readable() {
    let f = Fixture::new();
    let mut request = f.request("moved", "Latency");
    request.address.locator = WidgetLocator::Tab {
        tab_id: "t2".into(),
    };
    f.service.show(request.clone()).await.unwrap();
    let moved_key = WidgetKey {
        tab_id: "t2".into(),
        ..key("moved")
    };
    f.service
        .select(WidgetSelectRequest {
            key: moved_key.clone(),
            revision: 1,
            value: WidgetSelectValue::Choice {
                choice_id: "errors".into(),
            },
        })
        .unwrap();
    {
        let mut fresh = f.browser.snapshot.lock();
        fresh.snapshot.tabs[1].space_id = "s2".into();
        fresh
            .snapshot
            .panes
            .iter_mut()
            .find(|p| p.id == "p2")
            .unwrap()
            .space_id = "s2".into();
    }
    f.paste.targets.lock()[1].workspace_id = "s2".into();
    request.content = choices("Replacement");
    assert_eq!(
        f.service.show(request.clone()).await.unwrap_err().code,
        "widget_target_changed"
    );
    f.service.reconcile().await;
    assert!(matches!(
        f.service
            .content(WidgetContentRequest {
                key: moved_key,
                revision: 1
            })
            .unwrap()
            .body,
        WidgetBody::Choices { .. }
    ));
    let selected = f
        .service
        .selection(WidgetSelectionRequest {
            address: request.address,
            id: "moved".into(),
            wait_seconds: None,
        })
        .await
        .unwrap();
    assert_eq!(selected.status, WidgetSelectionStatus::Selected);
    assert_eq!(f.summaries()[0].space_id, "s1");
}

#[tokio::test]
async fn window_reports_compute_visibility_and_deregister_when_stream_drops() {
    let f = Fixture::new();
    assert_eq!(
        f.show("w", "Latency").await.displayed,
        WidgetDisplayed::NoWindow
    );
    let one = f.service.subscribe();
    assert_eq!(
        f.show("w", "Latency").await.displayed,
        WidgetDisplayed::WhenTabSelected
    );
    for blocker in [
        WidgetBlocker::Library,
        WidgetBlocker::Zoom,
        WidgetBlocker::Drag,
        WidgetBlocker::TooNarrow,
    ] {
        f.service
            .report_window(
                &one.window_id,
                WidgetWindowReport {
                    session_id: Some("daily".into()),
                    displayed_tab_id: Some("t1".into()),
                    blocker: Some(blocker),
                },
            )
            .unwrap();
        assert_eq!(
            f.show("w", "Latency").await.displayed,
            WidgetDisplayed::WhenVisible
        );
    }
    let two = f.service.subscribe();
    f.service
        .report_window(
            &two.window_id,
            WidgetWindowReport {
                session_id: Some("daily".into()),
                displayed_tab_id: Some("t1".into()),
                blocker: None,
            },
        )
        .unwrap();
    assert_eq!(f.show("w", "Latency").await.displayed, WidgetDisplayed::Now);
    let mut cross = f.request("cross", "Latency");
    cross.address.locator = WidgetLocator::Tab {
        tab_id: "t2".into(),
    };
    assert_eq!(
        f.service.show(cross).await.unwrap().displayed,
        WidgetDisplayed::WhenOpened
    );
    drop(two);
    assert_eq!(
        f.show("w", "Latency").await.displayed,
        WidgetDisplayed::WhenVisible
    );
    let old_id = one.window_id.clone();
    drop(one);
    assert_eq!(
        f.show("w", "Latency").await.displayed,
        WidgetDisplayed::NoWindow
    );
    assert_eq!(
        f.service
            .report_window(
                &old_id,
                WidgetWindowReport {
                    session_id: None,
                    displayed_tab_id: None,
                    blocker: None
                }
            )
            .unwrap_err()
            .code,
        "widget_usage"
    );
    let three = f.service.subscribe();
    assert_eq!(
        f.service
            .report_window(
                &three.window_id,
                WidgetWindowReport {
                    session_id: None,
                    displayed_tab_id: Some("t1".into()),
                    blocker: None
                }
            )
            .unwrap_err()
            .code,
        "widget_usage"
    );
    f.service
        .report_window(
            &three.window_id,
            WidgetWindowReport {
                session_id: Some("different-session".into()),
                displayed_tab_id: Some("t1".into()),
                blocker: None,
            },
        )
        .unwrap();
    assert_eq!(
        f.show("w", "Latency").await.displayed,
        WidgetDisplayed::WhenTabSelected
    );
}

#[tokio::test]
async fn selection_validates_choices_is_last_write_wins_and_rejects_wrong_body_kind() {
    let f = Fixture::new();
    f.show("choice", "Latency").await;
    assert_eq!(
        f.selection("choice").await.status,
        WidgetSelectionStatus::None
    );
    assert_eq!(
        f.select("choice", 1, "unknown").unwrap_err().code,
        "widget_usage"
    );
    f.select("choice", 1, "latency").unwrap();
    f.advance(5);
    f.select("choice", 1, "errors").unwrap();
    let selected = f.selection("choice").await;
    assert_eq!(selected.at_ms, Some(1_005));
    assert_eq!(
        serde_json::from_str::<serde_json::Value>(&selected.value_json.unwrap()).unwrap(),
        json!({"id":"errors","label":"Errors"})
    );
    let mut request = f.request("html", "unused");
    request.content = html("<p>Interactive result</p>");
    assert_eq!(
        f.service.show(request).await.unwrap().presentation,
        WidgetPresentation::Active
    );
    assert_eq!(
        f.selection("html").await.status,
        WidgetSelectionStatus::None
    );
    assert_eq!(
        f.select("html", 1, "errors").unwrap_err().code,
        "widget_selection_unavailable"
    );
    assert_eq!(
        f.select_page("choice", 1, r#"{"id":"errors"}"#)
            .unwrap_err()
            .code,
        "widget_selection_unavailable"
    );
    assert_eq!(
        f.service
            .selection(selection_request("missing", None))
            .await
            .unwrap_err()
            .code,
        "widget_target_not_found"
    );
    assert_eq!(
        f.service
            .selection(selection_request("choice", Some(3601)))
            .await
            .unwrap_err()
            .code,
        "widget_usage"
    );
}

#[tokio::test]
async fn page_selection_keeps_arbitrary_json_data_and_retains_it_until_explicit_clear() {
    let f = Fixture::new();
    let mut request = f.request("page", "unused");
    request.content = html("<button>First result</button>");
    f.service.show(request.clone()).await.unwrap();
    assert!(
        f.service
            .content(WidgetContentRequest {
                key: key("page"),
                revision: 1
            })
            .unwrap()
            .selection
            .is_none()
    );
    let waiter = f.waiter("page");
    await_waiters(&f.service, 1).await;
    let value = json!({
        "action": "filter",
        "range": [null, false, 0, -2.5, "3"],
        "at_ms": "2026-10-02T12:34:56Z",
        "nested": {"revision": 999, "key": ["other", "target"], "label": "<script>not code</script>"},
        "unicode": "日本語 🐈",
        "large_integer": 18_446_744_073_709_551_615_u64
    });
    f.select_page("page", 1, &serde_json::to_string_pretty(&value).unwrap())
        .unwrap();
    let selected = completed(waiter).await.unwrap();
    assert_eq!(
        (selected.status.clone(), selected.revision, selected.at_ms),
        (WidgetSelectionStatus::Selected, Some(1), Some(1_000))
    );
    assert_eq!(
        selected.value_json.as_deref(),
        Some(value.to_string().as_str())
    );
    assert_eq!(
        serde_json::from_str::<serde_json::Value>(selected.value_json.as_ref().unwrap()).unwrap(),
        value
    );
    assert_eq!(
        f.summaries()[0].selection.as_ref().unwrap().read_at_ms,
        Some(1_000)
    );
    f.advance(250);
    request.content = html("<button>Replacement result</button>");
    assert_eq!(f.service.show(request.clone()).await.unwrap().revision, 2);
    assert_eq!(
        f.service
            .content(WidgetContentRequest {
                key: key("page"),
                revision: 2
            })
            .unwrap()
            .selection,
        Some(selected.clone())
    );
    assert_eq!(f.selection("page").await, selected);
    assert_eq!(
        f.select_page("page", 1, r#"{"stale":true}"#)
            .unwrap_err()
            .code,
        "widget_stale"
    );
    assert_eq!(f.selection("page").await.value_json, selected.value_json);
    let mut other = address();
    other.source_pane_id = Some("p2".into());
    other.locator = WidgetLocator::Tab {
        tab_id: "t1".into(),
    };
    assert_eq!(
        f.service
            .selection(WidgetSelectionRequest {
                address: other.clone(),
                id: "page".into(),
                wait_seconds: None
            })
            .await
            .unwrap_err()
            .code,
        "widget_not_owner"
    );
    let mut takeover = request.clone();
    takeover.address = other;
    takeover.content = html("<p>Another source</p>");
    assert_eq!(
        f.service.show(takeover).await.unwrap_err().code,
        "widget_not_owner"
    );
    request.content = html("<button>Cleared result</button>");
    request.clear_selection = true;
    assert_eq!(f.service.show(request).await.unwrap().revision, 3);
    assert_eq!(
        f.selection("page").await.status,
        WidgetSelectionStatus::None
    );
    assert!(
        f.service
            .content(WidgetContentRequest {
                key: key("page"),
                revision: 3
            })
            .unwrap()
            .selection
            .is_none()
    );
    for value in [
        json!(null),
        json!(false),
        json!(7),
        json!("text"),
        json!([1, null]),
    ] {
        f.select_page("page", 3, &value.to_string()).unwrap();
        assert_eq!(
            f.selection("page").await.value_json.as_deref(),
            Some(value.to_string().as_str())
        );
    }
}

#[tokio::test]
async fn page_selection_rejects_malformed_nonfinite_and_oversized_utf8_without_mutation() {
    let f = Fixture::new();
    let mut request = f.request("page", "unused");
    request.content = html("<p>Result</p>");
    f.service.show(request).await.unwrap();
    f.select_page("page", 1, r#"{"accepted":true}"#).unwrap();
    for invalid in [
        "",
        "{",
        "undefined",
        "NaN",
        "Infinity",
        "[1e999]",
        r#"{"x":1} trailing"#,
    ] {
        assert_eq!(
            f.select_page("page", 1, invalid).unwrap_err().code,
            "widget_usage"
        );
    }
    let exact = format!("\"{}\"", "é".repeat((WIDGET_MAX_SELECTION_BYTES - 2) / 2));
    assert_eq!(exact.len(), WIDGET_MAX_SELECTION_BYTES);
    f.select_page("page", 1, &exact).unwrap();
    assert_eq!(
        f.select_page(
            "page",
            1,
            &format!("\"{}x\"", "é".repeat((WIDGET_MAX_SELECTION_BYTES - 2) / 2))
        )
        .unwrap_err()
        .code,
        "widget_too_large"
    );
    assert_eq!(
        f.selection("page").await.value_json.as_deref(),
        Some(exact.as_str())
    );
    // Bound raw wire bytes before parsing, including whitespace and escapes.
    let oversized_raw = format!("{}null", " ".repeat(WIDGET_MAX_SELECTION_BYTES));
    assert_eq!(
        f.select_page("page", 1, &oversized_raw).unwrap_err().code,
        "widget_too_large"
    );
    assert_eq!(
        f.selection("page").await.value_json.as_deref(),
        Some(exact.as_str())
    );
    let escaped = format!(" \"{}\" ", "\\u0061".repeat(2_000));
    f.select_page("page", 1, &escaped).unwrap();
    assert_eq!(
        f.selection("page").await.value_json.unwrap(),
        format!("\"{}\"", "a".repeat(2_000))
    );
}

#[tokio::test]
async fn waiters_observe_selection_user_removal_agent_close_and_zero_timeout() {
    let f = Fixture::new();
    f.show("selected", "Latency").await;
    let waiter = f.waiter("selected");
    await_waiters(&f.service, 1).await;
    f.select("selected", 1, "errors").unwrap();
    assert_eq!(
        completed(waiter).await.unwrap().status,
        WidgetSelectionStatus::Selected
    );
    await_waiters(&f.service, 0).await;
    f.show("removed", "Latency").await;
    let waiter = f.waiter("removed");
    await_waiters(&f.service, 1).await;
    f.remove("removed");
    let dismissed = completed(waiter).await.unwrap();
    assert_eq!(
        (dismissed.status, dismissed.removed_at_ms),
        (WidgetSelectionStatus::Dismissed, Some(1_000))
    );
    f.show("closed", "Latency").await;
    let waiter = f.waiter("closed");
    await_waiters(&f.service, 1).await;
    f.close("closed").await;
    assert_eq!(
        completed(waiter).await.unwrap_err().code,
        "widget_target_not_found"
    );
    f.show("timeout", "Latency").await;
    let timeout = f
        .service
        .selection(selection_request("timeout", Some(0)))
        .await
        .unwrap();
    assert_eq!(
        (timeout.status, timeout.revision),
        (WidgetSelectionStatus::Timeout, Some(1))
    );
    await_waiters(&f.service, 0).await;
}

#[tokio::test]
async fn waiter_retains_selection_if_revision_is_replaced_and_cleared_before_it_runs() {
    let f = Fixture::new();
    f.show("race", "Latency").await;
    let waiter = f.waiter("race");
    await_waiters(&f.service, 1).await;
    f.select("race", 1, "latency").unwrap();
    assert_eq!(
        f.summaries()[0].selection.as_ref().unwrap().read_at_ms,
        Some(1_000)
    );
    let mut replacement = f.request("race", "Replacement");
    replacement.clear_selection = true;
    f.service.show(replacement).await.unwrap();
    let selected = completed(waiter).await.unwrap();
    assert_eq!(
        (selected.status, selected.revision),
        (WidgetSelectionStatus::Selected, Some(1))
    );
    assert_eq!(
        f.selection("race").await.status,
        WidgetSelectionStatus::None
    );
}

#[tokio::test]
async fn ninth_waiter_is_busy_and_cancellation_releases_the_slot() {
    let f = Fixture::new();
    f.show("pending", "Latency").await;
    let mut waiters = Vec::new();
    for n in 1..=8 {
        waiters.push(f.waiter("pending"));
        await_waiters(&f.service, n).await;
    }
    assert_eq!(
        f.service
            .selection(selection_request("pending", Some(3600)))
            .await
            .unwrap_err()
            .code,
        "widget_busy"
    );
    let canceled = waiters.pop().unwrap();
    canceled.abort();
    assert!(canceled.await.unwrap_err().is_cancelled());
    await_waiters(&f.service, 7).await;
    waiters.push(f.waiter("pending"));
    await_waiters(&f.service, 8).await;
    f.select("pending", 1, "errors").unwrap();
    for waiter in waiters {
        assert_eq!(
            completed(waiter).await.unwrap().status,
            WidgetSelectionStatus::Selected
        );
    }
    await_waiters(&f.service, 0).await;
}

#[tokio::test]
async fn retirement_for_absent_tab_no_panes_and_endpoint_change_wakes_waiters() {
    for reason in 0..3 {
        let f = Fixture::new();
        f.show("live", "Latency").await;
        f.show("removed", "Latency").await;
        f.remove("removed");
        let waiter = f.waiter("live");
        await_waiters(&f.service, 1).await;
        let mut events = f.service.subscribe().events;
        {
            let mut fresh = f.browser.snapshot.lock();
            match reason {
                0 => fresh.snapshot.tabs.retain(|tab| tab.id != "t1"),
                1 => fresh.snapshot.panes.retain(|pane| pane.tab_id != "t1"),
                _ => fresh.endpoint_identity = "endpoint-generation-2".into(),
            }
        }
        f.service.reconcile().await;
        assert_eq!(
            completed(waiter).await.unwrap().status,
            WidgetSelectionStatus::Retired
        );
        assert_eq!(
            f.service
                .content(WidgetContentRequest {
                    key: key("live"),
                    revision: 1
                })
                .unwrap_err()
                .code,
            "widget_target_not_found"
        );
        assert!(f.summaries().is_empty());
        assert!(
            matches!(events.try_recv().unwrap(), WidgetEvent::Removed { key: removed, reason: WidgetRemovalReason::Retired, .. } if removed == key("live"))
        );
        if reason == 2 {
            for target in f.paste.targets.lock().iter_mut() {
                target.endpoint_identity = "endpoint-generation-2".into();
            }
            assert_eq!(f.show("removed", "Fresh").await.revision, 1);
        }
    }
}

#[tokio::test]
async fn failed_fresh_snapshot_retires_nothing_and_failed_paste_preserves_source_status() {
    let f = Fixture::new();
    f.show("live", "Latency").await;
    let mut stream = f.service.subscribe();
    f.browser.failed.store(true, Ordering::Relaxed);
    f.service.reconcile().await;
    assert_eq!(
        f.service
            .show(f.request("new", "Latency"))
            .await
            .unwrap_err()
            .code,
        "widget_herdr_unavailable"
    );
    assert_eq!(
        f.summaries()[0].source.as_ref().unwrap().status,
        WidgetSourceStatus::Present
    );
    f.browser.failed.store(false, Ordering::Relaxed);
    f.paste.failed.store(true, Ordering::Relaxed);
    f.paste.targets.lock()[0].agent_fingerprint = "different".into();
    f.service.reconcile().await;
    assert_eq!(
        f.summaries()[0].source.as_ref().unwrap().status,
        WidgetSourceStatus::Present
    );
    assert!(matches!(
        stream.events.try_recv(),
        Err(broadcast::error::TryRecvError::Empty)
    ));
    assert_eq!(
        f.service
            .list(WidgetListRequest { address: address() })
            .await
            .unwrap_err()
            .code,
        "widget_herdr_unavailable"
    );
}

#[tokio::test]
async fn source_status_tracks_fresh_fingerprint_restart_closed_and_return_without_retiring_target()
{
    let f = Fixture::new();
    f.show("live", "Latency").await;
    let mut stream = f.service.subscribe();
    f.service.reconcile().await;
    assert!(matches!(
        stream.events.try_recv(),
        Err(broadcast::error::TryRecvError::Empty)
    ));
    f.paste.targets.lock()[0].agent_fingerprint = "restarted-agent".into();
    f.service.reconcile().await;
    assert_eq!(
        f.summaries()[0].source.as_ref().unwrap().status,
        WidgetSourceStatus::Restarted
    );
    assert!(
        matches!(stream.events.try_recv().unwrap(), WidgetEvent::Upserted { widget, .. } if widget.source.as_ref().unwrap().status == WidgetSourceStatus::Restarted)
    );
    f.paste.targets.lock()[0].agent_fingerprint = "fingerprint-agent-one".into();
    f.service.reconcile().await;
    assert_eq!(
        f.summaries()[0].source.as_ref().unwrap().status,
        WidgetSourceStatus::Present
    );
    let source_pane = f.browser.snapshot.lock().snapshot.panes[0].clone();
    f.browser
        .snapshot
        .lock()
        .snapshot
        .panes
        .retain(|pane| pane.id != "p1");
    f.service.reconcile().await;
    assert_eq!(
        f.summaries()[0].source.as_ref().unwrap().status,
        WidgetSourceStatus::Closed
    );
    assert!(
        f.service
            .content(WidgetContentRequest {
                key: key("live"),
                revision: 1
            })
            .is_ok()
    );
    f.browser.snapshot.lock().snapshot.panes.push(source_pane);
    f.service.reconcile().await;
    assert_eq!(
        f.summaries()[0].source.as_ref().unwrap().status,
        WidgetSourceStatus::Present
    );
    f.browser
        .snapshot
        .lock()
        .snapshot
        .panes
        .iter_mut()
        .find(|pane| pane.id == "p1")
        .unwrap()
        .terminal_id = "restarted-terminal".into();
    f.service.reconcile().await;
    assert_eq!(
        f.summaries()[0].source.as_ref().unwrap().status,
        WidgetSourceStatus::Restarted
    );
}

#[tokio::test]
async fn shutdown_retires_every_waiter_and_rejects_further_operations() {
    let f = Fixture::new();
    f.show("live", "Latency").await;
    let one = f.waiter("live");
    let two = f.waiter("live");
    await_waiters(&f.service, 2).await;
    let mut stream = f.service.subscribe();
    f.service.shutdown();
    assert_eq!(
        completed(one).await.unwrap().status,
        WidgetSelectionStatus::Retired
    );
    assert_eq!(
        completed(two).await.unwrap().status,
        WidgetSelectionStatus::Retired
    );
    await_waiters(&f.service, 0).await;
    assert!(matches!(
        stream.events.try_recv().unwrap(),
        WidgetEvent::Removed {
            reason: WidgetRemovalReason::Retired,
            ..
        }
    ));
    f.service.shutdown();
    assert!(matches!(
        stream.events.try_recv(),
        Err(broadcast::error::TryRecvError::Empty)
    ));
    assert_eq!(
        f.service
            .show(f.request("new", "Latency"))
            .await
            .unwrap_err()
            .code,
        "widget_retired"
    );
    assert_eq!(
        f.service
            .list(WidgetListRequest { address: address() })
            .await
            .unwrap_err()
            .code,
        "widget_retired"
    );
    assert_eq!(
        f.service
            .selection(selection_request("live", None))
            .await
            .unwrap_err()
            .code,
        "widget_retired"
    );
    assert_eq!(
        f.service
            .content(WidgetContentRequest {
                key: key("live"),
                revision: 1
            })
            .unwrap_err()
            .code,
        "widget_retired"
    );
    assert_eq!(
        f.service
            .remove(WidgetRemoveRequest { key: key("live") })
            .unwrap_err()
            .code,
        "widget_retired"
    );
    assert_eq!(
        f.select("live", 1, "errors").unwrap_err().code,
        "widget_retired"
    );
}

#[tokio::test]
async fn waiter_keeps_dismissal_when_widget_reopens_before_waiter_runs() {
    let f = Fixture::new();
    f.show("race", "Latency").await;
    let waiter = f.waiter("race");
    await_waiters(&f.service, 1).await;
    f.remove("race");
    let mut reopen = f.request("race", "Reopened");
    reopen.reopen = true;
    assert_eq!(f.service.show(reopen).await.unwrap().revision, 2);
    let dismissed = completed(waiter).await.unwrap();
    assert_eq!(
        (
            dismissed.status,
            dismissed.revision,
            dismissed.removed_at_ms
        ),
        (WidgetSelectionStatus::Dismissed, Some(1), Some(1_000))
    );
    assert_eq!(
        f.selection("race").await.status,
        WidgetSelectionStatus::None
    );
}

#[tokio::test]
async fn fresh_cli_operation_retires_old_endpoint_generation_before_reusing_widget_key() {
    let f = Fixture::new();
    f.show("same", "Old generation").await;
    let waiter = f.waiter("same");
    await_waiters(&f.service, 1).await;
    f.browser.snapshot.lock().endpoint_identity = "endpoint-generation-2".into();
    for target in f.paste.targets.lock().iter_mut() {
        target.endpoint_identity = "endpoint-generation-2".into();
    }
    let shown = f.show("same", "New generation").await;
    assert_eq!(
        (shown.result, shown.revision),
        (WidgetShowResult::Opened, 1)
    );
    assert_eq!(
        completed(waiter).await.unwrap().status,
        WidgetSelectionStatus::Retired
    );
    let summaries = f.summaries();
    assert_eq!(
        summaries.iter().map(|w| &w.key).collect::<Vec<_>>(),
        [&key("same")]
    );
    let body = f
        .service
        .content(WidgetContentRequest {
            key: key("same"),
            revision: 1,
        })
        .unwrap()
        .body;
    assert!(
        matches!(body, WidgetBody::Choices { spec } if spec.choices[0].label == "New generation")
    );
}

#[tokio::test]
async fn content_validation_rejects_bad_digest_encoding_utf8_ids_and_oversized_inputs() {
    let f = Fixture::new();
    let mut request = f.request("bad", "Latency");
    request.id = "Bad/ID".into();
    assert_eq!(
        f.service.show(request).await.unwrap_err().code,
        "widget_usage"
    );
    let invalid = [
        WidgetContentInput::Html {
            content_base64: "!".into(),
            sha256: "0".repeat(64),
            from: WidgetInputKind::Stdin,
            name: None,
        },
        WidgetContentInput::Html {
            content_base64: STANDARD.encode("good HTML"),
            sha256: "0".repeat(64),
            from: WidgetInputKind::Stdin,
            name: None,
        },
        WidgetContentInput::Html {
            content_base64: STANDARD.encode([0xff]),
            sha256: format!("{:x}", Sha256::digest([0xff])),
            from: WidgetInputKind::Stdin,
            name: None,
        },
    ];
    for content in invalid {
        let mut request = f.request("bad", "Latency");
        request.content = content;
        assert_eq!(
            f.service.show(request).await.unwrap_err().code,
            "widget_usage"
        );
    }
    let mut request = f.request("large", "Latency");
    request.content = html(&"x".repeat(WIDGET_MAX_HTML_BYTES + 1));
    assert_eq!(
        f.service.show(request).await.unwrap_err().code,
        "widget_too_large"
    );
    let spec_json = " ".repeat(WIDGET_MAX_CHOICES_BYTES + 1);
    let mut request = f.request("large", "Latency");
    request.content = WidgetContentInput::Choices {
        sha256: format!("{:x}", Sha256::digest(spec_json.as_bytes())),
        spec_json,
        from: WidgetInputKind::File,
        name: None,
    };
    assert_eq!(
        f.service.show(request).await.unwrap_err().code,
        "widget_too_large"
    );
    assert!(f.list().await.is_empty());
}

#[tokio::test]
async fn owner_html_byte_cap_spans_tabs_and_releases_bytes_on_close_replace_and_user_remove() {
    let f = Fixture::new();
    let overhead = preflight::sanitize("<p></p>").unwrap().document.len();
    let document = format!("<p>{}</p>", "x".repeat(WIDGET_MAX_HTML_BYTES - overhead));
    assert!(document.len() < WIDGET_MAX_HTML_BYTES);
    let fits = WIDGET_MAX_TOTAL_HTML_BYTES / WIDGET_MAX_HTML_BYTES;
    let tabs_needed = (fits + 3).div_ceil(WIDGET_MAX_LIVE_PER_TAB);
    {
        let mut fresh = f.browser.snapshot.lock();
        for n in 4..=tabs_needed {
            let mut tab = fresh.snapshot.tabs[0].clone();
            tab.id = format!("t{n}");
            tab.number = n as u32;
            tab.pane_count = 1;
            tab.focused = false;
            tab.focused_pane_id = Some(format!("p{n}"));
            let mut pane = fresh.snapshot.panes[0].clone();
            pane.id = format!("p{n}");
            pane.tab_id = tab.id.clone();
            pane.terminal_id = format!("term{n}");
            pane.focused = false;
            fresh.snapshot.tabs.push(tab);
            fresh.snapshot.panes.push(pane);
        }
    }
    let input = html(&document);
    let request = |n: usize| {
        let mut request = f.request(&format!("large{n}"), "unused");
        request.address.locator = WidgetLocator::Tab {
            tab_id: format!("t{}", n / WIDGET_MAX_LIVE_PER_TAB + 1),
        };
        request.content = input.clone();
        request
    };
    f.service.show(request(0)).await.unwrap();
    let body = f
        .service
        .content(WidgetContentRequest {
            key: key("large0"),
            revision: 1,
        })
        .unwrap()
        .body;
    let WidgetBody::Html { document } = body else {
        panic!("expected HTML body");
    };
    assert_eq!(document.len(), WIDGET_MAX_HTML_BYTES);
    assert_eq!(f.service.store.lock().html_bytes, document.len());
    for n in 1..fits {
        f.advance(60_000);
        f.service.show(request(n)).await.unwrap();
    }
    assert_eq!(
        f.service.store.lock().html_bytes,
        WIDGET_MAX_TOTAL_HTML_BYTES
    );
    f.advance(60_000);
    let error = f.service.show(request(fits)).await.unwrap_err();
    assert_eq!(
        (error.code.as_str(), error.message.as_str()),
        ("widget_limit", "owner HTML byte limit reached")
    );
    f.close("large0").await;
    f.service.show(request(fits)).await.unwrap();
    let mut replacement = request(1);
    replacement.content = html("<p>Small replacement</p>");
    assert_eq!(
        f.service.show(replacement).await.unwrap().result,
        WidgetShowResult::Replaced
    );
    // The small replacement still consumes bytes: releasing almost 1 MiB
    // cannot admit another full-size document at the exact owner boundary.
    assert_eq!(
        f.service.show(request(fits + 1)).await.unwrap_err().code,
        "widget_limit"
    );
    f.remove("large2");
    f.service.show(request(fits + 1)).await.unwrap();
    assert_eq!(
        f.service.show(request(fits + 2)).await.unwrap_err().code,
        "widget_limit"
    );
    f.close("large1").await;
    f.service.show(request(fits + 2)).await.unwrap();
}

#[tokio::test]
async fn owner_snapshot_budget_accepts_boundary_rejects_growth_and_releases_on_removal_and_retirement()
 {
    let f = Fixture::new();
    let mut request = f.request("bound", "unused");
    request.content = html("<p>Result</p>");
    f.service.show(request.clone()).await.unwrap();
    let baseline = f.service.store.lock().snapshot_bytes;
    f.close("bound").await;
    assert_eq!(f.service.store.lock().snapshot_bytes, 0);
    // Herdr identity text is carried into summary metadata. A large identity
    // makes the exact aggregate boundary deterministic without thousands of tabs.
    let terminal_len =
        WIDGET_MAX_SNAPSHOT_BYTES - SNAPSHOT_ENVELOPE_BYTES - baseline + "term1".len();
    let set_terminal = |len: usize| {
        f.browser.snapshot.lock().snapshot.panes[0].terminal_id = "t".repeat(len);
        f.paste.targets.lock()[0].terminal_id = "t".repeat(len);
    };
    set_terminal(terminal_len);
    f.service.show(request.clone()).await.unwrap();
    assert_eq!(
        f.service.store.lock().snapshot_bytes + SNAPSHOT_ENVELOPE_BYTES,
        WIDGET_MAX_SNAPSHOT_BYTES
    );
    assert!(
        serde_json::to_vec(&f.service.subscribe().snapshot)
            .unwrap()
            .len()
            <= WIDGET_MAX_SNAPSHOT_BYTES
    );
    // Selection facts and maximum-width times fit reserved headroom without
    // rejecting selections after a widget has already been accepted.
    f.clock.store(u64::MAX, Ordering::Relaxed);
    f.select_page("bound", 1, r#"{"at_ms":"unchanged data"}"#)
        .unwrap();
    f.selection("bound").await;
    assert!(
        serde_json::to_vec(&f.service.subscribe().snapshot)
            .unwrap()
            .len()
            <= WIDGET_MAX_SNAPSHOT_BYTES
    );
    f.clock.store(1_000, Ordering::Relaxed);
    let before = f.service.store.lock().sequence;
    let mut replacement = request.clone();
    replacement.title = Some("a longer title than bound".into());
    replacement.content = html("<p>Replacement</p>");
    assert_eq!(
        f.service.show(replacement.clone()).await.unwrap_err().code,
        "widget_limit"
    );
    assert_eq!(f.service.store.lock().sequence, before);
    assert_eq!(
        f.service
            .content(WidgetContentRequest {
                key: key("bound"),
                revision: 1
            })
            .unwrap()
            .revision,
        1
    );
    let mut second = request.clone();
    second.id = "other".into();
    assert_eq!(
        f.service.show(second.clone()).await.unwrap_err().code,
        "widget_limit"
    );
    assert_eq!(f.list().await.len(), 1);
    // A smaller replacement releases the old incarnation's reserved bytes.
    set_terminal("term1".len());
    assert_eq!(f.service.show(replacement).await.unwrap().revision, 2);
    assert!(f.service.store.lock().snapshot_bytes < 4_096);
    f.service.show(second).await.unwrap();
    f.remove("bound");
    let remaining_bytes = summary_snapshot_bytes(&f.summaries()[0]).unwrap();
    assert_eq!(f.service.store.lock().snapshot_bytes, remaining_bytes);
    f.close("other").await;
    assert_eq!(f.service.store.lock().snapshot_bytes, 0);
    // Going one byte past the single-summary allowance is rejected too.
    set_terminal(terminal_len + 1);
    request.id = "fresh".into(); // Same-width title/id preserves exact sizing.
    assert_eq!(
        f.service.show(request.clone()).await.unwrap_err().code,
        "widget_limit"
    );
    set_terminal("term1".len());
    f.advance(60_000);
    f.service.show(request).await.unwrap();
    f.browser.snapshot.lock().endpoint_identity = "endpoint-generation-2".into();
    f.service.reconcile().await;
    assert_eq!(f.service.store.lock().snapshot_bytes, 0);
    f.browser.snapshot.lock().endpoint_identity = "endpoint-generation-1".into();
    f.show("shutdown", "Latency").await;
    f.service.shutdown();
    assert_eq!(f.service.store.lock().snapshot_bytes, 0);
}

#[tokio::test]
async fn lagged_waiter_retains_selection_outcome_despite_later_replacement_events() {
    let f = Fixture::new();
    f.show("lagged", "Latency").await;
    let waiter = f.waiter("lagged");
    await_waiters(&f.service, 1).await;
    f.select("lagged", 1, "errors").unwrap();
    for n in 0..300 {
        f.show("lagged", &format!("Replacement {n}")).await;
    }
    let selected = completed(waiter).await.unwrap();
    assert_eq!(
        (selected.status, selected.revision),
        (WidgetSelectionStatus::Selected, Some(1))
    );
    assert_eq!(
        serde_json::from_str::<serde_json::Value>(&selected.value_json.unwrap()).unwrap(),
        json!({"id":"errors","label":"Errors"})
    );
}
