use super::*;
use crate::browser::drafts::BrowserDraftIdentity;
use std::os::unix::fs::symlink;

struct Offline;
#[async_trait]
impl BrowserHerdrAdapter for Offline {
    async fn browser_snapshot(&self, _: &str) -> Result<BrowserHerdrSnapshot, InspectionError> {
        panic!("saved tab provenance must not contact Herdr")
    }
}

struct Fixture { root: PathBuf, service: BrowserService }
impl Fixture {
    fn new() -> Self {
        let root = env::temp_dir().join(format!("cb-saved-tab-{}", Uuid::new_v4()));
        let service = Self::service(&root);
        Self { root, service }
    }
    fn service(root: &Path) -> BrowserService {
        BrowserService::new(BrowserConfiguration {
            playwright_cli: "unused-playwright-cli".into(), default_url: "about:blank".into(),
            chromium_executable: None, node_executable: None, browser_helper: None,
            playwright_core: None, feedback_retention_seconds: 3600,
            feedback_max_store_bytes: 1024 * 1024,
        }, root.to_path_buf(), Arc::new(Offline)).unwrap()
    }
    fn receipt(&self) -> BrowserReceipt {
        self.service.load_or_create(&ResolvedTarget {
            endpoint_identity: "original-endpoint".into(), endpoint_path: "/original/herdr.sock".into(),
            session_id: "original-session".into(), space_id: "w1".into(),
            space_label: "Original Space".into(), tab_id: "w1:t1".into(),
            tab_label: "Original Tab".into(), tab_present: true,
        }).unwrap()
    }
    fn provenance(&self, key: &str) -> PathBuf {
        self.service.root.join("saved-tab-associations").join(format!("{key}.json"))
    }
}
impl Drop for Fixture { fn drop(&mut self) { let _ = fs::remove_dir_all(&self.root); } }

#[tokio::test]
async fn cleaned_receipt_saved_work_survives_restart_with_original_identity() {
    let fixture = Fixture::new();
    let mut receipt = fixture.receipt();
    let key = receipt.association_key.clone();
    assert!(fixture.service.saved_tab_work().unwrap().is_empty());
    let draft = fixture.service.draft_store().unwrap().open(&BrowserDraftIdentity {
        association_key: key.clone(), browser_incarnation: Uuid::new_v4().to_string(),
        target_id: "original-page".into(), document_generation: 1,
    }, None).unwrap();
    receipt.space_id = "w2".into();
    receipt.space_label = "Moved Space".into();
    receipt.tab_label = "Renamed Tab".into();
    fixture.service.store(&receipt).unwrap();
    assert_eq!(fixture.service.finish_cleanup(&mut receipt).await.unwrap().cleanup, BrowserCleanupState::Done);
    assert!(!fixture.service.tab_association_path(&key).exists());
    let restarted = Fixture::service(&fixture.root);
    let saved = restarted.cleanup_status().await.unwrap().saved_tabs;
    assert_eq!(saved.len(), 1);
    assert_eq!(saved[0].association_key, key);
    assert_eq!(saved[0].session_id, "original-session");
    assert_eq!(saved[0].tab_id, "w1:t1");
    assert_eq!(saved[0].tab_label, "Original Tab");
    assert_eq!(saved[0].space_id, "w1");
    assert_eq!(saved[0].space_label, "Original Space");
    assert_eq!(saved[0].draft_count, 1);
    assert_eq!(saved[0].saved_capture_count, 0);
    assert!(!saved[0].pending_capture);
    restarted.draft_store().unwrap().discard_draft(&key, &draft.draft_id, draft.revision).unwrap();
    assert!(restarted.cleanup_status().await.unwrap().saved_tabs.is_empty());
    assert!(restarted.load_saved_tab(&key).unwrap().is_some());
}

#[tokio::test]
async fn closing_an_existing_receipt_upgrades_missing_provenance_before_removal() {
    let fixture = Fixture::new();
    let mut receipt = fixture.receipt();
    fs::remove_file(fixture.provenance(&receipt.association_key)).unwrap();
    assert_eq!(fixture.service.finish_cleanup(&mut receipt).await.unwrap().cleanup, BrowserCleanupState::Done);
    assert!(!fixture.service.tab_association_path(&receipt.association_key).exists());
    let restarted = Fixture::service(&fixture.root);
    let saved = restarted.load_saved_tab(&receipt.association_key).unwrap().unwrap();
    assert_eq!(saved.endpoint_identity, "original-endpoint");
    assert_eq!(saved.tab_id, "w1:t1");
}

#[tokio::test]
async fn mismatched_or_replaced_provenance_cannot_authorize_or_delete_receipts() {
    let fixture = Fixture::new();
    let mut receipt = fixture.receipt();
    let path = fixture.provenance(&receipt.association_key);
    let original = fs::read(&path).unwrap();
    let mut saved: serde_json::Value = serde_json::from_slice(&original).unwrap();
    saved["tab_id"] = "w1:t2".into();
    atomic_write_json(&path, &saved).unwrap();
    assert_eq!(fixture.service.load_saved_tab(&receipt.association_key).unwrap_err().code, "browser_state_corrupt");
    assert!(fixture.service.finish_cleanup(&mut receipt).await.is_err());
    assert!(fixture.service.tab_association_path(&receipt.association_key).exists());
    assert!(Path::new(&receipt.profile_path).exists());
    saved["tab_id"] = "w1:t1".into();
    saved["endpoint_path"] = "/replacement/herdr.sock".into();
    atomic_write_json(&path, &saved).unwrap();
    assert!(fixture.service.finish_cleanup(&mut receipt).await.is_err());
    assert!(fixture.service.tab_association_path(&receipt.association_key).exists());
    fs::remove_file(&path).unwrap();
    let outside = fixture.root.join("outside.json");
    fs::write(&outside, &original).unwrap();
    symlink(&outside, &path).unwrap();
    assert!(fixture.service.load_saved_tab(&receipt.association_key).is_err());
    assert!(fixture.service.finish_cleanup(&mut receipt).await.is_err());
    assert_eq!(fs::read(&outside).unwrap(), original);
    assert!(fixture.service.load_saved_tab("../outside").is_err());
}
