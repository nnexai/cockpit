use std::collections::BTreeMap;
use std::fs;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

use cockpit_core::{HerdrAdapter, SessionChange};
use cockpit_herdr::{HerdrCliAdapter, HerdrCliConfig};
use cockpit_protocol::v1::{
    PaneSummary, ResourceMutationRequest, SessionSnapshotResponse, TerminalMode,
    TerminalOpenRequest,
};
use serde_json::json;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::UnixListener;

#[cfg(unix)]
static NEXT_TEMP_ID: AtomicU64 = AtomicU64::new(0);

#[cfg(unix)]
fn temp_id() -> String {
    format!(
        "{}-{}",
        std::process::id(),
        NEXT_TEMP_ID.fetch_add(1, Ordering::Relaxed)
    )
}

#[cfg(unix)]
fn script(body: &str) -> PathBuf {
    use std::os::unix::fs::PermissionsExt;
    let path = std::env::temp_dir().join(format!("cockpit-herdr-test-{}.sh", temp_id()));
    fs::write(&path, body).unwrap();
    let mut permissions = fs::metadata(&path).unwrap().permissions();
    permissions.set_mode(0o700);
    fs::set_permissions(&path, permissions).unwrap();
    path
}

#[cfg(unix)]
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
        protocol: 22,
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
async fn pane_topology_event_refreshes_scoped_subscriptions_without_false_disconnect() {
    let socket = std::env::temp_dir().join(format!("cockpit-herdr-topology-{}.sock", temp_id()));
    let listener = UnixListener::bind(&socket).unwrap();
    let fixture: serde_json::Value =
        serde_json::from_str(include_str!("fixtures/session-snapshot.json")).unwrap();
    let refreshed_result = fixture.get("result").cloned().unwrap();
    let server = tokio::spawn(async move {
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
        protocol: 22,
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
    let config = HerdrCliConfig::from_options(Some(fixture.clone()), None, None).unwrap();
    let adapter = HerdrCliAdapter::new(config);
    let error = adapter.session_snapshot("bad/name").await.unwrap_err();
    assert_eq!(error.code, "invalid_session_id");
    let request = cockpit_protocol::v1::TerminalOpenRequest {
        client_surface_id: "surface-1".into(),
        session_id: "default".into(),
        pane_id: "bad/id".into(),
        mode: cockpit_protocol::v1::TerminalMode::Observe,
        takeover: false,
        cols: 80,
        rows: 24,
        cell_width_px: 0,
        cell_height_px: 0,
        surface_cols: 80,
        surface_rows: 24,
    };
    let error = adapter.open_terminal(&request).await.unwrap_err();
    assert_eq!(error.code, "invalid_pane_id");
    drop(fs::remove_file(fixture));
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
        Some(fixture.clone()),
        Some("native-smoke".into()),
        Some(PathBuf::from("/private/native-smoke.sock")),
    )
    .unwrap();
    let sessions = HerdrCliAdapter::new(config).sessions().await.unwrap();
    assert_eq!(sessions.sessions.len(), 1);
    assert_eq!(sessions.sessions[0].id, "native-smoke");
    drop(fs::remove_file(fixture));
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
