use super::*;
use cockpit_protocol::v1::{PaneSummary, SessionSnapshotResponse, SpaceSummary};
use cockpit_protocol::projects::{BindMovedSpaceRequest, DirectoryCheckoutRequest, ProjectConfiguration};
use cockpit_protocol::orchestration::{AttachmentLaunchMode, CleanupAuthority, CleanupItem, CleanupState,
    OwnedResourceKind, ResourceOwnership, RetirementIdentity, RetirementState, RetirementTrigger,
    RunRetirement, TerminalOutcome};
use crate::HerdrAdapter;
use crate::library::LibraryService;
use crate::projects::test_support::NestedDirectoryAdapter;
use crate::sources::SourceService;
use std::{path::PathBuf, sync::{Arc, atomic::Ordering}};

fn identity() -> OwnedSpaceIdentity {
    OwnedSpaceIdentity {
        creation_operation_id: "creation-operation".into(),
        endpoint_identity: "unix-socket:/fixture/herdr.sock:pid=101:uid=1000:gid=1000:start=900".into(),
        session_id: "fixture".into(), workspace_id: "owned-space".into(), checkout_path: "/fixture/checkout".into(),
    }
}

fn native_peer() -> NativeProcessIdentity {
    NativeProcessIdentity {
        pid: 101, start_ticks: 900,
        kernel_boot_id: Some("3c050d8a-d994-47c2-bd34-f4249f746f22".into()),
    }
}
fn observation(identity: &OwnedSpaceIdentity) -> ProjectSnapshot {
    ProjectSnapshot {
        endpoint_identity: identity.endpoint_identity.clone(), boot_id: None, peer_process: Some(native_peer()),
        snapshot: SessionSnapshotResponse {
            session_id: identity.session_id.clone(), server_instance: "peer-derived-hash-not-a-boot".into(),
            version: "test".into(), protocol: 1, herdr_shell: None,
            focused_space_id: None, focused_tab_id: None, focused_pane_id: None,
            spaces: vec![], tabs: vec![], panes: vec![], agents: vec![],
        },
    }
}

#[test]
fn absent_space_requires_recorded_matching_physical_peer_and_bound_response() {
    let identity = identity();
    let peer = native_peer();
    let captured = SpaceIncarnation { boot: None, peer: Some(&peer) };
    let observed = observation(&identity);
    assert!(fresh_owned_space_absence(&identity, captured, &observed).unwrap());
    let mut foreign_endpoint = observation(&identity);
    foreign_endpoint.endpoint_identity = "foreign-endpoint".into();
    assert_eq!(fresh_owned_space_absence(&identity, captured, &foreign_endpoint).unwrap_err().code, "endpoint_changed");
    let mut foreign_session = observation(&identity);
    foreign_session.snapshot.session_id = "another-session".into();
    assert_eq!(fresh_owned_space_absence(&identity, captured, &foreign_session).unwrap_err().code, "endpoint_changed");
    let mut changed_pid = peer.clone();
    changed_pid.pid = 202;
    let mut changed_start = peer.clone();
    changed_start.start_ticks = 901;
    let mut changed_kernel = peer.clone();
    changed_kernel.kernel_boot_id = Some("8d161f3a-9a82-4509-8e58-83bf5cdd77f9".into());
    for changed in [changed_pid, changed_start, changed_kernel] {
        let mut observed = observation(&identity);
        observed.peer_process = Some(changed);
        assert_eq!(fresh_owned_space_absence(&identity, captured, &observed).unwrap_err().code, "space_peer_changed");
    }
}

#[test]
fn legacy_raw_boot_proves_owned_absence_but_conflicting_known_pairs_fail() {
    let identity = identity();
    let mut observed = observation(&identity);
    observed.boot_id = Some("actual-herdr-boot".into());
    observed.peer_process = None;
    let captured = SpaceIncarnation { boot: Some("actual-herdr-boot"), peer: None };
    assert!(fresh_owned_space_absence(&identity, captured, &observed).unwrap());
    observed.boot_id = Some("another-herdr-boot".into());
    assert_eq!(fresh_owned_space_absence(&identity, captured, &observed).unwrap_err().code, "space_boot_changed");

    let peer = native_peer();
    let captured = SpaceIncarnation { boot: Some("actual-herdr-boot"), peer: Some(&peer) };
    observed.peer_process = Some(peer.clone());
    assert_eq!(fresh_owned_space_absence(&identity, captured, &observed).unwrap_err().code, "space_boot_changed");
    observed.boot_id = Some("actual-herdr-boot".into());
    observed.peer_process.as_mut().unwrap().pid = 202;
    assert_eq!(fresh_owned_space_absence(&identity, captured, &observed).unwrap_err().code, "space_peer_changed");
}

#[test]
fn absent_raw_boot_never_implies_same_boot_or_retrospectively_records_fresh_peer() {
    let identity = identity();
    let mut observed = observation(&identity);
    observed.snapshot.server_instance = "actual-herdr-boot".into();
    for boot in [None, Some("")] {
        assert_eq!(captured_space_incarnation(&identity, &identity.session_id, &identity.endpoint_identity,
            &identity.workspace_id, boot, None).err().unwrap().code, "space_incarnation_unproven");
        assert_eq!(fresh_owned_space_absence(&identity, SpaceIncarnation { boot, peer: None }, &observed)
            .unwrap_err().code, "space_incarnation_unproven");
    }
    observed.peer_process = None;
    for boot in [None, Some(String::new())] {
        observed.boot_id = boot;
        assert_eq!(fresh_owned_space_absence(&identity,
            SpaceIncarnation { boot: Some("actual-herdr-boot"), peer: None }, &observed)
            .unwrap_err().code, "space_incarnation_unproven");
    }
}

#[test]
fn incomplete_native_peers_never_prove_captured_or_fresh_incarnation() {
    let identity = identity();
    let peer = native_peer();
    let mut zero_pid = peer.clone();
    zero_pid.pid = 0;
    let mut zero_start = peer.clone();
    zero_start.start_ticks = 0;
    let mut missing_kernel = peer.clone();
    missing_kernel.kernel_boot_id = None;
    let mut invalid_kernel = peer.clone();
    invalid_kernel.kernel_boot_id = Some("not-a-kernel-boot-uuid".into());
    let mut empty_kernel = peer.clone();
    empty_kernel.kernel_boot_id = Some(String::new());
    for invalid in [None, Some(zero_pid), Some(zero_start), Some(missing_kernel), Some(invalid_kernel), Some(empty_kernel)] {
        assert_eq!(captured_space_incarnation(&identity, &identity.session_id, &identity.endpoint_identity,
            &identity.workspace_id, None, invalid.as_ref()).err().unwrap().code, "space_incarnation_unproven");
        let mut observed = observation(&identity);
        observed.peer_process = invalid;
        assert_eq!(fresh_owned_space_absence(&identity,
            SpaceIncarnation { boot: None, peer: Some(&peer) }, &observed).unwrap_err().code, "space_incarnation_unproven");
        assert_eq!(current_physical_snapshot(&observed, &identity.session_id, &identity.endpoint_identity)
            .unwrap_err().code, "terminal_peer_unproven");
    }
}

#[test]
fn captured_placement_must_match_exact_owned_space_and_retain_recorded_incarnation() {
    let identity = identity();
    let peer = native_peer();
    let captured = captured_space_incarnation(&identity, &identity.session_id, &identity.endpoint_identity,
        &identity.workspace_id, None, Some(&peer)).unwrap();
    assert_eq!(captured.boot, None);
    assert_eq!(captured.peer, Some(&peer));
    let legacy = captured_space_incarnation(&identity, &identity.session_id, &identity.endpoint_identity,
        &identity.workspace_id, Some("actual-herdr-boot"), None).unwrap();
    assert_eq!(legacy.boot, Some("actual-herdr-boot"));
    assert_eq!(legacy.peer, None);
    for (session, endpoint, workspace) in [
        ("foreign-session", identity.endpoint_identity.as_str(), identity.workspace_id.as_str()),
        (identity.session_id.as_str(), "foreign-endpoint", identity.workspace_id.as_str()),
        (identity.session_id.as_str(), identity.endpoint_identity.as_str(), "foreign-space"),
    ] {
        assert_eq!(captured_space_incarnation(&identity, session, endpoint, workspace, None, Some(&peer))
            .err().unwrap().code, "space_placement_changed");
    }
}

#[test]
fn present_space_or_dangling_consumers_cannot_be_recorded_absent() {
    let identity = identity();
    let peer = native_peer();
    let captured = SpaceIncarnation { boot: None, peer: Some(&peer) };
    let mut observed = observation(&identity);
    observed.snapshot.spaces.push(SpaceSummary { id: identity.workspace_id.clone(), label: "Owned".into(),
        number: 1, tab_count: 0, pane_count: 0, focused: false, agent_status: "none".into(), git: None });
    assert!(!fresh_owned_space_absence(&identity, captured, &observed).unwrap());
    observed.snapshot.spaces.clear();
    observed.snapshot.panes.push(PaneSummary { id: "dangling-pane".into(), terminal_id: "actual-terminal".into(),
        space_id: identity.workspace_id.clone(), tab_id: "dangling-tab".into(), title: None, focused: false,
        agent: None, agent_status: "none".into(), revision: 1, cwd: Some(identity.checkout_path.clone()) });
    assert_eq!(fresh_owned_space_absence(&identity, captured, &observed).unwrap_err().code, "space_snapshot_conflict");
}

#[test]
fn current_git_consumer_census_requires_bound_response_and_complete_physical_peer() {
    let identity = identity();
    let mut observed = observation(&identity);
    assert!(current_physical_snapshot(&observed, &identity.session_id, &identity.endpoint_identity).is_ok());
    observed.endpoint_identity = "foreign-endpoint".into();
    assert_eq!(current_physical_snapshot(&observed, &identity.session_id, &identity.endpoint_identity)
        .unwrap_err().code, "endpoint_changed");
    observed.endpoint_identity = identity.endpoint_identity.clone();
    observed.snapshot.session_id = "foreign-session".into();
    assert_eq!(current_physical_snapshot(&observed, &identity.session_id, &identity.endpoint_identity)
        .unwrap_err().code, "endpoint_changed");
    observed.snapshot.session_id = identity.session_id.clone();
    observed.peer_process = None;
    for boot in [None, Some(String::new()), Some("actual-herdr-boot".into())] {
        observed.boot_id = boot;
        assert!(response_bound_snapshot(&observed, &identity.session_id, &identity.endpoint_identity).is_ok());
        assert_eq!(current_physical_snapshot(&observed, &identity.session_id, &identity.endpoint_identity)
            .unwrap_err().code, "terminal_peer_unproven");
    }
}

struct SpaceFixture {
    root: PathBuf,
    service: ProjectService,
    adapter: Arc<NestedDirectoryAdapter>,
    run: Run,
    resource: OwnedResourceRecord,
}
impl SpaceFixture {
    async fn new() -> Self {
        let root = std::env::temp_dir().join(format!("cockpit-space-incarnation-{}", uuid::Uuid::new_v4()));
        let source = root.join("source");
        std::fs::create_dir_all(&source).unwrap();
        let config = ProjectConfiguration::for_tests(&root);
        let adapter = Arc::new(NestedDirectoryAdapter::default());
        adapter.selection_available.store(true, Ordering::SeqCst);
        let source_snapshot = adapter.session_snapshot("session").await.unwrap();
        *adapter.bound_snapshot.lock() = Some(ProjectSnapshot {
            endpoint_identity: "endpoint".into(), boot_id: None, peer_process: Some(native_peer()),
            snapshot: source_snapshot,
        });
        let service = ProjectService::new(config.clone(), adapter.clone()).unwrap();
        let library = LibraryService::new(config.clone(), Arc::new(SourceService::new(&config, vec![]).unwrap()))
            .with_herdr(adapter.clone());
        let operation = service.prepare_directory("session", &DirectoryCheckoutRequest {
            operation_id: uuid::Uuid::new_v4().to_string(), checkout_path: source.to_string_lossy().into_owned(),
            source_workspace_id: "workspace".into(),
        }, &library).await.unwrap();
        let mut snapshot = adapter.session_snapshot("session").await.unwrap();
        snapshot.spaces.push(SpaceSummary { id: "owned-space".into(), label: "Actor".into(), number: 2,
            tab_count: 1, pane_count: 1, focused: false, agent_status: "none".into(), git: None });
        snapshot.tabs.push(cockpit_protocol::v1::TabSummary { id: "owned-space:actor".into(), space_id: "owned-space".into(),
            label: "actor".into(), number: 1, pane_count: 1, focused: false, focused_pane_id: Some("owned-space:actor:p1".into()) });
        snapshot.panes.push(PaneSummary { id: "owned-space:actor:p1".into(), terminal_id: "actor-terminal".into(),
            space_id: "owned-space".into(), tab_id: "owned-space:actor".into(), title: None, focused: false,
            agent: None, agent_status: "none".into(), revision: 1, cwd: Some(operation.plan.checkout_path.clone()) });
        *adapter.bound_snapshot.lock() = Some(ProjectSnapshot {
            endpoint_identity: operation.plan.endpoint_identity.clone(), boot_id: None, peer_process: Some(native_peer()),
            snapshot: snapshot.clone(),
        });
        *adapter.snapshot.lock() = Some(snapshot);
        let binding = BindMovedSpaceRequest {
            operation_id: operation.operation_id.clone(), expected_generation: operation.generation,
            endpoint_identity: operation.plan.endpoint_identity.clone(), workspace_id: "owned-space".into(),
            tab_id: "owned-space:actor".into(), pane_id: "owned-space:actor:p1".into(), terminal_id: "actor-terminal".into(),
            move_operation_id: uuid::Uuid::new_v4().to_string(),
        };
        service.bind_moved_space("session", &binding, &library).await.unwrap();
        let receipt = service.store.read_owned_creation(&operation.operation_id).unwrap().unwrap();
        let owned = receipt.resources.iter().find_map(|resource| match resource {
            OwnedResourceIdentity::Space { identity } => Some(identity.clone()), _ => None,
        }).unwrap();
        let run: Run = serde_json::from_value(serde_json::json!({
            "session_id": "session", "brief": "", "run_id": "worker", "kind": "worker", "label": "Actor",
            "root_id": "root", "attempt": 1, "launch_config": {}, "stage": "closed", "annotations": [],
            "setup": { "operation_id": operation.operation_id, "workspace_id": binding.workspace_id,
                "checkout_path": operation.plan.checkout_path, "ownership": "borrowed_directory", "effects": [], "warnings": [] },
            "attachment": { "attachment_id": "attachment", "epoch": 1, "state": "connected", "launch_attempt": 1,
                "location": { "boot_id": null, "peer_process": native_peer(), "terminal_id": binding.terminal_id,
                    "endpoint_identity": binding.endpoint_identity, "session_id": "session", "workspace_id": binding.workspace_id,
                    "tab_id": binding.tab_id, "pane_id": binding.pane_id, "launch_tag": "actor" }, "updated_at": "1" },
            "created_at": "1", "updated_at": "1"
        })).unwrap();
        let resource = OwnedResourceRecord {
            resource_id: "space-resource".into(), root_id: run.root_id.clone(), run_id: run.run_id.clone(),
            kind: OwnedResourceKind::Space, ownership: ResourceOwnership::Owned,
            creation_operation_id: Some(owned.creation_operation_id.clone()),
            identity: OwnedResourceIdentity::Space { identity: owned.clone() }, authority: CleanupAuthority::OwnedWhenSafe,
            consumers: vec![], integration_result_id: None,
            cleanup: CleanupItem { operation_id: None, state: CleanupState::Pending, reason: None, effects: vec![], updated_at: "1".into() },
        };
        let mut bound = observation(&owned);
        bound.snapshot.session_id = "session".into();
        *adapter.bound_snapshot.lock() = Some(bound);
        Self { root, service, adapter, run, resource }
    }
    fn retirement(&self) -> RunRetirement {
        let location = self.run.attachment.as_ref().unwrap().location.as_ref().unwrap();
        RunRetirement {
            retirement_id: "retirement".into(), trigger: RetirementTrigger::Accept, result_message_id: None,
            task_revision: None, replacement: None,
            identity: Some(RetirementIdentity {
                attachment_id: Some("attachment".into()), attachment_epoch: Some(1),
                launch_mode: Some(AttachmentLaunchMode::DirectProcess), run_attempt: 1, launch_attempt: 1,
                launch_tag: location.launch_tag.clone(), endpoint_identity: location.endpoint_identity.clone(),
                session_id: location.session_id.clone(), workspace_id: location.workspace_id.clone(),
                tab_id: location.tab_id.clone(), pane_id: location.pane_id.clone(), terminal_id: "actor-terminal".into(),
                herdr_boot_id: location.boot_id.clone(), herdr_peer: location.peer_process.clone(),
                conversation: None, process: None, shell: None,
            }),
            state: RetirementState::Retired { at: "1".into(), terminal: TerminalOutcome::AlreadyAbsent },
            created_at: "1".into(), updated_at: "1".into(),
        }
    }
}
impl Drop for SpaceFixture { fn drop(&mut self) { let _ = std::fs::remove_dir_all(&self.root); } }

#[tokio::test]
async fn owned_space_inspector_records_physical_absence_without_raw_boot() {
    let mut fixture = SpaceFixture::new().await;
    assert!(matches!(fixture.service.inspect_owned_resource(&fixture.resource, &fixture.run, None).await.unwrap(), CleanupEvaluation::Absent));
    fixture.adapter.bound_snapshot.lock().as_mut().unwrap().endpoint_identity = "foreign-endpoint".into();
    assert!(matches!(fixture.service.inspect_owned_resource(&fixture.resource, &fixture.run, None).await.unwrap(),
        CleanupEvaluation::Unknown(reason) if reason.code == "endpoint_changed"));
    fixture.adapter.bound_snapshot.lock().as_mut().unwrap().endpoint_identity =
        fixture.run.attachment.as_ref().unwrap().location.as_ref().unwrap().endpoint_identity.clone();
    let peer = native_peer();
    let mut changed_pid = peer.clone();
    changed_pid.pid = 202;
    let mut changed_start = peer.clone();
    changed_start.start_ticks = 901;
    let mut changed_kernel = peer.clone();
    changed_kernel.kernel_boot_id = Some("8d161f3a-9a82-4509-8e58-83bf5cdd77f9".into());
    for changed in [changed_pid, changed_start, changed_kernel] {
        fixture.adapter.bound_snapshot.lock().as_mut().unwrap().peer_process = Some(changed);
        assert!(matches!(fixture.service.inspect_owned_resource(&fixture.resource, &fixture.run, None).await.unwrap(),
            CleanupEvaluation::Unknown(reason) if reason.code == "space_peer_changed"));
    }
    fixture.adapter.bound_snapshot.lock().as_mut().unwrap().peer_process = Some(peer);
    fixture.run.attachment.as_mut().unwrap().location.as_mut().unwrap().boot_id = Some("recorded-raw-boot".into());
    fixture.adapter.bound_snapshot.lock().as_mut().unwrap().boot_id = Some("changed-raw-boot".into());
    assert!(matches!(fixture.service.inspect_owned_resource(&fixture.resource, &fixture.run, None).await.unwrap(),
        CleanupEvaluation::Unknown(reason) if reason.code == "space_boot_changed"));
    fixture.run.attachment.as_mut().unwrap().location.as_mut().unwrap().boot_id = None;
    fixture.adapter.bound_snapshot.lock().as_mut().unwrap().boot_id = None;
    fixture.adapter.bound_snapshot.lock().as_mut().unwrap().peer_process = None;
    assert!(matches!(fixture.service.inspect_owned_resource(&fixture.resource, &fixture.run, None).await.unwrap(),
        CleanupEvaluation::Unknown(reason) if reason.code == "space_incarnation_unproven"));
    *fixture.adapter.bound_snapshot.lock() = None;
    assert!(matches!(fixture.service.inspect_owned_resource(&fixture.resource, &fixture.run, None).await.unwrap(),
        CleanupEvaluation::Unknown(reason) if reason.code == "test_adapter_unused"));
}

#[tokio::test]
async fn owned_space_inspector_never_backfills_unrecorded_peer_from_fresh_observation() {
    let mut fixture = SpaceFixture::new().await;
    fixture.run.attachment.as_mut().unwrap().location.as_mut().unwrap().peer_process = None;
    assert!(matches!(fixture.service.inspect_owned_resource(&fixture.resource, &fixture.run, None).await.unwrap(),
        CleanupEvaluation::Unknown(reason) if reason.code == "space_incarnation_unproven"));
    assert!(fixture.run.attachment.as_ref().unwrap().location.as_ref().unwrap().peer_process.is_none());
}

#[tokio::test]
async fn retirement_peer_is_immutable_authority_and_not_replaced_by_new_attachment_peer() {
    let mut fixture = SpaceFixture::new().await;
    fixture.run.retirement = Some(fixture.retirement());
    assert!(matches!(fixture.service.inspect_owned_resource(&fixture.resource, &fixture.run, None).await.unwrap(), CleanupEvaluation::Absent));
    let mut replacement_peer = native_peer();
    replacement_peer.pid = 202;
    fixture.run.attachment.as_mut().unwrap().location.as_mut().unwrap().peer_process = Some(replacement_peer.clone());
    fixture.adapter.bound_snapshot.lock().as_mut().unwrap().peer_process = Some(replacement_peer);
    assert!(matches!(fixture.service.inspect_owned_resource(&fixture.resource, &fixture.run, None).await.unwrap(),
        CleanupEvaluation::Unknown(reason) if reason.code == "space_peer_changed"));
    assert_eq!(fixture.run.retirement.as_ref().unwrap().identity.as_ref().unwrap().herdr_peer, Some(native_peer()));
    fixture.run.retirement.as_mut().unwrap().identity.as_mut().unwrap().herdr_peer = None;
    assert!(matches!(fixture.service.inspect_owned_resource(&fixture.resource, &fixture.run, None).await.unwrap(),
        CleanupEvaluation::Unknown(reason) if reason.code == "space_incarnation_unproven"));
    assert!(fixture.run.retirement.as_ref().unwrap().identity.as_ref().unwrap().herdr_peer.is_none());
}

#[tokio::test]
async fn cleanup_effect_incarnation_requires_immutable_retired_peer_without_backfill() {
    let mut fixture = SpaceFixture::new().await;
    assert_eq!(fixture.service.validate_cleanup_incarnation(&fixture.run).await.unwrap_err().code, "space_incarnation_unproven");
    fixture.run.retirement = Some(fixture.retirement());
    fixture.service.validate_cleanup_incarnation(&fixture.run).await.unwrap();
    fixture.run.retirement.as_mut().unwrap().identity.as_mut().unwrap().herdr_peer = None;
    assert_eq!(fixture.service.validate_cleanup_incarnation(&fixture.run).await.unwrap_err().code, "space_incarnation_unproven");
    fixture.run.retirement = Some(fixture.retirement());
    fixture.run.retirement.as_mut().unwrap().state = RetirementState::Waiting { blockers: vec![] };
    assert_eq!(fixture.service.validate_cleanup_incarnation(&fixture.run).await.unwrap_err().code, "space_incarnation_unproven");
}

#[tokio::test]
async fn cleanup_effect_incarnation_refuses_foreign_response_or_replacement_peer() {
    let mut fixture = SpaceFixture::new().await;
    fixture.run.retirement = Some(fixture.retirement());
    fixture.adapter.bound_snapshot.lock().as_mut().unwrap().endpoint_identity = "foreign-endpoint".into();
    assert_eq!(fixture.service.validate_cleanup_incarnation(&fixture.run).await.unwrap_err().code, "endpoint_changed");
    fixture.adapter.bound_snapshot.lock().as_mut().unwrap().endpoint_identity =
        fixture.run.retirement.as_ref().unwrap().identity.as_ref().unwrap().endpoint_identity.clone();
    let mut replacement = native_peer();
    replacement.pid = 202;
    fixture.run.attachment.as_mut().unwrap().location.as_mut().unwrap().peer_process = Some(replacement.clone());
    fixture.adapter.bound_snapshot.lock().as_mut().unwrap().peer_process = Some(replacement);
    assert_eq!(fixture.service.validate_cleanup_incarnation(&fixture.run).await.unwrap_err().code, "space_peer_changed");
    fixture.adapter.bound_snapshot.lock().as_mut().unwrap().peer_process = None;
    assert_eq!(fixture.service.validate_cleanup_incarnation(&fixture.run).await.unwrap_err().code, "space_incarnation_unproven");
    *fixture.adapter.bound_snapshot.lock() = None;
    assert_eq!(fixture.service.validate_cleanup_incarnation(&fixture.run).await.unwrap_err().code, "test_adapter_unused");
}
