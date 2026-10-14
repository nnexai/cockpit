use super::*;
use super::retirement::*;
use super::herdr::{PaneProcessInfo, RuntimePane, RuntimeView};
use std::path::PathBuf;

const SESSION: &str = "retirement-tests";
const AT: &str = "2026-10-07T01:00:00Z";
const ROOT: &str = "12345678-1234-4234-8234-123456789abc";

struct Fixture { path: PathBuf, service: OrchestrationService, task: Task, auth: JobAuth }
impl Fixture {
    fn new() -> Self {
        let path = std::env::temp_dir().join(format!("cockpit-retirement-{}", id()));
        let store = OrchestrationStore::open(&path).unwrap();
        let (revision, _) = watch::channel(0);
        let service = OrchestrationService { store, revision, notifications: std::sync::Mutex::new(None), launch_config: Default::default() };
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
        worker.task_revision_at_start = Some(task.task_revision.clone());
        let source_output = SourceOutput::NonCode {
            result_message_id: "reviewed-result".into(), task_revision: task.task_revision.clone(),
            artifacts: Vec::new(), focused_checks: Vec::new(),
        };
        worker.source_output = Some(source_output.clone());
        worker.result.as_mut().unwrap().source_output = Some(source_output);
        let (mut credential, credential_file) = credentials::stage_credential(
            service.base(), &worker, credentials::JobRole::Worker,
        ).unwrap();
        credential.conversation = Some(conversation());
        let auth = credentials::read_job_auth(&credential_file).unwrap();
        state.runs.push(worker);
        let (_, checkout) = project_identity::resolve_owned_directory(&path).unwrap();
        let holder = claims::AttachmentReference { run_id: state.runs[1].run_id.clone(),
            attachment_id: auth.attachment_id.clone(), attachment_epoch: auth.attachment_epoch };
        claims::reserve_claim(&mut state, &checkout, &holder, &id()).unwrap();
        state.credentials.push(credential);
        state.task_ownership.push(TaskOwnership {
            root_id: ROOT.into(), task_id: task.task_id.clone(), generation: 1, schedule: None,
            owner: Some(TaskOwner {
                run_id: state.runs[1].run_id.clone(),
                conversation: Some(conversation()), claim_operation_id: id(),
            }),
        });
        locked.save(&mut state).unwrap();
        drop(locked);
        Self { path, service, task, auth }
    }
    fn state(&self) -> OrchestrationState { self.service.store.lock().unwrap().read().unwrap() }
    fn run(&self) -> Run { self.state().runs.into_iter().find(|r| r.run_id == "12345678-1234-4234-8234-123456789abd").unwrap() }
    fn caller(&self) -> AgentCaller { caller(self.auth.clone()) }
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
            run_id: "12345678-1234-4234-8234-123456789abd".into(), expected_task_revision: self.task.task_revision.clone(),
        }).unwrap();
    }
    fn intent(&self) -> TaskIntent {
        let expected_task_revision = {
            let locked = self.service.store.lock().unwrap();
            locked.tasks(ROOT).unwrap().task(&self.task.task_id).unwrap().task_revision.clone()
        };
        let result_message_id = self.run().result.map(|report| report.message_id);
        let intent = TaskIntent { intent_id: id(), root_id: ROOT.into(), task_id: self.task.task_id.clone(),
            run_id: "12345678-1234-4234-8234-123456789abd".into(), expected_task_revision, state: IntentState::Pending,
            origin: Some(DecisionOrigin::Browser), supervisor_run_id: None, conversation: None,
            result_message_id };
        self.edit(|s| s.task_intents.push(intent.clone()));
        intent
    }
}
impl Drop for Fixture { fn drop(&mut self) { let _ = std::fs::remove_dir_all(&self.path); } }

fn worker() -> Run {
    let mut run = new_run("12345678-1234-4234-8234-123456789abd", SESSION, RunKind::Worker, "12345678-1234-4234-8234-123456789abd".into(), ROOT, Some(ROOT.into()), None, 7);
    run.stage = RunStage::Reported;
    run.assignment_generation = Some(1);
    run.cleanup_policy = Some(WorkerCleanupPolicy::OwnedWhenSafe);
    run.attachment = Some(ExecutionAttachment {
        attachment_id: "12345678-1234-4234-8234-123456789abe".into(), epoch: 1,
        state: AttachmentState::Connected, location: None, conversation: Some(conversation()),
        launch_mode: Some(AttachmentLaunchMode::ShellAgent),
        process: None, shell: None, machine_id: Some("test-machine".into()), launch_attempt: 3,
        process_scope: None,
        updated_at: AT.into(),
    });
    run.dispatch = Some(DispatchState { launch_tag: Some("owned-launch".into()), endpoint_identity: Some("endpoint".into()),
        scope_unit: None, pre_move_scope: None, latch: None, latch_state: None, source_location: None,
        move_operation_id: None, staging_workspace_id: None,
        recovery: None, agent_started: true, step: DispatchStep::Launched, launch_attempt: 3, error: None, updated_at: AT.into() });
    run.attachment.as_mut().unwrap().location = Some(RunLocation { boot_id: Some("herdr-boot".into()), terminal_id: Some("terminal".into()),
        native_session_id: Some("native".into()), endpoint_identity: "endpoint".into(), session_id: SESSION.into(),
        peer_process: None,
        workspace_id: "space".into(), tab_id: "tab".into(), pane_id: "pane".into(), launch_tag: "owned-launch".into() });
    run.attachment.as_mut().unwrap().process = Some(NativeProcessIdentity { pid: 123, start_ticks: 456, kernel_boot_id: Some("kernel".into()) });
    run.attachment.as_mut().unwrap().shell = Some(shell_identity());
    let source_output = SourceOutput::NonCode {
        result_message_id: "reviewed-result".into(), task_revision: "revision".into(),
        artifacts: Vec::new(), focused_checks: Vec::new(),
    };
    run.source_output = Some(source_output.clone());
    run.result = Some(Report { message_id: "reviewed-result".into(), kind: ReportKind::Result,
        outcome: Some(ReportOutcome::Succeeded), summary: "Verified evidence".into(), plan: None,
        source_output: Some(source_output), at: AT.into() });
    run
}
fn conversation() -> ConversationIdentity {
    ConversationIdentity { harness: "omp".into(), session_id: "retirement-chat".into() }
}
fn caller(auth: JobAuth) -> AgentCaller {
    AgentCaller { auth, conversation: Some(conversation()), assignment_generation: Some(1),
        endpoint_identity: "endpoint".into(), session_id: SESSION.into(), workspace_id: "space".into(),
        tab_id: "tab".into(), pane_id: "pane".into(), boot_id: Some("herdr-boot".into()), terminal_id: Some("terminal".into()),
        native_session_id: Some("native".into()), actual_agent_kind: Some(NativeAgentKind::Omp),
        process: worker().attachment.unwrap().process }
}
fn runtime() -> RuntimeView {
    RuntimeView { endpoint_identity: "endpoint".into(), boot_id: Some("herdr-boot".into()), peer_process: None, focused_workspace_id: None, workspaces: vec![], panes: vec![RuntimePane {
        workspace_id: "space".into(), workspace_label: "Space".into(), tab_id: "tab".into(), tab_label: "owned-launch".into(),
        pane_id: "pane".into(), terminal_id: Some("terminal".into()), native_session_id: None, agent_name: None,
        native_session_file: None,
        cwd: None,
        agent_kind: None, launch_pending: false, agent_status: None, state_changed_at: None,
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
fn direct_terminal() -> (RetirementIdentity, RuntimeView, PaneProcessInfo) {
    let mut identity = identity();
    identity.launch_mode = Some(AttachmentLaunchMode::DirectProcess);
    let mut shell = identity.shell.clone().unwrap();
    shell.process = identity.process.clone().unwrap();
    identity.shell = Some(shell.clone());
    let mut runtime = runtime();
    runtime.panes[0].agent_kind = Some("omp".into());
    runtime.panes[0].agent_status = Some("idle".into());
    let info = PaneProcessInfo { pane_id: "pane".into(), shell_pid: Some(shell.process.pid),
        foreground_pgid: Some(shell.process.pid), processes: vec![(shell.process.pid, "agent".into())],
        shell_identity: Some(shell) };
    (identity, runtime, info)
}

#[test]
fn acceptance_commits_exact_receipt_revision_and_launch_identity() {
    let fixture = Fixture::new();
    assert!(fixture.service.retirement_queue().unwrap().is_empty());
    fixture.accept();
    let run = fixture.run();
    assert_eq!(run.stage, RunStage::Closed);
    assert_eq!(run.close_reason, Some(CloseReason::Accepted));
    let r = run.retirement.unwrap();
    assert_eq!(r.result_message_id.as_deref(), Some("reviewed-result"));
    assert_eq!(r.task_revision.as_deref(), Some(fixture.task.task_revision.as_str()));
    assert_eq!(r.trigger, RetirementTrigger::Accept);
    assert_eq!(r.identity.unwrap(), identity());
    assert_eq!(fixture.service.retirement_queue().unwrap().len(), 1);
    assert!(run.annotations.iter().any(|a| matches!(a.by, ActorRef::Dispatcher)));
    assert!(fixture.service.store.lock().unwrap().tasks(ROOT).unwrap().task(&fixture.task.task_id).unwrap().checked);
}

#[test]
fn acceptance_stops_durable_schedule_without_discarding_attempt_or_owner() {
    let fixture = Fixture::new();
    fixture.edit(|state| state.task_ownership[0].schedule = Some(TaskSchedule {
        operation_id: id(), principal_key: "root".into(), target: None,
        brief: "scheduled contract".into(), cleanup_policy: WorkerCleanupPolicy::OwnedWhenSafe,
        label: None, scheduled_at: now(),
    }));
    fixture.accept();
    let state = fixture.state();
    assert!(state.task_ownership[0].schedule.is_none());
    assert_eq!(state.task_ownership[0].generation, 1);
    assert_eq!(state.task_ownership[0].owner.as_ref().unwrap().run_id, fixture.run().run_id);
    assert!(state.runs.iter().any(|run| run.run_id == fixture.run().run_id && run.result.is_some()));
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
            run_id: "12345678-1234-4234-8234-123456789abd".into(), expected_task_revision: if invalid == 2 { "wrong-revision".into() } else { fixture.task.task_revision.clone() },
        });
        assert!(result.is_err());
        assert!(fixture.run().retirement.is_none());
    }
    let fixture = Fixture::new();
    fixture.mutate(&Actor::Operator(OperatorOrigin::Browser), OrchestrationAction::CancelRun { run_id: "12345678-1234-4234-8234-123456789abd".into() }).unwrap();
    assert!(fixture.run().retirement.is_none());
    assert!(fixture.service.retirement_queue().unwrap().is_empty());
    let mut state = OrchestrationState::default();
    let mut run = worker(); run.stage = RunStage::Closed; run.close_reason = Some(CloseReason::Accepted);
    state.runs.push(run);
    close_accepted(&mut state, 0, RetirementTrigger::AcceptRecovery, "old-revision");
    assert!(state.runs[0].retirement.is_none());
}

#[test]
fn incomplete_physical_evidence_is_retained_and_roots_are_excluded() {
    for missing in 0..7 {
        let mut run = worker();
        match missing {
            0 => run.attachment.as_mut().unwrap().location.as_mut().unwrap().terminal_id = None,
            1 => run.dispatch.as_mut().unwrap().launch_tag = Some("other".into()),
            2 => run.dispatch.as_mut().unwrap().endpoint_identity = Some("other".into()),
            3 => run.attachment.as_mut().unwrap().location = None,
            4 => run.dispatch.as_mut().unwrap().launch_attempt = 0,
            5 => run.attachment.as_mut().unwrap().process = None,
            _ => run.attachment.as_mut().unwrap().shell = None,
        }
        let r = retirement_for_accepted(&run, RetirementTrigger::Accept, "revision", AT).unwrap();
        assert!(matches!(r.state, RetirementState::Retained {
            reason: RetainReason::IdentityIncomplete, stopped: false, ..
        }), "incomplete physical proof must retain the worker without claiming it stopped");
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
        assert_eq!(r.result_message_id.as_deref(), Some("reviewed-result"));
        assert_eq!(r.task_revision.as_deref(), Some(fixture.task.task_revision.as_str()));
        let revision = fixture.state().revision;
        fixture.service.recover_intents().unwrap();
        assert_eq!(fixture.state().revision, revision);
        assert_eq!(fixture.run().retirement.unwrap().retirement_id, r.retirement_id);
    }
    for conflict in 0..3 {
        let fixture = Fixture::new(); fixture.intent();
        match conflict {
            0 => { fixture.service.store.lock().unwrap().tasks(ROOT).unwrap().update(&fixture.task.task_id, &fixture.task.task_revision, None, Some("Changed body")).unwrap(); },
            1 => fixture.edit(|s| s.runs[1].result.as_mut().unwrap().message_id = "replacement-result".into()),
            _ => fixture.edit(|s| s.task_intents[0].result_message_id = None),
        }
        fixture.service.recover_intents().unwrap();
        assert!(fixture.run().retirement.is_none());
        assert_eq!(fixture.state().task_intents[0].state, IntentState::Conflict);
        assert!(!fixture.service.store.lock().unwrap().tasks(ROOT).unwrap().task(&fixture.task.task_id).unwrap().checked);
    }
}

#[test]
fn operator_resolution_uses_current_revision_but_not_a_different_result() {
    let fixture = Fixture::new(); let intent = fixture.intent();
    let current = fixture.service.store.lock().unwrap().tasks(ROOT).unwrap().update(&fixture.task.task_id, &fixture.task.task_revision, None, Some("Explicitly reviewed updated body")).unwrap();
    assert_eq!(fixture.mutate(&Actor::Operator(OperatorOrigin::Browser),
        OrchestrationAction::IntentResolve { intent_id: intent.intent_id.clone(), apply: true }).unwrap_err().code,
        "source_output_conflict");
    assert!(fixture.run().retirement.is_none());
    fixture.mutate(&Actor::Operator(OperatorOrigin::Browser),
        OrchestrationAction::IntentResolve { intent_id: intent.intent_id, apply: false }).unwrap();
    fixture.mutate(&Actor::Operator(OperatorOrigin::Browser),
        OrchestrationAction::SendBack { run_id: fixture.run().run_id, text: "Report against the reviewed current scope".into() }).unwrap();
    fixture.mutate(&Actor::Agent(fixture.caller()), OrchestrationAction::Report {
        message_id: "fresh-result".into(), kind: ReportKind::Result, outcome: Some(ReportOutcome::Succeeded),
        summary: "Current non-code output".into(), plan: None,
        source_output: Some(SourceOutput::NonCode {
            result_message_id: "fresh-result".into(), task_revision: current.task_revision.clone(),
            artifacts: Vec::new(), focused_checks: Vec::new(),
        }), to_run_id: None, original_task_revision: None, original_claim_id: None,
    }).unwrap();
    let fresh_intent = fixture.intent();
    fixture.mutate(&Actor::Operator(OperatorOrigin::Browser),
        OrchestrationAction::IntentResolve { intent_id: fresh_intent.intent_id, apply: true }).unwrap();
    let r = fixture.run().retirement.unwrap();
    assert_eq!(r.trigger, RetirementTrigger::OperatorConflictResolution);
    assert_eq!(r.task_revision.as_deref(), Some(current.task_revision.as_str()));
    for missing_result in [false, true] {
        let fixture = Fixture::new(); let intent = fixture.intent();
        if missing_result {
            fixture.edit(|s| s.task_intents[0].result_message_id = None);
        } else {
            fixture.edit(|s| s.runs[1].result.as_mut().unwrap().message_id = "replacement".into());
        }
        assert!(fixture.mutate(&Actor::Operator(OperatorOrigin::Browser), OrchestrationAction::IntentResolve { intent_id: intent.intent_id, apply: true }).is_err());
        assert!(fixture.run().retirement.is_none());
        assert!(!fixture.service.store.lock().unwrap().tasks(ROOT).unwrap().task(&fixture.task.task_id).unwrap().checked);
    }
}


#[test]
fn cas_and_effect_authority_fence_launch_and_terminal_states() {
    for change in 0..6 {
        let fixture = Fixture::new();
        fixture.accept();
        let run_id = fixture.run().run_id;
        let retirement_id = fixture.run().retirement.unwrap().retirement_id;
        assert!(fixture.service.record_retirement(&run_id, "wrong", RetirementStateKind::Waiting,
            RetirementState::CloseIntent { at: AT.into() }).is_err());
        assert!(fixture.service.record_retirement(&run_id, &retirement_id, RetirementStateKind::Retired,
            RetirementState::CloseIntent { at: AT.into() }).is_err());
        fixture.edit(|s| match change {
            0 => s.runs[1].attempt += 1,
            1 => s.runs[1].dispatch.as_mut().unwrap().launch_attempt += 1,
            2 => s.runs[1].attachment.as_mut().unwrap().process.as_mut().unwrap().start_ticks += 1,
            3 => s.runs[1].attachment.as_mut().unwrap().epoch += 1,
            4 => s.runs[1].attachment.as_mut().unwrap().location.as_mut().unwrap().pane_id = "moved".into(),
            _ => s.runs[1].result.as_mut().unwrap().message_id = "other".into(),
        });
        fixture.service.record_retirement(&run_id, &retirement_id, RetirementStateKind::Waiting,
            RetirementState::CloseIntent { at: AT.into() }).unwrap();
        assert!(matches!(fixture.run().retirement.unwrap().state,
            RetirementState::Retained { reason: RetainReason::IdentityChanged, .. }));
        assert!(fixture.service.record_retirement(&run_id, &retirement_id, RetirementStateKind::Retained,
            RetirementState::Waiting { blockers: vec![] }).is_err());
    }
}

#[test]
fn open_descendant_runs_block_close_and_closed_history_does_not() {
    let fixture = Fixture::new(); fixture.accept();
    fixture.edit(|s| {
        let child = new_run("child", SESSION, RunKind::Worker, "child".into(), ROOT,
            Some(s.runs[1].run_id.clone()), None, 1);
        s.runs.push(child);
    });
    let run_id = fixture.run().run_id;
    assert_eq!(fixture.service.retirement_blockers(&run_id).unwrap(),
        vec![RetirementBlocker::OpenDescendantRuns]);
    fixture.edit(|s| s.runs[2].stage = RunStage::Closed);
    assert!(fixture.service.retirement_blockers(&run_id).unwrap().is_empty());
}

#[test]
fn terminal_classifier_preserves_live_shells_and_excludes_shared_tabs() {
    let identity = identity(); let mut runtime = runtime(); let shell = shell();
    assert_eq!(classify_terminal(&identity, &runtime, &shell), Err(TerminalDecision::Retain(RetainReason::UserActivity)));
    let mut sibling = runtime.panes[0].clone(); sibling.pane_id = "user-pane".into(); sibling.terminal_id = Some("user-terminal".into()); sibling.tab_id = "user-tab".into();
    runtime.panes.push(sibling);
    assert_eq!(classify_terminal(&identity, &runtime, &shell), Err(TerminalDecision::Retain(RetainReason::UserActivity)));
    runtime.panes[1].tab_id = "tab".into();
    assert_eq!(classify_terminal(&identity, &runtime, &shell), Err(TerminalDecision::Retain(RetainReason::SharedTab)));
    runtime.panes.pop(); runtime.panes[0].tab_label = "renamed".into();
    assert_eq!(classify_terminal(&identity, &runtime, &shell), Err(TerminalDecision::Retain(RetainReason::TabRenamed)));
    runtime.panes[0].tab_label = "owned-launch".into();
    let mut busy = shell; busy.processes.push((22, "user command".into()));
    assert_eq!(classify_terminal(&identity, &runtime, &busy), Err(TerminalDecision::Retain(RetainReason::UserActivity)));
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
fn primary_checkout_close_refusal_is_specific_even_without_fresh_observation() {
    let identity = identity();
    let present = runtime();
    let mut absent = runtime();
    absent.panes.clear();
    for fresh in [None, Some(&present), Some(&absent)] {
        assert!(matches!(classify_close(&identity,
            Some(Err(error("foreign_space_group_close_risk", "opaque adapter diagnostic"))), fresh),
            RetirementState::Retained { reason: RetainReason::PrimaryCheckoutSpace, stopped: false, .. }));
    }
    assert!(matches!(classify_close(&identity,
        Some(Err(error("foreign_space_group_close_risk_other", "opaque adapter diagnostic"))), None),
        RetirementState::Retained { reason: RetainReason::HerdrRefused, stopped: false, .. }));
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
                    lane: TaskLane::Ready,
                    current_run_id: None,
                    scheduled: false,
                    attempt: None,
                    dependencies: TaskDependencies { state: TaskDependencyState::None, unmet: Vec::new(), problems: Vec::new() },
                }).collect(),
            }
        };
        projection::snapshot(
            &OrchestrationSnapshotRequest { session_id: SESSION.into(), root_id: None },
            state, "token".into(), vec![board], Err(error("unavailable", "offline")), &now(),
            "fixture-store",
        ).unwrap()
    };
    let state = fixture.state();
    let view = project(&state);
    let attention = view.attention.iter().find(|a| a.kind == AttentionKind::RetirementUnconfirmed).unwrap();
    assert_eq!(attention.run_id.as_deref(), Some("12345678-1234-4234-8234-123456789abd"));
    assert_eq!(attention.since, AT);
    let retirement_id = fixture.run().retirement.unwrap().retirement_id;
    assert!(fixture.service.record_retirement("12345678-1234-4234-8234-123456789abd", &retirement_id, RetirementStateKind::Unknown,
        RetirementState::CloseIntent { at: AT.into() }).is_err());
    fixture.edit(|s| s.runs[1].retirement.as_mut().unwrap().state = RetirementState::Retained {
        at: AT.into(), reason: RetainReason::SharedTab, stopped: false,
    });
    assert!(!project(&fixture.state()).attention.iter().any(|a| a.kind == AttentionKind::RetirementUnconfirmed));
}

#[test]
fn closed_worker_cannot_append_messages_or_register_again() {
    let fixture = Fixture::new();
    fixture.accept();
    for action in [
        OrchestrationAction::Report {
            message_id: id(), kind: ReportKind::Progress, outcome: None,
            summary: "late history".into(), plan: None, source_output: None,
            to_run_id: None, original_task_revision: None, original_claim_id: None,
        },
        OrchestrationAction::RunRegister { operation_id: id() },
        OrchestrationAction::InboxPull { after_seq: 0, limit: 10 },
        OrchestrationAction::InboxAck { through_seq: 0 },
        OrchestrationAction::Annotate { run_id: fixture.run().run_id, text: "late annotation".into() },
    ] {
        let before = std::fs::read(fixture.service.base().join("state.json")).unwrap();
        assert!(fixture.mutate(&Actor::Agent(fixture.caller()), action.clone()).is_err(), "{action:?}");
        assert_eq!(std::fs::read(fixture.service.base().join("state.json")).unwrap(), before);
    }
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
fn cancellation_has_no_retirement_record() {
    let fixture = Fixture::new();
    fixture.mutate(&Actor::Operator(OperatorOrigin::Browser), OrchestrationAction::CancelRun { run_id: "12345678-1234-4234-8234-123456789abd".into() }).unwrap();
    let run = fixture.run();
    assert_eq!(run.close_reason, Some(CloseReason::Cancelled));
    assert!(run.retirement.is_none());
}

#[test]
fn execution_lease_is_exclusive_and_released_without_semantic_mutation() {
    let fixture = Fixture::new();
    let before = serde_json::to_value(fixture.state()).unwrap();
    let lease = fixture.service.execution_lease(ROOT).unwrap().unwrap();
    assert!(fixture.service.execution_lease(ROOT).unwrap().is_none());
    drop(lease);
    assert!(fixture.service.execution_lease(ROOT).unwrap().is_some());
    assert_eq!(serde_json::to_value(fixture.state()).unwrap(), before);
}

#[test]
fn terminal_preflight_fails_closed_on_every_identity_and_shell_mismatch() {
    let (identity, baseline, original) = direct_terminal();
    assert_eq!(classify_terminal(&identity, &baseline, &original), Ok(()));
    for contamination in 0..13 {
        let mut runtime = baseline.clone();
        let mut info = original.clone();
        let reason = match contamination {
            0 => { runtime.endpoint_identity = "restarted".into(); RetainReason::EndpointChanged },
            1 => { runtime.boot_id = Some("rebooted".into()); RetainReason::IdentityChanged },
            2 => { runtime.panes[0].terminal_id = Some("replacement".into()); RetainReason::PaneMoved },
            3 => { runtime.panes[0].workspace_id = "moved".into(); RetainReason::PaneMoved },
            4 => { runtime.panes[0].tab_id = "moved".into(); RetainReason::PaneMoved },
            5 => { runtime.panes[0].agent_kind = Some("foreign".into()); RetainReason::ForegroundProcess },
            6 => { runtime.panes[0].native_session_id = Some("foreign-chat".into()); RetainReason::ForegroundProcess },
            7 => { info.shell_pid = None; RetainReason::ForegroundProcess },
            8 => { info.foreground_pgid = Some(22); RetainReason::ForegroundProcess },
            9 => { info.processes.clear(); RetainReason::ForegroundProcess },
            10 => { info.processes.push((22, "background".into())); RetainReason::ForegroundProcess },
            11 => { info.shell_identity.as_mut().unwrap().process.start_ticks += 1; RetainReason::ForegroundProcess },
            _ => { runtime.boot_id = None; RetainReason::IdentityIncomplete },
        };
        assert_eq!(classify_terminal(&identity, &runtime, &info), Err(TerminalDecision::Retain(reason)));
    }
    let mut moved = baseline;
    moved.panes[0].pane_id = "moved".into();
    assert_eq!(classify_terminal(&identity, &moved, &original), Err(TerminalDecision::Retain(RetainReason::PaneMoved)));
    moved.panes.clear();
    assert_eq!(classify_terminal(&identity, &moved, &original), Err(TerminalDecision::Absent));
}

#[test]
fn exec_replacement_at_identical_pid_and_pgid_cannot_authorize_close() {
    let (identity, runtime, original_shell) = direct_terminal();
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
        Err(TerminalDecision::Retain(RetainReason::ForegroundProcess)));
}

#[test]
fn original_shell_builtin_activity_cannot_be_certified_idle_or_cancelled() {
    let identity = identity();
    let mut runtime = runtime();
    let original_shell = shell();
    let mut sibling = runtime.panes[0].clone();
    sibling.tab_id = "user-tab".into();
    sibling.pane_id = "user-pane".into();
    sibling.terminal_id = Some("user-terminal".into());
    runtime.panes.push(sibling);

    // A prompt and `while :; do :; done` have indistinguishable original-shell
    // PID/PGID/executable/argv evidence. Neither permits automatic cancellation.
    let indistinguishable_builtin = original_shell.clone();
    assert_eq!(classify_terminal(&identity, &runtime, &original_shell),
        Err(TerminalDecision::Retain(RetainReason::UserActivity)));
    assert_eq!(classify_terminal(&identity, &runtime, &indistinguishable_builtin),
        Err(TerminalDecision::Retain(RetainReason::UserActivity)));
    assert_eq!(runtime.panes[1].pane_id, "user-pane");
    assert_eq!(runtime.panes[1].tab_id, "user-tab");
    assert_eq!(runtime.panes.len(), 2);
}



#[test]
fn semantic_conversation_does_not_supply_missing_physical_proof() {
    let mut run = worker();
    run.attachment.as_mut().unwrap().process = None;
    run.attachment.as_mut().unwrap().shell = None;
    let record = retirement_for_accepted(&run, RetirementTrigger::Accept, "revision", AT).unwrap();
    let pinned = record.identity.unwrap();
    assert_eq!(pinned.conversation, Some(conversation()));
    assert!(pinned.process.is_none());
    assert!(pinned.shell.is_none());
    assert!(matches!(classify_terminal(&pinned, &runtime(), &shell()),
        Err(TerminalDecision::Retain(_))), "registered session cannot certify an owned pane is safe to close");
}

#[test]
fn receipt_conversation_compatibility_does_not_relax_accepted_run_identity() {
    for left_conversation in [None, Some(conversation())] {
        for right_conversation in [None, Some(conversation())] {
            let mut left = identity();
            left.conversation = left_conversation.clone();
            let mut right = identity();
            right.conversation = right_conversation;
            assert!(receipt_identity_matches(&left, &right));
            assert!(receipt_identity_matches(&right, &left));
        }
    }
    let mut run = worker();
    run.stage = RunStage::Closed;
    run.close_reason = Some(CloseReason::Accepted);
    let mut retirement = retirement_for_accepted(&run, RetirementTrigger::Accept, "revision", AT).unwrap();
    assert!(identity_matches(&run, &retirement));
    retirement.identity.as_mut().unwrap().conversation = None;
    assert!(receipt_identity_matches(&identity_for(&run).unwrap(), retirement.identity.as_ref().unwrap()));
    assert!(!identity_matches(&run, &retirement), "receipt compatibility grants no physical-effect admission");
    run.attachment.as_mut().unwrap().conversation = None;
    assert!(identity_matches(&run, &retirement));
    retirement.identity.as_mut().unwrap().conversation = Some(conversation());
    assert!(!identity_matches(&run, &retirement), "strict matching must reject missing run metadata too");
}

#[test]
fn accepted_late_conversation_preserves_creation_history_and_checkout_claims() {
    use sha2::{Digest, Sha256};
    for next in [
        RetirementState::Retained { at: AT.into(), reason: RetainReason::NativeProcessUnverifiable, stopped: false },
        RetirementState::Retired { at: AT.into(), terminal: TerminalOutcome::AlreadyAbsent },
        RetirementState::Retired { at: AT.into(), terminal: TerminalOutcome::ClosedByCockpit },
        RetirementState::Retired { at: AT.into(), terminal: TerminalOutcome::AbsentAfterUncertainClose },
    ] {
        let fixture = Fixture::new();
        // Capture the original launch receipt before semantic registration.
        let mut before_registration = fixture.run();
        before_registration.attachment.as_mut().unwrap().conversation = None;
        let identity = OwnedResourceIdentity::WorkerTerminal {
            identity: identity_for(&before_registration).unwrap(),
        };
        let creation_bytes = serde_json::to_vec(&identity).unwrap();
        let resource = OwnedResourceRecord {
            resource_id: format!("{:x}", Sha256::digest(&creation_bytes)),
            root_id: ROOT.into(), run_id: before_registration.run_id.clone(),
            kind: OwnedResourceKind::WorkerTerminal, ownership: ResourceOwnership::Owned,
            creation_operation_id: Some("owned-launch".into()), identity,
            authority: CleanupAuthority::OwnedWhenSafe,
            consumers: vec![AttachmentReference {
                run_id: before_registration.run_id.clone(),
                attachment_id: before_registration.attachment.as_ref().unwrap().attachment_id.clone(),
                attachment_epoch: before_registration.attachment.as_ref().unwrap().epoch,
            }],
            integration_result_id: None,
            cleanup: CleanupItem { operation_id: None, state: CleanupState::Unknown,
                reason: Some(super::cleanup::reason(CleanupReasonClass::Authority,
                    "terminal_identity_changed", "Old conversation mismatch")),
                effects: Vec::new(), updated_at: AT.into() },
        };
        fixture.edit(|state| state.owned_resources.push(resource.clone()));
        fixture.accept();
        let mut before = fixture.state();
        let accepted = &before.runs[1];
        let retirement = accepted.retirement.as_ref().unwrap();
        assert_eq!(retirement.identity.as_ref().unwrap().conversation, Some(conversation()));
        assert!(identity_matches(accepted, retirement));
        assert!(!before.checkout_claims.is_empty());
        let task_path = fixture.service.base().join("tasks").join(format!("{ROOT}.md"));
        let accepted_task_bytes = std::fs::read(&task_path).unwrap();
        fixture.service.record_retirement(&accepted.run_id, &retirement.retirement_id,
            RetirementStateKind::Waiting, next.clone()).unwrap();
        let after = fixture.state();
        assert_eq!(after.runs[1].retirement.as_ref().unwrap().state, next);
        assert_eq!(std::fs::read(task_path).unwrap(), accepted_task_bytes);
        assert_eq!(serde_json::to_vec(&after.owned_resources[0].identity).unwrap(), creation_bytes);
        let mut expected_resource = resource;
        if matches!(next, RetirementState::Retired { .. }) {
            expected_resource.cleanup.effects.push(CleanupEffectReceipt {
                effect_id: retirement.retirement_id.clone(), kind: OwnedResourceKind::WorkerTerminal,
                identity_digest: expected_resource.resource_id.clone(),
                state: if matches!(next, RetirementState::Retired { terminal: TerminalOutcome::ClosedByCockpit, .. }) {
                    CleanupEffectState::Applied
                } else { CleanupEffectState::Absent },
                evidence: None, updated_at: after.runs[1].retirement.as_ref().unwrap().updated_at.clone(),
            });
        }
        assert_eq!(after.owned_resources[0], expected_resource);
        // The transaction may change only retirement bookkeeping and its receipt,
        // never accepted Result/history, semantic ownership, claims or resources.
        before.revision = after.revision;
        before.runs[1].retirement = after.runs[1].retirement.clone();
        before.runs[1].updated_at = after.runs[1].updated_at.clone();
        before.owned_resources[0].cleanup = after.owned_resources[0].cleanup.clone();
        assert_eq!(serde_json::to_value(before).unwrap(), serde_json::to_value(after).unwrap());
    }
}

#[test]
fn reobservation_and_unsubmitted_intent_require_exact_saved_record_and_identity() {
    for state in [
        RetirementState::Retained { at: AT.into(), reason: RetainReason::NativeProcessUnverifiable, stopped: false },
        RetirementState::Retained { at: AT.into(), reason: RetainReason::ObservationUnavailable, stopped: false },
        RetirementState::Retained { at: AT.into(), reason: RetainReason::ForegroundProcess, stopped: false },
        RetirementState::CloseIntent { at: AT.into() },
    ] {
        for changed in 0..5 {
            let fixture = Fixture::new();
            fixture.accept();
            fixture.edit(|s| s.runs[1].retirement.as_mut().unwrap().state = state.clone());
            let run = fixture.run();
            let observed = run.retirement.unwrap();
            assert_eq!(fixture.service.retirement_queue().unwrap().len(), 1);
            match changed {
                0 => {},
                1 => fixture.edit(|s| s.runs[1].retirement.as_mut().unwrap().retirement_id = id()),
                2 => fixture.edit(|s| s.runs[1].retirement.as_mut().unwrap().state = RetirementState::CloseIntent { at: now() }),
                3 => fixture.edit(|s| s.runs[1].retirement.as_mut().unwrap().task_revision = Some("different".into())),
                _ => fixture.edit(|s| s.runs[1].attachment.as_mut().unwrap().epoch += 1),
            }
            let before = fixture.state();
            let result = fixture.service.retirement_waiting(&run.run_id, &observed, vec![]);
            match changed {
                0 => {
                    result.unwrap();
                    assert_eq!(fixture.run().retirement.unwrap().state, RetirementState::Waiting { blockers: vec![] });
                }
                4 => {
                    result.unwrap();
                    assert!(matches!(fixture.run().retirement.unwrap().state,
                        RetirementState::Retained { reason: RetainReason::IdentityChanged, .. }));
                }
                _ => {
                    assert!(result.is_err());
                    assert_eq!(serde_json::to_value(fixture.state()).unwrap(), serde_json::to_value(before).unwrap());
                }
            }
        }
    }
    for reason in [RetainReason::IdentityChanged, RetainReason::IdentityIncomplete, RetainReason::UserActivity,
        RetainReason::SharedTab, RetainReason::TabRenamed, RetainReason::PaneMoved, RetainReason::EndpointChanged,
        RetainReason::HerdrRefused, RetainReason::PrimaryCheckoutSpace] {
        let fixture = Fixture::new();
        fixture.accept();
        fixture.edit(|s| s.runs[1].retirement.as_mut().unwrap().state = RetirementState::Retained {
            at: AT.into(), reason, stopped: false,
        });
        let run = fixture.run();
        assert!(fixture.service.retirement_queue().unwrap().is_empty());
        assert!(fixture.service.retirement_waiting(&run.run_id, run.retirement.as_ref().unwrap(), vec![]).is_err());
    }
}

#[test]
fn replacement_retirement_uses_durable_root_authority_without_inventing_result_pins() {
    let mut previous = worker();
    previous.stage = RunStage::Closed;
    previous.close_reason = Some(CloseReason::Superseded);
    previous.task_id = Some(id());
    previous.result = None;
    previous.source_output = None;
    previous.attachment.as_mut().unwrap().launch_mode = Some(AttachmentLaunchMode::DirectProcess);
    let successor_id = id();
    let operation_id = id();
    let authority = ReplacementRetirementAuthority { operation_id: operation_id.clone(),
        previous_run_id: previous.run_id.clone(), previous_generation: 1, replacement_run_id: successor_id.clone() };
    let record = retirement_for_replacement(&previous, authority, AT).unwrap();
    assert_eq!(record.trigger, RetirementTrigger::Replacement);
    assert!(record.result_message_id.is_none() && record.task_revision.is_none());
    assert!(matches!(record.state, RetirementState::Waiting { .. }));
    previous.retirement = Some(record.clone());
    let mut successor = new_run(&successor_id, SESSION, RunKind::Worker, "successor".into(),
        ROOT, Some(ROOT.into()), previous.task_id.clone(), 8);
    successor.assignment_generation = Some(2);
    let mut state = OrchestrationState { runs: vec![previous.clone(), successor],
        task_ownership: vec![TaskOwnership { root_id: ROOT.into(), task_id: previous.task_id.clone().unwrap(),
            generation: 2, owner: None, schedule: None }], ..Default::default() };
    state.operations.push(super::store::DurableOperationRecord { operation_id: operation_id.clone(),
        actor_run_id: Some(ROOT.into()), principal_key: "root".into(), root_id: ROOT.into(),
        kind: super::store::DurableOperationKind::WorkerReplace, request_digest: "trusted committed replacement".into(),
        run_id: Some(successor_id.clone()), resume_operation_id: None,
        result: Some(OrchestrationActionResult::WorkerReplaced { start: WorkerStartResult {
            operation_id, run_id: successor_id, attempt: 8, duplicate: false },
            previous_run_id: previous.run_id.clone(), assignment_generation: 2,
            placement: ReplacementPlacement::IsolatedWorktree { base_commit: "a".repeat(40) } }),
        effect_receipts: Vec::new(), created_at: AT.into(), updated_at: AT.into() });
    assert!(authority_matches(&state, &previous, &record));
    for corrupt in 0..5 {
        let mut changed = record.clone();
        match corrupt {
            0 => changed.replacement.as_mut().unwrap().operation_id = id(),
            1 => changed.replacement.as_mut().unwrap().previous_generation = 2,
            2 => changed.replacement.as_mut().unwrap().replacement_run_id = id(),
            3 => changed.result_message_id = Some("not a replacement Result".into()),
            _ => changed.task_revision = Some("not a replacement task revision".into()),
        }
        assert!(!authority_matches(&state, &previous, &changed), "invalid authority {corrupt}");
    }
    state.task_ownership[0].generation = 1;
    assert!(!authority_matches(&state, &previous, &record));
}

#[test]
fn same_fd_peer_fence_handles_real_bootless_shape_without_none_or_conflicting_proof() {
    let mut identity = identity();
    identity.herdr_boot_id = None;
    let peer = NativeProcessIdentity { pid: 700, start_ticks: 800,
        kernel_boot_id: Some("12345678-1234-4234-8234-123456789abc".into()) };
    identity.herdr_peer = Some(peer.clone());
    let mut view = runtime();
    view.boot_id = None;
    view.peer_process = Some(peer);
    assert!(server_fence(&identity, &view).is_ok());
    view.peer_process.as_mut().unwrap().start_ticks += 1;
    assert_eq!(server_fence(&identity, &view), Err(RetainReason::IdentityChanged));
    view.peer_process = identity.herdr_peer.clone();
    identity.herdr_boot_id = Some("legacy-one".into());
    view.boot_id = Some("legacy-two".into());
    assert_eq!(server_fence(&identity, &view), Err(RetainReason::IdentityChanged));
    identity.herdr_boot_id = None;
    view.boot_id = None;
    identity.herdr_peer = None;
    view.peer_process = None;
    assert_eq!(server_fence(&identity, &view), Err(RetainReason::IdentityIncomplete));
    let no_proof = classify_close(&identity, Some(Ok(())), Some(&view));
    assert!(matches!(no_proof, RetirementState::Unknown { .. }), "an ACK and absent/unknown peer cannot prove closure");
}

#[test]
fn old_serialized_accepted_retirement_strings_remain_some_without_replacement_authority() {
    let record = retirement_for_accepted(&worker(), RetirementTrigger::Accept, "revision", AT).unwrap();
    let mut value = serde_json::to_value(&record).unwrap();
    value.as_object_mut().unwrap().remove("replacement");
    assert_eq!(value["result_message_id"], "reviewed-result");
    let restored: RunRetirement = serde_json::from_value(value).unwrap();
    assert_eq!(restored.result_message_id.as_deref(), Some("reviewed-result"));
    assert_eq!(restored.task_revision.as_deref(), Some("revision"));
    assert!(restored.replacement.is_none() && record_shape(&restored));
}
