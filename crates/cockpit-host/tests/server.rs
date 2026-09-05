use async_trait::async_trait;
use axum::{http::Request, serve};
use cockpit_core::{
    CockpitService, HerdrAdapter, InspectionError, SessionSubscription, TerminalSession,
};
use cockpit_host::server::{build_router, validate_bind, validate_static_root};
use cockpit_protocol::v1::{
    AgentSummary, CockpitMode, FocusRequest, FocusResponse, HerdrCompatibility, HerdrIdentity,
    LayoutPane, LayoutRect, PaneSummary, ResourceMutationRequest, ResourceMutationResponse,
    SessionListResponse, SessionSnapshotResponse, SessionSummary, SpaceSummary, TabLayout,
    TabSummary, TerminalOpenRequest,
};
use std::{
    io::{BufRead, BufReader},
    net::SocketAddr,
    path::PathBuf,
    process::{Command, Stdio},
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
    time::{SystemTime, UNIX_EPOCH},
};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::{TcpListener, TcpStream},
};
use tower::ServiceExt;

fn test_authority() -> SocketAddr {
    "127.0.0.1:43123".parse().expect("test authority")
}

fn service() -> CockpitService {
    CockpitService::new(
        CockpitMode::Test,
        Arc::new(FakeAdapter {
            snapshot: Err(InspectionError::new(
                "live_inspection_disabled",
                "live inspection disabled in test mode",
            )),
            sessions: Err(InspectionError::new(
                "live_inspection_disabled",
                "live inspection disabled in test mode",
            )),
            focus: Err(InspectionError::new(
                "live_inspection_disabled",
                "live inspection disabled in test mode",
            )),
            mutation: Err(InspectionError::new(
                "live_inspection_disabled",
                "live inspection disabled in test mode",
            )),
            mutation_calls: Arc::new(AtomicUsize::new(0)),
        }),
    )
}
#[derive(Clone)]
struct FakeAdapter {
    snapshot: Result<SessionSnapshotResponse, InspectionError>,
    sessions: Result<SessionListResponse, InspectionError>,
    focus: Result<FocusResponse, InspectionError>,
    mutation: Result<ResourceMutationResponse, InspectionError>,
    mutation_calls: Arc<AtomicUsize>,
}

#[async_trait]
impl HerdrAdapter for FakeAdapter {
    async fn inspect(&self) -> Result<HerdrCompatibility, InspectionError> {
        Ok(HerdrCompatibility::Compatible {
            identity: HerdrIdentity {
                version: "0.8.2".to_owned(),
                protocol: 20,
                schema_version: 1,
            },
        })
    }

    async fn inspect_session(
        &self,
        _session_id: &str,
    ) -> Result<HerdrCompatibility, InspectionError> {
        self.inspect().await
    }

    async fn sessions(&self) -> Result<SessionListResponse, InspectionError> {
        self.sessions.clone()
    }

    async fn session_snapshot(
        &self,
        _session_id: &str,
    ) -> Result<SessionSnapshotResponse, InspectionError> {
        self.snapshot.clone()
    }

    async fn focus(
        &self,
        _session_id: &str,
        _request: &FocusRequest,
    ) -> Result<FocusResponse, InspectionError> {
        self.focus.clone()
    }

    async fn mutate(
        &self,
        _session_id: &str,
        _request: &ResourceMutationRequest,
    ) -> Result<ResourceMutationResponse, InspectionError> {
        self.mutation_calls.fetch_add(1, Ordering::Relaxed);
        self.mutation.clone()
    }

    async fn subscribe_session(
        &self,
        _session_id: &str,
        _snapshot: &SessionSnapshotResponse,
    ) -> Result<SessionSubscription, InspectionError> {
        let (_sender, receiver) = tokio::sync::mpsc::channel(1);
        Ok(SessionSubscription { messages: receiver })
    }

    async fn open_terminal(
        &self,
        _request: &TerminalOpenRequest,
    ) -> Result<TerminalSession, InspectionError> {
        Err(InspectionError::new(
            "terminal_attach_failed",
            "terminal unavailable",
        ))
    }
}

fn live_service(
    snapshot_result: Result<SessionSnapshotResponse, InspectionError>,
) -> CockpitService {
    live_service_with_counter(snapshot_result, Arc::new(AtomicUsize::new(0)))
}

fn live_service_with_counter(
    snapshot_result: Result<SessionSnapshotResponse, InspectionError>,
    mutation_calls: Arc<AtomicUsize>,
) -> CockpitService {
    CockpitService::new(
        CockpitMode::Normal,
        Arc::new(FakeAdapter {
            snapshot: snapshot_result,
            sessions: Ok(SessionListResponse {
                sessions: vec![SessionSummary {
                    id: "session-1".to_owned(),
                    label: "Main".to_owned(),
                    is_default: true,
                    running: true,
                }],
            }),
            focus: Ok(FocusResponse {
                session_id: "session-1".to_owned(),
                kind: cockpit_protocol::v1::FocusKind::Pane,
                target_id: "pane-1".to_owned(),
                accepted: true,
            }),
            mutation: Ok(ResourceMutationResponse {
                session_id: "session-1".to_owned(),
                snapshot: snapshot(),
            }),
            mutation_calls,
        }),
    )
}

fn snapshot() -> SessionSnapshotResponse {
    SessionSnapshotResponse {
        session_id: "session-1".to_owned(),
        version: "0.8.2".to_owned(),
        protocol: 20,
        focused_space_id: Some("space-1".to_owned()),
        focused_tab_id: Some("tab-1".to_owned()),
        focused_pane_id: Some("pane-1".to_owned()),
        spaces: vec![SpaceSummary {
            id: "space-1".to_owned(),
            label: "Main".to_owned(),
            number: 1,
            tab_count: 1,
            pane_count: 1,
            focused: true,
            agent_status: "idle".to_owned(),
            git: None,
        }],
        tabs: vec![TabSummary {
            id: "tab-1".to_owned(),
            space_id: "space-1".to_owned(),
            label: "Shell".to_owned(),
            number: 1,
            pane_count: 1,
            focused: true,
        }],
        panes: vec![PaneSummary {
            id: "pane-1".to_owned(),
            terminal_id: "terminal-1".to_owned(),
            space_id: "space-1".to_owned(),
            tab_id: "tab-1".to_owned(),
            title: Some("Terminal".to_owned()),
            focused: true,
            agent: Some("codex".to_owned()),
            agent_status: "working".to_owned(),
            revision: 7,
        }],
        layouts: vec![TabLayout {
            space_id: "space-1".to_owned(),
            tab_id: "tab-1".to_owned(),
            area: LayoutRect {
                x: 0,
                y: 0,
                width: 80,
                height: 24,
            },
            focused_pane_id: Some("pane-1".to_owned()),
            panes: vec![LayoutPane {
                pane_id: "pane-1".to_owned(),
                focused: true,
                rect: LayoutRect {
                    x: 0,
                    y: 0,
                    width: 80,
                    height: 24,
                },
            }],
            zoomed: false,
        }],
        agents: vec![AgentSummary {
            pane_id: "pane-1".to_owned(),
            space_id: "space-1".to_owned(),
            tab_id: "tab-1".to_owned(),
            name: "codex".to_owned(),
            status: "working".to_owned(),
            title: Some("Terminal".to_owned()),
            focused: true,
            state_change_seq: 42,
        }],
    }
}

fn fixture_root() -> PathBuf {
    static NEXT_FIXTURE: AtomicUsize = AtomicUsize::new(0);
    let id = NEXT_FIXTURE.fetch_add(1, Ordering::Relaxed);
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("clock")
        .as_nanos();
    let root =
        std::env::temp_dir().join(format!("cockpit-host-{}-{nonce}-{id}", std::process::id()));
    std::fs::create_dir(&root).expect("fixture root");
    std::fs::write(
        root.join("index.html"),
        "<!doctype html><main>cockpit</main>",
    )
    .expect("index");
    std::fs::write(root.join("app.js"), "console.log('cockpit')").expect("asset");
    root
}

async fn request(addr: std::net::SocketAddr, path: &str) -> (u16, String) {
    let mut stream = TcpStream::connect(addr).await.expect("connect");
    let request = format!("GET {path} HTTP/1.1\r\nHost: {addr}\r\nConnection: close\r\n\r\n");
    stream.write_all(request.as_bytes()).await.expect("request");
    let mut bytes = Vec::new();
    stream.read_to_end(&mut bytes).await.expect("response");
    let response = String::from_utf8(bytes).expect("UTF-8 response");
    let mut lines = response.splitn(2, "\r\n");
    let status = lines
        .next()
        .and_then(|line| line.split_whitespace().nth(1))
        .expect("status line")
        .parse()
        .expect("status code");
    let body = response
        .split_once("\r\n\r\n")
        .map_or_else(String::new, |(_, body)| body.to_owned());
    (status, body)
}

#[tokio::test]
async fn serves_root_spa_assets_and_precise_fallbacks() {
    let root = fixture_root();
    let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind");
    let addr = listener.local_addr().expect("address");
    let router = build_router(service(), &root, addr).expect("router");
    let task = tokio::spawn(async move { serve(listener, router).await });

    let (status, root_body) = request(addr, "/").await;
    assert_eq!(status, 200);
    assert!(root_body.contains("cockpit"));
    let (status, deep_body) = request(addr, "/spaces/one").await;
    assert_eq!(status, 200);
    assert!(deep_body.contains("cockpit"));
    let (status, asset_body) = request(addr, "/app.js").await;
    assert_eq!(status, 200);
    assert!(asset_body.contains("console.log"));
    let (status, _) = request(addr, "/missing.js").await;
    assert_eq!(status, 404);

    let (status, body) = request(addr, "/api/v1/status").await;
    assert_eq!(status, 200);
    assert!(body.contains("\"mode\":\"test\""));
    assert!(body.contains("live_inspection_disabled"));
    let (status, body) = request(addr, "/api/v1/sessions").await;
    assert_eq!(status, 503);
    assert!(body.contains("\"code\":\"live_inspection_disabled\""));
    let (status, _) = request(addr, "/api/v1/session").await;
    assert_eq!(status, 404);
    let (status, _) = request(addr, "/api/v1/panes/pane-1/output").await;
    assert_eq!(status, 404);
    let (status, body) = request(addr, "/api/v1/unknown").await;
    assert_eq!(status, 404);
    assert!(body.contains("\"code\":\"not_found\""));
    let (status, _) = request(addr, "/api%FF/v1/session").await;
    assert_eq!(status, 404);
    let (status, body) = request(addr, "/api%2Fv1%2Funknown").await;
    assert_eq!(status, 404);
    assert!(body.contains("\"code\":\"not_found\""));

    task.abort();
    std::fs::remove_dir_all(root).expect("cleanup");
}

#[tokio::test]
async fn serves_live_session_routes() {
    let root = fixture_root();
    let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind");
    let addr = listener.local_addr().expect("address");
    let router = build_router(live_service(Ok(snapshot())), &root, addr).expect("router");
    let websocket_router = build_router(live_service(Ok(snapshot())), &root, addr).expect("router");
    let task = tokio::spawn(async move { serve(listener, router).await });

    let (status, body) = request(addr, "/api/v1/sessions/session-1/snapshot").await;
    assert_eq!(status, 200);
    assert!(body.contains("\"session_id\":\"session-1\""));
    assert!(body.contains("\"version\":\"0.8.2\""));

    let (status, body) = request(addr, "/api/v1/sessions").await;
    assert_eq!(status, 200);
    assert!(body.contains("\"id\":\"session-1\""));

    let (status, _) = request(addr, "/api/v1/session").await;
    assert_eq!(status, 404);
    let (status, _) = request(addr, "/api/v1/panes/pane-1/output").await;
    assert_eq!(status, 404);
    let response = websocket_router
        .oneshot(
            Request::builder()
                .uri("/api/v1/sessions/session-1/panes/w1A:p1/terminal?mode=observe&takeover=false&cols=80&rows=24&cell_width_px=8&cell_height_px=16")
                .header("host", addr.to_string())
                .header("connection", "Upgrade")
                .header("upgrade", "websocket")
                .header("sec-websocket-version", "13")
                .header("sec-websocket-key", "dGhlIHNhbXBsZSBub25jZQ==")
                .body(axum::body::Body::empty())
                .expect("websocket request"),
        )
        .await
        .expect("websocket response");
    // Axum rejects the synthetic upgrade before handler execution; 426 proves
    // the colon-qualified pane passed host route validation (invalid IDs return 400).
    assert_eq!(response.status(), 426);

    task.abort();
    std::fs::remove_dir_all(root).expect("cleanup");
}

#[tokio::test]
async fn mutation_route_returns_authoritative_snapshot_and_maps_errors() {
    let root = fixture_root();
    let route = "/api/v1/sessions/session-1/mutations";
    let request = || {
        Request::builder()
            .method("POST")
            .uri(route)
            .header("host", test_authority().to_string())
            .header("content-type", "application/json")
            .body(axum::body::Body::from(
                r#"{"type":"pane_close","pane_id":"pane-1"}"#,
            ))
            .expect("mutation request")
    };

    let response = build_router(live_service(Ok(snapshot())), &root, test_authority())
        .expect("router")
        .oneshot(request())
        .await
        .expect("mutation response");
    assert_eq!(response.status(), 200);
    let body = axum::body::to_bytes(response.into_body(), 256 * 1024)
        .await
        .expect("mutation body");
    let body: ResourceMutationResponse =
        serde_json::from_slice(&body).expect("mutation response JSON");
    assert_eq!(body.session_id, "session-1");
    assert_eq!(body.snapshot, snapshot());

    let response = build_router(service(), &root, test_authority())
        .expect("router")
        .oneshot(request())
        .await
        .expect("mutation error response");
    assert_eq!(response.status(), 503);
    let body = axum::body::to_bytes(response.into_body(), 4096)
        .await
        .expect("mutation error body");
    assert!(String::from_utf8_lossy(&body).contains("live_inspection_disabled"));

    std::fs::remove_dir_all(root).expect("cleanup");
}

#[tokio::test]
async fn mutation_route_rejects_malformed_and_oversized_bodies() {
    let root = fixture_root();
    let route = "/api/v1/sessions/session-1/mutations";
    let router =
        build_router(live_service(Ok(snapshot())), &root, test_authority()).expect("router");
    let malformed = Request::builder()
        .method("POST")
        .uri(route)
        .header("host", test_authority().to_string())
        .header("content-type", "application/json")
        .body(axum::body::Body::from(r#"{"type":"pane_close"}"#))
        .expect("malformed request");
    let response = router
        .clone()
        .oneshot(malformed)
        .await
        .expect("malformed response");
    assert!(response.status().is_client_error());

    let oversized = serde_json::json!({
        "type": "space_create",
        "cwd": null,
        "label": "x".repeat(64 * 1024),
    })
    .to_string();
    let request = Request::builder()
        .method("POST")
        .uri(route)
        .header("host", test_authority().to_string())
        .header("content-type", "application/json")
        .body(axum::body::Body::from(oversized))
        .expect("oversized request");
    let response = router.oneshot(request).await.expect("oversized response");
    assert_eq!(response.status(), 413);

    std::fs::remove_dir_all(root).expect("cleanup");
}

#[tokio::test]
async fn authority_guard_allows_exact_loopback_authority_and_optional_same_origin() {
    let root = fixture_root();
    let authority = test_authority();
    let router = build_router(service(), &root, authority).expect("IPv4 router");

    let response = router
        .clone()
        .oneshot(
            Request::builder()
                .uri("/api/v1/status")
                .header("host", authority.to_string())
                .header("origin", format!("http://{authority}"))
                .body(axum::body::Body::empty())
                .expect("same-origin request"),
        )
        .await
        .expect("same-origin response");
    assert_eq!(response.status(), 200);

    let response = router
        .oneshot(
            Request::builder()
                .uri("/api/v1/status")
                .header("host", authority.to_string())
                .body(axum::body::Body::empty())
                .expect("originless request"),
        )
        .await
        .expect("originless response");
    assert_eq!(response.status(), 200);

    let ipv6_authority: SocketAddr = "[::1]:43123".parse().expect("IPv6 authority");
    let response = build_router(service(), &root, ipv6_authority)
        .expect("IPv6 router")
        .oneshot(
            Request::builder()
                .uri("/api/v1/status")
                .header("host", ipv6_authority.to_string())
                .header("origin", format!("http://{ipv6_authority}"))
                .body(axum::body::Body::empty())
                .expect("IPv6 request"),
        )
        .await
        .expect("IPv6 response");
    assert_eq!(response.status(), 200);

    std::fs::remove_dir_all(root).expect("cleanup");
}

#[tokio::test]
async fn authority_guard_rejects_hostile_http_websocket_and_mutation_requests() {
    let root = fixture_root();
    let authority = test_authority();
    let mutation_calls = Arc::new(AtomicUsize::new(0));
    let router = build_router(
        live_service_with_counter(Ok(snapshot()), mutation_calls.clone()),
        &root,
        authority,
    )
    .expect("router");

    let response = router
        .clone()
        .oneshot(
            Request::builder()
                .uri("/api/v1/status")
                .header("host", "attacker.example:43123")
                .body(axum::body::Body::empty())
                .expect("hostile host request"),
        )
        .await
        .expect("hostile host response");
    assert_eq!(response.status(), 403);

    let response = router
        .clone()
        .oneshot(
            Request::builder()
                .uri("/api/v1/status")
                .header("host", authority.to_string())
                .header("origin", "http://localhost:43123")
                .body(axum::body::Body::empty())
                .expect("mismatching origin request"),
        )
        .await
        .expect("mismatching origin response");
    assert_eq!(response.status(), 403);

    let response = router
        .clone()
        .oneshot(
            Request::builder()
                .uri("/api/v1/sessions/session-1/events")
                .header("host", "localhost:43123")
                .header("connection", "Upgrade")
                .header("upgrade", "websocket")
                .header("sec-websocket-version", "13")
                .header("sec-websocket-key", "dGhlIHNhbXBsZSBub25jZQ==")
                .body(axum::body::Body::empty())
                .expect("hostile websocket request"),
        )
        .await
        .expect("hostile websocket response");
    assert_eq!(response.status(), 403);

    for (host, origin) in [
        ("attacker.example:43123", None),
        ("127.0.0.1:43123", Some("http://attacker.example:43123")),
    ] {
        let mut request = Request::builder()
            .method("POST")
            .uri("/api/v1/sessions/session-1/mutations")
            .header("host", host)
            .header("content-type", "application/json");
        if let Some(origin) = origin {
            request = request.header("origin", origin);
        }
        let response = router
            .clone()
            .oneshot(
                request
                    .body(axum::body::Body::from(
                        r#"{"type":"pane_close","pane_id":"pane-1"}"#,
                    ))
                    .expect("rejected mutation request"),
            )
            .await
            .expect("rejected mutation response");
        assert_eq!(response.status(), 403);
    }
    assert_eq!(mutation_calls.load(Ordering::Relaxed), 0);

    std::fs::remove_dir_all(root).expect("cleanup");
}

#[test]
fn rejects_non_loopback_and_invalid_static_roots() {
    let remote: std::net::SocketAddr = "192.0.2.1:8080".parse().expect("address");
    assert!(validate_bind(remote).is_err());

    let missing = fixture_root();
    std::fs::remove_dir_all(&missing).expect("remove fixture root");

    assert!(validate_static_root(missing).is_err());

    let root = fixture_root();
    std::fs::remove_file(root.join("index.html")).expect("remove index");
    assert!(validate_static_root(&root).is_err());
    std::fs::remove_dir_all(root).expect("cleanup");
}

#[test]
fn serve_startup_reports_bind_and_rejects_invalid_configuration() {
    let binary = env!("CARGO_BIN_EXE_cockpit");
    let root = fixture_root();
    let mut child = Command::new(binary)
        .args([
            "serve",
            "--bind",
            "127.0.0.1:0",
            "--static-dir",
            root.to_str().expect("UTF-8 root"),
            "--test-mode",
            "--herdr",
            "/definitely/not/herdr",
        ])
        .env_remove("COCKPIT_HERDR_SESSION")
        .env_remove("COCKPIT_HERDR_SOCKET")
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("start cockpit");
    let stdout = child.stdout.take().expect("stdout");
    let mut reader = BufReader::new(stdout);
    let mut line = String::new();
    reader.read_line(&mut line).expect("listening line");
    assert!(line.starts_with("listening http://127.0.0.1:"));
    let port = line
        .trim()
        .strip_prefix("listening http://127.0.0.1:")
        .expect("line prefix")
        .parse::<u16>()
        .expect("port");
    assert_eq!(
        line.trim_end(),
        format!("listening http://127.0.0.1:{port}")
    );
    child.kill().expect("stop cockpit");
    let _ = child.wait().expect("wait cockpit");

    let output = Command::new(binary)
        .args([
            "serve",
            "--bind",
            "192.0.2.1:8080",
            "--static-dir",
            root.to_str().expect("UTF-8 root"),
            "--test-mode",
        ])
        .env_remove("COCKPIT_HERDR_SESSION")
        .env_remove("COCKPIT_HERDR_SOCKET")
        .output()
        .expect("reject remote bind");
    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("loopback"));

    let missing = fixture_root();
    std::fs::remove_dir_all(&missing).expect("remove missing root");
    let output = Command::new(binary)
        .args([
            "serve",
            "--bind",
            "127.0.0.1:0",
            "--static-dir",
            missing.to_str().expect("UTF-8 root"),
            "--test-mode",
        ])
        .env_remove("COCKPIT_HERDR_SESSION")
        .env_remove("COCKPIT_HERDR_SOCKET")
        .output()
        .expect("reject missing root");
    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("static root"));

    std::fs::remove_file(root.join("index.html")).expect("remove index");
    let output = Command::new(binary)
        .args([
            "serve",
            "--bind",
            "127.0.0.1:0",
            "--static-dir",
            root.to_str().expect("UTF-8 root"),
            "--test-mode",
        ])
        .env_remove("COCKPIT_HERDR_SESSION")
        .env_remove("COCKPIT_HERDR_SOCKET")
        .output()
        .expect("reject missing index");
    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("static index"));

    std::fs::remove_dir_all(root).expect("cleanup");
}
