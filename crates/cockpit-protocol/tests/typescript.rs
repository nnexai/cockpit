use std::fs;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};

use cockpit_protocol::typescript::{check, render_v1, write_atomic};
use cockpit_protocol::v1::{
    AgentSummary, CockpitCapabilities, CockpitMode, FocusKind, FocusRequest, FocusResponse,
    HerdrCompatibility, HerdrIdentity, LayoutPane, LayoutRect, PaneMoveDestination,
    PaneOutputResponse, PaneResizeDirection, PaneSplitDirection, PaneSummary, PaneZoomMode,
    ResourceMutationRequest, SessionListResponse, SessionSnapshotResponse, SessionStreamMessage,
    SessionSummary, SpaceGitSummary, SpaceSummary, StatusResponse, TabLayout, TabSummary,
    TerminalCommand, TerminalMode, TerminalOpenRequest, TerminalScrollDirection,
    TerminalScrollSource, TerminalStreamMessage,
};
use serde_json::json;

static TEMP_ID: AtomicU64 = AtomicU64::new(0);

fn temporary_target() -> (PathBuf, PathBuf) {
    let id = TEMP_ID.fetch_add(1, Ordering::Relaxed);
    let directory = std::env::temp_dir().join(format!(
        "cockpit-protocol-typescript-{}-{id}",
        std::process::id()
    ));
    fs::create_dir(&directory).expect("temporary directory created");
    let target = directory.join("v1.ts");
    (directory, target)
}

fn remove_temporary_directory(directory: PathBuf) {
    fs::remove_dir_all(directory).expect("temporary directory removed");
}

#[test]
fn compatibility_uses_stable_status_tag() {
    let response = StatusResponse {
        protocol_version: "1".to_owned(),
        cockpit_version: "0.1.0".to_owned(),
        mode: CockpitMode::Test,
        capabilities: CockpitCapabilities {
            terminal_mouse_input: false,
        },
        herdr: HerdrCompatibility::Unavailable {
            code: "live_inspection_disabled".to_owned(),
            message: "live Herdr inspection is disabled in test mode".to_owned(),
        },
    };

    assert_eq!(
        serde_json::to_value(response).expect("status serializes"),
        json!({
            "protocol_version": "1",
            "cockpit_version": "0.1.0",
            "mode": "test",
            "capabilities": {
                "terminal_mouse_input": false
            },
            "herdr": {
                "status": "unavailable",
                "code": "live_inspection_disabled",
                "message": "live Herdr inspection is disabled in test mode"
            }
        })
    );
}

#[test]
fn legacy_status_defaults_capabilities() {
    let response: StatusResponse = serde_json::from_value(json!({
        "protocol_version": "1",
        "cockpit_version": "0.1.0",
        "mode": "normal",
        "herdr": {
            "status": "unavailable",
            "code": "test",
            "message": "test"
        }
    }))
    .expect("legacy status parses");

    assert!(!response.capabilities.terminal_mouse_input);
}

#[test]
fn session_dtos_use_exact_snake_case_wire_fields() {
    let rect = LayoutRect {
        x: 1,
        y: 2,
        width: 80,
        height: 24,
    };
    let response = SessionSnapshotResponse {
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
            agent_status: "working".to_owned(),
            git: Some(SpaceGitSummary {
                repository_key: "repo-opaque".to_owned(),
                repository: "cockpit".to_owned(),
                branch: Some("main".to_owned()),
                checkout_path: "/work/cockpit".to_owned(),
                is_linked_worktree: false,
            }),
        }],
        tabs: vec![TabSummary {
            id: "tab-1".to_owned(),
            space_id: "space-1".to_owned(),
            label: "Work".to_owned(),
            number: 1,
            pane_count: 1,
            focused: true,
        }],
        panes: vec![PaneSummary {
            id: "pane-1".to_owned(),
            terminal_id: "term-1".to_owned(),
            space_id: "space-1".to_owned(),
            tab_id: "tab-1".to_owned(),
            title: Some("Shell".to_owned()),
            focused: true,
            agent: Some("builder".to_owned()),
            agent_status: "running".to_owned(),
            revision: 7,
        }],
        layouts: vec![TabLayout {
            space_id: "space-1".to_owned(),
            tab_id: "tab-1".to_owned(),
            area: rect.clone(),
            focused_pane_id: Some("pane-1".to_owned()),
            panes: vec![LayoutPane {
                pane_id: "pane-1".to_owned(),
                focused: true,
                rect,
            }],
            zoomed: false,
        }],
        agents: vec![AgentSummary {
            pane_id: "pane-1".to_owned(),
            space_id: "space-1".to_owned(),
            tab_id: "tab-1".to_owned(),
            name: "builder".to_owned(),
            status: "running".to_owned(),
            title: None,
            focused: true,
            state_change_seq: 42,
        }],
    };

    assert_eq!(
        serde_json::to_value(response).expect("session serializes"),
        json!({
            "session_id": "session-1",
            "version": "0.8.2",
            "protocol": 20,
            "focused_space_id": "space-1",
            "focused_tab_id": "tab-1",
            "focused_pane_id": "pane-1",
            "spaces": [{
                "id": "space-1",
                "label": "Main",
                "number": 1,
                "tab_count": 1,
                "pane_count": 1,
                "focused": true,
                "agent_status": "working",
                "git": {
                    "repository_key": "repo-opaque",
                    "repository": "cockpit",
                    "branch": "main",
                    "checkout_path": "/work/cockpit",
                    "is_linked_worktree": false
                }
            }],
            "tabs": [{
                "id": "tab-1",
                "space_id": "space-1",
                "label": "Work",
                "number": 1,
                "pane_count": 1,
                "focused": true
            }],
            "panes": [{
                "id": "pane-1",
                "terminal_id": "term-1",
                "space_id": "space-1",
                "tab_id": "tab-1",
                "title": "Shell",
                "focused": true,
                "agent": "builder",
                "agent_status": "running",
                "revision": 7
            }],
            "layouts": [{
                "space_id": "space-1",
                "tab_id": "tab-1",
                "area": {"x": 1, "y": 2, "width": 80, "height": 24},
                "focused_pane_id": "pane-1",
                "panes": [{
                    "pane_id": "pane-1",
                    "focused": true,
                    "rect": {"x": 1, "y": 2, "width": 80, "height": 24}
                }],
                "zoomed": false
            }],
            "agents": [{
                "pane_id": "pane-1",
                "space_id": "space-1",
                "tab_id": "tab-1",
                "name": "builder",
                "status": "running",
                "title": null,
                "focused": true,
                "state_change_seq": 42
            }]
        })
    );

    let output = PaneOutputResponse {
        pane_id: "pane-1".to_owned(),
        text: "hello\n".to_owned(),
        revision: Some(7),
    };
    assert_eq!(
        serde_json::to_value(output).expect("pane output serializes"),
        json!({"pane_id": "pane-1", "text": "hello\n", "revision": 7})
    );
}
#[test]
fn session_and_terminal_contracts_use_exact_wire_tags() {
    let session = SessionListResponse {
        sessions: vec![SessionSummary {
            id: "session-1".to_owned(),
            label: "Main".to_owned(),
            is_default: true,
            running: true,
        }],
    };
    assert_eq!(
        serde_json::to_value(session).expect("session list serializes"),
        json!({
            "sessions": [{
                "id": "session-1",
                "label": "Main",
                "is_default": true,
                "running": true
            }]
        })
    );

    let focus = FocusRequest {
        kind: FocusKind::Pane,
        target_id: "pane-1".to_owned(),
    };
    assert_eq!(
        serde_json::to_value(focus).expect("focus request serializes"),
        json!({"kind": "pane", "target_id": "pane-1"})
    );
    let focus_response = FocusResponse {
        session_id: "session-1".to_owned(),
        kind: FocusKind::Agent,
        target_id: "agent-1".to_owned(),
        accepted: true,
    };
    assert_eq!(
        serde_json::to_value(focus_response).expect("focus response serializes"),
        json!({
            "session_id": "session-1",
            "kind": "agent",
            "target_id": "agent-1",
            "accepted": true
        })
    );

    let session_event = SessionStreamMessage::Stale {
        session_id: "session-1".to_owned(),
        generation: 3,
        sequence: 8,
        code: "sequence_gap".to_owned(),
        message: "resync required".to_owned(),
    };
    assert_eq!(
        serde_json::to_value(session_event).expect("session event serializes"),
        json!({
            "type": "stale",
            "session_id": "session-1",
            "generation": 3,
            "sequence": 8,
            "code": "sequence_gap",
            "message": "resync required"
        })
    );

    let open = TerminalOpenRequest {
        session_id: "session-1".to_owned(),
        pane_id: "pane-1".to_owned(),
        mode: TerminalMode::Control,
        takeover: true,
        cols: 120,
        rows: 40,
        cell_width_px: 8,
        cell_height_px: 16,
    };
    assert_eq!(
        serde_json::to_value(open).expect("terminal open serializes"),
        json!({
            "session_id": "session-1",
            "pane_id": "pane-1",
            "mode": "control",
            "takeover": true,
            "cols": 120,
            "rows": 40,
            "cell_width_px": 8,
            "cell_height_px": 16
        })
    );
    assert!(
        serde_json::from_value::<TerminalOpenRequest>(json!({
            "session_id": "session-1",
            "pane_id": "pane-1",
            "mode": "control",
            "takeover": true,
            "cols": 120,
            "rows": 40,
            "cell_width_px": 8,
            "cell_height_px": 16,
            "client_surface_id": "obsolete"
        }))
        .is_err()
    );

    let text = TerminalCommand::input_text("hello");
    assert_eq!(
        serde_json::to_value(text).expect("text input serializes"),
        json!({"type": "terminal.input", "text": "hello", "bytes": null})
    );
    let bytes = TerminalCommand::input_bytes("aGVsbG8=");
    assert_eq!(
        serde_json::to_value(bytes).expect("bytes input serializes"),
        json!({"type": "terminal.input", "text": null, "bytes": "aGVsbG8="})
    );
    let resize = TerminalCommand::Resize {
        cols: 120,
        rows: 40,
        cell_width_px: 8,
        cell_height_px: 16,
    };
    assert_eq!(
        serde_json::to_value(resize).expect("resize serializes"),
        json!({
            "type": "terminal.resize",
            "cols": 120,
            "rows": 40,
            "cell_width_px": 8,
            "cell_height_px": 16
        })
    );
    let scroll = TerminalCommand::Scroll {
        direction: TerminalScrollDirection::Down,
        lines: 5,
        source: TerminalScrollSource::PageKey,
        column: Some(2),
        row: Some(3),
        modifiers: 1,
    };
    assert_eq!(
        serde_json::to_value(scroll).expect("scroll serializes"),
        json!({
            "type": "terminal.scroll",
            "direction": "down",
            "lines": 5,
            "source": "page_key",
            "column": 2,
            "row": 3,
            "modifiers": 1
        })
    );
    assert_eq!(
        serde_json::to_value(TerminalCommand::Release).expect("release serializes"),
        json!({"type": "terminal.release"})
    );

    let terminal_event = TerminalStreamMessage::Frame {
        session_id: "session-1".to_owned(),
        pane_id: "pane-1".to_owned(),
        stream_id: "stream-1".to_owned(),
        seq: "18446744073709551616".to_owned(),
        encoding: "base64".to_owned(),
        width: 120,
        height: 40,
        full: true,
        bytes: "aGVsbG8=".to_owned(),
    };
    assert_eq!(
        serde_json::to_value(terminal_event).expect("terminal event serializes"),
        json!({
            "type": "frame",
            "session_id": "session-1",
            "pane_id": "pane-1",
            "stream_id": "stream-1",
            "seq": "18446744073709551616",
            "encoding": "base64",
            "width": 120,
            "height": 40,
            "full": true,
            "bytes": "aGVsbG8="
        })
    );
}

#[test]
fn terminal_input_requires_exactly_one_payload() {
    let invalid = TerminalCommand::Input {
        text: None,
        bytes: None,
    };
    assert!(serde_json::to_value(invalid).is_err());
    let invalid = TerminalCommand::Input {
        text: Some("text".to_owned()),
        bytes: Some("bytes".to_owned()),
    };
    assert!(serde_json::to_value(invalid).is_err());
    assert!(
        serde_json::from_value::<TerminalCommand>(json!({
            "type": "terminal.input",
            "text": "text",
            "bytes": null
        }))
        .is_ok()
    );
    assert!(
        serde_json::from_value::<TerminalCommand>(json!({
            "type": "terminal.input",
            "text": "text",
            "bytes": "bytes"
        }))
        .is_err()
    );
}

#[test]
fn terminal_commands_enforce_bounded_payloads() {
    let max_text = TerminalCommand::input_text("a".repeat(64 * 1024));
    assert!(serde_json::to_value(max_text).is_ok());
    let oversized_text = TerminalCommand::input_text("a".repeat(64 * 1024 + 1));
    assert!(serde_json::to_value(oversized_text).is_err());

    let max_bytes = TerminalCommand::input_bytes("a".repeat(87_384));
    assert!(serde_json::to_value(max_bytes).is_ok());
    let oversized_bytes = TerminalCommand::input_bytes("a".repeat(87_385));
    assert!(serde_json::to_value(oversized_bytes).is_err());

    let max_resize = TerminalCommand::Resize {
        cols: u16::MAX,
        rows: u16::MAX,
        cell_width_px: u16::MAX as u32,
        cell_height_px: 0,
    };
    assert!(serde_json::to_value(max_resize).is_ok());
    assert!(
        serde_json::from_value::<TerminalCommand>(json!({
            "type": "terminal.resize",
            "cols": 1,
            "rows": 1,
            "cell_width_px": 65535,
            "cell_height_px": 0
        }))
        .is_ok()
    );
    assert!(
        serde_json::from_value::<TerminalCommand>(json!({
            "type": "terminal.resize",
            "cols": 0,
            "rows": 1,
            "cell_width_px": 0,
            "cell_height_px": 0
        }))
        .is_err()
    );
    assert!(
        serde_json::from_value::<TerminalCommand>(json!({
            "type": "terminal.resize",
            "cols": 1,
            "rows": 1,
            "cell_width_px": 65536,
            "cell_height_px": 0
        }))
        .is_err()
    );

    let max_scroll = TerminalCommand::Scroll {
        direction: TerminalScrollDirection::Up,
        lines: u16::MAX as u32,
        source: TerminalScrollSource::Wheel,
        column: None,
        row: None,
        modifiers: 0,
    };
    assert!(serde_json::to_value(max_scroll).is_ok());
    for lines in [0, 65_536] {
        assert!(
            serde_json::from_value::<TerminalCommand>(json!({
                "type": "terminal.scroll",
                "direction": "up",
                "lines": lines,
                "source": "wheel",
                "column": null,
                "row": null,
                "modifiers": 0
            }))
            .is_err()
        );
    }
}

#[test]
fn resource_mutations_use_a_closed_snake_case_contract() {
    let mutations = [
        ResourceMutationRequest::SpaceCreate {
            cwd: Some("/work".into()),
            label: Some("Work".into()),
        },
        ResourceMutationRequest::SpaceRename {
            space_id: "space-1".into(),
            label: "New".into(),
        },
        ResourceMutationRequest::SpaceMoveBlock {
            space_ids: vec!["space-1".into()],
            before_space_id: Some("space-2".into()),
        },
        ResourceMutationRequest::SpaceClose {
            space_id: "space-1".into(),
        },
        ResourceMutationRequest::TabCreate {
            space_id: "space-1".into(),
            label: None,
        },
        ResourceMutationRequest::TabRename {
            tab_id: "tab-1".into(),
            label: "New".into(),
        },
        ResourceMutationRequest::TabMove {
            tab_id: "tab-1".into(),
            insert_index: 2,
        },
        ResourceMutationRequest::TabClose {
            tab_id: "tab-1".into(),
        },
        ResourceMutationRequest::PaneSplit {
            pane_id: "pane-1".into(),
            direction: PaneSplitDirection::Right,
            ratio: Some(0.4),
        },
        ResourceMutationRequest::PaneResize {
            pane_id: "pane-1".into(),
            direction: PaneResizeDirection::Left,
            amount: 0.1,
        },
        ResourceMutationRequest::PaneRename {
            pane_id: "pane-1".into(),
            label: None,
        },
        ResourceMutationRequest::PaneSwap {
            source_pane_id: "pane-1".into(),
            target_pane_id: "pane-2".into(),
        },
        ResourceMutationRequest::PaneMove {
            pane_id: "pane-1".into(),
            destination: PaneMoveDestination::ExistingTab {
                tab_id: "tab-2".into(),
                direction: PaneSplitDirection::Down,
                target_pane_id: Some("pane-2".into()),
                ratio: None,
            },
        },
        ResourceMutationRequest::PaneZoom {
            pane_id: "pane-1".into(),
            mode: PaneZoomMode::On,
        },
        ResourceMutationRequest::PaneClose {
            pane_id: "pane-1".into(),
        },
    ];
    let expected_types = [
        "space_create",
        "space_rename",
        "space_move_block",
        "space_close",
        "tab_create",
        "tab_rename",
        "tab_move",
        "tab_close",
        "pane_split",
        "pane_resize",
        "pane_rename",
        "pane_swap",
        "pane_move",
        "pane_zoom",
        "pane_close",
    ];

    for (mutation, expected_type) in mutations.into_iter().zip(expected_types) {
        let encoded = serde_json::to_value(&mutation).expect("mutation serializes");
        assert_eq!(encoded["type"], expected_type);
        assert_eq!(
            serde_json::from_value::<ResourceMutationRequest>(encoded).unwrap(),
            mutation
        );
    }
    assert!(
        serde_json::from_value::<ResourceMutationRequest>(
            json!({"type": "raw", "method": "layout.apply", "params": {}})
        )
        .is_err()
    );
}

#[test]
fn pane_move_destinations_preserve_space_terminology() {
    let destinations = [
        PaneMoveDestination::NewTab {
            space_id: Some("space-1".into()),
            label: Some("Tab".into()),
        },
        PaneMoveDestination::NewSpace {
            label: Some("Space".into()),
            tab_label: Some("Tab".into()),
        },
    ];
    assert_eq!(
        serde_json::to_value(&destinations[0]).unwrap(),
        json!({"type": "new_tab", "space_id": "space-1", "label": "Tab"})
    );
    assert_eq!(
        serde_json::to_value(&destinations[1]).unwrap(),
        json!({"type": "new_space", "label": "Space", "tab_label": "Tab"})
    );
}

#[test]
fn check_accepts_exact_generated_bytes() {
    let (directory, target) = temporary_target();
    let expected = render_v1();
    write_atomic(&target, expected.as_bytes()).expect("generated file written");
    assert!(check(&target).expect("generated file checked"));
    assert_eq!(
        fs::read(&target).expect("generated file read"),
        expected.as_bytes()
    );
    remove_temporary_directory(directory);
}

#[test]
fn check_detects_drift_without_rewriting() {
    let (directory, target) = temporary_target();
    let mut drifted = render_v1().into_bytes();
    let index = drifted.len() - 2;
    drifted[index] = b'X';
    fs::write(&target, &drifted).expect("drifted file written");

    assert!(!check(&target).expect("drifted file checked"));
    assert_eq!(fs::read(&target).expect("drifted file read"), drifted);
    remove_temporary_directory(directory);
}

#[test]
fn check_reports_missing_file_as_drift() {
    let (directory, target) = temporary_target();
    assert!(!check(&target).expect("missing file checked"));
    assert!(!target.exists());
    remove_temporary_directory(directory);
}
#[test]
fn compatible_status_round_trips() {
    let compatibility = HerdrCompatibility::Compatible {
        identity: HerdrIdentity {
            version: "0.8.2".to_owned(),
            protocol: 20,
            schema_version: 1,
        },
    };
    let encoded = serde_json::to_string(&compatibility).expect("compatibility serializes");
    let decoded: HerdrCompatibility = serde_json::from_str(&encoded).expect("compatibility parses");
    assert_eq!(decoded, compatibility);
}

#[test]
fn typescript_rendering_is_deterministic() {
    let rendered = render_v1();
    assert_eq!(rendered, render_v1());
    for type_name in [
        "CockpitMode",
        "HerdrIdentity",
        "HerdrCompatibility",
        "CockpitCapabilities",
        "StatusResponse",
        "ErrorResponse",
        "SpaceGitSummary",
        "SpaceGitStatus",
        "SpaceGitStatusResponse",
        "SpaceSummary",
        "TabSummary",
        "PaneSummary",
        "AgentSummary",
        "LayoutRect",
        "LayoutPane",
        "TabLayout",
        "SessionSnapshotResponse",
        "PaneOutputResponse",
        "SessionSummary",
        "SessionListResponse",
        "FocusKind",
        "FocusRequest",
        "FocusResponse",
        "PaneSplitDirection",
        "PaneResizeDirection",
        "PaneZoomMode",
        "PaneMoveDestination",
        "ResourceMutationRequest",
        "ResourceMutationResponse",
        "SessionStreamMessage",
        "TerminalMode",
        "TerminalOpenRequest",
        "TerminalScrollDirection",
        "TerminalScrollSource",
        "TerminalCommand",
        "TerminalOwnershipState",
        "TerminalStreamMessage",
    ] {
        assert!(
            rendered.contains(&format!("export type {type_name}")),
            "missing generated declaration for {type_name}"
        );
    }
}
