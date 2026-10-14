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
use cockpit_protocol::project_teardown::{
    WorkspaceTeardownAction, WorkspaceTeardownDirtyState, WorkspaceTeardownExecuteRequest,
    WorkspaceTeardownOutcome, WorkspaceTeardownOwnership, WorkspaceTeardownPreview,
    WorkspaceTeardownPreviewRequest, WorkspaceTeardownRecoveryState,
};
use crate::project_adapter::{ProjectInventory, ProjectWorktreeEntry, WorktreeStatus};

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
    assert!(matches!(fixture.service.inspect_owned_resource(&fixture.resource, &fixture.run, None, None).await.unwrap(), CleanupEvaluation::Absent));
    fixture.adapter.bound_snapshot.lock().as_mut().unwrap().endpoint_identity = "foreign-endpoint".into();
    assert!(matches!(fixture.service.inspect_owned_resource(&fixture.resource, &fixture.run, None, None).await.unwrap(),
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
        assert!(matches!(fixture.service.inspect_owned_resource(&fixture.resource, &fixture.run, None, None).await.unwrap(),
            CleanupEvaluation::Unknown(reason) if reason.code == "space_peer_changed"));
    }
    fixture.adapter.bound_snapshot.lock().as_mut().unwrap().peer_process = Some(peer);
    fixture.run.attachment.as_mut().unwrap().location.as_mut().unwrap().boot_id = Some("recorded-raw-boot".into());
    fixture.adapter.bound_snapshot.lock().as_mut().unwrap().boot_id = Some("changed-raw-boot".into());
    assert!(matches!(fixture.service.inspect_owned_resource(&fixture.resource, &fixture.run, None, None).await.unwrap(),
        CleanupEvaluation::Unknown(reason) if reason.code == "space_boot_changed"));
    fixture.run.attachment.as_mut().unwrap().location.as_mut().unwrap().boot_id = None;
    fixture.adapter.bound_snapshot.lock().as_mut().unwrap().boot_id = None;
    fixture.adapter.bound_snapshot.lock().as_mut().unwrap().peer_process = None;
    assert!(matches!(fixture.service.inspect_owned_resource(&fixture.resource, &fixture.run, None, None).await.unwrap(),
        CleanupEvaluation::Unknown(reason) if reason.code == "space_incarnation_unproven"));
    *fixture.adapter.bound_snapshot.lock() = None;
    assert!(matches!(fixture.service.inspect_owned_resource(&fixture.resource, &fixture.run, None, None).await.unwrap(),
        CleanupEvaluation::Unknown(reason) if reason.code == "test_adapter_unused"));
}

#[tokio::test]
async fn owned_space_inspector_never_backfills_unrecorded_peer_from_fresh_observation() {
    let mut fixture = SpaceFixture::new().await;
    fixture.run.attachment.as_mut().unwrap().location.as_mut().unwrap().peer_process = None;
    assert!(matches!(fixture.service.inspect_owned_resource(&fixture.resource, &fixture.run, None, None).await.unwrap(),
        CleanupEvaluation::Unknown(reason) if reason.code == "space_incarnation_unproven"));
    assert!(fixture.run.attachment.as_ref().unwrap().location.as_ref().unwrap().peer_process.is_none());
}

#[tokio::test]
async fn retirement_peer_is_immutable_authority_and_not_replaced_by_new_attachment_peer() {
    let mut fixture = SpaceFixture::new().await;
    fixture.run.retirement = Some(fixture.retirement());
    assert!(matches!(fixture.service.inspect_owned_resource(&fixture.resource, &fixture.run, None, None).await.unwrap(), CleanupEvaluation::Absent));
    let mut replacement_peer = native_peer();
    replacement_peer.pid = 202;
    fixture.run.attachment.as_mut().unwrap().location.as_mut().unwrap().peer_process = Some(replacement_peer.clone());
    fixture.adapter.bound_snapshot.lock().as_mut().unwrap().peer_process = Some(replacement_peer);
    assert!(matches!(fixture.service.inspect_owned_resource(&fixture.resource, &fixture.run, None, None).await.unwrap(),
        CleanupEvaluation::Unknown(reason) if reason.code == "space_peer_changed"));
    assert_eq!(fixture.run.retirement.as_ref().unwrap().identity.as_ref().unwrap().herdr_peer, Some(native_peer()));
    fixture.run.retirement.as_mut().unwrap().identity.as_mut().unwrap().herdr_peer = None;
    assert!(matches!(fixture.service.inspect_owned_resource(&fixture.resource, &fixture.run, None, None).await.unwrap(),
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

struct ManualFixture {
    root: PathBuf,
    configuration: ProjectConfiguration,
    service: ProjectService,
    adapter: Arc<NestedDirectoryAdapter>,
    checkout: PathBuf,
}

impl ManualFixture {
    fn new() -> Self {
        let root = std::env::temp_dir().join(format!("cockpit-manual-removal-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&root).unwrap();
        let configuration = ProjectConfiguration::for_tests(&root);
        let adapter = Arc::new(NestedDirectoryAdapter::default());
        let service = ProjectService::new(configuration.clone(), adapter.clone()).unwrap();
        let checkout = configure_manual_checkout(&root, &service, &adapter, "workspace");
        Self { root, configuration, service, adapter, checkout }
    }

    async fn preview(&self) -> WorkspaceTeardownPreview {
        self.service.teardown_preview("session", &WorkspaceTeardownPreviewRequest {
            workspace_id: "workspace".into(),
        }).await.unwrap()
    }

    fn restart(&mut self) {
        self.service = ProjectService::new(self.configuration.clone(), self.adapter.clone()).unwrap();
    }
}

impl Drop for ManualFixture {
    fn drop(&mut self) { let _ = std::fs::remove_dir_all(&self.root); }
}

fn configure_manual_checkout(
    root: &std::path::Path,
    service: &ProjectService,
    adapter: &NestedDirectoryAdapter,
    workspace_id: &str,
) -> PathBuf {
    let repository_root = root.join("repository");
    let checkout = root.join("live-linked-checkout");
    std::fs::create_dir_all(&repository_root).unwrap();
    std::fs::create_dir_all(&checkout).unwrap();
    std::fs::write(checkout.join("untracked.txt"), "reviewed dirty content").unwrap();
    let checkout_path = checkout.to_string_lossy().into_owned();
    let repository_path = repository_root.to_string_lossy().into_owned();
    let repository_key = repository_root.join(".git").to_string_lossy().into_owned();
    let mut snapshot = observation(&identity());
    snapshot.endpoint_identity = "endpoint".into();
    snapshot.snapshot.session_id = "session".into();
    snapshot.snapshot.spaces.push(SpaceSummary {
        id: workspace_id.into(), label: "Live linked checkout".into(), number: 1,
        tab_count: 0, pane_count: 1, focused: false, agent_status: "none".into(),
        git: Some(cockpit_protocol::v1::SpaceGitSummary {
            repository_key: repository_key.clone(), repository: "repository".into(),
            branch: Some("manual-topic".into()), checkout_path: checkout_path.clone(),
            is_linked_worktree: true,
        }),
    });
    snapshot.snapshot.panes.push(PaneSummary {
        id: "decoy-pane".into(), terminal_id: "decoy-terminal".into(), space_id: workspace_id.into(),
        tab_id: "decoy-tab".into(), title: None, focused: false, agent: None,
        agent_status: "none".into(), revision: 1,
        cwd: Some(root.join("stale-pane-cwd").to_string_lossy().into_owned()),
    });
    let mut stale_unbound_snapshot = snapshot.snapshot.clone();
    stale_unbound_snapshot.spaces[0].git.as_mut().unwrap().checkout_path =
        root.join("stale-unbound-checkout").to_string_lossy().into_owned();
    *adapter.snapshot.lock() = Some(stale_unbound_snapshot);
    *adapter.bound_snapshot.lock() = Some(snapshot);
    adapter.selection_available.store(true, Ordering::SeqCst);
    *adapter.inventory.lock() = Some(ProjectInventory {
        endpoint_identity: "endpoint".into(), repository_key,
        repository_root: repository_path.clone(), supported_methods: vec![],
        worktrees: vec![
            ProjectWorktreeEntry {
                checkout_path, branch: Some("manual-topic".into()),
                open_workspace_id: Some(workspace_id.into()), is_primary: false,
                is_linked_worktree: true, dirty: Some(true),
            },
            ProjectWorktreeEntry {
                checkout_path: repository_path, branch: Some("main".into()),
                open_workspace_id: Some("unrelated-space".into()), is_primary: true,
                is_linked_worktree: false, dirty: Some(false),
            },
        ],
    });
    *adapter.worktree_status.lock() = Some(WorktreeStatus {
        porcelain_sha256: "a".repeat(64), entries: vec!["untracked.txt".into()], count: 1,
    });
    *adapter.teardown_store.lock() = Some(service.store.clone());
    checkout
}

fn manual_request(
    preview: &WorkspaceTeardownPreview,
    action: WorkspaceTeardownAction,
) -> WorkspaceTeardownExecuteRequest {
    WorkspaceTeardownExecuteRequest {
        operation_id: preview.operation_id.clone(), workspace_id: preview.workspace_id.clone(),
        expected_endpoint_identity: preview.endpoint_identity.clone(),
        expected_checkout_path: preview.checkout_path.clone(), action,
        confirmation: preview.required_confirmation.clone().unwrap_or_default(),
        discard_fingerprint: preview.discard_fingerprint.clone(),
    }
}

#[tokio::test]
async fn manual_dirty_linked_removal_without_setup_journals_before_force_dispatch() {
    let fixture = ManualFixture::new();
    assert!(fixture.service.store.list().unwrap().is_empty());
    let preview = fixture.preview().await;
    assert!(uuid::Uuid::parse_str(&preview.operation_id).is_ok());
    assert_eq!(preview.ownership, WorkspaceTeardownOwnership::SelectedLinked);
    assert_eq!(preview.dirty_state, WorkspaceTeardownDirtyState::Dirty);
    assert_eq!(preview.discard_count, 1);
    assert_eq!(preview.discard_paths, vec!["untracked.txt"]);
    assert_eq!(preview.discard_fingerprint, Some("a".repeat(64)));
    assert_eq!(preview.required_confirmation.as_deref(), Some("DISCARD AND REMOVE"));
    assert!(preview.allowed_actions.contains(&WorkspaceTeardownAction::RemoveOwnedWorktree));
    let request = manual_request(&preview, WorkspaceTeardownAction::RemoveOwnedWorktree);
    let result = fixture.service.teardown_execute("session", &request).await.unwrap();
    assert_eq!(result.outcome, WorkspaceTeardownOutcome::Completed);
    let requests = fixture.adapter.remove_requests.lock();
    assert_eq!(requests.len(), 1);
    assert!(requests[0].force);
    assert_eq!(requests[0].endpoint_identity, "endpoint");
    assert_eq!(requests[0].workspace_id, "workspace");
    assert_eq!(requests[0].checkout_path, fixture.checkout.to_string_lossy());
    drop(requests);
    let observed = fixture.adapter.receipts_at_remove.lock();
    assert_eq!(observed.len(), 1);
    assert!(observed[0].manual);
    assert_eq!(observed[0].operation_id, request.operation_id);
    assert_eq!(observed[0].checkout_path, request.expected_checkout_path);
    assert_eq!(observed[0].state, TeardownReceiptState::Pending);
    drop(observed);
    assert_eq!(fixture.service.store.read_teardown_receipt(&request.operation_id).unwrap().unwrap().state,
        TeardownReceiptState::Completed);
    assert!(fixture.service.teardown_recoveries("session").unwrap().recoveries.is_empty());
    assert_eq!(fixture.adapter.inventory_requests.lock().as_slice(),
        &[preview.checkout_path.clone(), preview.checkout_path.clone()]);
    assert_eq!(fixture.adapter.status_requests.lock().as_slice(),
        &[preview.checkout_path.clone(), preview.checkout_path.clone()]);
    // A reused UUID must not dispatch against a target that exists again.
    assert_eq!(fixture.service.teardown_execute("session", &request).await.unwrap_err().code, "stale_identity");
    assert_eq!(fixture.adapter.remove_requests.lock().len(), 1);
}

#[tokio::test]
async fn manual_removal_ignores_conflicting_stale_setup_receipts_and_uses_fresh_space_git_source() {
    let fixture = SpaceFixture::new().await;
    let setup_id = fixture.run.setup.as_ref().unwrap().operation_id.as_ref().unwrap();
    let setup = fixture.service.store.load(setup_id).unwrap();
    let creation = fixture.service.store.read_owned_creation(setup_id).unwrap().unwrap();
    assert!(creation.resources.iter().any(|resource| matches!(resource,
        OwnedResourceIdentity::Space { identity } if identity.workspace_id == "owned-space")));
    // Two historical setup associations deliberately name the same Space.
    // Neither is authority for its current uniquely selected linked checkout.
    let mut second_plan = setup.plan.clone();
    second_plan.operation_id = uuid::Uuid::new_v4().to_string();
    second_plan.checkout_path = fixture.root.join("another-stale-checkout").to_string_lossy().into_owned();
    let second_setup = fixture.service.store.persist_plan(second_plan).unwrap();
    fixture.service.store.update(&second_setup.operation_id, None, |operation| {
        operation.workspace_id = Some("owned-space".into());
        operation.state = setup.state;
        Ok(())
    }).unwrap();
    let second_creation = crate::project_store::OwnedCreationReceipt {
        operation_id: second_setup.operation_id.clone(), git_capture_error: None, moved_space: None,
        resources: vec![OwnedResourceIdentity::Space { identity: OwnedSpaceIdentity {
            creation_operation_id: second_setup.operation_id.clone(),
            endpoint_identity: second_setup.plan.endpoint_identity.clone(),
            session_id: second_setup.session_id.clone(), workspace_id: "owned-space".into(),
            checkout_path: second_setup.plan.checkout_path.clone(),
        } }],
    };
    fixture.service.store.write_owned_creation(&second_creation).unwrap();
    assert_eq!(fixture.service.store.list().unwrap().iter()
        .filter(|operation| operation.workspace_id.as_deref() == Some("owned-space")).count(), 2);
    let checkout = configure_manual_checkout(&fixture.root, &fixture.service, &fixture.adapter, "owned-space");
    assert_ne!(setup.plan.checkout_path, checkout.to_string_lossy());
    let preview = fixture.service.teardown_preview("session", &WorkspaceTeardownPreviewRequest {
        workspace_id: "owned-space".into(),
    }).await.unwrap();
    assert_ne!(preview.operation_id, *setup_id);
    assert_ne!(preview.operation_id, second_setup.operation_id);
    assert_eq!(preview.checkout_path, checkout.to_string_lossy());
    assert_eq!(preview.ownership, WorkspaceTeardownOwnership::SelectedLinked);
    let request = manual_request(&preview, WorkspaceTeardownAction::RemoveOwnedWorktree);
    assert_eq!(fixture.service.teardown_execute("session", &request).await.unwrap().outcome,
        WorkspaceTeardownOutcome::Completed);
    assert_eq!(fixture.adapter.inventory_requests.lock().as_slice(),
        &[preview.checkout_path.clone(), preview.checkout_path.clone()]);
    let requests = fixture.adapter.remove_requests.lock();
    assert_eq!(requests.len(), 1);
    assert_eq!(requests[0].checkout_path, preview.checkout_path);
    assert!(requests[0].force);
    drop(requests);
    assert_eq!(fixture.service.store.read_owned_creation(setup_id).unwrap().unwrap(), creation);
    assert_eq!(fixture.service.store.load(setup_id).unwrap().plan.checkout_path, setup.plan.checkout_path);
    assert_eq!(fixture.service.store.read_owned_creation(&second_setup.operation_id).unwrap().unwrap(),
        second_creation);
    assert!(fixture.service.store.read_teardown_receipt(&preview.operation_id).unwrap().unwrap().manual);
}

#[tokio::test]
async fn manual_uncertain_removal_survives_service_restart_and_never_redispatches() {
    for durable_state in [TeardownReceiptState::Pending, TeardownReceiptState::OutcomeUnknown] {
        let mut fixture = ManualFixture::new();
        fixture.adapter.remove_error.store(true, Ordering::SeqCst);
        let preview = fixture.preview().await;
        let remove = manual_request(&preview, WorkspaceTeardownAction::RemoveOwnedWorktree);
        assert_eq!(fixture.service.teardown_execute("session", &remove).await.unwrap().outcome,
            WorkspaceTeardownOutcome::OutcomeUnknown);
        assert_eq!(fixture.adapter.receipts_at_remove.lock()[0].state, TeardownReceiptState::Pending);
        let mut receipt = fixture.service.store.read_teardown_receipt(&remove.operation_id).unwrap().unwrap();
        assert!(receipt.manual);
        assert_eq!(receipt.state, TeardownReceiptState::OutcomeUnknown);
        // Pending is the durable state if interruption precedes the error-state write.
        receipt.state = durable_state;
        fixture.service.store.write_teardown_receipt(&receipt).unwrap();
        fixture.restart();
        let recoveries = fixture.service.teardown_recoveries("session").unwrap().recoveries;
        assert_eq!(recoveries.len(), 1);
        assert_eq!(recoveries[0].operation_id, remove.operation_id);
        assert_eq!(recoveries[0].checkout_path, remove.expected_checkout_path);
        assert_eq!(recoveries[0].state, match durable_state {
            TeardownReceiptState::Pending => WorkspaceTeardownRecoveryState::Pending,
            _ => WorkspaceTeardownRecoveryState::OutcomeUnknown,
        });
        let recovery_preview = fixture.preview().await;
        assert_eq!(recovery_preview.operation_id, remove.operation_id);
        assert_eq!(recovery_preview.allowed_actions, vec![WorkspaceTeardownAction::ReconcileRemoveOutcome]);
        let reconcile = manual_request(&recovery_preview, WorkspaceTeardownAction::ReconcileRemoveOutcome);
        let mut different_uuid = remove.clone();
        different_uuid.operation_id = uuid::Uuid::new_v4().to_string();
        assert_eq!(fixture.service.teardown_execute("session", &different_uuid).await.unwrap_err().code,
            "stale_identity");
        assert_eq!(fixture.service.teardown_execute("session", &remove).await.unwrap_err().code,
            "teardown_not_allowed");
        for _ in 0..2 {
            assert_eq!(fixture.service.teardown_execute("session", &reconcile).await.unwrap().outcome,
                WorkspaceTeardownOutcome::OutcomeUnknown);
            assert_eq!(fixture.service.store.read_teardown_receipt(&remove.operation_id).unwrap().unwrap().state,
                durable_state);
            assert_eq!(fixture.adapter.remove_requests.lock().len(), 1);
        }
        let selected_space = fixture.adapter.bound_snapshot.lock().as_ref().unwrap().snapshot.spaces.clone();
        fixture.adapter.bound_snapshot.lock().as_mut().unwrap().snapshot.spaces.clear();
        assert_eq!(fixture.service.teardown_execute("session", &reconcile).await.unwrap_err().code,
            "association_conflict");
        assert_eq!(fixture.adapter.remove_requests.lock().len(), 1);
        fixture.adapter.bound_snapshot.lock().as_mut().unwrap().snapshot.panes.clear();
        // Inventory presence alone cannot prove the interrupted effect stopped.
        assert_eq!(fixture.service.teardown_execute("session", &reconcile).await.unwrap().outcome,
            WorkspaceTeardownOutcome::OutcomeUnknown);
        fixture.adapter.inventory.lock().as_mut().unwrap().worktrees.retain(|entry|
            entry.open_workspace_id.as_deref() != Some("workspace"));
        // Filesystem presence alone also keeps the journal unresolved.
        assert_eq!(fixture.service.teardown_execute("session", &reconcile).await.unwrap().outcome,
            WorkspaceTeardownOutcome::OutcomeUnknown);
        fixture.adapter.bound_snapshot.lock().as_mut().unwrap().snapshot.spaces = selected_space;
        std::fs::remove_dir_all(&fixture.checkout).unwrap();
        // A selected Space remaining live prevents completion even after Git
        // inventory and the physical checkout have both disappeared.
        assert_eq!(fixture.service.teardown_execute("session", &reconcile).await.unwrap().outcome,
            WorkspaceTeardownOutcome::OutcomeUnknown);
        assert_eq!(fixture.adapter.remove_requests.lock().len(), 1);
        fixture.adapter.bound_snapshot.lock().as_mut().unwrap().snapshot.spaces.clear();
        assert_eq!(fixture.service.teardown_execute("session", &reconcile).await.unwrap().outcome,
            WorkspaceTeardownOutcome::Completed);
        assert_eq!(fixture.service.store.read_teardown_receipt(&remove.operation_id).unwrap().unwrap().state,
            TeardownReceiptState::Completed);
        assert!(fixture.service.teardown_recoveries("session").unwrap().recoveries.is_empty());
        assert_eq!(fixture.adapter.remove_requests.lock().len(), 1);
        assert!(fixture.adapter.closed_workspaces.lock().unwrap().is_empty());
    }
}

#[tokio::test]
async fn manual_primary_checkout_is_refused_without_journal_or_adapter_effect() {
    let fixture = ManualFixture::new();
    fixture.adapter.inventory.lock().as_mut().unwrap().worktrees[0].is_primary = true;
    let preview = fixture.preview().await;
    assert!(!preview.allowed_actions.contains(&WorkspaceTeardownAction::RemoveOwnedWorktree));
    let mut request = manual_request(&preview, WorkspaceTeardownAction::RemoveOwnedWorktree);
    request.confirmation = "DISCARD AND REMOVE".into();
    assert_eq!(fixture.service.teardown_execute("session", &request).await.unwrap_err().code,
        "teardown_not_allowed");
    assert!(fixture.service.store.list_teardown_receipts().unwrap().is_empty());
    assert!(fixture.adapter.remove_requests.lock().is_empty());
    assert!(fixture.adapter.receipts_at_remove.lock().is_empty());
    assert!(fixture.adapter.status_requests.lock().is_empty());
}

#[tokio::test]
async fn manual_dirty_removal_rejects_changed_fingerprint_before_journaling() {
    let fixture = ManualFixture::new();
    let preview = fixture.preview().await;
    let request = manual_request(&preview, WorkspaceTeardownAction::RemoveOwnedWorktree);
    *fixture.adapter.worktree_status.lock() = Some(WorktreeStatus {
        porcelain_sha256: "b".repeat(64), entries: vec!["another-untracked.txt".into()], count: 1,
    });
    assert_eq!(fixture.service.teardown_execute("session", &request).await.unwrap_err().code,
        "stale_identity");
    assert!(fixture.service.store.list_teardown_receipts().unwrap().is_empty());
    assert!(fixture.adapter.remove_requests.lock().is_empty());
    let refreshed = fixture.preview().await;
    let mut unconfirmed = manual_request(&refreshed, WorkspaceTeardownAction::RemoveOwnedWorktree);
    unconfirmed.confirmation = "REMOVE WORKTREE".into();
    assert_eq!(fixture.service.teardown_execute("session", &unconfirmed).await.unwrap_err().code,
        "confirmation_required");
    assert!(fixture.service.store.list_teardown_receipts().unwrap().is_empty());
    assert!(fixture.adapter.remove_requests.lock().is_empty());
}

#[tokio::test]
async fn manual_uncertain_removal_in_historical_session_blocks_fresh_uuid_for_same_live_checkout() {
    let mut fixture = ManualFixture::new();
    fixture.adapter.remove_error.store(true, Ordering::SeqCst);
    let preview = fixture.preview().await;
    let remove = manual_request(&preview, WorkspaceTeardownAction::RemoveOwnedWorktree);
    assert_eq!(fixture.service.teardown_execute("session", &remove).await.unwrap().outcome,
        WorkspaceTeardownOutcome::OutcomeUnknown);
    fixture.restart();
    fixture.adapter.bound_snapshot.lock().as_mut().unwrap().snapshot.session_id = "reopened-session".into();
    let recovery_preview = fixture.service.teardown_preview("reopened-session", &WorkspaceTeardownPreviewRequest {
        workspace_id: "workspace".into(),
    }).await.unwrap();
    assert_eq!(recovery_preview.operation_id, remove.operation_id);
    assert_eq!(recovery_preview.allowed_actions, vec![WorkspaceTeardownAction::ReconcileRemoveOutcome]);
    let mut new_removal = remove.clone();
    new_removal.operation_id = uuid::Uuid::new_v4().to_string();
    assert_eq!(fixture.service.teardown_execute("reopened-session", &new_removal).await.unwrap_err().code,
        "stale_identity");
    let reconcile = manual_request(&recovery_preview, WorkspaceTeardownAction::ReconcileRemoveOutcome);
    assert_eq!(fixture.service.teardown_execute("reopened-session", &reconcile).await.unwrap().outcome,
        WorkspaceTeardownOutcome::OutcomeUnknown);
    assert_eq!(fixture.adapter.remove_requests.lock().len(), 1);
    let receipt = fixture.service.store.read_teardown_receipt(&remove.operation_id).unwrap().unwrap();
    assert_eq!(receipt.session_id, "session");
    assert_eq!(receipt.state, TeardownReceiptState::OutcomeUnknown);
}
