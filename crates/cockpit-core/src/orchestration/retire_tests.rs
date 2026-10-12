use std::{collections::VecDeque, path::PathBuf, sync::Arc};
use parking_lot::Mutex;
use cockpit_protocol::orchestration::*;
use super::{herdr::*, retire::{retire_run, ObservationBackoff}, retirement, *};

struct Fixture { root: PathBuf, service: Arc<OrchestrationService>, run: Run }
impl Fixture {
    fn new(state: RetirementState) -> Self {
        let root = std::env::temp_dir().join(format!("cockpit-retire-{}", uuid::Uuid::new_v4()));
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
        let mut run = new_run(&id, "fixture", RunKind::Worker, "Worker".into(), &id, None, None, 1);
        run.stage = RunStage::Closed;
        run.close_reason = Some(CloseReason::Accepted);
        run.cleanup_policy = Some(WorkerCleanupPolicy::OwnedWhenSafe);
        run.attachment = Some(ExecutionAttachment {
            attachment_id: uuid::Uuid::new_v4().to_string(), epoch: 1,
            state: AttachmentState::Connected, location: None, conversation: None,
            launch_mode: Some(AttachmentLaunchMode::ShellAgent),
            process: None, shell: None, machine_id: crate::process_identity::machine_id(),
            launch_attempt: 1, updated_at: now(),
        });
        run.attachment.as_mut().unwrap().location = Some(RunLocation {
            endpoint_identity: "endpoint".into(), session_id: "fixture".into(), workspace_id: "space".into(),
            tab_id: "tab".into(), pane_id: "pane".into(), terminal_id: Some("terminal".into()),
            launch_tag: "tag".into(), boot_id: None, native_session_id: None,
        });
        run.attachment.as_mut().unwrap().conversation = Some(ConversationIdentity {
            harness: "omp".into(), session_id: "conversation".into(),
        });
        run.attachment.as_mut().unwrap().process = Some(process());
        run.attachment.as_mut().unwrap().shell = Some(shell_identity());
        let mut launch = dispatch(DispatchStep::Launched);
        launch.agent_started = true;
        launch.launch_attempt = 1;
        launch.launch_tag = Some("tag".into());
        launch.endpoint_identity = Some("endpoint".into());
        run.dispatch = Some(launch);
        let source_output = SourceOutput::NonCode {
            result_message_id: "result".into(), task_revision: "reviewed-revision".into(),
            artifacts: Vec::new(), focused_checks: Vec::new(),
        };
        run.source_output = Some(source_output.clone());
        run.integration = Integration::NotApplicable {
            reason: "Accepted non-code output has no Git integration".into(),
            result_message_id: "result".into(),
        };
        run.result = Some(Report { message_id: "result".into(), kind: ReportKind::Result,
            outcome: Some(ReportOutcome::Succeeded), summary: "Complete".into(), plan: None,
            source_output: Some(source_output), at: now() });
        run.retirement = retirement::retirement_for_accepted(&run, RetirementTrigger::Accept, "reviewed-revision", &now());
        assert!(run.retirement.as_ref().unwrap().identity.is_some());
        run.retirement.as_mut().unwrap().state = state;
        let locked = service.store.lock().unwrap();
        let mut stored = locked.read().unwrap();
        stored.runs.push(run.clone());
        locked.save(&mut stored).unwrap();
        drop(locked);
        Self { root, service, run }
    }
    fn current(&self) -> Run {
        self.service.store.lock().unwrap().read().unwrap().runs.into_iter()
            .find(|r| r.run_id == self.run.run_id).unwrap()
    }
    fn change(&self, f: impl FnOnce(&mut Run)) {
        change_run(&self.service, &self.run.run_id, f);
    }
    async fn step(&self, herdr: &FakeHerdr, backoff: &ObservationBackoff) -> Result<(), InspectionError> {
        retire_run(&self.service, herdr, backoff, &self.run).await
    }
}
impl Drop for Fixture { fn drop(&mut self) { let _ = std::fs::remove_dir_all(&self.root); } }
fn change_run(service: &OrchestrationService, id: &str, f: impl FnOnce(&mut Run)) {
    let locked = service.store.lock().unwrap();
    let mut state = locked.read().unwrap();
    f(state.runs.iter_mut().find(|r| r.run_id == id).unwrap());
    locked.save(&mut state).unwrap();
}
fn process() -> NativeProcessIdentity {
    NativeProcessIdentity { pid: i32::MAX as u32, start_ticks: 1,
        kernel_boot_id: crate::process_identity::kernel_boot_id() }
}
fn waiting() -> RetirementState {
    RetirementState::Waiting { blockers: Vec::new() }
}
fn pin_direct(run: &mut Run) {
    let attachment = run.attachment.as_mut().unwrap();
    attachment.launch_mode = Some(AttachmentLaunchMode::DirectProcess);
    attachment.shell.as_mut().unwrap().process = attachment.process.clone().unwrap();
    run.retirement.as_mut().unwrap().identity = retirement::identity_for(run);
}
fn old() -> String {
    (time::OffsetDateTime::now_utc() - time::Duration::hours(1))
        .format(&time::format_description::well_known::Rfc3339).unwrap()
}
fn runtime(agent: bool) -> RuntimeView {
    RuntimeView { endpoint_identity: "endpoint".into(), boot_id: None, workspaces: vec![],
        panes: vec![RuntimePane { workspace_id: "space".into(), workspace_label: "Space".into(),
            tab_id: "tab".into(), tab_label: "tag".into(), pane_id: "pane".into(),
            terminal_id: Some("terminal".into()), native_session_id: Some("native".into()),
            agent_name: agent.then(|| "tag".into()), agent_kind: agent.then(|| "omp".into()),
            launch_pending: false, agent_status: None, state_changed_at: None }], }
}
fn absent() -> RuntimeView { let mut r = runtime(false); r.panes.clear(); r }
fn shell_identity() -> NativeShellIdentity {
    NativeShellIdentity { process: NativeProcessIdentity { pid: 123, start_ticks: 1,
            kernel_boot_id: crate::process_identity::kernel_boot_id() },
        executable_device: "1".into(), executable_inode: "2".into(), argv_digest: "a".repeat(64) }
}
fn original_shell_info() -> PaneProcessInfo {
    let shell = shell_identity();
    PaneProcessInfo { pane_id: "pane".into(), shell_pid: Some(shell.process.pid),
        foreground_pgid: Some(shell.process.pid), processes: vec![(shell.process.pid, "shell".into())],
        shell_identity: Some(shell) }
}
struct Script {
    runtimes: VecDeque<Result<RuntimeView, InspectionError>>,
    info: PaneProcessInfo,
    info_error: bool,
    infos: VecDeque<PaneProcessInfo>,
    events: Vec<&'static str>,
    close_count: usize,
    allow_close: bool,
    runtime_hook: Option<(usize, Box<dyn FnOnce() + Send>)>,
}
struct FakeHerdr { script: Mutex<Script> }
impl FakeHerdr {
    fn new(_: &Fixture, reads: Vec<RuntimeView>) -> Self {
        Self { script: Mutex::new(Script {
            runtimes: reads.into_iter().map(Ok).collect(), info: original_shell_info(), info_error: false,
            events: vec![], close_count: 0, runtime_hook: None,
            infos: VecDeque::new(),
            allow_close: false,
        }) }
    }
    fn closes(&self) -> usize { self.script.lock().close_count }
    fn runtime_calls(&self) -> usize { self.script.lock().events.iter().filter(|e| **e == "runtime").count() }
}
#[async_trait::async_trait]
impl OrchestrationHerdr for FakeHerdr {
    async fn runtime(&self, session: &str) -> Result<RuntimeView, InspectionError> {
        assert_eq!(session, "fixture");
        let mut s = self.script.lock();
        s.events.push("runtime");
        let count = s.events.iter().filter(|e| **e == "runtime").count();
        if s.runtime_hook.as_ref().is_some_and(|(at, _)| *at == count) {
            let (_, hook) = s.runtime_hook.take().unwrap();
            drop(s); hook(); s = self.script.lock();
        }
        s.runtimes.pop_front().unwrap_or_else(|| Err(InspectionError::new("offline", "No observation")))
    }
    async fn pane_process_info(&self, session: &str, endpoint: &str, pane: &str) -> Result<PaneProcessInfo, InspectionError> {
        assert_eq!((session, endpoint, pane), ("fixture", "endpoint", "pane"));
        let mut s = self.script.lock(); s.events.push("info");
        if s.info_error { Err(InspectionError::new("offline", "No process observation")) }
        else { Ok(s.infos.pop_front().unwrap_or_else(|| s.info.clone())) }
    }
    async fn close_pane(&self, session: &str, endpoint: &str, pane: &str) -> Result<(), InspectionError> {
        assert_eq!((session, endpoint, pane), ("fixture", "endpoint", "pane"));
        let mut script = self.script.lock();
        assert!(script.allow_close, "unproven physical consumers must never be cancelled");
        script.close_count += 1;
        Ok(())
    }
    async fn create_agent_tab(&self, _: &str, _: &AgentTabRequest) -> Result<RunLocation, InspectionError> {
        panic!("retirement cannot create or broadly close tabs/workspaces")
    }
}

#[tokio::test]
async fn original_shell_keeps_final_owned_pane_and_history() {
    let f = Fixture::new(waiting());
    let original = f.current();
    let h = FakeHerdr::new(&f, vec![runtime(false)]);
    let b = ObservationBackoff::default();
    f.step(&h, &b).await.unwrap();
    let retained = f.current();
    assert!(matches!(retained.retirement.as_ref().unwrap().state,
        RetirementState::Retained { reason: RetainReason::UserActivity, .. }));
    assert_eq!(serde_json::to_value(&retained.attachment).unwrap(), serde_json::to_value(&original.attachment).unwrap());
    assert_eq!(serde_json::to_value(&retained.result).unwrap(), serde_json::to_value(&original.result).unwrap());
    assert_eq!(retained.checkout_claim_id, original.checkout_claim_id);
    assert_eq!(retained.stage, RunStage::Closed);
    assert_eq!(h.closes(), 0);
    let bytes = std::fs::read(f.service.base().join("state.json")).unwrap();
    f.step(&h, &b).await.unwrap();
    assert_eq!(std::fs::read(f.service.base().join("state.json")).unwrap(), bytes);
    assert_eq!(h.closes(), 0);
}

#[tokio::test]
async fn siblings_in_other_tabs_survive_but_shared_tab_and_contamination_are_retained() {
    for shared in [false, true] {
        let f = Fixture::new(waiting());
        let mut view = runtime(false);
        let mut sibling = view.panes[0].clone();
        sibling.pane_id = "user-pane".into(); sibling.terminal_id = Some("user-terminal".into());
        sibling.tab_id = if shared { "tab" } else { "user-tab" }.into();
        view.panes.push(sibling.clone());
        let h = FakeHerdr::new(&f, vec![view]);
        f.step(&h, &ObservationBackoff::default()).await.unwrap();
        assert_eq!(h.closes(), 0);
        assert!(matches!(f.current().retirement.unwrap().state,
            RetirementState::Retained { reason, .. }
            if reason == if shared { RetainReason::SharedTab } else { RetainReason::UserActivity }));
    }
    for change in 0..5 {
        let f = Fixture::new(waiting());
        let mut contaminated = runtime(false);
        match change {
            0 => contaminated.panes[0].tab_label = "User renamed".into(),
            1 => contaminated.panes[0].tab_id = "moved".into(),
            2 => contaminated.panes[0].agent_kind = Some("new-agent".into()),
            3 => contaminated.endpoint_identity = "new-endpoint".into(),
            _ => contaminated.panes[0].launch_pending = true,
        }
        let h = FakeHerdr::new(&f, vec![contaminated]);
        f.step(&h, &ObservationBackoff::default()).await.unwrap();
        assert_eq!(h.closes(), 0);
        assert!(matches!(f.current().retirement.unwrap().state,
            RetirementState::Retained { .. }));
    }
}

#[tokio::test]
async fn foreground_process_never_authorizes_terminal_cancellation() {
    let f = Fixture::new(waiting());
    f.change(pin_direct);
    let h = FakeHerdr::new(&f, vec![runtime(false)]);
    h.script.lock().info.foreground_pgid = Some(999);
    f.step(&h, &ObservationBackoff::default()).await.unwrap();
    assert_eq!(h.closes(), 0);
    assert!(matches!(f.current().retirement.unwrap().state,
        RetirementState::Retained { reason: RetainReason::ForegroundProcess, .. }));
}

#[tokio::test]
async fn interrupted_close_observation_never_resends_the_historical_effect() {
    for seen in [false, true] {
        let f = Fixture::new(RetirementState::CloseIntent { at: now() });
        let original = f.current();
        let h = FakeHerdr::new(&f, vec![if seen { runtime(false) } else { absent() }]);
        f.step(&h, &ObservationBackoff::default()).await.unwrap();
        let current = f.current();
        if seen {
            assert!(matches!(current.retirement.unwrap().state,
                RetirementState::Unknown { phase: RetirementPhase::TerminalClose, .. }));
        } else {
            assert!(matches!(current.retirement.unwrap().state,
                RetirementState::Retired { terminal: TerminalOutcome::AbsentAfterUncertainClose, .. }));
        }
        assert_eq!(serde_json::to_value(&current.attachment).unwrap(), serde_json::to_value(&original.attachment).unwrap());
        assert_eq!(serde_json::to_value(&current.result).unwrap(), serde_json::to_value(&original.result).unwrap());
        f.step(&h, &ObservationBackoff::default()).await.unwrap();
        assert_eq!(h.closes(), 0);
    }
}

#[tokio::test]
async fn recovered_close_intent_performs_one_read_only_reclassification() {
    for seen in [0, 1, 2] {
        let f = Fixture::new(RetirementState::CloseIntent { at: now() });
        let h = FakeHerdr::new(&f, match seen { 0 => vec![absent()], 1 => vec![runtime(false)], _ => vec![] });
        f.step(&h, &ObservationBackoff::default()).await.unwrap();
        f.step(&h, &ObservationBackoff::default()).await.unwrap();
        assert_eq!(h.closes(), 0); assert_eq!(h.runtime_calls(), if seen == 0 { 1 } else { 2 });
        assert!(retirement::terminal(&f.current().retirement.unwrap().state));
    }
}

#[tokio::test]
async fn unavailable_reads_are_bounded_and_eventually_visible() {
    assert!(retirement::OBSERVATION_RETRY_SECS >= 30);
    let f = Fixture::new(waiting());
    let h = FakeHerdr::new(&f, vec![]);
    let b = ObservationBackoff::default();
    for _ in 0..1000 { f.step(&h, &b).await.unwrap(); }
    assert_eq!(h.runtime_calls(), 1);
    assert_eq!(h.closes(), 0);
    f.change(|r| r.retirement.as_mut().unwrap().created_at = old());
    f.step(&h, &ObservationBackoff::default()).await.unwrap();
    assert!(matches!(f.current().retirement.unwrap().state,
        RetirementState::Retained { reason: RetainReason::ObservationUnavailable, .. }));
}


#[tokio::test]
async fn open_descendant_runs_block_close_after_read_only_observation() {
    let f = Fixture::new(RetirementState::Waiting { blockers: vec![] });
    let mut child = new_run("child", "fixture", RunKind::Worker, "Child".into(), &f.run.run_id,
        Some(f.run.run_id.clone()), None, 1);
    child.stage = RunStage::Working;
    let locked = f.service.store.lock().unwrap();
    let mut state = locked.read().unwrap(); state.runs.push(child); locked.save(&mut state).unwrap(); drop(locked);
    let h = FakeHerdr::new(&f, vec![runtime(true)]);
    f.step(&h, &ObservationBackoff::default()).await.unwrap();
    assert_eq!(h.runtime_calls(), 1); assert_eq!(h.closes(), 0);
    assert!(matches!(f.current().retirement.unwrap().state,
        RetirementState::Waiting { blockers } if blockers.contains(&RetirementBlocker::OpenDescendantRuns)));
}


#[tokio::test]
async fn lease_and_durable_identity_change_fence_retirement_observation() {
    let f = Fixture::new(waiting());
    let h = FakeHerdr::new(&f, vec![runtime(false)]);
    let svc = Arc::clone(&f.service); let id = f.run.run_id.clone();
    h.script.lock().runtime_hook = Some((1, Box::new(move || {
        change_run(&svc, &id, |r| r.attachment.as_mut().unwrap().epoch += 1);
    })));
    f.step(&h, &ObservationBackoff::default()).await.unwrap();
    let retained = f.current().retirement.unwrap();
    assert!(matches!(retained.state,
        RetirementState::Retained { reason: RetainReason::IdentityChanged, .. }));
    f.step(&h, &ObservationBackoff::default()).await.unwrap();
    assert_eq!(f.current().retirement.unwrap(), retained);
    assert_eq!(h.closes(), 0);
    let f = Fixture::new(waiting());
    let h = FakeHerdr::new(&f, vec![runtime(false)]);
    let lease = f.service.execution_lease(&f.run.run_id).unwrap().unwrap();
    f.step(&h, &ObservationBackoff::default()).await.unwrap();
    assert_eq!(h.runtime_calls(), 0); assert_eq!(h.closes(), 0); drop(lease);
    f.step(&h, &ObservationBackoff::default()).await.unwrap();
    assert_eq!(h.closes(), 0);
    assert!(matches!(f.current().retirement.unwrap().state, RetirementState::Retained { .. }));
}


#[tokio::test]
async fn failed_close_verification_stays_unknown_across_owner_restart() {
    let f = Fixture::new(RetirementState::CloseIntent { at: now() });
    let h = FakeHerdr::new(&f, vec![]);
    f.step(&h, &ObservationBackoff::default()).await.unwrap();
    assert!(matches!(f.current().retirement.unwrap().state,
        RetirementState::Unknown { phase: RetirementPhase::TerminalClose, .. }));
    f.step(&h, &ObservationBackoff::default()).await.unwrap();
    assert_eq!(h.closes(), 0);
}

#[tokio::test]
async fn identity_change_and_replaced_retirement_queue_hint_are_fenced() {
    let f = Fixture::new(RetirementState::Waiting { blockers: vec![] });
    let h = FakeHerdr::new(&f, vec![runtime(true)]);
    h.script.lock().info.processes = vec![(process().pid, "omp".into())];
    let svc = Arc::clone(&f.service); let id = f.run.run_id.clone();
    h.script.lock().runtime_hook = Some((1, Box::new(move || {
        change_run(&svc, &id, |r| r.attachment.as_mut().unwrap().epoch += 1);
    })));
    f.step(&h, &ObservationBackoff::default()).await.unwrap();
    assert!(matches!(f.current().retirement.unwrap().state,
        RetirementState::Retained { reason: RetainReason::IdentityChanged, .. }));
    assert_eq!(h.closes(), 0);
    let f = Fixture::new(waiting());
    f.change(|r| r.retirement.as_mut().unwrap().retirement_id = uuid::Uuid::new_v4().to_string());
    let h = FakeHerdr::new(&f, vec![]);
    f.step(&h, &ObservationBackoff::default()).await.unwrap();
    assert_eq!(h.runtime_calls(), 0); assert_eq!(h.closes(), 0);
}


#[tokio::test]
async fn newly_running_descendant_during_close_preflight_cannot_be_closed() {
    let f = Fixture::new(RetirementState::Waiting { blockers: vec![] });
    let h = FakeHerdr::new(&f, vec![runtime(true)]);
    h.script.lock().info.processes = vec![(process().pid, "omp".into())];
    let svc = Arc::clone(&f.service); let id = f.run.run_id.clone();
    h.script.lock().runtime_hook = Some((1, Box::new(move || {
        let locked = svc.store.lock().unwrap();
        let mut state = locked.read().unwrap();
        let mut child = new_run("late-child", "fixture", RunKind::Worker, "Child".into(),
            &id, Some(id.clone()), None, 1);
        child.stage = RunStage::Working; state.runs.push(child);
        locked.save(&mut state).unwrap();
    })));
    f.step(&h, &ObservationBackoff::default()).await.unwrap();
    assert!(matches!(f.current().retirement.unwrap().state, RetirementState::Waiting { .. }));
    assert_eq!(h.closes(), 0);
}

#[tokio::test]
async fn missing_or_changed_shell_fingerprint_never_authorizes_close() {
    for missing in [false, true] {
        let f = Fixture::new(waiting());
        f.change(pin_direct);
        let h = FakeHerdr::new(&f, vec![runtime(false)]);
        let mut changed = original_shell_info();
        if missing { changed.shell_identity = None; }
        else { changed.shell_identity.as_mut().unwrap().argv_digest = "b".repeat(64); }
        h.script.lock().infos = vec![changed].into();
        f.step(&h, &ObservationBackoff::default()).await.unwrap();
        assert_eq!(h.closes(), 0);
        assert!(matches!(f.current().retirement.unwrap().state,
            RetirementState::Retained { reason: RetainReason::ForegroundProcess, .. }));
    }
}

#[cfg(target_os = "linux")]
#[tokio::test]
async fn real_same_pid_exec_sleep_and_exec_bash_are_not_the_original_launch_shell() {
    use tokio::io::{AsyncBufReadExt, AsyncWriteExt};
    for exec in ["exec sleep 60", "exec bash -c 'sleep 60; :'"] {
        for boundary in [1, 2] {
            let script = format!("printf 'ready\\n'; read -r gate; {exec}");
            let mut child = tokio::process::Command::new("bash").arg("-c").arg(script)
                .stdin(std::process::Stdio::piped()).stdout(std::process::Stdio::piped())
                .kill_on_drop(true).spawn().unwrap();
            let pid = child.id().unwrap();
            let mut output = tokio::io::BufReader::new(child.stdout.take().unwrap());
            let mut ready = String::new();
            tokio::time::timeout(std::time::Duration::from_secs(5), output.read_line(&mut ready))
                .await.unwrap().unwrap();
            assert_eq!(ready.trim(), "ready");
            let start = crate::process_identity::start_identity(pid as i32).unwrap();
            let baseline = crate::process_identity::executable_identity(pid as i32, start).unwrap();
            child.stdin.take().unwrap().write_all(b"go\n").await.unwrap();
            let fresh = tokio::time::timeout(std::time::Duration::from_secs(5), async {
                loop {
                    if let Ok(fresh) = crate::process_identity::executable_identity(pid as i32, start) {
                        if fresh != baseline { break fresh; }
                    }
                    tokio::time::sleep(std::time::Duration::from_millis(10)).await;
                }
            }).await.unwrap();
            assert_eq!(fresh.process, baseline.process, "exec must retain PID/start/boot identity");
            if exec.starts_with("exec bash") {
                assert_eq!(fresh.executable_device, baseline.executable_device);
                assert_eq!(fresh.executable_inode, baseline.executable_inode);
                assert_ne!(fresh.argv_digest, baseline.argv_digest);
            } else {
                assert_ne!(fresh.executable_inode, baseline.executable_inode);
            }
            let f = Fixture::new(waiting());
            f.change(|r| {
                let attachment = r.attachment.as_mut().unwrap();
                attachment.launch_mode = Some(AttachmentLaunchMode::DirectProcess);
                attachment.shell = Some(baseline.clone());
                attachment.process = Some(baseline.process.clone());
                r.retirement.as_mut().unwrap().identity = retirement::identity_for(r);
            });
            let info = |identity: NativeShellIdentity| PaneProcessInfo {
                pane_id: "pane".into(), shell_pid: Some(pid), foreground_pgid: Some(pid),
                processes: vec![(pid, "shell-or-exec".into())], shell_identity: Some(identity),
            };
            // Exercise fingerprint rejection directly, independently of which
            // stronger consumer guard rejects the actual retirement path first.
            let pinned = f.current().retirement.unwrap().identity.unwrap();
            assert!(matches!(retirement::classify_terminal(&pinned, &runtime(false), &info(fresh.clone())),
                Err(retirement::TerminalDecision::Retain(RetainReason::ForegroundProcess))));
            let h = FakeHerdr::new(&f, vec![runtime(false), runtime(false)]);
            h.script.lock().infos = if boundary == 1 { vec![info(fresh)] }
                else { vec![info(baseline), info(fresh)] }.into();
            let original = f.current();
            let result = f.step(&h, &ObservationBackoff::default()).await;
            child.kill().await.unwrap(); child.wait().await.unwrap();
            result.unwrap();
            assert_eq!(h.closes(), 0);
            assert!(matches!(f.current().retirement.unwrap().state,
                RetirementState::Retained { .. }));
            assert_eq!(serde_json::to_value(f.current().attachment).unwrap(),
                serde_json::to_value(&original.attachment).unwrap());
            assert_eq!(serde_json::to_value(f.current().result).unwrap(),
                serde_json::to_value(&original.result).unwrap());
        }
    }
}

#[cfg(target_os = "linux")]
#[tokio::test]
async fn original_shell_builtin_work_is_preserved() {
    use tokio::io::{AsyncBufReadExt, AsyncWriteExt};
    // Unchanged shell provenance does not certify absence of user activity.
    let mut child = tokio::process::Command::new("bash").arg("-c")
        .arg("printf 'ready\\n'; read -r gate; printf 'builtin-running\\n'; read -r second_gate")
        .stdin(std::process::Stdio::piped()).stdout(std::process::Stdio::piped())
        .kill_on_drop(true).spawn().unwrap();
    let pid = child.id().unwrap();
    let mut output = tokio::io::BufReader::new(child.stdout.take().unwrap());
    let mut input = child.stdin.take().unwrap();
    let mut line = String::new();
    tokio::time::timeout(std::time::Duration::from_secs(5), output.read_line(&mut line))
        .await.unwrap().unwrap();
    assert_eq!(line.trim(), "ready");
    let start = crate::process_identity::start_identity(pid as i32).unwrap();
    let baseline = crate::process_identity::executable_identity(pid as i32, start).unwrap();
    input.write_all(b"go\n").await.unwrap();
    line.clear();
    tokio::time::timeout(std::time::Duration::from_secs(5), output.read_line(&mut line))
        .await.unwrap().unwrap();
    assert_eq!(line.trim(), "builtin-running");
    let fresh = crate::process_identity::executable_identity(pid as i32, start).unwrap();
    assert_eq!(fresh, baseline);
    let f = Fixture::new(waiting());
    f.change(|r| {
        r.attachment.as_mut().unwrap().shell = Some(baseline.clone());
        r.retirement.as_mut().unwrap().identity.as_mut().unwrap().shell = Some(baseline);
    });
    let mut view = runtime(false);
    let mut sibling = view.panes[0].clone();
    sibling.pane_id = "user-pane".into(); sibling.terminal_id = Some("user-terminal".into());
    sibling.tab_id = "user-tab".into(); view.panes.push(sibling.clone());
    let mut after = absent(); after.panes.push(sibling);
    let h = FakeHerdr::new(&f, vec![view.clone(), view, after]);
    h.script.lock().info = PaneProcessInfo { pane_id: "pane".into(), shell_pid: Some(pid),
        foreground_pgid: Some(pid), processes: vec![(pid, "bash".into())], shell_identity: Some(fresh) };
    let result = f.step(&h, &ObservationBackoff::default()).await;
    child.kill().await.unwrap(); child.wait().await.unwrap();
    result.unwrap();
    assert_eq!(h.closes(), 0);
    assert!(matches!(f.current().retirement.unwrap().state,
        RetirementState::Retained { reason: RetainReason::UserActivity, .. }));
}



#[tokio::test]
async fn unknown_close_reconciles_later_absence_without_resending_close() {
    let f = Fixture::new(RetirementState::Unknown {
        at: old(), phase: RetirementPhase::TerminalClose, detail: "close outcome unknown".into(),
    });
    let h = FakeHerdr::new(&f, vec![absent()]);
    f.step(&h, &ObservationBackoff::default()).await.unwrap();
    assert!(matches!(f.current().retirement.unwrap().state,
        RetirementState::Retired { terminal: TerminalOutcome::AbsentAfterUncertainClose, .. }));
    assert_eq!(h.runtime_calls(), 1);
    assert_eq!(h.closes(), 0);
}



#[tokio::test]
async fn historical_unknown_close_retains_pinned_identity_across_attachment_refresh() {
    let f = Fixture::new(RetirementState::Unknown {
        at: old(), phase: RetirementPhase::TerminalClose, detail: "historical unknown close".into(),
    });
    let pinned = f.current().retirement.unwrap().identity.unwrap();
    f.change(|run| {
        let attachment = run.attachment.as_mut().unwrap();
        attachment.attachment_id = uuid::Uuid::new_v4().to_string();
        attachment.epoch += 1;
        attachment.location.as_mut().unwrap().pane_id = "replacement-pane".into();
    });
    let h = FakeHerdr::new(&f, vec![absent()]);
    f.step(&h, &ObservationBackoff::default()).await.unwrap();
    let current = f.current();
    let receipt = current.retirement.unwrap();
    assert_eq!(receipt.identity.unwrap(), pinned);
    assert!(matches!(receipt.state, RetirementState::Retired { terminal: TerminalOutcome::AbsentAfterUncertainClose, .. }));
    assert_eq!(current.attachment.unwrap().epoch, 2);
    assert_eq!(h.closes(), 0);
}


#[tokio::test]
async fn main_exit_and_empty_foreground_do_not_prove_unknown_pty_consumers_drained() {
    let f = Fixture::new(waiting());
    f.change(pin_direct);
    let original = f.current();
    let h = FakeHerdr::new(&f, vec![runtime(false), runtime(false), absent()]);
    {
        let mut script = h.script.lock();
        script.allow_close = true;
        script.info = PaneProcessInfo {
            pane_id: "pane".into(), shell_pid: Some(process().pid), foreground_pgid: None,
            processes: Vec::new(), shell_identity: None,
        };
    }
    f.step(&h, &ObservationBackoff::default()).await.unwrap();
    assert_eq!(h.closes(), 0);
    assert!(matches!(f.current().retirement.unwrap().state,
        RetirementState::Retained { reason: RetainReason::NativeProcessUnverifiable, stopped: true, .. }));
    let current = f.current();
    assert_eq!(serde_json::to_value(current.attachment).unwrap(), serde_json::to_value(original.attachment).unwrap());
    assert_eq!(serde_json::to_value(current.result).unwrap(), serde_json::to_value(original.result).unwrap());
    assert_eq!(current.checkout_claim_id, original.checkout_claim_id);
    f.step(&h, &ObservationBackoff::default()).await.unwrap();
    assert_eq!(h.closes(), 0);
}

#[tokio::test]
async fn registered_chat_without_process_proof_retains_its_physical_pane() {
    let f = Fixture::new(waiting());
    f.change(|run| {
        let attachment = run.attachment.as_mut().unwrap();
        attachment.process = None;
        attachment.shell = None;
        run.retirement.as_mut().unwrap().identity = retirement::identity_for(run);
    });
    let original = f.current();
    let h = FakeHerdr::new(&f, vec![runtime(false)]);
    f.step(&h, &ObservationBackoff::default()).await.unwrap();
    assert_eq!(h.closes(), 0);
    assert!(matches!(f.current().retirement.unwrap().state,
        RetirementState::Retained { reason: RetainReason::NativeProcessUnverifiable, stopped: false, .. }));
    assert_eq!(serde_json::to_value(f.current().attachment).unwrap(), serde_json::to_value(original.attachment).unwrap());
    assert_eq!(serde_json::to_value(f.current().result).unwrap(), serde_json::to_value(original.result).unwrap());
}
