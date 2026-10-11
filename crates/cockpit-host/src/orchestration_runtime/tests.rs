use super::*;
use cockpit_core::orchestration::herdr::{
    AgentTabRequest, DirectAgentTabRequest, RuntimePane, RuntimeView,
};
use cockpit_core::{
    HerdrAdapter, ProjectHerdrAdapter, SessionSubscription, TerminalSession,
    project_adapter::*,
};
use cockpit_protocol::v1 as wire;

#[derive(Clone)]
struct ObservedHerdr(Option<RuntimeView>);
#[async_trait::async_trait]
impl OrchestrationHerdr for ObservedHerdr {
    async fn runtime(&self, _: &str) -> Result<RuntimeView, InspectionError> {
        self.0
            .clone()
            .ok_or_else(|| InspectionError::new("offline", "No fresh endpoint"))
    }
    async fn create_agent_tab(
        &self,
        _: &str,
        _: &AgentTabRequest,
    ) -> Result<RunLocation, InspectionError> {
        panic!("a retry preflight must not create a terminal")
    }
    async fn create_direct_agent_tab(
        &self,
        _: &str,
        _: &DirectAgentTabRequest,
    ) -> Result<RunLocation, InspectionError> {
        panic!("a mutation resolution must not launch a direct process")
    }
    async fn pane_process_info(
        &self,
        _: &str,
        _: &str,
        _: &str,
    ) -> Result<cockpit_core::orchestration::herdr::PaneProcessInfo, InspectionError> {
        Err(InspectionError::new("fixture_process_unavailable", "The restored pane has no trusted process evidence"))
    }
    async fn close_pane(&self, _: &str, _: &str, _: &str) -> Result<(), InspectionError> {
        panic!("a retry preflight must not close a terminal")
    }
}

struct InspectionProjectAdapter(Option<ProjectInventory>);
#[async_trait::async_trait]
impl HerdrAdapter for InspectionProjectAdapter {
    async fn inspect(&self) -> Result<wire::HerdrCompatibility, InspectionError> { panic!("not inspected") }
    async fn inspect_session(&self, _: &str) -> Result<wire::HerdrCompatibility, InspectionError> { panic!("not inspected") }
    async fn sessions(&self) -> Result<wire::SessionListResponse, InspectionError> { panic!("not inspected") }
    async fn session_snapshot(&self, _: &str) -> Result<wire::SessionSnapshotResponse, InspectionError> { panic!("not inspected") }
    async fn focus(&self, _: &str, _: &wire::FocusRequest) -> Result<wire::FocusResponse, InspectionError> { panic!("resolution must not focus") }
    async fn mutate(&self, _: &str, _: &wire::ResourceMutationRequest) -> Result<wire::ResourceMutationResponse, InspectionError> { panic!("resolution must not mutate Herdr") }
    async fn subscribe_session(&self, _: &str, _: &wire::SessionSnapshotResponse) -> Result<SessionSubscription, InspectionError> { panic!("not subscribed") }
    async fn open_terminal(&self, _: &wire::TerminalOpenRequest) -> Result<TerminalSession, InspectionError> { panic!("resolution must not open a terminal") }
}
#[async_trait::async_trait]
impl ProjectHerdrAdapter for InspectionProjectAdapter {
    async fn project_endpoint_identity(&self, _: &str) -> Result<String, InspectionError> { Ok("endpoint".into()) }
    async fn project_inventory(&self, _: &str, _: &str) -> Result<ProjectInventory, InspectionError> {
        Ok(self.0.clone().expect("configured repository inventory"))
    }
    async fn project_worktree(&self, _: &str, _: &ProjectWorktreeRequest) -> Result<ProjectWorktreeResult, InspectionError> { panic!("resolution must not create a checkout") }
    async fn project_terminal(&self, _: &str, _: &ProjectTerminalRequest) -> Result<ProjectTerminalResult, InspectionError> { panic!("resolution must not create a terminal") }
    async fn project_worktree_dirty(&self, _: &str, _: u32, _: u32) -> Result<bool, InspectionError> { panic!("not inspected") }
    async fn project_close_workspace(&self, _: &str, _: &str, _: &str) -> Result<(), InspectionError> { panic!("resolution must not close a workspace") }
    async fn project_remove_worktree(&self, _: &str, _: &ProjectWorktreeRemoveRequest) -> Result<(), InspectionError> { panic!("resolution must not delete a checkout") }
}

fn reviewed() -> Run {
    serde_json::from_value(serde_json::json!({
        "session_id":"fixture","brief":"","run_id":"run","kind":"supervisor",
        "label":"Supervisor","root_id":"run","attempt":1,"stage":"active",
        "task_revision_at_start":null,"cleanup_policy":null,
        "dispatch":{"launch_tag":"tag","endpoint_identity":"endpoint","agent_started":false,
            "step":"launch_unknown","launch_attempt":0,"updated_at":"2026-10-06T00:00:00Z"},
        "annotations":[],
        "attachment":{"attachment_id":"attachment","epoch":1,"state":"disconnected",
            "location":{"endpoint_identity":"endpoint","session_id":"fixture","workspace_id":"space",
                "tab_id":"tab","pane_id":"pane","launch_tag":"tag","terminal_id":"terminal"},
            "main_session_id":null,"process":null,"shell":null,"machine_id":null,
            "launch_mode":"shell_agent",
            "launch_attempt":0,"updated_at":"2026-10-06T00:00:00Z"},
        "created_at":"2026-10-06T00:00:00Z","updated_at":"2026-10-06T00:00:00Z"
    })).unwrap()
}

fn observed() -> RuntimeView {
    RuntimeView {
        endpoint_identity: "endpoint".into(),
        boot_id: None,
        workspaces: vec![],
        panes: vec![RuntimePane {
            workspace_id: "space".into(),
            workspace_label: "Space".into(),
            tab_id: "tab".into(),
            tab_label: "tag".into(),
            pane_id: "pane".into(),
            terminal_id: Some("terminal".into()),
            native_session_id: None,
            agent_name: Some("tag".into()),
            agent_kind: Some("omp".into()),
            launch_pending: false,
            interactive_ready: false,
            agent_status: Some("working".into()),
            state_changed_at: None,
        }],
    }
}

fn empty_observation() -> ObservedHerdr {
    let mut runtime = observed();
    runtime.panes.clear();
    ObservedHerdr(Some(runtime))
}

struct Fixture {
    root: PathBuf,
    service: OrchestrationService,
}
impl Fixture {
    fn new() -> Self {
        let root = std::env::temp_dir().join(format!("cockpit-resume-{}", uuid::Uuid::new_v4()));
        fs::create_dir_all(&root).unwrap();
        let service = OrchestrationService::open(&ProjectConfiguration::for_tests(&root)).unwrap();
        Self { root, service }
    }
    fn projects(&self, inventory: Option<ProjectInventory>) -> ProjectService {
        ProjectService::new(ProjectConfiguration::for_tests(&self.root),
            Arc::new(InspectionProjectAdapter(inventory))).unwrap()
    }
    fn publish(&self, runs: &[Run]) {
        let state = serde_json::json!({
            "schema":2,"revision":7,"runs":runs,"messages":[],"subagents":[],
            "task_intents":[],"scope_intents":[],"assignment_intents":[],"credentials":[],
            "checkout_claims":[],"operations":[]
        });
        let mut file = OpenOptions::new().create(true).truncate(true).write(true)
            .mode(0o600).open(self.service.base().join("state.json")).unwrap();
        file.write_all(&serde_json::to_vec(&state).unwrap()).unwrap();
    }
    fn runtime(&self) -> ObservedHerdr {
        let mut runtime = observed();
        runtime.workspaces.push(RuntimeWorkspace {
            workspace_id: "space".into(), label: "Project".into(),
            cwd: self.root.to_string_lossy().into_owned(),
        });
        ObservedHerdr(Some(runtime))
    }
}
impl Drop for Fixture {
    fn drop(&mut self) { fs::remove_dir_all(&self.root).unwrap(); }
}


#[cfg(target_os = "linux")]
fn bind_process(run: &mut Run, pid: u32) {
    let process = NativeProcessIdentity {
        pid,
        start_ticks: cockpit_core::process_identity::start_identity(pid as i32).unwrap(),
        kernel_boot_id: cockpit_core::process_identity::kernel_boot_id(),
    };
    let attachment = run.attachment.as_mut().unwrap();
    attachment.main_session_id = Some("native".into());
    attachment.machine_id = verified_machine_id(&process);
    attachment.process = Some(process);
    run.dispatch.as_mut().unwrap().agent_started = true;
}

#[cfg(target_os = "linux")]
#[tokio::test]
async fn resume_reattaches_live_process_before_inspecting_missing_checkout_or_endpoint() {
    let fixture = Fixture::new();
    let mut run = reviewed();
    bind_process(&mut run, std::process::id());
    run.kind = RunKind::Worker;
    run.parent_run_id = Some("root".into());
    run.task_id = Some("original-task".into());
    let (_, mut checkout) = resolve_directory(&fixture.root, None, None).await.unwrap();
    checkout.path = fixture.root.join("absent-checkout").to_string_lossy().into_owned();
    run.checkout = Some(checkout.clone());
    run.target = Some(DispatchTarget::ExistingSpace { workspace_id: "retargeted-space".into() });
    let resolved = resolve_resume_input(&fixture.service, &ObservedHerdr(None), &run).await.unwrap();
    assert!(resolved.writer_exit_proof.is_none());
    assert_eq!(resolved.checkout.as_ref(), Some(&checkout));
    let observed = resolved.attachment_observation.unwrap();
    assert_eq!(observed.attachment_id, "attachment");
    assert_eq!(observed.epoch, 1);
    assert_eq!(observed.process, run.attachment.as_ref().unwrap().process);
    assert_eq!(run.task_id.as_deref(), Some("original-task"));
}


#[cfg(target_os = "linux")]
#[tokio::test]
async fn resume_reboot_proof_is_same_machine_only_and_unknown_roots_stay_protected() {
    let fixture = Fixture::new();
    let mut run = reviewed();
    run.kind = RunKind::Adopted;
    bind_process(&mut run, std::process::id());
    let attachment = run.attachment.as_mut().unwrap();
    let boot = attachment.process.as_mut().unwrap().kernel_boot_id.as_mut().unwrap();
    let replacement = if boot.starts_with('0') { "1" } else { "0" };
    boot.replace_range(0..1, replacement);
    fixture.publish(&[run.clone()]);
    let resolved = resolve_resume_input(&fixture.service, &empty_observation(), &run).await.unwrap();
    assert!(resolved.writer_exit_proof.is_some());
    assert!(resolved.attachment_observation.is_none());
    let mut drifted = empty_observation();
    drifted.0.as_mut().unwrap().endpoint_identity = "new-endpoint".into();
    assert!(resolve_resume_input(&fixture.service, &drifted, &run).await.unwrap().writer_exit_proof.is_some());
    let offline = resolve_resume_input(&fixture.service, &ObservedHerdr(None), &run).await.unwrap();
    assert!(offline.writer_exit_proof.is_none());
    assert_eq!(offline.observation_cause.unwrap().code, "resume_observation_unavailable");
    for occupant in 0..3 {
        let mut restored = observed();
        match occupant {
            0 => restored.panes[0].agent_kind = Some("foreign-agent".into()),
            1 => {
                let mut shared = restored.panes[0].clone();
                shared.pane_id = "shared-pane".into();
                shared.terminal_id = Some("shared-terminal".into());
                restored.panes.push(shared);
            }
            _ => restored.panes[0].agent_kind = None,
        }
        let refused = resolve_resume_input(&fixture.service, &ObservedHerdr(Some(restored)), &run).await.unwrap();
        assert!(refused.writer_exit_proof.is_none());
        assert!(refused.attachment_observation.is_none());
        let cause = refused.observation_cause.unwrap();
        assert_eq!(cause.code, "consumer_activity_unproven", "occupant {occupant}");
        assert_eq!(cause.class, ResumeCauseClass::Recover);
    }
    for change in 0..3 {
        let mut changed = run.clone();
        let attachment = changed.attachment.as_mut().unwrap();
        match change {
            0 => attachment.machine_id = None,
            1 => attachment.machine_id = Some("foreign-machine".into()),
            _ => attachment.process = None,
        }
        fixture.publish(&[changed.clone()]);
        let resolved = resolve_resume_input(&fixture.service, &ObservedHerdr(None), &changed).await.unwrap();
        assert!(resolved.writer_exit_proof.is_none());
        assert!(resolved.attachment_observation.is_none());
    }
}

#[cfg(target_os = "linux")]
#[tokio::test]
async fn same_boot_abrupt_exit_preserves_detached_consumer_refusal() {
    let fixture = Fixture::new();
    let mut run = reviewed();
    run.kind = RunKind::Adopted;
    bind_process(&mut run, std::process::id());
    run.attachment.as_mut().unwrap().process.as_mut().unwrap().start_ticks += 1;
    fixture.publish(&[run.clone()]);
    let resolved = resolve_resume_input(&fixture.service, &ObservedHerdr(None), &run).await.unwrap();
    assert!(resolved.writer_exit_proof.is_none());
    assert!(resolved.attachment_observation.is_none());
    let cause = resolved.observation_cause.unwrap();
    assert_eq!(cause.code, "consumer_activity_unproven");
    assert_eq!(cause.class, ResumeCauseClass::Recover);
}

#[tokio::test]
async fn lookup_unavailable_is_explicit_and_never_creates_state() {
    let fixture = Fixture::new();
    let request = ProjectRootLookupRequest {
        session_id: "fixture".into(), workspace_id: "space".into(), selected_root_id: None,
    };
    for runtime in [ObservedHerdr(None), empty_observation()] {
        let response = lookup_project_roots(&fixture.service, &runtime, &request).await.unwrap();
        assert!(matches!(response, ProjectRootLookupResponse::Unavailable { .. }));
    }
    assert!(!fixture.service.base().join("state.json").exists());
}

#[cfg(target_os = "linux")]
#[tokio::test]
async fn lookup_matches_saved_project_roots_and_uses_selected_then_latest() {
    let fixture = Fixture::new();
    let (project, _) = resolve_directory(&fixture.root, None, Some("space".into())).await.unwrap();
    let mut selected = reviewed();
    selected.run_id = "a".into();
    selected.root_id = "a".into();
    selected.project = Some(project.clone());
    bind_process(&mut selected, std::process::id());
    // Physical liveness, even a saved Connected flag, is not current SDK authentication.
    selected.attachment.as_mut().unwrap().state = AttachmentState::Connected;
    let mut latest = selected.clone();
    latest.run_id = "b".into();
    latest.root_id = "b".into();
    latest.updated_at = "2026-10-07T00:00:00Z".into();
    latest.attachment.as_mut().unwrap().process = None;
    let mut closed = latest.clone();
    closed.run_id = "closed".into();
    closed.root_id = "closed".into();
    closed.stage = RunStage::Closed;
    let mut unavailable = selected.clone();
    unavailable.run_id = "unavailable".into();
    unavailable.root_id = "unavailable".into();
    unavailable.project = None;
    unavailable.attachment.as_mut().unwrap().location.as_mut().unwrap().workspace_id = "other-space".into();
    fixture.publish(&[latest.clone(), selected.clone(), closed, unavailable]);
    assert!(fixture.service.recorded_runs_with_bindings("fixture").unwrap().2.is_empty());
    let before = fs::read(fixture.service.base().join("state.json")).unwrap();
    for (selected_root, preferred) in [
        (Some("a"), "a"), (None, "b"), (Some("closed"), "b"), (Some("unavailable"), "b"),
    ] {
        let request = ProjectRootLookupRequest {
            session_id: "fixture".into(), workspace_id: "space".into(),
            selected_root_id: selected_root.map(str::to_owned),
        };
        let ProjectRootLookupResponse::Resolved {
            project: actual, lookup_revision, preferred_root_id, roots,
        } = lookup_project_roots(&fixture.service, &fixture.runtime(), &request).await.unwrap()
        else { panic!("fresh local project must resolve"); };
        assert_eq!(actual.key, project.key);
        assert_eq!(lookup_revision, 7);
        assert_eq!(preferred_root_id.as_deref(), Some(preferred));
        assert_eq!(roots.len(), 2);
        let legacy = roots.iter().find(|r| r.root_id == "a").unwrap();
        assert_eq!(legacy.connection, RootConnection::Unknown);
        assert_eq!(legacy.resume, RootResumeAvailability::Blocked);
        assert_eq!(roots.iter().find(|r| r.root_id == "b").unwrap().connection, RootConnection::Unknown);
    }
    assert_eq!(before, fs::read(fixture.service.base().join("state.json")).unwrap());
}

#[cfg(target_os = "linux")]
#[tokio::test]
async fn same_space_unknown_root_blocks_automatic_start_and_lookup_not_another() {
    let fixture = Fixture::new();
    let projects = fixture.projects(None);
    fixture.publish(&[reviewed()]);
    let request = ProjectRootLookupRequest {
        session_id: "fixture".into(), workspace_id: "space".into(), selected_root_id: None,
    };
    let ProjectRootLookupResponse::Unavailable { code, .. } =
        lookup_project_roots(&fixture.service, &fixture.runtime(), &request).await.unwrap()
        else { panic!("unknown associated root must not be hidden as no match"); };
    assert_eq!(code, "project_identity_unavailable");
    let actor = Actor::Operator(OperatorOrigin::Browser);
    let request = start_request(Some(DispatchTarget::ExistingSpace { workspace_id: "space".into() }),
        SupervisorStartMode::ReuseOrResume);
    assert_eq!(resolve_mutation_input(&fixture.service, &projects, &fixture.runtime(), &actor, &request)
        .await.unwrap_err().code, "project_identity_unavailable");
    let another = start_request(Some(DispatchTarget::ExistingSpace { workspace_id: "space".into() }),
        SupervisorStartMode::Another);
    assert!(resolve_mutation_input(&fixture.service, &projects, &fixture.runtime(), &actor, &another)
        .await.unwrap().project.is_some());
    assert_eq!(fixture.service.recorded_runs("fixture").unwrap().1.len(), 1);
}

#[cfg(target_os = "linux")]
#[tokio::test]
async fn resume_does_not_reconnect_quiescing_live_attachment() {
    let fixture = Fixture::new();
    let mut run = reviewed();
    bind_process(&mut run, std::process::id());
    run.attachment.as_mut().unwrap().state = AttachmentState::Quiescing;
    fixture.publish(&[run.clone()]);
    let resolved = resolve_resume_input(&fixture.service, &ObservedHerdr(None), &run).await.unwrap();
    assert!(resolved.attachment_observation.is_none());
    assert!(resolved.writer_exit_proof.is_none());
    assert_eq!(run.attachment.unwrap().state, AttachmentState::Quiescing);
    assert!(resolved.observation_cause.is_some());
}

#[test]
fn root_lookup_preserves_guarded_refusal_without_inventing_connection() {
    let mut run = reviewed();
    assert_eq!(root_candidate(&run, false).connection, RootConnection::Unknown);
    run.resume = Some(ResumeRecord {
        operation_id: "resume".into(), run_id: run.run_id.clone(),
        previous_attachment_epoch: Some(1), phase: ResumePhase::Blocked,
        cause: Some(ResumeCause {
            code: "same_boot_replacement_unsupported".into(),
            message: "Current attachment and claim are preserved".into(),
            class: ResumeCauseClass::Decision,
        }),
        decision_id: Some("decision".into()), replacement_attachment_epoch: None,
        updated_at: run.updated_at.clone(),
    });
    let candidate = root_candidate(&run, false);
    assert_eq!(candidate.resume, RootResumeAvailability::Blocked);
    assert_eq!(candidate.connection, RootConnection::Unknown);
    assert_eq!(candidate.cause.unwrap().code, "same_boot_replacement_unsupported");
}
#[test]
fn living_process_without_current_authenticated_attachment_is_not_connected() {
    let mut run = reviewed();
    let pid = std::process::id();
    let Some(start_ticks) = cockpit_core::process_identity::start_identity(pid as i32) else { return };
    let Some(machine) = cockpit_core::process_identity::machine_id() else { return };
    let Some(boot) = cockpit_core::process_identity::kernel_boot_id() else { return };
    let attachment = run.attachment.as_mut().unwrap();
    attachment.process = Some(NativeProcessIdentity { pid, start_ticks, kernel_boot_id: Some(boot) });
    attachment.machine_id = Some(machine);
    attachment.state = AttachmentState::Connected;
    let legacy = root_candidate(&run, false);
    assert_eq!(legacy.connection, RootConnection::Unknown);
    assert_eq!(legacy.resume, RootResumeAvailability::Blocked);
    assert!(legacy.cause.is_some());
}

fn start_request(target: Option<DispatchTarget>, mode: SupervisorStartMode) -> OrchestrationMutationRequest {
    OrchestrationMutationRequest {
        session_id: "fixture".into(), expected_revision: None,
        action: OrchestrationAction::SupervisorStart {
            operation_id: uuid::Uuid::new_v4().to_string(), mode, target,
            label: None, selected_root_id: None, expected_project_key: None,
        },
    }
}

fn worker_start_request(target: DispatchTarget) -> OrchestrationMutationRequest {
    OrchestrationMutationRequest {
        session_id: "fixture".into(), expected_revision: None,
        action: OrchestrationAction::WorkerStart {
            operation_id: uuid::Uuid::new_v4().to_string(),
            root_id: "root".into(), task_id: "task".into(),
            expected_task_revision: "task-revision".into(),
            expected_doc_revision: "doc-revision".into(),
            target, label: None, brief: "Implement the task".into(),
            cleanup_policy: WorkerCleanupPolicy::OwnedWhenSafe,
        },
    }
}

#[cfg(target_os = "linux")]
#[tokio::test]
async fn setup_open_and_existing_space_resolve_same_exact_plain_directory() {
    let fixture = Fixture::new();
    let projects = fixture.projects(None);
    let actor = Actor::Operator(OperatorOrigin::Browser);
    let mut identities = Vec::new();
    for target in [
        DispatchTarget::ExistingSpace { workspace_id: "space".into() },
        DispatchTarget::Setup { request: WorkspaceSetupRequest::Open {
            path: fixture.root.to_string_lossy().into_owned(), label: None,
            task_name: None, focus: true,
        }},
    ] {
        for request in [
            start_request(Some(target.clone()), SupervisorStartMode::Another),
            worker_start_request(target),
        ] {
            let resolved = resolve_mutation_input(&fixture.service, &projects, &fixture.runtime(), &actor, &request)
                .await.unwrap();
            assert_eq!(resolved.checkout.as_ref().unwrap().path, fixture.root.to_string_lossy());
            identities.push(resolved.project.unwrap().key);
        }
    }
    assert!(identities.iter().all(|key| key == &identities[0]));
    let invalid_target = DispatchTarget::Setup {
        request: WorkspaceSetupRequest::Open {
            path: ".".into(), label: None, task_name: None, focus: false,
        },
    };
    for invalid in [
        start_request(Some(invalid_target.clone()), SupervisorStartMode::Another),
        worker_start_request(invalid_target),
    ] {
        assert!(resolve_mutation_input(&fixture.service, &projects, &fixture.runtime(), &actor, &invalid).await.is_err());
    }
}

#[cfg(target_os = "linux")]
#[tokio::test]
async fn configured_create_and_space_worktree_share_project_not_source_checkout() {
    let fixture = Fixture::new();
    let repository_path = fixture.root.join("repository");
    fs::create_dir(&repository_path).unwrap();
    for args in [
        vec!["init", "-q", "-b", "main"],
        vec!["-c", "user.name=Fixture", "-c", "user.email=fixture@example.invalid",
            "commit", "--allow-empty", "-qm", "Fixture"],
    ] {
        assert!(std::process::Command::new("git").args(args).current_dir(&repository_path)
            .status().unwrap().success());
    }
    let (project, checkout) = resolve_directory(&repository_path, None, None).await.unwrap();
    let common_dir = checkout.git.as_ref().unwrap().common_dir.clone();
    let inventory = ProjectInventory {
        endpoint_identity: "endpoint".into(), repository_key: common_dir,
        repository_root: repository_path.to_string_lossy().into_owned(),
        supported_methods: vec![],
        worktrees: vec![ProjectWorktreeEntry {
            checkout_path: repository_path.to_string_lossy().into_owned(), branch: Some("main".into()),
            open_workspace_id: Some("space".into()), is_primary: true,
            is_linked_worktree: false, dirty: Some(false),
        }],
    };
    let projects = fixture.projects(Some(inventory));
    let repositories = projects.repositories().await.unwrap().repositories;
    let repository = repositories.iter().find(|r| r.checkout_path == checkout.path).unwrap();
    let mut runtime = fixture.runtime();
    runtime.0.as_mut().unwrap().workspaces[0].cwd = checkout.path.clone();
    let actor = Actor::Operator(OperatorOrigin::Browser);
    for target in [
        DispatchTarget::Setup { request: WorkspaceSetupRequest::Create {
            repository_id: repository.repository_id.clone(), branch: Some("worker".into()),
            base_ref: Some("main".into()), checkout_path: None, label: None,
            task_name: Some("worker".into()), artifact_url: None, linked_artifact_urls: vec![], focus: true,
        }},
        DispatchTarget::SpaceWorktree { workspace_id: "space".into(), branch: None, base_ref: None },
    ] {
        for request in [
            start_request(Some(target.clone()), SupervisorStartMode::Another),
            worker_start_request(target),
        ] {
            let resolved = resolve_mutation_input(&fixture.service, &projects, &runtime, &actor, &request).await.unwrap();
            assert_eq!(resolved.project.unwrap().key, project.key);
            assert!(resolved.checkout.is_none(), "eventual fresh checkout cannot be the source checkout");
        }
    }
}

#[cfg(target_os = "linux")]
#[tokio::test]
async fn no_target_start_binds_created_dedicated_directory_in_core() {
    let fixture = Fixture::new();
    let projects = fixture.projects(None);
    let actor = Actor::Operator(OperatorOrigin::Browser);
    let request = start_request(None, SupervisorStartMode::Another);
    let resolved = resolve_mutation_input(&fixture.service, &projects, &ObservedHerdr(None), &actor, &request)
        .await.unwrap();
    assert!(resolved.project.is_none());
    let response = fixture.service.mutate_resolved(&actor, request, &resolved).unwrap();
    let OrchestrationActionResult::SupervisorStarted { start } = response.result else { panic!("start result"); };
    let run = fixture.service.run_for_review("fixture", &start.root_id).unwrap();
    let Some(DispatchTarget::Setup { request: WorkspaceSetupRequest::Open { path, .. } }) = run.target
        else { panic!("dedicated directory target"); };
    let ProjectSource::Directory { root } = run.project.unwrap().source else { panic!("directory project"); };
    assert_eq!(root, path);
    assert_ne!(root, fixture.root.to_string_lossy());
    assert_eq!(run.checkout.unwrap().path, path);
}

#[cfg(target_os = "linux")]
#[tokio::test]
async fn start_preserves_selected_legacy_live_root_without_retargeting_or_new_writer() {
    let fixture = Fixture::new();
    let projects = fixture.projects(None);
    let (project, checkout) = resolve_directory(&fixture.root, None, None).await.unwrap();
    let mut run = reviewed();
    run.project = Some(project);
    run.checkout = Some(checkout.clone());
    bind_process(&mut run, std::process::id());
    run.attachment.as_mut().unwrap().state = AttachmentState::Connected;
    // This fixture has no authenticated RunAttach; preserving its living writer
    // must not become permission to launch a replacement or report SDK connection.
    fixture.publish(&[run.clone()]);
    let actor = Actor::Operator(OperatorOrigin::Browser);
    let mut request = start_request(Some(DispatchTarget::ExistingSpace { workspace_id: "space".into() }),
        SupervisorStartMode::ReuseOrResume);
    let OrchestrationAction::SupervisorStart { selected_root_id, .. } = &mut request.action
        else { unreachable!() };
    *selected_root_id = Some(run.run_id.clone());
    let resolved = resolve_mutation_input(&fixture.service, &projects, &fixture.runtime(), &actor, &request)
        .await.unwrap();
    assert_eq!(resolved.checkout.as_ref(), Some(&checkout));
    assert_eq!(resolved.attachment_observation.as_ref().unwrap().state, AttachmentState::Connected);
    let response = fixture.service.mutate_resolved(&actor, request.clone(), &resolved).unwrap();
    let OrchestrationActionResult::SupervisorStarted { start } = response.result else { panic!("start result"); };
    assert_eq!(start.outcome, SupervisorStartOutcome::Resuming);
    assert_eq!(start.root_id, run.run_id);
    let (_, runs, bindings) = fixture.service.recorded_runs_with_bindings("fixture").unwrap();
    assert_eq!(runs.len(), 1);
    assert!(bindings.is_empty());
    let preserved = &runs[0];
    assert_eq!(preserved.project, run.project);
    assert_eq!(preserved.checkout.as_ref(), Some(&checkout));
    assert_eq!(serde_json::to_value(&preserved.target).unwrap(), serde_json::to_value(&run.target).unwrap());
    let attachment = preserved.attachment.as_ref().unwrap();
    let original = run.attachment.as_ref().unwrap();
    assert_eq!(attachment.state, AttachmentState::Disconnected);
    assert_eq!(attachment.attachment_id, original.attachment_id);
    assert_eq!(attachment.epoch, original.epoch);
    assert_eq!(attachment.process, original.process);
    assert_eq!(attachment.shell, original.shell);
    assert_eq!(attachment.machine_id, original.machine_id);
    assert_eq!(attachment.launch_mode, original.launch_mode);
    assert_eq!(serde_json::to_value(&preserved.dispatch).unwrap(), serde_json::to_value(&run.dispatch).unwrap());
    let candidate = root_candidate(preserved, false);
    assert_eq!(candidate.connection, RootConnection::Unknown);
    assert_eq!(candidate.resume, RootResumeAvailability::Blocked);
    let before = fs::read(fixture.service.base().join("state.json")).unwrap();
    let offline = OrchestrationRuntime {
        service: Arc::new(OrchestrationService::open(&ProjectConfiguration::for_tests(&fixture.root)).unwrap()),
        projects: Arc::new(projects), herdr: Arc::new(ObservedHerdr(None)),
        dispatcher: Mutex::new(None), task: Mutex::new(None), shutdown: CancellationToken::new(),
    };
    let replayed = offline.mutate(OperatorOrigin::Browser, request).await.unwrap();
    let OrchestrationActionResult::SupervisorStarted { start: replayed } = replayed.result
        else { panic!("replayed start result"); };
    assert!(replayed.duplicate);
    assert_eq!(replayed.root_id, run.run_id);
    assert_eq!(before, fs::read(fixture.service.base().join("state.json")).unwrap());
}

#[cfg(target_os = "linux")]
#[tokio::test]
async fn adopt_resolves_only_fresh_exact_caller_pane_workspace() {
    let fixture = Fixture::new();
    let projects = fixture.projects(None);
    let mut run = reviewed();
    bind_process(&mut run, std::process::id());
    let caller = cockpit_core::orchestration::BootstrapCaller {
        endpoint_identity: "endpoint".into(), session_id: "fixture".into(), workspace_id: "space".into(),
        tab_id: "tab".into(), pane_id: "pane".into(), boot_id: None, terminal_id: Some("terminal".into()),
        native_session_id: Some("native".into()), omp_session_id: Some("native".into()),
        agent_kind: Some(AgentKind::Main), actual_agent_kind: Some(cockpit_core::orchestration::NativeAgentKind::Omp),
        process: run.attachment.unwrap().process,
    };
    let actor = Actor::Bootstrap(caller);
    let request = OrchestrationMutationRequest {
        session_id: "fixture".into(), expected_revision: None,
        action: OrchestrationAction::RunAdopt { label: "Adopted".into() },
    };
    let mut runtime = fixture.runtime();
    runtime.0.as_mut().unwrap().panes[0].native_session_id = Some("native".into());
    let resolved = resolve_mutation_input(&fixture.service, &projects, &runtime, &actor, &request).await.unwrap();
    assert_eq!(resolved.checkout.unwrap().path, fixture.root.to_string_lossy());
    let mut unavailable = runtime.clone();
    unavailable.0.as_mut().unwrap().workspaces[0].cwd = ".".into();
    let unbound = resolve_mutation_input(&fixture.service, &projects, &unavailable, &actor, &request)
        .await.unwrap();
    assert!(unbound.project.is_none());
    assert!(unbound.checkout.is_none());
    runtime.0.as_mut().unwrap().panes[0].native_session_id = Some("foreign".into());
    assert_eq!(resolve_mutation_input(&fixture.service, &projects, &runtime, &actor, &request)
        .await.unwrap_err().code, "caller_mismatch");
}

#[cfg(target_os = "linux")]
#[tokio::test]
async fn legacy_process_machine_is_observed_only_with_exact_fresh_native_pane() {
    let fixture = Fixture::new();
    let mut run = reviewed();
    bind_process(&mut run, std::process::id());
    run.attachment.as_mut().unwrap().machine_id = None;
    let mut runtime = fixture.runtime();
    runtime.0.as_mut().unwrap().panes[0].native_session_id = Some("native".into());
    let resolved = resolve_resume_input(&fixture.service, &runtime, &run).await.unwrap();
    assert!(resolved.attachment_observation.unwrap().machine_id.is_some());
    assert!(resolved.checkout.is_some());
    let mut retargeted = run.clone();
    retargeted.target = Some(DispatchTarget::Setup { request: WorkspaceSetupRequest::Open {
        path: fixture.root.join("previous-checkout").to_string_lossy().into_owned(),
        label: None, task_name: None, focus: false,
    }});
    assert_eq!(resolve_resume_input(&fixture.service, &runtime, &retargeted)
        .await.unwrap_err().code, "checkout_identity_changed");
    runtime.0.as_mut().unwrap().endpoint_identity = "replacement-endpoint".into();
    let refused = resolve_resume_input(&fixture.service, &runtime, &run).await.unwrap();
    assert!(refused.attachment_observation.is_none());
    assert!(refused.writer_exit_proof.is_none());
}

#[test]
fn workspace_resolution_refuses_missing_relative_or_ambiguous_directory() {
    let mut runtime = observed();
    assert_eq!(authoritative_workspace(&runtime, "space").unwrap_err().code, "workspace_missing");
    runtime.workspaces.push(RuntimeWorkspace {
        workspace_id: "space".into(), label: "Space".into(), cwd: ".".into(),
    });
    assert_eq!(authoritative_workspace(&runtime, "space").unwrap_err().code, "workspace_cwd_missing");
    runtime.workspaces[0].cwd = "/absolute".into();
    runtime.workspaces.push(runtime.workspaces[0].clone());
    assert_eq!(authoritative_workspace(&runtime, "space").unwrap_err().code, "workspace_cwd_missing");
}
#[test]
fn installed_native_cockpit_uses_sibling_cli_never_gui_self() {
    let root =
        std::env::temp_dir().join(format!("cockpit-installed-cli-{}", std::process::id()));
    fs::create_dir_all(&root).unwrap();
    let native = root.join("cockpit");
    let sibling = root.join("cockpit-cli");
    fs::write(&native, b"native-gui").unwrap();
    fs::write(&sibling, b"actual-cli").unwrap();
    assert_eq!(resolve_cli_path(&native, CliProcessRole::Native), sibling);
    assert_eq!(resolve_cli_path(&native, CliProcessRole::Host), native);
    fs::remove_file(&sibling).unwrap();
    assert_eq!(
        resolve_cli_path(&native, CliProcessRole::Native),
        PathBuf::from("cockpit-cli")
    );
    // Cargo's explicitly named native GUI can use the separate host
    // sibling, while the installed native GUI never resolves to itself.
    assert_eq!(
        resolve_cli_path(&root.join("cockpit-tauri"), CliProcessRole::Native),
        native
    );
    fs::remove_dir_all(root).unwrap();
}
