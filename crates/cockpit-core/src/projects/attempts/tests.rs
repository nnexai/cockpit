use std::os::unix::fs::{DirBuilderExt, MetadataExt, PermissionsExt};
use std::path::{Path, PathBuf};
use std::sync::{Arc, atomic::Ordering};

use cockpit_protocol::library::{LibraryAddRequest, LibraryPhaseState, SpaceTarget};
use cockpit_protocol::orchestration::{OwnedAttemptDirectoryIdentity, OwnedResourceIdentity};
use cockpit_protocol::projects::{
    BindMovedSpaceRequest, OwnedAttemptDirectoryRequest, ProjectConfiguration,
    WorkspaceCheckoutOwnership, WorkspaceOperation, WorkspaceOperationState, WorkspaceOperationStep,
};
use cockpit_protocol::v1::{PaneSummary, SpaceSummary, TabSummary};

use crate::HerdrAdapter;
use crate::library::LibraryService;
use crate::projects::{ProjectService, error_response};
use crate::projects::test_support::NestedDirectoryAdapter;
use crate::sources::SourceService;

struct Fixture {
    root: PathBuf,
    config: ProjectConfiguration,
    adapter: Arc<NestedDirectoryAdapter>,
    service: ProjectService,
    library: LibraryService,
    request: OwnedAttemptDirectoryRequest,
}

impl Fixture {
    fn new() -> Self {
        let root = std::env::temp_dir().join(format!("cockpit-attempt-prep-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir(&root).unwrap();
        let root = std::fs::canonicalize(root).unwrap();
        let source = root.join("source-directory");
        std::fs::create_dir(&source).unwrap();
        std::fs::write(source.join("source.txt"), "original user project\n").unwrap();
        std::fs::create_dir(source.join("nested")).unwrap();
        std::fs::write(source.join("nested/keep.txt"), "original nested data\n").unwrap();
        let mut config = ProjectConfiguration::for_tests(&root);
        config.repository_roots = vec![source.to_string_lossy().into_owned()];
        let adapter = Arc::new(NestedDirectoryAdapter::default());
        adapter.selection_available.store(true, Ordering::SeqCst);
        let service = ProjectService::new(config.clone(), adapter.clone()).unwrap();
        let library = LibraryService::new(
            config.clone(), Arc::new(SourceService::new(&config, vec![]).unwrap()),
        ).with_herdr(adapter.clone());
        let request = OwnedAttemptDirectoryRequest {
            operation_id: uuid::Uuid::new_v4().to_string(),
            run_id: uuid::Uuid::new_v4().to_string(),
            source_root: source.to_string_lossy().into_owned(),
            source_workspace_id: "workspace".into(),
        };
        Self { root, config, adapter, service, library, request }
    }

    fn attempt_path(&self) -> PathBuf {
        Path::new(&self.config.state_root).join("orchestration/attempts").join(&self.request.run_id)
    }

    fn creation_identity(&self) -> OwnedAttemptDirectoryIdentity {
        let path = self.attempt_path();
        let metadata = std::fs::symlink_metadata(&path).unwrap();
        OwnedAttemptDirectoryIdentity {
            creation_operation_id: self.request.operation_id.clone(),
            root: path.to_string_lossy().into_owned(),
            device: metadata.dev().to_string(),
            inode: metadata.ino().to_string(),
        }
    }

    fn bind(&self, prepared: &WorkspaceOperation) -> BindMovedSpaceRequest {
        BindMovedSpaceRequest {
            operation_id: prepared.operation_id.clone(),
            expected_generation: prepared.generation,
            endpoint_identity: prepared.plan.endpoint_identity.clone(),
            workspace_id: "moved".into(),
            tab_id: "moved:worker".into(),
            pane_id: "moved:worker:p1".into(),
            terminal_id: "worker-terminal".into(),
            move_operation_id: uuid::Uuid::new_v4().to_string(),
        }
    }

    async fn move_snapshot(&self, checkout: &str) {
        let mut snapshot = self.adapter.session_snapshot("session").await.unwrap();
        snapshot.spaces.push(SpaceSummary {
            id: "moved".into(), label: "Worker".into(), number: 2,
            tab_count: 1, pane_count: 1, focused: false, agent_status: "none".into(), git: None,
        });
        snapshot.tabs.push(TabSummary {
            id: "moved:worker".into(), space_id: "moved".into(), label: "launch-attempt".into(),
            number: 1, pane_count: 1, focused: false, focused_pane_id: Some("moved:worker:p1".into()),
        });
        snapshot.panes.push(PaneSummary {
            id: "moved:worker:p1".into(), terminal_id: "worker-terminal".into(),
            space_id: "moved".into(), tab_id: "moved:worker".into(), title: None, focused: false,
            agent: None, agent_status: "none".into(), revision: 1, cwd: Some(checkout.into()),
        });
        *self.adapter.snapshot.lock() = Some(snapshot);
    }

    fn no_herdr_creation(&self) {
        assert!(self.adapter.worktree_requests.lock().unwrap().is_empty());
        assert!(self.adapter.terminal_requests.lock().unwrap().is_empty());
        assert!(self.adapter.closed_workspaces.lock().unwrap().is_empty());
        assert_eq!(self.adapter.inventory_calls.load(Ordering::SeqCst), 0);
    }

    fn source_unchanged(&self) {
        let source = Path::new(&self.request.source_root);
        assert_eq!(std::fs::read_to_string(source.join("source.txt")).unwrap(), "original user project\n");
        assert_eq!(std::fs::read_to_string(source.join("nested/keep.txt")).unwrap(), "original nested data\n");
        assert_eq!(std::fs::read_dir(source).unwrap().count(), 2);
        assert_eq!(std::fs::read_dir(source.join("nested")).unwrap().count(), 1);
        assert!(!source.join(".git").exists());
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.root);
    }
}

fn create_private_directory(path: &Path) {
    std::fs::DirBuilder::new().mode(0o700).create(path).unwrap();
}

#[tokio::test]
async fn preparation_creates_only_an_empty_private_per_run_directory_and_is_idempotent() {
    let fixture = Fixture::new();
    let source_before = std::fs::symlink_metadata(&fixture.request.source_root).unwrap();
    let first = fixture.service.prepare_owned_attempt_directory(
        "session", &fixture.request, &fixture.library,
    ).await.unwrap();
    let path = fixture.attempt_path();
    assert_eq!(first.operation_id, fixture.request.operation_id);
    assert_eq!(first.plan.checkout_path, path.to_string_lossy());
    assert_eq!(first.plan.ownership, WorkspaceCheckoutOwnership::OwnedAttemptDirectory);
    assert!(first.plan.repository.is_none());
    assert!(first.plan.branch.is_none());
    assert!(first.plan.base.is_none());
    assert_eq!(first.step, WorkspaceOperationStep::CheckoutPrepared);
    assert_eq!(first.state, WorkspaceOperationState::Running);
    assert!(first.workspace_id.is_none());
    assert!(first.tab_id.is_none());
    assert!(first.pane_id.is_none());
    let metadata = std::fs::symlink_metadata(&path).unwrap();
    assert!(metadata.is_dir());
    assert_eq!(metadata.permissions().mode() & 0o7777, 0o700);
    assert_eq!(std::fs::read_dir(&path).unwrap().count(), 0);
    assert!(!path.starts_with(&fixture.request.source_root));
    assert!(!Path::new(&fixture.request.source_root).starts_with(&path));
    assert_eq!(std::fs::read_dir(&fixture.config.worktree_root).unwrap().count(), 0);
    assert_eq!(first.owned_resources.len(), 1);
    assert_eq!(first.owned_resources[0].kind, "attempt_directory");
    assert_eq!(first.owned_resources[0].path, first.plan.checkout_path);
    assert!(first.owned_resources[0].created_by_operation);
    let receipt = fixture.service.store.read_owned_creation(&first.operation_id).unwrap().unwrap();
    assert_eq!(receipt.operation_id, fixture.request.operation_id);
    assert_eq!(receipt.resources, vec![OwnedResourceIdentity::AttemptDirectory {
        identity: fixture.creation_identity(),
    }]);
    assert!(receipt.moved_space.is_none());
    assert!(receipt.git_capture_error.is_none());
    let journal = fixture.service.store.owned_checkout(&first.operation_id).unwrap().unwrap();
    assert_eq!(journal.attempt_directory.as_ref(), Some(&fixture.request));
    assert!(journal.directory_requested);
    assert!(!journal.borrowed_directory);
    assert!(!journal.git_requested);
    assert!(journal.bind.is_none());
    let second = fixture.service.prepare_owned_attempt_directory(
        "session", &fixture.request, &fixture.library,
    ).await.unwrap();
    assert_eq!(second.generation, first.generation);
    assert_eq!(second.owned_resources.len(), 1);
    assert_eq!(second.owned_resources[0].kind, first.owned_resources[0].kind);
    assert_eq!(second.owned_resources[0].path, first.owned_resources[0].path);
    assert_eq!(second.owned_resources[0].created_by_operation, first.owned_resources[0].created_by_operation);
    assert_eq!(fixture.service.store.read_owned_creation(&first.operation_id).unwrap(), Some(receipt));
    assert_eq!(std::fs::read_dir(path).unwrap().count(), 0);
    assert_eq!(fixture.service.store.list().unwrap().len(), 1);
    let source_after = std::fs::symlink_metadata(&fixture.request.source_root).unwrap();
    assert_eq!((source_after.dev(), source_after.ino(), source_after.mode()),
        (source_before.dev(), source_before.ino(), source_before.mode()));
    fixture.source_unchanged();
    fixture.no_herdr_creation();
}

#[tokio::test]
async fn existing_unreceipted_output_is_unknown_and_never_owned_or_removed() {
    let fixture = Fixture::new();
    let path = fixture.attempt_path();
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::set_permissions(path.parent().unwrap(), std::fs::Permissions::from_mode(0o700)).unwrap();
    create_private_directory(&path);
    let output = path.join("user-output.txt");
    std::fs::write(&output, "foreign output must survive").unwrap();
    let original = fixture.creation_identity();
    for _ in 0..2 {
        let error = fixture.service.prepare_owned_attempt_directory(
            "session", &fixture.request, &fixture.library,
        ).await.unwrap_err();
        assert_eq!(error.code, "owned_attempt_directory_unknown");
        assert_eq!(std::fs::read_to_string(&output).unwrap(), "foreign output must survive");
        assert_eq!(fixture.creation_identity(), original);
        assert!(fixture.service.store.read_owned_creation(&fixture.request.operation_id).unwrap().is_none());
    }
    let operation = fixture.service.store.load(&fixture.request.operation_id).unwrap();
    assert_eq!(operation.state, WorkspaceOperationState::OutcomeUnknown);
    assert!(!operation.resume_allowed);
    assert!(operation.owned_resources.is_empty());
    assert!(operation.workspace_id.is_none());
    let journal = fixture.service.store.owned_checkout(&operation.operation_id).unwrap().unwrap();
    assert!(journal.directory_requested);
    assert!(journal.bind.is_none());
    fixture.source_unchanged();
    fixture.no_herdr_creation();
}

#[tokio::test]
async fn restart_revalidates_the_same_receipted_directory_and_preserves_worker_output() {
    let fixture = Fixture::new();
    let prepared = fixture.service.prepare_owned_attempt_directory(
        "session", &fixture.request, &fixture.library,
    ).await.unwrap();
    let identity = fixture.creation_identity();
    let receipt = fixture.service.store.read_owned_creation(&prepared.operation_id).unwrap().unwrap();
    let output = fixture.attempt_path().join(format!("worker-output-{}", uuid::Uuid::new_v4()));
    std::fs::write(&output, "unique worker output\n").unwrap();
    let restarted = ProjectService::new(fixture.config.clone(), fixture.adapter.clone()).unwrap();
    let retained = restarted.store.load(&prepared.operation_id).unwrap();
    let recovered = restarted.prepare_owned_attempt_directory(
        "session", &fixture.request, &fixture.library,
    ).await.unwrap();
    assert_eq!(recovered.generation, retained.generation);
    assert_eq!(recovered.operation_id, prepared.operation_id);
    assert_eq!(recovered.plan.checkout_path, prepared.plan.checkout_path);
    assert_eq!(recovered.owned_resources.len(), 1);
    assert_eq!(recovered.owned_resources[0].kind, prepared.owned_resources[0].kind);
    assert_eq!(recovered.owned_resources[0].path, prepared.owned_resources[0].path);
    assert_eq!(recovered.owned_resources[0].created_by_operation, prepared.owned_resources[0].created_by_operation);
    assert_eq!(fixture.creation_identity(), identity);
    assert_eq!(restarted.store.read_owned_creation(&prepared.operation_id).unwrap(), Some(receipt));
    assert_eq!(std::fs::read_to_string(&output).unwrap(), "unique worker output\n");
    assert_eq!(std::fs::read_dir(fixture.attempt_path()).unwrap().count(), 1);
    assert_eq!(std::fs::read_dir(fixture.attempt_path().parent().unwrap()).unwrap().count(), 1);
    fixture.source_unchanged();
    fixture.no_herdr_creation();
}

#[tokio::test]
async fn physical_replacement_cannot_inherit_the_creation_receipt_or_bind_a_space() {
    let fixture = Fixture::new();
    let prepared = fixture.service.prepare_owned_attempt_directory(
        "session", &fixture.request, &fixture.library,
    ).await.unwrap();
    let identity = fixture.creation_identity();
    let receipt = fixture.service.store.read_owned_creation(&prepared.operation_id).unwrap().unwrap();
    let path = fixture.attempt_path();
    std::fs::write(path.join("owned-output.txt"), "original attempt output").unwrap();
    let original = fixture.root.join("retained-original-attempt");
    std::fs::rename(&path, &original).unwrap();
    create_private_directory(&path);
    std::fs::write(path.join("foreign-output.txt"), "replacement belongs to someone else").unwrap();
    assert_ne!(fixture.creation_identity().inode, identity.inode);
    let restarted = ProjectService::new(fixture.config.clone(), fixture.adapter.clone()).unwrap();
    assert_eq!(restarted.prepare_owned_attempt_directory(
        "session", &fixture.request, &fixture.library,
    ).await.unwrap_err().code, "attempt_directory_conflict");
    let retained = restarted.store.load(&prepared.operation_id).unwrap();
    fixture.move_snapshot(&retained.plan.checkout_path).await;
    let binding = fixture.bind(&retained);
    assert_eq!(restarted.bind_moved_space(
        "session", &binding, &fixture.library,
    ).await.unwrap_err().code, "attempt_directory_conflict");
    assert!(restarted.store.owned_checkout(&prepared.operation_id).unwrap().unwrap().bind.is_none());
    assert!(restarted.store.load(&prepared.operation_id).unwrap().workspace_id.is_none());
    assert_eq!(restarted.store.read_owned_creation(&prepared.operation_id).unwrap(), Some(receipt));
    assert_eq!(std::fs::read_to_string(original.join("owned-output.txt")).unwrap(), "original attempt output");
    assert_eq!(std::fs::read_to_string(path.join("foreign-output.txt")).unwrap(), "replacement belongs to someone else");
    fixture.source_unchanged();
    fixture.no_herdr_creation();
}

#[tokio::test]
async fn interrupted_directory_intent_without_a_receipt_never_adopts_present_output() {
    let fixture = Fixture::new();
    let prepared = fixture.service.prepare_owned_attempt_directory(
        "session", &fixture.request, &fixture.library,
    ).await.unwrap();
    let identity = fixture.creation_identity();
    let output = fixture.attempt_path().join("output-before-crash.txt");
    std::fs::write(&output, "retain interrupted output").unwrap();
    // Simulate a crash between the exclusive mkdir and durable physical creation proof.
    std::fs::remove_file(Path::new(&fixture.config.state_root).join(
        format!("{}.owned.json", prepared.operation_id),
    )).unwrap();
    fixture.service.store.update(&prepared.operation_id, None, |operation| {
        operation.step = WorkspaceOperationStep::Validated;
        operation.state = WorkspaceOperationState::Running;
        operation.owned_resources.clear();
        operation.error = Some(error_response("owned_attempt_directory_pending", "interrupted"));
        Ok(())
    }).unwrap();
    let restarted = ProjectService::new(fixture.config.clone(), fixture.adapter.clone()).unwrap();
    assert_eq!(restarted.store.load(&prepared.operation_id).unwrap().state, WorkspaceOperationState::OutcomeUnknown);
    for _ in 0..2 {
        assert_eq!(restarted.prepare_owned_attempt_directory(
            "session", &fixture.request, &fixture.library,
        ).await.unwrap_err().code, "owned_attempt_directory_unknown");
        assert_eq!(fixture.creation_identity(), identity);
        assert_eq!(std::fs::read_to_string(&output).unwrap(), "retain interrupted output");
        assert!(restarted.store.read_owned_creation(&prepared.operation_id).unwrap().is_none());
    }
    let journal = restarted.store.owned_checkout(&prepared.operation_id).unwrap().unwrap();
    assert!(journal.directory_requested);
    assert_eq!(journal.attempt_directory.as_ref(), Some(&fixture.request));
    assert!(journal.bind.is_none());
    assert!(restarted.store.load(&prepared.operation_id).unwrap().owned_resources.is_empty());
    fixture.source_unchanged();
    fixture.no_herdr_creation();
}

#[tokio::test]
async fn bind_appends_only_the_exact_moved_space_and_selects_primary_library_paths() {
    let fixture = Fixture::new();
    let source_target = SpaceTarget { session_id: "session".into(), space_id: "workspace".into() };
    let added = fixture.library.start_add(LibraryAddRequest {
        input: fixture.request.source_root.clone(), provider_id: None, reference_depth: 0,
        follow: false, follow_mode: None, download_attachments: false, refresh_existing: false,
        label: None, target: Some(source_target.clone()),
    }).await.unwrap();
    loop {
        let operation = fixture.library.operation(&added.operation_id).await.unwrap();
        if operation.finished {
            assert!(operation.phases.iter().all(|phase| phase.state == LibraryPhaseState::Done));
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(5)).await;
    }
    let source = fixture.library.space_listing(&source_target).await.unwrap();
    assert!(!source.items.is_empty());
    assert!(source.repository_paths.is_empty());
    let prepared = fixture.service.prepare_owned_attempt_directory(
        "session", &fixture.request, &fixture.library,
    ).await.unwrap();
    let original_receipt = fixture.service.store.read_owned_creation(&prepared.operation_id).unwrap().unwrap();
    assert_eq!(original_receipt.resources.len(), 1);
    let identity = fixture.creation_identity();
    fixture.move_snapshot(&prepared.plan.checkout_path).await;
    let binding = fixture.bind(&prepared);
    let completed = fixture.service.bind_moved_space("session", &binding, &fixture.library).await.unwrap();
    assert_eq!(completed.state, WorkspaceOperationState::Completed);
    assert_eq!(completed.workspace_id.as_deref(), Some(binding.workspace_id.as_str()));
    assert_eq!(completed.tab_id.as_deref(), Some(binding.tab_id.as_str()));
    assert_eq!(completed.pane_id.as_deref(), Some(binding.pane_id.as_str()));
    assert_eq!(completed.plan.checkout_path, fixture.attempt_path().to_string_lossy());
    let selected = fixture.library.space_listing(&SpaceTarget {
        session_id: "session".into(), space_id: binding.workspace_id.clone(),
    }).await.unwrap();
    assert_eq!(selected.repository_paths, source.repository_paths);
    assert_eq!(selected.items.iter().map(|item| (&item.item_id, &item.item_path)).collect::<Vec<_>>(),
        source.items.iter().map(|item| (&item.item_id, &item.item_path)).collect::<Vec<_>>());
    assert_eq!(fixture.library.space_listing(&source_target).await.unwrap().items.len(), source.items.len());
    let receipt = fixture.service.store.read_owned_creation(&prepared.operation_id).unwrap().unwrap();
    assert_eq!(receipt.resources.len(), 2);
    assert_eq!(receipt.resources[0], original_receipt.resources[0]);
    assert_eq!(receipt.resources[0], OwnedResourceIdentity::AttemptDirectory { identity: identity.clone() });
    assert!(matches!(&receipt.resources[1], OwnedResourceIdentity::Space { identity: space }
        if space.creation_operation_id == binding.move_operation_id
            && space.endpoint_identity == binding.endpoint_identity
            && space.session_id == "session"
            && space.workspace_id == binding.workspace_id
            && space.checkout_path == prepared.plan.checkout_path));
    assert_eq!(receipt.moved_space.as_ref(), Some(&binding));
    assert_eq!(fixture.service.store.owned_checkout(&prepared.operation_id).unwrap().unwrap().bind, Some(binding.clone()));
    let repeated = fixture.service.bind_moved_space("session", &binding, &fixture.library).await.unwrap();
    assert_eq!(repeated.generation, completed.generation);
    assert_eq!(fixture.service.store.read_owned_creation(&prepared.operation_id).unwrap(), Some(receipt));
    assert_eq!(fixture.creation_identity(), identity);
    assert_eq!(std::fs::read_dir(fixture.attempt_path()).unwrap().count(), 0);
    let snapshot = fixture.adapter.session_snapshot("session").await.unwrap();
    assert_eq!(snapshot.panes.iter().filter(|pane| pane.space_id == binding.workspace_id).count(), 1);
    assert_eq!(snapshot.tabs.iter().filter(|tab| tab.space_id == binding.workspace_id).count(), 1);
    fixture.source_unchanged();
    fixture.no_herdr_creation();
}

#[tokio::test]
async fn binding_rejects_a_foreign_cwd_terminal_and_extra_pane_without_recording_ownership() {
    let fixture = Fixture::new();
    let prepared = fixture.service.prepare_owned_attempt_directory(
        "session", &fixture.request, &fixture.library,
    ).await.unwrap();
    let receipt = fixture.service.store.read_owned_creation(&prepared.operation_id).unwrap().unwrap();
    fixture.move_snapshot(&prepared.plan.checkout_path).await;
    let binding = fixture.bind(&prepared);
    let original = fixture.adapter.snapshot.lock().clone().unwrap();
    let mut wrong_cwd = original.clone();
    wrong_cwd.panes[0].cwd = Some(fixture.request.source_root.clone());
    *fixture.adapter.snapshot.lock() = Some(wrong_cwd);
    assert_eq!(fixture.service.bind_moved_space(
        "session", &binding, &fixture.library,
    ).await.unwrap_err().code, "association_conflict");
    *fixture.adapter.snapshot.lock() = Some(original.clone());
    let mut wrong_terminal = binding.clone();
    wrong_terminal.terminal_id = "foreign-terminal".into();
    assert_eq!(fixture.service.bind_moved_space(
        "session", &wrong_terminal, &fixture.library,
    ).await.unwrap_err().code, "association_conflict");
    let mut extra = original;
    let mut second = extra.panes[0].clone();
    second.id = "moved:worker:p2".into();
    second.terminal_id = "extra-terminal".into();
    extra.panes.push(second);
    extra.spaces.iter_mut().find(|space| space.id == "moved").unwrap().pane_count = 2;
    extra.tabs[0].pane_count = 2;
    *fixture.adapter.snapshot.lock() = Some(extra);
    assert_eq!(fixture.service.bind_moved_space(
        "session", &binding, &fixture.library,
    ).await.unwrap_err().code, "association_conflict");
    assert!(fixture.service.store.owned_checkout(&prepared.operation_id).unwrap().unwrap().bind.is_none());
    assert!(fixture.service.store.load(&prepared.operation_id).unwrap().workspace_id.is_none());
    assert_eq!(fixture.service.store.read_owned_creation(&prepared.operation_id).unwrap(), Some(receipt));
    fixture.source_unchanged();
    fixture.no_herdr_creation();
}
