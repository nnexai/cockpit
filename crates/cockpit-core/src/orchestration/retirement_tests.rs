use super::*;
use super::retirement::*;
use super::herdr::{PaneProcessInfo, RuntimePane, RuntimeView};
use std::path::PathBuf;

const SESSION: &str = "retirement-tests";
const AT: &str = "2026-10-07T01:00:00Z";
const ROOT: &str = "12345678-1234-4234-8234-123456789abc";

struct Fixture { path: PathBuf, service: OrchestrationService, task: Task }
impl Fixture {
    fn new() -> Self {
        let path = std::env::temp_dir().join(format!("cockpit-retirement-{}", id()));
        let store = OrchestrationStore::open(&path).unwrap();
        let (revision, _) = watch::channel(0);
        let service = OrchestrationService { store, revision };
        let locked = service.store.lock().unwrap();
        let mut state = locked.read().unwrap();
        let mut root = new_run(ROOT, SESSION, RunKind::Supervisor, "root".into(), ROOT, None, None, 1);
        root.stage = RunStage::Active;
        state.runs.push(root);
        locked.save(&mut state).unwrap();
        let task_id = uuid::Uuid::new_v4().to_string();
        let task = locked.tasks(ROOT).unwrap().create_authoring_with_id(
            &task_id, "Retire exact worker", "Preserve files", &[], None, None, None,
        ).unwrap();
        let mut worker = worker();
        worker.task_id = Some(task.task_id.clone());
        state.runs.push(worker);
        locked.save(&mut state).unwrap();
        drop(locked);
        Self { path, service, task }
    }
    fn state(&self) -> OrchestrationState { self.service.store.lock().unwrap().read().unwrap() }
    fn run(&self) -> Run { self.state().runs.into_iter().find(|r| r.run_id == "worker").unwrap() }
    fn edit(&self, change: impl FnOnce(&mut OrchestrationState)) {
        let locked = self.service.store.lock().unwrap();
        let mut state = locked.read().unwrap();
        change(&mut state);
        locked.save(&mut state).unwrap();
    }
    fn mutate(&self, actor: &Actor, action: OrchestrationAction) -> Result<OrchestrationMutationResponse, InspectionError> {
        self.service.mutate(actor, OrchestrationMutationRequest { session_id: SESSION.into(), expected_revision: None, action })
    }
    fn accept(&self) {
        self.mutate(&Actor::Operator(OperatorOrigin::Browser), OrchestrationAction::Accept {
            run_id: "worker".into(), expected_task_revision: self.task.task_revision.clone(),
        }).unwrap();
    }
    fn offer(&self) -> String {
        self.accept();
        let r = self.run().retirement.unwrap();
        self.service.record_retirement("worker", &r.retirement_id, RetirementStateKind::Waiting,
            RetirementState::NativeStopOffered { offered_at: AT.into() }).unwrap();
        r.retirement_id
    }
    fn intent(&self) -> TaskIntent {
        let intent = TaskIntent { intent_id: id(), root_id: ROOT.into(), task_id: self.task.task_id.clone(),
            run_id: "worker".into(), expected_task_revision: self.task.task_revision.clone(), state: IntentState::Pending,
            origin: Some(GrantOrigin::Browser), supervisor_run_id: None, omp_session_id: None,
            result_message_id: Some("reviewed-result".into()) };
        self.edit(|s| s.task_intents.push(intent.clone()));
        intent
    }
}
impl Drop for Fixture { fn drop(&mut self) { let _ = std::fs::remove_dir_all(&self.path); } }

fn worker() -> Run {
    let mut run = new_run("worker", SESSION, RunKind::Worker, "worker".into(), ROOT, Some(ROOT.into()), None, 7);
    run.stage = RunStage::Reported;
    run.dispatch = Some(DispatchState { launch_tag: Some("owned-launch".into()), endpoint_identity: Some("endpoint".into()),
        recovery: None, agent_started: true, step: DispatchStep::Launched, launch_attempt: 3, error: None, updated_at: AT.into() });
    run.location = Some(RunLocation { boot_id: Some("herdr-boot".into()), terminal_id: Some("terminal".into()),
        native_session_id: Some("native".into()), endpoint_identity: "endpoint".into(), session_id: SESSION.into(),
        workspace_id: "space".into(), tab_id: "tab".into(), pane_id: "pane".into(), launch_tag: "owned-launch".into() });
    run.bound_omp_session = Some("native".into());
    run.bound_omp_process = Some(NativeProcessIdentity { pid: 123, start_ticks: 456, kernel_boot_id: Some("kernel".into()) });
    run.launch_shell_identity = Some(shell_identity());
    run.result = Some(Report { message_id: "reviewed-result".into(), kind: ReportKind::Result,
        outcome: Some(ReportOutcome::Succeeded), summary: "Verified evidence".into(), plan: None, at: AT.into() });
    run
}
fn caller() -> AgentCaller {
    AgentCaller { endpoint_identity: "endpoint".into(), session_id: SESSION.into(), workspace_id: "space".into(),
        tab_id: "tab".into(), pane_id: "pane".into(), boot_id: Some("herdr-boot".into()), terminal_id: Some("terminal".into()),
        native_session_id: Some("native".into()), env_run: Some(("worker".into(), 7)), omp_session_id: Some("native".into()),
        agent_kind: Some(AgentKind::Main), actual_agent_kind: Some("omp".into()), subagent_id: None,
        main_omp_session_id: Some("native".into()), process: worker().bound_omp_process }
}
fn receipt(retirement_id: &str, outcome: NativeStopReceipt) -> OrchestrationAction {
    OrchestrationAction::RetirementNativeReceipt { retirement_id: retirement_id.into(), outcome }
}
fn runtime() -> RuntimeView {
    RuntimeView { endpoint_identity: "endpoint".into(), boot_id: Some("herdr-boot".into()), workspaces: vec![], panes: vec![RuntimePane {
        workspace_id: "space".into(), workspace_label: "Space".into(), tab_id: "tab".into(), tab_label: "owned-launch".into(),
        pane_id: "pane".into(), terminal_id: Some("terminal".into()), native_session_id: None, agent_name: None,
        agent_kind: None, launch_pending: false, interactive_ready: false, agent_status: None, state_changed_at: None,
    }] }
}
fn shell_identity() -> NativeShellIdentity {
    NativeShellIdentity {
        process: NativeProcessIdentity { pid: 11, start_ticks: 12, kernel_boot_id: Some("kernel".into()) },
        executable_device: "1".into(), executable_inode: "2".into(), argv_digest: "a".repeat(64),
    }
}
fn shell() -> PaneProcessInfo {
    PaneProcessInfo { pane_id: "pane".into(), shell_pid: Some(11), foreground_pgid: Some(11),
        processes: vec![(11, "shell".into())], shell_identity: Some(shell_identity()) }
}
fn identity() -> RetirementIdentity { retirement_for_accepted(&worker(), RetirementTrigger::Accept, "revision", AT).unwrap().identity.unwrap() }

#[test]
fn acceptance_commits_exact_receipt_revision_and_launch_identity() {
    let fixture = Fixture::new();
    assert!(fixture.service.retirement_queue().unwrap().is_empty());
    fixture.accept();
    let run = fixture.run();
    assert_eq!(run.stage, RunStage::Closed);
    assert_eq!(run.close_reason, Some(CloseReason::Accepted));
    let r = run.retirement.unwrap();
    assert_eq!(r.result_message_id, "reviewed-result");
    assert_eq!(r.task_revision, fixture.task.task_revision);
    assert_eq!(r.trigger, RetirementTrigger::Accept);
    assert_eq!(r.identity.unwrap(), identity());
    assert_eq!(fixture.service.retirement_queue().unwrap().len(), 1);
    assert!(run.annotations.iter().any(|a| matches!(a.by, ActorRef::Dispatcher)));
    assert!(fixture.service.store.lock().unwrap().tasks(ROOT).unwrap().task(&fixture.task.task_id).unwrap().checked);
}

#[test]
fn failed_acceptance_cancel_and_old_closed_runs_never_queue() {
    for invalid in 0..3 {
        let fixture = Fixture::new();
        fixture.edit(|s| { let run = &mut s.runs[1]; match invalid {
            0 => run.result.as_mut().unwrap().outcome = Some(ReportOutcome::Failed),
            1 => run.stage = RunStage::Working,
            _ => {},
        } });
        let result = fixture.mutate(&Actor::Operator(OperatorOrigin::Browser), OrchestrationAction::Accept {
            run_id: "worker".into(), expected_task_revision: if invalid == 2 { "wrong-revision".into() } else { fixture.task.task_revision.clone() },
        });
        assert!(result.is_err());
        assert!(fixture.run().retirement.is_none());
    }
    let fixture = Fixture::new();
    fixture.mutate(&Actor::Operator(OperatorOrigin::Browser), OrchestrationAction::CancelRun { run_id: "worker".into() }).unwrap();
    assert!(fixture.run().retirement.is_none());
    assert!(fixture.service.retirement_queue().unwrap().is_empty());
    let mut state = OrchestrationState::default();
    let mut run = worker(); run.stage = RunStage::Closed; run.close_reason = Some(CloseReason::Accepted);
    state.runs.push(run);
    close_accepted(&mut state, 0, RetirementTrigger::AcceptRecovery, "old-revision");
    assert!(state.runs[0].retirement.is_none());
}

#[test]
fn incomplete_evidence_is_retained_not_acted_and_roots_are_excluded() {
    for missing in 0..12 {
        let mut run = worker();
        match missing {
            0 => run.bound_omp_process = None,
            1 => run.bound_omp_session = None,
            2 => run.location.as_mut().unwrap().terminal_id = None,
            3 => run.dispatch.as_mut().unwrap().launch_tag = Some("other".into()),
            4 => run.dispatch.as_mut().unwrap().endpoint_identity = Some("other".into()),
            5 => run.location.as_mut().unwrap().native_session_id = Some("other".into()),
            6 => run.bound_omp_process.as_mut().unwrap().start_ticks = 0,
            7 => run.dispatch.as_mut().unwrap().agent_started = false,
            8 => run.dispatch.as_mut().unwrap().launch_attempt = 0,
            9 => run.launch_shell_identity = None,
            10 => run.launch_shell_identity.as_mut().unwrap().argv_digest.clear(),
            _ => run.launch_shell_identity.as_mut().unwrap().process.kernel_boot_id = Some("other".into()),
        }
        let r = retirement_for_accepted(&run, RetirementTrigger::Accept, "revision", AT).unwrap();
        assert!(r.identity.is_none());
        assert!(matches!(r.state, RetirementState::Retained { reason: RetainReason::IdentityIncomplete, native_stopped: false, .. }));
    }
    for kind in [RunKind::Supervisor, RunKind::Adopted] {
        let mut run = worker(); run.kind = kind;
        assert!(retirement_for_accepted(&run, RetirementTrigger::Accept, "revision", AT).is_none());
    }
    let mut run = worker(); run.dispatch = None;
    assert!(retirement_for_accepted(&run, RetirementTrigger::Accept, "revision", AT).is_none());
}

#[test]
fn recovery_is_idempotent_and_preserves_exact_result_and_revision() {
    for checked in [false, true] {
        let fixture = Fixture::new();
        fixture.intent();
        if checked { fixture.service.store.lock().unwrap().tasks(ROOT).unwrap().check(&fixture.task.task_id, &fixture.task.task_revision, true).unwrap(); }
        fixture.service.recover_intents().unwrap();
        let r = fixture.run().retirement.unwrap();
        assert_eq!(r.trigger, RetirementTrigger::AcceptRecovery);
        assert_eq!(r.result_message_id, "reviewed-result");
        assert_eq!(r.task_revision, fixture.task.task_revision);
        let revision = fixture.state().revision;
        fixture.service.recover_intents().unwrap();
        assert_eq!(fixture.state().revision, revision);
        assert_eq!(fixture.run().retirement.unwrap().retirement_id, r.retirement_id);
    }
    for conflict in [false, true] {
        let fixture = Fixture::new(); fixture.intent();
        if conflict {
            fixture.service.store.lock().unwrap().tasks(ROOT).unwrap().update(&fixture.task.task_id, &fixture.task.task_revision, None, Some("Changed body")).unwrap();
        } else { fixture.edit(|s| s.runs[1].result.as_mut().unwrap().message_id = "replacement-result".into()); }
        fixture.service.recover_intents().unwrap();
        assert!(fixture.run().retirement.is_none());
        assert_eq!(fixture.state().task_intents[0].state, IntentState::Conflict);
    }
}

#[test]
fn operator_resolution_uses_current_revision_but_not_a_different_result() {
    let fixture = Fixture::new(); let intent = fixture.intent();
    let current = fixture.service.store.lock().unwrap().tasks(ROOT).unwrap().update(&fixture.task.task_id, &fixture.task.task_revision, None, Some("Explicitly reviewed updated body")).unwrap();
    fixture.mutate(&Actor::Operator(OperatorOrigin::Browser), OrchestrationAction::IntentResolve { intent_id: intent.intent_id, apply: true }).unwrap();
    let r = fixture.run().retirement.unwrap();
    assert_eq!(r.trigger, RetirementTrigger::OperatorConflictResolution);
    assert_eq!(r.task_revision, current.task_revision);
    let fixture = Fixture::new(); let intent = fixture.intent();
    fixture.edit(|s| s.runs[1].result.as_mut().unwrap().message_id = "replacement".into());
    assert!(fixture.mutate(&Actor::Operator(OperatorOrigin::Browser), OrchestrationAction::IntentResolve { intent_id: intent.intent_id, apply: true }).is_err());
    assert!(fixture.run().retirement.is_none());
}

#[test]
fn native_receipt_is_exact_main_authority_and_never_rewrites_location() {
    for mismatch in 0..14 {
        let fixture = Fixture::new(); let retirement_id = fixture.offer();
        let mut caller = caller();
        match mismatch {
            0 => caller.agent_kind = Some(AgentKind::Subagent),
            1 => caller.process = None,
            2 => caller.process.as_mut().unwrap().pid += 1,
            3 => caller.process.as_mut().unwrap().start_ticks += 1,
            4 => caller.omp_session_id = Some("other".into()),
            5 => caller.endpoint_identity = "other".into(),
            6 => caller.terminal_id = Some("other".into()),
            7 => caller.pane_id = "moved".into(),
            8 => caller.workspace_id = "moved".into(),
            9 => caller.tab_id = "moved".into(),
            10 => caller.env_run = Some(("worker".into(), 8)),
            11 => caller.env_run = None,
            12 => caller.env_run = Some((ROOT.into(), 1)),
            _ => caller.actual_agent_kind = Some("other".into()),
        }
        let original = fixture.run().location.unwrap();
        assert!(fixture.mutate(&Actor::Agent(caller), receipt(&retirement_id, NativeStopReceipt::ShutdownRequested)).is_err());
        let after = fixture.run();
        assert!(matches!(after.retirement.unwrap().state, RetirementState::NativeStopOffered { .. }));
        assert!(same_launch_location(Some(&original), after.location.as_ref()));
    }
    let fixture = Fixture::new(); let retirement_id = fixture.offer();
    assert!(fixture.mutate(&Actor::Operator(OperatorOrigin::Browser), receipt(&retirement_id, NativeStopReceipt::ShutdownRequested)).is_err());
    assert!(fixture.mutate(&Actor::Agent(caller()), receipt("wrong-id", NativeStopReceipt::ShutdownRequested)).is_err());
    fixture.mutate(&Actor::Agent(caller()), receipt(&retirement_id, NativeStopReceipt::ShutdownRequested)).unwrap();
    assert!(matches!(fixture.run().retirement.unwrap().state, RetirementState::NativeStopRequested { .. }));
    assert!(fixture.mutate(&Actor::Agent(caller()), receipt(&retirement_id, NativeStopReceipt::ShutdownRequested)).is_err());
    assert!(fixture.mutate(&Actor::Agent(caller()), OrchestrationAction::Annotate { run_id: "worker".into(), text: "not admitted".into() }).is_err());
}

#[test]
fn deferred_preserves_offer_and_refusal_reason_is_typed_not_text() {
    let fixture = Fixture::new(); let retirement_id = fixture.offer();
    fixture.mutate(&Actor::Agent(caller()), receipt(&retirement_id, NativeStopReceipt::Deferred { reason: NativeDeferReason::EditorDraft })).unwrap();
    assert!(matches!(fixture.run().retirement.unwrap().state, RetirementState::NativeStopDeferred { offered_at, reason: NativeDeferReason::EditorDraft, .. } if offered_at == AT));
    fixture.mutate(&Actor::Agent(caller()), receipt(&retirement_id, NativeStopReceipt::Refused { reason: NativeRefuseReason::NativeRefused, text: "UserActivity user_activity".into() })).unwrap();
    assert!(matches!(fixture.run().retirement.unwrap().state, RetirementState::Retained { reason: RetainReason::NativeRefused, .. }));
    assert!(fixture.service.retirement_queue().unwrap().is_empty());
    let fixture = Fixture::new(); let retirement_id = fixture.offer();
    assert!(fixture.mutate(&Actor::Agent(caller()), receipt(&retirement_id, NativeStopReceipt::Refused { reason: NativeRefuseReason::UserActivity, text: "x".repeat(1025) })).is_err());
}

#[test]
fn cas_and_effect_authority_fence_launch_process_and_terminal_states() {
    for change in 0..7 {
        let fixture = Fixture::new(); fixture.accept(); let retirement_id = fixture.run().retirement.unwrap().retirement_id;
        assert!(fixture.service.record_retirement("worker", "wrong", RetirementStateKind::Waiting, RetirementState::NativeStopOffered { offered_at: AT.into() }).is_err());
        assert!(fixture.service.record_retirement("worker", &retirement_id, RetirementStateKind::NativeStopped, RetirementState::CloseIntent { at: AT.into() }).is_err());
        fixture.edit(|s| match change {
            0 => s.runs[1].attempt += 1,
            1 => s.runs[1].dispatch.as_mut().unwrap().launch_attempt += 1,
            2 => s.runs[1].bound_omp_process.as_mut().unwrap().start_ticks += 1,
            3 => s.runs[1].bound_omp_session = Some("other".into()),
            4 => s.runs[1].location.as_mut().unwrap().pane_id = "moved".into(),
            5 => s.runs[1].result.as_mut().unwrap().message_id = "other".into(),
            _ => s.runs[1].launch_shell_identity.as_mut().unwrap().executable_inode = "other".into(),
        });
        fixture.service.record_retirement("worker", &retirement_id, RetirementStateKind::Waiting, RetirementState::NativeStopOffered { offered_at: AT.into() }).unwrap();
        assert!(matches!(fixture.run().retirement.unwrap().state, RetirementState::Retained { reason: RetainReason::IdentityChanged, .. }));
        assert!(!fixture.service.retirement_effect_authorized("worker", &retirement_id).unwrap());
        assert!(fixture.service.record_retirement("worker", &retirement_id, RetirementStateKind::Retained, RetirementState::Waiting { blockers: vec![] }).is_err());
    }
    let fixture = Fixture::new(); let retirement_id = fixture.offer();
    fixture.mutate(&Actor::Agent(caller()), receipt(&retirement_id, NativeStopReceipt::ShutdownRequested)).unwrap();
    fixture.service.record_retirement("worker", &retirement_id, RetirementStateKind::NativeStopRequested,
        RetirementState::NativeStopped { at: AT.into(), evidence: NativeStopEvidence::ExitedAfterShutdownRequest }).unwrap();
    assert!(!fixture.service.retirement_effect_authorized("worker", &retirement_id).unwrap());
    fixture.service.record_retirement("worker", &retirement_id, RetirementStateKind::NativeStopped, RetirementState::CloseIntent { at: AT.into() }).unwrap();
    assert!(fixture.service.retirement_effect_authorized("worker", &retirement_id).unwrap());
    fixture.service.record_retirement("worker", &retirement_id, RetirementStateKind::CloseIntent,
        RetirementState::Unknown { at: AT.into(), phase: RetirementPhase::TerminalClose, detail: "uncertain".into() }).unwrap();
    assert!(!fixture.service.retirement_effect_authorized("worker", &retirement_id).unwrap());
    assert!(fixture.service.retirement_queue().unwrap().is_empty());
}

#[test]
fn descendants_and_telemetry_block_offer_and_shutdown_even_after_preflight() {
    let fixture = Fixture::new(); fixture.accept();
    fixture.edit(|s| {
        let mut child = new_run("child", SESSION, RunKind::Worker, "child".into(), ROOT, Some("worker".into()), None, 1);
        child.stage = RunStage::Active; s.runs.push(child);
        let grandchild = new_run("grandchild", SESSION, RunKind::Worker, "grandchild".into(), ROOT, Some("child".into()), None, 1);
        s.runs.push(grandchild);
        s.subagents.push(Subagent { run_id: "worker".into(), subagent_id: "child-native".into(), parent_subagent_id: None,
            bound_omp_session: None, role: None, label: "child".into(), status: SubagentStatus::Running, summary: None, last_control: None, updated_at: AT.into() });
    });
    assert_eq!(fixture.service.retirement_blockers("worker").unwrap(), vec![RetirementBlocker::OpenDescendantRuns, RetirementBlocker::RunningSubagents]);
    let retirement_id = fixture.run().retirement.unwrap().retirement_id;
    assert!(fixture.service.record_retirement("worker", &retirement_id, RetirementStateKind::Waiting, RetirementState::NativeStopOffered { offered_at: AT.into() }).is_err());
    fixture.edit(|s| { s.runs[2].stage = RunStage::Closed; s.runs[3].stage = RunStage::Closed; s.subagents[0].status = SubagentStatus::Done; });
    assert!(fixture.service.retirement_blockers("worker").unwrap().is_empty());
    fixture.service.record_retirement("worker", &retirement_id, RetirementStateKind::Waiting, RetirementState::NativeStopOffered { offered_at: AT.into() }).unwrap();
    fixture.edit(|s| s.subagents[0].status = SubagentStatus::Running);
    assert!(fixture.mutate(&Actor::Agent(caller()), receipt(&retirement_id, NativeStopReceipt::ShutdownRequested)).is_err());
}

#[test]
fn bind_stores_only_main_evidence_rebind_fences_and_retry_clears_it() {
    let fixture = Fixture::new();
    fixture.edit(|s| { s.runs[1].bound_omp_session = None; s.runs[1].bound_omp_process = None; s.runs[1].stage = RunStage::Preparing; });
    let mut subagent = caller(); subagent.agent_kind = Some(AgentKind::Subagent); subagent.subagent_id = Some("child".into());
    assert!(fixture.mutate(&Actor::Agent(subagent), OrchestrationAction::RunBindSession { omp_session_id: "native".into() }).is_err());
    assert!(fixture.run().bound_omp_process.is_none());
    fixture.mutate(&Actor::Agent(caller()), OrchestrationAction::RunBindSession { omp_session_id: "native".into() }).unwrap();
    assert_eq!(fixture.run().bound_omp_process, caller().process);
    let mut replaced = caller(); replaced.process.as_mut().unwrap().start_ticks += 1;
    assert_eq!(fixture.mutate(&Actor::Agent(replaced), OrchestrationAction::RunBindSession { omp_session_id: "native".into() }).unwrap_err().code, "session_mismatch");
    fixture.edit(|s| s.runs[1].dispatch.as_mut().unwrap().step = DispatchStep::LaunchUnknown);
    fixture.mutate(&Actor::Operator(OperatorOrigin::Browser), OrchestrationAction::RetryLaunch { run_id: "worker".into() }).unwrap();
    assert!(fixture.run().bound_omp_session.is_none());
    assert!(fixture.run().bound_omp_process.is_none());
    assert!(fixture.run().launch_shell_identity.is_none());
}

#[test]
fn terminal_classifier_allows_final_space_and_sibling_tabs_but_not_shared_tab() {
    let identity = identity(); let mut runtime = runtime(); let shell = shell();
    assert_eq!(classify_terminal(&identity, &runtime, &shell), Ok(()));
    let mut sibling = runtime.panes[0].clone(); sibling.pane_id = "user-pane".into(); sibling.terminal_id = Some("user-terminal".into()); sibling.tab_id = "user-tab".into();
    runtime.panes.push(sibling);
    assert_eq!(classify_terminal(&identity, &runtime, &shell), Ok(()));
    runtime.panes[1].tab_id = "tab".into();
    assert_eq!(classify_terminal(&identity, &runtime, &shell), Err(TerminalDecision::Retain(RetainReason::SharedTab)));
    runtime.panes.pop(); runtime.panes[0].tab_label = "renamed".into();
    assert_eq!(classify_terminal(&identity, &runtime, &shell), Err(TerminalDecision::Retain(RetainReason::TabRenamed)));
    runtime.panes[0].tab_label = "owned-launch".into();
    let mut busy = shell; busy.processes.push((22, "user command".into()));
    assert_eq!(classify_terminal(&identity, &runtime, &busy), Err(TerminalDecision::Retain(RetainReason::ForegroundProcess)));
}

#[test]
fn close_classifier_never_retries_unknown_and_requires_same_endpoint_absence() {
    let identity = identity(); let present = runtime(); let mut absent = runtime(); absent.panes.clear();
    assert!(matches!(classify_close(&identity, Some(Ok(())), Some(&absent)), RetirementState::Retired { terminal: TerminalOutcome::ClosedByCockpit, .. }));
    assert!(matches!(classify_close(&identity, None, Some(&absent)), RetirementState::Retired { terminal: TerminalOutcome::AbsentAfterUncertainClose, .. }));
    assert!(matches!(classify_close(&identity, None, Some(&present)), RetirementState::Unknown { phase: RetirementPhase::TerminalClose, .. }));
    assert!(matches!(classify_close(&identity, None, None), RetirementState::Unknown { .. }));
    assert!(matches!(classify_close(&identity, Some(Err(error("pane_not_found", "gone"))), Some(&absent)), RetirementState::Retired { terminal: TerminalOutcome::AlreadyAbsent, .. }));
    absent.endpoint_identity = "restarted".into();
    assert!(matches!(classify_close(&identity, Some(Ok(())), Some(&absent)), RetirementState::Unknown { .. }));
    assert!(matches!(classify_close(&identity, Some(Err(error("confirmation_required", "refused"))), None), RetirementState::Retained { reason: RetainReason::HerdrRefused, .. }));
}

#[test]
fn offer_requires_exact_process_in_pinned_pane_and_optional_evidence_is_honest() {
    let identity = identity(); let mut runtime = runtime();
    runtime.panes[0].agent_kind = Some("omp".into()); runtime.panes[0].native_session_id = Some("native".into());
    let mut info = shell(); info.processes = vec![(123, "omp".into())];
    assert_eq!(classify_offer(&identity, &runtime, Some(&info), true, Some("kernel")), OfferDecision::Offer);
    assert_eq!(classify_offer(&identity, &runtime, Some(&info), true, Some("other")), OfferDecision::Retain(RetainReason::IdentityChanged));
    info.processes.clear();
    assert_eq!(classify_offer(&identity, &runtime, Some(&info), true, Some("kernel")), OfferDecision::Retain(RetainReason::ProcessPaneMismatch));
    assert_eq!(classify_offer(&identity, &runtime, None, true, Some("kernel")), OfferDecision::Retain(RetainReason::NativeProcessUnverifiable));
    runtime.panes[0].agent_kind = None;
    assert_eq!(classify_offer(&identity, &runtime, None, false, Some("kernel")), OfferDecision::AlreadyExited);
    runtime.endpoint_identity = "restarted".into();
    assert_eq!(classify_offer(&identity, &runtime, None, false, Some("kernel")), OfferDecision::Retain(RetainReason::EndpointChanged));
}

#[test]
fn unknown_attention_is_visible_for_closed_runs_and_retention_is_not_recovery() {
    let fixture = Fixture::new();
    fixture.accept();
    fixture.edit(|s| s.runs[1].retirement.as_mut().unwrap().state = RetirementState::Unknown {
        at: AT.into(), phase: RetirementPhase::TerminalClose, detail: "uncertain close".into(),
    });
    let project = |state: &OrchestrationState| {
        let board = {
            let locked = fixture.service.store.lock().unwrap();
            let document = locked.tasks(ROOT).unwrap();
            TaskBoard {
                root_id: ROOT.into(),
                path: fixture.service.base().join("tasks").join(format!("{ROOT}.md"))
                    .to_string_lossy().into_owned(),
                doc_revision: document.doc_revision.clone(),
                unidentified_items: document.unidentified_items,
                diagnostics: Vec::new(),
                tasks: document.tasks.iter().cloned().map(|task| TaskView {
                    task,
                    lane: TaskLane::Queued,
                    current_run_id: None,
                    dependencies: TaskDependencies { state: TaskDependencyState::None, unmet: Vec::new(), problems: Vec::new() },
                }).collect(),
            }
        };
        projection::snapshot(
            &OrchestrationSnapshotRequest { session_id: SESSION.into(), root_id: None },
            state, "token".into(), vec![board], Err(error("unavailable", "offline")), &now(),
        ).unwrap()
    };
    let state = fixture.state();
    let view = project(&state);
    let attention = view.attention.iter().find(|a| a.kind == AttentionKind::RetirementUnconfirmed).unwrap();
    assert_eq!(attention.run_id.as_deref(), Some("worker"));
    assert_eq!(attention.since, AT);
    let retirement_id = fixture.run().retirement.unwrap().retirement_id;
    assert!(fixture.service.record_retirement("worker", &retirement_id, RetirementStateKind::Unknown,
        RetirementState::CloseIntent { at: AT.into() }).is_err());
    fixture.edit(|s| s.runs[1].retirement.as_mut().unwrap().state = RetirementState::Retained {
        at: AT.into(), reason: RetainReason::SharedTab, native_stopped: true,
    });
    assert!(!project(&fixture.state()).attention.iter().any(|a| a.kind == AttentionKind::RetirementUnconfirmed));
}

#[test]
fn closed_receipt_carve_out_does_not_admit_any_ordinary_action() {
    let fixture = Fixture::new();
    fixture.offer();
    let actions = vec![
        OrchestrationAction::TaskCreate { root_id: ROOT.into(), task_id: uuid::Uuid::new_v4().to_string(),
            title: "x".into(), description: "x".into(), depends_on: Vec::new(), follow_up_of: None,
            expected_doc_revision: None, source_revision: None },
        OrchestrationAction::TaskAssign { root_id: ROOT.into(), task_id: fixture.task.task_id.clone(), title: "x".into(), description: "x".into() },
        OrchestrationAction::TaskAssignmentResolve { root_id: ROOT.into(), task_id: fixture.task.task_id.clone(), expected_task_revision: None, assign: true },
        OrchestrationAction::TaskUpdate { root_id: ROOT.into(), task_id: fixture.task.task_id.clone(), expected_task_revision: fixture.task.task_revision.clone(), title: None, description: None },
        OrchestrationAction::TasksAssignIds { root_id: ROOT.into(), expected_doc_revision: "revision".into() },
        OrchestrationAction::SupervisorStart { target: None, label: None },
        OrchestrationAction::RunBindSession { omp_session_id: "native".into() },
        OrchestrationAction::RunAdopt { label: "x".into() },
        OrchestrationAction::RunPropose { task_id: fixture.task.task_id.clone(), parent_run_id: None, label: None,
            target: DispatchTarget::ExistingSpace { workspace_id: "space".into() }, prepare_brief: "x".into(), supersedes_run_id: None },
        OrchestrationAction::GrantPrepare { run_id: "worker".into(), plan_revision: "revision".into() },
        OrchestrationAction::GrantExecute { run_id: "worker".into(), plan_revision: "revision".into(), note: None },
        OrchestrationAction::Accept { run_id: "worker".into(), expected_task_revision: fixture.task.task_revision.clone() },
        OrchestrationAction::SendBack { run_id: "worker".into(), text: "x".into() },
        OrchestrationAction::CancelRun { run_id: "worker".into() },
        OrchestrationAction::RetryLaunch { run_id: "worker".into() },
        OrchestrationAction::ReconcileRun { run_id: "worker".into(), recovery: None },
        OrchestrationAction::IntentResolve { intent_id: "x".into(), apply: true },
        OrchestrationAction::MessageSend { message_id: id(), to_run_id: ROOT.into(), kind: MessageKind::Instruction, text: "x".into(), in_reply_to: None },
        OrchestrationAction::Annotate { run_id: "worker".into(), text: "x".into() },
        OrchestrationAction::InboxPull { after_seq: 0, limit: 10 },
        OrchestrationAction::InboxWoken { through_seq: 0, omp_session_id: "native".into() },
        OrchestrationAction::InboxAck { through_seq: 0 },
        OrchestrationAction::SubagentUpdate { subagent_id: "child".into(), parent_subagent_id: None, role: None,
            label: "child".into(), status: SubagentStatus::Running, summary: None },
        OrchestrationAction::SubagentControl { run_id: "worker".into(), subagent_id: "child".into(), op: SubagentOp::Cancel },
        OrchestrationAction::SubagentControlDone { seq: 0, applied: true, error: None },
    ];
    for action in actions {
        let before = serde_json::to_value(fixture.state()).unwrap();
        assert!(fixture.mutate(&Actor::Agent(caller()), action.clone()).is_err(), "{action:?}");
        assert_eq!(serde_json::to_value(fixture.state()).unwrap(), before);
    }
    // Reporting retains its pre-existing stale-history exception, not live authority.
    fixture.mutate(&Actor::Agent(caller()), OrchestrationAction::Report {
        message_id: id(), kind: ReportKind::Progress, outcome: None,
        summary: "late history".into(), plan: None, to_run_id: None,
    }).unwrap();
    assert!(fixture.state().messages.last().unwrap().stale);
    assert_eq!(fixture.run().result.unwrap().message_id, "reviewed-result");
}

#[test]
fn failed_atomic_save_persists_neither_closed_stage_nor_retirement() {
    let fixture = Fixture::new();
    let locked = fixture.service.store.lock().unwrap();
    let mut state = locked.read().unwrap();
    close_accepted(&mut state, 1, RetirementTrigger::Accept, &fixture.task.task_revision);
    assert!(state.runs[1].retirement.is_some());
    state.runs[1].annotations.push(Annotation { at: AT.into(), by: ActorRef::Dispatcher,
        text: "x".repeat(store::MAX_STATE_BYTES) });
    assert_eq!(locked.save(&mut state).unwrap_err().code, "orchestration_state_full");
    let durable = locked.read().unwrap();
    assert_eq!(durable.runs[1].stage, RunStage::Reported);
    assert!(durable.runs[1].close_reason.is_none());
    assert!(durable.runs[1].retirement.is_none());
}

#[test]
fn cancellation_has_no_retirement_record_annotation_or_receipt_authority() {
    let fixture = Fixture::new();
    fixture.mutate(&Actor::Operator(OperatorOrigin::Browser), OrchestrationAction::CancelRun { run_id: "worker".into() }).unwrap();
    let run = fixture.run();
    assert_eq!(run.close_reason, Some(CloseReason::Cancelled));
    assert!(run.retirement.is_none());
    assert!(!run.annotations.iter().any(|a| a.text.to_lowercase().contains("retirement")));
    assert!(fixture.mutate(&Actor::Agent(caller()), receipt("absent", NativeStopReceipt::ShutdownRequested)).is_err());
}

#[test]
fn receipt_interleaving_fences_owner_cas_and_execution_lease_is_exclusive() {
    let fixture = Fixture::new();
    let retirement_id = fixture.offer();
    let lease = fixture.service.execution_lease(ROOT).unwrap().unwrap();
    assert!(fixture.service.execution_lease(ROOT).unwrap().is_none());
    fixture.mutate(&Actor::Agent(caller()), receipt(&retirement_id, NativeStopReceipt::Deferred { reason: NativeDeferReason::Busy })).unwrap();
    assert!(fixture.service.record_retirement("worker", &retirement_id, RetirementStateKind::NativeStopOffered,
        RetirementState::Retained { at: AT.into(), reason: RetainReason::WorkerUnresponsive, native_stopped: false }).is_err());
    drop(lease);
    assert!(fixture.service.execution_lease(ROOT).unwrap().is_some());
}

#[test]
fn terminal_preflight_fails_closed_on_every_identity_and_shell_mismatch() {
    let identity = identity();
    for contamination in 0..10 {
        let mut runtime = runtime();
        let mut info = shell();
        let reason = match contamination {
            0 => { runtime.endpoint_identity = "restarted".into(); RetainReason::EndpointChanged },
            1 => { runtime.boot_id = Some("rebooted".into()); RetainReason::IdentityChanged },
            2 => { runtime.panes[0].terminal_id = Some("replacement".into()); RetainReason::PaneMoved },
            3 => { runtime.panes[0].workspace_id = "moved".into(); RetainReason::PaneMoved },
            4 => { runtime.panes[0].tab_id = "moved".into(); RetainReason::PaneMoved },
            5 => { runtime.panes[0].agent_kind = Some("omp".into()); RetainReason::ForegroundProcess },
            6 => { runtime.panes[0].launch_pending = true; RetainReason::ForegroundProcess },
            7 => { info.shell_pid = None; RetainReason::ForegroundProcess },
            8 => { info.foreground_pgid = Some(22); RetainReason::ForegroundProcess },
            _ => { info.processes.clear(); RetainReason::ForegroundProcess },
        };
        assert_eq!(classify_terminal(&identity, &runtime, &info), Err(TerminalDecision::Retain(reason)));
    }
    let mut moved = runtime();
    moved.panes[0].pane_id = "moved".into();
    assert_eq!(classify_terminal(&identity, &moved, &shell()), Err(TerminalDecision::Retain(RetainReason::PaneMoved)));
    moved.panes.clear();
    assert_eq!(classify_terminal(&identity, &moved, &shell()), Err(TerminalDecision::Absent));
    assert!(OBSERVATION_RETRY_SECS >= 30);
}

#[test]
fn legacy_intents_accept_but_missing_reviewed_result_identity_retains_worker() {
    for operator_resolution in [false, true] {
        let fixture = Fixture::new();
        let intent = fixture.intent();
        fixture.edit(|s| s.task_intents[0].result_message_id = None);
        if operator_resolution {
            fixture.mutate(&Actor::Operator(OperatorOrigin::Browser),
                OrchestrationAction::IntentResolve { intent_id: intent.intent_id, apply: true }).unwrap();
        } else {
            fixture.service.recover_intents().unwrap();
        }
        let run = fixture.run();
        assert_eq!(run.stage, RunStage::Closed);
        assert_eq!(run.close_reason, Some(CloseReason::Accepted));
        let retirement = run.retirement.unwrap();
        assert_eq!(retirement.result_message_id, "reviewed-result");
        assert!(retirement.identity.is_none());
        assert!(matches!(retirement.state, RetirementState::Retained {
            reason: RetainReason::IdentityIncomplete, native_stopped: false, ..
        }));
        assert!(fixture.service.retirement_queue().unwrap().is_empty());
        assert!(fixture.mutate(&Actor::Agent(caller()),
            receipt(&retirement.retirement_id, NativeStopReceipt::ShutdownRequested)).is_err());
        assert!(fixture.service.store.lock().unwrap().tasks(ROOT).unwrap().task(&fixture.task.task_id).unwrap().checked);
    }
}

#[test]
fn exec_replacement_at_identical_pid_and_pgid_cannot_authorize_close() {
    let identity = identity();
    let runtime = runtime();
    let original_shell = shell();
    assert_eq!(classify_terminal(&identity, &runtime, &original_shell), Ok(()));
    for replacement in 0..3 {
        let mut exec = original_shell.clone();
        let current = exec.shell_identity.as_mut().unwrap();
        match replacement {
            // exec sleep: executable changed, original shell PID/start/PGID remain.
            0 => current.executable_inode = "3".into(),
            // exec bash -c: same executable, distinct complete argv fingerprint.
            1 => current.argv_digest = "b".repeat(64),
            // Same executable/argv on a different filesystem is not the baseline.
            _ => current.executable_device = "4".into(),
        }
        assert_eq!(exec.shell_pid, original_shell.shell_pid);
        assert_eq!(exec.foreground_pgid, original_shell.foreground_pgid);
        assert_eq!(classify_terminal(&identity, &runtime, &exec),
            Err(TerminalDecision::Retain(RetainReason::ForegroundProcess)));
    }
    let mut unobserved = original_shell;
    unobserved.shell_identity = None;
    assert_eq!(classify_terminal(&identity, &runtime, &unobserved),
        Err(TerminalDecision::Retain(RetainReason::NativeProcessUnverifiable)));
}

#[test]
fn original_shell_builtin_activity_is_permitted_without_claiming_idle_proof() {
    let identity = identity();
    let mut runtime = runtime();
    let original_shell = shell();
    let mut sibling = runtime.panes[0].clone();
    sibling.tab_id = "user-tab".into();
    sibling.pane_id = "user-pane".into();
    sibling.terminal_id = Some("user-terminal".into());
    runtime.panes.push(sibling);

    // A prompt and `while :; do :; done` in the original launch shell can have
    // identical PID/PGID/executable/argv evidence. The owned-pane policy permits
    // both after native exit; this classifier does not claim shell idleness.
    let indistinguishable_builtin = original_shell.clone();
    assert_eq!(classify_terminal(&identity, &runtime, &original_shell), Ok(()));
    assert_eq!(classify_terminal(&identity, &runtime, &indistinguishable_builtin), Ok(()));
    assert_eq!(runtime.panes[1].pane_id, "user-pane");
    assert_eq!(runtime.panes[1].tab_id, "user-tab");
    assert_eq!(runtime.panes.len(), 2);
}
