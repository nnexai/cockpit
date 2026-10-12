use super::*;
use cockpit_core::orchestration::herdr::{
    AgentTabRequest, DirectAgentTabRequest, PaneProcessInfo, RuntimePane, RuntimeView,
};
use cockpit_core::{
    HerdrAdapter, ProjectHerdrAdapter, SessionSubscription, TerminalSession,
    project_adapter::*,
};
use cockpit_protocol::v1 as wire;
use std::{fs::OpenOptions, io::Write, os::unix::fs::OpenOptionsExt};

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
        "assignment_generation":null,
        "launch_config":{"harness_kind":"test-harness","command":"test-agent","extra_args":[],
            "resume_args":["--resume","{session_id}"],"inbox_prompt_hints":false,"routes":[]},
        "dispatch":{"launch_tag":"tag","endpoint_identity":"endpoint","agent_started":false,
            "step":"launch_unknown","launch_attempt":0,"updated_at":"2026-10-06T00:00:00Z"},
        "annotations":[],
        "attachment":{"attachment_id":"attachment","epoch":1,"state":"disconnected",
            "location":{"endpoint_identity":"endpoint","session_id":"fixture","workspace_id":"space",
                "tab_id":"tab","pane_id":"pane","launch_tag":"tag","terminal_id":"terminal"},
            "conversation":null,"process":null,"shell":null,"machine_id":null,
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
            agent_kind: Some("test-harness".into()),
            launch_pending: false,
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
        let mut configuration = ProjectConfiguration::for_tests(&root);
        configuration.orchestration = reviewed().launch_config;
        let service = OrchestrationService::open(&configuration).unwrap();
        Self { root, service }
    }
    fn projects(&self, inventory: Option<ProjectInventory>) -> ProjectService {
        ProjectService::new(ProjectConfiguration::for_tests(&self.root),
            Arc::new(InspectionProjectAdapter(inventory))).unwrap()
    }
    fn publish(&self, runs: &[Run]) {
        self.publish_with_claims(runs, &[]);
    }
    fn publish_with_claims(&self, runs: &[Run], claims: &[serde_json::Value]) {
        let state = serde_json::json!({
            "schema":3,"revision":7,"runs":runs,"messages":[],"task_ownership":[],"inbox_hints":[],
            "task_intents":[],"scope_intents":[],"assignment_intents":[],"credentials":[],
            "checkout_claims":claims,"operations":[],
            "resource_requests":[],"resource_sequence":0,"owned_resources":[]
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

#[test]
fn runtime_constructor_accepts_generic_configuration_without_extension_or_launch() {
    let fixture = Fixture::new();
    let mut configuration = ProjectConfiguration::for_tests(&fixture.root);
    configuration.orchestration = cockpit_protocol::projects::OrchestrationConfiguration {
        harness_kind: "independent-harness".into(),
        command: fixture.root.join("not-installed-agent").to_string_lossy().into_owned(),
        extra_args: vec!["--literal-harness-option".into(), "value with spaces".into()],
        resume_args: Some(vec!["resume".into(), "{session_id}".into()]),
        inbox_prompt_hints: false,
        routes: Vec::new(),
    };
    let sources = Arc::new(cockpit_core::sources::SourceService::new(&configuration, Vec::new()).unwrap());
    let library = Arc::new(LibraryService::new(configuration.clone(), sources));
    let herdr = Arc::new(HerdrCliAdapter::new(
        cockpit_herdr::HerdrCliConfig::from_options_with_environment(
            Some(fixture.root.join("not-installed-herdr")), Some("fixture".into()),
            Some(fixture.root.join("not-running.sock")), &Default::default(),
        ).unwrap(),
    ));
    let projects = Arc::new(ProjectService::new(configuration.clone(), herdr.clone()).unwrap());
    let runtime = OrchestrationRuntime::new(
        &configuration, herdr, projects, library, Some(fixture.root.join("config.toml")),
        CliProcessRole::Host,
    ).unwrap();
    assert!(runtime.dispatcher.try_lock().unwrap().is_some());
    assert!(runtime.task.try_lock().unwrap().is_none());
    assert!(!runtime.shutdown.is_cancelled());
    assert!(!runtime.service.base().join("state.json").exists());
    assert!(!configuration.orchestration.inbox_prompt_hints);
    assert!(!fixture.root.join("not-installed-agent").exists());
    assert!(!fixture.root.join("not-running.sock").exists());
}


#[cfg(target_os = "linux")]
fn bind_process(run: &mut Run, pid: u32) {
    let process = NativeProcessIdentity {
        pid,
        start_ticks: cockpit_core::process_identity::start_identity(pid as i32).unwrap(),
        kernel_boot_id: cockpit_core::process_identity::kernel_boot_id(),
    };
    let attachment = run.attachment.as_mut().unwrap();
    attachment.conversation = Some(ConversationIdentity {
        harness: run.launch_config.harness_kind.clone(), session_id: "conversation".into(),
    });
    attachment.machine_id = verified_machine_id(&process);
    attachment.process = Some(process);
    run.dispatch.as_mut().unwrap().agent_started = true;
}

#[cfg(target_os = "linux")]
struct ForegroundPty {
    parent: std::process::Child,
    shell: NativeShellIdentity,
}
#[cfg(target_os = "linux")]
impl ForegroundPty {
    fn new() -> Self {
        use std::io::{BufRead, BufReader};
        use std::process::{Command, Stdio};
        // Only this disposable child gets a controlling PTY. The handshake
        // happens after exec, so both OS foreground and executable evidence
        // describe the shell that remains blocked in its own read builtin.
        let mut parent = Command::new("python3").arg("-c").arg(
            "import os, pty, sys\n\
             pid, fd = pty.fork()\n\
             if pid == 0:\n\
             \x20\x20\x20\x20os.execv('/bin/sh', ['sh', '-c', 'printf \"ready\\\\n\"; read -r line'])\n\
             ready = b''\n\
             while b'ready' not in ready:\n\
             \x20\x20\x20\x20ready += os.read(fd, 1024)\n\
             print(pid, flush=True)\n\
             sys.stdin.readline()\n\
             os.write(fd, b'stop\\n')\n\
             os.waitpid(pid, 0)\n\
             os.close(fd)\n"
        ).stdin(Stdio::piped()).stdout(Stdio::piped()).spawn().unwrap();
        let mut line = String::new();
        BufReader::new(parent.stdout.take().unwrap()).read_line(&mut line).unwrap();
        let pid: u32 = line.trim().parse().unwrap();
        let start = cockpit_core::process_identity::start_identity(pid as i32).unwrap();
        let shell = cockpit_core::process_identity::executable_identity(pid as i32, start).unwrap();
        Self { parent, shell }
    }
    fn herdr(&self, restored: bool) -> OwnedPaneHerdr {
        let mut runtime = observed();
        runtime.panes[0].native_session_id = Some("native".into());
        if restored {
            runtime.endpoint_identity = "restored-endpoint".into();
            runtime.panes[0].tab_id = "restored-tab".into();
            runtime.panes[0].pane_id = "restored-pane".into();
        }
        let pid = self.shell.process.pid;
        let info = PaneProcessInfo {
            pane_id: runtime.panes[0].pane_id.clone(),
            shell_pid: Some(pid), foreground_pgid: Some(pid),
            processes: vec![(pid, "shell".into())], shell_identity: Some(self.shell.clone()),
        };
        OwnedPaneHerdr { runtime, info }
    }
}
#[cfg(target_os = "linux")]
impl Drop for ForegroundPty {
    fn drop(&mut self) {
        self.parent.stdin.take().unwrap().write_all(b"stop\n").unwrap();
        assert!(self.parent.wait().unwrap().success());
    }
}

#[cfg(target_os = "linux")]
struct OwnedPaneHerdr {
    runtime: RuntimeView,
    info: PaneProcessInfo,
}
#[cfg(target_os = "linux")]
#[async_trait::async_trait]
impl OrchestrationHerdr for OwnedPaneHerdr {
    async fn runtime(&self, session: &str) -> Result<RuntimeView, InspectionError> {
        assert_eq!(session, "fixture");
        Ok(self.runtime.clone())
    }
    async fn pane_process_info(&self, session: &str, endpoint: &str, pane: &str) -> Result<PaneProcessInfo, InspectionError> {
        assert_eq!(session, "fixture");
        assert_eq!(endpoint, self.runtime.endpoint_identity);
        assert_eq!(pane, self.info.pane_id);
        Ok(self.info.clone())
    }
    async fn create_agent_tab(&self, _: &str, _: &AgentTabRequest) -> Result<RunLocation, InspectionError> {
        panic!("root observation must not launch a terminal")
    }
    async fn create_direct_agent_tab(&self, _: &str, _: &DirectAgentTabRequest) -> Result<RunLocation, InspectionError> {
        panic!("root observation must not launch a process")
    }
    async fn close_pane(&self, _: &str, _: &str, _: &str) -> Result<(), InspectionError> {
        panic!("root recovery proof never grants cleanup authority")
    }
}

#[cfg(target_os = "linux")]
fn root_with_process(pid: u32) -> Run {
    let mut run = reviewed();
    bind_process(&mut run, pid);
    run.dispatch.as_mut().unwrap().step = DispatchStep::Launched;
    run.attachment.as_mut().unwrap().location.as_mut().unwrap().native_session_id = Some("native".into());
    run
}

#[cfg(target_os = "linux")]
fn dead_root() -> Run {
    let process = ForegroundPty::new();
    let run = root_with_process(process.shell.process.pid);
    drop(process);
    run
}

fn resume_request(run: &Run) -> OrchestrationMutationRequest {
    OrchestrationMutationRequest {
        session_id: run.session_id.clone(), expected_revision: None,
        action: OrchestrationAction::RunResume {
            operation_id: uuid::Uuid::new_v4().to_string(), run_id: run.run_id.clone(),
            expected_attachment_epoch: run.attachment.as_ref().map(|a| a.epoch),
            mode: ResumeMode::ReattachOrRestart, decision_id: None,
        },
    }
}

fn resume_result(response: OrchestrationMutationResponse) -> RunResumeResult {
    let OrchestrationActionResult::RunResumed { resume } = response.result
        else { panic!("expected root Resume result"); };
    resume
}

#[cfg(target_os = "linux")]
fn reserve_root_once(fixture: &Fixture, run: &Run, resolved: &ResolvedMutationInput) {
    let actor = Actor::Operator(OperatorOrigin::Browser);
    let request = resume_request(run);
    let first = resume_result(fixture.service.mutate_resolved(&actor, request.clone(), resolved).unwrap());
    assert!(!first.duplicate);
    assert_eq!(first.resume.phase, ResumePhase::Launching);
    assert_eq!(first.resume.previous_attachment_epoch, Some(1));
    assert_eq!(first.resume.replacement_attachment_epoch, Some(2));
    let replacement = fixture.service.run_for_review("fixture", &run.run_id).unwrap();
    let attachment = replacement.attachment.as_ref().unwrap();
    assert_eq!(attachment.epoch, 2);
    assert_eq!(attachment.state, AttachmentState::Reserved);
    assert_ne!(attachment.attachment_id, run.attachment.as_ref().unwrap().attachment_id);
    assert!(attachment.process.is_none());
    assert!(attachment.shell.is_none());
    assert!(attachment.location.is_none());
    assert_eq!(replacement.dispatch.as_ref().unwrap().launch_attempt, 1);
    let before_retry = fs::read(fixture.service.base().join("state.json")).unwrap();
    let retry = resume_result(fixture.service.mutate_resolved(&actor, request, resolved).unwrap());
    assert!(retry.duplicate);
    assert_eq!(retry.resume.replacement_attachment_epoch, Some(2));
    assert_eq!(before_retry, fs::read(fixture.service.base().join("state.json")).unwrap());
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
async fn resume_rebooted_root_uses_process_proof_and_unknown_roots_stay_protected() {
    let fixture = Fixture::new();
    let mut run = root_with_process(std::process::id());
    run.kind = RunKind::Adopted;
    let boot = run.attachment.as_mut().unwrap().process.as_mut().unwrap().kernel_boot_id.as_mut().unwrap();
    let replacement = if boot.starts_with('0') { "1" } else { "0" };
    boot.replace_range(0..1, replacement);
    fixture.publish(&[run.clone()]);
    for drift in [false, true] {
        let mut herdr = empty_observation();
        if drift { herdr.0.as_mut().unwrap().endpoint_identity = "new-endpoint".into(); }
        let resolved = resolve_resume_input(&fixture.service, &herdr, &run).await.unwrap();
        assert!(resolved.writer_exit_proof.is_none());
        assert!(resolved.root_main_recovery.is_some());
        assert!(resolved.attachment_observation.is_none());
        assert!(resolved.observation_cause.is_none());
    }
    let offline = resolve_resume_input(&fixture.service, &ObservedHerdr(None), &run).await.unwrap();
    assert!(offline.writer_exit_proof.is_none());
    assert!(offline.root_main_recovery.is_none());
    assert_eq!(offline.observation_cause.unwrap().code, "resume_observation_unavailable");
    for change in 0..3 {
        let mut changed = run.clone();
        let attachment = changed.attachment.as_mut().unwrap();
        match change {
            0 => attachment.machine_id = None,
            1 => attachment.machine_id = Some("foreign-machine".into()),
            _ => attachment.process = None,
        }
        fixture.publish(&[changed.clone()]);
        let refused = resolve_resume_input(&fixture.service, &empty_observation(), &changed).await.unwrap();
        assert!(refused.writer_exit_proof.is_none());
        assert!(refused.root_main_recovery.is_none());
        assert!(refused.attachment_observation.is_none());
        let cause = refused.observation_cause.as_ref().unwrap();
        assert_eq!(cause.code, "root_main_exit_unproven");
        assert_eq!(cause.class, ResumeCauseClass::Recover);
        let blocked = resume_result(fixture.service.mutate_resolved(&Actor::Operator(OperatorOrigin::Browser),
            resume_request(&changed), &refused).unwrap());
        assert_eq!(blocked.resume.phase, ResumePhase::Blocked);
        assert_eq!(serde_json::to_value(fixture.service.run_for_review("fixture", &changed.run_id).unwrap().attachment).unwrap(), serde_json::to_value(&changed.attachment).unwrap());
    }
}

#[cfg(target_os = "linux")]
#[tokio::test]
async fn resume_same_boot_dead_root_missing_container_reserves_only_root_epoch_once() {
    let fixture = Fixture::new();
    let run = dead_root();
    let mut worker = root_with_process(std::process::id());
    worker.run_id = "worker".into();
    worker.kind = RunKind::Worker;
    worker.parent_run_id = Some(run.run_id.clone());
    worker.attachment.as_mut().unwrap().attachment_id = "worker-attachment".into();
    let (project, checkout) = resolve_directory(&fixture.root, None, None).await.unwrap();
    worker.project = Some(project);
    worker.checkout = Some(checkout);
    worker.checkout_claim_id = Some("worker-claim".into());
    let claim = serde_json::json!({
        "claim_id":"worker-claim","checkout":worker.checkout,"revision":3,"state":"held",
        "holder":{"run_id":worker.run_id,"attachment_id":"worker-attachment","attachment_epoch":1},
        "transition_operation_id":null
    });
    fixture.publish_with_claims(&[run.clone(), worker.clone()], &[claim.clone()]);
    let resolved = resolve_resume_input(&fixture.service, &empty_observation(), &run).await.unwrap();
    assert!(resolved.writer_exit_proof.is_none(), "root exit is not an all-consumer cleanup proof");
    assert!(resolved.root_main_recovery.is_some());
    assert!(resolved.attachment_observation.is_none());
    assert!(resolved.observation_cause.is_none());
    reserve_root_once(&fixture, &run, &resolved);
    let preserved = fixture.service.run_for_review("fixture", &worker.run_id).unwrap();
    assert_eq!(serde_json::to_value(preserved).unwrap(), serde_json::to_value(worker).unwrap());
    let state: serde_json::Value = serde_json::from_slice(&fs::read(fixture.service.base().join("state.json")).unwrap()).unwrap();
    assert_eq!(state["checkout_claims"], serde_json::json!([claim]));
}

#[cfg(target_os = "linux")]
#[tokio::test]
async fn resume_dead_root_accepts_fresh_original_and_restored_idle_shells() {
    let shell = ForegroundPty::new();
    for restored in [false, true] {
        let fixture = Fixture::new();
        let mut run = dead_root();
        if !restored { run.attachment.as_mut().unwrap().shell = Some(shell.shell.clone()); }
        fixture.publish(&[run.clone()]);
        let herdr = shell.herdr(restored);
        let resolved = resolve_resume_input(&fixture.service, &herdr, &run).await.unwrap();
        assert!(resolved.writer_exit_proof.is_none());
        assert!(resolved.root_main_recovery.is_some(), "restored {restored}");
        assert!(resolved.attachment_observation.is_none());
        assert!(resolved.observation_cause.is_none());
        reserve_root_once(&fixture, &run, &resolved);
    }
}

#[cfg(target_os = "linux")]
#[tokio::test]
async fn resume_root_refuses_live_missing_container_and_unsettled_dispatch() {
    let fixture = Fixture::new();
    let live = ForegroundPty::new();
    let run = root_with_process(live.shell.process.pid);
    fixture.publish(&[run.clone()]);
    let refused = resolve_resume_input(&fixture.service, &empty_observation(), &run).await.unwrap();
    assert!(refused.root_main_recovery.is_none());
    assert!(refused.attachment_observation.is_none());
    assert!(refused.writer_exit_proof.is_none());
    assert_eq!(refused.observation_cause.as_ref().unwrap().code, "consumer_activity_unproven");
    let result = resume_result(fixture.service.mutate_resolved(&Actor::Operator(OperatorOrigin::Browser),
        resume_request(&run), &refused).unwrap());
    assert_eq!(result.resume.phase, ResumePhase::Blocked);
    assert_eq!(serde_json::to_value(fixture.service.run_for_review("fixture", &run.run_id).unwrap().attachment).unwrap(), serde_json::to_value(&run.attachment).unwrap());
    let dead = dead_root();
    for step in [DispatchStep::SetupRunning, DispatchStep::SetupUnknown, DispatchStep::LaunchIntent,
        DispatchStep::LaunchPending, DispatchStep::LaunchUnknown] {
        let mut changed = dead.clone();
        changed.dispatch.as_mut().unwrap().step = step;
        fixture.publish(&[changed.clone()]);
        let refused = resolve_resume_input(&fixture.service, &empty_observation(), &changed).await.unwrap();
        assert!(refused.writer_exit_proof.is_none());
        assert!(refused.root_main_recovery.is_none());
        assert!(refused.attachment_observation.is_none());
        assert_eq!(refused.observation_cause.unwrap().code, "root_main_exit_unproven", "{step:?}");
    }
    let mut mismatched = dead;
    mismatched.dispatch.as_mut().unwrap().launch_attempt += 1;
    fixture.publish(&[mismatched.clone()]);
    let refused = resolve_resume_input(&fixture.service, &empty_observation(), &mismatched).await.unwrap();
    assert!(refused.root_main_recovery.is_none());
    assert_eq!(refused.observation_cause.unwrap().code, "root_main_exit_unproven");
}

#[cfg(target_os = "linux")]
#[tokio::test]
async fn resume_root_refuses_pending_foreign_ambiguous_and_different_live_pid_occupants() {
    let fixture = Fixture::new();
    let run = dead_root();
    let shell = ForegroundPty::new();
    for occupant in 0..9 {
        fixture.publish(&[run.clone()]);
        let mut herdr = shell.herdr(true);
        match occupant {
            0 => herdr.runtime.panes[0].launch_pending = true,
            1 => herdr.runtime.panes[0].agent_kind = Some("foreign-agent".into()),
            2 => {
                let mut alias = herdr.runtime.panes[0].clone();
                alias.pane_id = "other-pane".into();
                alias.terminal_id = Some("other-terminal".into());
                herdr.runtime.panes.push(alias);
            }
            3 => herdr.runtime.panes[0].native_session_id = Some("foreign-native".into()),
            4 => herdr.info.shell_identity = None,
            5 => herdr.info.foreground_pgid = None,
            6 => herdr.info.shell_identity.as_mut().unwrap().argv_digest.push('0'),
            7 => herdr.runtime.panes[0].terminal_id = Some("different-terminal".into()),
            _ => herdr.info.processes.push((std::process::id(), "other-live-process".into())),
        }
        let refused = resolve_resume_input(&fixture.service, &herdr, &run).await.unwrap();
        assert!(refused.writer_exit_proof.is_none());
        assert!(refused.root_main_recovery.is_none(), "occupant {occupant}");
        assert!(refused.attachment_observation.is_none());
        let cause = refused.observation_cause.as_ref().unwrap();
        assert_eq!(cause.code, "consumer_activity_unproven", "occupant {occupant}");
        assert_eq!(cause.class, ResumeCauseClass::Recover);
        let result = resume_result(fixture.service.mutate_resolved(&Actor::Operator(OperatorOrigin::Browser),
            resume_request(&run), &refused).unwrap());
        assert_eq!(result.resume.phase, ResumePhase::Blocked);
        assert_eq!(serde_json::to_value(fixture.service.run_for_review("fixture", &run.run_id).unwrap().attachment).unwrap(), serde_json::to_value(&run.attachment).unwrap());
    }
}

#[cfg(target_os = "linux")]
#[tokio::test]
async fn resume_live_root_reattaches_restored_endpoint_with_current_authenticated_binding() {
    use cockpit_core::orchestration::{AgentCaller, NativeAgentKind, read_job_auth};
    let fixture = Fixture::new();
    let process = ForegroundPty::new();
    let mut original = process.herdr(false);
    original.runtime.workspaces.push(RuntimeWorkspace {
        workspace_id: "space".into(), label: "Project".into(),
        cwd: fixture.root.to_string_lossy().into_owned(),
    });
    let mut location = reviewed().attachment.unwrap().location.unwrap();
    location.native_session_id = Some("native".into());
    // Operator adoption issues the capability; registration pins its conversation.
    // Physical recovery evidence comes from the real disposable PTY, not registration.
    let actor = Actor::Operator(OperatorOrigin::Browser);
    let request = OrchestrationMutationRequest {
        session_id: "fixture".into(), expected_revision: None,
        action: OrchestrationAction::RunAdopt { label: "Root".into(), location: location.clone() },
    };
    let mut resolved = resolve_mutation_input(&fixture.service, &fixture.projects(None), &original, &actor, &request)
        .await.unwrap();
    let attachment = resolved.attachment_observation.as_mut().unwrap();
    attachment.process = Some(process.shell.process.clone());
    attachment.machine_id = verified_machine_id(&process.shell.process);
    let response = fixture.service.mutate_resolved(&actor, request, &resolved).unwrap();
    let OrchestrationActionResult::Run { run_id, .. } = response.result else { panic!("adopted root"); };
    let path = fixture.service.job_adoption_credential_file(&actor, "fixture", &run_id, &location).unwrap();
    let auth = read_job_auth(&path).unwrap();
    let conversation = ConversationIdentity { harness: "test-harness".into(), session_id: "opaque/chat:one".into() };
    let agent = AgentCaller {
        auth, conversation: conversation.clone(), assignment_generation: None,
        endpoint_identity: location.endpoint_identity.clone(),
        session_id: location.session_id.clone(), workspace_id: location.workspace_id.clone(),
        tab_id: location.tab_id.clone(), pane_id: location.pane_id.clone(), boot_id: location.boot_id.clone(),
        terminal_id: location.terminal_id.clone(), native_session_id: location.native_session_id.clone(),
        actual_agent_kind: Some(NativeAgentKind::Other("test-harness".into())),
        process: Some(process.shell.process.clone()),
    };
    let registration = fixture.service.mutate_resolved(&Actor::Agent(agent.clone()), OrchestrationMutationRequest {
        session_id: "fixture".into(), expected_revision: None,
        action: OrchestrationAction::RunRegister { operation_id: uuid::Uuid::new_v4().to_string() },
    }, &ResolvedMutationInput::default()).unwrap();
    assert!(matches!(registration.result, OrchestrationActionResult::RunRegistered {
        conversation: registered, assignment_generation: None, duplicate: false, ..
    } if registered == conversation));
    let run = fixture.service.run_for_review("fixture", &run_id).unwrap();
    assert!(fixture.service.recorded_runs_with_bindings("fixture").unwrap().2.contains_key(&run_id));
    let restored = process.herdr(true);
    let resolved = resolve_resume_input(&fixture.service, &restored, &run).await.unwrap();
    assert!(resolved.writer_exit_proof.is_none());
    assert!(resolved.root_main_recovery.is_none());
    assert!(resolved.observation_cause.is_none());
    let fresh = resolved.attachment_observation.as_ref().unwrap();
    let original = run.attachment.as_ref().unwrap();
    assert_eq!(fresh.process, original.process);
    assert_eq!(fresh.attachment_id, original.attachment_id);
    assert_eq!(fresh.epoch, original.epoch);
    let location = fresh.location.as_ref().unwrap();
    assert_eq!(location.endpoint_identity, "restored-endpoint");
    assert_eq!(location.tab_id, "restored-tab");
    assert_eq!(location.pane_id, "restored-pane");
    assert_eq!(location.terminal_id, original.location.as_ref().unwrap().terminal_id);
    let request = resume_request(&run);
    let operator = Actor::Operator(OperatorOrigin::Browser);
    let result = resume_result(fixture.service.mutate_resolved(&operator, request.clone(), &resolved).unwrap());
    assert!(!result.duplicate);
    assert_eq!(result.resume.phase, ResumePhase::Reattached);
    assert!(result.resume.replacement_attachment_epoch.is_none());
    let saved = fixture.service.run_for_review("fixture", &run_id).unwrap();
    assert_eq!(serde_json::to_value(saved.attachment.as_ref().unwrap()).unwrap(), serde_json::to_value(fresh).unwrap());
    assert!(fixture.service.recorded_runs_with_bindings("fixture").unwrap().2.contains_key(&run_id));
    let before = fs::read(fixture.service.base().join("state.json")).unwrap();
    assert!(resume_result(fixture.service.mutate_resolved(&operator, request, &resolved).unwrap()).duplicate);
    assert_eq!(before, fs::read(fixture.service.base().join("state.json")).unwrap());
}

#[cfg(target_os = "linux")]
#[tokio::test]
async fn resume_live_root_refuses_different_foreground_incarnation() {
    let fixture = Fixture::new();
    let original = ForegroundPty::new();
    let other = ForegroundPty::new();
    let run = root_with_process(original.shell.process.pid);
    fixture.publish(&[run.clone()]);
    let herdr = other.herdr(true);
    let refused = resolve_resume_input(&fixture.service, &herdr, &run).await.unwrap();
    assert!(refused.writer_exit_proof.is_none());
    assert!(refused.root_main_recovery.is_none());
    assert!(refused.attachment_observation.is_none());
    assert_eq!(refused.observation_cause.as_ref().unwrap().code, "consumer_activity_unproven");
    let result = resume_result(fixture.service.mutate_resolved(&Actor::Operator(OperatorOrigin::Browser),
        resume_request(&run), &refused).unwrap());
    assert_eq!(result.resume.phase, ResumePhase::Blocked);
    assert_eq!(serde_json::to_value(fixture.service.run_for_review("fixture", &run.run_id).unwrap().attachment).unwrap(), serde_json::to_value(&run.attachment).unwrap());
}

#[cfg(target_os = "linux")]
#[tokio::test]
async fn resume_worker_retains_reboot_all_consumer_proof_and_offline_refusal() {
    let fixture = Fixture::new();
    let process = ForegroundPty::new();
    let mut worker = root_with_process(process.shell.process.pid);
    worker.kind = RunKind::Worker;
    worker.parent_run_id = Some("root".into());
    worker.root_id = "root".into();
    worker.attachment.as_mut().unwrap().shell = Some(process.shell.clone());
    let (project, checkout) = resolve_directory(&fixture.root, None, None).await.unwrap();
    worker.project = Some(project);
    worker.checkout = Some(checkout);
    worker.checkout_claim_id = Some("worker-claim".into());
    let claim = serde_json::json!({
        "claim_id":"worker-claim","checkout":worker.checkout,"revision":1,"state":"held",
        "holder":{"run_id":worker.run_id,"attachment_id":"attachment","attachment_epoch":1},
        "transition_operation_id":null
    });
    drop(process);
    fixture.publish_with_claims(&[worker.clone()], &[claim.clone()]);
    let refused = resolve_resume_input(&fixture.service, &empty_observation(), &worker).await.unwrap();
    assert!(refused.writer_exit_proof.is_none());
    assert!(refused.root_main_recovery.is_none());
    assert!(refused.attachment_observation.is_none());
    assert_eq!(refused.observation_cause.unwrap().code, "consumer_activity_unproven");
    let attachment = worker.attachment.as_mut().unwrap();
    for process in [attachment.process.as_mut().unwrap(), &mut attachment.shell.as_mut().unwrap().process] {
        let boot = process.kernel_boot_id.as_mut().unwrap();
        let replacement = if boot.starts_with('0') { "1" } else { "0" };
        boot.replace_range(0..1, replacement);
    }
    fixture.publish_with_claims(&[worker.clone()], &[claim]);
    let resolved = resolve_resume_input(&fixture.service, &empty_observation(), &worker).await.unwrap();
    assert!(resolved.writer_exit_proof.is_some());
    assert!(resolved.root_main_recovery.is_none());
    assert!(resolved.attachment_observation.is_none());
    let offline = resolve_resume_input(&fixture.service, &ObservedHerdr(None), &worker).await.unwrap();
    assert!(offline.writer_exit_proof.is_none());
    assert!(offline.root_main_recovery.is_none());
    assert_eq!(offline.observation_cause.unwrap().code, "resume_observation_unavailable");
    let occupied = resolve_resume_input(&fixture.service, &ObservedHerdr(Some(observed())), &worker).await.unwrap();
    assert!(occupied.writer_exit_proof.is_none());
    assert!(occupied.root_main_recovery.is_none());
    assert_eq!(occupied.observation_cause.unwrap().code, "consumer_activity_unproven");
}

#[cfg(target_os = "linux")]
#[tokio::test]
async fn resume_root_terminal_ownership_fallback_is_same_endpoint_only() {
    let fixture = Fixture::new();
    let shell = ForegroundPty::new();
    let mut run = dead_root();
    run.attachment.as_mut().unwrap().location.as_mut().unwrap().terminal_id = None;
    fixture.publish(&[run.clone()]);
    let original = resolve_resume_input(&fixture.service, &shell.herdr(false), &run).await.unwrap();
    assert!(original.root_main_recovery.is_some());
    assert!(original.writer_exit_proof.is_none());
    let restored = resolve_resume_input(&fixture.service, &shell.herdr(true), &run).await.unwrap();
    assert!(restored.root_main_recovery.is_none());
    assert!(restored.writer_exit_proof.is_none());
    assert!(restored.attachment_observation.is_none());
    assert_eq!(restored.observation_cause.unwrap().code, "consumer_activity_unproven");
}

#[cfg(target_os = "linux")]
#[tokio::test]
async fn resume_dispatchless_adopted_root_requires_exact_retained_checkout_and_workspace() {
    let fixture = Fixture::new();
    let mut run = dead_root();
    run.kind = RunKind::Adopted;
    run.dispatch = None;
    let (project, checkout) = resolve_directory(&fixture.root, None, None).await.unwrap();
    run.project = Some(project);
    run.checkout = Some(checkout.clone());
    fixture.publish(&[run.clone()]);
    let mut runtime = fixture.runtime();
    runtime.0.as_mut().unwrap().panes.clear();
    let resolved = resolve_resume_input(&fixture.service, &runtime, &run).await.unwrap();
    assert!(resolved.root_main_recovery.is_some());
    assert!(resolved.writer_exit_proof.is_none());
    assert!(resolved.attachment_observation.is_none());
    reserve_root_once(&fixture, &run, &resolved);
    let replacement = fixture.service.run_for_review("fixture", &run.run_id).unwrap();
    assert!(matches!(replacement.target,
        Some(DispatchTarget::ExistingSpace { workspace_id }) if workspace_id == "space"));
    let setup = replacement.setup.as_ref().unwrap();
    assert_eq!(setup.checkout_path, checkout.path);
    assert_eq!(setup.workspace_id.as_deref(), Some("space"));
    assert!(setup.operation_id.is_none());
    assert!(setup.ownership.is_none());
    assert!(setup.effects.is_empty());
    assert_eq!(replacement.checkout, run.checkout);
    for change in 0..4 {
        let mut changed = run.clone();
        let mut unavailable = runtime.clone();
        match change {
            0 => changed.checkout = None,
            1 => unavailable.0.as_mut().unwrap().workspaces[0].cwd = fixture.root.join("foreign").to_string_lossy().into_owned(),
            2 => {
                let workspace = unavailable.0.as_ref().unwrap().workspaces[0].clone();
                unavailable.0.as_mut().unwrap().workspaces.push(workspace);
            }
            _ => changed.target = Some(DispatchTarget::ExistingSpace { workspace_id: "space".into() }),
        }
        fixture.publish(&[changed.clone()]);
        let refused = resolve_resume_input(&fixture.service, &unavailable, &changed).await.unwrap();
        assert!(refused.root_main_recovery.is_none(), "change {change}");
        assert!(refused.writer_exit_proof.is_none());
        assert!(refused.attachment_observation.is_none());
        let cause = refused.observation_cause.as_ref().unwrap();
        assert_eq!(cause.class, ResumeCauseClass::Recover);
        let blocked = resume_result(fixture.service.mutate_resolved(&Actor::Operator(OperatorOrigin::Browser),
            resume_request(&changed), &refused).unwrap());
        assert_eq!(blocked.resume.phase, ResumePhase::Blocked);
        assert_eq!(serde_json::to_value(fixture.service.run_for_review("fixture", &run.run_id).unwrap().attachment).unwrap(), serde_json::to_value(&changed.attachment).unwrap());
    }
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
    // Physical liveness, even a saved Connected flag, is not current registration.
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
        let unregistered = roots.iter().find(|r| r.root_id == "a").unwrap();
        assert_eq!(unregistered.connection, RootConnection::Unknown);
        assert_eq!(unregistered.resume, RootResumeAvailability::Blocked);
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
    let unregistered = root_candidate(&run, false);
    assert_eq!(unregistered.connection, RootConnection::Unknown);
    assert_eq!(unregistered.resume, RootResumeAvailability::Blocked);
    assert!(unregistered.cause.is_some());
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
async fn start_preserves_selected_unregistered_live_root_without_retargeting_or_new_writer() {
    let fixture = Fixture::new();
    let projects = fixture.projects(None);
    let (project, checkout) = resolve_directory(&fixture.root, None, None).await.unwrap();
    let process = ForegroundPty::new();
    let mut run = root_with_process(process.shell.process.pid);
    run.project = Some(project);
    run.checkout = Some(checkout.clone());
    let mut herdr = process.herdr(false);
    herdr.runtime.workspaces.push(RuntimeWorkspace {
        workspace_id: "space".into(), label: "Project".into(),
        cwd: fixture.root.to_string_lossy().into_owned(),
    });
    run.attachment.as_mut().unwrap().state = AttachmentState::Connected;
    // This fixture has no authenticated RunRegister; preserving its living writer
    // must not become permission to launch a replacement or report registration.
    fixture.publish(&[run.clone()]);
    let actor = Actor::Operator(OperatorOrigin::Browser);
    let mut request = start_request(Some(DispatchTarget::ExistingSpace { workspace_id: "space".into() }),
        SupervisorStartMode::ReuseOrResume);
    let OrchestrationAction::SupervisorStart { selected_root_id, .. } = &mut request.action
        else { unreachable!() };
    *selected_root_id = Some(run.run_id.clone());
    let resolved = resolve_mutation_input(&fixture.service, &projects, &herdr, &actor, &request)
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

#[tokio::test]
async fn operator_adopt_resolves_exact_pane_with_generic_harness_without_process_evidence() {
    let fixture = Fixture::new();
    let projects = fixture.projects(None);
    let actor = Actor::Operator(OperatorOrigin::Browser);
    let location = reviewed().attachment.unwrap().location.unwrap();
    let request = OrchestrationMutationRequest {
        session_id: "fixture".into(), expected_revision: None,
        action: OrchestrationAction::RunAdopt { label: "Adopted".into(), location: location.clone() },
    };
    let mut runtime = fixture.runtime();
    runtime.0.as_mut().unwrap().panes[0].agent_kind = Some("independent-harness".into());
    let resolved = resolve_mutation_input(&fixture.service, &projects, &runtime, &actor, &request).await.unwrap();
    assert_eq!(resolved.checkout.as_ref().unwrap().path, fixture.root.to_string_lossy());
    let attachment = resolved.attachment_observation.as_ref().unwrap();
    assert_eq!(serde_json::to_value(&attachment.location).unwrap(), serde_json::to_value(Some(&location)).unwrap());
    assert!(attachment.conversation.is_none());
    assert!(attachment.process.is_none());
    assert!(attachment.shell.is_none());
    assert!(attachment.machine_id.is_none());
    let response = fixture.service.mutate_resolved(&actor, request.clone(), &resolved).unwrap();
    let OrchestrationActionResult::Run { run_id, .. } = response.result else { panic!("adopted root"); };
    let adopted = fixture.service.run_for_review("fixture", &run_id).unwrap();
    assert_eq!(adopted.kind, RunKind::Adopted);
    assert!(adopted.assignment_generation.is_none());
    assert!(adopted.attachment.as_ref().unwrap().process.is_none());
    assert!(fixture.service.recorded_runs_with_bindings("fixture").unwrap().2.is_empty());
    let path = fixture.service.job_adoption_credential_file(&actor, "fixture", &run_id, &location).unwrap();
    let caller = cockpit_core::orchestration::AgentCaller {
        auth: cockpit_core::orchestration::read_job_auth(&path).unwrap(),
        conversation: ConversationIdentity { harness: "test-harness".into(), session_id: "opaque/session:without-pid".into() },
        assignment_generation: None,
        endpoint_identity: location.endpoint_identity.clone(), session_id: location.session_id.clone(),
        workspace_id: location.workspace_id.clone(), tab_id: location.tab_id.clone(),
        pane_id: location.pane_id.clone(), boot_id: None, terminal_id: location.terminal_id.clone(),
        native_session_id: None, actual_agent_kind: None, process: None,
    };
    let agent = Actor::Agent(caller.clone());
    let registration_request = OrchestrationMutationRequest {
        session_id: "fixture".into(), expected_revision: None,
        action: OrchestrationAction::RunRegister { operation_id: uuid::Uuid::new_v4().to_string() },
    };
    let registration_input = resolve_mutation_input(
        &fixture.service, &projects, &ObservedHerdr(None), &agent, &registration_request,
    ).await.unwrap();
    assert!(registration_input.attachment_observation.is_none());
    assert!(registration_input.writer_exit_proof.is_none());
    let registered = fixture.service.mutate_resolved(&agent, registration_request, &registration_input).unwrap();
    assert!(matches!(registered.result, OrchestrationActionResult::RunRegistered {
        conversation, assignment_generation: None, duplicate: false, ..
    } if conversation == caller.conversation));
    let registered = fixture.service.run_for_review("fixture", &run_id).unwrap();
    let attachment = registered.attachment.as_ref().unwrap();
    assert_eq!(attachment.conversation.as_ref(), Some(&caller.conversation));
    assert!(attachment.process.is_none());
    assert!(attachment.shell.is_none());
    assert!(attachment.machine_id.is_none());
    assert_eq!(resolve_mutation_input(&fixture.service, &projects, &runtime, &agent, &request)
        .await.unwrap_err().code, "actor_forbidden");
    let mut unavailable = runtime.clone();
    unavailable.0.as_mut().unwrap().workspaces[0].cwd = ".".into();
    let unbound = resolve_mutation_input(&fixture.service, &projects, &unavailable, &actor, &request)
        .await.unwrap();
    assert!(unbound.project.is_none());
    assert!(unbound.checkout.is_none());
    assert!(unbound.attachment_observation.unwrap().process.is_none());
}

#[tokio::test]
async fn operator_adopt_refuses_changed_or_ambiguous_pinned_endpoint_pane_terminal() {
    let fixture = Fixture::new();
    let projects = fixture.projects(None);
    let actor = Actor::Operator(OperatorOrigin::Browser);
    let location = reviewed().attachment.unwrap().location.unwrap();
    let request = OrchestrationMutationRequest {
        session_id: "fixture".into(), expected_revision: None,
        action: OrchestrationAction::RunAdopt { label: "Adopted".into(), location },
    };
    for change in 0..9 {
        let mut runtime = fixture.runtime();
        let view = runtime.0.as_mut().unwrap();
        match change {
            0 => view.endpoint_identity = "replacement-endpoint".into(),
            1 => view.panes[0].workspace_id = "different-space".into(),
            2 => view.panes[0].tab_id = "different-tab".into(),
            3 => view.panes[0].pane_id = "different-pane".into(),
            4 => view.panes[0].terminal_id = Some("different-terminal".into()),
            5 => view.panes[0].terminal_id = None,
            6 => view.panes.clear(),
            7 => view.panes.push(view.panes[0].clone()),
            _ => {
                let mut duplicate_terminal = view.panes[0].clone();
                duplicate_terminal.pane_id = "other-pane".into();
                view.panes.push(duplicate_terminal);
            }
        }
        assert_eq!(resolve_mutation_input(&fixture.service, &projects, &runtime, &actor, &request)
            .await.unwrap_err().code, "caller_mismatch", "change {change}");
    }
    let mut missing_terminal = request.clone();
    let OrchestrationAction::RunAdopt { location, .. } = &mut missing_terminal.action else { unreachable!() };
    location.terminal_id = None;
    assert_eq!(resolve_mutation_input(&fixture.service, &projects, &fixture.runtime(), &actor, &missing_terminal)
        .await.unwrap_err().code, "caller_mismatch");
    let mut empty_terminal = request.clone();
    let OrchestrationAction::RunAdopt { location, .. } = &mut empty_terminal.action else { unreachable!() };
    location.terminal_id = Some(String::new());
    let mut empty_runtime = fixture.runtime();
    empty_runtime.0.as_mut().unwrap().panes[0].terminal_id = Some(String::new());
    assert_eq!(resolve_mutation_input(&fixture.service, &projects, &empty_runtime, &actor, &empty_terminal)
        .await.unwrap_err().code, "caller_mismatch");
    let mut wrong_session = request;
    let OrchestrationAction::RunAdopt { location, .. } = &mut wrong_session.action else { unreachable!() };
    location.session_id = "other-physical-session".into();
    assert_eq!(resolve_mutation_input(&fixture.service, &projects, &fixture.runtime(), &actor, &wrong_session)
        .await.unwrap_err().code, "caller_mismatch");
    assert!(!fixture.service.base().join("state.json").exists());
}

#[tokio::test]
async fn operator_adopt_checks_optional_pinned_boot_and_native_receipts_without_requiring_them() {
    let fixture = Fixture::new();
    let projects = fixture.projects(None);
    let actor = Actor::Operator(OperatorOrigin::Browser);
    let mut location = reviewed().attachment.unwrap().location.unwrap();
    location.boot_id = Some("boot".into());
    location.native_session_id = Some("physical-receipt".into());
    let request = OrchestrationMutationRequest {
        session_id: "fixture".into(), expected_revision: None,
        action: OrchestrationAction::RunAdopt { label: "Adopted".into(), location },
    };
    let mut runtime = fixture.runtime();
    runtime.0.as_mut().unwrap().boot_id = Some("boot".into());
    runtime.0.as_mut().unwrap().panes[0].native_session_id = Some("physical-receipt".into());
    runtime.0.as_mut().unwrap().panes[0].agent_kind = None;
    assert!(resolve_mutation_input(&fixture.service, &projects, &runtime, &actor, &request).await.is_ok());
    for change in 0..4 {
        let mut mismatched = runtime.clone();
        let view = mismatched.0.as_mut().unwrap();
        match change {
            0 => view.boot_id = None,
            1 => view.boot_id = Some("other-boot".into()),
            2 => view.panes[0].native_session_id = None,
            _ => view.panes[0].native_session_id = Some("other-receipt".into()),
        }
        assert_eq!(resolve_mutation_input(&fixture.service, &projects, &mismatched, &actor, &request)
            .await.unwrap_err().code, "caller_mismatch", "change {change}");
    }
}

#[cfg(target_os = "linux")]
#[tokio::test]
async fn worker_process_machine_is_observed_only_with_exact_fresh_pane() {
    let fixture = Fixture::new();
    let mut run = reviewed();
    bind_process(&mut run, std::process::id());
    run.kind = RunKind::Worker;
    run.parent_run_id = Some("root".into());
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
fn exact_saved_pane_requires_unique_physical_identity_and_configured_harness() {
    let mut run = reviewed();
    let location = run.attachment.as_mut().unwrap().location.as_mut().unwrap();
    location.boot_id = Some("boot".into());
    location.native_session_id = Some("physical-receipt".into());
    let mut runtime = observed();
    runtime.boot_id = Some("boot".into());
    runtime.panes[0].native_session_id = Some("physical-receipt".into());
    assert!(exact_saved_pane(&runtime, &run));
    let mut unobserved_boot = runtime.clone();
    unobserved_boot.boot_id = None;
    assert!(exact_saved_pane(&unobserved_boot, &run));
    for change in 0..12 {
        let mut mismatched = runtime.clone();
        match change {
            0 => mismatched.endpoint_identity = "other-endpoint".into(),
            1 => mismatched.boot_id = Some("other-boot".into()),
            2 => mismatched.panes[0].workspace_id = "other-space".into(),
            3 => mismatched.panes[0].tab_id = "other-tab".into(),
            4 => mismatched.panes[0].pane_id = "other-pane".into(),
            5 => mismatched.panes[0].terminal_id = Some("other-terminal".into()),
            6 => mismatched.panes[0].native_session_id = None,
            7 => mismatched.panes[0].native_session_id = Some("other-receipt".into()),
            8 => mismatched.panes[0].agent_kind = Some("other-harness".into()),
            9 => mismatched.panes[0].launch_pending = true,
            10 => {
                let mut duplicate_pane = mismatched.panes[0].clone();
                duplicate_pane.terminal_id = Some("other-terminal".into());
                mismatched.panes.push(duplicate_pane);
            }
            _ => {
                let mut duplicate_terminal = mismatched.panes[0].clone();
                duplicate_terminal.pane_id = "other-pane".into();
                mismatched.panes.push(duplicate_terminal);
            }
        }
        assert!(!exact_saved_pane(&mismatched, &run), "change {change}");
    }
    let location = run.attachment.as_mut().unwrap().location.as_mut().unwrap();
    location.boot_id = None;
    location.native_session_id = None;
    runtime.panes[0].native_session_id = None;
    assert!(exact_saved_pane(&runtime, &run), "Absent optional evidence is not a contradiction");
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

#[cfg(target_os = "linux")]
#[tokio::test]
async fn resume_restored_root_terminal_id_requires_retained_physical_checkout_and_launch_receipt() {
    for retained_checkout in [true, false] {
        let fixture = Fixture::new();
        let mut run = dead_root();
        let (project, checkout) = resolve_directory(&fixture.root, None, None).await.unwrap();
        run.project = Some(project);
        run.checkout = retained_checkout.then(|| checkout.clone());
        run.target = Some(DispatchTarget::ExistingSpace { workspace_id: "space".into() });
        run.setup = Some(SetupSummary {
            operation_id: None, generation: None, workspace_id: Some("space".into()),
            checkout_path: checkout.path.clone(), repository_id: None, branch: None, base: None,
            base_commit: None, ownership: Some(cockpit_protocol::projects::WorkspaceCheckoutOwnership::BorrowedDirectory),
            effects: Vec::new(), warnings: Vec::new(), project_workspace_id: None,
        });
        run.attachment.as_mut().unwrap().launch_mode = Some(AttachmentLaunchMode::DirectProcess);
        fixture.publish(&[run.clone()]);
        let shell = ForegroundPty::new();
        let mut herdr = shell.herdr(false);
        herdr.runtime.endpoint_identity = "restored-daemon-endpoint".into();
        herdr.runtime.workspaces = vec![RuntimeWorkspace {
            workspace_id: "space".into(), label: "Restored root".into(), cwd: checkout.path.clone(),
        }];
        let pane = &mut herdr.runtime.panes[0];
        pane.terminal_id = Some("new-herdr-restored-terminal".into());
        pane.native_session_id = None;
        pane.agent_name = None;
        pane.agent_kind = None;
        let resolved = resolve_resume_input(&fixture.service, &herdr, &run).await.unwrap();
        assert!(resolved.writer_exit_proof.is_none(), "Root recovery never fabricates worker closure");
        if retained_checkout {
            assert!(resolved.root_main_recovery.is_some(), "{:?}", resolved.observation_cause);
            reserve_root_once(&fixture, &run, &resolved);
        } else {
            assert!(resolved.root_main_recovery.is_none());
            assert_eq!(resolved.observation_cause.as_ref().unwrap().code, "consumer_activity_unproven");
            let before = serde_json::to_value(&run.attachment).unwrap();
            let blocked = resume_result(fixture.service.mutate_resolved(&Actor::Operator(OperatorOrigin::Browser),
                resume_request(&run), &resolved).unwrap());
            assert_eq!(blocked.resume.phase, ResumePhase::Blocked);
            assert_eq!(serde_json::to_value(fixture.service.run_for_review("fixture", &run.run_id).unwrap().attachment).unwrap(), before);
        }
        assert!(same_machine_process_running(&shell.shell.process,
            cockpit_core::process_identity::machine_id().as_deref()).unwrap(),
            "The restored borrowed shell grants liaison launch authority, never cleanup authority");
    }
}
