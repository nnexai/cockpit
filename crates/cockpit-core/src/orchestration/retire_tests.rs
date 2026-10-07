use std::{collections::VecDeque, path::PathBuf, sync::Arc};
use parking_lot::Mutex;
use cockpit_protocol::orchestration::*;
use super::{herdr::*, retire::{retire_run, ObservationBackoff}, retirement, *};

struct Fixture { root: PathBuf, service: Arc<OrchestrationService>, run: Run }
impl Fixture {
    fn new(state: RetirementState) -> Self {
        let root = std::env::temp_dir().join(format!("cockpit-retire-{}", uuid::Uuid::new_v4()));
        let config = serde_json::from_value(serde_json::json!({
            "version":1,"repository_roots":[],"worktree_root":root.join("worktrees"),
            "companion_root":root.join("companions"),"state_root":root.join("state"),
            "cache_root":root.join("cache"),"library_root":root.join("library"),"notes_root":root.join("notes"),
            "branch_template":"test/{task}","checkout_template":"{task}","providers":[],"origins":{},
            "limits":{"catalog_depth":1,"catalog_entries":1,"git_timeout_ms":1000,"git_output_bytes":1024,
                "operation_timeout_ms":1000,"context_preview_bytes":1024,"context_preview_lines":10,
                "context_directory_entries":10,"context_tree_depth":1,"library_folder_files":10,"library_folder_bytes":1024,
                "library_file_bytes":1024,"library_space_pages":10,"library_attachment_bytes":1024,
                "library_item_attachment_bytes":1024,"library_max_items":10}
        })).unwrap();
        let service = Arc::new(OrchestrationService::open(&config).unwrap());
        let id = uuid::Uuid::new_v4().to_string();
        let mut run = new_run(&id, "fixture", RunKind::Worker, "Worker".into(), &id, None, None, 1);
        run.stage = RunStage::Closed;
        run.close_reason = Some(CloseReason::Accepted);
        run.location = Some(RunLocation {
            endpoint_identity: "endpoint".into(), session_id: "fixture".into(), workspace_id: "space".into(),
            tab_id: "tab".into(), pane_id: "pane".into(), terminal_id: Some("terminal".into()),
            launch_tag: "tag".into(), boot_id: None, native_session_id: None,
        });
        run.bound_omp_session = Some("native".into());
        run.bound_omp_process = Some(process());
        run.launch_shell_identity = Some(shell_identity());
        let mut launch = dispatch(DispatchStep::Launched);
        launch.agent_started = true;
        launch.launch_attempt = 1;
        launch.launch_tag = Some("tag".into());
        launch.endpoint_identity = Some("endpoint".into());
        run.dispatch = Some(launch);
        run.result = Some(Report { message_id: "result".into(), kind: ReportKind::Result,
            outcome: Some(ReportOutcome::Succeeded), summary: "Complete".into(), plan: None, at: now() });
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
    NativeProcessIdentity { pid: std::process::id(),
        start_ticks: crate::process_identity::start_identity(std::process::id() as i32).unwrap(),
        kernel_boot_id: crate::process_identity::kernel_boot_id() }
}
fn stopped() -> RetirementState {
    RetirementState::NativeStopped { at: now(), evidence: NativeStopEvidence::AlreadyExited }
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
            launch_pending: false, interactive_ready: false, agent_status: None, state_changed_at: None }], }
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
    response: Option<Result<(), InspectionError>>,
    events: Vec<&'static str>,
    close_count: usize,
    runtime_hook: Option<(usize, Box<dyn FnOnce() + Send>)>,
}
struct FakeHerdr { service: Arc<OrchestrationService>, run_id: String, script: Mutex<Script> }
impl FakeHerdr {
    fn new(f: &Fixture, reads: Vec<RuntimeView>) -> Self {
        Self { service: Arc::clone(&f.service), run_id: f.run.run_id.clone(), script: Mutex::new(Script {
            runtimes: reads.into_iter().map(Ok).collect(), info: original_shell_info(), info_error: false,
            response: Some(Ok(())), events: vec![], close_count: 0, runtime_hook: None,
            infos: VecDeque::new(),
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
        let stored = self.service.store.lock().unwrap().read().unwrap();
        let r = stored.runs.iter().find(|r| r.run_id == self.run_id).unwrap();
        assert!(matches!(r.retirement.as_ref().unwrap().state, RetirementState::CloseIntent { .. }));
        assert!(retirement::identity_matches(r, r.retirement.as_ref().unwrap()));
        let mut s = self.script.lock();
        assert_eq!(s.events, ["runtime", "info", "runtime", "info"]);
        s.events.push("close"); s.close_count += 1;
        assert_eq!(s.close_count, 1);
        s.response.take().unwrap()
    }
    async fn create_agent_tab(&self, _: &str, _: &AgentTabRequest) -> Result<RunLocation, InspectionError> {
        panic!("retirement cannot create or broadly close tabs/workspaces")
    }
    async fn start_agent(&self, _: &str, _: &AgentStartRequest) -> Result<(), InspectionError> {
        panic!("retirement cannot start or signal an agent")
    }
}

#[tokio::test]
async fn final_owned_pane_closes_once_with_durable_intent_and_double_fresh_reads() {
    let f = Fixture::new(stopped());
    let h = FakeHerdr::new(&f, vec![runtime(false), runtime(false), absent()]);
    let b = ObservationBackoff::default();
    f.step(&h, &b).await.unwrap();
    assert!(matches!(f.current().retirement.unwrap().state,
        RetirementState::Retired { terminal: TerminalOutcome::ClosedByCockpit, .. }));
    assert_eq!(h.closes(), 1);
    f.step(&h, &b).await.unwrap();
    assert_eq!(h.closes(), 1);
    assert_eq!(h.runtime_calls(), 3);
}

#[tokio::test]
async fn siblings_in_other_tabs_survive_but_shared_tab_and_contamination_are_retained() {
    for shared in [false, true] {
        let f = Fixture::new(stopped());
        let mut view = runtime(false);
        let mut sibling = view.panes[0].clone();
        sibling.pane_id = "user-pane".into(); sibling.terminal_id = Some("user-terminal".into());
        sibling.tab_id = if shared { "tab" } else { "user-tab" }.into();
        view.panes.push(sibling.clone());
        let mut after = absent(); after.panes.push(sibling);
        let h = FakeHerdr::new(&f, vec![view.clone(), view, after]);
        f.step(&h, &ObservationBackoff::default()).await.unwrap();
        assert_eq!(h.closes(), usize::from(!shared));
        if shared { assert!(matches!(f.current().retirement.unwrap().state,
            RetirementState::Retained { reason: RetainReason::SharedTab, native_stopped: true, .. })); }
    }
    for change in 0..5 {
        let f = Fixture::new(stopped());
        let mut contaminated = runtime(false);
        match change {
            0 => contaminated.panes[0].tab_label = "User renamed".into(),
            1 => contaminated.panes[0].tab_id = "moved".into(),
            2 => contaminated.panes[0].agent_kind = Some("new-agent".into()),
            3 => contaminated.endpoint_identity = "new-endpoint".into(),
            _ => contaminated.panes[0].launch_pending = true,
        }
        let h = FakeHerdr::new(&f, vec![runtime(false), contaminated]);
        f.step(&h, &ObservationBackoff::default()).await.unwrap();
        assert_eq!(h.closes(), 0);
        assert!(matches!(f.current().retirement.unwrap().state,
            RetirementState::Retained { native_stopped: true, .. }));
    }
}

#[tokio::test]
async fn foreground_process_or_second_preflight_unavailability_never_closes() {
    for unavailable in [false, true] {
        let f = Fixture::new(stopped());
        let h = FakeHerdr::new(&f, vec![runtime(false)]);
        if !unavailable { h.script.lock().info.foreground_pgid = Some(999); }
        f.step(&h, &ObservationBackoff::default()).await.unwrap();
        assert_eq!(h.closes(), 0);
        assert!(matches!(f.current().retirement.unwrap().state,
            RetirementState::Retained { native_stopped: true, .. }));
    }
}

#[tokio::test]
async fn close_response_matrix_is_terminal_and_never_resent() {
    for (code, seen, expected) in [
        (None, false, "closed"), (None, true, "unknown"),
        (Some("pane_not_found"), false, "absent"), (Some("pane_not_found"), true, "moved"),
        (Some("herdr_outcome_unknown"), false, "uncertain_absent"),
        (Some("herdr_outcome_unknown"), true, "unknown"),
        (Some("confirmation_required"), false, "refused"),
    ] {
        let f = Fixture::new(stopped());
        let h = FakeHerdr::new(&f, vec![runtime(false), runtime(false), if seen { runtime(false) } else { absent() }]);
        if let Some(code) = code { h.script.lock().response = Some(Err(InspectionError::new(code, "response"))); }
        let b = ObservationBackoff::default();
        f.step(&h, &b).await.unwrap();
        let state = f.current().retirement.unwrap().state;
        match expected {
            "closed" => assert!(matches!(state, RetirementState::Retired { terminal: TerminalOutcome::ClosedByCockpit, .. })),
            "absent" => assert!(matches!(state, RetirementState::Retired { terminal: TerminalOutcome::AlreadyAbsent, .. })),
            "uncertain_absent" => assert!(matches!(state, RetirementState::Retired { terminal: TerminalOutcome::AbsentAfterUncertainClose, .. })),
            "moved" => assert!(matches!(state, RetirementState::Retained { reason: RetainReason::PaneMoved, .. })),
            "refused" => assert!(matches!(state, RetirementState::Retained { reason: RetainReason::HerdrRefused, .. })),
            _ => assert!(matches!(state, RetirementState::Unknown { phase: RetirementPhase::TerminalClose, .. })),
        }
        // A new owner has no in-memory effect history; the durable terminal
        // outcome alone prevents a resend.
        f.step(&h, &ObservationBackoff::default()).await.unwrap();
        assert_eq!(h.closes(), 1);
        assert_eq!(h.runtime_calls(), 3);
    }
}

#[tokio::test]
async fn recovered_close_intent_performs_one_read_only_reclassification() {
    for seen in [0, 1, 2] {
        let f = Fixture::new(RetirementState::CloseIntent { at: now() });
        let h = FakeHerdr::new(&f, match seen { 0 => vec![absent()], 1 => vec![runtime(false)], _ => vec![] });
        f.step(&h, &ObservationBackoff::default()).await.unwrap();
        f.step(&h, &ObservationBackoff::default()).await.unwrap();
        assert_eq!(h.closes(), 0); assert_eq!(h.runtime_calls(), 1);
        assert!(retirement::terminal(&f.current().retirement.unwrap().state));
    }
}

#[tokio::test]
async fn unavailable_reads_are_bounded_and_eventually_visible() {
    assert!(retirement::OBSERVATION_RETRY_SECS >= 30);
    for waiting in [false, true] {
        let f = Fixture::new(if waiting { RetirementState::Waiting { blockers: vec![] } } else { stopped() });
        let h = FakeHerdr::new(&f, vec![]);
        let b = ObservationBackoff::default();
        for _ in 0..1000 { f.step(&h, &b).await.unwrap(); }
        assert_eq!(h.runtime_calls(), 1);
        assert_eq!(h.closes(), 0);
        f.change(|r| {
            let record = r.retirement.as_mut().unwrap();
            record.created_at = old();
            if !waiting { record.state = RetirementState::NativeStopped { at: old(), evidence: NativeStopEvidence::AlreadyExited }; }
        });
        // Simulate a fresh owner after the deadline; unavailable evidence
        // cannot leave an invisible retry loop running forever.
        f.step(&h, &ObservationBackoff::default()).await.unwrap();
        assert!(matches!(f.current().retirement.unwrap().state,
            RetirementState::Retained { reason: RetainReason::ObservationUnavailable, .. }));
    }
}

#[tokio::test]
async fn offer_uses_exact_process_pane_session_endpoint_and_fresh_read() {
    for mismatch in 0..5 {
        let f = Fixture::new(RetirementState::Waiting { blockers: vec![] });
        let mut view = runtime(true);
        match mismatch {
            1 => view.endpoint_identity = "replacement".into(),
            2 => view.panes[0].terminal_id = Some("replacement".into()),
            3 => view.panes[0].native_session_id = Some("replacement".into()),
            _ => {},
        }
        let h = FakeHerdr::new(&f, vec![view]);
        h.script.lock().info.processes = vec![(if mismatch == 4 { 999 } else { process().pid }, "omp".into())];
        f.step(&h, &ObservationBackoff::default()).await.unwrap();
        assert_eq!(h.closes(), 0);
        if mismatch == 0 { assert!(matches!(f.current().retirement.unwrap().state, RetirementState::NativeStopOffered { .. })); }
        else { assert!(matches!(f.current().retirement.unwrap().state, RetirementState::Retained { native_stopped: false, .. })); }
    }
}

#[tokio::test]
async fn open_native_descendants_block_offer_without_runtime_reads() {
    let f = Fixture::new(RetirementState::Waiting { blockers: vec![] });
    let mut child = new_run("child", "fixture", RunKind::Worker, "Child".into(), &f.run.run_id,
        Some(f.run.run_id.clone()), None, 1);
    child.stage = RunStage::Working;
    let locked = f.service.store.lock().unwrap();
    let mut state = locked.read().unwrap(); state.runs.push(child); locked.save(&mut state).unwrap(); drop(locked);
    let h = FakeHerdr::new(&f, vec![runtime(true)]);
    f.step(&h, &ObservationBackoff::default()).await.unwrap();
    assert_eq!(h.runtime_calls(), 0); assert_eq!(h.closes(), 0);
    assert!(matches!(f.current().retirement.unwrap().state,
        RetirementState::Waiting { blockers } if blockers.contains(&RetirementBlocker::OpenDescendantRuns)));
}

#[tokio::test]
async fn native_ack_and_missing_pane_are_not_exit_proof_and_timeout_is_unknown() {
    let f = Fixture::new(RetirementState::NativeStopRequested { at: now() });
    let h = FakeHerdr::new(&f, vec![absent()]);
    f.step(&h, &ObservationBackoff::default()).await.unwrap();
    assert!(matches!(f.current().retirement.unwrap().state, RetirementState::NativeStopRequested { .. }));
    assert_eq!(h.runtime_calls(), 0); assert_eq!(h.closes(), 0);
    f.change(|r| r.retirement.as_mut().unwrap().state = RetirementState::NativeStopRequested { at: old() });
    f.step(&h, &ObservationBackoff::default()).await.unwrap();
    assert!(matches!(f.current().retirement.unwrap().state,
        RetirementState::Unknown { phase: RetirementPhase::NativeStop, .. }));
}

#[cfg(unix)]
#[tokio::test]
async fn spawned_exact_incarnation_must_exit_before_native_stopped() {
    let mut child = tokio::process::Command::new("sleep").arg("60")
        .kill_on_drop(true).spawn().unwrap();
    let pid = child.id().unwrap();
    let identity = NativeProcessIdentity { pid,
        start_ticks: crate::process_identity::start_identity(pid as i32).unwrap(),
        kernel_boot_id: crate::process_identity::kernel_boot_id() };
    let f = Fixture::new(RetirementState::NativeStopRequested { at: now() });
    f.change(|r| { r.bound_omp_process = Some(identity.clone());
        r.retirement.as_mut().unwrap().identity.as_mut().unwrap().process = identity; });
    let h = FakeHerdr::new(&f, vec![]);
    f.step(&h, &ObservationBackoff::default()).await.unwrap();
    assert!(matches!(f.current().retirement.unwrap().state, RetirementState::NativeStopRequested { .. }));
    child.kill().await.unwrap(); child.wait().await.unwrap();
    h.script.lock().info_error = true;
    f.step(&h, &ObservationBackoff::default()).await.unwrap();
    assert!(matches!(f.current().retirement.unwrap().state,
        RetirementState::NativeStopped { evidence: NativeStopEvidence::ExitedAfterShutdownRequest, .. }));
    assert_eq!(h.closes(), 0);
}

#[tokio::test]
async fn lease_and_durable_identity_changes_fence_every_close() {
    for boundary in [1, 2] {
        let f = Fixture::new(stopped());
        let h = FakeHerdr::new(&f, vec![runtime(false), runtime(false), absent()]);
        let svc = Arc::clone(&f.service); let id = f.run.run_id.clone();
        h.script.lock().runtime_hook = Some((boundary, Box::new(move || {
            change_run(&svc, &id, |r| r.bound_omp_session = Some("replaced".into()));
        })));
        let result = f.step(&h, &ObservationBackoff::default()).await;
        if boundary == 1 {
            // The first durable save fences the changed identity by retaining
            // the pane instead of writing CloseIntent. A later CloseIntent CAS
            // must reject that terminal record rather than overwrite it.
            assert_eq!(result.unwrap_err().code, "retirement_state_changed");
        } else {
            result.unwrap();
        }
        assert_eq!(h.closes(), 0);
        assert_eq!(h.runtime_calls(), 2);
        let retained = f.current().retirement.unwrap();
        assert!(matches!(retained.state,
            RetirementState::Retained { reason: RetainReason::IdentityChanged, native_stopped: true, .. }));
        // A fresh owner must preserve the durable fence without issuing a
        // close or even repeating the read-only preflights.
        f.step(&h, &ObservationBackoff::default()).await.unwrap();
        assert_eq!(f.current().retirement.unwrap(), retained);
        assert_eq!(h.closes(), 0);
        assert_eq!(h.runtime_calls(), 2);
    }
    let f = Fixture::new(stopped());
    let h = FakeHerdr::new(&f, vec![runtime(false), runtime(false), absent()]);
    let lease = f.service.execution_lease(&f.run.run_id).unwrap().unwrap();
    f.step(&h, &ObservationBackoff::default()).await.unwrap();
    assert_eq!(h.runtime_calls(), 0); assert_eq!(h.closes(), 0); drop(lease);
    f.step(&h, &ObservationBackoff::default()).await.unwrap(); assert_eq!(h.closes(), 1);
}

#[tokio::test]
async fn offered_and_deferred_deadlines_do_not_issue_native_or_terminal_effects() {
    for deferred in [false, true] {
        let f = Fixture::new(if deferred {
            RetirementState::NativeStopDeferred { offered_at: old(), reason: NativeDeferReason::Busy, at: now() }
        } else {
            RetirementState::NativeStopOffered { offered_at:
                (time::OffsetDateTime::now_utc() - time::Duration::seconds(121))
                    .format(&time::format_description::well_known::Rfc3339).unwrap() }
        });
        let h = FakeHerdr::new(&f, vec![]);
        f.step(&h, &ObservationBackoff::default()).await.unwrap();
        assert_eq!(h.runtime_calls(), 0); assert_eq!(h.closes(), 0);
        assert!(matches!(f.current().retirement.unwrap().state,
            RetirementState::Retained { reason, native_stopped: false, .. }
            if reason == if deferred { RetainReason::WorkerBusyTimeout } else { RetainReason::WorkerUnresponsive }));
    }
}

#[tokio::test]
async fn running_subagent_telemetry_blocks_offer_without_external_observation() {
    let f = Fixture::new(RetirementState::Waiting { blockers: vec![] });
    let locked = f.service.store.lock().unwrap();
    let mut state = locked.read().unwrap();
    state.subagents.push(Subagent { run_id: f.run.run_id.clone(), subagent_id: "child".into(),
        parent_subagent_id: None, role: None, label: "Child".into(), status: SubagentStatus::Running,
        summary: None, last_control: None, updated_at: now() });
    locked.save(&mut state).unwrap(); drop(locked);
    let h = FakeHerdr::new(&f, vec![]);
    f.step(&h, &ObservationBackoff::default()).await.unwrap();
    assert_eq!(h.runtime_calls(), 0); assert_eq!(h.closes(), 0);
    assert!(matches!(f.current().retirement.unwrap().state, RetirementState::Waiting { blockers }
        if blockers == vec![RetirementBlocker::RunningSubagents]));
}

#[tokio::test]
async fn failed_close_verification_stays_unknown_across_owner_restart() {
    let f = Fixture::new(stopped());
    let h = FakeHerdr::new(&f, vec![runtime(false), runtime(false)]);
    h.script.lock().response = Some(Err(InspectionError::new("herdr_outcome_unknown", "Transport lost")));
    f.step(&h, &ObservationBackoff::default()).await.unwrap();
    assert!(matches!(f.current().retirement.unwrap().state,
        RetirementState::Unknown { phase: RetirementPhase::TerminalClose, .. }));
    f.step(&h, &ObservationBackoff::default()).await.unwrap();
    assert_eq!(h.runtime_calls(), 3); assert_eq!(h.closes(), 1);
}

#[tokio::test]
async fn identity_change_before_offer_and_replaced_retirement_queue_hint_are_fenced() {
    let f = Fixture::new(RetirementState::Waiting { blockers: vec![] });
    let h = FakeHerdr::new(&f, vec![runtime(true)]);
    h.script.lock().info.processes = vec![(process().pid, "omp".into())];
    let svc = Arc::clone(&f.service); let id = f.run.run_id.clone();
    h.script.lock().runtime_hook = Some((1, Box::new(move || {
        change_run(&svc, &id, |r| r.bound_omp_session = Some("replacement".into()));
    })));
    f.step(&h, &ObservationBackoff::default()).await.unwrap();
    assert!(matches!(f.current().retirement.unwrap().state,
        RetirementState::Retained { reason: RetainReason::IdentityChanged, native_stopped: false, .. }));
    assert_eq!(h.closes(), 0);
    let f = Fixture::new(stopped());
    f.change(|r| r.retirement.as_mut().unwrap().retirement_id = uuid::Uuid::new_v4().to_string());
    let h = FakeHerdr::new(&f, vec![]);
    f.step(&h, &ObservationBackoff::default()).await.unwrap();
    assert_eq!(h.runtime_calls(), 0); assert_eq!(h.closes(), 0);
}

#[tokio::test]
async fn old_process_incarnation_absence_is_distinct_from_pane_disappearance() {
    let f = Fixture::new(RetirementState::Waiting { blockers: vec![] });
    f.change(|r| {
        let mut previous = r.bound_omp_process.clone().unwrap();
        previous.start_ticks += 1;
        r.bound_omp_process = Some(previous.clone());
        r.retirement.as_mut().unwrap().identity.as_mut().unwrap().process = previous;
    });
    let h = FakeHerdr::new(&f, vec![runtime(false)]);
    f.step(&h, &ObservationBackoff::default()).await.unwrap();
    assert!(matches!(f.current().retirement.unwrap().state,
        RetirementState::NativeStopped { evidence: NativeStopEvidence::AlreadyExited, .. }));
    assert_eq!(h.closes(), 0);
    let f = Fixture::new(RetirementState::Waiting { blockers: vec![] });
    let h = FakeHerdr::new(&f, vec![absent()]);
    f.step(&h, &ObservationBackoff::default()).await.unwrap();
    assert!(matches!(f.current().retirement.unwrap().state,
        RetirementState::Retained { reason: RetainReason::ProcessPaneMismatch, native_stopped: false, .. }));
}

#[tokio::test]
async fn newly_running_descendant_during_offer_preflight_cannot_be_offered_shutdown() {
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
    assert_eq!(f.step(&h, &ObservationBackoff::default()).await.unwrap_err().code,
        "retirement_state_changed");
    assert!(matches!(f.current().retirement.unwrap().state, RetirementState::Waiting { .. }));
    assert_eq!(h.closes(), 0);
}

#[tokio::test]
async fn missing_or_changed_shell_fingerprint_is_retained_at_either_preflight() {
    for boundary in [1, 2] {
        for missing in [false, true] {
            let f = Fixture::new(stopped());
            let h = FakeHerdr::new(&f, vec![runtime(false), runtime(false)]);
            let mut changed = original_shell_info();
            if missing { changed.shell_identity = None; }
            else { changed.shell_identity.as_mut().unwrap().argv_digest = "b".repeat(64); }
            h.script.lock().infos = if boundary == 1 { vec![changed] }
                else { vec![original_shell_info(), changed] }.into();
            f.step(&h, &ObservationBackoff::default()).await.unwrap();
            assert_eq!(h.closes(), 0);
            assert!(matches!(f.current().retirement.unwrap().state,
                RetirementState::Retained { reason, native_stopped: true, .. }
                if reason == if missing { RetainReason::NativeProcessUnverifiable } else { RetainReason::ForegroundProcess }));
        }
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
            let f = Fixture::new(stopped());
            f.change(|r| {
                r.launch_shell_identity = Some(baseline.clone());
                r.retirement.as_mut().unwrap().identity.as_mut().unwrap().shell = baseline.clone();
            });
            let info = |identity: NativeShellIdentity| PaneProcessInfo {
                pane_id: "pane".into(), shell_pid: Some(pid), foreground_pgid: Some(pid),
                processes: vec![(pid, "shell-or-exec".into())], shell_identity: Some(identity),
            };
            let h = FakeHerdr::new(&f, vec![runtime(false), runtime(false)]);
            h.script.lock().infos = if boundary == 1 { vec![info(fresh)] }
                else { vec![info(baseline), info(fresh)] }.into();
            let result = f.step(&h, &ObservationBackoff::default()).await;
            child.kill().await.unwrap(); child.wait().await.unwrap();
            result.unwrap();
            assert_eq!(h.closes(), 0);
            assert!(matches!(f.current().retirement.unwrap().state,
                RetirementState::Retained { reason: RetainReason::ForegroundProcess, native_stopped: true, .. }));
        }
    }
}

#[cfg(target_os = "linux")]
#[tokio::test]
async fn original_shell_builtin_work_is_permitted_owned_pane_retirement_not_an_idle_claim() {
    use tokio::io::{AsyncBufReadExt, AsyncWriteExt};
    // `read` is work inside the original shell: no exec changes its fingerprint.
    // Parent-approved policy permits this owned shell to close after the native
    // worker exited; no readiness/idle inference is made from these OS values.
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
    let f = Fixture::new(stopped());
    f.change(|r| {
        r.launch_shell_identity = Some(baseline.clone());
        r.retirement.as_mut().unwrap().identity.as_mut().unwrap().shell = baseline;
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
    assert_eq!(h.closes(), 1);
    assert!(matches!(f.current().retirement.unwrap().state,
        RetirementState::Retired { terminal: TerminalOutcome::ClosedByCockpit, .. }));
}
