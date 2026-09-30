use super::*;
use std::{path::PathBuf, sync::Arc};
use tokio::sync::Mutex;
use async_trait::async_trait;
use crate::browser::{BrowserHerdrAdapter, BrowserHerdrSnapshot};
use crate::config::BrowserConfiguration;
use crate::paste_adapter::CommentPasteAdapter;
use cockpit_protocol::browser::{BrowserTarget, BrowserWorkScope};
use uuid::Uuid;

struct FixtureAdapter {
    snapshot: Mutex<BrowserHerdrSnapshot>,
    targets: Mutex<Vec<CommentPasteTarget>>,
    writes: Mutex<Vec<String>>,
    change_on_focus: std::sync::atomic::AtomicBool,
}
#[async_trait]
impl BrowserHerdrAdapter for FixtureAdapter {
    async fn browser_snapshot(&self, _: &str) -> Result<BrowserHerdrSnapshot, InspectionError> {
        Ok(self.snapshot.lock().await.clone())
    }
}
#[async_trait]
impl CommentPasteAdapter for FixtureAdapter {
    async fn comment_paste_targets(&self, _: &str) -> Result<Vec<CommentPasteTarget>, InspectionError> {
        Ok(self.targets.lock().await.clone())
    }
    async fn focus_comment_paste_target(&self, target: &CommentPasteTarget) -> Result<(), InspectionError> {
        let mut snapshot = self.snapshot.lock().await;
        snapshot.snapshot.focused_pane_id = Some(target.pane_id.clone());
        if self.change_on_focus.load(std::sync::atomic::Ordering::SeqCst) {
            snapshot.snapshot.focused_tab_id = Some("w1:t2".into());
        }
        Ok(())
    }
    async fn confirm_comment_paste_target_focus(&self, _: &CommentPasteTarget) -> Result<(), InspectionError> { Ok(()) }
    async fn send_comment_paste(&self, _: &CommentPasteTarget, payload: &str) -> Result<(), InspectionError> {
        self.writes.lock().await.push(payload.to_owned());
        Ok(())
    }
}
fn fixture() -> (BrowserService, Arc<FixtureAdapter>, PathBuf) {
    let root = std::env::temp_dir().join(format!("cockpit-browser-work-{}", Uuid::new_v4()));
    let snapshot = serde_json::from_value(json!({
        "session_id":"session", "server_instance":"server", "version":"1", "protocol":1,
        "focused_space_id":"w1", "focused_tab_id":"w1:t1", "focused_pane_id":"w1:p1",
        "spaces":[{"id":"w1","label":"Space","number":1,"tab_count":2,"pane_count":2,
            "focused":true,"agent_status":"idle","git":null}],
        "tabs":[
            {"id":"w1:t1","space_id":"w1","label":"One","number":1,"pane_count":1,"focused":true,"focused_pane_id":"w1:p1"},
            {"id":"w1:t2","space_id":"w1","label":"Two","number":2,"pane_count":1,"focused":false,"focused_pane_id":"w1:p2"}
        ],
        "panes":[
            {"id":"w1:p1","terminal_id":"term1","space_id":"w1","tab_id":"w1:t1","title":null,"focused":true,"agent":"omp","agent_status":"idle","revision":1},
            {"id":"w1:p2","terminal_id":"term2","space_id":"w1","tab_id":"w1:t2","title":null,"focused":false,"agent":"omp","agent_status":"idle","revision":1}
        ], "agents":[]
    })).unwrap();
    let recipient = |pane: &str, tab: &str, terminal: &str| CommentPasteTarget {
        endpoint_identity: "endpoint".into(), session_id: "session".into(),
        workspace_id: "w1".into(), tab_id: tab.into(), pane_id: pane.into(),
        terminal_id: terminal.into(), agent_label: "omp".into(), agent_fingerprint: terminal.into(),
    };
    let adapter = Arc::new(FixtureAdapter {
        snapshot: Mutex::new(BrowserHerdrSnapshot {
            endpoint_identity: "endpoint".into(), endpoint_path: "/fixture/socket".into(), snapshot,
        }),
        targets: Mutex::new(vec![recipient("w1:p2","w1:t2","term2"), recipient("w1:p1","w1:t1","term1")]),
        writes: Mutex::new(Vec::new()), change_on_focus: false.into(),
    });
    let service = service_for(root.clone(), adapter.clone());
    (service, adapter, root)
}
fn service_for(root: PathBuf, adapter: Arc<FixtureAdapter>) -> BrowserService {
    BrowserService::new(BrowserConfiguration {
        playwright_cli: "unused-playwright-cli".into(), default_url: "about:blank".into(),
        chromium_executable: None, node_executable: None, browser_helper: None,
        playwright_core: None, feedback_retention_seconds: 3600,
        feedback_max_store_bytes: 256 * 1024 * 1024,
    }, root, adapter.clone()).unwrap().with_paste_adapter(adapter)
}
fn seed_capture(service: &BrowserService, key: &str) -> (String, String, String) {
    seed_capture_for(service, key, "original-session", "old-space", "Original Space")
}
fn seed_capture_for(
    service: &BrowserService, key: &str, session_id: &str, space_id: &str, space_label: &str,
) -> (String, String, String) {
    let capture_id = Uuid::new_v4().to_string();
    let annotation_id = Uuid::new_v4().to_string();
    let incarnation = Uuid::new_v4().to_string();
    let png = BASE64.encode([137,80,78,71,13,10,26,10,0,0,0,13,b'I',b'H',b'D',b'R',0,0,0,1,0,0,0,1]);
    let context = serde_json::from_value(json!({
        "association_key":key,"session_id":session_id,"space_id":space_id,
        "space_label":space_label,"playwright_session":"old-browser","working_directory":"/old",
        "invocation":"old invocation","browser_instance":incarnation,"inline_provenance":null
    })).unwrap();
    let submission = serde_json::from_value(json!({
        "association_key":key,"browser_instance":incarnation,"capture_id":capture_id,
        "page":{"url":"https://example.test/","title":"Saved","tab_id":null,
            "document_id":"document","captured_at":"2026-09-09T00:00:00Z",
            "viewport":{"width":1.0,"height":1.0,"scroll_x":0.0,"scroll_y":0.0,
                "device_pixel_ratio":1.0,"visual_scale":1.0},"image_width":1,"image_height":1},
        "annotations":[{"id":annotation_id,"kind":"region","color":"#ff0000",
            "points":[],"bounds":{"x":0.0,"y":0.0,"width":1.0,"height":1.0},
            "element":null,"comment":"Saved note"}],"png_base64":png
    })).unwrap();
    service.feedback.save(context, submission).unwrap();
    (capture_id, annotation_id, png)
}
fn tab_scope(tab_id: &str) -> BrowserWorkScope {
    BrowserWorkScope::Tab { target: BrowserTarget {
        session_id: "session".into(), tab_id: Some(tab_id.into()), pane_id: None, endpoint_path: None,
    }}
}
fn send(scope: BrowserWorkScope, id: String) -> BrowserFeedbackSendRequest {
    BrowserFeedbackSendRequest { scope, operation_id: Uuid::new_v4().to_string(), ids: vec![id], acknowledge_duplicate_risk: false }
}
#[tokio::test]
async fn tab_delivery_requires_its_focused_tab_not_merely_the_same_space() {
    let (service, adapter, root) = fixture();
    let key = crate::browser::association_key("endpoint", "session", "w1:t2");
    let (_, id, _) = seed_capture(&service, &key);
    let error = service.send_feedback(send(tab_scope("w1:t2"), id)).await.unwrap_err();
    assert_eq!(error.code, "browser_feedback_tab_inactive");
    assert!(adapter.writes.lock().await.is_empty());
    assert_eq!(service.feedback.list(&key).unwrap().pending_count, 1);
    std::fs::remove_dir_all(root).unwrap();
}

#[tokio::test]
async fn tab_delivery_rechecks_focus_and_requires_duplicate_risk_acknowledgement() {
    let (service, adapter, root) = fixture();
    let key = crate::browser::association_key("endpoint", "session", "w1:t1");
    let (_, id, _) = seed_capture(&service, &key);
    adapter.change_on_focus.store(true, std::sync::atomic::Ordering::SeqCst);
    let rejected = service.send_feedback(send(tab_scope("w1:t1"), id.clone())).await.unwrap();
    assert_eq!(rejected.state, CommentPasteState::Rejected);
    assert!(adapter.writes.lock().await.is_empty());
    assert_eq!(service.feedback.list(&key).unwrap().pending_count, 1);
    adapter.change_on_focus.store(false, std::sync::atomic::Ordering::SeqCst);
    adapter.snapshot.lock().await.snapshot.focused_tab_id = Some("w1:t1".into());
    let target = adapter.targets.lock().await[1].clone();
    service.persist_outcome("unknown-operation", &key, &[id.clone()], Some(target.clone()),
        CommentPasteState::OutcomeUnknown, "unknown").unwrap();
    let mut request = send(tab_scope("w1:t1"), id.clone());
    let error = service.send_feedback(request.clone()).await.unwrap_err();
    assert_eq!(error.code, "browser_feedback_duplicate_risk");
    request.acknowledge_duplicate_risk = true;
    let accepted = service.send_feedback(request.clone()).await.unwrap();
    assert_eq!(accepted.state, CommentPasteState::Accepted);
    assert_eq!(accepted.target, Some(target));
    assert_eq!(accepted.acknowledged_ids, vec![id]);
    assert_eq!(accepted.pending_count, 0);
    let repeated = service.send_feedback(request).await.unwrap();
    assert_eq!(repeated.state, CommentPasteState::Accepted);
    let writes = adapter.writes.lock().await;
    assert_eq!(writes.len(), 1);
    let payload: Value = serde_json::from_str(writes[0].strip_prefix(PASTE_PREFIX).unwrap()
        .strip_suffix(PASTE_SUFFIX).unwrap()).unwrap();
    assert_eq!(payload["addressing"]["tab_id"], "w1:t1");
    assert_eq!(payload["captures"][0]["context"]["space_id"], "old-space");
    drop(writes);
    std::fs::remove_dir_all(root).unwrap();
}

#[tokio::test]
async fn cleanup_retries_failed_discard_and_removes_only_its_association_work() {
    let (service, adapter, root) = fixture();
    let target = crate::browser::ResolvedTarget {
        endpoint_identity: "endpoint".into(), endpoint_path: "/fixture/socket".into(),
        session_id: "session".into(), space_id: "w1".into(), space_label: "Space".into(),
        tab_id: "w1:t1".into(), tab_label: "One".into(), tab_present: true,
    };
    let mut receipt = service.load_or_create(&target).unwrap();
    let key = receipt.association_key.clone();
    let other_key = crate::browser::association_key("endpoint", "session", "w1:t2");
    let (capture_id, id, png) = seed_capture(&service, &key);
    let capture = service.feedback.list(&key).unwrap().captures.remove(0);
    let (acknowledged_capture, acknowledged_id, _) = seed_capture(&service, &key);
    service.feedback.ack(&key, &[acknowledged_id]).unwrap();
    let (other_capture, other_id, _) = seed_capture(&service, &other_key);
    let paste_targets = adapter.targets.lock().await;
    for (association, annotation, paste_target) in [
        (&key, &id, &paste_targets[1]),
        (&other_key, &other_id, &paste_targets[0]),
    ] {
        service.persist_outcome(association, association, &[annotation.clone()],
            Some(paste_target.clone()), CommentPasteState::OutcomeUnknown, "unknown").unwrap();
    }
    drop(paste_targets);
    let store = service.draft_store().unwrap();
    let identity = crate::browser::drafts::BrowserDraftIdentity {
        association_key: key.clone(), browser_incarnation: capture.context.browser_instance.clone(),
        target_id: "first-document".into(), document_generation: 1,
    };
    let draft = store.open(&identity, None).unwrap();
    let second = store.open(&crate::browser::drafts::BrowserDraftIdentity {
        target_id: "second-document".into(), ..identity.clone()
    }, None).unwrap();
    let other = store.open(&crate::browser::drafts::BrowserDraftIdentity {
        association_key: other_key.clone(), ..identity
    }, None).unwrap();
    let pending_path = service.root.join("drafts").join(format!("pending-{key}.json"));
    let preparation_path = service.root.join("drafts").join(format!("preparation-{key}.json"));
    std::fs::write(&pending_path, serde_json::to_vec(&json!({
        "format_version":1,"association_key":key,"browser_incarnation":capture.context.browser_instance,
        "draft_id":draft.draft_id,"draft_revision":1,"annotation_ids":[id],
        "original_annotation_digests":[],"context":capture.context,
        "submission":{"association_key":key,"browser_instance":capture.context.browser_instance,
            "capture_id":capture_id,"page":capture.page,"annotations":capture.annotations,"png_base64":png},
        "last_error":"interrupted"
    })).unwrap()).unwrap();
    std::fs::write(&preparation_path, serde_json::to_vec(&json!({
        "format_version":1,"association_key":key,"browser_incarnation":capture.context.browser_instance,
        "capture_id":capture_id,"draft_id":second.draft_id,"draft_revision":1,
        "annotation_ids":[id],"original_annotation_digests":[],"context":capture.context
    })).unwrap()).unwrap();
    // A failed artifact removal must keep both its capture record and the
    // browser receipt. Retry must work even when the image is now absent.
    let image_path = PathBuf::from(&capture.image_path);
    std::fs::remove_file(&image_path).unwrap();
    std::fs::create_dir(&image_path).unwrap();
    let failed = service.finish_cleanup(&mut receipt).await.unwrap();
    assert_eq!(failed.cleanup, cockpit_protocol::browser::BrowserCleanupState::Failed);
    let failure_status = service.cleanup_status().await.unwrap();
    let failure = failure_status.failures.iter().find(|failure| failure.association_key == key)
        .expect("failed discard must expose an actionable, exact-association cleanup failure");
    assert!(matches!(&failure.scope, cockpit_protocol::browser::BrowserCleanupScope::Tab { session_id, tab_id }
        if session_id == "session" && tab_id == "w1:t1"));
    assert!(service.load(&key).unwrap().is_some(), "failed discard must retain the receipt for Retry");
    assert!(service.root.join("feedback").join(format!("capture-{capture_id}.json")).exists());
    std::fs::remove_dir(&image_path).unwrap();
    let cleaned = service.retry_cleanup(cockpit_protocol::browser::BrowserCleanupRetryRequest {
        association_key: key.clone(),
    }).await.unwrap();
    assert!(cleaned.failures.iter().all(|failure| failure.association_key != key));
    assert!(service.load(&key).unwrap().is_none());
    assert!(service.feedback.list(&key).unwrap().captures.is_empty());
    assert!(service.feedback.load_delivery(&key).unwrap().is_none());
    for id in [&capture_id, &acknowledged_capture] {
        assert!(!service.root.join("feedback").join(format!("capture-{id}.json")).exists());
        assert!(!service.root.join("artifacts").join(format!("capture-{id}.png")).exists());
    }
    assert!(store.list(&key).unwrap().drafts.is_empty());
    assert!(!pending_path.exists());
    assert!(!preparation_path.exists());
    assert_eq!(service.feedback.list(&other_key).unwrap().captures[0].id, other_capture);
    assert_eq!(service.feedback.list(&other_key).unwrap().captures[0].pending_ids, vec![other_id]);
    assert!(service.feedback.load_delivery(&other_key).unwrap().is_some());
    assert_eq!(store.list(&other_key).unwrap().drafts[0].draft_id, other.draft_id);
    service.feedback.discard_association(&key).unwrap();
    store.discard_association(&key).unwrap();
    std::fs::remove_dir_all(root).unwrap();
}
