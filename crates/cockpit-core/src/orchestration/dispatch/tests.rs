use super::*;
use crate::orchestration::herdr::{AgentTabRequest, RuntimePane};
use cockpit_protocol::{
    projects::ProjectConfiguration,
    v1::{FocusRequest, FocusResponse, HerdrCompatibility, ResourceMutationRequest,
        ResourceMutationResponse, SessionListResponse, SessionSnapshotResponse, TerminalOpenRequest},
};
use crate::{
    project_adapter::{ProjectInventory,
        ProjectWorktreeRemoveRequest, ProjectWorktreeRequest, ProjectWorktreeResult},
    HerdrAdapter, ProjectHerdrAdapter, SessionSubscription, TerminalSession,
};

#[derive(Default)]
struct ReadOnlyProjectAdapter { inventory: Option<ProjectInventory> }
fn unused_project_call<T>() -> Result<T, InspectionError> {
    panic!("Prepared-launch test must not dispatch a project effect")
}
#[async_trait::async_trait]
impl HerdrAdapter for ReadOnlyProjectAdapter {
    async fn inspect(&self) -> Result<HerdrCompatibility, InspectionError> { unused_project_call() }
    async fn inspect_session(&self, _: &str) -> Result<HerdrCompatibility, InspectionError> { unused_project_call() }
    async fn sessions(&self) -> Result<SessionListResponse, InspectionError> { unused_project_call() }
    async fn session_snapshot(&self, _: &str) -> Result<SessionSnapshotResponse, InspectionError> { unused_project_call() }
    async fn focus(&self, _: &str, _: &FocusRequest) -> Result<FocusResponse, InspectionError> { unused_project_call() }
    async fn mutate(&self, _: &str, _: &ResourceMutationRequest) -> Result<ResourceMutationResponse, InspectionError> { unused_project_call() }
    async fn subscribe_session(&self, _: &str, _: &SessionSnapshotResponse) -> Result<SessionSubscription, InspectionError> { unused_project_call() }
    async fn open_terminal(&self, _: &TerminalOpenRequest) -> Result<TerminalSession, InspectionError> { unused_project_call() }
}
#[async_trait::async_trait]
impl ProjectHerdrAdapter for ReadOnlyProjectAdapter {
    async fn project_endpoint_identity(&self, _: &str) -> Result<String, InspectionError> {
        self.inventory.as_ref().map(|inventory| inventory.endpoint_identity.clone()).map_or_else(unused_project_call, Ok)
    }
    async fn project_inventory(&self, _: &str, _: &str) -> Result<ProjectInventory, InspectionError> {
        self.inventory.clone().map_or_else(unused_project_call, Ok)
    }
    async fn project_worktree(&self, _: &str, _: &ProjectWorktreeRequest) -> Result<ProjectWorktreeResult, InspectionError> { unused_project_call() }
    async fn project_worktree_dirty(&self, _: &str, _: u32, _: u32) -> Result<crate::project_adapter::WorktreeStatus, InspectionError> { unused_project_call() }
    async fn project_close_workspace(&self, _: &str, _: &str, _: &str) -> Result<(), InspectionError> { unused_project_call() }
    async fn project_remove_worktree(&self, _: &str, _: &ProjectWorktreeRemoveRequest) -> Result<(), InspectionError> { unused_project_call() }
}
fn runtime() -> RuntimeView {
    RuntimeView {
        endpoint_identity: "endpoint".into(),
        boot_id: None,
        peer_process: None,
        focused_workspace_id: Some("space".into()),
        workspaces: vec![super::super::herdr::RuntimeWorkspace {
            workspace_id: "space".into(), label: "Space".into(), cwd: "/".into(),
        }],
        panes: vec![RuntimePane {
            workspace_id: "space".into(),
            workspace_label: "Space".into(),
            tab_id: "tab".into(),
            tab_label: "tag".into(),
            pane_id: "pane".into(),
            cwd: None,
            terminal_id: Some("terminal".into()),
            native_session_id: None,
            native_session_file: None,
            agent_name: None,
            agent_kind: None,
            launch_pending: false,
            agent_status: Some("idle".into()),
            state_changed_at: None,
        }],
    }
}
#[test]
fn open_space_for_path_proves_canonical_checkout_and_preserves_real_space_id() {
    use crate::orchestration::herdr::RuntimeWorkspace;
    let root = std::env::temp_dir().join(format!("cockpit-open-space-{}", uuid::Uuid::new_v4()));
    let checkout = root.join("checkout");
    let other = root.join("other");
    std::fs::create_dir_all(&checkout).unwrap();
    std::fs::create_dir_all(&other).unwrap();
    let checkout = std::fs::canonicalize(checkout).unwrap();
    let mut observed = runtime();
    observed.workspaces = vec![
        RuntimeWorkspace {
            workspace_id: "missing-cwd".into(), label: "Unavailable".into(),
            cwd: root.join("missing").to_string_lossy().into_owned(),
        },
        RuntimeWorkspace {
            workspace_id: "project-space".into(), label: "Project".into(),
            cwd: checkout.to_string_lossy().into_owned(),
        },
    ];
    assert_eq!(open_space_for_path(&observed, checkout.to_str().unwrap()),
        Some("project-space".into()));
    assert_eq!(open_space_for_path(&observed, other.to_str().unwrap()), None);
    #[cfg(unix)]
    {
        let alias = root.join("alias");
        std::os::unix::fs::symlink(&checkout, &alias).unwrap();
        observed.workspaces[1].cwd = alias.to_string_lossy().into_owned();
        assert_eq!(open_space_for_path(&observed, checkout.to_str().unwrap()),
            Some("project-space".into()));
    }
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn launch_proof_uses_pinned_receipt_not_mutable_names() {
    let mut observed = runtime();
    observed.panes[0].agent_kind = Some("omp".into());
    observed.panes[0].native_session_id = Some("native".into());
    let location = receipt();
    for name in [None, Some("renamed"), Some("tag"), Some("omp")] {
        observed.panes[0].agent_name = name.map(str::to_owned);
        assert!(matches!(reconcile_tag(&observed, Some("endpoint"), "tag",
            Some(&location), Some("native"), false, "omp"), ReconciledLaunch::Agent));
    }
    // A different pane's display alias is not a competing launch receipt.
    let mut unrelated = observed.panes[0].clone();
    unrelated.pane_id = "unrelated".into();
    unrelated.terminal_id = Some("unrelated-terminal".into());
    unrelated.agent_name = Some("tag".into());
    observed.panes.push(unrelated);
    assert!(matches!(reconcile_tag(&observed, Some("endpoint"), "tag",
        Some(&location), Some("native"), false, "omp"), ReconciledLaunch::Agent));
    assert!(matches!(reconcile_tag(&observed, Some("endpoint"), "tag",
        None, Some("native"), false, "omp"), ReconciledLaunch::Conflict));
    observed.panes.pop();
    assert!(matches!(reconcile_tag(&observed, Some("endpoint"), "tag",
        None, Some("native"), true, "omp"), ReconciledLaunch::Tab));
    observed.panes[0].launch_pending = true;
    assert!(matches!(reconcile_tag(&observed, Some("endpoint"), "tag",
        Some(&location), Some("native"), true, "omp"), ReconciledLaunch::Tab));
    observed.panes[0].launch_pending = false;
    assert!(matches!(reconcile_tag(&observed, Some("endpoint"), "tag",
        Some(&location), None, true, "omp"), ReconciledLaunch::Agent));
    observed.panes[0].native_session_id = None;
    for (proof, expected) in [(false, false), (true, true)] {
        assert_eq!(matches!(reconcile_tag(&observed, Some("endpoint"), "tag",
            Some(&location), Some("native"), proof, "omp"), ReconciledLaunch::Agent), expected);
    }
    observed.panes[0].tab_label = "not-a-launch".into();
    observed.panes[0].agent_name = Some("tag".into());
    assert!(matches!(reconcile_tag(&observed, Some("endpoint"), "tag",
        None, None, false, "omp"), ReconciledLaunch::Unknown));
}
fn receipt() -> RunLocation {
    RunLocation {
        endpoint_identity: "endpoint".into(),
        session_id: "test-session".into(),
        workspace_id: "space".into(),
        tab_id: "tab".into(),
        pane_id: "pane".into(),
        launch_tag: "tag".into(),
        boot_id: Some("boot".into()),
        peer_process: None,
        terminal_id: Some("terminal".into()),
        native_session_id: Some("native".into()),
    }
}

#[test]
fn launch_proof_fences_available_boot_terminal_and_native_identity() {
    let mut observed = runtime();
    observed.boot_id = Some("boot".into());
    observed.panes[0].agent_name = Some("tag".into());
    observed.panes[0].agent_kind = Some("omp".into());
    observed.panes[0].native_session_id = Some("native".into());
    let location = receipt();
    assert!(matches!(
        reconcile_tag(
            &observed,
            Some("endpoint"),
            "tag",
            Some(&location),
            Some("native"), false, "omp"
        ),
        ReconciledLaunch::Agent
    ));
    for mismatch in 0..10 {
        let mut observed = observed.clone();
        let mut location = location.clone();
        match mismatch {
            0 => observed.boot_id = Some("other-boot".into()),
            1 => observed.panes[0].terminal_id = Some("other-terminal".into()),
            2 => observed.panes[0].terminal_id = None,
            3 => observed.panes[0].native_session_id = Some("other-native".into()),
            4 => location.native_session_id = Some("old-native".into()),
            5 => location.endpoint_identity = "old-endpoint".into(),
            6 => location.launch_tag = "other-tag".into(),
            7 => observed.panes[0].pane_id = "other-pane".into(),
            8 => observed.panes[0].workspace_id = "other-space".into(),
            _ => observed.panes[0].tab_id = "other-tab".into(),
        }
        assert!(
            matches!(
                reconcile_tag(
                    &observed,
                    Some("endpoint"),
                    "tag",
                    Some(&location),
                    Some("native"), true, "omp"
                ),
                ReconciledLaunch::Conflict
            ),
            "mismatch {mismatch}"
        );
    }
    observed.boot_id = None;
    observed.panes[0].native_session_id = None;
    assert!(
        matches!(
            reconcile_tag(
                &observed,
                Some("endpoint"),
                "tag",
                Some(&location),
                Some("native"), true, "omp"
            ),
            ReconciledLaunch::Agent
        ),
        "optional omission is unobserved, not a contradictory value"
    );
    assert!(matches!(reconcile_tag(&observed, Some("endpoint"), "tag",
        Some(&location), Some("native"), false, "omp"), ReconciledLaunch::Tab));
    let mut missing_terminal = location.clone();
    missing_terminal.terminal_id = None;
    assert!(matches!(reconcile_tag(&observed, Some("endpoint"), "tag",
        Some(&missing_terminal), Some("native"), true, "omp"), ReconciledLaunch::Tab));
}

fn test_attachment(run: &mut Run) -> &mut cockpit_protocol::orchestration::ExecutionAttachment {
    let updated_at = &run.updated_at;
    run.attachment.get_or_insert_with(|| cockpit_protocol::orchestration::ExecutionAttachment {
        attachment_id: uuid::Uuid::new_v4().to_string(),
        epoch: 1,
        state: cockpit_protocol::orchestration::AttachmentState::Reserved,
        launch_mode: None,
        location: None,
        conversation: None,
        process_scope: None,
        process: None,
        shell: None,
        machine_id: None,
        launch_attempt: 1,
        updated_at: updated_at.clone(),
    })
}

#[test]
fn endpoint_observation_deduplicates_across_endpoint_changes_per_epoch() {
    let mut fixture = ReviewFixture::new(RunStage::Working);
    fixture.worker_task(None);
    let mut observed = runtime();
    for endpoint in ["restart-one", "restart-two", "restart-three"] {
        observed.endpoint_identity = endpoint.into();
        fixture.service.record_observation(&observed, "test-session").unwrap();
    }
    let state = fixture.service.store.lock().unwrap().read().unwrap();
    assert_eq!(state.messages.iter().filter(|m| m.message_id.contains(":endpoint_changed:")).count(), 1);
    drop(state);
    fixture.change(|run| run.attachment.as_mut().unwrap().epoch += 1);
    fixture.service.record_observation(&observed, "test-session").unwrap();
    let state = fixture.service.store.lock().unwrap().read().unwrap();
    assert_eq!(state.messages.iter().filter(|m| m.message_id.contains(":endpoint_changed:")).count(), 2);
}

#[test]
fn idle_worker_notice_waits_for_deadline_and_survives_state_timestamp_changes() {
    use cockpit_protocol::orchestration::{ActorRef, MessageKind};
    let mut fixture = ReviewFixture::new(RunStage::Working);
    fixture.worker_task(None);
    let mut observed = runtime();
    observed.panes[0].state_changed_at = Some(now());
    assert!(fixture.service.record_observation(&observed, "test-session").unwrap().is_some());
    assert!(fixture.service.store.lock().unwrap().read().unwrap().messages.is_empty());
    observed.panes[0].state_changed_at = Some("2000-01-01T00:00:00Z".into());
    fixture.service.record_observation(&observed, "test-session").unwrap();
    observed.panes[0].state_changed_at = Some("2001-01-01T00:00:00Z".into());
    fixture.service.record_observation(&observed, "test-session").unwrap();
    let state = fixture.service.store.lock().unwrap().read().unwrap();
    assert_eq!(state.messages.iter().filter(|m| matches!(m.from, ActorRef::Dispatcher)
        && m.kind == MessageKind::Observation && m.message_id.contains(":idle_without_report:")).count(), 1);
}

#[test]
fn queued_resource_stall_waits_ten_minutes_and_notifies_once() {
    use cockpit_protocol::orchestration::{AttachmentReference, ResourceRequestRecord, ResourceRequestState, ResourceSourceIdentity};
    let mut fixture = ReviewFixture::new(RunStage::Working);
    fixture.worker_task(None);
    let run = fixture.current().1;
    let attachment = run.attachment.as_ref().unwrap();
    let locked = fixture.service.store.lock().unwrap();
    let mut state = locked.read().unwrap();
    state.resource_requests.push(ResourceRequestRecord {
        request_id: uuid::Uuid::new_v4().to_string(), sequence: 1, resource_key: "/output".into(),
        source_identity: ResourceSourceIdentity { checkout_key: "checkout".into(), source_commit: "a".repeat(40),
            source_tree: "b".repeat(40), input_configuration: "inputs".into(), toolchain_fingerprint: "toolchain".into() },
        holder: AttachmentReference { run_id: run.run_id.clone(), attachment_id: attachment.attachment_id.clone(), attachment_epoch: attachment.epoch },
        state: ResourceRequestState::Queued, build_processes: vec![], machine_id: "machine".into(),
        blocked_reason: None, updated_at: now(),
    });
    locked.save(&mut state).unwrap();
    drop(locked);
    assert!(fixture.service.resource_stall_delay().unwrap().is_some());
    assert!(fixture.service.store.lock().unwrap().read().unwrap().messages.is_empty());
    let locked = fixture.service.store.lock().unwrap();
    let mut state = locked.read().unwrap();
    state.resource_requests[0].updated_at = "2000-01-01T00:00:00Z".into();
    locked.save(&mut state).unwrap();
    drop(locked);
    assert!(fixture.service.resource_stall_delay().unwrap().is_none());
    fixture.service.resource_stall_delay().unwrap();
    let state = fixture.service.store.lock().unwrap().read().unwrap();
    assert_eq!(state.messages.iter().filter(|m| m.message_id.starts_with("resource-stall:")).count(), 1);
}

struct ReviewFixture {
    root: PathBuf,
    service: Arc<OrchestrationService>,
    config: ProjectConfiguration,
    run: Run,
}
impl ReviewFixture {
    fn new(stage: RunStage) -> Self {
        use cockpit_protocol::orchestration::RunKind;
        let root = std::env::temp_dir()
            .join(format!("cockpit-launch-review-{}", uuid::Uuid::new_v4()));
        let config = ProjectConfiguration {
            repository_roots: vec![],
            branch_template: "test/{task}".into(),
            checkout_template: "{task}".into(),
            limits: cockpit_protocol::projects::ProjectLimits {
                catalog_depth: 1,
                catalog_entries: 1,
                git_timeout_ms: 1000,
                git_output_bytes: 1024,
                operation_timeout_ms: 1000,
                context_preview_bytes: 1024,
                context_preview_lines: 10,
                context_directory_entries: 10,
                context_tree_depth: 1,
                library_folder_files: 10,
                library_folder_bytes: 1024,
                library_file_bytes: 1024,
                library_space_pages: 10,
                library_attachment_bytes: 1024,
                library_item_attachment_bytes: 1024,
                library_max_items: 10,
            },
            ..ProjectConfiguration::for_tests(&root)
        };
        let service = Arc::new(OrchestrationService::open(&config).unwrap());
        let id = uuid::Uuid::new_v4().to_string();
        let mut run = crate::orchestration::new_run(
            &id,
            "test-session",
            RunKind::Supervisor,
            "Review fixture".into(),
            &id,
            crate::orchestration::RunLineage::default(),
            1,
        );
        run.stage = stage;
        run.launch_config.resume_args = Some(vec!["--session".into(), "{session_id}".into()]);
        run.launch_config.process_scope = cockpit_protocol::projects::ScopeMode::Off;
        test_attachment(&mut run).location = Some(receipt());
        test_attachment(&mut run).conversation = Some(cockpit_protocol::orchestration::ConversationIdentity {
            harness: "omp".into(), session_id: "conversation".into(),
        });
        test_attachment(&mut run).shell = Some(cockpit_protocol::orchestration::NativeShellIdentity {
            process: cockpit_protocol::orchestration::NativeProcessIdentity {
                pid: 123, start_ticks: 1, kernel_boot_id: Some("00000000-0000-0000-0000-000000000001".into()),
            },
            executable_device: "1".into(), executable_inode: "2".into(), argv_digest: "a".repeat(64),
        });
        let mut dispatch = crate::orchestration::dispatch(DispatchStep::LaunchIntent);
        dispatch.agent_started = true;
        dispatch.launch_tag = Some("tag".into());
        dispatch.endpoint_identity = Some("endpoint".into());
        run.dispatch = Some(dispatch);
        {
            let locked = service.store.lock().unwrap();
            let mut state = locked.read().unwrap();
            state.runs.push(run.clone());
            locked.save(&mut state).unwrap();
        }
        Self { root, service, run, config }
    }
    fn current(&self) -> (u64, Run) {
        let state = self.service.store.lock().unwrap().read().unwrap();
        (
            state.revision,
            state
                .runs
                .into_iter()
                .find(|run| run.run_id == self.run.run_id)
                .unwrap(),
        )
    }
    fn change(&self, change: impl FnOnce(&mut Run)) {
        let locked = self.service.store.lock().unwrap();
        let mut state = locked.read().unwrap();
        change(
            state
                .runs
                .iter_mut()
                .find(|run| run.run_id == self.run.run_id)
                .unwrap(),
        );
        locked.save(&mut state).unwrap();
    }
    fn uncertain(&mut self, kind: cockpit_protocol::orchestration::RunKind) {
        if kind == RunKind::Worker {
            self.worker_task(None);
        }
        self.change(|run| {
            run.kind = kind;
            run.stage = RunStage::Starting;
            run.brief = "Preserved launch brief".into();
            let mut observed = runtime();
            observed.panes[0].agent_kind = Some("omp".into());
            let error = as_response(launch_phase_error(&observed, run, "tag"));
            let dispatch = run.dispatch.as_mut().unwrap();
            dispatch.agent_started = false;
            dispatch.step = DispatchStep::LaunchUnknown;
            dispatch.error = Some(error);
        });
        self.run = self.current().1;
        let locked = self.service.store.lock().unwrap();
        let mut state = locked.read().unwrap();
        super::super::messages::append(&mut state, super::super::messages::AppendMessage { from: cockpit_protocol::orchestration::ActorRef::Operator, to_run_id: &self.run.run_id, message_id: "preserved-message", kind: cockpit_protocol::orchestration::MessageKind::Observation, text: "Existing operator message", in_reply_to: None, report: None, stale: false, escalated_from: None }).unwrap();
        locked.save(&mut state).unwrap();
    }
    fn worker_task(&mut self, prerequisite: Option<(&str, bool)>) {
        let root_id = uuid::Uuid::new_v4().to_string();
        let task_id = uuid::Uuid::new_v4().to_string();
        let mut root = crate::orchestration::new_run(
            &root_id, "test-session", RunKind::Supervisor, "Task manager".into(),
            &root_id, crate::orchestration::RunLineage::default(), 1,
        );
        root.stage = RunStage::Active;
        let mut bytes = format!("- [ ] Consumer <!-- cockpit-task: {task_id} -->\n");
        if let Some((prerequisite_id, checked)) = prerequisite {
            bytes.push_str(&format!(
                "  <!-- cockpit-relations: depends_on={prerequisite_id} -->\n- [{}] Prerequisite <!-- cockpit-task: {prerequisite_id} -->\n",
                if checked { "x" } else { " " },
            ));
        }
        self.service.store.tasks_dir().write(
            super::super::tasks_md::root_filename(&root_id).unwrap(), bytes.as_bytes(),
        ).unwrap();
        let locked = self.service.store.lock().unwrap();
        let task_revision = locked.tasks(&root_id).unwrap().task(&task_id).unwrap().task_revision.clone();
        let mut state = locked.read().unwrap();
        let worker = state.runs.iter_mut().find(|run| run.run_id == self.run.run_id).unwrap();
        worker.kind = RunKind::Worker;
        worker.root_id = root_id.clone();
        worker.parent_run_id = Some(root_id);
        worker.task_id = Some(task_id);
        worker.task_revision_at_start = Some(task_revision);
        worker.assignment_generation = Some(1);
        state.task_ownership.push(cockpit_protocol::orchestration::TaskOwnership {
            root_id: worker.root_id.clone(), task_id: worker.task_id.clone().unwrap(),
            generation: 0, owner: None, schedule: None,
        });
        state.runs.push(root);
        locked.save(&mut state).unwrap();
        self.run = state.runs.iter().find(|run| run.run_id == self.run.run_id).unwrap().clone();
    }
    fn reserved_worker() -> Self {
        let mut fixture = Self::new(RunStage::Starting);
        fixture.worker_task(None);
        fixture.change(|run| {
            run.attachment = None;
            test_attachment(run);
            run.dispatch = Some(crate::orchestration::dispatch(DispatchStep::SetupPending));
            run.setup = Some(SetupSummary {
                operation_id: None, generation: None, workspace_id: Some("space".into()),
                checkout_path: "/".into(), repository_id: None, branch: None, base: None,
                base_commit: None, ownership: None, effects: vec![], warnings: vec![],
                project_workspace_id: None,
            });
        });
        fixture.run = fixture.current().1;
        fixture
    }
    fn another_worker(&self) -> Run {
        let root_id = uuid::Uuid::new_v4().to_string();
        let worker_id = uuid::Uuid::new_v4().to_string();
        let mut root = crate::orchestration::new_run(
            &root_id, "test-session", RunKind::Supervisor, "Other root".into(),
            &root_id, crate::orchestration::RunLineage::default(), 1,
        );
        root.stage = RunStage::Active;
        let mut worker = crate::orchestration::new_run(
            &worker_id, "test-session", RunKind::Worker, "Other worker".into(),
            &root_id, crate::orchestration::RunLineage { parent_run_id: Some(root_id.clone()), task_id: Some(uuid::Uuid::new_v4().to_string()) }, 1,
        );
        worker.stage = RunStage::Starting;
        worker.attachment = None;
        test_attachment(&mut worker);
        worker.dispatch = Some(crate::orchestration::dispatch(DispatchStep::SetupPending));
        let locked = self.service.store.lock().unwrap();
        let task_id = worker.task_id.as_deref().unwrap();
        self.service.store.tasks_dir().write(
            super::super::tasks_md::root_filename(&root_id).unwrap(),
            format!("- [ ] Other <!-- cockpit-task: {task_id} -->\n").as_bytes(),
        ).unwrap();
        worker.task_revision_at_start = Some(locked.tasks(&root_id).unwrap().task(task_id).unwrap().task_revision.clone());
        let mut state = locked.read().unwrap();
        worker.assignment_generation = Some(1);
        state.task_ownership.push(cockpit_protocol::orchestration::TaskOwnership {
            root_id: worker.root_id.clone(), task_id: worker.task_id.clone().unwrap(),
            generation: 0, owner: None, schedule: None,
        });
        state.runs.extend([root, worker.clone()]);
        locked.save(&mut state).unwrap();
        worker
    }
    fn staged_auth(&self, run: &Run) -> super::super::JobAuth {
        use super::super::{claims, credentials};
        let locked = self.service.store.lock().unwrap();
        let mut state = locked.read().unwrap();
        let role = if run.kind == RunKind::Worker {
            credentials::JobRole::Worker
        } else {
            credentials::JobRole::Root
        };
        if run.kind == RunKind::Worker {
            let path = self.root.join("auth-checkout");
            std::fs::create_dir_all(&path).unwrap();
            let (project, checkout) = super::super::project_identity::resolve_owned_directory(&path).unwrap();
            let attachment = run.attachment.as_ref().unwrap();
            let reference = claims::AttachmentReference {
                run_id: run.run_id.clone(), attachment_id: attachment.attachment_id.clone(),
                attachment_epoch: attachment.epoch,
            };
            claims::reserve_claim(&mut state, &checkout, &reference, &attachment.attachment_id).unwrap();
            state.runs.iter_mut().find(|r| r.run_id == run.run_id).unwrap().project = Some(project);
        }
        let current = state.runs.iter().find(|r| r.run_id == run.run_id).unwrap();
        let (record, path) = credentials::stage_credential(self.service.base(), current, role).unwrap();
        state.credentials.push(record);
        locked.save(&mut state).unwrap();
        credentials::read_job_auth(&path).unwrap()
    }
    fn set_prerequisite(&self, prerequisite: &str, checked: bool) {
        let locked = self.service.store.lock().unwrap();
        let document = locked.tasks(&self.run.root_id).unwrap();
        let task = document.task(prerequisite).unwrap();
        document.check(prerequisite, &task.task_revision, checked).unwrap();
    }
    fn prepared_launch(&mut self, prerequisite: &str, checked: bool) {
        self.worker_task(Some((prerequisite, checked)));
        self.change(|run| {
            run.stage = RunStage::Starting;
            run.target = Some(DispatchTarget::ExistingSpace { workspace_id: "space".into() });
            test_attachment(run).location = None;
            test_attachment(run).conversation = None;
            test_attachment(run).process = None;
            test_attachment(run).shell = None;
            run.dispatch = Some(crate::orchestration::dispatch(DispatchStep::SetupPending));
            run.setup = Some(SetupSummary {
                operation_id: None, generation: None, workspace_id: Some("space".into()),
                checkout_path: "/".into(), repository_id: None, branch: None, base: None,
                base_commit: None,
                ownership: None, effects: vec![], warnings: vec![], project_workspace_id: None,
            });
        });
        self.run = self.current().1;
    }
    fn dispatcher(&self, herdr: Arc<CountingHerdr>) -> Arc<Dispatcher> {
        let projects = Arc::new(ProjectService::new(self.config.clone(), Arc::new(ReadOnlyProjectAdapter::default())).unwrap());
        let sources = Arc::new(crate::sources::SourceService::new(&self.config, vec![]).unwrap());
        let library = Arc::new(LibraryService::new(self.config.clone(), sources));
        Arc::new(Dispatcher::new(Arc::clone(&self.service), projects, library, herdr,
            DispatcherSettings {
                harness_kind: self.run.launch_config.harness_kind.clone(),
                command: self.run.launch_config.command.clone(), start_timeout_ms: 3001,
                cli_path: self.root.join("cockpit-cli"), config_path: None, herdr_socket: None,
                extra_args: self.run.launch_config.extra_args.clone(),
                resume_args: self.run.launch_config.resume_args.clone(),
                max_parallel: self.run.launch_config.max_parallel,
                inbox_prompt_hints: self.run.launch_config.inbox_prompt_hints, herdr_executable: None,
            }))
    }
    async fn running_launch_shell(&mut self) -> tokio::process::Child {
        let child = tokio::process::Command::new("sh").args(["-c", "read line"])
            .stdin(std::process::Stdio::piped()).kill_on_drop(true).spawn().unwrap();
        let pid = child.id().unwrap();
        let process = cockpit_protocol::orchestration::NativeProcessIdentity {
            pid, start_ticks: crate::process_identity::start_identity(pid as i32).unwrap(),
            kernel_boot_id: crate::process_identity::kernel_boot_id(),
        };
        self.change(|run| {
            test_attachment(run).shell.as_mut().unwrap().process = process;
            test_attachment(run).machine_id = crate::process_identity::machine_id();
        });
        self.run = self.current().1;
        child
    }
    async fn stopped_launch_shell(&mut self) {
        let mut child = self.running_launch_shell().await;
        child.kill().await.unwrap();
        child.wait().await.unwrap();
    }
}
impl Drop for ReviewFixture {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.root);
    }
}
#[tokio::test]
async fn dispatch_reservation_stages_active_credential_and_durable_claim_before_launch() {
    use super::super::{claims::ClaimState, credentials};
    let fixture = ReviewFixture::reserved_worker();
    let checkout_path = fixture.root.join("checkout");
    std::fs::create_dir_all(&checkout_path).unwrap();
    let (project, checkout) = super::super::project_identity::resolve_directory(&checkout_path, None, None).await.unwrap();
    let (reserved, path) = fixture.service.reserve_dispatch_launch(
        &fixture.run, project.clone(), checkout.clone(), None, None,
    ).unwrap();
    let attachment = reserved.attachment.as_ref().unwrap();
    assert!(reserved.setup.as_ref().unwrap().base_commit.is_none(), "non-Git has no code baseline");
    assert_eq!(attachment.state, cockpit_protocol::orchestration::AttachmentState::Reserved);
    assert!(attachment.location.is_none() && attachment.process.is_none()
        && attachment.shell.is_none() && attachment.conversation.is_none());
    assert!(!reserved.dispatch.as_ref().unwrap().agent_started);
    assert_eq!(reserved.project.as_ref(), Some(&project));
    assert_eq!(reserved.checkout.as_ref(), Some(&checkout));
    let credential_bytes = std::fs::read(&path).unwrap();
    let auth = credentials::read_job_auth(&path).unwrap();
    let before = fixture.service.store.lock().unwrap().read().unwrap();
    assert_eq!(before.credentials.len(), 1);
    let credential = &before.credentials[0];
    assert_eq!(credential.state, credentials::CredentialState::Active);
    assert_eq!(credential.role, credentials::JobRole::Worker);
    assert_eq!(credential.run_id, reserved.run_id);
    assert_eq!(credential.root_id, reserved.root_id);
    assert_eq!(credential.attachment_id, attachment.attachment_id);
    assert_eq!(credential.attachment_epoch, attachment.epoch);
    assert_eq!(credential.assignment_generation, Some(1));
    assert_eq!(auth.assignment_generation, Some(1));
    let ownership = before.task_ownership.iter()
        .find(|record| record.task_id == reserved.task_id.as_deref().unwrap()).unwrap();
    assert_eq!(ownership.generation, 0);
    assert!(ownership.owner.is_none(), "physical launch cannot claim a cooperative conversation");
    assert_eq!(auth.credential_id, credential.credential_id);
    assert_eq!(credentials::credential_path(fixture.service.base(), credential).unwrap(), path);
    let authorized = credentials::validate_job_auth(&before, &auth).unwrap();
    assert_eq!(authorized.checkout_claim_id, reserved.checkout_claim_id);
    assert_eq!(before.checkout_claims.len(), 1);
    let claim = &before.checkout_claims[0];
    assert_eq!(Some(&claim.claim_id), reserved.checkout_claim_id.as_ref());
    assert_eq!(claim.state, ClaimState::Reserved);
    assert_eq!(claim.checkout, checkout);
    let holder = claim.holder.as_ref().unwrap();
    assert_eq!(holder.run_id, reserved.run_id);
    assert_eq!(holder.attachment_id, attachment.attachment_id);
    assert_eq!(holder.attachment_epoch, attachment.epoch);
    let claim_json = serde_json::to_value(claim).unwrap();
    for key in ["expires_at", "expiry", "lease", "heartbeat"] {
        assert!(claim_json.get(key).is_none(), "checkout ownership must not expire");
    }
    let reopened = OrchestrationService::open(&fixture.config).unwrap();
    let (retried, retry_path) = reopened.reserve_dispatch_launch(&reserved, project, checkout, None, None).unwrap();
    let after = reopened.store.lock().unwrap().read().unwrap();
    assert_eq!(retry_path, path);
    assert_eq!(std::fs::read(&retry_path).unwrap(), credential_bytes, "retry must not rotate credentials");
    assert_eq!(retried.checkout_claim_id, reserved.checkout_claim_id);
    assert_eq!(serde_json::to_value(&retried.attachment).unwrap(), serde_json::to_value(&reserved.attachment).unwrap());
    assert_eq!(serde_json::to_value(&after.credentials).unwrap(), serde_json::to_value(&before.credentials).unwrap());
    assert_eq!(serde_json::to_value(&after.checkout_claims).unwrap(), serde_json::to_value(&before.checkout_claims).unwrap());
    credentials::validate_job_auth(&after, &auth).unwrap();
}

#[tokio::test]
async fn dispatch_baseline_freezes_actual_head_before_launch_and_survives_retry() {
    let fixture = ReviewFixture::reserved_worker();
    let root = fixture.root.join("git-checkout");
    std::fs::create_dir_all(&root).unwrap();
    for args in [
        vec!["init", "-q"],
        vec!["-c", "user.name=Fixture", "-c", "user.email=fixture@example.invalid",
            "commit", "-q", "--allow-empty", "-m", "baseline"],
    ] {
        assert!(tokio::process::Command::new("git").current_dir(&root).args(args)
            .status().await.unwrap().success());
    }
    let (project, checkout) = super::super::project_identity::resolve_directory(&root, None, None).await.unwrap();
    let baseline = setup_base_commit(fixture.run.setup.as_ref().unwrap(), &checkout).await.unwrap().unwrap();
    let (reserved, _) = fixture.service.reserve_dispatch_launch(
        &fixture.run, project.clone(), checkout.clone(), Some(baseline.clone()), None,
    ).unwrap();
    assert_eq!(reserved.setup.as_ref().unwrap().base_commit.as_deref(), Some(baseline.as_str()));
    assert!(tokio::process::Command::new("git").current_dir(&root)
        .args(["-c", "user.name=Fixture", "-c", "user.email=fixture@example.invalid",
            "commit", "-q", "--allow-empty", "-m", "later work"])
        .status().await.unwrap().success());
    let later = crate::space_git::git_line(&root, &["rev-parse", "--verify", "HEAD^{commit}"]).await.unwrap();
    assert_ne!(baseline, later);
    assert_eq!(setup_base_commit(reserved.setup.as_ref().unwrap(), &checkout).await.unwrap(),
        Some(baseline.clone()));
    let reopened = OrchestrationService::open(&fixture.config).unwrap();
    let (retried, _) = reopened.reserve_dispatch_launch(
        &reserved, project, checkout, Some(later), None,
    ).unwrap();
    assert_eq!(retried.setup.as_ref().unwrap().base_commit, Some(baseline));
}

#[tokio::test]
async fn dispatch_reservation_denies_other_root_same_or_nested_checkout_without_mutation() {
    for nesting in 0..3 {
        let fixture = ReviewFixture::reserved_worker();
        let outer = fixture.root.join("checkout");
        let inner = outer.join("nested");
        std::fs::create_dir_all(&inner).unwrap();
        let (first_path, second_path) = match nesting {
            0 => (&outer, &outer),
            1 => (&outer, &inner),
            _ => (&inner, &outer),
        };
        let (project, checkout) = super::super::project_identity::resolve_directory(first_path, None, None).await.unwrap();
        fixture.service.reserve_dispatch_launch(&fixture.run, project, checkout, None, None).unwrap();
        let other = fixture.another_worker();
        assert_ne!(other.root_id, fixture.run.root_id);
        assert_ne!(other.run_id, fixture.run.run_id);
        let (project, checkout) = super::super::project_identity::resolve_directory(second_path, None, None).await.unwrap();
        let before = fixture.service.store.lock().unwrap().read().unwrap();
        let error = fixture.service.reserve_dispatch_launch(&other, project, checkout, None, None).unwrap_err();
        assert_eq!(error.code, if nesting == 0 { "checkout_claim_conflict" } else { "checkout_identity_changed" });
        let after = fixture.service.store.lock().unwrap().read().unwrap();
        assert_eq!(serde_json::to_value(after).unwrap(), serde_json::to_value(before).unwrap(),
            "denied competing worker must not acquire an epoch, credential or claim");
    }
}

#[tokio::test]
async fn dispatch_reservation_refuses_changed_inode_without_rotating_epoch_or_claim() {
    let fixture = ReviewFixture::reserved_worker();
    let path = fixture.root.join("checkout");
    std::fs::create_dir_all(&path).unwrap();
    let (project, checkout) = super::super::project_identity::resolve_directory(&path, None, None).await.unwrap();
    let (reserved, credential_path) = fixture.service.reserve_dispatch_launch(
        &fixture.run, project.clone(), checkout.clone(), None, None,
    ).unwrap();
    let credential_bytes = std::fs::read(&credential_path).unwrap();
    std::fs::rename(&path, fixture.root.join("previous-checkout")).unwrap();
    std::fs::create_dir(&path).unwrap();
    let (fresh_project, fresh_checkout) = super::super::project_identity::resolve_directory(&path, None, None).await.unwrap();
    assert_eq!(fresh_checkout.key, checkout.key);
    assert_ne!(fresh_checkout.directory_identity, checkout.directory_identity);
    let before = fixture.service.store.lock().unwrap().read().unwrap();
    for (project, checkout) in [(project, checkout), (fresh_project, fresh_checkout)] {
        let error = fixture.service.reserve_dispatch_launch(&reserved, project, checkout, None, None).unwrap_err();
        assert_eq!(error.code, "checkout_identity_changed");
        let after = fixture.service.store.lock().unwrap().read().unwrap();
        assert_eq!(serde_json::to_value(after).unwrap(), serde_json::to_value(&before).unwrap());
        assert_eq!(std::fs::read(&credential_path).unwrap(), credential_bytes);
    }
}

#[cfg(target_os = "linux")]
#[tokio::test]
async fn root_main_exit_proof_is_same_boot_and_independent_of_worker_consumers() {
    use cockpit_protocol::orchestration::{AttachmentState, NativeProcessIdentity};
    let machine = crate::process_identity::machine_id().unwrap();
    let boot = crate::process_identity::kernel_boot_id().unwrap();
    for case in 0..11 {
        let fixture = ReviewFixture::new(RunStage::Active);
        fixture.change(|run| {
            let attachment = test_attachment(run);
            attachment.state = AttachmentState::Disconnected;
            attachment.machine_id = Some(machine.clone());
            attachment.process = Some(NativeProcessIdentity { pid: i32::MAX as u32, start_ticks: 1, kernel_boot_id: Some(boot.clone()) });
            attachment.launch_attempt = 1;
            let dispatch = run.dispatch.as_mut().unwrap();
            dispatch.step = DispatchStep::Launched;
            dispatch.launch_attempt = 1;
            match case {
                1 => test_attachment(run).machine_id = None,
                2 => test_attachment(run).machine_id = Some("different-machine".into()),
                3 => test_attachment(run).process = Some(NativeProcessIdentity {
                    pid: std::process::id(), start_ticks: crate::process_identity::start_identity(std::process::id() as i32).unwrap(),
                    kernel_boot_id: Some(boot.clone()),
                }),
                4 => test_attachment(run).process = None,
                7 => run.dispatch.as_mut().unwrap().step = DispatchStep::LaunchUnknown,
                8 => test_attachment(run).state = AttachmentState::Quiescing,
                9 => test_attachment(run).conversation = None,
                10 => test_attachment(run).process.as_mut().unwrap().kernel_boot_id = Some(uuid::Uuid::new_v4().to_string()),
                _ => {}
            }
        });
        let current = fixture.current().1;
        if case == 5 {
            let id = uuid::Uuid::new_v4().to_string();
            let mut child = crate::orchestration::new_run(&id, "test-session", RunKind::Worker,
                "Independently live worker".into(), &current.root_id, crate::orchestration::RunLineage { parent_run_id: Some(current.run_id.clone()), task_id: Some(uuid::Uuid::new_v4().to_string()) }, 1);
            let attachment = test_attachment(&mut child);
            attachment.machine_id = Some(machine.clone());
            attachment.process = Some(NativeProcessIdentity { pid: std::process::id(),
                start_ticks: crate::process_identity::start_identity(std::process::id() as i32).unwrap(), kernel_boot_id: Some(boot.clone()) });
            let locked = fixture.service.store.lock().unwrap();
            let mut state = locked.read().unwrap();
            state.runs.push(child);
            locked.save(&mut state).unwrap();
        }
        if case == 6 {
            fixture.change(|run| run.checkout_claim_id = Some(uuid::Uuid::new_v4().to_string()));
        }
        let expected = fixture.current().1;
        let before = fixture.service.store.lock().unwrap().read().unwrap();
        let mut absent = runtime();
        absent.panes.clear();
        absent.endpoint_identity = "restarted-endpoint".into();
        let herdr = CountingHerdr::new(absent, false, false);
        let observed = fixture.service.observe_root_resume(&herdr, &expected).await;
        assert!(observed.writer_exit_proof.is_none(), "root authority never claims worker consumer closure");
        if matches!(case, 0 | 5 | 9 | 10) {
            let proof = observed.root_main_recovery.unwrap();
            assert_eq!(proof.run_id, expected.run_id);
            assert_eq!(proof.attachment.process, expected.attachment.as_ref().unwrap().process);
            assert!(proof.idle_shell.is_none());
            if case == 9 {
                assert!(proof.attachment.conversation.is_none(), "optional absent conversation does not invalidate exact physical exit proof");
            }
        } else {
            assert!(observed.root_main_recovery.is_none(), "case {case}");
            assert!(observed.observation_cause.is_some(), "case {case}");
        }
        let after = fixture.service.store.lock().unwrap().read().unwrap();
        assert_eq!(serde_json::to_value(after).unwrap(), serde_json::to_value(before).unwrap(),
            "observation must not change epochs, worker claims or durable history");
    }
}

#[cfg(target_os = "linux")]
#[tokio::test]
async fn automatic_root_observation_never_replaces_a_stopped_main() {
    use cockpit_protocol::orchestration::{
        AttachmentState, NativeProcessIdentity, OperatorOrigin, OrchestrationAction,
        OrchestrationMutationRequest, ResumePhase,
    };
    for unavailable in [false, true] {
        let fixture = ReviewFixture::new(RunStage::Active);
        fixture.change(|run| {
            let attachment = test_attachment(run);
            attachment.state = AttachmentState::Disconnected;
            attachment.machine_id = crate::process_identity::machine_id();
            attachment.launch_attempt = 1;
            attachment.process = Some(NativeProcessIdentity {
                pid: i32::MAX as u32, start_ticks: 1, kernel_boot_id: crate::process_identity::kernel_boot_id(),
            });
            let dispatch = run.dispatch.as_mut().unwrap();
            dispatch.step = DispatchStep::Launched;
            dispatch.launch_attempt = 1;
            run.target = Some(DispatchTarget::ExistingSpace { workspace_id: "space".into() });
            run.setup = Some(SetupSummary {
                operation_id: None, generation: None, workspace_id: Some("space".into()), checkout_path: "/".into(),
                repository_id: None, branch: None, base: None, base_commit: None, ownership: None,
                effects: Vec::new(), warnings: Vec::new(), project_workspace_id: None,
            });
        });
        let old = fixture.current().1;
        fixture.staged_auth(&old);
        let mut absent = runtime();
        absent.panes.clear();
        if !unavailable {
            // Keep the ordinary missing-pane diagnostic outside the resume invariants.
            fixture.service.record_observation(&absent, &old.session_id).unwrap();
        }
        let herdr = Arc::new(CountingHerdr::new(absent.clone(), unavailable, false));
        let input = fixture.service.observe_root_resume(herdr.as_ref(), &old).await;
        assert!(input.attachment_observation.is_none());
        assert_eq!(input.root_main_recovery.is_some(), !unavailable);
        assert_eq!(input.observation_cause.is_some(), unavailable);
        let before = fixture.service.store.lock().unwrap().read().unwrap();
        assert_eq!(before.runs.len(), 1);
        assert!(before.operations.is_empty());
        assert!(old.resume.is_none());
        let credential_path = super::super::credentials::credential_path(
            fixture.service.base(), &before.credentials[0],
        ).unwrap();
        let credential_bytes = std::fs::read(&credential_path).unwrap();
        let before = serde_json::to_value(before).unwrap();
        let dispatcher = fixture.dispatcher(herdr.clone());
        for _ in 0..3 {
            dispatcher.observe().await;
            let stopped = fixture.current().1;
            assert!(stopped.resume.is_none(), "automatic observation must not launch or block the root");
            assert_eq!(stopped.stage, old.stage);
            assert_eq!(stopped.attachment.as_ref().unwrap().epoch, old.attachment.as_ref().unwrap().epoch);
            assert_eq!(stopped.dispatch.as_ref().unwrap().launch_attempt, old.dispatch.as_ref().unwrap().launch_attempt);
            assert_eq!(serde_json::to_value(fixture.service.store.lock().unwrap().read().unwrap()).unwrap(), before,
                "repeated observer cycles must preserve the root, session, conversation, credentials and operations");
            assert_eq!(std::fs::read(&credential_path).unwrap(), credential_bytes);
        }
        herdr.assert_read_only();

        // The same stopped root can still be restarted by an explicit operator action.
        let available = CountingHerdr::new(absent, false, false);
        let input = fixture.service.observe_root_resume(&available, &fixture.current().1).await;
        assert!(input.root_main_recovery.is_some());
        assert!(input.observation_cause.is_none());
        fixture.service.mutate_resolved(&super::super::Actor::Operator(OperatorOrigin::Native),
            OrchestrationMutationRequest {
                session_id: old.session_id.clone(), expected_revision: None,
                action: OrchestrationAction::RunResume { run_id: old.run_id.clone(), operation_id: uuid::Uuid::new_v4().to_string(), expected_attachment_epoch: Some(old.attachment.as_ref().unwrap().epoch), confirmed_stopped_attachment_id: None },
            }, &input).unwrap();
        let restarted = fixture.current().1;
        assert_eq!(restarted.resume.as_ref().unwrap().phase, ResumePhase::Launching,
            "Explicit restart refusal: {:?}", restarted.resume.as_ref().unwrap().cause);
        assert_eq!(restarted.run_id, old.run_id);
        assert_eq!(restarted.root_id, old.root_id);
        assert_eq!(restarted.session_id, old.session_id);
        assert_eq!(restarted.attachment.as_ref().unwrap().conversation, old.attachment.as_ref().unwrap().conversation);
        assert_eq!(restarted.attachment.as_ref().unwrap().epoch, old.attachment.as_ref().unwrap().epoch + 1);
        assert_eq!(restarted.dispatch.as_ref().unwrap().launch_attempt, old.dispatch.as_ref().unwrap().launch_attempt + 1);
        assert_eq!(restarted.stage, RunStage::Starting);
        assert_eq!(fixture.service.store.lock().unwrap().read().unwrap().runs.len(), 1);
        available.assert_read_only();
    }
}




struct CountingHerdr {
    observed: RuntimeView,
    error: bool,
    runtime_calls: std::sync::atomic::AtomicUsize,
    create_calls: std::sync::atomic::AtomicUsize,
    start_calls: std::sync::atomic::AtomicUsize,
    process_info: Option<super::super::herdr::PaneProcessInfo>,
    process_calls: std::sync::atomic::AtomicUsize,
    entered: tokio::sync::Semaphore,
    release: tokio::sync::Semaphore,
    managed_launch: bool,
    pause_create: bool,
    create_entered: tokio::sync::Semaphore,
    create_release: tokio::sync::Semaphore,
    expected_command: Option<Vec<String>>,
    expected_harness: String,
    prompt_calls: std::sync::atomic::AtomicUsize,
    prompt_requests: parking_lot::Mutex<Vec<super::super::herdr::AgentPromptRequest>>,
    prompt_unknown: bool,
    event_messages: parking_lot::Mutex<Option<tokio::sync::mpsc::Receiver<crate::SessionChange>>>,
    event_subscriptions: std::sync::atomic::AtomicUsize,
}
impl CountingHerdr {
    fn new(observed: RuntimeView, error: bool, paused: bool) -> Self {
        Self {
            observed,
            error,
            runtime_calls: 0.into(),
            create_calls: 0.into(),
            start_calls: 0.into(),
            process_info: None,
            process_calls: 0.into(),
            entered: tokio::sync::Semaphore::new(0),
            release: tokio::sync::Semaphore::new(usize::from(!paused)),
            managed_launch: false,
            pause_create: false,
            create_entered: tokio::sync::Semaphore::new(0),
            create_release: tokio::sync::Semaphore::new(0),
            expected_command: None,
            expected_harness: "omp".into(),
            prompt_calls: 0.into(),
            prompt_requests: parking_lot::Mutex::new(Vec::new()),
            prompt_unknown: false,
            event_messages: parking_lot::Mutex::new(None),
            event_subscriptions: 0.into(),
        }
    }
    fn assert_read_only(&self) {
        use std::sync::atomic::Ordering;
        assert!(self.runtime_calls.load(Ordering::SeqCst) > 0);
        assert_eq!(self.create_calls.load(Ordering::SeqCst), 0);
        assert_eq!(self.start_calls.load(Ordering::SeqCst), 0);
    }
}
#[async_trait::async_trait]
impl OrchestrationHerdr for CountingHerdr {
    async fn session_changes(&self, _: &str) -> Result<SessionSubscription, InspectionError> {
        self.event_subscriptions.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        self.event_messages.lock().take().map(|messages| SessionSubscription { messages })
            .ok_or_else(|| InspectionError::new("disconnected", "Fixture event stream unavailable"))
    }
    async fn runtime(&self, _: &str) -> Result<RuntimeView, InspectionError> {
        self.runtime_calls
            .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        self.entered.add_permits(1);
        let _permit = self.release.acquire().await.unwrap();
        if self.error {
            Err(InspectionError::new(
                "runtime_unavailable",
                "Fresh read failed",
            ))
        } else {
            let mut observed = self.observed.clone();
            if self.managed_launch && self.start_calls.load(std::sync::atomic::Ordering::SeqCst) > 0 {
                observed.panes[0].launch_pending = true;
            }
            Ok(observed)
        }
    }
    async fn create_agent_tab(
        &self,
        _: &str,
        _: &AgentTabRequest,
    ) -> Result<RunLocation, InspectionError> {
        self.create_calls
            .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        panic!("read-only launch review must not create an ordinary working terminal")
    }
    async fn create_direct_agent_tab(
        &self,
        session: &str,
        request: &super::super::herdr::DirectAgentTabRequest,
    ) -> Result<RunLocation, InspectionError> {
        assert!(self.managed_launch, "read-only launch review must not launch a direct agent");
        assert_eq!(session, "test-session");
        assert_eq!(request.endpoint_identity, "endpoint");
        assert_eq!(request.workspace_id, "space");
        let separator = request.command.iter().rposition(|argument| argument == "--").expect("literal harness boundary");
        assert_eq!(request.command.get(1).map(String::as_str), Some("agent-exec"));
        if let Some(expected) = &self.expected_command {
            assert_eq!(&request.command[separator + 1..], expected.as_slice());
        } else {
            assert_eq!(request.command.get(separator + 1).map(String::as_str), Some("omp"));
        }
        assert_eq!(request.env.get("COCKPIT_HARNESS"), Some(&self.expected_harness));
        assert!(!request.env.contains_key("COCKPIT_SESSION_ID"), "launch must not freeze or fabricate chat identity");
        assert!(!request.env.contains_key("COCKPIT_OMP_EXTENSION"));
        assert!(!request.env.contains_key("COCKPIT_WORKSPACE_ID"), "placement-dependent environment is released only after actual receipt");
        assert!(matches!(request.env.get("COCKPIT_INBOX_HOOK").map(String::as_str), Some("0" | "1")));
        assert_eq!(request.env.get("COCKPIT_LAUNCH_TAG"), Some(&request.launch_tag));
        let auth = super::super::credentials::read_job_auth(
            std::path::Path::new(request.env.get("COCKPIT_JOB_CREDENTIAL_FILE").unwrap()),
        ).unwrap();
        assert_eq!(request.env.get("COCKPIT_ATTACHMENT_ID"), Some(&auth.attachment_id));
        assert_eq!(request.env.get("COCKPIT_ATTACHMENT_EPOCH"), Some(&auth.attachment_epoch.to_string()));
        // One submitted direct effect creates its tab and starts the PTY
        // command together; pausing its response cannot split those effects.
        self.create_calls.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        self.start_calls.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        self.create_entered.add_permits(1);
        if self.pause_create {
            self.create_release.acquire().await.unwrap().forget();
        }
        let mut location = receipt();
        location.native_session_id = None;
        location.launch_tag = request.launch_tag.clone();
        Ok(location)
    }
    async fn pane_process_info(
        &self,
        _: &str,
        _: &str,
        pane_id: &str,
    ) -> Result<super::super::herdr::PaneProcessInfo, InspectionError> {
        self.process_calls.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        let info = self.process_info.clone().ok_or_else(|| {
            InspectionError::new("process_unavailable", "No foreground process evidence")
        })?;
        if pane_id != info.pane_id {
            return Err(InspectionError::new("stale_identity", "Requested pane is not the exact configured process-info source"));
        }
        assert_eq!(pane_id, "pane");
        Ok(info)
    }
    async fn prompt_agent(
        &self, session: &str, request: &super::super::herdr::AgentPromptRequest,
    ) -> Result<(), InspectionError> {
        assert_eq!(session, "test-session");
        assert_eq!(request.location.endpoint_identity, "endpoint");
        assert_eq!(request.location.pane_id, "pane");
        assert_eq!(request.location.terminal_id.as_deref(), Some("terminal"));
        assert_eq!(request.harness_kind, self.expected_harness);
        assert_eq!(request.location.workspace_id, self.observed.panes[0].workspace_id);
        assert_eq!(request.location.tab_id, self.observed.panes[0].tab_id);
        assert_eq!(request.location.launch_tag, self.observed.panes[0].tab_label);
        assert_eq!(request.location.native_session_id, self.observed.panes[0].native_session_id);
        assert!(!request.text.contains("private pending instruction"), "hints must not copy inbox message bodies");
        self.prompt_requests.lock().push(request.clone());
        self.prompt_calls.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        if self.prompt_unknown {
            Err(InspectionError::new("herdr_outcome_unknown", "Prompt response lost after submission"))
        } else {
            Ok(())
        }
    }
    async fn close_pane(&self, _: &str, _: &str, _: &str) -> Result<(), InspectionError> {
        panic!("launch review must not close a pane")
    }
}

#[tokio::test]
async fn blocked_consumer_waits_without_effects_or_revision_churn_then_direct_check_releases_it() {
    use std::sync::atomic::Ordering;
    let prerequisite = uuid::Uuid::new_v4().to_string();
    let mut fixture = ReviewFixture::new(RunStage::Starting);
    fixture.prepared_launch(&prerequisite, false);
    let mut adapter = CountingHerdr::new(runtime(), false, false);
    adapter.managed_launch = true;
    let adapter = Arc::new(adapter);
    let dispatcher = fixture.dispatcher(Arc::clone(&adapter));
    let before = fixture.current();
    let consumer_revision = {
        let locked = fixture.service.store.lock().unwrap();
        locked.tasks(&fixture.run.root_id).unwrap().task(fixture.run.task_id.as_deref().unwrap()).unwrap().task_revision.clone()
    };
    for _ in 0..2 {
        dispatcher.step(&fixture.run).await.unwrap();
    }
    assert_eq!(fixture.current().0, before.0);
    assert_eq!(adapter.create_calls.load(Ordering::SeqCst), 0);
    assert_eq!(adapter.start_calls.load(Ordering::SeqCst), 0);
    fixture.set_prerequisite(&prerequisite, true);
    {
        let locked = fixture.service.store.lock().unwrap();
        assert_eq!(locked.tasks(&fixture.run.root_id).unwrap().task(fixture.run.task_id.as_deref().unwrap()).unwrap().task_revision, consumer_revision);
    }
    dispatcher.step(&fixture.run).await.unwrap();
    assert_eq!(adapter.create_calls.load(Ordering::SeqCst), 1);
    assert_eq!(adapter.start_calls.load(Ordering::SeqCst), 1);
    assert_eq!(fixture.current().1.dispatch.unwrap().step, DispatchStep::LaunchPending);
}

#[tokio::test]
async fn prerequisite_reopened_during_runtime_read_prevents_launch_intent_and_tab() {
    let prerequisite = uuid::Uuid::new_v4().to_string();
    let mut fixture = ReviewFixture::new(RunStage::Starting);
    fixture.prepared_launch(&prerequisite, true);
    let adapter = Arc::new(CountingHerdr::new(runtime(), false, true));
    let dispatcher = fixture.dispatcher(Arc::clone(&adapter));
    let run = fixture.run.clone();
    let pending = tokio::spawn(async move { dispatcher.step(&run).await });
    adapter.entered.acquire().await.unwrap().forget();
    fixture.set_prerequisite(&prerequisite, false);
    let before = fixture.current();
    adapter.release.add_permits(1);
    pending.await.unwrap().unwrap();
    adapter.assert_read_only();
    assert_eq!(fixture.current().0, before.0);
    assert_eq!(fixture.current().1.dispatch.unwrap().step, DispatchStep::SetupPending);
}

#[tokio::test]
async fn prerequisite_reopened_during_direct_launch_ack_retains_receipt_without_replaying() {
    use std::sync::atomic::Ordering;
    let prerequisite = uuid::Uuid::new_v4().to_string();
    let mut fixture = ReviewFixture::new(RunStage::Starting);
    fixture.prepared_launch(&prerequisite, true);
    let mut adapter = CountingHerdr::new(runtime(), false, false);
    adapter.managed_launch = true;
    adapter.pause_create = true;
    let adapter = Arc::new(adapter);
    let dispatcher = fixture.dispatcher(Arc::clone(&adapter));
    let launched = Arc::clone(&dispatcher);
    let run = fixture.run.clone();
    let mut pending = tokio::spawn(async move { launched.step(&run).await });
    // Credential/setup errors can finish dispatch before creation is reached.
    // Surface that result instead of waiting forever for an impossible ACK.
    tokio::select! {
        permit = adapter.create_entered.acquire() => permit.unwrap().forget(),
        result = &mut pending => panic!(
            "dispatcher completed before the direct-launch ACK pause: {result:?}; dispatch state: {:?}",
            fixture.current().1.dispatch,
        ),
    }
    // Direct creation has already submitted OMP before this ACK pause.
    // A dependency reopened now cannot make that accepted effect unstarted:
    // retain its pending receipt rather than reset SetupPending or replay.
    fixture.set_prerequisite(&prerequisite, false);
    adapter.create_release.add_permits(1);
    pending.await.unwrap().unwrap();
    let (revision, waiting) = fixture.current();
    assert_eq!(waiting.dispatch.as_ref().unwrap().step, DispatchStep::LaunchPending);
    assert_eq!(waiting.attachment.as_ref().unwrap().launch_mode,
        Some(cockpit_protocol::orchestration::AttachmentLaunchMode::DirectProcess));
    assert!(waiting.dispatch.as_ref().unwrap().error.is_none());
    let receipt = serde_json::to_value(waiting.attachment.as_ref().and_then(|attachment| attachment.location.as_ref())).unwrap();
    assert!(waiting.attachment.as_ref().and_then(|attachment| attachment.location.as_ref()).is_some());
    assert_eq!(adapter.create_calls.load(Ordering::SeqCst), 1);
    assert_eq!(adapter.start_calls.load(Ordering::SeqCst), 1, "direct tab creation already submits its command");
    dispatcher.step(&waiting).await.unwrap();
    assert_eq!(fixture.current().0, revision, "pending launch observation must not replay an accepted effect");
    fixture.set_prerequisite(&prerequisite, true);
    dispatcher.step(&waiting).await.unwrap();
    let resumed = fixture.current().1;
    assert_eq!(adapter.create_calls.load(Ordering::SeqCst), 1);
    assert_eq!(adapter.start_calls.load(Ordering::SeqCst), 1);
    assert_eq!(serde_json::to_value(resumed.attachment.as_ref().and_then(|attachment| attachment.location.as_ref())).unwrap(), receipt);
    assert_eq!(resumed.dispatch.as_ref().unwrap().launch_attempt, waiting.dispatch.as_ref().unwrap().launch_attempt);
    assert_eq!(resumed.dispatch.as_ref().unwrap().step, DispatchStep::LaunchPending);
    assert_eq!(resumed.brief, waiting.brief);
}

#[tokio::test]
async fn blocked_owned_recovery_preserves_incarnation_without_consuming_retry() {
    let prerequisite = uuid::Uuid::new_v4().to_string();
    let mut fixture = ReviewFixture::new(RunStage::Starting);
    fixture.uncertain(RunKind::Worker);
    fixture.worker_task(Some((&prerequisite, true)));
    fixture.change(|run| {
        test_attachment(run).conversation = None;
        test_attachment(run).location.as_mut().unwrap().native_session_id = None;
    });
    fixture.run = fixture.current().1;
    let adapter = Arc::new(CountingHerdr::new(runtime(), false, true));
    let service = Arc::clone(&fixture.service);
    let run = fixture.run.clone();
    let observed = Arc::clone(&adapter);
    let pending = tokio::spawn(async move { recover_owned_launch(&service, observed.as_ref(), &run).await });
    adapter.entered.acquire().await.unwrap().forget();
    fixture.set_prerequisite(&prerequisite, false);
    let before = fixture.current();
    adapter.release.add_permits(1);
    pending.await.unwrap().unwrap();
    adapter.assert_read_only();
    let retained = fixture.current().1;
    assert_eq!(serde_json::to_value(&retained.attachment).unwrap(), serde_json::to_value(&before.1.attachment).unwrap());
    assert_eq!(retained.dispatch.as_ref().unwrap().launch_attempt, before.1.dispatch.as_ref().unwrap().launch_attempt);
    let settled = serde_json::to_value(fixture.service.store.lock().unwrap().read().unwrap()).unwrap();
    recover_owned_launch(&fixture.service, adapter.as_ref(), &retained).await.unwrap();
    assert_eq!(serde_json::to_value(fixture.service.store.lock().unwrap().read().unwrap()).unwrap(), settled);
}

#[tokio::test]
async fn blocked_consumer_does_not_prevent_independent_worker_in_same_root_from_launching() {
    use std::sync::atomic::Ordering;
    let prerequisite = uuid::Uuid::new_v4().to_string();
    let mut fixture = ReviewFixture::new(RunStage::Starting);
    fixture.prepared_launch(&prerequisite, false);
    let independent_task = uuid::Uuid::new_v4().to_string();
    let independent_id = uuid::Uuid::new_v4().to_string();
    let mut independent = crate::orchestration::new_run(
        &independent_id, "test-session", RunKind::Worker, "Independent".into(),
        &fixture.run.root_id, crate::orchestration::RunLineage { parent_run_id: Some(fixture.run.root_id.clone()), task_id: Some(independent_task.clone()) }, 1,
    );
    independent.stage = RunStage::Starting;
    independent.setup = fixture.run.setup.clone();
    independent.target = fixture.run.target.clone();
    independent.dispatch = Some(crate::orchestration::dispatch(DispatchStep::SetupPending));
    {
        let locked = fixture.service.store.lock().unwrap();
        locked.tasks(&fixture.run.root_id).unwrap().create_authoring_with_id(
            &independent_task, "Independent", "", &[], None, (None, None),
        ).unwrap();
        independent.task_revision_at_start = Some(locked.tasks(&fixture.run.root_id).unwrap()
            .task(&independent_task).unwrap().task_revision.clone());
        let mut state = locked.read().unwrap();
        independent.assignment_generation = Some(1);
        state.task_ownership.push(cockpit_protocol::orchestration::TaskOwnership {
            root_id: independent.root_id.clone(), task_id: independent_task.clone(),
            generation: 0, owner: None, schedule: None,
        });
        state.runs.push(independent.clone());
        locked.save(&mut state).unwrap();
    }
    let mut adapter = CountingHerdr::new(runtime(), false, false);
    adapter.managed_launch = true;
    let adapter = Arc::new(adapter);
    let dispatcher = fixture.dispatcher(Arc::clone(&adapter));
    let waiting = serde_json::to_value(&fixture.current().1).unwrap();
    dispatcher.step(&fixture.run).await.unwrap();
    dispatcher.step(&independent).await.unwrap();
    assert_eq!(adapter.create_calls.load(Ordering::SeqCst), 1);
    assert_eq!(adapter.start_calls.load(Ordering::SeqCst), 1);
    assert_eq!(serde_json::to_value(&fixture.current().1).unwrap(), waiting);
    let revision = fixture.current().0;
    dispatcher.step(&fixture.run).await.unwrap();
    assert_eq!(fixture.current().0, revision);
    assert_eq!(fixture.service.run_for_review("test-session", &independent_id).unwrap().dispatch.unwrap().step, DispatchStep::LaunchPending);
}

#[tokio::test]
async fn blocked_task_physical_observation_does_not_grant_task_or_writer_authority() {
    let prerequisite = uuid::Uuid::new_v4().to_string();
    let mut fixture = ReviewFixture::new(RunStage::Starting);
    fixture.uncertain(RunKind::Worker);
    fixture.worker_task(Some((&prerequisite, false)));
    let location = serde_json::to_value(fixture.run.attachment.as_ref().and_then(|attachment| attachment.location.as_ref())).unwrap();
    let before = fixture.service.store.lock().unwrap().read().unwrap();
    let mut observed = runtime();
    observed.panes[0].agent_kind = Some("omp".into());
    observed.panes[0].native_session_id = Some("native".into());
    let adapter = CountingHerdr::new(observed, false, false);
    reconcile_current_launch(&fixture.service, &adapter, &fixture.run, 0).await.unwrap();
    adapter.assert_read_only();
    let current = fixture.current().1;
    // Physical readiness may settle the launch receipt, but cannot register
    // a conversation, claim a task, or promote its semantic lifecycle.
    assert_eq!(current.stage, RunStage::Starting);
    assert_eq!(current.attachment.as_ref().unwrap().conversation,
        fixture.run.attachment.as_ref().unwrap().conversation);
    assert!(current.attachment.as_ref().unwrap().process.is_none());
    let after = fixture.service.store.lock().unwrap().read().unwrap();
    assert!(after.credentials.is_empty());
    assert!(after.task_ownership.iter().all(|ownership| ownership.owner.is_none()));
    assert_eq!(serde_json::to_value(&after.task_ownership).unwrap(),
        serde_json::to_value(&before.task_ownership).unwrap());
    assert_eq!(serde_json::to_value(&after.checkout_claims).unwrap(),
        serde_json::to_value(&before.checkout_claims).unwrap());
    assert_eq!(serde_json::to_value(&after.messages).unwrap(),
        serde_json::to_value(&before.messages).unwrap());
    assert_eq!(serde_json::to_value(current.attachment.as_ref().and_then(|attachment| attachment.location.as_ref())).unwrap(), location);
}

#[tokio::test]
async fn physical_launch_review_preserves_lifecycle_without_conversation_registration() {
    for outcome in 0..10 {
        for stage in [
            RunStage::Active,
            RunStage::Working,
            RunStage::Reported,
        ] {
            let mut fixture = ReviewFixture::new(stage);
            fixture.change(|run| test_attachment(run).conversation = None);
            fixture.run = fixture.current().1;
            let mut observed = runtime();
            observed.panes[0].agent_name = Some("tag".into());
            observed.panes[0].agent_kind = Some("omp".into());
            observed.panes[0].native_session_id = Some("native".into());
            let expected = match outcome {
                0 => DispatchStep::Launched,
                1 => {
                    observed.panes[0].agent_name = None;
                    DispatchStep::Launched
                }
                2 => {
                    observed.panes.clear();
                    DispatchStep::LaunchUnknown
                }
                3 => {
                    observed.endpoint_identity = "restarted".into();
                    DispatchStep::NeedsReview
                }
                4 => {
                    observed.panes.push(observed.panes[0].clone());
                    DispatchStep::NeedsReview
                }
                6 => {
                    fixture.change(|run| test_attachment(run).location = None);
                    fixture.run = fixture.current().1;
                    DispatchStep::NeedsReview
                }
                7 => {
                    fixture.change(|run| {
                        test_attachment(run).location.as_mut().unwrap().launch_tag = "conflicting-tag".into()
                    });
                    fixture.run = fixture.current().1;
                    DispatchStep::NeedsReview
                }
                8 => {
                    observed.panes[0].native_session_id = None;
                    DispatchStep::LaunchUnknown
                }
                9 => {
                    observed.panes[0].agent_name = Some("renamed".into());
                    DispatchStep::Launched
                }
                _ => DispatchStep::NeedsReview,
            };
            let herdr = CountingHerdr::new(observed, outcome == 5, false);
            reconcile_current_launch(&fixture.service, &herdr, &fixture.run, 0)
                .await
                .unwrap();
            herdr.assert_read_only();
            let (_, current) = fixture.current();
            assert_eq!(current.dispatch.as_ref().unwrap().step, expected);
            let mut preserved = fixture.run.clone();
            preserved.dispatch = current.dispatch.clone();
            preserved.updated_at = current.updated_at.clone();
            if expected == DispatchStep::Launched {
                assert_eq!(current.attachment.as_ref().unwrap().state,
                    cockpit_protocol::orchestration::AttachmentState::Disconnected);
                assert_eq!(current.dispatch.as_ref().unwrap().error.as_ref().unwrap().code,
                    "run_unregistered");
                preserved.attachment.as_mut().unwrap().state =
                    cockpit_protocol::orchestration::AttachmentState::Disconnected;
            }
            assert_eq!(
                serde_json::to_value(current).unwrap(),
                serde_json::to_value(preserved).unwrap()
            );
        }
    }
}

#[tokio::test]
async fn late_review_cannot_rewrite_explicit_retry_or_drop_root_result() {
    for retry in [true, false] {
        let fixture = ReviewFixture::new(RunStage::Active);
        let mut observed = runtime();
        observed.panes[0].agent_name = Some("tag".into());
        observed.panes[0].agent_kind = Some("omp".into());
        observed.panes[0].native_session_id = Some("native".into());
        let herdr = Arc::new(CountingHerdr::new(observed, false, true));
        let service = Arc::clone(&fixture.service);
        let reviewed = fixture.run.clone();
        let adapter = Arc::clone(&herdr);
        let pending =
            tokio::spawn(
                async move { reconcile_current_launch(&service, adapter.as_ref(), &reviewed, 0).await },
            );
        herdr.entered.acquire().await.unwrap().forget();
        fixture.change(|run| {
            if retry {
                let dispatch = run.dispatch.as_mut().unwrap();
                dispatch.launch_attempt += 1;
                dispatch.agent_started = false;
                dispatch.step = DispatchStep::SetupPending;
                test_attachment(run).location = None;
                test_attachment(run).conversation = None;
                test_attachment(run).shell = None;
                run.stage = RunStage::Starting;
            } else {
                run.result = Some(cockpit_protocol::orchestration::Report {
                    message_id: "fresh-result".into(),
                    kind: cockpit_protocol::orchestration::ReportKind::Result,
                    outcome: Some(cockpit_protocol::orchestration::ReportOutcome::Succeeded),
                    summary: "A root result while still Active".into(),
                    plan: None,
                    source_output: None,
                    at: now(),
                });
            }
        });
        let before = fixture.current();
        herdr.release.add_permits(1);
        pending.await.unwrap().unwrap();
        herdr.assert_read_only();
        let after = fixture.current();
        if retry {
            assert_eq!(after.0, before.0, "obsolete review must not save");
            assert_eq!(
                serde_json::to_value(after.1).unwrap(),
                serde_json::to_value(before.1).unwrap()
            );
        } else {
            assert_eq!(after.1.stage, RunStage::Active);
            assert_eq!(after.1.result.unwrap().message_id, "fresh-result");
            assert_eq!(after.1.dispatch.unwrap().step, DispatchStep::Launched);
        }
    }
}

#[tokio::test]
async fn exact_physical_receipt_resolves_launch_without_registering_conversation() {
    for kind in [RunKind::Supervisor, RunKind::Worker] {
        for name in [None, Some("renamed")] {
            let mut fixture = ReviewFixture::new(RunStage::Starting);
            fixture.uncertain(kind);
            let original = fixture.run.clone();
            let mut observed = runtime();
            observed.panes[0].agent_name = name.map(str::to_owned);
            observed.panes[0].agent_kind = Some("omp".into());
            observed.panes[0].native_session_id = Some("native".into());
            let herdr = CountingHerdr::new(observed, false, false);
            reconcile_current_launch(&fixture.service, &herdr, &original, 0).await.unwrap();
            let current = fixture.current().1;
            assert_eq!(current.stage, RunStage::Starting);
            assert!(current.dispatch.as_ref().unwrap().agent_started);
            assert_eq!(current.dispatch.as_ref().unwrap().step, DispatchStep::Launched);
            assert_eq!(current.attachment.as_ref().unwrap().state,
                cockpit_protocol::orchestration::AttachmentState::Disconnected);
            assert!(current.attachment.as_ref().unwrap().process.is_none());
            let state = fixture.service.store.lock().unwrap().read().unwrap();
            assert!(state.credentials.is_empty(), "physical observation does not mint semantic capabilities");
            assert!(state.task_ownership.iter().all(|ownership| ownership.owner.is_none()));
            assert_eq!(state.messages.len(), 1);
            reconcile_current_launch(&fixture.service, &herdr, &current, 0).await.unwrap();
            let reread = fixture.current().1;
            assert_eq!(reread.stage, RunStage::Starting);
            assert_eq!(reread.dispatch.as_ref().unwrap().step, DispatchStep::Launched);
            herdr.assert_read_only();
        }
    }
}

#[tokio::test]
async fn unknown_launch_rejects_replacement_and_keeps_unproven_identity_unknown() {
    use cockpit_protocol::orchestration::RunKind;
    for mismatch in 0..7 {
        let mut fixture = ReviewFixture::new(RunStage::Starting);
        fixture.uncertain(RunKind::Supervisor);
        let mut observed = runtime();
        observed.panes[0].agent_kind = Some("omp".into());
        observed.panes[0].native_session_id = Some("native".into());
        match mismatch {
            0 => observed.panes[0].native_session_id = Some("replacement".into()),
            1 => observed.panes[0].terminal_id = Some("replacement".into()),
            2 => observed.panes[0].pane_id = "replacement".into(),
            3 => observed.panes[0].workspace_id = "replacement".into(),
            4 => observed.panes[0].tab_id = "replacement".into(),
            5 => {
                fixture.change(|run| test_attachment(run).location.as_mut().unwrap().session_id = "replacement".into());
                fixture.run = fixture.current().1;
            }
            _ => observed.panes[0].native_session_id = None,
        }
        let before = fixture.service.store.lock().unwrap().read().unwrap();
        let herdr = CountingHerdr::new(observed, false, false);
        reconcile_current_launch(&fixture.service, &herdr, &fixture.run, 0).await.unwrap();
        let (_, current) = fixture.current();
        assert_eq!(current.stage, RunStage::Starting);
        assert!(!current.dispatch.as_ref().unwrap().agent_started);
        assert!(!matches!(current.dispatch.as_ref().unwrap().step, DispatchStep::Launched));
        assert_eq!(serde_json::to_value(&current.attachment).unwrap(),
            serde_json::to_value(&fixture.run.attachment).unwrap());
        let after = fixture.service.store.lock().unwrap().read().unwrap();
        assert!(after.credentials.is_empty());
        assert!(after.task_ownership.iter().all(|ownership| ownership.owner.is_none()));
        assert_eq!(serde_json::to_value(&after.checkout_claims).unwrap(),
            serde_json::to_value(&before.checkout_claims).unwrap());
        assert_eq!(serde_json::to_value(&after.messages).unwrap(),
            serde_json::to_value(&before.messages).unwrap());
        herdr.assert_read_only();
    }
}

#[cfg(target_os = "linux")]
#[tokio::test]
async fn process_fallback_requires_original_incarnation_in_pinned_foreground_pane() {
    use cockpit_protocol::orchestration::{NativeProcessIdentity, RunKind};
    let mut child = tokio::process::Command::new("sh").args(["-c", "read line"])
        .stdin(std::process::Stdio::piped()).kill_on_drop(true).spawn().unwrap();
    let pid = child.id().unwrap();
    let identity = NativeProcessIdentity {
        pid, start_ticks: crate::process_identity::start_identity(pid as i32).unwrap(),
        kernel_boot_id: crate::process_identity::kernel_boot_id(),
    };
    for mismatch in 0..11 {
        let mut fixture = ReviewFixture::new(RunStage::Starting);
        let kind = if mismatch == 10 { RunKind::Worker } else { RunKind::Supervisor };
        fixture.uncertain(kind);
        fixture.change(|run| {
            test_attachment(run).conversation = None;
            test_attachment(run).process = Some(identity.clone());
            test_attachment(run).machine_id = crate::process_identity::machine_id();
        });
        fixture.run = fixture.current().1;
        let mut observed = runtime();
        observed.panes[0].agent_kind = Some("omp".into());
        let mut herdr = CountingHerdr::new(observed, false, false);
        let mut info = super::super::herdr::PaneProcessInfo {
            pane_id: "pane".into(), shell_pid: Some(1), foreground_pgid: Some(pid),
            processes: vec![(pid, "omp".into())], shell_identity: None,
        };
        match mismatch {
            1 => fixture.change(|run| test_attachment(run).process.as_mut().unwrap().start_ticks += 1),
            2 => fixture.change(|run| test_attachment(run).process.as_mut().unwrap().kernel_boot_id = Some("other-boot".into())),
            3 => fixture.change(|run| test_attachment(run).process.as_mut().unwrap().kernel_boot_id = None),
            4 => info.processes = vec![(pid.saturating_add(1), "replacement-omp".into())],
            5 => info.foreground_pgid = None,
            6 => info.pane_id = "replacement-pane".into(),
            7 => info.processes.clear(),
            8 => fixture.change(|run| test_attachment(run).process.as_mut().unwrap().pid = u32::MAX),
            _ => {}
        }
        fixture.run = fixture.current().1;
        if mismatch == 1 { fixture.stopped_launch_shell().await; }
        if mismatch != 9 { herdr.process_info = Some(info); }
        reconcile_current_launch(&fixture.service, &herdr, &fixture.run, 0).await.unwrap();
        let current = fixture.current().1;
        let proven = matches!(mismatch, 0 | 10);
        assert_eq!(current.stage, RunStage::Starting,
            "physical process proof without conversation registration must not authorize work");
        assert_eq!(current.dispatch.as_ref().unwrap().step,
            if proven { DispatchStep::Launched } else { DispatchStep::NeedsReview });
        if proven {
            let attachment = current.attachment.as_ref().unwrap();
            assert_eq!(attachment.state, cockpit_protocol::orchestration::AttachmentState::Disconnected);
            assert_eq!(current.dispatch.as_ref().unwrap().error.as_ref().unwrap().code, "run_unregistered");
        }
        assert_eq!(current.attachment.as_ref().and_then(|attachment| attachment.process.as_ref()), fixture.run.attachment.as_ref().and_then(|attachment| attachment.process.as_ref()));
        assert_eq!(current.attachment.as_ref().and_then(|attachment| attachment.conversation.as_ref()), fixture.run.attachment.as_ref().and_then(|attachment| attachment.conversation.as_ref()));
        assert_eq!(serde_json::to_value(current.attachment.as_ref().and_then(|attachment| attachment.location.as_ref())).unwrap(),
            serde_json::to_value(fixture.run.attachment.as_ref().and_then(|attachment| attachment.location.as_ref())).unwrap());
        let messages = fixture.service.store.lock().unwrap().read().unwrap().messages;
        let brief_kind = if kind == RunKind::Worker {
            cockpit_protocol::orchestration::MessageKind::WorkBrief
        } else {
            cockpit_protocol::orchestration::MessageKind::SupervisorBrief
        };
        assert_eq!(messages.iter().filter(|message| message.kind == brief_kind).count(), 0);
        assert!(messages.iter().any(|message| message.message_id == "preserved-message"));
        if proven {
            let before = serde_json::to_value(fixture.service.store.lock().unwrap().read().unwrap()).unwrap();
            reconcile_current_launch(&fixture.service, &herdr, &current, 0).await.unwrap();
            assert_eq!(serde_json::to_value(fixture.service.store.lock().unwrap().read().unwrap()).unwrap(), before);
        }
        herdr.assert_read_only();
    }
    child.kill().await.unwrap();
    child.wait().await.unwrap();
    let mut fixture = ReviewFixture::new(RunStage::Starting);
    fixture.uncertain(RunKind::Supervisor);
    fixture.change(|run| test_attachment(run).process = Some(identity));
    fixture.stopped_launch_shell().await;
    fixture.run = fixture.current().1;
    let mut observed = runtime();
    observed.panes[0].agent_kind = Some("omp".into());
    let herdr = CountingHerdr::new(observed, false, false);
    reconcile_current_launch(&fixture.service, &herdr, &fixture.run, 0).await.unwrap();
    let retained = fixture.current().1;
    assert_eq!(retained.dispatch.as_ref().unwrap().step, DispatchStep::NeedsReview);
    assert_eq!(retained.dispatch.as_ref().unwrap().launch_attempt, fixture.run.dispatch.as_ref().unwrap().launch_attempt);
    assert_eq!(serde_json::to_value(&retained.attachment).unwrap(), serde_json::to_value(&fixture.run.attachment).unwrap());
    assert_eq!(herdr.process_calls.load(std::sync::atomic::Ordering::SeqCst), 0);
    herdr.assert_read_only();
}

#[cfg(target_os = "linux")]
#[tokio::test]
async fn failed_owned_start_retains_same_boot_container_and_run_state_without_retry() {
    use cockpit_protocol::orchestration::RunKind;
    for kind in [RunKind::Supervisor, RunKind::Worker] {
        let mut fixture = ReviewFixture::new(RunStage::Starting);
        fixture.uncertain(kind);
        fixture.change(|run| {
            test_attachment(run).conversation = None;
            test_attachment(run).location.as_mut().unwrap().native_session_id = None;
            // Historical operator retries do not consume automatic budget.
            run.dispatch.as_mut().unwrap().launch_attempt = 4;
        });
        fixture.stopped_launch_shell().await;
        fixture.run = fixture.current().1;
        // Reserve real credentials/claims without inventing a current
        // credential-authenticated extension registration.
        fixture.staged_auth(&fixture.run);
        fixture.run = fixture.current().1;
        let before = fixture.service.store.lock().unwrap().read().unwrap();
        let original = fixture.run.clone();
        let mut observed = runtime();
        observed.panes[0].launch_pending = true;
        observed.panes[0].agent_kind = Some("omp".into());
        observed.panes[0].native_session_id = Some("unbound-raw-main".into());
        observed.panes[0].tab_label = "renamed by user".into();
        let herdr = CountingHerdr::new(observed.clone(), false, false);
        reconcile_current_launch(&fixture.service, &herdr, &original, 0).await.unwrap();
        let fresh = fixture.current().1;
        herdr.assert_read_only();
        assert_eq!(fresh.dispatch.as_ref().unwrap().step, DispatchStep::NeedsReview);
        assert_eq!(fresh.dispatch.as_ref().unwrap().error.as_ref().unwrap().code, "writer_exit_unproven");
        assert_eq!(fresh.dispatch.as_ref().unwrap().launch_attempt, original.dispatch.as_ref().unwrap().launch_attempt);
        assert_eq!(serde_json::to_value(&fresh.attachment).unwrap(), serde_json::to_value(&original.attachment).unwrap());
        assert_eq!(fresh.task_id, original.task_id);
        assert_eq!(fresh.brief, original.brief);
        assert_eq!(fresh.checkout_claim_id, original.checkout_claim_id);
        let after = fixture.service.store.lock().unwrap().read().unwrap();
        assert_eq!(serde_json::to_value(&after.checkout_claims).unwrap(), serde_json::to_value(&before.checkout_claims).unwrap());
        assert_eq!(serde_json::to_value(&after.credentials).unwrap(), serde_json::to_value(&before.credentials).unwrap());
        assert!(fixture.service.store.lock().unwrap().read().unwrap().messages.iter()
            .any(|m| m.message_id == "preserved-message" && m.text == "Existing operator message"));
        let second = fixture.current().1;
        let adapter = CountingHerdr::new(observed, false, false);
        reconcile_current_launch(&fixture.service, &adapter, &second, 0).await.unwrap();
        let retained = fixture.current().1;
        assert_eq!(serde_json::to_value(&retained.attachment).unwrap(), serde_json::to_value(&original.attachment).unwrap());
        assert_eq!(retained.dispatch.as_ref().unwrap().launch_attempt, original.dispatch.as_ref().unwrap().launch_attempt);
        assert_eq!(retained.stage, original.stage);
        let after = fixture.service.store.lock().unwrap().read().unwrap();
        assert_eq!(serde_json::to_value(&after.checkout_claims).unwrap(), serde_json::to_value(&before.checkout_claims).unwrap());
        assert_eq!(serde_json::to_value(&after.credentials).unwrap(), serde_json::to_value(&before.credentials).unwrap());
        adapter.assert_read_only();
    }
}

#[cfg(target_os = "linux")]
#[tokio::test]
async fn live_launch_shell_and_later_exit_never_authorize_same_boot_container_cancellation() {
    let mut fixture = ReviewFixture::new(RunStage::Starting);
    fixture.uncertain(RunKind::Supervisor);
    fixture.change(|run| {
        test_attachment(run).conversation = None;
        test_attachment(run).location.as_mut().unwrap().native_session_id = None;
    });
    let mut shell = fixture.running_launch_shell().await;
    let herdr = CountingHerdr::new(runtime(), false, false);
    recover_owned_launch(&fixture.service, &herdr, &fixture.run).await.unwrap();
    let retained = fixture.current().1;
    assert!(!recovery_close_pending(&retained));
    assert!(!cancelled_launch_processes_stopped(&retained).unwrap());
    assert_eq!(retained.dispatch.as_ref().unwrap().step, DispatchStep::NeedsReview);
    assert_eq!(retained.dispatch.as_ref().unwrap().error.as_ref().unwrap().code,
        "launch_shell_activity_unproven");
    assert_eq!(retained.dispatch.as_ref().unwrap().launch_attempt, 1);
    assert_eq!(serde_json::to_value(&retained.attachment).unwrap(),
        serde_json::to_value(&fixture.run.attachment).unwrap());
    herdr.assert_read_only();
    recover_owned_launch(&fixture.service, &herdr, &retained).await.unwrap();
    herdr.assert_read_only();
    assert_eq!(fixture.current().1.dispatch.as_ref().unwrap().launch_attempt, 1);
    shell.kill().await.unwrap();
    shell.wait().await.unwrap();
    assert!(cancelled_launch_processes_stopped(&retained).unwrap());
    recover_owned_launch(&fixture.service, &herdr, &fixture.current().1).await.unwrap();
    let stopped = fixture.current().1;
    assert_eq!(stopped.dispatch.as_ref().unwrap().step, DispatchStep::NeedsReview);
    assert_eq!(stopped.dispatch.as_ref().unwrap().launch_attempt, retained.dispatch.as_ref().unwrap().launch_attempt);
    assert_eq!(stopped.dispatch.as_ref().unwrap().error.as_ref().unwrap().code, "writer_exit_unproven");
    assert_eq!(serde_json::to_value(&stopped.attachment).unwrap(), serde_json::to_value(&retained.attachment).unwrap());
    herdr.assert_read_only();
}

#[tokio::test]
async fn absent_layout_with_missing_original_shell_identity_never_sends_fresh_launch() {
    let mut fixture = ReviewFixture::new(RunStage::Starting);
    fixture.uncertain(RunKind::Supervisor);
    fixture.change(|run| {
        test_attachment(run).conversation = None;
        test_attachment(run).location.as_mut().unwrap().native_session_id = None;
        test_attachment(run).shell = None;
    });
    fixture.run = fixture.current().1;
    let herdr = CountingHerdr::new(runtime(), false, false);
    recover_owned_launch(&fixture.service, &herdr, &fixture.run).await.unwrap();
    let retained = fixture.current().1;
    assert_eq!(retained.dispatch.as_ref().unwrap().launch_attempt, 1);
    assert_eq!(retained.dispatch.as_ref().unwrap().error.as_ref().unwrap().code,
        "launch_shell_activity_unproven");
    assert!(retained.attachment.as_ref().and_then(|attachment| attachment.location.as_ref()).is_some());
    herdr.assert_read_only();
}

#[tokio::test]
async fn interrupted_preservation_intent_retains_old_pane_and_never_replays_creation() {
    {
        let mut fixture = ReviewFixture::new(RunStage::Starting);
        fixture.uncertain(RunKind::Supervisor);
        fixture.change(|run| {
            test_attachment(run).conversation = None;
            test_attachment(run).location.as_mut().unwrap().native_session_id = None;
        });
        fixture.stopped_launch_shell().await;
        fixture.run = fixture.current().1;
        let herdr = CountingHerdr::new(runtime(), false, false);
        // Reconcile an already persisted intent, never create a new effect.
        fixture.change(|run| {
            let d = run.dispatch.as_mut().unwrap();
            d.step = DispatchStep::NeedsReview;
            d.error = Some(ErrorResponse { code: RECOVERY_PRESERVE_INTENT.into(), message: "Interrupted creation".into() });
            run.annotations.push(cockpit_protocol::orchestration::Annotation {
                by: cockpit_protocol::orchestration::ActorRef::Dispatcher,
                text: format!("{RECOVERY_ATTEMPT_ANNOTATION}; launch_attempt={}", d.launch_attempt), at: now(),
            });
        });
        let intent = fixture.current().1;
        assert!(!fixture.service.finish_launch_recovery(&intent).unwrap(),
            "missing ordinary terminal receipt must never authorize retry");
        finish_owned_launch_recovery(&fixture.service, &herdr, &intent, 0).await.unwrap();
        let current = fixture.current().1;
        assert_eq!(current.dispatch.as_ref().unwrap().error.as_ref().unwrap().code,
            "automatic_space_preservation_unproven");
        assert_eq!(current.attachment.as_ref().and_then(|attachment| attachment.location.as_ref()).unwrap().terminal_id, fixture.run.attachment.as_ref().and_then(|attachment| attachment.location.as_ref()).unwrap().terminal_id);
        assert_eq!(herdr.create_calls.load(std::sync::atomic::Ordering::SeqCst), 0);
        let settled = serde_json::to_value(fixture.service.store.lock().unwrap().read().unwrap()).unwrap();
        finish_owned_launch_recovery(&fixture.service, &herdr, &current, 0).await.unwrap();
        assert_eq!(serde_json::to_value(fixture.service.store.lock().unwrap().read().unwrap()).unwrap(), settled);
    }
}


#[test]
fn ownership_rejects_foreign_split_reused_terminal_and_endpoint_not_labels() {
    let fixture = ReviewFixture::new(RunStage::Starting);
    let mut view = runtime();
    view.panes[0].tab_label = "renamed".into();
    assert!(owned_launch_pane(&view, &fixture.run).is_ok());
    for conflict in 0..4 {
        let mut view = view.clone();
        match conflict {
            0 => view.endpoint_identity = "reused endpoint".into(),
            1 => {
                let mut foreign = view.panes[0].clone();
                foreign.pane_id = "foreign".into();
                foreign.terminal_id = Some("foreign".into());
                view.panes.push(foreign);
            }
            2 => view.panes[0].terminal_id = Some("reused".into()),
            _ => view.panes[0].workspace_id = "foreign".into(),
        }
        assert!(owned_launch_pane(&view, &fixture.run).is_err());
        assert!(!owned_launch_absent(&view, &fixture.run));
    }
}

#[test]
fn root_registration_pins_generic_conversation_without_physical_process_evidence() {
    use cockpit_protocol::orchestration::{ConversationIdentity, OrchestrationAction, OrchestrationMutationRequest};
    use super::super::{Actor, AgentCaller};
    let fixture = ReviewFixture::new(RunStage::Active);
    fixture.change(|run| {
        test_attachment(run).conversation = None;
        test_attachment(run).process = None;
        test_attachment(run).shell = None;
        test_attachment(run).state = cockpit_protocol::orchestration::AttachmentState::Disconnected;
    });
    let old = fixture.current().1;
    let actor = Actor::Agent(Box::new(AgentCaller {
        auth: fixture.staged_auth(&old),
        conversation: Some(ConversationIdentity { harness: "omp".into(), session_id: "opaque/non-uuid chat".into() }),
        assignment_generation: None,
        endpoint_identity: String::new(), session_id: String::new(), workspace_id: String::new(),
        tab_id: String::new(), pane_id: String::new(), boot_id: None, terminal_id: None,
        native_session_id: None, actual_agent_kind: None, process: None,
    }));
    let action = OrchestrationAction::RunRegister { operation_id: uuid::Uuid::new_v4().to_string() };
    fixture.service.mutate(&actor, OrchestrationMutationRequest {
        session_id: old.session_id.clone(), expected_revision: None, action: action.clone(),
    }).unwrap();
    let fresh = fixture.current().1;
    let registered = fixture.service.store.lock().unwrap().read().unwrap();
    let wrong_route = fixture.service.mutate(&actor, OrchestrationMutationRequest {
        session_id: "different-stored-route".into(), expected_revision: None, action: action.clone(),
    }).unwrap_err();
    assert_eq!(wrong_route.code, "session_mismatch");
    assert_eq!(serde_json::to_value(fixture.service.store.lock().unwrap().read().unwrap()).unwrap(),
        serde_json::to_value(registered).unwrap(), "a valid conversation cannot redirect its stored run");
    assert_eq!(fresh.attachment.as_ref().unwrap().conversation.as_ref().unwrap().session_id, "opaque/non-uuid chat");
    assert_eq!(fresh.stage, old.stage);
    assert_eq!(fresh.attachment.as_ref().unwrap().epoch, old.attachment.as_ref().unwrap().epoch);
    assert!(fresh.attachment.as_ref().unwrap().process.is_none());
    let before = fixture.service.store.lock().unwrap().read().unwrap();
    fixture.service.mutate(&actor, OrchestrationMutationRequest {
        session_id: old.session_id, expected_revision: None, action,
    }).unwrap();
    assert_eq!(serde_json::to_value(fixture.service.store.lock().unwrap().read().unwrap()).unwrap(),
        serde_json::to_value(before).unwrap(), "registration replay does not rotate launch credentials or create physical proof");
}


#[test]
fn effect_lease_rereads_the_historical_start_marker_from_current_state() {
    let fixture = ReviewFixture::new(RunStage::Working);
    let mut queued = fixture.run.clone();
    queued.dispatch.as_mut().unwrap().agent_started = false;
    let (lease, fresh) = leased_run(&fixture.service, &queued).unwrap().unwrap();
    assert!(
        fresh.dispatch.unwrap().agent_started,
        "the initial-uncertain queue hint cannot bypass the read-only review path"
    );
    drop(lease);
}

fn now() -> String {
    time::OffsetDateTime::now_utc()
        .format(&time::format_description::well_known::Rfc3339)
        .expect("UTC time is representable")
}

#[tokio::test]
async fn generic_launch_preserves_literal_argv_and_durable_harness_policy() {
    use std::sync::atomic::Ordering;
    let prerequisite = uuid::Uuid::new_v4().to_string();
    let mut fixture = ReviewFixture::new(RunStage::Starting);
    fixture.prepared_launch(&prerequisite, true);
    let config = cockpit_protocol::projects::OrchestrationConfiguration {
        harness_kind: "plain-agent".into(),
        command: "/opt/plain agent".into(),
        extra_args: vec!["--fresh".into(), "literal ; $(no-shell)".into()],
        resume_args: Some(vec!["--resume".into(), "{session_id}".into()]),
        process_scope: cockpit_protocol::projects::ScopeMode::Off,
        inbox_prompt_hints: false,
        routes: Vec::new(),
        ..Default::default()
    };
    fixture.change(|run| run.launch_config = config.clone());
    fixture.run = fixture.current().1;
    let mut adapter = CountingHerdr::new(runtime(), false, false);
    adapter.managed_launch = true;
    adapter.expected_harness = "plain-agent".into();
    adapter.expected_command = Some(vec![
        "/opt/plain agent".into(), "--fresh".into(), "literal ; $(no-shell)".into(),
    ]);
    let adapter = Arc::new(adapter);
    fixture.dispatcher(Arc::clone(&adapter)).step(&fixture.run).await.unwrap();
    assert_eq!(adapter.create_calls.load(Ordering::SeqCst), 1);
    assert_eq!(adapter.start_calls.load(Ordering::SeqCst), 1);
    assert_eq!(serde_json::to_value(fixture.current().1.launch_config).unwrap(), serde_json::to_value(config).unwrap());
    assert!(fixture.current().1.attachment.as_ref().unwrap().conversation.is_none());
}

#[test]
fn bootstrap_is_usable_without_conversation_metadata_or_registration_skill_turn() {
    for kind in [RunKind::Supervisor, RunKind::Adopted, RunKind::Worker] {
        let run = crate::orchestration::new_run(
            "bootstrap-run", "session", kind, "Bootstrap".into(),
            "bootstrap-run", crate::orchestration::RunLineage::default(), 1,
        );
        let prompt = render_run_bootstrap(&run);
        assert!(prompt.contains("run context --json"));
        assert!(prompt.contains("inbox next --wait 0 --deliverable --json"));
        assert!(prompt.contains("do not start a competing long inbox"));
        assert!(!prompt.contains("skills show"));
        assert!(!prompt.contains("PI_SESSION_FILE"));
        assert!(!prompt.contains("--session-id"));
        if kind == RunKind::Worker {
            assert!(prompt.contains("run done"));
            assert!(prompt.contains("task show --self"));
            assert!(prompt.contains("task inputs --self --json"));
        } else {
            assert!(prompt.contains("task schedule"));
            assert!(prompt.contains("worker-replace"));
        }
    }
}

#[test]
fn disabled_hook_bootstrap_uses_one_event_driven_wait_and_revalidates_replacement_progress() {
    let mut run = crate::orchestration::new_run("run", "session", RunKind::Worker, "Task".into(), "root", crate::orchestration::RunLineage::default(), 2);
    run.launch_config.hook_args = Some(Vec::new());
    let prompt = render_run_bootstrap(&run);
    assert!(prompt.contains("one event-driven"));
    assert!(prompt.contains("inbox next --wait 600 --deliverable --json"));
    assert!(!prompt.contains("not start a competing"));
    assert!(prompt.contains("advisory"));
    assert!(prompt.contains("revalidate"));
    assert!(prompt.contains("whole-task"));
}

#[test]
fn initial_prompt_is_one_literal_argument_and_custom_placeholder_is_whole() {
    let mut run = crate::orchestration::new_run(
        "bootstrap-run", "session", RunKind::Worker, "Bootstrap".into(),
        "bootstrap-run", crate::orchestration::RunLineage::default(), 1,
    );
    let prompt = "quotes \" ; $(no-shell)\nwork brief";
    assert_eq!(prompt_args(&run, prompt.into()).unwrap(), [prompt]);
    run.launch_config.initial_prompt_args = Some(vec!["--prompt".into(), "{prompt}".into()]);
    assert_eq!(prompt_args(&run, prompt.into()).unwrap(), ["--prompt", prompt]);
    for invalid in [vec!["prefix-{prompt}".into()], vec!["{prompt}".into(), "{prompt}".into()], vec![]] {
        run.launch_config.initial_prompt_args = Some(invalid);
        assert_eq!(prompt_args(&run, prompt.into()).unwrap_err().code, "invalid_launch_settings");
    }
    assert_eq!(expand_argument(&["--hook".into(), "{hook_path}".into()], "{hook_path}", "/path with spaces/hook.ts").unwrap(),
        ["--hook", "/path with spaces/hook.ts"]);
}

#[test]
fn only_pending_effects_have_wake_deadlines_and_unknown_launch_is_not_replayed() {
    let mut run = crate::orchestration::new_run(
        "deadline-run", "session", RunKind::Worker, "Deadline".into(),
        "root", crate::orchestration::RunLineage::default(), 1,
    );
    assert!(pending_operation_delay(&run, 5000).is_none());
    for step in [DispatchStep::Launched, DispatchStep::NeedsReview, DispatchStep::SetupUnknown] {
        run.dispatch = Some(crate::orchestration::dispatch(step));
        assert!(pending_operation_delay(&run, 5000).is_none());
        assert!(!dispatch_actionable(&run));
    }
    for step in [DispatchStep::SetupRunning, DispatchStep::LaunchPending] {
        run.dispatch = Some(crate::orchestration::dispatch(step));
        assert!(pending_operation_delay(&run, 5000).unwrap() <= Duration::from_millis(250));
    }
    run.dispatch = Some(crate::orchestration::dispatch(DispatchStep::LaunchUnknown));
    assert!(pending_operation_delay(&run, 5000).is_some());
    run.dispatch.as_mut().unwrap().updated_at = "2000-01-01T00:00:00Z".into();
    assert!(pending_operation_delay(&run, 5000).is_none(), "expired unknown effects must not create an idle retry loop");
}

#[tokio::test]
async fn filesystem_receiver_consumes_changes_without_idle_reading() {
    let (sender, receiver) = tokio::sync::watch::channel(0);
    let mut receiver = Some(receiver);
    sender.send_replace(1);
    filesystem_changed(&mut receiver).await;
    assert_eq!(*receiver.as_ref().unwrap().borrow(), 1);
    assert!(!receiver.as_ref().unwrap().has_changed().unwrap());
    drop(sender);
    filesystem_changed(&mut receiver).await;
    assert!(receiver.is_none(), "closed watchers must be reinstalled, not spin");
}

fn inbox_hint_fixture(enabled: bool) -> (ReviewFixture, RuntimeView) {
    let mut fixture = ReviewFixture::new(RunStage::Active);
    fixture.change(|run| {
        run.launch_config.inbox_prompt_hints = enabled;
        test_attachment(run).state = cockpit_protocol::orchestration::AttachmentState::Connected;
        test_attachment(run).launch_mode = Some(cockpit_protocol::orchestration::AttachmentLaunchMode::DirectProcess);
    });
    fixture.run = fixture.current().1;
    let auth = fixture.staged_auth(&fixture.run);
    {
        let locked = fixture.service.store.lock().unwrap();
        let mut state = locked.read().unwrap();
        let credential = state.credentials.iter_mut()
            .find(|credential| credential.credential_id == auth.credential_id).unwrap();
        credential.conversation = fixture.run.attachment.as_ref().unwrap().conversation.clone();
        locked.save(&mut state).unwrap();
    }
    append_hint_instruction(&fixture, "hint-instruction");
    let mut observed = runtime();
    observed.panes[0].agent_kind = Some("omp".into());
    observed.panes[0].agent_name = Some("tag".into());
    observed.panes[0].native_session_id = Some("native".into());
    (fixture, observed)
}

#[tokio::test]
async fn initial_bootstrap_never_types_or_pins_unregistered_conversation_metadata() {
    let (mut fixture, mut observed) = inbox_hint_fixture(true);
    fixture.change(|run| {
        run.stage = RunStage::Starting;
        test_attachment(run).conversation = None;
        test_attachment(run).location.as_mut().unwrap().native_session_id = None;
    });
    fixture.run = fixture.current().1;
    observed.panes[0].native_session_file = Some("/advertised/session.jsonl".into());
    let adapter = Arc::new(CountingHerdr::new(observed, false, false));
    fixture.dispatcher(Arc::clone(&adapter)).hint_agents().await.unwrap();
    assert!(adapter.prompt_requests.lock().is_empty());
}

fn append_hint_instruction(fixture: &ReviewFixture, message_id: &str) {
    use super::super::messages::{append, AppendMessage};
    use cockpit_protocol::orchestration::{ActorRef, MessageKind};
    let locked = fixture.service.store.lock().unwrap();
    let mut state = locked.read().unwrap();
    append(&mut state, AppendMessage {
        from: ActorRef::Operator, to_run_id: &fixture.run.run_id, message_id,
        kind: MessageKind::Instruction, text: "private pending instruction",
        in_reply_to: None, report: None, stale: false, escalated_from: None,
    }).unwrap();
    locked.save(&mut state).unwrap();
}

#[tokio::test]
async fn ordinary_inbox_prompt_hints_are_off_by_default_for_registered_chat() {
    use std::sync::atomic::Ordering;
    assert!(!cockpit_protocol::projects::OrchestrationConfiguration::default().inbox_prompt_hints);
    let (fixture, observed) = inbox_hint_fixture(false);
    let adapter = Arc::new(CountingHerdr::new(observed, false, false));
    let dispatcher = fixture.dispatcher(Arc::clone(&adapter));
    let before = serde_json::to_value(fixture.service.store.lock().unwrap().read().unwrap()).unwrap();
    dispatcher.hint_agents().await.unwrap();
    dispatcher.hint_agents().await.unwrap();
    assert_eq!(adapter.prompt_calls.load(Ordering::SeqCst), 0);
    assert_eq!(serde_json::to_value(fixture.service.store.lock().unwrap().read().unwrap()).unwrap(), before);
}

#[tokio::test]
async fn busy_departed_or_contradictory_hint_targets_defer_without_submission() {
    use std::sync::atomic::Ordering;
    use super::super::store::HintState;
    for mismatch in ["busy", "blocked", "departed", "harness", "endpoint", "terminal", "native_session", "missing_native_session", "launch_tag"] {
        let (fixture, mut observed) = inbox_hint_fixture(true);
        match mismatch {
            "busy" => observed.panes[0].agent_status = Some("busy".into()),
            "blocked" => observed.panes[0].agent_status = Some("blocked".into()),
            "departed" => observed.panes.clear(),
            "harness" => observed.panes[0].agent_kind = Some("foreign-harness".into()),
            "endpoint" => observed.endpoint_identity = "replacement".into(),
            "native_session" => observed.panes[0].native_session_id = Some("replacement-conversation".into()),
            "missing_native_session" => observed.panes[0].native_session_id = None,
            "launch_tag" => observed.panes[0].tab_label = "replacement-launch".into(),
            _ => observed.panes[0].terminal_id = Some("replacement".into()),
        }
        let adapter = Arc::new(CountingHerdr::new(observed, false, false));
        fixture.dispatcher(Arc::clone(&adapter)).hint_agents().await.unwrap();
        assert_eq!(adapter.prompt_calls.load(Ordering::SeqCst), 0, "{mismatch}");
        let state = fixture.service.store.lock().unwrap().read().unwrap();
        let hint = state.inbox_hints.iter().find(|hint| hint.run_id == fixture.run.run_id).unwrap();
        assert!(matches!(hint.state, HintState::Deferred), "{mismatch}");
        assert!(hint.cause.is_some(), "{mismatch}");
        assert_eq!(hint.through_seq, 1);
        assert!(!serde_json::to_string(hint).unwrap().contains("private pending instruction"));
        assert_eq!(state.messages[0].stage, cockpit_protocol::orchestration::DeliveryStage::Stored);
    }
}

#[tokio::test]
async fn uncertain_prompt_is_durable_and_not_repeated_after_restart_or_new_message() {
    use std::sync::atomic::Ordering;
    use super::super::store::HintState;
    let (mut fixture, observed) = inbox_hint_fixture(true);
    let mut adapter = CountingHerdr::new(observed.clone(), false, false);
    adapter.prompt_unknown = true;
    let adapter = Arc::new(adapter);
    let dispatcher = fixture.dispatcher(Arc::clone(&adapter));
    dispatcher.hint_agents().await.unwrap();
    assert_eq!(adapter.prompt_calls.load(Ordering::SeqCst), 1);
    let before = fixture.service.store.lock().unwrap().read().unwrap();
    let hint = before.inbox_hints.iter().find(|hint| hint.run_id == fixture.run.run_id).unwrap();
    assert!(matches!(hint.state, HintState::Unknown));
    let attempt_id = hint.attempt_id.clone();
    assert_eq!(before.messages[0].stage, cockpit_protocol::orchestration::DeliveryStage::Stored);
    dispatcher.hint_agents().await.unwrap();
    append_hint_instruction(&fixture, "newer-hint-instruction");
    fixture.service = Arc::new(OrchestrationService::open(&fixture.config).unwrap());
    let restarted_adapter = Arc::new(CountingHerdr::new(observed, false, false));
    fixture.dispatcher(Arc::clone(&restarted_adapter)).hint_agents().await.unwrap();
    assert_eq!(adapter.prompt_calls.load(Ordering::SeqCst), 1);
    assert_eq!(restarted_adapter.prompt_calls.load(Ordering::SeqCst), 0);
    let after = fixture.service.store.lock().unwrap().read().unwrap();
    let hint = after.inbox_hints.iter().find(|hint| hint.run_id == fixture.run.run_id).unwrap();
    assert!(matches!(hint.state, HintState::Unknown));
    assert_eq!(hint.attempt_id, attempt_id);
    assert!(after.messages.iter().all(|message| message.stage == cockpit_protocol::orchestration::DeliveryStage::Stored));
    let credential_path = super::super::credentials::credential_path(
        fixture.service.base(), &after.credentials[0],
    ).unwrap();
    let actor = super::super::Actor::Agent(Box::new(super::super::AgentCaller {
        auth: super::super::credentials::read_job_auth(&credential_path).unwrap(),
        conversation: fixture.run.attachment.as_ref().unwrap().conversation.clone(),
        assignment_generation: None,
        endpoint_identity: String::new(), session_id: "deliberately-foreign-physical-session".into(), workspace_id: String::new(),
        tab_id: String::new(), pane_id: String::new(), boot_id: None, terminal_id: None,
        native_session_id: None, actual_agent_kind: None, process: None,
    }));
    let wrong_route = fixture.service.mutate(&actor, cockpit_protocol::orchestration::OrchestrationMutationRequest {
        session_id: "different-stored-route".into(), expected_revision: None,
        action: cockpit_protocol::orchestration::OrchestrationAction::InboxPull { after_seq: 0, limit: 100 },
    }).unwrap_err();
    assert_eq!(wrong_route.code, "session_mismatch");
    assert_eq!(serde_json::to_value(fixture.service.store.lock().unwrap().read().unwrap()).unwrap(),
        serde_json::to_value(&after).unwrap(), "wrong logical routing must not capture inbox or settle an uncertain hint");
    fixture.service.mutate(&actor, cockpit_protocol::orchestration::OrchestrationMutationRequest {
        session_id: fixture.run.session_id.clone(), expected_revision: None,
        action: cockpit_protocol::orchestration::OrchestrationAction::InboxPull { after_seq: 0, limit: 100 },
    }).unwrap();
    let captured = fixture.service.store.lock().unwrap().read().unwrap();
    assert!(matches!(captured.inbox_hints[0].state, HintState::Sent));
    assert_eq!(captured.inbox_hints[0].cause.as_deref(), Some("inbox_captured"));
    assert!(captured.messages.iter().all(|message| message.stage == cockpit_protocol::orchestration::DeliveryStage::Read));
    assert!(captured.messages.iter().all(|message| message.acked_at.is_none()));
}

#[tokio::test]
async fn successful_prompt_submission_is_not_inbox_capture_or_ack() {
    use std::sync::atomic::Ordering;
    use super::super::store::HintState;
    let (fixture, observed) = inbox_hint_fixture(true);
    let adapter = Arc::new(CountingHerdr::new(observed, false, false));
    fixture.dispatcher(Arc::clone(&adapter)).hint_agents().await.unwrap();
    assert_eq!(adapter.prompt_calls.load(Ordering::SeqCst), 1);
    let state = fixture.service.store.lock().unwrap().read().unwrap();
    assert!(matches!(state.inbox_hints[0].state, HintState::Sent));
    assert_eq!(state.messages[0].stage, cockpit_protocol::orchestration::DeliveryStage::Stored);
    assert!(state.messages[0].acked_at.is_none());
}

#[test]
fn same_chat_resume_substitutes_only_the_whole_opaque_session_argument() {
    use cockpit_protocol::orchestration::{ConversationIdentity, ResumePhase, ResumeRecord};
    let fixture = ReviewFixture::new(RunStage::Starting);
    let mut run = fixture.run.clone();
    run.launch_config.harness_kind = "omp".into();
    run.launch_config.extra_args = vec!["--fresh-only".into()];
    run.launch_config.resume_args = Some(vec![
        "--custom-resume".into(), "{session_id}".into(), "literal-{session_id}".into(),
    ]);
    assert_eq!(launch_args(&run).unwrap(), ["--fresh-only"]);
    run.attachment.as_mut().unwrap().conversation = Some(ConversationIdentity {
        harness: "omp".into(), session_id: "opaque/chat ; $(literal)".into(),
    });
    run.resume = Some(ResumeRecord {
        operation_id: uuid::Uuid::new_v4().to_string(), run_id: run.run_id.clone(),
        previous_attachment_epoch: Some(1), replacement_attachment_epoch: Some(2),
        phase: ResumePhase::Launching, cause: None, decision_id: None, updated_at: now(),
    });
    let reminder = render_run_bootstrap(&run);
    assert!(reminder.contains("resumed this conversation"));
    assert!(!reminder.contains("work brief"));
    assert_eq!(reminder.lines().count(), 1);
    assert_eq!(launch_args(&run).unwrap(), [
        "--custom-resume", "opaque/chat ; $(literal)", "literal-{session_id}",
    ]);
    run.launch_config.resume_args = None;
    assert!(resume_supported(&run.launch_config));
    assert_eq!(launch_args(&run).unwrap(), ["--resume", "opaque/chat ; $(literal)"]);
    run.launch_config.harness_kind = "plain-agent".into();
    assert!(!resume_supported(&run.launch_config));
    assert_eq!(launch_args(&run).unwrap_err().code, "harness_resume_unsupported");
    run.launch_config.resume_args = Some(vec!["--foreign-resume".into(), "{session_id}".into()]);
    assert!(resume_supported(&run.launch_config));
    assert_eq!(launch_args(&run).unwrap(), ["--foreign-resume", "opaque/chat ; $(literal)"]);
    run.launch_config.harness_kind = "omp".into();
    for invalid in [
        vec!["--resume".into()],
        vec!["prefix-{session_id}".into()],
        vec!["{session_id}".into(), "{session_id}".into()],
    ] {
        run.launch_config.resume_args = Some(invalid);
        assert_eq!(launch_args(&run).unwrap_err().code, "invalid_launch_settings");
    }
    run.launch_config.resume_args = Some(vec!["{session_id}".into()]);
    run.attachment.as_mut().unwrap().conversation = None;
    assert_eq!(launch_args(&run).unwrap_err().code, "resume_session_unavailable");
}

#[test]
fn generic_agent_observation_requires_configured_harness() {
    let mut observed = runtime();
    let location = receipt();
    observed.panes[0].agent_kind = Some("omp".into());
    assert!(matches!(reconcile_tag(&observed, Some("endpoint"), "tag", Some(&location),
        None, true, "plain-agent"), ReconciledLaunch::Tab));
    observed.panes[0].agent_kind = Some("plain-agent".into());
    assert!(matches!(reconcile_tag(&observed, Some("endpoint"), "tag", Some(&location),
        None, true, "plain-agent"), ReconciledLaunch::Agent));
}

#[tokio::test]
async fn interrupted_durable_hint_submission_becomes_unknown_without_resubmitting() {
    use std::sync::atomic::Ordering;
    use super::super::store::HintState;
    let (fixture, observed) = inbox_hint_fixture(true);
    assert_eq!(fixture.service.hint_queue().unwrap().len(), 1);
    {
        let locked = fixture.service.store.lock().unwrap();
        let mut state = locked.read().unwrap();
        state.inbox_hints[0].state = HintState::Submitting;
        locked.save(&mut state).unwrap();
    }
    let adapter = Arc::new(CountingHerdr::new(observed, false, false));
    fixture.dispatcher(Arc::clone(&adapter)).hint_agents().await.unwrap();
    assert_eq!(adapter.prompt_calls.load(Ordering::SeqCst), 0);
    let state = fixture.service.store.lock().unwrap().read().unwrap();
    assert!(matches!(state.inbox_hints[0].state, HintState::Unknown));
    assert_eq!(state.inbox_hints[0].cause.as_deref(), Some("submission_interrupted"));
    assert_eq!(state.messages[0].stage, cockpit_protocol::orchestration::DeliveryStage::Stored);
}

#[tokio::test]
async fn pane_events_coalesce_and_disconnect_reconnects_without_snapshot_polling() {
    use std::sync::atomic::Ordering;
    let adapter = Arc::new(CountingHerdr::new(runtime(), false, false));
    let (source, messages) = tokio::sync::mpsc::channel(8);
    *adapter.event_messages.lock() = Some(messages);
    let (sender, mut wakes) = tokio::sync::mpsc::channel(1);
    let stop = CancellationToken::new();
    let herdr: Arc<dyn OrchestrationHerdr> = adapter.clone();
    let task = tokio::spawn(watch_session(herdr, "test-session".into(), sender, stop.clone()));
    tokio::time::timeout(Duration::from_secs(2), wakes.recv()).await.unwrap().unwrap();
    for _ in 0..3 { source.send(crate::SessionChange::Changed).await.unwrap(); }
    tokio::task::yield_now().await;
    assert_eq!(wakes.len(), 1, "event bursts coalesce into one reconciliation");
    wakes.recv().await.unwrap();
    source.send(crate::SessionChange::Disconnected { code: "gap".into(), message: "fixture".into() }).await.unwrap();
    tokio::time::timeout(Duration::from_secs(2), wakes.recv()).await.unwrap().unwrap();
    tokio::time::timeout(Duration::from_secs(2), async {
        while adapter.event_subscriptions.load(Ordering::SeqCst) < 2 {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    }).await.unwrap();
    assert_eq!(adapter.runtime_calls.load(Ordering::SeqCst), 0, "event adapter must not drive idle runtime polling");
    stop.cancel();
    task.await.unwrap();
}

#[tokio::test(start_paused = true)]
async fn foreground_retirement_deadline_advances_without_global_idle_polling() {
    use cockpit_protocol::orchestration::{RetainReason, RetirementState, RetirementTrigger, RunRetirement};
    let fixture = ReviewFixture::new(RunStage::Closed);
    let mut run = fixture.run.clone();
    run.retirement = Some(RunRetirement {
        retirement_id: "pending-retirement".into(), trigger: RetirementTrigger::Accept,
        result_message_id: None, task_revision: None, replacement: None, identity: None,
        state: RetirementState::Retained {
            at: "old observation".into(), reason: RetainReason::ForegroundProcess, stopped: false,
        },
        created_at: "0".into(), updated_at: "0".into(),
    });
    let at = tokio::time::Instant::now();
    let mut retries = BTreeMap::from([(run.run_id.clone(), at)]);
    let due = due_retirement_retries(&mut retries, vec![run.clone()], at);
    assert_eq!(due.len(), 1);
    assert_eq!(retries[&run.run_id], at + RETIREMENT_RETRY_DELAY);
    assert!(due_retirement_retries(&mut retries, vec![run.clone()], at).is_empty(),
        "unchanged observations or lease contention must not leave a past deadline");
    tokio::time::advance(RETIREMENT_RETRY_DELAY - Duration::from_millis(1)).await;
    assert!(due_retirement_retries(&mut retries, vec![run.clone()], tokio::time::Instant::now()).is_empty());
    tokio::time::advance(Duration::from_millis(1)).await;
    assert_eq!(due_retirement_retries(&mut retries, vec![run.clone()], tokio::time::Instant::now()).len(), 1);
    assert_eq!(retries[&run.run_id], at + RETIREMENT_RETRY_DELAY * 2);
    run.retirement.as_mut().unwrap().state = RetirementState::Retained {
        at: "permanent guard".into(), reason: RetainReason::IdentityChanged, stopped: false,
    };
    assert!(due_retirement_retries(&mut retries, vec![run.clone()], at + RETIREMENT_RETRY_DELAY * 2).is_empty());
    assert!(retries.is_empty(), "permanent guards have no retry deadline");
    retries.insert(run.run_id.clone(), at);
    assert!(due_retirement_retries(&mut retries, Vec::new(), at).is_empty());
    assert!(retries.is_empty(), "retired or absent candidates have no retry deadline");
}

#[test]
fn new_scope_creation_receipt_remains_event_actionable_after_semantic_readiness() {
    let fixture = ReviewFixture::new(RunStage::Active);
    let mut run = fixture.run.clone();
    let dispatch = run.dispatch.as_mut().unwrap();
    dispatch.step = DispatchStep::Launched;
    dispatch.scope_unit = None;
    assert!(!scope_creation_pending(&run), "old uncontained attempts are never retrofitted");
    run.dispatch.as_mut().unwrap().scope_unit = Some("cockpit-r-fixture.scope".into());
    assert!(scope_creation_pending(&run));
    assert!(dispatch_actionable(&run), "semantic readiness must not discard the physical creation receipt");
    assert!(pending_operation_delay(&run, 60_000).is_none(),
        "the physical creation receipt is completed on events, not an idle retry timer");
    run.dispatch.as_mut().unwrap().updated_at = "2000-01-01T00:00:00Z".into();
    assert!(scope_creation_pending(&run));
    assert!(dispatch_actionable(&run));
    assert!(pending_operation_delay(&run, 60_000).is_none());
}

#[tokio::test]
async fn launch_root_capture_is_exact_even_when_the_harness_has_spawned_children() {
    let fixture = ReviewFixture::new(RunStage::Starting);
    let shell = fixture.run.attachment.as_ref().unwrap().shell.clone().unwrap();
    let mut adapter = CountingHerdr::new(runtime(), false, false);
    adapter.process_info = Some(super::super::herdr::PaneProcessInfo {
        pane_id: "pane".into(), shell_pid: Some(shell.process.pid),
        foreground_pgid: Some(shell.process.pid),
        processes: vec![(shell.process.pid, "root".into()), (shell.process.pid + 1, "child".into())],
        shell_identity: Some(shell.clone()),
    });
    let observed = read_launch_shell(&adapter, &receipt()).await;
    assert_eq!(observed, Some(shell));
    let mut foreign = receipt();
    foreign.pane_id = "foreign-pane".into();
    assert!(read_launch_shell(&adapter, &foreign).await.is_none());
}

#[test]
fn moved_terminal_requires_one_fresh_exclusive_destination_and_never_uses_focus() {
    let source = receipt();
    let mut observed = runtime();
    observed.boot_id = source.boot_id.clone();
    observed.workspaces.push(super::super::herdr::RuntimeWorkspace {
        workspace_id: "destination".into(), label: "Owned".into(), cwd: "/owned".into(),
    });
    observed.panes[0].workspace_id = "destination".into();
    observed.panes[0].workspace_label = "Owned".into();
    observed.panes[0].tab_id = "actual-tab".into();
    observed.panes[0].pane_id = "actual-pane".into();
    let actual = moved_location(&observed, &source).unwrap();
    assert_eq!(actual.workspace_id, "destination");
    assert_eq!(actual.tab_id, "actual-tab");
    assert_eq!(actual.pane_id, "actual-pane");
    assert_eq!(observed.focused_workspace_id.as_deref(), Some("space"));
    for mismatch in ["endpoint", "boot", "tag", "terminal", "duplicate", "foreign-pane", "source"] {
        let mut wrong = observed.clone();
        match mismatch {
            "endpoint" => wrong.endpoint_identity = "foreign".into(),
            "boot" => wrong.boot_id = None,
            "tag" => wrong.panes[0].tab_label = "foreign".into(),
            "terminal" => wrong.panes[0].terminal_id = Some("foreign".into()),
            "duplicate" => wrong.panes.push(wrong.panes[0].clone()),
            "foreign-pane" => {
                let mut foreign = wrong.panes[0].clone();
                foreign.terminal_id = Some("foreign".into());
                wrong.panes.push(foreign);
            }
            "source" => wrong.panes[0].workspace_id = source.workspace_id.clone(),
            _ => unreachable!(),
        }
        assert!(moved_location(&wrong, &source).is_none(), "{mismatch}");
    }
}

#[tokio::test]
async fn dependency_input_requires_actual_materialization_and_keeps_a_frozen_base_after_head_changes() {
    let root = std::env::temp_dir().join(format!("cockpit-scheduled-input-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir_all(&root).unwrap();
    for args in [
        vec!["init", "-q"],
        vec!["-c", "user.name=Fixture", "-c", "user.email=fixture@example.invalid",
            "commit", "-q", "--allow-empty", "-m", "base"],
    ] {
        assert!(tokio::process::Command::new("git").current_dir(&root).args(args).status().await.unwrap().success());
    }
    let path = root.to_str().unwrap();
    let before = immutable_schedule_base(path, "HEAD").await.unwrap();
    std::fs::write(root.join("prerequisite-output.txt"), "named dependency output\n").unwrap();
    assert!(tokio::process::Command::new("git").current_dir(&root).args(["add", "prerequisite-output.txt"])
        .status().await.unwrap().success());
    assert!(tokio::process::Command::new("git").current_dir(&root)
        .args(["-c", "user.name=Fixture", "-c", "user.email=fixture@example.invalid",
            "commit", "-q", "-m", "accepted prerequisite"]).status().await.unwrap().success());
    let accepted = immutable_schedule_base(path, "HEAD").await.unwrap();
    assert!(!schedule_commit_included(path, &accepted, &before).await.unwrap(), "checked/accepted does not publish the commit into an older base");
    assert!(schedule_commit_included(path, &accepted, &accepted).await.unwrap());
    assert!(tokio::process::Command::new("git").current_dir(&root).args(["reset", "--hard", &before])
        .status().await.unwrap().success());
    assert_eq!(immutable_schedule_base(path, "HEAD").await.unwrap(), before);
    assert!(schedule_commit_included(path, &accepted, &accepted).await.unwrap(),
        "the verified SHA remains the creation input, never the later moving HEAD");
    assert!(schedule_commit_included(path, "--fake-input", &accepted).await.is_err());
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn retained_space_restart_requires_exact_resumed_epoch_and_journalled_retained_location() {
    use cockpit_protocol::orchestration::{ResumePhase, ResumeRecord};
    let fixture = ReviewFixture::new(RunStage::Starting);
    let mut run = fixture.run.clone();
    run.kind = RunKind::Worker;
    let (_, checkout) = super::super::project_identity::resolve_owned_directory(&fixture.root).unwrap();
    run.checkout = Some(checkout.clone());
    run.setup = Some(SetupSummary {
        operation_id: None, generation: None, workspace_id: Some("retained-space".into()),
        checkout_path: checkout.path, repository_id: None, branch: None, base: None,
        base_commit: None, ownership: None, effects: Vec::new(), warnings: Vec::new(), project_workspace_id: Some("source-space".into()),
    });
    run.target = Some(DispatchTarget::SpaceWorktree { workspace_id: "source-space".into(), branch: None, base_ref: None });
    run.attachment.as_mut().unwrap().epoch = 2;
    run.resume = Some(ResumeRecord {
        operation_id: uuid::Uuid::new_v4().to_string(), run_id: run.run_id.clone(),
        previous_attachment_epoch: Some(1), replacement_attachment_epoch: Some(2),
        phase: ResumePhase::Launching, cause: None, decision_id: None, updated_at: now(),
    });
    run.dispatch = Some(crate::orchestration::dispatch(DispatchStep::SetupPending));
    assert!(resumed_checkout_epoch(&run));
    assert!(!restarts_in_retained_space(&run), "an epoch pin is not itself a physical launch receipt");
    let dispatch = run.dispatch.as_mut().unwrap();
    dispatch.launch_tag = Some("exact-new-epoch".into());
    dispatch.staging_workspace_id = Some("retained-space".into());
    assert!(restarts_in_retained_space(&run), "the existing bound Space receives the new scoped tab without a second move/bind");
    assert!(matches!(run.target.as_ref(), Some(DispatchTarget::SpaceWorktree { workspace_id, .. }) if workspace_id == "source-space"));
    let mut root = run.clone();
    root.kind = RunKind::Supervisor;
    assert!(resumed_checkout_epoch(&root), "supervisors retain the same checkout across a resumed epoch");
    assert!(restarts_in_retained_space(&root));
    for mismatch in ["epoch", "previous", "checkout", "source", "move", "unbound"] {
        let mut changed = run.clone();
        match mismatch {
            "epoch" => changed.attachment.as_mut().unwrap().epoch = 3,
            "previous" => changed.resume.as_mut().unwrap().previous_attachment_epoch = Some(0),
            "checkout" => changed.setup.as_mut().unwrap().checkout_path.push_str("-foreign"),
            "source" => changed.dispatch.as_mut().unwrap().staging_workspace_id = Some("source-space".into()),
            "move" => changed.dispatch.as_mut().unwrap().move_operation_id = Some(uuid::Uuid::new_v4().to_string()),
            "unbound" => changed.setup.as_mut().unwrap().workspace_id = None,
            _ => unreachable!(),
        }
        assert!(!restarts_in_retained_space(&changed), "{mismatch}");
    }
}

#[tokio::test]
async fn resumed_supervisor_uses_retained_space_or_fresh_focused_staging() {
    use cockpit_protocol::orchestration::{ResumePhase, ResumeRecord};
    use crate::orchestration::herdr::RuntimeWorkspace;

    for retained_open in [false, true] {
        let fixture = ReviewFixture::new(RunStage::Starting);
        let (_, checkout) = super::super::project_identity::resolve_owned_directory(&fixture.root).unwrap();
        fixture.change(|run| {
            run.kind = RunKind::Supervisor;
            run.checkout = Some(checkout.clone());
            run.setup = Some(SetupSummary {
                operation_id: None, generation: None, workspace_id: Some("retained-space".into()),
                checkout_path: checkout.path.clone(), repository_id: None, branch: None, base: None,
                base_commit: None, ownership: None, effects: Vec::new(), warnings: Vec::new(),
                project_workspace_id: Some("stale-source".into()),
            });
            run.target = Some(DispatchTarget::ExistingSpace { workspace_id: "retained-space".into() });
            run.attachment.as_mut().unwrap().epoch = 2;
            run.resume = Some(ResumeRecord {
                operation_id: uuid::Uuid::new_v4().to_string(), run_id: run.run_id.clone(),
                previous_attachment_epoch: Some(1), replacement_attachment_epoch: Some(2),
                phase: ResumePhase::Launching, cause: None, decision_id: None, updated_at: now(),
            });
            run.dispatch = Some(crate::orchestration::dispatch(DispatchStep::SetupPending));
        });
        let run = fixture.current().1;
        assert!(resumed_checkout_epoch(&run));
        let mut observed = runtime();
        if retained_open {
            observed.workspaces.push(RuntimeWorkspace {
                workspace_id: "retained-space".into(), label: "Retained".into(),
                cwd: checkout.path.clone(),
            });
        }
        let choice = resume_staging_choice(&run, &observed).unwrap();
        assert_eq!(choice.as_deref(), retained_open.then_some("retained-space"));
        let adapter = Arc::new(CountingHerdr::new(observed, false, false));
        let dispatcher = fixture.dispatcher(adapter);
        let staging = match choice {
            Some(retained) => retained,
            None => dispatcher.staging_space(&run).await.unwrap(),
        };
        assert_eq!(staging, if retained_open { "retained-space" } else { "space" });
        let mut staged = fixture.current().1;
        if !retained_open {
            assert_eq!(staged.dispatch.as_ref().unwrap().staging_workspace_id.as_deref(), Some(staging.as_str()));
        }
        // LaunchIntent journals the selected staging Space and launch tag.
        let dispatch = staged.dispatch.as_mut().unwrap();
        dispatch.staging_workspace_id = Some(staging);
        dispatch.launch_tag = Some("new-epoch".into());
        assert_eq!(restarts_in_retained_space(&staged), retained_open,
            "only a launch in the retained Space skips moving and rebinding");
    }
}

#[test]
fn resumed_space_choice_rejects_changed_or_ambiguous_retained_checkout() {
    let fixture = ReviewFixture::new(RunStage::Starting);
    let (_, checkout) = super::super::project_identity::resolve_owned_directory(&fixture.root).unwrap();
    let mut run = fixture.run.clone();
    run.setup = Some(SetupSummary {
        operation_id: None, generation: None, workspace_id: Some("space".into()),
        checkout_path: checkout.path.clone(), repository_id: None, branch: None, base: None,
        base_commit: None, ownership: None, effects: Vec::new(), warnings: Vec::new(),
        project_workspace_id: None,
    });
    let mut observed = runtime();
    assert_eq!(resume_staging_choice(&run, &observed).unwrap_err().code, "retained_space_checkout_changed");
    observed.workspaces[0].cwd = checkout.path;
    assert_eq!(resume_staging_choice(&run, &observed).unwrap().as_deref(), Some("space"));
    observed.workspaces.push(observed.workspaces[0].clone());
    assert_eq!(resume_staging_choice(&run, &observed).unwrap_err().code, "retained_space_checkout_changed");
}

#[tokio::test]
async fn owned_branch_generation_preserves_templates_and_full_attempt_identity() {
    let mut fixture = ReviewFixture::new(RunStage::Starting);
    let checkout = fixture.root.join("cockpit");
    std::fs::create_dir_all(&checkout).unwrap();
    fixture.config.repository_roots = vec![checkout.to_string_lossy().into_owned()];
    fixture.config.checkout_template = "{task_id}".into();
    for args in [
        vec!["init", "-q"],
        vec!["-c", "user.name=Fixture", "-c", "user.email=fixture@example.invalid",
            "commit", "-q", "--allow-empty", "-m", "source"],
    ] {
        assert!(tokio::process::Command::new("git").current_dir(&checkout)
            .args(args).status().await.unwrap().success());
    }
    let repository = crate::repositories::RepositoryCatalog::new(fixture.config.clone())
        .list().await.unwrap().repositories.into_iter().next().unwrap();
    assert_eq!(repository.name, "cockpit");
    let adapter = Arc::new(ReadOnlyProjectAdapter { inventory: Some(ProjectInventory {
        endpoint_identity: "endpoint".into(),
        repository_key: repository.common_dir.clone(),
        repository_root: repository.root.clone(),
        supported_methods: vec![],
        worktrees: vec![],
    }) });
    let request = WorkspaceSetupRequest::Create {
        repository_id: repository.repository_id.clone(), branch: None, base_ref: None,
        checkout_path: None, label: None, task_name: None, artifact_url: None,
        linked_artifact_urls: vec![], focus: false,
    };
    let mut worker = fixture.run.clone();
    worker.kind = RunKind::Worker;
    worker.task_id = Some(uuid::Uuid::new_v4().to_string());
    let mut other_task = worker.clone();
    other_task.task_id = Some(uuid::Uuid::new_v4().to_string());
    let mut replacement = worker.clone();
    replacement.run_id = uuid::Uuid::new_v4().to_string();
    let mut next_attempt = worker.clone();
    next_attempt.attempt += 1;
    let owners = [
        (fixture.run.clone(), fixture.run.root_id.clone()),
        (worker.clone(), worker.task_id.clone().unwrap()),
        (other_task.clone(), other_task.task_id.clone().unwrap()),
        (replacement, worker.task_id.clone().unwrap()),
        (next_attempt, worker.task_id.clone().unwrap()),
    ];
    for (template, derived_branch) in [
        ("cockpit/{repo}/{task_id}", "cockpit/cockpit/task"),
        ("custom/{repo}/task-{task_id}", "custom/cockpit/task-task"),
    ] {
        fixture.config.branch_template = template.into();
        let service = ProjectService::new(fixture.config.clone(), adapter.clone()).unwrap();
        let ordinary = service.plan(&fixture.run.session_id, &request).await.unwrap();
        assert_eq!(ordinary.branch.as_deref(), Some(derived_branch));
        let mut branches = std::collections::HashSet::new();
        for (run, owner) in &owners {
            let plan = service.plan_orchestration(
                &run.session_id, &request, &owned_branch_suffix(run),
            ).await.unwrap();
            let expected = format!("{derived_branch}-{owner}-run-{}-a{}", run.run_id, run.attempt);
            assert_eq!(plan.branch.as_deref(), Some(expected.as_str()));
            assert_eq!(plan.label, expected);
            assert!(branches.insert(plan.branch.unwrap()),
                "Task, run and attempt changes must each retain distinct ownership");
        }
        let mut explicit = request.clone();
        if let WorkspaceSetupRequest::Create { branch, .. } = &mut explicit {
            *branch = Some("task-requested-exact-branch".into());
        }
        let plan = service.plan_orchestration(
            &fixture.run.session_id, &explicit, &owned_branch_suffix(&fixture.run),
        ).await.unwrap();
        assert_eq!(plan.branch.as_deref(), Some("task-requested-exact-branch"));
    }
}

async fn path_schedule_fixture(mode: &str) -> (ReviewFixture, Arc<Dispatcher>) {
    use cockpit_protocol::orchestration::{OrchestrationActionResult, TaskOwnership, TaskSchedule};
    use super::super::store::{DurableOperationKind, DurableOperationRecord};
    let mut fixture = ReviewFixture::new(RunStage::Active);
    let repositories = fixture.root.join("repositories");
    let checkout = repositories.join("primary");
    std::fs::create_dir_all(&checkout).unwrap();
    // This review fixture intentionally permits only one catalog entry.
    // Configure the actual checkout, not its parent discovery directory.
    fixture.config.repository_roots = vec![checkout.to_string_lossy().into_owned()];
    fixture.config.branch_template = "test/{task_id}".into();
    fixture.config.checkout_template = "{task_id}".into();
    if mode != "directory" {
        for args in [
            vec!["init", "-q"],
            vec!["-c", "user.name=Fixture", "-c", "user.email=fixture@example.invalid",
                "commit", "-q", "--allow-empty", "-m", "source"],
        ] {
            assert!(tokio::process::Command::new("git").current_dir(&checkout)
                .args(args).status().await.unwrap().success());
        }
    }
    let (project, _) = super::super::project_identity::resolve_directory(&checkout, None, None).await.unwrap();
    let mut observed = runtime();
    observed.workspaces[0].workspace_id = "source-space".into();
    observed.workspaces[0].cwd = checkout.to_string_lossy().into_owned();
    if mode == "ambiguous" {
        let mut duplicate = observed.workspaces[0].clone();
        duplicate.workspace_id = "second-space".into();
        observed.workspaces.push(duplicate);
    }
    let inventory = ProjectInventory {
        endpoint_identity: "endpoint".into(),
        repository_key: match &project.source {
            cockpit_protocol::orchestration::ProjectSource::Git { common_dir } if mode != "foreign_inventory" => common_dir.clone(),
            _ => "not-the-source".into(),
        },
        repository_root: checkout.to_string_lossy().into_owned(),
        supported_methods: vec![],
        worktrees: vec![crate::project_adapter::ProjectWorktreeEntry {
            checkout_path: checkout.to_string_lossy().into_owned(), branch: None,
            open_workspace_id: Some("source-space".into()), is_primary: true, is_linked_worktree: false, dirty: Some(false),
        }],
    };
    let herdr = Arc::new(CountingHerdr::new(observed, false, false));
    let mut dispatcher = fixture.dispatcher(herdr);
    Arc::get_mut(&mut dispatcher).unwrap().projects = Arc::new(ProjectService::new(
        fixture.config.clone(), Arc::new(ReadOnlyProjectAdapter { inventory: Some(inventory) }),
    ).unwrap());
    let mut document = String::new();
    let locked = fixture.service.store.lock().unwrap();
    let mut state = locked.read().unwrap();
    if mode == "default" || mode == "explicit_worktree" {
        let mut source_project = project.clone();
        source_project.source_workspace_id = Some("source-space".into());
        state.runs.iter_mut().find(|run| run.run_id == fixture.run.run_id).unwrap().project = Some(source_project);
    }
    state.runs.iter_mut().find(|run| run.run_id == fixture.run.run_id).unwrap().launch_config.max_parallel = 4;
    state.credentials.push(super::super::credentials::test_job_credential(&fixture.run, super::super::JobRole::Root).0);
    for label in ["First independent task", "Second independent task"] {
        let task_id = uuid::Uuid::new_v4().to_string();
        document.push_str(&format!("- [ ] {label} <!-- cockpit-task: {task_id} -->\n"));
        let operation_id = uuid::Uuid::new_v4().to_string();
        let schedule = TaskSchedule { operation_id: operation_id.clone(), principal_key: "root-source-fixture".into(),
            target: if mode == "default" { None } else { Some(if mode == "explicit_space" {
                DispatchTarget::ExistingSpace { workspace_id: "source-space".into() }
            } else if mode == "explicit_worktree" {
                DispatchTarget::SpaceWorktree { workspace_id: "source-space".into(), branch: None, base_ref: None }
            } else {
                DispatchTarget::Setup { request: WorkspaceSetupRequest::Open {
                    path: checkout.to_string_lossy().into_owned(), label: None, task_name: None, focus: false,
                } }
            }) }, brief: "Execute independently".into(), cleanup_policy: cockpit_protocol::orchestration::WorkerCleanupPolicy::OwnedWhenSafe,
            label: None, scheduled_at: now() };
        state.operations.push(DurableOperationRecord { operation_id: operation_id.clone(),
            actor_run_id: Some(fixture.run.run_id.clone()), principal_key: schedule.principal_key.clone(),
            root_id: fixture.run.run_id.clone(), kind: DurableOperationKind::TaskSchedule,
            request_digest: "persisted authorized schedule fixture".into(), run_id: None, resume_operation_id: None,
            result: Some(OrchestrationActionResult::TaskScheduled { operation_id, task_id: task_id.clone(), duplicate: false }),
            effect_receipts: vec![], created_at: now(), updated_at: now() });
        state.task_ownership.push(TaskOwnership { root_id: fixture.run.run_id.clone(), task_id,
            generation: 0, owner: None, schedule: Some(schedule) });
    }
    fixture.service.store.tasks_dir().write(
        super::super::tasks_md::root_filename(&fixture.run.run_id).unwrap(), document.as_bytes(),
    ).unwrap();
    locked.save(&mut state).unwrap();
    drop(locked);
    (fixture, dispatcher)
}

#[tokio::test]
async fn capacity_stall_is_delayed_and_deduplicated() {
    let (fixture, dispatcher) = path_schedule_fixture("git").await;
    let candidates = fixture.service.schedule_candidates(4).unwrap();
    let first = dispatcher.resolve_schedule_target(&candidates[0]).await.unwrap();
    let second = dispatcher.resolve_schedule_target(&candidates[1]).await.unwrap();
    let project = second.project.as_ref().unwrap();
    let projects = BTreeMap::from([(candidates[1].schedule.operation_id.clone(),
        (project.key.clone(), project.repository_id.clone().unwrap_or_else(|| "Project".into())))]);
    fixture.service.scheduled_start(&candidates[0], &first).unwrap().unwrap();
    let mut waits = BTreeMap::new();
    let deadline = fixture.service.capacity_stalls(1, &projects, &mut waits).unwrap().unwrap();
    assert!(deadline > tokio::time::Instant::now());
    assert!(!fixture.service.store.lock().unwrap().read().unwrap().messages.iter().any(|m| m.message_id.starts_with("capacity-stall:")));
    for value in waits.values_mut() { *value = tokio::time::Instant::now(); }
    fixture.service.capacity_stalls(1, &projects, &mut waits).unwrap();
    fixture.service.capacity_stalls(1, &projects, &mut waits).unwrap();
    let state = fixture.service.store.lock().unwrap().read().unwrap();
    assert_eq!(state.messages.iter().filter(|m| m.message_id.starts_with("capacity-stall:")).count(), 1);
}

#[tokio::test]
async fn existing_space_root_plan_preserves_explicit_source_through_launch_reservation() {
    let (fixture, dispatcher) = path_schedule_fixture("default").await;
    fixture.change(|run| {
        run.stage = RunStage::Starting;
        run.target = Some(DispatchTarget::ExistingSpace { workspace_id: "source-space".into() });
        run.attachment = None;
        run.setup = None;
        run.dispatch = Some(crate::orchestration::dispatch(DispatchStep::SetupPending));
    });
    let root = fixture.current().1;
    dispatcher.plan(&root).await.unwrap();
    let planned = fixture.current().1;
    let setup = planned.setup.as_ref().unwrap();
    assert_eq!(setup.project_workspace_id.as_deref(), Some("source-space"));
    let (project, checkout) = super::super::project_identity::resolve_directory(
        std::path::Path::new(&setup.checkout_path), setup.repository_id.clone(), setup.project_workspace_id.clone(),
    ).await.unwrap();
    let (reserved, _) = fixture.service.reserve_dispatch_launch(&planned, project, checkout, None, None).unwrap();
    assert_eq!(reserved.project.unwrap().source_workspace_id.as_deref(), Some("source-space"));
}

#[tokio::test]
async fn lone_source_is_shared_and_stale_second_resolution_retries_isolated() {
    for mode in ["git", "default"] {
        let (fixture, dispatcher) = path_schedule_fixture(mode).await;
        let candidates = fixture.service.schedule_candidates(4).unwrap();
        assert_eq!(candidates.len(), 2);
        let first = dispatcher.resolve_schedule_target(&candidates[0]).await.unwrap();
        let stale = dispatcher.resolve_schedule_target(&candidates[1]).await.unwrap();
        assert!(first.shared_by_default && stale.shared_by_default);
        assert!(first.checkout.is_some());
        assert!(matches!(&first.normalized_target, Some(DispatchTarget::ExistingSpace { workspace_id }) if workspace_id == "source-space"));
        let start = fixture.service.scheduled_start(&candidates[0], &first).unwrap().unwrap();
        let run = fixture.service.run_for_review(&candidates[0].session_id, &start.run_id).unwrap();
        assert!(matches!(&run.target, Some(DispatchTarget::ExistingSpace { workspace_id }) if workspace_id == "source-space"));
        let before = fixture.service.store.lock().unwrap().read().unwrap().runs.len();
        assert_eq!(fixture.service.scheduled_start(&candidates[1], &stale).unwrap_err().code, "schedule_placement_changed");
        assert_eq!(fixture.service.store.lock().unwrap().read().unwrap().runs.len(), before);
        let isolated = dispatcher.resolve_schedule_target(&candidates[1]).await.unwrap();
        assert!(!isolated.shared_by_default);
        let second = fixture.service.scheduled_start(&candidates[1], &isolated).unwrap().unwrap();
        let run = fixture.service.run_for_review(&candidates[1].session_id, &second.run_id).unwrap();
        assert!(matches!(&run.target, Some(DispatchTarget::SpaceWorktree { workspace_id, .. }) if workspace_id == "source-space"));
        dispatcher.plan(&run).await.unwrap();
        let planned = fixture.service.run_for_review(&candidates[1].session_id, &second.run_id).unwrap();
        assert_eq!(planned.setup.unwrap().ownership, Some(WorkspaceCheckoutOwnership::OwnedWorktree));
    }
}

#[tokio::test]
async fn dirty_and_explicit_placements_do_not_implicitly_share() {
    for mode in ["git", "default", "explicit_worktree", "explicit_space"] {
        let (fixture, dispatcher) = path_schedule_fixture(mode).await;
        if mode == "git" || mode == "default" {
            std::fs::write(fixture.root.join("repositories/primary/untracked"), "user work").unwrap();
        }
        let candidate = fixture.service.schedule_candidates(4).unwrap().pop().unwrap();
        let resolved = dispatcher.resolve_schedule_target(&candidate).await.unwrap();
        assert!(!resolved.shared_by_default);
        let start = fixture.service.scheduled_start(&candidate, &resolved).unwrap().unwrap();
        let run = fixture.service.run_for_review(&candidate.session_id, &start.run_id).unwrap();
        if mode == "explicit_space" {
            assert!(matches!(run.target, Some(DispatchTarget::ExistingSpace { .. })));
        } else {
            assert!(matches!(run.target, Some(DispatchTarget::SpaceWorktree { .. })));
        }
    }
}

#[tokio::test]
async fn protected_claim_forces_isolation_and_claim_race_rejects_publication() {
    use super::super::claims::{CheckoutClaim, ClaimState};
    for claim_state in [ClaimState::Reserved, ClaimState::Held, ClaimState::Quiescing, ClaimState::Conflict, ClaimState::Released] {
        let (fixture, dispatcher) = path_schedule_fixture("default").await;
        let candidate = fixture.service.schedule_candidates(4).unwrap().pop().unwrap();
        let initially_shared = dispatcher.resolve_schedule_target(&candidate).await.unwrap();
        let checkout = initially_shared.checkout.clone().unwrap();
        let locked = fixture.service.store.lock().unwrap();
        let mut state = locked.read().unwrap();
        state.checkout_claims.push(CheckoutClaim { claim_id: uuid::Uuid::new_v4().to_string(), checkout,
            revision: 1, state: claim_state, holder: None, transition_operation_id: None });
        locked.save(&mut state).unwrap();
        drop(locked);
        if claim_state != ClaimState::Released {
            let before = fixture.service.store.lock().unwrap().read().unwrap().runs.len();
            assert_eq!(fixture.service.scheduled_start(&candidate, &initially_shared).unwrap_err().code, "schedule_placement_changed");
            assert_eq!(fixture.service.store.lock().unwrap().read().unwrap().runs.len(), before);
        }
        let current = dispatcher.resolve_schedule_target(&candidate).await.unwrap();
        assert_eq!(current.shared_by_default, claim_state == ClaimState::Released);
        if claim_state != ClaimState::Released {
            let start = fixture.service.scheduled_start(&candidate, &current).unwrap().unwrap();
            let run = fixture.service.run_for_review(&candidate.session_id, &start.run_id).unwrap();
            assert!(matches!(run.target, Some(DispatchTarget::SpaceWorktree { .. })));
        }
    }
}

#[tokio::test]
async fn changed_checkout_identity_rejects_stale_shared_resolution() {
    let (fixture, dispatcher) = path_schedule_fixture("default").await;
    let candidate = fixture.service.schedule_candidates(4).unwrap().pop().unwrap();
    let mut resolved = dispatcher.resolve_schedule_target(&candidate).await.unwrap();
    resolved.checkout.as_mut().unwrap().directory_identity.inode = "stale".into();
    let before = fixture.service.store.lock().unwrap().read().unwrap().runs.len();
    assert_eq!(fixture.service.scheduled_start(&candidate, &resolved).unwrap_err().code, "schedule_placement_changed");
    assert_eq!(fixture.service.store.lock().unwrap().read().unwrap().runs.len(), before);
}


#[tokio::test]
async fn dirty_after_resolution_is_rejected_under_publication_lock() {
    let (fixture, dispatcher) = path_schedule_fixture("default").await;
    let candidate = fixture.service.schedule_candidates(4).unwrap().pop().unwrap();
    let resolved = dispatcher.resolve_schedule_target(&candidate).await.unwrap();
    assert!(resolved.shared_by_default);
    std::fs::write(fixture.root.join("repositories/primary/user-file"), "work").unwrap();
    let before = fixture.service.store.lock().unwrap().read().unwrap().runs.len();
    assert_eq!(fixture.service.scheduled_start(&candidate, &resolved).unwrap_err().code, "schedule_placement_changed");
    assert_eq!(fixture.service.store.lock().unwrap().read().unwrap().runs.len(), before);
    let retry = dispatcher.resolve_schedule_target(&candidate).await.unwrap();
    assert!(!retry.shared_by_default);
}

#[tokio::test]
async fn implicit_shared_launch_gate_rechecks_cleanliness_without_dependencies_and_after_schedule_removal() {
    for mode in ["default", "explicit_space"] {
        let (fixture, dispatcher) = path_schedule_fixture(mode).await;
        let candidate = fixture.service.schedule_candidates(4).unwrap().pop().unwrap();
        let resolved = dispatcher.resolve_schedule_target(&candidate).await.unwrap();
        let checkout = resolved.checkout.clone().unwrap();
        let project = resolved.project.clone().unwrap();
        let start = fixture.service.scheduled_start(&candidate, &resolved).unwrap().unwrap();
        let run = fixture.service.run_for_review(&candidate.session_id, &start.run_id).unwrap();
        let locked = fixture.service.store.lock().unwrap();
        let mut state = locked.read().unwrap();
        state.task_ownership.iter_mut().find(|owner| owner.task_id == candidate.task_id).unwrap().schedule = None;
        locked.save(&mut state).unwrap();
        drop(locked);
        assert!(dispatcher.actual_launch_input_facts(&run, &project, &checkout).await.is_ok());
        std::fs::write(fixture.root.join("repositories/primary/user-file"), "work").unwrap();
        let result = dispatcher.actual_launch_input_facts(&run, &project, &checkout).await;
        if mode == "default" {
            assert_eq!(result.unwrap_err().code, "schedule_shared_checkout_dirty");
        } else {
            assert!(result.is_ok(), "explicit borrowing retains its existing semantics");
        }
    }
}
#[tokio::test]
async fn path_schedule_preserves_directory_ambiguous_and_unverified_git_space_boundaries() {
    for mode in ["directory", "ambiguous", "foreign_inventory", "explicit_space"] {
        let (fixture, dispatcher) = path_schedule_fixture(mode).await;
        let candidate = fixture.service.schedule_candidates(4).unwrap().pop().unwrap();
        let resolved = dispatcher.resolve_schedule_target(&candidate).await.unwrap();
        assert!(resolved.normalized_target.is_none(), "{mode}");
        assert!(resolved.checkout.is_some(), "{mode}: explicit borrowing or unverified Open must not become owned");
        if matches!(mode, "directory" | "ambiguous") {
            let start = fixture.service.scheduled_start(&candidate, &resolved).unwrap().unwrap();
            let run = fixture.service.run_for_review(&candidate.session_id, &start.run_id).unwrap();
            let error = dispatcher.plan(&run).await.unwrap_err();
            assert_eq!(error.code, "space_exists_for_path", "{mode}");
            fixture.service.record_dispatch(&start.run_id, DispatchUpdate::PlanFailed { error: as_response(error) }).unwrap();
            let failed = fixture.service.run_for_review(&candidate.session_id, &start.run_id).unwrap();
            assert!(plan_failed_before_effects(&failed));
            assert!(failed.setup.is_none());
        }
    }
}
