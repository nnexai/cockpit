use std::{collections::VecDeque, path::PathBuf, sync::Arc};
use parking_lot::Mutex;
use cockpit_protocol::orchestration::*;
use super::{herdr::*, retire::retire_run, retirement, *};

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
            process_scope: None,
            launch_attempt: 1, updated_at: now(),
        });
        run.attachment.as_mut().unwrap().location = Some(RunLocation {
            endpoint_identity: "endpoint".into(), session_id: "fixture".into(), workspace_id: "space".into(),
            tab_id: "tab".into(), pane_id: "pane".into(), terminal_id: Some("terminal".into()),
            launch_tag: "tag".into(), boot_id: legacy_test_boot(), peer_process: test_peer(), native_session_id: None,
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
    fn replacement(&mut self) {
        self.change(pin_direct);
        let locked = self.service.store.lock().unwrap();
        let mut state = locked.read().unwrap();
        let index = run_index(&state, &self.run.run_id).unwrap();
        let root_id = id();
        let task_id = id();
        let successor_id = id();
        let operation_id = id();
        let run = &mut state.runs[index];
        run.root_id = root_id.clone();
        run.parent_run_id = Some(root_id.clone());
        run.task_id = Some(task_id.clone());
        run.assignment_generation = Some(1);
        run.close_reason = Some(CloseReason::Superseded);
        run.result = None;
        run.source_output = None;
        run.retirement = retirement::retirement_for_replacement(run, ReplacementRetirementAuthority {
            operation_id: operation_id.clone(), previous_run_id: run.run_id.clone(),
            previous_generation: 1, replacement_run_id: successor_id.clone(),
        }, &now());
        let old_id = run.run_id.clone();
        let mut root = new_run(&root_id, "fixture", RunKind::Supervisor, "root".into(), &root_id, None, None, 1);
        root.stage = RunStage::Active;
        let mut successor = new_run(&successor_id, "fixture", RunKind::Worker, "successor".into(),
            &root_id, Some(root_id.clone()), Some(task_id.clone()), 2);
        successor.assignment_generation = Some(2);
        state.runs.push(root);
        state.runs.push(successor);
        state.task_ownership.push(TaskOwnership { root_id: root_id.clone(), task_id, generation: 2, owner: None, schedule: None });
        state.operations.push(super::store::DurableOperationRecord { operation_id: operation_id.clone(),
            actor_run_id: Some(root_id.clone()), principal_key: "root".into(), root_id,
            kind: super::store::DurableOperationKind::WorkerReplace, request_digest: "committed root replacement".into(),
            run_id: Some(successor_id.clone()), resume_operation_id: None,
            result: Some(OrchestrationActionResult::WorkerReplaced { start: WorkerStartResult {
                operation_id, run_id: successor_id, attempt: 2, duplicate: false },
                previous_run_id: old_id, assignment_generation: 2,
                placement: ReplacementPlacement::IsolatedDirectory { directory: "/isolated/successor".into() } }),
            effect_receipts: Vec::new(), created_at: now(), updated_at: now() });
        self.run = state.runs[index].clone();
        locked.save(&mut state).unwrap();
    }
    async fn step(&self, herdr: &FakeHerdr) -> Result<(), InspectionError> {
        retire_run(&self.service, herdr, &self.run).await
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
    let pid = std::process::id() as i32;
    let start = crate::process_identity::start_identity(pid).unwrap();
    let shell = crate::process_identity::executable_identity(pid, start).unwrap();
    attachment.process = Some(shell.process.clone());
    attachment.shell = Some(shell);
    run.retirement.as_mut().unwrap().identity = retirement::identity_for(run);
}
fn old() -> String {
    (time::OffsetDateTime::now_utc() - time::Duration::hours(1))
        .format(&time::format_description::well_known::Rfc3339).unwrap()
}
fn test_peer() -> Option<NativeProcessIdentity> {
    let pid = std::process::id() as i32;
    Some(NativeProcessIdentity { pid: pid as u32, start_ticks: crate::process_identity::start_identity(pid)?,
        kernel_boot_id: Some(crate::process_identity::kernel_boot_id()?) })
}
fn legacy_test_boot() -> Option<String> {
    if test_peer().is_some() { None } else { Some("legacy-test-herdr-boot".into()) }
}
fn runtime(agent: bool) -> RuntimeView {
    RuntimeView { endpoint_identity: "endpoint".into(), boot_id: legacy_test_boot(), peer_process: test_peer(), focused_workspace_id: None, workspaces: vec![],
        panes: vec![RuntimePane { workspace_id: "space".into(), workspace_label: "Space".into(),
            tab_id: "tab".into(), tab_label: "tag".into(), pane_id: "pane".into(),
            terminal_id: Some("terminal".into()), native_session_id: None,
            native_session_file: None,
            cwd: None,
            agent_name: agent.then(|| "tag".into()), agent_kind: agent.then(|| "omp".into()),
            launch_pending: false, agent_status: agent.then(|| "idle".into()), state_changed_at: None }], }
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
    close_error: Option<&'static str>,
    runtime_hook: Option<(usize, Box<dyn FnOnce() + Send>)>,
}
struct FakeHerdr { script: Mutex<Script> }
impl FakeHerdr {
    fn new(f: &Fixture, reads: Vec<RuntimeView>) -> Self {
        let run = f.current();
        let shell = run.attachment.as_ref().and_then(|a| a.shell.clone());
        let info = shell.map(|shell| PaneProcessInfo { pane_id: "pane".into(),
            shell_pid: Some(shell.process.pid), foreground_pgid: Some(shell.process.pid),
            processes: vec![(shell.process.pid, "agent".into())], shell_identity: Some(shell) })
            .unwrap_or_else(original_shell_info);
        Self { script: Mutex::new(Script {
            runtimes: reads.into_iter().map(Ok).collect(), info, info_error: false,
            events: vec![], close_count: 0, runtime_hook: None,
            infos: VecDeque::new(),
            allow_close: false, close_error: None,
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
        assert!(script.allow_close, "only the exact idle owned direct-process pane may close");
        script.close_count += 1;
        script.events.push("close");
        match script.close_error {
            Some(code) => Err(InspectionError::new(code, "Injected close failure")),
            None => Ok(()),
        }
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
    
    f.step(&h).await.unwrap();
    let retained = f.current();
    assert!(matches!(retained.retirement.as_ref().unwrap().state,
        RetirementState::Retained { reason: RetainReason::UserActivity, .. }));
    assert_eq!(serde_json::to_value(&retained.attachment).unwrap(), serde_json::to_value(&original.attachment).unwrap());
    assert_eq!(serde_json::to_value(&retained.result).unwrap(), serde_json::to_value(&original.result).unwrap());
    assert_eq!(retained.checkout_claim_id, original.checkout_claim_id);
    assert_eq!(retained.stage, RunStage::Closed);
    assert_eq!(h.closes(), 0);
    let bytes = std::fs::read(f.service.base().join("state.json")).unwrap();
    f.step(&h).await.unwrap();
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
        f.step(&h).await.unwrap();
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
        f.step(&h).await.unwrap();
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
    f.step(&h).await.unwrap();
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
        f.step(&h).await.unwrap();
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
        f.step(&h).await.unwrap();
        assert_eq!(h.closes(), 0);
    }
}

#[tokio::test]
async fn recovered_close_intent_performs_one_read_only_reclassification() {
    for seen in [0, 1, 2] {
        let f = Fixture::new(RetirementState::CloseIntent { at: now() });
        let h = FakeHerdr::new(&f, match seen { 0 => vec![absent()], 1 => vec![runtime(false)], _ => vec![] });
        
        f.step(&h).await.unwrap();
        f.step(&h).await.unwrap();
        assert_eq!(h.closes(), 0);
        assert_eq!(h.runtime_calls(), if seen == 0 { 1 } else { 2 });
        assert!(retirement::terminal(&f.current().retirement.unwrap().state));
    }
}

#[tokio::test]
async fn unavailable_observation_is_concrete_and_retries_on_next_event_without_expiry() {
    let f = Fixture::new(waiting());
    let h = FakeHerdr::new(&f, vec![]);
    f.step(&h).await.unwrap();
    let before = std::fs::read(f.service.base().join("state.json")).unwrap();
    f.step(&h).await.unwrap();
    assert_eq!(h.runtime_calls(), 2);
    assert_eq!(h.closes(), 0);
    assert_eq!(std::fs::read(f.service.base().join("state.json")).unwrap(), before);
    assert!(matches!(f.current().retirement.unwrap().state,
        RetirementState::Retained { reason: RetainReason::ObservationUnavailable, .. }));
    f.change(|r| r.retirement.as_mut().unwrap().created_at = old());
    h.script.lock().runtimes = vec![absent()].into_iter().map(Ok).collect();
    f.step(&h).await.unwrap();
    assert!(matches!(f.current().retirement.unwrap().state,
        RetirementState::Retired { terminal: TerminalOutcome::AlreadyAbsent, .. }));
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
    f.step(&h).await.unwrap();
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
    f.step(&h).await.unwrap();
    let retained = f.current().retirement.unwrap();
    assert!(matches!(retained.state,
        RetirementState::Retained { reason: RetainReason::IdentityChanged, .. }));
    f.step(&h).await.unwrap();
    assert_eq!(f.current().retirement.unwrap(), retained);
    assert_eq!(h.closes(), 0);
    let f = Fixture::new(waiting());
    let h = FakeHerdr::new(&f, vec![runtime(false)]);
    let lease = f.service.execution_lease(&f.run.run_id).unwrap().unwrap();
    f.step(&h).await.unwrap();
    assert_eq!(h.runtime_calls(), 0); assert_eq!(h.closes(), 0); drop(lease);
    f.step(&h).await.unwrap();
    assert_eq!(h.closes(), 0);
    assert!(matches!(f.current().retirement.unwrap().state, RetirementState::Retained { .. }));
}


#[tokio::test]
async fn failed_close_verification_stays_unknown_across_owner_restart() {
    let f = Fixture::new(RetirementState::CloseIntent { at: now() });
    let h = FakeHerdr::new(&f, vec![]);
    f.step(&h).await.unwrap();
    assert!(matches!(f.current().retirement.unwrap().state,
        RetirementState::Unknown { phase: RetirementPhase::TerminalClose, .. }));
    f.step(&h).await.unwrap();
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
    f.step(&h).await.unwrap();
    assert!(matches!(f.current().retirement.unwrap().state,
        RetirementState::Retained { reason: RetainReason::IdentityChanged, .. }));
    assert_eq!(h.closes(), 0);
    let f = Fixture::new(waiting());
    f.change(|r| r.retirement.as_mut().unwrap().retirement_id = uuid::Uuid::new_v4().to_string());
    let h = FakeHerdr::new(&f, vec![]);
    f.step(&h).await.unwrap();
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
    f.step(&h).await.unwrap();
    assert!(matches!(f.current().retirement.unwrap().state, RetirementState::Waiting { .. }));
    assert_eq!(h.closes(), 0);
}

#[tokio::test]
async fn missing_or_changed_shell_fingerprint_never_authorizes_close() {
    for missing in [false, true] {
        let f = Fixture::new(waiting());
        f.change(pin_direct);
        let h = FakeHerdr::new(&f, vec![runtime(true)]);
        let mut changed = h.script.lock().info.clone();
        if missing { changed.shell_identity = None; }
        else { changed.shell_identity.as_mut().unwrap().argv_digest = "b".repeat(64); }
        h.script.lock().infos = vec![changed].into();
        f.step(&h).await.unwrap();
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
            // Exercise fingerprint rejection directly and at both preflights.
            let pinned = f.current().retirement.unwrap().identity.unwrap();
            assert!(matches!(retirement::classify_terminal(&pinned, &runtime(true), &info(fresh.clone())),
                Err(retirement::TerminalDecision::Retain(RetainReason::ForegroundProcess))));
            let h = FakeHerdr::new(&f, vec![runtime(true), runtime(true)]);
            h.script.lock().infos = if boundary == 1 { vec![info(fresh)] }
                else { vec![info(baseline), info(fresh)] }.into();
            let original = f.current();
            let result = f.step(&h).await;
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
    let result = f.step(&h).await;
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
    f.step(&h).await.unwrap();
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
    f.step(&h).await.unwrap();
    let current = f.current();
    let receipt = current.retirement.unwrap();
    assert_eq!(receipt.identity.unwrap(), pinned);
    assert!(matches!(receipt.state, RetirementState::Retired { terminal: TerminalOutcome::AbsentAfterUncertainClose, .. }));
    assert_eq!(current.attachment.unwrap().epoch, 2);
    assert_eq!(h.closes(), 0);
}


#[tokio::test]
async fn exited_main_never_authorizes_close_without_live_process_proof() {
    let f = Fixture::new(waiting());
    f.change(|run| {
        pin_direct(run);
        let attachment = run.attachment.as_mut().unwrap();
        attachment.process = Some(process());
        attachment.shell.as_mut().unwrap().process = process();
        run.retirement.as_mut().unwrap().identity = retirement::identity_for(run);
    });
    let original = f.current();
    let h = FakeHerdr::new(&f, vec![runtime(false), runtime(false), absent()]);
    {
        let mut script = h.script.lock();
        script.allow_close = true;
        let shell = f.current().attachment.unwrap().shell.unwrap();
        script.info = PaneProcessInfo {
            pane_id: "pane".into(), shell_pid: Some(process().pid), foreground_pgid: Some(process().pid),
            processes: vec![(process().pid, "exited agent".into())], shell_identity: Some(shell),
        };
    }
    f.step(&h).await.unwrap();
    assert_eq!(h.closes(), 0);
    assert!(matches!(f.current().retirement.unwrap().state,
        RetirementState::Retained { reason: RetainReason::NativeProcessUnverifiable, stopped: true, .. }));
    let current = f.current();
    assert_eq!(serde_json::to_value(current.attachment).unwrap(), serde_json::to_value(original.attachment).unwrap());
    assert_eq!(serde_json::to_value(current.result).unwrap(), serde_json::to_value(original.result).unwrap());
    assert_eq!(current.checkout_claim_id, original.checkout_claim_id);
    f.step(&h).await.unwrap();
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
    f.step(&h).await.unwrap();
    assert_eq!(h.closes(), 0);
    assert!(matches!(f.current().retirement.unwrap().state,
        RetirementState::Retained { reason: RetainReason::NativeProcessUnverifiable, stopped: false, .. }));
    assert_eq!(serde_json::to_value(f.current().attachment).unwrap(), serde_json::to_value(original.attachment).unwrap());
    assert_eq!(serde_json::to_value(f.current().result).unwrap(), serde_json::to_value(original.result).unwrap());
}

fn add_claim(f: &Fixture) {
    let locked = f.service.store.lock().unwrap();
    let mut state = locked.read().unwrap();
    let run = state.runs.iter().find(|r| r.run_id == f.run.run_id).unwrap();
    let attachment = run.attachment.as_ref().unwrap();
    let holder = claims::AttachmentReference { run_id: run.run_id.clone(),
        attachment_id: attachment.attachment_id.clone(), attachment_epoch: attachment.epoch };
    let (_, checkout) = project_identity::resolve_owned_directory(&f.root).unwrap();
    claims::reserve_claim(&mut state, &checkout, &holder, &id()).unwrap();
    locked.save(&mut state).unwrap();
}

#[tokio::test]
async fn accepted_alive_idle_worker_closes_only_exact_owned_pane_once_and_preserves_claims() {
    for old_retention in [false, true] {
        for status in ["idle", "done"] {
            let f = Fixture::new(if old_retention {
                RetirementState::Retained { at: old(), reason: RetainReason::NativeProcessUnverifiable, stopped: false }
            } else { waiting() });
            f.change(pin_direct);
            add_claim(&f);
            let original = f.current();
            let before = f.service.store.lock().unwrap().read().unwrap();
            let recorded = original.retirement.as_ref().unwrap();
            assert!(super::retire::exact_running(recorded.identity.as_ref().unwrap().process.as_ref().unwrap(),
                crate::process_identity::kernel_boot_id().as_deref()).unwrap());
            let mut view = runtime(true);
            view.panes[0].agent_status = Some(status.into());
            if old_retention { view.panes[0].native_session_id = Some("conversation".into()); }
            let mut sibling = view.panes[0].clone();
            sibling.tab_id = "unrelated-tab".into();
            sibling.pane_id = "unrelated-pane".into();
            sibling.terminal_id = Some("unrelated-terminal".into());
            view.panes.push(sibling.clone());
            let mut after = absent();
            after.panes.push(sibling);
            let h = FakeHerdr::new(&f, vec![view.clone(), view, after]);
            h.script.lock().allow_close = true;
            let svc = Arc::clone(&f.service);
            let run_id = f.run.run_id.clone();
            h.script.lock().runtime_hook = Some((2, Box::new(move || {
                let current = svc.run_for_review("fixture", &run_id).unwrap();
                assert!(matches!(current.retirement.unwrap().state, RetirementState::CloseIntent { .. }));
            })));
            
            f.step(&h).await.unwrap();
            let current = f.current();
            let record = current.retirement.as_ref().unwrap();
            assert_eq!(record.retirement_id, recorded.retirement_id);
            assert_eq!(record.identity, recorded.identity);
            assert!(matches!(record.state, RetirementState::Retired { terminal: TerminalOutcome::ClosedByCockpit, .. }));
            assert_eq!(h.closes(), 1);
            assert_eq!(h.script.lock().events, vec!["runtime", "info", "runtime", "info", "close", "runtime"]);
            let after = f.service.store.lock().unwrap().read().unwrap();
            assert_eq!(serde_json::to_value(&after.checkout_claims).unwrap(), serde_json::to_value(&before.checkout_claims).unwrap());
            assert_eq!(serde_json::to_value(&current.attachment).unwrap(), serde_json::to_value(&original.attachment).unwrap());
            assert_eq!(serde_json::to_value(&current.result).unwrap(), serde_json::to_value(&original.result).unwrap());
            assert_eq!(current.checkout_claim_id, original.checkout_claim_id);
            assert_eq!(current.stage, RunStage::Closed);
            f.step(&h).await.unwrap();
            assert_eq!(h.closes(), 1);
        }
    }
}

#[tokio::test]
async fn busy_pending_and_unclassified_workers_wait_without_repetitive_writes_then_idle_closes() {
    for busy in 0..5 {
        let f = Fixture::new(waiting());
        f.change(pin_direct);
        let mut view = runtime(true);
        match busy {
            0 => view.panes[0].agent_status = Some("working".into()),
            1 => view.panes[0].agent_status = None,
            2 => view.panes[0].agent_status = Some("blocked".into()),
            3 => view.panes[0].launch_pending = true,
            _ => view.panes[0].agent_kind = None,
        }
        let h = FakeHerdr::new(&f, vec![view, runtime(true), runtime(true), absent()]);
        h.script.lock().allow_close = true;
        let before = std::fs::read(f.service.base().join("state.json")).unwrap();
        
        f.step(&h).await.unwrap();
        assert_eq!(f.current().retirement.unwrap().state, waiting());
        assert_eq!(h.runtime_calls(), 1);
        assert_eq!(h.closes(), 0);
        assert_eq!(std::fs::read(f.service.base().join("state.json")).unwrap(), before);
        // The next pane event observes idle immediately, without a 30s delay.
        f.step(&h).await.unwrap();
        assert_eq!(h.closes(), 1);
        assert!(matches!(f.current().retirement.unwrap().state, RetirementState::Retired { .. }));
    }
}

#[tokio::test]
async fn final_busy_or_unavailable_preflight_withdraws_only_unsubmitted_intent() {
    for unavailable in [false, true] {
        let f = Fixture::new(waiting());
        f.change(pin_direct);
        let mut busy = runtime(true);
        busy.panes[0].agent_status = Some("working".into());
        let h = FakeHerdr::new(&f, if unavailable { vec![runtime(true)] } else { vec![runtime(true), busy] });
        
        f.step(&h).await.unwrap();
        assert_eq!(h.closes(), 0);
        assert_eq!(f.current().retirement.unwrap().state, waiting());
        f.step(&h).await.unwrap();
        assert!(matches!(f.current().retirement.unwrap().state,
            RetirementState::Retained { reason: RetainReason::ObservationUnavailable, .. }));
        let saved = std::fs::read(f.service.base().join("state.json")).unwrap();
        f.step(&h).await.unwrap();
        assert_eq!(std::fs::read(f.service.base().join("state.json")).unwrap(), saved);
        h.script.lock().runtimes = vec![runtime(true), runtime(true), absent()].into_iter().map(Ok).collect();
        h.script.lock().allow_close = true;
        f.step(&h).await.unwrap();
        assert_eq!(h.closes(), 1);
    }
}

#[tokio::test]
async fn live_preflight_rejects_all_location_process_and_conversation_replacements_at_both_boundaries() {
    for boundary in [1, 2] {
        for changed in 0..12 {
            let f = Fixture::new(waiting());
            f.change(pin_direct);
            let mut bad = runtime(true);
            let mut info = FakeHerdr::new(&f, vec![]).script.lock().info.clone();
            match changed {
                0 => bad.endpoint_identity = "other".into(),
                1 => bad.panes[0].tab_label = "renamed".into(),
                2 => {
                    let mut sibling = bad.panes[0].clone();
                    sibling.pane_id = "sibling".into(); sibling.terminal_id = Some("sibling-terminal".into());
                    bad.panes.push(sibling);
                }
                3 => bad.panes[0].terminal_id = Some("replacement".into()),
                4 => bad.panes[0].workspace_id = "other".into(),
                5 => bad.panes[0].tab_id = "other".into(),
                6 => bad.panes[0].agent_kind = Some("foreign".into()),
                7 => bad.panes[0].native_session_id = Some("foreign-chat".into()),
                8 => info.shell_identity.as_mut().unwrap().process.start_ticks += 1,
                9 => info.shell_identity.as_mut().unwrap().executable_inode = "foreign".into(),
                10 => info.shell_identity.as_mut().unwrap().argv_digest = "b".repeat(64),
                _ => info.processes.push((999, "replacement foreground".into())),
            }
            let h = FakeHerdr::new(&f, if boundary == 1 { vec![bad] } else { vec![runtime(true), bad] });
            let original_info = h.script.lock().info.clone();
            h.script.lock().infos = if boundary == 1 { vec![info] } else { vec![original_info, info] }.into();
            f.step(&h).await.unwrap();
            assert_eq!(h.closes(), 0, "replacement {changed} at boundary {boundary}");
            assert!(matches!(f.current().retirement.unwrap().state, RetirementState::Retained { .. }));
        }
    }
}

#[tokio::test]
async fn accepted_result_and_run_identity_cas_fence_the_new_close_effect() {
    for boundary in [1, 2] {
        for changed in 0..6 {
            let f = Fixture::new(waiting());
            f.change(pin_direct);
            let h = FakeHerdr::new(&f, vec![runtime(true), runtime(true)]);
            let svc = Arc::clone(&f.service); let run_id = f.run.run_id.clone();
            h.script.lock().runtime_hook = Some((boundary, Box::new(move || {
                change_run(&svc, &run_id, |run| match changed {
                    0 => run.attachment.as_mut().unwrap().epoch += 1,
                    1 => run.result.as_mut().unwrap().message_id = "not-accepted".into(),
                    2 => run.result.as_mut().unwrap().outcome = Some(ReportOutcome::Failed),
                    3 => run.attachment.as_mut().unwrap().process.as_mut().unwrap().start_ticks += 1,
                    4 => run.attachment.as_mut().unwrap().location.as_mut().unwrap().session_id = "foreign".into(),
                    _ => run.close_reason = Some(CloseReason::Cancelled),
                });
            })));
            f.step(&h).await.unwrap();
            assert_eq!(h.closes(), 0);
            assert!(matches!(f.current().retirement.unwrap().state,
                RetirementState::Retained { reason: RetainReason::IdentityChanged, .. }));
        }
    }
}

#[tokio::test]
async fn live_close_requires_fresh_absence_and_failed_or_unknown_effects_never_replay() {
    for error in [None, Some("herdr_outcome_unknown"), Some("confirmation_required")] {
        let f = Fixture::new(waiting());
        f.change(pin_direct);
        let h = FakeHerdr::new(&f, vec![runtime(true), runtime(true), runtime(true), absent()]);
        { let mut s = h.script.lock(); s.allow_close = true; s.close_error = error; }
        f.step(&h).await.unwrap();
        assert_eq!(h.closes(), 1);
        let state = f.current().retirement.unwrap().state;
        if error == Some("confirmation_required") {
            assert!(matches!(state, RetirementState::Retained { reason: RetainReason::HerdrRefused, stopped: false, .. }));
        } else {
            assert!(matches!(state, RetirementState::Unknown { .. }));
            f.step(&h).await.unwrap();
            assert!(matches!(f.current().retirement.unwrap().state,
                RetirementState::Retired { terminal: TerminalOutcome::AbsentAfterUncertainClose, .. }));
        }
        assert_eq!(h.closes(), 1);
    }
}

#[tokio::test]
async fn primary_checkout_guard_refusal_retains_exact_worker_and_never_replays_close() {
    let f = Fixture::new(waiting());
    f.change(pin_direct);
    let original = f.current();
    let mut snapshot = runtime(true);
    let mut foreign = snapshot.panes[0].clone();
    foreign.workspace_id = "foreign-space".into();
    foreign.tab_id = "foreign-tab".into();
    foreign.pane_id = "foreign-pane".into();
    foreign.terminal_id = Some("foreign-terminal".into());
    snapshot.panes.push(foreign);
    let h = FakeHerdr::new(&f, vec![snapshot.clone(), snapshot.clone(), snapshot.clone()]);
    {
        let mut script = h.script.lock();
        script.allow_close = true;
        script.close_error = Some("foreign_space_group_close_risk");
    }
    f.step(&h).await.unwrap();
    let retained = f.current();
    assert!(matches!(retained.retirement.as_ref().unwrap().state,
        RetirementState::Retained { reason: RetainReason::PrimaryCheckoutSpace, stopped: false, .. }));
    assert_eq!(serde_json::to_value(&retained.attachment).unwrap(), serde_json::to_value(&original.attachment).unwrap());
    assert_eq!(serde_json::to_value(&retained.result).unwrap(), serde_json::to_value(&original.result).unwrap());
    assert_eq!(retained.checkout_claim_id, original.checkout_claim_id);
    assert!(f.service.retirement_queue().unwrap().is_empty());
    let reads = h.runtime_calls();
    f.step(&h).await.unwrap();
    assert_eq!(h.closes(), 1);
    assert_eq!(h.runtime_calls(), reads);
    assert_eq!(serde_json::to_value(f.current()).unwrap(), serde_json::to_value(retained).unwrap());
}

#[tokio::test]
async fn descendant_created_at_final_preflight_defers_without_submitting_close() {
    let f = Fixture::new(waiting());
    f.change(pin_direct);
    let h = FakeHerdr::new(&f, vec![runtime(true), runtime(true)]);
    let svc = Arc::clone(&f.service); let run_id = f.run.run_id.clone();
    h.script.lock().runtime_hook = Some((2, Box::new(move || {
        let locked = svc.store.lock().unwrap();
        let mut state = locked.read().unwrap();
        let mut child = new_run("late-child", "fixture", RunKind::Worker, "Child".into(),
            &run_id, Some(run_id.clone()), None, 1);
        child.stage = RunStage::Working;
        state.runs.push(child);
        locked.save(&mut state).unwrap();
    })));
    f.step(&h).await.unwrap();
    assert_eq!(h.closes(), 0);
    assert_eq!(f.current().retirement.unwrap().state,
        RetirementState::Waiting { blockers: vec![RetirementBlocker::OpenDescendantRuns] });
}

#[tokio::test]
async fn changed_accepted_receipt_during_observation_cannot_persist_a_new_close_intent() {
    let f = Fixture::new(waiting());
    f.change(pin_direct);
    let h = FakeHerdr::new(&f, vec![runtime(true)]);
    let svc = Arc::clone(&f.service); let run_id = f.run.run_id.clone();
    h.script.lock().runtime_hook = Some((1, Box::new(move || {
        change_run(&svc, &run_id, |run| {
            run.result.as_mut().unwrap().message_id = "new-result".into();
            run.retirement.as_mut().unwrap().result_message_id = Some("new-result".into());
        });
    })));
    assert_eq!(f.step(&h).await.unwrap_err().code, "retirement_state_changed");
    assert_eq!(h.closes(), 0);
    assert_eq!(f.current().retirement.unwrap().state, waiting());
}

#[tokio::test]
async fn old_process_retention_reopens_only_with_fresh_unchanged_accepted_worker_proof() {
    for changed in 0..5 {
        let f = Fixture::new(RetirementState::Retained {
            at: old(), reason: RetainReason::NativeProcessUnverifiable, stopped: false,
        });
        f.change(pin_direct);
        let mut view = runtime(true);
        match changed {
            0 => view.panes[0].tab_label = "user-renamed".into(),
            1 => {
                let mut other = view.panes[0].clone();
                other.pane_id = "shared-pane".into(); other.terminal_id = Some("shared-terminal".into());
                view.panes.push(other);
            }
            2 => f.change(|run| run.attachment.as_mut().unwrap().epoch += 1),
            3 => f.change(|run| run.close_reason = Some(CloseReason::Cancelled)),
            _ => f.change(|run| run.cleanup_policy = None),
        }
        let before = std::fs::read(f.service.base().join("state.json")).unwrap();
        let h = FakeHerdr::new(&f, vec![view]);
        f.step(&h).await.unwrap();
        assert_eq!(h.closes(), 0);
        assert_eq!(std::fs::read(f.service.base().join("state.json")).unwrap(), before);
    }
}

#[tokio::test]
async fn failed_post_close_snapshot_is_unknown_and_owner_restart_never_replays() {
    let f = Fixture::new(waiting());
    f.change(pin_direct);
    let h = FakeHerdr::new(&f, vec![runtime(true), runtime(true)]);
    h.script.lock().allow_close = true;
    f.step(&h).await.unwrap();
    assert_eq!(h.closes(), 1);
    assert!(matches!(f.current().retirement.unwrap().state, RetirementState::Unknown { .. }));
    h.script.lock().runtimes = vec![runtime(true), absent()].into_iter().map(Ok).collect();
    f.step(&h).await.unwrap();
    assert!(matches!(f.current().retirement.unwrap().state, RetirementState::Unknown { .. }));
    f.step(&h).await.unwrap();
    assert!(matches!(f.current().retirement.unwrap().state,
        RetirementState::Retired { terminal: TerminalOutcome::AbsentAfterUncertainClose, .. }));
    assert_eq!(h.closes(), 1);
}

#[tokio::test]
async fn native_metadata_and_registered_chat_are_not_required_for_exact_idle_pane_authority() {
    let f = Fixture::new(waiting());
    f.change(|run| {
        run.attachment.as_mut().unwrap().conversation = None;
        pin_direct(run);
    });
    let h = FakeHerdr::new(&f, vec![runtime(true), runtime(true), absent()]);
    h.script.lock().allow_close = true;
    f.step(&h).await.unwrap();
    assert_eq!(h.closes(), 1);
    assert!(matches!(f.current().retirement.unwrap().state,
        RetirementState::Retired { terminal: TerminalOutcome::ClosedByCockpit, .. }));
}

#[tokio::test]
async fn root_replacement_retires_exact_idle_old_pane_without_result_or_file_deletion() {
    let mut fixture = Fixture::new(waiting());
    fixture.replacement();
    std::fs::write(fixture.root.join("dirty-old-output"), "preserved old work\n").unwrap();
    let herdr = FakeHerdr::new(&fixture, vec![runtime(true), runtime(true), absent()]);
    herdr.script.lock().allow_close = true;
    fixture.step(&herdr).await.unwrap();
    assert_eq!(herdr.closes(), 1);
    let run = fixture.current();
    assert!(run.result.is_none());
    assert_eq!(run.close_reason, Some(CloseReason::Superseded));
    assert!(matches!(run.retirement.unwrap().state, RetirementState::Retired { terminal: TerminalOutcome::ClosedByCockpit, .. }));
    assert_eq!(std::fs::read_to_string(fixture.root.join("dirty-old-output")).unwrap(), "preserved old work\n");
    fixture.step(&herdr).await.unwrap();
    assert_eq!(herdr.closes(), 1);
}

#[tokio::test]
async fn replacement_busy_and_wrong_authority_do_not_close_until_the_exact_idle_event() {
    let mut fixture = Fixture::new(waiting());
    fixture.replacement();
    let mut busy = runtime(true);
    busy.panes[0].agent_status = Some("working".into());
    let herdr = FakeHerdr::new(&fixture, vec![busy, runtime(true), runtime(true), absent()]);
    herdr.script.lock().allow_close = true;
    fixture.step(&herdr).await.unwrap();
    assert_eq!(herdr.closes(), 0);
    assert!(matches!(fixture.current().retirement.unwrap().state, RetirementState::Waiting { .. }));
    fixture.step(&herdr).await.unwrap();
    assert_eq!(herdr.closes(), 1);
    let mut invalid = Fixture::new(waiting());
    invalid.replacement();
    invalid.change(|run| run.retirement.as_mut().unwrap().replacement.as_mut().unwrap().previous_generation = 99);
    let herdr = FakeHerdr::new(&invalid, vec![runtime(true)]);
    herdr.script.lock().allow_close = true;
    invalid.step(&herdr).await.unwrap();
    assert_eq!(herdr.closes(), 0);
    assert_eq!(herdr.runtime_calls(), 0);
    assert!(matches!(invalid.current().retirement.unwrap().state, RetirementState::Retained { reason: RetainReason::IdentityChanged, .. }));
}

#[tokio::test]
async fn replacement_resource_admissions_are_actionable_waits_without_queue_discard_or_pane_close() {
    for admission in [ResourceRequestState::Held, ResourceRequestState::Queued] {
        let mut fixture = Fixture::new(waiting());
        fixture.replacement();
        let attachment = fixture.current().attachment.unwrap();
        {
            let locked = fixture.service.store.lock().unwrap();
            let mut state = locked.read().unwrap();
            state.resource_requests.push(ResourceRequestRecord { request_id: id(), sequence: 1,
                resource_key: "shared-build".into(), source_identity: ResourceSourceIdentity {
                    checkout_key: "old-checkout".into(), source_commit: "a".repeat(40), source_tree: "b".repeat(40),
                    input_configuration: "input".into(), toolchain_fingerprint: "toolchain".into(),
                }, holder: AttachmentReference { run_id: fixture.run.run_id.clone(),
                    attachment_id: attachment.attachment_id, attachment_epoch: attachment.epoch },
                state: admission, build_processes: Vec::new(), machine_id: "machine".into(),
                blocked_reason: None, updated_at: now() });
            locked.save(&mut state).unwrap();
        }
        let herdr = FakeHerdr::new(&fixture, vec![runtime(true)]);
        herdr.script.lock().allow_close = true;
        fixture.step(&herdr).await.unwrap();
        assert_eq!(herdr.closes(), 0);
        assert!(matches!(fixture.current().retirement.unwrap().state,
            RetirementState::Waiting { blockers } if blockers.contains(&RetirementBlocker::ResourceAdmissions)));
        let state = fixture.service.store.lock().unwrap().read().unwrap();
        assert_eq!(state.resource_requests[0].state, admission);
    }
}
