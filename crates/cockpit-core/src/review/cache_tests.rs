use super::super::test_support::*;
use super::*;
use cockpit_protocol::review::{ReviewComparison, ReviewFileRequest};

#[tokio::test]
async fn file_cache_rejects_a_payload_from_another_viewer_binding_or_snapshot() {
    let fixture = service_fixture("file-cache-identity").await;
    std::fs::write(fixture.checkout.join("tracked.txt"), "base\nchanged\n")
        .expect("modify tracked file");
    let snapshot = fixture.snapshot(ReviewComparison::Unstaged).await;
    let file_id = snapshot
        .files
        .iter()
        .find(|file| file.new_path.as_deref() == Some("tracked.txt"))
        .expect("tracked change")
        .file_id
        .clone();
    let diff = fixture.file(&snapshot, &file_id).await;
    let name = file_cache_name(&snapshot.review_id, &file_id).expect("cache key");
    for field in [
        "viewer_id",
        "binding_id",
        "session_id",
        "review_id",
        "generation",
    ] {
        let mut payload = serde_json::to_value(&diff).expect("file cache payload");
        payload[field] = if field == "generation" {
            serde_json::json!(snapshot.generation + 1)
        } else {
            serde_json::json!(Uuid::new_v4().to_string())
        };
        atomic_write_json(fixture.service.store.state_dir(), &name, &payload)
            .expect("seed incorrectly bound payload");
        let error = fixture
            .service
            .file(
                FIXTURE_SESSION,
                &fixture.viewer_id,
                &ReviewFileRequest {
                    binding_id: fixture.binding_id.clone(),
                    review_id: snapshot.review_id.clone(),
                    generation: snapshot.generation,
                    file_id: file_id.clone(),
                    source_side: None,
                    source_offset: 0,
                    source_revision: None,
                },
            )
            .await
            .expect_err("foreign cached identity must not reach the viewer");
        assert_eq!(error.code, "review_read", "incorrect {field}");
    }
    atomic_write_json(fixture.service.store.state_dir(), &name, &diff).expect("restore cache");
    let restored = fixture.file(&snapshot, &file_id).await;
    assert_eq!(restored.new_source.as_deref(), Some("base\nchanged\n"));
    std::fs::remove_dir_all(fixture.workspace).expect("cleanup");
}
