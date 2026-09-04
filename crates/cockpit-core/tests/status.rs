use std::sync::{
    Arc,
    atomic::{AtomicUsize, Ordering},
};

use async_trait::async_trait;
use cockpit_core::{
    CockpitService, HerdrAdapter, InspectionError, SessionChange, SessionSubscription,
    TerminalSession,
};
use cockpit_protocol::v1::{
    CockpitMode, FocusKind, FocusRequest, FocusResponse, HerdrCompatibility, HerdrIdentity,
    LayoutPane, LayoutRect, PaneMoveDestination, PaneResizeDirection, PaneSummary,
    ResourceMutationRequest, ResourceMutationResponse, SessionListResponse,
    SessionSnapshotResponse, SessionSummary, TabLayout, TabSummary, TerminalCommand, TerminalMode,
    TerminalOpenRequest, TerminalOwnershipState, TerminalStreamMessage,
};
use tokio::sync::{Mutex, mpsc};

struct FakeAdapter {
    inspect_calls: Arc<AtomicUsize>,
    inspect_session_calls: Arc<Mutex<Vec<String>>>,
    snapshot_calls: Arc<Mutex<Vec<String>>>,
    focus_calls: Arc<Mutex<Vec<(String, FocusRequest)>>>,
    mutation_calls: Arc<Mutex<Vec<(String, ResourceMutationRequest)>>>,
    subscribe_calls: Arc<Mutex<Vec<String>>>,
    terminal_calls: Arc<Mutex<Vec<TerminalOpenRequest>>>,
    compatibility: Result<HerdrCompatibility, InspectionError>,
    session_compatibility: Result<HerdrCompatibility, InspectionError>,
    snapshots: Arc<Mutex<Result<SessionSnapshotResponse, InspectionError>>>,
    sessions_result: Result<SessionListResponse, InspectionError>,
    focus_result: Result<FocusResponse, InspectionError>,
    mutation_result: Result<ResourceMutationResponse, InspectionError>,
    subscribe_result: Arc<Mutex<Option<Result<SessionSubscription, InspectionError>>>>,
    terminal_result: Arc<Mutex<Option<Result<TerminalSession, InspectionError>>>>,
}

#[async_trait]
impl HerdrAdapter for FakeAdapter {
    async fn inspect(&self) -> Result<HerdrCompatibility, InspectionError> {
        self.inspect_calls.fetch_add(1, Ordering::SeqCst);
        self.compatibility.clone()
    }

    async fn inspect_session(
        &self,
        session_id: &str,
    ) -> Result<HerdrCompatibility, InspectionError> {
        self.inspect_session_calls
            .lock()
            .await
            .push(session_id.to_owned());
        self.session_compatibility.clone()
    }

    async fn sessions(&self) -> Result<SessionListResponse, InspectionError> {
        self.sessions_result.clone()
    }

    async fn session_snapshot(
        &self,
        session_id: &str,
    ) -> Result<SessionSnapshotResponse, InspectionError> {
        self.snapshot_calls.lock().await.push(session_id.to_owned());
        self.snapshots.lock().await.clone()
    }

    async fn focus(
        &self,
        session_id: &str,
        request: &FocusRequest,
    ) -> Result<FocusResponse, InspectionError> {
        self.focus_calls
            .lock()
            .await
            .push((session_id.to_owned(), request.clone()));
        self.focus_result.clone()
    }
    async fn mutate(
        &self,
        session_id: &str,
        request: &ResourceMutationRequest,
    ) -> Result<ResourceMutationResponse, InspectionError> {
        self.mutation_calls
            .lock()
            .await
            .push((session_id.to_owned(), request.clone()));
        self.mutation_result.clone()
    }

    async fn subscribe_session(
        &self,
        session_id: &str,
        _snapshot: &SessionSnapshotResponse,
    ) -> Result<SessionSubscription, InspectionError> {
        self.subscribe_calls
            .lock()
            .await
            .push(session_id.to_owned());
        self.subscribe_result
            .lock()
            .await
            .take()
            .unwrap_or_else(|| Ok(empty_subscription()))
    }

    async fn open_terminal(
        &self,
        request: &TerminalOpenRequest,
    ) -> Result<TerminalSession, InspectionError> {
        self.terminal_calls.lock().await.push(request.clone());
        self.terminal_result
            .lock()
            .await
            .take()
            .unwrap_or_else(|| Ok(empty_terminal()))
    }
}

fn compatible() -> HerdrCompatibility {
    HerdrCompatibility::Compatible {
        identity: HerdrIdentity {
            version: "0.8.2".into(),
            protocol: 22,
            schema_version: 1,
        },
    }
}

fn snapshot(session_id: &str) -> SessionSnapshotResponse {
    SessionSnapshotResponse {
        session_id: session_id.into(),
        version: "0.8.2".into(),
        protocol: 22,
        focused_space_id: Some("space-1".into()),
        focused_tab_id: Some("tab-1".into()),
        focused_pane_id: Some("pane-1".into()),
        spaces: vec![],
        tabs: vec![TabSummary {
            id: "tab-1".into(),
            space_id: "space-1".into(),
            label: "Main".into(),
            number: 1,
            pane_count: 1,
            focused: true,
        }],
        panes: vec![PaneSummary {
            id: "pane-1".into(),
            terminal_id: "terminal-1".into(),
            space_id: "space-1".into(),
            tab_id: "tab-1".into(),
            title: Some("Shell".into()),
            focused: true,
            agent: None,
            agent_status: "idle".into(),
            revision: 1,
        }],
        layouts: vec![TabLayout {
            space_id: "space-1".into(),
            tab_id: "tab-1".into(),
            area: LayoutRect {
                x: 0,
                y: 0,
                width: 80,
                height: 24,
            },
            focused_pane_id: Some("pane-1".into()),
            panes: vec![LayoutPane {
                pane_id: "pane-1".into(),
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
        agents: vec![],
    }
}

fn empty_subscription() -> SessionSubscription {
    let (_tx, rx) = mpsc::channel(1);
    SessionSubscription { messages: rx }
}

fn empty_terminal() -> TerminalSession {
    let (commands, _command_rx) = mpsc::channel(1);
    let (_message_tx, messages) = mpsc::channel(1);
    TerminalSession {
        stream_id: "stream-1".into(),
        messages,
        commands,
    }
}

fn fake() -> FakeAdapter {
    FakeAdapter {
        inspect_calls: Arc::new(AtomicUsize::new(0)),
        inspect_session_calls: Arc::new(Mutex::new(Vec::new())),
        snapshot_calls: Arc::new(Mutex::new(Vec::new())),
        focus_calls: Arc::new(Mutex::new(Vec::new())),
        mutation_calls: Arc::new(Mutex::new(Vec::new())),
        subscribe_calls: Arc::new(Mutex::new(Vec::new())),
        terminal_calls: Arc::new(Mutex::new(Vec::new())),
        compatibility: Ok(compatible()),
        session_compatibility: Ok(compatible()),
        snapshots: Arc::new(Mutex::new(Ok(snapshot("session-a")))),
        sessions_result: Ok(SessionListResponse {
            sessions: vec![SessionSummary {
                id: "session-a".into(),
                label: "A".into(),
                is_default: true,
                running: true,
            }],
        }),
        focus_result: Ok(FocusResponse {
            session_id: "session-a".into(),
            kind: FocusKind::Pane,
            target_id: "pane-1".into(),
            accepted: true,
        }),
        mutation_result: Ok(ResourceMutationResponse {
            session_id: "session-a".into(),
            snapshot: snapshot("session-a"),
        }),
        subscribe_result: Arc::new(Mutex::new(None)),
        terminal_result: Arc::new(Mutex::new(None)),
    }
}

#[tokio::test]
async fn test_mode_never_calls_adapter() {
    let adapter = fake();
    let calls = adapter.inspect_calls.clone();
    let session_calls = adapter.inspect_session_calls.clone();
    let service = CockpitService::new(CockpitMode::Test, Arc::new(adapter));

    let status = service.status().await;
    assert!(
        matches!(status.herdr, HerdrCompatibility::Unavailable { ref code, .. } if code == "live_inspection_disabled")
    );
    assert!(status.capabilities.terminal_mouse_input);
    assert_eq!(
        service.sessions().await.unwrap_err().code,
        "live_inspection_disabled"
    );
    assert_eq!(
        service
            .session_snapshot("session-a")
            .await
            .unwrap_err()
            .code,
        "live_inspection_disabled"
    );
    assert_eq!(calls.load(Ordering::SeqCst), 0);
    assert!(session_calls.lock().await.is_empty());
}

#[tokio::test]
async fn normal_status_and_sessions_use_installation_compatibility_cache() {
    let adapter = fake();
    let calls = adapter.inspect_calls.clone();
    let service = CockpitService::new(CockpitMode::Normal, Arc::new(adapter));

    let status = service.status().await;
    assert!(matches!(
        status.herdr,
        HerdrCompatibility::Compatible { .. }
    ));
    assert!(status.capabilities.terminal_mouse_input);
    assert!(service.sessions().await.is_ok());
    assert_eq!(calls.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn compatibility_cache_is_separate_per_session() {
    let adapter = fake();
    let session_calls = adapter.inspect_session_calls.clone();
    let snapshots = adapter.snapshots.clone();
    let service = CockpitService::new(CockpitMode::Normal, Arc::new(adapter));

    service.session_snapshot("session-a").await.unwrap();
    service.session_snapshot("session-a").await.unwrap();
    *snapshots.lock().await = Ok(snapshot("session-b"));
    service.session_snapshot("session-b").await.unwrap();

    assert_eq!(
        session_calls.lock().await.as_slice(),
        &[String::from("session-a"), String::from("session-b")]
    );
}

#[tokio::test]
async fn focus_is_validated_gated_and_delegated() {
    let adapter = fake();
    let calls = adapter.focus_calls.clone();
    let service = CockpitService::new(CockpitMode::Normal, Arc::new(adapter));
    let request = FocusRequest {
        kind: FocusKind::Pane,
        target_id: "pane-1".into(),
    };

    let response = service.focus("session-a", &request).await.unwrap();
    assert!(response.accepted);
    assert_eq!(
        calls.lock().await.as_slice(),
        &[(String::from("session-a"), request)]
    );
}
#[tokio::test]
async fn mutation_is_validated_gated_and_delegated() {
    let adapter = fake();
    let calls = adapter.mutation_calls.clone();
    let service = CockpitService::new(CockpitMode::Normal, Arc::new(adapter));
    let request = ResourceMutationRequest::PaneResize {
        pane_id: "pane-1".into(),
        direction: PaneResizeDirection::Right,
        amount: 0.1,
    };

    let response = service.mutate("session-a", &request).await.unwrap();
    assert_eq!(response.session_id, "session-a");
    assert_eq!(response.snapshot.session_id, "session-a");
    assert_eq!(
        calls.lock().await.as_slice(),
        &[(String::from("session-a"), request)]
    );
}

#[tokio::test]
async fn invalid_mutations_are_rejected_before_adapter_calls() {
    let adapter = fake();
    let calls = adapter.mutation_calls.clone();
    let session_calls = adapter.inspect_session_calls.clone();
    let service = CockpitService::new(CockpitMode::Normal, Arc::new(adapter));

    let invalid = [
        ResourceMutationRequest::SpaceMoveBlock {
            space_ids: vec!["space-1".into(), "space-1".into()],
            before_space_id: None,
        },
        ResourceMutationRequest::SpaceRename {
            space_id: "space-1".into(),
            label: " ".into(),
        },
        ResourceMutationRequest::PaneResize {
            pane_id: "pane-1".into(),
            direction: PaneResizeDirection::Right,
            amount: f64::INFINITY,
        },
        ResourceMutationRequest::PaneMove {
            pane_id: "pane-1".into(),
            destination: PaneMoveDestination::NewTab {
                space_id: Some("bad/id".into()),
                label: None,
            },
        },
    ];
    for request in invalid {
        assert!(service.mutate("session-a", &request).await.is_err());
    }

    assert!(calls.lock().await.is_empty());
    assert!(session_calls.lock().await.is_empty());
}

#[tokio::test]
async fn mutation_rejects_mismatched_response_identity() {
    let mut adapter = fake();
    adapter.mutation_result = Ok(ResourceMutationResponse {
        session_id: "session-b".into(),
        snapshot: snapshot("session-a"),
    });
    let service = CockpitService::new(CockpitMode::Normal, Arc::new(adapter));

    let error = service
        .mutate(
            "session-a",
            &ResourceMutationRequest::PaneClose {
                pane_id: "pane-1".into(),
            },
        )
        .await
        .unwrap_err();
    assert_eq!(error.code, "invalid_mutation_response");
}

#[tokio::test]
async fn mutation_rejects_mismatched_snapshot_identity() {
    let mut adapter = fake();
    adapter.mutation_result = Ok(ResourceMutationResponse {
        session_id: "session-a".into(),
        snapshot: snapshot("session-b"),
    });
    let service = CockpitService::new(CockpitMode::Normal, Arc::new(adapter));

    let error = service
        .mutate(
            "session-a",
            &ResourceMutationRequest::PaneClose {
                pane_id: "pane-1".into(),
            },
        )
        .await
        .unwrap_err();
    assert_eq!(error.code, "invalid_session_snapshot");
}

#[tokio::test]
async fn subscription_and_terminal_are_delegated() {
    let adapter = fake();
    let subscriptions = adapter.subscribe_calls.clone();
    let terminals = adapter.terminal_calls.clone();
    let service = CockpitService::new(CockpitMode::Normal, Arc::new(adapter));

    let mut subscription = service
        .subscribe_session("session-a", &snapshot("session-a"))
        .await
        .unwrap();
    let terminal = service
        .open_terminal(&TerminalOpenRequest {
            client_surface_id: "surface-1".into(),
            session_id: "session-a".into(),
            pane_id: "pane-1".into(),
            mode: TerminalMode::Observe,
            takeover: false,
            cols: 80,
            rows: 24,
            cell_width_px: 8,
            cell_height_px: 16,
            surface_cols: 80,
            surface_rows: 24,
        })
        .await
        .unwrap();

    assert!(subscription.messages.try_recv().is_err());
    assert_eq!(terminal.stream_id, "stream-1");
    assert_eq!(
        subscriptions.lock().await.as_slice(),
        &[String::from("session-a")]
    );
    assert_eq!(terminals.lock().await.len(), 1);
}

#[tokio::test]
async fn terminal_rejects_pane_reported_in_a_hidden_tab() {
    let adapter = fake();
    let snapshots = adapter.snapshots.clone();
    let terminals = adapter.terminal_calls.clone();
    let mut hidden = snapshot("session-a");
    hidden.panes[0].tab_id = "tab-2".into();
    *snapshots.lock().await = Ok(hidden);
    let service = CockpitService::new(CockpitMode::Normal, Arc::new(adapter));

    let error = service
        .open_terminal(&TerminalOpenRequest {
            client_surface_id: "surface-1".into(),
            session_id: "session-a".into(),
            pane_id: "pane-1".into(),
            mode: TerminalMode::Observe,
            takeover: false,
            cols: 80,
            rows: 24,
            cell_width_px: 8,
            cell_height_px: 16,
            surface_cols: 80,
            surface_rows: 24,
        })
        .await
        .unwrap_err();

    assert_eq!(
        error,
        InspectionError::new(
            "pane_not_visible",
            "terminal pane is not visible in the focused tab"
        )
    );
    assert!(terminals.lock().await.is_empty());
}

#[tokio::test]
async fn terminal_rejects_missing_pane_before_adapter_launch() {
    let adapter = fake();
    let snapshots = adapter.snapshots.clone();
    let terminals = adapter.terminal_calls.clone();
    let mut missing = snapshot("session-a");
    missing.panes.clear();
    missing.layouts[0].panes.clear();
    *snapshots.lock().await = Ok(missing);
    let service = CockpitService::new(CockpitMode::Normal, Arc::new(adapter));

    let error = service
        .open_terminal(&TerminalOpenRequest {
            client_surface_id: "surface-1".into(),
            session_id: "session-a".into(),
            pane_id: "pane-1".into(),
            mode: TerminalMode::Observe,
            takeover: false,
            cols: 80,
            rows: 24,
            cell_width_px: 8,
            cell_height_px: 16,
            surface_cols: 80,
            surface_rows: 24,
        })
        .await
        .unwrap_err();

    assert_eq!(error.code, "pane_not_visible");
    assert!(terminals.lock().await.is_empty());
}

#[tokio::test]
async fn adapter_errors_propagate_without_remapping() {
    let mut adapter = fake();
    let expected = InspectionError::new("focus_failed", "focus was rejected");
    adapter.focus_result = Err(expected.clone());
    let service = CockpitService::new(CockpitMode::Normal, Arc::new(adapter));
    let error = service
        .focus(
            "session-a",
            &FocusRequest {
                kind: FocusKind::Pane,
                target_id: "pane-1".into(),
            },
        )
        .await
        .unwrap_err();
    assert_eq!(error, expected);
}

#[tokio::test]
async fn invalid_requests_are_rejected_before_adapter_calls() {
    let adapter = fake();
    let session_calls = adapter.inspect_session_calls.clone();
    let terminals = adapter.terminal_calls.clone();
    let service = CockpitService::new(CockpitMode::Normal, Arc::new(adapter));

    assert_eq!(
        service.session_snapshot("bad/id").await.unwrap_err().code,
        "invalid_session_id"
    );
    assert_eq!(
        service
            .open_terminal(&TerminalOpenRequest {
                client_surface_id: "surface-1".into(),
                session_id: "session-a".into(),
                pane_id: "pane-1".into(),
                mode: TerminalMode::Observe,
                takeover: false,
                cols: 0,
                rows: 24,
                cell_width_px: 8,
                cell_height_px: 16,
                surface_cols: 80,
                surface_rows: 24,
            })
            .await
            .unwrap_err()
            .code,
        "invalid_terminal_dimensions"
    );
    assert!(session_calls.lock().await.is_empty());
    assert!(terminals.lock().await.is_empty());
}

#[tokio::test]
async fn incompatible_session_blocks_operation_and_is_not_cached() {
    let mut adapter = fake();
    adapter.session_compatibility = Ok(HerdrCompatibility::Incompatible {
        identity: None,
        code: "herdr_incompatible".into(),
        message: "unsupported Herdr".into(),
    });
    let session_calls = adapter.inspect_session_calls.clone();
    let service = CockpitService::new(CockpitMode::Normal, Arc::new(adapter));

    let first = service.session_snapshot("session-a").await.unwrap_err();
    let second = service.session_snapshot("session-a").await.unwrap_err();
    assert_eq!(
        first,
        InspectionError::new("herdr_incompatible", "unsupported Herdr")
    );
    assert_eq!(second, first);
    assert_eq!(session_calls.lock().await.len(), 2);
}

#[test]
fn stream_message_types_are_available_to_transport_owners() {
    let _ = SessionChange::Changed;
    let _ = TerminalOwnershipState::Observing;
    let _ = TerminalCommand::Release;
    let _ = TerminalStreamMessage::Closed {
        session_id: "session-a".into(),
        pane_id: "pane-1".into(),
        stream_id: "stream-1".into(),
        reason: "closed".into(),
    };
}
