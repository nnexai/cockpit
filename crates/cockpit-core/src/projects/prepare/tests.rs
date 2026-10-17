use std::path::PathBuf;
use std::sync::{Arc, atomic::Ordering};

use cockpit_protocol::library::{LibraryAddRequest, SpaceRepositoriesRequest};
use cockpit_protocol::projects::{ProjectConfiguration, WorkspaceSetupRequest};
use cockpit_protocol::v1::{PaneSummary, SpaceGitSummary, SpaceSummary, TabSummary};

use super::*;
use crate::HerdrAdapter;
use crate::library::LibraryService;
use crate::projects::test_support::{NestedDirectoryAdapter, git};
use crate::project_adapter::{ProjectInventory, ProjectWorktreeEntry, ProjectWorktreeResult};
use crate::sources::SourceService;

struct Fixture {
    root: PathBuf,
    config: ProjectConfiguration,
    adapter: Arc<NestedDirectoryAdapter>,
    service: ProjectService,
    library: LibraryService,
    request: OwnedCheckoutRequest,
}
impl Fixture {
    async fn new() -> Self {
        let root = std::env::temp_dir().join(format!("cockpit-checkout-prep-{}", uuid::Uuid::new_v4()));
        let repository = root.join("repository");
        std::fs::create_dir_all(&repository).unwrap();
        git(&repository, &["init", "-q"]);
        git(&repository, &["-c", "user.name=Fixture", "-c", "user.email=fixture@example.invalid", "commit", "--allow-empty", "-m", "base"]);
        let mut config = ProjectConfiguration::for_tests(&root);
        config.repository_roots = vec![repository.to_string_lossy().into_owned()];
        let adapter = Arc::new(NestedDirectoryAdapter::default());
        adapter.selection_available.store(true, Ordering::SeqCst);
        let service = ProjectService::new(config.clone(), adapter.clone()).unwrap();
        let library = LibraryService::new(config.clone(), Arc::new(SourceService::new(&config, vec![]).unwrap())).with_herdr(adapter.clone());
        let request = OwnedCheckoutRequest {
            operation_id: uuid::Uuid::new_v4().to_string(), repository_checkout: repository.to_string_lossy().into_owned(),
            checkout_path: root.join("worktrees/worker").to_string_lossy().into_owned(), branch: "worker".into(),
            base: crate::space_git::git_line(&repository, &["rev-parse", "HEAD"]).await.unwrap(), artifact: None,
            source_workspace_id: "workspace".into(),
        };
        Self { root, config, adapter, service, library, request }
    }
    fn bind(&self, prepared: &WorkspaceOperation) -> BindMovedSpaceRequest {
        BindMovedSpaceRequest {
            operation_id: prepared.operation_id.clone(), expected_generation: prepared.generation,
            endpoint_identity: prepared.plan.endpoint_identity.clone(), workspace_id: "moved".into(),
            tab_id: "moved:worker".into(), pane_id: "moved:worker:p1".into(), terminal_id: "worker-terminal".into(),
            move_operation_id: uuid::Uuid::new_v4().to_string(),
        }
    }
    async fn move_snapshot(&self, checkout: &str) {
        let mut snapshot = self.adapter.session_snapshot("session").await.unwrap();
        snapshot.spaces.push(SpaceSummary { id: "moved".into(), label: "Worker".into(), number: 2,
            tab_count: 1, pane_count: 1, focused: false, agent_status: "none".into(), git: None });
        snapshot.tabs.push(TabSummary { id: "moved:worker".into(), space_id: "moved".into(), label: "launch-attempt".into(),
            number: 1, pane_count: 1, focused: false, focused_pane_id: Some("moved:worker:p1".into()) });
        snapshot.panes.push(PaneSummary { id: "moved:worker:p1".into(), terminal_id: "worker-terminal".into(),
            space_id: "moved".into(), tab_id: "moved:worker".into(), title: None, focused: false,
            agent: None, agent_status: "none".into(), revision: 1, cwd: Some(checkout.into()) });
        *self.adapter.snapshot.lock() = Some(snapshot);
        self.registration_inventory(checkout, "moved").await;
    }
    fn no_shells(&self) {
        assert!(self.adapter.worktree_requests.lock().unwrap().iter().all(|request|
            request.mode == WorkspaceSetupMode::Open && request.open_existing_worktree));
        assert!(self.adapter.closed_workspaces.lock().unwrap().is_empty());
    }
    async fn registration_inventory(&self, checkout: &str, workspace: &str) {
        let repository = RepositoryCatalog::new(self.config.clone()).list().await.unwrap()
            .repositories.into_iter().find(|repository| !repository.is_linked_worktree).unwrap();
        *self.adapter.inventory.lock() = Some(ProjectInventory {
            endpoint_identity: "endpoint".into(), repository_key: repository.common_dir,
            repository_root: repository.root.clone(), supported_methods: vec!["worktree.open".into()],
            worktrees: vec![
                ProjectWorktreeEntry { checkout_path: repository.root, branch: None,
                    open_workspace_id: Some("workspace".into()), is_primary: true,
                    is_linked_worktree: false, dirty: Some(false) },
                ProjectWorktreeEntry { checkout_path: checkout.into(), branch: Some("worker".into()),
                    open_workspace_id: Some(workspace.into()), is_primary: false,
                    is_linked_worktree: true, dirty: Some(false) },
            ],
        });
    }
}
impl Drop for Fixture { fn drop(&mut self) { let _ = std::fs::remove_dir_all(&self.root); } }

#[tokio::test]
async fn owned_prep_is_filesystem_only_idempotent_and_retains_resolved_base() {
    let fixture = Fixture::new().await;
    let first = fixture.service.prepare_owned_checkout("session", &fixture.request, &fixture.library).await.unwrap();
    assert_eq!(first.step, WorkspaceOperationStep::CheckoutPrepared);
    assert_eq!(first.state, WorkspaceOperationState::Running);
    assert_eq!(first.plan.base, Some(fixture.request.base.clone()));
    assert!(first.workspace_id.is_none());
    assert_eq!(crate::space_git::git_line(Path::new(&first.plan.checkout_path), &["symbolic-ref", "--short", "HEAD"]).await.unwrap(), "worker");
    let receipt = fixture.service.store.read_owned_creation(&first.operation_id).unwrap().unwrap();
    assert_eq!(receipt.resources.len(), 2);
    assert!(receipt.resources.iter().all(|r| !matches!(r, OwnedResourceIdentity::Space { .. })));
    let second = fixture.service.prepare_owned_checkout("session", &fixture.request, &fixture.library).await.unwrap();
    assert_eq!(second.generation, first.generation);
    let mut foreign = fixture.request.clone(); foreign.branch = "foreign".into();
    assert_eq!(fixture.service.prepare_owned_checkout("session", &foreign, &fixture.library).await.unwrap_err().code, "association_conflict");
    assert!(fixture.adapter.worktree_requests.lock().unwrap().is_empty());
    fixture.no_shells();
}

#[tokio::test]
async fn bind_selects_existing_context_paths_and_captures_actual_move_operation() {
    let fixture = Fixture::new().await;
    let library = &fixture.library;
    let source_dir = fixture.root.join("context-source"); std::fs::create_dir(&source_dir).unwrap();
    std::fs::write(source_dir.join("context.txt"), "shared context").unwrap();
    let add = library.start_add(LibraryAddRequest {
        input: source_dir.to_string_lossy().into_owned(), provider_id: None, reference_depth: 0,
        follow: false, follow_mode: None, download_attachments: false, refresh_existing: false,
        label: None, target: Some(SpaceTarget { session_id: "session".into(), space_id: "workspace".into() }),
    }).await.unwrap();
    loop {
        let operation = library.operation(&add.operation_id).await.unwrap();
        if operation.finished { assert!(operation.phases.iter().all(|p| p.state == cockpit_protocol::library::LibraryPhaseState::Done)); break; }
        tokio::time::sleep(std::time::Duration::from_millis(5)).await;
    }
    let source = library.space_repositories(SpaceRepositoriesRequest {
        target: SpaceTarget { session_id: "session".into(), space_id: "workspace".into() },
        repository_paths: vec![fixture.request.repository_checkout.clone()],
    }).await.unwrap();
    let prepared = fixture.service.prepare_owned_checkout("session", &fixture.request, library).await.unwrap();
    fixture.move_snapshot(&prepared.plan.checkout_path).await;
    let binding = fixture.bind(&prepared);
    let completed = fixture.service.bind_moved_space("session", &binding, library).await.unwrap();
    assert_eq!(completed.state, WorkspaceOperationState::Completed);
    assert_eq!(completed.workspace_id.as_deref(), Some("moved"));
    assert_eq!(completed.pane_id.as_deref(), Some(binding.pane_id.as_str()));
    let target = library.space_listing(&SpaceTarget { session_id: "session".into(), space_id: "moved".into() }).await.unwrap();
    assert_eq!(target.repository_paths, source.repository_paths);
    assert_eq!(target.items.iter().map(|i| (&i.item_id, &i.item_path)).collect::<Vec<_>>(),
        source.items.iter().map(|i| (&i.item_id, &i.item_path)).collect::<Vec<_>>());
    let receipt = fixture.service.store.read_owned_creation(&prepared.operation_id).unwrap().unwrap();
    let space = receipt.resources.iter().find(|r| matches!(r, OwnedResourceIdentity::Space { .. })).unwrap();
    assert!(matches!(space, OwnedResourceIdentity::Space { identity } if identity.creation_operation_id == binding.move_operation_id));
    fixture.service.validate_owned_resource(space).await.unwrap();
    let repeated = fixture.service.bind_moved_space("session", &binding, library).await.unwrap();
    assert_eq!(repeated.generation, completed.generation);
    fixture.no_shells();
}

#[tokio::test]
async fn restart_finishes_exact_retained_bind_without_new_git_or_herdr_effect() {
    let fixture = Fixture::new().await;
    let prepared = fixture.service.prepare_owned_checkout("session", &fixture.request, &fixture.library).await.unwrap();
    fixture.move_snapshot(&prepared.plan.checkout_path).await;
    let binding = fixture.bind(&prepared);
    fixture.service.store.update_owned_checkout(&prepared.operation_id, Some(prepared.generation), |o, journal| {
        journal.as_mut().unwrap().bind = Some(binding.clone());
        o.workspace_id = Some(binding.workspace_id.clone()); o.tab_id = Some(binding.tab_id.clone()); o.pane_id = Some(binding.pane_id.clone());
        o.step = WorkspaceOperationStep::SpaceBound; Ok(())
    }).unwrap();
    let restarted = ProjectService::new(fixture.config.clone(), fixture.adapter.clone()).unwrap();
    let recovered = restarted.bind_moved_space("session", &binding, &fixture.library).await.unwrap();
    assert_eq!(recovered.state, WorkspaceOperationState::Completed);
    assert_eq!(restarted.store.read_owned_creation(&prepared.operation_id).unwrap().unwrap().resources.len(), 3);
    fixture.no_shells();
}

#[tokio::test]
async fn interrupted_git_without_creation_proof_is_never_replayed_or_adopted() {
    let fixture = Fixture::new().await;
    let prepared = fixture.service.prepare_owned_checkout("session", &fixture.request, &fixture.library).await.unwrap();
    // Simulate a crash after Git returned but before its immutable physical proof was durable.
    std::fs::remove_file(Path::new(&fixture.config.state_root).join(format!("{}.owned.json", prepared.operation_id))).unwrap();
    fixture.service.store.update(&prepared.operation_id, None, |o| {
        o.step = WorkspaceOperationStep::Validated; o.state = WorkspaceOperationState::Running;
        o.error = Some(error_response("owned_checkout_git_pending", "interrupted")); Ok(())
    }).unwrap();
    let restarted = ProjectService::new(fixture.config.clone(), fixture.adapter.clone()).unwrap();
    assert_eq!(restarted.store.load(&prepared.operation_id).unwrap().state, WorkspaceOperationState::OutcomeUnknown);
    assert_eq!(restarted.prepare_owned_checkout("session", &fixture.request, &fixture.library).await.unwrap_err().code, "owned_checkout_outcome_unknown");
    assert!(Path::new(&fixture.request.checkout_path).is_dir());
    assert!(restarted.store.read_owned_creation(&prepared.operation_id).unwrap().is_none());
    fixture.no_shells();
}

#[tokio::test]
async fn binding_refuses_foreign_terminal_extra_pane_stale_generation_and_dirty_checkout() {
    let fixture = Fixture::new().await;
    let prepared = fixture.service.prepare_owned_checkout("session", &fixture.request, &fixture.library).await.unwrap();
    fixture.move_snapshot(&prepared.plan.checkout_path).await;
    let binding = fixture.bind(&prepared);
    let mut stale = binding.clone(); stale.expected_generation -= 1;
    assert_eq!(fixture.service.bind_moved_space("session", &stale, &fixture.library).await.unwrap_err().code, "stale_generation");
    let mut foreign = binding.clone(); foreign.terminal_id = "foreign-terminal".into();
    assert_eq!(fixture.service.bind_moved_space("session", &foreign, &fixture.library).await.unwrap_err().code, "association_conflict");
    let original = fixture.adapter.snapshot.lock().clone().unwrap();
    let mut extra = original.clone(); let mut second = extra.panes[0].clone(); second.id = "other-pane".into(); second.terminal_id = "other-terminal".into(); extra.panes.push(second);
    *fixture.adapter.snapshot.lock() = Some(extra);
    assert_eq!(fixture.service.bind_moved_space("session", &binding, &fixture.library).await.unwrap_err().code, "association_conflict");
    *fixture.adapter.snapshot.lock() = Some(original);
    std::fs::write(Path::new(&fixture.request.checkout_path).join("user-work"), "protected").unwrap();
    assert_eq!(fixture.service.bind_moved_space("session", &binding, &fixture.library).await.unwrap_err().code, "owned_git_conflict");
    assert!(fixture.service.store.owned_checkout(&prepared.operation_id).unwrap().unwrap().bind.is_none());
    fixture.no_shells();
}

#[tokio::test]
async fn headless_open_borrows_non_git_directory_and_owns_only_moved_space() {
    let fixture = Fixture::new().await;
    let directory = fixture.root.join("user-directory"); std::fs::create_dir(&directory).unwrap();
    let file = directory.join("keep.txt"); std::fs::write(&file, "user data").unwrap();
    let plan = fixture.service.plan("session", &cockpit_protocol::projects::WorkspaceSetupRequest::Open {
        path: directory.to_string_lossy().into_owned(), label: Some("User directory".into()), task_name: None, focus: false,
    }).await.unwrap();
    let request = DirectoryCheckoutRequest { operation_id: plan.operation_id.clone(),
        checkout_path: directory.to_string_lossy().into_owned(), source_workspace_id: "workspace".into() };
    let prepared = fixture.service.prepare_directory("session", &request, &fixture.library).await.unwrap();
    assert_eq!(prepared.plan.ownership, WorkspaceCheckoutOwnership::BorrowedDirectory);
    assert_eq!(prepared.operation_id, plan.operation_id);
    assert_eq!(prepared.plan.label, "User directory");
    assert_eq!(fixture.service.prepare_directory("session", &request, &fixture.library).await.unwrap().generation, prepared.generation);
    assert!(fixture.service.store.read_owned_creation(&prepared.operation_id).unwrap().is_none());
    fixture.move_snapshot(&prepared.plan.checkout_path).await;
    let binding = fixture.bind(&prepared);
    fixture.service.bind_moved_space("session", &binding, &fixture.library).await.unwrap();
    let receipt = fixture.service.store.read_owned_creation(&prepared.operation_id).unwrap().unwrap();
    assert_eq!(receipt.resources.len(), 1);
    assert!(matches!(receipt.resources[0], OwnedResourceIdentity::Space { .. }));
    assert_eq!(std::fs::read_to_string(file).unwrap(), "user data");
    assert!(!directory.join(".git").exists());
    assert_eq!(fixture.adapter.inventory_calls.load(Ordering::SeqCst), 0);
    assert!(fixture.adapter.worktree_requests.lock().unwrap().is_empty());
    fixture.no_shells();
}

#[tokio::test]
async fn owned_preparation_adopts_the_exact_existing_create_plan_not_a_second_operation() {
    let fixture = Fixture::new().await;
    let first = fixture.service.prepare_owned_checkout("session", &fixture.request, &fixture.library).await.unwrap();
    let mut request = fixture.request.clone();
    request.operation_id = uuid::Uuid::new_v4().to_string();
    request.branch = "second-worker".into();
    request.checkout_path = fixture.root.join("worktrees/second-worker").to_string_lossy().into_owned();
    let mut plan = first.plan;
    plan.operation_id = request.operation_id.clone();
    plan.checkout_path = request.checkout_path.clone();
    plan.branch = Some(request.branch.clone());
    plan.label = "Retained setup label".into();
    fixture.service.store.persist_plan(plan).unwrap();
    let prepared = fixture.service.prepare_owned_checkout("session", &request, &fixture.library).await.unwrap();
    assert_eq!(prepared.operation_id, request.operation_id);
    assert_eq!(prepared.plan.label, "Retained setup label");
    assert_eq!(fixture.service.store.list().unwrap().len(), 2);
    fixture.no_shells();
}

#[tokio::test]
async fn bind_rejects_a_terminal_that_predates_actor_preparation() {
    let fixture = Fixture::new().await;
    let mut source = fixture.adapter.session_snapshot("session").await.unwrap();
    source.panes.push(PaneSummary {
        id: "existing-pane".into(), terminal_id: "worker-terminal".into(), space_id: "workspace".into(),
        tab_id: "existing-tab".into(), title: None, focused: false, agent: None,
        agent_status: "none".into(), revision: 1, cwd: Some(fixture.request.repository_checkout.clone()),
    });
    *fixture.adapter.snapshot.lock() = Some(source);
    let prepared = fixture.service.prepare_owned_checkout("session", &fixture.request, &fixture.library).await.unwrap();
    // Move-shaped fresh geometry cannot turn a pre-existing foreign consumer into this actor's ownership.
    fixture.adapter.snapshot.lock().as_mut().unwrap().panes.clear();
    fixture.move_snapshot(&prepared.plan.checkout_path).await;
    let binding = fixture.bind(&prepared);
    assert_eq!(fixture.service.bind_moved_space("session", &binding, &fixture.library).await.unwrap_err().code, "association_conflict");
    assert!(fixture.service.store.owned_checkout(&prepared.operation_id).unwrap().unwrap().bind.is_none());
    fixture.no_shells();
}

#[tokio::test]
async fn default_orchestration_workers_and_replacement_create_distinct_real_owned_git_branches() {
    let mut fixture = Fixture::new().await;
    fixture.config.branch_template = "cockpit/{repo}/{task_id}".into();
    let service = ProjectService::new(fixture.config.clone(), fixture.adapter.clone()).unwrap();
    let repository = RepositoryCatalog::new(fixture.config.clone()).list().await.unwrap()
        .repositories.into_iter().find(|repository| !repository.is_linked_worktree).unwrap();
    fixture.adapter.repository.set(repository.clone()).unwrap();
    let request = WorkspaceSetupRequest::Create {
        repository_id: repository.repository_id.clone(), branch: None,
        base_ref: Some(fixture.request.base.clone()), checkout_path: None,
        label: None, task_name: Some("Same human title".into()), artifact_url: None,
        linked_artifact_urls: Vec::new(), focus: false,
    };
    let first_task = uuid::Uuid::new_v4().to_string();
    let second_task = uuid::Uuid::new_v4().to_string();
    let mut prepared = Vec::new();
    let mut first_request = None;
    for (task_id, attempt) in [(&first_task, 1), (&second_task, 1), (&first_task, 2)] {
        let run_id = uuid::Uuid::new_v4().to_string();
        let suffix = format!("{task_id}-run-{run_id}-a{attempt}");
        let plan = service.plan_orchestration("session", &request, &suffix).await.unwrap();
        let checkout = OwnedCheckoutRequest {
            operation_id: plan.operation_id.clone(), repository_checkout: repository.checkout_path.clone(),
            checkout_path: plan.checkout_path.clone(), branch: plan.branch.clone().unwrap(),
            base: plan.base.clone().unwrap(), artifact: plan.artifact.clone(), source_workspace_id: "workspace".into(),
        };
        let operation = service.prepare_owned_checkout("session", &checkout, &fixture.library).await.unwrap();
        assert_eq!(operation.step, WorkspaceOperationStep::CheckoutPrepared);
        assert_eq!(crate::space_git::git_line(Path::new(&operation.plan.checkout_path),
            &["symbolic-ref", "--short", "HEAD"]).await.unwrap(), checkout.branch);
        assert_eq!(crate::space_git::git_line(Path::new(&repository.checkout_path),
            &["rev-parse", &format!("refs/heads/{}", checkout.branch)]).await.unwrap(), fixture.request.base);
        assert!(prepared.iter().all(|older: &WorkspaceOperation|
            older.plan.branch != operation.plan.branch && older.plan.checkout_path != operation.plan.checkout_path));
        if first_request.is_none() {
            std::fs::write(Path::new(&operation.plan.checkout_path).join("old-uncommitted-output.txt"), "keep old data\n").unwrap();
            first_request = Some(checkout);
        }
        prepared.push(operation);
    }
    assert_eq!(prepared.len(), 3);
    assert_eq!(std::fs::read_to_string(Path::new(&prepared[0].plan.checkout_path)
        .join("old-uncommitted-output.txt")).unwrap(), "keep old data\n");
    let retry = service.prepare_owned_checkout("session", first_request.as_ref().unwrap(), &fixture.library).await.unwrap();
    assert_eq!(retry.operation_id, prepared[0].operation_id);
    assert_eq!(retry.generation, prepared[0].generation);
    assert_eq!(retry.plan.branch, prepared[0].plan.branch);
    let ordinary = service.plan("session", &request).await.unwrap();
    assert_eq!(ordinary.branch, Some(format!("cockpit/{}/task", repository.name)));
    let mut explicit = request.clone();
    if let WorkspaceSetupRequest::Create { branch, .. } = &mut explicit {
        *branch = Some("requested-exact-branch".into());
    }
    let exact = service.plan_orchestration("session", &explicit, "must-not-change-explicit").await.unwrap();
    assert_eq!(exact.branch.as_deref(), Some("requested-exact-branch"));
    let explicit_checkout = OwnedCheckoutRequest {
        operation_id: exact.operation_id, repository_checkout: repository.checkout_path.clone(),
        checkout_path: exact.checkout_path, branch: exact.branch.unwrap(), base: exact.base.unwrap(),
        artifact: exact.artifact, source_workspace_id: "workspace".into(),
    };
    let exact = service.prepare_owned_checkout("session", &explicit_checkout, &fixture.library).await.unwrap();
    assert_eq!(crate::space_git::git_line(Path::new(&exact.plan.checkout_path),
        &["symbolic-ref", "--short", "HEAD"]).await.unwrap(), "requested-exact-branch");
    fixture.no_shells();
}

fn resumed_peer() -> cockpit_protocol::orchestration::NativeProcessIdentity {
    cockpit_protocol::orchestration::NativeProcessIdentity {
        pid: 101, start_ticks: 900,
        kernel_boot_id: Some("11111111-2222-4333-8444-555555555555".into()),
    }
}

impl Fixture {
    async fn original_bound_checkout(&self) -> WorkspaceOperation {
        let prepared = self.service.prepare_owned_checkout("session", &self.request, &self.library).await.unwrap();
        self.move_snapshot(&prepared.plan.checkout_path).await;
        self.service.bind_moved_space("session", &self.bind(&prepared), &self.library).await.unwrap()
    }

    async fn resumed_move(&self, checkout: &str) -> (
        cockpit_protocol::orchestration::RunLocation, BindMovedSpaceRequest,
    ) {
        let source = cockpit_protocol::orchestration::RunLocation {
            boot_id: None, peer_process: Some(resumed_peer()),
            terminal_id: Some("resumed-worker-terminal".into()), native_session_id: None,
            endpoint_identity: "endpoint".into(), session_id: "session".into(),
            workspace_id: "workspace".into(), tab_id: "workspace:waiting".into(),
            pane_id: "workspace:waiting:p1".into(), launch_tag: "resume-launch-epoch-2".into(),
        };
        let operation_id = uuid::Uuid::new_v4().to_string();
        let request = BindMovedSpaceRequest {
            operation_id: operation_id.clone(), move_operation_id: operation_id,
            expected_generation: 1, endpoint_identity: source.endpoint_identity.clone(),
            workspace_id: "resumed-space".into(), tab_id: "resumed-space:worker".into(),
            pane_id: "resumed-space:worker:p1".into(),
            terminal_id: source.terminal_id.clone().unwrap(),
        };
        let mut snapshot = self.adapter.session_snapshot("session").await.unwrap();
        snapshot.version = "0.9.3".into();
        // The original owned Space is gone; only the source project Space survives.
        snapshot.spaces.retain(|space| space.id == source.workspace_id);
        snapshot.tabs.clear();
        snapshot.panes.clear();
        snapshot.spaces.push(SpaceSummary {
            id: request.workspace_id.clone(), label: "Resumed worker".into(), number: 2,
            tab_count: 1, pane_count: 1, focused: false, agent_status: "none".into(), git: None,
        });
        snapshot.tabs.push(TabSummary {
            id: request.tab_id.clone(), space_id: request.workspace_id.clone(),
            label: source.launch_tag.clone(), number: 1, pane_count: 1, focused: false,
            focused_pane_id: Some(request.pane_id.clone()),
        });
        snapshot.panes.push(PaneSummary {
            id: request.pane_id.clone(), terminal_id: request.terminal_id.clone(),
            space_id: request.workspace_id.clone(), tab_id: request.tab_id.clone(),
            title: None, focused: false, agent: None, agent_status: "none".into(),
            revision: 1, cwd: Some(checkout.into()),
        });
        *self.adapter.snapshot.lock() = Some(snapshot.clone());
        *self.adapter.bound_snapshot.lock() = Some(crate::project_adapter::ProjectSnapshot {
            snapshot, endpoint_identity: source.endpoint_identity.clone(),
            boot_id: None, peer_process: source.peer_process.clone(),
        });
        self.registration_inventory(checkout, &request.workspace_id).await;
        (source, request)
    }
}

fn resumed_run(
    previous_operation_id: Option<&str>,
    checkout: &cockpit_protocol::orchestration::CheckoutIdentity,
    source: &cockpit_protocol::orchestration::RunLocation,
    request: &BindMovedSpaceRequest,
) -> cockpit_protocol::orchestration::Run {
    serde_json::from_value(serde_json::json!({
        "session_id": "session", "brief": "", "run_id": "same-worker", "kind": "worker",
        "label": "Resumed worker", "root_id": "unchanged-root", "attempt": 1,
        "launch_config": {}, "stage": "working", "annotations": [],
        "setup": {
            "operation_id": previous_operation_id, "workspace_id": request.workspace_id,
            "checkout_path": checkout.path, "effects": [], "warnings": []
        },
        "checkout": checkout,
        "dispatch": {
            "launch_tag": source.launch_tag, "endpoint_identity": source.endpoint_identity,
            "source_location": source, "move_operation_id": request.move_operation_id,
            "agent_started": true, "step": "placed", "launch_attempt": 2, "updated_at": "2"
        },
        "created_at": "1", "updated_at": "2"
    })).unwrap()
}

#[tokio::test]
async fn pending_resume_capture_keeps_original_filesystem_receipts_until_exact_placement() {
    use cockpit_protocol::orchestration::DispatchStep;
    use crate::orchestration::{OrchestrationService, herdr::{
        AgentTabRequest, OrchestrationHerdr, PaneProcessInfo, RuntimeView,
    }};
    use cockpit_protocol::orchestration::{OwnedResourceRecord, ResourceOwnership, Run, RunLocation, WorkerCleanupPolicy};

    struct NoTerminalAdapter;
    #[async_trait::async_trait]
    impl OrchestrationHerdr for NoTerminalAdapter {
        async fn runtime(&self, _: &str) -> Result<RuntimeView, InspectionError> { panic!("filesystem capture never observes terminals") }
        async fn create_agent_tab(&self, _: &str, _: &AgentTabRequest) -> Result<RunLocation, InspectionError> { panic!("capture never launches") }
        async fn pane_process_info(&self, _: &str, _: &str, _: &str) -> Result<PaneProcessInfo, InspectionError> { panic!("no attached terminal exists") }
        async fn close_pane(&self, _: &str, _: &str, _: &str) -> Result<(), InspectionError> { panic!("capture never closes") }
    }
    fn save_run(service: &OrchestrationService, run: &Run) {
        let path = service.base().join("state.json");
        let mut state: serde_json::Value = if path.exists() {
            serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap()
        } else {
            serde_json::json!({
                "schema": 4, "revision": 0, "runs": [], "messages": [], "task_ownership": [],
                "inbox_hints": [], "task_intents": [], "assignment_intents": [], "scope_intents": [],
                "credentials": [], "checkout_claims": [], "operations": [], "owned_resources": []
            })
        };
        state["runs"] = serde_json::json!([run]);
        std::fs::write(path, serde_json::to_vec(&state).unwrap()).unwrap();
    }
    fn captured(service: &OrchestrationService) -> Vec<OwnedResourceRecord> {
        let state: serde_json::Value = serde_json::from_slice(
            &std::fs::read(service.base().join("state.json")).unwrap(),
        ).unwrap();
        serde_json::from_value(state["owned_resources"].clone()).unwrap()
    }

    let fixture = Fixture::new().await;
    let original = fixture.original_bound_checkout().await;
    let original_receipt = fixture.service.store.read_owned_creation(&original.operation_id).unwrap().unwrap();
    let expected_filesystem: Vec<_> = original_receipt.resources.iter()
        .filter(|resource| !matches!(resource, OwnedResourceIdentity::Space { .. })).cloned().collect();
    assert_eq!(expected_filesystem.len(), 2);
    let (_, checkout) = crate::orchestration::project_identity::resolve_directory(
        Path::new(&original.plan.checkout_path), None, None,
    ).await.unwrap();
    let (source, request) = fixture.resumed_move(&checkout.path).await;
    let mut run = resumed_run(Some(&original.operation_id), &checkout, &source, &request);
    run.setup.as_mut().unwrap().workspace_id = original.workspace_id.clone();
    run.cleanup_policy = Some(WorkerCleanupPolicy::OwnedWhenSafe);
    run.setup.as_mut().unwrap().branch = original.plan.branch.clone();
    run.setup.as_mut().unwrap().base_commit = original.plan.base.clone();
    let orchestration = OrchestrationService::open(&fixture.config).unwrap();
    let restarted = ProjectService::new(fixture.config.clone(), fixture.adapter.clone()).unwrap();
    for step in [DispatchStep::MoveIntent, DispatchStep::MoveUnknown] {
        run.dispatch.as_mut().unwrap().step = step;
        assert_eq!(restarted.owned_resources_for_run(&run).await.unwrap(), expected_filesystem,
            "pending placement must capture real original Git pins without claiming either Space");
        assert!(restarted.owned_creation_limitation_for_run(&run).await.unwrap().is_none());
        assert!(restarted.store.read_owned_creation(&request.operation_id).unwrap().is_none(),
            "the pending move need not have a binding journal or receipt yet");
        save_run(&orchestration, &run);
        orchestration.capture_owned_resources(&restarted, &NoTerminalAdapter).await.unwrap();
        let records = captured(&orchestration);
        assert_eq!(records.iter().map(|record| record.identity.clone()).collect::<Vec<_>>(), expected_filesystem);
        assert!(records.iter().all(|record| record.ownership == ResourceOwnership::Owned
            && record.cleanup.reason.is_none()), "pending capture persists no synthetic UNKNOWN Git records");
    }
    let mut wrong_session = run.clone();
    wrong_session.session_id = "foreign-session".into();
    assert!(restarted.owned_resources_for_run(&wrong_session).await.is_err(),
        "a pending move never bypasses original creation receipt validation");
    save_run(&orchestration, &wrong_session);
    let before_error = std::fs::read(orchestration.base().join("state.json")).unwrap();
    assert!(orchestration.capture_owned_resources(&restarted, &NoTerminalAdapter).await.is_err());
    assert_eq!(std::fs::read(orchestration.base().join("state.json")).unwrap(), before_error,
        "a genuine receipt error propagates without fabricating missing Git pins");
    restarted.bind_resumed_space(
        "session", Some(&original.operation_id), &checkout, &source, &request, &fixture.library,
    ).await.unwrap();
    assert_eq!(restarted.owned_resources_for_run(&run).await.unwrap(), expected_filesystem,
        "even a completed new binding grants no Space until placement commits");
    save_run(&orchestration, &run);
    orchestration.capture_owned_resources(&restarted, &NoTerminalAdapter).await.unwrap();
    assert_eq!(captured(&orchestration).len(), 2, "binding completion alone grants no Space");
    run.dispatch.as_mut().unwrap().step = DispatchStep::Placed;
    assert!(restarted.owned_resources_for_run(&run).await.is_err(),
        "stable placement with a mismatched workspace must still refuse ownership");
    save_run(&orchestration, &run);
    let before_error = std::fs::read(orchestration.base().join("state.json")).unwrap();
    assert!(orchestration.capture_owned_resources(&restarted, &NoTerminalAdapter).await.is_err());
    assert_eq!(std::fs::read(orchestration.base().join("state.json")).unwrap(), before_error,
        "stable placement refusal preserves original records and inserts no UNKNOWN placeholders");
    run.setup.as_mut().unwrap().workspace_id = Some(request.workspace_id.clone());
    let mut expected = expected_filesystem;
    expected.extend(restarted.store.read_owned_creation(&request.operation_id).unwrap().unwrap().resources);
    assert_eq!(restarted.owned_resources_for_run(&run).await.unwrap(), expected,
        "the placement-complete observation captures the exact new Space with the original Git pins");
    save_run(&orchestration, &run);
    orchestration.capture_owned_resources(&restarted, &NoTerminalAdapter).await.unwrap();
    let records = captured(&orchestration);
    assert_eq!(records.iter().map(|record| record.identity.clone()).collect::<Vec<_>>(), expected);
    assert!(records.iter().all(|record| record.ownership == ResourceOwnership::Owned
        && record.cleanup.reason.is_none()), "placement completion retries capture without false UNKNOWN rows");
    assert_eq!(restarted.store.read_owned_creation(&original.operation_id).unwrap().unwrap(), original_receipt,
        "pending and bound observations never rewrite historical ownership");
    fixture.no_shells();
}

#[tokio::test]
async fn resumed_binding_preserves_original_git_creator_and_only_receipts_new_space() {
    let fixture = Fixture::new().await;
    let original = fixture.original_bound_checkout().await;
    let original_receipt = fixture.service.store.read_owned_creation(&original.operation_id).unwrap().unwrap();
    let original_journal = serde_json::to_value(fixture.service.store.owned_checkout(&original.operation_id).unwrap()).unwrap();
    let checkout_path = Path::new(&original.plan.checkout_path);
    std::fs::write(checkout_path.join("prior-output.txt"), "prior committed output\n").unwrap();
    git(checkout_path, &["add", "prior-output.txt"]);
    git(checkout_path, &["-c", "user.name=Fixture", "-c", "user.email=fixture@example.invalid",
        "commit", "-m", "worker progress before resume"]);
    let current_head = crate::space_git::git_line(checkout_path, &["rev-parse", "HEAD"]).await.unwrap();
    assert_ne!(current_head, fixture.request.base);
    std::fs::write(checkout_path.join("dirty-output.txt"), "keep unfinished work\n").unwrap();
    let (_, checkout) = crate::orchestration::project_identity::resolve_directory(checkout_path, None, None).await.unwrap();
    let (source, request) = fixture.resumed_move(&checkout.path).await;
    let completed = fixture.service.bind_resumed_space(
        "session", Some(&original.operation_id), &checkout, &source, &request, &fixture.library,
    ).await.unwrap();
    assert_eq!(completed.operation_id, request.move_operation_id);
    assert_eq!(completed.state, WorkspaceOperationState::Completed);
    assert_eq!(completed.workspace_id.as_deref(), Some(request.workspace_id.as_str()));
    let receipt = fixture.service.store.read_owned_creation(&request.operation_id).unwrap().unwrap();
    assert_eq!(receipt.operation_id, request.operation_id);
    assert_eq!(receipt.moved_space.as_ref(), Some(&request));
    assert_eq!(receipt.resources, vec![OwnedResourceIdentity::Space {
        identity: cockpit_protocol::orchestration::OwnedSpaceIdentity {
            creation_operation_id: request.move_operation_id.clone(),
            endpoint_identity: source.endpoint_identity.clone(), session_id: "session".into(),
            workspace_id: request.workspace_id.clone(), checkout_path: checkout.path.clone(),
        },
    }]);
    assert!(receipt.git_capture_error.is_none());
    let resumed_journal = fixture.service.store.owned_checkout(&request.operation_id).unwrap().unwrap();
    let lineage = resumed_journal.resume_binding.as_ref().unwrap();
    assert_eq!(lineage.previous_operation_id.as_deref(), Some(original.operation_id.as_str()));
    assert_eq!(lineage.expected_checkout, checkout);
    assert_eq!(serde_json::to_value(&lineage.source_location).unwrap(), serde_json::to_value(&source).unwrap());
    assert_eq!(resumed_journal.bind.as_ref(), Some(&request));
    assert_eq!(resumed_journal.request.base, current_head);
    fixture.service.validate_owned_resource(&receipt.resources[0]).await.unwrap();
    assert_eq!(fixture.service.store.read_owned_creation(&original.operation_id).unwrap().unwrap(), original_receipt);
    assert_eq!(serde_json::to_value(fixture.service.store.load(&original.operation_id).unwrap()).unwrap(),
        serde_json::to_value(&original).unwrap());
    assert_eq!(serde_json::to_value(fixture.service.store.owned_checkout(&original.operation_id).unwrap()).unwrap(),
        original_journal);
    assert_eq!(crate::orchestration::project_identity::resolve_directory(checkout_path, None, None).await.unwrap().1, checkout);
    assert_eq!(crate::space_git::git_line(checkout_path, &["rev-parse", "HEAD"]).await.unwrap(), current_head);
    assert_eq!(crate::space_git::git_line(checkout_path, &["symbolic-ref", "--short", "HEAD"]).await.unwrap(), "worker");
    assert_eq!(std::fs::read_to_string(checkout_path.join("prior-output.txt")).unwrap(), "prior committed output\n");
    assert_eq!(std::fs::read_to_string(checkout_path.join("dirty-output.txt")).unwrap(), "keep unfinished work\n");

    let run = resumed_run(Some(&original.operation_id), &checkout, &source, &request);
    let resources = fixture.service.owned_resources_for_run(&run).await.unwrap();
    let mut expected: Vec<_> = original_receipt.resources.iter().filter(|resource|
        matches!(resource, OwnedResourceIdentity::Worktree { .. }
            | OwnedResourceIdentity::Branch { .. } | OwnedResourceIdentity::AttemptDirectory { .. }))
        .cloned().collect();
    expected.extend(receipt.resources.clone());
    assert_eq!(resources, expected);
    assert_eq!(resources.iter().filter(|r| matches!(r, OwnedResourceIdentity::Worktree { .. })).count(), 1);
    assert_eq!(resources.iter().filter(|r| matches!(r, OwnedResourceIdentity::Branch { .. })).count(), 1);
    assert_eq!(resources.iter().filter(|r| matches!(r, OwnedResourceIdentity::Space { .. })).count(), 1);
    assert!(!resources.iter().any(|r|
        matches!(r, OwnedResourceIdentity::Space { identity } if identity.workspace_id == "moved")));
    for resource in &original_receipt.resources {
        if let OwnedResourceIdentity::Worktree { identity } | OwnedResourceIdentity::Branch { identity } = resource {
            assert_eq!(identity.creation_operation_id, original.operation_id);
        }
    }
    assert!(original_receipt.resources.iter().any(|r|
        matches!(r, OwnedResourceIdentity::Space { identity } if identity.workspace_id == "moved")));
    let repeated = fixture.service.bind_resumed_space(
        "session", Some(&original.operation_id), &checkout, &source, &request, &fixture.library,
    ).await.unwrap();
    assert_eq!(serde_json::to_value(&repeated).unwrap(), serde_json::to_value(&completed).unwrap());
    let restarted = ProjectService::new(fixture.config.clone(), fixture.adapter.clone()).unwrap();
    let recovered = restarted.bind_resumed_space(
        "session", Some(&original.operation_id), &checkout, &source, &request, &fixture.library,
    ).await.unwrap();
    assert_eq!(serde_json::to_value(recovered).unwrap(), serde_json::to_value(completed).unwrap());
    assert_eq!(restarted.owned_resources_for_run(&run).await.unwrap(), expected);
    assert_eq!(restarted.store.read_owned_creation(&request.operation_id).unwrap().unwrap(), receipt);
    assert_eq!(serde_json::to_value(restarted.store.owned_checkout(&request.operation_id).unwrap().unwrap()).unwrap(),
        serde_json::to_value(resumed_journal).unwrap());
    fixture.no_shells();
}

#[tokio::test]
async fn resumed_binding_borrowed_without_original_operation_never_adopts_filesystem_ownership() {
    let fixture = Fixture::new().await;
    let directory = fixture.root.join("existing-user-directory");
    std::fs::create_dir(&directory).unwrap();
    std::fs::write(directory.join("keep.txt"), "existing user data\n").unwrap();
    let (_, checkout) = crate::orchestration::project_identity::resolve_directory(&directory, None, None).await.unwrap();
    let (source, request) = fixture.resumed_move(&checkout.path).await;
    assert!(fixture.service.store.list().unwrap().is_empty());
    let root_identity = crate::orchestration::project_identity::resolve_directory(&fixture.root, None, None).await.unwrap().1;
    let mut pending = resumed_run(None, &checkout, &source, &request);
    pending.setup.as_mut().unwrap().workspace_id = Some("missing-predecessor".into());
    for step in [cockpit_protocol::orchestration::DispatchStep::MoveIntent,
        cockpit_protocol::orchestration::DispatchStep::MoveUnknown]
    {
        pending.dispatch.as_mut().unwrap().step = step;
        assert!(fixture.service.owned_resources_for_run(&pending).await.unwrap().is_empty(),
            "pending borrowed placement owns neither user filesystem nor an unbound Space");
        assert!(fixture.service.owned_creation_limitation_for_run(&pending).await.unwrap().is_none());
    }
    let completed = fixture.service.bind_resumed_space(
        "session", None, &checkout, &source, &request, &fixture.library,
    ).await.unwrap();
    let receipt = fixture.service.store.read_owned_creation(&request.operation_id).unwrap().unwrap();
    assert_eq!(receipt.resources.len(), 1);
    assert!(matches!(&receipt.resources[0], OwnedResourceIdentity::Space { identity }
        if identity.creation_operation_id == request.move_operation_id
            && identity.workspace_id == request.workspace_id && identity.checkout_path == checkout.path));
    assert_eq!(receipt.moved_space.as_ref(), Some(&request));
    assert!(receipt.git_capture_error.is_none());
    let resumed_journal = fixture.service.store.owned_checkout(&request.operation_id).unwrap().unwrap();
    let lineage = resumed_journal.resume_binding.as_ref().unwrap();
    assert!(lineage.previous_operation_id.is_none());
    assert_eq!(lineage.expected_checkout, checkout);
    assert_eq!(serde_json::to_value(&lineage.source_location).unwrap(), serde_json::to_value(&source).unwrap());
    assert_eq!(resumed_journal.bind.as_ref(), Some(&request));
    assert!(!resumed_journal.git_requested);
    let run = resumed_run(None, &checkout, &source, &request);
    assert_eq!(fixture.service.owned_resources_for_run(&run).await.unwrap(), receipt.resources);
    assert_eq!(fixture.service.store.list().unwrap().len(), 1);
    assert_eq!(crate::orchestration::project_identity::resolve_directory(&directory, None, None).await.unwrap().1, checkout);
    assert_eq!(crate::orchestration::project_identity::resolve_directory(&fixture.root, None, None).await.unwrap().1, root_identity);
    assert_eq!(std::fs::read_to_string(directory.join("keep.txt")).unwrap(), "existing user data\n");
    assert!(!directory.join(".git").exists());
    let restarted = ProjectService::new(fixture.config.clone(), fixture.adapter.clone()).unwrap();
    let retry = restarted.bind_resumed_space("session", None, &checkout, &source, &request, &fixture.library).await.unwrap();
    assert_eq!(serde_json::to_value(retry).unwrap(), serde_json::to_value(completed).unwrap());
    assert_eq!(restarted.owned_resources_for_run(&run).await.unwrap(), receipt.resources);
    assert_eq!(serde_json::to_value(restarted.store.owned_checkout(&request.operation_id).unwrap().unwrap()).unwrap(),
        serde_json::to_value(resumed_journal).unwrap());
    assert_eq!(fixture.adapter.inventory_calls.load(Ordering::SeqCst), 0);
    assert!(fixture.adapter.worktree_requests.lock().unwrap().is_empty());
    fixture.no_shells();
}

#[tokio::test]
async fn resumed_binding_rejects_wrong_source_peer_terminal_tag_and_response_endpoint() {
    let fixture = Fixture::new().await;
    let original = fixture.original_bound_checkout().await;
    let (_, checkout) = crate::orchestration::project_identity::resolve_directory(
        Path::new(&original.plan.checkout_path), None, None,
    ).await.unwrap();
    let (source, request) = fixture.resumed_move(&checkout.path).await;
    let mut changed_sources = Vec::new();
    let mut wrong = source.clone();
    wrong.peer_process.as_mut().unwrap().start_ticks += 1;
    changed_sources.push(wrong);
    let mut wrong = source.clone();
    wrong.peer_process.as_mut().unwrap().pid += 1;
    changed_sources.push(wrong);
    let mut wrong = source.clone();
    wrong.peer_process.as_mut().unwrap().kernel_boot_id = Some(uuid::Uuid::new_v4().to_string());
    changed_sources.push(wrong);
    let mut wrong = source.clone();
    wrong.peer_process.as_mut().unwrap().kernel_boot_id = Some("not-a-kernel-uuid".into());
    changed_sources.push(wrong);
    let mut wrong = source.clone();
    wrong.peer_process = None;
    changed_sources.push(wrong);
    let mut wrong = source.clone();
    wrong.terminal_id = Some("foreign-terminal".into());
    changed_sources.push(wrong);
    let mut wrong = source.clone();
    wrong.launch_tag = "foreign-launch-tag".into();
    changed_sources.push(wrong);
    for wrong in changed_sources {
        assert!(fixture.service.bind_resumed_space(
            "session", Some(&original.operation_id), &checkout, &wrong, &request, &fixture.library,
        ).await.is_err());
        assert!(fixture.service.store.read_owned_creation(&request.operation_id).unwrap().is_none());
    }
    let observed = fixture.adapter.bound_snapshot.lock().clone().unwrap();
    let mut wrong_endpoint = observed.clone();
    wrong_endpoint.endpoint_identity = "foreign-endpoint".into();
    *fixture.adapter.bound_snapshot.lock() = Some(wrong_endpoint);
    assert!(fixture.service.bind_resumed_space(
        "session", Some(&original.operation_id), &checkout, &source, &request, &fixture.library,
    ).await.is_err());
    let mut wrong_peer = observed.clone();
    wrong_peer.peer_process.as_mut().unwrap().start_ticks += 1;
    *fixture.adapter.bound_snapshot.lock() = Some(wrong_peer);
    assert!(fixture.service.bind_resumed_space(
        "session", Some(&original.operation_id), &checkout, &source, &request, &fixture.library,
    ).await.is_err());
    *fixture.adapter.bound_snapshot.lock() = Some(observed);
    assert!(fixture.service.store.read_owned_creation(&request.operation_id).unwrap().is_none());
    fixture.no_shells();
}

#[tokio::test]
async fn resumed_binding_rejects_foreign_checkout_device_inode_and_git_admin_identity() {
    let fixture = Fixture::new().await;
    let original = fixture.original_bound_checkout().await;
    let receipt = fixture.service.store.read_owned_creation(&original.operation_id).unwrap().unwrap();
    let (_, checkout) = crate::orchestration::project_identity::resolve_directory(
        Path::new(&original.plan.checkout_path), None, None,
    ).await.unwrap();
    let (source, request) = fixture.resumed_move(&checkout.path).await;
    let foreign_directory = fixture.root.join("foreign-directory");
    std::fs::create_dir(&foreign_directory).unwrap();
    let (_, foreign) = crate::orchestration::project_identity::resolve_directory(
        &foreign_directory, None, None,
    ).await.unwrap();
    let mut changed_checkouts = vec![foreign];
    let mut wrong = checkout.clone();
    wrong.directory_identity.device = format!("{}0", wrong.directory_identity.device);
    changed_checkouts.push(wrong);
    let mut wrong = checkout.clone();
    wrong.directory_identity.inode = format!("{}0", wrong.directory_identity.inode);
    changed_checkouts.push(wrong);
    let mut wrong = checkout.clone();
    wrong.git.as_mut().unwrap().common_dir = foreign_directory.to_string_lossy().into_owned();
    changed_checkouts.push(wrong);
    let mut wrong = checkout.clone();
    wrong.git.as_mut().unwrap().worktree_git_dir = foreign_directory.to_string_lossy().into_owned();
    changed_checkouts.push(wrong);
    let mut wrong = checkout.clone();
    wrong.key = "foreign-checkout-key".into();
    changed_checkouts.push(wrong);
    for wrong in changed_checkouts {
        assert!(fixture.service.bind_resumed_space(
            "session", Some(&original.operation_id), &wrong, &source, &request, &fixture.library,
        ).await.is_err());
        assert!(fixture.service.store.read_owned_creation(&request.operation_id).unwrap().is_none());
    }
    assert_eq!(fixture.service.store.read_owned_creation(&original.operation_id).unwrap().unwrap(), receipt);
    fixture.no_shells();
}

#[tokio::test]
async fn resumed_binding_pins_current_head_and_branch_across_exact_retry() {
    for change_branch in [false, true] {
        let fixture = Fixture::new().await;
        let original = fixture.original_bound_checkout().await;
        let path = Path::new(&original.plan.checkout_path);
        let (_, checkout) = crate::orchestration::project_identity::resolve_directory(path, None, None).await.unwrap();
        let (source, request) = fixture.resumed_move(&checkout.path).await;
        fixture.service.bind_resumed_space(
            "session", Some(&original.operation_id), &checkout, &source, &request, &fixture.library,
        ).await.unwrap();
        let receipt = fixture.service.store.read_owned_creation(&request.operation_id).unwrap().unwrap();
        if change_branch {
            git(path, &["switch", "-c", "foreign-branch"]);
        } else {
            git(path, &["-c", "user.name=Fixture", "-c", "user.email=fixture@example.invalid",
                "commit", "--allow-empty", "-m", "HEAD changed after binding"]);
        }
        let restarted = ProjectService::new(fixture.config.clone(), fixture.adapter.clone()).unwrap();
        assert!(restarted.bind_resumed_space(
            "session", Some(&original.operation_id), &checkout, &source, &request, &fixture.library,
        ).await.is_err());
        assert_eq!(restarted.store.read_owned_creation(&request.operation_id).unwrap().unwrap(), receipt);
        fixture.no_shells();
    }
}

#[tokio::test]
async fn resumed_binding_rejects_same_path_physical_checkout_replacement() {
    let fixture = Fixture::new().await;
    let original = fixture.original_bound_checkout().await;
    let path = Path::new(&original.plan.checkout_path);
    std::fs::write(path.join("keep.txt"), "original physical checkout\n").unwrap();
    let (_, checkout) = crate::orchestration::project_identity::resolve_directory(path, None, None).await.unwrap();
    let (source, request) = fixture.resumed_move(&checkout.path).await;
    let original_receipt = fixture.service.store.read_owned_creation(&original.operation_id).unwrap().unwrap();
    let retained = fixture.root.join("retained-original-checkout");
    std::fs::rename(path, &retained).unwrap();
    std::fs::create_dir(path).unwrap();
    std::fs::copy(retained.join(".git"), path.join(".git")).unwrap();
    std::fs::write(path.join("replacement.txt"), "foreign physical directory\n").unwrap();
    let actual = crate::orchestration::project_identity::resolve_directory(path, None, None).await.unwrap().1;
    assert_ne!(actual.directory_identity, checkout.directory_identity);
    assert!(fixture.service.bind_resumed_space(
        "session", Some(&original.operation_id), &checkout, &source, &request, &fixture.library,
    ).await.is_err());
    assert_eq!(fixture.service.store.read_owned_creation(&original.operation_id).unwrap().unwrap(), original_receipt);
    assert!(fixture.service.store.read_owned_creation(&request.operation_id).unwrap().is_none());
    assert_eq!(std::fs::read_to_string(retained.join("keep.txt")).unwrap(), "original physical checkout\n");
    assert_eq!(std::fs::read_to_string(path.join("replacement.txt")).unwrap(), "foreign physical directory\n");
    fixture.no_shells();
}

#[tokio::test]
async fn resumed_binding_cannot_rebind_original_operation_or_change_retained_lineage() {
    let fixture = Fixture::new().await;
    let original = fixture.original_bound_checkout().await;
    let receipt = fixture.service.store.read_owned_creation(&original.operation_id).unwrap().unwrap();
    let (_, checkout) = crate::orchestration::project_identity::resolve_directory(
        Path::new(&original.plan.checkout_path), None, None,
    ).await.unwrap();
    let (source, request) = fixture.resumed_move(&checkout.path).await;
    let mut old_rebind = request.clone();
    old_rebind.operation_id = original.operation_id.clone();
    old_rebind.expected_generation = original.generation;
    assert_eq!(fixture.service.bind_moved_space("session", &old_rebind, &fixture.library)
        .await.unwrap_err().code, "association_conflict");
    old_rebind.expected_generation = 1;
    assert_eq!(fixture.service.bind_resumed_space(
        "session", Some(&original.operation_id), &checkout, &source, &old_rebind, &fixture.library,
    ).await.unwrap_err().code, "association_conflict");
    fixture.service.bind_resumed_space(
        "session", Some(&original.operation_id), &checkout, &source, &request, &fixture.library,
    ).await.unwrap();
    assert_eq!(fixture.service.bind_resumed_space(
        "session", None, &checkout, &source, &request, &fixture.library,
    ).await.unwrap_err().code, "association_conflict");
    let mut changed_source = source.clone();
    changed_source.workspace_id = "different-source-space".into();
    assert_eq!(fixture.service.bind_resumed_space(
        "session", Some(&original.operation_id), &checkout, &changed_source, &request, &fixture.library,
    ).await.unwrap_err().code, "association_conflict");
    for changed_source in [
        cockpit_protocol::orchestration::RunLocation { tab_id: "other-waiting-tab".into(), ..source.clone() },
        cockpit_protocol::orchestration::RunLocation { pane_id: "other-waiting-pane".into(), ..source.clone() },
    ] {
        assert_eq!(fixture.service.bind_resumed_space(
            "session", Some(&original.operation_id), &checkout, &changed_source, &request, &fixture.library,
        ).await.unwrap_err().code, "association_conflict");
    }
    assert_eq!(fixture.service.store.read_owned_creation(&original.operation_id).unwrap().unwrap(), receipt);
    assert_eq!(serde_json::to_value(fixture.service.store.load(&original.operation_id).unwrap()).unwrap(),
        serde_json::to_value(original).unwrap());
    fixture.no_shells();
}

#[tokio::test]
async fn resumed_binding_resource_aggregation_refuses_wrong_creator_move_workspace_and_checkout() {
    let fixture = Fixture::new().await;
    let original = fixture.original_bound_checkout().await;
    let (_, checkout) = crate::orchestration::project_identity::resolve_directory(
        Path::new(&original.plan.checkout_path), None, None,
    ).await.unwrap();
    let (source, request) = fixture.resumed_move(&checkout.path).await;
    fixture.service.bind_resumed_space(
        "session", Some(&original.operation_id), &checkout, &source, &request, &fixture.library,
    ).await.unwrap();
    let run = resumed_run(Some(&original.operation_id), &checkout, &source, &request);
    let mut changed_runs = Vec::new();
    let mut wrong = run.clone();
    wrong.setup.as_mut().unwrap().operation_id = None;
    changed_runs.push(wrong);
    let mut wrong = run.clone();
    wrong.setup.as_mut().unwrap().operation_id = Some(uuid::Uuid::new_v4().to_string());
    changed_runs.push(wrong);
    let mut wrong = run.clone();
    wrong.setup.as_mut().unwrap().workspace_id = Some("foreign-space".into());
    changed_runs.push(wrong);
    let mut wrong = run.clone();
    wrong.dispatch.as_mut().unwrap().move_operation_id = Some(uuid::Uuid::new_v4().to_string());
    changed_runs.push(wrong);
    let mut wrong = run.clone();
    wrong.dispatch.as_mut().unwrap().move_operation_id = None;
    changed_runs.push(wrong);
    let mut wrong = run.clone();
    wrong.checkout.as_mut().unwrap().directory_identity.inode = "foreign-inode".into();
    changed_runs.push(wrong);
    let mut wrong = run.clone();
    wrong.checkout = None;
    changed_runs.push(wrong);
    for wrong in changed_runs {
        assert!(fixture.service.owned_resources_for_run(&wrong).await.is_err());
    }
    fixture.no_shells();
}

#[tokio::test]
async fn resumed_binding_rejects_owned_branch_changed_before_new_epoch_binding() {
    let fixture = Fixture::new().await;
    let original = fixture.original_bound_checkout().await;
    let receipt = fixture.service.store.read_owned_creation(&original.operation_id).unwrap().unwrap();
    let path = Path::new(&original.plan.checkout_path);
    let (_, checkout) = crate::orchestration::project_identity::resolve_directory(path, None, None).await.unwrap();
    let (source, request) = fixture.resumed_move(&checkout.path).await;
    git(path, &["switch", "-c", "unrelated-branch"]);
    assert!(fixture.service.bind_resumed_space(
        "session", Some(&original.operation_id), &checkout, &source, &request, &fixture.library,
    ).await.is_err());
    assert!(fixture.service.store.read_owned_creation(&request.operation_id).unwrap().is_none());
    assert_eq!(fixture.service.store.read_owned_creation(&original.operation_id).unwrap().unwrap(), receipt);
    assert_eq!(crate::space_git::git_line(path, &["symbolic-ref", "--short", "HEAD"]).await.unwrap(), "unrelated-branch");
    fixture.no_shells();
}

#[tokio::test]
async fn resumed_binding_borrowed_lineage_cannot_claim_an_original_creator() {
    let fixture = Fixture::new().await;
    let directory = fixture.root.join("borrowed-lineage-directory");
    std::fs::create_dir(&directory).unwrap();
    std::fs::write(directory.join("keep.txt"), "borrowed\n").unwrap();
    let (_, checkout) = crate::orchestration::project_identity::resolve_directory(&directory, None, None).await.unwrap();
    let (source, request) = fixture.resumed_move(&checkout.path).await;
    fixture.service.bind_resumed_space(
        "session", None, &checkout, &source, &request, &fixture.library,
    ).await.unwrap();
    let run = resumed_run(None, &checkout, &source, &request);
    let resources = fixture.service.owned_resources_for_run(&run).await.unwrap();
    assert_eq!(resources.len(), 1);
    assert!(matches!(&resources[0], OwnedResourceIdentity::Space { identity }
        if identity.creation_operation_id == request.operation_id));
    let mut wrong = run;
    wrong.setup.as_mut().unwrap().operation_id = Some(request.operation_id.clone());
    assert_eq!(fixture.service.owned_resources_for_run(&wrong).await.unwrap_err().code, "association_conflict");
    assert_eq!(fixture.service.bind_resumed_space(
        "session", Some(&request.operation_id), &checkout, &source, &request, &fixture.library,
    ).await.unwrap_err().code, "association_conflict");
    assert_eq!(std::fs::read_to_string(directory.join("keep.txt")).unwrap(), "borrowed\n");
    assert!(!directory.join(".git").exists());
    fixture.no_shells();
}

#[tokio::test]
async fn resumed_binding_borrowed_same_path_replacement_is_not_adopted_after_restart() {
    let fixture = Fixture::new().await;
    let directory = fixture.root.join("borrowed-replacement-directory");
    std::fs::create_dir(&directory).unwrap();
    std::fs::write(directory.join("keep.txt"), "retained original directory\n").unwrap();
    let (_, checkout) = crate::orchestration::project_identity::resolve_directory(&directory, None, None).await.unwrap();
    let (source, request) = fixture.resumed_move(&checkout.path).await;
    fixture.service.bind_resumed_space(
        "session", None, &checkout, &source, &request, &fixture.library,
    ).await.unwrap();
    let receipt = fixture.service.store.read_owned_creation(&request.operation_id).unwrap().unwrap();
    let retained = fixture.root.join("retained-borrowed-directory");
    std::fs::rename(&directory, &retained).unwrap();
    std::fs::create_dir(&directory).unwrap();
    std::fs::write(directory.join("foreign.txt"), "replacement user directory\n").unwrap();
    let replacement = crate::orchestration::project_identity::resolve_directory(&directory, None, None).await.unwrap().1;
    assert_ne!(replacement.directory_identity, checkout.directory_identity);
    let restarted = ProjectService::new(fixture.config.clone(), fixture.adapter.clone()).unwrap();
    assert!(restarted.bind_resumed_space(
        "session", None, &checkout, &source, &request, &fixture.library,
    ).await.is_err());
    assert_eq!(restarted.store.read_owned_creation(&request.operation_id).unwrap().unwrap(), receipt);
    assert_eq!(std::fs::read_to_string(retained.join("keep.txt")).unwrap(), "retained original directory\n");
    assert_eq!(std::fs::read_to_string(directory.join("foreign.txt")).unwrap(), "replacement user directory\n");
    assert!(!directory.join(".git").exists());
    fixture.no_shells();
}

#[tokio::test]
async fn moved_git_space_registers_existing_worktree_provenance_and_retries_without_new_resources() {
    let fixture = Fixture::new().await;
    let prepared = fixture.service.prepare_owned_checkout("session", &fixture.request, &fixture.library).await.unwrap();
    fixture.move_snapshot(&prepared.plan.checkout_path).await;
    let before = fixture.adapter.snapshot.lock().clone().unwrap();
    let binding = fixture.bind(&prepared);
    let completed = fixture.service.bind_moved_space("session", &binding, &fixture.library).await.unwrap();
    assert_eq!(completed.state, WorkspaceOperationState::Completed);
    let after = fixture.adapter.session_snapshot("session").await.unwrap();
    assert_eq!((after.spaces.len(), after.tabs.len(), after.panes.len()),
        (before.spaces.len(), before.tabs.len(), before.panes.len()));
    assert_eq!(after.tabs, before.tabs);
    assert_eq!(after.panes, before.panes);
    let space = after.spaces.iter().find(|space| space.id == binding.workspace_id).unwrap();
    assert_eq!(space.label, "Worker");
    assert!(moved_space_registered(space, prepared.plan.repository.as_ref().unwrap(), &prepared.plan.checkout_path));
    {
        let requests = fixture.adapter.worktree_requests.lock().unwrap();
        assert_eq!(requests.len(), 1);
        let request = &requests[0];
        assert_eq!(request.mode, WorkspaceSetupMode::Open);
        assert!(request.open_existing_worktree);
        assert_eq!(request.endpoint_identity, binding.endpoint_identity);
        assert_eq!(request.source_cwd, fixture.request.repository_checkout);
        assert_eq!(request.checkout_path, prepared.plan.checkout_path);
        assert_eq!(request.label, "Worker");
        assert!(!request.focus);
        assert!(request.branch.is_none() && request.base.is_none() && request.env.is_empty());
    }
    let repeated = fixture.service.bind_moved_space("session", &binding, &fixture.library).await.unwrap();
    assert_eq!(repeated.generation, completed.generation);
    // Crash after registration but before completion needs no new mutation or journal field.
    fixture.service.store.update(&prepared.operation_id, None, |operation| {
        operation.state = WorkspaceOperationState::Partial;
        operation.step = WorkspaceOperationStep::SpaceBound;
        Ok(())
    }).unwrap();
    *fixture.adapter.inventory.lock() = None;
    let restarted = ProjectService::new(fixture.config.clone(), fixture.adapter.clone()).unwrap();
    assert_eq!(restarted.bind_moved_space("session", &binding, &fixture.library).await.unwrap().state,
        WorkspaceOperationState::Completed);
    assert_eq!(fixture.adapter.worktree_requests.lock().unwrap().len(), 1);
    fixture.no_shells();
}

#[tokio::test]
async fn unprovable_registration_preflight_is_partial_without_herdr_mutation() {
    for case in ["wrong-target", "closed-primary-wrong-target", "missing-target", "duplicate-target", "stale-primary-space", "missing-primary",
        "duplicate-primary", "foreign-repository", "foreign-root", "foreign-endpoint", "unavailable-inventory",
        "ambiguous-cwd", "empty-label", "nonlinked-target"]
    {
        let fixture = Fixture::new().await;
        let prepared = fixture.service.prepare_owned_checkout("session", &fixture.request, &fixture.library).await.unwrap();
        fixture.move_snapshot(&prepared.plan.checkout_path).await;
        let binding = fixture.bind(&prepared);
        {
            let mut retained = fixture.adapter.inventory.lock();
            let inventory = retained.as_mut().unwrap();
            match case {
                "wrong-target" | "closed-primary-wrong-target" => {
                    inventory.worktrees[1].open_workspace_id = Some("foreign-space".into());
                    if case == "closed-primary-wrong-target" { inventory.worktrees[0].open_workspace_id = None; }
                },
                "missing-target" => { inventory.worktrees.pop(); },
                "duplicate-target" => inventory.worktrees.push(inventory.worktrees[1].clone()),
                "stale-primary-space" => inventory.worktrees[0].open_workspace_id = Some("missing-space".into()),
                "missing-primary" => { inventory.worktrees.remove(0); },
                "duplicate-primary" => inventory.worktrees.push(inventory.worktrees[0].clone()),
                "foreign-repository" => inventory.repository_key = "/foreign/.git".into(),
                "foreign-root" => inventory.repository_root = "/foreign".into(),
                "foreign-endpoint" => inventory.endpoint_identity = "foreign-endpoint".into(),
                "unavailable-inventory" => *retained = None,
                "nonlinked-target" => inventory.worktrees[1].is_linked_worktree = false,
                "ambiguous-cwd" => {
                    let mut snapshot = fixture.adapter.snapshot.lock();
                    let snapshot = snapshot.as_mut().unwrap();
                    snapshot.panes.push(PaneSummary {
                        id: "workspace:foreign:p1".into(), terminal_id: "foreign-terminal".into(),
                        space_id: "workspace".into(), tab_id: "workspace:foreign".into(),
                        title: None, focused: false, agent: None, agent_status: "none".into(), revision: 1,
                        cwd: Some(prepared.plan.checkout_path.clone()),
                    });
                },
                "empty-label" => fixture.adapter.snapshot.lock().as_mut().unwrap().spaces
                    .iter_mut().find(|space| space.id == binding.workspace_id).unwrap().label.clear(),
                _ => unreachable!(),
            }
        }
        let error = fixture.service.bind_moved_space("session", &binding, &fixture.library).await.unwrap_err();
        assert_eq!(error.code, if case == "unavailable-inventory" { "test_adapter_unused" } else { "association_conflict" }, "{case}");
        let operation = fixture.service.store.load(&prepared.operation_id).unwrap();
        assert_eq!(operation.state, WorkspaceOperationState::Partial, "{case}");
        assert_eq!(operation.step, WorkspaceOperationStep::SpaceBound, "{case}");
        assert!(!operation.resume_allowed, "{case}");
        assert!(fixture.adapter.worktree_requests.lock().unwrap().is_empty(), "{case}");
        assert!(fixture.adapter.remove_requests.lock().is_empty(), "{case}");
        assert!(Path::new(&prepared.plan.checkout_path).is_dir(), "{case}");
        fixture.no_shells();
    }
}

#[tokio::test]
async fn contradicting_registration_result_is_partial_without_cleanup() {
    for case in ["new-space", "wrong-space", "wrong-tab", "wrong-pane", "wrong-checkout",
        "wrong-git-key", "wrong-git-checkout", "not-linked"]
    {
        let fixture = Fixture::new().await;
        let prepared = fixture.service.prepare_owned_checkout("session", &fixture.request, &fixture.library).await.unwrap();
        fixture.move_snapshot(&prepared.plan.checkout_path).await;
        let binding = fixture.bind(&prepared);
        let repository = prepared.plan.repository.as_ref().unwrap();
        let mut result = ProjectWorktreeResult {
            workspace_id: binding.workspace_id.clone(), tab_id: Some(binding.tab_id.clone()),
            pane_id: Some(binding.pane_id.clone()), checkout_path: prepared.plan.checkout_path.clone(),
            branch: Some(fixture.request.branch.clone()), already_open: true,
        };
        let mut git = SpaceGitSummary {
            repository_key: repository.common_dir.clone(), repository: repository.name.clone(),
            branch: Some(fixture.request.branch.clone()), checkout_path: prepared.plan.checkout_path.clone(),
            is_linked_worktree: true,
        };
        match case {
            "new-space" => result.already_open = false,
            "wrong-space" => result.workspace_id = "foreign-space".into(),
            "wrong-tab" => result.tab_id = Some("foreign-tab".into()),
            "wrong-pane" => result.pane_id = Some("foreign-pane".into()),
            "wrong-checkout" => result.checkout_path = fixture.request.repository_checkout.clone(),
            "wrong-git-key" => git.repository_key = "/foreign/.git".into(),
            "wrong-git-checkout" => git.checkout_path = fixture.request.repository_checkout.clone(),
            "not-linked" => git.is_linked_worktree = false,
            _ => unreachable!(),
        }
        *fixture.adapter.open_existing.lock() = Some((result, git));
        assert_eq!(fixture.service.bind_moved_space("session", &binding, &fixture.library).await.unwrap_err().code,
            "association_conflict", "{case}");
        let partial = fixture.service.store.load(&prepared.operation_id).unwrap();
        assert_eq!(partial.state, WorkspaceOperationState::Partial, "{case}");
        assert_eq!(partial.step, WorkspaceOperationStep::SpaceBound, "{case}");
        assert!(!partial.resume_allowed, "{case}");
        assert_eq!(fixture.adapter.worktree_requests.lock().unwrap().len(), 1, "{case}");
        assert!(fixture.adapter.remove_requests.lock().is_empty(), "{case}");
        let receipt = fixture.service.store.read_owned_creation(&prepared.operation_id).unwrap().unwrap();
        assert_eq!(receipt.moved_space.as_ref(), Some(&binding), "{case}");
        assert!(Path::new(&prepared.plan.checkout_path).is_dir(), "{case}");
        fixture.no_shells();
    }
}

#[tokio::test]
async fn resumed_git_space_registers_existing_checkout_despite_borrowed_directory_journal() {
    let fixture = Fixture::new().await;
    let original = fixture.original_bound_checkout().await;
    let original_receipt = fixture.service.store.read_owned_creation(&original.operation_id).unwrap().unwrap();
    fixture.adapter.worktree_requests.lock().unwrap().clear();
    let (_, checkout) = crate::orchestration::project_identity::resolve_directory(
        Path::new(&original.plan.checkout_path), None, None,
    ).await.unwrap();
    let (source, request) = fixture.resumed_move(&checkout.path).await;
    let completed = fixture.service.bind_resumed_space(
        "session", Some(&original.operation_id), &checkout, &source, &request, &fixture.library,
    ).await.unwrap();
    assert_eq!(completed.state, WorkspaceOperationState::Completed);
    assert!(fixture.service.store.owned_checkout(&completed.operation_id).unwrap().unwrap().borrowed_directory);
    let snapshot = &fixture.adapter.bound_snapshot.lock().clone().unwrap().snapshot;
    assert!(snapshot.spaces.iter().any(|space| space.id == request.workspace_id
        && moved_space_registered(space, original.plan.repository.as_ref().unwrap(), &checkout.path)));
    let requests = fixture.adapter.worktree_requests.lock().unwrap();
    assert_eq!(requests.len(), 1);
    assert_eq!(requests[0].source_cwd, fixture.request.repository_checkout);
    assert_eq!(requests[0].checkout_path, checkout.path);
    assert_eq!(requests[0].label, "Resumed worker");
    drop(requests);
    assert_eq!(fixture.service.store.read_owned_creation(&original.operation_id).unwrap().unwrap(), original_receipt);
    fixture.no_shells();
}

#[tokio::test]
async fn primary_and_unrelated_git_placements_do_not_register_worktrees() {
    let fixture = Fixture::new().await;
    // Git directories borrowed without a Git-owning predecessor remain unrelated:
    // their resumed placement must not infer new repository associations.
    let (_, checkout) = crate::orchestration::project_identity::resolve_directory(
        Path::new(&fixture.request.repository_checkout), None, None,
    ).await.unwrap();
    let (source, request) = fixture.resumed_move(&checkout.path).await;
    let completed = fixture.service.bind_resumed_space(
        "session", None, &checkout, &source, &request, &fixture.library,
    ).await.unwrap();
    assert_eq!(completed.state, WorkspaceOperationState::Completed);
    assert_eq!(fixture.adapter.inventory_calls.load(Ordering::SeqCst), 0);
    assert!(fixture.adapter.worktree_requests.lock().unwrap().is_empty());
    // A retained primary repository plan is also a non-registration placement.
    let repository = RepositoryCatalog::new(fixture.config.clone()).list().await.unwrap()
        .repositories.into_iter().find(|repository| !repository.is_linked_worktree).unwrap();
    let mut primary = completed.clone();
    primary.plan.repository = Some(repository);
    let journal = fixture.service.store.owned_checkout(&completed.operation_id).unwrap().unwrap();
    let observed = fixture.adapter.snapshot.lock().clone().unwrap();
    fixture.service.register_moved_worktree("session", &primary, &journal, &request, &observed).await.unwrap();
    assert_eq!(fixture.adapter.inventory_calls.load(Ordering::SeqCst), 0);
    assert!(fixture.adapter.worktree_requests.lock().unwrap().is_empty());
    fixture.no_shells();
}

#[tokio::test]
async fn isolated_git_placement_without_open_primary_skips_only_registration_without_new_resources() {
    for source in ["unrelated-staging-space", "linked-only", "linked-with-open-primary"] {
        let fixture = Fixture::new().await;
        let prepared = fixture.service.prepare_owned_checkout("session", &fixture.request, &fixture.library).await.unwrap();
        fixture.move_snapshot(&prepared.plan.checkout_path).await;
        let binding = fixture.bind(&prepared);
        let repository = prepared.plan.repository.as_ref().unwrap();
        fixture.adapter.inventory.lock().as_mut().unwrap().worktrees[0].open_workspace_id = None;
        if source != "unrelated-staging-space" {
            let linked_path = fixture.root.join("worktrees/source-linked");
            git(Path::new(&repository.checkout_path), &["worktree", "add", "--detach",
                linked_path.to_str().unwrap(), &fixture.request.base]);
            let linked_path = linked_path.to_string_lossy().into_owned();
            let mut snapshot = fixture.adapter.snapshot.lock();
            let snapshot = snapshot.as_mut().unwrap();
            let source_space = snapshot.spaces.iter_mut().find(|space| space.id == "workspace").unwrap();
            source_space.git = Some(SpaceGitSummary {
                repository_key: repository.common_dir.clone(), repository: repository.name.clone(),
                branch: None, checkout_path: linked_path.clone(), is_linked_worktree: true,
            });
            source_space.tab_count = 1;
            source_space.pane_count = 1;
            snapshot.tabs.push(TabSummary { id: "workspace:source".into(), space_id: "workspace".into(),
                label: "Source".into(), number: 1, pane_count: 1, focused: false,
                focused_pane_id: Some("workspace:source:p1".into()) });
            snapshot.panes.push(PaneSummary { id: "workspace:source:p1".into(), terminal_id: "source-terminal".into(),
                space_id: "workspace".into(), tab_id: "workspace:source".into(), title: None, focused: false,
                agent: None, agent_status: "none".into(), revision: 1, cwd: Some(linked_path.clone()) });
            let mut inventory = fixture.adapter.inventory.lock();
            let inventory = inventory.as_mut().unwrap();
            inventory.worktrees.push(ProjectWorktreeEntry {
                checkout_path: linked_path, branch: None, open_workspace_id: Some("workspace".into()),
                is_primary: false, is_linked_worktree: true, dirty: Some(false),
            });
            if source == "linked-with-open-primary" {
                snapshot.spaces.push(SpaceSummary { id: "primary".into(), label: "Primary".into(), number: 3,
                    tab_count: 0, pane_count: 0, focused: false, agent_status: "none".into(),
                    git: Some(SpaceGitSummary {
                        repository_key: repository.common_dir.clone(), repository: repository.name.clone(),
                        branch: None, checkout_path: repository.root.clone(), is_linked_worktree: false,
                    }) });
                inventory.worktrees[0].open_workspace_id = Some("primary".into());
            }
        }
        let before = fixture.adapter.snapshot.lock().clone().unwrap();
        let completed = fixture.service.bind_moved_space("session", &binding, &fixture.library).await.unwrap();
        assert_eq!(completed.state, WorkspaceOperationState::Completed, "{source}");
        let after = fixture.adapter.session_snapshot("session").await.unwrap();
        assert_eq!((after.spaces.len(), after.tabs.len(), after.panes.len()),
            (before.spaces.len(), before.tabs.len(), before.panes.len()), "{source}");
        assert_eq!(after.tabs, before.tabs, "{source}");
        assert_eq!(after.panes, before.panes, "{source}");
        assert_eq!(fixture.adapter.inventory_calls.load(Ordering::SeqCst), 1, "{source}");
        assert_eq!(&*fixture.adapter.inventory_requests.lock(), &[repository.checkout_path.clone()], "{source}");
        let registration_count = usize::from(source == "linked-with-open-primary");
        assert_eq!(fixture.adapter.worktree_requests.lock().unwrap().len(), registration_count, "{source}");
        if registration_count == 0 {
            assert_eq!(after.spaces, before.spaces, "{source}");
        } else {
            assert!(after.spaces.iter().any(|space| space.id == binding.workspace_id
                && moved_space_registered(space, repository, &prepared.plan.checkout_path)));
        }
        assert!(fixture.adapter.remove_requests.lock().is_empty(), "{source}");
        assert!(Path::new(&prepared.plan.checkout_path).is_dir(), "{source}");
        fixture.no_shells();
    }
}
