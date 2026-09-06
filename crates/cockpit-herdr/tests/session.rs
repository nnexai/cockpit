use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::{
    Arc, Mutex, OnceLock,
    atomic::{AtomicU64, AtomicUsize, Ordering},
};
use std::time::{Duration, Instant};

use cockpit_core::{HerdrAdapter, SessionChange};
use cockpit_herdr::{HerdrCliAdapter, HerdrCliConfig};
use cockpit_protocol::v1::{
    FocusKind, FocusRequest, PaneSummary, ResourceMutationRequest, SessionSnapshotResponse,
};
use serde_json::json;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::UnixListener;

#[cfg(unix)]
static NEXT_TEMP_ID: AtomicU64 = AtomicU64::new(0);
#[cfg(unix)]
static TRAMPOLINE_USERS: AtomicUsize = AtomicUsize::new(0);
#[cfg(unix)]
static TRAMPOLINE_LOCK: Mutex<()> = Mutex::new(());
#[cfg(unix)]
static TRAMPOLINE: OnceLock<PathBuf> = OnceLock::new();

#[cfg(unix)]
fn temp_id() -> String {
    format!(
        "{}-{}",
        std::process::id(),
        NEXT_TEMP_ID.fetch_add(1, Ordering::Relaxed)
    )
}

#[cfg(unix)]
fn shared_trampoline() -> PathBuf {
    use std::os::unix::fs::PermissionsExt;
    let _guard = TRAMPOLINE_LOCK
        .lock()
        .unwrap_or_else(|error| error.into_inner());
    let path = TRAMPOLINE
        .get_or_init(|| {
            std::env::temp_dir().join(format!(
                "cockpit-herdr-trampoline-session-{}.sh",
                std::process::id()
            ))
        })
        .clone();
    if !path.exists() {
        fs::write(
            &path,
            "#!/bin/sh\nfixture=\"${0%.exec}.data\"\n. \"$fixture\"\n",
        )
        .unwrap();
    }
    let mut permissions = fs::metadata(&path).unwrap().permissions();
    permissions.set_mode(0o555);
    fs::set_permissions(&path, permissions).unwrap();
    TRAMPOLINE_USERS.fetch_add(1, Ordering::Relaxed);
    path
}

#[cfg(unix)]
struct Fixture {
    executable: PathBuf,
    data: PathBuf,
    trampoline: PathBuf,
}

#[cfg(unix)]
impl Fixture {
    fn path(&self) -> &Path {
        &self.executable
    }
}

#[cfg(unix)]
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.executable);
        let _ = fs::remove_file(&self.data);
        let _guard = TRAMPOLINE_LOCK
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        if TRAMPOLINE_USERS.fetch_sub(1, Ordering::Relaxed) == 1 {
            let _ = fs::remove_file(&self.trampoline);
        }
    }
}

#[cfg(unix)]
fn script(body: &str) -> Fixture {
    let id = temp_id();
    let executable = std::env::temp_dir().join(format!("cockpit-herdr-test-{id}.exec"));
    let data = std::env::temp_dir().join(format!("cockpit-herdr-test-{id}.data"));
    let trampoline = shared_trampoline();
    fs::write(&data, body).unwrap();
    std::os::unix::fs::symlink(&trampoline, &executable).unwrap();
    Fixture {
        executable,
        data,
        trampoline,
    }
}
#[cfg(unix)]
async fn serve_ping(listener: &UnixListener) {
    let (stream, _) = listener.accept().await.unwrap();
    let mut reader = BufReader::new(stream);
    let mut line = String::new();
    reader.read_line(&mut line).await.unwrap();
    let request: serde_json::Value = serde_json::from_str(&line).unwrap();
    assert_eq!(request["method"], "ping");
    let response = json!({
        "id": request["id"],
        "result": {"type": "pong", "version": "0.8.2", "protocol": 20}
    });
    reader
        .into_inner()
        .write_all(format!("{response}\n").as_bytes())
        .await
        .unwrap();
}

#[tokio::test]
async fn maps_redacted_snapshot_and_sanitizes_titles() {
    let socket = std::env::temp_dir().join(format!("cockpit-herdr-snapshot-{}.sock", temp_id()));
    let listener = UnixListener::bind(&socket).unwrap();
    let fixture: serde_json::Value =
        serde_json::from_str(include_str!("fixtures/session-snapshot.json")).unwrap();
    let expected_result = fixture.get("result").cloned().unwrap();
    let server = tokio::spawn(async move {
        let (stream, _) = listener.accept().await.unwrap();
        let mut reader = BufReader::new(stream);
        let mut request_line = String::new();
        reader.read_line(&mut request_line).await.unwrap();
        let request: serde_json::Value = serde_json::from_str(&request_line).unwrap();
        assert_eq!(
            request.get("method").and_then(serde_json::Value::as_str),
            Some("session.snapshot")
        );
        let id = request.get("id").cloned().unwrap();
        let mut stream = reader.into_inner();
        let wrong = json!({"id": "other-request", "result": {}});
        stream
            .write_all(format!("{wrong}\n").as_bytes())
            .await
            .unwrap();
        let response = json!({"id": id, "result": expected_result});
        stream
            .write_all(format!("{response}\n").as_bytes())
            .await
            .unwrap();

        let (stream, _) = listener.accept().await.unwrap();
        let mut reader = BufReader::new(stream);
        request_line.clear();
        reader.read_line(&mut request_line).await.unwrap();
        let request: serde_json::Value = serde_json::from_str(&request_line).unwrap();
        assert_eq!(request["method"], "worktree.list");
        assert_eq!(request["params"], json!({"workspace_id": "space-a"}));
        let response = json!({
            "id": request["id"],
            "error": {"code": "not_git_worktree", "message": "not a repository"}
        });
        reader
            .into_inner()
            .write_all(format!("{response}\n").as_bytes())
            .await
            .unwrap();
    });
    let config =
        HerdrCliConfig::from_options(None, Some("default".into()), Some(socket.clone())).unwrap();
    let snapshot = HerdrCliAdapter::new(config)
        .session_snapshot("default")
        .await
        .expect("snapshot should map");
    server.await.unwrap();
    assert_eq!(snapshot.session_id, "default");
    assert_eq!(snapshot.version, "0.8.2");
    assert_eq!(snapshot.spaces[0].id, "space-a");
    assert_eq!(snapshot.spaces[0].agent_status, "unknown");
    assert_eq!(snapshot.spaces[0].git, None);
    assert_eq!(
        snapshot.panes[0].title.as_deref(),
        Some("Fallback terminal")
    );
    assert_eq!(snapshot.agents[0].name, "assistant");
    assert_eq!(snapshot.agents[0].title.as_deref(), Some("Agent fallback"));
    assert_eq!(snapshot.layouts[0].panes[0].rect.width, 120);
    drop(fs::remove_file(socket));
}
#[cfg(unix)]
#[tokio::test]
async fn custom_socket_uses_named_identity_and_rejects_other_route_sessions() {
    let socket = std::env::temp_dir().join(format!("cockpit-herdr-named-{}.sock", temp_id()));
    let listener = UnixListener::bind(&socket).unwrap();
    let fixture: serde_json::Value =
        serde_json::from_str(include_str!("fixtures/session-snapshot.json")).unwrap();
    let expected_result = fixture.get("result").cloned().unwrap();
    let server = tokio::spawn(async move {
        let (stream, _) = listener.accept().await.unwrap();
        let mut reader = BufReader::new(stream);
        let mut request_line = String::new();
        reader.read_line(&mut request_line).await.unwrap();
        let request: serde_json::Value = serde_json::from_str(&request_line).unwrap();
        assert_eq!(request["method"], "session.snapshot");
        let response = json!({"id": request["id"], "result": expected_result});
        reader
            .into_inner()
            .write_all(format!("{response}\n").as_bytes())
            .await
            .unwrap();

        let (stream, _) = listener.accept().await.unwrap();
        let mut reader = BufReader::new(stream);
        request_line.clear();
        reader.read_line(&mut request_line).await.unwrap();
        let request: serde_json::Value = serde_json::from_str(&request_line).unwrap();
        assert_eq!(request["method"], "worktree.list");
        assert_eq!(request["params"], json!({"workspace_id": "space-a"}));
        let response = json!({
            "id": request["id"],
            "result": {
                "type": "worktree_list",
                "source": {
                    "repo_key": "repo-opaque",
                    "repo_name": "cockpit",
                    "repo_root": "/work/cockpit",
                    "source_checkout_path": "/work/cockpit"
                },
                "worktrees": [{
                    "path": "/work/cockpit",
                    "branch": "main",
                    "is_bare": false,
                    "is_detached": false,
                    "is_prunable": false,
                    "is_linked_worktree": false,
                    "label": "cockpit",
                    "open_workspace_id": "space-a"
                }]
            }
        });
        reader
            .into_inner()
            .write_all(format!("{response}\n").as_bytes())
            .await
            .unwrap();
    });
    let config =
        HerdrCliConfig::from_options(None, Some("named".into()), Some(socket.clone())).unwrap();
    let adapter = HerdrCliAdapter::new(config);
    let snapshot = adapter.session_snapshot("named").await.unwrap();
    assert_eq!(snapshot.session_id, "named");
    let git = snapshot.spaces[0].git.as_ref().unwrap();
    assert_eq!(git.repository_key, "repo-opaque");
    assert_eq!(git.repository, "cockpit");
    assert_eq!(git.branch.as_deref(), Some("main"));
    assert_eq!(git.checkout_path, "/work/cockpit");
    assert!(!git.is_linked_worktree);
    let error = adapter.session_snapshot("other").await.unwrap_err();
    assert_eq!(error.code, "session_not_selected");
    server.await.unwrap();
    drop(fs::remove_file(socket));
}

#[cfg(unix)]
#[tokio::test]
async fn rejects_unterminated_socket_response() {
    let socket = std::env::temp_dir().join(format!("cockpit-herdr-bounded-{}.sock", temp_id()));
    let listener = UnixListener::bind(&socket).unwrap();
    let server = tokio::spawn(async move {
        let (stream, _) = listener.accept().await.unwrap();
        let mut reader = BufReader::new(stream);
        let mut request = String::new();
        reader.read_line(&mut request).await.unwrap();
        let request: serde_json::Value = serde_json::from_str(&request).unwrap();
        let id = request.get("id").cloned().unwrap();
        let mut stream = reader.into_inner();
        let response = json!({"id": id, "result": {}});
        stream
            .write_all(response.to_string().as_bytes())
            .await
            .unwrap();
    });
    let config =
        HerdrCliConfig::from_options(None, Some("default".into()), Some(socket.clone())).unwrap();
    let error = HerdrCliAdapter::new(config)
        .session_snapshot("default")
        .await
        .unwrap_err();
    server.await.unwrap();
    assert_eq!(error.code, "bounded_output");
    drop(fs::remove_file(socket));
}
#[cfg(unix)]
#[tokio::test]
async fn accepts_pane_info_focus_response() {
    let socket = std::env::temp_dir().join(format!("cockpit-herdr-focus-{}.sock", temp_id()));
    let listener = UnixListener::bind(&socket).unwrap();
    let server = tokio::spawn(async move {
        let (stream, _) = listener.accept().await.unwrap();
        let mut reader = BufReader::new(stream);
        let mut request_line = String::new();
        reader.read_line(&mut request_line).await.unwrap();
        let request: serde_json::Value = serde_json::from_str(&request_line).unwrap();
        assert_eq!(
            request.get("method").and_then(serde_json::Value::as_str),
            Some("pane.focus")
        );
        let id = request.get("id").cloned().unwrap();
        let response = json!({
            "id": id,
            "result": {
                "type": "pane_info",
                "pane": {"pane_id": "w1:p1", "focused": true}
            }
        });
        let mut stream = reader.into_inner();
        stream
            .write_all(format!("{response}\n").as_bytes())
            .await
            .unwrap();
    });
    let config =
        HerdrCliConfig::from_options(None, Some("default".into()), Some(socket.clone())).unwrap();
    let request = cockpit_protocol::v1::FocusRequest {
        kind: cockpit_protocol::v1::FocusKind::Pane,
        target_id: "w1:p1".into(),
    };
    let response = HerdrCliAdapter::new(config)
        .focus("default", &request)
        .await
        .unwrap();
    server.await.unwrap();
    assert!(response.accepted);
    drop(fs::remove_file(socket));
}

#[cfg(unix)]
#[tokio::test]
async fn accepts_agent_info_focus_response() {
    let socket = std::env::temp_dir().join(format!("cockpit-herdr-agent-focus-{}.sock", temp_id()));
    let listener = UnixListener::bind(&socket).unwrap();
    let server = tokio::spawn(async move {
        let (stream, _) = listener.accept().await.unwrap();
        let mut reader = BufReader::new(stream);
        let mut request_line = String::new();
        reader.read_line(&mut request_line).await.unwrap();
        let request: serde_json::Value = serde_json::from_str(&request_line).unwrap();
        assert_eq!(
            request.get("method").and_then(serde_json::Value::as_str),
            Some("agent.focus")
        );
        assert_eq!(request.get("params"), Some(&json!({"target": "w1:p1"})));
        let id = request.get("id").cloned().unwrap();
        let response = json!({
            "id": id,
            "result": {
                "type": "agent_info",
                "agent": {"pane_id": "w1:p1", "focused": true}
            }
        });
        let mut stream = reader.into_inner();
        stream
            .write_all(format!("{response}\n").as_bytes())
            .await
            .unwrap();
    });
    let config =
        HerdrCliConfig::from_options(None, Some("default".into()), Some(socket.clone())).unwrap();
    let request = cockpit_protocol::v1::FocusRequest {
        kind: cockpit_protocol::v1::FocusKind::Agent,
        target_id: "w1:p1".into(),
    };
    let response = HerdrCliAdapter::new(config)
        .focus("default", &request)
        .await
        .expect("agent_info should be accepted for agent focus");
    server.await.unwrap();
    assert_eq!(response.session_id, "default");
    assert_eq!(response.kind, cockpit_protocol::v1::FocusKind::Agent);
    assert_eq!(response.target_id, "w1:p1");
    assert!(response.accepted);
    drop(fs::remove_file(socket));
}
#[cfg(unix)]
#[tokio::test]
async fn mutation_discards_raw_result_and_returns_a_fresh_snapshot() {
    let socket = std::env::temp_dir().join(format!("cockpit-herdr-mutate-{}.sock", temp_id()));
    let listener = UnixListener::bind(&socket).unwrap();
    let fixture: serde_json::Value =
        serde_json::from_str(include_str!("fixtures/session-snapshot.json")).unwrap();
    let expected_result = fixture.get("result").cloned().unwrap();
    let server = tokio::spawn(async move {
        let (stream, _) = listener.accept().await.unwrap();
        let mut reader = BufReader::new(stream);
        let mut line = String::new();
        reader.read_line(&mut line).await.unwrap();
        let request: serde_json::Value = serde_json::from_str(&line).unwrap();
        assert_eq!(request["method"], "workspace.rename");
        assert_eq!(
            request["params"],
            json!({"workspace_id": "space-a", "label": "Renamed"})
        );
        let id = request["id"].clone();
        let mut stream = reader.into_inner();
        stream
            .write_all(
                format!(
                    "{}\n",
                    json!({"id": id, "result": {"type": "workspace_renamed", "secret": "redact"}})
                )
                .as_bytes(),
            )
            .await
            .unwrap();

        let (stream, _) = listener.accept().await.unwrap();
        let mut reader = BufReader::new(stream);
        line.clear();
        reader.read_line(&mut line).await.unwrap();
        let request: serde_json::Value = serde_json::from_str(&line).unwrap();
        assert_eq!(request["method"], "session.snapshot");
        let id = request["id"].clone();
        let mut stream = reader.into_inner();
        stream
            .write_all(format!("{}\n", json!({"id": id, "result": expected_result})).as_bytes())
            .await
            .unwrap();
    });
    let config =
        HerdrCliConfig::from_options(None, Some("named".into()), Some(socket.clone())).unwrap();
    let response = HerdrCliAdapter::new(config)
        .mutate(
            "named",
            &ResourceMutationRequest::SpaceRename {
                space_id: "space-a".into(),
                label: "Renamed".into(),
            },
        )
        .await
        .unwrap();
    server.await.unwrap();
    assert_eq!(response.session_id, "named");
    assert_eq!(response.snapshot.session_id, "named");
    assert_eq!(response.snapshot.spaces[0].id, "space-a");
    let serialized = serde_json::to_value(response).unwrap();
    assert!(serialized.get("secret").is_none());
    drop(fs::remove_file(socket));
}
#[cfg(unix)]
#[tokio::test]
async fn mutation_refresh_failure_reports_applied_non_retryable_error() {
    let socket = std::env::temp_dir().join(format!("cockpit-herdr-mutate-fail-{}.sock", temp_id()));
    let listener = UnixListener::bind(&socket).unwrap();
    let server = tokio::spawn(async move {
        let (stream, _) = listener.accept().await.unwrap();
        let mut reader = BufReader::new(stream);
        let mut line = String::new();
        reader.read_line(&mut line).await.unwrap();
        let request: serde_json::Value = serde_json::from_str(&line).unwrap();
        assert_eq!(request["method"], "workspace.rename");
        let response = json!({"id": request["id"], "result": {"type": "workspace_renamed"}});
        reader
            .into_inner()
            .write_all(format!("{response}\n").as_bytes())
            .await
            .unwrap();

        let (stream, _) = listener.accept().await.unwrap();
        let mut reader = BufReader::new(stream);
        line.clear();
        reader.read_line(&mut line).await.unwrap();
        let request: serde_json::Value = serde_json::from_str(&line).unwrap();
        assert_eq!(request["method"], "session.snapshot");
        let response = json!({
            "id": request["id"],
            "error": {"code": "snapshot_unavailable", "message": "refresh failed"}
        });
        reader
            .into_inner()
            .write_all(format!("{response}\n").as_bytes())
            .await
            .unwrap();
    });
    let config =
        HerdrCliConfig::from_options(None, Some("default".into()), Some(socket.clone())).unwrap();
    let error = HerdrCliAdapter::new(config)
        .mutate(
            "default",
            &ResourceMutationRequest::SpaceRename {
                space_id: "space-a".into(),
                label: "Renamed".into(),
            },
        )
        .await
        .unwrap_err();
    assert_eq!(error.code, "mutation_applied_snapshot_failed");
    assert!(error.message.contains("may already be applied"));
    assert!(error.message.contains("only resync is safe"));
    server.await.unwrap();
    drop(fs::remove_file(socket));
}

#[cfg(unix)]
#[tokio::test]
async fn propagates_subscription_setup_error_without_id() {
    let socket = std::env::temp_dir().join(format!("cockpit-herdr-events-{}.sock", temp_id()));
    let listener = UnixListener::bind(&socket).unwrap();
    let server = tokio::spawn(async move {
        serve_ping(&listener).await;
        let (stream, _) = listener.accept().await.unwrap();
        let mut reader = BufReader::new(stream);
        let mut request_line = String::new();
        reader.read_line(&mut request_line).await.unwrap();
        let request: serde_json::Value = serde_json::from_str(&request_line).unwrap();
        assert_eq!(
            request.get("method").and_then(serde_json::Value::as_str),
            Some("events.subscribe")
        );
        let mut stream = reader.into_inner();
        stream
            .write_all(b"{\"error\":{\"code\":\"invalid_request\",\"message\":\"unsupported subscription\"}}\n")
            .await
            .unwrap();
    });
    let config =
        HerdrCliConfig::from_options(None, Some("default".into()), Some(socket.clone())).unwrap();
    let snapshot = cockpit_protocol::v1::SessionSnapshotResponse {
        session_id: "default".into(),
        version: "0.8.2".into(),
        protocol: 20,
        focused_space_id: None,
        focused_tab_id: None,
        focused_pane_id: None,
        spaces: Vec::new(),
        tabs: Vec::new(),
        panes: Vec::new(),
        layouts: Vec::new(),
        agents: Vec::new(),
    };
    let error = HerdrCliAdapter::new(config)
        .subscribe_session("default", &snapshot)
        .await
        .unwrap_err();
    server.await.unwrap();
    assert_eq!(error.code, "invalid_request");
    drop(fs::remove_file(socket));
}

#[cfg(unix)]
#[tokio::test]
async fn event_subscription_rejects_identity_mismatch_before_live() {
    let socket = std::env::temp_dir().join(format!("cockpit-herdr-event-ping-{}.sock", temp_id()));
    let listener = UnixListener::bind(&socket).unwrap();
    let server = tokio::spawn(async move {
        let (stream, _) = listener.accept().await.unwrap();
        let mut reader = BufReader::new(stream);
        let mut line = String::new();
        reader.read_line(&mut line).await.unwrap();
        let request: serde_json::Value = serde_json::from_str(&line).unwrap();
        assert_eq!(request["method"], "ping");
        let response = json!({
            "id": request["id"],
            "result": {"type": "pong", "version": "0.8.1", "protocol": 20}
        });
        reader
            .into_inner()
            .write_all(format!("{response}\n").as_bytes())
            .await
            .unwrap();
    });
    let config =
        HerdrCliConfig::from_options(None, Some("default".into()), Some(socket.clone())).unwrap();
    let snapshot = SessionSnapshotResponse {
        session_id: "default".into(),
        version: "0.8.2".into(),
        protocol: 20,
        focused_space_id: None,
        focused_tab_id: None,
        focused_pane_id: None,
        spaces: Vec::new(),
        tabs: Vec::new(),
        panes: Vec::new(),
        layouts: Vec::new(),
        agents: Vec::new(),
    };
    let error = HerdrCliAdapter::new(config)
        .subscribe_session("default", &snapshot)
        .await
        .unwrap_err();
    assert_eq!(error.code, "session_identity_mismatch");
    server.await.unwrap();
    drop(fs::remove_file(socket));
}
#[cfg(unix)]
#[tokio::test]
async fn event_subscription_terminates_on_identity_replacement_without_changed() {
    let socket =
        std::env::temp_dir().join(format!("cockpit-herdr-event-replace-{}.sock", temp_id()));
    let listener = UnixListener::bind(&socket).unwrap();
    let server = tokio::spawn(async move {
        let (stream, _) = listener.accept().await.unwrap();
        let mut reader = BufReader::new(stream);
        let mut line = String::new();
        reader.read_line(&mut line).await.unwrap();
        let request: serde_json::Value = serde_json::from_str(&line).unwrap();
        assert_eq!(request["method"], "ping");
        let response = json!({
            "id": request["id"],
            "result": {"type": "pong", "version": "0.8.2", "protocol": 20}
        });
        let mut stream = reader.into_inner();
        stream
            .write_all(format!("{response}\n").as_bytes())
            .await
            .unwrap();
        drop(stream);

        let (stream, _) = listener.accept().await.unwrap();
        let mut reader = BufReader::new(stream);
        line.clear();
        reader.read_line(&mut line).await.unwrap();
        let request: serde_json::Value = serde_json::from_str(&line).unwrap();
        assert_eq!(request["method"], "events.subscribe");
        let response = json!({
            "id": request["id"],
            "result": {"type": "subscription_started"}
        });
        let event = json!({"event": "pane.created", "data": {}});
        let mut stream = reader.into_inner();
        stream
            .write_all(format!("{response}\n{event}\n").as_bytes())
            .await
            .unwrap();
        drop(stream);

        let (stream, _) = listener.accept().await.unwrap();
        let mut reader = BufReader::new(stream);
        line.clear();
        reader.read_line(&mut line).await.unwrap();
        let request: serde_json::Value = serde_json::from_str(&line).unwrap();
        assert_eq!(request["method"], "session.snapshot");
        let response = json!({
            "id": request["id"],
            "result": {
                "type": "session_snapshot",
                "snapshot": {
                    "version": "0.8.1",
                    "protocol": 21,
                    "focused_workspace_id": null,
                    "focused_tab_id": null,
                    "focused_pane_id": null,
                    "workspaces": [],
                    "tabs": [],
                    "panes": [],
                    "layouts": [],
                    "agents": []
                }
            }
        });
        reader
            .into_inner()
            .write_all(format!("{response}\n").as_bytes())
            .await
            .unwrap();
    });
    let config =
        HerdrCliConfig::from_options(None, Some("default".into()), Some(socket.clone())).unwrap();
    let snapshot = SessionSnapshotResponse {
        session_id: "default".into(),
        version: "0.8.2".into(),
        protocol: 20,
        focused_space_id: None,
        focused_tab_id: None,
        focused_pane_id: None,
        spaces: Vec::new(),
        tabs: Vec::new(),
        panes: Vec::new(),
        layouts: Vec::new(),
        agents: Vec::new(),
    };
    let mut subscription = HerdrCliAdapter::new(config)
        .subscribe_session("default", &snapshot)
        .await
        .unwrap();
    let first = tokio::time::timeout(Duration::from_secs(1), subscription.messages.recv())
        .await
        .unwrap();
    // Topology is validated before publishing Changed, so identity replacement
    // must terminate immediately without exposing state from the wrong session.
    match first {
        Some(SessionChange::Disconnected { code, .. }) => {
            assert_eq!(code, "session_identity_mismatch");
        }
        other => panic!("expected terminal identity mismatch, got {other:?}"),
    }
    let end = tokio::time::timeout(Duration::from_secs(1), subscription.messages.recv())
        .await
        .unwrap();
    assert_eq!(end, None);
    server.await.unwrap();
    drop(fs::remove_file(socket));
}

#[cfg(unix)]
#[tokio::test]
async fn pane_topology_event_refreshes_scoped_subscriptions_without_false_disconnect() {
    let socket = std::env::temp_dir().join(format!("cockpit-herdr-topology-{}.sock", temp_id()));
    let listener = UnixListener::bind(&socket).unwrap();
    let fixture: serde_json::Value =
        serde_json::from_str(include_str!("fixtures/session-snapshot.json")).unwrap();
    let mut refreshed_result = fixture.get("result").cloned().unwrap();
    refreshed_result["snapshot"]["protocol"] = json!(20);
    let server = tokio::spawn(async move {
        serve_ping(&listener).await;
        let (stream, _) = listener.accept().await.unwrap();
        let mut reader = BufReader::new(stream);
        let mut line = String::new();
        reader.read_line(&mut line).await.unwrap();
        let request: serde_json::Value = serde_json::from_str(&line).unwrap();
        assert_eq!(request["method"], "events.subscribe");
        let subscriptions = request["params"]["subscriptions"].as_array().unwrap();
        assert!(subscriptions.contains(&json!({
            "type": "pane.agent_status_changed",
            "pane_id": "stale-pane"
        })));
        let id = request["id"].clone();
        let mut stream = reader.into_inner();
        stream
            .write_all(
                format!(
                    "{}\n{}\n",
                    json!({"id": id, "result": {"type": "subscription_started"}}),
                    json!({"event": "pane.created", "data": {}})
                )
                .as_bytes(),
            )
            .await
            .unwrap();

        let (stream, _) = listener.accept().await.unwrap();
        let mut reader = BufReader::new(stream);
        line.clear();
        reader.read_line(&mut line).await.unwrap();
        let request: serde_json::Value = serde_json::from_str(&line).unwrap();
        assert_eq!(request["method"], "session.snapshot");
        let response = json!({"id": request["id"], "result": refreshed_result});
        reader
            .into_inner()
            .write_all(format!("{response}\n").as_bytes())
            .await
            .unwrap();

        let (stream, _) = listener.accept().await.unwrap();
        let mut reader = BufReader::new(stream);
        line.clear();
        reader.read_line(&mut line).await.unwrap();
        let request: serde_json::Value = serde_json::from_str(&line).unwrap();
        assert_eq!(request["method"], "worktree.list");
        let response = json!({
            "id": request["id"],
            "result": {"type": "worktree_list", "source": {}, "worktrees": "malformed"}
        });
        reader
            .into_inner()
            .write_all(format!("{response}\n").as_bytes())
            .await
            .unwrap();

        serve_ping(&listener).await;
        let (stream, _) = listener.accept().await.unwrap();
        let mut reader = BufReader::new(stream);
        line.clear();
        reader.read_line(&mut line).await.unwrap();
        let request: serde_json::Value = serde_json::from_str(&line).unwrap();
        assert_eq!(request["method"], "events.subscribe");
        let subscriptions = request["params"]["subscriptions"].as_array().unwrap();
        assert!(subscriptions.contains(&json!({
            "type": "pane.agent_status_changed",
            "pane_id": "pane-a"
        })));
        assert!(!subscriptions.iter().any(|subscription| {
            subscription
                .get("pane_id")
                .and_then(serde_json::Value::as_str)
                == Some("stale-pane")
        }));
        let response = json!({
            "id": request["id"],
            "result": {"type": "subscription_started"}
        });
        let event = json!({"event": "layout.updated", "data": {}});
        let mut stream = reader.into_inner();
        stream
            .write_all(format!("{response}\n{event}\n").as_bytes())
            .await
            .unwrap();
    });

    let initial_snapshot = SessionSnapshotResponse {
        session_id: "default".into(),
        version: "0.8.2".into(),
        protocol: 20,
        focused_space_id: None,
        focused_tab_id: None,
        focused_pane_id: None,
        spaces: Vec::new(),
        tabs: Vec::new(),
        panes: vec![PaneSummary {
            id: "stale-pane".into(),
            terminal_id: "stale-terminal".into(),
            space_id: "stale-space".into(),
            tab_id: "stale-tab".into(),
            title: None,
            focused: false,
            agent: None,
            agent_status: "idle".into(),
            revision: 0,
        }],
        layouts: Vec::new(),
        agents: Vec::new(),
    };
    let config =
        HerdrCliConfig::from_options(None, Some("default".into()), Some(socket.clone())).unwrap();
    let mut subscription = HerdrCliAdapter::new(config)
        .subscribe_session("default", &initial_snapshot)
        .await
        .unwrap();
    let first = tokio::time::timeout(Duration::from_secs(1), subscription.messages.recv())
        .await
        .unwrap();
    let second = tokio::time::timeout(Duration::from_secs(1), subscription.messages.recv())
        .await
        .unwrap();
    assert_eq!(first, Some(SessionChange::Changed));
    assert_eq!(second, Some(SessionChange::Changed));
    server.await.unwrap();
    drop(fs::remove_file(socket));
}

#[cfg(unix)]
#[tokio::test]
async fn rejects_invalid_session_and_pane_before_terminal_spawn() {
    let fixture = script("#!/bin/sh\nexit 91\n");
    let config =
        HerdrCliConfig::from_options(Some(fixture.path().to_path_buf()), None, None).unwrap();
    let adapter = HerdrCliAdapter::new(config);
    let error = adapter.session_snapshot("bad/name").await.unwrap_err();
    assert_eq!(error.code, "invalid_session_id");
    let request = cockpit_protocol::v1::TerminalOpenRequest {
        session_id: "default".into(),
        pane_id: "bad/id".into(),
        mode: cockpit_protocol::v1::TerminalMode::Observe,
        takeover: false,
        cols: 80,
        rows: 24,
        cell_width_px: 0,
        cell_height_px: 0,
    };
    let error = adapter.open_terminal(&request).await.unwrap_err();
    assert_eq!(error.code, "invalid_pane_id");
}

#[test]
fn ambient_herdr_endpoint_variables_are_not_cockpit_configuration() {
    let mut environment = BTreeMap::new();
    environment.insert("HERDR_SESSION".into(), "ambient-session".into());
    environment.insert("HERDR_SOCKET_PATH".into(), "/private/ambient.sock".into());
    let config = HerdrCliConfig::from_options_with_environment(
        Some(PathBuf::from("herdr")),
        None,
        None,
        &environment,
    )
    .unwrap();
    assert_eq!(config.session(), None);
    assert_eq!(config.socket(), None);
}
#[cfg(unix)]
#[tokio::test]
async fn custom_socket_named_session_is_the_only_advertised_session() {
    let fixture = script(
        "#!/bin/sh\nprintf '%s' '{\"sessions\":[{\"name\":\"default\",\"running\":true,\"default\":true},{\"name\":\"native-smoke\",\"running\":true,\"default\":false}]}'\n",
    );
    let config = HerdrCliConfig::from_options(
        Some(fixture.path().to_path_buf()),
        Some("native-smoke".into()),
        Some(PathBuf::from("/private/native-smoke.sock")),
    )
    .unwrap();
    let sessions = HerdrCliAdapter::new(config).sessions().await.unwrap();
    assert_eq!(sessions.sessions.len(), 1);
    assert_eq!(sessions.sessions[0].id, "native-smoke");
}

#[tokio::test]
async fn socket_pinned_adapter_rejects_other_sessions() {
    let config = HerdrCliConfig::from_options(
        Some(PathBuf::from("herdr")),
        Some("named".into()),
        Some(PathBuf::from("/private/pinned.sock")),
    )
    .unwrap();
    let adapter = HerdrCliAdapter::new(config);
    let result = adapter.session_snapshot("other").await;
    assert_eq!(result.unwrap_err().code, "session_not_selected");
}

#[cfg(unix)]
#[tokio::test]
async fn subscription_receiver_drop_closes_idle_peer_socket() {
    let socket = std::env::temp_dir().join(format!("cockpit-herdr-events-drop-{}.sock", temp_id()));
    let listener = UnixListener::bind(&socket).unwrap();
    let server = tokio::spawn(async move {
        serve_ping(&listener).await;
        let (stream, _) = listener.accept().await.unwrap();
        let mut reader = BufReader::new(stream);
        let mut request = String::new();
        reader.read_line(&mut request).await.unwrap();
        let request: serde_json::Value = serde_json::from_str(&request).unwrap();
        let response = json!({"id": request["id"], "result": {"type": "subscription_started"}});
        reader
            .get_mut()
            .write_all(format!("{response}\n").as_bytes())
            .await
            .unwrap();
        let mut event = String::new();
        assert_eq!(
            tokio::time::timeout(Duration::from_secs(2), reader.read_line(&mut event))
                .await
                .unwrap()
                .unwrap(),
            0
        );
    });
    let config =
        HerdrCliConfig::from_options(None, Some("drop-idle".into()), Some(socket.clone())).unwrap();
    let snapshot = SessionSnapshotResponse {
        session_id: "drop-idle".into(),
        version: "0.8.2".into(),
        protocol: 20,
        focused_space_id: None,
        focused_tab_id: None,
        focused_pane_id: None,
        spaces: Vec::new(),
        tabs: Vec::new(),
        panes: Vec::new(),
        layouts: Vec::new(),
        agents: Vec::new(),
    };
    let subscription = HerdrCliAdapter::new(config)
        .subscribe_session("drop-idle", &snapshot)
        .await
        .unwrap();
    drop(subscription.messages);
    tokio::time::timeout(Duration::from_secs(3), server)
        .await
        .unwrap()
        .unwrap();
    drop(fs::remove_file(socket));
}

#[cfg(unix)]
fn assert_elapsed(start: Instant, max: Duration) {
    assert!(start.elapsed() <= max, "operation exceeded {:?}", max);
}

#[cfg(unix)]
fn focus_request() -> FocusRequest {
    FocusRequest {
        kind: FocusKind::Pane,
        target_id: "pane-1".into(),
    }
}

#[cfg(unix)]
#[tokio::test]
async fn bounded_peer_case_no_response() {
    let socket =
        std::env::temp_dir().join(format!("cockpit-herdr-peer-no-response-{}.sock", temp_id()));
    let listener = UnixListener::bind(&socket).unwrap();
    let requests = Arc::new(AtomicUsize::new(0));
    let seen = Arc::clone(&requests);
    let server = tokio::spawn(async move {
        let (stream, _) = listener.accept().await.unwrap();
        let mut reader = BufReader::new(stream);
        let mut request = String::new();
        reader.read_line(&mut request).await.unwrap();
        seen.fetch_add(1, Ordering::Release);
        tokio::select! {
            _ = tokio::time::sleep(Duration::from_secs(3)) => {}
            Ok((_, _)) = listener.accept() => { seen.fetch_add(1, Ordering::Release); }
        }
    });
    let config =
        HerdrCliConfig::from_options(None, Some("peer-no-response".into()), Some(socket.clone()))
            .unwrap();
    let started = Instant::now();
    let error = tokio::time::timeout(
        Duration::from_secs(3),
        HerdrCliAdapter::new(config).focus("peer-no-response", &focus_request()),
    )
    .await
    .unwrap()
    .unwrap_err();
    assert_elapsed(started, Duration::from_secs(3));
    assert_eq!(error.code, "request_outcome_unknown");
    tokio::time::timeout(Duration::from_secs(4), server)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(requests.load(Ordering::Acquire), 1);
    drop(fs::remove_file(socket));
}

#[cfg(unix)]
#[tokio::test]
async fn bounded_peer_case_partial_line() {
    let socket =
        std::env::temp_dir().join(format!("cockpit-herdr-peer-partial-{}.sock", temp_id()));
    let listener = UnixListener::bind(&socket).unwrap();
    let requests = Arc::new(AtomicUsize::new(0));
    let seen = Arc::clone(&requests);
    let server = tokio::spawn(async move {
        let (stream, _) = listener.accept().await.unwrap();
        let mut reader = BufReader::new(stream);
        let mut request = String::new();
        reader.read_line(&mut request).await.unwrap();
        seen.fetch_add(1, Ordering::Release);
        reader.get_mut().write_all(b"{\"id\":").await.unwrap();
        tokio::select! {
            _ = tokio::time::sleep(Duration::from_secs(3)) => {}
            Ok((_, _)) = listener.accept() => { seen.fetch_add(1, Ordering::Release); }
        }
    });
    let config =
        HerdrCliConfig::from_options(None, Some("peer-partial".into()), Some(socket.clone()))
            .unwrap();
    let started = Instant::now();
    let error = tokio::time::timeout(
        Duration::from_secs(3),
        HerdrCliAdapter::new(config).focus("peer-partial", &focus_request()),
    )
    .await
    .unwrap()
    .unwrap_err();
    assert_elapsed(started, Duration::from_secs(3));
    assert_eq!(error.code, "request_outcome_unknown");
    tokio::time::timeout(Duration::from_secs(4), server)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(requests.load(Ordering::Acquire), 1);
    drop(fs::remove_file(socket));
}

#[cfg(unix)]
#[tokio::test]
async fn bounded_peer_case_unrelated_then_matching() {
    let socket =
        std::env::temp_dir().join(format!("cockpit-herdr-peer-unrelated-{}.sock", temp_id()));
    let listener = UnixListener::bind(&socket).unwrap();
    let requests = Arc::new(AtomicUsize::new(0));
    let seen = Arc::clone(&requests);
    let server = tokio::spawn(async move {
        let (stream, _) = listener.accept().await.unwrap();
        let mut reader = BufReader::new(stream);
        let mut request = String::new();
        reader.read_line(&mut request).await.unwrap();
        seen.fetch_add(1, Ordering::Release);
        let request: serde_json::Value = serde_json::from_str(&request).unwrap();
        let id = request["id"].clone();
        let mut stream = reader.into_inner();
        stream
            .write_all(b"{\"id\":\"unrelated\",\"result\":{}}\n")
            .await
            .unwrap();
        stream
            .write_all(
                format!(
                    "{}\n",
                    json!({"id": id, "result": {"type": "pane_info", "pane": {"pane_id": "pane-1", "focused": true}}})
                )
                .as_bytes(),
            )
            .await
            .unwrap();
    });
    let config =
        HerdrCliConfig::from_options(None, Some("peer-unrelated".into()), Some(socket.clone()))
            .unwrap();
    let started = Instant::now();
    let response = tokio::time::timeout(
        Duration::from_secs(2),
        HerdrCliAdapter::new(config).focus("peer-unrelated", &focus_request()),
    )
    .await
    .unwrap()
    .unwrap();
    assert_elapsed(started, Duration::from_secs(2));
    assert!(response.accepted);
    assert_eq!(requests.load(Ordering::Acquire), 1);
    tokio::time::timeout(Duration::from_secs(1), server)
        .await
        .unwrap()
        .unwrap();
    drop(fs::remove_file(socket));
}

#[cfg(unix)]
#[tokio::test]
async fn bounded_peer_case_oversized_line() {
    let socket =
        std::env::temp_dir().join(format!("cockpit-herdr-peer-oversized-{}.sock", temp_id()));
    let listener = UnixListener::bind(&socket).unwrap();
    let requests = Arc::new(AtomicUsize::new(0));
    let seen = Arc::clone(&requests);
    let server = tokio::spawn(async move {
        let (stream, _) = listener.accept().await.unwrap();
        let mut reader = BufReader::new(stream);
        let mut request = String::new();
        reader.read_line(&mut request).await.unwrap();
        seen.fetch_add(1, Ordering::Release);
        let mut response = vec![b'x'; 1024 * 1024 + 1];
        response.push(b'\n');
        reader.into_inner().write_all(&response).await.unwrap();
    });
    let config =
        HerdrCliConfig::from_options(None, Some("peer-oversized".into()), Some(socket.clone()))
            .unwrap();
    let started = Instant::now();
    let error = tokio::time::timeout(
        Duration::from_secs(2),
        HerdrCliAdapter::new(config).focus("peer-oversized", &focus_request()),
    )
    .await
    .unwrap()
    .unwrap_err();
    assert_elapsed(started, Duration::from_secs(2));
    assert_eq!(error.code, "bounded_output");
    assert_eq!(requests.load(Ordering::Acquire), 1);
    tokio::time::timeout(Duration::from_secs(1), server)
        .await
        .unwrap()
        .unwrap();
    drop(fs::remove_file(socket));
}

#[cfg(unix)]
#[tokio::test]
async fn bounded_peer_case_malformed_json() {
    let socket =
        std::env::temp_dir().join(format!("cockpit-herdr-peer-malformed-{}.sock", temp_id()));
    let listener = UnixListener::bind(&socket).unwrap();
    let requests = Arc::new(AtomicUsize::new(0));
    let seen = Arc::clone(&requests);
    let server = tokio::spawn(async move {
        let (stream, _) = listener.accept().await.unwrap();
        let mut reader = BufReader::new(stream);
        let mut request = String::new();
        reader.read_line(&mut request).await.unwrap();
        seen.fetch_add(1, Ordering::Release);
        reader.into_inner().write_all(b"not-json\n").await.unwrap();
    });
    let config =
        HerdrCliConfig::from_options(None, Some("peer-malformed".into()), Some(socket.clone()))
            .unwrap();
    let started = Instant::now();
    let error = tokio::time::timeout(
        Duration::from_secs(2),
        HerdrCliAdapter::new(config).focus("peer-malformed", &focus_request()),
    )
    .await
    .unwrap()
    .unwrap_err();
    assert_elapsed(started, Duration::from_secs(2));
    assert_eq!(error.code, "malformed_response");
    assert_eq!(requests.load(Ordering::Acquire), 1);
    tokio::time::timeout(Duration::from_secs(1), server)
        .await
        .unwrap()
        .unwrap();
    drop(fs::remove_file(socket));
}

#[cfg(unix)]
#[tokio::test]
async fn bounded_peer_case_matching_server_error() {
    let socket = std::env::temp_dir().join(format!("cockpit-herdr-peer-error-{}.sock", temp_id()));
    let listener = UnixListener::bind(&socket).unwrap();
    let requests = Arc::new(AtomicUsize::new(0));
    let seen = Arc::clone(&requests);
    let server = tokio::spawn(async move {
        let (stream, _) = listener.accept().await.unwrap();
        let mut reader = BufReader::new(stream);
        let mut request = String::new();
        reader.read_line(&mut request).await.unwrap();
        seen.fetch_add(1, Ordering::Release);
        let request: serde_json::Value = serde_json::from_str(&request).unwrap();
        let response = json!({"id": request["id"], "error": {"code": "permission_denied", "message": "focus denied"}});
        reader
            .into_inner()
            .write_all(format!("{response}\n").as_bytes())
            .await
            .unwrap();
    });
    let config =
        HerdrCliConfig::from_options(None, Some("peer-error".into()), Some(socket.clone()))
            .unwrap();
    let started = Instant::now();
    let error = tokio::time::timeout(
        Duration::from_secs(2),
        HerdrCliAdapter::new(config).focus("peer-error", &focus_request()),
    )
    .await
    .unwrap()
    .unwrap_err();
    assert_elapsed(started, Duration::from_secs(2));
    assert_eq!(error.code, "permission_denied");
    assert_eq!(error.message, "focus denied");
    assert_eq!(requests.load(Ordering::Acquire), 1);
    tokio::time::timeout(Duration::from_secs(1), server)
        .await
        .unwrap()
        .unwrap();
    drop(fs::remove_file(socket));
}

#[cfg(unix)]
#[tokio::test]
async fn bounded_peer_case_response_after_deadline_is_unknown() {
    let socket = std::env::temp_dir().join(format!("cockpit-herdr-peer-late-{}.sock", temp_id()));
    let listener = UnixListener::bind(&socket).unwrap();
    let requests = Arc::new(AtomicUsize::new(0));
    let seen = Arc::clone(&requests);
    let server = tokio::spawn(async move {
        let (stream, _) = listener.accept().await.unwrap();
        let mut reader = BufReader::new(stream);
        let mut request = String::new();
        reader.read_line(&mut request).await.unwrap();
        seen.fetch_add(1, Ordering::Release);
        tokio::select! {
            _ = tokio::time::sleep(Duration::from_secs(3)) => {}
            Ok((_, _)) = listener.accept() => { seen.fetch_add(1, Ordering::Release); }
        }
    });
    let config =
        HerdrCliConfig::from_options(None, Some("peer-late".into()), Some(socket.clone())).unwrap();
    let started = Instant::now();
    let error = tokio::time::timeout(
        Duration::from_secs(3),
        HerdrCliAdapter::new(config).mutate(
            "peer-late",
            &ResourceMutationRequest::SpaceRename {
                space_id: "space-1".into(),
                label: "new".into(),
            },
        ),
    )
    .await
    .unwrap()
    .unwrap_err();
    assert_elapsed(started, Duration::from_secs(3));
    assert_eq!(error.code, "request_outcome_unknown");
    tokio::time::timeout(Duration::from_secs(4), server)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(requests.load(Ordering::Acquire), 1);
    drop(fs::remove_file(socket));
}

#[cfg(unix)]
#[tokio::test]
async fn bounded_peer_case_caller_cancellation() {
    let socket = std::env::temp_dir().join(format!("cockpit-herdr-peer-cancel-{}.sock", temp_id()));
    let listener = UnixListener::bind(&socket).unwrap();
    let requests = Arc::new(AtomicUsize::new(0));
    let closed = Arc::new(AtomicUsize::new(0));
    let seen = Arc::clone(&requests);
    let was_closed = Arc::clone(&closed);
    let server = tokio::spawn(async move {
        let (stream, _) = listener.accept().await.unwrap();
        let mut reader = BufReader::new(stream);
        let mut request = String::new();
        reader.read_line(&mut request).await.unwrap();
        seen.fetch_add(1, Ordering::Release);
        let mut next = String::new();
        tokio::time::timeout(Duration::from_secs(2), async {
            tokio::select! {
                result = reader.read_line(&mut next) => {
                    if result.unwrap() == 0 {
                        was_closed.store(1, Ordering::Release);
                    }
                }
                Ok((_, _)) = listener.accept() => {
                    seen.fetch_add(1, Ordering::Release);
                }
            }
        })
        .await
        .unwrap();
    });
    let config =
        HerdrCliConfig::from_options(None, Some("peer-cancel".into()), Some(socket.clone()))
            .unwrap();
    let adapter = HerdrCliAdapter::new(config);
    let task = tokio::spawn(async move { adapter.focus("peer-cancel", &focus_request()).await });
    let started = Instant::now();
    tokio::time::timeout(Duration::from_secs(1), async {
        while requests.load(Ordering::Acquire) == 0 {
            tokio::time::sleep(Duration::from_millis(1)).await;
        }
    })
    .await
    .unwrap();
    task.abort();
    assert!(task.await.unwrap_err().is_cancelled());
    tokio::time::timeout(Duration::from_secs(3), server)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(requests.load(Ordering::Acquire), 1);
    assert!(started.elapsed() < Duration::from_secs(3));
    assert_eq!(
        closed.load(Ordering::Acquire),
        1,
        "peer socket was not closed"
    );
    drop(fs::remove_file(socket));
}
