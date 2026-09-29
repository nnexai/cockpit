use super::*;
use std::{path::PathBuf, sync::Arc};
use tokio::sync::Mutex;
use async_trait::async_trait;
use crate::browser::{BrowserHerdrAdapter, BrowserHerdrSnapshot};
use crate::config::BrowserConfiguration;
use crate::paste_adapter::CommentPasteAdapter;
use cockpit_protocol::browser::{BrowserFeedbackAckRequest, BrowserTarget};
use cockpit_protocol::browser_view::{BrowserDraftRecoveryAction, BrowserDraftRecoveryRequest,
    BrowserViewCommandOutcome, BrowserViewDraftEditorState};
use uuid::Uuid;

const KEY: &str = "0123456789abcdef01234567";
struct FixtureAdapter {
    snapshot: Mutex<BrowserHerdrSnapshot>,
    offline: std::sync::atomic::AtomicBool,
    targets: Mutex<Vec<CommentPasteTarget>>,
    writes: Mutex<Vec<String>>,
    change_on_focus: std::sync::atomic::AtomicBool,
}
#[async_trait]
impl BrowserHerdrAdapter for FixtureAdapter {
    async fn browser_snapshot(&self, _: &str) -> Result<BrowserHerdrSnapshot, InspectionError> {
        assert!(!self.offline.load(std::sync::atomic::Ordering::SeqCst), "saved-work recovery contacted Herdr");
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
        }), offline: false.into(),
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
fn archive(service: &BrowserService) -> BrowserWorkScope {
    std::fs::write(service.root.join("legacy-archive").join(format!("{KEY}.json")),
        serde_json::to_vec(&json!({"association_key":KEY,"endpoint_identity":"old-endpoint",
            "endpoint_path":"/old/socket","session_id":"original-session","space_id":"old-space",
            "space_label":"Original Space","archived_at":"2026-09-29T00:00:00Z",
            "session_stopped":true,"candidates":[],"not_candidates":[]})).unwrap()).unwrap();
    BrowserWorkScope::LegacyArchive { association_key: KEY.into() }
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
fn saved_tab_target(endpoint_identity: &str) -> crate::browser::ResolvedTarget {
    crate::browser::ResolvedTarget {
        endpoint_identity: endpoint_identity.into(), endpoint_path: "/fixture/socket".into(),
        session_id: "session".into(), space_id: "w1".into(), space_label: "Original Space".into(),
        tab_id: "w1:t1".into(), tab_label: "One".into(), tab_present: true,
    }
}
fn send(scope: BrowserWorkScope, id: String, recipient: Option<CommentPasteTarget>) -> BrowserFeedbackSendRequest {
    BrowserFeedbackSendRequest { scope, recipient, operation_id: Uuid::new_v4().to_string(), ids: vec![id], acknowledge_duplicate_risk: false }
}
#[tokio::test]
async fn tab_delivery_requires_its_focused_tab_not_merely_the_same_space() {
    let (service, adapter, root) = fixture();
    let key = crate::browser::association_key("endpoint", "session", "w1:t2");
    let (_, id, _) = seed_capture(&service, &key);
    let error = service.send_feedback(send(tab_scope("w1:t2"), id.clone(), None)).await.unwrap_err();
    assert_eq!(error.code, "browser_feedback_tab_inactive");
    assert!(adapter.writes.lock().await.is_empty());
    let recipient = adapter.targets.lock().await[1].clone();
    let error = service.send_feedback(send(tab_scope("w1:t2"), id, Some(recipient))).await.unwrap_err();
    assert_eq!(error.code, "browser_feedback_recipient_invalid");
    assert_eq!(service.feedback.list(&key).unwrap().pending_count, 1);
    std::fs::remove_dir_all(root).unwrap();
}
#[tokio::test]
async fn legacy_recipients_are_focused_tab_agents_with_matching_terminals() {
    let (service, adapter, root) = fixture();
    let listed = service.legacy_recipients(BrowserLegacyRecipientsRequest { session_id: "session".into() }).await.unwrap();
    assert_eq!(listed, vec![adapter.targets.lock().await[1].clone()]);
    adapter.targets.lock().await[1].terminal_id = "replaced".into();
    assert!(service.legacy_recipients(BrowserLegacyRecipientsRequest { session_id: "session".into() }).await.unwrap().is_empty());
    std::fs::remove_dir_all(root).unwrap();
}
#[tokio::test]
async fn archive_work_is_recoverable_offline_and_retains_original_source() {
    let (service, adapter, root) = fixture();
    let scope = archive(&service);
    let (capture_id, id, png) = seed_capture(&service, KEY);
    let identity = crate::browser::drafts::BrowserDraftIdentity {
        association_key: KEY.into(), browser_incarnation: Uuid::new_v4().to_string(),
        target_id: "old-target".into(), document_generation: 1,
    };
    let draft = service.draft_store().unwrap().open(&identity, None).unwrap();
    adapter.offline.store(true, std::sync::atomic::Ordering::SeqCst);
    let lookup = service.feedback(&scope).await.unwrap();
    assert_eq!(lookup.feedback.captures[0].context.space_id, "old-space");
    assert_eq!(lookup.drafts.unwrap().drafts[0].draft_id, draft.draft_id);
    let image = service.feedback_image(BrowserFeedbackImageRequest { scope: scope.clone(), capture_id }).await.unwrap();
    assert_eq!(image.data_base64, png);
    let annotation_id = Uuid::new_v4().to_string();
    let annotated = service.browser_draft_recovery(BrowserDraftRecoveryRequest {
        scope: scope.clone(), action: BrowserDraftRecoveryAction::UpsertAnnotation {
            draft_id: draft.draft_id.clone(), expected_revision: draft.revision,
            annotation: serde_json::from_value(json!({
                "id":annotation_id,"kind":"region","color":"#ff0000","points":[],
                "bounds":{"x":0.0,"y":0.0,"width":1.0,"height":1.0},
                "evidence":null,"comment":"Recovered annotation"
            })).unwrap(),
        },
    }).await.unwrap();
    let BrowserViewCommandOutcome::Draft { draft } = annotated else { panic!("expected recovered annotation") };
    let edited = service.browser_draft_recovery(BrowserDraftRecoveryRequest {
        scope: scope.clone(), action: BrowserDraftRecoveryAction::SetEditor {
            draft_id: draft.draft_id.clone(), expected_revision: draft.revision,
            editor: BrowserViewDraftEditorState { selected_annotation_id: Some(annotation_id.clone()), notes_open: true,
                note_annotation_id: Some(annotation_id), note_text: "Recovered note".into() },
        },
    }).await.unwrap();
    let BrowserViewCommandOutcome::Draft { draft: edited } = edited else { panic!("expected edited draft") };
    assert_eq!(edited.editor.note_text, "Recovered note");
    assert_eq!(edited.target_id, identity.target_id);
    assert_eq!(edited.document_generation, identity.document_generation);
    let ack = service.acknowledge_feedback(BrowserFeedbackAckRequest { scope: scope.clone(), ids: vec![id.clone()] }).await.unwrap();
    assert_eq!(ack.acknowledged_ids, vec![id]);
    let discarded = service.browser_draft_recovery(BrowserDraftRecoveryRequest {
        scope, action: BrowserDraftRecoveryAction::DiscardDraft {
            draft_id: edited.draft_id, expected_revision: edited.revision,
        },
    }).await.unwrap();
    let BrowserViewCommandOutcome::DraftInventory { inventory } = discarded else { panic!("expected inventory") };
    assert!(inventory.drafts.is_empty());
    std::fs::remove_dir_all(root).unwrap();
}
#[tokio::test]
async fn legacy_send_requires_explicit_unchanged_recipient_and_preserves_duplicate_risk() {
    let (service, adapter, root) = fixture();
    let scope = archive(&service);
    let (_, id, _) = seed_capture(&service, KEY);
    let error = service.send_feedback(send(scope.clone(), id.clone(), None)).await.unwrap_err();
    assert_eq!(error.code, "browser_feedback_recipient_required");
    let recipient = adapter.targets.lock().await[1].clone();
    let mut changed = recipient.clone();
    changed.agent_fingerprint = "other-agent".into();
    let error = service.send_feedback(send(scope.clone(), id.clone(), Some(changed))).await.unwrap_err();
    assert_eq!(error.code, "browser_feedback_recipient_changed");
    adapter.change_on_focus.store(true, std::sync::atomic::Ordering::SeqCst);
    let error = service.send_feedback(send(scope.clone(), id.clone(), Some(recipient.clone()))).await.unwrap_err();
    assert_eq!(error.code, "browser_feedback_recipient_changed");
    assert!(adapter.writes.lock().await.is_empty());
    assert_eq!(service.feedback.list(KEY).unwrap().pending_count, 1);
    adapter.change_on_focus.store(false, std::sync::atomic::Ordering::SeqCst);
    adapter.snapshot.lock().await.snapshot.focused_tab_id = Some("w1:t1".into());
    service.persist_outcome("unknown-operation", KEY, &[id.clone()], Some(recipient.clone()),
        CommentPasteState::OutcomeUnknown, "unknown").unwrap();
    let mut retry = send(scope, id, Some(recipient));
    let error = service.send_feedback(retry.clone()).await.unwrap_err();
    assert_eq!(error.code, "browser_feedback_duplicate_risk");
    retry.acknowledge_duplicate_risk = true;
    let accepted = service.send_feedback(retry).await.unwrap();
    assert_eq!(accepted.state, CommentPasteState::Accepted);
    let writes = adapter.writes.lock().await;
    assert_eq!(writes.len(), 1);
    assert!(writes[0].starts_with(PASTE_PREFIX));
    assert!(writes[0].ends_with(PASTE_SUFFIX));
    let payload: Value = serde_json::from_str(writes[0].strip_prefix(PASTE_PREFIX).unwrap().strip_suffix(PASTE_SUFFIX).unwrap()).unwrap();
    assert_eq!(payload["captures"][0]["context"]["space_id"], "old-space");
    drop(writes);
    std::fs::remove_dir_all(root).unwrap();
}

#[tokio::test]
async fn pending_legacy_capture_retry_and_discard_need_no_herdr() {
    let (service, adapter, root) = fixture();
    let scope = archive(&service);
    let (capture_id, id, png) = seed_capture(&service, KEY);
    let capture = service.feedback.list(KEY).unwrap().captures.remove(0);
    // Frozen save reached the feedback store before the old process exited,
    // but its draft-consumption receipt did not finish.
    let pending = json!({
        "format_version":1,"association_key":KEY,"browser_incarnation":capture.context.browser_instance,
        "draft_id":Uuid::new_v4().to_string(),"draft_revision":1,"annotation_ids":[id],
        "original_annotation_digests":[],"context":capture.context,
        "submission":{"association_key":KEY,"browser_instance":capture.context.browser_instance,
            "capture_id":capture_id,"page":capture.page,"annotations":capture.annotations,"png_base64":png},
        "last_error":"interrupted"
    });
    // A legacy pending receipt lives in the durable draft store initialized
    // by the browser before capture; seed that store through its real API.
    service.draft_store().unwrap();
    let pending_path = service.root.join("drafts").join(format!("pending-{KEY}.json"));
    std::fs::write(&pending_path, serde_json::to_vec(&pending).unwrap()).unwrap();
    adapter.offline.store(true, std::sync::atomic::Ordering::SeqCst);
    assert!(service.feedback(&scope).await.unwrap().drafts.unwrap().pending_capture.is_some());
    let outcome = service.browser_draft_recovery(BrowserDraftRecoveryRequest {
        scope: scope.clone(), action: BrowserDraftRecoveryAction::RetryPending,
    }).await.unwrap();
    let BrowserViewCommandOutcome::Capture {
        capture: cockpit_protocol::browser_view::BrowserViewCaptureOutcome::Saved { saved },
    } = outcome else { panic!("expected frozen capture recovery") };
    assert_eq!(saved.capture_id, capture_id);
    let recovered = service.feedback(&scope).await.unwrap();
    assert!(recovered.drafts.unwrap().pending_capture.is_none());
    assert_eq!(recovered.feedback.captures.len(), 1);
    assert_eq!(recovered.feedback.pending_count, 1);
    std::fs::write(&pending_path, serde_json::to_vec(&pending).unwrap()).unwrap();
    service.browser_draft_recovery(BrowserDraftRecoveryRequest {
        scope: scope.clone(), action: BrowserDraftRecoveryAction::DiscardPending,
    }).await.unwrap();
    let discarded = service.feedback(&scope).await.unwrap();
    assert!(discarded.drafts.unwrap().pending_capture.is_none());
    assert_eq!(discarded.feedback.captures[0].id, capture_id);
    assert_eq!(discarded.feedback.pending_count, 1);
    std::fs::remove_dir_all(root).unwrap();
}

#[tokio::test]
async fn archive_key_cannot_authorize_unarchived_or_mismatched_saved_work() {
    let (service, adapter, root) = fixture();
    seed_capture(&service, KEY);
    adapter.offline.store(true, std::sync::atomic::Ordering::SeqCst);
    let scope = BrowserWorkScope::LegacyArchive { association_key: KEY.into() };
    assert!(service.feedback(&scope).await.is_err());
    let authorized = archive(&service);
    let path = service.root.join("legacy-archive").join(format!("{KEY}.json"));
    let mut record: Value = serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
    record["association_key"] = json!("89abcdef0123456701234567");
    std::fs::write(&path, serde_json::to_vec(&record).unwrap()).unwrap();
    assert!(service.feedback(&authorized).await.is_err());
    assert_eq!(service.feedback.list(KEY).unwrap().pending_count, 1);
    std::fs::remove_dir_all(root).unwrap();
}

#[tokio::test]
async fn saved_tab_dirty_editor_recovers_after_tab_retirement_cleanup_and_endpoint_reuse() {
    for absent in [false, true] {
        let (service, adapter, root) = fixture();
        let mut receipt = service.load_or_create(&saved_tab_target("endpoint")).unwrap();
        let key = receipt.association_key.clone();
        let scope = BrowserWorkScope::SavedTab { association_key: key.clone() };
        let (capture_id, id, png) = seed_capture_for(&service, &key, "session", "w1", "Original Space");
        let identity = crate::browser::drafts::BrowserDraftIdentity {
            association_key: key.clone(), browser_incarnation: Uuid::new_v4().to_string(),
            target_id: "retired-target".into(), document_generation: 7,
        };
        let store = service.draft_store().unwrap();
        let draft = store.open(&identity, None).unwrap();
        let annotation_id = Uuid::new_v4().to_string();
        let draft = store.upsert_annotation_recovery(&key, &draft.draft_id, draft.revision,
            serde_json::from_value(json!({
                "id":annotation_id,"kind":"region","color":"#ff0000","points":[],
                "bounds":{"x":0.0,"y":0.0,"width":1.0,"height":1.0},
                "evidence":null,"comment":"Original note"
            })).unwrap()).unwrap();
        {
            let mut snapshot = adapter.snapshot.lock().await;
            snapshot.snapshot.panes.retain(|pane| pane.tab_id != "w1:t1");
            snapshot.snapshot.focused_tab_id = Some("w1:t2".into());
            snapshot.snapshot.focused_pane_id = Some("w1:p2".into());
            if absent {
                snapshot.snapshot.tabs.retain(|tab| tab.id != "w1:t1");
            } else {
                let tab = snapshot.snapshot.tabs.iter_mut().find(|tab| tab.id == "w1:t1").unwrap();
                tab.pane_count = 0;
                tab.focused_pane_id = None;
                tab.focused = false;
            }
        }
        let editor = BrowserViewDraftEditorState {
            selected_annotation_id: Some(annotation_id.clone()), notes_open: true,
            note_annotation_id: Some(annotation_id), note_text: "Dirty note saved after retirement".into(),
        };
        let edited = service.browser_draft_recovery(BrowserDraftRecoveryRequest {
            scope: scope.clone(), action: BrowserDraftRecoveryAction::SetEditor {
                draft_id: draft.draft_id.clone(), expected_revision: draft.revision, editor: editor.clone(),
            },
        }).await.unwrap();
        let BrowserViewCommandOutcome::Draft { draft: edited } = edited else { panic!("expected retired draft") };
        assert_eq!(edited.editor.note_text, editor.note_text);
        assert_eq!(edited.target_id, identity.target_id);
        assert_eq!(edited.document_generation, identity.document_generation);

        service.finish_cleanup(&mut receipt).await.unwrap();
        assert!(service.load(&key).unwrap().is_none());
        let mut replacement_target = saved_tab_target("replacement-endpoint");
        replacement_target.space_label = "Replacement Space".into();
        let replacement = service.load_or_create(&replacement_target).unwrap();
        assert_ne!(replacement.association_key, key);
        let (replacement_capture, _, _) = seed_capture_for(
            &service, &replacement.association_key, "session", "w1", "Replacement Space",
        );
        let replacement_identity = crate::browser::drafts::BrowserDraftIdentity {
            association_key: replacement.association_key.clone(), ..identity.clone()
        };
        let replacement_draft = store.open(&replacement_identity, None).unwrap();
        adapter.snapshot.lock().await.endpoint_identity = "replacement-endpoint".into();
        drop(store);
        drop(service);
        let service = service_for(root.clone(), adapter.clone());
        adapter.offline.store(true, std::sync::atomic::Ordering::SeqCst);
        let lookup = service.feedback(&scope).await.unwrap();
        assert_eq!(lookup.feedback.captures.iter().map(|capture| &capture.id).collect::<Vec<_>>(), vec![&capture_id]);
        assert_eq!(lookup.feedback.captures[0].context.space_label, "Original Space");
        let inventory = lookup.drafts.unwrap();
        assert_eq!(inventory.drafts.iter().map(|draft| &draft.draft_id).collect::<Vec<_>>(), vec![&edited.draft_id]);
        assert_eq!(inventory.drafts[0].editor.note_text, editor.note_text);
        let mut restarted_editor = editor.clone();
        restarted_editor.note_text = "Dirty note saved after owner restart".into();
        let recovered = service.browser_draft_recovery(BrowserDraftRecoveryRequest {
            scope: scope.clone(), action: BrowserDraftRecoveryAction::SetEditor {
                draft_id: edited.draft_id.clone(), expected_revision: edited.revision,
                editor: restarted_editor.clone(),
            },
        }).await.unwrap();
        let BrowserViewCommandOutcome::Draft { draft: edited } = recovered else { panic!("expected restarted draft") };
        assert_eq!(edited.editor.note_text, restarted_editor.note_text);
        let image = service.feedback_image(BrowserFeedbackImageRequest {
            scope: scope.clone(), capture_id: capture_id.clone(),
        }).await.unwrap();
        assert_eq!(image.data_base64, png);
        assert!(service.feedback_image(BrowserFeedbackImageRequest {
            scope: scope.clone(), capture_id: replacement_capture.clone(),
        }).await.is_err());
        for action in [
            BrowserDraftRecoveryAction::SetEditor {
                draft_id: replacement_draft.draft_id.clone(), expected_revision: replacement_draft.revision,
                editor: editor.clone(),
            },
            BrowserDraftRecoveryAction::DiscardDraft {
                draft_id: replacement_draft.draft_id.clone(), expected_revision: replacement_draft.revision,
            },
        ] {
            let error = service.browser_draft_recovery(BrowserDraftRecoveryRequest {
                scope: scope.clone(), action,
            }).await.unwrap_err();
            assert_eq!(error.code, "browser_draft_identity");
        }
        service.acknowledge_feedback(BrowserFeedbackAckRequest {
            scope: scope.clone(), ids: vec![id],
        }).await.unwrap();
        let discarded = service.browser_draft_recovery(BrowserDraftRecoveryRequest {
            scope: scope.clone(), action: BrowserDraftRecoveryAction::DiscardDraft {
                draft_id: edited.draft_id, expected_revision: edited.revision,
            },
        }).await.unwrap();
        let BrowserViewCommandOutcome::DraftInventory { inventory } = discarded else { panic!("expected inventory") };
        assert!(inventory.drafts.is_empty());
        let replacement_scope = BrowserWorkScope::SavedTab { association_key: replacement.association_key };
        let replacement_lookup = service.feedback(&replacement_scope).await.unwrap();
        assert_eq!(replacement_lookup.feedback.captures[0].id, replacement_capture);
        assert_eq!(replacement_lookup.feedback.pending_count, 1);
        assert_eq!(replacement_lookup.drafts.unwrap().drafts[0].draft_id, replacement_draft.draft_id);
        std::fs::remove_dir_all(root).unwrap();
    }
}

#[tokio::test]
async fn saved_tab_feedback_requires_an_explicit_current_focused_recipient() {
    let (service, adapter, root) = fixture();
    let mut receipt = service.load_or_create(&saved_tab_target("endpoint")).unwrap();
    let key = receipt.association_key.clone();
    let scope = BrowserWorkScope::SavedTab { association_key: key.clone() };
    let (_, id, _) = seed_capture_for(&service, &key, "session", "w1", "Original Space");
    service.finish_cleanup(&mut receipt).await.unwrap();
    let error = service.send_feedback(send(scope.clone(), id.clone(), None)).await.unwrap_err();
    assert_eq!(error.code, "browser_feedback_recipient_required");
    let recipient = adapter.targets.lock().await[0].clone();
    let error = service.send_feedback(send(scope.clone(), id.clone(), Some(recipient))).await.unwrap_err();
    assert_eq!(error.code, "browser_feedback_recipient_changed");
    let recipient = adapter.targets.lock().await[1].clone();
    let mut changed = recipient.clone();
    changed.agent_fingerprint = "replacement-agent".into();
    let error = service.send_feedback(send(scope.clone(), id.clone(), Some(changed))).await.unwrap_err();
    assert_eq!(error.code, "browser_feedback_recipient_changed");
    adapter.change_on_focus.store(true, std::sync::atomic::Ordering::SeqCst);
    let error = service.send_feedback(send(scope.clone(), id.clone(), Some(recipient.clone()))).await.unwrap_err();
    assert_eq!(error.code, "browser_feedback_recipient_changed");
    assert!(adapter.writes.lock().await.is_empty());
    assert_eq!(service.feedback.list(&key).unwrap().pending_count, 1);
    adapter.change_on_focus.store(false, std::sync::atomic::Ordering::SeqCst);
    // Saved work belongs to One, but only the explicitly chosen agent of the
    // currently focused Two is authorized to receive it.
    let recipient = adapter.targets.lock().await[0].clone();
    let accepted = service.send_feedback(send(scope.clone(), id, Some(recipient))).await.unwrap();
    assert_eq!(accepted.state, CommentPasteState::Accepted);
    assert_eq!(accepted.pending_count, 0);
    let payloads = adapter.writes.lock().await;
    assert_eq!(payloads.len(), 1);
    let payload: Value = serde_json::from_str(payloads[0].strip_prefix(PASTE_PREFIX).unwrap().strip_suffix(PASTE_SUFFIX).unwrap()).unwrap();
    assert_eq!(payload["captures"][0]["context"]["space_label"], "Original Space");
    drop(payloads);
    adapter.offline.store(true, std::sync::atomic::Ordering::SeqCst);
    assert_eq!(service.feedback(&scope).await.unwrap().feedback.pending_count, 0);
    std::fs::remove_dir_all(root).unwrap();
}
