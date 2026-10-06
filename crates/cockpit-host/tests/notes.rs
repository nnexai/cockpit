use std::{path::PathBuf, sync::Arc};

use axum::{
    body::{Body, to_bytes},
    http::{Request, StatusCode},
};
use cockpit_core::{CockpitService, notes::NotesService};
use cockpit_herdr::{HerdrCliAdapter, HerdrCliConfig};
use cockpit_host::server::build_router;
use cockpit_protocol::v1::CockpitMode;
use serde_json::{Value, json};
use tower::ServiceExt;

struct Fixture {
    root: PathBuf,
    notes_id: String,
}
impl Fixture {
    fn new() -> Self {
        let root =
            std::env::temp_dir().join(format!("cockpit-notes-transport-{}", uuid::Uuid::new_v4()));
        let notes_id = uuid::Uuid::new_v4().to_string();
        std::fs::create_dir_all(root.join("notes").join(&notes_id)).unwrap();
        std::fs::create_dir(root.join("dist")).unwrap();
        std::fs::write(
            root.join("dist/index.html"),
            "<!doctype html><main>test</main>",
        )
        .unwrap();
        Self { root, notes_id }
    }
    fn folder(&self) -> PathBuf {
        self.root.join("notes").join(&self.notes_id)
    }
    fn router(&self) -> axum::Router {
        // Pinned Notes requests must not try to launch this deliberately nonexistent executable.
        let herdr = Arc::new(HerdrCliAdapter::new(HerdrCliConfig {
            executable: self.root.join("no-herdr"),
            session: None,
            socket: None,
        }));
        let service = CockpitService::new(CockpitMode::Test, herdr)
            .with_notes(NotesService::new(self.root.join("notes")));
        build_router(
            service,
            self.root.join("dist"),
            "127.0.0.1:43123".parse().unwrap(),
        )
        .unwrap()
    }
    fn cli(&self, arguments: &[&str]) -> std::process::Output {
        std::process::Command::new(env!("CARGO_BIN_EXE_cockpit"))
            .args(["notes", "--notes", &self.notes_id])
            .args(arguments)
            .env("COCKPIT_NOTES_ROOT", self.root.join("notes"))
            .env("XDG_CONFIG_HOME", self.root.join("config"))
            .env("XDG_DATA_HOME", self.root.join("data"))
            .env("XDG_STATE_HOME", self.root.join("state"))
            .env("XDG_CACHE_HOME", self.root.join("cache"))
            .env("COCKPIT_HERDR_EXECUTABLE", self.root.join("no-herdr"))
            .env_remove("COCKPIT_CONFIG")
            .output()
            .unwrap()
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.root);
    }
}

async fn post(fixture: &Fixture, operation: Value, origin: bool) -> (StatusCode, Value) {
    let body =
        json!({"target": {"kind": "notes", "notes_id": fixture.notes_id}, "operation": operation});
    let mut request = Request::post("/api/v1/notes")
        .header("host", "127.0.0.1:43123")
        .header("content-type", "application/json");
    if origin {
        request = request.header("origin", "http://127.0.0.1:43123");
    }
    let response = fixture
        .router()
        .oneshot(request.body(Body::from(body.to_string())).unwrap())
        .await
        .unwrap();
    let status = response.status();
    let bytes = to_bytes(response.into_body(), 4 * 1024 * 1024)
        .await
        .unwrap();
    (status, serde_json::from_slice(&bytes).unwrap())
}

#[tokio::test]
async fn notes_http_rejects_stale_edits_without_overwriting_external_content() {
    let fixture = Fixture::new();
    let (status, first) = post(
        &fixture,
        json!({"op": "scratchpad_replace", "content": "original", "expected_revision": "absent"}),
        true,
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let revision = first["result"]["document"]["revision"].as_str().unwrap();
    std::fs::write(fixture.folder().join("scratchpad.md"), "external editor").unwrap();
    let (status, error) = post(&fixture, json!({"op": "scratchpad_replace", "content": "stale draft", "expected_revision": revision}), true).await;
    assert_eq!(status, StatusCode::CONFLICT);
    assert_eq!(error["code"], "notes_conflict");
    assert_eq!(
        std::fs::read_to_string(fixture.folder().join("scratchpad.md")).unwrap(),
        "external editor"
    );
    let (status, read) = post(&fixture, json!({"op": "scratchpad_read"}), true).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(read["result"]["document"]["content"], "external editor");
}

#[tokio::test]
async fn notes_http_refuses_originless_mutations_and_symlink_escape() {
    let fixture = Fixture::new();
    let (status, _) = post(
        &fixture,
        json!({"op": "scratchpad_append", "text": "must not write", "expected_revision": null}),
        false,
    )
    .await;
    assert!(status.is_client_error());
    assert!(!fixture.folder().join("scratchpad.md").exists());
    let outside = fixture.root.join("outside.md");
    std::fs::write(&outside, "private outside content").unwrap();
    std::os::unix::fs::symlink(&outside, fixture.folder().join("scratchpad.md")).unwrap();
    let (status, error) = post(&fixture, json!({"op": "scratchpad_read"}), true).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_eq!(error["code"], "notes_unsafe_path");
    let (status, error) = post(
        &fixture,
        json!({"op": "scratchpad_replace", "content": "overwrite", "expected_revision": "absent"}),
        true,
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_eq!(error["code"], "notes_unsafe_path");
    assert_eq!(
        std::fs::read_to_string(outside).unwrap(),
        "private outside content"
    );
}

#[test]
fn notes_cli_pinned_writes_need_no_herdr_and_conflicts_have_stable_exit_json() {
    let fixture = Fixture::new();
    let first = fixture.cli(&["scratchpad", "append", "--text", "original"]);
    assert!(
        first.status.success(),
        "{}",
        String::from_utf8_lossy(&first.stderr)
    );
    let response: Value = serde_json::from_slice(&first.stdout).unwrap();
    let revision = response["result"]["document"]["revision"].as_str().unwrap();
    std::fs::write(fixture.folder().join("scratchpad.md"), "external editor").unwrap();
    let stale = fixture.cli(&[
        "scratchpad",
        "append",
        "--text",
        "stale draft",
        "--expected-revision",
        revision,
    ]);
    assert_eq!(stale.status.code(), Some(9));
    let error: Value = serde_json::from_slice(&stale.stdout).unwrap();
    assert_eq!(error["error"]["code"], "notes_conflict");
    assert_eq!(
        std::fs::read_to_string(fixture.folder().join("scratchpad.md")).unwrap(),
        "external editor"
    );
}

#[test]
fn notes_cli_rejects_invalid_utf8_and_conflicting_payload_or_selector_flags() {
    let fixture = Fixture::new();
    let invalid_file = fixture.root.join("invalid.md");
    std::fs::write(&invalid_file, [0xff]).unwrap();
    let invalid = fixture.cli(&[
        "scratchpad",
        "replace",
        "--file",
        invalid_file.to_str().unwrap(),
        "--expected-revision",
        "absent",
    ]);
    assert_eq!(invalid.status.code(), Some(11));
    let error: Value = serde_json::from_slice(&invalid.stdout).unwrap();
    assert_eq!(error["error"]["code"], "notes_invalid_encoding");
    let conflicting = fixture.cli(&["comment", "add", "--todo", "abc", "--text", "x", "--stdin"]);
    assert_eq!(conflicting.status.code(), Some(2));
    let error: Value = serde_json::from_slice(&conflicting.stdout).unwrap();
    assert_eq!(error["error"]["code"], "notes_usage");
    let missing_revision = fixture.cli(&["todo", "complete", "--id", "abc"]);
    assert_eq!(missing_revision.status.code(), Some(2));
    let error: Value = serde_json::from_slice(&missing_revision.stdout).unwrap();
    assert_eq!(error["error"]["code"], "notes_usage");
    assert!(!fixture.folder().join("scratchpad.md").exists());
}

#[tokio::test]
async fn notes_http_refuses_unknown_fields_and_oversize_before_storage_mutation() {
    let fixture = Fixture::new();
    let (status, error) = post(
        &fixture,
        json!({
            "op": "scratchpad_append", "text": "must not write", "expected_revision": null,
            "unrecognized": "must be rejected"
        }),
        true,
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_eq!(error["code"], "notes_usage");
    let body = json!({
        "target": {"kind": "notes", "notes_id": fixture.notes_id},
        "operation": {"op": "scratchpad_append", "text": "x".repeat(4 * 1024 * 1024), "expected_revision": null}
    });
    let response = fixture
        .router()
        .oneshot(
            Request::post("/api/v1/notes")
                .header("host", "127.0.0.1:43123")
                .header("origin", "http://127.0.0.1:43123")
                .header("content-type", "application/json")
                .body(Body::from(body.to_string()))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    let bytes = to_bytes(response.into_body(), 1024).await.unwrap();
    let error: Value = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(error["code"], "notes_too_large");
    assert!(!fixture.folder().join("scratchpad.md").exists());
}
